//! Minimal STAC POST-search client for AWS Open Data scene discovery.
//!
//! emem materializers don't keep a long-lived index of every Sentinel scene;
//! they ask the public Element84 STAC API at request time for the latest
//! cloud-acceptable item that intersects the cell, then range-read its COG
//! assets. The STAC response carries the URLs and the per-asset CRS code,
//! which is exactly what `crate::cog` and `crate::proj` need.
//!
//! Endpoint: <https://earth-search.aws.element84.com/v1/search> — anonymous,
//! no API key, public AWS Open Data backed.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Element84 AWS Open Data STAC — anonymous, no API key. Sentinel-2 L2A,
/// Sentinel-1 GRD, Cop-DEM, Landsat, NAIP.
pub const STAC_ELEMENT84_V1: &str = "https://earth-search.aws.element84.com/v1/search";

/// Microsoft Planetary Computer STAC — anonymous search; asset URLs are
/// Azure Blob URLs that need a free anonymous SAS token (see
/// `mpc_sas_token`). Used for `sentinel-1-rtc` (the only free RTC-format
/// Sentinel-1 catalog with proper UTM-projected COG tiles).
pub const STAC_MPC_V1: &str = "https://planetarycomputer.microsoft.com/api/stac/v1/search";

/// One STAC item: scene metadata + per-band asset URLs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StacItem {
    /// Item id, e.g. `S2C_30UXC_20260425_0_L2A`.
    pub id: String,
    /// `eo:cloud_cover` percent (Sentinel-2 only).
    pub cloud_cover: Option<f64>,
    /// ISO 8601 capture time.
    pub datetime: String,
    /// EPSG of the assets in this item.
    pub epsg: Option<u32>,
    /// Per-band asset URL: `assets[name].href`.
    pub assets: std::collections::BTreeMap<String, String>,
    /// Raw collection name (`sentinel-2-l2a`, `sentinel-1-grd`, …).
    pub collection: String,
}

/// Request a single best item from the STAC API at the given (lng, lat)
/// point. `datetime` is an RFC 3339 interval like
/// `"2026-01-01T00:00:00Z/2026-04-27T00:00:00Z"`. Using `intersects: Point`
/// instead of `bbox` ensures we get a tile that *actually contains* the
/// requested coordinate — a bbox query can match neighbouring tiles that
/// only overlap the bbox, leaving the sample point outside the raster.
pub async fn search_one(
    client: &Client,
    collection: &str,
    lng: f64,
    lat: f64,
    datetime: &str,
    max_cloud: Option<f64>,
) -> Result<Option<StacItem>, String> {
    search_one_at(
        client,
        STAC_ELEMENT84_V1,
        collection,
        lng,
        lat,
        datetime,
        max_cloud,
    )
    .await
}

/// Like [`search_one`] but parameterised on the STAC host URL so callers
/// can route between Element84 (anonymous AWS Open Data) and Microsoft
/// Planetary Computer (anonymous, asset URLs need SAS — see
/// [`mpc_sas_token`]).
pub async fn search_one_at(
    client: &Client,
    search_url: &str,
    collection: &str,
    lng: f64,
    lat: f64,
    datetime: &str,
    max_cloud: Option<f64>,
) -> Result<Option<StacItem>, String> {
    let _stage =
        crate::latency::StageTimer::new("stac.search", format!("{collection} @ {lng:.3},{lat:.3}"));
    let mut body = json!({
        "intersects": {"type": "Point", "coordinates": [lng, lat]},
        "limit": 1,
        "collections": [collection],
        "datetime": datetime,
        "sortby": [{"field": "properties.datetime", "direction": "desc"}],
    });
    if let Some(c) = max_cloud {
        body["query"] = json!({"eo:cloud_cover": {"lt": c}});
    }
    let resp = client
        .post(search_url)
        .header("content-type", "application/json")
        .header(
            "user-agent",
            concat!(
                "emem.dev/",
                env!("CARGO_PKG_VERSION"),
                " (avijeet@vortx.ai)"
            ),
        )
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("stac http: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("stac status {}", resp.status()));
    }
    let v: Value = resp.json().await.map_err(|e| format!("stac json: {e}"))?;
    let feats = match v.get("features").and_then(|f| f.as_array()) {
        Some(a) => a,
        None => return Ok(None),
    };
    let f = match feats.first() {
        Some(f) => f,
        None => return Ok(None),
    };
    Ok(Some(parse_stac_feature(f, collection)))
}

