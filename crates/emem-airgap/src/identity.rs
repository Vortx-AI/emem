//! The node's own key: generated once, kept, and never regenerated.

use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};

use crate::b32;

/// Schema identifier for a v1 join request.
pub const JOIN_REQUEST_SCHEMA_V1: &str = "emem.join_request.v1";

/// A node's persisted identity.
///
/// Deliberately the same shape agents already use in
/// `~/.config/emem/agent_identity.json`: `alg`, `seed_hex`, `pubkey_b32`,
/// `pubkey8`, `role`, `created`, `note`. A device and an agent are both
/// parties that hold a key and answer for what they sign, so there is no
/// reason for two formats, and one format means the same tooling reads both.
///
/// The seed is written once. Regenerating it would give the node a new
/// identity, orphaning every custody record it had already signed and any
/// endorsement an operator had already issued for the old key, so the loader
/// never overwrites an existing file.
#[derive(Clone, Serialize, Deserialize)]
pub struct NodeKeyFile {
    /// Always `"ed25519"`.
    pub alg: String,
    /// The 32-byte seed, hex. The private half; the file is written 0600.
    pub seed_hex: String,
    /// Public key, base32-nopad lowercase.
    pub pubkey_b32: String,
    /// First eight characters of `pubkey_b32`, the short form used in prose.
    pub pubkey8: String,
    /// What this key is for, in a sentence.
    pub role: String,
    /// Date the key was created, YYYY-MM-DD.
    pub created: String,
    /// Why the file exists, for whoever finds it later.
    pub note: String,
}

/// Hand-written so the private seed can never reach a log.
///
/// The derived Debug printed `seed_hex` in full. Nothing in this crate logs
/// the struct today, but a derived impl is a loaded gun: the next person to
/// add a `dbg!` or an error context while debugging an enrolment would have
/// written the node's private key into a file that leaves the machine.
impl std::fmt::Debug for NodeKeyFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeKeyFile")
            .field("alg", &self.alg)
            .field("seed_hex", &"<redacted>")
            .field("pubkey_b32", &self.pubkey_b32)
            .field("pubkey8", &self.pubkey8)
            .field("role", &self.role)
            .field("created", &self.created)
            .finish_non_exhaustive()
    }
}

impl NodeKeyFile {
    /// Build the file contents for a fresh seed.
    pub fn new(seed: [u8; 32], created: &str, role: &str) -> Self {
        let key = SigningKey::from_bytes(&seed);
        let pubkey_b32 = b32(key.verifying_key().as_bytes());
        Self {
            alg: "ed25519".into(),
            seed_hex: hex(&seed),
            pubkey_b32: pubkey_b32.clone(),
            pubkey8: pubkey_b32.chars().take(8).collect(),
            role: role.to_string(),
            created: created.to_string(),
            note: "Persisted so this node's identity survives a restart. Deleting or \
                   regenerating it orphans every custody record already signed under the \
                   old key, and any endorsement issued for it. Keep it, back it up, and \
                   do not copy it to a second machine: two nodes sharing one key cannot \
                   be told apart."
                .into(),
        }
    }

    /// The signing key this file carries.
    pub fn signing_key(&self) -> Option<SigningKey> {
        let raw = unhex(&self.seed_hex)?;
        Some(SigningKey::from_bytes(&raw))
    }
}

/// A node asking to be enrolled.
///
/// It proves exactly one thing: whoever produced it holds the private half of
/// `node_key`. The platform and hardware model are the node's own claims about
/// itself and are evidence of nothing, which is why this is a request and not
/// an enrolment. An endorser reads it, decides whether the claims are true by
/// means outside this protocol (usually by having installed the machine), and
/// only then signs a platform attestation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JoinRequest {
    /// MUST equal [`JOIN_REQUEST_SCHEMA_V1`].
    pub schema: String,
    /// The key asking to be enrolled, base32-nopad lowercase.
    pub node_key: String,
    /// Substrate profile the node intends to write under.
    pub profile: String,
    /// Device platform id the node claims to be.
    pub platform: String,
    /// EAT `hwmodel` claim the node reports for itself.
    pub hwmodel: String,
    /// When the request was made, RFC 3339 UTC.
    pub created_at: String,
    /// What this request does and does not establish, in the signed body so a
    /// reader cannot be handed the claims without the caveat.
    pub proves: String,
    /// The node's own signature over
    /// [`emem_attest::join_request_preimage_v1`].
    pub self_signature: String,
    /// What to do with this file, for the human carrying it.
    pub next_step: String,
}

