//! Pure-Rust Cloud-Optimized GeoTIFF (COG) point sampler.
//!
//! emem materializers want one number per cell — not a full raster. This
//! module range-reads just the IFD + the single tile that covers the
//! requested pixel, so a per-cell recall touches a few hundred KB of a
//! gigabyte-scale Sentinel-2 / -1 scene instead of pulling the whole thing.
//!
//! Wire-level steps for one sample:
//! 1. Range-read the first 64 KiB of the COG.
//! 2. Parse TIFF header + IFD0 entries (we ignore overviews for a point query).
//! 3. Pull `TileOffsets` / `TileByteCounts` arrays out of the buffer (or fetch
//!    them with a second range read if they fall past 64 KiB — they don't for
//!    Sentinel-2 / -1 in practice, but the code handles it).
//! 4. Compute world ↔ pixel transform from `ModelPixelScale` + `ModelTiepoint`.
//! 5. Caller supplies a world coordinate (already in the COG's CRS); we map
//!    to (col, row), find the containing tile, range-read it, Deflate-
//!    decompress, undo Predictor 2 (horizontal differencing), extract the
//!    pixel value.
//!
//! Supports the slice of TIFF that AWS-Open-Data Sentinel-2 L2A and Sentinel-1
//! GRD ship: little-endian std TIFF, Compression 8 (Deflate), Predictor 1 or 2,
//! BitsPerSample 16 (uint), and the GeoTIFF tags. Also supports **BigTIFF**
//! (TIFF 6.0 extension with 16-byte header and 64-bit offsets, magic 0x002B)
//! — used by the EU JRC's single-COG global rasters (e.g. GFC2020 V3, 41 GB).
//! Anything fancier (BE byte order, JPEG2000, planar layouts, etc.) returns an
//! error rather than silently doing the wrong thing — the protocol's
//! no-fallback rule applies.

use std::collections::HashMap;
use std::io::Read;
use std::sync::{Arc, LazyLock};

use bytes::Bytes;
use flate2::read::ZlibDecoder;
use reqwest::Client;
use tokio::sync::{Mutex, OnceCell};

/// Errors specific to the COG sampler. Bubbled up through `FetchError::Transport`
/// at the dispatcher boundary so callers don't have to thread two error types.
#[derive(Debug, thiserror::Error)]
pub enum CogError {
    /// HTTP / network failure.
    #[error("transport: {0}")]
    Transport(String),
    /// Parser ran out of bytes; usually means the IFD's external arrays
    /// live past the head we range-read. Caller can refetch a bigger window.
    #[error("short read: needed {needed} bytes at offset {offset}")]
    ShortRead { needed: usize, offset: u64 },
    /// Buffer doesn't start with `II*\0` (or `MM\0*` if BE — we don't yet
    /// support BE because Sentinel-2 / -1 are LE).
    #[error("not a TIFF: bad magic {0:#x}")]
    BadMagic(u32),
    /// Tag we need is missing from IFD0.
    #[error("missing tag {0}")]
    MissingTag(u16),
    /// Asked for a feature outside the supported subset.
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// Deflate failure / corrupt tile.
    #[error("inflate: {0}")]
    Inflate(String),
}

/// Parsed COG metadata sufficient for point sampling.
#[derive(Debug, Clone)]
pub struct CogProfile {
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// Bits per sample (8 / 16 / 32).
    pub bits_per_sample: u16,
    /// 1 = unsigned int, 2 = signed int, 3 = float.
    pub sample_format: u16,
    /// 8 = Deflate. We hard-fail other codecs so behaviour is honest.
    pub compression: u16,
    /// 1 = no predictor, 2 = horizontal differencing, 3 = floating-point
    /// predictor (libtiff `fpAcc` byte-plane shuffle + diff). 1/2 cover
    /// Sentinel-2; 3 is what MPC's Sentinel-1 RTC ships for f32 backscatter.
    pub predictor: u16,
    /// Tile width.
    pub tile_w: u32,
    /// Tile height.
    pub tile_h: u32,
    /// Number of tile columns.
    pub tile_cols: u32,
    /// Number of tile rows.
    pub tile_rows: u32,
    /// Channels per pixel (SamplesPerPixel). 1 for grayscale rasters; >1 for
    /// multi-band COGs (e.g. RGB scenes or per-pixel embedding stacks).
    pub samples_per_pixel: u16,
    /// 1 = chunky (samples interleaved per pixel), 2 = planar (samples in
    /// separate planes). Only chunky is supported.
    pub planar_config: u16,
    /// Per-tile byte offsets into the COG.
    pub tile_offsets: Vec<u64>,
    /// Per-tile compressed byte counts.
    pub tile_byte_counts: Vec<u64>,
    /// Pixel size in CRS units (sx, sy). North-up: positive sx, positive sy
    /// means row 0 is the north edge.
    pub pixel_scale: (f64, f64),
    /// Tiepoint: pixel (i, j) maps to world (x, y).
    pub tiepoint: (f64, f64, f64, f64),
    /// EPSG code if we found a ProjectedCSTypeGeoKey (3072) in
    /// GeoKeyDirectory; None means the caller has to know the CRS via STAC.
    pub epsg: Option<u32>,
    /// GDAL_NODATA string if the tag was present.
    pub nodata: Option<String>,
}

impl CogProfile {
    /// Map world (x, y) in the COG's CRS to (col, row) in pixel space.
    pub fn world_to_pixel(&self, world_x: f64, world_y: f64) -> (i64, i64) {
        let (sx, sy) = self.pixel_scale;
        let (i, j, _, x, y) = (
            self.tiepoint.0,
            self.tiepoint.1,
            self.tiepoint.2,
            self.tiepoint.3,
            // tiepoint.4 is world Y of pixel (i, j)
            // (we stored as `tiepoint: (i, j, k, x, y)` collapsing z)
            // The struct only has 4 floats, but the GeoTIFF tiepoint has 6 doubles;
            // we kept (i, j, x, y) — z is implicitly zero for the 2D case.
            self.tiepoint.3, // unused placeholder
        );
        let _ = (i, j, x, y); // silence unused warnings for placeholder layout
        let i = self.tiepoint.0;
        let j = self.tiepoint.1;
        let x = self.tiepoint.2;
        let y = self.tiepoint.3;
        let col = (i + (world_x - x) / sx).round() as i64;
        let row = (j + (y - world_y) / sy).round() as i64;
        (col, row)
    }
}

/// One slot in the profile cache: a shared `OnceCell` that holds the
/// `Arc<CogProfile>` once the first caller finishes fetching+parsing.
/// Concurrent callers for the same URL park on this cell.
type ProfileCacheSlot = Arc<OnceCell<Arc<CogProfile>>>;

/// Process-wide cache of parsed COG profiles, keyed by URL. A COG's IFD
/// is immutable upstream — for JRC GFC2020 V3 the external TileOffsets +
/// TileByteCounts arrays alone are ~110 MB (LONG8 over 6.9 M tiles), and
/// the retry loop in `open_profile_uncached` re-fetches them from byte 0
/// each iteration. Caching once per URL turns subsequent `open_profile`
/// calls into a HashMap lookup, taking `/v1/eudr_dds` per-cell sampling
/// from ~97 s back to interactive latency.
///
/// `OnceCell` per slot delivers the single-flight contract: concurrent
/// callers for the same URL all park on the same `get_or_try_init`
/// future, so exactly ONE upstream fetch happens; the rest receive the
/// resulting `Arc` by clone. `tokio::sync::Mutex` because we hold the
/// outer map lock only long enough to look up / insert the slot, while
/// the `OnceCell` initialisation itself awaits the upstream fetch.
static PROFILE_CACHE: LazyLock<Mutex<HashMap<String, ProfileCacheSlot>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Range-read the head of a COG and parse IFD0. Returns the metadata needed
/// for `sample_pixel`.
///
/// Results are cached per URL for the lifetime of the process and
/// single-flighted across concurrent callers — for any given URL only
/// ONE upstream fetch+parse happens; subsequent calls clone the cached
/// `Arc<CogProfile>` without touching the network.
pub async fn open_profile(client: &Client, url: &str) -> Result<Arc<CogProfile>, CogError> {
    // Grab (or insert) the OnceCell slot for this URL. Outer lock is
    // released before we await initialisation so concurrent open_profile
    // calls for *other* URLs aren't blocked behind us.
    let cell = {
        let mut guard = PROFILE_CACHE.lock().await;
        guard
            .entry(url.to_string())
            .or_insert_with(|| Arc::new(OnceCell::new()))
            .clone()
    };
    // First caller drives the fetch+parse; everyone else parks on the
    // same future and gets the same Arc when it lands.
    let arc = cell
        .get_or_try_init(|| async { open_profile_uncached(client, url).await.map(Arc::new) })
        .await?;
    Ok(arc.clone())
}

/// Pre-warm the profile cache for `url` so the first user-facing recall
/// avoids the IFD + tile-array fetch. Wiring into server startup is a
/// follow-up; this just exposes the helper so the caller can decide.
pub async fn prewarm_profile(client: &Client, url: &str) -> Result<(), CogError> {
    open_profile(client, url).await.map(drop)
}

/// The first read pulls 64 KiB which is enough for Sentinel-2 / -1 in
/// practice. COGs that use the **end-of-file IFD layout** (IFD0 written
/// after all the tile data, e.g. JRC Global Surface Water — 40000×40000
/// pixels with IFD0 at byte ~86 M out of ~86 M file size) refer to
/// external arrays and tag values at offsets that span the entire file.
/// Each such reference triggers a fresh `ShortRead{needed}` — different
/// entries have different `needed` values — so we loop the retry,
/// expanding the buffer to cover whatever offset parse_profile is stuck
/// on, up to a hard cap of 8 iterations to avoid pathological cases.
async fn open_profile_uncached(client: &Client, url: &str) -> Result<CogProfile, CogError> {
    let mut buf = http_range(client, url, 0, 65535).await?;
    for _ in 0..8 {
        match parse_profile(&buf) {
            Ok(p) => return Ok(p),
            Err(CogError::ShortRead { needed, offset: 0 }) if needed > buf.len() => {
                // Push past `needed` so adjacent tags (GeoKeyDir, NODATA,
                // ColorMap, etc.) that live a few hundred bytes further on
                // also fit and we don't take another round-trip for them.
                let end = (needed as u64).saturating_add(65535);
                let next = http_range(client, url, 0, end).await?;
                if next.len() <= buf.len() {
                    return Err(CogError::Transport(format!(
                        "open_profile retry: range request not honored or file truncated \
                         (had {} bytes, asked for {}, got {})",
                        buf.len(),
                        end,
                        next.len()
                    )));
                }
                buf = next;
            }
            Err(e) => return Err(e),
        }
    }
    // Final try after the bounded retries — surface whatever error remains.
    parse_profile(&buf)
}

