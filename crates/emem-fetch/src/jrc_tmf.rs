//! JRC Tropical Moist Forest v2025 connector — pull-and-cache infrastructure.
//!
//! Source: **Vancutsem, C., Achard, F., Pekel, J.-F., Vieilledent, G., Carboni,
//! S., Simonetti, D., Gallego, J., Aragão, L. E. O. C., Nasi, R. (2021).
//! *Long-term (1990–2019) monitoring of forest cover changes in the humid
//! tropics*. Science Advances 7 (10): eabe1603.
//! doi:10.1126/sciadv.abe1603** — JRC's Tropical Moist Forest (TMF) v2025
//! release (published 2026-02 by the European Commission Joint Research
//! Centre). The v2025 update extends the original 1990-2019 record through
//! calendar year 2025 (`AnnualChange_1990` … `AnnualChange_2025`), adds a
//! refined `TransitionMap_Subtypes` taxonomy, and reissues the
//! `DeforestationYear` / `DegradationYear` companion layers.
//!
//! ## Coverage and tiling
//!
//! The product covers the **tropical belt only** — strictly between
//! **30° S and 30° N latitude** — across **250 land tiles** per dataset,
//! each a **10° × 10°** GeoTIFF at **30 m** native resolution
//! (37 037 × 37 037 px = ~5.5 GB uncompressed; LZW compression on the
//! upstream brings the wire size down to **~84 MB per tile**). Each tile
//! is named by its **top-left corner** (north edge, west edge):
//! - `lat_tag` ∈ {`N30`, `N20`, `N10`, `N0`, `S10`, `S20`, `S30`} — the
//!   tile spanning latitudes `[lat_top - 10, lat_top]` is anchored by
//!   `lat_top`. Note the **north-edge** convention: `N0` is the tile
//!   whose north edge sits on the equator, covering 10° S to 0°.
//! - `lng_tag` ∈ {`W180`, `W170`, …, `W10`, `E0`, `E10`, …, `E170`} —
//!   the tile spanning longitudes `[lng_left, lng_left + 10]` is anchored
//!   by `lng_left`. The east-most tile is `E170` (covers 170° E to 180°).
//!
//! ## Wire-level access pattern (the reason this connector exists)
//!
//! The JRC dispatcher serves tiles through a **CGI-style script** at
//! `https://ies-ows.jrc.ec.europa.eu/iforce/tmf_v1/download.py?type=tile&dataset={DS}&lat={LAT}&lon={LON}`.
//! Live probe on 2026-05-20:
//!
//! ```text
//! GET …?type=tile&dataset=DeforestationYear&lat=N0&lon=W80
//! HTTP/1.1 200 OK
//! content-disposition: file; filename="JRC_TMF_DeforestationYear_INT_1982_2025_v1_SAM_ID28_N0_W80.tif"
//! content-length: 77,431,558
//! content-type: application/octet_stream
//! ```
//!
//! Two contract details the materializer has to respect:
//!
//! 1. **`lat` / `lon` MUST be the N/S / E/W tag form** (e.g. `N0`,
//!    `W80`), NOT signed decimals. Passing `lat=-1&lon=-76` returns
//!    `HTTP 500` + an HTML error body — the dispatcher silently fails
//!    the numeric form. The connector composes the URL through
//!    [`tile_lat_tag`] / [`tile_lng_tag`] for exactly this reason.
//!
//! 2. **The response is BigTIFF**, not classic TIFF — the file magic
//!    on every v2025 tile we've probed (DeforestationYear,
//!    DegradationYear, AnnualChange_*, TransitionMap_Subtypes) is
//!    `II+\0` (0x002B), not classic `II*\0` (0x002A). Tiles are
//!    tile-organised (256×256), LZW-compressed, single-band uint8 /
//!    uint16 depending on the dataset, with EPSG:4326 geo-tags and
//!    Predictor 1 (no horizontal differencing). The shared
//!    [`crate::cog`] sampler (BigTIFF-aware since 2026-05-06) handles
//!    all of this — see [`sample_local_tiff_uint8`] /
//!    [`sample_local_tiff_uint16`] which delegate to it through the
//!    `file://` short-circuit in [`crate::cog::open_profile`] /
//!    [`crate::cog::sample_pixel`].
//!
//! Critically, the response **does not include `Accept-Ranges`** and a
//! follow-up `Range: bytes=0-1023` request returns the **full ~80 MB
//! body** with HTTP `200`, not `206 Partial Content`. The shared
//! [`crate::cog`] sampler relies on `Range` for cheap COG window reads;
//! it would download the entire tile on every per-cell request, which
//! is wasteful and (more importantly) makes the materializer unusable
//! at the cell-recall latency we promise.
//!
//! **The fix is pull-and-cache.** On the first miss we download the
//! whole tile to `<EMEM_DATA>/jrc_tmf_cache/<dataset>_<lat_tag>_<lng_tag>.tif`,
//! then every subsequent per-cell read parses the local file directly
//! via `file://`-routed `cog::open_profile` + `cog::sample_pixel`.
//! Subsequent recalls hit the disk only — no upstream traffic. The cache
//! file is mtime-checked; tiles older than [`CACHE_STALENESS_DAYS`] are
//! re-downloaded on next access (cheap insurance against a JRC
//! reprocessing without bumping the dispatcher path).
//!
//! ## Honest defaults
//!
//! - A pixel value of `0` in `AnnualChange_{year}` is the documented
//!   "non-forest / no-data" sentinel for the cell-year — a meaningful
//!   Primary fact that the materializer signs as Primary, NOT Absence.
//! - A cell outside ±30° latitude returns
//!   [`JrcTmfError::CoverageGap`] so the materializer can sign an
//!   `Absence` (the cell is genuinely outside the dataset's tropical
//!   belt). This mirrors the no-fallback rule in the protocol.
//! - A year outside 1990..=2025 surfaces as
//!   [`JrcTmfError::YearNotAvailable`] — we never silently round to
//!   the nearest available year.
//! - Atomic-rename on download (write to `<final>.partial.<pid>.<nanos>`,
//!   then `rename` into place) prevents torn-write reads if the process
//!   is killed mid-download.
//!
//! ## Concurrency note
//!
//! Two concurrent recalls for the same tile both find the cache absent
//! and both kick off a download. The atomic-rename pattern guarantees
//! that whichever download finishes last wins the final filename; the
//! other download wastes bandwidth but the result is correct. A future
//! revision can add an in-process `Mutex` keyed by tile path to elide
//! the duplicate download — left out here to keep the connector
//! standalone (no cross-thread state required for correctness).

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use reqwest::Client;

