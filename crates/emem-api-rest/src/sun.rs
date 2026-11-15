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
    /// A Sentinel-2 L2A item id. The answer adds the sun its granule's own
    /// angle grid gives at each place, at the granule's sensing time. With
    /// no `times` or `range`, the rows are at that sensing time.
    #[serde(default)]
    pub stac_item: Option<String>,
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

/// The record a request signs, before its cid is known. `granule` is the
/// block a `stac_item` adds.
fn record(
    places: &[Place],
    times: &[i64],
    req: &SunPositionReq,
    granule: Option<JsonValue>,
) -> JsonValue {
    let mut inputs = match &req.range {
        Some(r) => json!({
            "places": places.iter().map(Place::to_json).collect::<Vec<_>>(),
            "range": {"from": iso_second(times[0]), "to": r.to.trim(), "step_minutes": r.step_minutes},
        }),
        None => json!({
            "places": places.iter().map(Place::to_json).collect::<Vec<_>>(),
            "times": times.iter().map(|t| iso_second(*t)).collect::<Vec<_>>(),
        }),
    };
    if let (Some(id), Some(i)) = (req.stac_item.as_deref(), inputs.as_object_mut()) {
        i.insert("stac_item".into(), json!(id.trim()));
    }
    let body = compute(places, times);
    let mut out = json!({
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
    });
    if let (Some(g), Some(o)) = (granule, out.as_object_mut()) {
        o.insert("granule".into(), g);
    }
    out
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
    match record.get("granule") {
        Some(g) if !g.is_null() => granule_recompute(&places, g),
        _ => Ok(()),
    }
}

/// What a Sentinel-2 L2A granule's own metadata (`MTD_TL.xml`) says about
/// the sun: when the tile was sensed, and the sun angle grid it publishes.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GranuleSun {
    /// `SENSING_TIME` as the metadata writes it, e.g.
    /// `2026-03-06T05:41:22.395934Z`.
    pub sensing_time: String,
    /// `HORIZONTAL_CS_CODE`, e.g. `EPSG:32643`.
    pub epsg: u32,
    /// The tile's upper-left corner (`Geoposition resolution="10"`).
    pub ulx: f64,
    pub uly: f64,
    /// `COL_STEP`, `ROW_STEP` of the sun grid, metres.
    pub col_step: f64,
    pub row_step: f64,
    /// `Sun_Angles_Grid` values, row by row from the north.
    pub zenith: Vec<Vec<f64>>,
    pub azimuth: Vec<Vec<f64>>,
}

/// The text of the first `<tag ...>text</tag>` in `xml`.
fn xml_text<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}");
    let mut from = 0;
    loop {
        let at = from + xml[from..].find(&open)?;
        let rest = &xml[at + open.len()..];
        // `<ULX>`, not `<ULX_OTHER>`.
        if rest.starts_with('>') || rest.starts_with(' ') {
            let body = &rest[rest.find('>')? + 1..];
            let end = body.find(&format!("</{tag}>"))?;
            return Some(body[..end].trim());
        }
        from = at + open.len();
    }
}

/// The part of `xml` from the first `start` to the first `end` after it.
fn xml_block<'a>(xml: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let rest = &xml[xml.find(start)?..];
    Some(&rest[..rest.find(end)? + end.len()])
}

/// A sun grid's angle grid (`<Zenith>` or `<Azimuth>`): its steps and its
/// values. A `NaN` value stays NaN.
type AngleGrid = (f64, f64, Vec<Vec<f64>>);

fn angle_grid(block: &str) -> Result<AngleGrid, String> {
    let step = |tag: &str| -> Result<f64, String> {
        xml_text(block, tag)
            .and_then(|t| t.parse::<f64>().ok())
            .filter(|v| *v > 0.0)
            .ok_or_else(|| format!("the sun grid has no {tag}"))
    };
    let (col_step, row_step) = (step("COL_STEP")?, step("ROW_STEP")?);
    let mut rows = Vec::new();
    let mut rest = block;
    while let Some(at) = rest.find("<VALUES>") {
        let body = &rest[at + "<VALUES>".len()..];
        let end = body
            .find("</VALUES>")
            .ok_or("an unclosed VALUES row in the sun grid")?;
        let row = body[..end]
            .split_ascii_whitespace()
            .map(|v| {
                v.parse::<f64>()
                    .map_err(|e| format!("sun grid value `{v}`: {e}"))
            })
            .collect::<Result<Vec<f64>, String>>()?;
        rows.push(row);
        rest = &body[end..];
    }
    if rows.len() < 2 || rows[0].len() < 2 || rows.iter().any(|r| r.len() != rows[0].len()) {
        return Err("the sun grid is not a rectangle of at least 2 by 2 values".into());
    }
    Ok((col_step, row_step, rows))
}

