//! NASA FIRMS active-fire connector — open-data path.
//!
//! Source: **NASA FIRMS** (Fire Information for Resource Management System,
//! firms.modaps.eosdis.nasa.gov). Active-fire detections from MODIS
//! (Aqua + Terra, 1 km, 4×/day) and VIIRS (S-NPP, NOAA-20, NOAA-21,
//! 375 m, 8-10×/day combined). Near-real-time lag is ~3 hours from
//! satellite overpass.
//!
//! The FIRMS REST `/api/area/...` and `/api/country/...` endpoints
//! require a free `MAP_KEY` registration — under the project_open_data
//! "no key-gated sources for default build" rule those are out. The
//! anonymous bulk-CSV download under
//! `https://firms.modaps.eosdis.nasa.gov/data/active_fire/<source>/csv/`
//! is fully open: HTTP 200, no auth, no Earthdata redirect, ETag,
//! `accept-ranges: bytes`. Verified live 2026-05-06.
//!
//! Wire path:
//! 1. On first call, fetch all four sensors' `..._Global_24h.csv` files
//!    (~14 MB total combined).
//! 2. Parse each into a unified `FireDetection` list (~10⁵ detections
//!    globally on a typical day).
//! 3. Cache the resulting list in a process-local `RwLock` keyed by
//!    fetched-at instant. Subsequent calls within the cache TTL
//!    (60 minutes; FIRMS regenerates the file every ~3 hours) skip
//!    HTTP entirely.
//! 4. After TTL expires, do a conditional GET with `If-None-Match` so
//!    we only re-pull bytes when upstream actually rotated.
//!
//! The per-cell query is a linear scan filtered by bbox + tslot. With
//! ≤200 k detections worldwide per day this is microseconds — an
//! R-tree adds dependency surface for no measurable win at this size.
//!
//! Confidence normalization is the single load-bearing detail: MODIS
//! ships `confidence` as an integer 0..100 while VIIRS ships it as the
//! string `low|nominal|high`. We unify into a `u8` 0..100 (VIIRS:
//! low=20, nominal=50, high=90) so downstream code reads one type. The
//! sensor field in the output disambiguates.

use std::collections::HashMap;
use std::str::FromStr;
use std::time::{Duration, Instant};

use bytes::Bytes;
use reqwest::Client;
use tokio::sync::RwLock;

const FIRMS_BASE_URL: &str = "https://firms.modaps.eosdis.nasa.gov/data/active_fire";

/// Cache TTL — re-validate after this duration (FIRMS regenerates the
/// file every ~3 hours so 60 minutes is comfortably more frequent than
/// the upstream rotation cadence). The actual re-pull is a conditional
/// GET; if upstream hasn't rotated we don't transfer the bytes.
const CACHE_TTL: Duration = Duration::from_secs(60 * 60);

/// Source-side rolling window the bulk CSV covers. A query for a tslot
/// older than this returns Absence with reason `outside_window`.
const WINDOW_SECS: i64 = 24 * 3600;

/// Each sensor maps to one source ID + one file prefix at FIRMS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Sensor {
    /// MODIS Aqua + Terra fused into one MCD14DL_NRT global feed.
    Modis,
    /// VIIRS Suomi-NPP (VNP14IMGTDL_NRT).
    ViirsSnpp,
    /// VIIRS NOAA-20 (VJ114IMGTDL_NRT).
    ViirsJ1,
    /// VIIRS NOAA-21 (VJ214IMGTDL_NRT).
    ViirsJ2,
}

