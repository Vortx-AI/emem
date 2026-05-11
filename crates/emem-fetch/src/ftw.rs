//! Fields of The World (FTW) — agricultural field-boundary supplement.
//!
//! ## What this is for
//!
//! emem's polygon-recall path resolves a place name to a single rough
//! boundary via OSM/Nominatim. For *farm* queries that boundary is the
//! cadastral envelope around an entire estate; the user actually wants
//! the per-field polygons inside. Fields of The World publishes a global
//! product of ~3.17 billion 10 m-resolution field polygons across 241
//! countries (2024–2025), distributed as a single PMTiles archive on
//! Source Cooperative S3 (anonymous, HTTP range-supported, CC-BY-4.0).
//! Reference: <https://fieldsofthe.world/>, <https://source.coop/ftw>.
//!
//! ## How the fetcher works
//!
//! 1. One process-wide `AsyncPmTilesReader` is built lazily against the
//!    `global.pmtiles` URL (CloudFront-fronted source.coop). The reader
//!    keeps an in-memory `MokaCache` for PMTiles directory entries —
//!    the FTW archive uses leaf directories, so the first hit in any
//!    region costs ~2 round-trips and subsequent hits in the same region
//!    are one round-trip.
//! 2. For a `Bbox` query we compute the covering Web-Mercator tile range
//!    at the requested zoom (auto-picked from the header's `max_zoom`
//!    when not specified, capped at 14 to keep per-query tile count
//!    bounded — at z=14 one tile covers ~2.4 km, so a 5 km bbox touches
//!    ≤9 tiles).
//! 3. Each tile blob is decompressed by the PMTiles reader (gzip) and
//!    decoded by `mvt-reader` to vector features. Tile-local coordinates
//!    (0..extent) are projected to WGS84 with the standard Web-Mercator
//!    math. The hash of the concatenated decompressed tile bytes is the
//!    provenance source CID.
//!
//! ## What ships in the response
//!
//! `FieldCollection` carries a Vec<FieldPolygon> ready to drop into a
//! GeoJSON FeatureCollection: each polygon has WGS84 ring coordinates,
//! the MVT property dict (FTW emits FIBOA-compatible attributes; the
//! actual key set is whatever the global product chose to encode), and
//! a computed `area_m2` via the shoelace formula on the projected ring
//! (planar approximation — off by <1 % at mid-latitudes for fields up
//! to a few km across, which is the whole working range).
//!
//! ## License
//!
//! The global product is CC-BY-4.0. The fetcher returns the attribution
//! string and license alongside every collection so callers can quote
//! both in agent-facing receipts. The benchmark CC-BY-NC countries
//! (Latvia, Portugal) are NOT part of the global product — they only
//! appear in the labeled training set, which this fetcher does not read.

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::OnceCell;

use emem_core::Bbox;

/// Canonical URL of the FTW global-product PMTiles archive. Hosted
/// behind Source Cooperative's CloudFront over the same S3 bucket
/// (`us-west-2.opendata.source.coop/ftw/...`); anonymous, range-read.
/// The single file is ~2.14 TB — we never download more than the
/// header + directory entries + the tiles we actually need.
pub const FTW_PMTILES_URL: &str =
    "https://data.source.coop/ftw/global-data/predictions/vectors/alpha/global.pmtiles";

/// License for the global product (per asset metadata at source.coop).
/// The website summary mentions CC-BY-SA-4.0; we go by the per-asset
/// metadata, which is what the bytes themselves carry.
pub const FTW_LICENSE: &str = "CC-BY-4.0";

/// Attribution string required by the CC-BY-4.0 license. Surfaces in
/// the response so an agent can quote it without an extra registry
/// round-trip.
pub const FTW_ATTRIBUTION: &str =
    "Fields of The World (Taylor Geospatial Institute, ASU Kerner Lab, Microsoft AI for Good, \
     Washington University, Clark University)";

/// Default zoom level when the caller doesn't request one. The FTW
/// global product is rendered up to high zooms; z=14 strikes the
/// balance between tile count (a 5 × 5 km bbox at z=14 is ~4 tiles)
/// and the field-scale detail farms actually need. Clamped to the
/// archive's `max_zoom` at runtime so we never request a tile the
/// archive doesn't carry.
const DEFAULT_ZOOM: u8 = 14;

