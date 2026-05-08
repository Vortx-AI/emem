//! CHIRPS daily-precipitation connector — anonymous COG path.
//!
//! Source: **Funk, C., P. Peterson, M. Landsfeld, D. Pedreros,
//! J. Verdin, S. Shukla, G. Husak, J. Rowland, L. Harrison,
//! A. Hoell, J. Michaelsen (2015). *The climate hazards infrared
//! precipitation with stations — a new environmental record for
//! monitoring extremes*. Sci. Data 2, 150066. doi:10.1038/sdata.2015.66**.
//! Daily precipitation v2.0 at 0.05° (~5.5 km) — IR-based satellite
//! retrievals blended with rain-gauge stations. Reference standard for
//! tropical agronomy because ERA5 underestimates convective tails.
//!
//! Wire path: anonymous HTTPS Range over UCSB CHC's nginx mirror at
//! `https://data.chc.ucsb.edu/products/CHIRPS-2.0/global_daily/cogs/p05/{year}/chirps-v2.0.{year}.{month}.{day}.cog`.
//! Each daily COG is ~6.5 MB Float32 (mm/day), tiled, EPSG:4326.
//! No auth, no key. Verified live 2026-05-08 (Last-Modified after a
//! ~30-day publication latency).
//!
//! Coverage:
//! - Global ±50° latitude (the satellite IR retrieval breaks down
//!   poleward of that and the upstream rasters are clipped). Pixels
//!   outside that band MUST sign Absence with `out_of_bounds`, not a
//!   silent zero.
//! - Daily back to 1981-01-01. Two operational notes:
//!   * Final-quality data lags ~30 days. Calls inside the lag window
//!     surface as 404 from the upstream and we map to a structured
//!     `NotPublished` error so the caller can retry later.
//!   * Pixel value `-9999.0` is the documented NoData (open ocean +
//!     IR fill gaps). The sampler signs Absence on NoData rather than
//!     coercing to 0 mm — distinguishes "no rain here" from "no
//!     measurement here".

use std::time::Duration;

use reqwest::Client;

use crate::cog::{self, CogError};

/// UCSB CHC mirror — HTTPS, no auth, accept-ranges supported. Tested
/// against the 2.0 release; 2.1 (when it lands) will need a separate
/// scheme since the file naming + temporal coverage will differ.
const CHIRPS_BASE_URL: &str = "https://data.chc.ucsb.edu/products/CHIRPS-2.0/global_daily/cogs/p05";

/// Latitude bound — CHIRPS is clipped to ±50°. Pulled out as constants so
/// callers can reuse the bound for their own coverage tables.
pub const CHIRPS_NORTH_LAT: f64 = 50.0;
/// Latitude bound — see [`CHIRPS_NORTH_LAT`].
pub const CHIRPS_SOUTH_LAT: f64 = -50.0;

/// Documented NoData sentinel in CHIRPS Float32 rasters. Pixels with this
/// exact value are unmeasured (open ocean, polar IR gap). Compared with
/// `==` rather than tolerance because the upstream emits the literal
/// `-9999.0` as a Float32 bit pattern.
pub const CHIRPS_NODATA: f32 = -9999.0;

/// Earliest daily file the upstream serves. Used by the materializer's
/// `data_availability` advert and to short-circuit obviously-out-of-record
/// calls before paying the round-trip.
pub const CHIRPS_RECORD_START_YEAR: i32 = 1981;

/// One CHIRPS sample plus the metadata an attestation needs to cite it.
#[derive(Debug, Clone)]
pub struct ChirpsSample {
    /// Daily precipitation in millimetres. Always `>= 0.0` for a valid
    /// fetch; NoData pixels are surfaced as [`ChirpsError::NoData`] not
    /// as a zero value here.
    pub mm_per_day: f64,
    /// Calendar year the COG covers. Mirrors the input `year` so receipts
    /// can be replayed without re-deriving from the URL.
    pub year: i32,
    /// Calendar month (1..=12).
    pub month: u32,
    /// Calendar day-of-month (1..=31).
    pub day: u32,
    /// Fully-resolved upstream URL the responder hit. Surfaced on the
    /// signed Fact so a verifier can re-issue the same Range request.
    pub upstream_url: String,
}

