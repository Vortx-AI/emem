//! Köppen-Geiger climate-zone fetcher.
//!
//! Source: **Beck, H.E., N.E. Zimmermann, T.R. McVicar, N. Vergopolan,
//! A. Berg, E.F. Wood (2018). *Present and future Köppen-Geiger climate
//! classification maps at 1-km resolution*. Scientific Data 5, 180214.
//! doi:10.1038/sdata.2018.214** — V1, present-day (1980-2016) panel,
//! published as a CC-BY-4.0 dataset on Figshare.
//!
//! The published artefact is a ZIP archive (`Beck_KG_V1.zip`,
//! ~71 MB) containing twelve GeoTIFFs at three resolutions
//! (0.0083°/0.083°/0.5°) for present and future panels with their
//! confidence layers. The 1-km present-day raster
//! (`Beck_KG_V1_present_0p0083.tif`, 43200×21600 px, uint8, PackBits-
//! compressed) is the canonical product an agent that asks
//! "What climate zone is this?" wants.
//!
//! The wire path:
//! 1. Range-read the last ~64 KiB of the ZIP, locate the End-of-
//!    Central-Directory record, walk the central directory to the
//!    `Beck_KG_V1_present_0p0083.tif` entry, and capture its local-
//!    header offset + compressed/uncompressed sizes.
//! 2. Range-read the local file header (~80 B) to skip past name +
//!    extra fields.
//! 3. Range-read the deflate stream (~5.7 MB) and inflate to memory
//!    (~22 MB). The result is a small, self-contained TIFF.
//! 4. Persist that TIFF to `<EMEM_DATA>/cache/koppen/Beck_KG_V1_present_0p0083.tif`
//!    via atomic temp-file rename so subsequent recalls bypass the
//!    fetch entirely. Bytes-on-disk match exactly what
//!    `unzip Beck_KG_V1.zip Beck_KG_V1_present_0p0083.tif` would
//!    produce.
//! 5. Per-cell sampling parses the cached TIFF (small enough to load
//!    fully into memory), PackBits-decodes the single 4320×2160 tile
//!    that contains the requested pixel, and reads one byte. The
//!    integer 1..=30 maps to a Köppen-Geiger class string via
//!    [`KOPPEN_CLASSES`].
//!
//! Honest defaults — pixel value 0 means "outside the land mask"
//! (open ocean, polar interior). The fetcher returns
//! [`KoppenError::NoData`] for that case so the materializer can sign
//! an `Absence` rather than silently picking a default class.

use std::io::Read;
use std::path::{Path, PathBuf};

use bytes::Bytes;
use flate2::read::DeflateDecoder;
use reqwest::Client;

/// Beck et al. 2018 Köppen-Geiger class strings, indexed by the integer
/// pixel value `1..=30` published in `Beck_KG_V1_present_0p0083.tif`.
///
/// The index is `class_int - 1`; the file's `legend.txt` is the
/// authoritative source for this mapping. Reproduced here as a const so
/// the wire path is one allocation-free lookup, and so anyone reviewing
/// the materializer can audit the table in place rather than chasing it
/// across files.
///
/// Citation: Beck et al. 2018, Sci Data 5, 180214 (doi:10.1038/sdata.2018.214).
pub const KOPPEN_CLASSES: [&str; 30] = [
    "Af",  // 1  Tropical, rainforest
    "Am",  // 2  Tropical, monsoon
    "Aw",  // 3  Tropical, savannah
    "BWh", // 4  Arid, desert, hot
    "BWk", // 5  Arid, desert, cold
    "BSh", // 6  Arid, steppe, hot
    "BSk", // 7  Arid, steppe, cold
    "Csa", // 8  Temperate, dry summer, hot summer
    "Csb", // 9  Temperate, dry summer, warm summer
    "Csc", // 10 Temperate, dry summer, cold summer
    "Cwa", // 11 Temperate, dry winter, hot summer
    "Cwb", // 12 Temperate, dry winter, warm summer
    "Cwc", // 13 Temperate, dry winter, cold summer
    "Cfa", // 14 Temperate, no dry season, hot summer
    "Cfb", // 15 Temperate, no dry season, warm summer
    "Cfc", // 16 Temperate, no dry season, cold summer
    "Dsa", // 17 Cold, dry summer, hot summer
    "Dsb", // 18 Cold, dry summer, warm summer
    "Dsc", // 19 Cold, dry summer, cold summer
    "Dsd", // 20 Cold, dry summer, very cold winter
    "Dwa", // 21 Cold, dry winter, hot summer
    "Dwb", // 22 Cold, dry winter, warm summer
    "Dwc", // 23 Cold, dry winter, cold summer
    "Dwd", // 24 Cold, dry winter, very cold winter
    "Dfa", // 25 Cold, no dry season, hot summer
    "Dfb", // 26 Cold, no dry season, warm summer
    "Dfc", // 27 Cold, no dry season, cold summer
    "Dfd", // 28 Cold, no dry season, very cold winter
    "ET",  // 29 Polar, tundra
    "EF",  // 30 Polar, frost
];

