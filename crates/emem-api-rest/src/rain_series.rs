//! Daily rain at a place: the recent past, the next two weeks, and how each
//! total ranks against the same days of the last thirty years.
//!
//! Flood, drought and landslide outlooks all turn on rain, and until this
//! route emem had none it could put in a series: `weather.precipitation_mm`
//! is the current hour only, `era5.precip` is one signed fact per hour, and
//! CHIRPS final data lags a month. One call here returns, as one signed
//! derivation:
//!
//! - the past `past_days` of daily totals, from ERA5 reanalysis where it has
//!   arrived (about five days behind) and from the forecast model's own
//!   analysis for the days ERA5 has not reached, each day tagged with which;
//! - the next `forecast_days` of daily totals from the forecast model, each
//!   with its lead time;
//! - totals over 1 to 90 days back and 1 to 14 days ahead, each ranked
//!   against every total of the same length ending (or, ahead, starting)
//!   within a week of the same day of year in the climatology years, as a
//!   non-exceedance probability and its standard-normal score. Over a 90-day
//!   window that score is the standardized precipitation index computed
//!   without a fitted distribution (Farahmand & AghaKouchak 2015).

use std::time::Instant;

use axum::extract::State;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value as JsonValue};

use emem_fact::{Derivation, DerivativeFact, Fact, FactCid};

use crate::band_raster::{bad_request, upstream_error};
use crate::change_attribution::json_to_cbor;
use crate::{ApiError, AppState, EmemJson};

const FN_KEY: &str = "rain_series@1";
const PAST_WINDOWS: [u32; 6] = [1, 3, 7, 14, 30, 90];
const AHEAD_WINDOWS: [u32; 4] = [1, 3, 7, 14];
/// Days either side of the day of year whose totals join the climatology:
/// 15 totals a year instead of one, so 30 years give 450 to rank against.
const SEASON_HALF_WIDTH_DAYS: i64 = 7;
const MAX_PAST_DAYS: u32 = 92;
const MAX_FORECAST_DAYS: u32 = 16;
const MAX_CLIMATOLOGY_YEARS: u32 = 40;
/// The forecast model's past days only fill what ERA5 has not reached yet.
const ANALYSIS_DAYS: u32 = 10;

#[derive(Debug, Deserialize)]
pub struct RainSeriesReq {
    #[serde(default)]
    pub lat: Option<f64>,
    #[serde(default, alias = "lon")]
    pub lng: Option<f64>,
    /// A cell64 in place of lat/lng.
    #[serde(default, alias = "cell64")]
    pub cell: Option<String>,
    /// Days back, 1..=92. Default 30.
    #[serde(default)]
    pub past_days: Option<u32>,
    /// Days ahead, 1..=16. Default 7.
    #[serde(default)]
    pub forecast_days: Option<u32>,
    /// Complete years before this one to rank against, 0..=40. Default 30; 0
    /// skips the ranking.
    #[serde(default)]
    pub climatology_years: Option<u32>,
}

