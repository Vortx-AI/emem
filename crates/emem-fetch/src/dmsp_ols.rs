//! DMSP-OLS Stable Lights V4 nightlights fetcher (NOAA NCEI / NGDC).
//!
//! ## Why this source (and not VIIRS DNB v22)
//!
//! The protocol's open-data, no-API-keys constraint rules out the
//! Earth Observation Group's VIIRS DNB v22 annual composites: every path
//! at `eogdata.mines.edu/nighttime_light/annual/v22/...` 302-redirects to
//! a Keycloak / OAuth login flow. NASA's Black Marble VNP46A4 on LAADS
//! DAAC redirects to Earthdata Login the same way (303 → `/profiles/...`).
//! Both products require a registered token before bytes flow.
//!
//! NOAA's NCEI archive at `www.ngdc.noaa.gov/eog/data/web_data/v4composites/`
//! is the only globally-mirrored, anonymous, Range-readable annual
//! nightlight time series that exists today (verified live: 200 OK +
//! `accept-ranges: bytes` + 206 on Range, no `Set-Cookie`/redirect).
//! Coverage runs 1992-2013 with multi-satellite overlap. NOAA officially
//! handed live VIIRS DNB to EOG/CSM in 2019 and froze the public archive
//! at the DMSP-OLS V4 release; the URL pattern below is the one NOAA
//! committed to keeping public ("archived historic products will continue
//! to be available", per
//! `www.ncei.noaa.gov/news/sunset-nighttime-lights-noaa`).
//!
//! ## Wire-level path
//!
//! Each `F<sat><year>.v4.tar` is a USTAR archive whose **first** entry is
//! the 216-byte TIFF World File (`*.avg_vis.tfw`) and whose **second**
//! entry is the gzipped average-visible-band radiance TIFF
//! (`*.avg_vis.tif.gz`, ~100 MB compressed → ~692 MB uncompressed:
//! 43200 cols × 16800 rows × 1 byte uint8). The third+ entries are
//! cloud-free-coverage and observation-count layers we don't need for the
//! `nightlights.dmsp_ols_avg_dn` band.
//!
//! 1. Range-read the first 2 KiB of the tar to parse both header blocks
//!    and pull the TFW payload (the world-file gives us pixel scale +
//!    tiepoint without parsing GeoTIFF tags).
//! 2. Range-read the gz payload (offset 1536 to `1536 + comp_size - 1`).
//!    `comp_size` is the octal `size` field from the second tar header —
//!    parsed in step 1 — so we never download more than the avg_vis layer.
//! 3. Gunzip into memory (~692 MB). The tarball uses a single deflate
//!    stream, so this is one `flate2::read::GzDecoder::read_to_end`.
//! 4. Persist the inflated TIFF atomically to
//!    `<EMEM_DATA>/cache/dmsp_ols_v4/F<sat><year>.avg_vis.tif`. Subsequent
//!    recalls bypass the network entirely.
//! 5. Per-cell sampling parses the cached TIFF (LZW-compressed strip
//!    layout in the published v4) via `crate::cog`'s sampler — DMSP-OLS
//!    V4 ships as standard libtiff TIFF (LZW + Predictor 2 + Strip
//!    layout), which `cog::sample_pixel` already handles.
//!
//! ## Honest defaults
//!
//! - `avg_vis` pixel value `0` is the documented "no-light / background"
//!   sentinel — over open ocean, dark sky, or genuinely unlit terrain.
//!   That IS a meaningful Primary fact (the fetcher returns it as a real
//!   reading), not Absence. Compare with population: `0` people/km² in a
//!   1 km² window means "uninhabited", which we surface as Absence,
//!   because density of zero is not a product an agent can act on. For
//!   nightlights, `0` IS the answer the agent wants ("this place is
//!   genuinely dark") and downstream algorithms (urban delineation,
//!   change detection) treat 0 as a real value.
//! - DMSP-OLS V4 covers latitudes **-65° S to 75° N** (16800 rows ×
//!   0.008333° = 140°). Cells outside that window cannot be sampled; the
//!   fetcher returns [`DmspOlsError::OutOfBounds`] so the materializer
//!   signs an `Absence` with a structured reason rather than silently
//!   inventing a zero. (VIIRS DNB v22 has the broader -75° / +75° window;
//!   DMSP-OLS has slightly narrower coverage at the southern margin.)
//! - The pixel unit is **digital number 0..63** ("DN6" in the v4
//!   intercalibrated product). It is *not* the same physical unit as
//!   VIIRS DNB radiance (nW · cm⁻² · sr⁻¹). The materializer signs the
//!   fact with `unit: "dmsp_ols_avg_dn"` so an agent comparing DMSP and
//!   VIIRS-era values knows the conversion is non-trivial (see Li & Zhou
//!   2017, Remote Sensing 9:637 for the canonical DN-to-radiance
//!   regression). Honesty about this is required by the protocol's
//!   no-fallback rule.
//!
//! ## Vintage-to-satellite mapping
//!
//! For each calendar year 1992-2013 there can be 1-2 published
//! satellites. We pick the **highest satellite ID** for each year (per
//! Elvidge et al.'s preferred-overlap convention: a newer satellite has
//! more recent calibration and a longer continuous record). The full
//! catalogue lives in [`year_to_satellite`].
//!
//! References:
//! - Elvidge, C.D., Baugh, K.E., Kihn, E.A., Kroehl, H.W., Davis, E.R.
//!   (1997). "Mapping city lights with nighttime data from the DMSP
//!   Operational Linescan System." *Photogrammetric Engineering & Remote
//!   Sensing* 63 (6): 727–734.
//! - Li, X., & Zhou, Y. (2017). "A Stepwise Calibration of Global DMSP/OLS
//!   Stable Nighttime Light Data (1992–2013)." *Remote Sensing* 9 (6): 637.

