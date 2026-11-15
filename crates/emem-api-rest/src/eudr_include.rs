//! The optional sections of `/v1/eudr_dds`: `include: ["season", "imagery", "burn"]`.
//!
//! A platform that files statements wanted the verdict and the evidence a
//! reviewer looks at in one call: the vegetation season of each plot, a true
//! colour picture of it for each year since 2020, and the burned months. Each
//! section starts when the request does, runs beside the verdict on its own
//! task, and has its own time budget. The answer waits for a section no
//! longer than its budget; a section that has not finished comes back as
//! `pending` with a token, and `GET /v1/eudr_dds/include/{token}` returns it
//! once it has. A slow upstream never delays the verdict past the budgets,
//! and an unfinished section is never an error.
//!
//! Each section reuses the engine that already serves it: `season` is the
//! `fields` engine of `/v1/field_series` (NBR with the registered canopy-loss
//! rule, and NDVI), `imagery` is the scene pipeline of `/v1/scene.png`, and
//! `burn` is the MODIS burned-area materializer that `/v1/field_burn_scar`
//! warms. The sections name each plot by its index and its commitment,
//! never by the caller's plot id.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use axum::extract::Path as AxumPath;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::{json, Value as JsonValue};

use crate::band_raster::BBox;
use crate::AppState;

/// The sections a request may name.
pub(crate) const SECTIONS: [&str; 3] = ["season", "imagery", "burn"];

/// How long the answer waits for each section, at most.
fn budget(section: &str) -> Duration {
    Duration::from_secs(match section {
        "season" => 25,
        "imagery" => 20,
        _ => 15,
    })
}

/// How long a finished section stays fetchable by its token.
const KEEP: Duration = Duration::from_secs(6 * 3600);
/// Sections kept at once; the oldest goes first past it.
const MAX_KEPT: usize = 4096;

/// Plots grouped for one `fields` read stay within this many degrees a side:
/// the `fields` engine reads one window of at most 512 pixels at 10 m.
const SEASON_GROUP_DEG: f64 = 0.04;
/// Scenes each season series reads.
const SEASON_SCENES: usize = 96;
/// The first day the canopy-loss rule judges: the day after the cut-off.
const CANOPY_FROM: &str = "2021-01-01";
/// The first year a picture is made for.
const IMAGERY_FROM_YEAR: i32 = 2020;
/// Scene cloud ceiling for a year's picture; the clearest under it is used.
const IMAGERY_MAX_CLOUD: f64 = 40.0;
/// Pictures made at once.
const IMAGERY_AT_ONCE: usize = 6;
/// MODIS burned area is 500 m: about one cell a 25 ha of plot, at most this.
const BURN_MAX_CELLS: usize = 16;
/// The part of the `burn` budget left to read the months once warmed.
const BURN_READ_TIME: Duration = Duration::from_secs(3);

/// One plot as the sections read it.
#[derive(Debug, Clone)]
pub(crate) struct IncludePlot {
    pub(crate) idx: usize,
    /// The plot's commitment: the only name the sections give it.
    pub(crate) commitment: String,
    pub(crate) bbox: BBox,
    /// The plot as a GeoJSON Polygon (a `bbox` or a Point as its box).
    pub(crate) geometry: JsonValue,
    pub(crate) area_ha: f64,
}

/// Sections by token: when each started, its name, and its result once done.
type Kept = HashMap<String, (Instant, &'static str, Option<JsonValue>)>;

static RESULTS: LazyLock<Mutex<Kept>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// Sections that run at once on this responder. A section outlives its
/// request, so the route's concurrency cap does not bound it; this does.
/// One that waits for its turn reads `pending`.
const SECTIONS_AT_ONCE: usize = 6;
static SECTION_SLOTS: LazyLock<tokio::sync::Semaphore> =
    LazyLock::new(|| tokio::sync::Semaphore::new(SECTIONS_AT_ONCE));

fn results() -> std::sync::MutexGuard<'static, Kept> {
    RESULTS.lock().unwrap_or_else(|e| e.into_inner())
}

