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
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
