//! `s2_index_series@1`: a spectral index over an area, through time, as one
//! signed derivation.
//!
//! A trajectory is one 10 m cell. A field, a plot or a catchment is thousands
//! of them, and the honest summary of an area on a date is a distribution, not
//! a number: how much of it the sky let the satellite see, and what the clear
//! part looked like. This reads every pinned Sentinel-2 scene in a window over
//! the area, masks each pixel by its scene classification, computes the index
//! per pixel, and reports per scene the clear-sky fraction and the
//! distribution (median, p10..p90, mean, std) over the clear pixels whose
//! centres fall inside the area. Scenes the sky mostly hid are listed with the
//! reason, never dropped silently and never averaged in.
//!
//! On top of the rows it states three things a reader would otherwise compute
//! wrongly by eye, each with the caution that bounds it:
//! - a Theil-Sen trend with Kendall's S and a normal-approximation p, which
//!   does not remove the seasonal cycle and says so;
//! - a same-season anomaly for the latest scene, against earlier years' scenes
//!   within 30 days of its day-of-year, when the window holds any;
//! - a block map per scene (frames an animation can play) and a first-to-last
//!   change map as GeoJSON, with the day-of-year gap that confounds it.
//!
//! Everything is pinned in the signed record: the scene ids and assets, the
//! mask policy, the index formula, the quantile rule, the selection rule. A
//! stranger re-derives every number from the same scenes.

use std::time::Instant;

use axum::extract::State;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value as JsonValue};

use emem_fact::{Derivation, DerivativeFact, Fact, FactCid, FieldBinding};

use crate::band_raster::{
    aoi_cid, bad_request, parse_ymd, read_masked_scene, scl_asset, upstream_error, BBox,
    DEFAULT_SCL_REJECT, MAX_SIDE_PX, S2_BANDS,
};
use crate::change_attribution::json_to_cbor;
use crate::{ApiError, AppState, EmemJson};

const FN_KEY: &str = "s2_index_series@1";
/// Scenes read per series. Each is four COG window reads (two bands, and the
/// scene classification under each), so 24 is about a hundred range reads.
const MAX_SCENES: usize = 24;
const DEFAULT_SCENES: usize = 16;
/// A window longer than this is a climatology, which this is not.
const MAX_WINDOW_DAYS: i64 = 4 * 366;
/// Fewer clear pixels than this and a scene's quantiles describe noise.
const MIN_SUPPORT_PX: usize = 30;
/// Scene-level cloud ceiling for candidates. The per-pixel mask is what
/// decides; this only stops reading scenes that are almost all cloud.
const CANDIDATE_MAX_CLOUD: f64 = 80.0;
const SCENE_READ_CONCURRENCY: usize = 6;
/// Same-season baseline half-width, in days of year.
const SEASON_HALF_WIDTH_DAYS: i64 = 30;
/// A scene at least this clear is `quality: clear`; below it, `partial`.
const CLEAR_SCENE_FRACTION: f64 = 0.9;
/// Below this many kept scenes a trend is not reported.
const MIN_TREND_SCENES: usize = 6;

/// One index: its two Sentinel-2 bands and the formula `(a - b) / (a + b)`.
struct Index {
    key: &'static str,
    a: &'static str,
    b: &'static str,
    formula: &'static str,
    reference: &'static str,
}

const INDICES: [Index; 2] = [
    Index {
        key: "ndvi",
        a: "s2.B08",
        b: "s2.B04",
        formula: "(B08 - B04) / (B08 + B04)",
        reference: "Rouse et al. 1974; Tucker 1979",
    },
    Index {
        key: "ndwi",
        a: "s2.B03",
        b: "s2.B08",
        formula: "(B03 - B08) / (B03 + B08)",
        reference: "McFeeters 1996",
    },
];

#[derive(Debug, Deserialize)]
pub struct FieldSeriesReq {
    /// WGS-84 bbox. Give this or `geometry`.
    #[serde(default)]
    pub bbox: Option<BBox>,
    /// A GeoJSON Polygon (or a Feature wrapping one), WGS-84. Pixels count
    /// when their centre is inside the outer ring and outside every hole.
    #[serde(default)]
    pub geometry: Option<JsonValue>,
    /// `ndvi` (default) or `ndwi`.
    #[serde(default)]
    pub index: Option<String>,
    pub start_date: String,
    pub end_date: String,
    #[serde(default)]
    pub max_scenes: Option<usize>,
    /// Scenes whose clear-sky share of the area is below this are excluded,
    /// with the share stated. Default 0.4.
    #[serde(default)]
    pub min_clear_fraction: Option<f64>,
    /// Map blocks per side, 2..=8. Default 6.
    #[serde(default)]
    pub blocks: Option<u32>,
    /// SCL classes rejected per pixel; defaults to the composite's policy.
    #[serde(default)]
    pub mask_policy: Option<Vec<u8>>,
}

/// The area: a bbox, plus the polygon rings when one was given.
struct Aoi {
    bbox: BBox,
    /// Rings in WGS-84 `[lng, lat]`; the first is the outer ring.
    rings: Vec<Vec<[f64; 2]>>,
}