fn new_token() -> String {
    use rand::RngCore;
    let mut b = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut b);
    data_encoding::BASE32_NOPAD.encode(&b).to_lowercase()
}

/// A section started for a request.
pub(crate) struct Running {
    section: &'static str,
    token: String,
    handle: tokio::task::JoinHandle<JsonValue>,
    deadline: tokio::time::Instant,
}

/// Starts `section` over `plots` on its own task. Its result is kept under
/// the token whether or not the answer waits for it.
pub(crate) fn start(
    s: &AppState,
    section: &'static str,
    plots: Vec<IncludePlot>,
    wait: Option<Duration>,
) -> Running {
    let token = new_token();
    {
        let mut r = results();
        let now = Instant::now();
        r.retain(|_, (at, _, _)| now.duration_since(*at) < KEEP);
        while r.len() >= MAX_KEPT {
            let Some(oldest) = r.iter().min_by_key(|(_, v)| v.0).map(|(k, _)| k.clone()) else {
                break;
            };
            r.remove(&oldest);
        }
        r.insert(token.clone(), (now, section, None));
    }
    let s = s.clone();
    let kept = token.clone();
    let handle = tokio::spawn(async move {
        let _slot = SECTION_SLOTS.acquire().await;
        let out = match section {
            "season" => season(s, plots).await,
            "imagery" => imagery(s, plots).await,
            _ => burn(s, plots).await,
        };
        if let Some(slot) = results().get_mut(&kept) {
            slot.2 = Some(out.clone());
        }
        out
    });
    let wait = wait.map_or(budget(section), |w| w.min(budget(section)));
    Running {
        section,
        token,
        handle,
        deadline: tokio::time::Instant::now() + wait,
    }
}

fn pending(section: &str, token: &str) -> JsonValue {
    json!({
        "status": "pending",
        "section": section,
        "token": token,
        "fetch": format!("/v1/eudr_dds/include/{token}"),
        "retry_after_s": 15,
        "note": "The section is still running and keeps running after this answer. Fetch it by its token; it stays there for six hours after it finishes.",
    })
}

/// The section, if it finishes by its deadline, else `pending`.
pub(crate) async fn collect(r: Running) -> JsonValue {
    let Running {
        section,
        token,
        handle,
        deadline,
    } = r;
    match tokio::time::timeout_at(deadline, handle).await {
        Ok(Ok(mut v)) => {
            if let Some(o) = v.as_object_mut() {
                o.insert("status".into(), json!("done"));
                o.insert("token".into(), json!(token));
            }
            v
        }
        Ok(Err(e)) => json!({
            "status": "unavailable",
            "section": section,
            "why": format!("the section stopped: {e}"),
        }),
        Err(_) => pending(section, &token),
    }
}

/// `GET /v1/eudr_dds/include/{token}`: a section an answer left pending.
pub(crate) async fn get_include(AxumPath(token): AxumPath<String>) -> impl IntoResponse {
    let found = results()
        .get(&token)
        .map(|(_, section, v)| (*section, v.clone()));
    match found {
        Some((_, Some(mut v))) => {
            if let Some(o) = v.as_object_mut() {
                o.insert("status".into(), json!("done"));
                o.insert("token".into(), json!(token));
            }
            (StatusCode::OK, Json(v))
        }
        Some((section, None)) => (StatusCode::OK, Json(pending(section, &token))),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "code": "include_unknown_or_expired",
                "message": "no section under this token on this responder: it was never issued here, it expired six hours after it finished, or the responder restarted. Send the /v1/eudr_dds request again with the same include.",
            })),
        ),
    }
}

fn today() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    day(now / 86_400)
}