/// Upstream URL: Figshare ndownloader endpoint that 302-redirects to the
/// Figshare/AWS S3 object hosting the dataset. Range reads survive the
/// redirect (verified live), which is what makes the partial-fetch path
/// viable. The figshare *www* endpoint is WAF-protected and rejects the
/// initial GET; the *ndownloader* subdomain is not. Both URLs expose the
/// identical archive (verified by 32-byte md5 in the figshare API).
const KOPPEN_ZIP_URL: &str = "https://ndownloader.figshare.com/files/12407516";

/// Filename inside the ZIP for the 1-km present-day classification.
const TIFF_NAME: &str = "Beck_KG_V1_present_0p0083.tif";

/// Sub-directory under `<EMEM_DATA>` that holds the extracted TIFF.
const CACHE_SUBDIR: &str = "cache/koppen";

/// Errors specific to the Köppen fetcher. Bubbled up through
/// `FetchError::Transport` at the materializer boundary so callers don't
/// have to thread two error types.
#[derive(Debug, thiserror::Error)]
pub enum KoppenError {
    /// HTTP / network failure.
    #[error("transport: {0}")]
    Transport(String),
    /// The ZIP central directory didn't contain the expected TIFF.
    /// Usually means the upstream archive was reorganised — investigate
    /// before silently switching files.
    #[error("zip layout: {0}")]
    ZipLayout(String),
    /// The TIFF inside the ZIP didn't match the expected layout (e.g.
    /// dimensions changed, compression switched away from PackBits).
    /// Hard-fail rather than guess.
    #[error("tiff layout: {0}")]
    TiffLayout(String),
    /// Local I/O error reading or writing the cached TIFF.
    #[error("io: {0}")]
    Io(String),
    /// Pixel landed on the dataset's no-data sentinel (value 0). Cell is
    /// outside the Beck land + coastal mask (open ocean, polar
    /// interior). Materializers should sign this as an `Absence`.
    #[error("no_data: cell at ({lat:.6},{lng:.6}) is outside the Beck Köppen-Geiger land mask")]
    NoData { lat: f64, lng: f64 },
    /// The pixel value is non-zero but outside the documented `1..=30`
    /// range. Indicates a parse error or upstream corruption — the
    /// protocol's no-fallback rule applies.
    #[error("class_out_of_range: pixel {value} at ({lat:.6},{lng:.6}) is not a documented Köppen class (1..=30)")]
    ClassOutOfRange { value: u8, lat: f64, lng: f64 },
}

/// One Köppen sample: the integer pixel value (1..=30) and its canonical
/// class string from [`KOPPEN_CLASSES`]. Both are returned so the fact
/// the materializer signs can carry the agent-facing string in `value`
/// while the integer stays available for downstream cube embedding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KoppenClass {
    /// Integer code 1..=30 (matches `legend.txt` from Beck et al. 2018).
    pub code: u8,
    /// Canonical class string (e.g. "Af", "BWh", "Dfb").
    pub label: &'static str,
}

/// Resolve the cache directory for the extracted TIFF. Honors `EMEM_DATA`
/// the same way the rest of the protocol does.
fn cache_dir() -> PathBuf {
    let base = std::env::var("EMEM_DATA").unwrap_or_else(|_| "/home/ubuntu/emem/var/emem".into());
    Path::new(&base).join(CACHE_SUBDIR)
}

/// Path for the cached, fully-extracted Köppen TIFF.
fn cache_path() -> PathBuf {
    cache_dir().join(TIFF_NAME)
}

/// Top-level entry point: ensure the 1-km present-day Köppen TIFF is on
/// disk, then sample one pixel for the requested cell centre.
///
/// `lat` and `lng` are WGS84 degrees. The Beck V1 raster is global
/// EPSG:4326 with a fixed 0.0083° grid origin at (-180.0, 90.0).
///
/// First call from a cold cache pays the one-time ~6 MB download +
/// inflate cost (a few seconds on a normal link). Every subsequent call
/// re-reads the cached file — which the host kernel will keep in page
/// cache after the first warmup — and returns in microseconds.
pub async fn fetch_koppen_class_for_cell(
    client: &Client,
    lat: f64,
    lng: f64,
) -> Result<KoppenClass, KoppenError> {
    let path = cache_path();
    if !path.exists() {
        ensure_cached_tiff(client).await?;
    }
    sample_local_tiff(&path, lat, lng)
}

/// Public so the API layer can surface the upstream URL in receipts'
/// `Source.url` field without duplicating the constant.
pub fn upstream_url() -> &'static str {
    KOPPEN_ZIP_URL
}

/// Public so the API layer can surface the source scheme + filename in
/// `Source.id` for traceability.
pub fn upstream_member_name() -> &'static str {
    TIFF_NAME
}