/// Parse one STAC `feature` object into a [`StacItem`]. Shared by
/// [`search_one_at`] (first feature) and [`search_many_at`] (every feature).
fn parse_stac_feature(f: &Value, collection: &str) -> StacItem {
    let id = f
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let props = f.get("properties").cloned().unwrap_or(Value::Null);
    let cloud_cover = props.get("eo:cloud_cover").and_then(|v| v.as_f64());
    let datetime_str = props
        .get("datetime")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let epsg = props
        .get("proj:epsg")
        .and_then(|v| v.as_u64())
        .map(|n| n as u32);
    let mut assets = std::collections::BTreeMap::new();
    if let Some(a) = f.get("assets").and_then(|a| a.as_object()) {
        for (k, v) in a {
            if let Some(href) = v.get("href").and_then(|h| h.as_str()) {
                assets.insert(k.clone(), href.to_string());
            }
        }
    }
    StacItem {
        id,
        cloud_cover,
        datetime: datetime_str,
        epsg,
        assets,
        collection: collection.to_string(),
    }
}

/// Like [`search_one_at`] but returns up to `limit` items (newest first)
/// that intersect the point. Used by the Sentinel-2 materializer to gather
/// multiple candidate scenes so a single cloudy latest pixel does not force
/// a false Absence — the materializer can fall through to the next-newest
/// scene whose per-pixel SCL is clear. Same anonymous STAC POST; only the
/// `limit` and the return arity differ.
// The argument list mirrors the STAC search parameters one-for-one
// (host, collection, point, datetime, cloud, limit); collapsing them into a
// struct would obscure the call sites for no real gain.
#[allow(clippy::too_many_arguments)]
pub async fn search_many_at(
    client: &Client,
    search_url: &str,
    collection: &str,
    lng: f64,
    lat: f64,
    datetime: &str,
    max_cloud: Option<f64>,
    limit: usize,
) -> Result<Vec<StacItem>, String> {
    let limit = limit.clamp(1, 50);
    let mut body = json!({
        "intersects": {"type": "Point", "coordinates": [lng, lat]},
        "limit": limit,
        "collections": [collection],
        "datetime": datetime,
        "sortby": [{"field": "properties.datetime", "direction": "desc"}],
    });
    if let Some(c) = max_cloud {
        body["query"] = json!({"eo:cloud_cover": {"lt": c}});
    }
    let resp = client
        .post(search_url)
        .header("content-type", "application/json")
        .header(
            "user-agent",
            concat!(
                "emem.dev/",
                env!("CARGO_PKG_VERSION"),
                " (avijeet@vortx.ai)"
            ),
        )
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("stac http: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("stac status {}", resp.status()));
    }
    let v: Value = resp.json().await.map_err(|e| format!("stac json: {e}"))?;
    let feats = match v.get("features").and_then(|f| f.as_array()) {
        Some(a) => a,
        None => return Ok(Vec::new()),
    };
    Ok(feats
        .iter()
        .map(|f| parse_stac_feature(f, collection))
        .collect())
}

/// Process-wide cache of MPC SAS tokens, keyed by collection. Microsoft
/// Planetary Computer issues anonymous read-only SAS tokens for any
/// public-data collection; tokens last ~1 hour. We refresh proactively
/// at 50 minutes so we don't race the expiry on a long materialize call.
struct CachedSas {
    token: String,
    fetched_at: Instant,
}
static SAS_CACHE: Mutex<Option<(String, CachedSas)>> = Mutex::new(None);