fn day(days: i64) -> String {
    let (y, m, d) = crate::civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

fn day_number(ymd: &str) -> i64 {
    crate::band_raster::parse_ymd(ymd)
        .map(|(y, m, d)| crate::days_from_civil(y, m, d))
        .unwrap_or(0)
}

/// The day number of a burn: its day of year in the year of the month's fact.
fn burn_day(unix: i64, doy: i64) -> i64 {
    let year = &day(unix.div_euclid(86_400))[..4];
    day_number(&format!("{year}-01-01")) + doy - 1
}

fn union(a: &BBox, b: &BBox) -> BBox {
    BBox {
        min_lat: a.min_lat.min(b.min_lat),
        max_lat: a.max_lat.max(b.max_lat),
        min_lng: a.min_lng.min(b.min_lng),
        max_lng: a.max_lng.max(b.max_lng),
    }
}

/// Plots grouped west to east while each group's box stays `side` degrees.
fn groups(plots: Vec<IncludePlot>, side: f64) -> Vec<Vec<IncludePlot>> {
    let mut plots = plots;
    plots.sort_by(|a, b| a.bbox.min_lng.total_cmp(&b.bbox.min_lng));
    let mut out: Vec<(BBox, Vec<IncludePlot>)> = Vec::new();
    for p in plots {
        let fits = |g: &(BBox, Vec<IncludePlot>)| {
            let u = union(&g.0, &p.bbox);
            u.max_lng - u.min_lng <= side && u.max_lat - u.min_lat <= side
        };
        match out.iter_mut().find(|g| fits(g)) {
            Some(g) => {
                g.0 = union(&g.0, &p.bbox);
                g.1.push(p);
            }
            None => out.push((p.bbox.clone(), vec![p])),
        }
    }
    out.into_iter().map(|(_, g)| g).collect()
}

/// The windows the season reads: the canopy-loss rule compares each date with
/// the same weeks a year earlier, and one series spans at most four years, so
/// loss from 2021 to today is judged over two windows.
fn season_windows(today: &str) -> Vec<(String, String, String, String)> {
    let history = crate::canopy_loss::HISTORY_DAYS;
    let max_days = 4 * 366 - 1;
    let from = day_number(CANOPY_FROM);
    let end = day_number(today);
    let mut out = Vec::new();
    let mut lo = from;
    while lo <= end {
        let start = lo - history;
        let stop = (start + max_days).min(end);
        out.push((day(start), day(stop), day(lo), day(stop)));
        lo = stop + 1;
    }
    out
}

async fn field_series_call(s: &AppState, body: JsonValue) -> Result<JsonValue, String> {
    let req: crate::field_series::FieldSeriesReq =
        serde_json::from_value(body).map_err(|e| format!("request: {e}"))?;
    crate::field_series::field_series(req, s)
        .await
        .map_err(|e| e.1.message)
}

/// `season`: per plot, Sentinel-2 NBR with the registered canopy-loss rule
/// from 2021-01-01 to today, and the NDVI series of the last window, from
/// the `fields` engine of `/v1/field_series`.
async fn season(s: AppState, plots: Vec<IncludePlot>) -> JsonValue {
    let started = Instant::now();
    let windows = season_windows(&today());
    let groups = groups(plots, SEASON_GROUP_DEG);
    let mut per_plot: Vec<JsonValue> = Vec::new();
    for group in groups {
        let fields: Vec<JsonValue> = group
            .iter()
            .map(|p| json!({"id": p.commitment, "geometry": p.geometry}))
            .collect();
        let mut calls = Vec::new();
        for (start, end, lo, hi) in &windows {
            calls.push((
                "nbr",
                json!({
                    "fields": fields, "index": "nbr", "start_date": start, "end_date": end,
                    "max_scenes": SEASON_SCENES, "canopy_loss": {"from": lo, "to": hi},
                }),
            ));
        }
        if let Some((start, end, _, _)) = windows.last() {
            calls.push((
                "ndvi",
                json!({"fields": fields, "index": "ndvi", "start_date": start, "end_date": end,
                       "max_scenes": SEASON_SCENES}),
            ));
        }
        let got = futures_util::future::join_all(calls.into_iter().map(|(index, body)| {
            let s = s.clone();
            async move {
                let got = field_series_call(&s, body.clone()).await;
                (index, body, got)
            }
        }))
        .await;
        for p in &group {
            let mut nbr = Vec::new();
            let mut ndvi = JsonValue::Null;
            for (index, body, res) in &got {
                let entry = match res {
                    Ok(out) => {
                        let summary = out["fields"]["summary"]
                            .as_array()
                            .and_then(|a| a.iter().find(|f| f["id"] == json!(p.commitment)))
                            .cloned()
                            .unwrap_or(JsonValue::Null);
                        json!({
                            "window": [body["start_date"], body["end_date"]],
                            "field": summary,
                            "series_token": out["token"],
                        })
                    }
                    Err(e) => json!({
                        "window": [body["start_date"], body["end_date"]],
                        "error": e,
                    }),
                };
                if *index == "nbr" {
                    nbr.push(entry);
                } else {
                    ndvi = entry;
                }
            }
            per_plot.push(json!({
                "plot": p.idx,
                "plot_commitment": p.commitment,
                "nbr_canopy_loss": nbr,
                "ndvi": ndvi,
            }));
        }
    }
    per_plot.sort_by_key(|p| p["plot"].as_u64());
    json!({
        "section": "season",
        "method": "the fields engine of /v1/field_series: each plot's median Sentinel-2 NBR per clear scene, judged by the registered canopy-loss rule from 2021-01-01 in windows of at most four years, and its NDVI over the last window",
        "canopy_loss_rule": crate::canopy_loss::KEY,
        "plots": per_plot,
        "seconds": (started.elapsed().as_secs_f64() * 100.0).round() / 100.0,
    })
}

/// The clearest Sentinel-2 scene over the point in the window: the lowest
/// scene cloud cover, the newest of equals, with the catalogue that holds it.
/// Its asset urls are as the catalogue gave them, unsigned.
async fn clearest_scene(lat: f64, lng: f64, window: &str) -> Result<Option<Scene>, String> {
    let cli = crate::s2_http_client();
    let mut failures = Vec::new();
    for host in crate::s2_catalogues() {
        match emem_fetch::stac::search_many_at(
            &cli,
            host,
            "sentinel-2-l2a",
            lng,
            lat,
            window,
            Some(IMAGERY_MAX_CLOUD),
            250,
        )
        .await
        {
            Ok(items) => {
                let best = items.into_iter().min_by(|a, b| {
                    a.cloud_cover
                        .unwrap_or(100.0)
                        .total_cmp(&b.cloud_cover.unwrap_or(100.0))
                        .then_with(|| b.datetime.cmp(&a.datetime))
                });
                return Ok(best.map(|item| Scene { host, item }));
            }
            Err(e) => failures.push(format!("{host}: {e}")),
        }
    }
    Err(failures.join("; "))
}

/// A chosen scene and the catalogue it came from.
#[derive(Debug, Clone)]
struct Scene {
    host: &'static str,
    item: emem_fetch::stac::StacItem,
}

/// The box and year a picture is chosen for: the box's corners as bits, so
/// two plots with one box share a choice and no rounding merges two boxes.
type SceneKey = ([u64; 4], i32);

fn scene_key(b: &BBox, year: i32) -> SceneKey {
    (
        [
            b.min_lat.to_bits(),
            b.max_lat.to_bits(),
            b.min_lng.to_bits(),
            b.max_lng.to_bits(),
        ],
        year,
    )
}

/// How long the current year's choice holds: a clearer pass can arrive any
/// day. A past year's choice holds for good, as no pass is added to it.
const CURRENT_YEAR_KEEP: Duration = Duration::from_secs(24 * 3600);
/// Choices kept at once; the oldest goes first past it.
const MAX_SCENES: usize = 16_384;

/// Whether a choice made `age` ago for `year` still holds in `now_year`.
fn scene_holds(age: Duration, year: i32, now_year: i32) -> bool {
    year < now_year || age < CURRENT_YEAR_KEEP
}

/// The scene chosen for each plot box and year, or that there is none, so a
/// warm `imagery` call reads no catalogue. A catalogue error is not kept.
type ChosenScenes = HashMap<SceneKey, (Instant, Option<Scene>)>;

static SCENES: LazyLock<Mutex<ChosenScenes>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// The scene for `key`, from [`SCENES`] while it holds, else from `search`,
/// which is kept.
async fn chosen_scene<F, Fut>(
    key: SceneKey,
    now_year: i32,
    search: F,
) -> Result<Option<Scene>, String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<Option<Scene>, String>>,
{
    {
        let kept = SCENES.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, scene)) = kept.get(&key) {
            if scene_holds(at.elapsed(), key.1, now_year) {
                return Ok(scene.clone());
            }
        }
    }
    let found = search().await?;
    let mut kept = SCENES.lock().unwrap_or_else(|e| e.into_inner());
    if kept.len() >= MAX_SCENES && !kept.contains_key(&key) {
        if let Some(oldest) = kept.iter().min_by_key(|(_, v)| v.0).map(|(k, _)| *k) {
            kept.remove(&oldest);
        }
    }
    kept.insert(key, (Instant::now(), found.clone()));
    Ok(found)
}

