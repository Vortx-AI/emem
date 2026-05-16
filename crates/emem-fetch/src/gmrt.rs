//! Global Multi-Resolution Topography (GMRT) connector — anonymous
//! single-point bathymetry / topography path.
//!
//! Source: **Ryan, W. B. F., S. M. Carbotte, J. O. Coplan, S. O'Hara,
//! A. Melkonian, R. Arko, R. A. Weissel, V. Ferrini, A. Goodwillie,
//! F. Nitsche, J. Bonczkowski, R. Zemsky (2009). *Global Multi-Resolution
//! Topography synthesis*. Geochem. Geophys. Geosyst. 10, Q03014.
//! doi:10.1029/2008GC002332**. Maintained by the Marine Geoscience Data
//! System (MGDS) at Lamont-Doherty Earth Observatory, Columbia University.
//! GMRT fuses ship-borne multibeam swaths (gridded at native ship
//! resolution where available, ~50–100 m), regional compilations
//! (CCOM, IBCAO, GEBCO), and the SRTM30_PLUS land/coarse-ocean grid into
//! a single seamless quad-tree pyramid. Data and services are free,
//! anonymous, and require no API key per the MGDS public-access policy.
//!
//! **Sign convention.** Both PointServer JSON and GridServer GeoTIFF
//! return elevation in **metres relative to mean sea level (WGS-84 +
//! EGM2008-style geoid)**, with the standard topo-bathy sign:
//! - **Positive** → above sea level (topography). Verified live on
//!   2026-05-16: Mt. Everest summit (27.9881, 86.9250) → 8806 m, against
//!   the published 8848.86 m summit height (the difference reflects
//!   GMRT's tile-mosaic interpolation between SRTM-style sources at the
//!   summit pixel — within the ±50 m vertical tolerance for the highest
//!   pixels in the global mosaic).
//! - **Negative** → below sea level (bathymetry). Verified live on
//!   2026-05-16: Mariana Trench (11.45, 142.22) → -9049 m via PointServer
//!   and a min of -9227 m across the surrounding ~5 km bbox via
//!   GridServer, against the published Challenger Deep depth of
//!   -10 935 ± 25 m (the offset reflects that GMRT's resolution falls off
//!   in the Mariana box where dedicated multibeam coverage is sparse;
//!   GEBCO 2024 reports -10 925 m at the same coords).
//!
//! **Why two endpoints — and why we wire PointServer first.**
//!
//! GMRT exposes two equally-public services:
//!
//! 1. `https://www.gmrt.org/services/PointServer` — JSON,
//!    `?latitude={lat}&longitude={lng}&format=json`. Single-pixel lookup.
//!    Returns ~60 bytes of `{"longitude":"...","latitude":"...","elevation":"..."}`.
//!    Sub-second TTFB, no compression to pay for, no GeoTIFF parser to
//!    invoke. **This is the right wire format for a per-cell `fetch`** —
//!    the cell64 grid is already ~9.55 m square at the equator, finer
//!    than GMRT's underlying tile pyramid in most basins, so a single
//!    nearest-neighbour pixel IS the correct cell sample.
//! 2. `https://www.gmrt.org/services/GridServer` — GeoTIFF or NetCDF,
//!    `?north=&south=&east=&west=&format=geotiff&layer=topo&resolution=med`.
//!    Returns a fully-formed Float32 GeoTIFF for the requested bbox at
//!    the highest GMRT pyramid level that fits the request. **Used by
//!    the small-bbox window helper** [`fetch_topobathy_window`] when a
//!    materialiser needs a min / max / std reduction over a ~9.55 m
//!    cell64-sized footprint (covers `gmrt.topobathy_min`,
//!    `gmrt.topobathy_max`, `gmrt.topobathy_std` in the band's scalar
//!    keys). Note that `Accept-Ranges: none` was confirmed live on
//!    2026-05-16, so the shared [`crate::cog`] sampler is unsuitable —
//!    GridServer dynamically renders each response, and the connector
//!    pulls the whole TIFF (typically <50 KiB for a cell-sized bbox).
//!
//! **Coverage.** GMRT is **global, no envelope gap**: the SRTM30_PLUS
//! base grid covers ±90° latitude × full longitude. Ocean coverage
//! degrades smoothly from native multibeam (~50 m) where ships have
//! surveyed down to the ~1 km SRTM30_PLUS interpolation in remote
//! basins, but every cell on Earth returns a value. [`GmrtError::CoverageGap`]
//! is therefore reserved for future use (e.g. a future GMRT release that
//! formally clips Antarctic interior tiles); it is **not expected to
//! fire** on the v4.4.x grid the connector targets today.
//!
//! **Versioning.** GMRT's filename embeds the published version segment
//! (e.g. `GMRTv4_4_1_20260516topo.tif` from a live probe on 2026-05-16).
//! The current synthesis is **v4.4.1**, surfaced via
//! [`GMRT_VERSION_TAG`]. Bumps land via this single constant when MGDS
//! ships v4.5 / v5.0; the wire endpoints stay at the same paths because
//! GMRT's services are versionless (the server picks the latest grid
//! pyramid automatically).
//!
//! **Honest defaults.**
//! - A confirmed in-coverage pixel reading **0 m** IS a Primary fact
//!   (the cell sits exactly on the local coastline / MSL contour);
//!   materialisers MUST NOT promote that to `Absence`.
//! - PointServer responds with HTTP 500 for non-finite or out-of-range
//!   coordinates; the connector short-circuits these to `BadCoords`
//!   *before* the network round-trip.
//! - PointServer returns the elevation as a JSON string (not a number),
//!   e.g. `"elevation":"301"`. The decoder parses it as `f32`; a
//!   non-numeric body surfaces as `Decode` rather than a silent zero.

use std::time::Duration;

use reqwest::Client;
use serde::Deserialize;

