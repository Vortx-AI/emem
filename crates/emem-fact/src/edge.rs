//! Temporal knowledge-graph edges — a signed, content-addressed relation
//! between two facts, valid over a half-open `[valid_from, valid_to)`
//! interval (Zep / Graphiti edge model, arXiv 2501.13956).
//!
//! An `EdgeFact` is a *sibling* of the frozen [`crate::fact::Fact`] enum,
//! NOT a fourth variant. Keeping it separate means the v0.0.7 / v0.0.8
//! `Fact` CBOR layout is byte-untouched and every legacy fact / receipt /
//! attestation still verifies bit-for-bit. Edges ride additively inside
//! the attestation envelope and the receipt preimage.
//!
//! ## Bi-temporal semantics
//!
//! - `valid_from` — the planet-side moment the relation became true
//!   (Unix-epoch tslot scale).
//! - `valid_to`   — the moment it stopped being true. `None` means "still
//!   valid". A later edge with a larger `valid_from` for the same
//!   `(subj, pred, obj)` *supersedes* the earlier one; the storage layer
//!   keeps both rows (non-destructive) and an `as_of` read picks the
//!   newest edge whose `valid_from <= as_of` and whose `valid_to` (if
//!   any) is `> as_of`.
//!
//! ## Canonical CBOR + CID
//!
//! Fields serialise in declaration order via `ciborium` with
//! `skip_serializing_if = "Option::is_none"` on every optional so absent
//! fields contribute zero bytes — exactly the discipline
//! [`crate::scope::Scope`] uses. The CID is
//! `base32_nopad_lc(blake3(canonical_cbor))`, the same recipe the rest of
//! the protocol uses for content addresses.

use serde::{Deserialize, Serialize};

use crate::cid::{EdgeCid, FactCid, SchemaCid};
use emem_core::AttesterKey;

/// A signed temporal relation `subj --pred--> obj` valid over
/// `[valid_from, valid_to)`. Content-addressed under canonical CBOR.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EdgeFact {
    /// CID of the subject fact.
    pub subj: FactCid,
    /// Predicate label (free-form, e.g. `"replaced_by"`, `"co_located_with"`).
    pub pred: String,
    /// CID of the object fact.
    pub obj: FactCid,
    /// Valid-time start (Unix-epoch tslot scale). Inclusive.
    pub valid_from: u64,
    /// Valid-time end. `None` means "still valid". Exclusive when set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_to: Option<u64>,
    /// Attester-declared confidence in `[0, 1]`.
    pub confidence: f32,
    /// ed25519 pubkey of the signer.
    pub signer: AttesterKey,
    /// ISO 8601 wall-clock at signing time.
    pub signed_at: String,
    /// Optional CDDL schema fragment CID describing the predicate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_cid: Option<SchemaCid>,
    /// Optional free-form provenance note.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl EdgeFact {
    /// Canonical CBOR encoding. Deterministic for a given `EdgeFact`
    /// regardless of platform — `ciborium` emits map keys in declaration
    /// order and `skip_serializing_if` keeps absent optionals out of the
    /// byte stream.
    pub fn to_canonical_cbor(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(256);
        // ciborium::into_writer is infallible for EdgeFact (no map keys
        // that can fail to encode); ignore the result to keep the helper
        // total — matches Scope::to_canonical_cbor.
        let _ = ciborium::into_writer(self, &mut buf);
        buf
    }

    /// 32-byte blake3 of the canonical CBOR.
    pub fn blake3_digest(&self) -> [u8; 32] {
        *blake3::hash(&self.to_canonical_cbor()).as_bytes()
    }

    /// Content address: `base32_nopad_lc(blake3(canonical_cbor))`.
    pub fn cid(&self) -> EdgeCid {
        EdgeCid::new(
            data_encoding::BASE32_NOPAD
                .encode(&self.blake3_digest())
                .to_lowercase(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(valid_from: u64, valid_to: Option<u64>) -> EdgeFact {
        EdgeFact {
            subj: FactCid::new("subj-cid"),
            pred: "replaced_by".into(),
            obj: FactCid::new("obj-cid"),
            valid_from,
            valid_to,
            confidence: 1.0,
            signer: AttesterKey([7u8; 32]),
            signed_at: "2026-05-29T00:00:00Z".into(),
            schema_cid: None,
            note: None,
        }
    }

    #[test]
    fn cid_is_stable_across_two_encodings() {
        let e = mk(10, None);
        assert_eq!(e.cid(), e.cid());
        assert_eq!(e.to_canonical_cbor(), e.to_canonical_cbor());
    }

    #[test]
    fn cid_changes_when_valid_from_changes() {
        assert_ne!(mk(10, None).cid(), mk(20, None).cid());
    }

    #[test]
    fn absent_optionals_drop_from_json() {
        let e = mk(10, None);
        let j = serde_json::to_string(&e).unwrap();
        assert!(!j.contains("valid_to"));
        assert!(!j.contains("schema_cid"));
        assert!(!j.contains("note"));
    }

    #[test]
    fn round_trips_via_cbor() {
        let e = mk(10, Some(20));
        let bytes = e.to_canonical_cbor();
        let back: EdgeFact = ciborium::de::from_reader(bytes.as_slice()).unwrap();
        assert_eq!(e, back);
    }
}