/// TIFF flavour discovered from the 16-byte header. BigTIFF widens
/// every offset, count, and inline-value field from u32 to u64; the
/// IFD entry stride goes from 12 to 20 bytes, the IFD entry count
/// becomes u64, and the per-IFD "next IFD" pointer becomes u64.
/// See TIFF 6.0 §2 and the BigTIFF spec (LibTIFF wiki, §"BigTIFF").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TiffFlavor {
    /// Classic TIFF 6.0: magic 0x002A, 8-byte header, u32 offsets,
    /// 12-byte IFD entries with u32 count + u32 inline value.
    Standard,
    /// BigTIFF: magic 0x002B, 16-byte header, u64 offsets, 20-byte
    /// IFD entries with u64 count + u64 inline value. Required for
    /// files > 4 GiB (e.g. JRC GFC2020 V3, 41 GB single-COG).
    Big,
}

impl TiffFlavor {
    /// Bytes per IFD entry on disk (12 for std, 20 for BigTIFF).
    fn entry_stride(self) -> usize {
        match self {
            TiffFlavor::Standard => 12,
            TiffFlavor::Big => 20,
        }
    }
    /// Bytes for the IFD's entry-count prefix at the start of an IFD
    /// (u16 in std → 2 bytes; u64 in BigTIFF → 8 bytes).
    fn entry_count_size(self) -> usize {
        match self {
            TiffFlavor::Standard => 2,
            TiffFlavor::Big => 8,
        }
    }
    /// Width of the inline-value / external-offset slot inside one
    /// IFD entry. Total payload size ≤ this width is stored inline;
    /// anything bigger dereferences to an external offset.
    fn value_slot_size(self) -> usize {
        match self {
            TiffFlavor::Standard => 4,
            TiffFlavor::Big => 8,
        }
    }
}

/// Read the entry-count prefix at the start of an IFD. Returns the
/// number of IFD entries that follow, branching on TIFF flavour
/// (u16 in std, u64 in BigTIFF).
fn read_entry_count(buf: &[u8], off: usize, flavor: TiffFlavor) -> Result<usize, CogError> {
    let n = flavor.entry_count_size();
    if buf.len() < off + n {
        return Err(CogError::ShortRead {
            needed: off + n,
            offset: 0,
        });
    }
    Ok(match flavor {
        TiffFlavor::Standard => u16::from_le_bytes([buf[off], buf[off + 1]]) as usize,
        TiffFlavor::Big => u64::from_le_bytes(buf[off..off + 8].try_into().unwrap()) as usize,
    })
}

/// One decoded IFD entry, flavour-agnostic. `cnt` and inline value
/// have already been widened to u64; downstream consumers cast as
/// needed. `raw` is the inline-value byte slice (length matches
/// `flavor.value_slot_size()`) so callers that need to peek at the
/// low u16 (e.g. SHORT inline reads) can without re-deriving the
/// entry layout.
struct IfdEntry<'a> {
    tag: u16,
    typ: u16,
    cnt: u64,
    /// 8-byte LE encoding of the inline value or external offset.
    /// For Standard TIFF the upper 4 bytes are zero; for BigTIFF
    /// they carry the high 32 bits of a 64-bit offset.
    val_u64: u64,
    /// Raw inline-value slot (4 or 8 bytes depending on flavour).
    /// Equal to the low N bytes of `val_u64.to_le_bytes()`. Kept
    /// separately because some callers (BitsPerSample / SampleFormat)
    /// want to read SHORTs out of it without re-encoding.
    raw: &'a [u8],
}

/// Parse one IFD entry at byte offset `e` in `buf` under the given
/// TIFF flavour. The caller is responsible for ensuring `buf` is
/// long enough; that's checked once for the whole entries block in
/// the parser before the entry loop.
fn read_ifd_entry(buf: &[u8], e: usize, flavor: TiffFlavor) -> IfdEntry<'_> {
    let tag = u16::from_le_bytes([buf[e], buf[e + 1]]);
    let typ = u16::from_le_bytes([buf[e + 2], buf[e + 3]]);
    match flavor {
        TiffFlavor::Standard => {
            let cnt = u32::from_le_bytes([buf[e + 4], buf[e + 5], buf[e + 6], buf[e + 7]]) as u64;
            let raw = &buf[e + 8..e + 12];
            // Sign-extend (well, zero-extend) the inline u32 slot into
            // a u64 so the rest of the parser can treat both flavours
            // uniformly. For SHORT/LONG inline values only the low
            // bytes are meaningful; the unused upper bytes stay zero.
            let val_u64 = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as u64;
            IfdEntry {
                tag,
                typ,
                cnt,
                val_u64,
                raw,
            }
        }
        TiffFlavor::Big => {
            let cnt = u64::from_le_bytes(buf[e + 4..e + 12].try_into().unwrap());
            let raw = &buf[e + 12..e + 20];
            let val_u64 = u64::from_le_bytes(raw.try_into().unwrap());
            IfdEntry {
                tag,
                typ,
                cnt,
                val_u64,
                raw,
            }
        }
    }
}

/// Bytesize of a TIFF data type. Used for the "fits inline?" check
/// — if `cnt * type_size ≤ value_slot_size`, the entry's value
/// lives inside the IFD entry; otherwise the entry's value field
/// is an offset to an external array.
fn tiff_type_size(typ: u16) -> usize {
    match typ {
        1 | 2 | 6 | 7 => 1,              // BYTE / ASCII / SBYTE / UNDEFINED
        3 | 8 => 2,                      // SHORT / SSHORT
        4 | 9 | 11 | 13 => 4,            // LONG / SLONG / FLOAT / IFD
        5 | 10 | 12 | 16 | 17 | 18 => 8, // RATIONAL / SRATIONAL / DOUBLE / LONG8 / SLONG8 / IFD8
        _ => 0,
    }
}

/// Read one element from a TileOffsets / TileByteCounts style array
/// of integer offsets. Honours BigTIFF's LONG8 (type 16) on top of
/// the classic SHORT (3) / LONG (4). All other types are an error.
fn read_offset_array_elem(buf: &[u8], base: usize, idx: usize, typ: u16) -> Result<u64, CogError> {
    match typ {
        3 => {
            let p = base + idx * 2;
            if buf.len() < p + 2 {
                return Err(CogError::ShortRead {
                    needed: p + 2,
                    offset: 0,
                });
            }
            Ok(u16::from_le_bytes(buf[p..p + 2].try_into().unwrap()) as u64)
        }
        4 => {
            let p = base + idx * 4;
            if buf.len() < p + 4 {
                return Err(CogError::ShortRead {
                    needed: p + 4,
                    offset: 0,
                });
            }
            Ok(u32::from_le_bytes(buf[p..p + 4].try_into().unwrap()) as u64)
        }
        16 => {
            let p = base + idx * 8;
            if buf.len() < p + 8 {
                return Err(CogError::ShortRead {
                    needed: p + 8,
                    offset: 0,
                });
            }
            Ok(u64::from_le_bytes(buf[p..p + 8].try_into().unwrap()))
        }
        other => Err(CogError::Unsupported(format!(
            "tile_offsets / tile_byte_counts type {other} (expected SHORT=3, LONG=4, or LONG8=16)"
        ))),
    }
}

