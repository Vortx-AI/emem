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
//! BitsPerSample 16 (uint), and the GeoTIFF tags. Anything fancier (BigTIFF,
//! JPEG2000, LZW, etc.) returns an error rather than silently doing the wrong
//! thing — the protocol's no-fallback rule applies.

use std::io::Read;

use bytes::Bytes;
use flate2::read::ZlibDecoder;
use reqwest::Client;

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

/// Range-read the head of a COG and parse IFD0. Returns the metadata needed
/// for `sample_pixel`.
///
/// The first read pulls 64 KiB which is enough for Sentinel-2 / -1 in
/// practice. COGs that use the **end-of-file IFD layout** (IFD0 written
/// after all the tile data, e.g. JRC Global Surface Water — 40000×40000
/// pixels with IFD0 at byte ~86 M out of ~86 M file size) refer to
/// external arrays and tag values at offsets that span the entire file.
/// Each such reference triggers a fresh `ShortRead{needed}` — different
/// entries have different `needed` values — so we loop the retry,
/// expanding the buffer to cover whatever offset parse_profile is stuck
/// on, up to a hard cap of 8 iterations to avoid pathological cases.
pub async fn open_profile(client: &Client, url: &str) -> Result<CogProfile, CogError> {
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
    if magic != 42 {
        return Err(CogError::BadMagic(magic));
    }
    let ifd0_off = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]) as usize;
    if buf.len() < ifd0_off + 2 {
        return Err(CogError::ShortRead {
            needed: ifd0_off + 2,
            offset: 0,
        });
    }
    let n = u16::from_le_bytes([buf[ifd0_off], buf[ifd0_off + 1]]) as usize;
    let entries_start = ifd0_off + 2;
    if buf.len() < entries_start + n * 12 {
        return Err(CogError::ShortRead {
            needed: entries_start + n * 12,
            offset: 0,
        });
    }

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
    let mut tile_offsets_ref: Option<(usize, usize)> = None; // (cnt, off)
    let mut tile_byte_counts_ref: Option<(usize, usize)> = None;
    // TIFF strip tags. Hansen GFC, older USGS DEMs, and some MODIS subsets
    // ship as stripped TIFFs (no tile tags). Strips are essentially tiles
    // of width = image_width and height = rows_per_strip; synthesize the
    // tile_* fields from them so the downstream sampler stays uniform.
    let mut rows_per_strip: Option<u32> = None;
    let mut strip_offsets_ref: Option<(usize, usize)> = None;
    let mut strip_byte_counts_ref: Option<(usize, usize)> = None;
    let mut pixel_scale: Option<(f64, f64)> = None;
    let mut tiepoint: Option<(f64, f64, f64, f64)> = None;
    let mut geokey_ref: Option<(usize, usize)> = None;
    let mut nodata: Option<String> = None;

    for i in 0..n {
        let e = entries_start + i * 12;
        let tag = u16::from_le_bytes([buf[e], buf[e + 1]]);
        let _typ = u16::from_le_bytes([buf[e + 2], buf[e + 3]]);
        let cnt = u32::from_le_bytes([buf[e + 4], buf[e + 5], buf[e + 6], buf[e + 7]]) as usize;
        let raw = &buf[e + 8..e + 12];
        let val_u32 = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize;
        let val_u16_first = u16::from_le_bytes([raw[0], raw[1]]);

        match tag {
            256 => width = Some(val_u32 as u32),
            257 => height = Some(val_u32 as u32),
            258 => {
                // BitsPerSample is a SHORT array of length `samples_per_pixel`.
                // TIFF packs values inline only when total size ≤ 4 bytes;
                // beyond that, the entry's value field is an offset to an
                // external array. Single-band files (cnt=1) fit inline;
                // multi-band files like WRI GDM v1.2 (cnt=8, 16 bytes)
                // dereference. We pick the first u16 either way and assume
                // the bands share BitsPerSample — the existing per-sample
                // decoders downstream all read at this resolution. If a
                // future multi-band file mixes bit-widths the open_profile
                // would need an array readback, but every multi-band raster
                // we sample today uses a uniform width.
                bits_per_sample = if cnt * 2 <= 4 {
                    val_u16_first
                } else {
                    let off = val_u32;
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
            322 => tile_w = Some(val_u32 as u32),
            323 => tile_h = Some(val_u32 as u32),
            324 => tile_offsets_ref = Some((cnt, val_u32)),
            325 => tile_byte_counts_ref = Some((cnt, val_u32)),
            // Strip TIFF tags (273/278/279). When present without tile tags
            // (322..=325), strips are folded into the tile model below.
            273 => strip_offsets_ref = Some((cnt, val_u32)),
            278 => rows_per_strip = Some(val_u32 as u32),
            279 => strip_byte_counts_ref = Some((cnt, val_u32)),
            339 => {
                // SampleFormat is also a per-band SHORT array. Same
                // inline-vs-offset rule as BitsPerSample (tag 258). Read
                // the first entry; downstream decoders assume uniform
                // sample format across bands.
                sample_format = if cnt * 2 <= 4 {
                    val_u16_first
                } else {
                    let off = val_u32;
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
                // ModelPixelScale: 3 doubles (sx, sy, sz)
                if cnt < 2 {
                    continue;
                }
                let off = val_u32;
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
                let off = val_u32;
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
                geokey_ref = Some((cnt, val_u32));
            }
            42113 => {
                // GDAL_NODATA: ASCII
                if cnt <= 4 {
                    let s = std::str::from_utf8(&raw[..cnt.min(4)])
                        .unwrap_or("")
                        .trim_end_matches('\0')
                        .to_string();
                    nodata = Some(s);
                } else {
                    let off = val_u32;
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
    let (toff_cnt, toff_off) = tile_offsets_ref.ok_or(CogError::MissingTag(324))?;
    let (tbc_cnt, tbc_off) = tile_byte_counts_ref.ok_or(CogError::MissingTag(325))?;
    if toff_cnt != tbc_cnt {
        return Err(CogError::Unsupported(format!(
            "tile_offsets cnt {toff_cnt} != tile_byte_counts cnt {tbc_cnt}"
        )));
    }

    if buf.len() < toff_off + toff_cnt * 4 {
        return Err(CogError::ShortRead {
            needed: toff_off + toff_cnt * 4,
            offset: 0,
        });
    }
    if buf.len() < tbc_off + tbc_cnt * 4 {
        return Err(CogError::ShortRead {
            needed: tbc_off + tbc_cnt * 4,
            offset: 0,
        });
    }
    let mut tile_offsets = Vec::with_capacity(toff_cnt);
    for k in 0..toff_cnt {
        let p = toff_off + k * 4;
        tile_offsets.push(u32::from_le_bytes(buf[p..p + 4].try_into().unwrap()) as u64);
    }
    let mut tile_byte_counts = Vec::with_capacity(tbc_cnt);
    for k in 0..tbc_cnt {
        let p = tbc_off + k * 4;
        tile_byte_counts.push(u32::from_le_bytes(buf[p..p + 4].try_into().unwrap()) as u64);
    }

    let tile_cols = width.div_ceil(tile_w);
    let tile_rows = height.div_ceil(tile_h);
    if (tile_cols as usize) * (tile_rows as usize) != toff_cnt {
        return Err(CogError::Unsupported(format!(
            "tile grid {}x{} != tile_offsets count {}",
            tile_cols, tile_rows, toff_cnt
        )));
    }

    // Try to find EPSG via GeoKeyDirectory key 3072 (ProjectedCSTypeGeoKey).
    let mut epsg: Option<u32> = None;
    if let Some((cnt, off)) = geokey_ref {
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
            let mut dec =
                weezl::decode::Decoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8);
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