/// Hard cap on tiles fetched per `fetch_field_polygons_bbox` call.
/// At 16 tiles × ~250 KB/tile (typical FTW MVT) the per-query upper
/// bound is ~4 MB on the wire and ~5 s cold — both honest defaults
/// for agent-initiated calls. Callers that need wider coverage should
/// split their bbox.
const MAX_TILES_PER_QUERY: usize = 16;

/// Errors specific to the FTW fetcher.
#[derive(Debug, thiserror::Error)]
pub enum FtwError {
    /// Network / HTTP failure (DNS, TLS, 5xx, body read error).
    #[error("transport: {0}")]
    Transport(String),
    /// PMTiles directory or tile decode failure (malformed archive,
    /// unsupported compression, or invalid tile coord).
    #[error("pmtiles: {0}")]
    PmTiles(String),
    /// MVT (Mapbox Vector Tile) wire-format decode failure.
    #[error("mvt decode: {0}")]
    MvtDecode(String),
    /// Bbox is outside Web-Mercator's valid range (latitudes > 85.0511
    /// or < -85.0511). FTW does not cover the poles regardless.
    #[error("bbox outside Web-Mercator range: {0}")]
    BboxOutOfRange(String),
    /// Bbox would touch more than `MAX_TILES_PER_QUERY` tiles at the
    /// chosen zoom. Caller should split the bbox or pass a lower zoom.
    #[error("bbox touches {tiles} tiles at z={zoom} (cap {cap}); split bbox or pass lower zoom")]
    TooManyTiles { tiles: usize, zoom: u8, cap: usize },
}

/// One agricultural field polygon ready to drop into GeoJSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldPolygon {
    /// GeoJSON-shaped geometry (always a Polygon with exterior ring +
    /// optional interior rings in WGS84). Stored as a raw JSON value so
    /// the downstream serializer can pass it through unchanged.
    pub geometry: serde_json::Value,
    /// FIBOA-style properties lifted from the MVT feature.  Values are
    /// preserved as their MVT-native type (string / number / bool) so a
    /// caller can branch on `crop_type` or `determination_datetime`
    /// without an extra parse.
    pub properties: serde_json::Map<String, serde_json::Value>,
    /// Planar area in square metres computed via the shoelace formula
    /// on the projected ring after equirectangular scaling at the
    /// polygon's mean latitude. Accurate to <1 % for fields up to a
    /// few km across; good enough for "total cropland area" displays.
    pub area_m2: f64,
}

/// Result of one `fetch_field_polygons_bbox` call.
#[derive(Debug, Clone, Serialize)]
pub struct FieldCollection {
    /// Number of returned features (post-filter, post-projection).
    pub count: usize,
    /// Sum of `area_m2` across all returned features.
    pub total_area_m2: f64,
    /// Zoom level that was actually fetched (after clamping to the
    /// archive's `max_zoom`).
    pub zoom_used: u8,
    /// (z, x, y) triples of tiles that were read. Empty when the bbox
    /// covers no tiles in the archive — distinct from "tiles read but
    /// no features" which yields a non-empty list with `count == 0`.
    pub tiles_read: Vec<(u8, u32, u32)>,
    /// blake3 of the concatenated decompressed MVT tile bytes (sorted
    /// by (z, x, y) for stability). Serves as the provenance source
    /// CID — a verifier can re-read the same tiles and reproduce this.
    pub source_cid: String,
    /// Provider URL that served the response (the canonical FTW
    /// PMTiles URL). Surfaces in the receipt for license attribution.
    pub provider_url: String,
    /// License string for redistribution. Always `"CC-BY-4.0"`.
    pub license: String,
    /// Attribution string required by the license.
    pub attribution: String,
    /// The features themselves.
    pub features: Vec<FieldPolygon>,
}

