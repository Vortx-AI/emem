//! Where the sun is, `POST /v1/sun_position` (`sun_position@1`).
//!
//! A shadow, a glint or a dark slope in a scene is read against the sun's
//! place in the sky at that instant. This module computes it with the
//! equations of the NOAA Solar Calculator spreadsheet, after Meeus,
//! *Astronomical Algorithms*, and signs the whole answer as one derivation
//! whose value holds its inputs. The answer is a deterministic function of
//! those inputs, so a resolve of the token computes every row again
//! ([`recompute_matches`]) and says whether the signed rows still follow.

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

pub(crate) const FN_KEY: &str = "sun_position@1";
pub(crate) const MAX_PLACES: usize = 50;
pub(crate) const MAX_ROWS: usize = 2000;
/// The years NOAA states its spreadsheet valid for. This module computes
/// the Julian Day from the Unix time and has no such limit of its own, but
/// the low-accuracy solar coordinates lose accuracy far from 2000.
const FIRST_YEAR: i32 = 1901;
const LAST_YEAR: i32 = 2099;
/// Sunrise and sunset: the upper limb on the horizon, 34' of refraction plus
/// the sun's 16' semi-diameter below the geometric horizon.
pub(crate) const RISE_ZENITH_DEG: f64 = 90.833;
/// Passes of the sunrise, sunset and transit solutions. Each pass reads the
/// sun at the time the previous pass found; three agree to well under a
/// second.
const EVENT_PASSES: usize = 3;
const ANGLE_DECIMALS: f64 = 1e4;

pub(crate) const ALGORITHM: &str = "NOAA Solar Calculator spreadsheet equations (NOAA_Solar_Calculations_day.xls, NOAA Global Monitoring Laboratory), after Meeus, Astronomical Algorithms (low-accuracy solar coordinates)";
pub(crate) const STATED_ACCURACY: &str = "NOAA states that the sunrise and sunset results are theoretically accurate to within a minute between 72 degrees south and 72 degrees north, and within 10 minutes outside those latitudes, and that observed values vary with atmospheric composition, temperature and pressure (https://gml.noaa.gov/grad/solcalc/calcdetails.html)";
pub(crate) const REFRACTION: &str = "the spreadsheet's approximation for a standard atmosphere, in arcseconds: 0 above 85 degrees; 58.1/tan(e) - 0.07/tan(e)^3 + 0.000086/tan(e)^5 above 5 degrees; 1735 + e(-518.2 + e(103.4 + e(-12.79 + 0.711e))) above -0.575 degrees; else -20.772/tan(e). It is about 34 arcminutes at the horizon and changes with temperature and pressure there";

/// The sun at one place and instant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SunAt {
    /// Degrees clockwise from true north.
    pub azimuth_deg: f64,
    /// Apparent: the geometric elevation plus [`refraction_deg`].
    pub elevation_deg: f64,
    pub elevation_geometric_deg: f64,
    pub equation_of_time_min: f64,
}

/// Julian centuries since J2000.0, from Unix seconds. Universal time stands
/// in for terrestrial time, as in the spreadsheet: the 69 s between them
/// moves the sun by under 0.001 degrees.
fn julian_century(unix: f64) -> f64 {
    let jd = unix / 86_400.0 + 2_440_587.5;
    (jd - 2_451_545.0) / 36_525.0
}

/// Declination (degrees) and the equation of time (minutes) at `t`, Julian
/// centuries, in the spreadsheet's own terms.
fn declination_and_eot(t: f64) -> (f64, f64) {
    let l0 = (280.46646 + t * (36000.76983 + t * 0.0003032)).rem_euclid(360.0);
    let m = 357.52911 + t * (35999.05029 - 0.0001537 * t);
    let e = 0.016708634 - t * (0.000042037 + 0.0000001267 * t);
    let mr = m.to_radians();
    let c = mr.sin() * (1.914602 - t * (0.004817 + 0.000014 * t))
        + (2.0 * mr).sin() * (0.019993 - 0.000101 * t)
        + (3.0 * mr).sin() * 0.000289;
    let omega = (125.04 - 1934.136 * t).to_radians();
    let apparent_long = l0 + c - 0.00569 - 0.00478 * omega.sin();
    let mean_obliquity =
        23.0 + (26.0 + (21.448 - t * (46.815 + t * (0.00059 - t * 0.001813))) / 60.0) / 60.0;
    let obliquity = (mean_obliquity + 0.00256 * omega.cos()).to_radians();
    let declination = (obliquity.sin() * apparent_long.to_radians().sin())
        .asin()
        .to_degrees();
    let y = (obliquity / 2.0).tan().powi(2);
    let l0r = l0.to_radians();
    let eot = 4.0
        * (y * (2.0 * l0r).sin() - 2.0 * e * mr.sin() + 4.0 * e * y * mr.sin() * (2.0 * l0r).cos()
            - 0.5 * y * y * (4.0 * l0r).sin()
            - 1.25 * e * e * (2.0 * mr).sin())
        .to_degrees();
    (declination, eot)
}

