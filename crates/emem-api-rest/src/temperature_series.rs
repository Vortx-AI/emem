//! Daily air temperature at a place over a window, the growing degree-days it
//! adds up to, and where that total ranks among the same days of earlier
//! years.
//!
//! Crop stage, sowing-date checks and heat stress run on daily Tmax and Tmin.
//! Before this route emem held them only as one signed fact per day
//! (`power.t2m_max`, `power.t2m_min`) or per hour (`era5.t2m`), so a season
//! needed a backfill of hundreds of facts. One call here returns, as one
//! signed derivation:
//!
//! - every local day of the window: ERA5-Land where it has a value, ERA5
//!   where it has none, the forecast model's analysis for the days neither
//!   reanalysis has reached, and its forecast, with the lead, for days ahead,
//!   each day tagged with its source;
//! - growing degree-days per day and summed, by McMaster and Wilhelm (1997)
//!   method 1 or 2, and the heat-stress and frost days;
//! - the GDD total ranked against the span of the same calendar days and
//!   length in each earlier year. A window still open ranks only the span it
//!   has.

use std::collections::HashMap;
use std::time::Instant;

use axum::extract::State;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value as JsonValue};

use crate::band_raster::{bad_request, upstream_error};
use crate::rain_series::{
    day_of, get_json, iso, parse_date, point, rank_probability, sign_record, SeriesFact,
    ANALYSIS_DAYS,
};
use crate::{ApiError, AppState, EmemJson};

const FN_KEY: &str = "temperature_series@1";
const DAILY: &str = "temperature_2m_max,temperature_2m_min,temperature_2m_mean";
pub(crate) const MAX_WINDOW_DAYS: i64 = 366;
const MAX_PAST_DAYS: u32 = 92;
const MAX_FORECAST_DAYS: u32 = 16;
const MAX_CLIMATOLOGY_YEARS: u32 = 30;
const FROST_C: f64 = 0.0;

