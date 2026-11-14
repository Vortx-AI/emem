//! MODIS land products on the Microsoft Planetary Computer, read at a point.
//!
//! The Planetary Computer mirrors the LP DAAC MODIS Collection 6.1 tiles as
//! Cloud-Optimized GeoTIFFs: one STAC item per product, tile and composite,
//! one COG per science dataset. A point read is one STAC search for the
//! tile, then a range read of the COG header and of the one internal tile
//! that holds the pixel.
//!
//! Nothing here is product specific. A caller names the collection and the
//! asset (`modis-64A1-061` / `Burn_Date`, `modis-11A2-061` /
//! `LST_Day_1km`, ...) and applies the product's own scale, fill values and
//! QA bits to the raw value it gets back.
//!
//! The grid is the MODIS sinusoidal projection on a sphere of radius
//! 6 371 007.181 m: `x = R·λ·cos φ`, `y = R·φ`, angles in radians. 36 by 18
//! tiles, each `2πR/36` metres on a side, numbered `h` from the west edge
//! (x = -πR) and `v` from the north edge (y = πR/2).

use std::collections::BTreeMap;
use std::f64::consts::{FRAC_PI_2, PI};

use reqwest::Client;
use serde_json::{json, Value};

/// Radius of the sphere the MODIS sinusoidal grid is defined on, metres.
pub const SPHERE_RADIUS_M: f64 = 6_371_007.181;

/// Side of one MODIS tile in sinusoidal metres: a 36th of the circumference.
pub const TILE_SIZE_M: f64 = 2.0 * PI * SPHERE_RADIUS_M / 36.0;

const X_WEST: f64 = -PI * SPHERE_RADIUS_M;
const Y_NORTH: f64 = FRAC_PI_2 * SPHERE_RADIUS_M;

/// Sinusoidal `(x, y)` in metres of a WGS-84 point. MODIS treats the
/// geodetic latitude as a latitude on its sphere, as GDAL and the MODIS
/// tools do, so no datum shift is applied.
pub fn sinusoidal_xy(lat_deg: f64, lng_deg: f64) -> (f64, f64) {
    let lat = lat_deg.to_radians();
    (
        SPHERE_RADIUS_M * lng_deg.to_radians() * lat.cos(),
        SPHERE_RADIUS_M * lat,
    )
}

/// The MODIS tile `(h, v)` holding a sinusoidal point, or `None` off the
/// grid. A point on a shared edge belongs to the tile east or south of it,
/// the same floor rule a PixelIsArea raster uses for its pixels. The east
/// and south edges of the grid itself belong to the last tile.
pub fn tile_of(x: f64, y: f64) -> Option<(u32, u32)> {
    // A millimetre of slack, so the grid's own edge is on the grid.
    const SLACK_M: f64 = 1e-3;
    if !x.is_finite()
        || !y.is_finite()
        || x.abs() > -X_WEST + SLACK_M
        || y.abs() > Y_NORTH + SLACK_M
    {
        return None;
    }
    let h = ((x - X_WEST) / TILE_SIZE_M).floor().max(0.0);
    let v = ((Y_NORTH - y) / TILE_SIZE_M).floor().max(0.0);
    Some(((h as u32).min(35), (v as u32).min(17)))
}

/// `account/container` of an Azure blob URL, the form the Planetary
/// Computer token endpoint takes (`/api/sas/v1/token/{account}/{container}`).
///
/// A collection's own token can sign a different container from the one
/// its COGs are in: the `modis-64A1-061` token signs `modiseuwest/modis-061`
/// (the HDF files) and Azure refuses it on `modis-061-cogs` with a 403.
/// Asking for the container the asset is in avoids that.
pub fn blob_container(href: &str) -> Option<String> {
    let rest = href.strip_prefix("https://")?;
    let (host, path) = rest.split_once('/')?;
    let account = host.strip_suffix(".blob.core.windows.net")?;
    let container = path.split('/').next().filter(|c| !c.is_empty())?;
    if account.is_empty() {
        return None;
    }
    Some(format!("{account}/{container}"))
}

