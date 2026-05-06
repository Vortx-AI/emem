//! Sentinel-2 → Prithvi-EO-2.0 chip fetcher.
//!
//! Goal: produce a `[6, 224, 224]` reflectance chip at uniform 30 m
//! sampling, in HLS V2 band order (Blue, Green, Red, Narrow-NIR,
//! SWIR1, SWIR2). Physical extent: 224 × 30 m = **6720 m × 6720 m**
//! centred on the cell. This matches the spatial scale Prithvi was
//! pretrained on (HLS V2 30 m), so the embedding the sidecar produces
//! is in-distribution.
//!
//! Why not HLS V2 directly: HLS V2 isn't wired in this responder.
//! Sentinel-2 L2A IS wired (per-cell COG range reads against AWS
//! Open Data via `materialize_sentinel2_band`) and shares the same
//! atmospheric correction lineage as HLS-S30 (Sen2Cor). The chip
//! the model sees is therefore close-but-not-identical to its
//! pretraining distribution — small Landsat-9 cross-sensor terms
//! that HLS V2 harmonizes are absent. The receipt's
//! `honesty_warnings` flags this as `s2_l2a_substitute_for_hls_v2`.
//!
//! Native Sentinel-2 resolutions per band:
//!   * B02 / B03 / B04                 → 10 m
//!   * B8A / B11 / B12                 → 20 m
//!
//! To land on a uniform 30 m / 224×224 grid we:
//!   * fetch a 672×672 window at 10 m for B02/B03/B04, then 3:1 mean-pool
//!   * fetch a 336×336 window at 20 m for B8A/B11/B12, then 1.5:1 area-resize
//!
//! Output values are raw S2 L2A surface reflectance counts (0–10000 nominal),
//! which is the same scale the Prithvi mean/std vector
//! `[1087, 1342, 1433, 2734, 1958, 1363]` is computed against. The
//! sidecar handles per-band normalization.

use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::{parse_iso8601_unix, s2_http_client, s2_search_with_fallback, AppState};

/// HLS V2 band order Prithvi expects, mapped to Sentinel-2 L2A asset
/// aliases (the STAC item exposes these names on AWS Open Data).
/// Each entry is `(prithvi_band_name, [s2_asset_aliases])`.
const PRITHVI_BANDS: &[(&str, &[&str])] = &[
    ("Blue", &["blue", "B02"]),
    ("Green", &["green", "B03"]),
    ("Red", &["red", "B04"]),
    ("Narrow NIR", &["nir08", "B8A"]),
    ("SWIR1", &["swir16", "B11"]),
    ("SWIR2", &["swir22", "B12"]),
];

/// Prithvi target spatial scale: 30 m × 224 = 6720 m extent.
pub const PRITHVI_CHIP_PIXELS: u32 = 224;
pub const PRITHVI_CHIP_RESOLUTION_M: f64 = 30.0;
const PRITHVI_PHYSICAL_EXTENT_M: f64 = (PRITHVI_CHIP_PIXELS as f64) * PRITHVI_CHIP_RESOLUTION_M;

/// Result of one chip fetch: `chip` is row-major `[6, 224, 224]` f32 in
/// HLS V2 band order, raw reflectance counts. The remaining fields
/// carry the input-provenance the responder cites in the signed receipt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrithviChip {
    /// Flattened `[6 * 224 * 224]` reflectance counts (0–10000 nominal).
    pub chip: Vec<f32>,
    /// As `[[[f32; 224]; 224]; 6]` for the JSON request to the sidecar.
    /// Built lazily by `chip_as_3d()` to avoid double allocation.
    pub scene_id: String,
    pub scene_iso: String,
    pub scene_unix: i64,
    /// Asset URLs the chip was sourced from (one per band, same order).
    pub asset_urls: Vec<String>,
    /// Cloud cover fraction the upstream STAC reported for the picked scene.
    pub scene_cloud_cover: Option<f64>,
    /// The cloud / lookback tier the search settled on (cf.
    /// `s2_search_with_fallback`'s 3-tier ladder).
    pub used_cloud: f64,
    pub used_days: i64,
}

