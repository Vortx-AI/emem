//! The persistence chain, `POST /v1/persist`: did the state one clear pass
//! saw hold to a later date?
//!
//! A clear pass says what was on the ground that day and nothing after it.
//! The nearest pass, a radar pass or one model value each answer part of a
//! dark-month question, and none says whether the snow seen on 31 October was
//! still there on 3 December. That needs the observation and the weather
//! between, read together. This module reads them together under a declared
//! rule and signs the result as a derivation whose every input is cited: the
//! observation's fact, the modelled depth fact of the hour asked, and the
//! upstream hourly series as a content-addressed artifact a checker re-reads.
//!
//! Two melt readings are kept apart on purpose. ERA5-Land's snow model and a
//! degree-day bucket disagree in the polar autumn, where the sun is too low
//! for a degree above zero to melt what a spring degree-day melts. The rule
//! calls a disagreement UNKNOWN and shows both, rather than choose one.

use std::collections::BTreeMap;
use std::time::Instant;

use axum::extract::State;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value as JsonValue};

use emem_fact::{Derivation, DerivativeFact, Fact, FactCid};

use crate::band_raster::{bad_request, upstream_error, ARTIFACTS};
use crate::change_attribution::json_to_cbor;
use crate::{ApiError, AppState, EmemJson};

pub(crate) const FN_KEY: &str = "persistence_snow@1";
const NDSI_SNOW: f64 = 0.4;
const NDSI_NO_SNOW: f64 = 0.0;
/// Under this depth the modelled pack has melted out.
const MELT_OUT_M: f64 = 0.01;
/// At or above this depth the model holds snow on the ground (the ask rule).
const SNOW_ON_GROUND_M: f64 = 0.05;
/// ERA5-Land holds a fixed pack of about 27 to 33 m on glacier cells all
/// year; seasonal snow in the model stays under a few metres.
pub(crate) const GLACIER_PACK_M: f64 = 10.0;
/// Snow melt per positive degree-day, mm water equivalent: the low end of
/// the range for snow in Hock (2003).
const DDF_MM: f64 = 3.0;
/// Turns the modelled starting depth into water equivalent, kg per m^3.
const START_DENSITY: f64 = 300.0;
/// Open-Meteo states snowfall in cm at 7 cm per 10 mm of water.
const SNOWFALL_CM_PER_MM: f64 = 0.7;
/// A series with more missing hours than this share answers UNKNOWN.
const MAX_MISSING_SHARE: f64 = 0.1;
/// Longest window read, in days.
pub(crate) const MAX_DAYS: i64 = 200;

pub(crate) const RULE_TEXT: &str = "persistence_snow@1. The start state is the observation's: NDSI 0.4 or more is snow, under 0 no snow, between them snow or cloud. Two melt readings run from the observation hour to the hour asked. The model reading is ERA5-Land snow depth: the pack has melted out at the first hour under 0.01 m. The degree-day reading is a bucket that starts at the modelled depth at the observation times 300 kg/m^3, gains ERA5 snowfall (7 cm per 10 mm of water) and loses 3 mm of water per positive degree-day of ERA5-Land 2 m temperature (hourly, summed over 24): it has melted out when it reaches zero. Snow at the start: both keep a pack, PERSISTED_LIKELY; both melt out, RENEWED when the model holds 0.05 m or more at the hour asked, else BROKEN; they disagree, UNKNOWN. No snow at the start: the first hour the model holds 0.05 m or more breaks it, else PERSISTED_LIKELY. UNKNOWN as well when the observation read snow or cloud, when the model held no pack under a snow observation, when any modelled depth is 10 m or more (a glacier in the model), or when more than a tenth of any series is missing.";

#[derive(Debug, Deserialize)]
pub struct PersistReq {
    #[serde(default, alias = "cell64")]
    pub cell: Option<String>,
    #[serde(default)]
    pub lat: Option<f64>,
    #[serde(default)]
    pub lng: Option<f64>,
    /// `snow`, the only state this rule reads today.
    #[serde(default)]
    pub quantity: Option<String>,
    /// The day asked, `YYYY-MM-DD`.
    pub date: String,
    /// UTC hour of the day asked, 0 to 23. Default 12.
    #[serde(default)]
    pub hour: Option<u32>,
    /// The `fact_cid` of an `indices.ndsi` fact at this cell to start from.
    /// Default: the last pass emem reads at or before the day asked.
    #[serde(default)]
    pub observation: Option<String>,
}

/// The observation a chain starts from.
#[derive(Debug, Clone)]
pub(crate) struct Obs {
    pub(crate) fact_cid: String,
    pub(crate) band: String,
    pub(crate) value: f64,
    pub(crate) unix: i64,
}

fn iso_hour(u: i64) -> String {
    let s = crate::iso8601_utc(u.max(0) as u64);
    format!("{}:00Z", &s[..13])
}