/// Fetch (or return cached) anonymous SAS token for an MPC collection.
/// Sign Azure asset URLs as `<href>?<token>` — token is the entire query
/// string starting with `sv=...`.
///
/// Retry policy: MPC's SAS endpoint is a small free service that returns
/// 429 Too Many Requests when many cells in a polygon recall race to
/// refresh the same collection's token. The cache (`SAS_CACHE`) is the
/// first line of defence — once one caller wins and stores a fresh
/// token, every other caller hits the cache. But on a cold cache or at
/// the 50-minute expiry boundary, the lock is process-wide non-async
/// so concurrent tasks can all bypass the cache and stampede the
/// upstream. This retry loop absorbs the resulting 429 burst with an
/// exponential backoff (200 ms → 400 ms → 800 ms → 1.6 s, capped at 5
/// attempts) and honours the server-supplied `Retry-After` header when
/// present (delegating both seconds-form and HTTP-date-form parsing to
/// the upstream's hint). 5xx responses follow the same backoff. The
/// 07:40 UTC outage on 2026-05-25 was a textbook fan-out 429 burst —
/// 11 simultaneous failures, no retry, all dropped — which this loop
/// is designed to prevent.
pub async fn mpc_sas_token(client: &Client, collection: &str) -> Result<String, String> {
    if let Ok(guard) = SAS_CACHE.lock() {
        if let Some((cached_collection, cached)) = guard.as_ref() {
            if cached_collection == collection
                && cached.fetched_at.elapsed() < Duration::from_secs(50 * 60)
            {
                return Ok(cached.token.clone());
            }
        }
    }
    let url = format!("https://planetarycomputer.microsoft.com/api/sas/v1/token/{collection}");
    const MAX_ATTEMPTS: u32 = 5;
    const BASE_BACKOFF_MS: u64 = 200;
    let mut last_err = String::from("mpc sas: unreached");
    for attempt in 1..=MAX_ATTEMPTS {
        let resp = match client
            .get(&url)
            .header(
                "user-agent",
                concat!(
                    "emem.dev/",
                    env!("CARGO_PKG_VERSION"),
                    " (avijeet@vortx.ai)"
                ),
            )
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                last_err = format!("mpc sas http (attempt {attempt}/{MAX_ATTEMPTS}): {e}");
                if attempt < MAX_ATTEMPTS {
                    let wait = Duration::from_millis(BASE_BACKOFF_MS * (1u64 << (attempt - 1)));
                    tokio::time::sleep(wait).await;
                    continue;
                }
                return Err(last_err);
            }
        };
        let status = resp.status();
        if status.is_success() {
            let v: Value = resp
                .json()
                .await
                .map_err(|e| format!("mpc sas json: {e}"))?;
            let token = v
                .get("token")
                .and_then(|t| t.as_str())
                .ok_or_else(|| "mpc sas response missing `token` field".to_string())?
                .to_string();
            if let Ok(mut guard) = SAS_CACHE.lock() {
                *guard = Some((
                    collection.to_string(),
                    CachedSas {
                        token: token.clone(),
                        fetched_at: Instant::now(),
                    },
                ));
            }
            return Ok(token);
        }
        // Retryable on 429 / 5xx; 4xx other than 429 is a permanent
        // error (bad collection, deprecated endpoint) and must not
        // burn the budget.
        let retryable = status.as_u16() == 429 || status.is_server_error();
        let retry_after_secs = if status.as_u16() == 429 {
            parse_retry_after_header(&resp)
        } else {
            None
        };
        last_err = format!(
            "mpc sas status {} (attempt {attempt}/{MAX_ATTEMPTS})",
            status
        );
        if !retryable || attempt == MAX_ATTEMPTS {
            return Err(last_err);
        }
        // Prefer the server's Retry-After value when present; otherwise
        // exponential backoff. Cap at 30 s so a maliciously-large
        // Retry-After doesn't wedge the request beyond the materializer
        // timeout budget.
        let backoff_ms = BASE_BACKOFF_MS * (1u64 << (attempt - 1));
        let wait_ms = retry_after_secs
            .map(|s| s.saturating_mul(1000).min(30_000))
            .unwrap_or(backoff_ms);
        tokio::time::sleep(Duration::from_millis(wait_ms)).await;
    }
    Err(last_err)
}

/// Parse an HTTP `Retry-After` header value. Supports the integer-seconds
/// form (RFC 9110 §10.2.3 delta-seconds); the HTTP-date form is silently
/// ignored (returns `None`) so the caller falls back to exponential
/// backoff. We deliberately don't pull `httpdate` for the parse — the
/// MPC endpoint always emits seconds in observed traffic, and an unparsed
/// HTTP-date just degrades to the same backoff we'd use without a hint.
fn parse_retry_after_header(resp: &reqwest::Response) -> Option<u64> {
    resp.headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
}