fn parse_aoi(req: &FieldSeriesReq) -> Result<Aoi, ApiError> {
    if let Some(g) = &req.geometry {
        let g = if g.get("type").and_then(|t| t.as_str()) == Some("Feature") {
            g.get("geometry").unwrap_or(&JsonValue::Null)
        } else {
            g
        };
        if g.get("type").and_then(|t| t.as_str()) != Some("Polygon") {
            return Err(bad_request(
                "geometry must be a GeoJSON Polygon (or a Feature wrapping one); send each part of a MultiPolygon as its own series".into(),
            ));
        }
        let rings: Vec<Vec<[f64; 2]>> = g
            .get("coordinates")
            .and_then(|c| c.as_array())
            .ok_or_else(|| bad_request("Polygon has no coordinates".into()))?
            .iter()
            .map(|ring| {
                ring.as_array()
                    .map(|pts| {
                        pts.iter()
                            .filter_map(|p| {
                                let p = p.as_array()?;
                                Some([p.first()?.as_f64()?, p.get(1)?.as_f64()?])
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            })
            .collect();
        let outer = rings
            .first()
            .filter(|r| r.len() >= 4)
            .ok_or_else(|| bad_request("Polygon outer ring needs at least 4 positions".into()))?;
        let (mut min_lng, mut min_lat, mut max_lng, mut max_lat) =
            (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for [lng, lat] in outer {
            min_lng = min_lng.min(*lng);
            max_lng = max_lng.max(*lng);
            min_lat = min_lat.min(*lat);
            max_lat = max_lat.max(*lat);
        }
        let bbox = BBox {
            min_lat,
            min_lng,
            max_lat,
            max_lng,
        };
        check_bbox(&bbox)?;
        return Ok(Aoi { bbox, rings });
    }
    let bbox = req
        .bbox
        .clone()
        .ok_or_else(|| bad_request("give the area as `bbox` or `geometry`".into()))?;
    check_bbox(&bbox)?;
    Ok(Aoi {
        bbox,
        rings: Vec::new(),
    })
}

fn check_bbox(b: &BBox) -> Result<(), ApiError> {
    if !(b.min_lat < b.max_lat && b.min_lng < b.max_lng) {
        return Err(bad_request(
            "bbox must satisfy min_lat < max_lat and min_lng < max_lng".into(),
        ));
    }
    if b.min_lat < -90.0 || b.max_lat > 90.0 || b.min_lng < -180.0 || b.max_lng > 180.0 {
        return Err(bad_request("bbox exceeds WGS-84 bounds".into()));
    }
    Ok(())
}

/// Even-odd ray cast. Points exactly on an edge fall either way, which at a
/// pixel centre on a vertex is a measure-zero question.
fn in_ring(x: f64, y: f64, ring: &[[f64; 2]]) -> bool {
    let mut inside = false;
    let n = ring.len();
    let mut j = n.wrapping_sub(1);
    for i in 0..n {
        let [xi, yi] = ring[i];
        let [xj, yj] = ring[j];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn in_polygon(x: f64, y: f64, rings: &[Vec<[f64; 2]>]) -> bool {
    match rings.split_first() {
        None => true,
        Some((outer, holes)) => in_ring(x, y, outer) && !holes.iter().any(|h| in_ring(x, y, h)),
    }
}

/// Nearest-rank quantile (Hyndman and Fan type 1) of an ascending slice: a
/// value some pixel actually had, never an interpolation between two.
fn quantile(sorted: &[f32], p: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let rank = (p * sorted.len() as f64).ceil().max(1.0) as usize;
    sorted[rank.min(sorted.len()) - 1] as f64
}

struct SceneStats {
    n_aoi: usize,
    n_clear: usize,
    mean: f64,
    std: f64,
    p10: f64,
    p25: f64,
    median: f64,
    p75: f64,
    p90: f64,
}

fn distribution(values: &mut [f32], n_aoi: usize) -> SceneStats {
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = values.len();
    let mean = if n > 0 {
        values.iter().map(|v| *v as f64).sum::<f64>() / n as f64
    } else {
        f64::NAN
    };
    let std = if n > 1 {
        (values
            .iter()
            .map(|v| (*v as f64 - mean).powi(2))
            .sum::<f64>()
            / (n - 1) as f64)
            .sqrt()
    } else {
        f64::NAN
    };
    SceneStats {
        n_aoi,
        n_clear: n,
        mean,
        std,
        p10: quantile(values, 0.10),
        p25: quantile(values, 0.25),
        median: quantile(values, 0.50),
        p75: quantile(values, 0.75),
        p90: quantile(values, 0.90),
    }
}

/// Theil-Sen slope (median of pairwise slopes) and Kendall's S with its
/// no-ties normal approximation. Returns (slope, intercept, s, z, p).
fn theil_sen_kendall(t: &[f64], y: &[f64]) -> Option<(f64, f64, i64, f64, f64)> {
    let n = t.len();
    if n < 2 || n != y.len() {
        return None;
    }
    let mut slopes = Vec::with_capacity(n * (n - 1) / 2);
    let mut s: i64 = 0;
    for i in 0..n {
        for j in (i + 1)..n {
            let dt = t[j] - t[i];
            if dt != 0.0 {
                slopes.push((y[j] - y[i]) / dt);
            }
            s += match (y[j] - y[i]).partial_cmp(&0.0) {
                Some(std::cmp::Ordering::Greater) => 1,
                Some(std::cmp::Ordering::Less) => -1,
                _ => 0,
            } * if dt >= 0.0 { 1 } else { -1 };
        }
    }
    if slopes.is_empty() {
        return None;
    }
    slopes.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let m = slopes.len();
    let slope = if m % 2 == 1 {
        slopes[m / 2]
    } else {
        (slopes[m / 2 - 1] + slopes[m / 2]) / 2.0
    };
    let mut resid: Vec<f64> = t.iter().zip(y).map(|(ti, yi)| yi - slope * ti).collect();
    resid.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let intercept = if n % 2 == 1 {
        resid[n / 2]
    } else {
        (resid[n / 2 - 1] + resid[n / 2]) / 2.0
    };
    let nf = n as f64;
    let var = nf * (nf - 1.0) * (2.0 * nf + 5.0) / 18.0;
    let z = if s > 0 {
        (s as f64 - 1.0) / var.sqrt()
    } else if s < 0 {
        (s as f64 + 1.0) / var.sqrt()
    } else {
        0.0
    };
    let p = erfc(z.abs() / std::f64::consts::SQRT_2);
    Some((slope, intercept, s, z, p))
}

/// Complementary error function, Abramowitz and Stegun 7.1.26 (|error| below
/// 1.5e-7), enough for a p-value reported to three figures.
fn erfc(x: f64) -> f64 {
    let t = 1.0 / (1.0 + 0.327_591_1 * x.abs());
    let poly = t
        * (0.254_829_592
            + t * (-0.284_496_736
                + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));
    let r = poly * (-x * x).exp();
    if x >= 0.0 {
        r
    } else {
        2.0 - r
    }
}

fn day_of_year(day_number: i64) -> (i32, i64) {
    let (y, m, d) = crate::civil_from_days(day_number);
    (
        y,
        crate::days_from_civil(y, m, d) - crate::days_from_civil(y, 1, 1) + 1,
    )
}

/// Circular distance between two days of year.
fn doy_gap(a: i64, b: i64) -> i64 {
    let d = (a - b).abs();
    d.min(365 - d)
}

/// Pick up to `k` of `n` ascending items, evenly spaced in rank, keeping both
/// ends: the series covers the window instead of its most recent month.
fn spread(n: usize, k: usize) -> Vec<usize> {
    if n <= k {
        return (0..n).collect();
    }
    if k == 1 {
        return vec![n - 1];
    }
    let mut out: Vec<usize> = (0..k)
        .map(|i| ((i as f64) * (n - 1) as f64 / (k - 1) as f64).round() as usize)
        .collect();
    out.dedup();
    out
}

fn item_day(it: &emem_fetch::stac::StacItem) -> Option<i64> {
    let (y, m, d) = parse_ymd(it.datetime.get(..10)?)?;
    Some(crate::days_from_civil(y, m, d))
}

/// For the newest candidate, the least cloudy candidate in each earlier year
/// within `SEASON_HALF_WIDTH_DAYS` of its day of year (at most three years
/// back). Even spacing alone almost never lands on the same season twice, and
/// without a same-season pair there is no anomaly and no honest change map.
fn same_season_partners(
    candidates: &[&emem_fetch::stac::StacItem],
    chosen: &[usize],
) -> Vec<usize> {
    let Some(&newest) = chosen.iter().max() else {
        return Vec::new();
    };
    let Some(newest_day) = item_day(candidates[newest]) else {
        return Vec::new();
    };
    let (newest_year, newest_doy) = day_of_year(newest_day);
    let mut best: std::collections::BTreeMap<i32, (usize, f64)> = Default::default();
    for (i, it) in candidates.iter().enumerate() {
        let Some(day) = item_day(it) else { continue };
        let (y, doy) = day_of_year(day);
        if y >= newest_year
            || y < newest_year - 3
            || doy_gap(doy, newest_doy) > SEASON_HALF_WIDTH_DAYS
        {
            continue;
        }
        let cloud = it.cloud_cover.unwrap_or(100.0);
        if best.get(&y).is_none_or(|(_, c)| cloud < *c) {
            best.insert(y, (i, cloud));
        }
    }
    best.into_values().map(|(i, _)| i).collect()
}

fn round6(x: f64) -> JsonValue {
    if x.is_finite() {
        json!((x * 1e6).round() / 1e6)
    } else {
        JsonValue::Null
    }
}

pub async fn field_series(req: FieldSeriesReq, s: &AppState) -> Result<JsonValue, ApiError> {
    let started = Instant::now();
    let aoi = parse_aoi(&req)?;
    let b = &aoi.bbox;
    let index_key = req.index.as_deref().unwrap_or("ndvi").to_ascii_lowercase();
    let index = INDICES.iter().find(|i| i.key == index_key).ok_or_else(|| {
        bad_request(format!(
            "index `{index_key}` is not served; this executor computes {}",
            INDICES.iter().map(|i| i.key).collect::<Vec<_>>().join(", ")
        ))
    })?;
    let aliases_of = |band: &str| {
        S2_BANDS
            .iter()
            .find(|(k, _)| *k == band)
            .map(|(_, a)| *a)
            .unwrap_or(&[])
    };
    let (alias_a, alias_b) = (aliases_of(index.a), aliases_of(index.b));

    let (sy, sm, sd) = parse_ymd(&req.start_date)
        .ok_or_else(|| bad_request(format!("start_date `{}` is not YYYY-MM-DD", req.start_date)))?;
    let (ey, em, ed) = parse_ymd(&req.end_date)
        .ok_or_else(|| bad_request(format!("end_date `{}` is not YYYY-MM-DD", req.end_date)))?;
    let start_day = crate::days_from_civil(sy, sm, sd);
    let end_day = crate::days_from_civil(ey, em, ed);
    if start_day > end_day {
        return Err(bad_request(
            "start_date must be on or before end_date".into(),
        ));
    }
    let window_days = end_day - start_day + 1;
    if window_days > MAX_WINDOW_DAYS {
        return Err(bad_request(format!(
            "the window is {window_days} days; the cap is {MAX_WINDOW_DAYS}. Split it, or use emem_compare_same_doy for a multi-year same-season comparison."
        )));
    }
    let max_scenes = req
        .max_scenes
        .unwrap_or(DEFAULT_SCENES)
        .clamp(3, MAX_SCENES);
    let min_clear = req.min_clear_fraction.unwrap_or(0.4).clamp(0.05, 1.0);
    let blocks = req.blocks.unwrap_or(6).clamp(2, 8);
    let mut reject_vec = req
        .mask_policy
        .clone()
        .unwrap_or_else(|| DEFAULT_SCL_REJECT.to_vec());
    reject_vec.sort_unstable();
    reject_vec.dedup();
    let reject: std::collections::BTreeSet<u8> = reject_vec.iter().copied().collect();

    // Same area, index, window and policy: same answer, from memory. A
    // dashboard re-runs its query for every reader, and each run would
    // otherwise re-read every scene and sign a fresh derivation of identical
    // rows. It also lets a call that outlived the host's budget (the work is
    // not cancelled) answer on the retry.
    let cache_key = serde_json::to_string(&json!({
        "bbox": [b.min_lat, b.min_lng, b.max_lat, b.max_lng],
        "rings": aoi.rings, "index": index.key, "start": req.start_date, "end": req.end_date,
        "max_scenes": max_scenes, "min_clear": min_clear, "blocks": blocks, "reject": reject_vec,
    }))
    .unwrap_or_default();
    if let Some(mut hit) = cache_get(&cache_key) {
        if let Some(c) = hit.get_mut("cache").and_then(|c| c.as_object_mut()) {
            c.insert("hit".into(), json!(true));
        }
        return Ok(hit);
    }

    let centre_lat = (b.min_lat + b.max_lat) / 2.0;
    let centre_lng = (b.min_lng + b.max_lng) / 2.0;
    let cli = crate::s2_http_client();

    // ── candidates: every scene in the window, both bands + SCL present. ──
    let datetime = format!("{}T00:00:00Z/{}T23:59:59Z", req.start_date, req.end_date);
    let mut failures: Vec<String> = Vec::new();
    let mut found = None;
    for host in crate::s2_catalogues() {
        let searched = emem_fetch::stac::search_many_at(
            &cli,
            host,
            "sentinel-2-l2a",
            centre_lng,
            centre_lat,
            &datetime,
            Some(CANDIDATE_MAX_CLOUD),
            200,
        )
        .await;
        let signed = match searched {
            Ok(v) if host == emem_fetch::stac::STAC_MPC_V1 => {
                crate::s2_sign_mpc_items(&cli, v).await
            }
            other => other,
        };
        match signed {
            Ok(v) => {
                found = Some((host, v));
                break;
            }
            Err(e) => failures.push(format!("{host}: {e}")),
        }
    }
    let (host, items) = found.ok_or_else(|| {
        upstream_error(format!(
            "stac: every Sentinel-2 catalogue failed: {}",
            failures.join("; ")
        ))
    })?;
    let usable: Vec<&emem_fetch::stac::StacItem> = items
        .iter()
        .filter(|it| {
            it.epsg.is_some()
                && scl_asset(it).is_some()
                && alias_a.iter().any(|a| it.assets.contains_key(*a))
                && alias_b.iter().any(|a| it.assets.contains_key(*a))
        })
        .collect();
    // One CRS for the whole series: the one most candidates share. An area on
    // a zone boundary is seen in two, and pixels from two CRSs are not one
    // lattice.
    let mut by_epsg: std::collections::BTreeMap<u32, usize> = Default::default();
    for it in &usable {
        *by_epsg.entry(it.epsg.unwrap_or(0)).or_default() += 1;
    }
    let epsg = by_epsg
        .iter()
        .max_by_key(|(_, n)| **n)
        .map(|(e, _)| *e)
        .ok_or_else(|| {
            upstream_error(format!(
                "no Sentinel-2 scene with {}, {} and SCL in {}..{} under {CANDIDATE_MAX_CLOUD}% scene cloud",
                index.a, index.b, req.start_date, req.end_date
            ))
        })?;
    // One scene per capture date: overlapping tiles see the same overpass, and
    // counting it twice would weight that day double. Keep the least cloudy.
    let mut per_date: std::collections::BTreeMap<String, &emem_fetch::stac::StacItem> =
        Default::default();
    for it in usable.iter().filter(|it| it.epsg == Some(epsg)) {
        let date = it.datetime.get(..10).unwrap_or("").to_string();
        let better = per_date
            .get(&date)
            .is_none_or(|cur| it.cloud_cover.unwrap_or(100.0) < cur.cloud_cover.unwrap_or(100.0));
        if better {
            per_date.insert(date, it);
        }
    }
    let candidates: Vec<&emem_fetch::stac::StacItem> = per_date.into_values().collect();
    let mut chosen: Vec<usize> = spread(candidates.len(), max_scenes);
    let same_season = same_season_partners(&candidates, &chosen);
    for i in &same_season {
        if !chosen.contains(i) {
            chosen.push(*i);
        }
    }
    chosen.sort_unstable();
    let picked: Vec<&emem_fetch::stac::StacItem> = chosen.iter().map(|i| candidates[*i]).collect();
    let anchor = picked
        .last()
        .copied()
        .ok_or_else(|| upstream_error("no candidate scene survived selection".into()))?;

    // ── the pixel lattice, from the newest picked scene. ──────────────────
    let anchor_url = alias_a
        .iter()
        .find_map(|a| anchor.assets.get(*a).cloned())
        .ok_or_else(|| upstream_error("anchor scene lost its band asset".into()))?;
    let prof = emem_fetch::cog::open_profile(&cli, &anchor_url)
        .await
        .map_err(|e| upstream_error(format!("open anchor COG: {e}")))?;
    let native_m = prof.pixel_scale.0.abs();
    if !(native_m > 0.0 && native_m.is_finite()) {
        return Err(upstream_error(format!(
            "anchor COG has implausible pixel scale {:?}",
            prof.pixel_scale
        )));
    }
    let to_utm = |lat: f64, lng: f64| {
        emem_fetch::proj::latlng_to_utm_with_epsg(lat, lng, epsg)
            .ok_or_else(|| upstream_error(format!("epsg {epsg} is not a UTM code")))
    };
    let utm_c = to_utm(centre_lat, centre_lng)?;
    let utm_min = to_utm(b.min_lat, b.min_lng)?;
    let utm_max = to_utm(b.max_lat, b.max_lng)?;
    let w = (((utm_max.easting - utm_min.easting).abs() / native_m).ceil() as u32).max(1);
    let h = (((utm_max.northing - utm_min.northing).abs() / native_m).ceil() as u32).max(1);
    if w > MAX_SIDE_PX || h > MAX_SIDE_PX {
        return Err(bad_request(format!(
            "the area is {w}x{h} px at {native_m} m; the cap is {MAX_SIDE_PX} per side. Shrink it or split it."
        )));
    }
    let (cc, cr) = prof.world_to_pixel(utm_c.easting, utm_c.northing);
    let (wx0, wy0) = prof.pixel_to_world(cc - (w as i64) / 2, cr - (h as i64) / 2);
    let centre_of = |r: u32, c: u32| {
        (
            wx0 + (c as f64 + 0.5) * native_m,
            wy0 - (r as f64 + 0.5) * native_m,
        )
    };
    // The area in UTM, so the inside test runs in the lattice's own metres.
    let rings_utm: Vec<Vec<[f64; 2]>> = aoi
        .rings
        .iter()
        .map(|ring| {
            ring.iter()
                .filter_map(|[lng, lat]| {
                    emem_fetch::proj::latlng_to_utm_with_epsg(*lat, *lng, epsg)
                        .map(|u| [u.easting, u.northing])
                })
                .collect()
        })
        .collect();
    let npix = (w as usize) * (h as usize);
    let mut inside = vec![false; npix];
    for r in 0..h {
        for c in 0..w {
            let (x, y) = centre_of(r, c);
            let in_box = x >= utm_min.easting.min(utm_max.easting)
                && x <= utm_min.easting.max(utm_max.easting)
                && y >= utm_min.northing.min(utm_max.northing)
                && y <= utm_min.northing.max(utm_max.northing);
            inside[(r * w + c) as usize] = if rings_utm.is_empty() {
                in_box
            } else {
                in_polygon(x, y, &rings_utm)
            };
        }
    }
    let n_inside = inside.iter().filter(|v| **v).count();
    if n_inside < MIN_SUPPORT_PX {
        return Err(bad_request(format!(
            "the area holds {n_inside} pixel centres at {native_m} m; a distribution needs at least {MIN_SUPPORT_PX}. Give a larger area."
        )));
    }

    // Block lattice: the window cut into blocks x blocks, each with its WGS-84
    // quad from the inverse projection of its corners.
    let block_of = |r: u32, c: u32| {
        (
            (r * blocks / h).min(blocks - 1),
            (c * blocks / w).min(blocks - 1),
        )
    };
    let mut block_quads: Vec<JsonValue> = Vec::new();
    for bi in 0..blocks {
        for bj in 0..blocks {
            let r0 = (bi * h).div_ceil(blocks);
            let r1 = ((bi + 1) * h).div_ceil(blocks);
            let c0 = (bj * w).div_ceil(blocks);
            let c1 = ((bj + 1) * w).div_ceil(blocks);
            let corner = |r: u32, c: u32| {
                let x = wx0 + c as f64 * native_m;
                let y = wy0 - r as f64 * native_m;
                emem_fetch::proj::utm_to_latlng_with_epsg(x, y, epsg)
                    .map(|(la, lo)| json!([round6(lo), round6(la)]))
                    .unwrap_or(JsonValue::Null)
            };
            block_quads.push(json!([[
                corner(r0, c0),
                corner(r0, c1),
                corner(r1, c1),
                corner(r1, c0),
                corner(r0, c0)
            ]]));
        }
    }

    // ── read every picked scene: two bands, each masked by its SCL. ───────
    // Bounded in chunks rather than a buffered stream: a stream of async
    // blocks borrowing the client is not provably Send to the MCP dispatcher's
    // spawn, and the plain futures `band_composite` joins are.
    let mut reads = Vec::with_capacity(picked.len());
    for chunk in picked.chunks(SCENE_READ_CONCURRENCY) {
        let pairs = futures_util::future::join_all(chunk.iter().map(|it| {
            futures_util::future::join(
                read_masked_scene(
                    &cli,
                    host,
                    it,
                    alias_a,
                    utm_c.easting,
                    utm_c.northing,
                    w,
                    h,
                    native_m,
                    &reject,
                ),
                read_masked_scene(
                    &cli,
                    host,
                    it,
                    alias_b,
                    utm_c.easting,
                    utm_c.northing,
                    w,
                    h,
                    native_m,
                    &reject,
                ),
            )
        }))
        .await;
        for (it, (a, b)) in chunk.iter().zip(pairs) {
            reads.push((*it, a, b));
        }
    }

    let nblocks = (blocks * blocks) as usize;
    let mut rows: Vec<JsonValue> = Vec::new();
    let mut excluded: Vec<JsonValue> = Vec::new();
    let mut sources: Vec<JsonValue> = Vec::new();
    let mut kept_days: Vec<i64> = Vec::new();
    let mut kept_medians: Vec<f64> = Vec::new();
    let mut kept_block_medians: Vec<Vec<Option<f64>>> = Vec::new();
    for (item, ra, rb) in reads {
        let date = item.datetime.get(..10).unwrap_or("").to_string();
        let (Some((ga, meta_a)), Some((gb, meta_b))) = (ra, rb) else {
            excluded.push(json!({
                "date": date, "scene": item.id,
                "reason": "unreadable: a band or its scene classification could not be read on this lattice",
            }));
            continue;
        };
        let mut clear: Vec<f32> = Vec::new();
        let mut per_block: Vec<Vec<f32>> = vec![Vec::new(); nblocks];
        let mut inside_per_block = vec![0usize; nblocks];
        for r in 0..h {
            for c in 0..w {
                let p = (r * w + c) as usize;
                if !inside[p] {
                    continue;
                }
                let (bi, bj) = block_of(r, c);
                let k = (bi * blocks + bj) as usize;
                inside_per_block[k] += 1;
                let (va, vb) = (ga[p], gb[p]);
                // 0 is L2A no-data after harmonising; NaN is masked.
                if !(va.is_finite() && vb.is_finite()) || va <= 0.0 || vb <= 0.0 {
                    continue;
                }
                let v = ((va - vb) / (va + vb)).clamp(-1.0, 1.0);
                clear.push(v);
                per_block[k].push(v);
            }
        }
        let st = distribution(&mut clear, n_inside);
        let clear_fraction = st.n_clear as f64 / st.n_aoi as f64;
        let day = crate::days_from_civil(
            date.get(..4).and_then(|x| x.parse().ok()).unwrap_or(1970),
            date.get(5..7).and_then(|x| x.parse().ok()).unwrap_or(1),
            date.get(8..10).and_then(|x| x.parse().ok()).unwrap_or(1),
        );
        sources.push(json!({"scene": item.id, "date": date, "a": meta_a, "b": meta_b}));
        if clear_fraction < min_clear || st.n_clear < MIN_SUPPORT_PX {
            excluded.push(json!({
                "date": date, "scene": item.id,
                "clear_fraction": round6(clear_fraction),
                "n_clear": st.n_clear,
                "reason": if st.n_clear < MIN_SUPPORT_PX {
                    format!("{} clear pixels, under the {MIN_SUPPORT_PX}-pixel support floor", st.n_clear)
                } else {
                    format!("{:.0}% of the area clear, under the {:.0}% floor", clear_fraction * 100.0, min_clear * 100.0)
                },
            }));
            continue;
        }
        let block_medians: Vec<Option<f64>> = per_block
            .iter_mut()
            .zip(&inside_per_block)
            .map(|(v, n_in)| {
                // A block counts only when at least half of its in-area
                // pixels are clear, so a cloud edge does not paint a block.
                if *n_in == 0 || v.len() * 2 < *n_in {
                    return None;
                }
                v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                Some(quantile(v, 0.5))
            })
            .collect();
        rows.push(json!({
            "date": date,
            "scene": item.id,
            "scene_cloud_cover": item.cloud_cover,
            // The scene classification misses haze and thin cloud edges, and
            // what it misses reads low, so a partly clear scene is the likely
            // place for a biased median. A chart draws these lighter.
            "quality": if clear_fraction >= CLEAR_SCENE_FRACTION { "clear" } else { "partial" },
            "clear_fraction": round6(clear_fraction),
            "n_clear": st.n_clear,
            "median": round6(st.median),
            "p10": round6(st.p10), "p25": round6(st.p25),
            "p75": round6(st.p75), "p90": round6(st.p90),
            "mean": round6(st.mean), "std": round6(st.std),
            "block_medians": block_medians.iter().map(|m| m.map(round6).unwrap_or(JsonValue::Null)).collect::<Vec<_>>(),
        }));
        kept_days.push(day);
        kept_medians.push(st.median);
        kept_block_medians.push(block_medians);
    }

    // ── what a reader would otherwise get wrong by eye. ───────────────────
    let mut cautions: Vec<String> = Vec::new();
    let months: std::collections::BTreeSet<u32> = kept_days
        .iter()
        .map(|d| crate::civil_from_days(*d).1)
        .collect();
    let trend = if rows.len() >= MIN_TREND_SCENES {
        let t: Vec<f64> = kept_days.iter().map(|d| *d as f64 / 365.25).collect();
        theil_sen_kendall(&t, &kept_medians).map(|(slope, _icpt, s, z, p)| {
            json!({
                "method": "Theil-Sen slope of the per-scene median; Kendall S with the no-ties normal approximation, two-sided",
                "slope_per_year": round6(slope),
                "kendall_s": s,
                "z": round6(z),
                "p_value": round6(p),
                "n": rows.len(),
                "seasonal_cycle_removed": false,
            })
        })
    } else {
        None
    };
    if trend.is_some() && (window_days < 365 || months.len() < 6) {
        cautions.push(format!(
            "the trend is over {window_days} days with clear scenes in {} of 12 months: on a seasonal index it measures the season as much as change. Compare the same season across years (emem_compare_same_doy) before calling it change.",
            months.len()
        ));
    }
    // A trend line through a strongly seasonal series reports the sampling
    // calendar as much as change. Say how much bigger the swing is.
    if let Some(slope) = trend
        .as_ref()
        .and_then(|t| t.get("slope_per_year"))
        .and_then(|v| v.as_f64())
    {
        let (lo, hi) = kept_medians
            .iter()
            .fold((f64::MAX, f64::MIN), |(a, b), m| (a.min(*m), b.max(*m)));
        let span_years =
            (kept_days.last().unwrap_or(&0) - kept_days.first().unwrap_or(&0)) as f64 / 365.25;
        let fitted = (slope * span_years).abs();
        let swing = hi - lo;
        if swing > 4.0 * fitted {
            cautions.push(format!(
                "the series swings {swing:.2} across scenes while the fitted trend moves {fitted:.2} over the window: the seasonal cycle dominates, so read the trend as weak evidence and use the same-season anomaly for change."
            ));
        }
    }
    if rows.len() < MIN_TREND_SCENES {
        cautions.push(format!(
            "{} scene(s) cleared the mask; a trend needs {MIN_TREND_SCENES}, so none is reported.",
            rows.len()
        ));
    }
    let anomaly = kept_days.last().and_then(|last_day| {
        let (last_year, last_doy) = day_of_year(*last_day);
        let baseline: Vec<f64> = kept_days
            .iter()
            .zip(&kept_medians)
            .filter(|(d, _)| {
                let (y, doy) = day_of_year(**d);
                y < last_year && doy_gap(doy, last_doy) <= SEASON_HALF_WIDTH_DAYS
            })
            .map(|(_, m)| *m)
            .collect();
        if baseline.is_empty() {
            return None;
        }
        let mut sorted = baseline.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let base = if sorted.len() % 2 == 1 {
            sorted[sorted.len() / 2]
        } else {
            (sorted[sorted.len() / 2 - 1] + sorted[sorted.len() / 2]) / 2.0
        };
        let latest = *kept_medians.last()?;
        Some(json!({
            "method": format!("latest scene's median minus the median of earlier years' scene medians within {SEASON_HALF_WIDTH_DAYS} days of its day of year"),
            "latest": round6(latest),
            "same_season_baseline": round6(base),
            "anomaly": round6(latest - base),
            "baseline_n": baseline.len(),
        }))
    });
    if anomaly.is_none() && !rows.is_empty() {
        cautions.push(if window_days >= 365 {
            format!("no same-season anomaly: no earlier-year scene within {SEASON_HALF_WIDTH_DAYS} days of the latest one's day of year was clear enough (scene cloud under {CANDIDATE_MAX_CLOUD}% and the area's clear floor). That season is cloudy here; the latest value has no like-for-like baseline.")
        } else {
            format!("no same-season anomaly: the window is {window_days} days, so it holds no earlier year. Start it at least a year before the end to get one.")
        });
    }
    // Pair the latest kept scene with the earlier-year kept scene nearest its
    // day of year; only without one fall back to the first, and say so.
    let from_idx = kept_days.last().and_then(|last| {
        let (ly, ld) = day_of_year(*last);
        kept_days
            .iter()
            .enumerate()
            .filter(|(_, d)| day_of_year(**d).0 < ly)
            .min_by_key(|(_, d)| doy_gap(day_of_year(**d).1, ld))
            .map(|(i, _)| i)
            .filter(|i| doy_gap(day_of_year(kept_days[*i]).1, ld) <= SEASON_HALF_WIDTH_DAYS)
    });
    let pairing = if from_idx.is_some() {
        "same_season"
    } else {
        "first_to_last"
    };
    let change_map = match (
        kept_block_medians.get(from_idx.unwrap_or(0)),
        kept_block_medians.last(),
    ) {
        (Some(first), Some(last)) if kept_block_medians.len() >= 2 => {
            let from = from_idx.unwrap_or(0);
            let (first_day, last_day) = (kept_days[from], kept_days[kept_days.len() - 1]);
            let gap = doy_gap(day_of_year(first_day).1, day_of_year(last_day).1);
            if gap > SEASON_HALF_WIDTH_DAYS {
                cautions.push(format!(
                    "the change map compares scenes {gap} days of year apart: part of every block's change is the season."
                ));
            }
            let features: Vec<JsonValue> = (0..nblocks)
                .map(|k| {
                    let (f, l) = (first[k], last[k]);
                    json!({
                        "type": "Feature",
                        "geometry": {"type": "Polygon", "coordinates": block_quads[k]},
                        "properties": {
                            "block": k,
                            "first": f.map(round6),
                            "last": l.map(round6),
                            "change": match (f, l) { (Some(f), Some(l)) => round6(l - f), _ => JsonValue::Null },
                        },
                    })
                })
                .collect();
            Some(json!({
                "type": "FeatureCollection",
                "from": rows.get(from).and_then(|r| r.get("date")).cloned(),
                "to": rows.last().and_then(|r| r.get("date")).cloned(),
                "pairing": pairing,
                "day_of_year_gap": gap,
                "features": features,
            }))
        }
        _ => None,
    };

    // ── sign the series. ──────────────────────────────────────────────────
    let aoi_id = aoi_cid(b)?;
    let estimand = json!({
        "quantity": format!("{} of the area, per Sentinel-2 overpass", index.key.to_uppercase()),
        "operator": index.formula,
        "reference": index.reference,
        "support": format!("{native_m} m Sentinel-2 L2A pixels whose centre lies inside the area ({n_inside} pixels)"),
        "statistic": "per scene: median and nearest-rank p10/p25/p75/p90 over clear pixels, plus mean and sample std",
        "mask": {"reject_scl": reject_vec, "keep_snow_11": !reject.contains(&11)},
        "unit": "dimensionless, -1..1",
        "clear_fraction_means": "share of the area's pixels with an unmasked, in-swath reading of both bands",
        "known_bias": "the scene classification misses haze and thin cloud edges; what it misses lowers the index, so `partial` scenes can read low",
    });
    let record = json!({
        "schema": "emem.field_series.v1",
        "fn_key": FN_KEY,
        "aoi": {"bbox": {"min_lat": b.min_lat, "min_lng": b.min_lng, "max_lat": b.max_lat, "max_lng": b.max_lng},
                "polygon": if aoi.rings.is_empty() { JsonValue::Null } else { json!(aoi.rings) }},
        "aoi_cid": aoi_id,
        "index": index.key,
        "estimand": estimand,
        "window": {"start": req.start_date, "end": req.end_date, "days": window_days},
        "selection": {
            "rule": "one scene per capture date (least scene cloud), then up to max_scenes evenly spaced in time keeping both ends, plus for the newest the least cloudy scene within 30 days of its day of year in each of up to three earlier years",
            "same_season_added": same_season.len(),
            "candidate_dates": candidates.len(),
            "max_scenes": max_scenes,
            "scene_max_cloud_pct": CANDIDATE_MAX_CLOUD,
            "epsg": epsg,
            "min_clear_fraction": min_clear,
            "min_support_px": MIN_SUPPORT_PX,
        },
        // The window's north-west corner in the CRS: with width, height and
        // the step it fixes every pixel and every block quad, so the map
        // re-derives from the record.
        "grid": {"width": w, "height": h, "native_m": native_m, "epsg": epsg, "blocks_per_side": blocks,
                 "x0": wx0, "y0": wy0, "origin": "north-west corner of the window, in the scene CRS"},
        "rows": rows,
        "excluded": excluded,
        "trend": trend,
        "anomaly": anomaly,
        "sources": sources,
    });
    let signed_at = emem_storage::server::iso8601_now();
    let centre_cell = emem_codec::geo::cell64_from_latlng(centre_lat, centre_lng);
    let first_slot = kept_days.first().copied().unwrap_or(start_day).max(0) as u64;
    let last_slot = kept_days.last().copied().unwrap_or(end_day).max(0) as u64;
    let fact = DerivativeFact {
        cell: centre_cell.clone(),
        band: "field.series".to_string(),
        tslot_window: [first_slot, last_slot],
        op: "index_series".to_string(),
        parents: vec![],
        value: json_to_cbor(&record),
        confidence: 1.0,
        derivation: Derivation {
            fn_key: FN_KEY.to_string(),
            args: None,
        },
        schema_cid: s.manifests.schema_cid.clone(),
        signer: s.identity.pubkey,
        signed_at: signed_at.clone(),
    };
    let att = emem_fact::Attestation::build_and_sign_v1(
        vec![Fact::Derivative(fact)],
        vec![],
        s.manifests.registry_cid.clone(),
        s.manifests.schema_cid.clone(),
        &s.identity.signing,
        s.identity.epoch,
        signed_at,
        None,
    )
    .map_err(|e| upstream_error(format!("series attestation: {e}")))?;
    let cids = s
        .storage
        .put_attestation(&att)
        .await
        .map_err(ApiError::from)?;
    let derivation_cid = cids
        .first()
        .map(|c| c.as_str().to_string())
        .ok_or_else(|| upstream_error("store returned no cid for the series".into()))?;
    let receipt = s.sign_receipt_field(
        "emem.field_series",
        vec![centre_cell.clone()],
        vec![FactCid::new(derivation_cid.clone())],
        false,
        started,
        FieldBinding {
            aoi_cid: aoi_id.clone(),
            derivation_cid: derivation_cid.clone(),
        },
    );
    let out = json!({
        "schema": "emem.field_series.v1",
        "algorithm_key": FN_KEY,
        "index": index.key,
        "estimand": record["estimand"],
        "window": record["window"],
        "columns": ["date", "median", "p10", "p25", "p75", "p90", "mean", "std", "clear_fraction", "n_clear"],
        "rows": record["rows"],
        "excluded": record["excluded"],
        "trend": record["trend"],
        "anomaly": record["anomaly"],
        "change_map": change_map,
        "blocks": {
            "per_side": blocks,
            "order": "row-major from the north-west block; each row's block_medians follows this order",
            "quads": block_quads,
            "rule": "a block's median counts when at least half of its in-area pixels are clear",
        },
        "selection": record["selection"],
        "cautions": cautions,
        "derivation_cid": derivation_cid,
        "token": format!("emem:fact:{centre_cell}:{derivation_cid}"),
        "reproducibility": "every scene id, asset and mask is pinned in the signed record (resolve the token for it); re-reading those scenes with the stated formula, mask, quantile rule and selection gives the same rows",
        "receipt": receipt,
        "cache": {
            "hit": false,
            "computed_at": emem_storage::server::iso8601_now(),
            "ttl_s": cache_ttl_secs(),
            "_means": "a repeat of this exact request inside ttl_s returns this same signed series; a scene acquired since computed_at appears after it expires",
        },
    });
    cache_put(cache_key, out.clone());
    Ok(out)
}

type SeriesCache = std::sync::Mutex<std::collections::HashMap<String, (JsonValue, u64)>>;

fn series_cache() -> &'static SeriesCache {
    static CACHE: std::sync::OnceLock<SeriesCache> = std::sync::OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// `EMEM_FIELD_SERIES_CACHE_SECS`, default six hours: a Sentinel-2 revisit is
/// about five days, so a series six hours old misses at most one overpass.
fn cache_ttl_secs() -> u64 {
    std::env::var("EMEM_FIELD_SERIES_CACHE_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(6 * 3600)
        .min(86_400)
}

const SERIES_CACHE_CAP: usize = 256;

fn cache_get(key: &str) -> Option<JsonValue> {
    let now = crate::now_unix_ms();
    let mut map = series_cache().lock().ok()?;
    match map.get(key) {
        Some((v, exp)) if now < *exp => Some(v.clone()),
        Some(_) => {
            map.remove(key);
            None
        }
        None => None,
    }
}

fn cache_put(key: String, value: JsonValue) {
    let ttl_ms = cache_ttl_secs() * 1000;
    if ttl_ms == 0 || key.is_empty() {
        return;
    }
    let now = crate::now_unix_ms();
    if let Ok(mut map) = series_cache().lock() {
        map.retain(|_, (_, exp)| *exp > now);
        if map.len() >= SERIES_CACHE_CAP {
            if let Some(oldest) = map
                .iter()
                .min_by_key(|(_, (_, e))| *e)
                .map(|(k, _)| k.clone())
            {
                map.remove(&oldest);
            }
        }
        map.insert(key, (value, now + ttl_ms));
    }
}

/// The MCP answer: what a model charts and cites, inside the host's ~24 KB
/// result budget. The map frames (per-row block medians, block quads) and the
/// change map's GeoJSON are the bulk and are for a renderer, so they stay on
/// the REST response, and the change map is summarised. `_projection` names
/// every cut and where it is whole.
pub(crate) fn mcp_projection(mut v: JsonValue) -> JsonValue {
    let Some(obj) = v.as_object_mut() else {
        return v;
    };
    let mut omitted: Vec<&str> = Vec::new();
    // Four decimals is a ten-thousandth of the index range, finer than a
    // Sentinel-2 reflectance step moves it; the signed record keeps six.
    let round4 = |v: &mut JsonValue| {
        if let Some(x) = v.as_f64().filter(|_| v.is_f64()) {
            *v = json!((x * 1e4).round() / 1e4);
        }
    };
    for key in ["rows", "excluded"] {
        if let Some(rows) = obj.get_mut(key).and_then(|r| r.as_array_mut()) {
            for row in rows {
                if let Some(r) = row.as_object_mut() {
                    r.remove("block_medians");
                    r.remove("scene");
                    r.remove("scene_cloud_cover");
                    r.values_mut().for_each(round4);
                }
            }
        }
    }
    omitted.push(
        "rows[].block_medians, rows[].scene, rows[].scene_cloud_cover (in the signed record)",
    );
    if obj.remove("blocks").is_some() {
        omitted.push("blocks");
    }
    if let Some(cm) = obj.remove("change_map").filter(|c| !c.is_null()) {
        let changes: Vec<f64> = cm["features"]
            .as_array()
            .map(|f| {
                f.iter()
                    .filter_map(|x| x["properties"]["change"].as_f64())
                    .collect()
            })
            .unwrap_or_default();
        let n = changes.len();
        let mut sorted = changes.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        // The map, compact: the block values in `blocks.order` and the
        // window's WGS-84 corners. A renderer draws the grid from these;
        // the per-block GeoJSON quads stay on the REST answer.
        let props = |key: &str| -> Vec<JsonValue> {
            cm["features"]
                .as_array()
                .map(|f| {
                    f.iter()
                        .map(|x| match x["properties"][key].as_f64() {
                            Some(v) => json!((v * 1e4).round() / 1e4),
                            None => JsonValue::Null,
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        let corners: Vec<[f64; 2]> = cm["features"]
            .as_array()
            .map(|f| {
                f.iter()
                    .filter_map(|x| x["geometry"]["coordinates"][0].as_array())
                    .flatten()
                    .filter_map(|p| Some([p[0].as_f64()?, p[1].as_f64()?]))
                    .collect()
            })
            .unwrap_or_default();
        let bounds = corners.iter().fold(None, |acc: Option<[f64; 4]>, [x, y]| {
            Some(match acc {
                None => [*x, *y, *x, *y],
                Some([a, b, c, d]) => [a.min(*x), b.min(*y), c.max(*x), d.max(*y)],
            })
        });
        let per_side = (props("change").len() as f64).sqrt().round() as usize;
        obj.insert(
            "change_grid".into(),
            json!({
                "per_side": per_side,
                "order": "row-major from the north-west block",
                "bounds_wgs84": bounds.map(|[a, b, c, d]| json!({"min_lng": a, "min_lat": b, "max_lng": c, "max_lat": d})),
                "from": props("first"), "to": props("last"), "change": props("change"),
            }),
        );
        obj.insert(
            "change_summary".into(),
            json!({
                "from": cm["from"], "to": cm["to"], "pairing": cm["pairing"],
                "day_of_year_gap": cm["day_of_year_gap"],
                "blocks_compared": n,
                "median_change": if n > 0 { round6(sorted[n / 2]) } else { JsonValue::Null },
                "min_change": sorted.first().map(|x| round6(*x)),
                "max_change": sorted.last().map(|x| round6(*x)),
            }),
        );
        omitted.push("change_map (summarised as change_summary)");
    }
    obj.insert(
        "_projection".into(),
        json!({
            "omitted": omitted,
            "whole_at": "POST /v1/field_series with the same arguments returns the map frames and the change map as GeoJSON; the signed record behind `token` holds every row's block medians and the grid origin",
        }),
    );
    v
}

pub async fn post_field_series(
    State(s): State<AppState>,
    EmemJson(req): EmemJson<FieldSeriesReq>,
) -> Result<Json<JsonValue>, ApiError> {
    Ok(Json(field_series(req, &s).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_rank_quantiles_are_values_a_pixel_had() {
        let v: Vec<f32> = (1..=10).map(|i| i as f32 / 10.0).collect();
        assert_eq!(quantile(&v, 0.5), v[4] as f64);
        assert_eq!(quantile(&v, 0.1), v[0] as f64);
        assert_eq!(quantile(&v, 0.9), v[8] as f64);
        assert_eq!(quantile(&v, 1.0), v[9] as f64);
        assert!(quantile(&[], 0.5).is_nan());
    }

    #[test]
    fn theil_sen_recovers_a_line_through_an_outlier() {
        let t: Vec<f64> = (0..10).map(|i| i as f64).collect();
        let mut y: Vec<f64> = t.iter().map(|x| 0.2 + 0.05 * x).collect();
        y[7] = 5.0;
        let (slope, icpt, s, _z, p) = theil_sen_kendall(&t, &y).unwrap();
        assert!((slope - 0.05).abs() < 1e-12, "{slope}");
        assert!((icpt - 0.2).abs() < 1e-12, "{icpt}");
        assert_eq!(s, 45 - 2 * 2, "the outlier reverses two of the 45 pairs");
        assert!(p < 0.001, "{p}");
        let flat = theil_sen_kendall(&t, &[0.3; 10]).unwrap();
        assert_eq!((flat.0, flat.2), (0.0, 0));
        assert!((flat.4 - 1.0).abs() < 1e-6);
    }

    #[test]
    fn erfc_matches_known_values() {
        assert!((erfc(0.0) - 1.0).abs() < 2e-7);
        assert!((erfc(1.0) - 0.157_299_207).abs() < 2e-7);
        assert!((erfc(-1.0) - 1.842_700_793).abs() < 2e-7);
    }

    #[test]
    fn a_hole_is_outside_the_polygon() {
        let outer = vec![
            [0.0, 0.0],
            [10.0, 0.0],
            [10.0, 10.0],
            [0.0, 10.0],
            [0.0, 0.0],
        ];
        let hole = vec![[4.0, 4.0], [6.0, 4.0], [6.0, 6.0], [4.0, 6.0], [4.0, 4.0]];
        let rings = vec![outer, hole];
        assert!(in_polygon(1.0, 1.0, &rings));
        assert!(!in_polygon(5.0, 5.0, &rings));
        assert!(!in_polygon(11.0, 5.0, &rings));
        assert!(in_polygon(5.0, 5.0, &[]));
    }

    #[test]
    fn the_spread_keeps_both_ends_and_spaces_the_rest() {
        assert_eq!(spread(5, 10), vec![0, 1, 2, 3, 4]);
        assert_eq!(spread(100, 3), vec![0, 50, 99]);
        let s = spread(40, 16);
        assert_eq!((s[0], *s.last().unwrap(), s.len()), (0, 39, 16));
    }

    #[test]
    fn the_mcp_answer_keeps_the_chart_and_names_what_it_cut() {
        let full = json!({
            "rows": [{"date": "2026-01-01", "median": 0.5, "block_medians": [0.4, 0.6]}],
            "blocks": {"quads": [[[1, 2]]]},
            "change_map": {"from": "2025-01-03", "to": "2026-01-01", "pairing": "same_season",
                "day_of_year_gap": 2, "features": [
                    {"properties": {"change": 0.1}}, {"properties": {"change": -0.3}},
                    {"properties": {"change": null}}]},
            "token": "emem:fact:c:d",
        });
        let p = mcp_projection(full);
        assert!(p["rows"][0].get("block_medians").is_none());
        assert_eq!(p["rows"][0]["median"], json!(0.5));
        assert!(p.get("blocks").is_none() && p.get("change_map").is_none());
        assert_eq!(p["change_summary"]["blocks_compared"], json!(2));
        assert_eq!(p["change_summary"]["min_change"], json!(-0.3));
        assert_eq!(p["change_summary"]["pairing"], json!("same_season"));
        assert_eq!(p["change_grid"]["change"], json!([0.1, -0.3, null]));
        assert_eq!(p["token"], json!("emem:fact:c:d"));
        assert_eq!(p["_projection"]["omitted"].as_array().unwrap().len(), 3);
        let q = mcp_projection(
            json!({"rows": [{"median": 0.123456, "n_clear": 9595, "scene": "S2A_x"}]}),
        );
        assert_eq!(q["rows"][0]["median"], json!(0.1235));
        assert_eq!(q["rows"][0]["n_clear"], json!(9595));
        assert!(q["rows"][0].get("scene").is_none());
    }

    #[test]
    fn a_cached_series_is_served_until_it_expires() {
        cache_put(
            "k-test".into(),
            json!({"rows": [1], "cache": {"hit": false}}),
        );
        assert_eq!(cache_get("k-test").unwrap()["rows"], json!([1]));
        assert!(cache_get("k-absent").is_none());
    }

    #[test]
    fn day_of_year_distance_wraps_the_new_year() {
        assert_eq!(doy_gap(360, 5), 10);
        assert_eq!(doy_gap(100, 130), 30);
    }
}
