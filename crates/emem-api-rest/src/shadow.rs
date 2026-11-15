//! Heights from shadows and the sun from a shadow.
//!
//! `POST /v1/shadow_height` (`shadow_height@1`) reads a shadow line in one
//! Sentinel-2 scene and turns its length into the height of the object that
//! cast it, under the sun the scene itself records. `POST /v1/sun_from_shadow`
//! (`sun_from_shadow@1`) runs the other way: from an object of known height
//! and its measured shadow, the sun that cast it, and whether a claimed
//! instant fits. Both compare against `sun_position@1` and sign one
//! derivation each.

use std::time::Instant;

use axum::extract::State;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value as JsonValue};

use emem_fact::FactCid;

use crate::band_raster::{bad_request, upstream_error};
use crate::sun::{self, Signing};
use crate::{ApiError, AppState, EmemJson};

pub(crate) const HEIGHT_FN: &str = "shadow_height@1";
pub(crate) const FROM_SHADOW_FN: &str = "sun_from_shadow@1";
/// Sentinel-2's finest pixel. A shadow line given as two points, with no
/// raster, is taken as measured on it.
const S2_PIXEL_M: f64 = 10.0;
const DEFAULT_BEARING_TOLERANCE_DEG: f64 = 10.0;
const DEM_BAND: &str = "copdem30m.elevation_mean";
/// WGS-84.
const WGS84_A: f64 = 6_378_137.0;
const WGS84_E2: f64 = 6.694_379_990_141_3e-3;

fn r4(x: f64) -> f64 {
    (x * 1e4).round() / 1e4
}

fn r2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// The signed angle from `b` to `a` in degrees, in (-180, 180].
fn angle_off(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(360.0);
    if d > 180.0 {
        d - 360.0
    } else {
        d
    }
}

/// Ground distance in metres and the true bearing from `p` to `q`, each
/// `(lat, lng)`, on the WGS-84 ellipsoid's local radii at the midpoint. Over
/// the few kilometres a shadow spans this is within a millimetre of the
/// geodesic.
pub(crate) fn distance_and_bearing(p: (f64, f64), q: (f64, f64)) -> (f64, f64) {
    let phi = ((p.0 + q.0) / 2.0).to_radians();
    let w = 1.0 - WGS84_E2 * phi.sin().powi(2);
    let meridian = WGS84_A * (1.0 - WGS84_E2) / w.powf(1.5);
    let normal = WGS84_A / w.sqrt();
    let north = (q.0 - p.0).to_radians() * meridian;
    let east = angle_off(q.1, p.1).to_radians() * normal * phi.cos();
    (
        north.hypot(east),
        east.atan2(north).to_degrees().rem_euclid(360.0),
    )
}

/// A point of a shadow line: `[col, row]` on a raster, or `{lat, lng}`.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Point {
    Pixel(f64, f64),
    LatLng(f64, f64),
}

fn parse_point(v: &JsonValue, what: &str) -> Result<Point, String> {
    if let Some(a) = v.as_array() {
        if let [c, r] = a.as_slice() {
            if let (Some(c), Some(r)) = (c.as_f64(), r.as_f64()) {
                return Ok(Point::Pixel(c, r));
            }
        }
    }
    match (
        v.get("lat").and_then(|x| x.as_f64()),
        v.get("lng").and_then(|x| x.as_f64()),
    ) {
        (Some(lat), Some(lng))
            if (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lng) =>
        {
            Ok(Point::LatLng(lat, lng))
        }
        _ => Err(format!(
            "`line.{what}` is [col, row] on the raster or {{lat, lng}}"
        )),
    }
}

/// The georeferencing a raster's signed record pins: the first pixel's
/// centre, the steps and the CRS.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Grid {
    pub epsg: u32,
    pub x0: f64,
    pub y0: f64,
    pub dx: f64,
    pub dy: f64,
    pub width: u32,
    pub height: u32,
}

impl Grid {
    fn from_record(rec: &JsonValue) -> Option<Grid> {
        let g = rec.pointer("/artifact/grid")?;
        Some(Grid {
            epsg: g["epsg"].as_u64()? as u32,
            x0: g["x0"].as_f64()?,
            y0: g["y0"].as_f64()?,
            dx: g["dx"].as_f64()?,
            dy: g["dy"].as_f64()?,
            width: g["width"].as_u64()? as u32,
            height: g["height"].as_u64()? as u32,
        })
    }

    /// `(lat, lng)` of a pixel coordinate: integers are pixel centres, as
    /// the grid header names the first pixel's centre.
    fn latlng(&self, col: f64, row: f64) -> Option<(f64, f64)> {
        emem_fetch::proj::utm_to_latlng_with_epsg(
            self.x0 + col * self.dx,
            self.y0 + row * self.dy,
            self.epsg,
        )
    }

    fn holds(&self, col: f64, row: f64) -> bool {
        (-0.5..=self.width as f64 - 0.5).contains(&col)
            && (-0.5..=self.height as f64 - 0.5).contains(&row)
    }
}