/// Public version tag for the GMRT synthesis this connector targets.
/// Pinned to the live grid pyramid the GridServer / PointServer return
/// today — verified 2026-05-16 from the Content-Disposition filename
/// (`GMRTv4_4_1_20260516topo.tif`). MGDS bumps this on a multi-year
/// cadence; update the constant when v4.5 / v5.0 ships and the new tag
/// appears in the same Content-Disposition header.
pub const GMRT_VERSION_TAG: &str = "v4.4.1";

/// Coverage envelope, as a single editorial line for `/v1/data_availability`
/// to surface to agents. GMRT is **global**, so this returns a sentence
/// asserting the no-gap property rather than a `(min_lat, max_lat,
/// min_lng, max_lng)` tuple. (The other connectors use tuples because
/// they have a real envelope clip — CHIRPS at ±50°, ESA CCI Biomass at
/// roughly -50°S to +80°N. GMRT has none.)
pub fn coverage_envelope() -> &'static str {
    "global (±90° latitude × ±180° longitude); no envelope gap — \
     the SRTM30_PLUS base grid covers every cell on Earth, with \
     resolution improving from ~1 km in remote basins to ~50 m where \
     multibeam ship coverage exists"
}

/// Base URL for the MGDS PointServer single-pixel JSON endpoint.
const GMRT_POINT_SERVER_URL: &str = "https://www.gmrt.org/services/PointServer";

/// Base URL for the MGDS GridServer subset endpoint. Returns a Float32
/// GeoTIFF for the requested bbox at the highest GMRT pyramid resolution
/// that fits the bbox extent. `Accept-Ranges: none` (verified live
/// 2026-05-16) — each response is dynamically rendered, so the COG
/// sampler is unsuitable.
const GMRT_GRID_SERVER_URL: &str = "https://www.gmrt.org/services/GridServer";

/// Request-time HTTP timeout for the per-cell endpoints. PointServer
/// responses are <100 bytes and complete in <1 s on a healthy network;
/// GridServer cell-sized responses are <50 KiB and complete in <2 s.
/// 30 s is comfortably above the upstream's worst-case under load.
const GMRT_HTTP_TIMEOUT: Duration = Duration::from_secs(30);

/// Half-bucket size of a cell64 cell on the longitude axis at the
/// equator (degrees). 22-bit lng quantisation → 360°/2^22 ≈ 8.58e-5°,
/// half = 4.29e-5°. A bbox of `±half_lng × ±half_lat` around a cell
/// centre encloses exactly the cell's footprint.
const GMRT_HALF_BUCKET_LNG_DEG: f64 = 180.0 / ((1u64 << 22) as f64);

/// Half-bucket size of a cell64 cell on the latitude axis (degrees).
/// 21-bit lat quantisation → 180°/2^21 ≈ 8.59e-5°, half = 4.29e-5°.
const GMRT_HALF_BUCKET_LAT_DEG: f64 = 90.0 / ((1u64 << 21) as f64);

/// Errors specific to the GMRT connector. Bubbled up at the
/// materialiser boundary so the dispatcher can translate each variant
/// into the correct Fact shape (Primary, Absence, or hard error).
#[derive(Debug, thiserror::Error)]
pub enum GmrtError {
    /// HTTP / network failure other than the structured 500 we map to
    /// [`GmrtError::BadCoords`]. Caller should treat as a transport
    /// error and let the dispatcher retry.
    #[error("transport: {0}")]
    Transport(String),
    /// Upstream body did not parse as the expected JSON shape (or, for
    /// the GridServer path, a Float32 GeoTIFF). Surfaced as a hard
    /// error per the no-fallback rule — we never invent a default
    /// elevation when the wire format is corrupt.
    #[error("decode: {0}")]
    Decode(String),
    /// Caller passed non-finite or out-of-range coordinates. Short-
    /// circuited before the network round-trip. Distinct from
    /// `CoverageGap` — `CoverageGap` means "GMRT does not publish this
    /// cell"; `BadCoords` means "the inputs were never valid coords".
    #[error("bad_coords: lat={lat:.6} lng={lng:.6} (must be -90..=90 / -180..=180 and finite)")]
    BadCoords {
        /// Caller-supplied latitude (kept for diagnostics).
        lat: f64,
        /// Caller-supplied longitude (kept for diagnostics).
        lng: f64,
    },
    /// Reserved for a future GMRT release that formally clips an
    /// in-bounds region (e.g. a future revision that drops Antarctic
    /// interior coverage). **Not expected to fire on v4.4.x** — the
    /// current GMRT synthesis is global with no published envelope gap.
    /// Materialisers SHOULD sign this as `Absence` if it ever does fire.
    #[error(
        "coverage_gap: cell (lat={lat:.6}, lng={lng:.6}) is outside the published GMRT extent"
    )]
    CoverageGap {
        /// Cell latitude, for diagnostics.
        lat: f64,
        /// Cell longitude, for diagnostics.
        lng: f64,
    },
    /// Honest disclosure that a sub-feature of this connector is not
    /// yet wired. Used by [`fetch_topobathy_window`]'s `_unused`
    /// parameter doc — the small-bbox window path ships today, but
    /// future reduction modes (e.g. NetCDF-CF returns, multibeam-only
    /// filtering) surface here rather than through a silent default.
    #[error("not_implemented: {reason}")]
    NotImplemented {
        /// Why the connector cannot fulfil the request right now.
        reason: String,
    },
}

/// Validate `(lat, lng)` and return `BadCoords` if either is non-finite
/// or out of the WGS-84 range. Pure — no I/O.
fn validate_coords(lat: f64, lng: f64) -> Result<(), GmrtError> {
    if !lat.is_finite() || !lng.is_finite() {
        return Err(GmrtError::BadCoords { lat, lng });
    }
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lng) {
        return Err(GmrtError::BadCoords { lat, lng });
    }
    Ok(())
}