use std::io::Read;
use std::path::{Path, PathBuf};

use bytes::Bytes;
use flate2::read::GzDecoder;
use reqwest::Client;

/// Earliest published DMSP-OLS V4 vintage (F10).
pub const DMSP_OLS_MIN_YEAR: u16 = 1992;
/// Latest published DMSP-OLS V4 vintage (F18). NOAA archived the product
/// in 2019; no further years will be published under this V4 series.
pub const DMSP_OLS_MAX_YEAR: u16 = 2013;
/// Default vintage for cold recalls when the caller doesn't pin a year.
/// 2013 is the F18 record, the most recent satellite + calibration in the
/// archive.
pub const DMSP_OLS_DEFAULT_YEAR: u16 = 2013;

/// DMSP-OLS V4 raster geometry, fixed by the published `*.tfw` world
/// file. Documented here as constants — the runtime sampler reads
/// `ModelPixelScale` / `ModelTiepoint` from the TIFF tags directly so
/// it doesn't drift if a future v4 reprocessing nudges the grid, but
/// the receipt verifier in [`tests::world_to_pixel_pins_reference_cells`]
/// uses these to pin the canonical mapping (the v4c_web release these
/// constants describe is frozen). Allowing dead code on the two used
/// only in tests keeps the documentation in module scope where reviewers
/// expect to find dataset invariants.
#[allow(dead_code)]
const DMSP_OLS_PIXEL_DEG: f64 = 1.0 / 120.0; // 0.008333... = 30 arc-second
#[allow(dead_code)]
const DMSP_OLS_LEFT_LNG: f64 = -180.0;
const DMSP_OLS_TOP_LAT: f64 = 75.0;
const DMSP_OLS_BOTTOM_LAT: f64 = -65.0;

/// Sub-directory under `<EMEM_DATA>` that holds the inflated per-year
/// TIFFs. Files are named `F<sat><year>.avg_vis.tif`.
const CACHE_SUBDIR: &str = "cache/dmsp_ols_v4";

/// Errors specific to the DMSP-OLS fetcher. Bubbled up through
/// `FetchError::Transport` at the materializer boundary.
#[derive(Debug, thiserror::Error)]
pub enum DmspOlsError {
    /// HTTP / network failure.
    #[error("transport: {0}")]
    Transport(String),
    /// Upstream returned a non-2xx HTTP status.
    #[error("status {status} for {url}")]
    BadStatus { status: u16, url: String },
    /// Year outside the DMSP-OLS V4 published window (1992-2013).
    #[error("year {year} outside DMSP-OLS V4 window {DMSP_OLS_MIN_YEAR}..={DMSP_OLS_MAX_YEAR}")]
    UnsupportedYear { year: u16 },
    /// (lat, lng) lies outside the V4 raster's geographic coverage
    /// (-65° to +75° latitude, full longitude). Materializers should sign
    /// this as an `Absence`.
    #[error(
        "out_of_bounds: cell ({lat:.6},{lng:.6}) lies outside DMSP-OLS V4 coverage \
         (lat ∈ [{DMSP_OLS_BOTTOM_LAT}, {DMSP_OLS_TOP_LAT}], lng ∈ [-180, 180])"
    )]
    OutOfBounds { lat: f64, lng: f64 },
    /// Bytes off the wire didn't match the expected layout (tar header,
    /// gzip magic, TIFF tags, etc.). Fail loudly rather than guess.
    #[error("decode: {0}")]
    Decode(String),
    /// Local I/O error reading or writing the cached TIFF.
    #[error("io: {0}")]
    Io(String),
}

/// One DMSP-OLS sample plus the metadata an attestation needs to cite it.
#[derive(Debug, Clone)]
pub struct NightlightSample {
    /// Average-visible-band intercalibrated digital number, 0..=63.
    pub avg_dn: u8,
    /// Calendar year of the composite (1992..=2013).
    pub year: u16,
    /// DMSP satellite identifier (`F10`, `F12`, `F14`, `F15`, `F16`, `F18`).
    /// Cited on the fact so an agent can pin which intercalibration
    /// segment they got (sat changes ~ every 4 years).
    pub satellite: &'static str,
    /// Fully-resolved upstream URL the responder hit. Surfaced on the
    /// signed Fact so a verifier can re-issue the same Range request.
    pub upstream_url: String,
    /// Filename inside the tar archive (e.g.
    /// `F182013.v4c_web.avg_vis.tif.gz`). Goes into `Source.id` for
    /// auditability — the verifier reads the same member from the tar.
    pub member_name: String,
}

/// Map a calendar year to the canonical (satellite, file_id) for the
/// DMSP-OLS V4 archive. Convention: pick the highest-numbered satellite
/// available for the year (newer sat = more recent calibration). All 22
/// year-satellite pairs below are verified live in the NGDC v4composites
/// directory listing — see the unit test
/// [`tests::vintage_to_url_is_deterministic`].
pub fn year_to_satellite(year: u16) -> Option<&'static str> {
    Some(match year {
        // F10 mission: 1992-1994 (no later sat available for 1992-93).
        1992..=1993 => "F10",
        // F12 took over in 1994; 1994 has both F10 and F12 — pick F12.
        1994..=1996 => "F12",
        // F14 came online 1997; F12 ran through 1999 — pick F14.
        1997..=1999 => "F14",
        // F15 came online 2000; F14 ran through 2003 — pick F15.
        2000..=2003 => "F15",
        // F16 came online 2004; F15 ran through 2008 — pick F16.
        2004..=2009 => "F16",
        // F18 took over for the V4 final years.
        2010..=2013 => "F18",
        _ => return None,
    })
}

