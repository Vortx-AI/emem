//! NASA OPERA Land Surface Disturbance Alert from HLS (DIST-ALERT-HLS V1).
//!
//! **Product:** `OPERA_L3_DIST-ALERT-HLS_V1` — near-real-time (2–4 day
//! latency) vegetation-disturbance alerts derived from Harmonized Landsat
//! Sentinel-2 (HLS), 30 m, global, daily since 2022. CMR STAC collection
//! `OPERA_L3_DIST-ALERT-HLS_V1_1` (concept-id `C2746980408-LPCLOUD`),
//! distributed by NASA LP DAAC as Cloud-Optimized GeoTIFFs.
//!
//! **The two layers emem cares about:**
//! - `VEG-DIST-STATUS` (uint8): 0 = no disturbance; 1–4 = provisional
//!   (<50% / ≥50% change, first / recurrent); 5–8 = confirmed; 255 = no-data.
//! - `VEG-DIST-DATE` (int16): the date of FIRST disturbance detection,
//!   encoded as **days since 2020-12-31** (so day 1 = 2021-01-01). -1 / no
//!   data where there is no alert.
//!
//! **Access — the honest story.** Unlike RADD (Requester-Pays S3 with no
//! public COG), OPERA DIST-ALERT is **NOT requester-pays**: it is gated by
//! NASA **Earthdata Login (EDL)**. The operator provisions a free 60-day EDL
//! bearer token (urs.earthdata.nasa.gov/api/users/find_or_create_token) and
//! stores it **server-side** — via `EMEM_EARTHDATA_TOKEN` or a file path in
//! `EMEM_EARTHDATA_TOKEN_FILE`. The credential is NEVER committed to the
//! repository (the code is public). When no token is configured, the
//! materializer signs an honest `Absence` with reason `not_enabled` — exactly
//! the RADD pattern — so the responder behaves identically to today and never
//! breaks. No hardcoded token, no fabricated alert.
//!
//! This module ships the product constants, the EDL token loader, the
//! date-decode (days-since-epoch → real calendar date), and a structured
//! [`OperaDistError`] so the materializer can disclose precisely why a pixel
//! is unavailable. The live STAC-discovery + COG range-read against the
//! EDL-protected host is staged behind [`fetch_alert`] (it returns a
//! structured `NotImplemented` describing the remaining wiring) so the
//! credential path + optionality land now without fabricating data.

use std::path::Path;

/// CMR STAC collection id for OPERA DIST-ALERT-HLS V1.
pub const OPERA_DIST_COLLECTION: &str = "OPERA_L3_DIST-ALERT-HLS_V1_1";

/// CMR concept-id for the collection (stable handle).
pub const OPERA_DIST_CONCEPT_ID: &str = "C2746980408-LPCLOUD";

/// EDL-protected LP DAAC host the COGs live behind.
pub const OPERA_DIST_HOST: &str = "https://data.lpdaac.earthdatacloud.nasa.gov";

/// The `VEG-DIST-DATE` epoch: the int16 value is the number of days AFTER
/// this date. This is exactly the EUDR cut-off date (Regulation (EU)
/// 2023/1115 Article 1(2)), which is a happy coincidence — a positive
/// `VEG-DIST-DATE` is, by construction, a post-cut-off disturbance.
pub const VEG_DIST_DATE_EPOCH: (i32, u32, u32) = (2020, 12, 31);

/// `VEG-DIST-STATUS` value at or above which a pixel counts as a CONFIRMED
/// disturbance (vs provisional). Confirmed = detected in ≥2 HLS observations.
pub const VEG_DIST_STATUS_CONFIRMED_MIN: u8 = 5;

/// Errors from the OPERA DIST-ALERT connector.
#[derive(Debug, thiserror::Error)]
pub enum OperaDistError {
    /// No Earthdata Login token configured on this responder. The
    /// materializer MUST sign an honest `Absence` (never fabricate an
    /// alert). This is the default, no-regression state.
    #[error(
        "not_enabled: OPERA DIST-ALERT (NASA near-real-time disturbance) is optional and not \
         provisioned on this responder. It is EDL-gated (NOT requester-pays); an operator can \
         enable it by setting EMEM_EARTHDATA_TOKEN (or EMEM_EARTHDATA_TOKEN_FILE) to a free \
         60-day Earthdata Login token from urs.earthdata.nasa.gov/api/users/find_or_create_token \
         and restarting. Until then this band signs a signed Absence."
    )]
    NotEnabled,

    /// Token is configured but the live STAC-discovery + COG range-read
    /// pipeline is not yet wired. Honest disclosure, never a placeholder
    /// Primary.
    #[error("not_implemented: {0}")]
    NotImplemented(String),

    /// STAC / CMR discovery failed (transport, JSON, no granule).
    #[error("discovery: {0}")]
    Discovery(String),

    /// HTTP / network failure during the authenticated COG fetch.
    #[error("transport: {0}")]
    Transport(String),

    /// COG decode failure.
    #[error("decode: {0}")]
    Decode(String),
}

/// One decoded OPERA DIST-ALERT pixel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperaDistPixel {
    /// `VEG-DIST-STATUS` (0 = none, 1–4 provisional, 5–8 confirmed, 255 nodata).
    pub veg_dist_status: u8,
    /// `VEG-DIST-DATE` raw int16 (days since 2020-12-31; ≤0 = no alert/nodata).
    pub veg_dist_date_days: i16,
}

impl OperaDistPixel {
    /// True when this pixel is a confirmed disturbance.
    pub fn is_confirmed(&self) -> bool {
        self.veg_dist_status >= VEG_DIST_STATUS_CONFIRMED_MIN && self.veg_dist_status != 255
    }