fn parse_profile(buf: &[u8]) -> Result<CogProfile, CogError> {
    if buf.len() < 8 {
        return Err(CogError::ShortRead {
            needed: 8,
            offset: 0,
        });
    }
    if &buf[..2] != b"II" {
        return Err(CogError::Unsupported(
            "big-endian TIFF (MM) not yet supported".into(),
        ));
    }
    let magic = u16::from_le_bytes([buf[2], buf[3]]) as u32;
    let (flavor, ifd0_off) = match magic {
        42 => {
            // Classic TIFF 6.0: IFD0 offset is a u32 at bytes 4..8.
            let off = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]) as usize;
            (TiffFlavor::Standard, off)
        }
        43 => {
            // BigTIFF: bytes 4..6 = offset bytesize (must be 8),
            // bytes 6..8 = constant 0, bytes 8..16 = u64 IFD0 offset.
            if buf.len() < 16 {
                return Err(CogError::ShortRead {
                    needed: 16,
                    offset: 0,
                });
            }
            let offset_bytesize = u16::from_le_bytes([buf[4], buf[5]]);
            let zero = u16::from_le_bytes([buf[6], buf[7]]);
            if offset_bytesize != 8 || zero != 0 {
                return Err(CogError::Unsupported(format!(
                    "BigTIFF header: expected offset_bytesize=8 zero=0, got bytesize={offset_bytesize} zero={zero}"
                )));
            }
            let off = u64::from_le_bytes(buf[8..16].try_into().unwrap()) as usize;
            (TiffFlavor::Big, off)
        }
        _ => return Err(CogError::BadMagic(magic)),
    };

    let entry_count_size = flavor.entry_count_size();
    if buf.len() < ifd0_off + entry_count_size {
        return Err(CogError::ShortRead {
            needed: ifd0_off + entry_count_size,
            offset: 0,
        });
    }
    let n = read_entry_count(buf, ifd0_off, flavor)?;
    let entries_start = ifd0_off + entry_count_size;
    let stride = flavor.entry_stride();
    if buf.len() < entries_start + n * stride {
        return Err(CogError::ShortRead {
            needed: entries_start + n * stride,
            offset: 0,
        });
    }

    let value_slot = flavor.value_slot_size();
    let mut width: Option<u32> = None;
    let mut height: Option<u32> = None;
    let mut bits_per_sample: u16 = 8;
    let mut sample_format: u16 = 1;
    let mut compression: u16 = 1;
    let mut predictor: u16 = 1;
    let mut samples_per_pixel: u16 = 1;
    let mut planar_config: u16 = 1;
    let mut tile_w: Option<u32> = None;
    let mut tile_h: Option<u32> = None;
    // (cnt, off, typ) — typ carries SHORT/LONG/LONG8 so the array
    // read below knows the element width. LONG8 is the BigTIFF-only
    // case; std files use LONG or SHORT.
    let mut tile_offsets_ref: Option<(usize, u64, u16)> = None;
    let mut tile_byte_counts_ref: Option<(usize, u64, u16)> = None;
    // TIFF strip tags. Hansen GFC, older USGS DEMs, and some MODIS subsets
    // ship as stripped TIFFs (no tile tags). Strips are essentially tiles
    // of width = image_width and height = rows_per_strip; synthesize the
    // tile_* fields from them so the downstream sampler stays uniform.
    let mut rows_per_strip: Option<u32> = None;
    let mut strip_offsets_ref: Option<(usize, u64, u16)> = None;
    let mut strip_byte_counts_ref: Option<(usize, u64, u16)> = None;
    let mut pixel_scale: Option<(f64, f64)> = None;
    let mut tiepoint: Option<(f64, f64, f64, f64)> = None;
    let mut geokey_ref: Option<(usize, u64)> = None;
    let mut nodata: Option<String> = None;

    for i in 0..n {
        let e = entries_start + i * stride;
        let ent = read_ifd_entry(buf, e, flavor);
        let tag = ent.tag;
        let typ = ent.typ;
        let cnt = ent.cnt as usize;
        let val_u64 = ent.val_u64;
        let val_usize = val_u64 as usize;
        let val_u32 = val_u64 as u32;
        let raw = ent.raw;
        let val_u16_first = u16::from_le_bytes([raw[0], raw[1]]);

        match tag {
            256 => width = Some(val_u32),
            257 => height = Some(val_u32),
            258 => {
                // BitsPerSample is a SHORT array of length `samples_per_pixel`.
                // TIFF packs values inline only when total size ≤ the
                // value-slot width (4 std / 8 BigTIFF); beyond that, the
                // entry's value field is an offset to an external array.
                // Single-band files (cnt=1) fit inline; multi-band files
                // like WRI GDM v1.2 (cnt=8, 16 bytes) dereference. We pick
                // the first u16 either way and assume the bands share
                // BitsPerSample — the existing per-sample decoders
                // downstream all read at this resolution. If a future
                // multi-band file mixes bit-widths the open_profile would
                // need an array readback, but every multi-band raster we
                // sample today uses a uniform width.
                bits_per_sample = if cnt * 2 <= value_slot {
                    val_u16_first
                } else {
                    let off = val_usize;
                    if buf.len() < off + 2 {
                        return Err(CogError::ShortRead {
                            needed: off + 2,
                            offset: 0,
                        });
                    }
                    u16::from_le_bytes([buf[off], buf[off + 1]])
                };
            }
            259 => compression = val_u16_first,
            277 => samples_per_pixel = val_u16_first,
            284 => planar_config = val_u16_first,
            317 => predictor = val_u16_first,
            322 => tile_w = Some(val_u32),
            323 => tile_h = Some(val_u32),
            324 => tile_offsets_ref = Some((cnt, val_u64, typ)),
            325 => tile_byte_counts_ref = Some((cnt, val_u64, typ)),
            // Strip TIFF tags (273/278/279). When present without tile tags
            // (322..=325), strips are folded into the tile model below.
            273 => strip_offsets_ref = Some((cnt, val_u64, typ)),
            278 => rows_per_strip = Some(val_u32),
            279 => strip_byte_counts_ref = Some((cnt, val_u64, typ)),
            339 => {
                // SampleFormat is also a per-band SHORT array. Same
                // inline-vs-offset rule as BitsPerSample (tag 258). Read
                // the first entry; downstream decoders assume uniform
                // sample format across bands.
                sample_format = if cnt * 2 <= value_slot {
                    val_u16_first
                } else {
                    let off = val_usize;
                    if buf.len() < off + 2 {
                        return Err(CogError::ShortRead {
                            needed: off + 2,
                            offset: 0,
                        });
                    }
                    u16::from_le_bytes([buf[off], buf[off + 1]])
                };
            }
            33550 => {
                // ModelPixelScale: 3 doubles (sx, sy, sz). Total 24 bytes
                // — never inline in std (slot=4) or BigTIFF (slot=8), so
                // always dereferenced. Kept as an offset read for safety
                // even on a hypothetical inline encoder.
                if cnt < 2 {
                    continue;
                }
                let off = val_usize;
                if buf.len() < off + 16 {
                    return Err(CogError::ShortRead {
                        needed: off + 16,
                        offset: 0,
                    });
                }
                let sx = f64::from_le_bytes(buf[off..off + 8].try_into().unwrap());
                let sy = f64::from_le_bytes(buf[off + 8..off + 16].try_into().unwrap());
                pixel_scale = Some((sx, sy));
            }
            33922 => {
                // ModelTiepoint: 6 doubles (i, j, k, x, y, z) — 2D = use 4
                if cnt < 6 {
                    continue;
                }
                let off = val_usize;
                if buf.len() < off + 48 {
                    return Err(CogError::ShortRead {
                        needed: off + 48,
                        offset: 0,
                    });
                }
                let i = f64::from_le_bytes(buf[off..off + 8].try_into().unwrap());
                let j = f64::from_le_bytes(buf[off + 8..off + 16].try_into().unwrap());
                // skip k at off+16..off+24
                let x = f64::from_le_bytes(buf[off + 24..off + 32].try_into().unwrap());
                let y = f64::from_le_bytes(buf[off + 32..off + 40].try_into().unwrap());
                tiepoint = Some((i, j, x, y));
            }
            34735 => {
                // GeoKeyDirectory: u16 array, 4 u16 per key
                geokey_ref = Some((cnt, val_u64));
            }
            42113 => {
                // GDAL_NODATA: ASCII. Inline if it fits in the value slot
                // (4 bytes std, 8 bytes BigTIFF); else dereference.
                if cnt <= value_slot {
                    let s = std::str::from_utf8(&raw[..cnt.min(value_slot)])
                        .unwrap_or("")
                        .trim_end_matches('\0')
                        .to_string();
                    nodata = Some(s);
                } else {
                    let off = val_usize;
                    if buf.len() < off + cnt {
                        return Err(CogError::ShortRead {
                            needed: off + cnt,
                            offset: 0,
                        });
                    }
                    let s = std::str::from_utf8(&buf[off..off + cnt])
                        .unwrap_or("")
                        .trim_end_matches('\0')
                        .to_string();
                    nodata = Some(s);
                }
            }
            _ => {}
        }
    }

    let width = width.ok_or(CogError::MissingTag(256))?;
    let height = height.ok_or(CogError::MissingTag(257))?;
    // If the IFD ships strips instead of tiles (Hansen GFC, older NOAA/USGS
    // GeoTIFFs), synthesize tile_w / tile_h / tile_offsets / tile_byte_counts
    // from the strip tags so the rest of the sampler can stay tile-shaped.
    let strip_mode = tile_w.is_none()
        && tile_h.is_none()
        && tile_offsets_ref.is_none()
        && tile_byte_counts_ref.is_none()
        && strip_offsets_ref.is_some()
        && strip_byte_counts_ref.is_some();
    if strip_mode {
        // Default RowsPerStrip is the full image height when the tag is
        // absent (TIFF 6.0 §10), but every TIFF we've seen in the wild
        // sets it.
        let rps = rows_per_strip.unwrap_or(height);
        tile_w = Some(width);
        tile_h = Some(rps);
        tile_offsets_ref = strip_offsets_ref;
        tile_byte_counts_ref = strip_byte_counts_ref;
    }
    let tile_w = tile_w.ok_or(CogError::MissingTag(322))?;
    let tile_h = tile_h.ok_or(CogError::MissingTag(323))?;
    let pixel_scale = pixel_scale.ok_or(CogError::MissingTag(33550))?;
    let tiepoint = tiepoint.ok_or(CogError::MissingTag(33922))?;
    let (toff_cnt, toff_val, toff_typ) = tile_offsets_ref.ok_or(CogError::MissingTag(324))?;
    let (tbc_cnt, tbc_val, tbc_typ) = tile_byte_counts_ref.ok_or(CogError::MissingTag(325))?;
    if toff_cnt != tbc_cnt {
        return Err(CogError::Unsupported(format!(
            "tile_offsets cnt {toff_cnt} != tile_byte_counts cnt {tbc_cnt}"
        )));
    }

    // Decode TileOffsets. SHORT/LONG/LONG8 are the only types we expect
    // here; LONG8 is BigTIFF-only and is how the JRC GFC2020 V3 file
    // (41 GB) encodes its 6.9 M tile offsets. The array can be inline
    // (cnt * type_size ≤ value_slot) or external (cnt > 1 in practice
    // is always external because both fields land past 4 bytes).
    let toff_elem = tiff_type_size(toff_typ);
    if toff_elem == 0 {
        return Err(CogError::Unsupported(format!(
            "tile_offsets type {toff_typ} unknown"
        )));
    }
    let toff_total = toff_cnt * toff_elem;
    let toff_base = if toff_total <= value_slot {
        // Inline — re-encode val_u64 as up to 8 raw bytes and read
        // from there. This branch is dead for any real-world tile-grid
        // (cnt ≥ 1, elem ≥ 2), but kept for protocol completeness.
        // We can't directly point into `buf` for the inline case, so
        // serialise to a stable byte buffer that we then index.
        let bytes = toff_val.to_le_bytes();
        let mut tile_offsets = Vec::with_capacity(toff_cnt);
        for k in 0..toff_cnt {
            tile_offsets.push(read_offset_array_elem(&bytes, 0, k, toff_typ)?);
        }
        let mut tile_byte_counts = Vec::with_capacity(tbc_cnt);
        let tbc_elem = tiff_type_size(tbc_typ);
        if tbc_elem == 0 {
            return Err(CogError::Unsupported(format!(
                "tile_byte_counts type {tbc_typ} unknown"
            )));
        }
        let tbc_total = tbc_cnt * tbc_elem;
        if tbc_total <= value_slot {
            let bytes_b = tbc_val.to_le_bytes();
            for k in 0..tbc_cnt {
                tile_byte_counts.push(read_offset_array_elem(&bytes_b, 0, k, tbc_typ)?);
            }
        } else {
            let off = tbc_val as usize;
            if buf.len() < off + tbc_total {
                return Err(CogError::ShortRead {
                    needed: off + tbc_total,
                    offset: 0,
                });
            }
            for k in 0..tbc_cnt {
                tile_byte_counts.push(read_offset_array_elem(buf, off, k, tbc_typ)?);
            }
        }
        return finish_profile(
            width,
            height,
            bits_per_sample,
            sample_format,
            compression,
            predictor,
            samples_per_pixel,
            planar_config,
            tile_w,
            tile_h,
            tile_offsets,
            tile_byte_counts,
            pixel_scale,
            tiepoint,
            geokey_ref,
            nodata,
            buf,
        );
    } else {
        toff_val as usize
    };
    if buf.len() < toff_base + toff_total {
        return Err(CogError::ShortRead {
            needed: toff_base + toff_total,
            offset: 0,
        });
    }
    let mut tile_offsets = Vec::with_capacity(toff_cnt);
    for k in 0..toff_cnt {
        tile_offsets.push(read_offset_array_elem(buf, toff_base, k, toff_typ)?);
    }

    let tbc_elem = tiff_type_size(tbc_typ);
    if tbc_elem == 0 {
        return Err(CogError::Unsupported(format!(
            "tile_byte_counts type {tbc_typ} unknown"
        )));
    }
    let tbc_total = tbc_cnt * tbc_elem;
    let tbc_base = if tbc_total <= value_slot {
        let bytes = tbc_val.to_le_bytes();
        let mut tile_byte_counts = Vec::with_capacity(tbc_cnt);
        for k in 0..tbc_cnt {
            tile_byte_counts.push(read_offset_array_elem(&bytes, 0, k, tbc_typ)?);
        }
        return finish_profile(
            width,
            height,
            bits_per_sample,
            sample_format,
            compression,
            predictor,
            samples_per_pixel,
            planar_config,
            tile_w,
            tile_h,
            tile_offsets,
            tile_byte_counts,
            pixel_scale,
            tiepoint,
            geokey_ref,
            nodata,
            buf,
        );
    } else {
        tbc_val as usize
    };
    if buf.len() < tbc_base + tbc_total {
        return Err(CogError::ShortRead {
            needed: tbc_base + tbc_total,
            offset: 0,
        });
    }
    let mut tile_byte_counts = Vec::with_capacity(tbc_cnt);
    for k in 0..tbc_cnt {
        tile_byte_counts.push(read_offset_array_elem(buf, tbc_base, k, tbc_typ)?);
    }

    finish_profile(
        width,
        height,
        bits_per_sample,
        sample_format,
        compression,
        predictor,
        samples_per_pixel,
        planar_config,
        tile_w,
        tile_h,
        tile_offsets,
        tile_byte_counts,
        pixel_scale,
        tiepoint,
        geokey_ref,
        nodata,
        buf,
    )
}