/// Read a granule's sensing time and sun grid from its `MTD_TL.xml`.
pub(crate) fn parse_granule_sun(xml: &str) -> Result<GranuleSun, String> {
    let sensing_time = xml_text(xml, "SENSING_TIME")
        .ok_or("the granule metadata has no SENSING_TIME")?
        .to_string();
    sensing_unix(&sensing_time)?;
    let epsg = xml_text(xml, "HORIZONTAL_CS_CODE")
        .and_then(|c| c.strip_prefix("EPSG:"))
        .and_then(|c| c.parse::<u32>().ok())
        .ok_or("the granule metadata has no EPSG HORIZONTAL_CS_CODE")?;
    let geo = xml_block(xml, "<Geoposition resolution=\"10\">", "</Geoposition>")
        .ok_or("the granule metadata has no 10 m Geoposition")?;
    let corner = |tag: &str| -> Result<f64, String> {
        xml_text(geo, tag)
            .and_then(|t| t.parse::<f64>().ok())
            .ok_or_else(|| format!("the 10 m Geoposition has no {tag}"))
    };
    let (ulx, uly) = (corner("ULX")?, corner("ULY")?);
    let grid = xml_block(xml, "<Sun_Angles_Grid>", "</Sun_Angles_Grid>")
        .ok_or("the granule metadata has no Sun_Angles_Grid")?;
    let zen = xml_block(grid, "<Zenith>", "</Zenith>").ok_or("the sun grid has no Zenith")?;
    let az = xml_block(grid, "<Azimuth>", "</Azimuth>").ok_or("the sun grid has no Azimuth")?;
    let (col_step, row_step, zenith) = angle_grid(zen)?;
    let (az_col, az_row, azimuth) = angle_grid(az)?;
    if (az_col, az_row) != (col_step, row_step)
        || azimuth.len() != zenith.len()
        || azimuth[0].len() != zenith[0].len()
    {
        return Err("the sun zenith and azimuth grids differ in shape".into());
    }
    Ok(GranuleSun {
        sensing_time,
        epsg,
        ulx,
        uly,
        col_step,
        row_step,
        zenith,
        azimuth,
    })
}

/// Unix seconds, with the fraction, of a metadata instant such as
/// `2026-03-06T05:41:22.395934Z`.
pub(crate) fn sensing_unix(s: &str) -> Result<f64, String> {
    let t = s.trim().trim_end_matches(['Z', 'z']);
    let (whole, frac) = t.split_once('.').unwrap_or((t, ""));
    let secs = parse_instant(&format!("{whole}Z"))?;
    if frac.is_empty() {
        return Ok(secs as f64);
    }
    if !frac.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("`{s}` is not an instant"));
    }
    let frac: f64 = format!("0.{frac}")
        .parse()
        .map_err(|e| format!("`{s}`: {e}"))?;
    Ok(secs as f64 + frac)
}

/// What a grid row needs besides the place and its four nodes.
#[derive(Debug, Clone, PartialEq)]
struct GridHeader {
    epsg: u32,
    ulx: f64,
    uly: f64,
    col_step: f64,
    row_step: f64,
    sensing_time: String,
}

impl GridHeader {
    fn of(g: &GranuleSun) -> Self {
        GridHeader {
            epsg: g.epsg,
            ulx: g.ulx,
            uly: g.uly,
            col_step: g.col_step,
            row_step: g.row_step,
            sensing_time: g.sensing_time.clone(),
        }
    }

    /// The header a signed `granule` block holds.
    fn from_record(r: &JsonValue) -> Result<Self, String> {
        let num = |v: &JsonValue, what: &str| {
            v.as_f64()
                .ok_or_else(|| format!("the granule has no {what}"))
        };
        Ok(GridHeader {
            epsg: r["crs"]
                .as_str()
                .and_then(|c| c.strip_prefix("EPSG:"))
                .and_then(|c| c.parse().ok())
                .ok_or("the granule has no crs")?,
            ulx: num(&r["grid"]["origin"][0], "grid origin")?,
            uly: num(&r["grid"]["origin"][1], "grid origin")?,
            col_step: num(&r["grid"]["step_m"][0], "grid step")?,
            row_step: num(&r["grid"]["step_m"][1], "grid step")?,
            sensing_time: r["sensing_time"]
                .as_str()
                .ok_or("the granule has no sensing_time")?
                .to_string(),
        })
    }
}

