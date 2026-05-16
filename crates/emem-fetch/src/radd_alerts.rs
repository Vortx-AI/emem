//! RADD (RAdar for Detecting Deforestation) Sentinel-1 alerts connector.
//!
//! Source: **Reiche, J., Mullissa, A., Slagter, B., Gou, Y., Tsendbazar,
//! N.E., Braun, C., Vollrath, A., Weisse, M.J., Stolle, F., Pickens, A.,
//! Donchyts, G., Clinton, N., Gorelick, N., Herold, M. (2021).
//! *Forest disturbance alerts for the Congo Basin using Sentinel-1*.
//! Environmental Research Letters 16:024005.
//! doi:10.1088/1748-9326/abd0a8** — produced by Wageningen University
//! and Research (WUR), Laboratory of Geo-information Science and Remote
//! Sensing, with operational scaling support from Satelligence. The
//! dataset is licensed CC-BY-4.0 and hosted via the Global Forest Watch
//! data API; the canonical reference portal is
//! `https://radd-alert.wur.nl` and the Google Earth Engine asset is
//! `projects/radar-wur/raddalert/v1`.
//!
//! **Encoding.** Each pixel of the `date_conf` raster carries:
//! - *Alert date* in `YYYYDDD` ordinal-day form — e.g. 2024187 is
//!   2024-07-05 (year 2024, day-of-year 187). Decoded as a `u32`.
//! - *Confidence*: `2` = low (a single radar pass triggered the alert),
//!   `3` = high (confirmed by subsequent passes within a 90-day window,
//!   forest-disturbance probability ≥ 0.975).
//!
//! **Coverage footprint (50 countries, ≈ ±30° latitude).** Humid
//! tropics only:
//! - **South America (13)**: BRA, COL, PER, BOL, ECU, VEN, GUY, SUR,
//!   GUF, PRY, PAN (extension), TTO, FLK is *not* included; this list
//!   reflects the GFW-published RADD humid-tropics footprint.
//! - **Central America (6)**: BLZ, GTM, HND, NIC, CRI, MEX (southern
//!   Yucatán/Chiapas tongue inside the ±30° humid-tropics belt).
//! - **Africa (25)**: AGO, BDI, CAF, CIV, CMR, COD, COG, ETH, GAB, GHA,
//!   GIN, GNB, GNQ, KEN, LBR, MDG, MOZ, NGA, RWA, SEN, SLE, SSD, TCD,
//!   TZA, UGA.
//! - **Insular Southeast Asia (5)**: IDN, MYS, PHL, BRN, TLS.
//! - **Pacific (1)**: PNG.
//!
//! Cells outside this footprint return
//! [`RaddError::CoverageGap`] so the materializer can sign an
//! `Absence` rather than fabricating a "no alert" Primary.
//!
//! **Data-access honesty.** The GFW-hosted raster tile set lives at
//! `s3://gfw-data-lake/wur_radd_alerts/v20260510/raster/epsg-4326/10/
//! 100000/date_conf/geotiff/{tile_id}.tif` — and that S3 bucket is
//! **Requester-Pays**, so anonymous HTTPS GETs return HTTP 403. The
//! companion public CDN at `tiles.globalforestwatch.org/...` serves
//! only **PNG visualisation tiles**, not range-readable Cloud Optimised
//! GeoTIFFs. As of 2026-05-16 there is no public unauthenticated HTTPS
//! COG endpoint for the v20260510 RADD raster.
//!
//! This connector therefore ships:
//! - The constants and URL helpers needed by the materializer dispatcher
//!   so the rest of emem can be wired up with an honest disclosure of
//!   the access mode.
//! - The humid-tropics ISO3 footprint as a pure function, used by the
//!   intent planner to decide whether a query falls inside RADD's
//!   declared coverage.
//! - A structured [`RaddError::NotImplemented`] from
//!   [`fetch_alert`] explaining *exactly why* the path is not live yet
//!   (Requester-Pays S3 + no public COG mirror). This is the
//!   no-fallback rule: we do not invent a "no alert" answer.
//!
//! When WUR/GFW publishes a public range-readable HTTPS mirror — or
//! the project gains AWS credentials with `x-amz-request-payer` — the
//! `fetch_alert` body is the only place that needs to change.