/// Final stage of `parse_profile`: assemble the [`CogProfile`] from
/// decoded scalars + already-materialised tile arrays. Pulled out
/// of the main parser because the inline-vs-external array decode
/// has two early-return paths, and duplicating the GeoKey + tile-grid
/// check + struct construction across them would invite drift.
#[allow(clippy::too_many_arguments)]
fn finish_profile(
    width: u32,
    height: u32,
    bits_per_sample: u16,
    sample_format: u16,
    compression: u16,
    predictor: u16,
    samples_per_pixel: u16,
    planar_config: u16,
    tile_w: u32,
    tile_h: u32,
    tile_offsets: Vec<u64>,
    tile_byte_counts: Vec<u64>,
    pixel_scale: (f64, f64),
    tiepoint: (f64, f64, f64, f64),
    geokey_ref: Option<(usize, u64)>,
    nodata: Option<String>,
    buf: &[u8],
) -> Result<CogProfile, CogError> {
    let tile_cols = width.div_ceil(tile_w);
    let tile_rows = height.div_ceil(tile_h);
    let toff_cnt = tile_offsets.len();
    if (tile_cols as usize) * (tile_rows as usize) != toff_cnt {
        return Err(CogError::Unsupported(format!(
            "tile grid {}x{} != tile_offsets count {}",
            tile_cols, tile_rows, toff_cnt
        )));
    }

    // Try to find EPSG via GeoKeyDirectory key 3072 (ProjectedCSTypeGeoKey).
    let mut epsg: Option<u32> = None;
    if let Some((cnt, off_u64)) = geokey_ref {
        let off = off_u64 as usize;
        if buf.len() >= off + cnt * 2 {
            // Header: 4 u16 — version, key_revision, minor_revision, num_keys
            let num_keys = u16::from_le_bytes(buf[off + 6..off + 8].try_into().unwrap()) as usize;
            for k in 0..num_keys {
                let kp = off + 8 + k * 8;
                if buf.len() < kp + 8 {
                    break;
                }
                let key_id = u16::from_le_bytes(buf[kp..kp + 2].try_into().unwrap());
                let tiff_tag_loc = u16::from_le_bytes(buf[kp + 2..kp + 4].try_into().unwrap());
                let _count = u16::from_le_bytes(buf[kp + 4..kp + 6].try_into().unwrap());
                let value = u16::from_le_bytes(buf[kp + 6..kp + 8].try_into().unwrap());
                if key_id == 3072 && tiff_tag_loc == 0 {
                    epsg = Some(value as u32);
                    break;
                }
            }
        }
    }

    Ok(CogProfile {
        width,
        height,
        bits_per_sample,
        sample_format,
        compression,
        predictor,
        tile_w,
        tile_h,
        tile_cols,
        tile_rows,
        samples_per_pixel,
        planar_config,
        tile_offsets,
        tile_byte_counts,
        pixel_scale,
        tiepoint,
        epsg,
        nodata,
    })
}

/// Sample one pixel. `world_x` / `world_y` must be in the COG's CRS already
/// — for Sentinel-2 / -1 that's the per-tile UTM zone (use `crate::proj`).
/// Returns the raw value (post-decompress + post-predictor) as f64. Caller
/// applies any per-band scale/offset (e.g. Sentinel-2 reflectance scale).
pub async fn sample_pixel(
    client: &Client,
    url: &str,
    profile: &CogProfile,
    world_x: f64,
    world_y: f64,
) -> Result<f64, CogError> {
    if profile.compression != 8 && profile.compression != 5 {
        return Err(CogError::Unsupported(format!(
            "compression={} (Deflate (8) and LZW (5) supported)",
            profile.compression
        )));
    }
    if profile.bits_per_sample != 16
        && profile.bits_per_sample != 8
        && profile.bits_per_sample != 32
    {
        return Err(CogError::Unsupported(format!(
            "bits_per_sample={} (8/16/32 supported)",
            profile.bits_per_sample
        )));
    }

    // Pixel coordinates within the full image.
    let (col, row) = profile.world_to_pixel(world_x, world_y);
    if col < 0 || row < 0 || col >= profile.width as i64 || row >= profile.height as i64 {
        return Err(CogError::Unsupported(format!(
            "world ({world_x:.3},{world_y:.3}) maps to pixel ({col},{row}) outside image {}x{}",
            profile.width, profile.height
        )));
    }
    let col = col as u32;
    let row = row as u32;
    let tile_col = col / profile.tile_w;
    let tile_row = row / profile.tile_h;
    let tile_idx = (tile_row * profile.tile_cols + tile_col) as usize;
    if tile_idx >= profile.tile_offsets.len() {
        return Err(CogError::Unsupported(format!(
            "tile_idx {tile_idx} out of range"
        )));
    }
    let intra_col = col - tile_col * profile.tile_w;
    let intra_row = row - tile_row * profile.tile_h;

    let off = profile.tile_offsets[tile_idx];
    let len = profile.tile_byte_counts[tile_idx];
    if len == 0 {
        return Err(CogError::Unsupported(format!(
            "tile {tile_idx} byte_count=0 (sparse — empty)"
        )));
    }
    let tile_compressed = http_range(client, url, off, off + len - 1).await?;

    // Decompress per the profile's `compression` tag.
    //   8 — Deflate / Adler32-framed zlib (Sentinel-2 / -1).
    //   5 — TIFF LZW: MSB-first bit packing, code size starts at 8, with
    //       in-stream Clear (256) and EOI (257) codes. weezl::BitOrder::Msb
    //       matches the TIFF spec; standard TIFF readers (libtiff, image-rs)
    //       use the same configuration.
    let mut tile_bytes = Vec::with_capacity(
        (profile.tile_w as usize)
            * (profile.tile_h as usize)
            * (profile.samples_per_pixel as usize)
            * (profile.bits_per_sample as usize / 8),
    );
    match profile.compression {
        8 => {
            let mut decoder = ZlibDecoder::new(&tile_compressed[..]);
            decoder
                .read_to_end(&mut tile_bytes)
                .map_err(|e| CogError::Inflate(e.to_string()))?;
        }
        5 => {
            // TIFF's LZW variant bumps code-size one entry earlier than the
            // standard GIF-style algorithm — without this the decoder
            // emits "invalid code" the moment the stream crosses a code
            // boundary. weezl::decode::Decoder::with_tiff_size_switch is
            // the libtiff-compatible mode every TIFF reader uses.
            let mut dec = weezl::decode::Decoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8);
            tile_bytes = dec
                .decode(&tile_compressed[..])
                .map_err(|e| CogError::Inflate(format!("lzw: {e}")))?;
        }
        _ => unreachable!("compression already validated above"),
    }

    // Undo Predictor 2 (horizontal differencing) row by row.
    if profile.predictor == 2 {
        let bps = (profile.bits_per_sample / 8) as usize;
        let row_bytes = (profile.tile_w as usize) * bps;
        match profile.bits_per_sample {
            16 => {
                for r in 0..profile.tile_h as usize {
                    let base = r * row_bytes;
                    let mut prev: u16 =
                        u16::from_le_bytes(tile_bytes[base..base + 2].try_into().unwrap());
                    for c in 1..profile.tile_w as usize {
                        let p = base + c * 2;
                        let cur_diff = u16::from_le_bytes(tile_bytes[p..p + 2].try_into().unwrap());
                        let v = prev.wrapping_add(cur_diff);
                        tile_bytes[p..p + 2].copy_from_slice(&v.to_le_bytes());
                        prev = v;
                    }
                }
            }
            8 => {
                for r in 0..profile.tile_h as usize {
                    let base = r * row_bytes;
                    let mut prev: u8 = tile_bytes[base];
                    for c in 1..profile.tile_w as usize {
                        let p = base + c;
                        let v = prev.wrapping_add(tile_bytes[p]);
                        tile_bytes[p] = v;
                        prev = v;
                    }
                }
            }
            32 => {
                for r in 0..profile.tile_h as usize {
                    let base = r * row_bytes;
                    let mut prev: u32 =
                        u32::from_le_bytes(tile_bytes[base..base + 4].try_into().unwrap());
                    for c in 1..profile.tile_w as usize {
                        let p = base + c * 4;
                        let cur_diff = u32::from_le_bytes(tile_bytes[p..p + 4].try_into().unwrap());
                        let v = prev.wrapping_add(cur_diff);
                        tile_bytes[p..p + 4].copy_from_slice(&v.to_le_bytes());
                        prev = v;
                    }
                }
            }
            _ => unreachable!(),
        }
    } else if profile.predictor == 3 {
        // Floating-point predictor (TIFF spec §14, libtiff `fpAcc`). Per
        // row: (1) undo horizontal byte-differencing across the full row,
        // then (2) reverse the MSB-first byte-plane shuffle. For an LE
        // f32 source value, the encoded byte planes are stored
        // [MSB-plane, …, LSB-plane], so for output pixel i and byte b
        // (b=0 LSB, b=bps-1 MSB):
        //   out[i*bps + b] = row[(bps - 1 - b) * tile_w + i]
        let bps = (profile.bits_per_sample / 8) as usize;
        if bps != 4 {
            return Err(CogError::Unsupported(format!(
                "predictor=3 only supported for 32-bit float (got bits_per_sample={})",
                profile.bits_per_sample
            )));
        }
        let tw = profile.tile_w as usize;
        let row_bytes = tw * bps;
        for r in 0..profile.tile_h as usize {
            let base = r * row_bytes;
            // 1) Undo horizontal byte-differencing across the whole row.
            for i in 1..row_bytes {
                tile_bytes[base + i] = tile_bytes[base + i].wrapping_add(tile_bytes[base + i - 1]);
            }
            // 2) Reverse the MSB-first byte-plane shuffle into LE-f32 bytes.
            let row: Vec<u8> = tile_bytes[base..base + row_bytes].to_vec();
            for i in 0..tw {
                for b in 0..bps {
                    tile_bytes[base + i * bps + b] = row[(bps - 1 - b) * tw + i];
                }
            }
        }
    } else if profile.predictor != 1 {
        return Err(CogError::Unsupported(format!(
            "predictor={} (1/2/3 supported)",
            profile.predictor
        )));
    }

    let bps = (profile.bits_per_sample / 8) as usize;
    let pixel_off =
        (intra_row as usize) * (profile.tile_w as usize) * bps + (intra_col as usize) * bps;
    if tile_bytes.len() < pixel_off + bps {
        return Err(CogError::Unsupported(format!(
            "decompressed tile too small: {} bytes, need >= {}",
            tile_bytes.len(),
            pixel_off + bps
        )));
    }
    let value = match (profile.bits_per_sample, profile.sample_format) {
        (16, 1) => {
            u16::from_le_bytes(tile_bytes[pixel_off..pixel_off + 2].try_into().unwrap()) as f64
        }
        (16, 2) => {
            i16::from_le_bytes(tile_bytes[pixel_off..pixel_off + 2].try_into().unwrap()) as f64
        }
        (8, 1) => tile_bytes[pixel_off] as f64,
        (8, 2) => (tile_bytes[pixel_off] as i8) as f64,
        (32, 3) => {
            f32::from_le_bytes(tile_bytes[pixel_off..pixel_off + 4].try_into().unwrap()) as f64
        }
        (32, 1) => {
            u32::from_le_bytes(tile_bytes[pixel_off..pixel_off + 4].try_into().unwrap()) as f64
        }
        (32, 2) => {
            i32::from_le_bytes(tile_bytes[pixel_off..pixel_off + 4].try_into().unwrap()) as f64
        }
        (b, sf) => {
            return Err(CogError::Unsupported(format!(
                "bits={b} sample_format={sf}"
            )))
        }
    };
    Ok(value)
}