/// The scene a shadow is read in: its time and the sun its metadata records.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Scene {
    pub id: String,
    pub datetime: String,
    pub catalogue: String,
    pub sun_azimuth: Option<f64>,
    pub sun_elevation: Option<f64>,
    /// Which property the sun came from.
    pub sun_source: &'static str,
    pub view_azimuth: Option<f64>,
    pub view_incidence: Option<f64>,
}

/// The sun of a STAC item's properties: `view:sun_*` (Earth Search), else
/// the granule's mean solar angles (`s2:mean_solar_*`, Planetary Computer).
fn sun_of_properties(props: &JsonValue) -> (Option<f64>, Option<f64>, &'static str) {
    let az = props["view:sun_azimuth"].as_f64();
    let el = props["view:sun_elevation"].as_f64();
    if az.is_some() && el.is_some() {
        return (az, el, "view:sun_azimuth, view:sun_elevation");
    }
    let az = props["s2:mean_solar_azimuth"].as_f64();
    let el = props["s2:mean_solar_zenith"].as_f64().map(|z| 90.0 - z);
    if az.is_some() && el.is_some() {
        return (az, el, "s2:mean_solar_azimuth, 90 - s2:mean_solar_zenith");
    }
    (None, None, "none: the item records no sun")
}

/// Fetch a Sentinel-2 L2A item by id from the public catalogue that names
/// items that way.
async fn fetch_item(id: &str) -> Result<Scene, ApiError> {
    if id.is_empty()
        || id.len() > 120
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'-')
    {
        return Err(bad_request(format!(
            "`scene` `{id}` is not a Sentinel-2 L2A item id"
        )));
    }
    let base = if id.contains("_MSIL2A_") {
        "https://planetarycomputer.microsoft.com/api/stac/v1/collections/sentinel-2-l2a/items"
    } else {
        "https://earth-search.aws.element84.com/v1/collections/sentinel-2-l2a/items"
    };
    let url = format!("{base}/{id}");
    let resp = crate::s2_http_client()
        .get(&url)
        .header("user-agent", emem_core::outbound::user_agent())
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| upstream_error(format!("stac item {id}: {e}")))?;
    if resp.status().as_u16() == 404 {
        return Err(bad_request(format!(
            "`scene` `{id}`: the catalogue ({base}) has no such item"
        )));
    }
    if !resp.status().is_success() {
        return Err(upstream_error(format!(
            "stac item {id}: status {}",
            resp.status()
        )));
    }
    let item: JsonValue = resp
        .json()
        .await
        .map_err(|e| upstream_error(format!("stac item {id}: {e}")))?;
    let props = &item["properties"];
    let (sun_azimuth, sun_elevation, sun_source) = sun_of_properties(props);
    Ok(Scene {
        id: id.to_string(),
        datetime: props["datetime"].as_str().unwrap_or("").to_string(),
        catalogue: url,
        sun_azimuth,
        sun_elevation,
        sun_source,
        view_azimuth: props["view:azimuth"].as_f64(),
        view_incidence: props["view:incidence_angle"].as_f64(),
    })
}

/// The whole second of an item's `datetime` (`2025-01-15T06:52:43.024Z`).
fn item_unix(datetime: &str) -> Result<i64, String> {
    let head = datetime
        .get(..19)
        .ok_or_else(|| format!("the item's datetime `{datetime}` is not an ISO 8601 instant"))?;
    sun::parse_instant(&format!("{head}Z"))
}

/// Height from a shadow of horizontal length `length_m` under a sun at
/// `elevation_deg`, plus the rise of the ground from base to tip.
pub(crate) fn height_of(length_m: f64, elevation_deg: f64, rise_m: f64) -> f64 {
    length_m * elevation_deg.to_radians().tan() + rise_m
}

#[derive(Debug, Deserialize)]
pub struct ShadowHeightReq {
    /// A Sentinel-2 L2A STAC item id. Give this or `raster`.
    #[serde(default)]
    pub scene: Option<String>,
    /// An `emem:raster:` token of one Sentinel-2 band, from `/v1/band_raster`.
    #[serde(default)]
    pub raster: Option<String>,
    /// `{from, to}`: `from` the object's base, `to` the tip of its shadow.
    /// Each is `[col, row]` on the raster or `{lat, lng}`.
    pub line: JsonValue,
    /// Add the rise of the ground from base to tip, from the DEM.
    #[serde(default)]
    pub slope_correction: Option<bool>,
    /// How far the line's bearing may turn from the anti-solar azimuth before
    /// the answer warns, degrees. Default 10.
    #[serde(default)]
    pub bearing_tolerance_deg: Option<f64>,
}

