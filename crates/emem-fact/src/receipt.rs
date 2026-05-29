//! Receipt — proof of recall, with cost self-declaration. Spec §7.

use serde::{Deserialize, Serialize};

use crate::cid::{FactCid, RegistryCid, SchemaCid};
use crate::scope::Scope;
use emem_core::{AttesterKey, KeyEpoch, Signature};

/// Returned with every read response. Cryptographically rebindable evidence
/// that a particular set of facts was served.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    /// ULID.
    pub request_id: String,
    /// ISO 8601 serve time.
    pub served_at: String,
    /// "recall" | "verify" | "find_similar" | ...
    pub primitive: String,
    /// If served via emem.intent, the intent type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
    /// cell64 references in the response.
    pub cells: Vec<String>,
    /// Fact CIDs cited.
    pub fact_cids: Vec<FactCid>,
    /// CID of the response schema.
    pub schema_cid: SchemaCid,
    /// Inclusion proof to the current attestation root, if applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merkle_proof: Option<MerkleProof>,
    /// Responder pubkey.
    pub responder: AttesterKey,
    /// Responder key rotation epoch.
    pub responder_key_epoch: KeyEpoch,
    /// ed25519 signature.
    pub signature: Signature,
    /// Per-source version pins (e.g. {"geotessera.v1": "2024"}).
    pub source_versions: std::collections::BTreeMap<String, String>,
    /// CID of registry used to serve.
    pub registry_cid: RegistryCid,
    /// Cost / latency / freshness self-declaration.
    pub cost: Cost,
    /// Bi-temporal filter that produced this response, when the caller
    /// pinned either valid-time (`as_of_tslot`) or transaction-time
    /// (`as_of_signed_at`). Recorded in the receipt body so a verifier
    /// can replay the same query later by reissuing it with the same
    /// bound. Omitted from JSON when the caller did not constrain either
    /// axis — back-compat for pre-bi-temporal receipts that never
    /// carried this field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_of: Option<AsOfReceipt>,
    /// Multi-tenant scope (`{user_id, agent_id, run_id, org_id}`) the
    /// responder honoured when serving this response. Recorded so an
    /// offline verifier reconstructs the same `blake3(canonical_cbor(
    /// Scope))` segment used in the signature preimage. Omitted from
    /// JSON when the caller supplied no scope — back-compat for
    /// pre-v0.0.8 receipts that never carried this field; their
    /// preimage rule did not include a scope segment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<Scope>,
}

/// Replay-able bi-temporal filter recorded in a [`Receipt`] when the
/// caller pinned valid-time or transaction-time. The verifier reconstructs
/// the query by reissuing the same read with these two fields populated.
/// At least one of the two is `Some` when this struct is present in a
/// receipt; an "unbounded" caller skips emitting the struct entirely so
/// existing offline verifiers continue to round-trip pre-bi-temporal
/// receipts byte-for-byte.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct AsOfReceipt {
    /// Valid-time bound — only facts with `tslot <= valid_time` were
    /// considered. `None` means valid-time was unconstrained.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_time: Option<u64>,
    /// Transaction-time bound — only facts with `signed_at <=
    /// transaction_time` were considered. RFC 3339 string, identical to
    /// the request's `as_of_signed_at`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_time: Option<String>,
}

impl AsOfReceipt {
    /// `true` when both bounds are `None`. Such an `AsOfReceipt` has no
    /// information to bind, and the receipt-signing path drops the
    /// `as_of` segment from the preimage — keeps every pre-v0.0.8
    /// receipt verifiable byte-for-byte. Callers should pass `None`
    /// rather than constructing one of these, but it's safe if they do.
    pub fn is_unbounded(&self) -> bool {
        self.valid_time.is_none() && self.transaction_time.is_none()
    }