/// One picture: the clearest scene of `year` over the plot, cropped to the
/// plot's box at 10 m, stored as an artifact.
async fn picture(p: &IncludePlot, year: i32, now_year: i32) -> JsonValue {
    // The cap `request_visual_evidence` reads under, which this replaces.
    let _permit = crate::visual_materialize_permit().await;
    let lat = (p.bbox.min_lat + p.bbox.max_lat) / 2.0;
    let lng = (p.bbox.min_lng + p.bbox.max_lng) / 2.0;
    let end = if year >= now_year {
        format!("{}T23:59:59Z", today())
    } else {
        format!("{year}-12-31T23:59:59Z")
    };
    let window = format!("{year}-01-01T00:00:00Z/{end}");
    let chosen = chosen_scene(scene_key(&p.bbox, year), now_year, || {
        clearest_scene(lat, lng, &window)
    })
    .await;
    let item = match chosen {
        Ok(Some(Scene { host, item })) if host == emem_fetch::stac::STAC_MPC_V1 => {
            match crate::s2_sign_mpc_items(&crate::s2_http_client(), vec![item]).await {
                Ok(mut v) => match v.pop() {
                    Some(it) => it,
                    None => {
                        return json!({"year": year, "error": "catalogue: signing returned no item"})
                    }
                },
                Err(e) => return json!({"year": year, "error": format!("catalogue: {e}")}),
            }
        }
        Ok(Some(Scene { item, .. })) => item,
        Ok(None) => {
            return json!({"year": year, "error": format!("no Sentinel-2 scene under {IMAGERY_MAX_CLOUD} % cloud in {year}")})
        }
        Err(e) => return json!({"year": year, "error": format!("catalogue: {e}")}),
    };
    let mut qs = HashMap::new();
    qs.insert(
        "bbox".to_string(),
        format!(
            "{},{},{},{}",
            p.bbox.min_lng, p.bbox.min_lat, p.bbox.max_lng, p.bbox.max_lat
        ),
    );
    let sb = match crate::scene_bbox(&qs) {
        Ok(Some(b)) => b,
        Ok(None) => return json!({"year": year, "error": "no box"}),
        Err(e) => return json!({"year": year, "error": e}),
    };
    // The chosen item itself. Planetary Computer drops the fraction of a
    // cloud threshold (`lt 2.9` keeps only scenes under 2 %), so a second
    // search for the item by its own cloud cover missed it.
    let item_id = item.id.clone();
    match crate::render_scene_rgb(&crate::s2_http_client(), item, lat, lng, sb.size).await {
        Ok(scene) => {
            let png = scene.png;
            let len = png.len();
            let cid = tokio::task::spawn_blocking(move || crate::band_raster::ARTIFACTS.put(&png))
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r.map_err(|e| e.to_string()));
            match cid {
                Ok(cid) => json!({
                    "year": year,
                    "url": format!("/v1/artifacts/{cid}"),
                    "artifact_cid": cid,
                    "media_type": "image/png",
                    "byte_len": len,
                    "width": scene.w,
                    "height": scene.h,
                    "scene_id": scene.item_id,
                    "datetime": scene.item_datetime,
                    "cloud_cover": scene.cloud_cover,
                    "epsg": scene.epsg,
                    "bbox_crs": [scene.bbox_crs.0, scene.bbox_crs.1, scene.bbox_crs.2, scene.bbox_crs.3],
                    "pixel_size_m": [scene.pixel_size.0, scene.pixel_size.1],
                    "stretch_p2_p98": [
                        [scene.stretch_p2_p98.0 .0, scene.stretch_p2_p98.0 .1],
                        [scene.stretch_p2_p98.1 .0, scene.stretch_p2_p98.1 .1],
                        [scene.stretch_p2_p98.2 .0, scene.stretch_p2_p98.2 .1],
                    ],
                }),
                Err(e) => json!({"year": year, "error": format!("artifact store: {e}")}),
            }
        }
        Err(e) => json!({"year": year, "scene_id": item_id, "error": e}),
    }
}