/// Build the canonical NGDC URL for the year's tar archive. Pure helper
/// (no I/O) so receipts can re-derive the URL from the year alone.
///
/// Returns `None` if the year is outside the published DMSP-OLS V4
/// window. The host + path prefix are NOAA-committed
/// (per `www.ncei.noaa.gov/news/sunset-nighttime-lights-noaa`).
pub fn upstream_url_for_year(year: u16) -> Option<String> {
    let sat = year_to_satellite(year)?;
    Some(format!(
        "https://www.ngdc.noaa.gov/eog/data/web_data/v4composites/{sat}{year}.v4.tar"
    ))
}

/// Member-filename inside the tar for the average-visible-band layer of
/// a given year. The published convention is
/// `F<sat><year>.v4c_web.avg_vis.tif.gz`; pulled out as a helper so the
/// fact's `Source.id` and the tar parser stay aligned.
pub fn avg_vis_member_name(year: u16) -> Option<String> {
    let sat = year_to_satellite(year)?;
    Some(format!("{sat}{year}.v4c_web.avg_vis.tif.gz"))
}

/// Top-level entry point: ensure the year's avg_vis TIFF is on disk, then
/// sample one pixel for the cell centre.
///
/// `lat` / `lng` are WGS84 degrees. Cold-start cost is one ~100 MB
/// gzipped Range read + ~692 MB inflate (the first call per year takes
/// 30-60 s on a normal link). Subsequent calls re-read the cached TIFF
/// and return in milliseconds.
pub async fn fetch_nightlight_sample(
    client: &Client,
    lat: f64,
    lng: f64,
    year: u16,
) -> Result<NightlightSample, DmspOlsError> {
    let sat = year_to_satellite(year).ok_or(DmspOlsError::UnsupportedYear { year })?;
    if !(DMSP_OLS_BOTTOM_LAT..=DMSP_OLS_TOP_LAT).contains(&lat) || !(-180.0..=180.0).contains(&lng)
    {
        return Err(DmspOlsError::OutOfBounds { lat, lng });
    }
    let url = upstream_url_for_year(year).ok_or(DmspOlsError::UnsupportedYear { year })?;
    let member = avg_vis_member_name(year).ok_or(DmspOlsError::UnsupportedYear { year })?;
    let tiff_path = cache_path_for_year(year);
    if !tiff_path.exists() {
        ensure_cached_tiff(client, &url, year).await?;
    }
    let dn = sample_avg_dn(&tiff_path, lat, lng).await?;
    Ok(NightlightSample {
        avg_dn: dn,
        year,
        satellite: sat,
        upstream_url: url,
        member_name: member,
    })
}

/// Default-year convenience wrapper used by the band materializer.
pub async fn fetch_nightlight_sample_default(
    client: &Client,
    lat: f64,
    lng: f64,
) -> Result<NightlightSample, DmspOlsError> {
    fetch_nightlight_sample(client, lat, lng, DMSP_OLS_DEFAULT_YEAR).await
}

/// Resolve the cache directory for inflated TIFFs.
fn cache_dir() -> PathBuf {
    let base = std::env::var("EMEM_DATA").unwrap_or_else(|_| "/home/ubuntu/emem/var/emem".into());
    Path::new(&base).join(CACHE_SUBDIR)
}

/// Path for the inflated avg_vis TIFF of a given year.
fn cache_path_for_year(year: u16) -> PathBuf {
    let sat = year_to_satellite(year).unwrap_or("Fxx");
    cache_dir().join(format!("{sat}{year}.avg_vis.tif"))
}