/// Download the 1-km present-day TIFF from the figshare archive,
/// inflate, and persist atomically to `<EMEM_DATA>/cache/koppen/`.
///
/// Concurrent calls race-but-don't-corrupt: each task writes to a
/// per-task temp file (`*.tmpXXX`) and renames into place. Whichever
/// rename wins becomes the canonical cached TIFF; the loser's bytes are
/// identical (same upstream, same inflate path) so the race is benign.
async fn ensure_cached_tiff(client: &Client) -> Result<(), KoppenError> {
    let entry = locate_member_in_zip(client, KOPPEN_ZIP_URL, TIFF_NAME).await?;
    let lh = read_local_header(client, KOPPEN_ZIP_URL, entry.local_header_offset).await?;
    let data_start = entry.local_header_offset + 30 + lh.name_len as u64 + lh.extra_len as u64;
    let data_end = data_start + entry.compressed_size as u64 - 1;
    let compressed = http_range(client, KOPPEN_ZIP_URL, data_start, data_end).await?;
    if compressed.len() as u32 != entry.compressed_size {
        return Err(KoppenError::Transport(format!(
            "short read of {TIFF_NAME}: expected {} bytes, got {}",
            entry.compressed_size,
            compressed.len()
        )));
    }
    // Inflate the deflate stream (compression method 8 in the central
    // directory). The Köppen TIFF entry is ~5.7 MB compressed, ~22 MB
    // inflated — comfortable for an in-memory inflate.
    let mut decoder = DeflateDecoder::new(&compressed[..]);
    let mut tiff_bytes = Vec::with_capacity(entry.uncompressed_size as usize);
    decoder
        .read_to_end(&mut tiff_bytes)
        .map_err(|e| KoppenError::Transport(format!("inflate: {e}")))?;
    if tiff_bytes.len() as u32 != entry.uncompressed_size {
        return Err(KoppenError::ZipLayout(format!(
            "inflated size mismatch: expected {}, got {}",
            entry.uncompressed_size,
            tiff_bytes.len()
        )));
    }
    if tiff_bytes.get(..4) != Some(b"II*\x00") {
        return Err(KoppenError::TiffLayout(
            "inflated bytes are not a little-endian TIFF (missing II*\\0 magic)".into(),
        ));
    }
    let dir = cache_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| KoppenError::Io(format!("mkdir {}: {e}", dir.display())))?;
    let final_path = cache_path();
    // tokio::task::spawn_blocking would be nicer for the kernel-bound
    // 22 MB write, but std::fs is fine here — this path runs at most
    // once per responder lifetime.
    let tmp_path = final_path.with_extension(format!("tmp.{}", std::process::id()));
    std::fs::write(&tmp_path, &tiff_bytes)
        .map_err(|e| KoppenError::Io(format!("write tmp {}: {e}", tmp_path.display())))?;
    std::fs::rename(&tmp_path, &final_path).map_err(|e| {
        KoppenError::Io(format!(
            "rename {} -> {}: {e}",
            tmp_path.display(),
            final_path.display()
        ))
    })?;
    Ok(())
}

/// Sample the local cached TIFF at `(lat, lng)` and decode the pixel to
/// a [`KoppenClass`].
fn sample_local_tiff(path: &Path, lat: f64, lng: f64) -> Result<KoppenClass, KoppenError> {
    let buf = std::fs::read(path)
        .map_err(|e| KoppenError::Io(format!("read {}: {e}", path.display())))?;
    let v = sample_tiff_bytes(&buf, lat, lng)?;
    if v == 0 {
        return Err(KoppenError::NoData { lat, lng });
    }
    if !(1..=30).contains(&v) {
        return Err(KoppenError::ClassOutOfRange { value: v, lat, lng });
    }
    Ok(KoppenClass {
        code: v,
        label: KOPPEN_CLASSES[(v - 1) as usize],
    })
}