impl Sensor {
    /// FIRMS source-id path component (`<source>` in the URL template).
    pub const fn source_id(self) -> &'static str {
        match self {
            Sensor::Modis => "modis-c6.1",
            Sensor::ViirsSnpp => "suomi-npp-viirs-c2",
            Sensor::ViirsJ1 => "noaa-20-viirs-c2",
            Sensor::ViirsJ2 => "noaa-21-viirs-c2",
        }
    }

    /// FIRMS file-prefix used in the bulk-CSV filename.
    pub const fn file_prefix(self) -> &'static str {
        match self {
            Sensor::Modis => "MODIS_C6_1",
            Sensor::ViirsSnpp => "SUOMI_VIIRS_C2",
            Sensor::ViirsJ1 => "J1_VIIRS_C2",
            Sensor::ViirsJ2 => "J2_VIIRS_C2",
        }
    }

    /// Single bit in the per-cell sensor bitmask (so a recall response
    /// can answer "which satellites saw this fire" cheaply).
    pub const fn bitmask(self) -> u8 {
        match self {
            Sensor::Modis => 0b0001,
            Sensor::ViirsSnpp => 0b0010,
            Sensor::ViirsJ1 => 0b0100,
            Sensor::ViirsJ2 => 0b1000,
        }
    }

    /// Short tag for the sensors array in CBOR output.
    pub const fn tag(self) -> &'static str {
        match self {
            Sensor::Modis => "modis",
            Sensor::ViirsSnpp => "viirs_snpp",
            Sensor::ViirsJ1 => "viirs_j1",
            Sensor::ViirsJ2 => "viirs_j2",
        }
    }

    /// Iteration order over all four sensors. Stable so the output
    /// `sensors` array stays comparable across recalls.
    pub const fn all() -> [Sensor; 4] {
        [
            Sensor::Modis,
            Sensor::ViirsSnpp,
            Sensor::ViirsJ1,
            Sensor::ViirsJ2,
        ]
    }
}

/// Bulk-CSV URL for a sensor + window. Only the 24h window is wired —
/// the 7-day file would let an agent backfill a recent week without
/// any extra HTTP, but the per-tslot recall path doesn't need it yet.
fn csv_url(sensor: Sensor) -> String {
    format!(
        "{base}/{src}/csv/{prefix}_Global_24h.csv",
        base = FIRMS_BASE_URL,
        src = sensor.source_id(),
        prefix = sensor.file_prefix(),
    )
}

/// One fire detection, post-normalization. The unified shape lets the
/// per-cell scan return one type regardless of which CSV it came from.
#[derive(Debug, Clone)]
pub struct FireDetection {
    pub lat: f64,
    pub lng: f64,
    /// Acquisition time, decoded from `acq_date` (YYYY-MM-DD) + `acq_time` (HHMM).
    pub acq_unix_s: i64,
    /// Fire Radiative Power, MW. Both MODIS and VIIRS publish this in
    /// the same physical unit so no per-sensor scaling is needed.
    pub frp_mw: f32,
    /// Brightness temperature, Kelvin. MODIS reports band 21
    /// (`brightness`); VIIRS reports I-4 (`bright_ti4`). Same physical
    /// quantity, slightly different bands — useful as a magnitude
    /// indicator but NOT directly comparable across sensors.
    pub brightness_k: f32,
    pub sensor: Sensor,
    /// Confidence normalized to 0..100. MODIS is already that scale;
    /// VIIRS is mapped low=20, nominal=50, high=90 (mid-range of each
    /// labelled band, per FIRMS attribute table).
    pub confidence_0_100: u8,
    /// Day or night detection. `D` for daytime overpass.
    pub daynight: char,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViirsConfidence {
    Low,
    Nominal,
    High,
}

impl ViirsConfidence {
    /// Map the FIRMS string label to a 0..100 score. Mid-range of each
    /// labelled band: low ≈ [0,40), nominal ≈ [40,80), high ≈ [80,100].
    pub const fn as_u8(self) -> u8 {
        match self {
            ViirsConfidence::Low => 20,
            ViirsConfidence::Nominal => 50,
            ViirsConfidence::High => 90,
        }
    }
}

impl FromStr for ViirsConfidence {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "l" | "low" => Ok(Self::Low),
            "n" | "nominal" => Ok(Self::Nominal),
            "h" | "high" => Ok(Self::High),
            _ => Err(()),
        }
    }
}

