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
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
    /// `s2:processing_baseline` verbatim, e.g. `"05.11"`. Sentinel-2 only,
    /// and `None` when the catalogue does not publish the property (early
    /// L2A items predate it). The baseline decides how a raw ESA DN maps to
    /// reflectance: from baseline 04.00 (2022-01-25) onward ESA encodes
    /// L2A reflectance with a `BOA_ADD_OFFSET` of -1000 DN, so
    /// `reflectance = (DN - 1000) * 1e-4` against a catalogue that serves
    /// ESA's DNs unaltered. Element84 removes that offset before publishing
    /// (see [`s2_dn_is_harmonised`]), which is why the value path scales by
    /// 1e-4 with no offset term. Parsed and carried so a future provider
    /// swap has the fact available to branch on rather than having to
    /// guess; nothing is defaulted, because a wrong default here biases
    /// every index silently.
    #[serde(default)]
    pub processing_baseline: Option<String>,
}

/// The ESA processing baseline from which Sentinel-2 L2A reflectance
/// carries a `BOA_ADD_OFFSET` of -1000 DN. Baselines are published as
/// zero-padded `"MM.mm"` strings, which compare correctly as strings.
/// Source: ESA's 2022-01-25 baseline 04.00 product notice.
pub const S2_BOA_OFFSET_BASELINE: &str = "04.00";

/// The `BOA_ADD_OFFSET` ESA applies from [`S2_BOA_OFFSET_BASELINE`] onward,
/// in DN. A catalogue that serves ESA's DNs verbatim needs this ADDED to the
/// DN before the 1e-4 scale; a harmonised catalogue has already done it.
pub const S2_BOA_ADD_OFFSET_DN: f64 = -1000.0;

/// Does `search_url` serve Sentinel-2 L2A DNs with ESA's `BOA_ADD_OFFSET`
/// already folded in?
///
/// Element84's AWS Open Data mirror harmonises the L2A DN range across the
/// baseline 04.00 boundary: it publishes `DN_esa - 1000` for post-baseline
/// scenes so that a single `DN * 1e-4` reads correctly for every scene in
/// the archive. Microsoft Planetary Computer publishes ESA's DNs unaltered,
/// so the same `DN * 1e-4` over an MPC scene overstates reflectance by 0.1
/// in every band.
///
/// That difference cancels in no band-ratio index. NDVI is invariant to a
/// common *scale* on NIR and red but not to a common *offset*: adding `k` to
/// both bands changes `(NIR-RED)/(NIR+RED)` to
/// `(NIR-RED)/(NIR+RED+2k)`, which pulls the ratio toward zero. A +1000 DN
/// offset on both bands biases NDVI toward zero by roughly a third at
/// typical canopy reflectance. The bias is smooth, plausible-looking, and
/// invisible without a reference, which is why the value path asserts the
/// provider instead of trusting it.
pub fn s2_dn_is_harmonised(search_url: &str) -> bool {
    search_url == STAC_ELEMENT84_V1
}