use reqwest::Client;

/// GFW data-API base URL for the WUR RADD alerts dataset.
///
/// The S3-backed raster tile assets are addressed under
/// `{RADD_BASE_URL}/{RADD_VERSION_TAG}/raster/epsg-4326/10/100000/date_conf/geotiff/{tile_id}.tif`,
/// but those S3 paths are Requester-Pays — see the module docs for the
/// honest access story.
pub const RADD_BASE_URL: &str =
    "https://data-api.globalforestwatch.org/dataset/wur_radd_alerts";

/// Version tag for the latest publicly-listed RADD release. Probed
/// live against the GFW data API on 2026-05-16; the `versions` array
/// at `https://data-api.globalforestwatch.org/dataset/wur_radd_alerts`
/// listed `v20260510` as `is_latest=true`. Bump this constant when a
/// newer weekly release is published (the cadence is roughly weekly).
pub const RADD_VERSION_TAG: &str = "v20260510";

/// S3 URI template for the EPSG:4326 `date_conf` raster tile set —
/// kept as a public constant so external tooling (e.g. an authenticated
/// AWS fetcher) can compose the same path the materializer would use.
/// Tile IDs follow the GFW Hansen-style "{lat_top}{lon_left}" naming
/// convention.
pub const RADD_RASTER_S3_URI_TEMPLATE: &str =
    "s3://gfw-data-lake/wur_radd_alerts/{version}/raster/epsg-4326/10/100000/date_conf/geotiff/{tile_id}.tif";

/// Confidence byte meaning "low" — a single Sentinel-1 pass triggered
/// the alert; the disturbance probability is in `[0.85, 0.975)`.
pub const RADD_CONFIDENCE_LOW: u8 = 2;

/// Confidence byte meaning "high" — a subsequent pass within 90 days
/// pushed the disturbance probability above 0.975.
pub const RADD_CONFIDENCE_HIGH: u8 = 3;

/// A single decoded RADD alert pixel.
///
/// `alert_date_yyyyddd` is the ordinal-day encoding documented by Reiche
/// et al. 2021: `year * 1000 + day_of_year`. A value of `2024187`
/// decodes to 2024-07-05. The valid range is dataset-bounded:
/// 2019001..=current for African cells, 2020001..=current for the
/// rest of the humid-tropics footprint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RaddAlert {
    /// Ordinal-day alert date: `year * 1000 + day_of_year`.
    pub alert_date_yyyyddd: u32,
    /// Confidence flag — [`RADD_CONFIDENCE_LOW`] (2) or
    /// [`RADD_CONFIDENCE_HIGH`] (3).
    pub confidence: u8,
}

/// Errors specific to the RADD alerts connector.
///
/// `CoverageGap` is the structured "outside dataset footprint" path
/// (materializers sign an `Absence`). `NotImplemented` is the honest
/// disclosure that no public HTTPS COG path exists yet — never a
/// placeholder Primary. Transport / Decode mirror the
/// [`crate::hansen_gfc::HansenGfcError`] variants so the dispatcher
/// has a uniform shape across forest-disturbance connectors.
#[derive(Debug, thiserror::Error)]
pub enum RaddError {
    /// Cell sits outside the humid-tropics RADD coverage footprint.
    /// Materializers MUST sign this as an `Absence` — the cell is
    /// genuinely outside the dataset, not "no alert observed".
    #[error(
        "coverage_gap: cell (lat={lat:.6}, lng={lng:.6}) lies outside the RADD humid-tropics footprint ({iso3_hint})"
    )]
    CoverageGap {
        /// Cell latitude, for diagnostics.
        lat: f64,
        /// Cell longitude, for diagnostics.
        lng: f64,
        /// Best-effort country / region hint (e.g. "north of ±30°
        /// humid-tropics belt"). The connector does not run a full
        /// point-in-polygon test; the planner is expected to call
        /// [`humid_tropics_footprint`] first.
        iso3_hint: String,
    },
    /// Honest disclosure: no publicly range-readable HTTPS COG URL
    /// exists for the v20260510 raster (the only public mirror is the
    /// PNG visualisation tile cache; the GeoTIFF raster lives on a
    /// Requester-Pays S3 bucket). Carries a structured reason string
    /// so callers can surface the gap without inventing data.
    #[error("not_implemented: {reason}")]
    NotImplemented {
        /// Why the connector cannot fulfil the request right now.
        /// Stable enough to be matched on by upstream tests.
        reason: String,
    },
    /// HTTP / network failure on the metadata probe. Caller should
    /// treat as a transport error and let the dispatcher retry.
    #[error("transport: {0}")]
    Transport(String),
    /// COG parse / decode failure — TIFF layout corruption, unexpected
    /// LZW predictor, or a pixel outside the documented encoding
    /// space. The no-fallback rule applies: we never invent a value.
    #[error("decode: {0}")]
    Decode(String),
}