/// Errors specific to the CHIRPS connector. Bubbled up at the
/// materializer boundary so the dispatcher can translate each variant
/// into the right Fact shape (Primary, Absence, or hard error).
#[derive(Debug, thiserror::Error)]
pub enum ChirpsError {
    /// Cell centre falls outside the dataset's latitude clip (±50°) or
    /// the longitude is non-finite. Materialiser MUST sign as Absence
    /// — the cell is genuinely outside CHIRPS' coverage.
    #[error("out_of_bounds: lat={lat:.6} lng={lng:.6} (CHIRPS coverage is ±50° latitude, ±180° longitude)")]
    OutOfBounds {
        /// Cell latitude in degrees.
        lat: f64,
        /// Cell longitude in degrees.
        lng: f64,
    },
    /// Date earlier than the start of record (1981-01-01) or
    /// structurally invalid (month/day out of range). Caller MUST sign
    /// Absence with `before_record` — distinct from "transient outage".
    #[error("before_record: {year}-{month:02}-{day:02} predates CHIRPS start of record ({CHIRPS_RECORD_START_YEAR}-01-01)")]
    BeforeRecord {
        /// Requested year.
        year: i32,
        /// Requested month.
        month: u32,
        /// Requested day.
        day: u32,
    },
    /// Date is plausible but the COG hasn't been published yet (final-
    /// quality CHIRPS lags ~30 days). 404 on a recent file falls here.
    /// Caller can retry later; signing as a transient error rather than
    /// Absence keeps the responder honest about provenance.
    #[error("not_published: COG {url} not yet published (CHIRPS final-quality has ~30-day lag)")]
    NotPublished {
        /// The 404'ing URL — preserved for diagnostics.
        url: String,
    },
    /// Pixel value was the documented NoData sentinel (-9999.0). The
    /// pixel is on a structural gap (open ocean cell aligned to land,
    /// polar IR coverage gap inside the ±50° band, etc.). Caller SHOULD
    /// sign Absence with `nodata`.
    #[error("nodata: CHIRPS pixel at ({lat:.6},{lng:.6}) on {year}-{month:02}-{day:02} carried sentinel -9999.0")]
    NoData {
        /// Cell latitude.
        lat: f64,
        /// Cell longitude.
        lng: f64,
        /// Year of the requested COG.
        year: i32,
        /// Month of the requested COG.
        month: u32,
        /// Day of the requested COG.
        day: u32,
    },
    /// HTTP / network failure other than 404.
    #[error("transport: {0}")]
    Transport(String),
    /// COG parse / decode failure.
    #[error("decode: {0}")]
    Decode(String),
}

impl ChirpsError {
    /// Translate a [`CogError`] into the right CHIRPS variant. 404 ->
    /// `NotPublished` (the file genuinely doesn't exist yet); all other
    /// transports stay as `Transport` for the dispatcher to retry.
    fn from_cog(e: CogError, url: &str) -> Self {
        let msg = e.to_string().to_lowercase();
        if msg.contains("status 404") || msg.contains("not found") {
            return ChirpsError::NotPublished { url: url.into() };
        }
        match e {
            CogError::Transport(s) => ChirpsError::Transport(s),
            other => ChirpsError::Decode(other.to_string()),
        }
    }
}

/// Compute the canonical CHIRPS daily-COG URL for `(year, month, day)`.
/// Pure helper so receipts can re-derive the URL deterministically.
///
/// The convention is `chirps-v2.0.YYYY.MM.DD.cog` with zero-padded MM/DD
/// — verified against the upstream directory listing.
pub fn url_for(year: i32, month: u32, day: u32) -> String {
    format!("{CHIRPS_BASE_URL}/{year}/chirps-v2.0.{year}.{month:02}.{day:02}.cog")
}

/// Validate calendar (year, month, day). Returns `Some(date)` for a
/// real Gregorian date in CHIRPS' record window, `None` otherwise.
/// Overflow-safe: handles leap years via Hinnant civil-day arithmetic
/// (no allocations, no chrono dep on the hot path).
fn valid_date(year: i32, month: u32, day: u32) -> Option<()> {
    if !(1..=12).contains(&month) {
        return None;
    }
    let dim = days_in_month(year, month)?;
    if !(1..=dim).contains(&day) {
        return None;
    }
    Some(())
}

