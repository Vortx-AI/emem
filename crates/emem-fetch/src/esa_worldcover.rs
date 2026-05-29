//! ESA WorldCover 2021 v200 URL helpers.
//!
//! Source: **Zanaga, D., et al. (2022). *ESA WorldCover 10 m 2021 v200*.**
//! Published as anonymous-readable Cloud-Optimized GeoTIFF tiles on
//! `s3://esa-worldcover` (eu-central-1, CC BY 4.0). Coverage spans land
//! and coastal strips on a **3°×3° tile grid named by the SW corner**
//! (e.g. `N42W096`).
//!
//! Pixel semantics (`uint8`):
//!
//! - `10` Tree cover, `20` Shrubland, `30` Grassland, `40` Cropland,
//!   `50` Built-up, `60` Bare/sparse vegetation, `70` Snow/ice,
//!   `80` Permanent water bodies, `90` Herbaceous wetland,
//!   `95` Mangroves, `100` Moss & lichen.
//! - `0` is the no-data marker (mostly open-ocean tile borders that
//!   v200 still ships).
//!
//! This module is intentionally tiny: it exposes the pure URL builder
//! used by the materializer (`materialize_esa_worldcover_2021` in
//! `emem-api-rest`) AND by the polygon-bbox prewarm path
//! (`prewarm_polygon_static_cog_bands`). Keeping the URL pattern in one
//! place means the prewarm and the per-cell sampler always reach for
//! the same tile, so the bytes the prewarm pulled into `TILE_CACHE` are
//! the bytes the per-cell `sample_pixel` then reads.

/// Base URL for the WorldCover 2021 v200 map tiles (anonymous S3).
const ESA_WORLDCOVER_BASE_URL: &str =
    "https://esa-worldcover.s3.eu-central-1.amazonaws.com/v200/2021/map";

/// Public version tag (matches the bucket path and the v200 release
/// notes shipped with the dataset).
pub const ESA_WORLDCOVER_VERSION_TAG: &str = "v200.2021";

/// Build the SW-corner tile tag for `(lat, lng)`, e.g. `("N42", "W096")`.
///
/// Latitude is anchored at the SOUTH edge of a 3° band (so a cell at
/// `lat = 42.5` lives in the `N42`...`N45` tile, labelled `N42`); the
/// same logic flips sign for the southern hemisphere. Longitude is
/// anchored at the WEST edge, with E/W picked by sign.
///
/// This is the exact convention the WorldCover v200 product manual
/// publishes and that the per-cell materializer in `emem-api-rest`
/// already used inline. Centralising it here keeps the prewarm + the
/// materializer in lock-step.
pub fn tile_corner_tags(lat: f64, lng: f64) -> (String, String) {
    let lat_floor = (lat / 3.0).floor() as i32 * 3;
    let lng_floor = (lng / 3.0).floor() as i32 * 3;
    let lat_tag = if lat_floor >= 0 {
        format!("N{:02}", lat_floor)
    } else {
        format!("S{:02}", lat_floor.abs())
    };
    let lng_tag = if lng_floor >= 0 {
        format!("E{:03}", lng_floor)
    } else {
        format!("W{:03}", lng_floor.abs())
    };
    (lat_tag, lng_tag)
}

/// Full upstream URL for the WorldCover 2021 v200 tile covering the
/// given `(lat, lng)`. Pure — no I/O. Caller is responsible for
/// catching the 404 over open ocean (the dataset publishes no tile
/// there) and signing Absence if appropriate.
pub fn tile_url_for(lat: f64, lng: f64) -> String {
    let (lat_tag, lng_tag) = tile_corner_tags(lat, lng);
    format!("{ESA_WORLDCOVER_BASE_URL}/ESA_WorldCover_10m_2021_v200_{lat_tag}{lng_tag}_Map.tif")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_url_anchors_at_sw_corner() {
        // 42.5N, 95.5W -> N42W096 (matches the Bbox helper in emem_core).
        let url = tile_url_for(42.5, -95.5);
        assert!(url.contains("N42W096"), "got {url}");
        assert!(url.ends_with("_Map.tif"));
    }

    #[test]
    fn tile_url_southern_hemisphere() {
        // -3.2S, -60.1W -> S06W063 (3° grid with SW-corner anchor).
        let (lat_tag, lng_tag) = tile_corner_tags(-3.2, -60.1);
        assert_eq!(lat_tag, "S06");
        assert_eq!(lng_tag, "W063");
    }
}