/// Pure-Rust TIFF sampler tuned for the Beck Köppen-Geiger raster:
/// little-endian, 8-bit unsigned, single-band, tiled, PackBits-
/// compressed, EPSG:4326 with a fixed pixel grid. We deliberately keep
/// this private to the module rather than extending `cog.rs` — PackBits
/// is the only TIFF dialect this dataset uses, no other materializer
/// needs it, and inlining keeps the hot path one allocation.
fn sample_tiff_bytes(buf: &[u8], lat: f64, lng: f64) -> Result<u8, KoppenError> {
    if buf.len() < 16 {
        return Err(KoppenError::TiffLayout("file too small".into()));
    }
    if &buf[..4] != b"II*\x00" {
        return Err(KoppenError::TiffLayout(
            "not a little-endian TIFF (II*\\0)".into(),
        ));
    }
    let ifd0 = u32::from_le_bytes(buf[4..8].try_into().unwrap()) as usize;
    if buf.len() < ifd0 + 2 {
        return Err(KoppenError::TiffLayout("IFD0 offset past end".into()));
    }
    let n = u16::from_le_bytes(buf[ifd0..ifd0 + 2].try_into().unwrap()) as usize;
    let entries_start = ifd0 + 2;
    if buf.len() < entries_start + n * 12 {
        return Err(KoppenError::TiffLayout("IFD0 truncated".into()));
    }

    let mut width: Option<u32> = None;
    let mut height: Option<u32> = None;
    let mut bits_per_sample: u16 = 8;
    let mut compression: u16 = 1;
    let mut samples_per_pixel: u16 = 1;
    let mut tile_w: Option<u32> = None;
    let mut tile_h: Option<u32> = None;
    let mut tile_offsets_ref: Option<(usize, usize)> = None;
    let mut tile_byte_counts_ref: Option<(usize, usize)> = None;
    let mut pixel_scale: Option<(f64, f64)> = None;
    let mut tiepoint: Option<(f64, f64, f64, f64)> = None;

    for i in 0..n {
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
            322 => tile_w = Some(val_u32 as u32),
            323 => tile_h = Some(val_u32 as u32),
            324 => tile_offsets_ref = Some((cnt, val_u32)),
            325 => tile_byte_counts_ref = Some((cnt, val_u32)),
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

    let width = width.ok_or_else(|| KoppenError::TiffLayout("missing ImageWidth".into()))?;
    let height = height.ok_or_else(|| KoppenError::TiffLayout("missing ImageLength".into()))?;
    let tile_w = tile_w.ok_or_else(|| KoppenError::TiffLayout("missing TileWidth".into()))?;
    let tile_h = tile_h.ok_or_else(|| KoppenError::TiffLayout("missing TileLength".into()))?;
    let pixel_scale =
        pixel_scale.ok_or_else(|| KoppenError::TiffLayout("missing ModelPixelScale".into()))?;
    let tiepoint =
        tiepoint.ok_or_else(|| KoppenError::TiffLayout("missing ModelTiepoint".into()))?;
    let (toff_cnt, toff_off) =
        tile_offsets_ref.ok_or_else(|| KoppenError::TiffLayout("missing TileOffsets".into()))?;
    let (tbc_cnt, tbc_off) = tile_byte_counts_ref
        .ok_or_else(|| KoppenError::TiffLayout("missing TileByteCounts".into()))?;
    if toff_cnt != tbc_cnt {
        return Err(KoppenError::TiffLayout(format!(
            "tile_offsets cnt {toff_cnt} != tile_byte_counts cnt {tbc_cnt}"
        )));
    }
    if compression != 32773 {
        return Err(KoppenError::TiffLayout(format!(
            "expected PackBits compression (32773), got {compression}"
        )));
    }
    if bits_per_sample != 8 || samples_per_pixel != 1 {
        return Err(KoppenError::TiffLayout(format!(
            "expected uint8 single-band raster (got bps={bits_per_sample}, spp={samples_per_pixel})"
        )));
    }

    // Tile offsets / byte counts arrays.
    let n_tiles = toff_cnt;
    let need_off = toff_off + n_tiles * 4;
    let need_bc = tbc_off + n_tiles * 4;
    if buf.len() < need_off || buf.len() < need_bc {
        return Err(KoppenError::TiffLayout("tile arrays past end".into()));
    }
    let mut tile_offsets: Vec<u64> = Vec::with_capacity(n_tiles);
    let mut tile_byte_counts: Vec<u64> = Vec::with_capacity(n_tiles);
    for k in 0..n_tiles {
        let p = toff_off + k * 4;
        tile_offsets.push(u32::from_le_bytes(buf[p..p + 4].try_into().unwrap()) as u64);
        let p = tbc_off + k * 4;
        tile_byte_counts.push(u32::from_le_bytes(buf[p..p + 4].try_into().unwrap()) as u64);
    }

    let tile_cols = width.div_ceil(tile_w);
    // tile_rows is implicit in n_tiles = tile_cols * tile_rows; we don't
    // need it because the bounds check below is "tile_idx < n_tiles".
    let _tile_rows = height.div_ceil(tile_h);

    // World-to-pixel for EPSG:4326: x = lng, y = lat, sx > 0, sy > 0
    // (north-up: tiepoint.y is the north edge).
    let (sx, sy) = pixel_scale;
    let (i0, j0, x, y) = tiepoint;
    let col_f = i0 + (lng - x) / sx;
    let row_f = j0 + (y - lat) / sy;
    let col = col_f.round() as i64;
    let row = row_f.round() as i64;
    if col < 0 || row < 0 || col >= width as i64 || row >= height as i64 {
        return Err(KoppenError::TiffLayout(format!(
            "world ({lat:.6},{lng:.6}) maps to pixel ({col},{row}) outside image {width}x{height}"
        )));
    }
    let col = col as u32;
    let row = row as u32;
    let tile_col = col / tile_w;
    let tile_row = row / tile_h;
    let tile_idx = (tile_row * tile_cols + tile_col) as usize;
    if tile_idx >= n_tiles {
        return Err(KoppenError::TiffLayout(format!(
            "tile_idx {tile_idx} out of range (have {n_tiles})"
        )));
    }
    let toff = tile_offsets[tile_idx] as usize;
    let tlen = tile_byte_counts[tile_idx] as usize;
    if buf.len() < toff + tlen {
        return Err(KoppenError::TiffLayout(format!(
            "tile {tile_idx} bytes past end"
        )));
    }
    let raw_tile = &buf[toff..toff + tlen];
    let expected = (tile_w as usize) * (tile_h as usize);
    let decoded = packbits_decode(raw_tile, expected)?;
    if decoded.len() < expected {
        return Err(KoppenError::TiffLayout(format!(
            "PackBits-decoded tile too short: {} < {}",
            decoded.len(),
            expected
        )));
    }
    let intra_col = (col - tile_col * tile_w) as usize;
    let intra_row = (row - tile_row * tile_h) as usize;
    let pix = decoded[intra_row * (tile_w as usize) + intra_col];
    Ok(pix)
}

/// PackBits (TIFF compression 32773) — Apple's run-length-encoding
/// scheme. Header byte `n`:
///   - `n` in `0..=127`: copy the next `n + 1` bytes literally.
///   - `n` in `129..=255`: repeat the next byte `257 - n` times.
///   - `n == 128`: no-op (reserved).
///
/// The decoder stops once the requested `expected_len` bytes have been
/// produced; PackBits streams may include trailing junk.
fn packbits_decode(buf: &[u8], expected_len: usize) -> Result<Vec<u8>, KoppenError> {
    let mut out: Vec<u8> = Vec::with_capacity(expected_len);
    let mut i = 0usize;
    while i < buf.len() && out.len() < expected_len {
        let n = buf[i];
        i += 1;
        if n < 128 {
            let count = (n as usize) + 1;
            if i + count > buf.len() {
                return Err(KoppenError::TiffLayout(format!(
                    "packbits: literal {count} past end at {i}"
                )));
            }
            out.extend_from_slice(&buf[i..i + count]);
            i += count;
        } else if n == 128 {
            // no-op
        } else {
            let count = 257 - (n as usize);
            if i >= buf.len() {
                return Err(KoppenError::TiffLayout(format!(
                    "packbits: run header at {i} but no value byte"
                )));
            }
            let v = buf[i];
            i += 1;
            for _ in 0..count {
                out.push(v);
            }
        }
    }
    Ok(out)
}

/// One central-directory entry — just the fields we actually need.
#[derive(Debug, Clone)]
struct ZipEntry {
    compressed_size: u32,
    uncompressed_size: u32,
    local_header_offset: u64,
}

/// Local-file-header parse result.
#[derive(Debug, Clone, Copy)]
struct ZipLocalHeader {
    name_len: u16,
    extra_len: u16,
}

/// Walk the End-of-Central-Directory record + central directory at the
/// end of the ZIP and return the entry matching `name`.
async fn locate_member_in_zip(
    client: &Client,
    url: &str,
    name: &str,
) -> Result<ZipEntry, KoppenError> {
    // ZIP central directories are typically a few KB; pull the last 64 KiB
    // so an unexpectedly long comment doesn't force a second round-trip.
    const TAIL_BYTES: u64 = 65_536;
    let total = http_content_length(client, url).await?;
    let start = total.saturating_sub(TAIL_BYTES);
    let end = total - 1;
    let tail = http_range(client, url, start, end).await?;
    let eocd = find_eocd(&tail).ok_or_else(|| {
        KoppenError::ZipLayout(format!("no EOCD in last {} bytes of {url}", tail.len()))
    })?;
    let cd_size = u32::from_le_bytes(tail[eocd + 12..eocd + 16].try_into().unwrap()) as u64;
    let cd_offset = u32::from_le_bytes(tail[eocd + 16..eocd + 20].try_into().unwrap()) as u64;
    let cd_local_start = (cd_offset.checked_sub(start)).ok_or_else(|| {
        KoppenError::ZipLayout(format!(
            "central directory at offset {cd_offset} but tail starts at {start}; \
             ZIP > {TAIL_BYTES} B of trailing data — refetch needed"
        ))
    })? as usize;
    let cd_local_end = cd_local_start + cd_size as usize;
    if cd_local_end > tail.len() {
        return Err(KoppenError::ZipLayout(format!(
            "central directory ({cd_size} B) extends past tail buffer ({} B)",
            tail.len()
        )));
    }
    let cd = &tail[cd_local_start..cd_local_end];
    let mut p = 0usize;
    while p < cd.len() {
        if &cd[p..p + 4] != b"PK\x01\x02" {
            return Err(KoppenError::ZipLayout(format!(
                "central-directory entry at {p} missing PK\\x01\\x02 signature"
            )));
        }
        if cd.len() < p + 46 {
            return Err(KoppenError::ZipLayout(
                "central-directory entry truncated".into(),
            ));
        }
        let csize = u32::from_le_bytes(cd[p + 20..p + 24].try_into().unwrap());
        let usize_ = u32::from_le_bytes(cd[p + 24..p + 28].try_into().unwrap());
        let name_len = u16::from_le_bytes(cd[p + 28..p + 30].try_into().unwrap()) as usize;
        let extra_len = u16::from_le_bytes(cd[p + 30..p + 32].try_into().unwrap()) as usize;
        let comment_len = u16::from_le_bytes(cd[p + 32..p + 34].try_into().unwrap()) as usize;
        let lh_offset = u32::from_le_bytes(cd[p + 42..p + 46].try_into().unwrap()) as u64;
        if cd.len() < p + 46 + name_len {
            return Err(KoppenError::ZipLayout(
                "central-directory name truncated".into(),
            ));
        }
        let entry_name = std::str::from_utf8(&cd[p + 46..p + 46 + name_len]).unwrap_or("");
        if entry_name == name {
            return Ok(ZipEntry {
                compressed_size: csize,
                uncompressed_size: usize_,
                local_header_offset: lh_offset,
            });
        }
        p += 46 + name_len + extra_len + comment_len;
    }
    Err(KoppenError::ZipLayout(format!(
        "no central-directory entry named '{name}' in {url}"
    )))
}

/// Find the End-of-Central-Directory record by scanning backwards for
/// the `PK\x05\x06` signature. ZIP comments are bounded at 64 KiB so a
/// reverse scan over a 64 KiB tail is sufficient.
fn find_eocd(buf: &[u8]) -> Option<usize> {
    const SIG: &[u8] = b"PK\x05\x06";
    if buf.len() < 22 {
        return None;
    }
    let upper = buf.len() - 22;
    let mut i = upper;
    loop {
        if &buf[i..i + 4] == SIG {
            return Some(i);
        }
        if i == 0 {
            return None;
        }
        i -= 1;
    }
}

/// Range-read the 30-byte fixed local file header and return the
/// variable-length name + extra field sizes (so the caller knows how
/// many bytes to skip before the deflate stream begins).
async fn read_local_header(
    client: &Client,
    url: &str,
    lh_offset: u64,
) -> Result<ZipLocalHeader, KoppenError> {
    let bytes = http_range(client, url, lh_offset, lh_offset + 29).await?;
    if bytes.len() < 30 || &bytes[..4] != b"PK\x03\x04" {
        return Err(KoppenError::ZipLayout(format!(
            "local file header at {lh_offset} missing PK\\x03\\x04 signature"
        )));
    }
    let name_len = u16::from_le_bytes(bytes[26..28].try_into().unwrap());
    let extra_len = u16::from_le_bytes(bytes[28..30].try_into().unwrap());
    Ok(ZipLocalHeader {
        name_len,
        extra_len,
    })
}

/// HEAD-style content length probe via a single-byte Range read. Avoids
/// committing to a full GET on hosts whose HEAD returns an unhelpful
/// status (the figshare ndownloader endpoint, in particular, strips
/// content-length on HEAD before redirecting).
async fn http_content_length(client: &Client, url: &str) -> Result<u64, KoppenError> {
    let resp = client
        .get(url)
        .header("range", "bytes=0-0")
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
        .map_err(|e| KoppenError::Transport(e.to_string()))?;
    if resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(KoppenError::Transport(format!(
            "expected 206 PartialContent for content-length probe, got {} from {url}",
            resp.status()
        )));
    }
    let cr = resp
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| KoppenError::Transport(format!("no Content-Range on probe of {url}")))?;
    // Format: "bytes 0-0/<total>"
    let total = cr
        .rsplit('/')
        .next()
        .and_then(|s| s.parse::<u64>().ok())
        .ok_or_else(|| {
            KoppenError::Transport(format!("malformed Content-Range '{cr}' from {url}"))
        })?;
    Ok(total)
}

