//! Attestation envelope — spec §6.

use serde::{Deserialize, Serialize};

use crate::cid::{RegistryCid, SchemaCid};
use crate::edge::EdgeFact;
use crate::fact::Fact;
use crate::scope::Scope;
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
    /// v0: ed25519(blake3(batch_root || registry_cid || schema_cid)).
    /// v1 (`preimage_version = 1`): ed25519 over
    /// [`emem_attest::attestation_preimage_v1`] — domain-separated,
    /// length-prefixed segments.
    pub signature: Signature,
    /// ISO 8601 wall clock at attestation submission.
    pub attested_at: String,
    /// Optional multi-tenant [`Scope`] under which these facts are
    /// written (v0.0.8). Additive + serde-default: a pre-v0.0.8
    /// attestation deserialises with `scope: None` and round-trips
    /// byte-identically. The scope is an *index hint* for the storage
    /// layer — it is deliberately NOT folded into `batch_root` or the
    /// signature preimage, so adding it never changes an attestation's
    /// merkle root or verification. When present and non-empty, the
    /// storage layer writes scope-index rows so a later scoped recall
    /// can range-scan only this tenant's facts; the receipt-binding of
    /// scope (the `scope_blake3` preimage segment) is handled separately
    /// on the READ side and is unaffected by this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<Scope>,
    /// Which signature-preimage and merkle rule this attestation was
    /// signed under. `0` (omitted from JSON/CBOR) = the legacy v0 rule:
    /// unseparated concatenation preimage + unprefixed merkle hashing.
    /// `1` = [`emem_attest::PREIMAGE_V1`]: tagged length-prefixed
    /// preimage segments, RFC 6962-style 0x00/0x01 leaf/node prefixes,
    /// and duplicate-leaf rejection at verification. Additive — every
    /// pre-v1 attestation deserialises to `0` and verifies under its
    /// original rule.
    #[serde(default, skip_serializing_if = "u8_is_zero")]
    pub preimage_version: u8,
}

fn u8_is_zero(v: &u8) -> bool {
    *v == 0
}

/// Error from [`Attestation::build_and_sign_v1`].
#[derive(Debug, thiserror::Error)]
pub enum AttestBuildError {
    #[error("canonical CBOR encoding failed: {0}")]
    Cbor(String),
    /// Two facts/edges produced identical canonical CBOR. v1 verifiers
    /// reject duplicate merkle leaves (root-equivocation guard), so the
    /// signer refuses to produce such a batch instead of silently
    /// deduplicating — the caller has a bug worth surfacing.
    #[error("duplicate fact/edge leaf in attestation batch")]
    DuplicateLeaf,
}

impl Attestation {
    /// Build, root, and sign an attestation under the v1 rules. The ONE
    /// canonical signing path — responder code and demo binaries must
    /// not hand-roll preimages.
    ///
    /// Leaves are `blake3(canonical_cbor(fact))` for every fact plus
    /// [`EdgeFact::blake3_digest`] for every edge, bytewise-sorted; the
    /// root is [`emem_attest::merkle_root_v1`]; the signed bytes are
    /// [`emem_attest::attestation_preimage_v1`].
    #[allow(clippy::too_many_arguments)]
    pub fn build_and_sign_v1(
        facts: Vec<Fact>,
        edges: Vec<EdgeFact>,
        registry_cid: RegistryCid,
        schema_cid: SchemaCid,
        signing: &ed25519_dalek::SigningKey,
        attester_key_epoch: KeyEpoch,
        attested_at: String,
        scope: Option<Scope>,
    ) -> Result<Attestation, AttestBuildError> {
        use ed25519_dalek::Signer;

        let mut leaves: Vec<[u8; 32]> = Vec::with_capacity(facts.len() + edges.len());
        for f in &facts {
            let buf = crate::cbor::to_canonical_cbor(f)
                .map_err(|e| AttestBuildError::Cbor(e.to_string()))?;
            leaves.push(crate::cbor::blake3_32(&buf));
        }
        for e in &edges {
            leaves.push(e.blake3_digest());
        }
        leaves.sort();
        if emem_attest::has_adjacent_duplicate(&leaves) {
            return Err(AttestBuildError::DuplicateLeaf);
        }
        let batch_root = emem_attest::merkle_root_v1(&leaves);
        let msg = emem_attest::attestation_preimage_v1(
            &batch_root,
            registry_cid.as_str(),
            schema_cid.as_str(),
        );
        let sig = signing.sign(&msg);
        let mut sig_bytes = [0u8; 64];
        sig_bytes.copy_from_slice(&sig.to_bytes());
        let mut pk = [0u8; 32];
        pk.copy_from_slice(signing.verifying_key().as_bytes());

        Ok(Attestation {
            facts,
            edges,
            batch_root,
            attester: AttesterKey(pk),
            attester_key_epoch,
            registry_cid,
            schema_cid,
            signature: Signature(sig_bytes),
            attested_at,
            scope,
            preimage_version: emem_attest::PREIMAGE_V1,
        })
    }
}