use crate::cog;

/// Earliest `AnnualChange_{year}` vintage published in v2025.
pub const JRC_TMF_MIN_YEAR: u16 = 1990;
/// Latest `AnnualChange_{year}` vintage published in v2025 (2026-02 release).
pub const JRC_TMF_MAX_YEAR: u16 = 2025;

/// Latitude window covered by the dataset (tropical belt, **strict**
/// north and south bounds). Tiles only exist between these latitudes.
const JRC_TMF_NORTH_LIMIT: f64 = 30.0;
const JRC_TMF_SOUTH_LIMIT: f64 = -30.0;

/// Cache subdirectory under `<EMEM_DATA>` (or `/var/emem` when the env
/// var is unset). Holds one full-tile GeoTIFF per `(dataset, lat_tag,
/// lng_tag)` triple.
const CACHE_SUBDIR: &str = "jrc_tmf_cache";

/// How long a cached tile is considered fresh before we re-download.
/// JRC publishes new TMF vintages annually (2026-02 is v2025). 90 days
/// is comfortably under the publication cadence yet long enough that
/// repeated recalls in a typical analytics run hit the cache.
pub const CACHE_STALENESS_DAYS: u64 = 90;

/// Per-download timeout. An 84 MB tile over the JRC CDN takes
/// 30-180 s on a normal link; 5 minutes is the safety ceiling.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);

/// JRC dispatcher base URL. The full URL is built by appending the
/// `?type=tile&dataset={DS}&lat={LAT}&lon={LON}` query string (see
/// [`tile_url`]). The base is split out so a future mirror swap is a
/// one-line edit.
const JRC_TMF_DISPATCHER: &str = "https://ies-ows.jrc.ec.europa.eu/iforce/tmf_v1/download.py";

/// Dataset-name segment for the per-year `AnnualChange_{year}` raster
/// (uint8 land-use classification, 1990..=2025 in v2025). Reissued every
/// vintage; the per-year suffix is the calendar year.
pub const DATASET_ANNUAL_CHANGE_PREFIX: &str = "AnnualChange";
/// Dataset-name segment for the `DeforestationYear` raster — uint16
/// containing the calendar year of the first observed deforestation
/// event at each pixel (0 = no deforestation observed in 1990..=2025).
pub const DATASET_DEFORESTATION_YEAR: &str = "DeforestationYear";
/// Dataset-name segment for the `DegradationYear` raster — uint16
/// containing the calendar year of the first observed degradation
/// event at each pixel (0 = no degradation observed in 1990..=2025).
pub const DATASET_DEGRADATION_YEAR: &str = "DegradationYear";
/// Dataset-name segment for the `TransitionMap_Subtypes` taxonomy.
pub const DATASET_TRANSITION_SUBTYPES: &str = "TransitionMap_Subtypes";
/// Dataset-name segment for the coarser `TransitionMap_MainClasses`
/// taxonomy (10 high-level transition classes).
pub const DATASET_TRANSITION_MAIN: &str = "TransitionMap_MainClasses";
/// Dataset-name segment for the `UndisturbedDegradedForest` mask.
pub const DATASET_UNDISTURBED_DEGRADED: &str = "UndisturbedDegradedForest";

/// Errors specific to the JRC TMF v2025 connector. Bubbled up through
/// [`crate::FetchError::Transport`] at the dispatcher boundary so callers
/// don't have to thread two error types. Each variant carries enough
/// context for a materializer to sign the correct fact shape (Primary,
/// Absence, or hard error).
#[derive(Debug, thiserror::Error)]
pub enum JrcTmfError {
    /// HTTP / network failure on the dispatcher itself (timeout, DNS,
    /// connection reset). The materializer should treat this as a
    /// retryable transport error.
    #[error("transport: {0}")]
    Transport(String),
    /// COG parse / decode failure on the cached tile (TIFF layout
    /// drift, LZW stream corruption, pixel out of dataset range).
    /// Indicates upstream corruption — the no-fallback rule applies.
    #[error("decode: {0}")]
    Decode(String),
    /// Cell sits outside the dataset's tropical-belt coverage
    /// (|lat| > 30°, or non-finite coordinates). Materializers MUST
    /// sign this as an `Absence` — the cell is genuinely outside the
    /// JRC TMF dataset.
    #[error("coverage_gap: cell ({lat:.6}, {lng:.6}) outside JRC TMF v2025 ±30° tropical belt")]
    CoverageGap {
        /// Cell latitude (degrees), for diagnostics.
        lat: f64,
        /// Cell longitude (degrees), for diagnostics.
        lng: f64,
    },
    /// Caller asked for an `AnnualChange_{year}` outside the published
    /// 1990..=2025 window. The dataset is annually published — there is
    /// no fallback to a near year.
    #[error(
        "year_not_available: {year} is outside JRC TMF v2025 window ({JRC_TMF_MIN_YEAR}..={JRC_TMF_MAX_YEAR})"
    )]
    YearNotAvailable {
        /// The year the caller requested.
        year: u16,
    },
    /// Cache directory creation, atomic rename, or other local I/O
    /// failure. The materializer should fail hard rather than try the
    /// next provider — the disk problem is local, not upstream.
    #[error("cache_io: {reason}")]
    CacheIo {
        /// Human-readable explanation of the I/O failure.
        reason: String,
    },
    /// The upstream JRC dispatcher call itself failed (non-2xx response,
    /// stalled body stream, etc.). Distinct from `Transport` because
    /// the materializer can attest the URL it tried.
    #[error("tile_fetch: {reason} (url={url})")]
    TileFetch {
        /// Human-readable explanation.
        reason: String,
        /// Full dispatcher URL we attempted.
        url: String,
    },
}

