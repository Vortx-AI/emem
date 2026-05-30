//! cell64 — token-economical, locality-preserving cell encoding.
//!
//! A 64-bit cell ID encodes as 4 base-65,536 digits (4 bigrams = ~17 chars,
//! ≤ 4 tokens under cl100k/o200k). The alphabet is constructed to be
//! Hilbert-ordered so that adjacent codepoints map to adjacent cells —
//! spatially proximate cells share string prefixes through the cell ID's
//! own bit structure (mode|res|base|path), not through alphabet ordering.

use crate::alphabet::{ALPHABET, ALPHABET_INDEX};
use emem_core::Cell;

/// Encode a 64-bit cell ID as a `cell64` string. Output is exactly 4 bigrams
/// joined by `.`, e.g. `"ento.bria.calo.tris"`.
pub fn to_cell64(cell: Cell) -> String {
    let raw = cell.0;
    let d0 = ((raw >> 48) & 0xFFFF) as usize;
    let d1 = ((raw >> 32) & 0xFFFF) as usize;
    let d2 = ((raw >> 16) & 0xFFFF) as usize;
    let d3 = (raw & 0xFFFF) as usize;
    format!(
        "{}.{}.{}.{}",
        ALPHABET[d0], ALPHABET[d1], ALPHABET[d2], ALPHABET[d3]
    )
}

/// Cheap shape-only check: is this string syntactically a cell64?
/// Four dot-separated bigrams, each present in the alphabet. Useful as
/// a router predicate to tell apart "Mount Everest" (a place name)
/// from "damO.zb000.wapu.yAxe" (already a cell64) without hitting the
/// geocoder.
pub fn is_cell64_shape(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 4 {
        return false;
    }
    parts.iter().all(|sym| ALPHABET_INDEX.contains_key(*sym))
}

/// Looser shape detector: does this string *look like* it was meant to be a
/// cell64 even if it doesn't decode? A cell64 is exactly 4 dot-separated
/// tokens, each a short (≤5 char) run of ASCII alphanumerics with no spaces.
/// This is the predicate the API uses to tell "typo'd cell ID" apart from a
/// place name: a malformed cell ID (e.g. one bigram outside the alphabet)
/// must surface as a typed `invalid_cell64` error rather than be fed to the
/// fuzzy geocoder, where it would dress junk up as a confident place.
///
/// Returns true for things like `"ento.bria.calo.tXYZ"` (4 short alnum
/// tokens, dotted) but false for `"Mount Everest"`, `"not-a-cell"`,
/// `"São Paulo, Brazil"`, or any string with spaces / commas / hyphens.
pub fn looks_like_cell64(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 4 {
        return false;
    }
    parts.iter().all(|p| {
        !p.is_empty()
            && p.len() <= 5
            && p.bytes().all(|b| b.is_ascii_alphanumeric())
    })
}

/// Decode a `cell64` string back to a 64-bit cell ID. O(1) per bigram via
/// the precomputed reverse index.
pub fn from_cell64(s: &str) -> Result<Cell, CodecError> {
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 4 {
        return Err(CodecError::WrongLength(parts.len()));
    }
    let mut raw: u64 = 0;
    for (i, sym) in parts.iter().enumerate() {
        let idx = *ALPHABET_INDEX
            .get(sym)
            .ok_or_else(|| CodecError::UnknownSymbol((*sym).to_string()))?;
        raw |= (idx as u64) << (48 - i * 16);
    }
    Ok(Cell(raw))
}

/// Codec errors.
#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    /// cell64 must be exactly 4 dot-separated bigrams.
    #[error("cell64 must have 4 bigrams, got {0}")]
    WrongLength(usize),

    /// A bigram was not in the alphabet.
    #[error("unknown cell64 symbol: {0}")]
    UnknownSymbol(String),

    /// A cell64 was decoded successfully, but its `mode|res|base` prefix
    /// is not the geo-aperture variant; cannot reverse-map to lat/lng.
    #[error("cell64 is not a geo-aperture cell (raw=0x{0:016x})")]
    NotGeoCell(u64),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        for raw in [0u64, 1, 0x1234_5678_9abc_def0, u64::MAX] {
            let c = Cell::from_raw(raw);
            let s = to_cell64(c);
            let c2 = from_cell64(&s).expect("decode");
            assert_eq!(c, c2, "raw={raw:x} via {s}");
        }
    }

    #[test]
    fn looks_like_cell64_discriminates_typos_from_place_names() {
        // A real cell64 is obviously cell64-shaped.
        let real = to_cell64(Cell::from_raw(0x1234_5678_9abc_def0));
        assert!(looks_like_cell64(&real));
        assert!(is_cell64_shape(&real));

        // 4 short alnum dotted tokens that don't decode: a typo'd cell id.
        // Shape matches, decode fails — this is the case the API rejects.
        assert!(looks_like_cell64("ZZ99.ab12.cd34.ef56"));
        assert!(!is_cell64_shape("ZZ99.ab12.cd34.ef56"));

        // Place names and junk are NOT cell64-shaped.
        for not_cell in [
            "Mount Everest",
            "not-a-cell",
            "São Paulo, Brazil",
            "a.b.c",          // only 3 parts
            "aa.bb.cc.ddeeff", // last token too long (>5 chars)
            "",
            "ento.bria.calo.tris.extra", // 5 parts
        ] {
            assert!(
                !looks_like_cell64(not_cell),
                "{not_cell:?} must not look like a cell64"
            );
        }
    }
}