fn iso_day(u: i64) -> String {
    crate::iso8601_utc(u.max(0) as u64)[..10].to_string()
}

fn r2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

pub(crate) fn cbor_f64(v: &ciborium::Value) -> Option<f64> {
    match v {
        ciborium::Value::Float(f) => Some(*f),
        ciborium::Value::Integer(i) => i128::from(*i).to_string().parse().ok(),
        _ => None,
    }
}

/// When a signed fact was observed: its source's capture time, else the
/// middle of its time slot.
pub(crate) fn observed_unix(p: &emem_fact::PrimaryFact) -> Option<i64> {
    p.sources
        .iter()
        .find_map(|s| s.captured_at.as_deref())
        .and_then(crate::parse_iso8601_unix)
        .or_else(|| {
            let tempo = crate::band_tempo_for_key(&p.band)?;
            let slot = tempo.slot_seconds() as i64;
            Some(p.tslot as i64 * slot + slot / 2)
        })
}

/// The hourly series of one Open-Meteo archive response, both models.
#[derive(Debug, Default)]
pub(crate) struct Series {
    pub(crate) hours: Vec<i64>,
    pub(crate) t2m: Vec<Option<f64>>,
    pub(crate) snowfall: Vec<Option<f64>>,
    pub(crate) depth: Vec<Option<f64>>,
}

pub(crate) fn parse_series(body: &JsonValue) -> Result<Series, String> {
    let arr = |k: &str| -> Result<Vec<Option<f64>>, String> {
        Ok(body
            .pointer(&format!("/hourly/{k}"))
            .and_then(|v| v.as_array())
            .ok_or_else(|| format!("archive response has no hourly.{k}"))?
            .iter()
            .map(|v| v.as_f64())
            .collect())
    };
    let hours: Vec<i64> = body
        .pointer("/hourly/time")
        .and_then(|v| v.as_array())
        .ok_or("archive response has no hourly.time")?
        .iter()
        .map(|t| {
            t.as_str()
                .and_then(crate::parse_iso_utc_hour)
                .unwrap_or(i64::MIN)
        })
        .collect();
    let out = Series {
        t2m: arr("temperature_2m_era5_land")?,
        snowfall: arr("snowfall_era5")?,
        depth: arr("snow_depth_era5_land")?,
        hours,
    };
    let n = out.hours.len();
    if out.t2m.len() != n || out.snowfall.len() != n || out.depth.len() != n {
        return Err("archive series have different lengths".into());
    }
    Ok(out)
}

pub(crate) fn series_url(lat: f64, lng: f64, from_unix: i64, to_unix: i64) -> String {
    format!(
        "https://archive-api.open-meteo.com/v1/archive?latitude={lat:.4}&longitude={lng:.4}&hourly=temperature_2m,snowfall,snow_depth&start_date={}&end_date={}&timezone=UTC&models=era5,era5_land",
        iso_day(from_unix),
        iso_day(to_unix)
    )
}

#[derive(Debug, Default)]
struct Day {
    t_max: Option<f64>,
    thaw_hours: u32,
    pdd: f64,
    snowfall: f64,
    depth_min: Option<f64>,
    depth_max: Option<f64>,
}

fn state_of_ndsi(v: f64) -> &'static str {
    if v >= NDSI_SNOW {
        "snow"
    } else if v < NDSI_NO_SNOW {
        "no_snow"
    } else {
        "snow_or_cloud"
    }
}