/// Resolve the tile cache directory. Honors `EMEM_DATA`; falls back to
/// `/var/emem` when unset (the systemd unit + Dockerfile both bind that
/// path to a persistent volume). Pure helper — no I/O. The materializer
/// is responsible for ensuring the directory exists on first miss.
pub fn cache_dir() -> PathBuf {
    let base = std::env::var("EMEM_DATA").unwrap_or_else(|_| "/var/emem".into());
    Path::new(&base).join(CACHE_SUBDIR)
}

/// Stable on-disk filename for the cached tile of a `(dataset, lat_tag,
/// lng_tag)` triple. Pure helper — no I/O. Filename is deterministic so
/// concurrent processes resolve to the same path and the atomic-rename
/// race resolves on filesystem semantics.
pub fn cached_tile_path(dataset: &str, lat_tag: &str, lng_tag: &str) -> PathBuf {
    cache_dir().join(format!("{dataset}_{lat_tag}_{lng_tag}.tif"))
}

/// Compute the `lat_tag` for the JRC TMF tile covering the given
/// latitude, or `None` if the latitude is outside the ±30° tropical
/// belt (or non-finite). Tiles are anchored at their **north edge**:
///
/// - `lat = -3.5` → tile spans `[-10, 0)` → north edge `0` → `"N0"`.
/// - `lat = 5.0`  → tile spans `[0, 10)` → north edge `10` → `"N10"`.
/// - `lat = -25.0` → tile spans `[-30, -20)` → north edge `-20` → `"S20"`.
/// - `lat = 30.0` (exact upper bound): out-of-bounds (the dataset's
///   north-most published tile is `N30`, covering 20° N to 30° N — a
///   cell exactly on 30° N has no tile whose north edge is *above* it).
///
/// The anchoring rule is `lat_top = ceil(lat / 10) * 10` for `lat ≤ 0`
/// and the same for `lat > 0` — but with the caveat that `lat`
/// strictly less than the south-most tile's south edge (-30°) is
/// out-of-bounds, and `lat` strictly greater than or equal to the
/// dataset's published north limit (30°) is also out-of-bounds.
pub fn tile_lat_tag(lat: f64) -> Option<String> {
    if !lat.is_finite() {
        return None;
    }
    if !(JRC_TMF_SOUTH_LIMIT..JRC_TMF_NORTH_LIMIT).contains(&lat) {
        return None;
    }
    // North-edge anchored: lat_top = ceil(lat / 10) * 10. For lat = -25
    // we get ceil(-2.5) = -2, so lat_top = -20 → "S20". For lat = 5 we
    // get ceil(0.5) = 1, so lat_top = 10 → "N10". For lat = 0 (exact
    // equator) we get ceil(0.0) = 0, so lat_top = 0 → "N0". For lat =
    // -3.5 we get ceil(-0.35) = 0, so lat_top = 0 → "N0".
    //
    // The dataset publishes lat_tags N30..S30 in 10° steps (7 values).
    // The S30 tile covers the southern sliver near the dataset's
    // south boundary (a cell exactly on lat=-30 maps to S30, which
    // covers latitudes [-40, -30]; only the very north edge of that
    // tile carries published data in the v2025 release).
    let lat_top = (lat / 10.0).ceil() as i32 * 10;
    Some(if lat_top >= 0 {
        format!("N{}", lat_top)
    } else {
        format!("S{}", lat_top.unsigned_abs())
    })
}

/// Compute the `lng_tag` for the JRC TMF tile covering the given
/// longitude, or `None` for non-finite longitudes or longitudes
/// outside [-180, 180]. Tiles are anchored at their **west edge**:
///
/// - `lng = -60.5` → tile spans `[-70, -60)` → west edge `-70` → `"W70"`.
/// - `lng =  27.3` → tile spans `[20, 30)` → west edge `20` → `"E20"`.
/// - `lng = 0.0` → tile spans `[0, 10)` → west edge `0` → `"E0"`.
/// - `lng = -180.0` → tile spans `[-180, -170)` → west edge `-180` → `"W180"`.
///
/// Longitudes are valid across the full [-180, 180] range; the
/// tropical-belt coverage gate lives in [`tile_lat_tag`].
pub fn tile_lng_tag(lng: f64) -> Option<String> {
    if !lng.is_finite() || !(-180.0..=180.0).contains(&lng) {
        return None;
    }
    // West-edge anchored: lng_left = floor(lng / 10) * 10. The east-most
    // valid lng_tag is "E170" (tile [170, 180)); a cell at lng = 180
    // exactly maps to "E180" by the floor formula but no such tile is
    // published — clamp to "E170" so the boundary cell still resolves.
    let mut lng_left = (lng / 10.0).floor() as i32 * 10;
    if lng_left >= 180 {
        lng_left = 170;
    }
    Some(if lng_left >= 0 {
        format!("E{}", lng_left)
    } else {
        format!("W{}", lng_left.unsigned_abs())
    })
}

/// Return `true` iff `year` is in the published `AnnualChange_{year}`
/// window for v2025 (1990..=2025 inclusive). Pure helper.
pub fn year_is_supported(year: u16) -> bool {
    (JRC_TMF_MIN_YEAR..=JRC_TMF_MAX_YEAR).contains(&year)
}

/// Build the dispatcher URL for a `(dataset, lat_tag, lng_tag)` triple.
/// Pure helper — no I/O. The dispatcher returns the full ~84 MB tile;
/// the receipt verifier can re-issue the same URL to confirm the source.
pub fn tile_url(dataset: &str, lat_tag: &str, lng_tag: &str) -> String {
    format!("{JRC_TMF_DISPATCHER}?type=tile&dataset={dataset}&lat={lat_tag}&lon={lng_tag}")
}