/// `imagery`: per plot, one true-colour Sentinel-2 picture for each year
/// since 2020, from the clearest pass of that year, at its native 10 m.
async fn imagery(_s: AppState, plots: Vec<IncludePlot>) -> JsonValue {
    use futures_util::StreamExt;
    let started = Instant::now();
    let now_year: i32 = today()[..4].parse().unwrap_or(IMAGERY_FROM_YEAR);
    // Each job owns its plot: a stream of futures that borrowed from here
    // would not be provably `Send` on its task.
    let jobs: Vec<(usize, i32, IncludePlot)> = plots
        .iter()
        .enumerate()
        .flat_map(|(i, p)| (IMAGERY_FROM_YEAR..=now_year).map(move |y| (i, y, p.clone())))
        .collect();
    let mut done: Vec<(usize, JsonValue)> = futures_util::stream::iter(
        jobs.into_iter()
            .map(move |(i, y, p)| async move { (i, picture(&p, y, now_year).await) }),
    )
    .buffer_unordered(IMAGERY_AT_ONCE)
    .collect()
    .await;
    done.sort_by_key(|(i, v)| (*i, v["year"].as_i64()));
    let per_plot: Vec<JsonValue> = plots
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let years: Vec<JsonValue> = done
                .iter()
                .filter(|(j, _)| *j == i)
                .map(|(_, v)| v.clone())
                .collect();
            json!({"plot": p.idx, "plot_commitment": p.commitment, "years": years})
        })
        .collect();
    json!({
        "section": "imagery",
        "method": "for each year since 2020, the Sentinel-2 L2A scene over the plot centre with the lowest scene cloud cover (under 40 %, the newest of equals), read at its native 10 m, cropped to the plot's box, with a per-scene 2 to 98 percentile stretch, as PNG. Brightness is not comparable between years. The scene chosen for a plot box and a past year is kept, and this year's for 24 hours, so a repeat call reads no catalogue. Each picture is stored as an artifact at its url",
        "plots": per_plot,
        "seconds": (started.elapsed().as_secs_f64() * 100.0).round() / 100.0,
    })
}