/// The chain over one series, under [`RULE_TEXT`]. Pure: the checker in
/// `tools/film_claims.py` recomputes the same numbers from the same bytes.
pub(crate) fn judge(obs: &Obs, to_unix: i64, series: &Series) -> JsonValue {
    let obs_hour = obs.unix.div_euclid(3600) * 3600;
    let to_hour = to_unix.div_euclid(3600) * 3600;
    let at = |h: i64| series.hours.iter().position(|x| *x == h);
    let start = at(obs_hour);
    let window: Vec<usize> = series
        .hours
        .iter()
        .enumerate()
        .filter(|(_, h)| **h > obs_hour && **h <= to_hour)
        .map(|(i, _)| i)
        .collect();
    let n = window.len();
    let missing = |v: &Vec<Option<f64>>| window.iter().filter(|i| v[**i].is_none()).count();
    let gaps = [
        ("ERA5-Land 2 m temperature", missing(&series.t2m)),
        ("ERA5 snowfall", missing(&series.snowfall)),
        ("ERA5-Land snow depth", missing(&series.depth)),
    ];

    let depth_start = start.and_then(|i| series.depth[i]);
    let depth_end = at(to_hour).and_then(|i| series.depth[i]);
    let mut days: BTreeMap<String, Day> = BTreeMap::new();
    let (mut thaw_hours, mut pdd, mut snow_total) = (0u32, 0.0f64, 0.0f64);
    let mut t_max: Option<(f64, i64)> = None;
    let mut d_min: Option<(f64, i64)> = None;
    let mut d_max: Option<f64> = None;
    let mut last_thaw: Option<i64> = None;
    let mut model_out: Option<i64> = None;
    let mut model_snow_at: Option<i64> = None;
    let mut glacier = depth_start.is_some_and(|d| d >= GLACIER_PACK_M);
    let mut swe = depth_start.map_or(0.0, |d| d * START_DENSITY);
    let swe_start = swe;
    let mut dd_out: Option<i64> = None;
    for &i in &window {
        let h = series.hours[i];
        let day = days.entry(iso_day(h)).or_default();
        if let Some(t) = series.t2m[i] {
            day.t_max = Some(day.t_max.map_or(t, |m: f64| m.max(t)));
            if t_max.is_none_or(|(m, _)| t > m) {
                t_max = Some((t, h));
            }
            if t > 0.0 {
                thaw_hours += 1;
                day.thaw_hours += 1;
                pdd += t / 24.0;
                day.pdd += t / 24.0;
                last_thaw = Some(h);
            }
        }
        let fall = series.snowfall[i].unwrap_or(0.0);
        snow_total += fall;
        day.snowfall += fall;
        if let Some(d) = series.depth[i] {
            day.depth_min = Some(day.depth_min.map_or(d, |m: f64| m.min(d)));
            day.depth_max = Some(day.depth_max.map_or(d, |m: f64| m.max(d)));
            if d_min.is_none_or(|(m, _)| d < m) {
                d_min = Some((d, h));
            }
            d_max = Some(d_max.map_or(d, |m: f64| m.max(d)));
            glacier |= d >= GLACIER_PACK_M;
            if model_out.is_none() && d < MELT_OUT_M {
                model_out = Some(h);
            }
            if model_snow_at.is_none() && d >= SNOW_ON_GROUND_M {
                model_snow_at = Some(h);
            }
        }
        // The bucket keeps running after it melts out, so a pack built by
        // later snowfall shows in swe_at_date_mm.
        let gain = fall / SNOWFALL_CM_PER_MM;
        let melt = series.t2m[i].map_or(0.0, |t| t.max(0.0) * DDF_MM / 24.0);
        let before = swe;
        swe = (swe + gain - melt).max(0.0);
        if dd_out.is_none() && before > 0.0 && swe <= 0.0 {
            dd_out = Some(h);
        }
    }
    let after = |t: Option<i64>| -> f64 {
        t.map_or(0.0, |t| {
            window
                .iter()
                .filter(|i| series.hours[**i] > t)
                .map(|i| series.snowfall[*i].unwrap_or(0.0))
                .sum()
        })
    };
    let since_thaw = after(last_thaw);
    let start_state = state_of_ndsi(obs.value);
    let gap_worst = gaps.iter().map(|g| g.1).max().unwrap_or(0);

    let day_phrase = |h: i64| iso_day(h);
    let (verdict, reason): (&str, String) = if n == 0 {
        (
            "unknown",
            "the hour asked is not after the observation".into(),
        )
    } else if glacier {
        (
            "unknown",
            format!(
                "the model holds a pack of {} m or more here, which in ERA5-Land is a glacier or a perennial accumulation held all year",
                GLACIER_PACK_M
            ),
        )
    } else if (gap_worst as f64) > MAX_MISSING_SHARE * n as f64 {
        let g = gaps.iter().max_by_key(|g| g.1).map(|g| g.0).unwrap_or("");
        (
            "unknown",
            format!("{gap_worst} of {n} hours of {g} are missing"),
        )
    } else if start_state == "snow_or_cloud" {
        (
            "unknown",
            format!(
                "the pass read NDSI {}, which is snow or cloud",
                r2(obs.value)
            ),
        )
    } else if start_state == "snow" && depth_start.is_none_or(|d| d < MELT_OUT_M) {
        (
            "unknown",
            "the model held no pack at the hour the pass read snow".into(),
        )
    } else if start_state == "snow" {
        match (model_out, dd_out) {
            (None, None) => (
                "persisted_likely",
                "both melt readings keep a pack through the window".into(),
            ),
            (Some(m), Some(d)) => {
                let later = m.max(d);
                let fallen = after(Some(later));
                if depth_end.is_some_and(|x| x >= SNOW_ON_GROUND_M) {
                    (
                        "renewed",
                        format!(
                            "both melt readings melt it out (the model on {}, the degree-day bucket on {}), and {} cm of snow fell after the later",
                            day_phrase(m),
                            day_phrase(d),
                            r2(fallen)
                        ),
                    )
                } else {
                    (
                        "broken",
                        format!(
                            "both melt readings melt it out (the model on {}, the degree-day bucket on {})",
                            day_phrase(m),
                            day_phrase(d)
                        ),
                    )
                }
            }
            (Some(m), None) => (
                "unknown",
                format!(
                    "the two melt readings disagree: the model melts it out on {}, the degree-day bucket keeps a pack",
                    day_phrase(m)
                ),
            ),
            (None, Some(d)) => (
                "unknown",
                format!(
                    "the two melt readings disagree: the degree-day bucket melts it out on {}, the model keeps a pack (lowest {} m)",
                    day_phrase(d),
                    d_min.map_or(0.0, |x| r2(x.0))
                ),
            ),
        }
    } else if depth_start.is_some_and(|d| d >= SNOW_ON_GROUND_M) {
        (
            "unknown",
            "the pass read no snow where the model held a pack".into(),
        )
    } else {
        match model_snow_at {
            Some(h) => (
                "broken",
                format!(
                    "the model holds snow from {}, after {} cm of snowfall",
                    iso_hour(h),
                    r2(window
                        .iter()
                        .filter(|i| series.hours[**i] <= h)
                        .map(|i| series.snowfall[*i].unwrap_or(0.0))
                        .sum())
                ),
            ),
            None => (
                "persisted_likely",
                "the model holds no snow on the ground at any hour of the window".into(),
            ),
        }
    };

    let state_on_date = match depth_end {
        Some(d) if d >= GLACIER_PACK_M => "glacier_pack",
        Some(d) if d >= SNOW_ON_GROUND_M => "snow",
        Some(d) if d < MELT_OUT_M => "no_snow",
        Some(_) => "thin_layer",
        None => "unknown",
    };
    let daily: Vec<JsonValue> = days
        .iter()
        .map(|(k, d)| {
            json!({
                "day": k,
                "t_max_c": d.t_max.map(r2),
                "thaw_hours": d.thaw_hours,
                "positive_degree_days": r2(d.pdd),
                "snowfall_cm": r2(d.snowfall),
                "depth_min_m": d.depth_min.map(r2),
                "depth_max_m": d.depth_max.map(r2),
            })
        })
        .collect();
    let thaw_days = days.values().filter(|d| d.thaw_hours > 0).count();
    let max_fall_day = days
        .iter()
        .max_by(|a, b| a.1.snowfall.total_cmp(&b.1.snowfall))
        .filter(|(_, d)| d.snowfall > 0.0)
        .map(|(k, d)| json!({"day": k, "cm": r2(d.snowfall)}));
    json!({
        "window": {"from": iso_hour(obs_hour), "to": iso_hour(to_hour), "hours": n},
        "start_state": start_state,
        "thaw": {
            "hours_above_0c": thaw_hours,
            "days_with_thaw": thaw_days,
            "positive_degree_days": r2(pdd),
            "max_t2m_c": t_max.map(|(v, h)| json!({"value": r2(v), "at": iso_hour(h)})),
            "last_thaw_hour": last_thaw.map(iso_hour),
            "source": "ERA5-Land 2 m temperature, hourly",
        },
        "snowfall": {
            "total_cm": r2(snow_total),
            "since_last_thaw_cm": r2(since_thaw),
            "max_day": max_fall_day,
            "source": "ERA5 snowfall, hourly, 0.25 degrees",
        },
        "model_pack": {
            "depth_at_observation_m": depth_start.map(r2),
            "min_m": d_min.map(|(v, h)| json!({"value": r2(v), "at": iso_hour(h)})),
            "max_m": d_max.map(r2),
            "melt_out_at": model_out.map(iso_hour),
            "depth_at_date_m": depth_end.map(r2),
            "source": "ERA5-Land snow depth, hourly, 0.1 degrees",
        },
        "degree_day_pack": {
            "swe_start_mm": r2(swe_start),
            "melt_out_at": dd_out.map(iso_hour),
            "swe_at_date_mm": r2(swe),
        },
        "missing_hours": gaps.iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<serde_json::Map<_, _>>(),
        "daily": daily,
        "verdict": verdict,
        "verdict_reason": reason,
        "state_on_date": {"state": state_on_date, "depth_m": depth_end.map(r2), "source": "ERA5-Land snow depth"},
    })
}