/// One MODIS item: a tile and a composite period.
#[derive(Debug, Clone, PartialEq)]
pub struct ModisPcItem {
    /// STAC item id, e.g. `MCD64A1.A2023213.h30v10.061.2023284120835`. The
    /// last part is when LP DAAC produced the granule.
    pub id: String,
    /// First day of the composite, `YYYY-MM-DD`.
    pub start_date: String,
    /// Horizontal tile number.
    pub h: u32,
    /// Vertical tile number.
    pub v: u32,
    /// `assets[name].href`, unsigned.
    pub assets: BTreeMap<String, String>,
}

/// The items in one STAC search response that belong to tile `(h, v)`.
/// An item with no `start_datetime`, or one the catalogue labels with
/// another tile, is left out rather than guessed at.
pub fn parse_items(body: &Value, h: u32, v: u32) -> Vec<ModisPcItem> {
    let Some(feats) = body.get("features").and_then(|f| f.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for f in feats {
        let props = f.get("properties").unwrap_or(&Value::Null);
        let tile_h = props.get("modis:horizontal-tile").and_then(|x| x.as_u64());
        let tile_v = props.get("modis:vertical-tile").and_then(|x| x.as_u64());
        if tile_h != Some(h as u64) || tile_v != Some(v as u64) {
            continue;
        }
        let Some(start) = props
            .get("start_datetime")
            .and_then(|x| x.as_str())
            .and_then(|s| s.get(..10))
        else {
            continue;
        };
        let Some(id) = f.get("id").and_then(|x| x.as_str()) else {
            continue;
        };
        let mut assets = BTreeMap::new();
        if let Some(a) = f.get("assets").and_then(|a| a.as_object()) {
            for (k, av) in a {
                if let Some(href) = av.get("href").and_then(|x| x.as_str()) {
                    assets.insert(k.clone(), href.to_string());
                }
            }
        }
        out.push(ModisPcItem {
            id: id.to_string(),
            start_date: start.to_string(),
            h,
            v,
            assets,
        });
    }
    out
}

/// Items of `collection` for tile `(h, v)` whose period intersects
/// `datetime` (an RFC 3339 interval), by the tile numbers the catalogue
/// states rather than by footprint. An item's footprint is a four-point
/// polygon in degrees that only approximates the curved tile, so a point
/// search near a tile edge can return the neighbour or nothing.
///
/// One page of at most `limit` items; a caller asking for a short window
/// gets all of it.
pub async fn search_tile(
    client: &Client,
    collection: &str,
    h: u32,
    v: u32,
    datetime: &str,
    limit: usize,
) -> Result<Vec<ModisPcItem>, String> {
    let _stage = crate::latency::StageTimer::new(
        "stac.search_modis_tile",
        format!("{collection} h{h:02}v{v:02} {datetime}"),
    );
    let body = json!({
        "collections": [collection],
        "query": {
            "modis:horizontal-tile": {"eq": h},
            "modis:vertical-tile": {"eq": v},
        },
        "datetime": datetime,
        "limit": limit,
    });
    let resp = crate::stac::post_search(client, crate::stac::STAC_MPC_V1, &body).await?;
    if resp.get("features").and_then(|f| f.as_array()).is_none() {
        return Err("stac response has no features array".into());
    }
    Ok(parse_items(&resp, h, v))
}

/// One pixel read from a COG: its raw value and where it is in the image.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PixelRead {
    /// The stored number, before any scale or offset.
    pub raw: f64,
    /// Column of the pixel holding the point.
    pub col: i64,
    /// Row of the pixel holding the point.
    pub row: i64,
}