/// One day of the answer: `(day number, millimetres, source, lead days)`.
type Day = (i64, Option<f64>, &'static str, Option<i64>);

fn iso(day: i64) -> String {
    let (y, m, d) = crate::civil_from_days(day);
    format!("{y:04}-{m:02}-{d:02}")
}

fn day_of(iso: &str) -> Option<i64> {
    let (y, m, d) = crate::band_raster::parse_ymd(iso)?;
    Some(crate::days_from_civil(y, m, d))
}

/// `{"daily": {"time": [...], "precipitation_sum": [...]}}` as day -> mm.
fn daily_values(body: &JsonValue) -> Result<Vec<(i64, Option<f64>)>, String> {
    let times = body
        .pointer("/daily/time")
        .and_then(|v| v.as_array())
        .ok_or("the answer has no daily.time")?;
    let vals = body
        .pointer("/daily/precipitation_sum")
        .and_then(|v| v.as_array())
        .ok_or("the answer has no daily.precipitation_sum")?;
    Ok(times
        .iter()
        .zip(vals)
        .filter_map(|(t, v)| Some((day_of(t.as_str()?)?, v.as_f64())))
        .collect())
}

/// Total over `len` days starting at `first`, or None when any day is missing.
fn total(by_day: &std::collections::HashMap<i64, f64>, first: i64, len: u32) -> Option<f64> {
    (first..first + len as i64)
        .map(|d| by_day.get(&d).copied())
        .sum()
}

/// Every climatology year's totals of `len` days whose anchor day falls within
/// a week of `anchor`'s day of year, the anchor being the window's last day
/// (`ending`) or first day (ahead).
fn season_totals(
    clim: &std::collections::HashMap<i64, f64>,
    years: (i32, i32),
    anchor: i64,
    len: u32,
    ending: bool,
) -> Vec<f64> {
    let (_, m, d) = crate::civil_from_days(anchor);
    // 29 February anchors to the 28th in years without it.
    let d = if m == 2 && d == 29 { 28 } else { d };
    let mut out = Vec::new();
    for y in years.0..=years.1 {
        let centre = crate::days_from_civil(y, m, d);
        for k in -SEASON_HALF_WIDTH_DAYS..=SEASON_HALF_WIDTH_DAYS {
            let a = centre + k;
            let first = if ending { a - len as i64 + 1 } else { a };
            if let Some(t) = total(clim, first, len) {
                out.push(t);
            }
        }
    }
    out
}

/// Non-exceedance probability of `x` among `sample`, with ties split and the
/// Weibull (i/(n+1)) offset, so it never reaches 0 or 1.
fn rank_probability(x: f64, sample: &[f64]) -> Option<f64> {
    if sample.is_empty() {
        return None;
    }
    let below = sample.iter().filter(|v| **v < x - 1e-9).count() as f64;
    let equal = sample.iter().filter(|v| (**v - x).abs() <= 1e-9).count() as f64;
    Some((below + 0.5 * equal + 0.5) / (sample.len() as f64 + 1.0))
}

/// Inverse of the standard normal CDF (Acklam's rational approximation,
/// relative error under 1.2e-9).
fn probit(p: f64) -> f64 {
    const A: [f64; 6] = [
        -3.969683028665376e1,
        2.209460984245205e2,
        -2.759285104469687e2,
        1.38357751867269e2,
        -3.066479806614716e1,
        2.506628277459239,
    ];
    const B: [f64; 5] = [
        -5.447609879822406e1,
        1.615858368580409e2,
        -1.556989798598866e2,
        6.680131188771972e1,
        -1.328068155288572e1,
    ];
    const C: [f64; 6] = [
        -7.784894002430293e-3,
        -3.223964580411365e-1,
        -2.400758277161838,
        -2.549732539343734,
        4.374664141464968,
        2.938163982698783,
    ];
    const D: [f64; 4] = [
        7.784695709041462e-3,
        3.224671290700398e-1,
        2.445134137142996,
        3.754408661907416,
    ];
    let p = p.clamp(1e-12, 1.0 - 1e-12);
    let tail = |q: f64| {
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    };
    if p < 0.02425 {
        tail((-2.0 * p.ln()).sqrt())
    } else if p > 1.0 - 0.02425 {
        -tail((-2.0 * (1.0 - p).ln()).sqrt())
    } else {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
            / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    }
}

/// McKee et al. (1993) classes for a standardized precipitation score.
fn class_of(z: f64) -> &'static str {
    match z {
        z if z <= -2.0 => "extremely dry",
        z if z <= -1.5 => "severely dry",
        z if z <= -1.0 => "moderately dry",
        z if z < 1.0 => "near normal",
        z if z < 1.5 => "moderately wet",
        z if z < 2.0 => "very wet",
        _ => "extremely wet",
    }
}

fn ranked(x: f64, sample: &[f64]) -> JsonValue {
    match rank_probability(x, sample) {
        Some(p) => {
            let z = probit(p);
            json!({"compared_with": sample.len(), "non_exceedance": (p * 1e4).round() / 1e4,
                   "z": (z * 1e3).round() / 1e3, "class": class_of(z)})
        }
        None => JsonValue::Null,
    }
}

/// Past days in order: ERA5 where it has a value, the forecast model's
/// analysis where it does not, missing otherwise.
fn merge_past(
    first: i64,
    last: i64,
    era5: &std::collections::HashMap<i64, f64>,
    analysis: &std::collections::HashMap<i64, f64>,
) -> Vec<Day> {
    (first..=last)
        .map(|d| match (era5.get(&d), analysis.get(&d)) {
            (Some(v), _) => (d, Some(*v), "era5", None),
            (None, Some(v)) => (d, Some(*v), "model_analysis", None),
            (None, None) => (d, None, "missing", None),
        })
        .collect()
}

