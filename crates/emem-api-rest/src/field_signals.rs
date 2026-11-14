//! Field-level signals a compliance pack asks for by name, composed from
//! signed per-cell facts so every number cites the observations it came
//! from:
//!
//! - `field_burn_scar@1`: residue burning on a field over a window, by the
//!   registered `residue_burn_multisensor@1` rule (NBR drop, NDTI drop,
//!   MODIS burned area) applied between consecutive clear Sentinel-2 dates.
//!   NBR and the tillage NDTI are read over the whole field at 20 m by the
//!   field series engine, one signed series per index.
//! - `field_actual_et@1`: actual evapotranspiration over a window, the
//!   MOD16A2 8-day composites (`modis.et_8day`) pro-rated to its edges.
//!
//! The MODIS reads warm through the backfill preparer first, so a field
//! nobody asked about before answers on the first call, partially if the
//! budget runs out, and both say what they could not evaluate.

use std::time::Instant;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value as JsonValue};

use emem_core::tslot::{Tempo, Tslot};
use emem_core::ErrorCode;
use emem_fact::{Fact, FactCid};
use emem_primitives::cbor_ops::as_f64;
use emem_primitives::trajectory::{trajectory_keeping, TrajectoryReq};

use crate::{ApiError, AppState, BackfillReq, EmemJson, ErrorBody};

const DAY: i64 = 86_400;

/// The field and window both signals take.
#[derive(Debug, Deserialize)]
pub struct FieldReq {
    /// GeoJSON Polygon / MultiPolygon, or `{bbox: [w, s, e, n]}`.
    #[serde(default)]
    pub geometry_geojson: Option<JsonValue>,
    /// Or the field's cells directly.
    #[serde(default)]
    pub cells: Option<Vec<String>>,
    /// First day, `YYYY-MM-DD`, inclusive.
    pub start: String,
    /// Last day, `YYYY-MM-DD`, inclusive.
    pub end: String,
    /// Burn scar only: about how many blocks the field's map is cut into
    /// (default 16, at most 64), as whole blocks per side from 2 to 8.
    #[serde(default)]
    pub max_cells: Option<usize>,
    /// Warm-up budget in milliseconds (default 25000, at most 300000). The
    /// warm-up detaches when it runs out, so a retry reads what it finished.
    #[serde(default)]
    pub budget_ms: Option<u64>,
}

fn bad_request(msg: impl Into<String>) -> ApiError {
    ApiError(
        StatusCode::BAD_REQUEST,
        ErrorBody {
            code: ErrorCode::InvalidArgument,
            message: msg.into(),
            details: None,
        },
    )
}

fn day_unix(d: &str) -> Option<i64> {
    crate::parse_iso8601_unix(&format!("{}T00:00:00Z", d.trim()))
}

fn day_str(unix: i64) -> String {
    crate::iso8601_utc(unix.max(0) as u64)[..10].to_string()
}

/// The window as `[start, end)` in Unix seconds, `end` being the day after
/// the last one.
fn window(req: &FieldReq) -> Result<(i64, i64), ApiError> {
    let s = day_unix(&req.start).ok_or_else(|| bad_request("`start` must be YYYY-MM-DD"))?;
    let e = day_unix(&req.end).ok_or_else(|| bad_request("`end` must be YYYY-MM-DD"))? + DAY;
    if e <= s {
        return Err(bad_request("`end` must not be before `start`"));
    }
    if e - s > 400 * DAY {
        return Err(bad_request(
            "the window may span at most 400 days; ask per season",
        ));
    }
    Ok((s, e))
}

/// The field's cells, and its centroid cell.
fn field_cells(req: &FieldReq, n: usize) -> Result<(Vec<String>, String, Option<f64>), ApiError> {
    if let Some(cells) = req.cells.as_ref().filter(|c| !c.is_empty()) {
        let mut ll = Vec::new();
        for c in cells {
            let p = emem_codec::latlng_from_cell64(c)
                .map_err(|e| bad_request(format!("cell {c}: {e}")))?;
            ll.push((p.lat_deg, p.lng_deg));
        }
        let lat = ll.iter().map(|p| p.0).sum::<f64>() / ll.len() as f64;
        let lng = ll.iter().map(|p| p.1).sum::<f64>() / ll.len() as f64;
        let centre = emem_codec::cell64_from_latlng(lat, lng);
        return Ok((cells.iter().take(n).cloned().collect(), centre, None));
    }
    let g = req
        .geometry_geojson
        .as_ref()
        .ok_or_else(|| bad_request("give `geometry_geojson` or `cells`"))?;
    let (bbox, poly, area_ha) = crate::extract_plot_geometry(g).map_err(bad_request)?;
    let cells = match poly {
        Some(p) => crate::sample_cells_in_polygon(bbox, n, &[p]),
        None => crate::sample_cells_in_bbox(bbox, n),
    };
    let (mn_la, mx_la, mn_ln, mx_ln) = bbox;
    let centre = emem_codec::cell64_from_latlng((mn_la + mx_la) / 2.0, (mn_ln + mx_ln) / 2.0);
    if cells.is_empty() {
        return Err(bad_request("the geometry holds no cell centre"));
    }
    Ok((cells, centre, Some(area_ha)))
}

