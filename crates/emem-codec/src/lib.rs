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

pub mod alphabet;
pub mod cell64;
pub mod cid64;
pub mod geo;
pub mod grid;
pub mod hilbert;
pub mod tslot_text;
pub mod vec64;

pub use cell64::{from_cell64, is_cell64_shape, looks_like_cell64, to_cell64};
pub use cid64::{from_cid64, to_cid64};
pub use geo::{
    cell64_from_latlng, cell_from_latlng, cells_in_bbox, equal_area_weight_for, latlng_from_cell64,
    BboxDeg, LatLng, CELL_PITCH_M_EQUATOR,
};
pub use tslot_text::{from_tslot_text, to_tslot_text};

use emem_core::substrates::AddressSpace;

/// Which address space a subject string belongs to, or `None` when it is
/// neither and must be refused.
///
/// This lives here rather than beside [`AddressSpace`] in `emem-core` for one
/// mechanical reason: deciding the answer needs the cell64 alphabet, and
/// `emem-codec` already depends on `emem-core`, so the reverse edge would be a
/// cycle. The type is declared where the registry reads it; the predicate is
/// implemented where the alphabet lives.
///
/// Nothing decided this before. `CanonicalKey.cell` is a `String` and the
/// storage key is its bytes, so the record layer will happily key a fact by
/// `"hello"`. The read path guards a NEARBY question, whether a string that
/// failed to parse should reach the fuzzy geocoder, which is about not
/// dressing junk up as a confident place. It says nothing about what a write
/// may key, and the two must not be confused: one protects an answer, this
/// protects the index.
///
/// The two spaces are distinguishable without ambiguity, which is what makes
/// carrying both in one column safe. A cell64 is exactly four dot-separated
/// alphabet symbols and can never contain a colon; an `emem:entity:` token
/// always leads with a scheme containing two. A string matching neither
/// belongs to no address space and is refused rather than guessed at, because
/// a subject nobody can resolve is a fact nobody can ever cite.
pub fn address_space_of_subject(subject: &str) -> Option<AddressSpace> {
    if subject.starts_with("emem:entity:") || subject.starts_with("meme:") {
        // The legacy `meme:` spelling still resolves, so it still addresses;
        // calling it unaddressable here would strand records the resolver can
        // still read.
        let body = subject
            .strip_prefix("emem:entity:")
            .or_else(|| subject.strip_prefix("meme:"))
            .unwrap_or("");
        return (!body.is_empty() && body.chars().all(|c| c.is_ascii_alphanumeric()))
            .then(|| AddressSpace::new(AddressSpace::ENTITY_CID));
    }
    // Deliberately the LOOSE shape, not a successful decode. A subject shaped
    // like a cell64 that does not decode is a malformed PLACE, which the geo
    // layer already reports as a typed `invalid_cell64`; calling it
    // unaddressable would replace a precise error with a vague one.
    looks_like_cell64(subject).then(|| AddressSpace::new(AddressSpace::GEO_CELL64))
}

/// Whether a profile declaring `space` may key a fact by `subject`.
///
/// Both halves must hold: the subject has to belong to an address space, and
/// it has to be THE space the profile declares. A codebase profile writing at
/// a cell64 is as wrong as a satellite profile writing at an entity id, and
/// neither is caught by looking at the string alone.
pub fn subject_admitted_by(space: &AddressSpace, subject: &str) -> bool {
    address_space_of_subject(subject).is_some_and(|s| &s == space)
}

pub use vec64::{to_vec64, vec64_to_cid};

#[cfg(test)]
mod address_space_tests {
    use super::*;

    /// Every subject the record layer could be handed lands in exactly one
    /// address space, or in none and is refused.
    ///
    /// The junk cases are the point. `CanonicalKey.cell` is a `String` and the
    /// storage key is its raw bytes, so absent this predicate a fact can be
    /// keyed by anything at all, including the empty string. A fact at a
    /// subject nobody can resolve is worse than a rejected write: it is a
    /// permanent row in an append-only log that no citation can ever reach.
    #[test]
    fn a_subject_belongs_to_one_address_space_or_none() {
        let geo = AddressSpace::new(AddressSpace::GEO_CELL64);
        let ent = AddressSpace::new(AddressSpace::ENTITY_CID);

        for s in ["defi.zb294.qokO.xAxe", "ento.bria.calo.tXYZ"] {
            assert_eq!(address_space_of_subject(s).as_ref(), Some(&geo), "{s}");
            assert!(subject_admitted_by(&geo, s), "{s}");
            assert!(
                !subject_admitted_by(&ent, s),
                "{s} must not pass as an identity"
            );
        }
        for s in [
            "emem:entity:zzzzn5rk3wubxsbnrpxevbtqha",
            "meme:zzzzn5rk3wubxsbnrpxevbtqha", // legacy spelling still resolves
        ] {
            assert_eq!(address_space_of_subject(s).as_ref(), Some(&ent), "{s}");
            assert!(subject_admitted_by(&ent, s), "{s}");
            assert!(
                !subject_admitted_by(&geo, s),
                "{s} must not pass as a place"
            );
        }
        for s in [
            "",                                   // the empty subject
            "hello",                              // a word
            "not.a.cell",                         // dotted junk, too few tokens
            "defi.zb294.qokO",                    // a truncated cell64
            "emem:entity:",                       // a scheme with no body
            "emem:fact:defi.zb294.qokO.xAxe:abc", // a FACT token, not a subject
            "emem:bundle:abc",                    // a bundle handle, not a subject
        ] {
            assert_eq!(
                address_space_of_subject(s),
                None,
                "{s:?} must belong to no address space and be refused"
            );
            assert!(
                !subject_admitted_by(&geo, s) && !subject_admitted_by(&ent, s),
                "{s:?}"
            );
        }
    }
}
