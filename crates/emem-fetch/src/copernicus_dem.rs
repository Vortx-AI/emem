//! Copernicus DEM GLO-30 URL helpers — **AWS Open Data Registry** mirror.
//!
//! Source: **ESA / Copernicus Cop-DEM GLO-30** (30 m global digital surface
//! model derived from TanDEM-X). Distributed by ESA under CC BY 4.0 and
//! re-published by AWS Open Data at
//! `s3://copernicus-dem-30m` (`copernicus-dem-30m.s3.amazonaws.com`,
//! `us-east-1`, anonymous read).
//!
//! Coverage: global land surface; 1°×1° GeoTIFF tiles named by the
//! **integer-degree corner**. Tiles use the same naming convention as the
//! ESA archive (`Copernicus_DSM_COG_10_{lat_band}_00_{lon_band}_00_DEM`)
//! where `lat_band` is `N{abs(floor(lat))}` for northern hemisphere or
//! `S{abs(floor(lat))}` for southern, and `lon_band` is
//! `E{abs(floor(lng))}` / `W{abs(floor(lng))}`. The tile spans 1° in each
//! direction from its named corner (so tile `N27_00_E086` covers Everest
//! at 27.99°N 86.92°E).
//!
//! Coverage gaps relative to ESA's full archive:
//!
//! - The AWS mirror tracks ESA's published tile set: no tile is published
//!   over the open ocean (the per-tile materializer must catch the 404
//!   and sign Absence rather than retry forever).
//! - Polar interiors below ~|lat| 85° are present but values are
//!   interpolated from sparser TanDEM-X coverage; we still sign Primary
//!   for them (the upstream returns a number, and the confidence is set
//!   on the materializer side).
//!
//! This module is intentionally tiny: pure URL builder + corner-tag
//! helpers. The materializer (`materialize_elevation_mean` in
//! `emem-api-rest`) AND the polygon-bbox prewarm path
//! (`prewarm_polygon_static_cog_bands`) both reach for the same tile URL
//! here, so the bytes the prewarm pulled into `TILE_CACHE` are the bytes
//! the per-cell `cog::sample_pixel` later reads.

/// Base URL for the Copernicus DEM GLO-30 tiles on the AWS Open Data
/// Registry mirror (anonymous S3, us-east-1).
const COPDEM_BASE_URL: &str = "https://copernicus-dem-30m.s3.amazonaws.com";

/// Public version tag. The AWS Open Data Registry mirror tracks the ESA
/// 2021 GLO-30 release; we pin the AWS provenance string here so a fact's
/// `Source.scheme` can declare which mirror signed the value.
pub const COPDEM_VERSION_TAG: &str = "aws.opendata.v1";

/// Build the corner-tag pair for `(lat, lng)`, e.g. `("N27", "E086")`.
///
/// Tiles are 1°×1° and named by the SOUTH-WEST integer-degree corner:
///   - lat 27.99 (north) → floor = 27 → `N27`
///   - lat -3.5 (south)  → floor = -4 → `S04`
///   - lng 86.92 (east)  → floor = 86 → `E086`
///   - lng -60.5 (west)  → floor = -61 → `W061`
///
/// This matches the Cop-DEM AWS bucket layout exactly (lifted from the
/// realdemo's `copdem_url_for_latlon`, which is the in-tree source of
/// truth used by the live demo binary against the real bucket).
pub fn tile_corner_tags(lat: f64, lng: f64) -> (String, String) {
    let lat_floor = lat.floor() as i32;
    let lng_floor = lng.floor() as i32;
    let lat_band = if lat_floor >= 0 {
        format!("N{:02}", lat_floor)
    } else {
        format!("S{:02}", lat_floor.abs())
    };
    let lng_band = if lng_floor >= 0 {
        format!("E{:03}", lng_floor)
    } else {
        format!("W{:03}", lng_floor.abs())
    };
    (lat_band, lng_band)
}

/// Full upstream URL for the Cop-DEM GLO-30 tile covering `(lat, lng)`.
/// Pure — no I/O. Caller is responsible for catching the 404 over open
/// ocean (the dataset publishes no tile there) and signing Absence.
pub fn tile_url_for(lat: f64, lng: f64) -> String {
    let (lat_band, lng_band) = tile_corner_tags(lat, lng);
    format!(
        "{COPDEM_BASE_URL}/Copernicus_DSM_COG_10_{lat_band}_00_{lng_band}_00_DEM/Copernicus_DSM_COG_10_{lat_band}_00_{lng_band}_00_DEM.tif"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_url_everest_northern_hemisphere() {
        // Everest summit: 27.99°N, 86.92°E lives in tile N27_E086.
        let (lat_band, lng_band) = tile_corner_tags(27.99, 86.92);
        assert_eq!(lat_band, "N27");
        assert_eq!(lng_band, "E086");
        let url = tile_url_for(27.99, 86.92);
        assert_eq!(
            url,
            "https://copernicus-dem-30m.s3.amazonaws.com/Copernicus_DSM_COG_10_N27_00_E086_00_DEM/Copernicus_DSM_COG_10_N27_00_E086_00_DEM.tif"
        );
    }

    #[test]
    fn tile_url_amazon_southern_hemisphere() {
        // Amazon: -3.5°S, -60.5°W lives in tile S04_W061 (floor convention).
        let (lat_band, lng_band) = tile_corner_tags(-3.5, -60.5);
        assert_eq!(lat_band, "S04");
        assert_eq!(lng_band, "W061");
        let url = tile_url_for(-3.5, -60.5);
        assert_eq!(
            url,
            "https://copernicus-dem-30m.s3.amazonaws.com/Copernicus_DSM_COG_10_S04_00_W061_00_DEM/Copernicus_DSM_COG_10_S04_00_W061_00_DEM.tif"
        );
    }
}