/// Sample a `w × h` window of pixels centred on the world point.
/// Returns a row-major `Vec<f64>` (length `w*h`); pixels outside the
/// image bounds are filled with `0.0`. Iterates over the 1-4 tiles
/// covering the window, range-reads each once, and memcpys the
/// intersecting region. Supports the same set of (compression,
/// predictor, bits_per_sample) combinations as [`sample_pixel`] and
/// is restricted to `samples_per_pixel = 1` (Sentinel-2 / -1 single-band
/// COGs — the multimodal RGB compose path opens B04, B03, B02 separately).
pub async fn sample_window(
    client: &Client,
    url: &str,
    profile: &CogProfile,
    centre_x: f64,
    centre_y: f64,
    w: u32,
    h: u32,
) -> Result<Vec<f64>, CogError> {
    if profile.compression != 8 && profile.compression != 5 {
        return Err(CogError::Unsupported(format!(
            "compression={} (Deflate (8) and LZW (5) supported)",
            profile.compression
        )));
    }
    if profile.bits_per_sample != 16
        && profile.bits_per_sample != 8
        && profile.bits_per_sample != 32
    {
        return Err(CogError::Unsupported(format!(
            "bits_per_sample={} (8/16/32 supported)",
            profile.bits_per_sample
        )));
    }
    if profile.samples_per_pixel != 1 {
        return Err(CogError::Unsupported(format!(
            "sample_window: spp=1 only (got {})",
            profile.samples_per_pixel
        )));
    }
    let bps = (profile.bits_per_sample / 8) as usize;

    let (centre_col, centre_row) = profile.world_to_pixel(centre_x, centre_y);
    let half_w = (w as i64) / 2;
    let half_h = (h as i64) / 2;
    let want_col0 = centre_col - half_w;
    let want_row0 = centre_row - half_h;
    let want_col1 = want_col0 + (w as i64);
    let want_row1 = want_row0 + (h as i64);
    // Clip to image bounds; pixels outside stay zero in the output.
    let col0 = want_col0.max(0).min(profile.width as i64) as u32;
    let row0 = want_row0.max(0).min(profile.height as i64) as u32;
    let col1 = want_col1.max(0).min(profile.width as i64) as u32;
    let row1 = want_row1.max(0).min(profile.height as i64) as u32;

    let mut out = vec![0.0f64; (w as usize) * (h as usize)];
    if col0 >= col1 || row0 >= row1 {
        return Ok(out);
    }

    let tile_col_a = col0 / profile.tile_w;
    let tile_col_b = (col1 - 1) / profile.tile_w;
    let tile_row_a = row0 / profile.tile_h;
    let tile_row_b = (row1 - 1) / profile.tile_h;

    for tr in tile_row_a..=tile_row_b {
        for tc in tile_col_a..=tile_col_b {
            let tile_idx = (tr * profile.tile_cols + tc) as usize;
            if tile_idx >= profile.tile_offsets.len() {
                continue;
            }
            let off = profile.tile_offsets[tile_idx];
            let len = profile.tile_byte_counts[tile_idx];
            if len == 0 {
                continue;
            }
            let tile_compressed = http_range(client, url, off, off + len - 1).await?;
            let mut tile_bytes =
                Vec::with_capacity((profile.tile_w as usize) * (profile.tile_h as usize) * bps);
            match profile.compression {
                8 => {
                    let mut decoder = ZlibDecoder::new(&tile_compressed[..]);
                    decoder
                        .read_to_end(&mut tile_bytes)
                        .map_err(|e| CogError::Inflate(e.to_string()))?;
                }
                5 => {
                    let mut dec =
                        weezl::decode::Decoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8);
                    tile_bytes = dec
                        .decode(&tile_compressed[..])
                        .map_err(|e| CogError::Inflate(format!("lzw: {e}")))?;
                }
                _ => unreachable!(),
            }
            // Predictors 1/2/3 — same logic as sample_pixel. Kept
            // duplicated rather than refactored so sample_pixel's hot
            // path stays unchanged.
            if profile.predictor == 2 {
                let row_bytes = (profile.tile_w as usize) * bps;
                match profile.bits_per_sample {
                    16 => {
                        for r in 0..profile.tile_h as usize {
                            let base = r * row_bytes;
                            let mut prev: u16 =
                                u16::from_le_bytes(tile_bytes[base..base + 2].try_into().unwrap());
                            for c in 1..profile.tile_w as usize {
                                let p = base + c * 2;
                                let cur_diff =
                                    u16::from_le_bytes(tile_bytes[p..p + 2].try_into().unwrap());
                                let v = prev.wrapping_add(cur_diff);
                                tile_bytes[p..p + 2].copy_from_slice(&v.to_le_bytes());
                                prev = v;
                            }
                        }
                    }
                    8 => {
                        for r in 0..profile.tile_h as usize {
                            let base = r * row_bytes;
                            let mut prev: u8 = tile_bytes[base];
                            for c in 1..profile.tile_w as usize {
                                let p = base + c;
                                let v = prev.wrapping_add(tile_bytes[p]);
                                tile_bytes[p] = v;
                                prev = v;
                            }
                        }
                    }
                    32 => {
                        for r in 0..profile.tile_h as usize {
                            let base = r * row_bytes;
                            let mut prev: u32 =
                                u32::from_le_bytes(tile_bytes[base..base + 4].try_into().unwrap());
                            for c in 1..profile.tile_w as usize {
                                let p = base + c * 4;
                                let cur_diff =
                                    u32::from_le_bytes(tile_bytes[p..p + 4].try_into().unwrap());
                                let v = prev.wrapping_add(cur_diff);
                                tile_bytes[p..p + 4].copy_from_slice(&v.to_le_bytes());
                                prev = v;
                            }
                        }
                    }
                    _ => unreachable!(),
                }
            } else if profile.predictor == 3 {
                if bps != 4 {
                    return Err(CogError::Unsupported(format!(
                        "predictor=3 only supported for 32-bit float (got bits_per_sample={})",
                        profile.bits_per_sample
                    )));
                }
                let tw = profile.tile_w as usize;
                let row_bytes = tw * bps;
                for r in 0..profile.tile_h as usize {
                    let base = r * row_bytes;
                    for i in 1..row_bytes {
                        tile_bytes[base + i] =
                            tile_bytes[base + i].wrapping_add(tile_bytes[base + i - 1]);
                    }
                    let row: Vec<u8> = tile_bytes[base..base + row_bytes].to_vec();
                    for i in 0..tw {
                        for b in 0..bps {
                            tile_bytes[base + i * bps + b] = row[(bps - 1 - b) * tw + i];
                        }
                    }
                }
            } else if profile.predictor != 1 {
                return Err(CogError::Unsupported(format!(
                    "predictor={} (1/2/3 supported)",
                    profile.predictor
                )));
            }

            // Memcpy the intersection of [tile bbox] ∩ [window bbox]
            // into the output buffer. Output is row-major, indexed
            // off `want_col0` / `want_row0` (so pixels clipped at the
            // image edge land in the correct cell of the output).
            let tile_col0 = tc * profile.tile_w;
            let tile_row0 = tr * profile.tile_h;
            let inter_col_a = col0.max(tile_col0);
            let inter_col_b = col1.min(tile_col0 + profile.tile_w);
            let inter_row_a = row0.max(tile_row0);
            let inter_row_b = row1.min(tile_row0 + profile.tile_h);
            for r in inter_row_a..inter_row_b {
                for c in inter_col_a..inter_col_b {
                    let intra_col = (c - tile_col0) as usize;
                    let intra_row = (r - tile_row0) as usize;
                    let p = (intra_row * profile.tile_w as usize + intra_col) * bps;
                    if p + bps > tile_bytes.len() {
                        continue;
                    }
                    let v = match (profile.bits_per_sample, profile.sample_format) {
                        (16, 1) => {
                            u16::from_le_bytes(tile_bytes[p..p + 2].try_into().unwrap()) as f64
                        }
                        (16, 2) => {
                            i16::from_le_bytes(tile_bytes[p..p + 2].try_into().unwrap()) as f64
                        }
                        (8, 1) => tile_bytes[p] as f64,
                        (8, 2) => (tile_bytes[p] as i8) as f64,
                        (32, 3) => {
                            f32::from_le_bytes(tile_bytes[p..p + 4].try_into().unwrap()) as f64
                        }
                        (32, 1) => {
                            u32::from_le_bytes(tile_bytes[p..p + 4].try_into().unwrap()) as f64
                        }
                        (32, 2) => {
                            i32::from_le_bytes(tile_bytes[p..p + 4].try_into().unwrap()) as f64
                        }
                        _ => 0.0,
                    };
                    let out_col = (c as i64 - want_col0) as usize;
                    let out_row = (r as i64 - want_row0) as usize;
                    if out_col < w as usize && out_row < h as usize {
                        out[out_row * w as usize + out_col] = v;
                    }
                }
            }
        }
    }
    Ok(out)
}

