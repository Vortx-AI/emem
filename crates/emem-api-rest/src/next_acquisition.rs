//! When the next picture of a place is expected, from each satellite's own
//! repeat cycle.
//!
//! The agent that builds the public geo.qa site asked for "the next planned
//! picture" of a cell. ESA and USGS publish acquisition plans, but as large
//! files at addresses that change. So this answers from what the catalogue
//! already shows. Each satellite repeats its ground track exactly: Sentinel-2
//! every 10 days, Sentinel-1 every 12, Landsat every 16. A pass seen over this
//! point recurs one cycle later. That is a projection, not a plan: a satellite
//! can skip a pass (Sentinel-1 does), so every line says how many passes
//! support it and how many it missed.

use crate::{ApiError, ErrorBody, ErrorCode};
use axum::{extract::Path, http::StatusCode, Json};
use serde_json::{json, Value as JsonValue};

/// How far back the catalogue is read: four Sentinel-2 cycles, three of
/// Landsat. Enough to see a cycle twice.
const WINDOW_DAYS: i64 = 48;
/// Two passes of one track lie a whole number of cycles apart to within the
/// minutes a scene's start time varies. An hour is a wide margin.
const SLACK_S: i64 = 3600;
/// Answers are kept this long per cell: the catalogue gains a pass every few
/// days, and a public page must not cost three catalogue searches per view.
const CACHE_S: u64 = 3 * 3600;

struct Sensor {
    name: &'static str,
    collection: &'static str,
    cycle_days: i64,
}

const SENSORS: [Sensor; 3] = [
    Sensor {
        name: "Sentinel-2",
        collection: "sentinel-2-l2a",
        cycle_days: 10,
    },
    Sensor {
        name: "Sentinel-1",
        collection: "sentinel-1-grd",
        cycle_days: 12,
    },
    Sensor {
        name: "Landsat",
        collection: "landsat-c2-l2",
        cycle_days: 16,
    },
];

/// One scene of one satellite over the point.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Pass {
    pub platform: String,
    pub track: String,
    pub at: i64,
    pub id: String,
}

/// The next expected pass of one satellite on one track.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Projection {
    pub platform: String,
    pub track: String,
    pub cycle_days: i64,
    pub passes_seen: usize,
    pub passes_missed: i64,
    pub last_seen: i64,
    pub last_item: String,
    pub next_expected: i64,
}

/// The track a scene was taken on. Sentinel-1 and Landsat name it; a
/// Sentinel-2 item does not, so its overpass time, to the nearest ten minutes,
/// stands for it (each orbit crosses a place at the same time of day).
fn track_of(sensor: &str, props: &JsonValue, at: i64) -> Option<String> {
    match sensor {
        "Sentinel-1" => Some(format!(
            "relative orbit {} {}",
            props.get("sat:relative_orbit")?.as_u64()?,
            props
                .get("sat:orbit_state")
                .and_then(JsonValue::as_str)
                .unwrap_or("unknown direction")
        )),
        "Landsat" => Some(format!(
            "WRS path {} row {}",
            props.get("landsat:wrs_path")?.as_str()?,
            props.get("landsat:wrs_row")?.as_str()?
        )),
        _ => {
            let tod = (at.rem_euclid(86_400) + 300) / 600 * 600 % 86_400;
            Some(format!("{:02}:{:02} UTC", tod / 3600, tod % 3600 / 60))
        }
    }
}

/// Each satellite's next pass on each track, soonest first. Two scenes of one
/// track within the slack are one pass (a pass can be cut into several items).
pub(crate) fn project(passes: &[Pass], cycle_days: i64, now: i64) -> Vec<Projection> {
    let cycle = cycle_days * 86_400;
    let mut groups: std::collections::BTreeMap<(String, String), Vec<(i64, String)>> =
        std::collections::BTreeMap::new();
    for p in passes {
        groups
            .entry((p.platform.clone(), p.track.clone()))
            .or_default()
            .push((p.at, p.id.clone()));
    }
    let mut out = Vec::new();
    for ((platform, track), mut seen) in groups {
        seen.sort();
        let mut times: Vec<(i64, String)> = Vec::new();
        for (t, id) in seen {
            match times.last() {
                Some((last, _)) if t - last <= SLACK_S => {}
                _ => times.push((t, id)),
            }
        }
        let Some((last, last_item)) = times.last().cloned() else {
            continue;
        };
        // The gaps between passes in whole cycles. A gap of two cycles is one
        // pass the satellite did not take, or the catalogue does not hold.
        let missed: i64 = times
            .windows(2)
            .map(|w| ((w[1].0 - w[0].0 + cycle / 2) / cycle - 1).max(0))
            .sum();
        let k = (now - last).div_euclid(cycle) + 1;
        out.push(Projection {
            platform,
            track,
            cycle_days,
            passes_seen: times.len(),
            passes_missed: missed,
            last_seen: last,
            last_item,
            next_expected: last + k * cycle,
        });
    }
    out.sort_by_key(|p| p.next_expected);
    out
}