/// Where a place falls on a granule's sun grid: its easting and northing in
/// the tile's CRS (to the millimetre), and its column and row on the grid
/// (to a millionth), whose node `(r, c)` is at
/// `(ULX + c * COL_STEP, ULY - r * ROW_STEP)`.
fn grid_position(g: &GridHeader, lat: f64, lng: f64) -> Option<[f64; 4]> {
    let u = emem_fetch::proj::latlng_to_utm_with_epsg(lat, lng, g.epsg)?;
    let x = (u.easting * 1e3).round() / 1e3;
    let y = (u.northing * 1e3).round() / 1e3;
    let col = ((x - g.ulx) / g.col_step * 1e6).round() / 1e6;
    let row = ((g.uly - y) / g.row_step * 1e6).round() / 1e6;
    Some([x, y, col, row])
}

/// The grid cell that holds `(col, row)` (its upper-left node), and its four
/// nodes in the order (r, c), (r, c+1), (r+1, c), (r+1, c+1). `None`
/// outside the grid or when a node is NaN.
type Nodes = ([usize; 2], [f64; 4], [f64; 4]);

fn nodes_at(g: &GranuleSun, col: f64, row: f64) -> Option<Nodes> {
    let (rows, cols) = (g.zenith.len(), g.zenith.first()?.len());
    if !(0.0..=(cols - 1) as f64).contains(&col) || !(0.0..=(rows - 1) as f64).contains(&row) {
        return None;
    }
    let c = (col.floor() as usize).min(cols - 2);
    let r = (row.floor() as usize).min(rows - 2);
    let four = |grid: &[Vec<f64>]| {
        [
            grid[r][c],
            grid[r][c + 1],
            grid[r + 1][c],
            grid[r + 1][c + 1],
        ]
    };
    let (z, a) = (four(&g.zenith), four(&g.azimuth));
    z.iter()
        .chain(a.iter())
        .all(|v| v.is_finite())
        .then_some(([r, c], z, a))
}

/// Bilinear interpolation at `(fc, fr)`, each 0 to 1, inside four nodes.
fn bilinear(n: [f64; 4], fc: f64, fr: f64) -> f64 {
    let top = n[0] + (n[1] - n[0]) * fc;
    let bottom = n[2] + (n[3] - n[2]) * fc;
    top + (bottom - top) * fr
}

/// An azimuth difference folded into [-180, 180).
fn azimuth_diff(a: f64, b: f64) -> f64 {
    (a - b + 180.0).rem_euclid(360.0) - 180.0
}

/// The columns of a `granule.rows` row.
const GRANULE_COLUMNS: [&str; 15] = [
    "place",
    "x",
    "y",
    "grid_col",
    "grid_row",
    "grid_cell",
    "zenith_nodes",
    "azimuth_nodes",
    "zenith_deg",
    "azimuth_deg",
    "elevation_deg",
    "ephemeris_azimuth_deg",
    "ephemeris_elevation_deg",
    "d_azimuth_deg",
    "d_elevation_deg",
];