/// Sample all `samples_per_pixel` channels at one pixel. Returns a Vec of
/// length equal to `profile.samples_per_pixel`. Works for chunky (planar=1)
/// rasters only; we hard-fail on planar=2 because the COGs we read
/// (Sentinel-2, Sentinel-1, multi-band STAC scenes) use chunky.
pub async fn sample_pixel_multi(
    client: &Client,
    url: &str,
    profile: &CogProfile,
    world_x: f64,
    world_y: f64,
) -> Result<Vec<f64>, CogError> {
    if profile.compression != 8 && profile.compression != 5 {
        return Err(CogError::Unsupported(format!(
            "compression={} (only Deflate (8) and LZW (5) supported in multi-band)",
            profile.compression
        )));
    }
    if profile.planar_config != 1 {
        return Err(CogError::Unsupported(format!(
            "planar_config={} (only chunky=1 supported)",
            profile.planar_config
        )));
    }
    let bps = (profile.bits_per_sample / 8) as usize;
    if !(profile.bits_per_sample == 8
        || profile.bits_per_sample == 16
        || profile.bits_per_sample == 32)
    {
        return Err(CogError::Unsupported(format!(
            "bits_per_sample={} (8/16/32 supported)",
            profile.bits_per_sample
        )));
    }
    let spp = profile.samples_per_pixel as usize;
    let stride = bps * spp;

    let (col, row) = profile.world_to_pixel(world_x, world_y);
    if col < 0 || row < 0 || col >= profile.width as i64 || row >= profile.height as i64 {
        return Err(CogError::Unsupported(format!(
            "world ({world_x:.3},{world_y:.3}) maps to pixel ({col},{row}) outside image {}x{}",
            profile.width, profile.height
        )));
    }
    let col = col as u32;
    let row = row as u32;
    let tile_col = col / profile.tile_w;
    let tile_row = row / profile.tile_h;
    let tile_idx = (tile_row * profile.tile_cols + tile_col) as usize;
    if tile_idx >= profile.tile_offsets.len() {
        return Err(CogError::Unsupported(format!(
            "tile_idx {tile_idx} out of range"
        )));
    }
    let intra_col = col - tile_col * profile.tile_w;
    let intra_row = row - tile_row * profile.tile_h;

    let off = profile.tile_offsets[tile_idx];
    let len = profile.tile_byte_counts[tile_idx];
    if len == 0 {
        return Err(CogError::Unsupported(format!(
            "tile {tile_idx} byte_count=0 (sparse — empty)"
        )));
    }
    let tile_compressed = http_range(client, url, off, off + len - 1).await?;

    let mut tile_bytes =
        Vec::with_capacity((profile.tile_w as usize) * (profile.tile_h as usize) * stride);
    match profile.compression {
        8 => {
            // Deflate.
            let mut decoder = ZlibDecoder::new(&tile_compressed[..]);
            decoder
                .read_to_end(&mut tile_bytes)
                .map_err(|e| CogError::Inflate(e.to_string()))?;
        }
        5 => {
            // LZW. TIFF spec dictates the MSB-first bit order with a
            // size-switch on the dictionary clear code — same as the
            // single-band sampler. weezl handles both.
            let mut dec = weezl::decode::Decoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8);
            tile_bytes = dec
                .decode(&tile_compressed[..])
                .map_err(|e| CogError::Inflate(format!("lzw: {e}")))?;
        }
        _ => unreachable!("compression already gated to 5 or 8 above"),
    }

    if profile.predictor == 2 {
        // Horizontal differencing applies per-sample within a row.
        let row_bytes = (profile.tile_w as usize) * stride;
        for r in 0..profile.tile_h as usize {
            for c_idx in 1..profile.tile_w as usize {
                for sample in 0..spp {
                    let p_prev = r * row_bytes + (c_idx - 1) * stride + sample * bps;
                    let p_cur = r * row_bytes + c_idx * stride + sample * bps;
                    match bps {
                        1 => {
                            let v = tile_bytes[p_prev].wrapping_add(tile_bytes[p_cur]);
                            tile_bytes[p_cur] = v;
                        }
                        2 => {
                            let prev = u16::from_le_bytes(
                                tile_bytes[p_prev..p_prev + 2].try_into().unwrap(),
                            );
                            let cur = u16::from_le_bytes(
                                tile_bytes[p_cur..p_cur + 2].try_into().unwrap(),
                            );
                            let v = prev.wrapping_add(cur);
                            tile_bytes[p_cur..p_cur + 2].copy_from_slice(&v.to_le_bytes());
                        }
                        4 => {
                            let prev = u32::from_le_bytes(
                                tile_bytes[p_prev..p_prev + 4].try_into().unwrap(),
                            );
                            let cur = u32::from_le_bytes(
                                tile_bytes[p_cur..p_cur + 4].try_into().unwrap(),
                            );
                            let v = prev.wrapping_add(cur);
                            tile_bytes[p_cur..p_cur + 4].copy_from_slice(&v.to_le_bytes());
                        }
                        _ => unreachable!(),
                    }
                }
            }
        }
    } else if profile.predictor != 1 {
        return Err(CogError::Unsupported(format!(
            "predictor={} (1/2 supported)",
            profile.predictor
        )));
    }

    let pixel_off =
        (intra_row as usize) * (profile.tile_w as usize) * stride + (intra_col as usize) * stride;
    if tile_bytes.len() < pixel_off + stride {
        return Err(CogError::Unsupported(format!(
            "decompressed tile too small: {} bytes, need >= {}",
            tile_bytes.len(),
            pixel_off + stride
        )));
    }
    let mut out = Vec::with_capacity(spp);
    for sample in 0..spp {
        let p = pixel_off + sample * bps;
        let v = match (profile.bits_per_sample, profile.sample_format) {
            (16, 1) => u16::from_le_bytes(tile_bytes[p..p + 2].try_into().unwrap()) as f64,
            (16, 2) => i16::from_le_bytes(tile_bytes[p..p + 2].try_into().unwrap()) as f64,
            (8, 1) => tile_bytes[p] as f64,
            (8, 2) => (tile_bytes[p] as i8) as f64,
            (32, 3) => f32::from_le_bytes(tile_bytes[p..p + 4].try_into().unwrap()) as f64,
            (32, 1) => u32::from_le_bytes(tile_bytes[p..p + 4].try_into().unwrap()) as f64,
            (32, 2) => i32::from_le_bytes(tile_bytes[p..p + 4].try_into().unwrap()) as f64,
            (b, sf) => {
                return Err(CogError::Unsupported(format!(
                    "bits={b} sample_format={sf}"
                )))
            }
        };
        out.push(v);
    }
    Ok(out)
}

