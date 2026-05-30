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
use crate::{
    parse_iso8601_unix, s1_search_with_fallback, s2_http_client, s2_search_with_fallback, AppState,
};

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

// ── Galileo multimodal: S1 (VV,VH) + DEM (elevation, slope) ───────────────
//
// Galileo is multimodal (arXiv:2502.09356). Its space-time tensor carries
// S1 (VV,VH γ0 dB) alongside the S2 bands; its space tensor carries the
// SRTM group (elevation, slope). We fetch S1 + DEM at the SAME cell+time
// the S2 chip is sampled, on the same 8×8 / 30 m target grid, so all three
// modalities co-register. When S1 or DEM can't be fetched the caller falls
// back to S2-only and signs under the S2-only fn_key — never zero-filled
// and claimed multimodal.

/// 8×8 S1 chip at 30 m: VV and VH γ0 in dB, each a row-major H*W plane.
/// `[vv_plane, vh_plane]` band-major, matching `GalileoChip.chip` layout.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GalileoS1Chip {
    /// `[2 * H * W]` band-major: VV plane then VH plane, dB. T=1.
    pub chip: Vec<f32>,
    pub scene_id: String,
    pub scene_iso: String,
    pub scene_unix: i64,
    pub asset_urls: Vec<String>,
    pub used_days: i64,
}

impl GalileoS1Chip {
    /// `[T=1][H][W][2]` (VV, VH) for the sidecar's `s1_chip` field.
    pub fn as_4d(&self) -> Vec<Vec<Vec<Vec<f32>>>> {
        let h = GALILEO_CHIP_PIXELS as usize;
        let w = GALILEO_CHIP_PIXELS as usize;
        let bands = 2usize; // VV, VH
        let mut plane: Vec<Vec<Vec<f32>>> = Vec::with_capacity(h);
        for r in 0..h {
            let mut row: Vec<Vec<f32>> = Vec::with_capacity(w);
            for c in 0..w {
                let mut px: Vec<f32> = Vec::with_capacity(bands);
                for b in 0..bands {
                    px.push(self.chip[b * h * w + r * w + c]);
                }
                row.push(px);
            }
            plane.push(row);
        }
        vec![plane]
    }
}

/// 8×8 DEM chip at 30 m: elevation (m) + slope (degrees) finite-difference
/// gradient magnitude. Each is a row-major H*W plane, band-major.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GalileoDemChip {
    /// `[2 * H * W]` band-major: elevation plane (m) then slope plane (deg).
    pub chip: Vec<f32>,
    pub asset_url: String,
}

impl GalileoDemChip {
    /// `[H][W][2]` (elevation, slope) for the sidecar's `srtm_chip` field
    /// — the space tensor has no time dimension.
    pub fn as_3d(&self) -> Vec<Vec<Vec<f32>>> {
        let h = GALILEO_CHIP_PIXELS as usize;
        let w = GALILEO_CHIP_PIXELS as usize;
        let bands = 2usize; // elevation, slope
        let mut plane: Vec<Vec<Vec<f32>>> = Vec::with_capacity(h);
        for r in 0..h {
            let mut row: Vec<Vec<f32>> = Vec::with_capacity(w);
            for c in 0..w {
                let mut px: Vec<f32> = Vec::with_capacity(bands);
                for b in 0..bands {
                    px.push(self.chip[b * h * w + r * w + c]);
                }
                row.push(px);
            }
            plane.push(row);
        }
        plane
    }
}

// ── Galileo multimodal: TerraClimate TIME group (def, soil, aet) ──────────
//
// Galileo's TIME tensor (single-pixel per timestep, no spatial dimension —
// `t_x` is `[T, C]`) carries `TIME_BANDS = ERA5(2) + TC(3) + VIIRS(1)`. We
// wire the TerraClimate group: `def` (climate water deficit), `soil` (soil
// moisture), `aet` (actual evapotranspiration). emem doesn't yet have ERA5
// or VIIRS, so those stay masked-absent — the TC group flips to seen only.
//
// Galileo ingests the **raw Google Earth Engine band value** (the GEE DN =
// physical_mm / 0.1, since both GEE and the THREDDS NetCDF pack these as
// int16 with scale_factor 0.1). emem's terraclimate connector fetches via
// THREDDS NCSS; `fetch_terraclimate_monthly_dn` returns the climatological
// monthly-mean DN, so we feed the value in the exact scale Galileo's
// normalizer was fit on. When TerraClimate can't be fetched the modality
// stays absent and the caller signs the S2(+S1+DEM)-only embedding — never
// zero-fill-and-claim.

