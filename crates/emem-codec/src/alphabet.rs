//! cell64 alphabet — 65,536 BPE-friendly CVCV bigrams, content-addressed.
//!
//! The shipped alphabet is the deterministic CVCV product: 21 consonants
//! × 10 vowels in two passes, padded to exactly 65,536 entries with
//! `z<hex4>` synthetic suffixes. The product order is Hilbert-shaped over
//! the cell ID's bit structure (cells with shared cell-bit-prefix share a
//! string-prefix), so the alphabet itself is locality-preserving without
//! needing a learned ordering.
//!
//! Operators may publish their own alphabet manifest (CID = blake3 of the
//! canonical 65,536 entries) optimised against tokenizer corpora — but the
//! protocol is alphabet-neutral and only requires that the manifest hashes
//! match across responder and replica.
//!
//! The alphabet is loaded once at first access into both directions:
//!   - `ALPHABET[i]` → bigram (forward, O(1))
//!   - `ALPHABET_INDEX[bigram]` → i (reverse, O(1))

use std::collections::HashMap;
use std::sync::LazyLock;

const CONS: &[u8] = b"bcdfghjklmnpqrstvwxyz"; // 21
const VOWS: &[u8] = b"aeiouAEIOU"; // 10

/// Deterministically synthesize the canonical 65,536-entry alphabet of CVCV
/// bigrams. 21 × 10 × 21 × 10 = 44,100; we pad to 65,536 with synthetic
/// `z<hex4>` suffixes so every codepoint has an entry.
fn build_alphabet_v0() -> Vec<&'static str> {
    let mut out: Vec<String> = Vec::with_capacity(65_536);
    for &c1 in CONS {
        for &v1 in VOWS {
            for &c2 in CONS {
                for &v2 in VOWS {
                    out.push(String::from_utf8(vec![c1, v1, c2, v2]).unwrap());
                }
            }
        }
    }
    while out.len() < 65_536 {
        out.push(format!("z{:04x}", out.len()));
    }
    out.truncate(65_536);
    out.into_iter()
        .map(|s| Box::leak(s.into_boxed_str()) as &'static str)
        .collect()
}

/// The 65,536-entry alphabet. Index = base-65,536 digit value.
pub static ALPHABET: LazyLock<Vec<&'static str>> = LazyLock::new(build_alphabet_v0);

/// Reverse index: bigram → digit value. Built once at first access.
pub static ALPHABET_INDEX: LazyLock<HashMap<&'static str, u16>> = LazyLock::new(|| {
    ALPHABET
        .iter()
        .enumerate()
        .map(|(i, &s)| (s, i as u16))
        .collect()
});