/// ERA5-Land's first day on the Open-Meteo archive; it returns earlier days
/// empty.
pub(crate) fn first_archive_day() -> i64 {
    crate::days_from_civil(1950, 1, 1)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemperatureSeriesReq {
    #[serde(default)]
    pub lat: Option<f64>,
    #[serde(default, alias = "lon")]
    pub lng: Option<f64>,
    /// A cell64 in place of lat/lng.
    #[serde(default, alias = "cell64")]
    pub cell: Option<String>,
    /// First day, YYYY-MM-DD. With `end`, in place of past_days/forecast_days.
    #[serde(default)]
    pub start: Option<String>,
    #[serde(default)]
    pub end: Option<String>,
    /// Days before today, 1..=92. Default 30.
    #[serde(default)]
    pub past_days: Option<u32>,
    /// Days from today on, 0..=16. Default 7.
    #[serde(default)]
    pub forecast_days: Option<u32>,
    /// GDD base temperature, 0..=20 C. Default 10.
    #[serde(default)]
    pub base_c: Option<f64>,
    /// `average` (default) or `capped`.
    #[serde(default)]
    pub method: Option<String>,
    /// Upper limit for `capped`, above base_c and at most 45 C. Default 30.
    #[serde(default)]
    pub cap_c: Option<f64>,
    /// Tmax at or above this is a heat-stress day, 25..=50 C. Default 35.
    #[serde(default)]
    pub heat_c: Option<f64>,
    /// Earlier years to rank against, 0..=30. Default 10; 0 skips it.
    #[serde(default)]
    pub climatology_years: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Window {
    Dates { first: i64, last: i64 },
    Recent { past: u32, ahead: u32 },
}

impl Window {
    /// First and last day, given the local `today`.
    fn days(self, today: i64) -> (i64, i64) {
        match self {
            Window::Dates { first, last } => (first, last),
            Window::Recent { past, ahead } => (today - past as i64, today + ahead as i64 - 1),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Method {
    Average,
    Capped { cap: f64 },
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Params {
    window: Window,
    base: f64,
    method: Method,
    heat: f64,
    years: u32,
}

fn window_of(req: &TemperatureSeriesReq, utc_today: i64) -> Result<Window, String> {
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
                    "`start` ({s}) is before 1950-01-01, where ERA5-Land begins"
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
            let reach = utc_today + MAX_FORECAST_DAYS as i64 - 1;
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

fn params(req: &TemperatureSeriesReq, utc_today: i64) -> Result<Params, String> {
    let window = window_of(req, utc_today)?;
    let base = req.base_c.unwrap_or(10.0);
    if !(0.0..=20.0).contains(&base) {
        return Err(format!("base_c takes 0 to 20 degrees C, got {base}"));
    }
    let method = match req.method.as_deref().unwrap_or("average") {
        "average" => {
            if req.cap_c.is_some() {
                return Err("cap_c applies only to method \"capped\"".into());
            }
            Method::Average
        }
        "capped" => {
            let cap = req.cap_c.unwrap_or(30.0);
            if !(cap > base && cap <= 45.0) {
                return Err(format!(
                    "cap_c must be above base_c ({base}) and at most 45 degrees C, got {cap}"
                ));
            }
            Method::Capped { cap }
        }
        other => {
            return Err(format!(
                "method takes \"average\" or \"capped\", got \"{other}\""
            ))
        }
    };
    let heat = req.heat_c.unwrap_or(35.0);
    if !(25.0..=50.0).contains(&heat) {
        return Err(format!("heat_c takes 25 to 50 degrees C, got {heat}"));
    }
    let years = req.climatology_years.unwrap_or(10);
    if years > MAX_CLIMATOLOGY_YEARS {
        return Err(format!(
            "climatology_years takes 0 to {MAX_CLIMATOLOGY_YEARS}, got {years}"
        ));
    }
    Ok(Params {
        window,
        base,
        method,
        heat,
        years,
    })
}

/// Growing degree-days of one day (McMaster and Wilhelm 1997). Method 1
/// averages first and floors the result at zero. Method 2 holds Tmax and Tmin
/// each within [base, cap] first, so a hot afternoon adds no more than the
/// cap allows and a cold night takes nothing away.
fn gdd(tmax: f64, tmin: f64, base: f64, method: Method) -> f64 {
    match method {
        Method::Average => ((tmax + tmin) / 2.0 - base).max(0.0),
        Method::Capped { cap } => (tmax.clamp(base, cap) + tmin.clamp(base, cap)) / 2.0 - base,
    }
}

fn r2(v: f64) -> f64 {
    // An empty f64 sum is -0.0, and the record would sign "-0.0".
    (v * 100.0).round() / 100.0 + 0.0
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Temps {
    max: f64,
    min: f64,
    mean: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Day {
    day: i64,
    t: Option<Temps>,
    source: &'static str,
    lead: Option<i64>,
}

/// Open-Meteo's `daily` block as day -> temperatures. A day counts only with
/// both Tmax and Tmin: Open-Meteo leaves a local day empty until its model
/// covers all of it.
fn daily_temps(body: &JsonValue) -> Result<HashMap<i64, Temps>, String> {
    let col = |k: &str| {
        body.pointer(&format!("/daily/{k}"))
            .and_then(|v| v.as_array())
            .ok_or_else(|| format!("the answer has no daily.{k}"))
    };
    let (times, max, min, mean) = (
        col("time")?,
        col("temperature_2m_max")?,
        col("temperature_2m_min")?,
        col("temperature_2m_mean")?,
    );
    Ok(times
        .iter()
        .enumerate()
        .filter_map(|(i, t)| {
            let temps = Temps {
                max: max.get(i)?.as_f64()?,
                min: min.get(i)?.as_f64()?,
                mean: mean.get(i).and_then(|v| v.as_f64()),
            };
            Some((day_of(t.as_str()?)?, temps))
        })
        .collect())
}

/// The window's days in order. Before `today`: ERA5-Land, then ERA5, then the
/// forecast model's analysis, else missing. From `today` on: the forecast
/// with its lead, else not yet.
fn merge_days(
    first: i64,
    last: i64,
    today: i64,
    land: &HashMap<i64, Temps>,
    era5: &HashMap<i64, Temps>,
    model: &HashMap<i64, Temps>,
) -> Vec<Day> {
    (first..=last)
        .map(|day| {
            if day >= today {
                return match model.get(&day) {
                    Some(t) => Day {
                        day,
                        t: Some(*t),
                        source: "forecast",
                        lead: Some(day - today),
                    },
                    None => Day {
                        day,
                        t: None,
                        source: "not_yet",
                        lead: None,
                    },
                };
            }
            let (t, source) = match (land.get(&day), era5.get(&day), model.get(&day)) {
                (Some(t), _, _) => (Some(*t), "era5_land"),
                (None, Some(t), _) => (Some(*t), "era5"),
                (None, None, Some(t)) => (Some(*t), "model_analysis"),
                (None, None, None) => (None, "missing"),
            };
            Day {
                day,
                t,
                source,
                lead: None,
            }
        })
        .collect()
}

/// The same calendar day `back` years earlier; 29 February moves to the 28th.
fn years_before(day: i64, back: u32) -> i64 {
    let (y, m, d) = crate::civil_from_days(day);
    let d = if m == 2 && d == 29 { 28 } else { d };
    crate::days_from_civil(y - back as i32, m, d)
}

/// GDD total over `len` days from `first`, or None when a day has no value.
fn span_total(
    by_day: &HashMap<i64, Temps>,
    first: i64,
    len: i64,
    base: f64,
    method: Method,
) -> Option<f64> {
    (first..first + len)
        .map(|d| by_day.get(&d).map(|t| r2(gdd(t.max, t.min, base, method))))
        .sum::<Option<f64>>()
        .map(r2)
}

/// Each earlier year's total over the span that starts on the same calendar
/// day and has the same length, oldest first. A year that starts before the
/// archive, or has a day with no value, is left out.
fn year_totals(
    by_day: &HashMap<i64, Temps>,
    first: i64,
    len: i64,
    years: u32,
    base: f64,
    method: Method,
) -> Vec<(i32, f64)> {
    (1..=years)
        .rev()
        .filter_map(|k| {
            let f = years_before(first, k);
            if f < first_archive_day() {
                return None;
            }
            Some((
                crate::civil_from_days(f).0,
                span_total(by_day, f, len, base, method)?,
            ))
        })
        .collect()
}

/// Where `this` falls among the earlier years' totals.
fn rank_among_years(this: f64, years: &[(i32, f64)]) -> JsonValue {
    let sample: Vec<f64> = years.iter().map(|(_, t)| *t).collect();
    let by_year: Vec<JsonValue> = years
        .iter()
        .map(|(y, t)| json!({"year": y, "gdd": t}))
        .collect();
    let Some(p) = rank_probability(this, &sample) else {
        return json!({"years_used": 0, "by_year": by_year, "non_exceedance": null,
                      "note": "no earlier year has a value for every day of this span"});
    };
    let mean = sample.iter().sum::<f64>() / sample.len() as f64;
    let fold = |f: fn(f64, f64) -> f64, from: f64| sample.iter().copied().fold(from, f);
    json!({
        "years_used": sample.len(),
        "by_year": by_year,
        "non_exceedance": (p * 1e4).round() / 1e4,
        "years_below": sample.iter().filter(|v| **v < this - 1e-9).count(),
        "years_above": sample.iter().filter(|v| **v > this + 1e-9).count(),
        "mean_gdd": r2(mean),
        "min_gdd": fold(f64::min, f64::INFINITY),
        "max_gdd": fold(f64::max, f64::NEG_INFINITY),
        "anomaly_gdd": r2(this - mean),
    })
}

fn archive_url(lat: f64, lng: f64, model: &str, from: i64, to: i64) -> String {
    format!(
        "https://archive-api.open-meteo.com/v1/archive?latitude={lat:.4}&longitude={lng:.4}\
         &start_date={}&end_date={}&daily={DAILY}&timezone=auto&models={model}",
        iso(from),
        iso(to)
    )
}

/// Lengths of the spans to rank, each from the window's first day: the days
/// before today that all have a value, then, if forecast days continue it,
/// the span through them.
fn span_lengths(days: &[Day]) -> Vec<usize> {
    let observed = days
        .iter()
        .take_while(|d| d.t.is_some() && d.lead.is_none())
        .count();
    let valued = days.iter().take_while(|d| d.t.is_some()).count();
    let mut out = Vec::new();
    if observed > 0 {
        out.push(observed);
    }
    if valued > observed {
        out.push(valued);
    }
    out
}

/// The record's plain statement of how final its totals are. Counts are of
/// the window's days by source.
fn status(
    window_open: bool,
    last: i64,
    analysis: usize,
    forecast: usize,
    not_yet: usize,
    missing: usize,
) -> String {
    let mut s = if window_open {
        format!(
            "The window is still open: it runs to {}, so the totals are not final.",
            iso(last)
        )
    } else {
        "The window is closed.".to_string()
    };
    if analysis > 0 {
        s.push_str(&format!(
            " {analysis} recent day(s) are the forecast model's analysis; a later call takes the \
             reanalysis for them once it arrives, so the totals can move."
        ));
    }
    if forecast > 0 {
        s.push_str(&format!(
            " {forecast} day(s) are forecast; gdd_observed leaves them out."
        ));
    }
    if not_yet > 0 {
        s.push_str(&format!(
            " {not_yet} day(s) have no value yet, and the totals leave them out."
        ));
    }
    if missing > 0 {
        s.push_str(&format!(
            " {missing} past day(s) have no value from any source, and the totals leave them out."
        ));
    }
    if !window_open && missing == 0 && analysis == 0 {
        s.push_str(" Every day has a reanalysis value, so the totals are final.");
    }
    s
}

pub async fn post_temperature_series(
    State(s): State<AppState>,
    EmemJson(req): EmemJson<TemperatureSeriesReq>,
) -> Result<Json<JsonValue>, ApiError> {
    Ok(Json(temperature_series(req, &s).await?))
}

pub(crate) async fn temperature_series(
    req: TemperatureSeriesReq,
    s: &AppState,
) -> Result<JsonValue, ApiError> {
    let started = Instant::now();
    let (lat, lng) = point(req.lat, req.lng, req.cell.as_deref())?;
    let now = crate::now_unix() as i64;
    let utc_today = now.div_euclid(86_400);
    let p = params(&req, utc_today).map_err(bad_request)?;
    let cell = emem_codec::geo::cell64_from_latlng(lat, lng);

    // The forecast comes first when the window reaches the last days: its
    // UTC offset fixes the local today, and a past_days window counts from it.
    let needs_forecast = match p.window {
        Window::Recent { .. } => true,
        Window::Dates { last, .. } => last >= utc_today - ANALYSIS_DAYS as i64 - 1,
    };
    let forecast_url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={lat:.4}&longitude={lng:.4}\
         &daily={DAILY}&past_days={ANALYSIS_DAYS}&forecast_days={MAX_FORECAST_DAYS}&timezone=auto"
    );
    let fc_body = if needs_forecast {
        Some(
            get_json(&forecast_url)
                .await
                .map_err(|e| upstream_error(format!("temperature forecast: {e}")))?,
        )
    } else {
        None
    };
    let offset_of = |b: &JsonValue| b.get("utc_offset_seconds").and_then(|v| v.as_i64());
    let local_today = |offset: Option<i64>| (now + offset.unwrap_or(0)).div_euclid(86_400);
    let today = local_today(fc_body.as_ref().and_then(offset_of));
    let (first, last) = p.window.days(today);
    let len = last - first + 1;

    // One ERA5-Land request covers the climatology years and the window's
    // past days. The archive refuses an end past the UTC today.
    let ar_first = if p.years > 0 {
        years_before(first, p.years)
    } else {
        first
    }
    .max(first_archive_day());
    let ar_last = last.min(today - 1).min(utc_today);
    let land_url = archive_url(lat, lng, "era5_land", ar_first, ar_last);
    let land_body = if ar_first <= ar_last {
        Some(
            get_json(&land_url)
                .await
                .map_err(|e| upstream_error(format!("ERA5-Land archive: {e}")))?,
        )
    } else {
        None
    };
    let land = match &land_body {
        Some(b) => daily_temps(b).map_err(|e| upstream_error(format!("ERA5-Land archive: {e}")))?,
        None => HashMap::new(),
    };
    // ERA5 only over the days ERA5-Land left empty: none for a closed inland
    // window, the last few for a current one, all of them over water.
    let gaps: Vec<i64> = if land_body.is_some() {
        (ar_first..=ar_last)
            .filter(|d| !land.contains_key(d))
            .collect()
    } else {
        Vec::new()
    };
    let era5_url = gaps
        .first()
        .zip(gaps.last())
        .map(|(a, b)| archive_url(lat, lng, "era5", *a, *b));
    let era5_body = match &era5_url {
        Some(u) => Some(
            get_json(u)
                .await
                .map_err(|e| upstream_error(format!("ERA5 archive: {e}")))?,
        ),
        None => None,
    };
    let era5 = match &era5_body {
        Some(b) => daily_temps(b).map_err(|e| upstream_error(format!("ERA5 archive: {e}")))?,
        None => HashMap::new(),
    };
    let model = match &fc_body {
        Some(b) => {
            daily_temps(b).map_err(|e| upstream_error(format!("temperature forecast: {e}")))?
        }
        None => HashMap::new(),
    };
    let zone_body = fc_body.as_ref().or(land_body.as_ref());
    // Without a forecast the window ended at least eleven days ago, so this
    // moves only the `issued` date, not which source a day takes.
    let today = local_today(zone_body.and_then(offset_of));

    let days = merge_days(first, last, today, &land, &era5, &model);
    // ERA5 first and ERA5-Land over it, so ERA5-Land wins where both have a day.
    let reanalysis: HashMap<i64, Temps> = era5
        .iter()
        .chain(land.iter())
        .map(|(d, t)| (*d, *t))
        .collect();

    let gdds: Vec<Option<f64>> = days
        .iter()
        .map(|d| d.t.map(|t| r2(gdd(t.max, t.min, p.base, p.method))))
        .collect();
    let mut cum = 0.0;
    let rows: Vec<JsonValue> = days
        .iter()
        .zip(&gdds)
        .map(|(d, g)| {
            if let Some(g) = g {
                cum += g;
            }
            let mut row = json!({
                "date": iso(d.day),
                "tmax_c": d.t.map(|t| t.max),
                "tmin_c": d.t.map(|t| t.min),
                "tmean_c": d.t.and_then(|t| t.mean),
                "gdd": g,
                "gdd_cum": g.map(|_| r2(cum)),
                "source": d.source,
            });
            if let Some(l) = d.lead {
                row["lead_days"] = json!(l);
            }
            row
        })
        .collect();

    let count = |src: &str| days.iter().filter(|d| d.source == src).count();
    let forecast_n = count("forecast");
    let analysis_n = count("model_analysis");
    let sum_gdd = |forecast: Option<bool>| {
        r2(days
            .iter()
            .zip(&gdds)
            .filter(|(d, _)| forecast.is_none_or(|f| d.lead.is_some() == f))
            .filter_map(|(_, g)| *g)
            .sum())
    };
    let heat = |d: &&Day| d.t.is_some_and(|t| t.max >= p.heat);
    let frost = |d: &&Day| d.t.is_some_and(|t| t.min <= FROST_C);
    let in_forecast = |d: &&Day| d.lead.is_some();
    let window_open = last >= today;
    let totals = json!({
        "gdd": sum_gdd(None),
        "gdd_observed": sum_gdd(Some(false)),
        "gdd_forecast": sum_gdd(Some(true)),
        "heat_stress_days": days.iter().filter(heat).count(),
        "heat_stress_days_forecast": days.iter().filter(heat).filter(in_forecast).count(),
        "frost_days": days.iter().filter(frost).count(),
        "frost_days_forecast": days.iter().filter(frost).filter(in_forecast).count(),
        "days": {
            "in_window": len,
            "era5_land": count("era5_land"),
            "era5": count("era5"),
            "model_analysis": analysis_n,
            "forecast": forecast_n,
            "missing": count("missing"),
            "not_yet": count("not_yet"),
        },
        "complete": gdds.iter().all(Option::is_some),
        "window_open": window_open,
        "status": status(window_open, last, analysis_n, forecast_n, count("not_yet"), count("missing")),
    });

    // Each span ranks against the same span of earlier years, so a window
    // still open never meets whole seasons.
    let model_days = |n: usize| {
        days[..n]
            .iter()
            .filter(|d| matches!(d.source, "model_analysis" | "forecast"))
            .count()
    };
    let spans: Vec<JsonValue> = span_lengths(&days)
        .into_iter()
        .map(|n| {
            let this = r2(gdds[..n].iter().flatten().sum());
            let by_year = year_totals(&reanalysis, first, n as i64, p.years, p.base, p.method);
            let mut o = json!({
                "from": iso(first),
                "to": iso(first + n as i64 - 1),
                "days": n,
                "whole_window": n as i64 == len,
                "model_days": model_days(n),
                "gdd": this,
            });
            if let (Some(o), JsonValue::Object(r)) =
                (o.as_object_mut(), rank_among_years(this, &by_year))
            {
                o.extend(r);
            }
            o
        })
        .collect();
    let climatology = if p.years > 0 {
        json!({
            "years_requested": p.years,
            "from_year": crate::civil_from_days(years_before(first, p.years)).0,
            "to_year": crate::civil_from_days(years_before(first, 1)).0,
            "source": "ERA5-Land, and ERA5 on days ERA5-Land has no value: the rule the window's past days follow",
            "rule": "Each earlier year gives one total: the span that starts on the same calendar day (29 February moves to the 28th) and has the same number of days, under the same GDD rule. A year counts only when every day of its span has a value. non_exceedance is the share of those years whose total is below this one, ties split, with the Weibull i/(n+1) offset.",
            "spans": spans,
        })
    } else {
        JsonValue::Null
    };

    let reanalysis_through = days
        .iter()
        .filter(|d| matches!(d.source, "era5_land" | "era5"))
        .map(|d| d.day)
        .max();
    let era5_days = count("era5");
    let mut limits = vec![
        "Every value is the reanalysis grid cell's, not the field's own. An ERA5-Land cell is 0.1 degree \
         (about 11 km north to south), an ERA5 cell 0.25 degree (about 28 km). Open-Meteo adjusts the \
         cell's temperature to the height of the point from a 90 m elevation model; irrigation, canopy, \
         slope and aspect inside the cell do not enter. Near a coast Open-Meteo can answer from the \
         nearest ERA5-Land land cell instead, which may be several km away; `grid` names the cell that \
         answered for each source."
            .to_string(),
        "Tmax and Tmin are the highest and lowest of the local day's 24 hourly values. A min-max \
         thermometer also catches the extremes between the hours, so a station's daily range is usually \
         a little wider."
            .to_string(),
    ];
    if analysis_n + forecast_n > 0 {
        let reached = match reanalysis_through {
            Some(r) => format!(
                "The reanalysis reached {} on this call, {} day(s) before {}.",
                iso(r),
                today - r,
                iso(today)
            ),
            None => "The reanalysis has no day of this window yet.".to_string(),
        };
        limits.push(format!(
            "{reached} The {analysis_n} day(s) after it and before {} are the forecast model's own \
             analysis, and the {forecast_n} day(s) from {} on are its forecast, issued when this record \
             was signed. Both are model output on the forecast model's grid, not reanalysis.",
            iso(today),
            iso(today)
        ));
    } else {
        limits.push(
            "ERA5-Land and ERA5 arrive some days after the day they describe. A window that reaches \
             the days they do not cover yet takes the forecast model's analysis for those days. This \
             window ends before them, so no day here is model analysis or forecast."
                .to_string(),
        );
    }
    if era5_days > 0 {
        limits.push(format!(
            "{era5_days} day(s) come from ERA5 because ERA5-Land has no value there. ERA5-Land covers land \
             only, so a point on water that no land cell answers for falls back to the coarser grid."
        ));
    }
    let ranked_model_days = spans
        .iter()
        .filter_map(|s| s["model_days"].as_u64())
        .max()
        .unwrap_or(0);
    if p.years > 0 && ranked_model_days > 0 {
        limits.push(format!(
            "A ranked span here holds up to {ranked_model_days} day(s) of model analysis or forecast, \
             and the earlier years it meets are reanalysis only."
        ));
    }

    let grid_of = |b: &JsonValue| json!({"lat": b["latitude"], "lng": b["longitude"], "downscaled_to_elevation_m": b["elevation"]});
    let mut sources = Vec::new();
    if land_body.is_some() {
        sources.push(json!({
            "name": "ERA5-Land (Copernicus Climate Change Service; Munoz-Sabater et al. 2021, \
                     doi:10.5194/essd-13-4349-2021), daily values from the Open-Meteo historical weather API",
            "model": "era5_land", "grid_deg": 0.1, "url": land_url, "licence": "CC BY 4.0",
            "role": "past days and the climatology",
            "window_days": count("era5_land"),
        }));
    }
    if let Some(u) = &era5_url {
        sources.push(json!({
            "name": "ERA5 (Copernicus Climate Change Service; Hersbach et al. 2020, doi:10.1002/qj.3803), \
                     daily values from the Open-Meteo historical weather API",
            "model": "era5", "grid_deg": 0.25, "url": u, "licence": "CC BY 4.0",
            "role": "days ERA5-Land has no value for",
            "window_days": era5_days,
        }));
    }
    if fc_body.is_some() {
        sources.push(json!({
            "name": "Open-Meteo forecast API (best-match weather model for the location)",
            "url": forecast_url, "licence": "CC BY 4.0",
            "role": "days the reanalysis has not reached, and days ahead",
            "window_days": analysis_n + forecast_n,
        }));
    }

    let gdd_rule = match p.method {
        Method::Average => json!({
            "method": "average", "base_c": p.base,
            "formula": "max(0, (tmax_c + tmin_c) / 2 - base_c)",
            "reference": "McMaster and Wilhelm 1997, method 1",
        }),
        Method::Capped { cap } => json!({
            "method": "capped", "base_c": p.base, "cap_c": cap,
            "formula": "(clamp(tmax_c, base_c, cap_c) + clamp(tmin_c, base_c, cap_c)) / 2 - base_c",
            "reference": "McMaster and Wilhelm 1997, method 2; at base 10 and cap 30 this is the 86/50 F maize rule",
        }),
    };
    let mut gdd_rule = gdd_rule;
    gdd_rule["citation"] = json!("McMaster, G.S. and Wilhelm, W.W. (1997). Growing degree-days: one equation, two interpretations. Agricultural and Forest Meteorology 87, 291-300.");
    gdd_rule["heat_c"] = json!(p.heat);
    gdd_rule["frost_c"] = json!(FROST_C);
    gdd_rule["heat_stress_day"] = json!("tmax_c >= heat_c");
    gdd_rule["frost_day"] = json!("tmin_c <= frost_c");

    let asked = match p.window {
        Window::Dates { .. } => json!({"start": iso(first), "end": iso(last)}),
        Window::Recent { past, ahead } => json!({"past_days": past, "forecast_days": ahead}),
    };
    let record = json!({
        "schema": "emem.temperature_series.v1",
        "fn_key": FN_KEY,
        "location": {"lat": lat, "lng": lng, "cell": cell},
        "grid": {
            "era5_land": land_body.as_ref().map(grid_of),
            "era5": era5_body.as_ref().map(grid_of),
            "forecast": fc_body.as_ref().map(grid_of),
        },
        "day_boundary": {
            "days": "local",
            "timezone": zone_body.and_then(|b| b.get("timezone")).cloned(),
            "utc_offset_seconds": zone_body.and_then(offset_of),
            "why": "Tmax and Tmin are the extremes of a local day, and growing degree-day conventions count \
                    local days. A UTC day splits the local night wherever the offset is not zero. Days follow \
                    the zone's civil calendar, so where clocks change for summer time two days a year are 23 \
                    or 25 hours long.",
        },
        "issued": iso(today),
        "window": {"from": iso(first), "to": iso(last), "days": len, "asked_as": asked},
        "reanalysis_through": reanalysis_through.map(iso),
        "gdd_rule": gdd_rule,
        "days": rows,
        "totals": totals,
        "climatology": climatology,
        "estimand": {
            "quantity": "air temperature 2 m above the ground at the point's grid cell, per local day",
            "unit": "degC",
            "statistic": "tmax_c and tmin_c are the highest and lowest of the day's 24 hourly values and \
                          tmean_c their mean; gdd comes from tmax_c and tmin_c by gdd_rule",
        },
        "sources": sources,
        "limits": limits,
    });
    let what = SeriesFact {
        cell: &cell,
        band: "temperature.series",
        fn_key: FN_KEY,
        receipt_kind: "emem.temperature_series",
        days: (first, last),
    };
    sign_record(s, record, what, started).await
}

/// The MCP form: each day becomes one array in the order `days_columns`
/// names and each run of days from one source is one `day_sources` entry. A
/// host carries a tool result twice within about 24 KB, and a 150-day season
/// as day objects does not fit; with this it does. Longer windows can still
/// be cut by the slimmer, so `_projection.rest` holds the exact request that
/// returns the whole.
pub(crate) fn mcp_projection(mut v: JsonValue, args: JsonValue) -> JsonValue {
    const COLUMNS: [&str; 6] = ["date", "tmax_c", "tmin_c", "tmean_c", "gdd", "gdd_cum"];
    let mut runs: Vec<JsonValue> = Vec::new();
    if let Some(rows) = v.get_mut("days").and_then(|d| d.as_array_mut()) {
        for r in rows.iter_mut() {
            let (src, date) = (r["source"].clone(), r["date"].clone());
            match runs.last_mut() {
                Some(run) if run["source"] == src => {
                    run["to"] = date;
                    run["days"] = json!(run["days"].as_u64().unwrap_or(0) + 1);
                }
                _ => runs.push(json!({"source": src, "from": date, "to": date, "days": 1})),
            }
            *r = JsonValue::Array(
                COLUMNS
                    .iter()
                    .map(|k| r.get(*k).cloned().unwrap_or(JsonValue::Null))
                    .collect(),
            );
        }
    }
    v["days_columns"] = json!(COLUMNS);
    v["day_sources"] = json!(runs);
    for (parent, key) in [
        ("day_boundary", "why"),
        ("gdd_rule", "citation"),
        ("climatology", "source"),
    ] {
        if let Some(o) = v.get_mut(parent).and_then(|p| p.as_object_mut()) {
            o.remove(key);
        }
    }
    if let Some(o) = v.as_object_mut() {
        o.remove("estimand");
    }
    if let Some(list) = v.get_mut("sources").and_then(|s| s.as_array_mut()) {
        for s in list.iter_mut().filter_map(|s| s.as_object_mut()) {
            s.remove("url");
        }
    }
    v["_projection"] = json!({
        "days": "each day is an array in the order days_columns names; a forecast day's lead is its date minus `issued`",
        "left_out": "day_boundary.why, gdd_rule.citation, climatology.source, estimand and the source URLs; the signed record has them",
        "whole_at": "resolve `token` with emem_memory_token_resolve for the signed record, or send `rest`",
        "rest": {"method": "POST", "path": "/v1/temperature_series", "body": args},
    });
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(v: JsonValue) -> TemperatureSeriesReq {
        serde_json::from_value(v).unwrap()
    }

    fn day(y: i32, m: u32, d: u32) -> i64 {
        crate::days_from_civil(y, m, d)
    }

    fn t(max: f64, min: f64) -> Temps {
        Temps {
            max,
            min,
            mean: None,
        }
    }

    #[test]
    fn average_gdd_on_worked_days() {
        let m = Method::Average;
        // (30 + 20) / 2 - 10 = 15.
        assert_eq!(gdd(30.0, 20.0, 10.0, m), 15.0);
        // Tmin under the base still counts in method 1: (18 + 6) / 2 - 10 = 2.
        assert_eq!(gdd(18.0, 6.0, 10.0, m), 2.0);
        // A mean under the base is zero, never negative: (12 + 4) / 2 = 8.
        assert_eq!(gdd(12.0, 4.0, 10.0, m), 0.0);
        // A hot day is not capped: (40 + 28) / 2 - 10 = 24.
        assert_eq!(gdd(40.0, 28.0, 10.0, m), 24.0);
        // Ludhiana, 2025-06-15, ERA5-Land: (39.9 + 29.2) / 2 - 10 = 24.55.
        assert_eq!(r2(gdd(39.9, 29.2, 10.0, m)), 24.55);
    }

    #[test]
    fn an_empty_total_is_plain_zero() {
        let empty: f64 = std::iter::empty::<f64>().sum();
        assert!(r2(empty).is_sign_positive());
        assert_eq!(serde_json::to_string(&json!(r2(empty))).unwrap(), "0.0");
    }

    #[test]
    fn capped_gdd_on_worked_days() {
        let m = Method::Capped { cap: 30.0 };
        // Tmax over the cap: (30 + 22) / 2 - 10 = 16, where method 1 gives 19.
        assert_eq!(gdd(36.0, 22.0, 10.0, m), 16.0);
        // Tmin under the base is raised to it: (18 + 10) / 2 - 10 = 4, not 2.
        assert_eq!(gdd(18.0, 6.0, 10.0, m), 4.0);
        // Both under the base: zero.
        assert_eq!(gdd(8.0, 2.0, 10.0, m), 0.0);
        // Both over the cap: (30 + 30) / 2 - 10 = 20, the most a day can add.
        assert_eq!(gdd(34.0, 31.0, 10.0, m), 20.0);
        // Ludhiana, 2025-06-15: (30 + 29.2) / 2 - 10 = 19.6.
        assert_eq!(r2(gdd(39.9, 29.2, 10.0, m)), 19.6);
        // Another base and cap: (min(33, 34) + max(5, 8)) / 2 - 8 = 12.5.
        assert_eq!(gdd(33.0, 5.0, 8.0, Method::Capped { cap: 34.0 }), 12.5);
    }

    #[test]
    fn a_window_is_dates_or_recent_days() {
        let today = day(2026, 11, 20);
        let w = params(
            &req(json!({"start": "2025-06-15", "end": "2025-10-15"})),
            today,
        )
        .unwrap();
        assert_eq!(
            w.window,
            Window::Dates {
                first: day(2025, 6, 15),
                last: day(2025, 10, 15)
            }
        );
        assert_eq!(
            (w.base, w.method, w.heat, w.years),
            (10.0, Method::Average, 35.0, 10)
        );
        let r = params(&req(json!({})), today).unwrap();
        assert_eq!(r.window, Window::Recent { past: 30, ahead: 7 });
        assert_eq!(r.window.days(today), (day(2026, 10, 21), day(2026, 11, 26)));
        // No days ahead ends the window yesterday.
        let p = params(&req(json!({"past_days": 5, "forecast_days": 0})), today).unwrap();
        assert_eq!(p.window.days(today), (today - 5, today - 1));
        let capped = params(&req(json!({"past_days": 5, "method": "capped"})), today).unwrap();
        assert_eq!(capped.method, Method::Capped { cap: 30.0 });
    }

    #[test]
    fn a_bad_window_is_refused_with_its_reason() {
        let today = day(2026, 11, 20);
        let err = |v: JsonValue| params(&req(v), today).unwrap_err();
        assert!(err(json!({"start": "2025-06-15"})).contains("both"));
        assert!(
            err(json!({"start": "2025-06-15", "end": "2025-06-20", "past_days": 3}))
                .contains("not both")
        );
        assert!(err(json!({"start": "2025-02-30", "end": "2025-03-20"})).contains("YYYY-MM-DD"));
        assert!(err(json!({"start": "2025-6-1", "end": "2025-06-20"})).contains("YYYY-MM-DD"));
        assert!(err(json!({"start": "1949-12-31", "end": "1950-01-20"})).contains("1950-01-01"));
        assert!(err(json!({"start": "2025-06-20", "end": "2025-06-15"})).contains("before"));
        assert!(err(json!({"start": "2024-01-01", "end": "2025-01-01"})).contains("367 days"));
        assert!(
            err(json!({"start": "2026-12-07", "end": "2026-12-10"})).contains("forecast reaches")
        );
        assert!(err(json!({"past_days": 0})).contains("past_days"));
        assert!(err(json!({"past_days": 93})).contains("past_days"));
        assert!(err(json!({"forecast_days": 17})).contains("forecast_days"));
        assert!(err(json!({"base_c": 21})).contains("base_c"));
        assert!(err(json!({"method": "triangle"})).contains("method"));
        assert!(err(json!({"cap_c": 30})).contains("capped"));
        assert!(
            err(json!({"method": "capped", "base_c": 12, "cap_c": 12})).contains("above base_c")
        );
        assert!(err(json!({"method": "capped", "cap_c": 46})).contains("at most 45"));
        assert!(err(json!({"heat_c": 24})).contains("heat_c"));
        assert!(err(json!({"climatology_years": 31})).contains("climatology_years"));
        // A leap year of 366 days is the longest window.
        assert!(params(
            &req(json!({"start": "2024-01-01", "end": "2024-12-31"})),
            today
        )
        .is_ok());
        // The last day the forecast reaches may start a window.
        assert!(params(
            &req(json!({"start": "2026-12-05", "end": "2026-12-10"})),
            today
        )
        .is_ok());
        assert!(serde_json::from_value::<TemperatureSeriesReq>(json!({"base": 8})).is_err());
    }

    #[test]
    fn each_day_takes_the_first_source_that_has_it() {
        let land: HashMap<i64, Temps> = [(1, t(30.0, 20.0))].into_iter().collect();
        let era5: HashMap<i64, Temps> = [(1, t(29.0, 19.0)), (2, t(28.0, 18.0))]
            .into_iter()
            .collect();
        let model: HashMap<i64, Temps> =
            [(2, t(27.0, 17.0)), (3, t(26.0, 16.0)), (5, t(25.0, 15.0))]
                .into_iter()
                .collect();
        // Today is day 5: days 1 to 4 are past, 5 and 6 ahead.
        let got: Vec<_> = merge_days(1, 6, 5, &land, &era5, &model)
            .iter()
            .map(|d| (d.day, d.t.map(|t| t.max), d.source, d.lead))
            .collect();
        assert_eq!(
            got,
            vec![
                (1, Some(30.0), "era5_land", None),
                (2, Some(28.0), "era5", None),
                (3, Some(26.0), "model_analysis", None),
                (4, None, "missing", None),
                (5, Some(25.0), "forecast", Some(0)),
                (6, None, "not_yet", None),
            ]
        );
    }

    #[test]
    fn open_meteo_days_need_both_extremes() {
        let body = json!({"daily": {
            "time": ["2025-06-15", "2025-06-16", "2025-06-17"],
            "temperature_2m_max": [39.9, null, 27.9],
            "temperature_2m_min": [29.2, 27.4, 24.9],
            "temperature_2m_mean": [34.5, 31.8, null],
        }});
        let got = daily_temps(&body).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(
            got[&day(2025, 6, 15)],
            Temps {
                max: 39.9,
                min: 29.2,
                mean: Some(34.5)
            }
        );
        assert_eq!(got[&day(2025, 6, 17)].mean, None);
        assert!(daily_temps(&json!({"daily": {"time": []}})).is_err());
    }

    #[test]
    fn earlier_years_total_the_same_calendar_span() {
        // Every day of 2020 to 2025 at a steady 25/15: 10 GDD a day.
        let by: HashMap<i64, Temps> = (day(2020, 1, 1)..=day(2025, 12, 31))
            .map(|d| (d, t(25.0, 15.0)))
            .collect();
        let got = year_totals(&by, day(2026, 6, 15), 30, 5, 10.0, Method::Average);
        assert_eq!(
            got,
            vec![
                (2021, 300.0),
                (2022, 300.0),
                (2023, 300.0),
                (2024, 300.0),
                (2025, 300.0)
            ]
        );
        // 2020 has a gap in its span, so it is left out rather than short.
        let mut holed = by.clone();
        holed.remove(&day(2020, 6, 20));
        let got = year_totals(&holed, day(2026, 6, 15), 30, 6, 10.0, Method::Average);
        assert_eq!(got.first().map(|(y, _)| *y), Some(2021));
        assert_eq!(got.len(), 5);
        // 29 February starts on the 28th in a year without it.
        assert_eq!(years_before(day(2024, 2, 29), 1), day(2023, 2, 28));
        // Years that start before 1950 are left out even where a value exists.
        let early: HashMap<i64, Temps> = (day(1945, 1, 1)..=day(1951, 12, 31))
            .map(|d| (d, t(25.0, 15.0)))
            .collect();
        let got = year_totals(&early, day(1952, 3, 1), 10, 5, 10.0, Method::Average);
        assert_eq!(got, vec![(1950, 100.0), (1951, 100.0)]);
    }

    #[test]
    fn a_total_ranks_among_the_years() {
        let years: Vec<(i32, f64)> = (2016..=2025)
            .map(|y| (y, (y - 2015) as f64 * 100.0))
            .collect();
        // 550 is above five of ten years: (5 + 0.5) / 11.
        let r = rank_among_years(550.0, &years);
        assert_eq!(r["years_used"], 10);
        assert_eq!(r["years_below"], 5);
        assert_eq!(r["years_above"], 5);
        assert_eq!(r["non_exceedance"], 0.5);
        assert_eq!(r["mean_gdd"], 550.0);
        assert_eq!(r["min_gdd"], 100.0);
        assert_eq!(r["max_gdd"], 1000.0);
        assert_eq!(r["anomaly_gdd"], 0.0);
        // Above every year is (10 + 0.5) / 11, never 1.
        assert_eq!(rank_among_years(5000.0, &years)["non_exceedance"], 0.9545);
        let none = rank_among_years(100.0, &[]);
        assert!(none["non_exceedance"].is_null());
    }

    #[test]
    fn an_open_window_says_its_totals_are_partial() {
        let open = status(true, day(2026, 12, 1), 5, 7, 3, 0);
        assert!(open.contains("still open"), "{open}");
        assert!(open.contains("not final"), "{open}");
        assert!(open.contains("5 recent day(s)"), "{open}");
        assert!(open.contains("7 day(s) are forecast"), "{open}");
        assert!(open.contains("3 day(s) have no value yet"), "{open}");
        let closed = status(false, day(2025, 10, 15), 0, 0, 0, 0);
        assert!(closed.contains("are final"), "{closed}");
        // Closed, but its last days are model analysis: they can still move.
        let recent = status(false, day(2026, 11, 19), 6, 0, 0, 0);
        assert!(!recent.contains("are final"), "{recent}");
        let holed = status(false, day(2025, 10, 15), 0, 0, 0, 2);
        assert!(!holed.contains("are final"), "{holed}");
    }

    #[test]
    fn a_partial_window_ranks_only_the_span_it_has() {
        // A 120-day season from 2026-06-15 with today at day 41: 40 days of
        // reanalysis, 16 of forecast, then 64 with no value yet.
        let first = day(2026, 6, 15);
        let today = first + 40;
        let land: HashMap<i64, Temps> = (first..today).map(|d| (d, t(30.0, 20.0))).collect();
        let model: HashMap<i64, Temps> = (today..today + 16).map(|d| (d, t(32.0, 22.0))).collect();
        let days = merge_days(first, first + 119, today, &land, &HashMap::new(), &model);
        assert_eq!(span_lengths(&days), vec![40, 56]);
        // The earlier years total 40 days, not 120.
        let by: HashMap<i64, Temps> = (day(2015, 1, 1)..=day(2025, 12, 31))
            .map(|d| (d, t(30.0, 20.0)))
            .collect();
        let got = year_totals(&by, first, 40, 10, 10.0, Method::Average);
        assert_eq!(got.len(), 10);
        assert!(got.iter().all(|(_, total)| *total == 600.0), "{got:?}");
        // A closed, complete window is one span, the whole window.
        let closed = merge_days(first, first + 29, today, &land, &HashMap::new(), &model);
        assert_eq!(span_lengths(&closed), vec![30]);
        // A window that starts today ranks its forecast span alone.
        let ahead = merge_days(today, today + 20, today, &land, &HashMap::new(), &model);
        assert_eq!(span_lengths(&ahead), vec![16]);
    }

    #[test]
    fn mcp_days_are_rows_of_columns() {
        let row = |date: &str, source: &str| {
            json!({"date": date, "tmax_c": 30.0, "tmin_c": 20.0, "tmean_c": 25.0,
                   "gdd": 15.0, "gdd_cum": 15.0, "source": source})
        };
        let v = json!({
            "days": [row("2026-11-19", "era5_land"), row("2026-11-20", "model_analysis"),
                     row("2026-11-21", "model_analysis"), row("2026-11-22", "forecast")],
            "day_boundary": {"days": "local", "why": "long prose"},
            "estimand": {"unit": "degC"},
            "sources": [{"name": "ERA5-Land", "url": "https://archive-api.open-meteo.com/v1/archive?x"}],
        });
        let args = json!({"lat": 18.52, "lng": 73.86, "past_days": 3, "forecast_days": 1});
        let p = mcp_projection(v, args.clone());
        assert_eq!(
            p["days"][0],
            json!(["2026-11-19", 30.0, 20.0, 25.0, 15.0, 15.0])
        );
        assert_eq!(p["days_columns"][4], "gdd");
        assert_eq!(
            p["day_sources"],
            json!([
                {"source": "era5_land", "from": "2026-11-19", "to": "2026-11-19", "days": 1},
                {"source": "model_analysis", "from": "2026-11-20", "to": "2026-11-21", "days": 2},
                {"source": "forecast", "from": "2026-11-22", "to": "2026-11-22", "days": 1},
            ])
        );
        assert!(p["day_boundary"].get("why").is_none());
        assert!(p.get("estimand").is_none());
        assert!(p["sources"][0].get("url").is_none());
        assert_eq!(p["sources"][0]["name"], "ERA5-Land");
        assert_eq!(p["_projection"]["rest"]["body"], args);
    }
}