/// TerraClimate TIME-group chip: the three Galileo TC bands `[def, soil,
/// aet]` as raw GEE DN, for a single timestep (T=1). No spatial dimension —
/// Galileo's TIME tensor is per-timestep scalar-per-band.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GalileoTcChip {
    /// `[def, soil, aet]` in raw GEE DN (= physical mm / 0.1), in
    /// Galileo's `TC_BANDS` order.
    pub def_soil_aet: [f32; 3],
    /// Upstream NCSS URLs hit (def, soil, aet) — pinned in the receipt so a
    /// verifier can replay each query.
    pub asset_urls: Vec<String>,
    /// Calendar month (1..=12) the climatological value was taken for —
    /// co-registered with the S2 scene month.
    pub month: u8,
}

impl GalileoTcChip {
    /// `[T=1][C=3]` (def, soil, aet) for the sidecar's `tc_chip` field —
    /// the TIME tensor has no spatial dimension.
    pub fn as_time_2d(&self) -> Vec<Vec<f32>> {
        vec![self.def_soil_aet.to_vec()]
    }
}

/// Fetch the Galileo TerraClimate TIME-group chip (`def`, `soil`, `aet` as
/// raw GEE DN) for `cell64` at calendar `month`. Uses the climatological
/// monthly mean over the WMO 1991–2020 normal window (TerraClimate's NCSS
/// service serves the monthly series; the per-month climatology is the
/// deterministic, verifier-replayable value that co-registers with the S2
/// scene's month). The three variables are fetched concurrently. If ANY of
/// the three can't be fetched the whole TC modality is treated as absent
/// (Galileo's TC group is a single mask entry — partial TC would mislead
/// the encoder), so we return an error and the caller masks TIME absent.
pub async fn fetch_galileo_tc_chip(
    cell64: &str,
    month: u8,
    timeout: std::time::Duration,
) -> Result<GalileoTcChip, String> {
    use emem_fetch::terraclimate as tc;

    let info = emem_codec::latlng_from_cell64(cell64).map_err(|e| format!("cell decode: {e}"))?;
    let lat = info.lat_deg;
    let lng = info.lng_deg;
    let win = tc::NORMAL_WINDOW;

    let (def_res, soil_res, aet_res) = tokio::join!(
        tc::fetch_terraclimate_monthly_dn(&tc::DEF, lat, lng, month, win, timeout),
        tc::fetch_terraclimate_monthly_dn(&tc::SOIL, lat, lng, month, win, timeout),
        tc::fetch_terraclimate_monthly_dn(&tc::AET, lat, lng, month, win, timeout),
    );
    let def = def_res.map_err(|e| format!("terraclimate def: {e}"))?;
    let soil = soil_res.map_err(|e| format!("terraclimate soil: {e}"))?;
    let aet = aet_res.map_err(|e| format!("terraclimate aet: {e}"))?;

    Ok(GalileoTcChip {
        def_soil_aet: [def.value as f32, soil.value as f32, aet.value as f32],
        asset_urls: vec![def.url, soil.url, aet.url],
        month,
    })
}