/// HTTP Range read returning raw bytes. Mirrors the convention used by
/// `cog::http_range` (private there) so behaviour is consistent. We
/// don't reuse that fn because it returns the COG-specific error type.
async fn http_range(
    client: &Client,
    url: &str,
    start: u64,
    end_inclusive: u64,
) -> Result<Bytes, KoppenError> {
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
        .map_err(|e| KoppenError::Transport(e.to_string()))?;
    if !(resp.status() == reqwest::StatusCode::PARTIAL_CONTENT
        || resp.status() == reqwest::StatusCode::OK)
    {
        return Err(KoppenError::Transport(format!(
            "status {} for range {}-{} on {url}",
            resp.status(),
            start,
            end_inclusive
        )));
    }
    resp.bytes()
        .await
        .map_err(|e| KoppenError::Transport(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All 30 published Beck Köppen-Geiger codes resolve to their
    /// canonical class strings. The mapping is the dataset's
    /// `legend.txt` — keep this test in sync when the table is touched.
    #[test]
    fn class_table_matches_beck_legend() {
        // Spot-check the boundaries: first / last / one per A/B/C/D/E
        // family + the two most common climates Singapore (Af=1) and
        // Phoenix (BWh=4) plus the "very cold winter" Dwd=24 the table
        // has historically gotten wrong in third-party reproductions.
        assert_eq!(KOPPEN_CLASSES.len(), 30);
        assert_eq!(KOPPEN_CLASSES[0], "Af");
        assert_eq!(KOPPEN_CLASSES[1], "Am");
        assert_eq!(KOPPEN_CLASSES[2], "Aw");
        assert_eq!(KOPPEN_CLASSES[3], "BWh");
        assert_eq!(KOPPEN_CLASSES[4], "BWk");
        assert_eq!(KOPPEN_CLASSES[5], "BSh");
        assert_eq!(KOPPEN_CLASSES[6], "BSk");
        assert_eq!(KOPPEN_CLASSES[7], "Csa");
        assert_eq!(KOPPEN_CLASSES[13], "Cfa");
        assert_eq!(KOPPEN_CLASSES[14], "Cfb");
        assert_eq!(KOPPEN_CLASSES[15], "Cfc");
        assert_eq!(KOPPEN_CLASSES[23], "Dwd");
        assert_eq!(KOPPEN_CLASSES[24], "Dfa");
        assert_eq!(KOPPEN_CLASSES[25], "Dfb");
        assert_eq!(KOPPEN_CLASSES[26], "Dfc");
        assert_eq!(KOPPEN_CLASSES[27], "Dfd");
        assert_eq!(KOPPEN_CLASSES[28], "ET");
        assert_eq!(KOPPEN_CLASSES[29], "EF");

        // Every class string must be 2 or 3 ASCII chars matching the
        // standard Köppen alphabet.
        for s in KOPPEN_CLASSES {
            assert!(
                (2..=3).contains(&s.len()),
                "class string {s:?} has unexpected length"
            );
            assert!(s.is_ascii(), "class string {s:?} not ASCII");
            let head = s.chars().next().unwrap();
            assert!(
                matches!(head, 'A' | 'B' | 'C' | 'D' | 'E'),
                "class string {s:?} doesn't start with A/B/C/D/E"
            );
        }
    }

    /// PackBits decoding handles literals, repeats, and the 128 no-op
    /// against a tiny hand-rolled stream.
    #[test]
    fn packbits_handles_literal_repeat_and_noop() {
        // Stream layout:
        //   0x02 0x10 0x11 0x12       -- literal: 3 bytes (0x10, 0x11, 0x12)
        //   0xFE 0xFF                 -- run: 0xFF repeated (257-254)=3 times
        //   0x80                      -- no-op
        //   0x00 0xAA                 -- literal: 1 byte (0xAA)
        let stream = [0x02, 0x10, 0x11, 0x12, 0xFE, 0xFF, 0x80, 0x00, 0xAA];
        let out = packbits_decode(&stream, 7).unwrap();
        assert_eq!(out, vec![0x10, 0x11, 0x12, 0xFF, 0xFF, 0xFF, 0xAA]);
    }

    /// Looking up an out-of-range pixel value from a synthesised TIFF
    /// surfaces `ClassOutOfRange` rather than a default class. Builds a
    /// minimal one-tile uint8 PackBits TIFF in memory so we can drive
    /// the sampler end-to-end without the network.
    #[test]
    fn sample_tiff_out_of_range_value_returns_error() {
        // Fabricate a 2x2 TIFF, single 2x2 PackBits-compressed tile, all
        // pixels = 99 (clearly outside 1..=30). The geographic frame
        // is a degenerate 1°×1° pixel at world origin so the lookup
        // for (lat, lng) = (-0.5, 0.5) lands on pixel (0,0).
        let mut tif: Vec<u8> = Vec::new();
        tif.extend_from_slice(b"II*\x00");
        tif.extend_from_slice(&8u32.to_le_bytes()); // IFD0 at offset 8
                                                    // We'll patch entry payloads as we go. Reserve space for IFD0.
                                                    // 17 entries fit our needs; use 12 for simplicity — width,
                                                    // length, bps, comp, spp, tile_w, tile_h, tile_offsets,
                                                    // tile_byte_counts, sample_format, model_pixel_scale,
                                                    // model_tiepoint.
        let n_entries: u16 = 12;
        let entries_pos = tif.len();
        tif.extend_from_slice(&n_entries.to_le_bytes());
        // Reserve 12-byte slots for each entry, fill later.
        for _ in 0..n_entries {
            tif.extend_from_slice(&[0u8; 12]);
        }
        tif.extend_from_slice(&0u32.to_le_bytes()); // next IFD = 0

        // Append payloads.
        let pixel_scale_off = tif.len();
        tif.extend_from_slice(&1.0f64.to_le_bytes()); // sx
        tif.extend_from_slice(&1.0f64.to_le_bytes()); // sy
        tif.extend_from_slice(&0.0f64.to_le_bytes()); // sz
        let tiepoint_off = tif.len();
        for v in [0.0f64, 0.0, 0.0, 0.0, 0.0, 0.0] {
            tif.extend_from_slice(&v.to_le_bytes());
        }
        let tile_offsets_off = tif.len();
        // Placeholder; patch after we know tile_data_off.
        tif.extend_from_slice(&0u32.to_le_bytes());
        let tile_byte_counts_off = tif.len();
        tif.extend_from_slice(&0u32.to_le_bytes());
        // PackBits-encode 4 pixels of value 99: run of 4 -> header = 257-4 = 253.
        let tile_data_off = tif.len();
        tif.extend_from_slice(&[253u8, 99u8]);
        let tile_data_len: u32 = 2;

        // Patch tile_offsets / tile_byte_counts.
        tif[tile_offsets_off..tile_offsets_off + 4]
            .copy_from_slice(&(tile_data_off as u32).to_le_bytes());
        tif[tile_byte_counts_off..tile_byte_counts_off + 4]
            .copy_from_slice(&tile_data_len.to_le_bytes());

        // Fill IFD entries (12 bytes each: tag(2) typ(2) cnt(4) val(4)).
        let put_entry = |tif: &mut Vec<u8>, idx: u16, tag: u16, typ: u16, cnt: u32, val: u32| {
            let p = entries_pos + 2 + (idx as usize) * 12;
            tif[p..p + 2].copy_from_slice(&tag.to_le_bytes());
            tif[p + 2..p + 4].copy_from_slice(&typ.to_le_bytes());
            tif[p + 4..p + 8].copy_from_slice(&cnt.to_le_bytes());
            tif[p + 8..p + 12].copy_from_slice(&val.to_le_bytes());
        };
        put_entry(&mut tif, 0, 256, 3, 1, 2); // ImageWidth = 2
        put_entry(&mut tif, 1, 257, 3, 1, 2); // ImageLength = 2
        put_entry(&mut tif, 2, 258, 3, 1, 8); // BitsPerSample = 8
        put_entry(&mut tif, 3, 259, 3, 1, 32773); // Compression = PackBits
        put_entry(&mut tif, 4, 277, 3, 1, 1); // SamplesPerPixel = 1
        put_entry(&mut tif, 5, 322, 3, 1, 2); // TileWidth = 2
        put_entry(&mut tif, 6, 323, 3, 1, 2); // TileLength = 2
        put_entry(&mut tif, 7, 324, 4, 1, tile_offsets_off as u32);
        put_entry(&mut tif, 8, 325, 4, 1, tile_byte_counts_off as u32);
        put_entry(&mut tif, 9, 339, 3, 1, 1); // SampleFormat = unsigned
        put_entry(&mut tif, 10, 33550, 12, 3, pixel_scale_off as u32);
        put_entry(&mut tif, 11, 33922, 12, 6, tiepoint_off as u32);

        // `sample_tiff_bytes` returns the raw byte (99); the caller
        // (`sample_local_tiff`) is what enforces the 1..=30 contract.
        // Drive both layers so this test pins the full validation path.
        let raw = sample_tiff_bytes(&tif, -0.5, 0.5).expect("synthesised TIFF must parse");
        assert_eq!(raw, 99);

        // Round-trip via the file-based path — write the bytes to a
        // temp file and call `sample_local_tiff`, which is the function
        // the materializer actually invokes.
        let tmp = std::env::temp_dir().join(format!(
            "emem_koppen_test_{}_{}.tif",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::write(&tmp, &tif).expect("write tmp tif");
        let res = sample_local_tiff(&tmp, -0.5, 0.5);
        let _ = std::fs::remove_file(&tmp);
        let err = res.expect_err("99 should fail the 1..=30 validation");
        assert!(
            matches!(err, KoppenError::ClassOutOfRange { value: 99, .. }),
            "expected ClassOutOfRange{{value: 99}}, got {err:?}"
        );
    }

    /// `sample_local_tiff` surfaces `NoData` when the pixel value is 0
    /// (Beck's documented oceanic / out-of-mask sentinel) — protocol
    /// rule: never invent a default class for a missing measurement.
    #[test]
    fn sample_local_tiff_zero_pixel_is_no_data() {
        // Same minimal TIFF construction as the out-of-range test but
        // with pixel value 0 baked into the PackBits stream.
        let mut tif: Vec<u8> = Vec::new();
        tif.extend_from_slice(b"II*\x00");
        tif.extend_from_slice(&8u32.to_le_bytes());
        let n_entries: u16 = 12;
        let entries_pos = tif.len();
        tif.extend_from_slice(&n_entries.to_le_bytes());
        for _ in 0..n_entries {
            tif.extend_from_slice(&[0u8; 12]);
        }
        tif.extend_from_slice(&0u32.to_le_bytes());

        let pixel_scale_off = tif.len();
        tif.extend_from_slice(&1.0f64.to_le_bytes());
        tif.extend_from_slice(&1.0f64.to_le_bytes());
        tif.extend_from_slice(&0.0f64.to_le_bytes());
        let tiepoint_off = tif.len();
        for v in [0.0f64, 0.0, 0.0, 0.0, 0.0, 0.0] {
            tif.extend_from_slice(&v.to_le_bytes());
        }
        let tile_offsets_off = tif.len();
        tif.extend_from_slice(&0u32.to_le_bytes());
        let tile_byte_counts_off = tif.len();
        tif.extend_from_slice(&0u32.to_le_bytes());
        let tile_data_off = tif.len();
        // Run of 4 zeros: header = 257-4 = 253, value = 0.
        tif.extend_from_slice(&[253u8, 0u8]);
        let tile_data_len: u32 = 2;
        tif[tile_offsets_off..tile_offsets_off + 4]
            .copy_from_slice(&(tile_data_off as u32).to_le_bytes());
        tif[tile_byte_counts_off..tile_byte_counts_off + 4]
            .copy_from_slice(&tile_data_len.to_le_bytes());

        let put_entry = |tif: &mut Vec<u8>, idx: u16, tag: u16, typ: u16, cnt: u32, val: u32| {
            let p = entries_pos + 2 + (idx as usize) * 12;
            tif[p..p + 2].copy_from_slice(&tag.to_le_bytes());
            tif[p + 2..p + 4].copy_from_slice(&typ.to_le_bytes());
            tif[p + 4..p + 8].copy_from_slice(&cnt.to_le_bytes());
            tif[p + 8..p + 12].copy_from_slice(&val.to_le_bytes());
        };
        put_entry(&mut tif, 0, 256, 3, 1, 2);
        put_entry(&mut tif, 1, 257, 3, 1, 2);
        put_entry(&mut tif, 2, 258, 3, 1, 8);
        put_entry(&mut tif, 3, 259, 3, 1, 32773);
        put_entry(&mut tif, 4, 277, 3, 1, 1);
        put_entry(&mut tif, 5, 322, 3, 1, 2);
        put_entry(&mut tif, 6, 323, 3, 1, 2);
        put_entry(&mut tif, 7, 324, 4, 1, tile_offsets_off as u32);
        put_entry(&mut tif, 8, 325, 4, 1, tile_byte_counts_off as u32);
        put_entry(&mut tif, 9, 339, 3, 1, 1);
        put_entry(&mut tif, 10, 33550, 12, 3, pixel_scale_off as u32);
        put_entry(&mut tif, 11, 33922, 12, 6, tiepoint_off as u32);

        let tmp = std::env::temp_dir().join(format!(
            "emem_koppen_test_zero_{}_{}.tif",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::write(&tmp, &tif).expect("write tmp tif");
        let res = sample_local_tiff(&tmp, -0.5, 0.5);
        let _ = std::fs::remove_file(&tmp);
        let err = res.expect_err("pixel=0 must surface NoData");
        assert!(
            matches!(err, KoppenError::NoData { .. }),
            "expected NoData, got {err:?}"
        );
    }
}