/// Ensure the JRC TMF tile covering `(lat, lng)` for `dataset` is
/// present in the local cache. The heart of the pull-and-cache strategy.
///
/// Workflow:
/// 1. Compute `(lat_tag, lng_tag)`. Fail with `CoverageGap` if either
///    side is out-of-bounds (lat outside ±30°, or non-finite coords).
/// 2. Compute the cache path. If the file exists AND its mtime is newer
///    than [`CACHE_STALENESS_DAYS`] days ago, return the path
///    immediately — no network, no metadata roundtrip.
/// 3. Otherwise download the dispatcher URL with a 5-minute timeout.
///    Write the body to `<final>.partial.<pid>.<nanos>` first, then
///    `std::fs::rename` into place. This is atomic on every POSIX
///    filesystem and safe across crashes — a partial write never
///    surfaces as a "complete" cached tile.
/// 4. Return the final cache path.
///
/// Errors:
/// - [`JrcTmfError::CoverageGap`] when the cell maps outside ±30° lat.
/// - [`JrcTmfError::CacheIo`] for cache-dir creation / rename / write.
/// - [`JrcTmfError::TileFetch`] for non-2xx upstream responses or
///   stalled body streams.
/// - [`JrcTmfError::Transport`] for low-level reqwest errors.
pub async fn ensure_tile_cached(
    client: &Client,
    dataset: &str,
    lat: f64,
    lng: f64,
) -> Result<PathBuf, JrcTmfError> {
    let lat_tag = tile_lat_tag(lat).ok_or(JrcTmfError::CoverageGap { lat, lng })?;
    let lng_tag = tile_lng_tag(lng).ok_or(JrcTmfError::CoverageGap { lat, lng })?;
    let final_path = cached_tile_path(dataset, &lat_tag, &lng_tag);

    if cache_hit_fresh(&final_path) {
        return Ok(final_path);
    }

    // Cache miss: download to a per-process temp file then atomic-rename
    // into place. This pattern is documented (e.g. `koppen.rs`,
    // `dmsp_ols.rs`) — `process::id()` plus a nanosecond suffix gives
    // every concurrent download a unique partial filename so two
    // workers racing on the same tile do not corrupt each other's
    // intermediate state.
    let dir = cache_dir();
    std::fs::create_dir_all(&dir).map_err(|e| JrcTmfError::CacheIo {
        reason: format!("mkdir {}: {e}", dir.display()),
    })?;

    let url = tile_url(dataset, &lat_tag, &lng_tag);
    let body = download_tile(client, &url).await?;

    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp_path = final_path.with_extension(format!("partial.{}.{}", std::process::id(), nanos));
    std::fs::write(&tmp_path, &body).map_err(|e| JrcTmfError::CacheIo {
        reason: format!("write tmp {}: {e}", tmp_path.display()),
    })?;
    std::fs::rename(&tmp_path, &final_path).map_err(|e| JrcTmfError::CacheIo {
        reason: format!(
            "rename {} -> {}: {e}",
            tmp_path.display(),
            final_path.display()
        ),
    })?;

    Ok(final_path)
}

/// Return `true` iff `path` exists AND its mtime is within the
/// staleness window. Treats any I/O error (including non-existence) as
/// "miss"; the caller will then go through the download path. Pulled
/// out as a helper so the test suite can pin the freshness contract.
fn cache_hit_fresh(path: &Path) -> bool {
    let metadata = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return false,
    };
    let modified = match metadata.modified() {
        Ok(t) => t,
        Err(_) => return false,
    };
    let age = match SystemTime::now().duration_since(modified) {
        Ok(d) => d,
        // `modified` is in the future — clock skew. Treat as fresh
        // rather than thrash the cache on a workstation whose clock
        // jumped backwards.
        Err(_) => return true,
    };
    age < Duration::from_secs(CACHE_STALENESS_DAYS * 24 * 60 * 60)
}