/// Does an item on this baseline carry ESA's `BOA_ADD_OFFSET`? `None`
/// (baseline not published) returns `None`: the honest answer is "unknown",
/// not "no". Callers reading a harmonised catalogue do not need to ask.
pub fn s2_baseline_has_boa_offset(processing_baseline: Option<&str>) -> Option<bool> {
    let b = processing_baseline?;
    Some(b >= S2_BOA_OFFSET_BASELINE)
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
        .header("user-agent", emem_core::outbound::user_agent())
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
    // Absent on non-Sentinel-2 collections and on early L2A items. Left as
    // `None` rather than defaulted: see `StacItem::processing_baseline`.
    let processing_baseline = props
        .get("s2:processing_baseline")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
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
        processing_baseline,
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
        .header("user-agent", emem_core::outbound::user_agent())
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
/// public-data collection. The token's authoritative expiry is the
/// `msft:expiry` field of the sign response (mirrored in the SAS `se=`
/// param). Observed lifetimes are ~45 min, not the hour they once were,
/// and MPC has changed them before — so we key the cache off that
/// timestamp per token rather than assuming a fixed lifetime. A hardcoded
/// "assume 60 min, refresh at 50" TTL served the token for the 5 min
/// between its real 45-min expiry and our 50-min refresh, during which
/// Azure rejects every signed URL with `403 AuthenticationFailed`
/// ("Signature not valid in the specified time frame"). See issue #8.
struct CachedSas {
    token: String,
    fetched_at: Instant,
    /// How long *this* token is safe to serve from cache: its remaining
    /// lifetime at fetch time minus [`SAS_REFRESH_MARGIN`], clamped to
    /// `[SAS_MIN_TTL, SAS_MAX_TTL]`. Derived per token from `msft:expiry`,
    /// never a constant.
    valid_for: Duration,
}
static SAS_CACHE: Mutex<Option<(String, CachedSas)>> = Mutex::new(None);

/// Refresh a cached SAS token this long before its true expiry, so a
/// long materialize call never races the expiry and we never hand Azure a
/// signature that has expired (or is about to) mid-fetch.
const SAS_REFRESH_MARGIN: Duration = Duration::from_secs(5 * 60);
/// Never cache a token for less than this. Guards against a refetch storm
/// if MPC ever returns a token already within [`SAS_REFRESH_MARGIN`] of
/// its expiry — the token is still valid for this floor, so serving it
/// briefly is safe and cheaper than hammering the sign endpoint.
const SAS_MIN_TTL: Duration = Duration::from_secs(60);
/// Never cache a token for longer than this, even if `msft:expiry` claims
/// a far-future expiry (clock skew, or a schema surprise). Over-caching is
/// the failure mode we're fixing, so we bound it; a legitimately longer
/// token just costs one extra sign call per hour, well inside rate limits.
const SAS_MAX_TTL: Duration = Duration::from_secs(55 * 60);
/// Conservative fallback TTL, used only when `msft:expiry` is missing or
/// unparseable. Short enough to stay well inside any historical MPC token
/// lifetime, so an upstream schema change degrades to "refresh often",
/// never "serve expired".
const SAS_FALLBACK_TTL: Duration = Duration::from_secs(30 * 60);

/// Fetch (or return cached) anonymous SAS token for an MPC collection.
/// Sign Azure asset URLs as `<href>?<token>` — token is the entire query
/// string starting with `sv=...`.
///
/// Retry policy: MPC's SAS endpoint is a small free service that returns
/// 429 Too Many Requests when many cells in a polygon recall race to
/// refresh the same collection's token. The cache (`SAS_CACHE`) is the
/// first line of defence — once one caller wins and stores a fresh
/// token, every other caller hits the cache. But on a cold cache or at
/// the per-token refresh boundary, the lock is process-wide non-async
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
            if cached_collection == collection && cached.fetched_at.elapsed() < cached.valid_for {
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
            .header("user-agent", emem_core::outbound::user_agent())
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
            // Key the cache off the token's real expiry, not a fixed
            // lifetime. `msft:expiry` is RFC 3339 UTC; if it's absent or
            // unparseable we fall back to a short conservative TTL so we
            // degrade to "refresh often", never "serve expired".
            let valid_for = v
                .get("msft:expiry")
                .and_then(|e| e.as_str())
                .and_then(parse_rfc3339_utc_to_unix)
                .and_then(|expiry_unix| {
                    let now_unix =
                        SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs() as i64;
                    let remaining = expiry_unix - now_unix;
                    (remaining > 0).then(|| Duration::from_secs(remaining as u64))
                })
                .map(|remaining| {
                    remaining
                        .saturating_sub(SAS_REFRESH_MARGIN)
                        .clamp(SAS_MIN_TTL, SAS_MAX_TTL)
                })
                .unwrap_or(SAS_FALLBACK_TTL);
            if let Ok(mut guard) = SAS_CACHE.lock() {
                *guard = Some((
                    collection.to_string(),
                    CachedSas {
                        token: token.clone(),
                        fetched_at: Instant::now(),
                        valid_for,
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

/// Parse an RFC 3339 / ISO 8601 **UTC** timestamp
/// (`YYYY-MM-DDThh:mm:ss[.frac][Z]`) to Unix epoch seconds. This is the
/// exact shape of MPC's `msft:expiry` (e.g. `2026-07-05T14:37:25Z`).
///
/// Fractional seconds and a trailing `Z`/`z` are tolerated. Any explicit
/// numeric offset (`+hh:mm` / `-hh:mm`) or malformed field yields `None`,
/// so the caller falls back to a conservative TTL rather than silently
/// mis-reading a zoned timestamp as UTC. Parsed by hand to keep `chrono`
/// out of the fetch crate — same rationale as `firms::days_from_civil`.
fn parse_rfc3339_utc_to_unix(s: &str) -> Option<i64> {
    let (date, rest) = s.trim().split_once(['T', 't'])?;

    let mut dp = date.split('-');
    let y: i32 = dp.next()?.parse().ok()?;
    let mo: u32 = dp.next()?.parse().ok()?;
    let d: u32 = dp.next()?.parse().ok()?;
    if dp.next().is_some() || !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }

    // Strip an optional trailing UTC designator, then reject any remaining
    // sign: a bare `hh:mm:ss[.frac]` time never contains `+`/`-`, so their
    // presence means a non-UTC offset we refuse to guess at.
    let time = rest.strip_suffix(['Z', 'z']).unwrap_or(rest);
    if time.contains('+') || time.contains('-') {
        return None;
    }
    // Drop fractional seconds if present.
    let time = time.split('.').next().unwrap_or(time);

    let mut tp = time.split(':');
    let h: u32 = tp.next()?.parse().ok()?;
    let mi: u32 = tp.next()?.parse().ok()?;
    // Seconds are optional in RFC 3339's `partial-time`; default to 0.
    let se: u32 = tp.next().unwrap_or("0").parse().ok()?;
    if tp.next().is_some() || h > 23 || mi > 59 || se > 60 {
        return None; // se == 60 tolerates a leap second
    }

    Some(days_from_civil(y, mo, d) * 86_400 + h as i64 * 3600 + mi as i64 * 60 + se as i64)
}

/// Days since the Unix epoch (1970-01-01) for a proleptic-Gregorian date,
/// via Howard Hinnant's `days_from_civil`. Duplicated from `firms.rs` /
/// `opera_dist.rs` by design — the crate keeps this tiny helper local to
/// each user rather than pulling in `chrono` for one date conversion.
fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y / 400 } else { (y - 399) / 400 };
    let yoe = (y - era * 400) as u32;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    (era as i64) * 146_097 + (doe as i64) - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_s2_processing_baseline_property() {
        // Property shape as Element84 publishes it on a sentinel-2-l2a item.
        let feat = json!({
            "id": "S2C_30UXC_20260425_0_L2A",
            "properties": {
                "datetime": "2026-04-25T11:16:19Z",
                "eo:cloud_cover": 3.1,
                "proj:epsg": 32630,
                "s2:processing_baseline": "05.11"
            },
            "assets": {"red": {"href": "https://example.invalid/B04.tif"}}
        });
        let item = parse_stac_feature(&feat, "sentinel-2-l2a");
        assert_eq!(item.processing_baseline.as_deref(), Some("05.11"));
        assert_eq!(
            s2_baseline_has_boa_offset(item.processing_baseline.as_deref()),
            Some(true)
        );
    }

    #[test]
    fn missing_processing_baseline_stays_unknown_not_defaulted() {
        // Collections other than Sentinel-2 (and early L2A items) omit the
        // property. It must read back as "unknown", never as a guessed
        // baseline: a wrong guess silently biases every index.
        let feat = json!({
            "id": "S1A_IW_GRDH_20260425",
            "properties": {"datetime": "2026-04-25T11:16:19Z", "proj:epsg": 32630},
            "assets": {}
        });
        let item = parse_stac_feature(&feat, "sentinel-1-rtc");
        assert_eq!(item.processing_baseline, None);
        assert_eq!(s2_baseline_has_boa_offset(None), None);
    }

    #[test]
    fn boa_offset_applies_from_baseline_04_00_onward() {
        // The 2022-01-25 boundary: 03.01 has no offset, 04.00 is the first
        // baseline that does, and every later baseline keeps it.
        assert_eq!(s2_baseline_has_boa_offset(Some("02.07")), Some(false));
        assert_eq!(s2_baseline_has_boa_offset(Some("03.01")), Some(false));
        assert_eq!(s2_baseline_has_boa_offset(Some("04.00")), Some(true));
        assert_eq!(s2_baseline_has_boa_offset(Some("05.11")), Some(true));
    }

    #[test]
    fn only_element84_serves_harmonised_s2_dns() {
        // The invariant the Sentinel-2 value path is built on. Element84
        // removes ESA's BOA_ADD_OFFSET before publishing, so `DN * 1e-4` is
        // correct with no offset term. MPC serves ESA's DNs verbatim, so the
        // same arithmetic is wrong there by 0.1 reflectance per band.
        assert!(s2_dn_is_harmonised(STAC_ELEMENT84_V1));
        assert!(!s2_dn_is_harmonised(STAC_MPC_V1));
        // Anything unrecognised is not assumed harmonised.
        assert!(!s2_dn_is_harmonised(
            "https://stac.example.invalid/v1/search"
        ));
    }

    #[test]
    fn ndvi_is_scale_invariant_but_not_offset_invariant() {
        // Why the provider assertion exists rather than a comment. Synthetic
        // DNs (not a measurement): a canopy-like NIR/red pair.
        let ndvi = |nir: f64, red: f64| (nir - red) / (nir + red);
        let (nir_dn, red_dn) = (3000.0, 1000.0);
        let truth = ndvi(nir_dn * 1e-4, red_dn * 1e-4);
        // Scale cancels: the DN-domain ratio equals the reflectance-domain one.
        assert!((ndvi(nir_dn, red_dn) - truth).abs() < 1e-12);
        // Offset does not cancel. Reading ESA-offset DNs (what MPC serves)
        // as if they were harmonised biases NDVI toward zero.
        let biased = ndvi(
            (nir_dn - S2_BOA_ADD_OFFSET_DN) * 1e-4,
            (red_dn - S2_BOA_ADD_OFFSET_DN) * 1e-4,
        );
        assert!(
            biased.abs() < truth.abs(),
            "unremoved BOA offset must pull NDVI toward zero: biased={biased} truth={truth}"
        );
        // Removing the offset recovers the truth exactly.
        let corrected = ndvi(
            ((nir_dn - S2_BOA_ADD_OFFSET_DN) + S2_BOA_ADD_OFFSET_DN) * 1e-4,
            ((red_dn - S2_BOA_ADD_OFFSET_DN) + S2_BOA_ADD_OFFSET_DN) * 1e-4,
        );
        assert!((corrected - truth).abs() < 1e-12);
    }

    #[test]
    fn rfc3339_utc_parses_the_mpc_expiry_shape() {
        // The exact `msft:expiry` string observed live from the sign
        // endpoint. Epoch checked against days_from_civil so the test is
        // self-contained (no chrono).
        let unix = parse_rfc3339_utc_to_unix("2026-07-05T14:37:25Z").unwrap();
        let expected = days_from_civil(2026, 7, 5) * 86_400 + 14 * 3600 + 37 * 60 + 25;
        assert_eq!(unix, expected);
    }

    #[test]
    fn rfc3339_epoch_and_fractional_and_lowercase_t_and_z() {
        assert_eq!(parse_rfc3339_utc_to_unix("1970-01-01T00:00:00Z"), Some(0));
        // Fractional seconds are dropped; lowercase 't'/'z' accepted.
        assert_eq!(
            parse_rfc3339_utc_to_unix("2026-07-05t14:37:25.512z"),
            parse_rfc3339_utc_to_unix("2026-07-05T14:37:25Z"),
        );
        // Seconds are optional (RFC 3339 partial-time).
        assert_eq!(
            parse_rfc3339_utc_to_unix("2026-07-05T14:37Z"),
            parse_rfc3339_utc_to_unix("2026-07-05T14:37:00Z"),
        );
    }

    #[test]
    fn rfc3339_rejects_zoned_and_malformed() {
        // Non-UTC offsets must be refused, not silently read as UTC —
        // otherwise the caller would cache against a wrong instant.
        assert_eq!(parse_rfc3339_utc_to_unix("2026-07-05T14:37:25+05:30"), None);
        assert_eq!(parse_rfc3339_utc_to_unix("2026-07-05T14:37:25-05:00"), None);
        // Garbage / missing fields fall back (caller uses SAS_FALLBACK_TTL).
        assert_eq!(parse_rfc3339_utc_to_unix(""), None);
        assert_eq!(parse_rfc3339_utc_to_unix("2026-07-05"), None);
        assert_eq!(parse_rfc3339_utc_to_unix("2026-13-05T00:00:00Z"), None);
        assert_eq!(parse_rfc3339_utc_to_unix("2026-07-05T24:00:00Z"), None);
    }

    /// The core of issue #8: a token with less real lifetime than the old
    /// hardcoded 50-min TTL must yield a `valid_for` that refreshes *before*
    /// the true expiry, never after. This exercises the same arithmetic the
    /// success path runs, without a network round-trip.
    #[test]
    fn valid_for_refreshes_before_a_45min_token_expires() {
        let derive = |remaining_secs: i64| -> Duration {
            Duration::from_secs(remaining_secs as u64)
                .saturating_sub(SAS_REFRESH_MARGIN)
                .clamp(SAS_MIN_TTL, SAS_MAX_TTL)
        };
        // Live-observed ~45-min token: refresh at 40 min, 5 min before expiry.
        assert_eq!(derive(45 * 60), Duration::from_secs(40 * 60));
        // A token already inside the refresh margin is floored, not zero, so
        // it's still served briefly instead of triggering a refetch storm.
        assert_eq!(derive(3 * 60), SAS_MIN_TTL);
        // A suspiciously long expiry is capped so over-caching can't recur.
        assert_eq!(derive(6 * 3600), SAS_MAX_TTL);
        // Crucially: the derived TTL is always strictly less than the token's
        // real remaining lifetime for anything longer than the margin.
        for mins in [10, 20, 30, 44, 45, 50] {
            let remaining = Duration::from_secs(mins * 60);
            assert!(
                derive((mins * 60) as i64) < remaining,
                "valid_for must be < real lifetime for a {mins}-min token"
            );
        }
    }
}