/// Errors the materializer needs to distinguish.
#[derive(Debug, thiserror::Error)]
pub enum FirmsError {
    /// Network / transport failure.
    #[error("firms transport: {0}")]
    Transport(String),
    /// Upstream returned non-2xx.
    #[error("firms upstream {sensor:?} returned status {status}")]
    Upstream { sensor: Sensor, status: u16 },
    /// CSV decode / row parse failure (corrupt upstream).
    #[error("firms csv parse: {0}")]
    Parse(String),
    /// Tslot is outside the 24h rolling window. The materializer signs
    /// a structured Absence rather than fail; this variant is the
    /// hint the materializer maps to that Absence.
    #[error("firms tslot outside 24h window")]
    OutsideWindow,
}

/// Per-cell summary returned to the materializer. CBOR-friendly shape.
#[derive(Debug, Clone)]
pub struct FireSummary {
    pub detections: u32,
    pub frp_max_mw: f32,
    pub frp_sum_mw: f32,
    pub last_detection_unix_s: i64,
    pub conf_max: u8,
    /// Bitmask: MODIS=1, VIIRS-SNPP=2, VIIRS-J1=4, VIIRS-J2=8.
    pub sensors_bitmask: u8,
    /// 0..255 ratio: 0 = night-only, 255 = day-only.
    pub daynight_mix_u8: u8,
    /// Origin URL of each upstream CSV that contributed at least one
    /// detection to this summary — for the signed receipt's `sources[]`.
    pub source_urls: Vec<String>,
}

/// Process-local cache. Initialised on first fetch; subsequent calls
/// reuse the parsed detection vector if still inside `CACHE_TTL`.
struct CacheEntry {
    fetched_at: Instant,
    detections: Vec<FireDetection>,
    /// Per-sensor `ETag` so the next refresh can do a conditional GET.
    etags: HashMap<Sensor, String>,
}

static CACHE: tokio::sync::OnceCell<RwLock<Option<CacheEntry>>> =
    tokio::sync::OnceCell::const_new();

async fn cache() -> &'static RwLock<Option<CacheEntry>> {
    CACHE.get_or_init(|| async { RwLock::new(None) }).await
}

/// Public entry point. Given a cell bbox and a target unix timestamp,
/// returns a fire summary or `OutsideWindow` if the tslot is outside
/// the 24h rolling window. The caller (materializer) decides how to
/// surface zero-detection results — typically as a signed Absence.
pub async fn fetch_fires_for_cell(
    client: &Client,
    bbox_min_lat: f64,
    bbox_max_lat: f64,
    bbox_min_lng: f64,
    bbox_max_lng: f64,
    tslot_start_unix_s: i64,
    tslot_end_unix_s: i64,
) -> Result<FireSummary, FirmsError> {
    refresh_cache_if_stale(client).await?;
    let guard = cache().await.read().await;
    let entry = guard
        .as_ref()
        .ok_or_else(|| FirmsError::Transport("cache empty after refresh".into()))?;

    // Window-guard. The 24h file's youngest detection sets the upper
    // bound; queries newer than that are simply "no data yet". Older
    // queries fall outside the file's coverage.
    let max_acq = entry
        .detections
        .iter()
        .map(|d| d.acq_unix_s)
        .max()
        .unwrap_or(0);
    let window_floor = max_acq.saturating_sub(WINDOW_SECS);
    if tslot_end_unix_s < window_floor {
        return Err(FirmsError::OutsideWindow);
    }

    let mut frp_max: f32 = 0.0;
    let mut frp_sum: f32 = 0.0;
    let mut conf_max: u8 = 0;
    let mut sensors_bitmask: u8 = 0;
    let mut count: u32 = 0;
    let mut last_unix: i64 = 0;
    let mut day_count: u32 = 0;
    let mut source_urls: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    for d in &entry.detections {
        if d.lat < bbox_min_lat
            || d.lat > bbox_max_lat
            || d.lng < bbox_min_lng
            || d.lng > bbox_max_lng
        {
            continue;
        }
        if d.acq_unix_s < tslot_start_unix_s || d.acq_unix_s > tslot_end_unix_s {
            continue;
        }
        count += 1;
        frp_max = frp_max.max(d.frp_mw);
        frp_sum += d.frp_mw;
        conf_max = conf_max.max(d.confidence_0_100);
        sensors_bitmask |= d.sensor.bitmask();
        if d.acq_unix_s > last_unix {
            last_unix = d.acq_unix_s;
        }
        if d.daynight == 'D' || d.daynight == 'd' {
            day_count += 1;
        }
        source_urls.insert(csv_url(d.sensor));
    }

    let daynight_mix_u8 = (day_count.saturating_mul(255))
        .checked_div(count)
        .map(|v| v.min(255) as u8)
        .unwrap_or(0);

    Ok(FireSummary {
        detections: count,
        frp_max_mw: frp_max,
        frp_sum_mw: frp_sum,
        last_detection_unix_s: last_unix,
        conf_max,
        sensors_bitmask,
        daynight_mix_u8,
        source_urls: source_urls.into_iter().collect(),
    })
}

