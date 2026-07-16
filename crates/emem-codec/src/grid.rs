//! The canonical field-grid encoding: the bytes an `artifact_cid` hashes.
//!
//! docs/plans/field-tokens.md, section 1: a field artifact is "a
//! deterministic little-endian f32 row-major grid with a fixed 64-byte
//! header carrying dims, nodata, and CRS; GeoTIFF rendering is a lossy
//! VIEW of the artifact, never the signed thing, because compressors
//! are not byte-stable across versions." This module is that encoding,
//! and determinism here is a protocol rule, not a style preference:
//! two responders that compute the same field must emit the same bytes
//! or content addressing itself splits.
//!
//! Layout, 64-byte header then `width * height` little-endian `f32`
//! values, row-major, north row first:
//!
//! ```text
//!   offset  size  field
//!        0     8  magic "EMEMGRD1"
//!        8     4  width  (u32 LE, pixels per row)
//!       12     4  height (u32 LE, rows)
//!       16     4  epsg   (u32 LE; 4326 for the shipped grid)
//!       20     4  nodata (f32 LE bit pattern; see NaN rule)
//!       24     8  lat0   (f64 LE, latitude of the FIRST row's centre)
//!       32     8  lng0   (f64 LE, longitude of the first column's centre)
//!       40     8  dlat   (f64 LE, latitude step per row, negative going south)
//!       48     8  dlng   (f64 LE, longitude step per column)
//!       56     8  reserved, zero
//! ```
//!
//! The NaN rule mirrors canonical CBOR's (whitepaper section 5): every
//! NaN payload folds to the one quiet NaN `0x7fc0_0000` and `-0.0`
//! becomes `0.0` before the bytes are laid down. Without this, two
//! encoders that agree on every value still disagree on the digest.

/// Magic bytes; the trailing 1 is the version.
pub const GRID_MAGIC: [u8; 8] = *b"EMEMGRD1";
/// Header length in bytes.
pub const GRID_HEADER_LEN: usize = 64;
/// The canonical quiet NaN bit pattern, the CBOR discipline applied to f32.
pub const CANONICAL_NAN_F32: u32 = 0x7fc0_0000;

/// Everything the header carries. `nodata` is compared by bit pattern,
/// so a NaN nodata is representable and stable.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridHeader {
    pub width: u32,
    pub height: u32,
    pub epsg: u32,
    pub nodata: f32,
    pub lat0: f64,
    pub lng0: f64,
    pub dlat: f64,
    pub dlng: f64,
}