/// One grid row: the sun the granule's grid gives at a place, beside the
/// ephemeris at the sensing time, and grid minus ephemeris. The nodes are
/// inputs, so a resolve computes the row again from the record alone.
fn granule_row(
    i: usize,
    p: &Place,
    g: &GridHeader,
    nodes: Option<Nodes>,
) -> Result<JsonValue, String> {
    let eph = sun_at(p.lat, p.lng, sensing_unix(&g.sensing_time)?);
    let (eph_az, eph_el) = (
        round_angle(eph.azimuth_deg),
        round_angle(eph.elevation_geometric_deg),
    );
    let pos = grid_position(g, p.lat, p.lng);
    let mut row = vec![json!(i)];
    row.extend(match pos {
        Some(v) => v.map(|x| json!(x)).to_vec(),
        None => vec![JsonValue::Null; 4],
    });
    let interpolated = match (pos, nodes) {
        (Some([_, _, col, gr]), Some(([r, c], zn, an))) => {
            let (fc, fr) = (col - c as f64, gr - r as f64);
            if !(0.0..=1.0).contains(&fc) || !(0.0..=1.0).contains(&fr) {
                return Err(format!(
                    "place {i}: grid position ({col}, {gr}) is not in grid cell ({r}, {c})"
                ));
            }
            // Azimuths are unwrapped about the first node, so a cell that
            // crosses north does not average 359 and 1 to 180.
            let unwrapped = an.map(|a| an[0] + azimuth_diff(a, an[0]));
            let zenith = round_angle(bilinear(zn, fc, fr));
            let azimuth = round_angle(bilinear(unwrapped, fc, fr)).rem_euclid(360.0);
            let elevation = round_angle(90.0 - zenith);
            Some((json!([r, c]), zn, an, zenith, azimuth, elevation))
        }
        _ => None,
    };
    match interpolated {
        Some((cell, zn, an, zenith, azimuth, elevation)) => row.extend([
            cell,
            json!(zn),
            json!(an),
            json!(zenith),
            json!(azimuth),
            json!(elevation),
            json!(eph_az),
            json!(eph_el),
            json!(round_angle(azimuth_diff(azimuth, eph_az))),
            json!(round_angle(elevation - eph_el)),
        ]),
        None => row.extend([
            JsonValue::Null,
            JsonValue::Null,
            JsonValue::Null,
            JsonValue::Null,
            JsonValue::Null,
            JsonValue::Null,
            json!(eph_az),
            json!(eph_el),
            JsonValue::Null,
            JsonValue::Null,
        ]),
    }
    Ok(JsonValue::Array(row))
}

/// The nodes a signed grid row holds, or `None` for a row off the grid.
fn nodes_of_row(row: &JsonValue) -> Result<Option<Nodes>, String> {
    if row[5].is_null() {
        return Ok(None);
    }
    let idx = |k: usize| {
        row[5][k]
            .as_u64()
            .map(|v| v as usize)
            .ok_or("a grid row's grid_cell is not two indices")
    };
    let four = |col: usize| -> Result<[f64; 4], String> {
        let a = row[col]
            .as_array()
            .filter(|a| a.len() == 4)
            .ok_or("a grid row's nodes are not four numbers")?;
        let mut out = [0.0; 4];
        for (o, v) in out.iter_mut().zip(a) {
            *o = v.as_f64().ok_or("a grid node is not a number")?;
        }
        Ok(out)
    };
    Ok(Some(([idx(0)?, idx(1)?], four(6)?, four(7)?)))
}

/// Where a granule's sun grid came from.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GranuleSource {
    pub item: String,
    /// The STAC item URL.
    pub catalogue: String,
    /// The granule metadata URL, without a token.
    pub metadata: String,
    /// The item's `datetime`: on Planetary Computer the datatake start,
    /// which can be minutes before the tile's own sensing time.
    pub item_datetime: String,
}

/// The `granule` block of a record: where the grid came from, the sensing
/// time, the grid's frame, and one row per place.
fn granule_block(
    places: &[Place],
    g: &GranuleSun,
    src: &GranuleSource,
) -> Result<JsonValue, String> {
    let h = GridHeader::of(g);
    let rows = places
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let nodes = grid_position(&h, p.lat, p.lng).and_then(|[_, _, c, r]| nodes_at(g, c, r));
            granule_row(i, p, &h, nodes)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let lag = match (
        sensing_unix(&g.sensing_time),
        sensing_unix(&src.item_datetime),
    ) {
        (Ok(a), Ok(b)) => json!(((a - b) * 1e3).round() / 1e3),
        _ => JsonValue::Null,
    };
    Ok(json!({
        "stac_item": src.item,
        "catalogue": src.catalogue,
        "metadata": src.metadata,
        "item_datetime": src.item_datetime,
        "sensing_time": g.sensing_time,
        "sensing_minus_item_datetime_s": lag,
        "crs": format!("EPSG:{}", g.epsg),
        "grid": {
            "origin": [g.ulx, g.uly],
            "step_m": [g.col_step, g.row_step],
            "shape": [g.zenith.len(), g.zenith[0].len()],
            "nodes": "Sun_Angles_Grid of the granule metadata: the node in row r and column c is at (ULX + c * COL_STEP, ULY - r * ROW_STEP), the first at the tile's upper-left corner",
        },
        "method": "each place is projected to the tile's CRS, and the zenith and azimuth are interpolated bilinearly between the four grid nodes around it (azimuths unwrapped about the first node). elevation_deg is 90 minus that zenith, geometric, as the granule gives it. The ephemeris columns are sun_position@1 at the sensing time, geometric elevation. d_azimuth_deg and d_elevation_deg are grid minus ephemeris. A place off the grid has the ephemeris only",
        "columns": GRANULE_COLUMNS,
        "rows": rows,
    }))
}

