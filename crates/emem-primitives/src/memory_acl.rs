//! Memory ACL — per-agent namespaces + ed25519 attester binding.
//!
//! The Anthropic memory tool is globally writable by default. To let
//! multiple agents share one responder without trampling each other's
//! state, an `attester` block can sign each write:
//!
//! ```text
//! sig = ed25519(blake3("emem.memory_write|" || verb || "|" || path || "|" || body_hash))
//! ```
//!
//! When an attester is present, the path namespace
//! `/memories/by_attester/<pubkey8>/...` is write-restricted to that
//! pubkey's signer. `<pubkey8>` = first 8 chars of the base32-nopad
//! lowercased pubkey. Other paths (including bare `/memories/...`)
//! remain open for back-compat with un-attested callers.
//!
//! When an attester is absent, behaviour is unchanged — the receipt
//! still binds the path + bytes to the responder identity.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

/// Path prefix that triggers attester ownership enforcement.
pub const BY_ATTESTER_PREFIX: &str = "/memories/by_attester/";

/// Length of the pubkey shortcode used in `/memories/by_attester/<short>/...`.
pub const PUBKEY_SHORT_LEN: usize = 8;

/// Attester binding carried alongside a memory write. The signature
/// covers a stable preimage; the responder verifies it before granting
/// any write that targets the attester-scoped namespace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryAttester {
    /// Base32-nopad lowercased 32-byte ed25519 pubkey.
    pub pubkey_b32: String,
    /// Base32-nopad lowercased 64-byte ed25519 signature over the
    /// `attester_preimage(verb, path, body_hash)` blake3 digest.
    pub sig_b32: String,
}

impl MemoryAttester {
    /// Build the 8-character shortcode used in
    /// `/memories/by_attester/<short>/...`.
    pub fn pubkey_short(&self) -> String {
        pubkey_short_from_b32(&self.pubkey_b32)
    }
}

/// Compute the canonical short form of a base32 pubkey: the first 8
/// chars of the lowercased base32-nopad encoding.
pub fn pubkey_short_from_b32(pubkey_b32: &str) -> String {
    let lc = pubkey_b32.to_lowercase();
    lc.chars().take(PUBKEY_SHORT_LEN).collect()
}

/// Hash the canonical attester preimage. The shape is fixed so a
/// client can sign offline without a server round-trip.
pub fn attester_preimage(verb: &str, path: &str, body_hash: &[u8; 32]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(b"emem.memory_write|");
    h.update(verb.as_bytes());
    h.update(b"|");
    h.update(path.as_bytes());
    h.update(b"|");
    h.update(body_hash);
    *h.finalize().as_bytes()
}

/// Compute the body-hash component of the attester preimage: blake3
/// over the file bytes. For verbs that don't carry a body (delete,
/// rename) callers pass an empty slice.
pub fn body_hash(bytes: &[u8]) -> [u8; 32] {
    *blake3::hash(bytes).as_bytes()
}

/// Verdict from validating an attester binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttestationVerdict {
    /// Signature verifies. Caller may proceed.
    Ok,
    /// Pubkey is malformed (wrong length, bad base32).
    BadPubkey,
    /// Signature is malformed or doesn't verify against the preimage.
    BadSignature,
    /// The namespace `/memories/by_attester/<short>/...` was targeted
    /// but the supplied pubkey's shortcode doesn't match.
    NamespaceMismatch,
}