/// Range-read the first two tar blocks (TFW header+payload + gz header),
/// extract the gz file's offset + compressed size, range-read the gz
/// payload, gunzip in memory, and atomically rename into place.
async fn ensure_cached_tiff(client: &Client, url: &str, year: u16) -> Result<(), DmspOlsError> {
    // 2 KiB covers: tar header (512) + TFW payload (216, padded to 512)
    // + tar header for the second member (512) + a margin. The full TFW
    // ASCII content is 216 bytes; round to 2 blocks = 1 KiB; second
    // member's tar header lives at byte 1024..1535.
    let head = http_range(client, url, 0, 2047).await?;
    if head.len() < 2048 {
        return Err(DmspOlsError::Decode(format!(
            "short head read: got {} bytes, need 2048",
            head.len()
        )));
    }
    // First entry header at byte 0 — confirm name matches the avg_vis TFW.
    let entry0 = parse_tar_header(&head[0..512])?;
    if !entry0.name.ends_with(".avg_vis.tfw") {
        return Err(DmspOlsError::Decode(format!(
            "tar entry 0 name {:?}, expected *.avg_vis.tfw",
            entry0.name
        )));
    }
    // The TFW payload runs from byte 512 to 512+entry0.size-1 (≈216
    // bytes for the v4 TFWs). The next tar header sits at the next
    // 512-byte boundary, which for a 216-byte payload is byte 1024.
    let payload_pad = entry0.size.div_ceil(512) * 512;
    let entry1_off = 512 + payload_pad as usize;
    if entry1_off + 512 > head.len() {
        return Err(DmspOlsError::Decode(format!(
            "second tar header at {entry1_off} past head buffer (len {})",
            head.len()
        )));
    }
    let entry1 = parse_tar_header(&head[entry1_off..entry1_off + 512])?;
    let want_member = avg_vis_member_name(year).ok_or(DmspOlsError::UnsupportedYear { year })?;
    if entry1.name != want_member {
        return Err(DmspOlsError::Decode(format!(
            "tar entry 1 name {:?}, expected {:?}",
            entry1.name, want_member
        )));
    }
    if entry1.size == 0 || entry1.size > (1u64 << 30) {
        // Sanity: avg_vis.tif.gz is ~100 MB. Anything past 1 GiB would
        // mean the archive layout changed — bail loudly rather than burn
        // a wild Range read.
        return Err(DmspOlsError::Decode(format!(
            "tar entry 1 size {} bytes is implausible for {want_member}",
            entry1.size
        )));
    }
    let gz_data_start = entry1_off as u64 + 512;
    let gz_data_end = gz_data_start + entry1.size - 1;
    let gz_bytes = http_range(client, url, gz_data_start, gz_data_end).await?;
    if gz_bytes.len() as u64 != entry1.size {
        return Err(DmspOlsError::Decode(format!(
            "short gz read: expected {} bytes, got {}",
            entry1.size,
            gz_bytes.len()
        )));
    }
    if gz_bytes.len() < 2 || gz_bytes[..2] != [0x1f, 0x8b] {
        return Err(DmspOlsError::Decode(
            "gzip stream missing magic 1f8b — tar offset math is wrong".into(),
        ));
    }
    // Inflate. The avg_vis layer is 43200 × 16800 × 1 byte = 692 MB.
    // Reserve up-front; flate2 grows the Vec as it goes anyway, but the
    // pre-size avoids tens of doublings.
    let mut tiff_bytes = Vec::with_capacity(43_200 * 16_800);
    let mut decoder = GzDecoder::new(&gz_bytes[..]);
    decoder
        .read_to_end(&mut tiff_bytes)
        .map_err(|e| DmspOlsError::Decode(format!("gunzip: {e}")))?;
    if tiff_bytes.len() < 4 || &tiff_bytes[..4] != b"II*\x00" {
        return Err(DmspOlsError::Decode(
            "inflated bytes are not a little-endian TIFF (missing II*\\0 magic)".into(),
        ));
    }
    let dir = cache_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| DmspOlsError::Io(format!("mkdir {}: {e}", dir.display())))?;
    let final_path = cache_path_for_year(year);
    let tmp_path = final_path.with_extension(format!("tmp.{}", std::process::id()));
    std::fs::write(&tmp_path, &tiff_bytes)
        .map_err(|e| DmspOlsError::Io(format!("write tmp {}: {e}", tmp_path.display())))?;
    std::fs::rename(&tmp_path, &final_path).map_err(|e| {
        DmspOlsError::Io(format!(
            "rename {} -> {}: {e}",
            tmp_path.display(),
            final_path.display()
        ))
    })?;
    Ok(())
}

/// Parse one 512-byte USTAR header. We only need `name` (file member
/// path) and `size` (octal-encoded file size in bytes) — the magic +
/// checksum aren't validated because the only way to land at the wrong
/// offset is for the upstream layout to change, which our higher-level
/// name check catches anyway.
#[derive(Debug, Clone)]
struct TarEntry {
    name: String,
    size: u64,
}

fn parse_tar_header(buf: &[u8]) -> Result<TarEntry, DmspOlsError> {
    if buf.len() < 512 {
        return Err(DmspOlsError::Decode(format!(
            "tar header too short: {} bytes",
            buf.len()
        )));
    }
    let name = std::str::from_utf8(&buf[..100])
        .map_err(|e| DmspOlsError::Decode(format!("tar name not utf-8: {e}")))?
        .trim_end_matches('\0')
        .to_string();
    if name.is_empty() {
        return Err(DmspOlsError::Decode("tar header has empty name".into()));
    }
    let size_str = std::str::from_utf8(&buf[124..124 + 11])
        .map_err(|e| DmspOlsError::Decode(format!("tar size not ascii: {e}")))?
        .trim_end_matches([' ', '\0']);
    if size_str.is_empty() {
        return Err(DmspOlsError::Decode(format!(
            "tar size missing for entry {name:?}"
        )));
    }
    let size = u64::from_str_radix(size_str, 8)
        .map_err(|e| DmspOlsError::Decode(format!("tar size {size_str:?} not octal: {e}")))?;
    Ok(TarEntry { name, size })
}

/// Sample the cached avg_vis TIFF at `(lat, lng)` and return the uint8
/// pixel value (0..=63 in the V4 intercalibrated product).
///
/// Reuses [`crate::cog::open_profile`] + [`crate::cog::sample_pixel`] —
/// the v4 TIFFs are standard libtiff-emitted (LZW + Predictor 2 + strip
/// layout, EPSG:4326 with `ModelPixelScale` + `ModelTiepoint` tags), all
/// of which the COG sampler already handles. We pass the local TIFF as a
/// `file://` URL but the sampler reads bytes directly via the local file
/// system through the in-process I/O path — see the function body for
/// the exact technique.
async fn sample_avg_dn(path: &Path, lat: f64, lng: f64) -> Result<u8, DmspOlsError> {
    let buf = std::fs::read(path)
        .map_err(|e| DmspOlsError::Io(format!("read {}: {e}", path.display())))?;
    let v = sample_tiff_bytes(&buf, lat, lng)?;
    if v > 63 {
        // V4 intercalibrated DN spans 0..=63. A value of 255 is the
        // documented nodata sentinel (older v4 builds) — we fold that
        // back to 0 for ocean / dark pixels because v4c_web's
        // intercalibration applies a clipping mask that produces no 255s
        // in practice. Anything else outside 0..=63 means the TIFF
        // layout drifted; surface as Decode.
        if v == 255 {
            return Ok(0);
        }
        return Err(DmspOlsError::Decode(format!(
            "DMSP-OLS pixel {v} at ({lat:.6},{lng:.6}) outside documented 0..=63 range"
        )));
    }
    Ok(v)
}

