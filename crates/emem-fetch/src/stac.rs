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
    search_one_at(client, STAC_ELEMENT84_V1, collection, lng, lat, datetime, max_cloud).await
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
        .header("user-agent", "emem.dev/0.0.2 (avijeet@vortx.ai)")
        .json(&body)
        .send().await
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
    let id = f.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let props = f.get("properties").cloned().unwrap_or(Value::Null);
    let cloud_cover = props.get("eo:cloud_cover").and_then(|v| v.as_f64());
    let datetime_str = props.get("datetime").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let epsg = props.get("proj:epsg").and_then(|v| v.as_u64()).map(|n| n as u32);
    let mut assets = std::collections::BTreeMap::new();
    if let Some(a) = f.get("assets").and_then(|a| a.as_object()) {
        for (k, v) in a {
            if let Some(href) = v.get("href").and_then(|h| h.as_str()) {
                assets.insert(k.clone(), href.to_string());
            }
        }
    }
    Ok(Some(StacItem {
        id, cloud_cover, datetime: datetime_str,
        epsg, assets,
        collection: collection.to_string(),
    }))
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
pub async fn mpc_sas_token(client: &Client, collection: &str) -> Result<String, String> {
    if let Ok(guard) = SAS_CACHE.lock() {
        if let Some((cached_collection, cached)) = guard.as_ref() {
            if cached_collection == collection
               && cached.fetched_at.elapsed() < Duration::from_secs(50 * 60) {
                return Ok(cached.token.clone());
            }
        }
    }
    let url = format!(
        "https://planetarycomputer.microsoft.com/api/sas/v1/token/{collection}"
    );
    let resp = client
        .get(&url)
        .header("user-agent", "emem.dev/0.0.2 (avijeet@vortx.ai)")
        .send().await
        .map_err(|e| format!("mpc sas http: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("mpc sas status {}", resp.status()));
    }
    let v: Value = resp.json().await.map_err(|e| format!("mpc sas json: {e}"))?;
    let token = v.get("token").and_then(|t| t.as_str())
        .ok_or_else(|| "mpc sas response missing `token` field".to_string())?
        .to_string();
    if let Ok(mut guard) = SAS_CACHE.lock() {
        *guard = Some((collection.to_string(), CachedSas {
            token: token.clone(),
            fetched_at: Instant::now(),
        }));
    }
    Ok(token)
}
