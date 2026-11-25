//! Daily rain at a place: the recent past, the next two weeks, and how each
//! total ranks against the same days of the last thirty years.
//!
//! Flood, drought and landslide outlooks all turn on rain, and until this
//! route emem had none it could put in a series: `weather.precipitation_mm`
//! is the current hour only, `era5.precip` is one signed fact per hour, and
//! CHIRPS final data lags a month. One call here returns, as one signed
//! derivation:
//!
//! - the daily totals of the window, from ERA5 reanalysis where it has
//!   arrived (about five days behind), from the forecast model's own analysis
//!   for the days ERA5 has not reached, and from its forecast, with the lead,
//!   for days ahead, each day tagged with which. The window is the last
//!   `past_days` and next `forecast_days`, or a season named by `start` and
//!   `end`;
//! - for the recent window, totals over 1 to 90 days back and 1 to 14 days
//!   ahead; for a named window, its own total. Each is ranked against every
//!   total of the same length ending (or, ahead, starting) within a week of
//!   the same day of year in the climatology years, as a non-exceedance
//!   probability and its standard-normal score. Over a 90-day window that
//!   score is the standardized precipitation index computed without a fitted
//!   distribution (Farahmand & AghaKouchak 2015).

use emem_storage::PutAttestationDetached;
use std::time::Instant;

use axum::extract::State;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value as JsonValue};

use emem_fact::{Derivation, DerivativeFact, Fact, FactCid};

use crate::band_raster::{bad_request, upstream_error};
use crate::change_attribution::json_to_cbor;
use crate::{ApiError, AppState, EmemJson};

const FN_KEY: &str = "rain_series@3";
const PAST_WINDOWS: [u32; 6] = [1, 3, 7, 14, 30, 90];
const AHEAD_WINDOWS: [u32; 4] = [1, 3, 7, 14];
/// Days either side of the day of year whose totals join the climatology:
/// 15 totals a year instead of one, so 30 years give 450 to rank against.
const SEASON_HALF_WIDTH_DAYS: i64 = 7;
/// A total under this is a trace and counts as no rain. Otherwise, in a season
/// whose sample is almost all zeros, 0.1 mm outranks every zero and reads as
/// extremely wet.
const TRACE_MM: f64 = 1.0;
/// Above this share of rainless totals a standardized score says little, so no
/// class is given (Stagge et al. 2015 on zero-inflated SPI).
const MAX_RAINLESS_SHARE: f64 = 0.5;
const MAX_PAST_DAYS: u32 = 92;
const MAX_FORECAST_DAYS: u32 = 16;
const MAX_CLIMATOLOGY_YEARS: u32 = 40;
pub(crate) const MAX_WINDOW_DAYS: i64 = 366;
/// The forecast model's past days only fill what ERA5 has not reached yet.
pub(crate) const ANALYSIS_DAYS: u32 = 10;

/// ERA5's first day on the Open-Meteo archive.
pub(crate) fn first_archive_day() -> i64 {
    crate::days_from_civil(1940, 1, 1)
}

// A field this route does not read is refused: `start_date` and `end_date`
// were once accepted and ignored, and the caller got the default window.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RainSeriesReq {
    #[serde(default)]
    pub lat: Option<f64>,
    #[serde(default, alias = "lon")]
    pub lng: Option<f64>,
    /// A cell64 in place of lat/lng.
    #[serde(default, alias = "cell64")]
    pub cell: Option<String>,
    /// First UTC day, YYYY-MM-DD. With `end`, in place of past_days and
    /// forecast_days.
    #[serde(default, alias = "start_date")]
    pub start: Option<String>,
    #[serde(default, alias = "end_date")]
    pub end: Option<String>,
    /// Days back, 1..=92. Default 30.
    #[serde(default)]
    pub past_days: Option<u32>,
    /// Days ahead, 0..=16. Default 7.
    #[serde(default)]
    pub forecast_days: Option<u32>,
    /// Complete years before this one to rank against, 0..=40. Default 30; 0
    /// skips the ranking.
    #[serde(default)]
    pub climatology_years: Option<u32>,
}