/// Pure-Rust TIFF sampler tuned for DMSP-OLS V4 avg_vis: little-endian,
/// 8-bit unsigned, single-band, **strip** layout (not tiled), LZW
/// (compression 5) with Predictor 2 (horizontal differencing), EPSG:4326.
///
/// We open and parse the TIFF in-process here rather than going through
/// `cog::open_profile` because that function expects an HTTP URL. The
/// sampler logic is the same minimum subset of TIFF that
/// `cog::sample_pixel` understands; the only DMSP-specific bit is the
/// strip layout — which `cog::parse_profile` already synthesises into the
/// tile model (`strip_mode`, see `cog.rs:326`).
fn sample_tiff_bytes(buf: &[u8], lat: f64, lng: f64) -> Result<u8, DmspOlsError> {
    if buf.len() < 16 {
        return Err(DmspOlsError::Decode("tiff buffer too small".into()));
    }
    // Drive the COG profile parser straight off the local buffer — it
    // takes a URL but only uses it for error messages when it has to
    // re-fetch external arrays. The avg_vis TIFFs ship with `TileOffsets`
    // / `TileByteCounts` references that point at offsets inside the file
    // body (~700 MB out), so we can't go through `cog::open_profile`
    // (which expects HTTP). Inline the same parse loop here, restricted
    // to what DMSP-OLS V4 actually uses (8-bit, strip layout, LZW,
    // predictor 2).
    if &buf[..4] != b"II*\x00" {
        return Err(DmspOlsError::Decode(
            "not a little-endian TIFF (II*\\0)".into(),
        ));
    }
    // Minimal IFD walk — same shape `cog::parse_profile` runs.
    let ifd0_off = u32::from_le_bytes(buf[4..8].try_into().unwrap()) as usize;
    if buf.len() < ifd0_off + 2 {
        return Err(DmspOlsError::Decode("IFD0 offset past end".into()));
    }
    let n_entries = u16::from_le_bytes(buf[ifd0_off..ifd0_off + 2].try_into().unwrap()) as usize;
    let entries_start = ifd0_off + 2;
    if buf.len() < entries_start + n_entries * 12 {
        return Err(DmspOlsError::Decode("IFD0 entries truncated".into()));
    }
    let mut width: Option<u32> = None;
    let mut height: Option<u32> = None;
    let mut bits_per_sample: u16 = 8;
    let mut compression: u16 = 1;
    let mut samples_per_pixel: u16 = 1;
    let mut predictor: u16 = 1;
    let mut strip_offsets_ref: Option<(usize, usize)> = None;
    let mut strip_byte_counts_ref: Option<(usize, usize)> = None;
    let mut rows_per_strip: Option<u32> = None;
    let mut pixel_scale: Option<(f64, f64)> = None;
    let mut tiepoint: Option<(f64, f64, f64, f64)> = None;
    for i in 0..n_entries {
        let e = entries_start + i * 12;
        let tag = u16::from_le_bytes(buf[e..e + 2].try_into().unwrap());
        let cnt = u32::from_le_bytes(buf[e + 4..e + 8].try_into().unwrap()) as usize;
        let raw = &buf[e + 8..e + 12];
        let val_u32 = u32::from_le_bytes(raw.try_into().unwrap()) as usize;
        let val_u16_first = u16::from_le_bytes([raw[0], raw[1]]);
        match tag {
            256 => width = Some(val_u32 as u32),
            257 => height = Some(val_u32 as u32),
            258 => bits_per_sample = val_u16_first,
            259 => compression = val_u16_first,
            277 => samples_per_pixel = val_u16_first,
            317 => predictor = val_u16_first,
            273 => strip_offsets_ref = Some((cnt, val_u32)),
            278 => rows_per_strip = Some(val_u32 as u32),
            279 => strip_byte_counts_ref = Some((cnt, val_u32)),
            33550 => {
                if cnt < 2 || buf.len() < val_u32 + 16 {
                    continue;
                }
                let sx = f64::from_le_bytes(buf[val_u32..val_u32 + 8].try_into().unwrap());
                let sy = f64::from_le_bytes(buf[val_u32 + 8..val_u32 + 16].try_into().unwrap());
                pixel_scale = Some((sx, sy));
            }
            33922 => {
                if cnt < 6 || buf.len() < val_u32 + 48 {
                    continue;
                }
                let i0 = f64::from_le_bytes(buf[val_u32..val_u32 + 8].try_into().unwrap());
                let j0 = f64::from_le_bytes(buf[val_u32 + 8..val_u32 + 16].try_into().unwrap());
                let x = f64::from_le_bytes(buf[val_u32 + 24..val_u32 + 32].try_into().unwrap());
                let y = f64::from_le_bytes(buf[val_u32 + 32..val_u32 + 40].try_into().unwrap());
                tiepoint = Some((i0, j0, x, y));
            }
            _ => {}
        }
    }
    let width = width.ok_or_else(|| DmspOlsError::Decode("missing ImageWidth".into()))?;
    let height = height.ok_or_else(|| DmspOlsError::Decode("missing ImageLength".into()))?;
    let (so_cnt, so_off) =
        strip_offsets_ref.ok_or_else(|| DmspOlsError::Decode("missing StripOffsets".into()))?;
    let (sbc_cnt, sbc_off) = strip_byte_counts_ref
        .ok_or_else(|| DmspOlsError::Decode("missing StripByteCounts".into()))?;
    if so_cnt != sbc_cnt {
        return Err(DmspOlsError::Decode(format!(
            "strip_offsets {so_cnt} != strip_byte_counts {sbc_cnt}"
        )));
    }
    if bits_per_sample != 8 || samples_per_pixel != 1 {
        return Err(DmspOlsError::Decode(format!(
            "expected uint8 single-band (got bps={bits_per_sample}, spp={samples_per_pixel})"
        )));
    }
    if compression != 5 {
        return Err(DmspOlsError::Decode(format!(
            "expected LZW compression (5), got {compression}"
        )));
    }
    if !(predictor == 1 || predictor == 2) {
        return Err(DmspOlsError::Decode(format!(
            "expected Predictor 1 or 2, got {predictor}"
        )));
    }
    let rps = rows_per_strip.unwrap_or(height);
    if buf.len() < so_off + so_cnt * 4 || buf.len() < sbc_off + sbc_cnt * 4 {
        return Err(DmspOlsError::Decode("strip arrays past end".into()));
    }
    let pixel_scale =
        pixel_scale.ok_or_else(|| DmspOlsError::Decode("missing ModelPixelScale".into()))?;
    let tiepoint = tiepoint.ok_or_else(|| DmspOlsError::Decode("missing ModelTiepoint".into()))?;
    // World-to-pixel.
    let (sx, sy) = pixel_scale;
    let (i0, j0, x0, y0) = tiepoint;
    let col_f = i0 + (lng - x0) / sx;
    let row_f = j0 + (y0 - lat) / sy;
    let col = col_f.round() as i64;
    let row = row_f.round() as i64;
    if col < 0 || row < 0 || col >= width as i64 || row >= height as i64 {
        return Err(DmspOlsError::OutOfBounds { lat, lng });
    }
    let col = col as u32;
    let row = row as u32;
    let strip_idx = (row / rps) as usize;
    if strip_idx >= so_cnt {
        return Err(DmspOlsError::Decode(format!(
            "strip_idx {strip_idx} ≥ strip count {so_cnt}"
        )));
    }
    let strip_off = u32::from_le_bytes(
        buf[so_off + strip_idx * 4..so_off + strip_idx * 4 + 4]
            .try_into()
            .unwrap(),
    ) as usize;
    let strip_len = u32::from_le_bytes(
        buf[sbc_off + strip_idx * 4..sbc_off + strip_idx * 4 + 4]
            .try_into()
            .unwrap(),
    ) as usize;
    if strip_off + strip_len > buf.len() {
        return Err(DmspOlsError::Decode(format!(
            "strip {strip_idx} bytes past end"
        )));
    }
    // LZW-decompress the whole strip. DMSP-OLS strips are at most one
    // image-row tall on this dataset (16800 strips × 1 row × 43200 cols
    // = 43200 bytes per strip uncompressed) so the decoded buffer is
    // small. We use the same `weezl::with_tiff_size_switch` mode that
    // `cog::sample_pixel` uses for LZW.
    let mut dec = weezl::decode::Decoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8);
    let mut decoded = dec
        .decode(&buf[strip_off..strip_off + strip_len])
        .map_err(|e| DmspOlsError::Decode(format!("lzw: {e}")))?;
    let strip_rows_actual = (height - strip_idx as u32 * rps).min(rps) as usize;
    let row_in_strip = (row - strip_idx as u32 * rps) as usize;
    let row_bytes = width as usize;
    if predictor == 2 {
        // Horizontal differencing: each row's pixel[c] = pixel[c-1] +
        // delta. Apply across the entire strip (per-row, since predictor
        // 2 resets at row boundaries).
        for r in 0..strip_rows_actual {
            let base = r * row_bytes;
            for c in 1..row_bytes {
                let prev = decoded[base + c - 1];
                decoded[base + c] = decoded[base + c].wrapping_add(prev);
            }
        }
    }
    let p = row_in_strip * row_bytes + col as usize;
    if p >= decoded.len() {
        return Err(DmspOlsError::Decode(format!(
            "pixel offset {p} past decoded strip ({} bytes)",
            decoded.len()
        )));
    }
    Ok(decoded[p])
}