/// Atmospheric refraction in degrees at a geometric elevation, [`REFRACTION`].
pub(crate) fn refraction_deg(elevation: f64) -> f64 {
    let tan = elevation.to_radians().tan();
    let arcsec = if elevation > 85.0 {
        0.0
    } else if elevation > 5.0 {
        58.1 / tan - 0.07 / tan.powi(3) + 0.000086 / tan.powi(5)
    } else if elevation > -0.575 {
        1735.0
            + elevation * (-518.2 + elevation * (103.4 + elevation * (-12.79 + elevation * 0.711)))
    } else {
        -20.772 / tan
    };
    arcsec / 3600.0
}

/// The sun at `lat`, `lng` (degrees, east positive) at Unix second `unix`.
pub(crate) fn sun_at(lat: f64, lng: f64, unix: f64) -> SunAt {
    let (declination, eot) = declination_and_eot(julian_century(unix));
    let minutes_utc = unix.rem_euclid(86_400.0) / 60.0;
    let true_solar = (minutes_utc + eot + 4.0 * lng).rem_euclid(1440.0);
    let hour_angle = true_solar / 4.0 - 180.0;
    let (phi, dec, h) = (
        lat.to_radians(),
        declination.to_radians(),
        hour_angle.to_radians(),
    );
    let cos_zenith = (phi.sin() * dec.sin() + phi.cos() * dec.cos() * h.cos()).clamp(-1.0, 1.0);
    let geometric = 90.0 - cos_zenith.acos().to_degrees();
    // Meeus eq. 13.5, measured from the south and westward, turned to the
    // north. It is the spreadsheet's arccos form, defined at every hour angle.
    let from_south = h
        .sin()
        .atan2(h.cos() * phi.sin() - dec.tan() * phi.cos())
        .to_degrees();
    SunAt {
        azimuth_deg: (from_south + 180.0).rem_euclid(360.0),
        elevation_deg: geometric + refraction_deg(geometric),
        elevation_geometric_deg: geometric,
        equation_of_time_min: eot,
    }
}

/// Hour angle of sunrise in degrees for a declination, or why there is none
/// that day: `Err(true)` the sun stays up, `Err(false)` it stays down.
fn rise_hour_angle(lat: f64, declination: f64) -> Result<f64, bool> {
    let (phi, dec) = (lat.to_radians(), declination.to_radians());
    let x = RISE_ZENITH_DEG.to_radians().cos() / (phi.cos() * dec.cos()) - phi.tan() * dec.tan();
    if !x.is_finite() || x < -1.0 {
        return Err(true);
    }
    if x > 1.0 {
        return Err(false);
    }
    Ok(x.acos().to_degrees())
}

/// The solar day's events at a place, in Unix seconds (not yet rounded).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DayEvents {
    pub noon: f64,
    pub rise: Option<f64>,
    pub set: Option<f64>,
    /// `Some(true)` the sun stays up all day, `Some(false)` it stays down.
    pub polar: Option<bool>,
}

/// The transit, sunrise and sunset of the solar day whose transit is nearest
/// 12:00 local mean solar time on the UTC day starting at `day0` (Unix
/// seconds). Each event is solved at its own instant, as the NOAA web
/// calculator refines it, so the declination and the equation of time are
/// those of the event and not of midnight.
pub(crate) fn day_events(lat: f64, lng: f64, day0: f64) -> DayEvents {
    let at = |minutes: f64| day0 + minutes * 60.0;
    let mut noon = at(720.0 - 4.0 * lng);
    for _ in 0..EVENT_PASSES {
        let (_, eot) = declination_and_eot(julian_century(noon));
        noon = at(720.0 - 4.0 * lng - eot);
    }
    let mut polar = None;
    let mut event = |sign: f64| -> Option<f64> {
        let mut t = noon;
        for _ in 0..EVENT_PASSES {
            let (dec, eot) = declination_and_eot(julian_century(t));
            match rise_hour_angle(lat, dec) {
                Ok(ha) => t = at(720.0 - 4.0 * (lng + sign * ha) - eot),
                Err(up) => {
                    polar = Some(up);
                    return None;
                }
            }
        }
        Some(t)
    };
    let rise = event(1.0);
    let set = event(-1.0);
    DayEvents {
        noon,
        rise,
        set,
        polar,
    }
}

fn round_angle(x: f64) -> f64 {
    (x * ANGLE_DECIMALS).round() / ANGLE_DECIMALS
}

/// `2026-03-21T00:53:40Z` for a whole Unix second.
pub(crate) fn iso_second(unix: i64) -> String {
    let (y, m, d) = crate::civil_from_days(unix.div_euclid(86_400));
    let s = unix.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        s / 3600,
        s / 60 % 60,
        s % 60
    )
}

