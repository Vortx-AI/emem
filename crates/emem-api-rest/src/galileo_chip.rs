//! Sentinel-2 → Galileo-Tiny S2-only chip fetcher.
//!
//! Galileo-Tiny is multimodal but its smallest in-distribution input
//! is a single S2 timestep over a small spatial window. We fetch a
//! `[T=1, H=8, W=8, 10]` chip at uniform 30 m sampling — physical
//! extent 240 m × 240 m centred on the cell. The 10 bands are the
//! Galileo `S2_BANDS` list:
//!
//!   `B2 (Blue), B3 (Green), B4 (Red), B5 (RE1), B6 (RE2), B7 (RE3),`
//!   `B8 (NIR-10m), B8A (NIR-20m), B11 (SWIR1), B12 (SWIR2)`.
//!
//! Bands at native S2 resolutions: 10 m for B2/B3/B4/B8; 20 m for
//! B5/B6/B7/B8A/B11/B12. To land on the 8×8 / 30 m target:
//!   * 10 m bands → fetch 24×24 window → 3:1 mean-pool (`block_mean_pool`)
//!   * 20 m bands → fetch 12×12 window → 1.5:1 bilinear
//!
//! Reuses the resampling helpers + `s2_search_with_fallback` from
//! `prithvi_chip` so the two paths share STAC + COG infrastructure.

use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::prithvi_chip::{block_mean_pool_pub, resample_to_chip_pub};
use crate::{parse_iso8601_unix, s2_http_client, s2_search_with_fallback, AppState};

/// Galileo S2_BANDS in canonical order — must match the
/// `S2_BANDS` list in `single_file_galileo.py`.
const GALILEO_S2_BANDS: &[(&str, &[&str])] = &[
    ("B2", &["blue", "B02"]),
    ("B3", &["green", "B03"]),
    ("B4", &["red", "B04"]),
    ("B5", &["rededge1", "B05"]),
    ("B6", &["rededge2", "B06"]),
    ("B7", &["rededge3", "B07"]),
    ("B8", &["nir", "B08"]),
    ("B8A", &["nir08", "B8A"]),
    ("B11", &["swir16", "B11"]),
    ("B12", &["swir22", "B12"]),
];

/// Galileo chip target: 8 spatial × 30 m = 240 m extent at the cell.
pub const GALILEO_CHIP_PIXELS: u32 = 8;
pub const GALILEO_CHIP_T: u32 = 1;
pub const GALILEO_CHIP_RESOLUTION_M: f64 = 30.0;
const GALILEO_PHYSICAL_EXTENT_M: f64 = (GALILEO_CHIP_PIXELS as f64) * GALILEO_CHIP_RESOLUTION_M;

/// Result of one Galileo chip fetch. `chip` is row-major
/// `[T=1 * H=8 * W=8 * 10]` f32 — Galileo's `[T, H, W, C]` layout.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GalileoChip {
    pub chip: Vec<f32>,
    pub scene_id: String,
    pub scene_iso: String,
    pub scene_unix: i64,
    pub asset_urls: Vec<String>,
    pub scene_cloud_cover: Option<f64>,
    pub used_cloud: f64,
    pub used_days: i64,
}

impl GalileoChip {
    /// Build the `[[[[f32; 10]; W]; H]; T]` JSON the sidecar's
    /// `GalileoRequest.s2_chip` expects.
    pub fn as_4d(&self) -> Vec<Vec<Vec<Vec<f32>>>> {
        let h = GALILEO_CHIP_PIXELS as usize;
        let w = GALILEO_CHIP_PIXELS as usize;
        let t = GALILEO_CHIP_T as usize;
        let bands = GALILEO_S2_BANDS.len();
        let mut out: Vec<Vec<Vec<Vec<f32>>>> = Vec::with_capacity(t);
        // `chip` is laid out band-major (one full plane per band) so we
        // need to transpose into [T][H][W][bands] for the wire shape.
        for ti in 0..t {
            let mut plane: Vec<Vec<Vec<f32>>> = Vec::with_capacity(h);
            for r in 0..h {
                let mut row: Vec<Vec<f32>> = Vec::with_capacity(w);
                for c in 0..w {
                    let mut px: Vec<f32> = Vec::with_capacity(bands);
                    for b in 0..bands {
                        // Layout: chip[band][t][r][c] flattened band-major.
                        let idx = (b * t * h * w) + (ti * h * w) + (r * w) + c;
                        px.push(self.chip[idx]);
                    }
                    row.push(px);
                }
                plane.push(row);
            }
            out.push(plane);
        }
        out
    }
}