fn verdict_word(v: &str) -> &'static str {
    match v {
        "persisted_likely" => "PERSISTED LIKELY",
        "broken" => "BROKEN",
        "renewed" => "RENEWED",
        _ => "UNKNOWN",
    }
}

/// One sentence over a chain, for the response and for `/v1/ask`.
pub(crate) fn sentence(chain: &JsonValue, obs: &Obs) -> String {
    let g = |p: &str| chain.pointer(p).cloned().unwrap_or(JsonValue::Null);
    let num = |p: &str| g(p).as_f64().map(|x| format!("{x}"));
    let from = iso_day(obs.unix);
    let to = g("/window/to")
        .as_str()
        .map(|s| s[..10].to_string())
        .unwrap_or_default();
    let what = match g("/start_state").as_str() {
        Some("snow") => "the snow it saw",
        Some("no_snow") => "the bare ground it saw",
        _ => "what it saw",
    };
    let verdict = g("/verdict");
    let mut s = format!(
        "Whether {what} on {from} (NDSI {}) lasted to {to}: {}, because {}.",
        r2(obs.value),
        verdict_word(verdict.as_str().unwrap_or("")),
        g("/verdict_reason").as_str().unwrap_or("")
    );
    if let (Some(fall), Some(pdd), Some(th)) = (
        num("/snowfall/total_cm"),
        num("/thaw/positive_degree_days"),
        g("/thaw/hours_above_0c").as_u64(),
    ) {
        s.push_str(&format!(
            " Between them: {fall} cm of snowfall in ERA5, {th} hours above 0 °C and {pdd} positive degree-days in ERA5-Land."
        ));
    }
    if let Some(d) = num("/model_pack/depth_at_date_m") {
        s.push_str(&format!(
            " On {to} the model holds {d} m of snow. A model value, not an observation."
        ));
    }
    s
}