/// Lazily-initialized process-wide PMTiles reader. PMTiles' moka cache
/// is keyed by directory offset, so the same reader benefits every
/// subsequent query — first call to any region pays the directory
/// round-trips, later calls reuse them.
///
/// We hold `Arc<...>` so concurrent requests share one reader (the
/// pmtiles reader is `Sync`-safe via the underlying reqwest client).
static FTW_READER: OnceCell<
    Arc<pmtiles::AsyncPmTilesReader<pmtiles::HttpBackend, pmtiles::HashMapCache>>,
> = OnceCell::const_new();

/// Build (or return cached) the PMTiles reader against `FTW_PMTILES_URL`.
async fn reader(
) -> Result<Arc<pmtiles::AsyncPmTilesReader<pmtiles::HttpBackend, pmtiles::HashMapCache>>, FtwError>
{
    FTW_READER
        .get_or_try_init(|| async {
            // pmtiles 0.23 brings its own reqwest 0.13; we use *that*
            // re-export to ensure type-compatibility with the backend.
            let client = pmtiles::reqwest::Client::builder()
                .user_agent(concat!(
                    "emem.dev/",
                    env!("CARGO_PKG_VERSION"),
                    " (+https://emem.dev; avijeet@vortx.ai)"
                ))
                .timeout(Duration::from_secs(45))
                .pool_max_idle_per_host(8)
                .build()
                .map_err(|e| FtwError::Transport(e.to_string()))?;
            let r = pmtiles::AsyncPmTilesReader::<pmtiles::HttpBackend, pmtiles::HashMapCache>::new_with_cached_url(
                pmtiles::HashMapCache::default(),
                client,
                FTW_PMTILES_URL,
            )
            .await
            .map_err(|e| FtwError::PmTiles(e.to_string()))?;
            Ok::<_, FtwError>(Arc::new(r))
        })
        .await
        .cloned()
}

/// Fetch every FTW field polygon whose bounding-box intersects `bbox`.
///
/// `zoom = None` → auto-pick (`min(DEFAULT_ZOOM, archive.max_zoom)`).
/// Returns `FieldCollection` even when zero features fall in the bbox —
/// `count == 0` with non-empty `tiles_read` means "we looked, there is
/// no field here", distinct from "the archive has no coverage at the
/// requested zoom" which yields empty `tiles_read`.
pub async fn fetch_field_polygons_bbox(
    bbox: &Bbox,
    zoom: Option<u8>,
) -> Result<FieldCollection, FtwError> {
    let reader = reader().await?;
    let hdr = reader.get_header();
    let explicit_zoom = zoom.is_some();
    let requested_zoom = zoom.unwrap_or(DEFAULT_ZOOM);
    let mut zoom_used = requested_zoom.min(hdr.max_zoom).max(hdr.min_zoom);

    // Auto-shrink the zoom when the caller didn't pin one and the
    // resulting tile range exceeds the per-query cap. Each step down
    // is a 4× reduction in tile count, so the loop terminates quickly
    // even for country-scale bboxes. If the caller pinned a zoom
    // explicitly we honour it and surface the cap as a hard error
    // — the agent that asked for z=14 over a too-wide bbox knows
    // what to split.
    let (tx_min, tx_max, ty_min, ty_max) = loop {
        let r = bbox_to_tile_range(bbox, zoom_used)?;
        let tiles_w = (r.1 - r.0 + 1) as usize;
        let tiles_h = (r.3 - r.2 + 1) as usize;
        let total = tiles_w.saturating_mul(tiles_h);
        if total <= MAX_TILES_PER_QUERY {
            break r;
        }
        if explicit_zoom || zoom_used <= hdr.min_zoom {
            return Err(FtwError::TooManyTiles {
                tiles: total,
                zoom: zoom_used,
                cap: MAX_TILES_PER_QUERY,
            });
        }
        zoom_used -= 1;
    };

    let mut tiles_read: Vec<(u8, u32, u32)> = Vec::new();
    let mut features: Vec<FieldPolygon> = Vec::new();
    let mut hasher = blake3::Hasher::new();
    // Stable iteration order matters for source_cid determinism.
    for ty in ty_min..=ty_max {
        for tx in tx_min..=tx_max {
            let coord = pmtiles::TileCoord::new(zoom_used, tx, ty)
                .map_err(|e| FtwError::PmTiles(format!("invalid tile coord: {e}")))?;
            let tile_bytes = reader
                .get_tile_decompressed(coord)
                .await
                .map_err(|e| FtwError::PmTiles(e.to_string()))?;
            let Some(bytes) = tile_bytes else { continue };
            tiles_read.push((zoom_used, tx, ty));
            hasher.update(&bytes);
            let tile_features = decode_tile_to_polygons(zoom_used, tx, ty, bbox, &bytes)?;
            features.extend(tile_features);
        }
    }

    let total_area_m2 = features.iter().map(|f| f.area_m2).sum();
    let source_cid = data_encoding::BASE32_NOPAD
        .encode(&hasher.finalize().as_bytes()[..20])
        .to_ascii_lowercase();

    Ok(FieldCollection {
        count: features.len(),
        total_area_m2,
        zoom_used,
        tiles_read,
        source_cid,
        provider_url: FTW_PMTILES_URL.into(),
        license: FTW_LICENSE.into(),
        attribution: FTW_ATTRIBUTION.into(),
        features,
    })
}