/// Warm `band` over the window at `cells`; returns whether it converged.
async fn warm(s: &AppState, cells: &[String], band: &str, win: (i64, i64), budget_ms: u64) -> bool {
    let req = BackfillReq {
        cell: String::new(),
        cells: None,
        band: band.to_string(),
        start_unix: Some(win.0),
        end_unix: Some(win.1),
        max_facts: None,
        budget_ms: Some(budget_ms),
        refresh: false,
    };
    match crate::backfill_prepare(cells.to_vec(), req, s).await {
        Ok(v) => v["converged"].as_bool().unwrap_or(false),
        Err(_) => false,
    }
}

/// One signed observation: its day, value, the acquisition it names, and cid.
#[derive(Debug, Clone)]
struct Obs {
    unix: i64,
    value: f64,
    captured_at: Option<String>,
    cid: String,
}

/// The stored series of `band` at `cell` inside the window, with each
/// fact's own acquisition time (a tslot is a bucket, not a date).
async fn series(s: &AppState, cell: &str, band: &str, win: (i64, i64)) -> Vec<Obs> {
    let tempo = crate::band_tempo_for_key(band).unwrap_or(Tempo::Fast);
    let req = TrajectoryReq {
        cell: cell.to_string(),
        band: band.to_string(),
        window: [
            Tslot::from_unix(win.0, tempo).0,
            Tslot::from_unix(win.1 - 1, tempo).0,
        ],
        as_of_tslot: None,
        as_of_signed_at: None,
        scope: None,
    };
    let Ok(tr) = trajectory_keeping(&req, s, &|p| !crate::superseded_read(p)).await else {
        return Vec::new();
    };
    let cids: Vec<FactCid> = tr
        .series
        .iter()
        .map(|p| FactCid::new(p.fact_cid.clone()))
        .collect();
    let facts = s
        .storage
        .get_facts_many_uncited(&cids)
        .await
        .unwrap_or_default();
    let mut out = Vec::new();
    for (p, f) in tr.series.iter().zip(facts) {
        let Some(v) = as_f64(&p.value).filter(|v| v.is_finite()) else {
            continue;
        };
        let captured_at = match f {
            Some(Fact::Primary(pf)) => pf.sources.first().and_then(|src| src.captured_at.clone()),
            _ => None,
        };
        let unix = captured_at
            .as_deref()
            .and_then(crate::parse_iso8601_unix)
            .unwrap_or_else(|| Tslot(p.tslot).to_unix_start(tempo));
        if unix >= win.0 && unix < win.1 {
            out.push(Obs {
                unix,
                value: v,
                captured_at,
                cid: p.fact_cid.clone(),
            });
        }
    }
    out.sort_by_key(|o| o.unix);
    out
}

// ─────────────────────────── field_burn_scar@1 ───────────────────────────

/// `residue_burn_multisensor@1` thresholds.
const DNBR_BURN: f64 = 0.20;
const DNDTI_BURN: f64 = 0.10;
/// Below the Key & Benson "low severity" floor a drop is not reported.
const DNBR_FLOOR: f64 = 0.10;
/// Two clear dates further apart than this cannot bracket a residue fire:
/// the scar is ploughed in or regrown within weeks.
const MAX_GAP_DAYS: i64 = 30;

/// The indices the rule reads, computed by the field series engine on the
/// field's own 20 m pixels: NBR from B8A and B12, and NDTI, the tillage
/// index, from B11 and B12 (van Deventer et al. 1997). The stored bands of
/// those names are not read. `indices.ndti` is another index under the same
/// name, the Lacaux turbidity index (B04 - B03) / (B04 + B03), and
/// `indices.nbr` pairs B08 (10 m) with B12 (20 m).
const BURN_NBR_INDEX: &str = "nbr";
const BURN_NDTI_INDEX: &str = "ndti";
/// Scenes per series: the engine's cap, so a season keeps the clear dates a
/// pair needs.
const BURN_MAX_SCENES: usize = 96;
/// Clear pixels a scene needs over the field to count: the engine's lowest
/// floor, so a small field still answers. Each row states its own count.
const BURN_MIN_SUPPORT_PX: usize = 4;