pub(crate) const CAUTION: &str = "model_output: a rule over a reanalysis, not an observation of the day asked. ERA5-Land and ERA5 cells are about 9 and 28 km wide, so a valley, a roof or a cleared square can differ. Sentinel-1 is not read into this chain: a VV drop flags wet snow only against a dry pass of the same orbit.";

/// Read the series, store it, judge it and sign the chain. Returns the
/// signed record with `chain_fact_cid` and `token` added.
pub(crate) async fn build_chain(
    s: &AppState,
    cell: &str,
    obs: &Obs,
    to_unix: i64,
    depth_fact: Option<(String, f64)>,
) -> Result<JsonValue, String> {
    if to_unix <= obs.unix {
        return Err("the hour asked is not after the observation".into());
    }
    if to_unix - obs.unix > MAX_DAYS * 86_400 {
        return Err(format!(
            "the observation is more than {MAX_DAYS} days before the hour asked"
        ));
    }
    let info = emem_codec::latlng_from_cell64(cell).map_err(|e| format!("cell decode: {e}"))?;
    let url = series_url(info.lat_deg, info.lng_deg, obs.unix, to_unix);
    let timeout = std::time::Duration::from_secs(crate::materializer_timeout_secs());
    let resp = tokio::time::timeout(
        timeout,
        crate::reqwest_client()
            .get(&url)
            .header("user-agent", emem_core::outbound::user_agent())
            .send(),
    )
    .await
    .map_err(|_| format!("reanalysis archive timeout after {}s", timeout.as_secs()))?
    .map_err(|e| format!("reanalysis archive https: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("reanalysis archive status {}", resp.status()));
    }
    let raw = resp
        .bytes()
        .await
        .map_err(|e| format!("reanalysis archive body: {e}"))?;
    let body: JsonValue =
        serde_json::from_slice(&raw).map_err(|e| format!("reanalysis archive json: {e}"))?;
    let series = parse_series(&body)?;
    let artifact_cid = ARTIFACTS
        .put(&raw)
        .map_err(|e| format!("artifact store: {e}"))?;
    let mut record = judge(obs, to_unix, &series);
    let to_hour = to_unix.div_euclid(3600) * 3600;
    if let Some(m) = record.as_object_mut() {
        m.insert("schema".into(), json!("emem.persistence.v1"));
        m.insert("fn_key".into(), json!(FN_KEY));
        m.insert("quantity".into(), json!("snow"));
        m.insert("cell".into(), json!(cell));
        m.insert(
            "observation".into(),
            json!({
                "fact_cid": obs.fact_cid, "band": obs.band, "value": obs.value,
                "observed_at": crate::iso8601_utc(obs.unix.max(0) as u64),
                "state": state_of_ndsi(obs.value),
            }),
        );
        m.insert(
            "series".into(),
            json!({
                "artifact_cid": artifact_cid,
                "byte_len": raw.len(),
                "url": format!("/v1/artifacts/{artifact_cid}"),
                "source_url": url,
                "models": {"temperature_2m": "era5_land", "snowfall": "era5", "snow_depth": "era5_land"},
                "grid": {"lat": body.get("latitude"), "lng": body.get("longitude"), "elevation_m": body.get("elevation")},
                "licence": "CC BY 4.0, Copernicus Climate Change Service ERA5 and ERA5-Land through Open-Meteo",
            }),
        );
        m.insert(
            "depth_fact".into(),
            match &depth_fact {
                Some((cid, v)) => {
                    json!({"fact_cid": cid, "band": "era5.snow_depth", "value": v, "at": iso_hour(to_hour)})
                }
                None => JsonValue::Null,
            },
        );
        m.insert(
            "rule".into(),
            json!({
                "text": RULE_TEXT, "ndsi_snow": NDSI_SNOW, "ndsi_no_snow": NDSI_NO_SNOW,
                "melt_out_m": MELT_OUT_M, "snow_on_ground_m": SNOW_ON_GROUND_M,
                "glacier_pack_m": GLACIER_PACK_M, "ddf_mm_per_degc_day": DDF_MM,
                "start_density_kg_m3": START_DENSITY, "snowfall_cm_per_mm": SNOWFALL_CM_PER_MM,
                "max_missing_share": MAX_MISSING_SHARE,
            }),
        );
        m.insert(
            "provenance".into(),
            json!({"class": "model_output", "caution": CAUTION}),
        );
    }
    let sent = sentence(&record, obs);
    if let Some(m) = record.as_object_mut() {
        m.insert("sentence".into(), json!(sent));
    }

    let day = |u: i64| (u.max(0) / 86_400) as u64;
    let mut parents = vec![FactCid::new(obs.fact_cid.clone())];
    if let Some((cid, _)) = &depth_fact {
        parents.push(FactCid::new(cid.clone()));
    }
    let signed_at = emem_storage::server::iso8601_now();
    let fact = DerivativeFact {
        cell: cell.to_string(),
        band: "persistence.chain".to_string(),
        tslot_window: [day(obs.unix), day(to_unix)],
        op: "persistence".to_string(),
        parents,
        value: json_to_cbor(&record),
        confidence: 0.6,
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
    .map_err(|e| format!("chain attestation: {e}"))?;
    let cids = s
        .storage
        .put_attestation(&att)
        .await
        .map_err(|e| format!("store: {e}"))?;
    let cid = cids
        .first()
        .map(|c| c.as_str().to_string())
        .ok_or("store returned no cid for the chain")?;
    if let Some(m) = record.as_object_mut() {
        m.insert("chain_fact_cid".into(), json!(cid));
        m.insert("token".into(), json!(format!("emem:fact:{cell}:{cid}")));
    }
    Ok(record)
}

/// What `/v1/ask` carries of a chain: the verdict, the numbers it turned on,
/// and the token of the signed record that holds the rest.
pub(crate) fn brief(chain: &JsonValue) -> JsonValue {
    let g = |p: &str| chain.pointer(p).cloned().unwrap_or(JsonValue::Null);
    json!({
        "rule": FN_KEY,
        "verdict": g("/verdict"),
        "verdict_reason": g("/verdict_reason"),
        "window": g("/window"),
        "snowfall_total_cm": g("/snowfall/total_cm"),
        "positive_degree_days": g("/thaw/positive_degree_days"),
        "model_melt_out_at": g("/model_pack/melt_out_at"),
        "degree_day_melt_out_at": g("/degree_day_pack/melt_out_at"),
        "token": g("/token"),
        "full_chain": "POST /v1/persist with this cell, the date and the hour returns every link; the token resolves to the signed record",
    })
}

fn first_primary(
    resp: &emem_primitives::recall::RecallResp,
    band: &str,
) -> Option<(String, emem_fact::PrimaryFact)> {
    resp.facts
        .iter()
        .enumerate()
        .filter_map(|(i, f)| match f {
            Fact::Primary(p) if p.band == band => resp
                .receipt
                .fact_cids
                .get(i)
                .map(|c| (c.as_str().to_string(), p.clone())),
            _ => None,
        })
        .max_by_key(|(_, p)| p.tslot)
}

/// `POST /v1/persist`.
pub async fn post_persist(
    State(s): State<AppState>,
    EmemJson(req): EmemJson<PersistReq>,
) -> Result<Json<JsonValue>, ApiError> {
    let started = Instant::now();
    if let Some(q) = req.quantity.as_deref() {
        if !q.eq_ignore_ascii_case("snow") {
            return Err(bad_request(format!(
                "quantity `{q}`: this rule reads snow only. Water, ice and bare-field chains need their own rules, and none is declared yet"
            )));
        }
    }
    let cell = match (&req.cell, req.lat, req.lng) {
        (Some(c), _, _) => c.clone(),
        (None, Some(la), Some(lo))
            if (-90.0..=90.0).contains(&la) && (-180.0..=180.0).contains(&lo) =>
        {
            emem_codec::geo::cell64_from_latlng(la, lo)
        }
        _ => {
            return Err(bad_request(
                "give `cell` (a cell64) or `lat` and `lng`".into(),
            ))
        }
    };
    emem_codec::latlng_from_cell64(&cell)
        .map_err(|e| bad_request(format!("`cell` is not a cell64: {e}")))?;
    let (y, m, d) = crate::band_raster::parse_ymd(&req.date)
        .ok_or_else(|| bad_request(format!("`date` `{}` is not YYYY-MM-DD", req.date)))?;
    let hour = req.hour.unwrap_or(12);
    if hour > 23 {
        return Err(bad_request("`hour` is a UTC hour, 0 to 23".into()));
    }
    let to_unix = crate::days_from_civil(y, m, d) * 86_400 + hour as i64 * 3600;
    let now = crate::now_unix_s();
    if to_unix > now - 6 * 86_400 {
        return Err(bad_request(
            "the reanalysis archive runs about five days behind: ask for an hour at least six days ago".into(),
        ));
    }

    // The observation: the one named, or the last pass at or before the day.
    let obs = match &req.observation {
        Some(cid) => {
            let got = s
                .storage
                .get_facts_many(&[FactCid::new(cid.clone())])
                .await
                .map_err(ApiError::from)?;
            let Some(Some(Fact::Primary(p))) = got.into_iter().next() else {
                return Err(bad_request(format!(
                    "`observation` {cid} is not a signed reading on this responder"
                )));
            };
            if p.band != "indices.ndsi" || p.cell != cell {
                return Err(bad_request(format!(
                    "`observation` must be an indices.ndsi fact at {cell}; it is {} at {}",
                    p.band, p.cell
                )));
            }
            let value = cbor_f64(&p.value)
                .ok_or_else(|| bad_request("observation has no number".into()))?;
            let unix =
                observed_unix(&p).ok_or_else(|| bad_request("observation has no time".into()))?;
            Obs {
                fact_cid: cid.clone(),
                band: p.band.clone(),
                value,
                unix,
            }
        }
        None => {
            let tempo =
                crate::band_tempo_for_key("indices.ndsi").unwrap_or(emem_core::tslot::Tempo::Fast);
            let req_ndsi = emem_primitives::recall::RecallReq {
                cell: cell.clone(),
                bands: Some(vec!["indices.ndsi".into()]),
                as_of_tslot: Some(emem_core::tslot::Tslot::from_unix(to_unix, tempo).0),
                ..Default::default()
            };
            let (resp, _) = crate::recall_with_auto_materialize_capped(&req_ndsi, &s, None).await?;
            let Some((cid, p)) = first_primary(&resp, "indices.ndsi") else {
                return Err(upstream_error(format!(
                    "no Sentinel-2 NDSI reading at or before {} could be read at {cell}",
                    req.date
                )));
            };
            let value = cbor_f64(&p.value).unwrap_or(f64::NAN);
            let unix = observed_unix(&p).unwrap_or(to_unix);
            Obs {
                fact_cid: cid,
                band: p.band.clone(),
                value,
                unix,
            }
        }
    };
    if obs.unix > to_unix {
        return Err(bad_request(format!(
            "the observation ({}) is after the hour asked",
            iso_hour(obs.unix)
        )));
    }

    // The modelled depth at the hour asked, as its own signed fact.
    let req_depth = emem_primitives::recall::RecallReq {
        cell: cell.clone(),
        bands: Some(vec!["era5.snow_depth".into()]),
        tslot: Some((to_unix / 3600) as u64),
        ..Default::default()
    };
    let depth_fact = match crate::recall_with_auto_materialize_capped(&req_depth, &s, None).await {
        Ok((resp, _)) => first_primary(&resp, "era5.snow_depth")
            .and_then(|(cid, p)| cbor_f64(&p.value).map(|v| (cid, v))),
        Err(_) => None,
    };

    let record = build_chain(&s, &cell, &obs, to_unix, depth_fact.clone())
        .await
        .map_err(upstream_error)?;
    let mut cids = vec![FactCid::new(obs.fact_cid.clone())];
    if let Some((c, _)) = &depth_fact {
        cids.push(FactCid::new(c.clone()));
    }
    if let Some(c) = record.get("chain_fact_cid").and_then(|v| v.as_str()) {
        cids.push(FactCid::new(c.to_string()));
    }
    let receipt = s.sign_receipt(
        "emem.persist",
        vec![cell.clone()],
        cids,
        false,
        started,
        None,
    );
    let mut out = record;
    if let Some(m) = out.as_object_mut() {
        m.insert(
            "receipt".into(),
            serde_json::to_value(&receipt).unwrap_or(JsonValue::Null),
        );
    }
    Ok(Json(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn series(hours: &[(i64, f64, f64, f64)]) -> Series {
        Series {
            hours: hours.iter().map(|h| h.0).collect(),
            t2m: hours.iter().map(|h| Some(h.1)).collect(),
            snowfall: hours.iter().map(|h| Some(h.2)).collect(),
            depth: hours.iter().map(|h| Some(h.3)).collect(),
        }
    }

    fn obs(v: f64, unix: i64) -> Obs {
        Obs {
            fact_cid: "x".into(),
            band: "indices.ndsi".into(),
            value: v,
            unix,
        }
    }

    const H: i64 = 3600;

    #[test]
    fn a_pack_kept_by_both_readings_persisted() {
        let s = series(&[
            (0, -5.0, 0.0, 0.2),
            (H, -4.0, 0.0, 0.2),
            (2 * H, -3.0, 0.5, 0.21),
        ]);
        let c = judge(&obs(0.7, 0), 2 * H, &s);
        assert_eq!(c["verdict"], "persisted_likely", "{c}");
        assert_eq!(c["window"]["hours"], 2);
        assert_eq!(c["snowfall"]["total_cm"], 0.5);
    }

    #[test]
    fn when_the_readings_disagree_the_chain_says_unknown() {
        // 3 cm of modelled pack is 9 mm of water; 30 hours at +6 C is 7.5
        // positive degree-days, 22.5 mm of melt by degree-day. The model keeps
        // 2 cm, as ERA5-Land did at Santa Claus Village in November 2024.
        let mut h = vec![(0, -2.0, 0.0, 0.03)];
        for i in 1..=30 {
            h.push((i * H, 6.0, 0.0, 0.02));
        }
        let c = judge(&obs(0.7, 0), 30 * H, &series(&h));
        assert_eq!(c["verdict"], "unknown", "{c}");
        let r = c["verdict_reason"].as_str().unwrap();
        assert!(r.contains("disagree"), "{r}");
        assert!(c["degree_day_pack"]["melt_out_at"].is_string(), "{c}");
        assert!(c["model_pack"]["melt_out_at"].is_null(), "{c}");
        assert_eq!(c["thaw"]["hours_above_0c"], 30);
        assert_eq!(c["thaw"]["positive_degree_days"], 7.5);
    }

    #[test]
    fn melted_out_by_both_then_new_snow_is_renewed() {
        let mut h = vec![(0, -2.0, 0.0, 0.03)];
        for i in 1..=20 {
            h.push((i * H, 8.0, 0.0, 0.0));
        }
        for i in 21..=30 {
            h.push((i * H, -6.0, 2.0, 0.02 * (i - 20) as f64));
        }
        let c = judge(&obs(0.7, 0), 30 * H, &series(&h));
        assert_eq!(c["verdict"], "renewed", "{c}");
        assert_eq!(c["state_on_date"]["state"], "snow");
        assert_eq!(c["snowfall"]["total_cm"], 20.0);
    }

    #[test]
    fn melted_out_and_still_bare_is_broken() {
        let mut h = vec![(0, -2.0, 0.0, 0.03)];
        for i in 1..=20 {
            h.push((i * H, 8.0, 0.0, 0.0));
        }
        let c = judge(&obs(0.7, 0), 20 * H, &series(&h));
        assert_eq!(c["verdict"], "broken", "{c}");
        assert_eq!(c["state_on_date"]["state"], "no_snow");
    }

    #[test]
    fn a_glacier_cell_is_unknown() {
        let s = series(&[(0, -10.0, 0.0, 26.67), (H, -10.0, 0.0, 26.67)]);
        let c = judge(&obs(0.8, 0), H, &s);
        assert_eq!(c["verdict"], "unknown");
        assert!(c["verdict_reason"].as_str().unwrap().contains("glacier"));
        assert_eq!(c["state_on_date"]["state"], "glacier_pack");
    }

    #[test]
    fn bare_ground_then_snow_is_broken_by_the_snowfall() {
        let s = series(&[
            (0, 2.0, 0.0, 0.0),
            (H, -3.0, 4.0, 0.03),
            (2 * H, -3.0, 4.0, 0.07),
        ]);
        let c = judge(&obs(-0.2, 0), 2 * H, &s);
        assert_eq!(c["verdict"], "broken", "{c}");
        assert!(
            c["verdict_reason"].as_str().unwrap().contains("8 cm"),
            "{c}"
        );
    }

    #[test]
    fn snow_or_cloud_at_the_start_is_unknown() {
        let s = series(&[(0, -5.0, 0.0, 0.2), (H, -5.0, 0.0, 0.2)]);
        assert_eq!(judge(&obs(0.2, 0), H, &s)["verdict"], "unknown");
    }

    #[test]
    fn a_series_with_gaps_is_unknown() {
        let mut s = series(&[
            (0, -5.0, 0.0, 0.2),
            (H, -5.0, 0.0, 0.2),
            (2 * H, -5.0, 0.0, 0.2),
        ]);
        s.depth[1] = None;
        let c = judge(&obs(0.7, 0), 2 * H, &s);
        assert_eq!(c["verdict"], "unknown");
        assert!(c["verdict_reason"].as_str().unwrap().contains("missing"));
    }

    #[test]
    fn the_series_parses_both_models_from_one_response() {
        let body = json!({"hourly": {
            "time": ["2024-10-31T00:00", "2024-10-31T01:00"],
            "temperature_2m_era5_land": [-3.5, -3.4],
            "snowfall_era5": [0.0, 0.07],
            "snow_depth_era5_land": [0.03, 0.02],
        }});
        let s = parse_series(&body).unwrap();
        assert_eq!(s.hours[1] - s.hours[0], 3600);
        assert_eq!(s.snowfall[1], Some(0.07));
        assert!(parse_series(&json!({"hourly": {"time": []}})).is_err());
    }
}