/// Read the pixel holding sinusoidal `(x, y)` from the COG at `href`
/// (unsigned). The pixel comes from the COG's own geotransform, floored to
/// the pixel that contains the point, as GDAL picks it. ORNL's subset
/// service picks the same pixel except for a point within about a
/// millimetre of a pixel edge: the h13v04 COG stores its origin 0.5 mm
/// south of the exact grid, so a point on that edge falls on the other side.
pub async fn read_point(client: &Client, href: &str, x: f64, y: f64) -> Result<PixelRead, String> {
    let container = blob_container(href).ok_or_else(|| format!("not an Azure blob url: {href}"))?;
    let token = crate::stac::mpc_sas_token(client, &container).await?;
    let sep = if href.contains('?') { '&' } else { '?' };
    let url = format!("{href}{sep}{token}");
    let name = href.rsplit('/').next().unwrap_or(href);
    let profile = crate::cog::open_profile(client, &url)
        .await
        .map_err(|e| format!("open {name}: {e}"))?;
    let (col, row) = profile.world_to_pixel(x, y);
    let raw = crate::cog::sample_pixel(client, &url, &profile, x, y)
        .await
        .map_err(|e| format!("read {name}: {e}"))?;
    Ok(PixelRead { raw, col, row })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cog::CogProfile;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn tile_side_is_a_36th_of_the_circumference() {
        // LP DAAC states the tile as 1 111 950.5197665 m.
        assert!(close(TILE_SIZE_M, 1_111_950.519_766_5, 1e-6));
    }

    /// Corners LP DAAC wrote into the h30v10 granule of August 2023: its
    /// north-west corner (WESTBOUNDINGCOORDINATE at NORTHBOUNDINGCOORDINATE)
    /// and its south-east G-ring point, against the tile's own edges from
    /// the COG tiepoint and the item's `proj:geometry`.
    #[test]
    fn sinusoidal_hits_the_corners_of_h30v10() {
        let (x, y) = sinusoidal_xy(-9.999_999_999_1, 121.851_193_415);
        assert!(close(x, 13_343_406.236_016, 0.05), "west edge x {x}");
        assert!(close(y, -1_111_950.519_672, 0.05), "north edge y {y}");
        let (x, y) = sinusoidal_xy(-19.999_999_998_2, 138.343_110_407_9);
        assert!(close(x, 14_455_356.755_683, 0.05), "east edge x {x}");
        assert!(close(y, -2_223_901.039_339, 0.05), "south edge y {y}");
        // The same edges are whole tiles from the grid origin.
        assert!(close(X_WEST + 30.0 * TILE_SIZE_M, 13_343_406.236_016, 0.01));
        assert!(close(
            Y_NORTH - 10.0 * TILE_SIZE_M,
            -1_111_950.519_672,
            0.01
        ));
    }

    #[test]
    fn tiles_of_known_points() {
        // ORNL's subset service names h30v10 for this point.
        let (x, y) = sinusoidal_xy(-12.5, 132.5);
        assert_eq!(tile_of(x, y), Some((30, 10)));
        // The origin sits on four tiles; it belongs to the one south-east.
        assert_eq!(tile_of(0.0, 0.0), Some((18, 9)));
        let (x, y) = sinusoidal_xy(0.001, -0.001);
        assert_eq!(tile_of(x, y), Some((17, 8)));
        // The grid's own east and south edges stay on the grid.
        let (x, y) = sinusoidal_xy(0.0, 180.0);
        assert_eq!(tile_of(x, y), Some((35, 9)));
        let (x, y) = sinusoidal_xy(-90.0, 0.0);
        assert_eq!(tile_of(x, y), Some((18, 17)));
        assert_eq!(tile_of(-X_WEST * 1.01, 0.0), None);
        assert_eq!(tile_of(f64::NAN, 0.0), None);
    }

    /// The h30v10 Burn_Date COG's own header: 2400 x 2400 int16 at
    /// 463.3127 m, 512-pixel tiles, PixelIsArea, tiepoint at the tile's
    /// north-west corner.
    fn h30v10_profile() -> CogProfile {
        CogProfile {
            width: 2400,
            height: 2400,
            bits_per_sample: 16,
            sample_format: 2,
            compression: 8,
            predictor: 1,
            tile_w: 512,
            tile_h: 512,
            tile_cols: 5,
            tile_rows: 5,
            samples_per_pixel: 1,
            planar_config: 1,
            tile_offsets: vec![0; 25],
            tile_byte_counts: vec![0; 25],
            pixel_scale: (463.312_716_527_916_5, 463.312_716_527_916_56),
            tiepoint: (0.0, 0.0, 13_343_406.236_016, -1_111_950.519_672),
            epsg: None,
            nodata: Some("-1".into()),
            pixel_is_point: false,
        }
    }

    /// ORNL's subset service answered this point with the pixel whose
    /// lower-left corner is (14384006.54, -1390401.50) in h30v10. The point
    /// is 0.04 m inside that pixel's top edge, so a reader that rounded, or
    /// that missed the edge by float error, would take the row above.
    #[test]
    fn pixel_choice_matches_ornl_on_an_edge() {
        let p = h30v10_profile();
        let (x, y) = sinusoidal_xy(-12.5, 132.5);
        let (col, row) = p.world_to_pixel(x, y);
        assert_eq!((col, row), (2246, 600));
        let (ulx, uly) = p.pixel_to_world(col, row);
        let lly = uly - p.pixel_scale.1;
        // ORNL prints the corner to the centimetre from its own grid origin.
        assert!(close(ulx, 14_384_006.54, 0.1), "xll {ulx}");
        assert!(close(lly, -1_390_401.50, 0.1), "yll {lly}");
    }

    /// The pixel holding a point, not the nearest pixel centre: a point in
    /// the right half of a pixel stays in it.
    #[test]
    fn pixel_choice_floors() {
        let p = h30v10_profile();
        let (ulx, uly) = p.pixel_to_world(100, 200);
        let s = p.pixel_scale.0;
        assert_eq!(p.world_to_pixel(ulx + 0.9 * s, uly - 0.9 * s), (100, 200));
        assert_eq!(p.world_to_pixel(ulx + 0.1 * s, uly - 0.1 * s), (100, 200));
        assert_eq!(p.world_to_pixel(ulx + 1.0 * s, uly), (101, 200));
    }

    #[test]
    fn blob_container_of_asset_urls() {
        assert_eq!(
            blob_container(
                "https://modiseuwest.blob.core.windows.net/modis-061-cogs/MCD64A1/30/10/2023213/x_Burn_Date.tif"
            )
            .as_deref(),
            Some("modiseuwest/modis-061-cogs")
        );
        assert_eq!(blob_container("https://example.org/a/b.tif"), None);
        assert_eq!(
            blob_container("http://modiseuwest.blob.core.windows.net/c/x.tif"),
            None
        );
        assert_eq!(
            blob_container("https://modiseuwest.blob.core.windows.net/"),
            None
        );
    }

    #[test]
    fn parse_items_keeps_only_the_asked_tile() {
        let body = json!({"features": [
            {"id": "MCD64A1.A2023213.h30v10.061.2023284120835",
             "properties": {"start_datetime": "2023-08-01T00:00:00.000000Z",
                            "modis:horizontal-tile": 30, "modis:vertical-tile": 10},
             "assets": {"Burn_Date": {"href": "https://a.blob.core.windows.net/c/bd.tif"},
                        "QA": {"href": "https://a.blob.core.windows.net/c/qa.tif"}}},
            {"id": "MCD64A1.A2023213.h31v10.061.2023284120835",
             "properties": {"start_datetime": "2023-08-01T00:00:00.000000Z",
                            "modis:horizontal-tile": 31, "modis:vertical-tile": 10},
             "assets": {}},
            {"id": "no-start",
             "properties": {"modis:horizontal-tile": 30, "modis:vertical-tile": 10},
             "assets": {}}
        ]});
        let items = parse_items(&body, 30, 10);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].start_date, "2023-08-01");
        assert_eq!(items[0].assets.len(), 2);
        assert!(parse_items(&json!({}), 30, 10).is_empty());
    }
}