/// Local mean solar time of a whole Unix second at `lng`, `HH:MM:SS`: UTC
/// plus 4 minutes per degree east, to the second.
fn lmst(unix: i64, lng: f64) -> String {
    let s = (unix as f64 + lng * 240.0).round() as i64;
    let s = s.rem_euclid(86_400);
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

/// One ISO 8601 UTC instant to Unix seconds: `YYYY-MM-DDTHH:MM[:SS]` with
/// `Z` or `+00:00`. Whole seconds only, so the signed instant is the one sent.
pub(crate) fn parse_instant(s: &str) -> Result<i64, String> {
    let t = s.trim();
    let body = t
        .strip_suffix('Z')
        .or_else(|| t.strip_suffix('z'))
        .or_else(|| t.strip_suffix("+00:00"))
        .ok_or_else(|| {
            format!(
                "`{s}` is not a UTC instant: end it with Z or +00:00, as in 2026-03-21T06:00:00Z"
            )
        })?;
    let (date, time) = body
        .split_once(['T', 't', ' '])
        .ok_or_else(|| format!("`{s}` has no time: send YYYY-MM-DDTHH:MM:SSZ"))?;
    let (y, m, d) = crate::band_raster::parse_ymd(date)
        .ok_or_else(|| format!("`{s}`: the date is not YYYY-MM-DD"))?;
    if !(FIRST_YEAR..=LAST_YEAR).contains(&y) {
        return Err(format!(
            "`{s}`: the year must be {FIRST_YEAR} to {LAST_YEAR}, the years NOAA states its spreadsheet valid for"
        ));
    }
    let day = crate::days_from_civil(y, m, d);
    if crate::civil_from_days(day) != (y, m, d) {
        return Err(format!("`{s}`: {date} is not a day of the calendar"));
    }
    if time.contains('.') || time.contains(',') {
        return Err(format!(
            "`{s}`: send whole seconds. The signed answer names each instant as it was read"
        ));
    }
    let parts: Vec<&str> = time.split(':').collect();
    let num = |p: &str, max: i64| -> Option<i64> {
        (p.len() == 2 && p.bytes().all(|b| b.is_ascii_digit()))
            .then(|| p.parse::<i64>().ok())
            .flatten()
            .filter(|v| *v <= max)
    };
    let (h, mi, se) = match parts.as_slice() {
        [h, mi] => (num(h, 23), num(mi, 59), Some(0)),
        [h, mi, se] => (num(h, 23), num(mi, 59), num(se, 59)),
        _ => (None, None, None),
    };
    match (h, mi, se) {
        (Some(h), Some(mi), Some(se)) => Ok(day * 86_400 + h * 3600 + mi * 60 + se),
        _ => Err(format!("`{s}`: the time is not HH:MM or HH:MM:SS")),
    }
}

/// One place of a request: `{lat, lng}` or `{cell}`, with an optional `id`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Place {
    pub id: Option<String>,
    pub cell: Option<String>,
    pub lat: f64,
    pub lng: f64,
}

impl Place {
    fn to_json(&self) -> JsonValue {
        let mut p = json!({"lat": self.lat, "lng": self.lng});
        if let Some(id) = &self.id {
            p["id"] = json!(id);
        }
        if let Some(c) = &self.cell {
            p["cell"] = json!(c);
        }
        p
    }
}

pub(crate) fn parse_places(raw: &JsonValue) -> Result<Vec<Place>, String> {
    let list = raw
        .as_array()
        .ok_or("`places` is a list of {lat, lng} or {cell}")?;
    if list.is_empty() || list.len() > MAX_PLACES {
        return Err(format!(
            "`places` takes 1 to {MAX_PLACES} places; this call gave {}",
            list.len()
        ));
    }
    list.iter()
        .enumerate()
        .map(|(i, p)| {
            let id = p.get("id").and_then(|v| match v {
                JsonValue::String(s) => Some(s.clone()),
                JsonValue::Number(n) => Some(n.to_string()),
                _ => None,
            });
            let (lat, lng, cell) = match (
                p.get("cell").and_then(|v| v.as_str()),
                p.get("lat").and_then(|v| v.as_f64()),
                p.get("lng").and_then(|v| v.as_f64()),
            ) {
                (Some(c), None, None) => {
                    let ll = emem_codec::latlng_from_cell64(c)
                        .map_err(|e| format!("place {i}: `{c}` is not a cell64: {e}"))?;
                    (ll.lat_deg, ll.lng_deg, Some(c.to_string()))
                }
                (None, Some(lat), Some(lng)) => (lat, lng, None),
                _ => {
                    return Err(format!(
                        "place {i}: give `lat` and `lng`, or `cell`, not both"
                    ))
                }
            };
            if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lng) {
                return Err(format!(
                    "place {i}: lat must be -90 to 90 and lng -180 to 180"
                ));
            }
            Ok(Place { id, cell, lat, lng })
        })
        .collect()
}

#[derive(Debug, Clone, Deserialize)]
pub struct SunRange {
    pub from: String,
    pub to: String,
    pub step_minutes: i64,
}