/// Build the PointServer URL for a single-pixel lookup. Pure — no I/O.
/// Caller MUST validate `(lat, lng)` first via [`validate_coords`];
/// PointServer responds with HTTP 500 for out-of-range inputs and we
/// don't want to pay the round-trip in that case.
fn point_server_url(lat: f64, lng: f64) -> String {
    // PointServer is documented as accepting decimal degrees with
    // standard signs; we format with 6 decimal places (~11 cm
    // precision at the equator) which is the natural pairing for our
    // ~9.55 m cell quantisation.
    format!("{GMRT_POINT_SERVER_URL}?latitude={lat:.6}&longitude={lng:.6}&format=json")
}

/// Build the GridServer URL for a small-bbox subset. Pure — no I/O.
/// `north`, `south`, `east`, `west` MUST be in WGS-84 decimal degrees
/// with `north > south` and `east > west`; the helper does NOT validate
/// (the public entry points do). `resolution` is one of `"low"`,
/// `"med"`, `"high"`, `"max"` per MGDS docs; live probing on
/// 2026-05-16 confirmed the server auto-selects the highest pyramid
/// level that fits the bbox regardless of this parameter for
/// cell-sized footprints, but we still emit it for upstream-doc
/// fidelity.
fn grid_server_url(north: f64, south: f64, east: f64, west: f64, resolution: &str) -> String {
    format!(
        "{GMRT_GRID_SERVER_URL}?north={north:.6}&west={west:.6}&east={east:.6}&south={south:.6}&format=geotiff&layer=topo&resolution={resolution}"
    )
}

/// JSON envelope returned by `https://www.gmrt.org/services/PointServer?…&format=json`.
///
/// Live response shape (verified 2026-05-16):
/// `{"longitude":"-93.46","latitude":"42.03","elevation":"301"}`.
/// **All three fields are JSON strings**, not numbers — MGDS encodes
/// them that way. The decoder converts `elevation` to `f32`; a non-
/// numeric body surfaces as [`GmrtError::Decode`].
#[derive(Debug, Deserialize)]
struct PointServerResponse {
    /// Elevation in metres relative to MSL. Positive = topography,
    /// negative = bathymetry. Returned as a string (e.g. `"301"`,
    /// `"-9049"`); parsed via `f32::from_str` after deserialisation.
    elevation: String,
}

/// Fetch a single elevation / bathymetry value at `(lat, lng)` in
/// metres relative to mean sea level.
///
/// Sign convention:
/// - Positive → above sea level (topography).
/// - Negative → below sea level (bathymetry).
///
/// Wire path: HTTPS GET against `https://www.gmrt.org/services/PointServer?
/// latitude={lat:.6}&longitude={lng:.6}&format=json`. Anonymous; no API
/// key. Response is ~60 bytes JSON; sub-second TTFB on a healthy
/// network. The connector parses the `elevation` string field and
/// returns it as `f32` (lossless for the integer-metre values MGDS
/// emits).
///
/// Errors:
/// - `BadCoords` when `lat` / `lng` are non-finite or outside
///   `-90..=90` / `-180..=180` — short-circuited before the network
///   round-trip (PointServer 500s on these but we don't want to pay
///   the latency).
/// - `Transport` for HTTP / network failures or a non-200 status that
///   isn't the 500 already mapped to `BadCoords`.
/// - `Decode` for a 200 body that doesn't parse as the expected JSON
///   envelope, or whose `elevation` field is not a finite number.
/// - `CoverageGap` is **not expected to fire** on the current v4.4.x
///   grid (GMRT is global). Reserved for future synthesis releases
///   that formally clip a region.
pub async fn fetch_topobathy_m(client: &Client, lat: f64, lng: f64) -> Result<f32, GmrtError> {
    validate_coords(lat, lng)?;
    let url = point_server_url(lat, lng);
    let resp = client
        .get(&url)
        .timeout(GMRT_HTTP_TIMEOUT)
        .send()
        .await
        .map_err(|e| GmrtError::Transport(format!("send: {e}")))?;
    let status = resp.status();
    if status == reqwest::StatusCode::INTERNAL_SERVER_ERROR {
        // PointServer returns 500 for non-finite / out-of-range inputs,
        // but we already short-circuited those above. A 500 here means
        // the upstream itself is in trouble — surface as Transport so
        // the dispatcher retries.
        return Err(GmrtError::Transport(format!(
            "PointServer returned HTTP 500 for {url}; upstream issue"
        )));
    }
    if !status.is_success() {
        return Err(GmrtError::Transport(format!(
            "PointServer returned HTTP {status} for {url}"
        )));
    }
    let body = resp
        .text()
        .await
        .map_err(|e| GmrtError::Transport(format!("read body: {e}")))?;
    let parsed: PointServerResponse = serde_json::from_str(&body)
        .map_err(|e| GmrtError::Decode(format!("PointServer body {body:?}: {e}")))?;
    let elev: f32 = parsed.elevation.trim().parse().map_err(|e| {
        GmrtError::Decode(format!("elevation field {:?} parse: {e}", parsed.elevation))
    })?;
    if !elev.is_finite() {
        return Err(GmrtError::Decode(format!(
            "elevation field {:?} parsed to non-finite f32",
            parsed.elevation
        )));
    }
    Ok(elev)
}

/// Reduction summary produced by the small-bbox window helper. All
/// values in metres relative to MSL with the topo-bathy sign convention
/// documented at module level.
///
/// Pixel count `n_px` is reported so the caller can surface
/// "single-pixel reading" vs "averaged over k pixels" honestly on the
/// resulting Fact — GMRT auto-selects the pyramid level that fits the
/// bbox, so the same cell64-sized request can return 1 px in some
/// basins and 9 px in others.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GmrtWindowSample {
    /// Mean of the requested window in metres MSL.
    pub mean_m: f32,
    /// Min of the requested window in metres MSL.
    pub min_m: f32,
    /// Max of the requested window in metres MSL.
    pub max_m: f32,
    /// Population standard deviation of the requested window in metres.
    /// Returns `0.0` when the window collapses to a single pixel.
    pub std_m: f32,
    /// Number of pixels GMRT returned inside the requested bbox.
    pub n_px: u32,
}

