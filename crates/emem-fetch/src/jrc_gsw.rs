//! JRC Global Surface Water v1.4 (2021 release) URL helpers.
//!
//! Source: **Pekel, J.-F., et al. (2016). *High-resolution mapping of
//! global surface water and its long-term changes.* Nature 540, 418–422
//! (doi:10.1038/nature20584)**, with the 2021 update from the European
//! Commission's Joint Research Centre published on
//! `storage.googleapis.com/global-surface-water/downloads2021/...`.
//!
//! Layout: **10°×10° tile grid named by the NW corner**, each tile a
//! single global Cloud-Optimized GeoTIFF over its 10° × 10° patch. The
//! `recurrence/` directory holds `recurrence_{lon_left}_{lat_top}v1_4_2021.tif`
//! where `lon_left = floor(lng/10)*10` (with `E`/`W` suffix) and
//! `lat_top = ceil(lat/10)*10` (with `N`/`S` suffix). Range-readable
//! (verified live: 206 Partial Content).
//!
//! Pixel semantics (`uint8`):
//!
//! - `0..=100` — inter-annual recurrence percentage (1984–2021).
//! - `255` — no-data marker (permanent non-water and unmapped polar
//!   interiors). Materializers MUST sign Absence rather than treat
//!   `255` as a value.
//!
//! This module is intentionally tiny: it exposes the pure URL builder
//! used by the materializer (`materialize_jrc_gsw_recurrence` in
//! `emem-api-rest`) AND by the polygon-bbox prewarm path
//! (`prewarm_polygon_static_cog_bands`). Keeping the URL pattern in
//! one place means the prewarm and the per-cell sampler always reach
//! for the same tile, so the bytes the prewarm pulled into
//! `TILE_CACHE` are the bytes the per-cell `sample_pixel` then reads.

/// Base URL for the JRC GSW v1.4 (2021 release) recurrence tile dir.
const JRC_GSW_RECURRENCE_BASE_URL: &str =
    "https://storage.googleapis.com/global-surface-water/downloads2021/recurrence";

/// Public version tag matching the bucket path.
pub const JRC_GSW_VERSION_TAG: &str = "v1_4.2021";

/// Build the NW-corner tile tag for `(lat, lng)`, e.g.
/// `("80E", "30N")`. Same logic the materializer uses inline: `lon_left
/// = floor(lng/10)*10` with `E`/`W` suffix, `lat_top = ceil(lat/10)*10`
/// with `N`/`S` suffix.
pub fn tile_corner_tags(lat: f64, lng: f64) -> (String, String) {
    let lon_edge = (lng / 10.0).floor() as i32 * 10;
    let lon_left = if lon_edge >= 0 {
        format!("{}E", lon_edge)
    } else {
        format!("{}W", lon_edge.abs())
    };
    let lat_edge = (lat / 10.0).ceil() as i32 * 10;
    let lat_top = if lat_edge >= 0 {
        format!("{}N", lat_edge)
    } else {
        format!("{}S", lat_edge.abs())
    };
    (lon_left, lat_top)
}

/// Full upstream URL for the JRC GSW v1.4 recurrence tile covering
/// `(lat, lng)`. Pure — no I/O. Caller is responsible for treating
/// `255` from the pixel sampler as a no-data marker and signing
/// Absence.
pub fn recurrence_tile_url_for(lat: f64, lng: f64) -> String {
    let (lon_left, lat_top) = tile_corner_tags(lat, lng);
    format!("{JRC_GSW_RECURRENCE_BASE_URL}/recurrence_{lon_left}_{lat_top}v1_4_2021.tif")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recurrence_url_iowa() {
        // ~42N, ~-95E (Iowa) -> lon_left=100W, lat_top=50N tile.
        let url = recurrence_tile_url_for(42.0, -95.0);
        assert!(url.contains("100W_50N"), "got {url}");
        assert!(url.ends_with("v1_4_2021.tif"));
    }

    #[test]
    fn recurrence_url_southern_hemisphere() {
        // Sao Paulo region ~-23.5S, -46.6E.
        let (lon_left, lat_top) = tile_corner_tags(-23.5, -46.6);
        assert_eq!(lon_left, "50W");
        assert_eq!(lat_top, "20S");
    }
}