fn iso(t: i64) -> String {
    let (y, m, d) = crate::civil_from_days(t.div_euclid(86_400));
    let s = t.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        s / 3600,
        s % 3600 / 60,
        s % 60
    )
}

async fn passes_for(
    cli: &reqwest::Client,
    sensor: &Sensor,
    lat: f64,
    lng: f64,
    from: i64,
    to: i64,
) -> Result<Vec<Pass>, String> {
    let body = json!({
        "collections": [sensor.collection],
        "intersects": {"type": "Point", "coordinates": [lng, lat]},
        "datetime": format!("{}/{}", iso(from), iso(to)),
        "limit": 100,
        "fields": {"include": [
            "id", "properties.datetime", "properties.platform",
            "properties.sat:relative_orbit", "properties.sat:orbit_state",
            "properties.landsat:wrs_path", "properties.landsat:wrs_row",
        ]},
    });
    let resp = cli
        .post(emem_fetch::stac::STAC_ELEMENT84_V1)
        .json(&body)
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| format!("catalogue search failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("catalogue answered HTTP {}", resp.status()));
    }
    let v: JsonValue = resp
        .json()
        .await
        .map_err(|e| format!("catalogue answer unreadable: {e}"))?;
    let mut out = Vec::new();
    for f in v
        .get("features")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
    {
        let props = &f["properties"];
        let (Some(at), Some(platform)) = (
            props
                .get("datetime")
                .and_then(JsonValue::as_str)
                .and_then(crate::rfc3339_to_unix_secs),
            props.get("platform").and_then(JsonValue::as_str),
        ) else {
            continue;
        };
        let Some(track) = track_of(sensor.name, props, at) else {
            continue;
        };
        out.push(Pass {
            platform: platform.to_string(),
            track,
            at,
            id: f["id"].as_str().unwrap_or_default().to_string(),
        });
    }
    Ok(out)
}

type Cache = std::sync::Mutex<std::collections::HashMap<String, (std::time::Instant, JsonValue)>>;