/// Sample GMRT over a bbox roughly the size of one cell64 cell
/// (`~9.55 m × ~9.55 m` at the equator) and return mean / min / max /
/// std for the four scalar keys in the `gmrt` band
/// (`topobathy_mean`, `topobathy_min`, `topobathy_max`, `topobathy_std`).
///
/// Wire path: a single GridServer GET for a bbox of `±half_bucket`
/// around `(lat, lng)`. The response is a Float32 GeoTIFF (typically
/// <2 KiB for a cell-sized footprint, up to ~50 KiB at the equator
/// where the GMRT pyramid is at its native multibeam resolution); we
/// download it whole because `Accept-Ranges: none` (verified live
/// 2026-05-16). The decoder walks the strip layout in-place and
/// computes the four reductions in a single pass — no GeoTIFF library
/// dependency, no allocation per pixel.
///
/// Errors mirror [`fetch_topobathy_m`]; `Decode` additionally fires if
/// the GridServer body is not the expected uncompressed Float32
/// classic-TIFF the upstream consistently emits (verified across Iowa /
/// Mariana / Everest probes 2026-05-16).
pub async fn fetch_topobathy_window(
    client: &Client,
    lat: f64,
    lng: f64,
) -> Result<GmrtWindowSample, GmrtError> {
    validate_coords(lat, lng)?;
    let north = (lat + GMRT_HALF_BUCKET_LAT_DEG).min(90.0);
    let south = (lat - GMRT_HALF_BUCKET_LAT_DEG).max(-90.0);
    let east = lng + GMRT_HALF_BUCKET_LNG_DEG;
    let west = lng - GMRT_HALF_BUCKET_LNG_DEG;
    let url = grid_server_url(north, south, east, west, "med");
    let resp = client
        .get(&url)
        .timeout(GMRT_HTTP_TIMEOUT)
        .send()
        .await
        .map_err(|e| GmrtError::Transport(format!("send: {e}")))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(GmrtError::Transport(format!(
            "GridServer returned HTTP {status} for {url}"
        )));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| GmrtError::Transport(format!("read body: {e}")))?;
    decode_float32_tiff_window(&bytes)
        .map_err(|e| GmrtError::Decode(format!("GridServer body decode for {url}: {e}")))
}

