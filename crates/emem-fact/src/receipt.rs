//! Receipt — proof of recall, with cost self-declaration. Spec §7.

use serde::{Deserialize, Serialize};

use emem_core::{AttesterKey, KeyEpoch, Signature};
use crate::cid::{FactCid, RegistryCid, SchemaCid};

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