/// The DEM at a point, as a signed fact: (fact_cid, metres).
async fn dem_at(s: &AppState, lat: f64, lng: f64) -> Result<(String, f64), String> {
    let cell = emem_codec::geo::cell64_from_latlng(lat, lng);
    let req = emem_primitives::recall::RecallReq {
        cell: cell.clone(),
        bands: Some(vec![DEM_BAND.into()]),
        ..Default::default()
    };
    let (resp, _) = crate::recall_with_auto_materialize_capped(&req, s, None)
        .await
        .map_err(|e| format!("{DEM_BAND} at {cell}: {}", e.1.message))?;
    resp.facts
        .iter()
        .enumerate()
        .find_map(|(i, f)| match f {
            emem_fact::Fact::Primary(p) if p.band == DEM_BAND => Some((
                resp.receipt.fact_cids.get(i)?.as_str().to_string(),
                crate::persist::cbor_f64(&p.value)?,
            )),
            _ => None,
        })
        .ok_or_else(|| format!("no {DEM_BAND} reading at {cell}"))
}

pub(crate) async fn shadow_height(
    req: ShadowHeightReq,
    s: &AppState,
) -> Result<JsonValue, ApiError> {
    let started = Instant::now();
    let tolerance = req
        .bearing_tolerance_deg
        .unwrap_or(DEFAULT_BEARING_TOLERANCE_DEG);
    if !(1.0..=45.0).contains(&tolerance) {
        return Err(bad_request("`bearing_tolerance_deg` is 1 to 45".into()));
    }
    let from = parse_point(&req.line["from"], "from").map_err(bad_request)?;
    let to = parse_point(&req.line["to"], "to").map_err(bad_request)?;
    // The scene, its sun, and the grid when the line is in pixels.
    let (scene, grid, raster) = match (&req.raster, &req.scene) {
        (Some(token), None) => {
            let r = crate::band_raster::raster_resolve(
                crate::band_raster::RasterResolveReq {
                    token: token.clone(),
                    spot_check: None,
                },
                s,
            )
            .await?;
            let rec = &r["derivation"];
            if rec["fn_key"] != json!("band_raster@1") {
                return Err(bad_request(format!(
                    "`raster` is a {} record; a shadow is read on one Sentinel-2 scene (band_raster@1)",
                    rec["fn_key"]
                )));
            }
            let src = &rec["sources"][0];
            let grid = Grid::from_record(rec)
                .ok_or_else(|| bad_request("the raster's record pins no grid".into()))?;
            let mut scene = Scene {
                id: src["id"].as_str().unwrap_or("").to_string(),
                datetime: src["captured_at"].as_str().unwrap_or("").to_string(),
                catalogue: src["catalogue"].as_str().unwrap_or("").to_string(),
                sun_azimuth: src["sun_azimuth_deg"].as_f64(),
                sun_elevation: src["sun_elevation_deg"].as_f64(),
                sun_source:
                    "the raster's signed record (sources[0].sun_*_deg, from the item's view:sun_*)",
                view_azimuth: src["view_azimuth_deg"].as_f64(),
                view_incidence: src["view_incidence_deg"].as_f64(),
            };
            // A Planetary Computer item has no view:sun_*, so its raster record
            // pins no sun; the item itself states the granule's mean angles.
            if scene.sun_elevation.is_none() {
                if let Ok(item) = fetch_item(&scene.id).await {
                    scene.sun_azimuth = item.sun_azimuth;
                    scene.sun_elevation = item.sun_elevation;
                    scene.sun_source = item.sun_source;
                    scene.view_azimuth = scene.view_azimuth.or(item.view_azimuth);
                    scene.view_incidence = scene.view_incidence.or(item.view_incidence);
                }
            }
            let raster = json!({"token": token.trim(), "derivation_cid": r["derivation_cid"],
                "band": r["band"], "artifact_cid": r["artifact"]["artifact_cid"],
                "grid": rec["artifact"]["grid"]});
            (scene, Some(grid), Some(raster))
        }
        (None, Some(id)) => (fetch_item(id.trim()).await?, None, None),
        _ => return Err(bad_request(
            "give `scene` (a Sentinel-2 L2A item id) or `raster` (an emem:raster: token), not both"
                .into(),
        )),
    };
    let ends = |p: Point, what: &str| -> Result<(f64, f64), ApiError> {
        match (p, grid) {
            (Point::LatLng(lat, lng), _) => Ok((lat, lng)),
            (Point::Pixel(c, r), Some(g)) => {
                if !g.holds(c, r) {
                    return Err(bad_request(format!(
                        "`line.{what}` [{c}, {r}] is outside the raster ({} x {} px)",
                        g.width, g.height
                    )));
                }
                g.latlng(c, r)
                    .ok_or_else(|| bad_request(format!("epsg {} is not a UTM code", g.epsg)))
            }
            (Point::Pixel(..), None) => Err(bad_request(format!(
                "`line.{what}` is in pixels, which needs `raster`; with `scene` give {{lat, lng}}"
            ))),
        }
    };
    let base = ends(from, "from")?;
    let tip = ends(to, "to")?;
    let pixel_m = grid.map(|g| g.dx.abs()).unwrap_or(S2_PIXEL_M);
    let (length_m, bearing) = distance_and_bearing(base, tip);
    if length_m < pixel_m {
        return Err(bad_request(format!(
            "the line is {length_m:.1} m, shorter than one {pixel_m} m pixel: no height can be read from it"
        )));
    }
    let unix = item_unix(&scene.datetime).map_err(upstream_error)?;
    let eph = sun::sun_at(base.0, base.1, unix as f64);
    let (meta_az, meta_el) = (scene.sun_azimuth, scene.sun_elevation);
    let (used_az, used_el, used) = match (meta_az, meta_el) {
        (Some(a), Some(e)) => (a, e, "the scene's metadata"),
        _ => (
            eph.azimuth_deg,
            eph.elevation_deg,
            "sun_position@1, because the scene records no sun",
        ),
    };
    if used_el <= 1.0 {
        return Err(bad_request(format!(
            "the sun stood {used_el:.2} degrees high: a shadow under a sun that low is not read"
        )));
    }

    let mut parents: Vec<FactCid> = Vec::new();
    if let Some(r) = &raster {
        if let Some(c) = r["derivation_cid"].as_str() {
            parents.push(FactCid::new(c.to_string()));
        }
    }
    let mut cautions: Vec<String> = vec![
        "A thin top, such as a spire or a mast, casts a shadow narrower than a pixel. Its tip may not show, and the height then reads short.".into(),
        "The sun angles of a scene are its granule's mean; across a 110 km tile they change by up to about a degree. `sun` gives sun_position@1 at the base and the item's time beside them.".into(),
    ];
    let slope = if req.slope_correction == Some(true) {
        let (b, t) =
            futures_util::future::join(dem_at(s, base.0, base.1), dem_at(s, tip.0, tip.1)).await;
        let ((bc, bz), (tc, tz)) = (b.map_err(upstream_error)?, t.map_err(upstream_error)?);
        parents.push(FactCid::new(bc.clone()));
        parents.push(FactCid::new(tc.clone()));
        cautions.push("The Copernicus DEM is a surface model: it holds buildings and trees. At the foot of a building it reads the roof, so ask for the slope correction only where base and tip are on open ground.".into());
        Some(
            json!({"dem_band": DEM_BAND, "base_m": r2(bz), "tip_m": r2(tz), "rise_m": r2(tz - bz),
            "base_fact": bc, "tip_fact": tc,
            "rule": "height = horizontal length x tan(elevation) + (ground at the tip - ground at the base): a shadow that falls up a slope is shorter, and one that falls down it is longer"}),
        )
    } else {
        None
    };
    let rise = slope
        .as_ref()
        .and_then(|v| v["rise_m"].as_f64())
        .unwrap_or(0.0);
    let height = |el: f64, l: f64| r2(height_of(l, el, rise));
    let anti_solar = (used_az + 180.0).rem_euclid(360.0);
    let off = angle_off(bearing, anti_solar);
    let within = off.abs() <= tolerance;
    if !within {
        cautions.push(format!(
            "The line runs {:.1} degrees from the anti-solar azimuth ({:.1}), more than the {tolerance} degree tolerance. It may not be this object's shadow, or its base or tip is misplaced: from the base, a shadow points away from the sun.",
            off.abs(),
            r4(anti_solar)
        ));
    }
    if scene.id.contains("_MSIL2A_") {
        cautions.push("This is a Planetary Computer item. Its datetime is the start of the datatake, which can be several minutes before the tile was sensed, so sun_position@1 at that time can differ from the scene's own angles by more than the tile's spread.".into());
    }
    let h = height(used_el, length_m);
    if let Some(inc) = scene.view_incidence {
        cautions.push(format!(
            "The scene views {inc:.1} degrees off nadir, so in the image the top of a {h:.0} m object leans about {:.0} m from its base, away from the sensor. Measure the base on the ground, not under the top.",
            h * inc.to_radians().tan()
        ));
    }
    let record = json!({
        "schema": "emem.shadow_height.v1",
        "fn_key": HEIGHT_FN,
        "scene": {"id": scene.id, "datetime": scene.datetime, "catalogue": scene.catalogue,
                  "sun_source": scene.sun_source, "sun_azimuth_deg": meta_az, "sun_elevation_deg": meta_el,
                  "view_azimuth_deg": scene.view_azimuth, "view_incidence_deg": scene.view_incidence},
        "raster": raster,
        "line": {
            "input": req.line,
            "base": {"lat": base.0, "lng": base.1},
            "tip": {"lat": tip.0, "lng": tip.1},
            "length_m": r2(length_m),
            "bearing_deg": r4(bearing),
            "pixel_m": pixel_m,
            "pixels": if grid.is_some() { "[col, row] on the raster: integers are pixel centres, from the first pixel's centre at [0, 0]" } else { "two points; the line is taken as measured on 10 m Sentinel-2 pixels" },
        },
        "sun": {
            "used": used,
            "metadata": {"azimuth_deg": meta_az, "elevation_deg": meta_el},
            "sun_position@1": {"at": sun::iso_second(unix), "place": "the base", "azimuth_deg": r4(eph.azimuth_deg), "elevation_deg": r4(eph.elevation_deg), "elevation_geometric_deg": r4(eph.elevation_geometric_deg)},
            "difference": {
                "azimuth_deg": meta_az.map(|a| r4(angle_off(a, eph.azimuth_deg))),
                "elevation_deg": meta_el.map(|e| r4(e - eph.elevation_deg)),
                "means": "metadata minus sun_position@1 at the base and the item's datetime, read to the second",
            },
        },
        "bearing_check": {"anti_solar_azimuth_deg": r4(anti_solar), "line_bearing_deg": r4(bearing),
                          "off_by_deg": r4(off), "tolerance_deg": tolerance, "within": within},
        "slope_correction": slope,
        "height_m": h,
        "height_range_m": [height(used_el, length_m - pixel_m), height(used_el, length_m + pixel_m)],
        "height_by_sun_position_m": height(eph.elevation_deg, length_m),
        "height_by_sun_position_range_m": [height(eph.elevation_deg, length_m - pixel_m), height(eph.elevation_deg, length_m + pixel_m)],
        "formula": "height = length x tan(sun elevation) (+ rise of the ground from base to tip with slope_correction). The range is the length minus and plus one pixel, as the ends of a line on a raster are each known to about half a pixel",
        "cautions": cautions,
    });
    let cell = emem_codec::geo::cell64_from_latlng(base.0, base.1);
    let day = unix.div_euclid(86_400).max(0) as u64;
    let how = Signing {
        cell: &cell,
        band: "shadow.height",
        op: "shadow_height",
        fn_key: HEIGHT_FN,
        days: [day, day],
        parents: parents.clone(),
    };
    let cid = sun::sign_derivation(s, how, &record).await?;
    let mut cids = vec![FactCid::new(cid)];
    cids.extend(parents);
    let provenance = json!({"class": "deterministic_index", "computed_by_responder": true,
        "on_resolve": "not computed again when the token is resolved: it reads the scene's metadata and the DEM. The record pins both, and the line"});
    Ok(sun::finish(
        s,
        record,
        "emem.shadow_height",
        &cell,
        cids,
        started,
        provenance,
    ))
}