/// Compute the inclusive `(tx_min, tx_max, ty_min, ty_max)` Web-Mercator
/// tile range that covers `bbox` at `zoom`. Rejects bboxes outside the
/// Mercator envelope (|lat| > 85.0511).
fn bbox_to_tile_range(bbox: &Bbox, zoom: u8) -> Result<(u32, u32, u32, u32), FtwError> {
    // Web Mercator's lat limit, derived from the inverse-Gudermannian
    // singularity. FTW global product itself doesn't cover the poles.
    const MERC_LAT_LIMIT: f64 = 85.05112877980659;
    if bbox.lat_min < -MERC_LAT_LIMIT || bbox.lat_max > MERC_LAT_LIMIT {
        return Err(FtwError::BboxOutOfRange(format!(
            "lat range [{:.4}, {:.4}] exceeds Web-Mercator ±{:.4}",
            bbox.lat_min, bbox.lat_max, MERC_LAT_LIMIT
        )));
    }
    let tx_min = lon_to_tile_x(bbox.lon_min, zoom);
    let tx_max = lon_to_tile_x(bbox.lon_max, zoom);
    // Tile y is inverted: lat_max (north) → small y, lat_min (south) → large y.
    let ty_min = lat_to_tile_y(bbox.lat_max, zoom);
    let ty_max = lat_to_tile_y(bbox.lat_min, zoom);
    Ok((tx_min, tx_max, ty_min, ty_max))
}

/// Web-Mercator longitude → tile x at zoom z.
fn lon_to_tile_x(lon: f64, z: u8) -> u32 {
    let n = 1u32 << z;
    let x = ((lon + 180.0) / 360.0) * (n as f64);
    (x.floor().max(0.0) as u32).min(n - 1)
}

/// Web-Mercator latitude → tile y at zoom z.
fn lat_to_tile_y(lat: f64, z: u8) -> u32 {
    let n = 1u32 << z;
    let lat_rad = lat.to_radians();
    let y = (1.0 - ((lat_rad.tan() + 1.0 / lat_rad.cos()).ln() / std::f64::consts::PI)) / 2.0
        * (n as f64);
    (y.floor().max(0.0) as u32).min(n - 1)
}

/// Project a tile-local pixel `(px, py)` at tile (z, tx, ty) with the
/// MVT-declared `extent` (canonically 4096) to WGS84 `(lat, lon)`.
fn tile_pixel_to_wgs84(z: u8, tx: u32, ty: u32, px: f64, py: f64, extent: u32) -> (f64, f64) {
    let n = (1u64 << z) as f64;
    let x_norm = ((tx as f64) + px / (extent as f64)) / n;
    let y_norm = ((ty as f64) + py / (extent as f64)) / n;
    let lon = x_norm * 360.0 - 180.0;
    // Inverse Web-Mercator: lat = atan(sinh(π·(1−2y))).
    let lat_rad = (std::f64::consts::PI * (1.0 - 2.0 * y_norm)).sinh().atan();
    (lat_rad.to_degrees(), lon)
}