impl PrithviChip {
    /// Build the `[[[f32; 224]; 224]; 6]` JSON the sidecar's
    /// `PrithviRequest.chip` expects.
    pub fn as_3d(&self) -> Vec<Vec<Vec<f32>>> {
        let n = PRITHVI_CHIP_PIXELS as usize;
        let mut out: Vec<Vec<Vec<f32>>> = Vec::with_capacity(6);
        for b in 0..6 {
            let mut plane: Vec<Vec<f32>> = Vec::with_capacity(n);
            for r in 0..n {
                let start = (b * n * n) + (r * n);
                plane.push(self.chip[start..start + n].to_vec());
            }
            out.push(plane);
        }
        out
    }
}

/// Fetch the 6-band Prithvi chip for `cell64`. Returns either a fully
/// populated `PrithviChip` or a string error suitable for surfacing as
/// a 5xx via `ApiError`.
pub async fn fetch_prithvi_chip(
    cell64: &str,
    _s: &AppState,
    target_unix: Option<i64>,
) -> Result<PrithviChip, String> {
    let info = emem_codec::latlng_from_cell64(cell64).map_err(|e| format!("cell decode: {e}"))?;
    let lat = info.lat_deg;
    let lng = info.lng_deg;

    let now_unix = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let cli = s2_http_client();
    let (item, used_cloud, used_days) =
        s2_search_with_fallback(&cli, lng, lat, target_unix, now_unix).await?;
    let epsg = item
        .epsg
        .ok_or_else(|| "stac item missing proj:epsg".to_string())?;
    let utm = emem_fetch::proj::latlng_to_utm_with_epsg(lat, lng, epsg)
        .ok_or_else(|| format!("epsg {epsg} not a UTM code"))?;

    let mut chip_flat: Vec<f32> =
        Vec::with_capacity(6 * (PRITHVI_CHIP_PIXELS as usize) * (PRITHVI_CHIP_PIXELS as usize));
    let mut asset_urls: Vec<String> = Vec::with_capacity(6);

    for (label, aliases) in PRITHVI_BANDS {
        let url = aliases
            .iter()
            .find_map(|a| item.assets.get(*a).cloned())
            .ok_or_else(|| {
                format!(
                    "stac item missing any of {:?} for Prithvi band {label}",
                    aliases
                )
            })?;
        let prof = emem_fetch::cog::open_profile(&cli, &url)
            .await
            .map_err(|e| format!("open COG {url}: {e}"))?;

        // Pixel count to fetch at the band's NATIVE resolution so the
        // physical extent is uniform 6720 m. Round-half-to-even on the
        // ratio so 30 m bands (theoretical, not present in S2 today)
        // would land cleanly.
        let native_m = prof.pixel_scale.0.abs();
        if !(native_m > 0.0 && native_m.is_finite()) {
            return Err(format!(
                "COG {url} has implausible pixel_scale {:?}",
                prof.pixel_scale
            ));
        }
        let native_pixels = (PRITHVI_PHYSICAL_EXTENT_M / native_m).round() as u32;
        let raw = emem_fetch::cog::sample_window(
            &cli,
            &url,
            &prof,
            utm.easting,
            utm.northing,
            native_pixels,
            native_pixels,
        )
        .await
        .map_err(|e| format!("sample_window {url}: {e}"))?;

        let resampled = resample_to_chip(&raw, native_pixels, native_pixels, PRITHVI_CHIP_PIXELS);
        if resampled.len() != (PRITHVI_CHIP_PIXELS as usize).pow(2) {
            return Err(format!(
                "resample produced {} samples, want {}",
                resampled.len(),
                (PRITHVI_CHIP_PIXELS as usize).pow(2)
            ));
        }
        chip_flat.extend(resampled);
        asset_urls.push(url);
    }

    Ok(PrithviChip {
        chip: chip_flat,
        scene_id: item.id.clone(),
        scene_iso: item.datetime.clone(),
        scene_unix: parse_iso8601_unix(&item.datetime).unwrap_or(0),
        asset_urls,
        scene_cloud_cover: item.cloud_cover,
        used_cloud,
        used_days,
    })
}

/// Resample `src` (size `src_w × src_h`) to `dst_n × dst_n`. Picks the
/// fastest correct path based on the ratio:
///
/// * Integer down-ratio (`src_w == k * dst_n`): k×k mean pooling. For the
///   10 m → 30 m case (k=3) this is exact down-sampling and preserves
///   scale-invariance the model expects.
/// * Otherwise: bilinear interpolation. For the 20 m → 30 m case
///   (1.5:1) bilinear is the standard choice; we accept the small
///   anti-aliasing imperfection because the alternative (true area-resize)
///   is many lines of code we don't yet have a use for.
///
/// Inputs are `f64` (the COG sampler's native dtype); outputs are `f32`
/// because the model + the wire response are f32.
/// Public re-export — same dispatch as `resample_to_chip` but
/// accessible to `galileo_chip` without duplicating the integer/
/// bilinear branching.
pub(crate) fn resample_to_chip_pub(src: &[f64], src_w: u32, src_h: u32, dst_n: u32) -> Vec<f32> {
    resample_to_chip(src, src_w, src_h, dst_n)
}