fn severity(dnbr: f64) -> &'static str {
    match dnbr {
        d if d < 0.10 => "unburned",
        d if d < 0.27 => "low",
        d if d < 0.44 => "moderate_low",
        d if d < 0.66 => "moderate_high",
        _ => "high",
    }
}

fn ladder(agreement: usize) -> &'static str {
    match agreement {
        0 => "no_burn",
        1 => "single_sensor_signal",
        2 => "likely_burn",
        _ => "high_confidence_burn",
    }
}

/// Blocks per side of the field's map: about `max_cells` blocks in all.
fn blocks_per_side(max_cells: Option<usize>) -> u32 {
    let n = max_cells.unwrap_or(16).clamp(1, 64) as f64;
    (n.sqrt().round() as u32).clamp(2, 8)
}

/// MCD64A1 burned at the centroid between two days: Burn_Date is a day of
/// year inside the month the fact covers, 0 unburned.
fn modis_burned(ba: &[Obs], from: i64, to: i64) -> Option<(bool, Vec<String>)> {
    let months: Vec<&Obs> = ba
        .iter()
        .filter(|o| o.unix <= to && o.unix + 31 * DAY >= from)
        .collect();
    if months.is_empty() {
        return None;
    }
    let burned = months.iter().any(|o| {
        let doy = o.value as i64;
        if doy <= 0 {
            return false;
        }
        let year_start = {
            let y = &day_str(o.unix)[..4];
            day_unix(&format!("{y}-01-01")).unwrap_or(o.unix)
        };
        let day = year_start + (doy - 1) * DAY;
        day >= from - DAY && day <= to + DAY
    });
    Some((burned, months.iter().map(|o| o.cid.clone()).collect()))
}

/// The field as the series engine takes it: its polygon, or a bbox (from a
/// `{bbox}`, a point's envelope, or the cells given).
fn series_area(req: &FieldReq) -> Result<(JsonValue, String, Option<f64>), ApiError> {
    if let Some(cells) = req.cells.as_ref().filter(|c| !c.is_empty()) {
        let (mut south, mut north) = (f64::MAX, f64::MIN);
        let (mut west, mut east) = (f64::MAX, f64::MIN);
        for c in cells {
            let b = emem_codec::latlng_from_cell64(c)
                .map_err(|e| bad_request(format!("cell {c}: {e}")))?
                .bbox_deg;
            south = south.min(b.min_lat);
            north = north.max(b.max_lat);
            west = west.min(b.min_lng);
            east = east.max(b.max_lng);
        }
        let centre = emem_codec::cell64_from_latlng((south + north) / 2.0, (west + east) / 2.0);
        let bbox = json!({"min_lat": south, "max_lat": north, "min_lng": west, "max_lng": east});
        return Ok((json!({"bbox": bbox}), centre, None));
    }
    let g = req
        .geometry_geojson
        .as_ref()
        .ok_or_else(|| bad_request("give `geometry_geojson` or `cells`"))?;
    let (bbox, poly, area_ha) = crate::extract_plot_geometry(g).map_err(bad_request)?;
    let (mn_la, mx_la, mn_ln, mx_ln) = bbox;
    let centre = emem_codec::cell64_from_latlng((mn_la + mx_la) / 2.0, (mn_ln + mx_ln) / 2.0);
    let area = if poly.is_some() {
        json!({"geometry": g})
    } else {
        json!({"bbox": {"min_lat": mn_la, "max_lat": mx_la, "min_lng": mn_ln, "max_lng": mx_ln}})
    };
    Ok((area, centre, Some(area_ha)))
}

/// The series request for one index over the field and window.
fn burn_series_req(
    area: &JsonValue,
    req: &FieldReq,
    index: &str,
    blocks: u32,
) -> Result<crate::field_series::FieldSeriesReq, ApiError> {
    let mut body = json!({
        "index": index,
        "start_date": req.start.trim(),
        "end_date": req.end.trim(),
        "max_scenes": BURN_MAX_SCENES,
        "min_support_px": BURN_MIN_SUPPORT_PX,
        "blocks": blocks,
    });
    if let (Some(o), Some(a)) = (body.as_object_mut(), area.as_object()) {
        o.extend(a.clone());
    }
    serde_json::from_value(body).map_err(|e| bad_request(format!("field series request: {e}")))
}

