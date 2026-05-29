//! Vault memory kind (v0.0.8) — AEAD-sealed secrets at rest.
//!
//! A `kind=vault` memory entry is encrypted before it touches sled. The
//! plaintext is NEVER written to the regular memory-file trees, so it is
//! structurally invisible to the BGE search indexer and the
//! contradiction scanner (both of which only ever read the plaintext
//! file trees). The sealed envelope lives in its own
//! [`emem_storage::TREE_MEMORY_VAULT`] tree keyed by path.
//!
//! ## Crypto
//!
//! - **KDF**: HKDF-SHA512 with `ikm = responder ed25519 secret bytes`
//!   (32 B), `salt = "emem.vault.v1"`, `info = "chacha20poly1305-key"`
//!   → a 32-byte ChaCha20-Poly1305 key. The key is derived per call from
//!   the responder identity, never persisted.
//! - **AEAD**: ChaCha20-Poly1305 with a fresh random 96-bit nonce per
//!   seal and `aad = path` bytes (so a ciphertext can't be replayed under
//!   a different path).
//! - **Capability**: a `memory_view` of a Vault entry returns
//!   ciphertext-only UNLESS the caller presents an ed25519 signature over
//!   the preimage `blake3("emem.vault_open|" || path || "|" || nonce)`
//!   that verifies under the attester pubkey whose secret derived the
//!   AEAD key (i.e. the responder's own pubkey). With a valid cap the
//!   responder decrypts and returns the plaintext.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use ed25519_dalek::{Signature, SigningKey, Verifier, VerifyingKey};
use hkdf::Hkdf;
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::Sha512;

/// HKDF salt — domain-separates the vault key from any other use of the
/// responder secret.
const VAULT_HKDF_SALT: &[u8] = b"emem.vault.v1";
/// HKDF info string — binds the derived bytes to the AEAD they key.
const VAULT_HKDF_INFO: &[u8] = b"chacha20poly1305-key";
/// Capability preimage domain tag. Must match the `/verify`-style
/// preimage every caller signs to open a vault entry.
const VAULT_OPEN_DOMAIN: &str = "emem.vault_open|";

/// The sealed envelope persisted in `TREE_MEMORY_VAULT`. Canonical CBOR.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SealedVaultEntry {
    /// Schema discriminator for wire stability.
    #[serde(default = "default_vault_schema")]
    pub schema: String,
    /// Memory path the secret lives at (also the AEAD AAD).
    pub path: String,
    /// 96-bit AEAD nonce (12 bytes), base32-nopad-lc.
    pub nonce_b32: String,
    /// ChaCha20-Poly1305 ciphertext (includes the 16-byte tag),
    /// base32-nopad-lc.
    pub ciphertext_b32: String,
    /// AEAD additional-authenticated-data = the path bytes. Stored
    /// explicitly so an offline auditor sees exactly what was bound.
    pub aad: String,
    /// Responder pubkey (base32-nopad-lc) whose secret derived the AEAD
    /// key — also the key a capability signature must verify under.
    pub responder_pubkey_b32: String,
    /// Plaintext byte length (so callers can size buffers without
    /// decrypting; does not leak content).
    pub plaintext_len: u64,
    /// ISO 8601 UTC instant the entry was sealed.
    pub sealed_at: String,
}

fn default_vault_schema() -> String {
    "emem.vault.v1".to_string()
}

/// Errors from the vault seal/open path.
#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    /// AEAD encrypt/decrypt failed (bad key, tampered ciphertext, wrong
    /// AAD).
    #[error("aead: {0}")]
    Aead(String),
    /// The supplied capability signature did not verify.
    #[error("capability: {0}")]
    Capability(String),
    /// Malformed wire field (base32, length, etc.).
    #[error("decode: {0}")]
    Decode(String),
}

/// Derive the per-responder ChaCha20-Poly1305 key from the responder
/// ed25519 secret via HKDF-SHA512. Pure + deterministic for a given
/// secret, so the same responder always opens its own sealed entries.
fn derive_key(responder_secret: &[u8; 32]) -> Key {
    let hk = Hkdf::<Sha512>::new(Some(VAULT_HKDF_SALT), responder_secret);
    let mut okm = [0u8; 32];
    // expand is infallible for a 32-byte OKM with SHA-512.
    hk.expand(VAULT_HKDF_INFO, &mut okm)
        .expect("HKDF-SHA512 expand to 32 bytes is infallible");
    Key::from(okm)
}

/// The capability preimage an opener must sign: the 32-byte blake3 of
/// `"emem.vault_open|" || path || "|" || nonce_bytes`.
pub fn vault_open_preimage_digest(path: &str, nonce: &[u8]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(VAULT_OPEN_DOMAIN.as_bytes());
    h.update(path.as_bytes());
    h.update(b"|");
    h.update(nonce);
    *h.finalize().as_bytes()
}

/// Seal `plaintext` under the responder secret, binding the path as AAD.
/// Returns the envelope ready to persist in `TREE_MEMORY_VAULT`.
pub fn seal(
    responder_secret: &[u8; 32],
    responder_pubkey: &[u8; 32],
    path: &str,
    plaintext: &[u8],
    sealed_at: String,
) -> Result<SealedVaultEntry, VaultError> {
    let key = derive_key(responder_secret);
    let cipher = ChaCha20Poly1305::new(&key);
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from(nonce_bytes);
    let ciphertext = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: plaintext,
                aad: path.as_bytes(),
            },
        )
        .map_err(|e| VaultError::Aead(format!("encrypt: {e}")))?;
    Ok(SealedVaultEntry {
        schema: default_vault_schema(),
        path: path.to_string(),
        nonce_b32: b32(&nonce_bytes),
        ciphertext_b32: b32(&ciphertext),
        aad: path.to_string(),
        responder_pubkey_b32: b32(responder_pubkey),
        plaintext_len: plaintext.len() as u64,
        sealed_at,
    })
}