/// Public re-export — same k×k mean pool used by the integer-divisor
/// branch above, exposed for Galileo's 24×24 → 8×8 (k=3) pool.
pub(crate) fn block_mean_pool_pub(src: &[f64], src_w: usize, src_h: usize, k: usize) -> Vec<f32> {
    block_mean_pool(src, src_w, src_h, k)
}

fn resample_to_chip(src: &[f64], src_w: u32, src_h: u32, dst_n: u32) -> Vec<f32> {
    if src_w == dst_n && src_h == dst_n {
        return src.iter().map(|v| *v as f32).collect();
    }
    if src_w == src_h && src_w % dst_n == 0 {
        let k = (src_w / dst_n) as usize;
        return block_mean_pool(src, src_w as usize, src_h as usize, k);
    }
    bilinear_resize(src, src_w, src_h, dst_n, dst_n)
}

/// k×k mean pooling for integer down-ratios. `src` is row-major
/// `src_w × src_h`. Output is `(src_w / k) × (src_h / k)`.
fn block_mean_pool(src: &[f64], src_w: usize, src_h: usize, k: usize) -> Vec<f32> {
    let dst_w = src_w / k;
    let dst_h = src_h / k;
    let mut out = Vec::with_capacity(dst_w * dst_h);
    let inv = 1.0 / (k as f64 * k as f64);
    for dy in 0..dst_h {
        for dx in 0..dst_w {
            let mut s = 0.0_f64;
            let row0 = dy * k;
            let col0 = dx * k;
            for r in row0..row0 + k {
                let row_off = r * src_w;
                for c in col0..col0 + k {
                    s += src[row_off + c];
                }
            }
            out.push((s * inv) as f32);
        }
    }
    out
}