    /// The real calendar date (Y, M, D) of first disturbance detection, or
    /// `None` when there is no alert (`veg_dist_date_days <= 0`). Decoded as
    /// `2020-12-31 + days` so the surfaced date is a true, scientific
    /// acquisition-derived date — never an anchor or a guess.
    pub fn alert_date(&self) -> Option<(i32, u32, u32)> {
        if self.veg_dist_date_days <= 0 {
            return None;
        }
        Some(add_days_to_epoch(
            VEG_DIST_DATE_EPOCH,
            self.veg_dist_date_days as i64,
        ))
    }

    /// ISO-8601 date string of the alert, or `None` when no alert.
    pub fn alert_date_iso(&self) -> Option<String> {
        self.alert_date()
            .map(|(y, m, d)| format!("{y:04}-{m:02}-{d:02}"))
    }
}

/// Load the Earthdata Login bearer token from the environment, preferring an
/// inline `EMEM_EARTHDATA_TOKEN` over a file path in
/// `EMEM_EARTHDATA_TOKEN_FILE`. Returns `None` when neither is set (or the
/// value/file is empty) — the no-regression default. The token is a secret:
/// callers MUST NOT log it.
pub fn earthdata_token_from_env() -> Option<String> {
    if let Ok(tok) = std::env::var("EMEM_EARTHDATA_TOKEN") {
        let t = tok.trim();
        if !t.is_empty() {
            return Some(t.to_string());
        }
    }
    if let Ok(path) = std::env::var("EMEM_EARTHDATA_TOKEN_FILE") {
        if let Ok(contents) = std::fs::read_to_string(Path::new(&path)) {
            let t = contents.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    None
}

/// True when OPERA DIST-ALERT is enabled on this responder (a token is set).
pub fn is_enabled() -> bool {
    earthdata_token_from_env().is_some()
}

/// Civil-from-days helper (Howard Hinnant's algorithm) — add `days` to a
/// `(year, month, day)` and return the resulting civil date. Used to decode
/// `VEG-DIST-DATE` (days since 2020-12-31) into a real calendar date without
/// pulling in `chrono`.
fn add_days_to_epoch(epoch: (i32, u32, u32), days: i64) -> (i32, u32, u32) {
    let z0 = days_from_civil(epoch.0, epoch.1, epoch.2) + days;
    civil_from_days(z0)
}

/// Days since 1970-01-01 for a civil date (Hinnant).
fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Civil date from days-since-1970 (Hinnant).
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    ((if m <= 2 { y + 1 } else { y }) as i32, m, d)
}

/// Fetch + decode the OPERA DIST-ALERT pixel at `(lat, lng)`.
///
/// Behaviour contract (the materializer relies on this):
/// - `auth_bearer == None` → `Err(NotEnabled)` (sign honest Absence; the
///   no-regression default).
/// - `auth_bearer == Some(_)` → the live STAC-discovery + EDL-authenticated
///   COG range-read. Staged: returns `Err(NotImplemented(..))` describing the
///   remaining wiring rather than fabricating a pixel.
pub async fn fetch_alert(
    _client: &reqwest::Client,
    _lat: f64,
    _lng: f64,
    auth_bearer: Option<&str>,
) -> Result<Option<OperaDistPixel>, OperaDistError> {
    if auth_bearer.is_none() {
        return Err(OperaDistError::NotEnabled);
    }
    Err(OperaDistError::NotImplemented(
        "OPERA DIST-ALERT token is configured but the live fetch is staged: \
         the remaining wiring is (1) a CMR-STAC search against \
         OPERA_L3_DIST-ALERT-HLS_V1_1 for the most-recent granule covering the \
         point (the STAC JSON itself is EDL-gated, so the Bearer token is sent \
         on the search too), (2) extract the VEG-DIST-STATUS + VEG-DIST-DATE COG \
         hrefs, (3) Bearer-authenticated cog range-read at the pixel. No pixel is \
         fabricated in the meantime."
            .to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn token_absent_is_not_enabled() {
        // Passing `None` (the no-token state) maps to NotEnabled — the
        // honest-Absence default the materializer relies on.
        let cli = reqwest::Client::new();
        let r = fetch_alert(&cli, 0.0, 0.0, None).await;
        assert!(matches!(r, Err(OperaDistError::NotEnabled)));
    }

    #[test]
    fn veg_dist_date_decodes_to_real_calendar_date() {
        // day 1 = 2021-01-01 (the day after the 2020-12-31 epoch).
        let p = OperaDistPixel {
            veg_dist_status: 6,
            veg_dist_date_days: 1,
        };
        assert_eq!(p.alert_date(), Some((2021, 1, 1)));
        assert_eq!(p.alert_date_iso().as_deref(), Some("2021-01-01"));
        assert!(p.is_confirmed());

        // A full year later: 2020-12-31 + 366 days (2024 is a leap year is
        // irrelevant; 2021 is not) = 2022-01-01.
        let p2 = OperaDistPixel {
            veg_dist_status: 3,
            veg_dist_date_days: 366,
        };
        assert_eq!(p2.alert_date(), Some((2022, 1, 1)));
        assert!(!p2.is_confirmed()); // status 3 = provisional

        // No alert.
        let none = OperaDistPixel {
            veg_dist_status: 0,
            veg_dist_date_days: 0,
        };
        assert_eq!(none.alert_date(), None);
        assert!(!none.is_confirmed());
    }

    #[test]
    fn epoch_roundtrip() {
        // days_from_civil ∘ civil_from_days is identity around the epoch.
        let e = days_from_civil(2020, 12, 31);
        assert_eq!(civil_from_days(e), (2020, 12, 31));
        assert_eq!(civil_from_days(e + 1), (2021, 1, 1));
    }
}