/// `GET /v1/cells/:cell64/next_acquisition`.
pub(crate) async fn get_next_acquisition(
    Path(cell64): Path<String>,
) -> Result<Json<JsonValue>, ApiError> {
    let centre = emem_codec::latlng_from_cell64(&cell64).map_err(|e| {
        ApiError(
            StatusCode::BAD_REQUEST,
            ErrorBody {
                code: ErrorCode::InvalidCell,
                message: format!("{e}"),
                details: None,
            },
        )
    })?;
    static CACHE: std::sync::OnceLock<Cache> = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some((_, v)) = cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&cell64)
        .filter(|(at, _)| at.elapsed().as_secs() < CACHE_S)
    {
        return Ok(Json(v.clone()));
    }
    let now = crate::now_unix_s();
    let from = now - WINDOW_DAYS * 86_400;
    let cli = crate::reqwest_client();
    let (lat, lng) = (centre.lat_deg, centre.lng_deg);
    let (s2, s1, ls) = tokio::join!(
        passes_for(&cli, &SENSORS[0], lat, lng, from, now),
        passes_for(&cli, &SENSORS[1], lat, lng, from, now),
        passes_for(&cli, &SENSORS[2], lat, lng, from, now),
    );
    let mut lines = Vec::new();
    let mut unavailable = Vec::new();
    for (sensor, got) in SENSORS.iter().zip([s2, s1, ls]) {
        match got {
            Ok(passes) => {
                for p in project(&passes, sensor.cycle_days, now) {
                    lines.push(json!({
                        "sensor": sensor.name,
                        "platform": p.platform,
                        "track": p.track,
                        "next_expected": iso(p.next_expected),
                        "cycle_days": p.cycle_days,
                        "last_seen": iso(p.last_seen),
                        "last_item": p.last_item,
                        "passes_seen": p.passes_seen,
                        "passes_missed": p.passes_missed,
                    }));
                }
            }
            Err(e) => unavailable.push(json!({"sensor": sensor.name, "reason": e})),
        }
    }
    lines.sort_by(|a, b| {
        a["next_expected"]
            .as_str()
            .cmp(&b["next_expected"].as_str())
    });
    let complete = unavailable.is_empty();
    let body = json!({
        "cell64": cell64,
        "centre": {"lat_deg": lat, "lng_deg": lng},
        "as_of": iso(now),
        "kind": "projection",
        "next": lines.first().cloned(),
        "passes": lines,
        "unavailable": unavailable,
        "basis": format!(
            "Each satellite repeats its ground track exactly (Sentinel-2 every 10 days, Sentinel-1 every 12, Landsat every 16). \
             The passes over this point in the last {WINDOW_DAYS} days, from the Earth Search catalogue, are projected one cycle on."
        ),
        "caveats": [
            "A projection, not the operators' acquisition plan: a satellite can skip a pass. passes_missed counts the cycles in the window with no scene on that track.",
            "An optical pass is a picture, not a clear one: cloud is not forecast here.",
            "Not a signed fact. The scene ids it rests on are listed, so the projection can be redone from the catalogue.",
        ],
    });
    if complete {
        let mut c = cache.lock().unwrap_or_else(|e| e.into_inner());
        if c.len() > 4096 {
            c.clear();
        }
        c.insert(cell64, (std::time::Instant::now(), body.clone()));
    }
    Ok(Json(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pass(platform: &str, track: &str, at: i64) -> Pass {
        Pass {
            platform: platform.into(),
            track: track.into(),
            at,
            id: format!("{platform}-{at}"),
        }
    }

    fn t(s: &str) -> i64 {
        crate::rfc3339_to_unix_secs(s).unwrap()
    }

    /// Dhaulana on 2026-11-06, as the catalogue held it.
    #[test]
    fn each_satellite_recurs_one_cycle_after_its_last_pass() {
        let s2 = [
            pass("sentinel-2b", "05:40 UTC", t("2026-10-15T05:41:02Z")),
            pass("sentinel-2b", "05:40 UTC", t("2026-10-25T05:41:03Z")),
            pass("sentinel-2b", "05:40 UTC", t("2026-11-04T05:41:04Z")),
            pass("sentinel-2c", "05:40 UTC", t("2026-10-20T05:41:03Z")),
            pass("sentinel-2c", "05:40 UTC", t("2026-10-30T05:41:00Z")),
        ];
        let p = project(&s2, 10, t("2026-11-06T07:00:00Z"));
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].platform, "sentinel-2c", "the soonest comes first");
        assert_eq!(iso(p[0].next_expected), "2026-11-09T05:41:00Z");
        assert_eq!(iso(p[1].next_expected), "2026-11-14T05:41:04Z");
        assert_eq!(p[1].passes_seen, 3);
        assert_eq!(p[1].passes_missed, 0);
    }

    #[test]
    fn a_skipped_pass_is_counted_and_the_cycle_still_holds() {
        // Sentinel-1D, relative orbit 63: 10-02 and 10-14, nothing on 10-26.
        let s1 = [
            pass(
                "sentinel-1d",
                "relative orbit 63 descending",
                t("2026-10-02T00:43:42Z"),
            ),
            pass(
                "sentinel-1d",
                "relative orbit 63 descending",
                t("2026-10-14T00:43:42Z"),
            ),
            pass(
                "sentinel-1d",
                "relative orbit 136 descending",
                t("2026-09-25T00:51:48Z"),
            ),
            pass(
                "sentinel-1d",
                "relative orbit 136 descending",
                t("2026-10-19T00:51:48Z"),
            ),
        ];
        let p = project(&s1, 12, t("2026-11-06T07:00:00Z"));
        let o63 = p.iter().find(|x| x.track.contains("63")).unwrap();
        assert_eq!(
            iso(o63.next_expected),
            "2026-11-07T00:43:42Z",
            "10-26 passed, 11-07 next"
        );
        let o136 = p.iter().find(|x| x.track.contains("136")).unwrap();
        assert_eq!(
            o136.passes_missed, 1,
            "10-07 is missing between 09-25 and 10-19"
        );
        assert_eq!(iso(o136.next_expected), "2026-11-12T00:51:48Z");
    }

    #[test]
    fn one_pass_cut_into_several_items_is_one_pass() {
        let s2 = [
            pass("sentinel-2a", "05:40 UTC", t("2026-10-22T05:41:12Z")),
            pass("sentinel-2a", "05:40 UTC", t("2026-10-22T05:41:20Z")),
        ];
        let p = project(&s2, 10, t("2026-11-06T07:00:00Z"));
        assert_eq!(p[0].passes_seen, 1);
        assert_eq!(iso(p[0].next_expected), "2026-11-11T05:41:12Z");
    }

    #[test]
    fn a_sentinel_2_track_is_its_overpass_time() {
        let at = t("2026-11-04T05:39:59Z");
        assert_eq!(track_of("Sentinel-2", &json!({}), at).unwrap(), "05:40 UTC");
        let late = t("2026-11-04T23:58:00Z");
        assert_eq!(
            track_of("Sentinel-2", &json!({}), late).unwrap(),
            "00:00 UTC"
        );
        let ls = json!({"landsat:wrs_path": "146", "landsat:wrs_row": "040"});
        assert_eq!(
            track_of("Landsat", &ls, at).unwrap(),
            "WRS path 146 row 040"
        );
        assert!(
            track_of("Sentinel-1", &json!({}), at).is_none(),
            "no orbit, no track"
        );
    }
}