async fn http_range(
    client: &Client,
    url: &str,
    start: u64,
    end_inclusive: u64,
) -> Result<Bytes, CogError> {
    // `file://<absolute-path>` short-circuits to a direct seek+read of
    // the local filesystem. Used by connectors that pull a one-time
    // bulk download (e.g. WRI GDM v1.2 single global COG, Zenodo no-Range)
    // and then sample the cached file like any other COG via the
    // shared sampler infrastructure. Keeps every COG-decoding code
    // path (IFD parse, tile sampling, multi-band sampler) unchanged.
    if let Some(path) = url.strip_prefix("file://") {
        use std::io::{Read, Seek, SeekFrom};
        let mut f = std::fs::File::open(path)
            .map_err(|e| CogError::Transport(format!("file open {path}: {e}")))?;
        f.seek(SeekFrom::Start(start))
            .map_err(|e| CogError::Transport(format!("file seek {path}: {e}")))?;
        let n = (end_inclusive - start + 1) as usize;
        let mut buf = vec![0u8; n];
        f.read_exact(&mut buf)
            .map_err(|e| CogError::Transport(format!("file read {path}: {e}")))?;
        return Ok(Bytes::from(buf));
    }
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
        .map_err(|e| CogError::Transport(e.to_string()))?;
    if !(resp.status() == reqwest::StatusCode::PARTIAL_CONTENT
        || resp.status() == reqwest::StatusCode::OK)
    {
        return Err(CogError::Transport(format!(
            "status {} for range {}-{} on {}",
            resp.status(),
            start,
            end_inclusive,
            url
        )));
    }
    resp.bytes()
        .await
        .map_err(|e| CogError::Transport(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal valid BigTIFF byte buffer in memory with exactly
    /// the IFD tags `parse_profile` needs to succeed. The layout is:
    ///
    /// ```text
    ///   off  bytes  field
    ///    0   16     header: II 2B 00 08 00 00 IFD0_OFFSET (u64)
    ///   16   24     ModelPixelScale doubles (sx, sy, sz)
    ///   40   48     ModelTiepoint doubles  (i, j, k, x, y, z)
    ///   88    8     IFD entry count (u64)
    ///   96   N*20   IFD entries
    ///   ...   8     next-IFD offset (u64, 0)
    ///   ...  rest   external arrays (TileOffsets / TileByteCounts)
    /// ```
    ///
    /// 2×2 tiles of 16×16 pixels each → 32×32 image. Compression=1
    /// (no codec — `parse_profile` never decompresses; that's
    /// `sample_pixel`'s job). The tile arrays are tiny synthetic
    /// LONG8 values that exercise the BigTIFF-only type-16 path.
    fn build_synthetic_bigtiff() -> Vec<u8> {
        let mut buf = Vec::new();
        // Header.
        buf.extend_from_slice(b"II");
        buf.extend_from_slice(&0x002B_u16.to_le_bytes());
        buf.extend_from_slice(&8_u16.to_le_bytes());
        buf.extend_from_slice(&0_u16.to_le_bytes());
        // IFD0 offset will be patched once we know the size of the
        // pre-IFD blob (header + ModelPixelScale + ModelTiepoint).
        let ifd0_off_pos = buf.len();
        buf.extend_from_slice(&0_u64.to_le_bytes());

        // ModelPixelScale doubles: sx=1.0, sy=1.0, sz=0.0
        let pixel_scale_off = buf.len() as u64;
        buf.extend_from_slice(&1.0_f64.to_le_bytes());
        buf.extend_from_slice(&1.0_f64.to_le_bytes());
        buf.extend_from_slice(&0.0_f64.to_le_bytes());

        // ModelTiepoint doubles: (i=0, j=0, k=0, x=100, y=200, z=0)
        let tiepoint_off = buf.len() as u64;
        for v in [0.0_f64, 0.0, 0.0, 100.0, 200.0, 0.0] {
            buf.extend_from_slice(&v.to_le_bytes());
        }

        // Patch IFD0 offset.
        let ifd0_off = buf.len() as u64;
        buf[ifd0_off_pos..ifd0_off_pos + 8].copy_from_slice(&ifd0_off.to_le_bytes());

        // Tags we'll write (in ascending order per TIFF spec):
        //  256 Width=32, 257 Height=32, 258 BitsPerSample=8,
        //  259 Compression=1, 277 SamplesPerPixel=1,
        //  284 PlanarConfig=1, 317 Predictor=1,
        //  322 TileWidth=16, 323 TileLength=16,
        //  324 TileOffsets (LONG8 array of 4), 325 TileByteCounts (LONG8 array of 4),
        //  339 SampleFormat=1, 33550 ModelPixelScale, 33922 ModelTiepoint
        let n_entries: u64 = 14;
        buf.extend_from_slice(&n_entries.to_le_bytes());

        // Helper closure to push a BigTIFF entry with inline 8-byte
        // value slot. `val` already encodes whatever the tag's value
        // field should hold (SHORT-in-slot, LONG-in-slot, or u64 offset).
        let push_entry = |buf: &mut Vec<u8>, tag: u16, typ: u16, cnt: u64, val: u64| {
            buf.extend_from_slice(&tag.to_le_bytes());
            buf.extend_from_slice(&typ.to_le_bytes());
            buf.extend_from_slice(&cnt.to_le_bytes());
            buf.extend_from_slice(&val.to_le_bytes());
        };
        // For SHORT inline values, the low 2 bytes carry the value and
        // the rest is zero — `to_le_bytes` of a u16-widened-to-u64 does
        // exactly that.
        push_entry(&mut buf, 256, 4, 1, 32); // Width LONG
        push_entry(&mut buf, 257, 4, 1, 32); // Height LONG
        push_entry(&mut buf, 258, 3, 1, 8); // BitsPerSample SHORT
        push_entry(&mut buf, 259, 3, 1, 1); // Compression SHORT (none)
        push_entry(&mut buf, 277, 3, 1, 1); // SamplesPerPixel SHORT
        push_entry(&mut buf, 284, 3, 1, 1); // PlanarConfig SHORT
        push_entry(&mut buf, 317, 3, 1, 1); // Predictor SHORT
        push_entry(&mut buf, 322, 3, 1, 16); // TileWidth SHORT
        push_entry(&mut buf, 323, 3, 1, 16); // TileLength SHORT
                                             // We need to know where the external TileOffsets / TileByteCounts
                                             // arrays will live; pre-allocate by remembering positions to patch.
        let tile_offsets_entry_val_pos = buf.len() + 12;
        push_entry(&mut buf, 324, 16, 4, 0); // TileOffsets LONG8 [4]
        let tile_bc_entry_val_pos = buf.len() + 12;
        push_entry(&mut buf, 325, 16, 4, 0); // TileByteCounts LONG8 [4]
        push_entry(&mut buf, 339, 3, 1, 1); // SampleFormat SHORT
        push_entry(&mut buf, 33550, 12, 3, pixel_scale_off);
        push_entry(&mut buf, 33922, 12, 6, tiepoint_off);

        // Next-IFD offset (BigTIFF: u64).
        buf.extend_from_slice(&0_u64.to_le_bytes());

        // External TileOffsets / TileByteCounts arrays (4 entries each, LONG8).
        let toff_array_off = buf.len() as u64;
        for off in [10_000_u64, 11_000, 12_000, 13_000] {
            buf.extend_from_slice(&off.to_le_bytes());
        }
        let tbc_array_off = buf.len() as u64;
        for n in [256_u64, 256, 256, 256] {
            buf.extend_from_slice(&n.to_le_bytes());
        }

        // Patch the TileOffsets / TileByteCounts entry value slots.
        buf[tile_offsets_entry_val_pos..tile_offsets_entry_val_pos + 8]
            .copy_from_slice(&toff_array_off.to_le_bytes());
        buf[tile_bc_entry_val_pos..tile_bc_entry_val_pos + 8]
            .copy_from_slice(&tbc_array_off.to_le_bytes());

        buf
    }

    /// `parse_profile` MUST recognise BigTIFF's `0x002B` magic word and
    /// decode the 16-byte header into the same `CogProfile` shape it
    /// produces for classic TIFF. The synthetic file exercises every
    /// BigTIFF-only path: 8-byte IFD entry count, 20-byte entries, u64
    /// value slot, and LONG8 (type 16) TileOffsets / TileByteCounts.
    /// Any regression in flavour-routing surfaces as a missing tag or
    /// wrong scalar here.
    #[test]
    fn parse_profile_decodes_bigtiff_header() {
        let buf = build_synthetic_bigtiff();
        let prof = parse_profile(&buf).expect("BigTIFF header must parse");
        assert_eq!(prof.width, 32);
        assert_eq!(prof.height, 32);
        assert_eq!(prof.bits_per_sample, 8);
        assert_eq!(prof.compression, 1);
        assert_eq!(prof.predictor, 1);
        assert_eq!(prof.samples_per_pixel, 1);
        assert_eq!(prof.planar_config, 1);
        assert_eq!(prof.tile_w, 16);
        assert_eq!(prof.tile_h, 16);
        assert_eq!(prof.tile_cols, 2);
        assert_eq!(prof.tile_rows, 2);
        assert_eq!(prof.tile_offsets, vec![10_000, 11_000, 12_000, 13_000]);
        assert_eq!(prof.tile_byte_counts, vec![256, 256, 256, 256]);
        assert_eq!(prof.pixel_scale, (1.0, 1.0));
        assert_eq!(prof.tiepoint, (0.0, 0.0, 100.0, 200.0));
        assert_eq!(prof.sample_format, 1);
    }

    /// The std-TIFF magic `0x002A` path must still parse correctly —
    /// the BigTIFF additive support must not regress classic IFD
    /// decoding. Builds a minimal 32×32 single-tile std TIFF in
    /// memory and asserts the parsed profile matches the synthetic.
    #[test]
    fn parse_profile_decodes_standard_tiff_header() {
        // Layout: 8-byte header, then ModelPixelScale (24 B) + ModelTiepoint
        // (48 B), then IFD0 with 14 entries × 12 B + 2 B count + 4 B next.
        let mut buf = Vec::new();
        buf.extend_from_slice(b"II");
        buf.extend_from_slice(&0x002A_u16.to_le_bytes());
        let ifd0_off_pos = buf.len();
        buf.extend_from_slice(&0_u32.to_le_bytes());

        let pixel_scale_off = buf.len() as u32;
        for v in [1.0_f64, 1.0, 0.0] {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        let tiepoint_off = buf.len() as u32;
        for v in [0.0_f64, 0.0, 0.0, 100.0, 200.0, 0.0] {
            buf.extend_from_slice(&v.to_le_bytes());
        }

        let ifd0_off = buf.len() as u32;
        buf[ifd0_off_pos..ifd0_off_pos + 4].copy_from_slice(&ifd0_off.to_le_bytes());

        let n_entries: u16 = 14;
        buf.extend_from_slice(&n_entries.to_le_bytes());

        // 12-byte std TIFF entries with a 4-byte inline value slot.
        let push_entry = |buf: &mut Vec<u8>, tag: u16, typ: u16, cnt: u32, val: u32| {
            buf.extend_from_slice(&tag.to_le_bytes());
            buf.extend_from_slice(&typ.to_le_bytes());
            buf.extend_from_slice(&cnt.to_le_bytes());
            buf.extend_from_slice(&val.to_le_bytes());
        };
        push_entry(&mut buf, 256, 4, 1, 32);
        push_entry(&mut buf, 257, 4, 1, 32);
        push_entry(&mut buf, 258, 3, 1, 8);
        push_entry(&mut buf, 259, 3, 1, 1);
        push_entry(&mut buf, 277, 3, 1, 1);
        push_entry(&mut buf, 284, 3, 1, 1);
        push_entry(&mut buf, 317, 3, 1, 1);
        push_entry(&mut buf, 322, 3, 1, 16);
        push_entry(&mut buf, 323, 3, 1, 16);
        let tile_offsets_entry_val_pos = buf.len() + 8;
        push_entry(&mut buf, 324, 4, 4, 0); // TileOffsets LONG [4]
        let tile_bc_entry_val_pos = buf.len() + 8;
        push_entry(&mut buf, 325, 4, 4, 0);
        push_entry(&mut buf, 339, 3, 1, 1);
        push_entry(&mut buf, 33550, 12, 3, pixel_scale_off);
        push_entry(&mut buf, 33922, 12, 6, tiepoint_off);
        buf.extend_from_slice(&0_u32.to_le_bytes()); // next IFD

        let toff_array_off = buf.len() as u32;
        for off in [10_000_u32, 11_000, 12_000, 13_000] {
            buf.extend_from_slice(&off.to_le_bytes());
        }
        let tbc_array_off = buf.len() as u32;
        for n in [256_u32, 256, 256, 256] {
            buf.extend_from_slice(&n.to_le_bytes());
        }
        buf[tile_offsets_entry_val_pos..tile_offsets_entry_val_pos + 4]
            .copy_from_slice(&toff_array_off.to_le_bytes());
        buf[tile_bc_entry_val_pos..tile_bc_entry_val_pos + 4]
            .copy_from_slice(&tbc_array_off.to_le_bytes());

        let prof = parse_profile(&buf).expect("standard TIFF must still parse");
        assert_eq!(prof.width, 32);
        assert_eq!(prof.height, 32);
        assert_eq!(prof.bits_per_sample, 8);
        assert_eq!(prof.tile_w, 16);
        assert_eq!(prof.tile_h, 16);
        assert_eq!(prof.tile_offsets, vec![10_000, 11_000, 12_000, 13_000]);
        assert_eq!(prof.tile_byte_counts, vec![256, 256, 256, 256]);
    }

    /// Write a synthetic BigTIFF (the same buffer the parse tests above
    /// use) to a unique temp file and return its `file://` URL. The
    /// cog sampler short-circuits `file://` URLs through a direct
    /// filesystem read in `http_range`, so we can exercise the full
    /// `open_profile` pipeline (cache lookup → uncached fetch → parse)
    /// without touching the network.
    fn write_synthetic_bigtiff_to_tempfile() -> String {
        let mut buf = build_synthetic_bigtiff();
        // open_profile's first read is a 64 KiB range. The `file://`
        // short-circuit uses `read_exact`, which fails on EOF — so we
        // pad the synthetic file (it's only a few hundred bytes of
        // real content) up past the 64 KiB initial window. The trailing
        // zeros are never parsed; parse_profile stops after the IFD0
        // chain ends with a 0 next-IFD pointer.
        buf.resize(buf.len().max(128 * 1024), 0);
        // Unique filename per call so two tests running in the same
        // process never collide on a cached entry from a prior test.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("emem_cog_cache_test_{pid}_{nanos}.tif"));
        std::fs::write(&path, &buf).expect("write temp tiff");
        format!("file://{}", path.display())
    }

    /// Second `open_profile` call for the same URL must return the same
    /// `Arc<CogProfile>` — i.e. the cache hit short-circuits past the
    /// fetch+parse work. Without this, the JRC GFC2020 V3 path's
    /// ~110 MB external-array re-fetch happens per sample.
    #[tokio::test]
    async fn open_profile_is_cached_after_first_call() {
        let url = write_synthetic_bigtiff_to_tempfile();
        let client = Client::new();
        let p1 = open_profile(&client, &url).await.expect("first call");
        let p2 = open_profile(&client, &url).await.expect("second call");
        assert!(
            Arc::ptr_eq(&p1, &p2),
            "cache hit must return the same Arc (got fresh allocations)"
        );
    }

    /// Concurrent open_profile calls for the same URL must single-flight
    /// the underlying fetch+parse — every caller gets the same `Arc`
    /// back. We can't directly count file opens (the `file://` branch
    /// is sync inside an async function), so the contract is pinned by
    /// pointer equality across 8 concurrent callers: if any caller
    /// raced past the OnceCell and produced its own Arc, ptr_eq would
    /// fail for at least one pair.
    #[tokio::test]
    async fn open_profile_single_flights_concurrent_callers() {
        let url = write_synthetic_bigtiff_to_tempfile();
        let client = Client::new();
        let mut handles = Vec::new();
        for _ in 0..8 {
            let cli = client.clone();
            let u = url.clone();
            handles.push(tokio::spawn(async move {
                open_profile(&cli, &u)
                    .await
                    .expect("concurrent open_profile")
            }));
        }
        let mut arcs: Vec<Arc<CogProfile>> = Vec::new();
        for h in handles {
            arcs.push(h.await.expect("join task"));
        }
        let first = arcs[0].clone();
        for (i, a) in arcs.iter().enumerate().skip(1) {
            assert!(
                Arc::ptr_eq(&first, a),
                "caller {i} got a different Arc — single-flight broken"
            );
        }
    }

    /// Live JRC GFC2020 V3 timing assertion. First call exercises the
    /// full retry loop (~110 MB of TileOffsets + TileByteCounts at
    /// LONG8 over 6.9 M tiles) and is slow; the second call must be a
    /// pure HashMap + Arc::clone and finish in well under 100 ms. Gated
    /// behind `#[ignore]` so CI doesn't hit JEODPP; run manually with
    /// `cargo test --ignored -p emem-fetch open_profile_jrc_gfc2020_v3_second_call_is_fast --nocapture`.
    #[tokio::test]
    #[ignore = "live network test against jeodpp.jrc.ec.europa.eu — run with --ignored"]
    async fn open_profile_jrc_gfc2020_v3_second_call_is_fast() {
        let url = "https://jeodpp.jrc.ec.europa.eu/ftp/jrc-opendata/FOREST/GFC2020/LATEST/single-cog/JRC_GFC2020_V3_COG.tif";
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .expect("client");
        let t1 = std::time::Instant::now();
        let p1 = match open_profile(&client, url).await {
            Ok(p) => p,
            Err(CogError::Transport(s)) => {
                eprintln!("[skip] open_profile_jrc_gfc2020_v3_second_call_is_fast: transport: {s}");
                return;
            }
            Err(e) => panic!("unexpected COG error: {e}"),
        };
        let first = t1.elapsed();
        let t2 = std::time::Instant::now();
        let p2 = open_profile(&client, url).await.expect("second call");
        let second = t2.elapsed();
        eprintln!(
            "JRC GFC2020 V3 open_profile timing: first={:?} second={:?}",
            first, second
        );
        assert!(Arc::ptr_eq(&p1, &p2), "cache hit must return the same Arc");
        assert!(
            second.as_millis() < 100,
            "second call must hit cache, got {:?} (first was {:?})",
            second,
            first
        );
    }

    /// Header smoke test against the live 41 GB JRC GFC2020 V3 BigTIFF.
    /// Gated behind `#[ignore]` so CI doesn't hit the JRC's JEODPP
    /// bucket. Run manually with
    /// `cargo test --ignored -p emem-fetch jrc_gfc2020_header_parses`.
    ///
    /// The test fetches the first 64 KiB of the file, lets `open_profile`
    /// run its retry loop (the TileByteCounts array lands ~55 MB into
    /// the file and triggers a second range read), and prints the parsed
    /// profile. The asserts pin the shape we expect for a global
    /// 10 m EPSG:4326 uint8 forest mask.
    #[tokio::test]
    #[ignore = "live network test against jeodpp.jrc.ec.europa.eu — run with --ignored"]
    async fn jrc_gfc2020_header_parses() {
        let url = "https://jeodpp.jrc.ec.europa.eu/ftp/jrc-opendata/FOREST/GFC2020/LATEST/single-cog/JRC_GFC2020_V3_COG.tif";
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .expect("client");
        let prof = match open_profile(&client, url).await {
            Ok(p) => p,
            Err(CogError::Transport(s)) => {
                // Network failure → skip per the test spec; do not
                // fail the run on JEODPP outage.
                eprintln!("[skip] jrc_gfc2020_header_parses: transport: {s}");
                return;
            }
            Err(e) => panic!("unexpected COG error: {e}"),
        };
        eprintln!(
            "JRC GFC2020 V3 parsed profile: \
             width={} height={} bits={} sample_format={} compression={} \
             predictor={} tile_w={} tile_h={} tile_cols={} tile_rows={} \
             samples_per_pixel={} planar={} pixel_scale={:?} tiepoint={:?} \
             epsg={:?} nodata={:?} tile_offsets.len={} tile_byte_counts.len={}",
            prof.width,
            prof.height,
            prof.bits_per_sample,
            prof.sample_format,
            prof.compression,
            prof.predictor,
            prof.tile_w,
            prof.tile_h,
            prof.tile_cols,
            prof.tile_rows,
            prof.samples_per_pixel,
            prof.planar_config,
            prof.pixel_scale,
            prof.tiepoint,
            prof.epsg,
            prof.nodata,
            prof.tile_offsets.len(),
            prof.tile_byte_counts.len(),
        );
        // Global 10 m raster — must be very wide.
        assert!(
            prof.width > 1_000_000,
            "width={} too small for a 10 m global product",
            prof.width
        );
        assert!(
            prof.height > 1_000_000,
            "height={} too small for a 10 m global product",
            prof.height
        );
        // Single-band uint8 forest mask.
        assert_eq!(prof.bits_per_sample, 8, "GFC2020 V3 is 8-bit");
        assert_eq!(prof.samples_per_pixel, 1, "GFC2020 V3 is single-band");
        // 1024×1024 tiles are typical for global JRC COGs (we observed
        // tile_w=1024 in the 1024-byte header probe). Allow any power
        // of two ≥ 256 to avoid false failures if JRC bumps the tile size.
        assert!(
            prof.tile_w >= 256 && prof.tile_w.is_power_of_two(),
            "tile_w={} unexpected",
            prof.tile_w
        );
        assert!(
            prof.tile_h >= 256 && prof.tile_h.is_power_of_two(),
            "tile_h={} unexpected",
            prof.tile_h
        );
        // Compression must be 1 (none), 5 (LZW), or 8 (Deflate) for
        // the downstream sample path to succeed.
        assert!(
            matches!(prof.compression, 1 | 5 | 8),
            "compression={} not supported",
            prof.compression
        );
        // EPSG: the GeoKeyDirectory reader in this module only looks at
        // key 3072 (ProjectedCSTypeGeoKey). GFC2020 V3 is a geographic
        // (lat/lng) raster, so it stores its CRS under key 2048
        // (GeographicTypeGeoKey) which we don't parse — `epsg` is
        // therefore `None`, and the `jrc_gfc2020` connector knows the
        // CRS out-of-band (EPSG:4326). If `epsg` is `Some(...)` we
        // expect 4326; otherwise `None` is acceptable.
        match prof.epsg {
            None => {}
            Some(4326) => {}
            Some(other) => panic!("unexpected EPSG {other} (want None or 4326)"),
        }
        // Tiepoint should map pixel (0,0) to (-180°, +northern-edge°)
        // — pin the (x, y) but allow drift in the latitude edge in
        // case the JRC reshapes the bounding box.
        let (i, j, x, y) = prof.tiepoint;
        assert_eq!((i, j), (0.0, 0.0), "pixel origin must be (0,0)");
        assert!(
            (x - (-180.0)).abs() < 1e-6,
            "world-origin x must be -180°, got {x}"
        );
        assert!(
            (0.0..=90.0).contains(&y),
            "world-origin y must be in [0°, 90°], got {y}"
        );
        // ~10 m at the equator → pixel scale is 1/12000 deg.
        let expected_scale = 1.0_f64 / 12_000.0;
        assert!(
            (prof.pixel_scale.0 - expected_scale).abs() < 1e-9
                && (prof.pixel_scale.1 - expected_scale).abs() < 1e-9,
            "pixel_scale {:?} not ~ {expected_scale}",
            prof.pixel_scale
        );
    }
}