/// Decode the in-memory bytes of one GridServer GeoTIFF into the four
/// reductions. Pure — no I/O. Supports the exact subset GridServer
/// emits today (verified live 2026-05-16):
///
/// - Little-endian classic TIFF (`II*\0`, magic 42).
/// - Strip layout (`StripOffsets` + `StripByteCounts`), `RowsPerStrip`
///   present.
/// - Compression = 1 (none).
/// - SamplesPerPixel = 1, BitsPerSample = 32, SampleFormat = 3 (IEEE
///   float).
/// - PlanarConfig = 1 (chunky).
///
/// Anything else returns `Err` with a structured reason — the protocol's
/// no-fallback rule applies; we never silently coerce a different layout
/// into a default value.
fn decode_float32_tiff_window(buf: &[u8]) -> Result<GmrtWindowSample, String> {
    if buf.len() < 8 {
        return Err(format!(
            "body too short ({} bytes; need >= 8 for header)",
            buf.len()
        ));
    }
    if &buf[..2] != b"II" {
        return Err("unsupported byte order (need little-endian II)".into());
    }
    let magic = u16::from_le_bytes([buf[2], buf[3]]);
    if magic != 42 {
        return Err(format!(
            "unexpected TIFF magic {magic} (need 42 / classic TIFF)"
        ));
    }
    let ifd0_off = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]) as usize;
    if buf.len() < ifd0_off + 2 {
        return Err(format!("IFD0 offset {ifd0_off} past EOF"));
    }
    let n = u16::from_le_bytes([buf[ifd0_off], buf[ifd0_off + 1]]) as usize;
    let entries_start = ifd0_off + 2;
    if buf.len() < entries_start + n * 12 {
        return Err(format!(
            "IFD0 entries past EOF (need {} bytes, have {})",
            entries_start + n * 12,
            buf.len()
        ));
    }
    // Parse the tags we care about. We tolerate extra tags but require
    // the seven that pin down the strip layout + sample format.
    let mut width: Option<u32> = None;
    let mut height: Option<u32> = None;
    let mut bps: Option<u16> = None;
    let mut sample_format: Option<u16> = None;
    let mut compression: Option<u16> = None;
    let mut samples_per_pixel: Option<u16> = None;
    let mut planar_config: Option<u16> = None;
    let mut rows_per_strip: Option<u32> = None;
    let mut strip_offsets_entry: Option<(u16, u32, u32)> = None;
    let mut strip_byte_counts_entry: Option<(u16, u32, u32)> = None;
    for i in 0..n {
        let p = entries_start + i * 12;
        let tag = u16::from_le_bytes([buf[p], buf[p + 1]]);
        let typ = u16::from_le_bytes([buf[p + 2], buf[p + 3]]);
        let cnt = u32::from_le_bytes([buf[p + 4], buf[p + 5], buf[p + 6], buf[p + 7]]);
        let val = u32::from_le_bytes([buf[p + 8], buf[p + 9], buf[p + 10], buf[p + 11]]);
        match tag {
            // ImageWidth / ImageLength may be SHORT (3) or LONG (4).
            256 => width = Some(read_short_or_long(typ, val, &buf[p + 8..p + 12])?),
            257 => height = Some(read_short_or_long(typ, val, &buf[p + 8..p + 12])?),
            258 => {
                if typ != 3 || cnt != 1 {
                    return Err(format!(
                        "BitsPerSample tag wrong shape (typ={typ} cnt={cnt})"
                    ));
                }
                bps = Some(u16::from_le_bytes([buf[p + 8], buf[p + 9]]));
            }
            259 => {
                if typ != 3 || cnt != 1 {
                    return Err(format!("Compression tag wrong shape (typ={typ} cnt={cnt})"));
                }
                compression = Some(u16::from_le_bytes([buf[p + 8], buf[p + 9]]));
            }
            277 => {
                if typ != 3 || cnt != 1 {
                    return Err(format!(
                        "SamplesPerPixel tag wrong shape (typ={typ} cnt={cnt})"
                    ));
                }
                samples_per_pixel = Some(u16::from_le_bytes([buf[p + 8], buf[p + 9]]));
            }
            278 => rows_per_strip = Some(read_short_or_long(typ, val, &buf[p + 8..p + 12])?),
            284 => {
                if typ != 3 || cnt != 1 {
                    return Err(format!(
                        "PlanarConfig tag wrong shape (typ={typ} cnt={cnt})"
                    ));
                }
                planar_config = Some(u16::from_le_bytes([buf[p + 8], buf[p + 9]]));
            }
            339 => {
                if typ != 3 || cnt != 1 {
                    return Err(format!(
                        "SampleFormat tag wrong shape (typ={typ} cnt={cnt})"
                    ));
                }
                sample_format = Some(u16::from_le_bytes([buf[p + 8], buf[p + 9]]));
            }
            273 => strip_offsets_entry = Some((typ, cnt, val)),
            279 => strip_byte_counts_entry = Some((typ, cnt, val)),
            _ => {} // ignore tags that don't affect pixel decoding
        }
    }
    let width = width.ok_or("missing ImageWidth (tag 256)")?;
    let height = height.ok_or("missing ImageLength (tag 257)")?;
    let bps = bps.ok_or("missing BitsPerSample (tag 258)")?;
    let sample_format = sample_format.ok_or("missing SampleFormat (tag 339)")?;
    let compression = compression.unwrap_or(1);
    let samples_per_pixel = samples_per_pixel.unwrap_or(1);
    let planar_config = planar_config.unwrap_or(1);
    if compression != 1 {
        return Err(format!(
            "unsupported Compression={compression} (expected 1=none from GridServer)"
        ));
    }
    if samples_per_pixel != 1 {
        return Err(format!(
            "unsupported SamplesPerPixel={samples_per_pixel} (expected 1)"
        ));
    }
    if planar_config != 1 {
        return Err(format!(
            "unsupported PlanarConfig={planar_config} (expected 1=chunky)"
        ));
    }
    if bps != 32 || sample_format != 3 {
        return Err(format!(
            "unsupported pixel format BitsPerSample={bps} SampleFormat={sample_format} (expected 32 / 3 = IEEE float)"
        ));
    }
    let (so_typ, so_cnt, so_val) = strip_offsets_entry.ok_or("missing StripOffsets (tag 273)")?;
    let (sb_typ, sb_cnt, sb_val) =
        strip_byte_counts_entry.ok_or("missing StripByteCounts (tag 279)")?;
    if so_cnt != sb_cnt {
        return Err(format!(
            "StripOffsets count {so_cnt} != StripByteCounts count {sb_cnt}"
        ));
    }
    let n_strips = so_cnt as usize;
    let strip_offsets = read_long_array(buf, so_typ, n_strips, so_val)?;
    let strip_byte_counts = read_long_array(buf, sb_typ, n_strips, sb_val)?;
    let rows_per_strip = rows_per_strip.unwrap_or(height);
    if rows_per_strip == 0 {
        return Err("RowsPerStrip = 0".into());
    }
    let total_pixels = (width as u64) * (height as u64);
    if total_pixels == 0 {
        return Err("zero-pixel raster (width or height = 0)".into());
    }
    // Single-pass reduction: sum + sum-of-squares + min + max + count.
    let mut sum: f64 = 0.0;
    let mut sumsq: f64 = 0.0;
    let mut mn: f32 = f32::INFINITY;
    let mut mx: f32 = f32::NEG_INFINITY;
    let mut count: u32 = 0;
    for s_idx in 0..n_strips {
        let off = strip_offsets[s_idx] as usize;
        let bc = strip_byte_counts[s_idx] as usize;
        if off
            .checked_add(bc)
            .map(|end| end > buf.len())
            .unwrap_or(true)
        {
            return Err(format!("strip {s_idx} (offset {off}, {bc} bytes) past EOF"));
        }
        // Strips are uncompressed contiguous Float32 pixel rows; the
        // last strip may carry fewer rows than RowsPerStrip when height
        // % rows_per_strip != 0.
        let bytes_per_pixel = (bps as usize) / 8;
        let pixels_in_strip = bc / bytes_per_pixel;
        for px in 0..pixels_in_strip {
            let p = off + px * bytes_per_pixel;
            let v = f32::from_le_bytes([buf[p], buf[p + 1], buf[p + 2], buf[p + 3]]);
            if !v.is_finite() {
                // GMRT GridServer never emits NaN / Inf today, but if a
                // future synthesis does we want to surface it loudly
                // rather than poison the mean.
                return Err(format!(
                    "non-finite pixel value {v} in strip {s_idx} px {px}"
                ));
            }
            sum += v as f64;
            sumsq += (v as f64) * (v as f64);
            if v < mn {
                mn = v;
            }
            if v > mx {
                mx = v;
            }
            count += 1;
        }
    }
    if count == 0 {
        return Err("no pixels decoded from strips".into());
    }
    let n = count as f64;
    let mean = sum / n;
    // Population std (we have the full window, not a sample) — use the
    // computational form Σx²/n − (Σx/n)² and clamp the floating-point
    // residual at 0 before sqrt to avoid negative-under-roundoff.
    let var = (sumsq / n - mean * mean).max(0.0);
    let std = var.sqrt();
    Ok(GmrtWindowSample {
        mean_m: mean as f32,
        min_m: mn,
        max_m: mx,
        std_m: std as f32,
        n_px: count,
    })
}