/// Refresh the cache if expired. Holds the write lock across HTTP so
/// concurrent recalls don't all race the upstream — first one fetches,
/// the rest wait and read the populated cache.
async fn refresh_cache_if_stale(client: &Client) -> Result<(), FirmsError> {
    let needs_refresh = {
        let guard = cache().await.read().await;
        match guard.as_ref() {
            None => true,
            Some(e) => e.fetched_at.elapsed() > CACHE_TTL,
        }
    };
    if !needs_refresh {
        return Ok(());
    }
    let mut guard = cache().await.write().await;
    // Re-check inside the write lock to avoid double-fetch races.
    if let Some(e) = guard.as_ref() {
        if e.fetched_at.elapsed() <= CACHE_TTL {
            return Ok(());
        }
    }
    // Carry the previous etag map for conditional GETs.
    let prev_etags = guard.as_ref().map(|e| e.etags.clone()).unwrap_or_default();

    let mut all: Vec<FireDetection> = Vec::with_capacity(200_000);
    let mut new_etags: HashMap<Sensor, String> = HashMap::new();
    for sensor in Sensor::all() {
        let url = csv_url(sensor);
        let mut req = client.get(&url).header(
            "user-agent",
            "emem.dev-fetch/0.x (https://emem.dev/contact)",
        );
        if let Some(etag) = prev_etags.get(&sensor) {
            req = req.header("if-none-match", etag);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| FirmsError::Transport(format!("{sensor:?}: {e}")))?;
        let status = resp.status();
        let etag = resp
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        if status.as_u16() == 304 {
            // Upstream unchanged. Reuse previous detections for this sensor.
            if let Some(prev) = guard
                .as_ref()
                .map(|e| e.detections.iter().filter(|d| d.sensor == sensor).cloned())
            {
                all.extend(prev);
            }
            if let Some(et) = etag {
                new_etags.insert(sensor, et);
            }
            continue;
        }
        if !status.is_success() {
            return Err(FirmsError::Upstream {
                sensor,
                status: status.as_u16(),
            });
        }
        let body: Bytes = resp
            .bytes()
            .await
            .map_err(|e| FirmsError::Transport(format!("{sensor:?} body: {e}")))?;
        let parsed = parse_firms_csv(&body, sensor)?;
        tracing::debug!(target:"emem_fetch::firms", sensor=?sensor, n=parsed.len(), "loaded firms csv");
        all.extend(parsed);
        if let Some(et) = etag {
            new_etags.insert(sensor, et);
        }
    }

    *guard = Some(CacheEntry {
        fetched_at: Instant::now(),
        detections: all,
        etags: new_etags,
    });
    Ok(())
}