#[derive(Debug, Deserialize)]
pub struct SunPositionReq {
    /// 1 to 50 places, each `{lat, lng}` or `{cell}`, with an optional `id`.
    pub places: JsonValue,
    /// ISO 8601 UTC instants. Give this or `range`.
    #[serde(default)]
    pub times: Option<Vec<String>>,
    /// `{from, to, step_minutes}`: every step from `from` to `to`, both ends
    /// included when the step lands on `to`.
    #[serde(default)]
    pub range: Option<SunRange>,
}

/// The instants of a request, checked against the row cap before any is made.
pub(crate) fn instants(
    times: Option<&[String]>,
    range: Option<&SunRange>,
    places: usize,
) -> Result<Vec<i64>, String> {
    let out = match (times, range) {
        (Some(t), None) => {
            if t.is_empty() {
                return Err("`times` is empty".into());
            }
            if t.len().saturating_mul(places) > MAX_ROWS {
                return Err(format!(
                    "{} places x {} times is {} rows; the cap is {MAX_ROWS}",
                    places,
                    t.len(),
                    t.len().saturating_mul(places)
                ));
            }
            t.iter()
                .map(|s| parse_instant(s))
                .collect::<Result<Vec<_>, _>>()?
        }
        (None, Some(r)) => {
            if r.step_minutes < 1 {
                return Err("`range.step_minutes` must be 1 or more".into());
            }
            let (from, to) = (parse_instant(&r.from)?, parse_instant(&r.to)?);
            if to < from {
                return Err("`range.to` is before `range.from`".into());
            }
            let step = r.step_minutes.saturating_mul(60);
            let n = (to - from) / step + 1;
            if (n as u128).saturating_mul(places as u128) > MAX_ROWS as u128 {
                return Err(format!(
                    "{places} places x {n} steps is {} rows; the cap is {MAX_ROWS}. Widen step_minutes or shorten the range",
                    (n as u128).saturating_mul(places as u128)
                ));
            }
            (0..n).map(|k| from + k * step).collect()
        }
        _ => return Err("give `times` (a list of UTC instants) or `range`, not both".into()),
    };
    Ok(out)
}

/// The rows and days of an answer: everything [`recompute_matches`] checks.
pub(crate) fn compute(places: &[Place], times: &[i64]) -> JsonValue {
    let mut rows: Vec<JsonValue> = Vec::with_capacity(places.len() * times.len());
    let mut dates: Vec<i64> = times.iter().map(|t| t.div_euclid(86_400)).collect();
    dates.sort_unstable();
    dates.dedup();
    let mut days: Vec<JsonValue> = Vec::with_capacity(places.len() * dates.len());
    for (i, p) in places.iter().enumerate() {
        for t in times {
            let s = sun_at(p.lat, p.lng, *t as f64);
            rows.push(json!([
                i,
                iso_second(*t),
                round_angle(s.azimuth_deg),
                round_angle(s.elevation_deg),
                round_angle(s.elevation_geometric_deg),
            ]));
        }
        for d in &dates {
            let ev = day_events(p.lat, p.lng, (*d * 86_400) as f64);
            let noon = ev.noon.round() as i64;
            let at_noon = sun_at(p.lat, p.lng, noon as f64);
            let rise = ev.rise.map(|x| x.round() as i64);
            let set = ev.set.map(|x| x.round() as i64);
            let (y, m, dd) = crate::civil_from_days(*d);
            let kind = match ev.polar {
                Some(true) => "sun_stays_up",
                Some(false) => "sun_stays_down",
                None => "rise_and_set",
            };
            days.push(json!([
                i,
                format!("{y:04}-{m:02}-{dd:02}"),
                rise.map(iso_second),
                iso_second(noon),
                set.map(iso_second),
                rise.map(|r| lmst(r, p.lng)),
                lmst(noon, p.lng),
                set.map(|s| lmst(s, p.lng)),
                match (rise, set) {
                    (Some(r), Some(s)) => json!(round_angle((s - r) as f64 / 60.0)),
                    _ => JsonValue::Null,
                },
                round_angle(at_noon.elevation_deg),
                round_angle(at_noon.equation_of_time_min),
                kind,
            ]));
        }
    }
    json!({"rows": rows, "days": days})
}