/// Days in a given month, handling Gregorian leap years.
fn days_in_month(year: i32, month: u32) -> Option<u32> {
    Some(match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => return None,
    })
}

fn is_leap_year(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0)
}

/// Read one pixel from the CHIRPS daily COG at `(lat, lng)` for the
/// given calendar date. Returns mm/day plus the upstream URL hit.
///
/// Coverage rules — encoded as variants of [`ChirpsError`]:
/// - Latitude outside ±50° → [`ChirpsError::OutOfBounds`].
/// - Date before 1981-01-01 → [`ChirpsError::BeforeRecord`].
/// - File not yet published (final-quality 30-day lag) →
///   [`ChirpsError::NotPublished`].
/// - Pixel == -9999.0 sentinel → [`ChirpsError::NoData`].
///
/// `timeout` bounds the entire fetch (IFD head + tile range read);
/// the caller's HTTP client is used as-is so connection pooling is
/// preserved across recalls.
pub async fn fetch_chirps_daily(
    lat: f64,
    lng: f64,
    year: i32,
    month: u32,
    day: u32,
    timeout: Duration,
) -> Result<ChirpsSample, ChirpsError> {
    if !lat.is_finite() || !lng.is_finite() {
        return Err(ChirpsError::OutOfBounds { lat, lng });
    }
    if !(CHIRPS_SOUTH_LAT..=CHIRPS_NORTH_LAT).contains(&lat) || !(-180.0..=180.0).contains(&lng) {
        return Err(ChirpsError::OutOfBounds { lat, lng });
    }
    if year < CHIRPS_RECORD_START_YEAR {
        return Err(ChirpsError::BeforeRecord { year, month, day });
    }
    if valid_date(year, month, day).is_none() {
        // Treat structurally invalid dates as before-record so callers
        // get a clear "this date is bogus" rather than a stray 404 from
        // the upstream.
        return Err(ChirpsError::BeforeRecord { year, month, day });
    }
    let url = url_for(year, month, day);

    // Local client with the caller's timeout — the shared `s2_http_client`
    // in api-rest carries a 90 s default which is too long for chirps
    // (one Range read; 8-15 s is typical).
    let cli = Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| ChirpsError::Transport(format!("client build: {e}")))?;

    let profile = cog::open_profile(&cli, &url)
        .await
        .map_err(|e| ChirpsError::from_cog(e, &url))?;

    // CHIRPS rasters are EPSG:4326 (geographic). The COG sampler
    // expects world coordinates in the COG's CRS — for 4326 that's
    // (lng, lat) directly.
    let raw = cog::sample_pixel(&cli, &url, &profile, lng, lat)
        .await
        .map_err(|e| ChirpsError::from_cog(e, &url))?;

    // The Float32 source value lands in `raw` as f64 already. NoData
    // is the literal sentinel `-9999.0` per the dataset spec; compare
    // as f32 to match the upstream bit pattern exactly.
    if (raw as f32) == CHIRPS_NODATA {
        return Err(ChirpsError::NoData {
            lat,
            lng,
            year,
            month,
            day,
        });
    }
    if !raw.is_finite() || raw < 0.0 {
        return Err(ChirpsError::Decode(format!(
            "implausible CHIRPS value {raw} at ({lat:.6},{lng:.6}) {year}-{month:02}-{day:02} (expected mm/day >= 0 or -9999 sentinel)"
        )));
    }
    Ok(ChirpsSample {
        mm_per_day: raw,
        year,
        month,
        day,
        upstream_url: url,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `url_for` reproduces the documented filename pattern for known
    /// dates, including leading zeros on month + day. Pinned against
    /// the upstream directory listing convention.
    #[test]
    fn url_for_zero_pads_and_uses_v2_0_naming() {
        // Mumbai monsoon reference date — used in the live test plan.
        assert_eq!(
            url_for(2023, 7, 15),
            "https://data.chc.ucsb.edu/products/CHIRPS-2.0/global_daily/cogs/p05/2023/chirps-v2.0.2023.07.15.cog"
        );
        // First day of record.
        assert_eq!(
            url_for(1981, 1, 1),
            "https://data.chc.ucsb.edu/products/CHIRPS-2.0/global_daily/cogs/p05/1981/chirps-v2.0.1981.01.01.cog"
        );
        // Year boundary, single-digit day.
        assert_eq!(
            url_for(2000, 12, 5),
            "https://data.chc.ucsb.edu/products/CHIRPS-2.0/global_daily/cogs/p05/2000/chirps-v2.0.2000.12.05.cog"
        );
    }

    /// Latitude bounds: ±50° is INclusive; just-past is OUT. Surfaces
    /// the structured `OutOfBounds` variant — Antarctica + high-Arctic
    /// recalls must Absence-out rather than silently 0 mm.
    #[tokio::test]
    async fn out_of_bounds_above_50n() {
        let err = fetch_chirps_daily(60.0, 0.0, 2023, 7, 15, Duration::from_secs(5))
            .await
            .unwrap_err();
        assert!(matches!(err, ChirpsError::OutOfBounds { .. }), "got {err}");
    }

    /// South of -50° is also out of coverage.
    #[tokio::test]
    async fn out_of_bounds_below_50s() {
        let err = fetch_chirps_daily(-75.0, 0.0, 2023, 7, 15, Duration::from_secs(5))
            .await
            .unwrap_err();
        assert!(matches!(err, ChirpsError::OutOfBounds { .. }), "got {err}");
    }

    /// Pre-1981 dates short-circuit before any HTTP — the responder
    /// MUST sign `before_record` Absence rather than gambling on the
    /// upstream returning 404.
    #[tokio::test]
    async fn before_record_short_circuits() {
        let err = fetch_chirps_daily(0.0, 0.0, 1980, 12, 31, Duration::from_secs(5))
            .await
            .unwrap_err();
        assert!(matches!(err, ChirpsError::BeforeRecord { .. }), "got {err}");
    }

    /// Structurally invalid dates (Feb 30, month=13) collapse into the
    /// same `BeforeRecord` arm — agents shouldn't be able to confuse a
    /// bogus date with a transient upstream outage.
    #[tokio::test]
    async fn invalid_dates_short_circuit() {
        let err = fetch_chirps_daily(0.0, 0.0, 2023, 2, 30, Duration::from_secs(5))
            .await
            .unwrap_err();
        assert!(matches!(err, ChirpsError::BeforeRecord { .. }), "got {err}");
        let err = fetch_chirps_daily(0.0, 0.0, 2023, 13, 1, Duration::from_secs(5))
            .await
            .unwrap_err();
        assert!(matches!(err, ChirpsError::BeforeRecord { .. }), "got {err}");
    }

    /// `from_cog` distinguishes 404 (file not yet published) from
    /// other transports. Pins the dispatcher's contract.
    #[test]
    fn from_cog_404_maps_to_not_published() {
        let url = "https://data.chc.ucsb.edu/products/CHIRPS-2.0/global_daily/cogs/p05/2099/chirps-v2.0.2099.01.01.cog";
        let cog_404 =
            CogError::Transport(format!("status 404 Not Found for range 0-65535 on {url}"));
        let err = ChirpsError::from_cog(cog_404, url);
        match &err {
            ChirpsError::NotPublished { url: u } => assert_eq!(u, url),
            other => panic!("expected NotPublished, got {other:?}"),
        }
        // Non-404 transports stay Transport so the dispatcher can retry.
        let cog_503 = CogError::Transport("status 503 Service Unavailable".into());
        let err = ChirpsError::from_cog(cog_503, url);
        assert!(matches!(err, ChirpsError::Transport(_)), "got {err:?}");
        // Decode failures keep their identity.
        let cog_decode = CogError::BadMagic(0xdead);
        let err = ChirpsError::from_cog(cog_decode, url);
        assert!(matches!(err, ChirpsError::Decode(_)), "got {err:?}");
    }

    /// Leap-year handling pinned: 2020-02-29 is valid, 2021-02-29 isn't.
    #[test]
    fn leap_year_validation() {
        assert!(valid_date(2020, 2, 29).is_some());
        assert!(valid_date(2021, 2, 29).is_none());
        assert!(valid_date(2000, 2, 29).is_some()); // div-by-400
        assert!(valid_date(1900, 2, 29).is_none()); // div-by-100 not 400
        assert!(valid_date(2023, 4, 31).is_none()); // April has 30
    }
}