/// Parse a FIRMS CSV body into typed detections. Handles both the
/// MODIS schema (numeric `confidence`, fields `brightness` + `bright_t31`)
/// and the VIIRS schema (string `confidence`, fields `bright_ti4` +
/// `bright_ti5`) by inspecting the header row.
pub fn parse_firms_csv(body: &[u8], sensor: Sensor) -> Result<Vec<FireDetection>, FirmsError> {
    let text =
        std::str::from_utf8(body).map_err(|e| FirmsError::Parse(format!("non-utf8 body: {e}")))?;
    let mut lines = text.lines();
    let header = lines
        .next()
        .ok_or_else(|| FirmsError::Parse("empty csv".into()))?;
    let cols: Vec<&str> = header.split(',').map(str::trim).collect();

    let idx = |name: &str| cols.iter().position(|c| *c == name);
    let i_lat = idx("latitude").ok_or_else(|| FirmsError::Parse("no latitude col".into()))?;
    let i_lng = idx("longitude").ok_or_else(|| FirmsError::Parse("no longitude col".into()))?;
    let i_conf = idx("confidence").ok_or_else(|| FirmsError::Parse("no confidence col".into()))?;
    let i_acq_date = idx("acq_date").ok_or_else(|| FirmsError::Parse("no acq_date col".into()))?;
    let i_acq_time = idx("acq_time").ok_or_else(|| FirmsError::Parse("no acq_time col".into()))?;
    let i_frp = idx("frp").ok_or_else(|| FirmsError::Parse("no frp col".into()))?;
    let i_daynight = idx("daynight").ok_or_else(|| FirmsError::Parse("no daynight col".into()))?;
    // Brightness column varies by sensor.
    let i_bright = idx("brightness")
        .or_else(|| idx("bright_ti4"))
        .ok_or_else(|| FirmsError::Parse("no brightness/bright_ti4 col".into()))?;

    let mut out = Vec::new();
    for (row_idx, line) in lines.enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split(',').collect();
        let parse_err = |what: &str| {
            FirmsError::Parse(format!(
                "row {row_idx} sensor={sensor:?}: {what}; line=`{line}`"
            ))
        };
        let lat: f64 = fields
            .get(i_lat)
            .and_then(|s| s.trim().parse().ok())
            .ok_or_else(|| parse_err("bad latitude"))?;
        let lng: f64 = fields
            .get(i_lng)
            .and_then(|s| s.trim().parse().ok())
            .ok_or_else(|| parse_err("bad longitude"))?;
        let frp: f32 = fields
            .get(i_frp)
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0.0);
        let bright: f32 = fields
            .get(i_bright)
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0.0);
        let conf_raw = fields.get(i_conf).map(|s| s.trim()).unwrap_or("");
        let conf_u8 = if let Ok(n) = conf_raw.parse::<u8>() {
            n.min(100)
        } else if let Ok(v) = ViirsConfidence::from_str(conf_raw) {
            v.as_u8()
        } else {
            // Skip rows with un-decodable confidence rather than
            // poison the whole file.
            tracing::warn!(target:"emem_fetch::firms", row=row_idx, conf=conf_raw, "skip row: bad confidence");
            continue;
        };
        let acq_date = fields.get(i_acq_date).map(|s| s.trim()).unwrap_or("");
        let acq_time = fields.get(i_acq_time).map(|s| s.trim()).unwrap_or("");
        let acq_unix = parse_firms_acq_to_unix(acq_date, acq_time)
            .ok_or_else(|| parse_err("bad acq_date/acq_time"))?;
        let daynight = fields
            .get(i_daynight)
            .and_then(|s| s.trim().chars().next())
            .unwrap_or('?');
        out.push(FireDetection {
            lat,
            lng,
            acq_unix_s: acq_unix,
            frp_mw: frp,
            brightness_k: bright,
            sensor,
            confidence_0_100: conf_u8,
            daynight,
        });
    }
    Ok(out)
}