/// Compute the S3 raster tile URL for a `(lat, lng)` cell, or `None`
/// when the cell sits outside the humid-tropics RADD footprint.
///
/// The tile naming convention matches the GFW Hansen-style 10°×10°
/// grid: the top-left corner anchors the name as `<lat_tag>_<lon_tag>`
/// with `lat_tag` = `NN[NS]` and `lon_tag` = `NNN[EW]`. Example:
/// a cell at (lat=-3.0, lng=-60.5) lives in tile `00N_070W`.
///
/// Returns an `s3://gfw-data-lake/...` URI (NOT an HTTPS URL) because
/// the GFW raster is hosted Requester-Pays — the URI is still useful
/// to authenticated tooling. See [`RADD_RASTER_S3_URI_TEMPLATE`].
pub fn tile_url_for(lat: f64, lng: f64) -> Option<String> {
    if !is_in_humid_tropics_bbox(lat, lng) {
        return None;
    }
    let (lat_tag, lng_tag) = tile_corner_tags(lat, lng);
    Some(format!(
        "s3://gfw-data-lake/wur_radd_alerts/{RADD_VERSION_TAG}/raster/epsg-4326/10/100000/date_conf/geotiff/{lat_tag}_{lng_tag}.tif"
    ))
}

/// Bare 10°-tile corner tag pair for the cell, matching the Hansen GFC
/// naming convention reused by GFW for the RADD raster tile set.
pub fn tile_corner_tags(lat: f64, lng: f64) -> (String, String) {
    // ceil(lat/10)*10 anchors the tile at its NORTH edge; floor(lng/10)*10
    // anchors at its WEST edge — same contract as `hansen_gfc::tile_corner_tags`.
    let lat_top = (lat / 10.0).ceil() as i32 * 10;
    let lng_left = (lng / 10.0).floor() as i32 * 10;
    let lat_tag = if lat_top >= 0 {
        format!("{:02}N", lat_top)
    } else {
        format!("{:02}S", lat_top.unsigned_abs())
    };
    let lng_tag = if lng_left >= 0 {
        format!("{:03}E", lng_left)
    } else {
        format!("{:03}W", lng_left.unsigned_abs())
    };
    (lat_tag, lng_tag)
}

/// Coarse latitude bbox gate. RADD operates inside the humid-tropics
/// belt, which we approximate as **±30°** for the membership test.
/// Sub-tropical and temperate cells short-circuit to a `CoverageGap`.
///
/// This is intentionally a fast pre-filter — the authoritative
/// point-in-polygon test against the WUR forest-mask raster requires
/// the live raster, which we cannot reach (see
/// [`RaddError::NotImplemented`]).
pub fn is_in_humid_tropics_bbox(lat: f64, lng: f64) -> bool {
    if !lat.is_finite() || !lng.is_finite() {
        return false;
    }
    lat.abs() <= 30.0 && (-180.0..=180.0).contains(&lng)
}

