//! The custody record: the weakest honest claim a node can sign.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::{b32, NodeIdentity};

/// Schema identifier for a v1 custody record.
pub const CUSTODY_SCHEMA_V1: &str = "emem.custody.v1";

/// What a node signs to say it received some bytes.
///
/// Every field is bound into the signature by
/// [`emem_attest::custody_preimage_v1`], so none of them can be edited after
/// signing without the signature failing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Custody {
    /// MUST equal [`CUSTODY_SCHEMA_V1`].
    pub schema: String,
    /// The node making the claim.
    pub node: NodeIdentity,
    /// The name the payload arrived under.
    pub name: String,
    /// blake3 of the payload bytes, base32-nopad lowercase.
    pub payload_digest: String,
    /// Payload length in bytes.
    pub size_bytes: u64,
    /// Wall clock at observation, RFC 3339 UTC. Supplied by the caller, not
    /// read from the system clock inside this crate, so a run is reproducible
    /// and a machine with no reliable clock cannot silently invent one.
    pub observed_at: String,
    /// What this record establishes, written into the record rather than left
    /// to a reader's assumption. See [`ASSURANCE`].
    pub assurance: String,
    /// ed25519 signature by `node.node_key` over the custody preimage,
    /// base32-nopad lowercase.
    pub signature: String,
}

/// The sentence every custody record carries about itself.
///
/// It is in the signed body on purpose. A reader who has the record but not
/// this documentation still learns the limit, and an intermediary cannot strip
/// the caveat without breaking the signature.
pub const ASSURANCE: &str = "custody_only: the holder of this node key states these bytes arrived \
                             under this name at this time. Nothing here attests how the payload \
                             was produced, and this is NOT an emem.os_trace.v1 execution record.";

/// What can go wrong building or checking a custody record.
#[derive(Debug, thiserror::Error)]
pub enum CustodyError {
    /// The schema string is not the v1 identifier.
    #[error("wrong schema: expected {CUSTODY_SCHEMA_V1}, got {0}")]
    WrongSchema(String),
    /// A base32 field did not decode.
    #[error("{field} is not base32-nopad lowercase")]
    NotBase32 {
        /// Which field failed.
        field: &'static str,
    },
    /// The node key is not a valid ed25519 public key.
    #[error("node_key is not a valid ed25519 public key")]
    BadKey,
    /// The signature did not verify over the preimage.
    #[error("signature does not verify against node_key")]
    BadSignature,
    /// The assurance sentence was altered.
    #[error("assurance text does not match the one this version signs")]
    AssuranceAltered,
}

/// The outcome of checking a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustodyVerdict {
    /// Signature verifies and the record is internally consistent.
    Valid,
}

impl Custody {
    /// Sign a custody record for one payload.
    ///
    /// `observed_at` is a parameter rather than a clock read: this crate runs
    /// where the clock may be wrong and the run should be reproducible.
    pub fn sign(
        key: &SigningKey,
        node: NodeIdentity,
        name: &str,
        payload: &[u8],
        observed_at: &str,
    ) -> Self {
        let payload_digest = b32(blake3::hash(payload).as_bytes());
        let size_bytes = payload.len() as u64;
        let pre = emem_attest::custody_preimage_v1(
            CUSTODY_SCHEMA_V1,
            &payload_digest,
            &node.profile,
            &node.platform,
            observed_at,
            size_bytes,
            name,
        );
        let sig = key.sign(&pre);
        Self {
            schema: CUSTODY_SCHEMA_V1.to_string(),
            node,
            name: name.to_string(),
            payload_digest,
            size_bytes,
            observed_at: observed_at.to_string(),
            assurance: ASSURANCE.to_string(),
            signature: b32(&sig.to_bytes()),
        }
    }

    /// Recompute the 32 bytes this record's signature covers.
    pub fn preimage(&self) -> [u8; 32] {
        emem_attest::custody_preimage_v1(
            &self.schema,
            &self.payload_digest,
            &self.node.profile,
            &self.node.platform,
            &self.observed_at,
            self.size_bytes,
            &self.name,
        )
    }

    /// Check the record against itself, with nothing but the bytes.
    ///
    /// No network, no registry, no stored state: whoever holds the record can
    /// run this, which is the property that makes an air-gapped node's output
    /// worth anything once it reaches the ground.
    pub fn verify(&self) -> Result<CustodyVerdict, CustodyError> {
        if self.schema != CUSTODY_SCHEMA_V1 {
            return Err(CustodyError::WrongSchema(self.schema.clone()));
        }
        // The caveat is signed, so an altered one is a broken record rather
        // than a quiet downgrade of what the reader is told.
        if self.assurance != ASSURANCE {
            return Err(CustodyError::AssuranceAltered);
        }
        let key_bytes =
            decode32(&self.node.node_key).ok_or(CustodyError::NotBase32 { field: "node_key" })?;
        let vk = VerifyingKey::from_bytes(&key_bytes).map_err(|_| CustodyError::BadKey)?;
        let sig_bytes = data_encoding::BASE32_NOPAD
            .decode(self.signature.to_uppercase().as_bytes())
            .ok()
            .and_then(|v| <[u8; 64]>::try_from(v.as_slice()).ok())
            .ok_or(CustodyError::NotBase32 { field: "signature" })?;
        // verify_strict, not verify. The permissive check accepts small-order
        // and non-canonical public keys, which lets one signature validate
        // under more than one key. For a record whose whole purpose is to say
        // WHICH node held some bytes, that ambiguity is the bug. The same fix
        // was already made elsewhere in this codebase; it belongs here too.
        vk.verify_strict(&self.preimage(), &Signature::from_bytes(&sig_bytes))
            .map_err(|_| CustodyError::BadSignature)?;
        Ok(CustodyVerdict::Valid)
    }