/// FIRMS ships `acq_date` as `YYYY-MM-DD` (UTC) and `acq_time` as a
/// zero-padded `HHMM` string. Combine into a Unix epoch second.
pub fn parse_firms_acq_to_unix(acq_date: &str, acq_time: &str) -> Option<i64> {
    // Accept either `HHMM` (4 chars) or `HMM` (3 chars when the leading
    // hour is 0..9 unpadded) — FIRMS pads, but be lenient.
    let acq_time = acq_time.trim();
    let (h, m) = match acq_time.len() {
        4 => (
            acq_time.get(..2)?.parse::<u32>().ok()?,
            acq_time.get(2..4)?.parse::<u32>().ok()?,
        ),
        3 => (
            acq_time.get(..1)?.parse::<u32>().ok()?,
            acq_time.get(1..3)?.parse::<u32>().ok()?,
        ),
        _ => return None,
    };
    if h > 23 || m > 59 {
        return None;
    }
    let mut parts = acq_date.split('-');
    let y: i32 = parts.next()?.parse().ok()?;
    let mo: u32 = parts.next()?.parse().ok()?;
    let d: u32 = parts.next()?.parse().ok()?;
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    let days = days_from_civil(y, mo, d);
    Some(days * 86_400 + (h as i64) * 3600 + (m as i64) * 60)
}