pub async fn post_shadow_height(
    State(s): State<AppState>,
    EmemJson(req): EmemJson<ShadowHeightReq>,
) -> Result<Json<JsonValue>, ApiError> {
    Ok(Json(shadow_height(req, &s).await?))
}

#[derive(Debug, Clone, Deserialize)]
pub struct ShadowMeasure {
    pub length_m: f64,
    /// From the object's base to the tip of its shadow, degrees clockwise
    /// from true north.
    pub bearing_deg: f64,
    /// Default one 10 m Sentinel-2 pixel.
    #[serde(default)]
    pub length_uncertainty_m: Option<f64>,
    /// Default the angle that `length_uncertainty_m` spans at the tip.
    #[serde(default)]
    pub bearing_uncertainty_deg: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct SunFromShadowReq {
    pub height_m: f64,
    #[serde(default)]
    pub height_uncertainty_m: Option<f64>,
    pub shadow: ShadowMeasure,
    /// `{lat, lng}` or `{cell}`.
    pub place: JsonValue,
    /// The claimed instant, ISO 8601 UTC.
    pub datetime: String,
}

/// The inputs of a sun-from-shadow question, checked.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FromShadow {
    pub place: sun::Place,
    pub height: f64,
    pub height_unc: f64,
    pub length: f64,
    pub length_unc: f64,
    pub bearing: f64,
    pub bearing_unc: f64,
    pub claimed: i64,
}

