//! Sentinel-2 → Clay Foundation Model v1.5 chip fetcher.
//!
//! Mirrors `prithvi_chip.rs` but produces the 10-band 256×256 input
//! Clay v1.5's wavelength-conditioned ViT-L/8 expects. The band order
//! is verbatim from `configs/metadata.yaml` in the Clay repo:
//!
//!   blue, green, red, rededge1, rededge2, rededge3,
//!   nir, nir08, swir16, swir22
//!
//! Clay anchors at 10 m pitch (256 px × 10 m = 2560 m extent), so 10 m
//! bands (B02/B03/B04/B08) fetch at 256×256 native and 20 m bands
//! (B05/B06/B07/B8A/B11/B12) fetch at 128×128 native and resample up
//! to 256×256. The resampler reuses the bilinear path exposed via
//! `prithvi_chip::resample_to_chip_pub`.
//!
//! Per the Clay wall-to-wall tutorial, raw S2 L2A reflectance counts
//! (0..10000) flow to the sidecar verbatim — the sidecar applies the
//! per-band mean/std normalisation via `torchvision.transforms.v2.Normalize`
//! before the encoder forward pass.

use serde::{Deserialize, Serialize};
use std::time::SystemTime;

use crate::{parse_iso8601_unix, prithvi_chip, s2_http_client, s2_search_with_fallback, AppState};

/// Clay v1.5 S2 L2A band order (metadata.yaml). Each entry is
/// `(clay_band_name, [s2_asset_aliases])`. The sidecar's `wavelengths`
/// vector uses this same order.
const CLAY_BANDS: &[(&str, &[&str])] = &[
    ("blue", &["blue", "B02"]),
    ("green", &["green", "B03"]),
    ("red", &["red", "B04"]),
    ("rededge1", &["rededge1", "B05"]),
    ("rededge2", &["rededge2", "B06"]),
    ("rededge3", &["rededge3", "B07"]),
    ("nir", &["nir", "B08"]),
    ("nir08", &["nir08", "B8A"]),
    ("swir16", &["swir16", "B11"]),
    ("swir22", &["swir22", "B12"]),
];

/// Clay target spatial scale: 10 m × 256 = 2560 m extent.
pub const CLAY_CHIP_PIXELS: u32 = 256;
pub const CLAY_CHIP_RESOLUTION_M: f64 = 10.0;
const CLAY_PHYSICAL_EXTENT_M: f64 = (CLAY_CHIP_PIXELS as f64) * CLAY_CHIP_RESOLUTION_M;
const CLAY_BANDS_LEN: usize = 10;

/// Result of one chip fetch: `chip` is row-major
/// `[10, 256, 256]` f32 in Clay's metadata.yaml band order, raw
/// S2 L2A reflectance counts. The remaining fields carry the
/// input-provenance the responder cites in the signed receipt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClayChip {
    /// Flattened `[10 * 256 * 256]` reflectance counts (0–10000 nominal).
    pub chip: Vec<f32>,
    pub scene_id: String,
    pub scene_iso: String,
    pub scene_unix: i64,
    /// Asset URLs the chip was sourced from (one per band, same order).
    pub asset_urls: Vec<String>,
    /// Cloud cover fraction the upstream STAC reported for the picked scene.
    pub scene_cloud_cover: Option<f64>,
    /// The cloud / lookback tier the search settled on.
    pub used_cloud: f64,
    pub used_days: i64,
}

impl ClayChip {
    /// Build the `[[[f32; 256]; 256]; 10]` JSON shape the sidecar's
    /// `ClayRequest.chip` field expects (`Vec<Vec<Vec<f32>>>`).
    pub fn as_3d(&self) -> Vec<Vec<Vec<f32>>> {
        let n = CLAY_CHIP_PIXELS as usize;
        let mut out: Vec<Vec<Vec<f32>>> = Vec::with_capacity(CLAY_BANDS_LEN);
        for b in 0..CLAY_BANDS_LEN {
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

/// Fetch the 10-band Clay chip for `cell64`. Returns either a fully
/// populated `ClayChip` or a string error suitable for surfacing as a
/// 5xx via `ApiError`. Reuses the same STAC + cloud fallback ladder as
/// the Prithvi fetcher.
pub async fn fetch_clay_chip(
    cell64: &str,
    _s: &AppState,
    target_unix: Option<i64>,
) -> Result<ClayChip, String> {
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

    let mut chip_flat: Vec<f32> = Vec::with_capacity(
        CLAY_BANDS_LEN * (CLAY_CHIP_PIXELS as usize) * (CLAY_CHIP_PIXELS as usize),
    );
    let mut asset_urls: Vec<String> = Vec::with_capacity(CLAY_BANDS_LEN);

    for (label, aliases) in CLAY_BANDS {
        let url = aliases
            .iter()
            .find_map(|a| item.assets.get(*a).cloned())
            .ok_or_else(|| {
                format!(
                    "stac item missing any of {:?} for Clay band {label}",
                    aliases
                )
            })?;
        let prof = emem_fetch::cog::open_profile(&cli, &url)
            .await
            .map_err(|e| format!("open COG {url}: {e}"))?;

        // Pixel count to fetch at the band's NATIVE resolution so the
        // physical extent is uniform 2560 m. 10 m bands → 256 px,
        // 20 m bands → 128 px.
        let native_m = prof.pixel_scale.0.abs();
        if !(native_m > 0.0 && native_m.is_finite()) {
            return Err(format!(
                "COG {url} has implausible pixel_scale {:?}",
                prof.pixel_scale
            ));
        }
        let native_pixels = (CLAY_PHYSICAL_EXTENT_M / native_m).round() as u32;
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

        let resampled = prithvi_chip::resample_to_chip_pub(
            &raw,
            native_pixels,
            native_pixels,
            CLAY_CHIP_PIXELS,
        );
        if resampled.len() != (CLAY_CHIP_PIXELS as usize).pow(2) {
            return Err(format!(
                "resample produced {} samples, want {}",
                resampled.len(),
                (CLAY_CHIP_PIXELS as usize).pow(2)
            ));
        }
        chip_flat.extend(resampled);
        asset_urls.push(url);
    }

    Ok(ClayChip {
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

    /// Round-trip the row-major flatten + `as_3d()` shape so the
    /// sidecar always sees `[10, 256, 256]` regardless of any future
    /// constant changes.
    #[test]
    fn clay_chip_as_3d_layout() {
        let n = CLAY_CHIP_PIXELS as usize;
        let mut chip = Vec::with_capacity(CLAY_BANDS_LEN * n * n);
        for b in 0..CLAY_BANDS_LEN {
            for r in 0..n {
                for c in 0..n {
                    chip.push(((b * 1_000_000) + (r * 1000) + c) as f32);
                }
            }
        }
        let cc = ClayChip {
            chip,
            scene_id: "test".into(),
            scene_iso: "2026-05-10T00:00:00Z".into(),
            scene_unix: 0,
            asset_urls: vec![],
            scene_cloud_cover: None,
            used_cloud: 20.0,
            used_days: 14,
        };
        let three_d = cc.as_3d();
        assert_eq!(three_d.len(), CLAY_BANDS_LEN);
        assert_eq!(three_d[0].len(), n);
        assert_eq!(three_d[0][0].len(), n);
        // Spot-check the linearization: band 3, row 7, col 11 should
        // recover (3*1_000_000 + 7*1000 + 11) = 3_007_011.
        assert_eq!(three_d[3][7][11], 3_007_011.0);
    }
}