    /// Canonical CBOR encoding. Deterministic for a given `AsOfReceipt`
    /// regardless of platform — `ciborium` encodes maps in declaration
    /// order via serde's field-visit sequence, and
    /// `skip_serializing_if = "Option::is_none"` keeps absent fields
    /// out of the byte stream. The byte stream is what the signer
    /// hashes into the preimage and what an offline verifier rebuilds.
    pub fn to_canonical_cbor(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(32);
        // ciborium::into_writer is infallible for AsOfReceipt
        // (Option<u64> + Option<String> fields only); ignore the result
        // to keep the helper total.
        let _ = ciborium::into_writer(self, &mut buf);
        buf
    }

    /// 32-byte blake3 of [`AsOfReceipt::to_canonical_cbor`]. The
    /// preimage segment a responder hashes into the signed bytes when
    /// the receipt carries a non-unbounded `as_of`.
    pub fn blake3_digest(&self) -> [u8; 32] {
        let bytes = self.to_canonical_cbor();
        *blake3::hash(&bytes).as_bytes()
    }

    /// Lowercase hex of [`AsOfReceipt::blake3_digest`]. The wire form
    /// that gets concatenated into the preimage byte stream between
    /// `[scope_blake3_hex|]` (if any) and `primitive`.
    pub fn blake3_hex(&self) -> String {
        data_encoding::HEXLOWER.encode(&self.blake3_digest())
    }
}

/// Empirical cost+latency+freshness self-declared in every receipt.
/// See spec §20.4.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cost {
    /// Protocol credits charged for this call.
    pub credits: u64,
    /// Observed p50 latency for this primitive class, ms.
    pub latency_p50_ms: u32,
    /// Observed p99 latency, ms.
    pub latency_p99_ms: u32,
    /// Age of the stalest source in the response, seconds.
    pub source_freshness_s: u32,
    /// Whether the response was served from cache.
    pub was_cached: bool,
}

/// Merkle inclusion proof for a fact within an attestation batch root.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MerkleProof {
    /// Leaf index in the canonical-sorted batch.
    pub leaf_index: u32,
    /// Sibling hashes from leaf to root.
    pub path: Vec<[u8; 32]>,
    /// The expected batch root.
    pub root: [u8; 32],
}

#[cfg(test)]
mod as_of_receipt_tests {
    use super::*;

    #[test]
    fn unbounded_default_is_unbounded() {
        assert!(AsOfReceipt::default().is_unbounded());
    }

    #[test]
    fn any_bound_makes_it_bound() {
        let v = AsOfReceipt {
            valid_time: Some(1_700_000_000),
            transaction_time: None,
        };
        assert!(!v.is_unbounded());
        let t = AsOfReceipt {
            valid_time: None,
            transaction_time: Some("2026-05-29T00:00:00Z".into()),
        };
        assert!(!t.is_unbounded());
    }

    #[test]
    fn canonical_cbor_is_deterministic() {
        let a = AsOfReceipt {
            valid_time: Some(1_700_000_000),
            transaction_time: Some("2026-05-29T00:00:00Z".into()),
        };
        assert_eq!(a.to_canonical_cbor(), a.to_canonical_cbor());
    }

    #[test]
    fn digest_changes_when_valid_time_flips_one_bit() {
        let a = AsOfReceipt {
            valid_time: Some(1_700_000_000),
            transaction_time: None,
        };
        let b = AsOfReceipt {
            valid_time: Some(1_700_000_001),
            transaction_time: None,
        };
        assert_ne!(a.blake3_digest(), b.blake3_digest());
    }

    #[test]
    fn blake3_hex_is_64_lowercase_chars() {
        let a = AsOfReceipt {
            valid_time: Some(42),
            transaction_time: None,
        };
        let h = a.blake3_hex();
        assert_eq!(h.len(), 64);
        assert!(h
            .chars()
            .all(|c| c.is_ascii_hexdigit() && (c.is_ascii_digit() || c.is_ascii_lowercase())));
    }

    #[test]
    fn transaction_time_only_differs_from_valid_time_only() {
        let v = AsOfReceipt {
            valid_time: Some(1_700_000_000),
            transaction_time: None,
        };
        let t = AsOfReceipt {
            valid_time: None,
            transaction_time: Some("2026-05-29T00:00:00Z".into()),
        };
        assert_ne!(v.blake3_digest(), t.blake3_digest());
    }
}