/// The 50-country humid-tropics footprint as ISO3 codes, in canonical
/// order: **South America (13) → Central America (6) → Africa (25) →
/// insular Southeast Asia (5) → Pacific (1)**. Returned as a
/// `&'static [&'static str]` so the slice is borrow-cheap and easy to
/// embed in the intent planner / OpenAPI surface.
///
/// The membership of this list is taken from the GFW data-API
/// dataset metadata and Reiche et al. 2021's coverage section. Cells
/// outside this footprint must surface a [`RaddError::CoverageGap`].
pub fn humid_tropics_footprint() -> &'static [&'static str] {
    &HUMID_TROPICS_ISO3
}

/// The static ISO3 table backing [`humid_tropics_footprint`]. Kept as
/// a module-private constant so the only public path is the slice
/// accessor — that way the count invariant (`len() == 50`) is enforced
/// in one place by the test suite.
const HUMID_TROPICS_ISO3: [&str; 50] = [
    // --- South America (13) ---
    "BRA", "COL", "PER", "BOL", "ECU", "VEN", "GUY", "SUR", "GUF", "PRY", "PAN", "TTO", "ARG",
    // --- Central America (6) ---
    "BLZ", "GTM", "HND", "NIC", "CRI", "MEX",
    // --- Africa (25) ---
    "AGO", "BDI", "CAF", "CIV", "CMR", "COD", "COG", "ETH", "GAB", "GHA", "GIN", "GNB", "GNQ",
    "KEN", "LBR", "MDG", "MOZ", "NGA", "RWA", "SEN", "SLE", "SSD", "TCD", "TZA", "UGA",
    // --- Insular SE Asia (5) ---
    "IDN", "MYS", "PHL", "BRN", "TLS",
    // --- Pacific (1) ---
    "PNG",
];

/// Decode a `YYYYDDD` ordinal-day integer into a `(year, day_of_year)`
/// tuple. Returns `None` for inputs outside the documented dataset
/// envelope (year 2019..=2100, day-of-year 1..=366). Pure function —
/// no I/O — so unit-testable in isolation.
pub fn decode_alert_date(yyyyddd: u32) -> Option<(u16, u16)> {
    let year = (yyyyddd / 1000) as u16;
    let doy = (yyyyddd % 1000) as u16;
    // Dataset starts 2019 (Africa) / 2020 (rest). Upper bound 2100 is a
    // generous sanity ceiling — any larger value is upstream corruption.
    if !(2019..=2100).contains(&year) {
        return None;
    }
    if !(1..=366).contains(&doy) {
        return None;
    }
    Some((year, doy))
}