/// The sentence every join request carries about itself.
pub const PROVES: &str = "possession_of_node_key_only: this signature shows the sender holds the \
                          private half of node_key. The platform and hwmodel below are the node's \
                          own claims about itself and are not evidence. An endorser must decide \
                          whether they are true before signing a platform attestation.";

impl JoinRequest {
    /// Build and self-sign a join request.
    pub fn sign(
        key: &SigningKey,
        profile: &str,
        platform: &str,
        hwmodel: &str,
        created_at: &str,
    ) -> Self {
        let node_key = b32(key.verifying_key().as_bytes());
        let pre = emem_attest::join_request_preimage_v1(
            JOIN_REQUEST_SCHEMA_V1,
            &node_key,
            profile,
            platform,
            hwmodel,
            created_at,
        );
        use ed25519_dalek::Signer;
        let sig = key.sign(&pre);
        Self {
            schema: JOIN_REQUEST_SCHEMA_V1.into(),
            node_key,
            profile: profile.into(),
            platform: platform.into(),
            hwmodel: hwmodel.into(),
            created_at: created_at.into(),
            proves: PROVES.into(),
            self_signature: b32(&sig.to_bytes()),
            next_step: "Carry this to a connected machine holding the endorser key. Verify the \
                        self-signature, satisfy yourself the platform claim is true, then issue \
                        an emem.platform_attestation.v0 for this node_key and POST it to \
                        /v1/enroll_attested. Return the attestation to this node's input \
                        directory so it can carry its own endorsement."
                .into(),
        }
    }

    /// Check the self-signature, with nothing but the request.
    pub fn verify(&self) -> bool {
        use ed25519_dalek::{Signature, VerifyingKey};
        if self.schema != JOIN_REQUEST_SCHEMA_V1 || self.proves != PROVES {
            return false;
        }
        let Some(pk) = decode32(&self.node_key) else {
            return false;
        };
        let Ok(vk) = VerifyingKey::from_bytes(&pk) else {
            return false;
        };
        let Some(sig) = data_encoding::BASE32_NOPAD
            .decode(self.self_signature.to_uppercase().as_bytes())
            .ok()
            .and_then(|v| <[u8; 64]>::try_from(v.as_slice()).ok())
        else {
            return false;
        };
        let pre = emem_attest::join_request_preimage_v1(
            &self.schema,
            &self.node_key,
            &self.profile,
            &self.platform,
            &self.hwmodel,
            &self.created_at,
        );
        vk.verify_strict(&pre, &Signature::from_bytes(&sig)).is_ok()
    }
}

fn decode32(s: &str) -> Option<[u8; 32]> {
    data_encoding::BASE32_NOPAD
        .decode(s.to_uppercase().as_bytes())
        .ok()
        .and_then(|v| <[u8; 32]>::try_from(v.as_slice()).ok())
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_join_request_proves_key_possession_and_nothing_else() {
        let k = SigningKey::from_bytes(&[4u8; 32]);
        let j = JoinRequest::sign(
            &k,
            "orbital.satellite.v1",
            "nvidia.jetson-orin",
            "jetson-orin-nx",
            "2026-08-20T09:00:00Z",
        );
        assert!(j.verify());
        assert!(j.proves.starts_with("possession_of_node_key_only"));
    }

    /// Every claim is bound, so an endorser cannot be handed a request whose
    /// platform was edited after the node signed it.
    #[test]
    fn editing_any_claim_breaks_the_self_signature() {
        let k = SigningKey::from_bytes(&[4u8; 32]);
        let base = JoinRequest::sign(&k, "p", "pl", "hw", "t");
        for mutate in [
            |j: &mut JoinRequest| j.platform = "tpm2.host".into(),
            |j: &mut JoinRequest| j.profile = "robot.fleet.v1".into(),
            |j: &mut JoinRequest| j.hwmodel = "something-else".into(),
            |j: &mut JoinRequest| j.created_at = "2027-01-01T00:00:00Z".into(),
            |j: &mut JoinRequest| j.proves = "attested".into(),
        ] {
            let mut t = base.clone();
            mutate(&mut t);
            assert!(!t.verify(), "an edited claim must not verify");
        }
    }

    /// The key file round-trips, and the short form matches the long one.
    #[test]
    fn the_key_file_round_trips() {
        let f = NodeKeyFile::new([5u8; 32], "2026-08-20", "test node");
        let k = f.signing_key().expect("seed decodes");
        assert_eq!(b32(k.verifying_key().as_bytes()), f.pubkey_b32);
        assert_eq!(f.pubkey8, f.pubkey_b32[..8]);
        assert_eq!(f.alg, "ed25519");
    }
}