/// Fetch the 8×8 / 30 m Sentinel-1 RTC chip (VV + VH γ0 dB) for `cell64`
/// near `target_unix`. Reuses the same `s1_search_with_fallback` ladder
/// the scalar `sentinel1_raw` materializer uses (15→30→60 d). VH is
/// additive here — the scalar VV path is untouched. Linear γ0 power →
/// dB via `10·log10`; non-positive pixels (water-masked / nodata) become
/// NaN so the sidecar's per-modality normalization sees them as absent
/// rather than `-inf`.
pub async fn fetch_galileo_s1_chip(
    cell64: &str,
    target_unix: Option<i64>,
) -> Result<GalileoS1Chip, String> {
    let info = emem_codec::latlng_from_cell64(cell64).map_err(|e| format!("cell decode: {e}"))?;
    let lat = info.lat_deg;
    let lng = info.lng_deg;

    let now_unix = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let cli = s2_http_client();
    let (item, used_days) = s1_search_with_fallback(&cli, lng, lat, target_unix, now_unix).await?;
    let epsg = item
        .epsg
        .ok_or_else(|| "stac item missing proj:epsg".to_string())?;
    let utm = emem_fetch::proj::latlng_to_utm_with_epsg(lat, lng, epsg)
        .ok_or_else(|| format!("epsg {epsg} not a UTM code"))?;
    // RTC assets are anonymous Azure blobs requiring a SAS query string.
    let sas = emem_fetch::stac::mpc_sas_token(&cli, "sentinel-1-rtc")
        .await
        .map_err(|e| format!("mpc sas: {e}"))?;

    let h = GALILEO_CHIP_PIXELS as usize;
    let w = GALILEO_CHIP_PIXELS as usize;
    let mut chip_flat: Vec<f32> = Vec::with_capacity(2 * h * w);
    let mut asset_urls: Vec<String> = Vec::with_capacity(2);

    // VV then VH — Galileo's S1_BANDS order. MPC RTC ships both
    // polarisations as separate single-band COGs.
    for (label, aliases) in [("VV", ["vv", "VV"]), ("VH", ["vh", "VH"])] {
        let url_raw = aliases
            .iter()
            .find_map(|a| item.assets.get(*a).cloned())
            .ok_or_else(|| format!("stac item missing {label} asset for Galileo S1"))?;
        let sep = if url_raw.contains('?') { '&' } else { '?' };
        let url = format!("{url_raw}{sep}{sas}");
        let prof = emem_fetch::cog::open_profile(&cli, &url)
            .await
            .map_err(|e| format!("open S1 {label} COG: {e}"))?;
        let native_m = prof.pixel_scale.0.abs();
        if !(native_m > 0.0 && native_m.is_finite()) {
            return Err(format!(
                "S1 {label} COG implausible pixel_scale {:?}",
                prof.pixel_scale
            ));
        }
        let native_pixels = (GALILEO_PHYSICAL_EXTENT_M / native_m).round().max(1.0) as u32;
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
        .map_err(|e| format!("sample_window S1 {label}: {e}"))?;
        // Resample linear γ0 power to the 8×8 grid, THEN convert to dB.
        // (Mean-pooling in linear power, not dB, is the physically correct
        // order — averaging dB is a geometric mean of power.)
        let mut resampled_lin = if native_pixels == GALILEO_CHIP_PIXELS {
            raw.iter().map(|v| *v as f32).collect::<Vec<f32>>()
        } else if native_pixels.is_multiple_of(GALILEO_CHIP_PIXELS) {
            let k = (native_pixels / GALILEO_CHIP_PIXELS) as usize;
            block_mean_pool_pub(&raw, native_pixels as usize, native_pixels as usize, k)
        } else {
            resample_to_chip_pub(&raw, native_pixels, native_pixels, GALILEO_CHIP_PIXELS)
        };
        if resampled_lin.len() != h * w {
            return Err(format!(
                "Galileo S1 {label} resample produced {} samples, want {}",
                resampled_lin.len(),
                h * w
            ));
        }
        for v in &mut resampled_lin {
            // Non-positive linear power = water mask / nodata → NaN so the
            // sidecar normalizes it as absent rather than emitting -inf dB.
            *v = if v.is_finite() && *v > 0.0 {
                10.0 * v.log10()
            } else {
                f32::NAN
            };
        }
        chip_flat.append(&mut resampled_lin);
        asset_urls.push(url_raw);
    }

    Ok(GalileoS1Chip {
        chip: chip_flat,
        scene_id: item.id.clone(),
        scene_iso: item.datetime.clone(),
        scene_unix: parse_iso8601_unix(&item.datetime).unwrap_or(0),
        asset_urls,
        used_days,
    })
}