/// Decode one MVT tile blob into `FieldPolygon`s, projected to WGS84
/// and bbox-clipped (we keep any polygon whose ring touches the bbox —
/// fields straddling tile edges are common at field-scale boundaries).
fn decode_tile_to_polygons(
    z: u8,
    tx: u32,
    ty: u32,
    bbox: &Bbox,
    tile_bytes: &[u8],
) -> Result<Vec<FieldPolygon>, FtwError> {
    let reader = mvt_reader::Reader::new(tile_bytes.to_vec())
        .map_err(|e| FtwError::MvtDecode(format!("{e:?}")))?;
    let layers = reader
        .get_layer_metadata()
        .map_err(|e| FtwError::MvtDecode(format!("{e:?}")))?;
    let mut out = Vec::new();
    for (idx, layer) in layers.iter().enumerate() {
        let extent = if layer.extent == 0 {
            4096
        } else {
            layer.extent
        };
        let features = reader
            .get_features(idx)
            .map_err(|e| FtwError::MvtDecode(format!("{e:?}")))?;
        for feat in features {
            let Some(polygons) = polygon_to_geojson(&feat.geometry, z, tx, ty, extent) else {
                continue; // non-polygon feature (point/line) — FTW shouldn't emit these but skip safely
            };
            for (geom_json, area_m2) in polygons {
                // Bbox intersection check on bounding rect of ring 0
                // (the exterior). Cheaper than full intersection and
                // sufficient since we only fetched bbox-covering tiles.
                if !geom_intersects_bbox(&geom_json, bbox) {
                    continue;
                }
                let properties = match &feat.properties {
                    Some(p) => mvt_props_to_json(p),
                    None => serde_json::Map::new(),
                };
                out.push(FieldPolygon {
                    geometry: geom_json,
                    properties,
                    area_m2,
                });
            }
        }
    }
    Ok(out)
}

/// Translate `mvt-reader`'s `geo_types::Geometry<f32>` (polygon family
/// only) into a GeoJSON Polygon value plus a precomputed planar area.
///
/// Returns one entry per polygon (a `MultiPolygon` expands to N).
fn polygon_to_geojson(
    geom: &geo_types::Geometry<f32>,
    z: u8,
    tx: u32,
    ty: u32,
    extent: u32,
) -> Option<Vec<(serde_json::Value, f64)>> {
    match geom {
        geo_types::Geometry::Polygon(p) => {
            Some(vec![project_polygon_with_area(p, z, tx, ty, extent)])
        }
        geo_types::Geometry::MultiPolygon(mp) => Some(
            mp.0.iter()
                .map(|p| project_polygon_with_area(p, z, tx, ty, extent))
                .collect(),
        ),
        _ => None,
    }
}

/// Project a single `geo_types::Polygon<f32>` to a GeoJSON Polygon
/// Feature `geometry` block and compute its planar area (m²).
fn project_polygon_with_area(
    p: &geo_types::Polygon<f32>,
    z: u8,
    tx: u32,
    ty: u32,
    extent: u32,
) -> (serde_json::Value, f64) {
    let project_ring = |ring: &geo_types::LineString<f32>| -> Vec<[f64; 2]> {
        ring.0
            .iter()
            .map(|c| {
                let (lat, lon) = tile_pixel_to_wgs84(z, tx, ty, c.x as f64, c.y as f64, extent);
                // GeoJSON convention: [lon, lat].
                [lon, lat]
            })
            .collect()
    };
    let mut rings: Vec<Vec<[f64; 2]>> = Vec::with_capacity(1 + p.interiors().len());
    let exterior = project_ring(p.exterior());
    let area_m2 = planar_area_m2(&exterior);
    // GeoJSON requires the ring to be closed (first == last); the MVT
    // wire form doesn't repeat the closing vertex.
    let close = |mut r: Vec<[f64; 2]>| {
        if r.len() >= 3 && r.first() != r.last() {
            r.push(*r.first().unwrap());
        }
        r
    };
    rings.push(close(exterior));
    for hole in p.interiors() {
        rings.push(close(project_ring(hole)));
    }
    let geometry = serde_json::json!({
        "type": "Polygon",
        "coordinates": rings,
    });
    (geometry, area_m2)
}