    /// Whether these bytes are the ones this record covers.
    ///
    /// Separate from [`Self::verify`] because they answer different questions:
    /// verify asks whether the record is genuine, this asks whether the file
    /// in front of you is the one it is about. A reader that has only the
    /// record can do the first; only a reader holding the payload can do both.
    pub fn covers(&self, payload: &[u8]) -> bool {
        b32(blake3::hash(payload).as_bytes()) == self.payload_digest
    }
}

fn decode32(s: &str) -> Option<[u8; 32]> {
    data_encoding::BASE32_NOPAD
        .decode(s.to_uppercase().as_bytes())
        .ok()
        .and_then(|v| <[u8; 32]>::try_from(v.as_slice()).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(key: &SigningKey) -> NodeIdentity {
        NodeIdentity {
            node_key: b32(key.verifying_key().as_bytes()),
            profile: "orbital.satellite.v1".into(),
            platform: "nvidia.jetson-orin".into(),
        }
    }

    fn key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    #[test]
    fn a_signed_record_verifies_with_nothing_but_itself() {
        let k = key();
        let c = Custody::sign(
            &k,
            node(&k),
            "frame_001.tif",
            b"pixels",
            "2026-08-20T09:00:00Z",
        );
        assert_eq!(c.verify().unwrap(), CustodyVerdict::Valid);
        assert!(c.covers(b"pixels"));
        assert!(!c.covers(b"different pixels"));
    }

    /// Every signed field must be load-bearing. If one can be edited without
    /// breaking the signature, it is decoration and a reader should not trust
    /// it.
    #[test]
    fn every_bound_field_breaks_the_signature_when_edited() {
        let k = key();
        let base = Custody::sign(&k, node(&k), "a.tif", b"bytes", "2026-08-20T09:00:00Z");

        let mut t = base.clone();
        t.name = "b.tif".into();
        assert!(t.verify().is_err(), "name must be bound");

        let mut t = base.clone();
        t.payload_digest = b32(blake3::hash(b"other").as_bytes());
        assert!(t.verify().is_err(), "payload digest must be bound");

        let mut t = base.clone();
        t.size_bytes += 1;
        assert!(t.verify().is_err(), "size must be bound");

        let mut t = base.clone();
        t.observed_at = "2027-01-01T00:00:00Z".into();
        assert!(t.verify().is_err(), "observed_at must be bound");

        let mut t = base.clone();
        t.node.profile = "robot.fleet.v1".into();
        assert!(t.verify().is_err(), "profile must be bound");

        let mut t = base.clone();
        t.node.platform = "tpm2.host".into();
        assert!(t.verify().is_err(), "platform must be bound");
    }

    /// The caveat is signed, so it cannot be quietly removed by anyone
    /// forwarding the record.
    #[test]
    fn the_assurance_sentence_cannot_be_stripped() {
        let k = key();
        let mut c = Custody::sign(&k, node(&k), "a.tif", b"b", "2026-08-20T09:00:00Z");
        c.assurance = "attested execution, verified".into();
        assert!(matches!(c.verify(), Err(CustodyError::AssuranceAltered)));
    }

    /// A different key's signature must not pass.
    #[test]
    fn another_key_cannot_speak_for_this_node() {
        let k = key();
        let other = SigningKey::from_bytes(&[9u8; 32]);
        let mut c = Custody::sign(&k, node(&k), "a.tif", b"b", "2026-08-20T09:00:00Z");
        c.node.node_key = b32(other.verifying_key().as_bytes());
        assert!(matches!(c.verify(), Err(CustodyError::BadSignature)));
    }

    /// Signing is deterministic: the same inputs give byte-identical records,
    /// which is what lets a run be reproduced and compared.
    #[test]
    fn signing_is_deterministic() {
        let k = key();
        let a = Custody::sign(&k, node(&k), "a.tif", b"b", "2026-08-20T09:00:00Z");
        let b = Custody::sign(&k, node(&k), "a.tif", b"b", "2026-08-20T09:00:00Z");
        assert_eq!(a.signature, b.signature);
        assert_eq!(a.payload_digest, b.payload_digest);
    }

    /// The custody preimage must not collide with the OS-trace one. If it did,
    /// a custody signature could be replayed as execution evidence, which is
    /// the whole thing this separation exists to prevent.
    #[test]
    fn custody_and_os_trace_preimages_are_different_domains() {
        let c = emem_attest::custody_preimage_v1(CUSTODY_SCHEMA_V1, "d", "p", "pl", "t", 1, "n");
        let o = emem_attest::os_trace_preimage_v1(
            CUSTODY_SCHEMA_V1,
            &[0u8; 32],
            "p",
            0,
            1,
            &[0u8; 32],
            std::iter::empty::<&str>(),
            None,
        );
        assert_ne!(c, o);
    }
}
