//! emem-codec — agent-native string codecs.
//!
//! Implements the four token-economical codecs from spec §3:
//!
//! | Codec   | Purpose                                            | Token target |
//! |---------|----------------------------------------------------|--------------|
//! | cell64  | 64-bit cell → 4-bigram string, locality-preserving | ≤ 4 tokens   |
//! | tslot   | u64 time slot → base-32 short form                 | ≤ 2 tokens   |
//! | vec64   | 1792D fp16 vector → 8-byte blake3 prefix, base-32  | ≤ 3 tokens   |
//! | cid64   | 32-byte fact CID → 8-byte prefix, base-32          | ≤ 3 tokens   |
//!
//! The cell64 alphabet (65,536 BPE-friendly bigrams) is loaded from
//! `data/cell64-alphabet-v0.bin`. Generation lives in `tools/measure_alphabet.py`.

#![forbid(unsafe_code)]

pub mod cell64;
pub mod tslot_text;
pub mod vec64;
pub mod cid64;
pub mod alphabet;
pub mod hilbert;
pub mod geo;

pub use cell64::{to_cell64, from_cell64, is_cell64_shape};
pub use tslot_text::{to_tslot_text, from_tslot_text};
pub use vec64::{to_vec64, vec64_to_cid};
pub use cid64::{to_cid64, from_cid64};
pub use geo::{cell_from_latlng, cell64_from_latlng, latlng_from_cell64, LatLng, BboxDeg};