/// Hinnant's days-from-civil — same routine the rest of emem-fetch
/// uses for date arithmetic. Pure, no allocation.
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
    fn parse_acq_handles_padded_and_unpadded_time() {
        // FIRMS pads acq_time to 4 chars; 4-char path is the live shape.
        let day = days_from_civil(2026, 5, 6);
        let unix_1842 = parse_firms_acq_to_unix("2026-05-06", "1842").unwrap();
        assert_eq!(unix_1842, day * 86_400 + 18 * 3600 + 42 * 60);
        // 3-char "be lenient" path: leading hour-digit-zero is dropped, so
        // "842" is 08:42 UTC, NOT 18:42. Locking the lenient semantics so
        // a future refactor that swaps the two branches is caught.
        let unix_842 = parse_firms_acq_to_unix("2026-05-06", "842").unwrap();
        assert_eq!(unix_842, day * 86_400 + 8 * 3600 + 42 * 60);
        assert_ne!(unix_842, unix_1842);
    }

    #[test]
    fn parse_acq_rejects_garbage() {
        assert!(parse_firms_acq_to_unix("2026-05-06", "abcd").is_none());
        assert!(parse_firms_acq_to_unix("2026-05-06", "2500").is_none()); // hour overflow
        assert!(parse_firms_acq_to_unix("2026-05-06", "1865").is_none()); // minute overflow
        assert!(parse_firms_acq_to_unix("2026/05/06", "1842").is_none()); // wrong separator
        assert!(parse_firms_acq_to_unix("not-a-date", "1842").is_none());
    }

    #[test]
    fn viirs_confidence_normalises_to_0_100() {
        assert_eq!(ViirsConfidence::Low.as_u8(), 20);
        assert_eq!(ViirsConfidence::Nominal.as_u8(), 50);
        assert_eq!(ViirsConfidence::High.as_u8(), 90);
        assert_eq!(
            "low".parse::<ViirsConfidence>().unwrap(),
            ViirsConfidence::Low
        );
        assert_eq!(
            "Nominal".parse::<ViirsConfidence>().unwrap(),
            ViirsConfidence::Nominal
        );
        assert_eq!(
            "h".parse::<ViirsConfidence>().unwrap(),
            ViirsConfidence::High
        );
        assert!("medium".parse::<ViirsConfidence>().is_err());
    }

    #[test]
    fn parse_firms_csv_modis_shape() {
        // Synthetic MODIS row matching the live FIRMS schema. confidence is integer.
        let csv = "latitude,longitude,brightness,scan,track,acq_date,acq_time,satellite,confidence,version,bright_t31,frp,daynight\n\
                   -1.5,30.2,320.5,1.1,1.1,2026-05-06,1342,T,84,6.1NRT,295.0,17.4,D\n\
                   2.0,-50.0,302.0,1.0,1.0,2026-05-06,0418,A,55,6.1NRT,288.0,5.5,N\n";
        let fires = parse_firms_csv(csv.as_bytes(), Sensor::Modis).unwrap();
        assert_eq!(fires.len(), 2);
        assert_eq!(fires[0].sensor, Sensor::Modis);
        assert!((fires[0].lat - -1.5).abs() < 1e-6);
        assert!((fires[0].frp_mw - 17.4).abs() < 1e-3);
        assert_eq!(fires[0].confidence_0_100, 84);
        assert_eq!(fires[0].daynight, 'D');
    }

    #[test]
    fn parse_firms_csv_viirs_shape_normalises_string_confidence() {
        // Synthetic VIIRS row. confidence is string {low|nominal|high}.
        let csv = "latitude,longitude,bright_ti4,scan,track,acq_date,acq_time,satellite,confidence,version,bright_ti5,frp,daynight\n\
                   -1.5,30.2,330.0,0.4,0.4,2026-05-06,1330,N,nominal,2.0NRT,300.0,12.0,D\n\
                   2.0,-50.0,318.0,0.4,0.4,2026-05-06,0405,1,high,2.0NRT,295.0,8.5,N\n\
                   3.0,-51.0,302.0,0.4,0.4,2026-05-06,0410,2,low,2.0NRT,290.0,2.0,N\n";
        let fires = parse_firms_csv(csv.as_bytes(), Sensor::ViirsSnpp).unwrap();
        assert_eq!(fires.len(), 3);
        assert_eq!(fires[0].confidence_0_100, 50); // nominal
        assert_eq!(fires[1].confidence_0_100, 90); // high
        assert_eq!(fires[2].confidence_0_100, 20); // low
        assert!((fires[1].brightness_k - 318.0).abs() < 1e-3);
    }

    #[test]
    fn parse_firms_csv_skips_undecodable_confidence_rows() {
        let csv = "latitude,longitude,bright_ti4,scan,track,acq_date,acq_time,satellite,confidence,version,bright_ti5,frp,daynight\n\
                   1.0,2.0,300.0,0.4,0.4,2026-05-06,1200,N,bogus,2.0NRT,290.0,1.0,D\n\
                   3.0,4.0,300.0,0.4,0.4,2026-05-06,1200,N,nominal,2.0NRT,290.0,1.0,D\n";
        let fires = parse_firms_csv(csv.as_bytes(), Sensor::ViirsSnpp).unwrap();
        // First row is dropped (bad confidence); second is kept.
        assert_eq!(fires.len(), 1);
        assert!((fires[0].lat - 3.0).abs() < 1e-6);
    }

    #[test]
    fn sensor_bitmask_is_unique_per_sensor() {
        let masks: Vec<u8> = Sensor::all().iter().map(|s| s.bitmask()).collect();
        assert_eq!(masks, vec![0b0001, 0b0010, 0b0100, 0b1000]);
    }

    #[test]
    fn csv_url_uses_documented_template() {
        // FIRMS bulk-CSV URL pattern verified anonymous in the deepscan
        // research agent's probes (HTTP 200, no auth, ETag).
        assert_eq!(
            csv_url(Sensor::Modis),
            "https://firms.modaps.eosdis.nasa.gov/data/active_fire/modis-c6.1/csv/MODIS_C6_1_Global_24h.csv"
        );
        assert_eq!(
            csv_url(Sensor::ViirsSnpp),
            "https://firms.modaps.eosdis.nasa.gov/data/active_fire/suomi-npp-viirs-c2/csv/SUOMI_VIIRS_C2_Global_24h.csv"
        );
    }
}