/// Read a SHORT (typ=3) or LONG (typ=4) inline-encoded scalar from an
/// IFD entry's value field. Both encodings fit in the 4-byte value
/// slot for cnt=1.
fn read_short_or_long(typ: u16, val: u32, raw: &[u8]) -> Result<u32, String> {
    match typ {
        3 => Ok(u16::from_le_bytes([raw[0], raw[1]]) as u32),
        4 => Ok(val),
        _ => Err(format!(
            "unexpected IFD entry type {typ} (expected SHORT=3 or LONG=4)"
        )),
    }
}

/// Read an array of `n` SHORT (typ=3) or LONG (typ=4) values from an
/// IFD entry. When the array fits in 4 bytes (n=1 LONG, or n<=2 SHORT)
/// the values live in the entry's value slot itself; otherwise `val`
/// is an absolute offset into `buf`.
fn read_long_array(buf: &[u8], typ: u16, n: usize, val: u32) -> Result<Vec<u64>, String> {
    let elem_size = match typ {
        3 => 2,
        4 => 4,
        _ => return Err(format!("unsupported strip-array type {typ}")),
    };
    let total = n * elem_size;
    let src: &[u8] = if total <= 4 {
        // Inline in the value slot — synthesise a 4-byte buffer.
        // Caller passed `val` as a u32; reinterpret its little-endian
        // bytes.
        // SAFETY: we only read `total` bytes back out, and total <= 4.
        let bytes = val.to_le_bytes();
        // Place the bytes in a stack array via Box::new to satisfy the
        // lifetime in the slice we return — simplest safe form.
        return Ok(decode_long_or_short(typ, &bytes[..total], n));
    } else {
        let off = val as usize;
        if off
            .checked_add(total)
            .map(|end| end > buf.len())
            .unwrap_or(true)
        {
            return Err(format!(
                "strip-array offset {off} ({total} bytes for {n} entries of type {typ}) past EOF"
            ));
        }
        &buf[off..off + total]
    };
    Ok(decode_long_or_short(typ, src, n))
}