/// `burn`: per plot, the MODIS MCD64A1 burned-area months since 2021, read
/// at about one cell for each 25 ha of the plot (500 m pixels).
async fn burn(s: AppState, plots: Vec<IncludePlot>) -> JsonValue {
    let started = Instant::now();
    let cutoff = day_number(CANOPY_FROM);
    // A month before the first day, in case a month's fact is dated before
    // the month it covers; a burn before the first day is not counted.
    let from = (cutoff - 31) * 86_400;
    let to = (day_number(&today()) + 1) * 86_400;
    let per_plot = futures_util::future::join_all(plots.into_iter().map(|p| {
        let s = s.clone();
        async move {
            let n = ((p.area_ha / 25.0).ceil() as usize).clamp(1, BURN_MAX_CELLS);
            let cells: Vec<String> = match crate::extract_plot_geometry(&p.geometry) {
                Ok((bbox, Some(poly), _)) => crate::sample_cells_in_polygon(bbox, n, &[poly]),
                Ok((bbox, None, _)) => crate::sample_cells_in_bbox(bbox, n),
                Err(_) => Vec::new(),
            };
            // The warm-up stops short of the budget, so the months it read
            // make the answer; it carries on after, for a later request.
            let warm_ms = budget("burn").saturating_sub(BURN_READ_TIME).as_millis() as u64;
            let converged = crate::field_signals::warm_band(
                &s,
                &cells,
                "modis.burned_area_monthly",
                (from, to),
                warm_ms,
            )
            .await;
            let mut months: std::collections::BTreeMap<String, (usize, Vec<i64>, Vec<String>)> =
                Default::default();
            let mut read = 0usize;
            for cell in &cells {
                let band = "modis.burned_area_monthly";
                let obs = crate::field_signals::band_series(&s, cell, band, (from, to)).await;
                read += usize::from(!obs.is_empty());
                for (unix, doy, cid) in obs {
                    let month = day(unix / 86_400)[..7].to_string();
                    let e = months.entry(month).or_default();
                    if doy > 0.0 && burn_day(unix, doy as i64) >= cutoff {
                        e.0 += 1;
                        e.1.push(doy as i64);
                    }
                    e.2.push(cid);
                }
            }
            let burned: Vec<JsonValue> = months
                .iter()
                .filter(|(_, v)| v.0 > 0)
                .map(|(m, v)| {
                    json!({"month": m, "burned_cells": v.0, "burn_day_of_year": v.1,
                           "fact_cids": v.2})
                })
                .collect();
            json!({
                "plot": p.idx,
                "plot_commitment": p.commitment,
                "cells": cells.len(),
                "cells_read": read,
                "months_read": months.len(),
                "burned_months": burned,
                // Nothing read is not "no burn".
                "any_burn_since_2021": (!months.is_empty()).then_some(!burned.is_empty()),
                "complete": converged,
            })
        }
    }))
    .await;
    json!({
        "section": "burn",
        "method": "MODIS MCD64A1 burned area (500 m, monthly) since 2021, read at about one cell for each 25 ha of the plot, at most 16; a month counts as burned at a cell when its burn day of year is above 0 and the day is on or after 2021-01-01. any_burn_since_2021 is null when no month was read. complete is false when the months were still being read when the section's budget ended",
        "plots": per_plot,
        "seconds": (started.elapsed().as_secs_f64() * 100.0).round() / 100.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_season_covers_2021_to_today_in_windows_of_four_years() {
        let w = season_windows("2026-11-14");
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].2, "2021-01-01");
        assert_eq!(w[1].3, "2026-11-14");
        for (start, end, lo, hi) in &w {
            let span = day_number(end) - day_number(start) + 1;
            assert!(span <= 4 * 366, "{start}..{end}");
            assert_eq!(
                day_number(lo) - day_number(start),
                crate::canopy_loss::HISTORY_DAYS
            );
            assert!(day_number(lo) <= day_number(hi));
        }
        assert_eq!(day_number(&w[1].2), day_number(&w[0].3) + 1, "no gap");
    }

    #[test]
    fn a_burn_is_dated_by_its_day_of_year() {
        let jan_2021 = day_number("2021-01-01") * 86_400;
        assert_eq!(day(burn_day(jan_2021, 1)), "2021-01-01");
        assert_eq!(day(burn_day(jan_2021, 32)), "2021-02-01");
        // A December 2020 month burned on day 350 is before the first day.
        let dec_2020 = day_number("2020-12-01") * 86_400;
        assert!(burn_day(dec_2020, 350) < day_number(CANOPY_FROM));
    }

    fn scene(id: &str) -> Scene {
        Scene {
            host: emem_fetch::stac::STAC_ELEMENT84_V1,
            item: serde_json::from_value(json!({
                "id": id,
                "cloud_cover": 1.0,
                "datetime": "2023-06-01T10:00:00Z",
                "epsg": 32630,
                "assets": {},
                "collection": "sentinel-2-l2a",
            }))
            .unwrap(),
        }
    }

    #[test]
    fn a_past_years_scene_holds_and_this_years_for_a_day() {
        let day = Duration::from_secs(24 * 3600);
        assert!(scene_holds(day * 400, 2023, 2026));
        assert!(scene_holds(day - Duration::from_secs(1), 2026, 2026));
        assert!(!scene_holds(day, 2026, 2026));
        assert!(!scene_holds(day * 2, 2026, 2026));
    }

    /// A warm call takes the kept choice and asks no catalogue; another
    /// year or another box asks again; an error is not kept.
    #[tokio::test]
    async fn a_warm_call_reads_no_catalogue() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let asked = AtomicUsize::new(0);
        let b = BBox {
            min_lat: -6.123_456,
            max_lat: -6.12,
            min_lng: 106.8,
            max_lng: 106.81,
        };
        let search = |id: &'static str| {
            let asked = &asked;
            move || async move {
                asked.fetch_add(1, Ordering::SeqCst);
                Ok(Some(scene(id)))
            }
        };
        let got = chosen_scene(scene_key(&b, 2023), 2026, search("a"))
            .await
            .unwrap();
        assert_eq!(got.unwrap().item.id, "a");
        let got = chosen_scene(scene_key(&b, 2023), 2026, search("b"))
            .await
            .unwrap();
        assert_eq!(got.unwrap().item.id, "a", "the kept choice");
        assert_eq!(asked.load(Ordering::SeqCst), 1);
        chosen_scene(scene_key(&b, 2024), 2026, search("c"))
            .await
            .unwrap();
        let other = BBox {
            max_lng: 106.810_001,
            ..b.clone()
        };
        chosen_scene(scene_key(&other, 2023), 2026, search("d"))
            .await
            .unwrap();
        assert_eq!(asked.load(Ordering::SeqCst), 3);

        // No scene at all is a choice too; a failed search is not.
        let none_box = BBox {
            min_lat: 50.0,
            max_lat: 50.01,
            min_lng: 3.0,
            max_lng: 3.01,
        };
        let empty = || async { Ok(None) };
        assert!(chosen_scene(scene_key(&none_box, 2021), 2026, empty)
            .await
            .unwrap()
            .is_none());
        let fail_box = BBox {
            min_lat: 51.0,
            max_lat: 51.01,
            min_lng: 3.0,
            max_lng: 3.01,
        };
        let failing = || async { Err("stac status 503".to_string()) };
        assert!(chosen_scene(scene_key(&fail_box, 2021), 2026, failing)
            .await
            .is_err());
        let got = chosen_scene(scene_key(&fail_box, 2021), 2026, search("e"))
            .await
            .unwrap();
        assert_eq!(got.unwrap().item.id, "e", "the error was not kept");
    }

    #[test]
    fn plots_group_within_the_window() {
        let p = |i: usize, lng: f64| IncludePlot {
            idx: i,
            commitment: format!("c{i}"),
            bbox: BBox {
                min_lat: 5.0,
                max_lat: 5.01,
                min_lng: lng,
                max_lng: lng + 0.01,
            },
            geometry: JsonValue::Null,
            area_ha: 1.0,
        };
        let g = groups(vec![p(0, -6.0), p(1, -5.98), p(2, -5.5)], SEASON_GROUP_DEG);
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].len(), 2);
    }
}