/// Shoelace area on a ring whose vertices are `[lon, lat]` in degrees,
/// converted to metres via equirectangular scaling at the ring's mean
/// latitude. Accurate to <1 % for fields up to a few km across, which
/// is the entire working scale for FTW field polygons.
fn planar_area_m2(ring: &[[f64; 2]]) -> f64 {
    if ring.len() < 3 {
        return 0.0;
    }
    let mean_lat: f64 = ring.iter().map(|p| p[1]).sum::<f64>() / (ring.len() as f64);
    let lat_rad = mean_lat.to_radians();
    // metres per degree on the WGS84 ellipsoid at lat_rad (small-angle
    // approximation — Bowring 1985 § 2.4, error <0.5 % to 60° lat)
    const M_PER_DEG_LAT: f64 = 111_132.92;
    let m_per_deg_lon = 111_412.84 * lat_rad.cos() - 93.5 * (lat_rad * 3.0).cos();
    let mut acc = 0.0;
    for i in 0..ring.len() {
        let j = (i + 1) % ring.len();
        let (xi, yi) = (ring[i][0] * m_per_deg_lon, ring[i][1] * M_PER_DEG_LAT);
        let (xj, yj) = (ring[j][0] * m_per_deg_lon, ring[j][1] * M_PER_DEG_LAT);
        acc += xi * yj - xj * yi;
    }
    (acc / 2.0).abs()
}