/// Fetch the 10-band Galileo S2 chip for `cell64`. Returns either a
/// fully populated `GalileoChip` or a string error.
pub async fn fetch_galileo_chip(
    cell64: &str,
    _s: &AppState,
    target_unix: Option<i64>,
) -> Result<GalileoChip, String> {
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

    let h = GALILEO_CHIP_PIXELS as usize;
    let w = GALILEO_CHIP_PIXELS as usize;
    let t = GALILEO_CHIP_T as usize;
    let bands = GALILEO_S2_BANDS.len();
    let mut chip_flat: Vec<f32> = Vec::with_capacity(bands * t * h * w);
    let mut asset_urls: Vec<String> = Vec::with_capacity(bands);

    for (label, aliases) in GALILEO_S2_BANDS {
        let url = aliases
            .iter()
            .find_map(|a| item.assets.get(*a).cloned())
            .ok_or_else(|| {
                format!(
                    "stac item missing any of {:?} for Galileo S2 band {label}",
                    aliases
                )
            })?;
        let prof = emem_fetch::cog::open_profile(&cli, &url)
            .await
            .map_err(|e| format!("open COG {url}: {e}"))?;

        let native_m = prof.pixel_scale.0.abs();
        if !(native_m > 0.0 && native_m.is_finite()) {
            return Err(format!(
                "COG {url} has implausible pixel_scale {:?}",
                prof.pixel_scale
            ));
        }
        let native_pixels = (GALILEO_PHYSICAL_EXTENT_M / native_m).round() as u32;
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

        // Resample to 8×8 at 30 m equiv. The integer-divisor branch
        // catches 24/8 = 3 (10 m bands); bilinear handles 12/8 = 1.5
        // (20 m bands).
        let mut resampled = if native_pixels == GALILEO_CHIP_PIXELS {
            raw.iter().map(|v| *v as f32).collect()
        } else if native_pixels.is_multiple_of(GALILEO_CHIP_PIXELS) {
            let k = (native_pixels / GALILEO_CHIP_PIXELS) as usize;
            block_mean_pool_pub(&raw, native_pixels as usize, native_pixels as usize, k)
        } else {
            resample_to_chip_pub(&raw, native_pixels, native_pixels, GALILEO_CHIP_PIXELS)
        };
        if resampled.len() != h * w {
            return Err(format!(
                "Galileo resample produced {} samples, want {}",
                resampled.len(),
                h * w
            ));
        }
        // T=1 — just append the plane as-is. When we ever go to T>1 the
        // outer loop here picks up the temporal dimension and we
        // interleave per-timestep planes.
        chip_flat.append(&mut resampled);
    }
    asset_urls.extend(
        GALILEO_S2_BANDS
            .iter()
            .filter_map(|(_, aliases)| aliases.iter().find_map(|a| item.assets.get(*a).cloned())),
    );

    Ok(GalileoChip {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `as_4d` round-trips the band-major flat buffer into Galileo's
    /// expected `[T][H][W][C]` JSON layout.
    #[test]
    fn galileo_chip_as_4d_layout() {
        let h = GALILEO_CHIP_PIXELS as usize;
        let w = GALILEO_CHIP_PIXELS as usize;
        let t = GALILEO_CHIP_T as usize;
        let bands = GALILEO_S2_BANDS.len();
        let total = bands * t * h * w;
        let mut flat = Vec::with_capacity(total);
        for b in 0..bands {
            for _ in 0..(t * h * w) {
                flat.push(b as f32); // every plane filled with band index
            }
        }
        let chip = GalileoChip {
            chip: flat,
            scene_id: "S2A_TEST".into(),
            scene_iso: "2024-06-01T00:00:00Z".into(),
            scene_unix: 0,
            asset_urls: vec![],
            scene_cloud_cover: None,
            used_cloud: 40.0,
            used_days: 30,
        };
        let four_d = chip.as_4d();
        assert_eq!(four_d.len(), t);
        assert_eq!(four_d[0].len(), h);
        assert_eq!(four_d[0][0].len(), w);
        assert_eq!(four_d[0][0][0].len(), bands);
        // Pixel (t=0, r=0, c=0) should carry [0, 1, 2, 3, ..., 9] —
        // one value per band.
        assert_eq!(
            four_d[0][0][0],
            (0..bands as u32).map(|i| i as f32).collect::<Vec<_>>()
        );
    }
}