/// The record a request signs, before its cid is known.
fn record(places: &[Place], times: &[i64], req: &SunPositionReq) -> JsonValue {
    let inputs = match &req.range {
        Some(r) => json!({
            "places": places.iter().map(Place::to_json).collect::<Vec<_>>(),
            "range": {"from": iso_second(times[0]), "to": r.to.trim(), "step_minutes": r.step_minutes},
        }),
        None => json!({
            "places": places.iter().map(Place::to_json).collect::<Vec<_>>(),
            "times": times.iter().map(|t| iso_second(*t)).collect::<Vec<_>>(),
        }),
    };
    let body = compute(places, times);
    json!({
        "schema": "emem.sun_position.v1",
        "fn_key": FN_KEY,
        "inputs": inputs,
        "algorithm": {
            "name": ALGORITHM,
            "stated_accuracy": STATED_ACCURACY,
            "julian_day": "JD = unix / 86400 + 2440587.5, from the instant itself, without the spreadsheet's day-serial approximation that limits it to 1901 to 2099. Universal time stands in for terrestrial time, as in the spreadsheet",
            "azimuth": "degrees clockwise from true north. Meeus eq. 13.5 (atan2), the same angle as the spreadsheet's arccos form",
            "elevation": "elevation_deg is apparent: the geometric elevation plus the refraction below. elevation_geometric_deg has no refraction",
            "refraction": REFRACTION,
            "events": format!("sunrise and sunset: the upper limb on the horizon, at a zenith of {RISE_ZENITH_DEG} degrees (34 arcminutes of refraction and a 16 arcminute semi-diameter). Solar noon: the transit, hour angle 0. Each event is solved at its own instant in {EVENT_PASSES} passes, as the NOAA web calculator refines it"),
            "solar_day": "a date's events are those of the solar day whose transit falls nearest 12:00 local mean solar time on that UTC date. Its sunrise can be on the UTC date before and its sunset on the date after",
            "local_mean_solar_time": "UTC plus 4 minutes per degree of east longitude, to the second. It is not a time zone and has no daylight saving. Apparent solar time is local mean solar time plus equation_of_time_min",
            "day_kinds": "rise_and_set; sun_stays_up (no sunrise or sunset, polar day); sun_stays_down (polar night)",
        },
        "rounding": "angles to 4 decimals of a degree, minutes to 4 decimals, instants to the nearest whole second",
        "columns": ["place", "time", "azimuth_deg", "elevation_deg", "elevation_geometric_deg"],
        "rows": body["rows"],
        "day_columns": ["place", "date", "sunrise_utc", "solar_noon_utc", "sunset_utc", "sunrise_lmst", "solar_noon_lmst", "sunset_lmst", "day_length_min", "noon_elevation_deg", "equation_of_time_min", "kind"],
        "days": body["days"],
        "counts": {"places": places.len(), "times": times.len(), "rows": places.len() * times.len()},
    })
}

/// The inputs of a signed `sun_position@1` record, read back.
fn inputs_of(record: &JsonValue) -> Result<(Vec<Place>, Vec<i64>), String> {
    let inputs = &record["inputs"];
    // A cell's place is pinned at the centre it was read at, beside the cell.
    let places: Vec<Place> = inputs["places"]
        .as_array()
        .ok_or("the record holds no places")?
        .iter()
        .map(|p| {
            Ok(Place {
                id: p["id"].as_str().map(str::to_string),
                cell: p["cell"].as_str().map(str::to_string),
                lat: p["lat"].as_f64().ok_or("a place has no lat")?,
                lng: p["lng"].as_f64().ok_or("a place has no lng")?,
            })
        })
        .collect::<Result<_, String>>()?;
    let times: Option<Vec<String>> = inputs.get("times").and_then(|t| {
        t.as_array().map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
    });
    let range: Option<SunRange> = inputs
        .get("range")
        .and_then(|r| serde_json::from_value(r.clone()).ok());
    let times = instants(times.as_deref(), range.as_ref(), places.len())?;
    Ok((places, times))
}

/// Do two JSON values agree, numbers to within a billionth? The signed
/// numbers are already rounded, so this compares at the stated rounding.
pub(crate) fn same_json(a: &JsonValue, b: &JsonValue) -> bool {
    match (a, b) {
        (JsonValue::Number(x), JsonValue::Number(y)) => match (x.as_f64(), y.as_f64()) {
            (Some(x), Some(y)) => (x - y).abs() <= 1e-9,
            _ => false,
        },
        (JsonValue::Array(x), JsonValue::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(a, b)| same_json(a, b))
        }
        (JsonValue::Object(x), JsonValue::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).is_some_and(|w| same_json(v, w)))
        }
        _ => a == b,
    }
}

/// Compute a signed `sun_position@1` record again from the inputs it holds.
/// `Ok` when every row and every day matches to the stated rounding.
pub(crate) fn recompute_matches(record: &JsonValue) -> Result<(), String> {
    if record["fn_key"] != json!(FN_KEY) {
        return Err(format!("the record is not {FN_KEY}"));
    }
    let (places, times) = inputs_of(record)?;
    let again = compute(&places, &times);
    for key in ["rows", "days"] {
        let (signed, now) = (&record[key], &again[key]);
        let (Some(s), Some(n)) = (signed.as_array(), now.as_array()) else {
            return Err(format!("the record has no `{key}`"));
        };
        if s.len() != n.len() {
            return Err(format!(
                "`{key}`: the record holds {} and its inputs give {}",
                s.len(),
                n.len()
            ));
        }
        if let Some(i) = (0..s.len()).find(|i| !same_json(&s[*i], &n[*i])) {
            return Err(format!("`{key}` {i}: signed {}, computed {}", s[i], n[i]));
        }
    }
    Ok(())
}