/// Encoding refusals, each a caller error stated plainly.
#[derive(Debug, PartialEq, Eq)]
pub enum GridError {
    /// `values.len() != width * height`.
    DimensionMismatch { expected: usize, got: usize },
    /// Zero-sized grids have no canonical bytes worth signing.
    EmptyGrid,
    /// Decode: the bytes are not a grid (bad magic, short header,
    /// or a length that disagrees with the header's dims).
    NotAGrid(&'static str),
}

fn canonical_f32(v: f32) -> u32 {
    if v.is_nan() {
        CANONICAL_NAN_F32
    } else if v == 0.0 {
        // folds -0.0 to 0.0; the comparison is true for both zeros.
        0
    } else {
        v.to_bits()
    }
}

/// Encode a grid to its canonical bytes. Deterministic by construction:
/// fixed header layout, canonical NaN and zero, little-endian
/// throughout, no compression.
pub fn encode_grid(h: &GridHeader, values: &[f32]) -> Result<Vec<u8>, GridError> {
    let cells = (h.width as usize) * (h.height as usize);
    if cells == 0 {
        return Err(GridError::EmptyGrid);
    }
    if values.len() != cells {
        return Err(GridError::DimensionMismatch {
            expected: cells,
            got: values.len(),
        });
    }
    let mut out = Vec::with_capacity(GRID_HEADER_LEN + 4 * cells);
    out.extend_from_slice(&GRID_MAGIC);
    out.extend_from_slice(&h.width.to_le_bytes());
    out.extend_from_slice(&h.height.to_le_bytes());
    out.extend_from_slice(&h.epsg.to_le_bytes());
    out.extend_from_slice(&canonical_f32(h.nodata).to_le_bytes());
    out.extend_from_slice(&h.lat0.to_le_bytes());
    out.extend_from_slice(&h.lng0.to_le_bytes());
    out.extend_from_slice(&h.dlat.to_le_bytes());
    out.extend_from_slice(&h.dlng.to_le_bytes());
    out.extend_from_slice(&[0u8; 8]);
    debug_assert_eq!(out.len(), GRID_HEADER_LEN);
    for v in values {
        out.extend_from_slice(&canonical_f32(*v).to_le_bytes());
    }
    Ok(out)
}

/// Decode canonical grid bytes. The inverse of [`encode_grid`] for
/// well-formed input; anything else is a typed refusal, never a panic.
pub fn decode_grid(bytes: &[u8]) -> Result<(GridHeader, Vec<f32>), GridError> {
    if bytes.len() < GRID_HEADER_LEN {
        return Err(GridError::NotAGrid("shorter than the 64-byte header"));
    }
    if bytes[..8] != GRID_MAGIC {
        return Err(GridError::NotAGrid("bad magic"));
    }
    let u32_at = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
    let f64_at = |o: usize| f64::from_le_bytes(bytes[o..o + 8].try_into().unwrap());
    let h = GridHeader {
        width: u32_at(8),
        height: u32_at(12),
        epsg: u32_at(16),
        nodata: f32::from_bits(u32_at(20)),
        lat0: f64_at(24),
        lng0: f64_at(32),
        dlat: f64_at(40),
        dlng: f64_at(48),
    };
    let cells = (h.width as usize) * (h.height as usize);
    if cells == 0 {
        return Err(GridError::NotAGrid("zero-sized grid"));
    }
    let expected = GRID_HEADER_LEN + 4 * cells;
    if bytes.len() != expected {
        return Err(GridError::NotAGrid("length disagrees with header dims"));
    }
    let values = bytes[GRID_HEADER_LEN..]
        .chunks_exact(4)
        .map(|c| f32::from_bits(u32::from_le_bytes(c.try_into().unwrap())))
        .collect();
    Ok((h, values))
}

/// The value at `(row, col)`, row 0 being the `lat0` row. `None` out of
/// bounds; the caller decides whether that is an error.
pub fn grid_value_at(h: &GridHeader, values: &[f32], row: u32, col: u32) -> Option<f32> {
    if row >= h.height || col >= h.width {
        return None;
    }
    values
        .get(row as usize * h.width as usize + col as usize)
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(w: u32, hgt: u32) -> GridHeader {
        GridHeader {
            width: w,
            height: hgt,
            epsg: 4326,
            nodata: f32::NAN,
            lat0: 12.97,
            lng0: 77.59,
            dlat: -0.0001,
            dlng: 0.0001,
        }
    }

    #[test]
    fn roundtrip_is_exact_and_deterministic() {
        let h = header(3, 2);
        let vals = [1.0f32, 2.5, -3.25, 0.0, 917.75, 42.0];
        let a = encode_grid(&h, &vals).unwrap();
        let b = encode_grid(&h, &vals).unwrap();
        assert_eq!(a, b, "same grid must yield identical bytes");
        let (h2, v2) = decode_grid(&a).unwrap();
        assert_eq!(h2.width, 3);
        assert_eq!(v2, vals);
        assert_eq!(a.len(), GRID_HEADER_LEN + 4 * 6);
    }

    #[test]
    fn nan_payloads_and_negative_zero_fold_to_one_digest() {
        let h = header(2, 1);
        let quiet = f32::NAN;
        let payload = f32::from_bits(0x7fc0_1234); // a NaN with a payload
        let a = encode_grid(&h, &[quiet, -0.0]).unwrap();
        let b = encode_grid(&h, &[payload, 0.0]).unwrap();
        assert_eq!(
            a, b,
            "NaN payloads and zero signs must not split the digest"
        );
        let (_, v) = decode_grid(&a).unwrap();
        assert!(v[0].is_nan());
        assert_eq!(v[0].to_bits(), CANONICAL_NAN_F32);
    }

    #[test]
    fn refusals_are_typed_never_panics() {
        let h = header(2, 2);
        assert_eq!(
            encode_grid(&h, &[1.0]),
            Err(GridError::DimensionMismatch {
                expected: 4,
                got: 1
            })
        );
        assert_eq!(encode_grid(&header(0, 5), &[]), Err(GridError::EmptyGrid));
        assert!(matches!(decode_grid(b"tiny"), Err(GridError::NotAGrid(_))));
        let mut good = encode_grid(&h, &[1.0, 2.0, 3.0, 4.0]).unwrap();
        good.truncate(good.len() - 1);
        assert!(matches!(decode_grid(&good), Err(GridError::NotAGrid(_))));
    }

    #[test]
    fn value_lookup_by_row_col() {
        let h = header(3, 2);
        let vals = [10.0f32, 11.0, 12.0, 20.0, 21.0, 22.0];
        assert_eq!(grid_value_at(&h, &vals, 0, 0), Some(10.0));
        assert_eq!(grid_value_at(&h, &vals, 1, 2), Some(22.0));
        assert_eq!(grid_value_at(&h, &vals, 2, 0), None);
    }
}
