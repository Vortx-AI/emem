//! Attestation envelope — spec §6.

use serde::{Deserialize, Serialize};

use crate::cid::{RegistryCid, SchemaCid};
use crate::edge::EdgeFact;
use crate::fact::Fact;
use emem_core::{AttesterKey, KeyEpoch, Signature};

/// A signed batch of facts with a Merkle root over their CIDs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attestation {
    /// One or more facts.
    pub facts: Vec<Fact>,
    /// Temporal knowledge-graph edges carried alongside the facts.
    /// Additive (v0.0.9): absent / empty on every pre-edge attestation,
    /// so the JSON round-trips byte-identically and the merkle leaf set
    /// is unchanged when this is empty. When non-empty, each edge's
    /// `blake3(canonical_cbor(edge))` is folded into the leaf set BEFORE
    /// sorting so the signature commits to the edges too.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<EdgeFact>,
    /// blake3 Merkle root over fact_cids in canonical sort order.
    pub batch_root: [u8; 32],
    /// ed25519 attester pubkey.
    pub attester: AttesterKey,
    /// Key rotation epoch.
    pub attester_key_epoch: KeyEpoch,
    /// CID of function registry version in force at attestation time.
    pub registry_cid: RegistryCid,
    /// CID of CDDL profile in force.
    pub schema_cid: SchemaCid,
    /// ed25519(blake3(batch_root || registry_cid || schema_cid)).
    pub signature: Signature,
    /// ISO 8601 wall clock at attestation submission.
    pub attested_at: String,
}