/// Compute a signed `granule` block's rows again from the nodes and the
/// sensing time it holds.
fn granule_recompute(places: &[Place], block: &JsonValue) -> Result<(), String> {
    let h = GridHeader::from_record(block)?;
    let rows = block["rows"]
        .as_array()
        .ok_or("the granule block has no rows")?;
    if rows.len() != places.len() {
        return Err(format!(
            "`granule.rows`: the record holds {} and its inputs give {}",
            rows.len(),
            places.len()
        ));
    }
    for (i, (signed, p)) in rows.iter().zip(places).enumerate() {
        let again = granule_row(i, p, &h, nodes_of_row(signed)?)?;
        if !same_json(signed, &again) {
            return Err(format!(
                "`granule.rows` {i}: signed {signed}, computed {again}"
            ));
        }
    }
    Ok(())
}

/// Fetch a Sentinel-2 L2A item and its granule metadata, and read the
/// granule's sensing time and sun grid.
async fn fetch_granule_sun(id: &str) -> Result<(GranuleSun, GranuleSource), ApiError> {
    let url = crate::shadow::s2_item_url(id, "stac_item")?;
    let cli = crate::s2_http_client();
    let get = |u: String| {
        cli.get(u)
            .header("user-agent", emem_core::outbound::user_agent())
            .timeout(std::time::Duration::from_secs(25))
            .send()
    };
    let resp = get(url.clone())
        .await
        .map_err(|e| upstream_error(format!("stac item {id}: {e}")))?;
    if resp.status().as_u16() == 404 {
        return Err(bad_request(format!(
            "`stac_item` `{id}`: the catalogue has no such item ({url})"
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
    // Planetary Computer names the asset granule-metadata, Earth Search
    // granule_metadata.
    let href = ["granule-metadata", "granule_metadata"]
        .iter()
        .find_map(|k| item["assets"][*k]["href"].as_str())
        .ok_or_else(|| upstream_error(format!("stac item {id} links no granule metadata")))?
        .to_string();
    let signed = if href.contains(".blob.core.windows.net/") {
        let sas = emem_fetch::stac::mpc_sas_token(&cli, "sentinel-2-l2a")
            .await
            .map_err(|e| upstream_error(format!("granule metadata token: {e}")))?;
        format!("{href}?{sas}")
    } else {
        href.clone()
    };
    let resp = get(signed)
        .await
        .map_err(|e| upstream_error(format!("granule metadata of {id}: {e}")))?;
    if !resp.status().is_success() {
        return Err(upstream_error(format!(
            "granule metadata of {id}: status {}",
            resp.status()
        )));
    }
    let xml = resp
        .text()
        .await
        .map_err(|e| upstream_error(format!("granule metadata of {id}: {e}")))?;
    let g = parse_granule_sun(&xml)
        .map_err(|e| upstream_error(format!("granule metadata of {id}: {e}")))?;
    Ok((
        g,
        GranuleSource {
            item: id.to_string(),
            catalogue: url,
            metadata: href,
            item_datetime: item["properties"]["datetime"]
                .as_str()
                .unwrap_or("")
                .to_string(),
        },
    ))
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
    let granule = match req.stac_item.as_deref() {
        Some(id) => Some(fetch_granule_sun(id.trim()).await?),
        None => None,
    };
    // With an item and no instants, the rows are at the granule's sensing
    // time, to the second.
    let times = match &granule {
        Some((g, _)) if req.times.is_none() && req.range.is_none() => {
            vec![sensing_unix(&g.sensing_time)
                .map_err(upstream_error)?
                .round() as i64]
        }
        _ => {
            instants(req.times.as_deref(), req.range.as_ref(), places.len()).map_err(bad_request)?
        }
    };
    let block = match &granule {
        Some((g, src)) => Some(granule_block(&places, g, src).map_err(upstream_error)?),
        None => None,
    };
    let record = record(&places, &times, &req, block);
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
            stac_item: None,
        };
        let places = parse_places(&req.places).unwrap();
        let times = instants(req.times.as_deref(), None, places.len()).unwrap();
        let rec = record(&places, &times, &req, None);
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
            stac_item: None,
        };
        let places = parse_places(&req.places).unwrap();
        let times = instants(None, req.range.as_ref(), 1).unwrap();
        let rec = record(&places, &times, &req, None);
        assert_eq!(rec["rows"].as_array().unwrap().len(), 5);
        let signed: JsonValue = serde_json::to_value(json_to_cbor(&rec)).unwrap();
        assert_eq!(recompute_matches(&signed), Ok(()));
    }

    /// A hand-made `MTD_TL.xml` in the shape of a real one (T43RGM, 6
    /// March 2026): a 3 by 3 sun grid at 5 km steps, then a viewing grid
    /// with NaN cells that must not be read as the sun's.
    const MTD_TL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<n1:Level-2A_Tile_ID>
  <n1:General_Info>
    <TILE_ID metadataLevel="Brief">S2A_OPER_MSI_L2A_TL_2APS_20260310T110618_A055901_T43RGM_N05.12</TILE_ID>
    <SENSING_TIME metadataLevel="Standard">2026-03-06T05:41:22.395934Z</SENSING_TIME>
  </n1:General_Info>
  <n1:Geometric_Info>
    <Tile_Geocoding metadataLevel="Brief">
      <HORIZONTAL_CS_NAME>WGS84 / UTM zone 43N</HORIZONTAL_CS_NAME>
      <HORIZONTAL_CS_CODE>EPSG:32643</HORIZONTAL_CS_CODE>
      <Geoposition resolution="10">
        <ULX>699960</ULX>
        <ULY>3200040</ULY>
        <XDIM>10</XDIM>
        <YDIM>-10</YDIM>
      </Geoposition>
      <Geoposition resolution="60">
        <ULX>1</ULX>
        <ULY>2</ULY>
      </Geoposition>
    </Tile_Geocoding>
    <Tile_Angles metadataLevel="Standard">
      <Sun_Angles_Grid>
        <Zenith>
          <COL_STEP unit="m">5000</COL_STEP>
          <ROW_STEP unit="m">5000</ROW_STEP>
          <Values_List>
            <VALUES>40 41 42</VALUES>
            <VALUES>40.5 41.5 42.5</VALUES>
            <VALUES>41 42 43</VALUES>
          </Values_List>
        </Zenith>
        <Azimuth>
          <COL_STEP unit="m">5000</COL_STEP>
          <ROW_STEP unit="m">5000</ROW_STEP>
          <Values_List>
            <VALUES>147 148 149</VALUES>
            <VALUES>359 1 150</VALUES>
            <VALUES>359 1 151</VALUES>
          </Values_List>
        </Azimuth>
      </Sun_Angles_Grid>
      <Mean_Sun_Angle>
        <ZENITH_ANGLE unit="deg">39.0746963568344</ZENITH_ANGLE>
        <AZIMUTH_ANGLE unit="deg">147.550829656832</AZIMUTH_ANGLE>
      </Mean_Sun_Angle>
      <Viewing_Incidence_Angles_Grids bandId="0" detectorId="1">
        <Zenith>
          <COL_STEP unit="m">5000</COL_STEP>
          <ROW_STEP unit="m">5000</ROW_STEP>
          <Values_List>
            <VALUES>NaN NaN 3</VALUES>
            <VALUES>NaN NaN 3</VALUES>
          </Values_List>
        </Zenith>
      </Viewing_Incidence_Angles_Grids>
    </Tile_Angles>
  </n1:Geometric_Info>
</n1:Level-2A_Tile_ID>
"#;

    fn granule_source() -> GranuleSource {
        GranuleSource {
            item: "S2A_MSIL2A_20260306T053251_R105_T43RGM_20260310T110618".into(),
            catalogue: "https://planetarycomputer.microsoft.com/api/stac/v1/collections/sentinel-2-l2a/items/S2A_MSIL2A_20260306T053251_R105_T43RGM_20260310T110618".into(),
            metadata: "https://sentinel2l2a01.blob.core.windows.net/x/MTD_TL.xml".into(),
            item_datetime: "2026-03-06T05:32:51.024000Z".into(),
        }
    }

    /// The place at a given easting and northing of the fixture's zone.
    fn place_at(x: f64, y: f64) -> Place {
        let (lat, lng) = emem_fetch::proj::utm_to_latlng_with_epsg(x, y, 32643).unwrap();
        Place {
            id: None,
            cell: None,
            lat,
            lng,
        }
    }

    #[test]
    fn the_granule_metadata_gives_its_sensing_time_and_sun_grid() {
        let g = parse_granule_sun(MTD_TL).unwrap();
        assert_eq!(g.sensing_time, "2026-03-06T05:41:22.395934Z");
        assert_eq!(g.epsg, 32643);
        assert_eq!((g.ulx, g.uly), (699_960.0, 3_200_040.0), "the 10 m corner");
        assert_eq!((g.col_step, g.row_step), (5000.0, 5000.0));
        assert_eq!(g.zenith[1], vec![40.5, 41.5, 42.5]);
        assert_eq!(g.azimuth[2], vec![359.0, 1.0, 151.0]);
        let t = sensing_unix(&g.sensing_time).unwrap();
        assert!((t - (unix("2026-03-06T05:41:22Z") + 0.395934)).abs() < 1e-6);
        assert!(parse_granule_sun("<x/>").is_err());
        assert!(sensing_unix("2026-03-06T05:41:22.3x9Z").is_err());
    }

    /// A place half a step east and one and a half down from the corner:
    /// the zenith is the mean of its four nodes, the azimuth crosses north
    /// without averaging to 180, and the ephemeris at the sensing time
    /// stands beside both with the difference.
    #[test]
    fn the_grid_sun_is_bilinear_and_computes_again() {
        let g = parse_granule_sun(MTD_TL).unwrap();
        let places = vec![
            place_at(699_960.0 + 2_500.0, 3_200_040.0 - 7_500.0),
            place_at(699_960.0 + 7_500.0, 3_200_040.0 - 2_500.0),
            place_at(699_960.0 + 30_000.0, 3_200_040.0 - 2_500.0),
        ];
        let block = granule_block(&places, &g, &granule_source()).unwrap();
        assert_eq!(block["sensing_minus_item_datetime_s"], json!(511.372));
        let rows = block["rows"].as_array().unwrap();
        let r = &rows[0];
        assert_eq!(r[5], json!([1, 0]));
        assert_eq!(r[8], json!(41.25), "zenith {r}");
        assert_eq!(r[9], json!(0.0), "azimuth {r}");
        assert_eq!(r[10], json!(48.75));
        let eph = sun_at(
            places[0].lat,
            places[0].lng,
            sensing_unix(&g.sensing_time).unwrap(),
        );
        assert_eq!(r[11], json!(round_angle(eph.azimuth_deg)));
        assert_eq!(r[12], json!(round_angle(eph.elevation_geometric_deg)));
        assert_eq!(
            r[14],
            json!(round_angle(
                48.75 - round_angle(eph.elevation_geometric_deg)
            ))
        );
        // The second place: zenith (41 + 42 + 41.5 + 42.5) / 4 = 41.75.
        assert_eq!(rows[1][8], json!(41.75));
        // Off the grid: the ephemeris only.
        assert!(rows[2][8].is_null() && rows[2][5].is_null());
        assert!(rows[2][11].is_number());

        let req = SunPositionReq {
            places: JsonValue::Null,
            times: None,
            range: None,
            stac_item: Some(granule_source().item),
        };
        let times = vec![sensing_unix(&g.sensing_time).unwrap().round() as i64];
        let rec = record(&places, &times, &req, Some(block));
        assert_eq!(rec["inputs"]["stac_item"], json!(granule_source().item));
        assert_eq!(rec["inputs"]["times"], json!(["2026-03-06T05:41:22Z"]));
        let signed: JsonValue = serde_json::to_value(json_to_cbor(&rec)).unwrap();
        assert_eq!(recompute_matches(&signed), Ok(()));
        let mut forged = signed.clone();
        forged["granule"]["rows"][0][6][0] = json!(41.0);
        assert!(recompute_matches(&forged).is_err(), "a node changed");
        let mut late = signed;
        late["granule"]["sensing_time"] = json!("2026-03-06T05:51:22.395934Z");
        assert!(recompute_matches(&late).is_err(), "another sensing time");
    }
}