/// One day of the answer: `(day number, millimetres, source, lead days)`.
type Day = (i64, Option<f64>, &'static str, Option<i64>);

pub(crate) fn iso(day: i64) -> String {
    let (y, m, d) = crate::civil_from_days(day);
    format!("{y:04}-{m:02}-{d:02}")
}

pub(crate) fn day_of(iso: &str) -> Option<i64> {
    let (y, m, d) = crate::band_raster::parse_ymd(iso)?;
    Some(crate::days_from_civil(y, m, d))
}

pub(crate) fn parse_date(field: &str, s: &str) -> Result<i64, String> {
    // The round trip refuses 2025-02-30 and 2025-6-1, which parse_ymd lets by.
    day_of(s)
        .filter(|d| iso(*d) == s)
        .ok_or_else(|| format!("`{field}` must be a calendar date as YYYY-MM-DD, got `{s}`"))
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Window {
    Dates { first: i64, last: i64 },
    Recent { past: u32, ahead: u32 },
}

impl Window {
    /// First and last UTC day, given `today`.
    fn days(self, today: i64) -> (i64, i64) {
        match self {
            Window::Dates { first, last } => (first, last),
            Window::Recent { past, ahead } => (today - past as i64, today + ahead as i64 - 1),
        }
    }
}

fn window_of(req: &RainSeriesReq, today: i64) -> Result<Window, String> {
    match (req.start.as_deref(), req.end.as_deref()) {
        (None, None) => {
            let past = req.past_days.unwrap_or(30);
            let ahead = req.forecast_days.unwrap_or(7);
            if !(1..=MAX_PAST_DAYS).contains(&past) || ahead > MAX_FORECAST_DAYS {
                return Err(format!(
                    "past_days takes 1 to {MAX_PAST_DAYS} and forecast_days 0 to {MAX_FORECAST_DAYS}"
                ));
            }
            Ok(Window::Recent { past, ahead })
        }
        (Some(s), Some(e)) => {
            if req.past_days.is_some() || req.forecast_days.is_some() {
                return Err(
                    "give `start` and `end`, or `past_days` and `forecast_days`, not both".into(),
                );
            }
            let first = parse_date("start", s)?;
            let last = parse_date("end", e)?;
            if first < first_archive_day() {
                return Err(format!(
                    "`start` ({s}) is before 1940-01-01, where ERA5 begins"
                ));
            }
            if last < first {
                return Err(format!("`end` ({e}) is before `start` ({s})"));
            }
            let len = last - first + 1;
            if len > MAX_WINDOW_DAYS {
                return Err(format!(
                    "the window is {len} days; the most is {MAX_WINDOW_DAYS}"
                ));
            }
            let reach = today + MAX_FORECAST_DAYS as i64 - 1;
            if first > reach {
                return Err(format!(
                    "`start` ({s}) is after {}, the last day the forecast reaches",
                    iso(reach)
                ));
            }
            Ok(Window::Dates { first, last })
        }
        _ => Err("give both `start` and `end`, or neither".into()),
    }
}

/// The climatology years for a window: the complete years before this one
/// for the recent window, the years before the first day's year for a named
/// one, so a season is never ranked against itself.
fn climatology_years(window: Window, today: i64, years: u32) -> (i32, i32) {
    let anchor = match window {
        Window::Dates { first, .. } => first,
        Window::Recent { .. } => today,
    };
    let (y, _, _) = crate::civil_from_days(anchor);
    (y - years as i32, y - 1)
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
/// The sample, and how many years it came from: its totals overlap, fifteen
/// a year, so the years and not the totals say how far its tails reach.
fn season_totals(
    clim: &std::collections::HashMap<i64, f64>,
    years: (i32, i32),
    anchor: i64,
    len: u32,
    ending: bool,
) -> (Vec<f64>, usize) {
    let (_, m, d) = crate::civil_from_days(anchor);
    // 29 February anchors to the 28th in years without it.
    let d = if m == 2 && d == 29 { 28 } else { d };
    let mut out = Vec::new();
    let mut with_data = 0;
    for y in years.0..=years.1 {
        let centre = crate::days_from_civil(y, m, d);
        let before = out.len();
        for k in -SEASON_HALF_WIDTH_DAYS..=SEASON_HALF_WIDTH_DAYS {
            let a = centre + k;
            let first = if ending { a - len as i64 + 1 } else { a };
            if let Some(t) = total(clim, first, len) {
                out.push(t);
            }
        }
        if out.len() > before {
            with_data += 1;
        }
    }
    (out, with_data)
}

/// Non-exceedance probability of `x` among `sample`, with ties split and the
/// Weibull (i/(n+1)) offset, so it never reaches 0 or 1.
pub(crate) fn rank_probability(x: f64, sample: &[f64]) -> Option<f64> {
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

fn ranked(x: f64, sample: &[f64], years: usize) -> JsonValue {
    let trace = |v: f64| if v < TRACE_MM { 0.0 } else { v };
    let sample: Vec<f64> = sample.iter().map(|v| trace(*v)).collect();
    let Some(p) = rank_probability(trace(x), &sample) else {
        return JsonValue::Null;
    };
    // Weibull's offset for the years of record: a total beyond all of them is
    // about one in years + 1, not one in the count of overlapping totals.
    let bound = 0.5 / (years.max(1) as f64 + 1.0);
    let p = p.clamp(bound, 1.0 - bound);
    let rainless = sample.iter().filter(|v| **v == 0.0).count() as f64 / sample.len() as f64;
    let mut out = json!({"compared_with": sample.len(), "years": years,
                         "non_exceedance": (p * 1e4).round() / 1e4,
                         "rainless_share": (rainless * 1e3).round() / 1e3});
    if rainless > MAX_RAINLESS_SHARE {
        out["z"] = JsonValue::Null;
        out["class"] = json!("not classed: most windows like this one get no rain in this season");
    } else {
        let z = probit(p);
        out["z"] = json!((z * 1e3).round() / 1e3);
        out["class"] = json!(class_of(z));
    }
    out
}

/// The window's days in order. Before `today`: ERA5 where it has a value, the
/// forecast model's analysis where it does not, missing otherwise. From
/// `today` on: the forecast with its lead, else not yet.
fn merge_days(
    first: i64,
    last: i64,
    today: i64,
    era5: &std::collections::HashMap<i64, f64>,
    model: &std::collections::HashMap<i64, f64>,
) -> Vec<Day> {
    (first..=last)
        .map(|d| {
            if d >= today {
                return match model.get(&d) {
                    Some(v) => (d, Some(*v), "forecast", Some(d - today)),
                    None => (d, None, "not_yet", None),
                };
            }
            match (era5.get(&d), model.get(&d)) {
                (Some(v), _) => (d, Some(*v), "era5", None),
                (None, Some(v)) => (d, Some(*v), "model_analysis", None),
                (None, None) => (d, None, "missing", None),
            }
        })
        .collect()
}

/// Lengths of the spans of a named window to total, each from its first day:
/// the days before today that all have a value, then, if forecast days
/// continue it, the span through them.
fn span_lengths(days: &[Day]) -> Vec<usize> {
    let observed = days
        .iter()
        .take_while(|(_, v, _, lead)| v.is_some() && lead.is_none())
        .count();
    let valued = days.iter().take_while(|(_, v, _, _)| v.is_some()).count();
    let mut out = Vec::new();
    if observed > 0 {
        out.push(observed);
    }
    if valued > observed {
        out.push(valued);
    }
    out
}

pub(crate) async fn get_json(url: &str) -> Result<JsonValue, String> {
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

/// The point a series is for: `lat` and `lng`, or the centre of `cell`.
pub(crate) fn point(
    lat: Option<f64>,
    lng: Option<f64>,
    cell: Option<&str>,
) -> Result<(f64, f64), ApiError> {
    let (lat, lng) = match (lat, lng, cell) {
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
    Ok((lat, lng))
}

pub(crate) async fn rain_series(req: RainSeriesReq, s: &AppState) -> Result<JsonValue, ApiError> {
    let started = Instant::now();
    let (lat, lng) = point(req.lat, req.lng, req.cell.as_deref())?;
    let today = crate::now_unix() as i64 / 86_400;
    let window = window_of(&req, today).map_err(bad_request)?;
    let years = req.climatology_years.unwrap_or(30);
    if years > MAX_CLIMATOLOGY_YEARS {
        return Err(bad_request(format!(
            "climatology_years takes 0 to {MAX_CLIMATOLOGY_YEARS}, got {years}"
        )));
    }
    let cell = emem_codec::geo::cell64_from_latlng(lat, lng);
    let (first, last) = window.days(today);
    let yesterday = today - 1;
    let clim_years = climatology_years(window, today, years);

    // The forecast model fills the days ERA5 has not reached and gives the
    // days ahead; a window that ends before them needs no forecast call.
    let (fc_past, fc_ahead) = match window {
        Window::Recent { past, ahead } => (ANALYSIS_DAYS.min(past), ahead),
        Window::Dates { .. } => (
            if last >= today - ANALYSIS_DAYS as i64 {
                (today - first).clamp(0, ANALYSIS_DAYS as i64) as u32
            } else {
                0
            },
            (last - today + 1).clamp(0, MAX_FORECAST_DAYS as i64) as u32,
        ),
    };
    let forecast_url = (fc_past > 0 || fc_ahead > 0).then(|| {
        format!(
            "https://api.open-meteo.com/v1/forecast?latitude={lat:.4}&longitude={lng:.4}\
             &daily=precipitation_sum&past_days={fc_past}&forecast_days={fc_ahead}&timezone=UTC"
        )
    });
    // ERA5 from far enough before the first climatology year that its first
    // totals are whole, to the window's end or the last climatology year,
    // in one request: about 11,000 values for 30 years.
    let longest = match window {
        Window::Recent { .. } => 90,
        Window::Dates { first, last } => last - first + 1,
    };
    let era5_from = if years > 0 {
        (crate::days_from_civil(clim_years.0, 1, 1) - longest - SEASON_HALF_WIDTH_DAYS).min(first)
    } else {
        first
    }
    .max(first_archive_day());
    let era5_to = if years > 0 {
        last.max(crate::days_from_civil(clim_years.1 + 1, 12, 31))
    } else {
        last
    }
    .min(yesterday);
    let archive_url = (era5_from <= era5_to).then(|| {
        format!(
            "https://archive-api.open-meteo.com/v1/archive?latitude={lat:.4}&longitude={lng:.4}\
             &start_date={}&end_date={}&daily=precipitation_sum&timezone=UTC&models=era5",
            iso(era5_from),
            iso(era5_to)
        )
    });
    let fetch = |url: Option<String>, what: &'static str| async move {
        match url {
            None => Ok(vec![]),
            Some(u) => daily_values(
                &get_json(&u)
                    .await
                    .map_err(|e| upstream_error(format!("{what}: {e}")))?,
            )
            .map_err(|e| upstream_error(format!("{what}: {e}"))),
        }
    };
    let (fc, ar) = tokio::join!(
        fetch(forecast_url.clone(), "rain forecast"),
        fetch(archive_url.clone(), "ERA5 archive")
    );
    let (fc, ar) = (fc?, ar?);

    let era5: std::collections::HashMap<i64, f64> =
        ar.iter().filter_map(|(d, v)| Some((*d, (*v)?))).collect();
    let model: std::collections::HashMap<i64, f64> =
        fc.iter().filter_map(|(d, v)| Some((*d, (*v)?))).collect();
    let days_in = merge_days(first, last, today, &era5, &model);
    let valued: std::collections::HashMap<i64, f64> = days_in
        .iter()
        .filter_map(|(d, v, _, _)| Some((*d, (*v)?)))
        .collect();
    // The days before the first climatology year stay in, so its totals are
    // whole; nothing after the last one does, so a window never ranks against
    // its own days.
    let clim: std::collections::HashMap<i64, f64> = era5
        .iter()
        .filter(|(d, _)| crate::civil_from_days(**d).0 <= clim_years.1)
        .map(|(d, v)| (*d, *v))
        .collect();
    let rank = |t: f64, anchor: i64, len: u32, ending: bool| {
        let (sample, n_years) = if years > 0 {
            season_totals(&clim, clim_years, anchor, len, ending)
        } else {
            (vec![], 0)
        };
        ranked(t, &sample, n_years)
    };

    let round = |v: f64| (v * 10.0).round() / 10.0;
    let mut record_totals = serde_json::Map::new();
    match window {
        Window::Recent { past, ahead } => {
            let mut past_totals = Vec::new();
            for w in PAST_WINDOWS.into_iter().filter(|w| *w <= past) {
                let from = yesterday - w as i64 + 1;
                if let Some(t) = total(&valued, from, w) {
                    past_totals.push(json!({"days": w, "from": iso(from), "to": iso(yesterday),
                                            "mm": round(t), "climatology": rank(t, yesterday, w, true)}));
                }
            }
            let mut ahead_totals = Vec::new();
            for w in AHEAD_WINDOWS.into_iter().filter(|w| *w <= ahead) {
                if let Some(t) = total(&valued, today, w) {
                    ahead_totals.push(json!({"days": w, "from": iso(today), "to": iso(today + w as i64 - 1),
                                             "mm": round(t), "climatology": rank(t, today, w, false)}));
                }
            }
            record_totals.insert("past_totals".into(), json!(past_totals));
            record_totals.insert("forecast_totals".into(), json!(ahead_totals));
        }
        Window::Dates { .. } => {
            let spans: Vec<JsonValue> = span_lengths(&days_in)
                .into_iter()
                .filter_map(|n| {
                    let to = first + n as i64 - 1;
                    let t = total(&valued, first, n as u32)?;
                    Some(
                        json!({"days": n, "from": iso(first), "to": iso(to), "mm": round(t),
                                "through": if to >= today { "forecast" } else { "observed" },
                                "whole_window": to == last,
                                "climatology": rank(t, to, n as u32, true)}),
                    )
                })
                .collect();
            record_totals.insert("window_totals".into(), json!(spans));
        }
    }
    let days: Vec<JsonValue> = days_in
        .iter()
        .map(|(d, v, src, lead)| {
            let mut o = json!({"date": iso(*d), "mm": v.map(round), "source": src});
            if let Some(l) = lead {
                o["lead_days"] = json!(l);
            }
            o
        })
        .collect();
    let count = |src: &str| days_in.iter().filter(|(_, _, s, _)| *s == src).count();
    let ahead_days = count("forecast");
    let mut cautions = Vec::new();
    if first < today {
        cautions.push(
            "Past days are ERA5 reanalysis, or the forecast model's own analysis for the days \
             ERA5 has not reached; neither is a rain gauge. ERA5 is a 0.25 degree grid (about \
             28 km), so a point gets its grid cell's rain, and it under-reads short convective \
             bursts."
                .to_string(),
        );
    }
    if ahead_days > 0 {
        cautions.push(format!(
            "Days from {} on are a weather model's forecast, issued when this record was signed. \
             A total that includes them is ranked against ERA5's: different models on different \
             grids, so it ranks high a little more easily than an observed total would.",
            iso(today)
        ));
    }
    let gaps = count("missing");
    if gaps > 0 {
        cautions.push(format!(
            "{gaps} past day(s) had no value from either source, and totals that span them are left out."
        ));
    }
    let not_yet = count("not_yet");
    if not_yet > 0 {
        cautions.push(format!(
            "{not_yet} day(s) of the window lie past the forecast's reach and have no value yet."
        ));
    }
    let asked_as = match window {
        Window::Dates { .. } => json!({"start": iso(first), "end": iso(last)}),
        Window::Recent { past, ahead } => json!({"past_days": past, "forecast_days": ahead}),
    };
    let mut sources = Vec::new();
    if let Some(u) = &forecast_url {
        sources.push(
            json!({"name": "Open-Meteo forecast API (best-match weather model for the location)",
                            "url": u, "licence": "CC BY 4.0",
                            "role": "days ahead, and past days ERA5 has not reached"}),
        );
    }
    if let Some(u) = &archive_url {
        sources.push(json!({"name": "Open-Meteo historical weather API, ERA5 (Hersbach et al. 2020)",
                            "url": u, "licence": "CC BY 4.0", "role": "past days and the climatology"}));
    }
    let mut record = json!({
        "schema": "emem.rain_series.v1",
        "fn_key": FN_KEY,
        "location": {"lat": lat, "lng": lng, "cell": cell},
        "issued": iso(today),
        "window": {"asked_as": asked_as, "from": iso(first), "to": iso(last), "days": last - first + 1},
        "days": days,
        "climatology": if years > 0 {
            json!({"years": [clim_years.0, clim_years.1], "source": "ERA5 daily totals",
                   "rule": format!("every total of the same length ending (past) or starting (ahead) within {SEASON_HALF_WIDTH_DAYS} days of the same day of year, each year; non-exceedance by rank, ties split, Weibull offset; z is its standard-normal quantile; classes after McKee et al. 1993")})
        } else {
            JsonValue::Null
        },
        "estimand": {"quantity": "daily precipitation total at the point, UTC days",
                     "unit": "mm", "statistic": "sum over UTC days"},
        "sources": sources,
        "cautions": cautions,
    });
    for (k, v) in record_totals {
        record[k] = v;
    }
    let what = SeriesFact {
        cell: &cell,
        band: "rain.series",
        fn_key: FN_KEY,
        receipt_kind: "emem.rain_series",
        days: (first, last),
    };
    sign_record(s, record, what, started).await
}

/// Where and what a signed series record is.
pub(crate) struct SeriesFact<'a> {
    pub cell: &'a str,
    pub band: &'a str,
    pub fn_key: &'a str,
    pub receipt_kind: &'static str,
    /// First and last day of the series, as days since 1970-01-01.
    pub days: (i64, i64),
}

/// Signs `record` as one derivative fact, stores it, and returns the record
/// with its cid, token and receipt beside it.
pub(crate) async fn sign_record(
    s: &AppState,
    record: JsonValue,
    what: SeriesFact<'_>,
    started: Instant,
) -> Result<JsonValue, ApiError> {
    let SeriesFact {
        cell,
        band,
        fn_key,
        receipt_kind,
        days,
    } = what;
    let op = fn_key.split('@').next().unwrap_or(fn_key);
    let signed_at = emem_storage::server::iso8601_now();
    let fact = DerivativeFact {
        cell: cell.to_string(),
        band: band.to_string(),
        tslot_window: [days.0.max(0) as u64, days.1.max(0) as u64],
        op: op.to_string(),
        parents: vec![],
        value: json_to_cbor(&record),
        confidence: 1.0,
        derivation: Derivation {
            fn_key: fn_key.to_string(),
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
    .map_err(|e| upstream_error(format!("{op} attestation: {e}")))?;
    let cids = s
        .storage
        .put_attestation_detached(&att)
        .await
        .map_err(ApiError::from)?;
    let derivation_cid = cids
        .first()
        .map(|c| c.as_str().to_string())
        .ok_or_else(|| upstream_error(format!("store returned no cid for the {op} record")))?;
    let receipt = s.sign_receipt(
        receipt_kind,
        vec![cell.to_string()],
        vec![FactCid::new(derivation_cid.clone())],
        false,
        started,
        None,
    );
    let mut out = record;
    out["algorithm_key"] = json!(fn_key);
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
        let p = merge_days(1, 4, 100, &era5, &model);
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
        let (ending, years) = season_totals(&clim, (2000, 2002), anchor, 7, true);
        assert_eq!(ending.len(), 45);
        assert_eq!(years, 3);
        assert!(ending.iter().all(|t| (*t - 7.0).abs() < 1e-9));
    }

    #[test]
    fn a_trace_of_rain_in_a_dry_season_is_not_wet() {
        let mut sample = vec![0.0; 445];
        sample.extend([2.0, 3.0, 5.0, 8.0, 13.0]);
        let r = ranked(0.1, &sample, 30);
        assert!(r["z"].is_null(), "{r}");
        assert!(
            r["class"].as_str().unwrap().starts_with("not classed"),
            "{r}"
        );
    }

    #[test]
    fn the_tails_reach_as_far_as_the_years_of_record() {
        let sample: Vec<f64> = (1..=450).map(f64::from).collect();
        let r = ranked(10_000.0, &sample, 30);
        let z = r["z"].as_f64().unwrap();
        assert!((z - probit(1.0 - 0.5 / 31.0)).abs() < 1e-3, "{r}");
        assert_eq!(r["class"], "extremely wet");
        let low = ranked(-1.0, &sample, 30);
        assert!(low["z"].as_f64().unwrap() > -2.2, "{low}");
    }

    fn req(body: serde_json::Value) -> Result<RainSeriesReq, serde_json::Error> {
        serde_json::from_value(body)
    }

    #[test]
    fn a_field_the_route_does_not_read_is_refused() {
        let e = req(json!({"lat": 30.9, "lng": 75.85, "from": "2025-06-15"})).unwrap_err();
        assert!(e.to_string().contains("unknown field `from`"), "{e}");
        let r = req(json!({"lat": 30.9, "lng": 75.85,
                           "start_date": "2025-06-15", "end_date": "2025-10-15"}))
        .unwrap();
        assert_eq!(r.start.as_deref(), Some("2025-06-15"));
        assert_eq!(r.end.as_deref(), Some("2025-10-15"));
    }

    #[test]
    fn a_named_window_is_read_as_its_dates() {
        let today = crate::days_from_civil(2026, 11, 25);
        let r = req(json!({"lat": 30.9, "lng": 75.85, "start": "2025-06-15", "end": "2025-10-15"}))
            .unwrap();
        let w = window_of(&r, today).unwrap();
        let (first, last) = w.days(today);
        assert_eq!(
            (iso(first), iso(last)),
            ("2025-06-15".into(), "2025-10-15".into())
        );
        assert_eq!(last - first + 1, 123);
        assert_eq!(climatology_years(w, today, 30), (1995, 2024));
    }

    #[test]
    fn the_recent_window_keeps_its_defaults_and_takes_no_forecast() {
        let today = crate::days_from_civil(2026, 11, 25);
        let w = window_of(&req(json!({"lat": 1.0, "lng": 2.0})).unwrap(), today).unwrap();
        assert_eq!(w, Window::Recent { past: 30, ahead: 7 });
        assert_eq!(climatology_years(w, today, 30), (1996, 2025));
        let w0 = window_of(
            &req(json!({"lat": 1.0, "lng": 2.0, "past_days": 92, "forecast_days": 0})).unwrap(),
            today,
        )
        .unwrap();
        assert_eq!(w0.days(today), (today - 92, today - 1));
    }

    #[test]
    fn a_bad_window_is_refused_with_its_reason() {
        let today = crate::days_from_civil(2026, 11, 25);
        let bad = |b: serde_json::Value| window_of(&req(b).unwrap(), today).unwrap_err();
        assert!(bad(json!({"start": "2025-06-15"})).contains("both"));
        assert!(
            bad(json!({"start": "2025-06-15", "end": "2025-10-15", "past_days": 7}))
                .contains("not both")
        );
        assert!(bad(json!({"start": "2025-02-30", "end": "2025-03-01"})).contains("calendar date"));
        assert!(bad(json!({"start": "1939-12-31", "end": "1940-01-31"})).contains("1940"));
        assert!(bad(json!({"start": "2025-10-15", "end": "2025-06-15"})).contains("before"));
        assert!(bad(json!({"start": "2024-01-01", "end": "2025-01-01"})).contains("367 days"));
        assert!(
            bad(json!({"start": "2026-12-11", "end": "2026-12-20"})).contains("forecast reaches")
        );
        assert!(bad(json!({"past_days": 93})).contains("past_days takes 1 to 92"));
        assert!(bad(json!({"forecast_days": 17})).contains("forecast_days 0 to 16"));
    }

    #[test]
    fn a_named_window_totals_what_it_has_observed_then_through_the_forecast() {
        let today = 10;
        let era5: HashMap<i64, f64> = (5..8).map(|d| (d, 1.0)).collect();
        let model: HashMap<i64, f64> = (8..13).map(|d| (d, 2.0)).collect();
        let days = merge_days(5, 14, today, &era5, &model);
        assert_eq!(span_lengths(&days), vec![5, 8]);
        assert_eq!(days[5].2, "forecast");
        assert_eq!(days[9].2, "not_yet");
        let past_only = merge_days(5, 7, today, &era5, &model);
        assert_eq!(span_lengths(&past_only), vec![3]);
    }

    #[test]
    fn an_ordinary_total_ranks_as_before() {
        let sample: Vec<f64> = (1..=99).map(f64::from).collect();
        let r = ranked(50.0, &sample, 30);
        assert!(r["z"].as_f64().unwrap().abs() < 1e-3, "{r}");
        assert_eq!(r["class"], "near normal");
    }
}