/// What a derivation is signed as: where, under which band, op and
/// function, over which days, and from which signed facts.
pub(crate) struct Signing<'a> {
    pub cell: &'a str,
    pub band: &'a str,
    pub op: &'a str,
    pub fn_key: &'a str,
    pub days: [u64; 2],
    pub parents: Vec<FactCid>,
}

/// Sign `record` as one derivative fact and return its cid.
pub(crate) async fn sign_derivation(
    s: &AppState,
    how: Signing<'_>,
    record: &JsonValue,
) -> Result<String, ApiError> {
    let fn_key = how.fn_key;
    let signed_at = emem_storage::server::iso8601_now();
    let fact = DerivativeFact {
        cell: how.cell.to_string(),
        band: how.band.to_string(),
        tslot_window: how.days,
        op: how.op.to_string(),
        parents: how.parents,
        value: json_to_cbor(record),
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
    .map_err(|e| upstream_error(format!("{fn_key} attestation: {e}")))?;
    let cids = s
        .storage
        .put_attestation_detached(&att)
        .await
        .map_err(ApiError::from)?;
    cids.first()
        .map(|c| c.as_str().to_string())
        .ok_or_else(|| upstream_error(format!("store returned no cid for {fn_key}")))
}

/// What a resolve of a token whose record holds its own inputs does.
pub(crate) const ON_RESOLVE: &str = "a resolve of the token computes the answer again from the inputs the record holds, and sets provenance.responder_recomputed to true only when every value matches";

/// Add the token, its provenance and a receipt over the signed fact (and
/// the facts it read) to an answer.
pub(crate) fn finish(
    s: &AppState,
    mut out: JsonValue,
    route: &'static str,
    cell: &str,
    cids: Vec<FactCid>,
    started: Instant,
    provenance: JsonValue,
) -> JsonValue {
    let cid = cids
        .first()
        .map(|c| c.as_str().to_string())
        .unwrap_or_default();
    let receipt = s.sign_receipt(route, vec![cell.to_string()], cids, false, started, None);
    if let Some(o) = out.as_object_mut() {
        o.insert("derivation_cid".into(), json!(cid));
        o.insert("token".into(), json!(format!("emem:fact:{cell}:{cid}")));
        o.insert("provenance".into(), provenance);
        o.insert(
            "receipt".into(),
            serde_json::to_value(&receipt).unwrap_or(JsonValue::Null),
        );
    }
    out
}

pub(crate) async fn sun_position(req: SunPositionReq, s: &AppState) -> Result<JsonValue, ApiError> {
    let started = Instant::now();
    let places = parse_places(&req.places).map_err(bad_request)?;
    let times =
        instants(req.times.as_deref(), req.range.as_ref(), places.len()).map_err(bad_request)?;
    let record = record(&places, &times, &req);
    let cell = places[0]
        .cell
        .clone()
        .unwrap_or_else(|| emem_codec::geo::cell64_from_latlng(places[0].lat, places[0].lng));
    let first = times
        .iter()
        .min()
        .copied()
        .unwrap_or(0)
        .div_euclid(86_400)
        .max(0) as u64;
    let last = times
        .iter()
        .max()
        .copied()
        .unwrap_or(0)
        .div_euclid(86_400)
        .max(0) as u64;
    let how = Signing {
        cell: &cell,
        band: "sun.position",
        op: "sun_position",
        fn_key: FN_KEY,
        days: [first, last],
        parents: vec![],
    };
    let cid = sign_derivation(s, how, &record).await?;
    let provenance = json!({"class": "deterministic_index", "computed_by_responder": true, "on_resolve": ON_RESOLVE});
    Ok(finish(
        s,
        record,
        "emem.sun_position",
        &cell,
        vec![FactCid::new(cid)],
        started,
        provenance,
    ))
}

pub async fn post_sun_position(
    State(s): State<AppState>,
    EmemJson(req): EmemJson<SunPositionReq>,
) -> Result<Json<JsonValue>, ApiError> {
    Ok(Json(sun_position(req, &s).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unix(s: &str) -> f64 {
        parse_instant(s).unwrap() as f64
    }

    /// Minutes between an event and a published time `HH:MM` on `date` in a
    /// zone `tz` hours east of UTC.
    fn minutes_off(event: f64, date: &str, hhmm: &str, tz: f64) -> f64 {
        let local = unix(&format!("{date}T{hhmm}:00Z"));
        (event - (local - tz * 3600.0)) / 60.0
    }

    /// Meeus, Astronomical Algorithms, example 25.a: on 1992 October 13.0 TD
    /// the low-accuracy method gives a declination of -7.78507 degrees. The
    /// spreadsheet's equations are that method.
    #[test]
    fn the_declination_matches_meeus_example_25a() {
        let (dec, _) = declination_and_eot(julian_century(unix("1992-10-13T00:00:00Z")));
        assert!((dec - -7.78507).abs() < 0.0005, "{dec}");
    }

    /// Published sunrise, transit and sunset from the US Naval Observatory
    /// (aa.usno.navy.mil, Rise/Set/Transit API, read 14 November 2026, to the
    /// minute). Each event here is within one minute of the published one.
    #[test]
    fn sunrise_noon_and_sunset_match_published_times_within_a_minute() {
        // (lat, lng, date, tz east of UTC, rise, transit, set)
        let cases = [
            // New Delhi, 21 March 2026, IST.
            (
                28.6139,
                77.2090,
                "2026-03-21",
                5.5,
                "06:24",
                "12:28",
                "18:33",
            ),
            // Sydney, 21 June 2026, AEST. Its sunrise is on 20 June in UTC.
            (
                -33.8688,
                151.2093,
                "2026-06-21",
                10.0,
                "07:00",
                "11:57",
                "16:54",
            ),
            // Greenwich, 21 December 2026, UTC.
            (
                51.4769,
                -0.0005,
                "2026-12-21",
                0.0,
                "08:03",
                "11:58",
                "15:53",
            ),
            // 0 N 0 E, 23 September 2026, UTC.
            (0.0, 0.0, "2026-09-23", 0.0, "05:49", "11:52", "17:56"),
        ];
        for (lat, lng, date, tz, rise, transit, set) in cases {
            let ev = day_events(lat, lng, unix(&format!("{date}T00:00:00Z")));
            let r = minutes_off(ev.rise.unwrap(), date, rise, tz);
            let n = minutes_off(ev.noon, date, transit, tz);
            let s = minutes_off(ev.set.unwrap(), date, set, tz);
            for (what, off) in [("rise", r), ("transit", n), ("set", s)] {
                assert!(
                    off.abs() <= 1.0,
                    "{date} {lat},{lng} {what}: {off:.2} min off"
                );
            }
        }
    }

    /// At the equator on the March equinox the sun at solar noon stands
    /// within a few tenths of a degree of the zenith.
    #[test]
    fn the_equinox_noon_sun_at_the_equator_is_overhead() {
        let ev = day_events(0.0, 0.0, unix("2025-03-20T00:00:00Z"));
        let s = sun_at(0.0, 0.0, ev.noon);
        assert!(s.elevation_geometric_deg > 89.5, "{s:?}");
        assert_eq!(
            s.elevation_deg, s.elevation_geometric_deg,
            "no refraction above 85 degrees"
        );
        // At the transit the sun is due north or due south.
        let off_meridian = (s.azimuth_deg - 180.0)
            .abs()
            .min(s.azimuth_deg.min(360.0 - s.azimuth_deg));
        assert!(off_meridian < 0.5, "{s:?}");
    }

    /// The sun rises in the east, stands south at a northern noon, sets in
    /// the west, and refraction lifts it most at the horizon.
    #[test]
    fn azimuth_and_refraction_follow_the_day() {
        let ev = day_events(28.6139, 77.2090, unix("2026-03-21T00:00:00Z"));
        let morning = sun_at(28.6139, 77.2090, ev.noon - 4.0 * 3600.0);
        let noon = sun_at(28.6139, 77.2090, ev.noon);
        let evening = sun_at(28.6139, 77.2090, ev.noon + 4.0 * 3600.0);
        assert!((60.0..120.0).contains(&morning.azimuth_deg), "{morning:?}");
        assert!((noon.azimuth_deg - 180.0).abs() < 0.5, "{noon:?}");
        assert!((240.0..300.0).contains(&evening.azimuth_deg), "{evening:?}");
        // At sunrise the sun's centre is 0.833 degrees below the horizon.
        let rise = sun_at(28.6139, 77.2090, ev.rise.unwrap());
        assert!(
            (rise.elevation_geometric_deg - (90.0 - RISE_ZENITH_DEG)).abs() < 0.01,
            "{rise:?}"
        );
        assert!((refraction_deg(0.0) - 1735.0 / 3600.0).abs() < 1e-12);
        assert_eq!(refraction_deg(86.0), 0.0);
    }

    #[test]
    fn a_polar_day_and_a_polar_night_have_no_sunrise() {
        let summer = day_events(78.2232, 15.6267, unix("2026-06-21T00:00:00Z"));
        assert_eq!(
            (summer.rise, summer.set, summer.polar),
            (None, None, Some(true))
        );
        let winter = day_events(78.2232, 15.6267, unix("2026-12-21T00:00:00Z"));
        assert_eq!(
            (winter.rise, winter.set, winter.polar),
            (None, None, Some(false))
        );
    }

    #[test]
    fn instants_parse_utc_only_and_whole_seconds() {
        assert_eq!(parse_instant("1970-01-01T00:00:00Z"), Ok(0));
        assert_eq!(parse_instant("2026-03-21T06:00Z"), Ok(1_774_072_800));
        assert_eq!(
            parse_instant("2026-03-21T06:00:00+00:00"),
            parse_instant("2026-03-21 06:00:00Z")
        );
        for bad in [
            "2026-03-21T06:00:00",
            "2026-03-21T06:00:00+05:30",
            "2026-03-21T06:00:00.5Z",
            "2026-02-30T06:00:00Z",
            "2026-03-21T24:00:00Z",
            "2150-03-21T06:00:00Z",
        ] {
            assert!(parse_instant(bad).is_err(), "{bad}");
        }
    }

    /// 50 places is the most; 50 places x 40 times is the 2,000-row cap, and
    /// one more row is refused before any is computed.
    #[test]
    fn the_row_cap_holds_for_times_and_ranges() {
        let many: Vec<JsonValue> = (0..=MAX_PLACES)
            .map(|i| json!({"lat": i as f64, "lng": 0.0}))
            .collect();
        assert!(parse_places(&JsonValue::Array(many[..MAX_PLACES].to_vec())).is_ok());
        assert!(parse_places(&JsonValue::Array(many)).is_err());
        assert!(parse_places(&json!([{"lat": 1.0, "lng": 2.0, "cell": "x"}])).is_err());
        let times: Vec<String> = (0..40)
            .map(|h| iso_second(1_774_051_200 + h * 3600))
            .collect();
        assert_eq!(instants(Some(&times), None, 50).unwrap().len(), 40);
        let mut one_more = times.clone();
        one_more.push(iso_second(1_774_051_200));
        assert!(instants(Some(&one_more), None, 50).is_err());
        let range = |to: &str, step| SunRange {
            from: "2026-03-21T00:00:00Z".into(),
            to: to.into(),
            step_minutes: step,
        };
        // 00:00 to 23:59 every minute is 1,440 rows for one place.
        assert_eq!(
            instants(None, Some(&range("2026-03-21T23:59:00Z", 1)), 1)
                .unwrap()
                .len(),
            1440
        );
        assert!(instants(None, Some(&range("2026-03-21T23:59:00Z", 1)), 2).is_err());
        assert!(instants(None, Some(&range("2026-03-21T23:59:00Z", 0)), 1).is_err());
        assert!(instants(None, Some(&range("2026-03-20T00:00:00Z", 60)), 1).is_err());
        assert!(instants(Some(&times), Some(&range("2026-03-21T23:59:00Z", 60)), 1).is_err());
    }

    /// Several places and times: one row per place and time, place-major,
    /// one day row per place and UTC date, and the signed record computes
    /// again to the same rows. A changed number does not.
    #[test]
    fn several_places_and_times_sign_a_record_that_computes_again() {
        let req = SunPositionReq {
            places: json!([{"id": "delhi", "lat": 28.6139, "lng": 77.209}, {"cell": emem_codec::geo::cell64_from_latlng(-33.8688, 151.2093)}]),
            times: Some(vec![
                "2026-03-21T06:00:00Z".into(),
                "2026-03-22T06:00:00Z".into(),
                "2026-03-22T07:30Z".into(),
            ]),
            range: None,
        };
        let places = parse_places(&req.places).unwrap();
        let times = instants(req.times.as_deref(), None, places.len()).unwrap();
        let rec = record(&places, &times, &req);
        assert_eq!(rec["rows"].as_array().unwrap().len(), 6);
        assert_eq!(rec["days"].as_array().unwrap().len(), 4);
        assert_eq!(rec["rows"][3][0], json!(1));
        assert_eq!(rec["rows"][2][1], json!("2026-03-22T07:30:00Z"));
        assert_eq!(rec["days"][1][1], json!("2026-03-22"));
        assert_eq!(
            rec["inputs"]["places"][1]["cell"],
            places[1].cell.clone().unwrap()
        );
        let signed: JsonValue = serde_json::to_value(json_to_cbor(&rec)).unwrap();
        assert_eq!(recompute_matches(&signed), Ok(()));
        let mut forged = signed.clone();
        forged["rows"][4][3] = json!(forged["rows"][4][3].as_f64().unwrap() + 0.0001);
        assert!(recompute_matches(&forged).is_err());
        let mut moved = signed;
        moved["inputs"]["places"][0]["lat"] = json!(28.7);
        assert!(recompute_matches(&moved).is_err());
        // A range signs its expansion and computes again from the range.
        let req = SunPositionReq {
            places: json!([{"lat": 0.0, "lng": 0.0}]),
            times: None,
            range: Some(SunRange {
                from: "2026-09-23T05:00:00Z".into(),
                to: "2026-09-23T07:00:00Z".into(),
                step_minutes: 30,
            }),
        };
        let places = parse_places(&req.places).unwrap();
        let times = instants(None, req.range.as_ref(), 1).unwrap();
        let rec = record(&places, &times, &req);
        assert_eq!(rec["rows"].as_array().unwrap().len(), 5);
        let signed: JsonValue = serde_json::to_value(json_to_cbor(&rec)).unwrap();
        assert_eq!(recompute_matches(&signed), Ok(()));
    }
}