/// Fetch the 8×8 / 30 m Copernicus DEM GLO-30 chip (elevation in metres +
/// slope in degrees) for `cell64`. Slope is the finite-difference gradient
/// magnitude of the elevation tile: `slope = atan(|∇z|) · 180/π` where
/// `|∇z| = sqrt((dz/dx)² + (dz/dy)²)` over the NATIVE-resolution window
/// (so the gradient uses true 30 m pixel spacing), then both elevation and
/// slope are resampled to the 8×8 target. This matches Galileo's SRTM
/// group, whose slope is GEE `ee.Terrain.slope` (degrees) — see
/// galileo/src/data/earthengine/srtm.py.
pub async fn fetch_galileo_dem_chip(cell64: &str) -> Result<GalileoDemChip, String> {
    let info = emem_codec::latlng_from_cell64(cell64).map_err(|e| format!("cell decode: {e}"))?;
    let lat = info.lat_deg;
    let lng = info.lng_deg;

    let cli = s2_http_client();
    let url = emem_fetch::copernicus_dem::tile_url_for(lat, lng);
    let prof = emem_fetch::cog::open_profile(&cli, &url)
        .await
        .map_err(|e| format!("open DEM COG {url}: {e}"))?;
    // Cop-DEM GLO-30 is geographic (EPSG:4326) with degree pixel scale; the
    // sampler addresses it in the COG's own CRS via world_to_pixel, so we
    // sample at the cell's (lng, lat) directly. native_m here is the
    // along-track ground spacing used only for the slope denominator.
    let (sx_deg, sy_deg) = prof.pixel_scale;
    if !(sx_deg.abs() > 0.0 && sx_deg.is_finite() && sy_deg.abs() > 0.0 && sy_deg.is_finite()) {
        return Err(format!(
            "DEM COG implausible pixel_scale {:?}",
            prof.pixel_scale
        ));
    }
    // Ground spacing per pixel in metres: longitude scales with cos(lat).
    const M_PER_DEG_LAT: f64 = 111_320.0;
    let dx_m = sx_deg.abs() * M_PER_DEG_LAT * lat.to_radians().cos();
    let dy_m = sy_deg.abs() * M_PER_DEG_LAT;
    // Sample a window large enough to cover the 240 m extent at native res,
    // plus a 1-pixel halo on each side so the central-difference gradient is
    // defined for every output pixel.
    let native_pixels = (GALILEO_PHYSICAL_EXTENT_M / dx_m.max(dy_m))
        .round()
        .max(2.0) as u32;
    let win = native_pixels + 2;
    let raw = emem_fetch::cog::sample_window(&cli, &url, &prof, lng, lat, win, win)
        .await
        .map_err(|e| format!("sample_window DEM: {e}"))?;
    let win_u = win as usize;

    // Slope (degrees) per interior pixel via central differences; halo
    // pixels use one-sided differences (clamped indices). Then crop the
    // 1-pixel halo off both elevation and slope to native_pixels².
    let inner = native_pixels as usize;
    let mut elev_inner = Vec::with_capacity(inner * inner);
    let mut slope_inner = Vec::with_capacity(inner * inner);
    for r in 0..inner {
        for c in 0..inner {
            // (+1) offset accounts for the halo.
            let rr = r + 1;
            let cc = c + 1;
            let z = raw[rr * win_u + cc];
            let zl = raw[rr * win_u + (cc - 1)];
            let zr = raw[rr * win_u + (cc + 1)];
            let zu = raw[(rr - 1) * win_u + cc];
            let zd = raw[(rr + 1) * win_u + cc];
            let dzdx = (zr - zl) / (2.0 * dx_m);
            let dzdy = (zd - zu) / (2.0 * dy_m);
            let grad = (dzdx * dzdx + dzdy * dzdy).sqrt();
            let slope_deg = grad.atan().to_degrees();
            elev_inner.push(z as f32);
            slope_inner.push(slope_deg as f32);
        }
    }

    let h = GALILEO_CHIP_PIXELS as usize;
    let w = GALILEO_CHIP_PIXELS as usize;
    let resample = |src: &[f32]| -> Vec<f32> {
        let src64: Vec<f64> = src.iter().map(|v| *v as f64).collect();
        if native_pixels == GALILEO_CHIP_PIXELS {
            src.to_vec()
        } else if native_pixels.is_multiple_of(GALILEO_CHIP_PIXELS) {
            let k = (native_pixels / GALILEO_CHIP_PIXELS) as usize;
            block_mean_pool_pub(&src64, native_pixels as usize, native_pixels as usize, k)
        } else {
            resample_to_chip_pub(&src64, native_pixels, native_pixels, GALILEO_CHIP_PIXELS)
        }
    };
    let elev_8 = resample(&elev_inner);
    let slope_8 = resample(&slope_inner);
    if elev_8.len() != h * w || slope_8.len() != h * w {
        return Err(format!(
            "Galileo DEM resample produced {}+{} samples, want {}",
            elev_8.len(),
            slope_8.len(),
            h * w
        ));
    }
    let mut chip = Vec::with_capacity(2 * h * w);
    chip.extend_from_slice(&elev_8);
    chip.extend_from_slice(&slope_8);
    Ok(GalileoDemChip {
        chip,
        asset_url: url,
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

    /// S1 chip `as_4d` transposes the band-major [VV, VH] flat buffer into
    /// `[T=1][H][W][2]`. Pixel (0,0) carries [vv, vh].
    #[test]
    fn galileo_s1_chip_as_4d_layout() {
        let h = GALILEO_CHIP_PIXELS as usize;
        let w = GALILEO_CHIP_PIXELS as usize;
        let mut flat = Vec::with_capacity(2 * h * w);
        for b in 0..2 {
            for _ in 0..(h * w) {
                flat.push(b as f32);
            }
        }
        let chip = GalileoS1Chip {
            chip: flat,
            scene_id: "S1A_TEST".into(),
            scene_iso: "2024-06-01T00:00:00Z".into(),
            scene_unix: 0,
            asset_urls: vec![],
            used_days: 15,
        };
        let four_d = chip.as_4d();
        assert_eq!(four_d.len(), 1);
        assert_eq!(four_d[0].len(), h);
        assert_eq!(four_d[0][0].len(), w);
        assert_eq!(four_d[0][0][0], vec![0.0_f32, 1.0]); // [VV, VH]
    }

    /// TC chip `as_time_2d` produces the `[T=1][C=3]` (def, soil, aet) TIME
    /// tensor the sidecar's `tc_chip` field expects — no spatial dim.
    #[test]
    fn galileo_tc_chip_as_time_2d_layout() {
        let chip = GalileoTcChip {
            def_soil_aet: [657.0, 692.0, 562.0],
            asset_urls: vec![
                "https://example/def".into(),
                "https://example/soil".into(),
                "https://example/aet".into(),
            ],
            month: 7,
        };
        let t2 = chip.as_time_2d();
        assert_eq!(t2.len(), 1); // T=1
        assert_eq!(t2[0].len(), 3); // [def, soil, aet]
        assert_eq!(t2[0], vec![657.0_f32, 692.0, 562.0]);
    }

    /// DEM chip `as_3d` transposes the band-major [elev, slope] flat buffer
    /// into `[H][W][2]` (the space tensor has no time dim).
    #[test]
    fn galileo_dem_chip_as_3d_layout() {
        let h = GALILEO_CHIP_PIXELS as usize;
        let w = GALILEO_CHIP_PIXELS as usize;
        let mut flat = Vec::with_capacity(2 * h * w);
        for b in 0..2 {
            for _ in 0..(h * w) {
                flat.push(b as f32 * 10.0);
            }
        }
        let chip = GalileoDemChip {
            chip: flat,
            asset_url: "https://example/dem.tif".into(),
        };
        let three_d = chip.as_3d();
        assert_eq!(three_d.len(), h);
        assert_eq!(three_d[0].len(), w);
        assert_eq!(three_d[0][0], vec![0.0_f32, 10.0]); // [elevation, slope]
    }
}