fn from_shadow_inputs(req: &SunFromShadowReq) -> Result<FromShadow, String> {
    let place = sun::parse_places(&JsonValue::Array(vec![req.place.clone()]))?.remove(0);
    let m = &req.shadow;
    let length_unc = m.length_uncertainty_m.unwrap_or(S2_PIXEL_M);
    let height_unc = req.height_uncertainty_m.unwrap_or(0.0);
    if req.height_m <= 0.0
        || !req.height_m.is_finite()
        || !(0.0..req.height_m).contains(&height_unc)
    {
        return Err(
            "`height_m` must be above 0 and `height_uncertainty_m` 0 or more and under it".into(),
        );
    }
    if m.length_m <= 0.0 || !m.length_m.is_finite() || !(0.0..m.length_m).contains(&length_unc) {
        return Err("`shadow.length_m` must be above 0 and `shadow.length_uncertainty_m` 0 or more and under it".into());
    }
    if !m.bearing_deg.is_finite() {
        return Err("`shadow.bearing_deg` must be a number of degrees".into());
    }
    let bearing_unc = m
        .bearing_uncertainty_deg
        .unwrap_or_else(|| (length_unc / m.length_m).atan().to_degrees());
    if !(0.0..=90.0).contains(&bearing_unc) {
        return Err("`shadow.bearing_uncertainty_deg` is 0 to 90".into());
    }
    Ok(FromShadow {
        place,
        height: req.height_m,
        height_unc,
        length: m.length_m,
        length_unc,
        bearing: m.bearing_deg.rem_euclid(360.0),
        bearing_unc,
        claimed: sun::parse_instant(&req.datetime)?,
    })
}