/// One clear scene of a field series: its day, its median over the field's
/// clear pixels, and each block's median.
#[derive(Debug, Clone)]
struct SceneRow {
    unix: i64,
    date: String,
    scene: String,
    median: f64,
    blocks: Vec<Option<f64>>,
}

fn scene_rows(series: &JsonValue) -> Vec<SceneRow> {
    let mut rows: Vec<SceneRow> = series["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            let date = r["date"].as_str()?.to_string();
            Some(SceneRow {
                unix: day_unix(&date)?,
                scene: r["scene"].as_str().unwrap_or_default().to_string(),
                median: r["median"].as_f64().filter(|v| v.is_finite())?,
                blocks: r["block_medians"]
                    .as_array()
                    .map(|b| b.iter().map(|v| v.as_f64()).collect())
                    .unwrap_or_default(),
                date,
            })
        })
        .collect();
    rows.sort_by_key(|r| r.unix);
    rows
}

fn near(rows: &[SceneRow], unix: i64) -> Option<&SceneRow> {
    rows.iter()
        .filter(|o| (o.unix - unix).abs() <= DAY)
        .min_by_key(|o| (o.unix - unix).abs())
}

/// One scope's reading of one pair of dates: NBR on both, the drops, and
/// how many of the sensors evaluated voted burn.
#[derive(Debug, Clone, Copy)]
struct Judged {
    pre: f64,
    post: f64,
    dnbr: f64,
    dndti: Option<f64>,
    agree: usize,
    evaluated: usize,
}

/// What the rule found over the two series.
#[derive(Debug, Default)]
struct BurnFindings {
    /// One per scope (the field, or a block) and pair whose NBR fell by the
    /// reporting floor or more.
    signals: Vec<JsonValue>,
    /// One per pair that passed the event test.
    events: Vec<JsonValue>,
    /// A sensor voted burn somewhere.
    any_vote: bool,
}

/// `residue_burn_multisensor@1` between consecutive clear NBR scenes, on the
/// field's median and on each block's median. NDTI is the scene within a
/// day of each NBR scene; MODIS is the centroid's monthly burned area.
fn burn_findings(nbr: &[SceneRow], ndti: &[SceneRow], ba: &[Obs]) -> BurnFindings {
    let mut out = BurnFindings::default();
    for w in nbr.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        if b.unix - a.unix > MAX_GAP_DAYS * DAY {
            continue;
        }
        let (ta, tb) = (near(ndti, a.unix), near(ndti, b.unix));
        let modis = modis_burned(ba, a.unix, b.unix);
        let judge =
            |pre: Option<f64>, post: Option<f64>, t_pre: Option<f64>, t_post: Option<f64>| {
                let (pre, post) = (pre?, post?);
                let dnbr = pre - post;
                let dndti = t_pre.zip(t_post).map(|(x, y)| x - y);
                let votes = [
                    Some(dnbr >= DNBR_BURN),
                    dndti.map(|d| d >= DNDTI_BURN),
                    modis.as_ref().map(|(m, _)| *m),
                ];
                Some(Judged {
                    pre,
                    post,
                    dnbr,
                    dndti,
                    agree: votes.iter().flatten().filter(|v| **v).count(),
                    evaluated: votes.iter().flatten().count(),
                })
            };
        let r4 = |x: f64| (x * 1e4).round() / 1e4;
        let mut signal = |scope: &str, block: Option<usize>, j: Judged| {
            if j.dnbr < DNBR_FLOOR {
                return;
            }
            out.any_vote |= j.agree > 0;
            out.signals.push(json!({
                "scope": scope,
                "block": block,
                "pre": {"date": a.date, "nbr": r4(j.pre), "scene": a.scene},
                "post": {"date": b.date, "nbr": r4(j.post), "scene": b.scene},
                "dnbr": r4(j.dnbr),
                "severity": severity(j.dnbr),
                "dndti": j.dndti.map(r4),
                "modis_burned": modis.as_ref().map(|(m, _)| *m),
                "sensors_agreeing": j.agree,
                "sensors_evaluated": j.evaluated,
                "verdict": ladder(j.agree),
            }));
        };
        let field = judge(
            Some(a.median),
            Some(b.median),
            ta.map(|t| t.median),
            tb.map(|t| t.median),
        );
        if let Some(j) = field {
            signal("field", None, j);
        }
        let (mut observed, mut strong, mut high) = (0usize, 0usize, 0usize);
        for k in 0..a.blocks.len().min(b.blocks.len()) {
            let block_of =
                |t: Option<&SceneRow>| t.and_then(|t| t.blocks.get(k).copied().flatten());
            let Some(j) = judge(a.blocks[k], b.blocks[k], block_of(ta), block_of(tb)) else {
                continue;
            };
            observed += 1;
            strong += usize::from(j.agree >= 2);
            high += usize::from(j.agree >= 3);
            signal("block", Some(k), j);
        }
        let field_agree = field.map_or(0, |j| j.agree);
        let need = 2usize.max((observed as f64 * 0.10).ceil() as usize);
        if field_agree >= 2 || strong >= need {
            out.events.push(json!({
                "pre_date": a.date,
                "post_date": b.date,
                "post_scene": b.scene,
                "field_dnbr": field.map(|j| r4(j.dnbr)),
                "field_dndti": field.and_then(|j| j.dndti).map(r4),
                "field_verdict": ladder(field_agree),
                "modis_burned": modis.as_ref().map(|(m, _)| *m),
                "blocks_with_burn_signature": strong,
                "blocks_observed": observed,
                "fraction": if observed > 0 { r4(strong as f64 / observed as f64) } else { 0.0 },
                "high_confidence_blocks": high,
            }));
        }
    }
    out
}