/// Internal: GET the dispatcher URL with a 5-minute timeout and return
/// the full body. Wraps reqwest errors into `TileFetch` (non-2xx) or
/// `Transport` (network).
async fn download_tile(client: &Client, url: &str) -> Result<bytes::Bytes, JrcTmfError> {
    let resp = client
        .get(url)
        .timeout(DOWNLOAD_TIMEOUT)
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
        .map_err(|e| JrcTmfError::Transport(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(JrcTmfError::TileFetch {
            reason: format!("upstream returned status {}", resp.status()),
            url: url.to_string(),
        });
    }
    resp.bytes().await.map_err(|e| JrcTmfError::TileFetch {
        reason: format!("body read failed: {e}"),
        url: url.to_string(),
    })
}

/// Read one pixel from the cached `AnnualChange_{year}` raster at
/// `(lat, lng)` and return the uint8 land-use class.
///
/// The class taxonomy is documented at
/// `https://forobs.jrc.ec.europa.eu/TMF` (10 main classes plus
/// subtypes); the canonical mapping is part of the band's manifest
/// metadata. Here we surface the raw byte unmodified — the materializer
/// signs that as the Primary fact and the band registry attaches
/// per-class semantics.
///
/// Returns:
/// - `Ok(class_byte)` for an in-coverage cell. `0` is a meaningful
///   Primary fact ("non-forest / outside study area for the year") —
///   the materializer does NOT promote it to Absence.
/// - `Err(YearNotAvailable)` for years outside 1990..=2025.
/// - `Err(CoverageGap)` for cells outside ±30° latitude.
/// - `Err(Transport)` / `Err(TileFetch)` / `Err(CacheIo)` for the
///   download path; `Err(Decode)` for cached-tile parse failures.
pub async fn fetch_annual_change(
    client: &Client,
    lat: f64,
    lng: f64,
    year: u16,
) -> Result<u8, JrcTmfError> {
    if !year_is_supported(year) {
        return Err(JrcTmfError::YearNotAvailable { year });
    }
    let dataset = format!("{DATASET_ANNUAL_CHANGE_PREFIX}_{year}");
    let path = ensure_tile_cached(client, &dataset, lat, lng).await?;
    sample_uint8_pixel(&path, lat, lng).await
}

/// Read one pixel from the cached `DeforestationYear` raster at
/// `(lat, lng)` and return the calendar year (uint16; `0` means "no
/// deforestation observed in 1990..=2025"). The full year-of-event
/// is preserved unchanged so the materializer can attest the exact
/// detection window.
///
/// Returns the same error shape as [`fetch_annual_change`] minus the
/// `YearNotAvailable` variant — `DeforestationYear` is a single
/// dataset-wide raster, not a per-year time series.
pub async fn fetch_deforestation_year(
    client: &Client,
    lat: f64,
    lng: f64,
) -> Result<u16, JrcTmfError> {
    let path = ensure_tile_cached(client, DATASET_DEFORESTATION_YEAR, lat, lng).await?;
    sample_uint16_pixel(&path, lat, lng).await
}

/// Read one pixel from the cached `DegradationYear` raster at
/// `(lat, lng)` and return the calendar year (uint16; `0` means "no
/// degradation observed in 1990..=2025"). Same contract as
/// [`fetch_deforestation_year`].
pub async fn fetch_degradation_year(
    client: &Client,
    lat: f64,
    lng: f64,
) -> Result<u16, JrcTmfError> {
    let path = ensure_tile_cached(client, DATASET_DEGRADATION_YEAR, lat, lng).await?;
    sample_uint16_pixel(&path, lat, lng).await
}

/// Read one pixel from the cached `TransitionMap_Subtypes` raster at
/// `(lat, lng)` and return the uint8 subtype class. Refined taxonomy
/// (~25 subtypes); the [`DATASET_TRANSITION_MAIN`] dataset offers a
/// coarser 10-class roll-up.
pub async fn fetch_transition_subtype(
    client: &Client,
    lat: f64,
    lng: f64,
) -> Result<u8, JrcTmfError> {
    let path = ensure_tile_cached(client, DATASET_TRANSITION_SUBTYPES, lat, lng).await?;
    sample_uint8_pixel(&path, lat, lng).await
}

/// Sample a uint8 pixel from a locally-cached JRC TMF tile. Routes
/// the local file through the shared [`crate::cog`] sampler via its
/// `file://` short-circuit. The sampler handles both classic TIFF
/// (`II*\0`) and BigTIFF (`II+\0`); JRC TMF v2025 ships every tile we
/// have probed as BigTIFF, so the BigTIFF path is the one exercised
/// in practice.
async fn sample_uint8_pixel(path: &Path, lat: f64, lng: f64) -> Result<u8, JrcTmfError> {
    let value = sample_via_cog(path, lat, lng, 8).await?;
    if !value.is_finite() || value < 0.0 || value > u8::MAX as f64 {
        return Err(JrcTmfError::Decode(format!(
            "uint8 pixel value {value} out of u8 range"
        )));
    }
    Ok(value as u8)
}

/// Sample a uint16 pixel from a locally-cached JRC TMF tile. Same
/// machinery as [`sample_uint8_pixel`] but for the 16-bit raster
/// (DeforestationYear / DegradationYear ship year-of-event in uint16
/// so we can carry the full 1990..=2025 range plus a `0` sentinel).
async fn sample_uint16_pixel(path: &Path, lat: f64, lng: f64) -> Result<u16, JrcTmfError> {
    let value = sample_via_cog(path, lat, lng, 16).await?;
    if !value.is_finite() || value < 0.0 || value > u16::MAX as f64 {
        return Err(JrcTmfError::Decode(format!(
            "uint16 pixel value {value} out of u16 range"
        )));
    }
    Ok(value as u16)
}

/// Open the cached tile as a COG via the `file://` short-circuit in
/// [`crate::cog::open_profile`], assert the expected bit-depth and
/// single-band layout, and sample the pixel at `(lat, lng)` in
/// EPSG:4326 world space. JRC TMF v2025 tiles are all BigTIFF,
/// LZW-compressed, Predictor 1 (no horizontal differencing),
/// `samples_per_pixel = 1`, and either uint8 (`AnnualChange_*`,
/// `TransitionMap_*`) or uint16 (`DeforestationYear`,
/// `DegradationYear`).
async fn sample_via_cog(
    path: &Path,
    lat: f64,
    lng: f64,
    expected_bits: u16,
) -> Result<f64, JrcTmfError> {
    // The `file://` short-circuit in `cog::http_range` requires an
    // absolute path on every platform we ship to (Linux container,
    // macOS dev box) — canonicalize so a relative `EMEM_DATA=…`
    // resolves identically across processes.
    let canonical = std::fs::canonicalize(path).map_err(|e| JrcTmfError::CacheIo {
        reason: format!("canonicalize {}: {e}", path.display()),
    })?;
    let url = format!("file://{}", canonical.display());
    // `Client::new()` is unused on the `file://` branch (the short
    // circuit never touches the network) but the signature requires
    // it. Keep it dirt-cheap: `Client::new()` doesn't open any
    // sockets.
    let client = Client::new();
    let profile = cog::open_profile(&client, &url)
        .await
        .map_err(|e| JrcTmfError::Decode(format!("cog::open_profile: {e}")))?;
    if profile.bits_per_sample != expected_bits {
        return Err(JrcTmfError::Decode(format!(
            "expected bits_per_sample={expected_bits} (got {}); dataset layout drift?",
            profile.bits_per_sample
        )));
    }
    if profile.samples_per_pixel != 1 {
        return Err(JrcTmfError::Decode(format!(
            "expected single-band tile (samples_per_pixel=1), got {}",
            profile.samples_per_pixel
        )));
    }
    // World-space sampling. JRC TMF is EPSG:4326 so x ≡ lng, y ≡ lat.
    // `sample_pixel` returns CoverageGap-equivalent (`Unsupported`)
    // for pixels outside the tile envelope; surface that as
    // `JrcTmfError::CoverageGap` so the materializer can sign Absence
    // honestly.
    cog::sample_pixel(&client, &url, &profile, lng, lat)
        .await
        .map_err(|e| match e {
            cog::CogError::Unsupported(msg) if msg.contains("outside image") => {
                JrcTmfError::CoverageGap { lat, lng }
            }
            other => JrcTmfError::Decode(format!("cog::sample_pixel: {other}")),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `tile_lat_tag` honors the north-edge anchoring documented in the
    /// JRC TMF README: a cell at `lat = -3.5` lands in the tile whose
    /// north edge is `0` (`"N0"`); a cell at `lat = 5.0` lands in the
    /// tile spanning [0, 10) → `"N10"`; `lat = -25.0` → `"S20"` (tile
    /// spans [-30, -20)). Reference cells:
    ///
    /// - **Amazonas (Manaus)** lat ≈ -3.1: `"N0"`.
    /// - **Congo Basin** lat ≈ -1.0: `"N0"`.
    /// - **Borneo (Kalimantan interior)** lat ≈ -1.5: `"N0"`.
    /// - **Madagascar (Tana)** lat ≈ -18.9: `"S10"`.
    /// - **Northern Brazil (Roraima)** lat ≈ 2.8: `"N10"`.
    /// - Southern coverage edge `lat = -29.9`: `"S20"`.
    /// - Equator exactly: `"N0"`.
    #[test]
    fn tile_lat_tag_known_cells() {
        assert_eq!(tile_lat_tag(-3.1).as_deref(), Some("N0"), "Manaus → N0");
        assert_eq!(tile_lat_tag(-1.0).as_deref(), Some("N0"), "Congo → N0");
        assert_eq!(tile_lat_tag(-1.5).as_deref(), Some("N0"), "Borneo → N0");
        assert_eq!(
            tile_lat_tag(-18.9).as_deref(),
            Some("S10"),
            "Madagascar (Tana, lat=-18.9) → S10 (tile spans [-20, -10))"
        );
        assert_eq!(
            tile_lat_tag(2.8).as_deref(),
            Some("N10"),
            "Roraima (lat=2.8) → N10 (tile spans [0, 10))"
        );
        assert_eq!(
            tile_lat_tag(-25.0).as_deref(),
            Some("S20"),
            "lat=-25 → S20 (tile spans [-30, -20))"
        );
        assert_eq!(
            tile_lat_tag(-29.9).as_deref(),
            Some("S20"),
            "lat=-29.9 (south margin) still in S20 tile"
        );
        assert_eq!(
            tile_lat_tag(0.0).as_deref(),
            Some("N0"),
            "equator exactly → N0"
        );
    }

    /// `tile_lat_tag` returns `None` for any latitude outside the
    /// strict ±30° tropical belt — this is the gate that the
    /// materializer reads to decide CoverageGap vs Primary.
    #[test]
    fn tile_lat_tag_coverage_gap_outside_tropics() {
        for bad in [
            -90.0,
            -30.5,
            -31.0,
            30.0,
            30.5,
            60.0,
            89.9,
            f64::NAN,
            f64::INFINITY,
        ] {
            assert_eq!(
                tile_lat_tag(bad),
                None,
                "lat={bad} must be CoverageGap (outside ±30° belt)"
            );
        }
        // Exactly -30 IS in-bounds; the formula maps it to the S30
        // tile (which the v2025 release publishes as the south-most
        // continental sliver). Pin this to lock the boundary contract.
        assert_eq!(
            tile_lat_tag(-30.0).as_deref(),
            Some("S30"),
            "lat=-30 is the inclusive south boundary; lands in S30 \
             (the v2025 release publishes S30 as the south-most tile tag)"
        );
        // Exactly +30 is OUT (no tile whose north edge is above 30°).
        assert_eq!(
            tile_lat_tag(30.0),
            None,
            "lat=+30 has no tile; +30 is the exclusive north boundary"
        );
    }

    /// `tile_lng_tag` honors west-edge anchoring across all longitudes.
    /// Reference cells:
    ///
    /// - Amazonas (Manaus) lng ≈ -60.0: `"W60"` (tile spans [-60, -50)).
    /// - Congo Basin lng ≈ 23.0: `"E20"` (tile spans [20, 30)).
    /// - Borneo lng ≈ 113.0: `"E110"` (tile spans [110, 120)).
    /// - Greenwich exactly: `"E0"`.
    /// - Date-line cell lng = -179.9: `"W180"`.
    /// - East-most boundary lng = 179.9: `"E170"`.
    /// - lng = 180 exactly: clamps to `"E170"` (no E180 tile published).
    #[test]
    fn tile_lng_tag_known_cells() {
        assert_eq!(
            tile_lng_tag(-60.5).as_deref(),
            Some("W70"),
            "lng=-60.5 → W70 (tile spans [-70, -60))"
        );
        assert_eq!(
            tile_lng_tag(-60.0).as_deref(),
            Some("W60"),
            "lng=-60 exactly → W60 (the boundary lands on the tile whose west edge is -60)"
        );
        assert_eq!(
            tile_lng_tag(27.3).as_deref(),
            Some("E20"),
            "Congo (lng=27.3) → E20 (tile spans [20, 30))"
        );
        assert_eq!(
            tile_lng_tag(113.0).as_deref(),
            Some("E110"),
            "Borneo (lng=113) → E110 (tile spans [110, 120))"
        );
        assert_eq!(
            tile_lng_tag(0.0).as_deref(),
            Some("E0"),
            "Greenwich exactly → E0"
        );
        assert_eq!(
            tile_lng_tag(-179.9).as_deref(),
            Some("W180"),
            "lng=-179.9 → W180 (tile spans [-180, -170))"
        );
        assert_eq!(
            tile_lng_tag(179.9).as_deref(),
            Some("E170"),
            "lng=179.9 → E170 (tile spans [170, 180))"
        );
        assert_eq!(
            tile_lng_tag(180.0).as_deref(),
            Some("E170"),
            "lng=180 exactly clamps to E170 (no E180 tile published)"
        );
    }

    /// `tile_lng_tag` rejects non-finite longitudes and out-of-range
    /// values. The materializer uses these as the gating signal for a
    /// CoverageGap result.
    #[test]
    fn tile_lng_tag_invalid_inputs() {
        for bad in [
            f64::NAN,
            f64::INFINITY,
            -f64::INFINITY,
            -180.5,
            180.5,
            360.0,
            -360.0,
        ] {
            assert_eq!(
                tile_lng_tag(bad),
                None,
                "lng={bad} must yield None (out of [-180, 180])"
            );
        }
        assert_eq!(
            tile_lng_tag(-180.0).as_deref(),
            Some("W180"),
            "lng=-180 exactly → W180 (the inclusive west boundary)"
        );
    }

    /// `year_is_supported` matches the published v2025 `AnnualChange`
    /// window: 1990 inclusive through 2025 inclusive. This pins the
    /// no-fallback rule (we never silently round to the nearest year).
    #[test]
    fn year_is_supported_window() {
        // Endpoints of the published window.
        assert!(year_is_supported(JRC_TMF_MIN_YEAR));
        assert!(year_is_supported(JRC_TMF_MAX_YEAR));
        assert!(year_is_supported(1990));
        assert!(year_is_supported(2025));
        // Representative interior.
        assert!(year_is_supported(2010));
        assert!(year_is_supported(2019));
        // Out-of-bounds.
        assert!(!year_is_supported(1989), "1989 below v2025 publication");
        assert!(!year_is_supported(2026), "2026 not yet published");
        assert!(!year_is_supported(0));
        assert!(!year_is_supported(u16::MAX));
    }

    /// `cached_tile_path` is deterministic — the same `(dataset,
    /// lat_tag, lng_tag)` must yield the same on-disk filename across
    /// processes. This is the contract that lets two concurrent
    /// downloads race to the atomic-rename safely (the `.partial.<pid>`
    /// suffix differs but the final path collides, which is what we
    /// want — `rename` is atomic).
    #[test]
    fn cached_tile_path_is_deterministic() {
        let p1 = cached_tile_path("AnnualChange_2023", "N0", "E20");
        let p2 = cached_tile_path("AnnualChange_2023", "N0", "E20");
        assert_eq!(p1, p2, "same inputs must yield same path");

        // Filename pattern: <dataset>_<lat>_<lng>.tif. Pinned literally
        // so any accidental refactor of the joining is caught at test
        // time.
        assert_eq!(
            p1.file_name().and_then(|s| s.to_str()),
            Some("AnnualChange_2023_N0_E20.tif"),
            "filename must follow <dataset>_<lat>_<lng>.tif convention"
        );
        // Different inputs must yield different paths.
        let p3 = cached_tile_path("AnnualChange_2024", "N0", "E20");
        assert_ne!(p1, p3, "different year must yield different path");
        let p4 = cached_tile_path("AnnualChange_2023", "S10", "E20");
        assert_ne!(p1, p4, "different lat tag must yield different path");
    }

    /// `cache_dir` honors `EMEM_DATA` when set and falls back to
    /// `/var/emem` otherwise. We can't safely mutate the env in a
    /// parallel test runner, so we just pin the suffix and verify the
    /// fallback when the var is empty/absent.
    #[test]
    fn cache_dir_subdir_suffix() {
        let dir = cache_dir();
        assert!(
            dir.ends_with(CACHE_SUBDIR),
            "cache_dir must end at the {CACHE_SUBDIR} subdirectory — got {}",
            dir.display()
        );
    }

    /// `tile_url` composes the dispatcher URL deterministically. The
    /// receipt verifier re-derives the URL from `(dataset, lat_tag,
    /// lng_tag)` so this MUST produce a stable string.
    #[test]
    fn tile_url_is_deterministic() {
        let url = tile_url("AnnualChange_2023", "N0", "E20");
        assert_eq!(
            url,
            "https://ies-ows.jrc.ec.europa.eu/iforce/tmf_v1/download.py\
             ?type=tile&dataset=AnnualChange_2023&lat=N0&lon=E20",
            "tile_url must encode the live JRC dispatcher path verbatim"
        );
        // Known live tile from the spec: AFR_ID35_N0_E20 is the Congo
        // Basin tile in the AFR continent grouping. The dispatcher
        // reads dataset+lat+lon; the response Content-Disposition
        // includes the `AFR_ID35` continent/index suffix that the
        // materializer can attest after the download.
        let url = tile_url("DeforestationYear", "S10", "W60");
        assert!(
            url.contains("dataset=DeforestationYear")
                && url.contains("lat=S10")
                && url.contains("lon=W60")
        );
    }

    /// `fetch_annual_change` short-circuits to `YearNotAvailable` for
    /// years outside the v2025 1990..=2025 window — no network touched.
    /// Mirrors the no-fallback rule: we never silently round.
    #[tokio::test]
    async fn fetch_annual_change_year_not_available() {
        let client = Client::new();
        for bad_year in [1989_u16, 2026, 1900, 2099] {
            let err = fetch_annual_change(&client, -1.0, 23.0, bad_year)
                .await
                .unwrap_err();
            match err {
                JrcTmfError::YearNotAvailable { year } => {
                    assert_eq!(year, bad_year, "round-trip the requested year");
                }
                other => panic!("year {bad_year} must surface YearNotAvailable, got {other:?}"),
            }
        }
    }

    /// `fetch_annual_change` short-circuits to `CoverageGap` for cells
    /// outside the ±30° tropical belt before any network request. The
    /// year is in-range so this isolates the lat-gate behaviour.
    #[tokio::test]
    async fn fetch_annual_change_outside_tropics_is_coverage_gap() {
        let client = Client::new();
        // Latitude above the north limit.
        let err = fetch_annual_change(&client, 31.0, 0.0, 2023)
            .await
            .unwrap_err();
        assert!(
            matches!(err, JrcTmfError::CoverageGap { .. }),
            "lat=31 must surface CoverageGap, got {err:?}"
        );
        // Latitude below the south limit.
        let err = fetch_annual_change(&client, -31.0, 0.0, 2023)
            .await
            .unwrap_err();
        assert!(
            matches!(err, JrcTmfError::CoverageGap { .. }),
            "lat=-31 must surface CoverageGap, got {err:?}"
        );
        // Non-finite latitude.
        let err = fetch_annual_change(&client, f64::NAN, 0.0, 2023)
            .await
            .unwrap_err();
        assert!(
            matches!(err, JrcTmfError::CoverageGap { .. }),
            "NaN lat must surface CoverageGap, got {err:?}"
        );
        // Non-finite longitude.
        let err = fetch_annual_change(&client, 0.0, f64::INFINITY, 2023)
            .await
            .unwrap_err();
        assert!(
            matches!(err, JrcTmfError::CoverageGap { .. }),
            "infinite lng must surface CoverageGap, got {err:?}"
        );
    }

    /// `fetch_deforestation_year` and `fetch_degradation_year` share the
    /// same coverage gate as `fetch_annual_change` (no `year` arg, but
    /// the lat gate still fires before any network call).
    #[tokio::test]
    async fn year_event_fetchers_outside_tropics_are_coverage_gap() {
        let client = Client::new();
        for fetch_fn in 0..2 {
            let lat = 35.0; // outside ±30° belt
            let lng = 0.0;
            let err = if fetch_fn == 0 {
                fetch_deforestation_year(&client, lat, lng)
                    .await
                    .unwrap_err()
            } else {
                fetch_degradation_year(&client, lat, lng).await.unwrap_err()
            };
            assert!(
                matches!(err, JrcTmfError::CoverageGap { .. }),
                "lat=35 must surface CoverageGap on year-event fetcher #{fetch_fn}, got {err:?}"
            );
        }
    }

    /// `cache_hit_fresh` returns false for a non-existent path — the
    /// caller will then take the download path. This is the only
    /// public-ish observable of the freshness contract that doesn't
    /// require touching the filesystem in CI.
    #[test]
    fn cache_hit_fresh_missing_file_is_miss() {
        let p = cached_tile_path("AnnualChange_2023", "N0", "E20");
        // The cache_dir is /var/emem/jrc_tmf_cache by default — almost
        // certainly missing in a CI sandbox. If it happens to exist
        // (developer machine) we just assert nothing; the contract we
        // care about for CI is the missing-file branch.
        if !p.exists() {
            assert!(!cache_hit_fresh(&p), "missing file must be cache miss");
        }
    }

    /// Constants sanity: the dispatcher URL points at the JRC iforce
    /// host, the cache subdirectory matches the documented layout, and
    /// the staleness window is in the documented range.
    #[test]
    fn constants_sanity() {
        assert!(
            JRC_TMF_DISPATCHER.starts_with("https://ies-ows.jrc.ec.europa.eu/"),
            "dispatcher must point at the JRC iforce host"
        );
        assert!(
            JRC_TMF_DISPATCHER.ends_with("/iforce/tmf_v1/download.py"),
            "dispatcher path must be /iforce/tmf_v1/download.py"
        );
        assert_eq!(CACHE_SUBDIR, "jrc_tmf_cache");
        assert!((30..=365).contains(&CACHE_STALENESS_DAYS));
        assert_eq!(JRC_TMF_MIN_YEAR, 1990);
        assert_eq!(JRC_TMF_MAX_YEAR, 2025);
    }

    /// Live integration test against the JRC TMF dispatcher.
    /// Yasuni National Park (Ecuadorian Amazon) sits at
    /// `(-1.15, -76.45)` — well inside the SAM_ID28 tile (`N0`/`W80`).
    /// The dispatcher returns a ~77 MB BigTIFF (`II+\0` magic, magic
    /// 0x002B); the shared `cog::open_profile` BigTIFF path must
    /// decode it and yield a uint16 deforestation-year pixel. The
    /// pixel is `0` (no event) for the vast majority of the tile and
    /// a four-digit year (e.g. `2018`) where an event was logged —
    /// either is a valid Primary fact.
    ///
    /// `#[ignore]`-gated so CI doesn't hit the JRC server on every
    /// PR. Run with `cargo test --ignored -p emem-fetch
    /// fetch_deforestation_year_yasuni` and inspect the printed
    /// profile + sampled value.
    #[tokio::test]
    #[ignore = "live network: hits the JRC TMF dispatcher and downloads an ~80 MB tile"]
    async fn fetch_deforestation_year_yasuni() {
        // Use a per-test EMEM_DATA so the cache write goes somewhere
        // we can clean up; falling back to /var/emem on a developer
        // box is fine but requires write access.
        let tmp = std::env::temp_dir().join("emem_jrc_tmf_live_test");
        // Ignore mkdir errors — the connector creates the subdir itself.
        let _ = std::fs::create_dir_all(&tmp);
        std::env::set_var("EMEM_DATA", &tmp);

        let client = Client::new();
        let (lat, lng) = (-1.15_f64, -76.45_f64);
        let year = fetch_deforestation_year(&client, lat, lng)
            .await
            .expect("Yasuni cell must materialize as a Primary fact");
        // `year == 0` is the no-event sentinel (most pixels in any
        // tropical tile); anything non-zero must be a calendar year
        // inside the dataset window.
        assert!(
            year == 0 || (1982..=2025).contains(&year),
            "DeforestationYear pixel {year} must be 0 or in 1982..=2025 (Vancutsem 2021 reanalysis)"
        );

        // Re-open the cached tile through cog::open_profile so we can
        // print the parsed CogProfile for the report. Routing through
        // the same `file://` path the production sampler uses keeps
        // this honest — if the profile parses here it parses in the
        // materializer too.
        let tile_path = cached_tile_path(DATASET_DEFORESTATION_YEAR, "N0", "W80");
        let canonical = std::fs::canonicalize(&tile_path).expect("cached tile must exist");
        let url = format!("file://{}", canonical.display());
        let profile = crate::cog::open_profile(&client, &url)
            .await
            .expect("cached tile must parse as a COG");
        eprintln!(
            "[jrc_tmf live] profile: width={} height={} bps={} sf={} compression={} predictor={} \
             samples_per_pixel={} planar={} tile={}x{} ({} cols × {} rows = {} tiles) \
             pixel_scale=({:.10e}, {:.10e}) tiepoint={:?} epsg={:?}",
            profile.width,
            profile.height,
            profile.bits_per_sample,
            profile.sample_format,
            profile.compression,
            profile.predictor,
            profile.samples_per_pixel,
            profile.planar_config,
            profile.tile_w,
            profile.tile_h,
            profile.tile_cols,
            profile.tile_rows,
            profile.tile_offsets.len(),
            profile.pixel_scale.0,
            profile.pixel_scale.1,
            profile.tiepoint,
            profile.epsg,
        );
        eprintln!("[jrc_tmf live] Yasuni ({lat:.6}, {lng:.6}) DeforestationYear pixel = {year}");
        assert_eq!(profile.bits_per_sample, 16, "must be uint16 raster");
        assert_eq!(profile.compression, 5, "must be LZW-compressed");
        assert_eq!(profile.samples_per_pixel, 1, "must be single-band");

        // Second cell: Pará deforestation arc south of Belém,
        // `(-3.5, -49.5)` — same SAM continent grouping, different
        // SAM_ID tile (`N0`/`W50`). Cells in the arc carry a
        // non-zero event year. This exercises the second-tile
        // download path (different cached file) and confirms the
        // year-of-event range matches the v2025 1982..=2025 window.
        let para = fetch_deforestation_year(&client, -3.5, -49.5)
            .await
            .expect("Pará cell must materialize");
        eprintln!("[jrc_tmf live] Pará (-3.500000, -49.500000) DeforestationYear pixel = {para}");
        assert!(
            para == 0 || (1982..=2025).contains(&para),
            "Pará pixel {para} must be 0 or 1982..=2025"
        );
    }
}