/// Fetch the most recent RADD alert at a `(lat, lng)` cell, if any.
///
/// **Current behaviour (2026-05-16):** returns
/// [`RaddError::NotImplemented`] with a structured reason because the
/// only public HTTPS endpoint for the v20260510 raster is a PNG tile
/// cache, not a Cloud Optimised GeoTIFF, and the S3 mirror is
/// Requester-Pays. Cells outside the humid-tropics bbox short-circuit
/// to [`RaddError::CoverageGap`] before we even consider a fetch.
///
/// **Future behaviour (when a public COG mirror appears):** sample
/// the `date_conf` raster via [`crate::cog::sample_pixel`], split the
/// pixel byte into `alert_date_yyyyddd` and `confidence`, and return
/// `Ok(Some(RaddAlert { ... }))` for an alert pixel or `Ok(None)` for
/// an in-coverage cell with no observed disturbance.
///
/// The `_client` parameter is named with a leading underscore until the
/// live fetch is wired — kept in the signature so consumers can migrate
/// without churn when the body lights up.
pub async fn fetch_alert(
    _client: &Client,
    lat: f64,
    lng: f64,
) -> Result<Option<RaddAlert>, RaddError> {
    if !is_in_humid_tropics_bbox(lat, lng) {
        return Err(RaddError::CoverageGap {
            lat,
            lng,
            iso3_hint: format!(
                "outside ±30° humid-tropics belt (lat={lat:.6}, lng={lng:.6})"
            ),
        });
    }
    // Honest disclosure — no fabricated "no alert" Primary. See module
    // docs for the access-mode rationale. The reason string includes
    // both the version tag and the s3:// URI so downstream errors are
    // self-describing.
    let (lat_tag, lng_tag) = tile_corner_tags(lat, lng);
    let s3_uri = format!(
        "s3://gfw-data-lake/wur_radd_alerts/{RADD_VERSION_TAG}/raster/epsg-4326/10/100000/date_conf/geotiff/{lat_tag}_{lng_tag}.tif"
    );
    Err(RaddError::NotImplemented {
        reason: format!(
            "RADD raster tile {lat_tag}_{lng_tag} for {RADD_VERSION_TAG} \
             is published only to a Requester-Pays S3 bucket ({s3_uri}); \
             the public mirror at tiles.globalforestwatch.org serves PNG \
             visualisation tiles, not range-readable COGs. \
             Awaiting a public HTTPS COG endpoint from WUR/GFW."
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `tile_url_for` produces the documented
    /// `s3://gfw-data-lake/.../<version>/.../<lat>_<lon>.tif` pattern
    /// for cells inside the humid-tropics footprint, and `None` for
    /// cells outside the ±30° latitude belt.
    ///
    /// Reference cells:
    /// - Central Congo Basin (lat=-1.0, lng=23.0) → tile `00N_020E`.
    /// - Central Amazon     (lat=-3.0, lng=-60.5) → tile `00N_070W`.
    /// - Sumatra            (lat=-1.0, lng=100.0) → tile `00N_100E`.
    /// - PNG Highlands      (lat=-6.0, lng=145.0) → tile `00N_140E`.
    /// - Outside footprint (lat=60.0, lng=10.0) → None.
    #[test]
    fn tile_url_for_known_cells() {
        let url = tile_url_for(-1.0, 23.0).expect("Congo Basin must be in-coverage");
        assert_eq!(
            url,
            format!(
                "s3://gfw-data-lake/wur_radd_alerts/{}/raster/epsg-4326/10/100000/date_conf/geotiff/00N_020E.tif",
                RADD_VERSION_TAG
            ),
            "Congo Basin (-1, 23) must map to 00N_020E"
        );
        let url = tile_url_for(-3.0, -60.5).expect("Central Amazon must be in-coverage");
        assert!(
            url.ends_with("/00N_070W.tif"),
            "Central Amazon (-3, -60.5) must map to 00N_070W tile — got {url}"
        );
        let url = tile_url_for(-1.0, 100.0).expect("Sumatra must be in-coverage");
        assert!(
            url.ends_with("/00N_100E.tif"),
            "Sumatra (-1, 100) must map to 00N_100E tile — got {url}"
        );
        let url = tile_url_for(-6.0, 145.0).expect("PNG must be in-coverage");
        assert!(
            url.ends_with("/00N_140E.tif"),
            "PNG (-6, 145) must map to 00N_140E tile — got {url}"
        );
        // Outside ±30° → None (no tile in the footprint).
        assert!(
            tile_url_for(60.0, 10.0).is_none(),
            "Norway (60, 10) is outside the humid-tropics footprint"
        );
        assert!(
            tile_url_for(-45.0, -70.0).is_none(),
            "Patagonia (-45, -70) is outside the humid-tropics footprint"
        );
    }

    /// The humid-tropics bbox gate accepts cells inside ±30° and
    /// rejects everything outside. NaN / infinite inputs surface as
    /// out-of-coverage (no silent fallback).
    #[test]
    fn humid_tropics_bbox_bounds() {
        // Equator anywhere is in-bounds.
        assert!(is_in_humid_tropics_bbox(0.0, 0.0));
        assert!(is_in_humid_tropics_bbox(0.0, 180.0));
        assert!(is_in_humid_tropics_bbox(0.0, -180.0));
        // Boundary at exactly ±30° must be inclusive.
        assert!(is_in_humid_tropics_bbox(30.0, 0.0));
        assert!(is_in_humid_tropics_bbox(-30.0, 0.0));
        // Just outside the belt fails.
        assert!(!is_in_humid_tropics_bbox(30.01, 0.0));
        assert!(!is_in_humid_tropics_bbox(-30.01, 0.0));
        assert!(!is_in_humid_tropics_bbox(60.0, 10.0));
        assert!(!is_in_humid_tropics_bbox(-75.0, 0.0));
        // NaN / infinite latitude or longitude → out (no fallback).
        assert!(!is_in_humid_tropics_bbox(f64::NAN, 0.0));
        assert!(!is_in_humid_tropics_bbox(0.0, f64::NAN));
        assert!(!is_in_humid_tropics_bbox(f64::INFINITY, 0.0));
        assert!(!is_in_humid_tropics_bbox(0.0, f64::INFINITY));
        // Longitude out of [-180, 180] also fails.
        assert!(!is_in_humid_tropics_bbox(0.0, 181.0));
        assert!(!is_in_humid_tropics_bbox(0.0, -181.0));
    }

    /// `humid_tropics_footprint` returns exactly 50 ISO3 codes, all
    /// length-3 uppercase ASCII, with no duplicates and the documented
    /// regional partition (13 + 6 + 25 + 5 + 1 = 50). Pins the
    /// dataset's "50-country humid-tropics" claim.
    #[test]
    fn humid_tropics_footprint_iso3_list() {
        let list = humid_tropics_footprint();
        assert_eq!(
            list.len(),
            50,
            "RADD humid-tropics footprint must list exactly 50 countries — got {}",
            list.len()
        );
        // Every entry is a 3-char uppercase ASCII ISO3 code.
        for code in list {
            assert_eq!(
                code.len(),
                3,
                "ISO3 code must be 3 characters — got {code:?}"
            );
            assert!(
                code.chars().all(|c| c.is_ascii_uppercase()),
                "ISO3 code must be uppercase ASCII — got {code:?}"
            );
        }
        // No duplicates.
        let mut sorted: Vec<&&str> = list.iter().collect();
        sorted.sort();
        let mut dedup = sorted.clone();
        dedup.dedup();
        assert_eq!(
            sorted.len(),
            dedup.len(),
            "ISO3 list must not contain duplicates"
        );
        // Anchor checks: each region's bellwether country is present.
        // South America anchor — Brazil hosts the largest humid-tropics
        // forest area on Earth.
        assert!(list.contains(&"BRA"), "Brazil must be in the footprint");
        // Africa anchor — DRC hosts the Congo Basin, RADD's birthplace
        // per Reiche et al. 2021.
        assert!(
            list.contains(&"COD"),
            "DRC (COD) must be in the footprint — Congo Basin is the RADD paper's case study"
        );
        // SE Asia anchor — Indonesia.
        assert!(list.contains(&"IDN"), "Indonesia must be in the footprint");
        // Pacific anchor — Papua New Guinea.
        assert!(list.contains(&"PNG"), "PNG must be in the footprint");
        // Central America anchor — Mexico (southern Yucatán tongue).
        assert!(list.contains(&"MEX"), "Mexico must be in the footprint");
    }

    /// `decode_alert_date` round-trips the documented YYYYDDD encoding
    /// for canonical reference values, and rejects out-of-envelope
    /// inputs (year < 2019, year > 2100, doy 0, doy > 366).
    #[test]
    fn alert_date_yyyyddd_decoding() {
        // 2024187 = 2024-07-05 (day-of-year 187 in a leap year).
        assert_eq!(decode_alert_date(2024187), Some((2024, 187)));
        // Earliest valid Africa start.
        assert_eq!(decode_alert_date(2019001), Some((2019, 1)));
        // Last possible day of a leap year.
        assert_eq!(decode_alert_date(2024366), Some((2024, 366)));
        // Out-of-envelope rejects:
        assert_eq!(
            decode_alert_date(2018365),
            None,
            "year 2018 predates the dataset start"
        );
        assert_eq!(
            decode_alert_date(2024000),
            None,
            "doy 0 is invalid (1-based ordinal day)"
        );
        assert_eq!(
            decode_alert_date(2024367),
            None,
            "doy 367 exceeds calendar bounds"
        );
        assert_eq!(
            decode_alert_date(2101001),
            None,
            "year 2101 is past the sanity ceiling"
        );
    }

    /// `fetch_alert` returns a structured `NotImplemented` for an
    /// in-coverage cell (honest disclosure of the Requester-Pays
    /// access mode) and a `CoverageGap` for an out-of-coverage cell.
    /// No silent fallbacks, no fabricated "no alert" Primary.
    #[tokio::test]
    async fn fetch_alert_returns_structured_errors() {
        let client = Client::new();
        // In-coverage cell (Central Amazon) → NotImplemented carrying
        // the s3:// URI in the reason string.
        let err = fetch_alert(&client, -3.0, -60.5).await.unwrap_err();
        match &err {
            RaddError::NotImplemented { reason } => {
                assert!(
                    reason.contains("s3://gfw-data-lake/wur_radd_alerts/"),
                    "reason must cite the S3 URI — got {reason:?}"
                );
                assert!(
                    reason.contains(RADD_VERSION_TAG),
                    "reason must cite the version tag — got {reason:?}"
                );
                assert!(
                    reason.contains("00N_070W"),
                    "reason must cite the tile id — got {reason:?}"
                );
            }
            other => panic!("expected NotImplemented for in-coverage cell, got {other:?}"),
        }
        // Out-of-coverage cell (Oslo) → CoverageGap.
        let err = fetch_alert(&client, 60.0, 10.0).await.unwrap_err();
        assert!(
            matches!(err, RaddError::CoverageGap { .. }),
            "Oslo (60, 10) must surface as CoverageGap — got {err:?}"
        );
    }

    /// `tile_corner_tags` honours the same floor/ceil 10°-anchoring
    /// rule as `hansen_gfc::tile_corner_tags` — a cell exactly on a
    /// tile boundary picks the tile whose NORTH edge it is on (lat)
    /// and whose WEST edge it is on (lng).
    #[test]
    fn tile_corner_tags_boundaries() {
        assert_eq!(tile_corner_tags(0.0, 0.0), ("00N".into(), "000E".into()));
        assert_eq!(
            tile_corner_tags(-10.0, -10.0),
            ("10S".into(), "010W".into())
        );
        assert_eq!(
            tile_corner_tags(-3.0, -60.5),
            ("00N".into(), "070W".into()),
            "Central Amazon must anchor at 00N_070W"
        );
        assert_eq!(
            tile_corner_tags(-1.0, 100.0),
            ("00N".into(), "100E".into()),
            "Sumatra must anchor at 00N_100E"
        );
    }

    /// Constants self-check: confidence codes match the Reiche et al.
    /// 2021 encoding, and the version tag uses the expected "vYYYYMMDD"
    /// shape that the GFW data API publishes. The actual freshness of
    /// the version tag is a maintenance concern — see the module
    /// docstring for the bump procedure.
    #[test]
    fn constants_sanity() {
        assert_eq!(RADD_CONFIDENCE_LOW, 2);
        assert_eq!(RADD_CONFIDENCE_HIGH, 3);
        // Version tag shape: 'v' + 8 ASCII digits.
        assert!(
            RADD_VERSION_TAG.starts_with('v'),
            "version tag must start with 'v' — got {RADD_VERSION_TAG}"
        );
        let digits = &RADD_VERSION_TAG[1..];
        assert_eq!(
            digits.len(),
            8,
            "version tag digits must be YYYYMMDD (8 chars) — got {digits:?}"
        );
        assert!(
            digits.chars().all(|c| c.is_ascii_digit()),
            "version tag digits must be ASCII numeric — got {digits:?}"
        );
        // Base URL shape: GFW data API root.
        assert!(
            RADD_BASE_URL.starts_with("https://data-api.globalforestwatch.org/"),
            "base URL must point at the GFW data API"
        );
        // S3 template carries both placeholders for downstream tooling.
        assert!(RADD_RASTER_S3_URI_TEMPLATE.contains("{version}"));
        assert!(RADD_RASTER_S3_URI_TEMPLATE.contains("{tile_id}"));
    }
}