/// What one series call gave: its rows, and how to cite it or why it failed.
fn series_summary(res: &Result<JsonValue, ApiError>) -> JsonValue {
    match res {
        Ok(v) => json!({
            "token": v["token"],
            "derivation_cid": v["derivation_cid"],
            "clear_scenes": v["rows"].as_array().map_or(0, Vec::len),
            "excluded_scenes": v["excluded"].as_array().map_or(0, Vec::len),
        }),
        Err(e) => json!({"error": e.1.message}),
    }
}

pub async fn post_field_burn_scar(
    State(s): State<AppState>,
    EmemJson(req): EmemJson<FieldReq>,
) -> Result<Json<JsonValue>, ApiError> {
    let started = Instant::now();
    let win = window(&req)?;
    let (area, centre, area_ha) = series_area(&req)?;
    let blocks = blocks_per_side(req.max_cells);
    let budget = req.budget_ms.unwrap_or(25_000).clamp(1_000, 300_000);
    let nbr_req = burn_series_req(&area, &req, BURN_NBR_INDEX, blocks)?;
    let ndti_req = burn_series_req(&area, &req, BURN_NDTI_INDEX, blocks)?;

    let centre_v = vec![centre.clone()];
    // A monthly fact is stamped the first of its month: the month holding
    // the window's first day starts before the window does.
    let ba_win = (win.0 - 31 * DAY, win.1);
    let (nbr_res, ndti_res, w_ba) = tokio::join!(
        crate::field_series::field_series(nbr_req, &s),
        crate::field_series::field_series(ndti_req, &s),
        warm(&s, &centre_v, "modis.burned_area_monthly", ba_win, budget),
    );
    let ba = series(&s, &centre, "modis.burned_area_monthly", ba_win).await;

    let nbr = nbr_res.as_ref().map(scene_rows).unwrap_or_default();
    let ndti = ndti_res.as_ref().map(scene_rows).unwrap_or_default();
    let found = burn_findings(&nbr, &ndti, &ba);
    let verdict = if !found.events.is_empty() {
        "burn"
    } else if nbr.len() < 2 {
        "inconclusive"
    } else if found.any_vote {
        "signal_only"
    } else {
        "no_burn"
    };

    let mut cited: Vec<String> = ba.iter().map(|o| o.cid.clone()).collect();
    for res in [&nbr_res, &ndti_res] {
        if let Some(c) = res.as_ref().ok().and_then(|v| v["derivation_cid"].as_str()) {
            cited.push(c.to_string());
        }
    }
    cited.sort();
    cited.dedup();
    let receipt = s.sign_receipt(
        "emem.field_burn_scar",
        centre_v.clone(),
        cited.iter().cloned().map(FactCid::new).collect(),
        false,
        started,
        None,
    );
    let citation = emem_core::algorithms::DEFAULT
        .lookup("residue_burn_multisensor@1")
        .map(|a| a.citation.clone())
        .unwrap_or_default();
    let spec = |key: &str| {
        crate::field_series::s2_index_spec(key).map(|(a, b, formula, reference)| {
            json!({"bands": [a, b], "formula": formula, "reference": reference})
        })
    };
    let support = nbr_res
        .as_ref()
        .ok()
        .and_then(|v| v["estimand"]["support"].as_str())
        .map(str::to_string);
    Ok(Json(json!({
        "schema": "emem.field_burn_scar.v1",
        "signal": "field_burn_scar@1",
        "algorithm_key": "residue_burn_multisensor@1",
        "window": {"start": req.start, "end": req.end},
        "cells": centre_v,
        "centroid_cell": centre,
        "field_area_ha": area_ha,
        "verdict": verdict,
        "burn_events": found.events.len(),
        "events": found.events,
        "cell_signals": found.signals,
        "method": {
            "reads": "the whole field: every 20 m Sentinel-2 L2A pixel whose centre lies inside it, on each clear scene in the window, through the field series engine (s2_index_series@1), one signed series per index",
            "nbr": spec(BURN_NBR_INDEX),
            "ndti": spec(BURN_NDTI_INDEX),
            "statistic": "per scene, the median over the field's clear pixels, and the same median per block of the field's bbox, cut into blocks_per_side x blocks_per_side",
            "pairs": "consecutive clear NBR scenes at most max_gap_days apart, with the NDTI scene within a day of each",
            "not_read": "the stored indices.ndti band, which is the Lacaux turbidity index (B04 - B03) / (B04 + B03) and not the tillage index the rule cites, and the stored indices.nbr band, which pairs B08 (10 m) with B12 (20 m)",
            "blocks_per_side": blocks,
        },
        "support": support,
        "series": {"nbr": series_summary(&nbr_res), "ndti": series_summary(&ndti_res)},
        "observations": {
            "clear_nbr_dates": nbr.len(),
            "clear_ndti_dates": ndti.len(),
            "blocks_per_side": blocks,
            "warmed": {"modis.burned_area_monthly": w_ba},
        },
        "thresholds": {
            "dnbr_burn": DNBR_BURN, "dndti_burn": DNDTI_BURN, "dnbr_reported_from": DNBR_FLOOR,
            "max_gap_days": MAX_GAP_DAYS,
            "event_needs": "on one pair of dates, the field median at likely_burn or better, or max(2, 10 % of the blocks clear on both dates) at likely_burn or better",
        },
        "sensors_evaluated": ["s2_index_series@1 nbr", "s2_index_series@1 ndti", "modis.burned_area_monthly"],
        "sensors_not_evaluated": [
            {"band": "cams.aod_550", "why": "CAMS is a 0.4° (~40 km) model field: a smoke spike there says smoke was in the region, not that this field burned, so it cannot be a per-field vote"},
            {"band": "cams.pm25", "why": "the same 0.4° field; useful for the regional air-quality story, not for attributing a fire to a field"},
        ],
        "reading": "An NBR drop alone is also what harvest looks like; the NDTI drop is what separates burning from tillage or mulch, and MCD64A1 is independent but 500 m and monthly. `single_sensor_signal` is triage, not evidence. `inconclusive` means fewer than two clear Sentinel-2 scenes over the field in the window, or a series that could not be read (see `series`): widen the window or retry.",
        "citation": citation,
        "receipt": receipt,
    })))
}