/// Verify an attester binding for a write to `path` with `verb` and
/// the (already-hashed) body. Returns the verdict — caller maps the
/// non-`Ok` arms to HTTP errors.
pub fn verify_attester(
    verb: &str,
    path: &str,
    body_hash: &[u8; 32],
    attester: &MemoryAttester,
) -> AttestationVerdict {
    let pk_bytes =
        match data_encoding::BASE32_NOPAD.decode(attester.pubkey_b32.to_uppercase().as_bytes()) {
            Ok(b) if b.len() == 32 => {
                let mut a = [0u8; 32];
                a.copy_from_slice(&b);
                a
            }
            _ => return AttestationVerdict::BadPubkey,
        };
    let sig_bytes =
        match data_encoding::BASE32_NOPAD.decode(attester.sig_b32.to_uppercase().as_bytes()) {
            Ok(b) if b.len() == 64 => {
                let mut a = [0u8; 64];
                a.copy_from_slice(&b);
                a
            }
            _ => return AttestationVerdict::BadSignature,
        };

    let preimage = attester_preimage(verb, path, body_hash);
    let pk = match ed25519_dalek::VerifyingKey::from_bytes(&pk_bytes) {
        Ok(p) => p,
        Err(_) => return AttestationVerdict::BadPubkey,
    };
    let sig = ed25519_dalek::Signature::from_bytes(&sig_bytes);
    if pk.verify_strict(&preimage, &sig).is_err() {
        return AttestationVerdict::BadSignature;
    }

    // Enforce namespace ownership when the path lands in the attester
    // sub-tree. Other paths accept the signature as advisory binding;
    // the receipt records the pubkey for audit.
    if let Some(rest) = path.strip_prefix(BY_ATTESTER_PREFIX) {
        let claimed = rest.split('/').next().unwrap_or("");
        let short = pubkey_short_from_b32(&attester.pubkey_b32);
        if claimed != short {
            return AttestationVerdict::NamespaceMismatch;
        }
    }
    AttestationVerdict::Ok
}

/// When an attester is absent, only the `by_attester` namespace is
/// gated — bare `/memories/...` writes remain open. This helper
/// returns `true` if the path requires an attester but the caller
/// supplied none.
pub fn namespace_requires_attester(path: &str) -> bool {
    path.starts_with(BY_ATTESTER_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use rand::rngs::OsRng;
    use rand::RngCore;

    fn fresh_signer() -> SigningKey {
        let mut sec = [0u8; 32];
        OsRng.fill_bytes(&mut sec);
        SigningKey::from_bytes(&sec)
    }

    fn pubkey_b32_of(sk: &SigningKey) -> String {
        data_encoding::BASE32_NOPAD
            .encode(sk.verifying_key().as_bytes())
            .to_lowercase()
    }

    fn sign_b32(sk: &SigningKey, preimage: &[u8; 32]) -> String {
        let sig = sk.sign(preimage);
        data_encoding::BASE32_NOPAD
            .encode(&sig.to_bytes())
            .to_lowercase()
    }

    #[test]
    fn good_signature_ok_in_open_namespace() {
        let sk = fresh_signer();
        let pubkey_b32 = pubkey_b32_of(&sk);
        let body = b"hello";
        let bh = body_hash(body);
        let verb = "create";
        let path = "/memories/notes.md";
        let preimage = attester_preimage(verb, path, &bh);
        let attester = MemoryAttester {
            pubkey_b32: pubkey_b32.clone(),
            sig_b32: sign_b32(&sk, &preimage),
        };
        assert_eq!(
            verify_attester(verb, path, &bh, &attester),
            AttestationVerdict::Ok
        );
    }

    #[test]
    fn tampered_body_rejected() {
        let sk = fresh_signer();
        let pubkey_b32 = pubkey_b32_of(&sk);
        let path = "/memories/notes.md";
        let verb = "create";
        let preimage = attester_preimage(verb, path, &body_hash(b"hello"));
        let attester = MemoryAttester {
            pubkey_b32,
            sig_b32: sign_b32(&sk, &preimage),
        };
        // Verify against a different body — must reject.
        assert_eq!(
            verify_attester(verb, path, &body_hash(b"different"), &attester),
            AttestationVerdict::BadSignature
        );
    }

    #[test]
    fn namespace_mismatch_rejected() {
        let sk_a = fresh_signer();
        let sk_b = fresh_signer();
        let pubkey_b_short = pubkey_short_from_b32(&pubkey_b32_of(&sk_b));
        // A signs but the path lands under B's namespace.
        let path = format!("/memories/by_attester/{}/notes.md", pubkey_b_short);
        let bh = body_hash(b"x");
        let preimage = attester_preimage("create", &path, &bh);
        let attester = MemoryAttester {
            pubkey_b32: pubkey_b32_of(&sk_a),
            sig_b32: sign_b32(&sk_a, &preimage),
        };
        assert_eq!(
            verify_attester("create", &path, &bh, &attester),
            AttestationVerdict::NamespaceMismatch
        );
    }

    #[test]
    fn malformed_pubkey_rejected() {
        let attester = MemoryAttester {
            pubkey_b32: "not-base32-😀".into(),
            sig_b32: "still-junk".into(),
        };
        assert_eq!(
            verify_attester("create", "/memories/x.md", &[0u8; 32], &attester),
            AttestationVerdict::BadPubkey
        );
    }
}