/// Cheap "does any vertex of this polygon land inside `bbox`" check —
/// good enough for the post-tile-filter pass since we already only
/// fetched bbox-covering tiles. We deliberately keep edge-crossing
/// polygons because farms commonly straddle tile boundaries.
fn geom_intersects_bbox(geom: &serde_json::Value, bbox: &Bbox) -> bool {
    let Some(coords) = geom.get("coordinates") else {
        return false;
    };
    let Some(rings) = coords.as_array() else {
        return false;
    };
    let Some(outer) = rings.first().and_then(|r| r.as_array()) else {
        return false;
    };
    let (mut lon_lo, mut lon_hi) = (f64::INFINITY, f64::NEG_INFINITY);
    let (mut lat_lo, mut lat_hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for v in outer {
        let Some(pair) = v.as_array() else { continue };
        let Some(lon) = pair.first().and_then(|x| x.as_f64()) else {
            continue;
        };
        let Some(lat) = pair.get(1).and_then(|x| x.as_f64()) else {
            continue;
        };
        if lon < lon_lo {
            lon_lo = lon;
        }
        if lon > lon_hi {
            lon_hi = lon;
        }
        if lat < lat_lo {
            lat_lo = lat;
        }
        if lat > lat_hi {
            lat_hi = lat;
        }
    }
    // Axis-aligned bbox-on-bbox intersection.
    !(lon_hi < bbox.lon_min
        || lon_lo > bbox.lon_max
        || lat_hi < bbox.lat_min
        || lat_lo > bbox.lat_max)
}

/// Translate MVT property values (`mvt_reader::feature::Value`) into
/// `serde_json::Value`, preserving the native type so callers can
/// branch on numeric attributes without re-parsing strings.
fn mvt_props_to_json(
    props: &std::collections::HashMap<String, mvt_reader::feature::Value>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut out = serde_json::Map::with_capacity(props.len());
    for (k, v) in props {
        out.insert(k.clone(), mvt_value_to_json(v));
    }
    out
}

fn mvt_value_to_json(v: &mvt_reader::feature::Value) -> serde_json::Value {
    use mvt_reader::feature::Value as V;
    match v {
        V::String(s) => serde_json::Value::String(s.clone()),
        V::Float(f) => serde_json::Number::from_f64(*f as f64)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        V::Double(d) => serde_json::Number::from_f64(*d)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        V::Int(i) => serde_json::Value::Number((*i).into()),
        V::UInt(u) => serde_json::Value::Number((*u).into()),
        V::SInt(s) => serde_json::Value::Number((*s).into()),
        V::Bool(b) => serde_json::Value::Bool(*b),
        V::Null => serde_json::Value::Null,
    }
}

/// Render a `FieldCollection` as a GeoJSON FeatureCollection — the
/// shape REST + MCP callers want.  Properties carry the FIBOA-derived
/// attrs plus the responder-side `area_m2`.
pub fn to_geojson_feature_collection(c: &FieldCollection) -> serde_json::Value {
    let features: Vec<serde_json::Value> = c
        .features
        .iter()
        .map(|f| {
            let mut props = f.properties.clone();
            props.insert(
                "area_m2".into(),
                serde_json::Number::from_f64(f.area_m2)
                    .map(serde_json::Value::Number)
                    .unwrap_or(serde_json::Value::Null),
            );
            serde_json::json!({
                "type": "Feature",
                "geometry": f.geometry,
                "properties": props,
            })
        })
        .collect();
    serde_json::json!({
        "type": "FeatureCollection",
        "features": features,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Web-Mercator tile-math against the canonical OSM slippy-map
    /// formula (Wikipedia "Slippy map tilenames"). Reference values
    /// computed both forms (ln(tan+sec) and asinh(tan)) agree to f64
    /// precision; we lock to (4823, 6160) at z=14 for (40.7128 N,
    /// 74.0060 W) so a future refactor can't silently shift the grid.
    #[test]
    fn lon_lat_to_tile_manhattan_z14() {
        let z = 14;
        let tx = lon_to_tile_x(-74.0060, z);
        let ty = lat_to_tile_y(40.7128, z);
        assert_eq!(tx, 4823, "tx mismatch at z=14 for -74.0060°");
        assert_eq!(ty, 6160, "ty mismatch at z=14 for 40.7128°");
    }

    /// Equator + prime meridian: at any zoom z, (0,0) sits at the
    /// boundary between tiles (n/2 - 1) and (n/2) where n = 2^z.
    /// `lon_to_tile_x(0, z)` and `lat_to_tile_y(0, z)` should both
    /// return n/2 (the tile whose west / north edge is the origin).
    #[test]
    fn equator_meridian_tile_is_centre() {
        for z in 0..=14u8 {
            let n = 1u32 << z;
            assert_eq!(lon_to_tile_x(0.0, z), n / 2, "lon at z={z}");
            assert_eq!(lat_to_tile_y(0.0, z), n / 2, "lat at z={z}");
        }
    }

    /// Inverse-projection round-trip — projecting the centre pixel of
    /// a tile must land on the tile's own centre lat/lng.
    #[test]
    fn tile_centre_projects_back_to_origin() {
        let z = 10;
        let tx = 512;
        let ty = 384;
        let extent = 4096;
        let (lat, lon) =
            tile_pixel_to_wgs84(z, tx, ty, (extent / 2) as f64, (extent / 2) as f64, extent);
        // Recover tile from the projected centre — must land on (tx, ty).
        assert_eq!(lon_to_tile_x(lon, z), tx);
        assert_eq!(lat_to_tile_y(lat, z), ty);
    }

    /// Bbox covering ~1 km² near Manhattan at z=14 must hit at most
    /// the per-query tile cap (≤16). At z=12 (one tile ≈ 9.7 km) a
    /// 1 km² bbox should fit in a single tile.
    #[test]
    fn bbox_to_tile_range_manhattan_small() {
        let bbox = Bbox::new(40.708, 40.717, -74.012, -74.000).unwrap();
        let (txmin, txmax, tymin, tymax) = bbox_to_tile_range(&bbox, 14).unwrap();
        let tiles = (txmax - txmin + 1) * (tymax - tymin + 1);
        assert!(
            (tiles as usize) <= MAX_TILES_PER_QUERY,
            "expected ≤{MAX_TILES_PER_QUERY} tiles at z=14, got {tiles}"
        );
        let (a, b, c, d) = bbox_to_tile_range(&bbox, 12).unwrap();
        let tiles_z12 = (b - a + 1) * (d - c + 1);
        // A ~1 km² bbox at z=12 (one tile ≈ 9.7 km) lands in 1 to 4
        // tiles depending on which side of an internal tile boundary
        // it falls on — we just want to lock that we never blow past
        // the per-query cap on a small AOI.
        assert!(
            (tiles_z12 as usize) <= MAX_TILES_PER_QUERY,
            "z=12 over 1 km² bbox should fit under tile cap, got {tiles_z12}"
        );
    }

    /// Polar bbox is rejected — FTW global product doesn't cover the
    /// poles and Web-Mercator is undefined past ±85.05113°.
    #[test]
    fn rejects_polar_bbox() {
        let bbox = Bbox::new(86.0, 89.0, 0.0, 1.0).unwrap();
        assert!(matches!(
            bbox_to_tile_range(&bbox, 14),
            Err(FtwError::BboxOutOfRange(_))
        ));
    }

    /// Shoelace area of a known 1° × 1° equator square is ≈12_321 km².
    /// (Ellipsoid value at the equator is 12_363 km² — the planar
    /// approximation lands within 0.4 %, which is the documented bound.)
    #[test]
    fn planar_area_equator_unit_square() {
        let ring = vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let a = planar_area_m2(&ring);
        let expected = 12_321e6; // ≈12_321 km²
        let rel_err = (a - expected).abs() / expected;
        assert!(
            rel_err < 0.02,
            "planar area off by {:.2} % (got {a}, expected {expected})",
            rel_err * 100.0
        );
    }

    /// Bbox intersection check rejects clearly-outside polygons.
    #[test]
    fn intersects_bbox_basic() {
        let bbox = Bbox::new(40.0, 41.0, -75.0, -74.0).unwrap();
        let inside = serde_json::json!({
            "type": "Polygon",
            "coordinates": [[[-74.5, 40.5], [-74.4, 40.5], [-74.4, 40.6], [-74.5, 40.6], [-74.5, 40.5]]]
        });
        let outside = serde_json::json!({
            "type": "Polygon",
            "coordinates": [[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [0.0, 0.0]]]
        });
        assert!(geom_intersects_bbox(&inside, &bbox));
        assert!(!geom_intersects_bbox(&outside, &bbox));
    }

    /// Network-gated smoke: fetch over Fresno County / Central Valley,
    /// California — one of the world's densest field-mosaic regions.
    /// At z=14 a 4 km × 4 km bbox must return >0 fields if FTW is
    /// genuinely covering this region; the assertion locks that we
    /// didn't accidentally short-circuit to an empty FeatureCollection.
    /// Run explicitly: `cargo test -p emem-fetch -- --ignored ftw_live`.
    #[tokio::test]
    #[ignore]
    async fn ftw_live_central_valley_returns_fields() {
        let bbox = Bbox::new(36.70, 36.74, -119.84, -119.80).unwrap();
        let coll = fetch_field_polygons_bbox(&bbox, Some(14)).await.expect(
            "FTW live fetch failed — check network or that the global.pmtiles \
             URL is still served at source.coop",
        );
        eprintln!(
            "FTW Central Valley @ z={}: {} fields, {:.2} ha total, tiles_read={:?}, source_cid={}",
            coll.zoom_used,
            coll.count,
            coll.total_area_m2 / 10_000.0,
            coll.tiles_read,
            coll.source_cid
        );
        assert_eq!(coll.license, "CC-BY-4.0");
        assert_eq!(coll.provider_url, FTW_PMTILES_URL);
        assert!(
            coll.count > 0,
            "Central Valley should yield at least one field — got 0 (FTW coverage gap or upstream change?)"
        );
        // Sanity: every returned feature must have a Polygon geometry
        // with a closed exterior ring of at least 4 coordinates.
        for f in &coll.features {
            assert_eq!(
                f.geometry.get("type").and_then(|v| v.as_str()),
                Some("Polygon")
            );
            let outer = f
                .geometry
                .get("coordinates")
                .and_then(|c| c.as_array())
                .and_then(|rings| rings.first())
                .and_then(|r| r.as_array())
                .expect("Polygon must have an exterior ring");
            assert!(
                outer.len() >= 4,
                "exterior ring must have >=4 coords (closed), got {}",
                outer.len()
            );
            assert_eq!(
                outer.first(),
                outer.last(),
                "exterior ring must be closed (first == last)"
            );
            assert!(f.area_m2 >= 0.0, "area_m2 must be non-negative");
        }
    }
}