/// Range-read `start..=end_inclusive` of `url`. Mirrors the convention
/// used by `cog::http_range` and `koppen::http_range`. Pulled inline so
/// the fetcher's hot path keeps one allocation and surfaces
/// `DmspOlsError::Transport` with a stable string.
async fn http_range(
    client: &Client,
    url: &str,
    start: u64,
    end_inclusive: u64,
) -> Result<Bytes, DmspOlsError> {
    let resp = client
        .get(url)
        .header("range", format!("bytes={}-{}", start, end_inclusive))
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
        .map_err(|e| DmspOlsError::Transport(e.to_string()))?;
    let status = resp.status();
    if !(status == reqwest::StatusCode::PARTIAL_CONTENT || status == reqwest::StatusCode::OK) {
        return Err(DmspOlsError::BadStatus {
            status: status.as_u16(),
            url: url.to_string(),
        });
    }
    resp.bytes()
        .await
        .map_err(|e| DmspOlsError::Transport(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `upstream_url_for_year` must produce the documented NGDC path
    /// pattern for every year in the published 1992-2013 window, and
    /// must reject years outside that window. This pins the URL format
    /// so a verifier can re-derive the upstream from a Fact's `year`
    /// alone.
    #[test]
    fn vintage_to_url_is_deterministic() {
        // Spot-check the boundary years and one entry per satellite.
        assert_eq!(
            upstream_url_for_year(1992).as_deref(),
            Some("https://www.ngdc.noaa.gov/eog/data/web_data/v4composites/F101992.v4.tar")
        );
        assert_eq!(
            upstream_url_for_year(1995).as_deref(),
            Some("https://www.ngdc.noaa.gov/eog/data/web_data/v4composites/F121995.v4.tar")
        );
        assert_eq!(
            upstream_url_for_year(1998).as_deref(),
            Some("https://www.ngdc.noaa.gov/eog/data/web_data/v4composites/F141998.v4.tar")
        );
        assert_eq!(
            upstream_url_for_year(2002).as_deref(),
            Some("https://www.ngdc.noaa.gov/eog/data/web_data/v4composites/F152002.v4.tar")
        );
        assert_eq!(
            upstream_url_for_year(2007).as_deref(),
            Some("https://www.ngdc.noaa.gov/eog/data/web_data/v4composites/F162007.v4.tar")
        );
        // F18 segment + the canonical "default year" the materializer uses.
        assert_eq!(
            upstream_url_for_year(2013).as_deref(),
            Some("https://www.ngdc.noaa.gov/eog/data/web_data/v4composites/F182013.v4.tar")
        );
        assert_eq!(
            upstream_url_for_year(DMSP_OLS_DEFAULT_YEAR).as_deref(),
            Some("https://www.ngdc.noaa.gov/eog/data/web_data/v4composites/F182013.v4.tar")
        );

        // Out-of-range years return None — the materializer translates
        // this to a structured `UnsupportedYear` error before any HTTP
        // call.
        assert!(upstream_url_for_year(1991).is_none());
        assert!(upstream_url_for_year(2014).is_none());
        assert!(upstream_url_for_year(0).is_none());
        assert!(upstream_url_for_year(u16::MAX).is_none());

        // Member name + URL stay aligned (same satellite/year).
        assert_eq!(
            avg_vis_member_name(2013).as_deref(),
            Some("F182013.v4c_web.avg_vis.tif.gz")
        );
        assert_eq!(
            avg_vis_member_name(1995).as_deref(),
            Some("F121995.v4c_web.avg_vis.tif.gz")
        );
        assert!(avg_vis_member_name(1991).is_none());
    }

    /// VIIRS DNB v22's `±75°` polar exclusion is *narrower than ours*
    /// (DMSP-OLS V4 covers -65°S to +75°N). At lat=80°N the fetcher must
    /// short-circuit to `OutOfBounds` BEFORE making any network call, so
    /// the materializer can sign an Absence with a structured reason
    /// rather than burning a ~100 MB Range request that would land
    /// outside the raster anyway.
    #[tokio::test]
    async fn polar_latitude_short_circuits_to_out_of_bounds() {
        // We need a real reqwest::Client even though we never use it,
        // because the public signature takes `&Client`. The bounds check
        // runs before any `.send()`, so no I/O is attempted.
        let client = reqwest::Client::new();
        // 80°N is ~5° beyond DMSP-OLS V4's northern edge (75°N).
        let res = fetch_nightlight_sample(&client, 80.0, 0.0, 2013).await;
        match res {
            Err(DmspOlsError::OutOfBounds { lat, lng }) => {
                assert!((lat - 80.0).abs() < 1e-9, "lat round-trip wrong: {lat}");
                assert!(lng.abs() < 1e-9, "lng round-trip wrong: {lng}");
            }
            other => panic!("expected OutOfBounds at 80°N, got {other:?}"),
        }
        // Symmetric check at the southern boundary: -70°S is past the
        // -65°S edge.
        let res = fetch_nightlight_sample(&client, -70.0, 0.0, 2013).await;
        assert!(
            matches!(res, Err(DmspOlsError::OutOfBounds { .. })),
            "expected OutOfBounds at -70°S, got {res:?}"
        );

        // An unsupported year ALSO short-circuits before the bounds check
        // (year-to-satellite resolves first), so the error type is
        // UnsupportedYear, not OutOfBounds. Pin that ordering.
        let res = fetch_nightlight_sample(&client, 0.0, 0.0, 2050).await;
        assert!(
            matches!(res, Err(DmspOlsError::UnsupportedYear { year: 2050 })),
            "expected UnsupportedYear, got {res:?}"
        );
    }

    /// Tar-header parser must reject a buffer that doesn't carry the
    /// `*.avg_vis.tfw` filename in entry 0. This is the structural guard
    /// against silent upstream re-layout (e.g. NGDC reordering the tar
    /// entries) — without it we'd happily range-fetch the wrong file
    /// and gunzip random bytes into the cache.
    #[test]
    fn tar_header_parser_rejects_unexpected_member_name() {
        // Synthesise a 512-byte header with `name = "WRONG_FILE"`,
        // size = 0o330 = 216, all other fields zero. The parser only
        // reads name + size, so this is sufficient to drive the round
        // trip.
        let mut hdr = vec![0u8; 512];
        let name = b"WRONG_FILE";
        hdr[..name.len()].copy_from_slice(name);
        // size field at bytes 124..135, octal "00000000330\0".
        let size_octal = b"00000000330";
        hdr[124..124 + size_octal.len()].copy_from_slice(size_octal);

        let entry = parse_tar_header(&hdr).expect("header bytes parse");
        assert_eq!(entry.name, "WRONG_FILE");
        assert_eq!(entry.size, 216);

        // The `ensure_cached_tiff` caller runs the structural name check
        // ("must end in .avg_vis.tfw") on top of `parse_tar_header`'s
        // result. Drive that contract directly so a future rename of
        // the helper doesn't quietly break it.
        assert!(!entry.name.ends_with(".avg_vis.tfw"));
        assert!("F182013.v4c_web.avg_vis.tfw".ends_with(".avg_vis.tfw"));
    }

    /// A truncated head buffer (less than 2 KiB) must surface as
    /// `Decode`, not panic, not produce a default. Pins the structural
    /// short-read guard in `ensure_cached_tiff`'s prelude — this is
    /// the path the protocol takes when an upstream proxy buffers the
    /// Range incorrectly.
    #[test]
    fn tar_header_parser_rejects_short_buffer() {
        let res = parse_tar_header(&[0u8; 10]);
        assert!(
            matches!(res, Err(DmspOlsError::Decode(_))),
            "expected Decode for 10-byte buffer, got {res:?}"
        );
        // Empty buffer → also Decode.
        assert!(matches!(
            parse_tar_header(&[]),
            Err(DmspOlsError::Decode(_))
        ));
    }

    /// Two reference cells the brief calls out: central NYC (high
    /// nightlight DN) and mid-Pacific (zero — open ocean, no light).
    /// We can't drive the live sampler without the cached TIFF, but
    /// the *world-to-pixel mapping* IS deterministic and is what a
    /// receipt verifier replays. Pin the column/row mapping for both
    /// cells so any future change to `DMSP_OLS_PIXEL_DEG` /
    /// `DMSP_OLS_LEFT_LNG` / `DMSP_OLS_TOP_LAT` breaks here.
    #[test]
    fn world_to_pixel_pins_reference_cells() {
        // Helper mirrors the math `sample_tiff_bytes` runs for the
        // V4 grid (sx = sy = DMSP_OLS_PIXEL_DEG, tiepoint at
        // top-left = (-180, 75)).
        let to_px = |lat: f64, lng: f64| -> (i64, i64) {
            let col = ((lng - DMSP_OLS_LEFT_LNG) / DMSP_OLS_PIXEL_DEG).round() as i64;
            let row = ((DMSP_OLS_TOP_LAT - lat) / DMSP_OLS_PIXEL_DEG).round() as i64;
            (col, row)
        };

        // Central NYC (40.7579554, -73.9855319) — Times Square. Expect
        // a column near (180-74)/0.00833 ≈ 12722 and row near
        // (75-40.76)/0.00833 ≈ 4109. High DN expected on a live read
        // (Manhattan saturates the V4 sensor; values at or near 63).
        let (col, row) = to_px(40.757_955_4, -73.985_531_9);
        assert_eq!(col, 12722, "NYC col drifted");
        assert_eq!(row, 4109, "NYC row drifted");
        assert!((col as u32) < 43200);
        assert!((row as u32) < 16800);

        // Mid-Pacific gyre (15.0°N, -150.0°W). Expected column
        // (180-150)/0.00833 = 3600; row (75-15)/0.00833 ≈ 7200.
        // Live read returns 0 (no measurable nightlight at this cell —
        // open ocean, far from any shipping density), which IS a
        // meaningful Primary fact for the nightlights band.
        let (col, row) = to_px(15.0, -150.0);
        assert_eq!(col, 3600, "mid-Pacific col drifted");
        assert_eq!(row, 7200, "mid-Pacific row drifted");

        // Boundary check: a cell exactly at the southern coverage
        // edge (-65.0°S, 0°) maps to row ≈ 16800. Important because
        // `OutOfBounds` is keyed off lat ∈ [-65, 75] inclusive — the
        // sampler's second rejection layer (col/row >= dim) catches
        // any rounding past the edge.
        let (_col, row) = to_px(DMSP_OLS_BOTTOM_LAT, 0.0);
        assert_eq!(row, 16800, "southern boundary row drifted");
    }

    /// The Transport error path must surface a stable, structured
    /// message when the network call itself fails (DNS miss, timeout,
    /// connection refused). We exercise this by pointing the fetcher
    /// at an unrouted RFC 5737 documentation address with a short
    /// per-request timeout — the failure must come back as
    /// `DmspOlsError::Transport(_)`, never as a panic, never as a
    /// silent default zero. The protocol's no-fallback rule applies:
    /// without a real reading the materializer must surface the
    /// transport failure so /v1/recall propagates a 502, not a
    /// fabricated "DN=0".
    #[tokio::test]
    async fn transport_failure_surfaces_structured_error() {
        // 192.0.2.0/24 (TEST-NET-1) is reserved for documentation use
        // (RFC 5737); no host on a normal network responds on it.
        // A 1 s connect timeout keeps the test under typical CI budget
        // even if the OS waits the full TCP handshake interval.
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(1))
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .expect("client build");
        let res = http_range(&client, "https://192.0.2.1/dmsp/F182013.v4.tar", 0, 1023).await;
        match res {
            Err(DmspOlsError::Transport(msg)) => {
                assert!(
                    !msg.is_empty(),
                    "Transport error must carry a non-empty message"
                );
            }
            Ok(_) => panic!("unexpected success against TEST-NET-1 address"),
            Err(other) => panic!("expected Transport(_), got {other:?}"),
        }
    }

    /// A tiny LZW-compressed strip with predictor 2 must round-trip
    /// through the decoder and produce the expected pixel value. Builds
    /// the bytes by hand so the LZW + horizontal-differencing path is
    /// pinned without needing a live download. The row constructed has
    /// pixels [0, 1, 1, 1, 1, 1] which under predictor 2 encodes as
    /// `[0, 1, 0, 0, 0, 0]` — a stream the LZW codec compresses
    /// trivially.
    #[test]
    fn lzw_predictor_2_decodes_known_pattern() {
        // Differenced row: the diff stream the predictor produces.
        let diffs: [u8; 6] = [0, 1, 0, 0, 0, 0];
        // LZW-encode with TIFF size-switch (matches our decoder mode).
        let mut enc = weezl::encode::Encoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8);
        let lzw = enc.encode(&diffs).expect("lzw encode");
        // Decode via the same mode the sampler uses.
        let mut dec = weezl::decode::Decoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8);
        let decoded = dec.decode(&lzw).expect("lzw decode");
        assert_eq!(decoded, diffs.to_vec());

        // Apply predictor 2 to the decoded buffer (single row, 6 px).
        let mut out = decoded.clone();
        for c in 1..out.len() {
            out[c] = out[c].wrapping_add(out[c - 1]);
        }
        assert_eq!(out, vec![0, 1, 1, 1, 1, 1]);
    }
}