// ─────────────────────────── field_actual_et@1 ───────────────────────────

/// MOD16A2 composites start on day 1, 9, 17, ... of each year and the
/// last one runs to 31 December, so it is 5 or 6 days long.
fn composite_days(start_unix: i64) -> i64 {
    let y: i32 = day_str(start_unix)[..4].parse().unwrap_or(1970);
    let next_year = day_unix(&format!("{:04}-01-01", y + 1)).unwrap_or(start_unix + 8 * DAY);
    ((next_year - start_unix) / DAY).clamp(1, 8)
}

pub async fn post_field_actual_et(
    State(s): State<AppState>,
    EmemJson(req): EmemJson<FieldReq>,
) -> Result<Json<JsonValue>, ApiError> {
    let started = Instant::now();
    let win = window(&req)?;
    let (_, centre, area_ha) = field_cells(&req, 1)?;
    let budget = req.budget_ms.unwrap_or(25_000).clamp(1_000, 300_000);
    // A composite that starts up to 7 days before the window still covers its first days.
    let reach = (win.0 - 7 * DAY, win.1);
    let warmed = warm(
        &s,
        std::slice::from_ref(&centre),
        "modis.et_8day",
        reach,
        budget,
    )
    .await;
    let comps = series(&s, &centre, "modis.et_8day", reach).await;

    let mut et_mm = 0.0;
    let mut covered_days = 0i64;
    let mut used = Vec::new();
    for c in &comps {
        let len = composite_days(c.unix);
        let (a, b) = (c.unix.max(win.0), (c.unix + len * DAY).min(win.1));
        if b <= a {
            continue;
        }
        let days = (b - a) / DAY;
        let share = days as f64 / len as f64;
        et_mm += c.value * share;
        covered_days += days;
        used.push(json!({
            "composite_start": c.captured_at.clone().unwrap_or_else(|| day_str(c.unix)),
            "composite_days": len,
            "et_mm": c.value,
            "days_in_window": days,
            "fact_cid": c.cid,
        }));
    }
    let window_days = (win.1 - win.0) / DAY;
    let coverage = covered_days as f64 / window_days as f64;
    let verdict = if used.is_empty() {
        "no_valid_composite"
    } else if coverage < 0.9 {
        "partial"
    } else {
        "complete"
    };
    let cited: Vec<FactCid> = used
        .iter()
        .filter_map(|u| u["fact_cid"].as_str().map(|c| FactCid::new(c.to_string())))
        .collect();
    let receipt = s.sign_receipt(
        "emem.field_actual_et",
        vec![centre.clone()],
        cited,
        false,
        started,
        None,
    );
    let r2 = |x: f64| (x * 100.0).round() / 100.0;
    Ok(Json(json!({
        "schema": "emem.field_actual_et.v1",
        "signal": "field_actual_et@1",
        "band": "modis.et_8day",
        "product": "MOD16A2 ET_500m (actual evapotranspiration, kg/m² = mm per composite)",
        "window": {"start": req.start, "end": req.end, "days": window_days},
        "cell": centre,
        "field_area_ha": area_ha,
        "pixel_m": 463,
        "verdict": verdict,
        "et_mm": if used.is_empty() { JsonValue::Null } else { json!(r2(et_mm)) },
        "et_m3_per_ha": if used.is_empty() { JsonValue::Null } else { json!(r2(et_mm * 10.0)) },
        "coverage": (coverage * 1e4).round() / 1e4,
        "composites": used,
        "warmed": warmed,
        "reading": "Water the field's vegetation evaporated and transpired, from MODIS at one 463 m pixel over the field's centre: a field smaller than the pixel shares it with its neighbours. It is not irrigation applied. `partial` sums only the days composites cover and does not extrapolate; `no_valid_composite` is what MOD16 gives urban, water and barren pixels, and dates it has not published.",
        "citation": "Running, S., Mu, Q., Zhao, M., Moreno, A. (2021). MODIS/Terra Net Evapotranspiration 8-Day L4 Global 500m SIN Grid V061. NASA EOSDIS LP DAAC. doi:10.5067/MODIS/MOD16A2.061",
        "receipt": receipt,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_composite_of_a_year_is_short() {
        assert_eq!(composite_days(day_unix("2024-12-26").unwrap()), 6);
        assert_eq!(composite_days(day_unix("2025-12-27").unwrap()), 5);
        assert_eq!(composite_days(day_unix("2025-06-10").unwrap()), 8);
    }

    #[test]
    fn severity_and_agreement_follow_the_registered_ladders() {
        assert_eq!(severity(0.05), "unburned");
        assert_eq!(severity(0.30), "moderate_low");
        assert_eq!(severity(0.70), "high");
        assert_eq!(ladder(0), "no_burn");
        assert_eq!(ladder(2), "likely_burn");
        assert_eq!(ladder(3), "high_confidence_burn");
    }

    fn row(date: &str, median: f64, blocks: &[Option<f64>]) -> SceneRow {
        SceneRow {
            unix: day_unix(date).unwrap(),
            date: date.to_string(),
            scene: format!("S2B_{date}"),
            median,
            blocks: blocks.to_vec(),
        }
    }

    /// A stubble fire on the field's west half: NBR falls on every block,
    /// and the tillage NDTI falls only where the residue burned. Ploughing
    /// the east half lowers NBR there too, but not NDTI, so those blocks
    /// stay single-sensor signals and the event counts the west.
    #[test]
    fn the_rule_separates_a_burn_from_tillage_by_the_ndti_drop() {
        let nbr = vec![
            row(
                "2025-10-20",
                0.40,
                &[Some(0.40), Some(0.41), Some(0.40), Some(0.41)],
            ),
            row(
                "2025-10-30",
                0.16,
                &[Some(0.12), Some(0.18), Some(0.13), Some(0.17)],
            ),
        ];
        let ndti = vec![
            row(
                "2025-10-20",
                0.18,
                &[Some(0.20), Some(0.17), Some(0.20), Some(0.17)],
            ),
            row(
                "2025-10-30",
                0.12,
                &[Some(0.05), Some(0.16), Some(0.06), Some(0.16)],
            ),
        ];
        let f = burn_findings(&nbr, &ndti, &[]);
        assert_eq!(f.events.len(), 1, "{:?}", f.events);
        let e = &f.events[0];
        assert_eq!(e["blocks_with_burn_signature"], 2);
        assert_eq!(e["blocks_observed"], 4);
        assert_eq!(e["post_date"], "2025-10-30");
        let block = |k: usize| {
            f.signals
                .iter()
                .find(|s| s["scope"] == "block" && s["block"] == k)
                .unwrap()
        };
        assert_eq!(block(0)["verdict"], "likely_burn");
        assert_eq!(block(1)["verdict"], "single_sensor_signal");
        assert!(f.any_vote);

        // Tillage alone: NBR falls, NDTI holds. No event.
        let ndti_held = vec![
            row("2025-10-20", 0.18, &[Some(0.18); 4]),
            row("2025-10-30", 0.17, &[Some(0.17); 4]),
        ];
        let t = burn_findings(&nbr, &ndti_held, &[]);
        assert!(t.events.is_empty(), "{:?}", t.events);
        assert!(t.any_vote, "the NBR drop is still a signal");
    }

    /// The rule reads the van Deventer tillage index on B11 and B12, and NBR
    /// on B8A and B12, from the engine that reads the field: not the stored
    /// `indices.ndti` band, which is the Lacaux turbidity index of B04 and
    /// B03 under the same name.
    #[test]
    fn the_burn_rule_reads_the_tillage_ndti() {
        let (a, b, formula, reference) =
            crate::field_series::s2_index_spec(BURN_NDTI_INDEX).unwrap();
        assert_eq!((a, b), ("s2.B11", "s2.B12"));
        assert_eq!(formula, "(B11 - B12) / (B11 + B12)");
        assert!(reference.contains("Van Deventer"), "{reference}");
        let (a, b, _, _) = crate::field_series::s2_index_spec(BURN_NBR_INDEX).unwrap();
        assert_eq!((a, b), ("s2.B8A", "s2.B12"));
        let req = FieldReq {
            geometry_geojson: Some(
                json!({"type": "Polygon", "coordinates": [[[75.80, 30.90], [75.804, 30.90], [75.804, 30.903], [75.80, 30.903], [75.80, 30.90]]]}),
            ),
            cells: None,
            start: "2025-10-01".into(),
            end: "2025-11-30".into(),
            max_cells: None,
            budget_ms: None,
        };
        let (area, _, _) = series_area(&req).unwrap();
        assert!(area.get("geometry").is_some(), "the polygon itself is read");
        let fs = burn_series_req(&area, &req, BURN_NDTI_INDEX, 4).unwrap();
        assert_eq!(fs.index.as_deref(), Some("ndti"));
        assert_eq!(fs.sensor, None, "Sentinel-2, the engine's default");
        assert_eq!(fs.blocks, Some(4));
    }

    #[test]
    fn max_cells_sets_about_that_many_blocks() {
        assert_eq!(blocks_per_side(None), 4);
        assert_eq!(blocks_per_side(Some(64)), 8);
        assert_eq!(blocks_per_side(Some(1)), 2);
        assert_eq!(blocks_per_side(Some(36)), 6);
    }

    #[test]
    fn modis_burn_date_counts_only_between_the_two_dates() {
        let o = |d: &str, v: f64| Obs {
            unix: day_unix(d).unwrap(),
            value: v,
            captured_at: None,
            cid: format!("c{d}"),
        };
        // October 2025, burned on day 290 (17 October).
        let ba = vec![o("2025-10-01", 290.0)];
        let (a, b) = (
            day_unix("2025-10-12").unwrap(),
            day_unix("2025-10-22").unwrap(),
        );
        assert_eq!(modis_burned(&ba, a, b).map(|x| x.0), Some(true));
        let (a, b) = (
            day_unix("2025-10-01").unwrap(),
            day_unix("2025-10-08").unwrap(),
        );
        assert_eq!(modis_burned(&ba, a, b).map(|x| x.0), Some(false));
        assert!(modis_burned(&[], a, b).is_none());
    }
}
