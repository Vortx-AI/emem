//! Field-level signals a compliance pack asks for by name, composed from
//! signed per-cell facts so every number cites the observations it came
//! from:
//!
//! - `field_burn_scar@1`: residue burning on a field over a window, by the
//!   registered `residue_burn_multisensor@1` rule (NBR drop, NDTI drop,
//!   MODIS burned area) applied between consecutive clear Sentinel-2 dates.
//! - `field_actual_et@1`: actual evapotranspiration over a window, the
//!   MOD16A2 8-day composites (`modis.et_8day`) pro-rated to its edges.
//!
//! Both warm what they read through the backfill preparer first, so a
//! field nobody asked about before answers on the first call, partially if
//! the budget runs out, and say what they could not evaluate.

use std::collections::BTreeMap;
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
use emem_primitives::trajectory::{trajectory, TrajectoryReq};

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
    /// Cells sampled inside the geometry (burn scar only; default 16, at most 64).
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
    let Ok(tr) = trajectory(&req, s).await else {
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

fn near(obs: &[Obs], unix: i64) -> Option<&Obs> {
    obs.iter()
        .filter(|o| (o.unix - unix).abs() <= DAY)
        .min_by_key(|o| (o.unix - unix).abs())
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

pub async fn post_field_burn_scar(
    State(s): State<AppState>,
    EmemJson(req): EmemJson<FieldReq>,
) -> Result<Json<JsonValue>, ApiError> {
    let started = Instant::now();
    let win = window(&req)?;
    let n = req.max_cells.unwrap_or(16).clamp(1, 64);
    let (cells, centre, area_ha) = field_cells(&req, n)?;
    let budget = req.budget_ms.unwrap_or(25_000).clamp(1_000, 300_000);

    let centre_v = vec![centre.clone()];
    let (w_nbr, w_ndti, w_ba) = tokio::join!(
        warm(&s, &cells, "indices.nbr", win, budget),
        warm(&s, &cells, "indices.ndti", win, budget),
        warm(&s, &centre_v, "modis.burned_area_monthly", win, budget),
    );
    let ba = series(&s, &centre, "modis.burned_area_monthly", win).await;

    let mut cited: Vec<String> = ba.iter().map(|o| o.cid.clone()).collect();
    let mut events = Vec::new();
    let mut clear_obs = Vec::new();
    // day -> cells with a clear NBR that day, for the per-date denominator.
    let mut observed_on: BTreeMap<i64, usize> = BTreeMap::new();
    for cell in &cells {
        let nbr = series(&s, cell, "indices.nbr", win).await;
        let ndti = series(&s, cell, "indices.ndti", win).await;
        clear_obs.push(nbr.len());
        for o in &nbr {
            *observed_on.entry(o.unix / DAY).or_default() += 1;
        }
        cited.extend(nbr.iter().map(|o| o.cid.clone()));
        cited.extend(ndti.iter().map(|o| o.cid.clone()));
        for w in nbr.windows(2) {
            let (a, b) = (&w[0], &w[1]);
            if b.unix - a.unix > MAX_GAP_DAYS * DAY {
                continue;
            }
            let dnbr = a.value - b.value;
            if dnbr < DNBR_FLOOR {
                continue;
            }
            let dndti = match (near(&ndti, a.unix), near(&ndti, b.unix)) {
                (Some(x), Some(y)) => Some((x.value - y.value, [x.cid.clone(), y.cid.clone()])),
                _ => None,
            };
            let modis = modis_burned(&ba, a.unix, b.unix);
            let signals = [
                Some(dnbr >= DNBR_BURN),
                dndti.as_ref().map(|(d, _)| *d >= DNDTI_BURN),
                modis.as_ref().map(|(m, _)| *m),
            ];
            let agreement = signals.iter().flatten().filter(|v| **v).count();
            events.push(json!({
                "cell": cell,
                "pre": {"date": a.captured_at.clone().unwrap_or_else(|| day_str(a.unix)), "nbr": a.value, "fact_cid": a.cid},
                "post": {"date": b.captured_at.clone().unwrap_or_else(|| day_str(b.unix)), "nbr": b.value, "fact_cid": b.cid},
                "dnbr": (dnbr * 1e4).round() / 1e4,
                "severity": severity(dnbr),
                "dndti": dndti.as_ref().map(|(d, _)| (d * 1e4).round() / 1e4),
                "ndti_fact_cids": dndti.as_ref().map(|(_, c)| c.clone()),
                "modis_burned": modis.as_ref().map(|(m, _)| *m),
                "sensors_agreeing": agreement,
                "sensors_evaluated": signals.iter().flatten().count(),
                "verdict": ladder(agreement),
                "_post_day": b.unix / DAY,
            }));
        }
    }

    // One fire is one post-burn acquisition seen by several cells; a lone
    // cell is a signal, not an event.
    let mut by_day: BTreeMap<i64, Vec<&JsonValue>> = BTreeMap::new();
    for e in &events {
        by_day
            .entry(e["_post_day"].as_i64().unwrap_or(0))
            .or_default()
            .push(e);
    }
    let mut burn_events = Vec::new();
    for (day, evs) in &by_day {
        let observed = observed_on
            .get(day)
            .copied()
            .unwrap_or(evs.len())
            .max(evs.len());
        let strong = evs
            .iter()
            .filter(|e| {
                matches!(
                    e["verdict"].as_str(),
                    Some("likely_burn" | "high_confidence_burn")
                )
            })
            .count();
        let need = 2usize.max((observed as f64 * 0.10).ceil() as usize);
        if strong >= need {
            burn_events.push(json!({
                "post_date": day_str(day * DAY),
                "cells_with_burn_signature": strong,
                "cells_observed": observed,
                "fraction": ((strong as f64 / observed as f64) * 1e4).round() / 1e4,
                "high_confidence_cells": evs.iter().filter(|e| e["verdict"] == "high_confidence_burn").count(),
            }));
        }
    }
    let mut sorted = clear_obs.clone();
    sorted.sort_unstable();
    let median_obs = sorted.get(sorted.len() / 2).copied().unwrap_or(0);
    let verdict = if !burn_events.is_empty() {
        "burn"
    } else if median_obs < 2 {
        "inconclusive"
    } else if events
        .iter()
        .any(|e| e["sensors_agreeing"].as_u64().unwrap_or(0) > 0)
    {
        "signal_only"
    } else {
        "no_burn"
    };
    for e in events.iter_mut() {
        if let Some(o) = e.as_object_mut() {
            o.remove("_post_day");
        }
    }
    cited.sort();
    cited.dedup();
    let receipt = s.sign_receipt(
        "emem.field_burn_scar",
        cells.clone(),
        cited.iter().cloned().map(FactCid::new).collect(),
        false,
        started,
        None,
    );
    let citation = emem_core::algorithms::DEFAULT
        .lookup("residue_burn_multisensor@1")
        .map(|a| a.citation.clone())
        .unwrap_or_default();
    Ok(Json(json!({
        "schema": "emem.field_burn_scar.v1",
        "signal": "field_burn_scar@1",
        "algorithm_key": "residue_burn_multisensor@1",
        "window": {"start": req.start, "end": req.end},
        "cells": cells,
        "centroid_cell": centre,
        "field_area_ha": area_ha,
        "verdict": verdict,
        "burn_events": burn_events.len(),
        "events": burn_events,
        "cell_signals": events,
        "observations": {
            "clear_nbr_dates_per_cell_median": median_obs,
            "cells": cells.len(),
            "warmed": {"indices.nbr": w_nbr, "indices.ndti": w_ndti, "modis.burned_area_monthly": w_ba},
        },
        "thresholds": {
            "dnbr_burn": DNBR_BURN, "dndti_burn": DNDTI_BURN, "dnbr_reported_from": DNBR_FLOOR,
            "max_gap_days": MAX_GAP_DAYS,
            "event_needs_cells": "max(2, 10 % of the cells clear on the post-burn date) at likely_burn or better",
        },
        "sensors_evaluated": ["indices.nbr", "indices.ndti", "modis.burned_area_monthly"],
        "sensors_not_evaluated": [
            {"band": "cams.aod_550", "why": "the algorithm's AOD spike needs a per-field seasonal baseline, not built yet"},
            {"band": "cams.pm25", "why": "same: needs a baseline"},
        ],
        "reading": "An NBR drop alone is also what harvest looks like; the NDTI drop is what separates burning from tillage or mulch, and MCD64A1 is independent but 500 m and monthly. `single_sensor_signal` is triage, not evidence. `inconclusive` means fewer than two clear Sentinel-2 dates per cell in the window: widen it or retry after the warm-up converges.",
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
