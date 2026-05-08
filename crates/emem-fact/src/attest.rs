//! Attestation envelope — spec §6.

use serde::{Deserialize, Serialize};

use crate::cid::{RegistryCid, SchemaCid};
use crate::fact::Fact;
use emem_core::{AttesterKey, KeyEpoch, Signature};

/// A signed batch of facts with a Merkle root over their CIDs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attestation {
    /// One or more facts.
    pub facts: Vec<Fact>,
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