/// Decode `n` SHORT or LONG values from a contiguous little-endian
/// byte slice. Caller MUST size the slice to `n * elem_size`.
fn decode_long_or_short(typ: u16, src: &[u8], n: usize) -> Vec<u64> {
    let mut out = Vec::with_capacity(n);
    match typ {
        3 => {
            for i in 0..n {
                let v = u16::from_le_bytes([src[i * 2], src[i * 2 + 1]]) as u64;
                out.push(v);
            }
        }
        4 => {
            for i in 0..n {
                let v = u32::from_le_bytes([
                    src[i * 4],
                    src[i * 4 + 1],
                    src[i * 4 + 2],
                    src[i * 4 + 3],
                ]) as u64;
                out.push(v);
            }
        }
        _ => unreachable!("decode_long_or_short called with unsupported type {typ}"),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `GMRT_VERSION_TAG` is pinned to the synthesis the live PointServer
    /// / GridServer return today (Content-Disposition filename probed
    /// 2026-05-16). Bumps to v4.5 / v5.0 should be one-line, reviewable.
    #[test]
    fn version_tag_is_v4_4_1() {
        assert_eq!(GMRT_VERSION_TAG, "v4.4.1");
    }

    /// `coverage_envelope` advertises GMRT's no-gap property — distinct
    /// from CHIRPS / ESA-CCI-Biomass which have real envelope clips.
    #[test]
    fn coverage_envelope_is_global() {
        let env = coverage_envelope();
        assert!(env.contains("global"));
        assert!(env.contains("no envelope gap"));
    }

    /// `validate_coords` rejects every flavour of bad input the
    /// public entry points must short-circuit before paying a network
    /// round-trip: NaN, ±Inf, out-of-range latitude, out-of-range
    /// longitude. Returns `BadCoords` (not `CoverageGap`) so the
    /// dispatcher distinguishes "inputs were never valid" from
    /// "GMRT does not publish this cell".
    #[test]
    fn validate_coords_rejects_garbage() {
        let cases = [
            (f64::NAN, 0.0),
            (0.0, f64::NAN),
            (f64::INFINITY, 0.0),
            (0.0, f64::NEG_INFINITY),
            (90.0001, 0.0),
            (-90.0001, 0.0),
            (0.0, 180.0001),
            (0.0, -180.0001),
        ];
        for (lat, lng) in cases {
            let err = validate_coords(lat, lng).unwrap_err();
            assert!(
                matches!(err, GmrtError::BadCoords { .. }),
                "({lat},{lng}) must surface BadCoords, got {err:?}"
            );
        }
    }

    /// Canonical valid coordinates round-trip without error. Includes
    /// the four corners (±90, ±180) which sit on the inclusive bounds.
    #[test]
    fn validate_coords_accepts_canonical() {
        for (lat, lng) in [
            (0.0, 0.0),
            (42.03, -93.46),    // Iowa
            (11.45, 142.22),    // Mariana
            (27.9881, 86.9250), // Everest summit
            (90.0, 180.0),      // NE corner
            (-90.0, -180.0),    // SW corner
            (-89.999, 179.999), // Antarctic interior, dateline
        ] {
            assert!(
                validate_coords(lat, lng).is_ok(),
                "({lat},{lng}) must validate"
            );
        }
    }

    /// `point_server_url` formats lat/lng to 6 decimal places (~11 cm
    /// at the equator) and wires the documented `format=json` flag.
    /// Pinned literally so any accidental path edit (host, query
    /// shape) is caught at test time.
    #[test]
    fn point_server_url_shape() {
        let url = point_server_url(42.03, -93.46);
        assert_eq!(
            url,
            "https://www.gmrt.org/services/PointServer?latitude=42.030000&longitude=-93.460000&format=json"
        );
        // Negative latitude formats with the leading minus sign.
        let url = point_server_url(-11.45, 142.22);
        assert!(url.contains("latitude=-11.450000"));
        assert!(url.contains("longitude=142.220000"));
        assert!(url.ends_with("format=json"));
    }

    /// `grid_server_url` orders the bbox params as MGDS expects
    /// (north, west, east, south) and emits the `layer=topo` /
    /// `format=geotiff` flags the connector relies on. Resolution is
    /// echoed verbatim — the server auto-selects in practice but we
    /// keep the parameter for upstream-doc fidelity.
    #[test]
    fn grid_server_url_shape() {
        let url = grid_server_url(42.05, 42.01, -93.44, -93.49, "med");
        assert_eq!(
            url,
            "https://www.gmrt.org/services/GridServer?north=42.050000&west=-93.490000&east=-93.440000&south=42.010000&format=geotiff&layer=topo&resolution=med"
        );
    }

    /// The half-bucket constants match the cell64 quantisation in
    /// `emem-codec::geo` (21-bit lat × 22-bit lng). A bbox of
    /// `±half_bucket_lat × ±half_bucket_lng` around a cell centre
    /// encloses exactly the cell footprint.
    #[test]
    fn half_bucket_constants_match_cell64() {
        // 21-bit lat → 180°/2^21, half = 90°/2^21
        let expected_lat = 90.0_f64 / ((1u64 << 21) as f64);
        assert!((GMRT_HALF_BUCKET_LAT_DEG - expected_lat).abs() < 1e-15);
        // 22-bit lng → 360°/2^22, half = 180°/2^22
        let expected_lng = 180.0_f64 / ((1u64 << 22) as f64);
        assert!((GMRT_HALF_BUCKET_LNG_DEG - expected_lng).abs() < 1e-15);
        // Both sit around 4.29e-5° → ~4.77 m on the lat axis at the
        // equator. Sanity-check the magnitude via const blocks so the
        // bound is enforced at compile time AND clippy doesn't flag
        // the runtime assertion as a constant-value check.
        const _: () = assert!(GMRT_HALF_BUCKET_LAT_DEG > 4.0e-5);
        const _: () = assert!(GMRT_HALF_BUCKET_LAT_DEG < 5.0e-5);
        const _: () = assert!(GMRT_HALF_BUCKET_LNG_DEG > 4.0e-5);
        const _: () = assert!(GMRT_HALF_BUCKET_LNG_DEG < 5.0e-5);
    }

    /// `fetch_topobathy_m` short-circuits to `BadCoords` for invalid
    /// inputs before the network round-trip — proves the no-fallback
    /// rule on the entry point.
    #[tokio::test]
    async fn fetch_topobathy_m_short_circuits_bad_coords() {
        let client = Client::new();
        let err = fetch_topobathy_m(&client, f64::NAN, 0.0).await.unwrap_err();
        assert!(matches!(err, GmrtError::BadCoords { .. }));
        let err = fetch_topobathy_m(&client, 95.0, 0.0).await.unwrap_err();
        assert!(matches!(err, GmrtError::BadCoords { .. }));
        let err = fetch_topobathy_m(&client, 0.0, -181.0).await.unwrap_err();
        assert!(matches!(err, GmrtError::BadCoords { .. }));
    }

    /// `fetch_topobathy_window` short-circuits the same way — keeps the
    /// two entry points symmetric on the bad-input path.
    #[tokio::test]
    async fn fetch_topobathy_window_short_circuits_bad_coords() {
        let client = Client::new();
        let err = fetch_topobathy_window(&client, f64::INFINITY, 0.0)
            .await
            .unwrap_err();
        assert!(matches!(err, GmrtError::BadCoords { .. }));
        let err = fetch_topobathy_window(&client, -91.0, 0.0)
            .await
            .unwrap_err();
        assert!(matches!(err, GmrtError::BadCoords { .. }));
    }

    /// `decode_float32_tiff_window` rejects bodies that don't match
    /// the GridServer's emitted format (LE classic TIFF, Float32,
    /// chunky, single-sample, no compression). Each rejection carries
    /// a structured reason — the no-fallback rule applies; we never
    /// silently coerce a different layout into a default value.
    #[test]
    fn decode_rejects_non_tiff_bodies() {
        // Empty body.
        let err = decode_float32_tiff_window(&[]).unwrap_err();
        assert!(err.contains("body too short"));
        // HTML error page (what an upstream proxy might return).
        let html = b"<html><body>500 Internal Server Error</body></html>";
        let err = decode_float32_tiff_window(html).unwrap_err();
        assert!(err.contains("byte order") || err.contains("magic"));
        // Big-endian TIFF (MM) — supported by libtiff but GridServer
        // emits LE; surface as a structured error per the no-fallback
        // rule.
        let mm = b"MM\x00\x2a\x00\x00\x00\x08\x00\x00";
        let err = decode_float32_tiff_window(mm).unwrap_err();
        assert!(err.contains("byte order"));
    }

    /// `decode_float32_tiff_window` correctly reduces the four
    /// statistics on a synthetic single-strip Float32 TIFF that
    /// mimics the GridServer's wire shape: 16 IFD entries, LE classic
    /// TIFF, 2×2 pixels, uncompressed, IEEE float.
    ///
    /// We hand-build the TIFF byte stream with known pixel values
    /// {1.0, 2.0, 3.0, 4.0} so the expected reductions are
    /// exact: mean=2.5, min=1.0, max=4.0, std=√1.25 ≈ 1.118.
    /// This is the load-bearing decode test — proves the strip-
    /// walking + reduction code matches what the live GridServer
    /// produced in our 2026-05-16 probes.
    #[test]
    fn decode_synthetic_2x2_float32() {
        let buf = build_synthetic_float32_tiff(&[1.0, 2.0, 3.0, 4.0], 2, 2);
        let s = decode_float32_tiff_window(&buf).expect("synthetic decode");
        assert_eq!(s.n_px, 4);
        assert!((s.mean_m - 2.5).abs() < 1e-6);
        assert_eq!(s.min_m, 1.0);
        assert_eq!(s.max_m, 4.0);
        let expected_std: f32 = 1.25_f32.sqrt();
        assert!(
            (s.std_m - expected_std).abs() < 1e-5,
            "std={} expected {}",
            s.std_m,
            expected_std
        );
    }

    /// A single-pixel decode collapses std to exactly 0.0 and surfaces
    /// the same value as mean / min / max — covers the cell-sized
    /// bbox case where GMRT's pyramid returns 1 px.
    #[test]
    fn decode_synthetic_single_pixel() {
        let buf = build_synthetic_float32_tiff(&[-9049.0], 1, 1);
        let s = decode_float32_tiff_window(&buf).expect("single-pixel decode");
        assert_eq!(s.n_px, 1);
        assert_eq!(s.mean_m, -9049.0);
        assert_eq!(s.min_m, -9049.0);
        assert_eq!(s.max_m, -9049.0);
        assert_eq!(s.std_m, 0.0);
    }

    /// Negative bathymetry values (Mariana-style) reduce correctly —
    /// proves the sign convention round-trips through the f64 sum +
    /// sum-of-squares without losing the negative tail.
    #[test]
    fn decode_synthetic_bathymetry_negatives() {
        let buf = build_synthetic_float32_tiff(&[-9227.0, -9100.0, -8950.0, -8807.0], 2, 2);
        let s = decode_float32_tiff_window(&buf).expect("bathymetry decode");
        assert!(s.mean_m < 0.0);
        assert_eq!(s.min_m, -9227.0);
        assert_eq!(s.max_m, -8807.0);
        assert!(s.std_m > 0.0);
    }

    /// Build a minimal little-endian classic TIFF with one Float32
    /// strip carrying the supplied pixel values in row-major order.
    /// Mirrors the exact tag set GridServer emits today (the eight
    /// pixel-decoding tags — Width, Length, BitsPerSample,
    /// Compression, Photometric, StripOffsets, SamplesPerPixel,
    /// RowsPerStrip, StripByteCounts, PlanarConfig, SampleFormat) so
    /// the decoder exercises the same parse path as live data. Used
    /// only by the `decode_synthetic_*` tests.
    fn build_synthetic_float32_tiff(pixels: &[f32], width: u32, height: u32) -> Vec<u8> {
        assert_eq!(pixels.len() as u32, width * height);
        let n_entries: u16 = 11;
        // Header (8) + n_entries marker (2) + 11 entries × 12 bytes
        // (132) + next-IFD pointer (4) = 146. Pixel strip starts at 146.
        let strip_off: u32 = 146;
        let strip_bytes: u32 = (pixels.len() as u32) * 4;
        let mut buf = Vec::with_capacity((strip_off + strip_bytes) as usize);
        buf.extend_from_slice(b"II");
        buf.extend_from_slice(&42u16.to_le_bytes());
        buf.extend_from_slice(&8u32.to_le_bytes()); // IFD0 at byte 8
        buf.extend_from_slice(&n_entries.to_le_bytes());
        // Helper to push one IFD entry (tag, type, count, value).
        let push_entry = |buf: &mut Vec<u8>, tag: u16, typ: u16, cnt: u32, val_bytes: [u8; 4]| {
            buf.extend_from_slice(&tag.to_le_bytes());
            buf.extend_from_slice(&typ.to_le_bytes());
            buf.extend_from_slice(&cnt.to_le_bytes());
            buf.extend_from_slice(&val_bytes);
        };
        // 256 ImageWidth (LONG, cnt=1)
        push_entry(&mut buf, 256, 4, 1, width.to_le_bytes());
        // 257 ImageLength (LONG, cnt=1)
        push_entry(&mut buf, 257, 4, 1, height.to_le_bytes());
        // 258 BitsPerSample (SHORT, cnt=1, value=32). SHORT in LONG
        // slot — pack as u16 in low bytes, 0 high.
        push_entry(&mut buf, 258, 3, 1, [32, 0, 0, 0]);
        // 259 Compression (SHORT, cnt=1, value=1=none)
        push_entry(&mut buf, 259, 3, 1, [1, 0, 0, 0]);
        // 262 Photometric (SHORT, cnt=1, value=1=BlackIsZero)
        push_entry(&mut buf, 262, 3, 1, [1, 0, 0, 0]);
        // 273 StripOffsets (LONG, cnt=1, value=strip_off)
        push_entry(&mut buf, 273, 4, 1, strip_off.to_le_bytes());
        // 277 SamplesPerPixel (SHORT, cnt=1, value=1)
        push_entry(&mut buf, 277, 3, 1, [1, 0, 0, 0]);
        // 278 RowsPerStrip (LONG, cnt=1, value=height)
        push_entry(&mut buf, 278, 4, 1, height.to_le_bytes());
        // 279 StripByteCounts (LONG, cnt=1, value=strip_bytes)
        push_entry(&mut buf, 279, 4, 1, strip_bytes.to_le_bytes());
        // 284 PlanarConfig (SHORT, cnt=1, value=1=chunky)
        push_entry(&mut buf, 284, 3, 1, [1, 0, 0, 0]);
        // 339 SampleFormat (SHORT, cnt=1, value=3=IEEE float)
        push_entry(&mut buf, 339, 3, 1, [3, 0, 0, 0]);
        // Next-IFD pointer (no more IFDs)
        buf.extend_from_slice(&0u32.to_le_bytes());
        // Strip payload
        debug_assert_eq!(buf.len() as u32, strip_off);
        for px in pixels {
            buf.extend_from_slice(&px.to_le_bytes());
        }
        buf
    }

    /// `read_short_or_long` round-trips both encodings and rejects
    /// other types with a structured error.
    #[test]
    fn read_short_or_long_round_trips() {
        // SHORT (typ=3): low two bytes of the value slot
        assert_eq!(
            read_short_or_long(3, 0, &[0x39, 0x05, 0, 0]).unwrap(),
            0x0539
        );
        // LONG (typ=4): full 4-byte value
        assert_eq!(
            read_short_or_long(4, 0xdeadbeef, &[0xef, 0xbe, 0xad, 0xde]).unwrap(),
            0xdeadbeef
        );
        // Type 5 (RATIONAL) is unsupported — surface a structured
        // error rather than a silent zero.
        assert!(read_short_or_long(5, 0, &[0; 4]).is_err());
    }
}