/// Everything `sun_from_shadow@1` answers from its inputs: the sun the
/// shadow implies, the sun at the claimed instant, and the windows of that
/// UTC date when the sun fits the shadow.
pub(crate) fn from_shadow(q: &FromShadow) -> JsonValue {
    let el = (q.height / q.length).atan().to_degrees();
    let el_lo = ((q.height - q.height_unc) / (q.length + q.length_unc))
        .atan()
        .to_degrees();
    let el_hi = ((q.height + q.height_unc) / (q.length - q.length_unc))
        .atan()
        .to_degrees();
    let az = (q.bearing + 180.0).rem_euclid(360.0);
    let (lat, lng) = (q.place.lat, q.place.lng);
    let fits = |t: i64| {
        let s = sun::sun_at(lat, lng, t as f64);
        s.elevation_deg >= el_lo
            && s.elevation_deg <= el_hi
            && angle_off(s.azimuth_deg, az).abs() <= q.bearing_unc
    };
    let at = sun::sun_at(lat, lng, q.claimed as f64);
    let consistent = fits(q.claimed);
    // Minute by minute over the UTC date, each edge then found to the second.
    let day0 = q.claimed.div_euclid(86_400) * 86_400;
    let edge = |mut inside: i64, mut outside: i64| {
        while (inside - outside).abs() > 1 {
            let mid = (inside + outside) / 2;
            if fits(mid) {
                inside = mid;
            } else {
                outside = mid;
            }
        }
        inside
    };
    let mut windows: Vec<JsonValue> = Vec::new();
    let mut start: Option<i64> = None;
    for k in 0..1440 {
        let t = day0 + k * 60;
        match (fits(t), start) {
            (true, None) => start = Some(if k == 0 { t } else { edge(t, t - 60) }),
            (false, Some(a)) => {
                windows.push(json!([
                    sun::iso_second(a),
                    sun::iso_second(edge(t - 60, t))
                ]));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(a) = start {
        let last = day0 + 86_399;
        let end = if fits(last) {
            last
        } else {
            edge(day0 + 1439 * 60, last)
        };
        windows.push(json!([sun::iso_second(a), sun::iso_second(end)]));
    }
    json!({
        "implied_sun": {
            "elevation_deg": r4(el),
            "elevation_range_deg": [r4(el_lo), r4(el_hi)],
            "azimuth_deg": r4(az),
            "azimuth_range_deg": [r4((az - q.bearing_unc).rem_euclid(360.0)), r4((az + q.bearing_unc).rem_euclid(360.0))],
        },
        "sun_at_claimed": {
            "at": sun::iso_second(q.claimed),
            "azimuth_deg": r4(at.azimuth_deg),
            "elevation_deg": r4(at.elevation_deg),
            "elevation_geometric_deg": r4(at.elevation_geometric_deg),
            "elevation_off_deg": r4(at.elevation_deg - el),
            "azimuth_off_deg": r4(angle_off(at.azimuth_deg, az)),
        },
        "consistent": consistent,
        "windows_utc": windows,
        "window_date": sun::iso_second(day0).get(..10).unwrap_or("").to_string(),
    })
}

fn from_shadow_record(q: &FromShadow) -> JsonValue {
    let mut place = json!({"lat": q.place.lat, "lng": q.place.lng});
    if let Some(c) = &q.place.cell {
        place["cell"] = json!(c);
    }
    json!({
        "schema": "emem.sun_from_shadow.v1",
        "fn_key": FROM_SHADOW_FN,
        "inputs": {
            "place": place,
            "height_m": q.height, "height_uncertainty_m": q.height_unc,
            "shadow": {"length_m": q.length, "length_uncertainty_m": q.length_unc,
                       "bearing_deg": q.bearing, "bearing_uncertainty_deg": q.bearing_unc},
            "datetime": sun::iso_second(q.claimed),
        },
        "answer": from_shadow(q),
        "rule": "the shadow implies a sun at elevation atan(height / length), from atan((height - dh) / (length + dl)) to atan((height + dh) / (length - dl)), and at azimuth bearing + 180 within the bearing uncertainty. The claimed instant is consistent when sun_position@1 there (apparent elevation, as a shadow falls under the refracted sun) is inside both ranges. windows_utc are the stretches of that UTC date when it is: sampled every minute, each edge found to the second, so a stretch shorter than a minute can be missed",
        "algorithm": sun::ALGORITHM,
        "rounding": "angles to 4 decimals of a degree, instants to the whole second",
    })
}

/// Compute a signed `sun_from_shadow@1` record again from its inputs.
pub(crate) fn recompute_from_shadow(record: &JsonValue) -> Result<(), String> {
    if record["fn_key"] != json!(FROM_SHADOW_FN) {
        return Err(format!("the record is not {FROM_SHADOW_FN}"));
    }
    let i = &record["inputs"];
    let req = SunFromShadowReq {
        height_m: i["height_m"].as_f64().unwrap_or(f64::NAN),
        height_uncertainty_m: i["height_uncertainty_m"].as_f64(),
        shadow: serde_json::from_value(i["shadow"].clone())
            .map_err(|e| format!("the record's shadow: {e}"))?,
        place: json!({"lat": i["place"]["lat"], "lng": i["place"]["lng"]}),
        datetime: i["datetime"].as_str().unwrap_or("").to_string(),
    };
    let q = from_shadow_inputs(&req)?;
    let again = from_shadow(&q);
    if sun::same_json(&again, &record["answer"]) {
        Ok(())
    } else {
        Err(format!(
            "the inputs give {again}, the record says {}",
            record["answer"]
        ))
    }
}

pub(crate) async fn sun_from_shadow(
    req: SunFromShadowReq,
    s: &AppState,
) -> Result<JsonValue, ApiError> {
    let started = Instant::now();
    let q = from_shadow_inputs(&req).map_err(bad_request)?;
    let record = from_shadow_record(&q);
    let cell = q
        .place
        .cell
        .clone()
        .unwrap_or_else(|| emem_codec::geo::cell64_from_latlng(q.place.lat, q.place.lng));
    let day = q.claimed.div_euclid(86_400).max(0) as u64;
    let how = Signing {
        cell: &cell,
        band: "sun.from_shadow",
        op: "sun_from_shadow",
        fn_key: FROM_SHADOW_FN,
        days: [day, day],
        parents: vec![],
    };
    let cid = sun::sign_derivation(s, how, &record).await?;
    let provenance = json!({"class": "deterministic_index", "computed_by_responder": true, "on_resolve": sun::ON_RESOLVE});
    Ok(sun::finish(
        s,
        record,
        "emem.sun_from_shadow",
        &cell,
        vec![FactCid::new(cid)],
        started,
        provenance,
    ))
}

pub async fn post_sun_from_shadow(
    State(s): State<AppState>,
    EmemJson(req): EmemJson<SunFromShadowReq>,
) -> Result<Json<JsonValue>, ApiError> {
    Ok(Json(sun_from_shadow(req, &s).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One minute of latitude is 1852 m by the old definition and about
    /// 1843 m at the equator on WGS-84; a degree of longitude at the equator
    /// is 111.32 km.
    #[test]
    fn distance_and_bearing_on_the_ellipsoid() {
        let (d, b) = distance_and_bearing((0.0, 0.0), (1.0 / 60.0, 0.0));
        assert!((d - 1842.9).abs() < 0.2, "{d}");
        assert!(b.abs() < 1e-9 || (b - 360.0).abs() < 1e-9, "{b}");
        let (d, b) = distance_and_bearing((0.0, 0.0), (0.0, 0.01));
        assert!((d - 1113.19).abs() < 0.05, "{d}");
        assert!((b - 90.0).abs() < 1e-9);
        let (_, b) = distance_and_bearing((25.0, 55.0), (24.999, 54.999));
        assert!((b - 222.0).abs() < 1.0, "{b}");
        // Across the antimeridian the short way round.
        let (d, _) = distance_and_bearing((0.0, 179.999), (0.0, -179.999));
        assert!((d - 222.6).abs() < 0.1, "{d}");
    }

    #[test]
    fn a_height_is_the_length_times_the_tangent_plus_the_rise() {
        assert!((height_of(100.0, 45.0, 0.0) - 100.0).abs() < 1e-9);
        assert!((height_of(100.0, 45.0, -3.0) - 97.0).abs() < 1e-9);
        // Burj Khalifa's 828 m under a 40 degree sun casts about 987 m.
        let l = 828.0 / 40f64.to_radians().tan();
        assert!((height_of(l, 40.0, 0.0) - 828.0).abs() < 1e-9);
        assert!((l - 986.8).abs() < 0.1, "{l}");
    }

    #[test]
    fn a_point_is_a_pixel_or_a_place() {
        assert_eq!(
            parse_point(&json!([12.5, 3]), "from"),
            Ok(Point::Pixel(12.5, 3.0))
        );
        assert_eq!(
            parse_point(&json!({"lat": 25.0, "lng": 55.0}), "to"),
            Ok(Point::LatLng(25.0, 55.0))
        );
        assert!(parse_point(&json!({"lat": 95.0, "lng": 55.0}), "to").is_err());
        assert!(parse_point(&json!([1]), "to").is_err());
    }

    /// Integers are pixel centres from the first pixel's centre, which the
    /// grid's x0 and y0 name, and dy runs south.
    #[test]
    fn a_pixel_coordinate_lands_on_its_centre() {
        let g = Grid {
            epsg: 32640,
            x0: 500_005.0,
            y0: 2_800_005.0,
            dx: 10.0,
            dy: -10.0,
            width: 100,
            height: 50,
        };
        let a = g.latlng(0.0, 0.0).unwrap();
        let b = g.latlng(10.0, 0.0).unwrap();
        let c = g.latlng(0.0, 10.0).unwrap();
        let (d, brg) = distance_and_bearing(a, b);
        assert!((d - 100.0).abs() < 0.1, "{d}");
        assert!((brg - 90.0).abs() < 0.1, "{brg}");
        let (d, brg) = distance_and_bearing(a, c);
        assert!((d - 100.0).abs() < 0.1, "{d}");
        assert!((brg - 180.0).abs() < 0.1, "{brg}");
        assert!(g.holds(-0.5, 49.5) && !g.holds(100.0, 0.0));
    }

    #[test]
    fn the_sun_is_read_from_either_catalogue() {
        let e84 = json!({"view:sun_azimuth": 150.5, "view:sun_elevation": 40.25});
        assert_eq!(sun_of_properties(&e84).1, Some(40.25));
        let mpc = json!({"s2:mean_solar_azimuth": 150.5, "s2:mean_solar_zenith": 49.75});
        let (az, el, src) = sun_of_properties(&mpc);
        assert_eq!((az, el), (Some(150.5), Some(40.25)));
        assert!(src.starts_with("s2:mean_solar"));
        assert_eq!(sun_of_properties(&json!({})).1, None);
        assert_eq!(
            item_unix("2025-01-15T06:52:43.024000Z"),
            sun::parse_instant("2025-01-15T06:52:43Z")
        );
    }

    fn delhi_question(datetime: &str) -> SunFromShadowReq {
        // A 10 m pole, a shadow 10 m long pointing north-west.
        SunFromShadowReq {
            height_m: 10.0,
            height_uncertainty_m: None,
            shadow: ShadowMeasure {
                length_m: 10.0,
                bearing_deg: 315.0,
                length_uncertainty_m: Some(0.2),
                bearing_uncertainty_deg: Some(2.0),
            },
            place: json!({"lat": 28.6139, "lng": 77.209}),
            datetime: datetime.into(),
        }
    }

    /// The shadow's own sun is found again: take the sun at an instant, cast
    /// the shadow it makes, and ask. The claimed instant fits, one window
    /// holds it, and an instant three hours later does not fit.
    #[test]
    fn the_sun_from_a_shadow_finds_the_instant_that_cast_it() {
        let t = sun::parse_instant("2026-03-21T05:30:00Z").unwrap();
        let s = sun::sun_at(28.6139, 77.209, t as f64);
        let mut req = delhi_question("2026-03-21T05:30:00Z");
        req.shadow.length_m = req.height_m / s.elevation_deg.to_radians().tan();
        req.shadow.bearing_deg = (s.azimuth_deg + 180.0).rem_euclid(360.0);
        let q = from_shadow_inputs(&req).unwrap();
        let a = from_shadow(&q);
        assert_eq!(a["consistent"], json!(true));
        assert!(
            a["sun_at_claimed"]["elevation_off_deg"]
                .as_f64()
                .unwrap()
                .abs()
                < 1e-3
        );
        let w = a["windows_utc"].as_array().unwrap();
        assert_eq!(w.len(), 1, "{a}");
        let (from, to) = (
            sun::parse_instant(w[0][0].as_str().unwrap()).unwrap(),
            sun::parse_instant(w[0][1].as_str().unwrap()).unwrap(),
        );
        assert!(from <= t && t <= to && to - from < 3600, "{a}");
        req.datetime = "2026-03-21T08:30:00Z".into();
        let later = from_shadow(&from_shadow_inputs(&req).unwrap());
        assert_eq!(later["consistent"], json!(false));
        assert_eq!(later["windows_utc"], a["windows_utc"]);
        // The signed record computes again; a changed answer does not.
        let rec = from_shadow_record(&q);
        let signed: JsonValue =
            serde_json::to_value(crate::change_attribution::json_to_cbor(&rec)).unwrap();
        assert_eq!(recompute_from_shadow(&signed), Ok(()));
        let mut forged = signed;
        forged["answer"]["consistent"] = json!(false);
        assert!(recompute_from_shadow(&forged).is_err());
    }

    #[test]
    fn a_sun_from_shadow_question_is_checked() {
        let mut q = delhi_question("2026-03-21T05:30:00Z");
        q.height_uncertainty_m = Some(20.0);
        assert!(from_shadow_inputs(&q).is_err());
        let mut q = delhi_question("2026-03-21T05:30:00Z");
        q.shadow.length_m = 0.0;
        assert!(from_shadow_inputs(&q).is_err());
        let mut q = delhi_question("2026-03-21T05:30:00");
        q.shadow.length_uncertainty_m = None;
        assert!(from_shadow_inputs(&q).is_err());
        // With no stated bearing uncertainty, the angle one length
        // uncertainty spans at the tip: 10 m at 100 m is 5.71 degrees.
        let mut q = delhi_question("2026-03-21T05:30:00Z");
        q.shadow = ShadowMeasure {
            length_m: 100.0,
            bearing_deg: -45.0,
            length_uncertainty_m: None,
            bearing_uncertainty_deg: None,
        };
        let f = from_shadow_inputs(&q).unwrap();
        assert!((f.bearing_unc - 5.7106).abs() < 1e-3);
        assert_eq!(f.bearing, 315.0);
    }
}