/// Standard bilinear resize for non-integer ratios. Edge pixels clamp
/// (no replication artefacts for our 1.5:1 case).
fn bilinear_resize(src: &[f64], src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> Vec<f32> {
    let mut out = Vec::with_capacity((dst_w as usize) * (dst_h as usize));
    let sx = (src_w as f64 - 1.0) / ((dst_w as f64 - 1.0).max(1.0));
    let sy = (src_h as f64 - 1.0) / ((dst_h as f64 - 1.0).max(1.0));
    for dy in 0..dst_h {
        let fy = (dy as f64) * sy;
        let y0 = fy.floor() as i64;
        let y1 = (y0 + 1).min(src_h as i64 - 1);
        let dy_f = fy - (y0 as f64);
        let y0c = y0.clamp(0, src_h as i64 - 1) as usize;
        let y1c = y1.clamp(0, src_h as i64 - 1) as usize;
        for dx in 0..dst_w {
            let fx = (dx as f64) * sx;
            let x0 = fx.floor() as i64;
            let x1 = (x0 + 1).min(src_w as i64 - 1);
            let dx_f = fx - (x0 as f64);
            let x0c = x0.clamp(0, src_w as i64 - 1) as usize;
            let x1c = x1.clamp(0, src_w as i64 - 1) as usize;
            let p00 = src[y0c * (src_w as usize) + x0c];
            let p01 = src[y0c * (src_w as usize) + x1c];
            let p10 = src[y1c * (src_w as usize) + x0c];
            let p11 = src[y1c * (src_w as usize) + x1c];
            let top = p00 * (1.0 - dx_f) + p01 * dx_f;
            let bot = p10 * (1.0 - dx_f) + p11 * dx_f;
            out.push((top * (1.0 - dy_f) + bot * dy_f) as f32);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 3×3 identity input → 3×3 mean pool with k=3 = single-pixel mean.
    /// Sanity check the row-major layout + averaging math.
    #[test]
    fn block_mean_pool_3x3_one_block() {
        let src = vec![
            1.0, 2.0, 3.0, //
            4.0, 5.0, 6.0, //
            7.0, 8.0, 9.0,
        ];
        let out = block_mean_pool(&src, 3, 3, 3);
        assert_eq!(out, vec![5.0_f32]); // mean of 1..9
    }

    /// 6×6 with k=3 → 2×2 output. Each output cell is the mean of a
    /// 3×3 block of the input.
    #[test]
    fn block_mean_pool_6x6_k3() {
        // Block (0,0) = all 1s, block (0,1) = all 2s, etc.
        let src: Vec<f64> = (0..36)
            .map(|i| {
                let r = i / 6;
                let c = i % 6;
                let br = r / 3;
                let bc = c / 3;
                (br * 2 + bc + 1) as f64
            })
            .collect();
        let out = block_mean_pool(&src, 6, 6, 3);
        assert_eq!(out, vec![1.0_f32, 2.0, 3.0, 4.0]);
    }

    /// 4×4 → 2×2 bilinear: output should match exact corner samples.
    #[test]
    fn bilinear_resize_corners_match() {
        let src = vec![
            1.0, 2.0, 3.0, 4.0, //
            5.0, 6.0, 7.0, 8.0, //
            9.0, 10.0, 11.0, 12.0, //
            13.0, 14.0, 15.0, 16.0,
        ];
        let out = bilinear_resize(&src, 4, 4, 2, 2);
        // Corner samples: (0,0), (0,3), (3,0), (3,3) = 1, 4, 13, 16.
        assert_eq!(out, vec![1.0_f32, 4.0, 13.0, 16.0]);
    }

    /// 1:1 passthrough — chip resolution match means f64 → f32 cast only.
    #[test]
    fn resample_to_chip_passthrough() {
        let src = vec![0.5_f64, 1.5, 2.5, 3.5];
        let out = resample_to_chip(&src, 2, 2, 2);
        assert_eq!(out, vec![0.5_f32, 1.5, 2.5, 3.5]);
    }

    /// 6×6 → 2×2 hits the integer-divisor block-pool branch.
    #[test]
    fn resample_to_chip_dispatches_to_block_pool_on_integer() {
        let src: Vec<f64> = (1..=36).map(|i| i as f64).collect();
        let out = resample_to_chip(&src, 6, 6, 2);
        assert_eq!(out.len(), 4);
    }

    /// 3×3 → 2×2 forces the bilinear branch (3/2 is not integer).
    #[test]
    fn resample_to_chip_dispatches_to_bilinear_on_non_integer() {
        let src = vec![
            1.0, 2.0, 3.0, //
            4.0, 5.0, 6.0, //
            7.0, 8.0, 9.0,
        ];
        let out = resample_to_chip(&src, 3, 3, 2);
        // Bilinear with corner-sampling gives extreme corners verbatim.
        assert!((out[0] - 1.0).abs() < 1e-6);
        assert!((out[3] - 9.0).abs() < 1e-6);
    }

    /// PrithviChip::as_3d should reshape the flat row-major buffer
    /// into the [6][224][224] JSON the sidecar's pydantic schema
    /// validates against.
    #[test]
    fn prithvi_chip_as_3d_layout() {
        // Build a tiny synthetic chip: 6 bands, each plane filled with
        // band_index * 1000 + flat_pixel_index. Then decode to 3D and
        // verify a couple of indices.
        let n = PRITHVI_CHIP_PIXELS as usize;
        let chip_len = 6 * n * n;
        let mut flat = Vec::with_capacity(chip_len);
        for b in 0..6 {
            for i in 0..(n * n) {
                flat.push((b * 1000 + i) as f32);
            }
        }
        let chip = PrithviChip {
            chip: flat,
            scene_id: "S2A_TEST".into(),
            scene_iso: "2024-06-01T00:00:00Z".into(),
            scene_unix: 0,
            asset_urls: vec![],
            scene_cloud_cover: Some(5.0),
            used_cloud: 40.0,
            used_days: 30,
        };
        let three_d = chip.as_3d();
        assert_eq!(three_d.len(), 6);
        assert_eq!(three_d[0].len(), n);
        assert_eq!(three_d[0][0].len(), n);
        // Band 0, row 0, col 0 = 0
        assert_eq!(three_d[0][0][0], 0.0);
        // Band 3, row 5, col 7 = 3*1000 + 5*224 + 7
        assert_eq!(three_d[3][5][7], (3 * 1000 + 5 * 224 + 7) as f32);
    }
}