async fn get_json(url: &str) -> Result<JsonValue, String> {
    let resp = crate::reqwest_client()
        .get(url)
        .header("user-agent", emem_core::outbound::user_agent())
        .send()
        .await
        .map_err(|e| format!("{e}"))?;
    if !resp.status().is_success() {
        return Err(format!("status {}", resp.status()));
    }
    resp.json().await.map_err(|e| format!("json: {e}"))
}

pub async fn post_rain_series(
    State(s): State<AppState>,
    EmemJson(req): EmemJson<RainSeriesReq>,
) -> Result<Json<JsonValue>, ApiError> {
    Ok(Json(rain_series(req, &s).await?))
}

pub(crate) async fn rain_series(req: RainSeriesReq, s: &AppState) -> Result<JsonValue, ApiError> {
    let started = Instant::now();
    let (lat, lng) = match (req.lat, req.lng, req.cell.as_deref()) {
        (Some(lat), Some(lng), _) => (lat, lng),
        (_, _, Some(c)) => {
            let ll = emem_codec::latlng_from_cell64(c)
                .map_err(|e| bad_request(format!("cell `{c}` does not decode: {e}")))?;
            (ll.lat_deg, ll.lng_deg)
        }
        _ => return Err(bad_request("give `lat` and `lng`, or a `cell`".into())),
    };
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lng) {
        return Err(bad_request(format!("({lat}, {lng}) is not a WGS-84 point")));
    }
    let past_days = req.past_days.unwrap_or(30);
    let forecast_days = req.forecast_days.unwrap_or(7);
    let years = req.climatology_years.unwrap_or(30);
    if !(1..=MAX_PAST_DAYS).contains(&past_days)
        || !(1..=MAX_FORECAST_DAYS).contains(&forecast_days)
        || years > MAX_CLIMATOLOGY_YEARS
    {
        return Err(bad_request(format!(
            "past_days takes 1 to {MAX_PAST_DAYS}, forecast_days 1 to {MAX_FORECAST_DAYS}, \
             climatology_years 0 to {MAX_CLIMATOLOGY_YEARS}"
        )));
    }
    let cell = emem_codec::geo::cell64_from_latlng(lat, lng);
    let today = crate::now_unix() as i64 / 86_400;
    let (this_year, _, _) = crate::civil_from_days(today);
    let first_past = today - past_days as i64;
    let yesterday = today - 1;

    let forecast_url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={lat:.4}&longitude={lng:.4}\
         &daily=precipitation_sum&past_days={}&forecast_days={forecast_days}&timezone=UTC",
        ANALYSIS_DAYS.min(past_days)
    );
    // ERA5 from the first climatology year (or the first past day) to yesterday,
    // in one request: about 11,000 values for 30 years.
    let clim_years = (this_year - years as i32, this_year - 1);
    let era5_from = if years > 0 {
        crate::days_from_civil(clim_years.0, 1, 1).min(first_past)
    } else {
        first_past
    };
    let archive_url = format!(
        "https://archive-api.open-meteo.com/v1/archive?latitude={lat:.4}&longitude={lng:.4}\
         &start_date={}&end_date={}&daily=precipitation_sum&timezone=UTC&models=era5",
        iso(era5_from),
        iso(yesterday)
    );
    let (fc, ar) = tokio::join!(get_json(&forecast_url), get_json(&archive_url));
    let fc = daily_values(&fc.map_err(|e| upstream_error(format!("rain forecast: {e}")))?)
        .map_err(|e| upstream_error(format!("rain forecast: {e}")))?;
    let ar = daily_values(&ar.map_err(|e| upstream_error(format!("ERA5 archive: {e}")))?)
        .map_err(|e| upstream_error(format!("ERA5 archive: {e}")))?;

    let era5: std::collections::HashMap<i64, f64> =
        ar.iter().filter_map(|(d, v)| Some((*d, (*v)?))).collect();
    let model: std::collections::HashMap<i64, f64> =
        fc.iter().filter_map(|(d, v)| Some((*d, (*v)?))).collect();
    let analysis: std::collections::HashMap<i64, f64> = model
        .iter()
        .filter(|(d, _)| **d < today)
        .map(|(d, v)| (*d, *v))
        .collect();
    let past = merge_past(first_past, yesterday, &era5, &analysis);
    let ahead: Vec<Day> = (today..today + forecast_days as i64)
        .map(|d| (d, model.get(&d).copied(), "forecast", Some(d - today)))
        .collect();
    let all_past: std::collections::HashMap<i64, f64> = past
        .iter()
        .filter_map(|(d, v, _, _)| Some((*d, (*v)?)))
        .collect();
    let clim: std::collections::HashMap<i64, f64> = era5
        .iter()
        .filter(|(d, _)| {
            let (y, _, _) = crate::civil_from_days(**d);
            y >= clim_years.0 && y <= clim_years.1
        })
        .map(|(d, v)| (*d, *v))
        .collect();

    let round = |v: f64| (v * 10.0).round() / 10.0;
    let mut past_totals = Vec::new();
    for w in PAST_WINDOWS.into_iter().filter(|w| *w <= past_days) {
        let first = yesterday - w as i64 + 1;
        if let Some(t) = total(&all_past, first, w) {
            let sample = if years > 0 {
                season_totals(&clim, clim_years, yesterday, w, true)
            } else {
                vec![]
            };
            past_totals.push(json!({"days": w, "from": iso(first), "to": iso(yesterday),
                                    "mm": round(t), "climatology": ranked(t, &sample)}));
        }
    }
    let ahead_map: std::collections::HashMap<i64, f64> = ahead
        .iter()
        .filter_map(|(d, v, _, _)| Some((*d, (*v)?)))
        .collect();
    let mut ahead_totals = Vec::new();
    for w in AHEAD_WINDOWS.into_iter().filter(|w| *w <= forecast_days) {
        if let Some(t) = total(&ahead_map, today, w) {
            let sample = if years > 0 {
                season_totals(&clim, clim_years, today, w, false)
            } else {
                vec![]
            };
            ahead_totals.push(
                json!({"days": w, "from": iso(today), "to": iso(today + w as i64 - 1),
                                     "mm": round(t), "climatology": ranked(t, &sample)}),
            );
        }
    }
    let days: Vec<JsonValue> = past
        .iter()
        .chain(ahead.iter())
        .map(|(d, v, src, lead)| {
            let mut o = json!({"date": iso(*d), "mm": v.map(round), "source": src});
            if let Some(l) = lead {
                o["lead_days"] = json!(l);
            }
            o
        })
        .collect();
    let gaps = past.iter().filter(|(_, v, _, _)| v.is_none()).count();
    let mut cautions = vec![
        "The days ahead are a weather model's forecast, issued when this record was signed; \
         past days are reanalysis (ERA5) or that model's own analysis, not a rain gauge."
            .to_string(),
        "The ranking compares totals with ERA5's at the same place. The forecast model and ERA5 \
         are different models on different grids, and ERA5 is known to under-read short \
         convective bursts, so a forecast total ranks high a little more easily than an \
         observed one would."
            .to_string(),
    ];
    if gaps > 0 {
        cautions.push(format!(
            "{gaps} past day(s) had no value from either source, and totals that span them are left out."
        ));
    }
    let record = json!({
        "schema": "emem.rain_series.v1",
        "fn_key": FN_KEY,
        "location": {"lat": lat, "lng": lng, "cell": cell},
        "issued": iso(today),
        "days": days,
        "past_totals": past_totals,
        "forecast_totals": ahead_totals,
        "climatology": if years > 0 {
            json!({"years": [clim_years.0, clim_years.1], "source": "ERA5 daily totals",
                   "rule": format!("every total of the same length ending (past) or starting (ahead) within {SEASON_HALF_WIDTH_DAYS} days of the same day of year, each year; non-exceedance by rank, ties split, Weibull offset; z is its standard-normal quantile; classes after McKee et al. 1993")})
        } else {
            JsonValue::Null
        },
        "estimand": {"quantity": "daily precipitation total at the point, UTC days",
                     "unit": "mm", "statistic": "sum over UTC days"},
        "sources": [
            {"name": "Open-Meteo forecast API (best-match weather model for the location)",
             "url": forecast_url, "licence": "CC BY 4.0", "role": "days ahead, and past days ERA5 has not reached"},
            {"name": "Open-Meteo historical weather API, ERA5 (Hersbach et al. 2020)",
             "url": archive_url, "licence": "CC BY 4.0", "role": "past days and the climatology"},
        ],
        "cautions": cautions,
    });
    let signed_at = emem_storage::server::iso8601_now();
    let fact = DerivativeFact {
        cell: cell.clone(),
        band: "rain.series".to_string(),
        tslot_window: [
            first_past.max(0) as u64,
            (today + forecast_days as i64 - 1).max(0) as u64,
        ],
        op: "rain_series".to_string(),
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
    .map_err(|e| upstream_error(format!("rain series attestation: {e}")))?;
    let cids = s
        .storage
        .put_attestation(&att)
        .await
        .map_err(ApiError::from)?;
    let derivation_cid = cids
        .first()
        .map(|c| c.as_str().to_string())
        .ok_or_else(|| upstream_error("store returned no cid for the rain series".into()))?;
    let receipt = s.sign_receipt(
        "emem.rain_series",
        vec![cell.clone()],
        vec![FactCid::new(derivation_cid.clone())],
        false,
        started,
        None,
    );
    let mut out = record;
    out["algorithm_key"] = json!(FN_KEY);
    out["derivation_cid"] = json!(derivation_cid);
    out["token"] = json!(format!("emem:fact:{cell}:{derivation_cid}"));
    out["receipt"] = serde_json::to_value(&receipt).unwrap_or(JsonValue::Null);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn probit_matches_known_quantiles() {
        assert!(probit(0.5).abs() < 1e-9);
        assert!((probit(0.975) - 1.959_964).abs() < 1e-5);
        assert!((probit(0.001) + 3.090_232).abs() < 1e-5);
    }

    #[test]
    fn a_total_ranks_by_its_place_in_the_sample() {
        let sample: Vec<f64> = (1..=99).map(f64::from).collect();
        assert!((rank_probability(50.0, &sample).unwrap() - 0.5).abs() < 1e-9);
        assert!(rank_probability(1000.0, &sample).unwrap() < 1.0);
        assert!(rank_probability(-1.0, &sample).unwrap() > 0.0);
        assert_eq!(
            class_of(probit(rank_probability(1000.0, &sample).unwrap())),
            "extremely wet"
        );
    }

    #[test]
    fn dry_days_tie_rather_than_rank_low() {
        // Ninety dry spells and ten wet ones: a dry spell is ordinary, not extreme.
        let mut sample = vec![0.0; 90];
        sample.extend((1..=10).map(f64::from));
        let p = rank_probability(0.0, &sample).unwrap();
        assert!(p > 0.4 && p < 0.5, "{p}");
    }

    #[test]
    fn a_total_needs_every_day() {
        let by: HashMap<i64, f64> = [(10, 1.0), (11, 2.0), (13, 4.0)].into_iter().collect();
        assert_eq!(total(&by, 10, 2), Some(3.0));
        assert_eq!(total(&by, 10, 4), None);
    }

    #[test]
    fn era5_wins_and_the_model_fills_what_it_has_not_reached() {
        let era5: HashMap<i64, f64> = [(1, 5.0), (2, 6.0)].into_iter().collect();
        let model: HashMap<i64, f64> = [(2, 9.0), (3, 7.0)].into_iter().collect();
        let p = merge_past(1, 4, &era5, &model);
        let got: Vec<_> = p.iter().map(|(d, v, s, _)| (*d, *v, *s)).collect();
        assert_eq!(
            got,
            vec![
                (1, Some(5.0), "era5"),
                (2, Some(6.0), "era5"),
                (3, Some(7.0), "model_analysis"),
                (4, None, "missing")
            ]
        );
    }

    #[test]
    fn the_season_sample_takes_fifteen_totals_a_year() {
        let start = crate::days_from_civil(2000, 1, 1);
        let end = crate::days_from_civil(2002, 12, 31);
        let clim: HashMap<i64, f64> = (start..=end).map(|d| (d, 1.0)).collect();
        let anchor = crate::days_from_civil(2026, 6, 15);
        let ending = season_totals(&clim, (2000, 2002), anchor, 7, true);
        assert_eq!(ending.len(), 45);
        assert!(ending.iter().all(|t| (*t - 7.0).abs() < 1e-9));
    }
}