/// Verify a capability and, on success, decrypt the sealed entry.
///
/// `cap_sig` is an ed25519 signature over
/// [`vault_open_preimage_digest`]`(path, nonce)`. It must verify under
/// the `responder_pubkey_b32` recorded in the envelope (the key whose
/// secret derived the AEAD key). On a valid cap we re-derive the AEAD key
/// from `responder_secret` and decrypt.
pub fn open(
    responder_secret: &[u8; 32],
    entry: &SealedVaultEntry,
    cap_sig_b32: &str,
) -> Result<Vec<u8>, VaultError> {
    let nonce_bytes = unb32(&entry.nonce_b32)?;
    if nonce_bytes.len() != 12 {
        return Err(VaultError::Decode(format!(
            "nonce must be 12 bytes, got {}",
            nonce_bytes.len()
        )));
    }
    // Verify the capability signature under the recorded responder key.
    let pk_bytes = unb32(&entry.responder_pubkey_b32)?;
    let pk_arr: [u8; 32] = pk_bytes
        .as_slice()
        .try_into()
        .map_err(|_| VaultError::Decode("responder pubkey not 32 bytes".into()))?;
    let vk = VerifyingKey::from_bytes(&pk_arr)
        .map_err(|e| VaultError::Capability(format!("bad responder pubkey: {e}")))?;
    let sig_bytes = unb32(cap_sig_b32)?;
    let sig_arr: [u8; 64] = sig_bytes
        .as_slice()
        .try_into()
        .map_err(|_| VaultError::Capability("capability signature not 64 bytes".into()))?;
    let sig = Signature::from_bytes(&sig_arr);
    let digest = vault_open_preimage_digest(&entry.path, &nonce_bytes);
    vk.verify(&digest, &sig)
        .map_err(|e| VaultError::Capability(format!("signature does not verify: {e}")))?;

    // Capability is valid → decrypt.
    let key = derive_key(responder_secret);
    let cipher = ChaCha20Poly1305::new(&key);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = unb32(&entry.ciphertext_b32)?;
    cipher
        .decrypt(
            nonce,
            Payload {
                msg: &ciphertext,
                aad: entry.aad.as_bytes(),
            },
        )
        .map_err(|e| VaultError::Aead(format!("decrypt: {e}")))
}

/// Compute the capability signature for `path`+`nonce` under a signing
/// key. Used by the test suite and by any first-party caller that holds
/// the responder secret; external callers compute this client-side (so
/// in a non-test build it is dead from the crate's POV).
#[cfg_attr(not(test), allow(dead_code))]
pub fn make_capability(signing: &SigningKey, path: &str, nonce: &[u8]) -> String {
    use ed25519_dalek::Signer;
    let digest = vault_open_preimage_digest(path, nonce);
    let sig = signing.sign(&digest);
    b32(&sig.to_bytes())
}

fn b32(bytes: &[u8]) -> String {
    data_encoding::BASE32_NOPAD.encode(bytes).to_lowercase()
}

fn unb32(s: &str) -> Result<Vec<u8>, VaultError> {
    data_encoding::BASE32_NOPAD
        .decode(s.to_uppercase().as_bytes())
        .map_err(|e| VaultError::Decode(format!("base32: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;

    fn fresh() -> ([u8; 32], [u8; 32], SigningKey) {
        let mut sec = [0u8; 32];
        OsRng.fill_bytes(&mut sec);
        let signing = SigningKey::from_bytes(&sec);
        let mut pk = [0u8; 32];
        pk.copy_from_slice(signing.verifying_key().as_bytes());
        (sec, pk, signing)
    }

    #[test]
    fn seal_then_open_with_valid_cap_round_trips() {
        let (sec, pk, signing) = fresh();
        let path = "/memories/secrets/openai.key";
        let plaintext = b"OPENAI_API_KEY=sk-test123";
        let entry = seal(&sec, &pk, path, plaintext, "2026-05-29T00:00:00Z".into()).unwrap();
        let nonce = unb32(&entry.nonce_b32).unwrap();
        let cap = make_capability(&signing, path, &nonce);
        let opened = open(&sec, &entry, &cap).unwrap();
        assert_eq!(opened, plaintext);
    }

    #[test]
    fn open_without_valid_cap_fails() {
        let (sec, pk, _signing) = fresh();
        let path = "/memories/secrets/x";
        let entry = seal(&sec, &pk, path, b"top secret", "t".into()).unwrap();
        // A capability signed by a DIFFERENT key must not open it.
        let (_sec2, _pk2, other) = fresh();
        let nonce = unb32(&entry.nonce_b32).unwrap();
        let bad_cap = make_capability(&other, path, &nonce);
        assert!(open(&sec, &entry, &bad_cap).is_err());
    }

    #[test]
    fn aad_path_binding_prevents_swap() {
        let (sec, pk, signing) = fresh();
        let entry = seal(&sec, &pk, "/memories/a", b"secret-a", "t".into()).unwrap();
        // Tamper: pretend this ciphertext lives at a different path.
        let mut moved = entry.clone();
        moved.path = "/memories/b".into();
        moved.aad = "/memories/b".into();
        let nonce = unb32(&moved.nonce_b32).unwrap();
        // Even a valid cap over the NEW path can't decrypt — AEAD AAD no
        // longer matches what was sealed.
        let cap = make_capability(&signing, &moved.path, &nonce);
        assert!(open(&sec, &moved, &cap).is_err());
    }
}
