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
//! lowercased pubkey.
//!
//! Whether a write to any *other* path (including bare `/memories/...`)
//! may go out un-attested is not decided here: it is the responder's
//! `MemoryWritePolicy`, which refuses un-attested writes by default. The
//! `by_attester` sub-tree is gated regardless of policy, because its
//! ownership claim only means something if it is enforced.
//!
//! `verb` is part of the preimage, so a signature authorising a `create`
//! cannot be replayed as a `delete` of the same path.
//!
//! It could, however, be replayed as ITSELF. Caller signatures are persisted in
//! the ledger so authorship can be re-verified offline, which means every past
//! signature is public and a replay needs no interception at all. v2 closes
//! that by binding the version of the path the write intends to replace:
//!
//! ```text
//! sig = ed25519(blake3("emem.memory_write.v2|" || verb || "|" || path
//!                      || "|" || body_hash || "|" || base))
//! ```
//!
//! `base` is the `file_cid` currently at `path`, or `BASE_ABSENT` when nothing
//! is there. Replaying a signature after the path has moved on names a base
//! that no longer matches, and is refused.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

/// Path prefix that triggers attester ownership enforcement.
pub const BY_ATTESTER_PREFIX: &str = "/memories/by_attester/";

/// Path prefix reserved to the operator, closed to every key including the
/// responder's own, over the memory write API.
///
/// The open root is first-writer-owns, which is a coherent rule but makes any
/// *well-known* name a land-grab target: one call claims `/memories/standard.md`
/// and only that key may ever write it again, on a log that by design cannot be
/// pruned. Names under the open root are therefore unreserved, and nothing
/// should build a protocol dependency on one.
///
/// This prefix is the squat-proof home for the names that do need to be
/// resolvable — documentation, discovery, anything an agent is told to read at
/// a fixed path. Refused unconditionally rather than gated on the responder key
/// so there is no signing path to abuse at all: content here is placed by the
/// operator out of band. Chosen to match RFC 8615's meaning, so the prefix
/// announces its own semantics to anyone who has met `.well-known` elsewhere.
pub const RESERVED_PREFIX: &str = "/memories/.well-known/";

/// Whether `path` is reserved to the operator and closed to API writes.
pub fn namespace_is_reserved(path: &str) -> bool {
    path.starts_with(RESERVED_PREFIX)
}

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

/// The `base` component used when the target path currently holds no file.
///
/// A literal rather than an empty string, so "there was nothing here" is a
/// value a verifier can see in the preimage rather than an absence it has to
/// infer from a missing separator.
pub const BASE_ABSENT: &str = "absent";

/// The v2 attester preimage: v1 plus the version of the path this write
/// intends to replace.
///
/// WHY THIS EXISTS. Every caller signature is PERSISTED in the ledger, by
/// design, so a third party can re-verify authorship offline rather than
/// trusting the responder's word. That makes v1 replayable by anyone, with no
/// privileged position and nothing to intercept: read a past signature off the
/// public log and send it again. `verb` in the preimage stops a `create` being
/// replayed as a `delete`, and `rename` was hardened to bind its source path,
/// but nothing stopped a signature being replayed as ITSELF. A replayed
/// `delete` removes whatever now sits at that path, including a file recreated
/// since; a replayed `create` reverts a path to older content whose signature
/// is still perfectly good.
///
/// WHY A BASE VERSION AND NOT A NONCE OR A TIMESTAMP. A nonce needs the server
/// to remember what it has seen, which is state that a restart loses and that
/// grows without bound. A timestamp needs a freshness window, and a window
/// means an old signature in the log stops verifying as fresh later, which
/// attacks the one property this protocol sells: a signature stays checkable
/// forever, offline, against bytes anyone can fetch. Binding the base version
/// keeps that. The signature is still verifiable in ten years, because the base
/// it names is in the same append-only log as the write itself.
///
/// It also makes every mutation a compare-and-swap, so two agents editing one
/// path can no longer silently lose each other's work. That is a second bug
/// fixed by the same field, not a side effect to apologise for.
///
/// The cost is a read before a mutation, and it is deliberately not charged to
/// the common case: creating a path that does not exist yet signs
/// `base = BASE_ABSENT`, which a caller knows without asking.
pub fn attester_preimage_v2(verb: &str, path: &str, body_hash: &[u8; 32], base: &str) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(b"emem.memory_write.v2|");
    h.update(verb.as_bytes());
    h.update(b"|");
    h.update(path.as_bytes());
    h.update(b"|");
    h.update(body_hash);
    h.update(b"|");
    h.update(base.as_bytes());
    *h.finalize().as_bytes()
}

/// Verbs where a replayed signature destroys or overwrites existing content,
/// and which therefore require v2.
///
/// Enforcement is graduated on measurement rather than on principle: across
/// thirty days of this responder's log there were 50 `create` calls and ZERO
/// `delete` or `rename`. Requiring v2 for the destructive pair costs no live
/// caller anything today, and closes the sharp edge now instead of at the end
/// of a deprecation window. The additive verbs keep accepting v1 while clients
/// migrate, because breaking a working signer to fix a weaker case would be
/// trading a real outage for a smaller risk.
pub const V2_REQUIRED_VERBS: &[&str] = &["delete", "rename"];

/// Whether this verb refuses a v1 signature.
pub fn requires_v2(verb: &str) -> bool {
    V2_REQUIRED_VERBS.contains(&verb)
}

/// Compute the body-hash component of the attester preimage: blake3
/// over the request's canonical body bytes. `delete` carries no body,
/// so it hashes an empty slice. `rename` carries its source path: see
/// [`rename_body_hash`].
pub fn body_hash(bytes: &[u8]) -> [u8; 32] {
    *blake3::hash(bytes).as_bytes()
}

/// The body-hash for a `rename`, which is blake3 over the *source*
/// path's UTF-8 bytes.
///
/// A rename's canonical body is the pair of paths it moves between. The
/// destination already rides the preimage's `path` component, so hashing
/// the source here makes one signature bind both ends. Without it the
/// signature authorises "move something to `new_path`" while naming no
/// source, and a captured signature could be replayed to drag a
/// different file to the same destination.
pub fn rename_body_hash(old_path: &str) -> [u8; 32] {
    body_hash(old_path.as_bytes())
}

/// Whether `pubkey_b32` may write at `path` under the namespace rule.
///
/// Paths outside [`BY_ATTESTER_PREFIX`] have no owner, so every key
/// passes. A path inside it passes only for the key whose shortcode
/// names the namespace.
///
/// This is the namespace half of [`verify_attester`], split out because
/// a rename has to answer "does this key own the source?" for a path the
/// signature does not cover. Verifying the signature against the source
/// would be checking one signature against two different messages, which
/// no ed25519 signature can satisfy.
pub fn namespace_ownership_ok(path: &str, pubkey_b32: &str) -> bool {
    match path.strip_prefix(BY_ATTESTER_PREFIX) {
        Some(rest) => rest.split('/').next().unwrap_or("") == pubkey_short_from_b32(pubkey_b32),
        None => true,
    }
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
    if !namespace_ownership_ok(path, &attester.pubkey_b32) {
        return AttestationVerdict::NamespaceMismatch;
    }
    AttestationVerdict::Ok
}

/// Verify an attester binding, accepting a v2 (base-bound) signature and, for
/// verbs where replay is not destructive, a v1 one.
///
/// v2 is tried first so a client that has migrated is never asked to fall back.
/// [`requires_v2`] names the verbs that refuse v1 outright: replaying one of
/// those destroys or overwrites whatever occupies the path today, which is the
/// harm, and the responder's log shows no live caller using either.
///
/// A v1 signature that verifies is REPORTED as such rather than silently
/// accepted, so the caller can be told it is signing a deprecated shape while
/// its write still succeeds.
pub fn verify_attester_versioned(
    verb: &str,
    path: &str,
    body_hash: &[u8; 32],
    base: &str,
    attester: &MemoryAttester,
) -> (AttestationVerdict, PreimageVersion) {
    let pk_bytes =
        match data_encoding::BASE32_NOPAD.decode(attester.pubkey_b32.to_uppercase().as_bytes()) {
            Ok(b) if b.len() == 32 => {
                let mut a = [0u8; 32];
                a.copy_from_slice(&b);
                a
            }
            _ => return (AttestationVerdict::BadPubkey, PreimageVersion::V2),
        };
    let sig_bytes =
        match data_encoding::BASE32_NOPAD.decode(attester.sig_b32.to_uppercase().as_bytes()) {
            Ok(b) if b.len() == 64 => {
                let mut a = [0u8; 64];
                a.copy_from_slice(&b);
                a
            }
            _ => return (AttestationVerdict::BadSignature, PreimageVersion::V2),
        };
    let pk = match ed25519_dalek::VerifyingKey::from_bytes(&pk_bytes) {
        Ok(p) => p,
        Err(_) => return (AttestationVerdict::BadPubkey, PreimageVersion::V2),
    };
    let sig = ed25519_dalek::Signature::from_bytes(&sig_bytes);

    let v2 = attester_preimage_v2(verb, path, body_hash, base);
    let version = if pk.verify_strict(&v2, &sig).is_ok() {
        PreimageVersion::V2
    } else if requires_v2(verb) {
        return (AttestationVerdict::BadSignature, PreimageVersion::V2);
    } else if pk
        .verify_strict(&attester_preimage(verb, path, body_hash), &sig)
        .is_ok()
    {
        PreimageVersion::V1
    } else {
        return (AttestationVerdict::BadSignature, PreimageVersion::V2);
    };

    if !namespace_ownership_ok(path, &attester.pubkey_b32) {
        return (AttestationVerdict::NamespaceMismatch, version);
    }
    (AttestationVerdict::Ok, version)
}

/// Which preimage a signature verified against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreimageVersion {
    /// `blake3("emem.memory_write|" || verb || "|" || path || "|" || body_hash)`.
    /// Replayable across a restart; accepted only for additive verbs.
    V1,
    /// v1 plus the base version being replaced. Not replayable.
    V2,
}

/// Whether `path` is gated by the namespace rule alone, independent of
/// the responder's write policy.
///
/// Only the `by_attester` sub-tree is: a write there claims ownership,
/// so it must prove it. Whether bare `/memories/...` also demands an
/// attester is a policy question the responder answers, not this rule.
pub fn namespace_requires_attester(path: &str) -> bool {
    path.starts_with(BY_ATTESTER_PREFIX)
}

#[cfg(test)]
mod tests {

    /// A signature that was valid once must not be valid again after the path
    /// has moved on.
    ///
    /// Every caller signature is PERSISTED in the ledger so a third party can
    /// re-verify authorship offline. That is a deliberate property and it is
    /// also why replay needs no interception: the signatures are public. The
    /// in-memory replay set on the responder closes this within one process
    /// lifetime and says so in its own comment; this closes it across a restart,
    /// because the base is read from the store rather than remembered.
    ///
    /// The CONTROL is the first assertion. Without it this test passes equally
    /// well against a build that rejects every signature, which would be a
    /// worse bug wearing this test as a pass.
    #[test]
    fn a_delete_signature_cannot_be_replayed_once_the_path_moves_on() {
        use ed25519_dalek::{Signer, SigningKey};
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        let pubkey_b32 = data_encoding::BASE32_NOPAD
            .encode(sk.verifying_key().as_bytes())
            .to_lowercase();
        let path = format!("/memories/by_attester/{}/note.md", &pubkey_b32[..8]);
        let bh = body_hash(b"");

        // The version of the file the delete was authorised against.
        let base_then = "cidaaaaaaaaaaaaaaaaaaaaaaa";
        let sig = sk.sign(&attester_preimage_v2("delete", &path, &bh, base_then));
        let att = MemoryAttester {
            pubkey_b32: pubkey_b32.clone(),
            sig_b32: data_encoding::BASE32_NOPAD
                .encode(&sig.to_bytes())
                .to_lowercase(),
        };

        // CONTROL: against the base it was signed for, it verifies.
        let (verdict, version) = verify_attester_versioned("delete", &path, &bh, base_then, &att);
        assert_eq!(verdict, AttestationVerdict::Ok, "control: must verify once");
        assert_eq!(version, PreimageVersion::V2);

        // THE ATTACK: the file was deleted, someone recreated it, and the same
        // signature is read off the public log and sent again. It now names a
        // base that is not there.
        let base_now = "cidbbbbbbbbbbbbbbbbbbbbbbb";
        let (replayed, _) = verify_attester_versioned("delete", &path, &bh, base_now, &att);
        assert_eq!(
            replayed,
            AttestationVerdict::BadSignature,
            "a delete signature replayed against a different version must be refused"
        );

        // And against a path that now holds nothing at all.
        let (on_absent, _) = verify_attester_versioned("delete", &path, &bh, BASE_ABSENT, &att);
        assert_eq!(on_absent, AttestationVerdict::BadSignature);
    }

    /// v1 is refused for the destructive verbs and still accepted for the
    /// additive ones, which is the migration this ships with.
    #[test]
    fn v1_is_refused_where_replay_destroys_and_accepted_where_it_does_not() {
        use ed25519_dalek::{Signer, SigningKey};
        let sk = SigningKey::from_bytes(&[9u8; 32]);
        let pubkey_b32 = data_encoding::BASE32_NOPAD
            .encode(sk.verifying_key().as_bytes())
            .to_lowercase();
        let path = format!("/memories/by_attester/{}/note.md", &pubkey_b32[..8]);
        let bh = body_hash(b"hello");

        let v1 = |verb: &str| {
            let sig = sk.sign(&attester_preimage(verb, &path, &bh));
            MemoryAttester {
                pubkey_b32: pubkey_b32.clone(),
                sig_b32: data_encoding::BASE32_NOPAD
                    .encode(&sig.to_bytes())
                    .to_lowercase(),
            }
        };

        // Additive: a v1 signature still works, so no live signer breaks today.
        let (verdict, version) =
            verify_attester_versioned("create", &path, &bh, BASE_ABSENT, &v1("create"));
        assert_eq!(
            verdict,
            AttestationVerdict::Ok,
            "v1 create must still verify"
        );
        assert_eq!(version, PreimageVersion::V1, "and be reported as v1");

        // Destructive: a v1 signature is refused outright.
        for verb in V2_REQUIRED_VERBS {
            let (verdict, _) = verify_attester_versioned(verb, &path, &bh, BASE_ABSENT, &v1(verb));
            assert_eq!(
                verdict,
                AttestationVerdict::BadSignature,
                "v1 `{verb}` must be refused: replaying it destroys what is there now"
            );
        }
    }

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

    /// A rename inside one's own namespace has to actually work. The
    /// responder used to verify the caller's signature a second time
    /// against a preimage built from `old_path`, which no signature over
    /// `new_path` can satisfy, so every such rename was refused.
    #[test]
    fn rename_within_own_namespace_verifies() {
        let sk = fresh_signer();
        let pubkey_b32 = pubkey_b32_of(&sk);
        let short = pubkey_short_from_b32(&pubkey_b32);
        let old_path = format!("/memories/by_attester/{short}/a.md");
        let new_path = format!("/memories/by_attester/{short}/b.md");

        let bh = rename_body_hash(&old_path);
        let attester = MemoryAttester {
            pubkey_b32: pubkey_b32.clone(),
            sig_b32: sign_b32(&sk, &attester_preimage("rename", &new_path, &bh)),
        };

        // The one signature covers the destination...
        assert_eq!(
            verify_attester("rename", &new_path, &bh, &attester),
            AttestationVerdict::Ok
        );
        // ...and the source is answered as a namespace question.
        assert!(namespace_ownership_ok(&old_path, &pubkey_b32));
    }

    /// The source path is bound by the signature, so a captured rename
    /// signature cannot be replayed to drag a different file to the
    /// same destination.
    #[test]
    fn rename_signature_binds_the_source_path() {
        let sk = fresh_signer();
        let pubkey_b32 = pubkey_b32_of(&sk);
        let short = pubkey_short_from_b32(&pubkey_b32);
        let new_path = format!("/memories/by_attester/{short}/b.md");
        let signed_source = format!("/memories/by_attester/{short}/a.md");
        let other_source = format!("/memories/by_attester/{short}/secrets.md");

        let attester = MemoryAttester {
            pubkey_b32,
            sig_b32: sign_b32(
                &sk,
                &attester_preimage("rename", &new_path, &rename_body_hash(&signed_source)),
            ),
        };

        assert_eq!(
            verify_attester(
                "rename",
                &new_path,
                &rename_body_hash(&other_source),
                &attester
            ),
            AttestationVerdict::BadSignature
        );
    }

    /// Moving a file *out* of someone else's namespace stays refused,
    /// and that refusal is a namespace verdict rather than a signature
    /// one, since the signature legitimately covers only the target.
    #[test]
    fn rename_out_of_a_foreign_namespace_refused() {
        let sk_a = fresh_signer();
        let sk_b = fresh_signer();
        let short_a = pubkey_short_from_b32(&pubkey_b32_of(&sk_a));
        let short_b = pubkey_short_from_b32(&pubkey_b32_of(&sk_b));

        // A signs a move of B's file into A's own namespace.
        let old_path = format!("/memories/by_attester/{short_b}/private.md");
        let new_path = format!("/memories/by_attester/{short_a}/stolen.md");
        let bh = rename_body_hash(&old_path);
        let attester = MemoryAttester {
            pubkey_b32: pubkey_b32_of(&sk_a),
            sig_b32: sign_b32(&sk_a, &attester_preimage("rename", &new_path, &bh)),
        };

        // The signature itself is honest about the destination.
        assert_eq!(
            verify_attester("rename", &new_path, &bh, &attester),
            AttestationVerdict::Ok
        );
        // The source is what stops it.
        assert!(!namespace_ownership_ok(&old_path, &attester.pubkey_b32));
    }

    #[test]
    fn unowned_paths_have_no_namespace_owner() {
        let sk = fresh_signer();
        let pubkey_b32 = pubkey_b32_of(&sk);
        // Nothing outside the sub-tree is owned, so the rule passes and
        // policy alone decides.
        assert!(namespace_ownership_ok("/memories/notes.md", &pubkey_b32));
        assert!(namespace_ownership_ok("/memories/by_attester", &pubkey_b32));
    }

    /// The verb is part of the preimage: a `create` signature must not
    /// be replayable as a `delete` of the same path.
    #[test]
    fn signature_does_not_cross_verbs() {
        let sk = fresh_signer();
        let pubkey_b32 = pubkey_b32_of(&sk);
        let path = "/memories/notes.md";
        let bh = body_hash(b"");
        let attester = MemoryAttester {
            pubkey_b32,
            sig_b32: sign_b32(&sk, &attester_preimage("create", path, &bh)),
        };
        assert_eq!(
            verify_attester("delete", path, &bh, &attester),
            AttestationVerdict::BadSignature
        );
    }

    /// The reserved prefix is closed to every key, and closed for every verb.
    ///
    /// It exists because the open root is first-writer-owns: one call claims
    /// `/memories/standard.md` and only that key may write it again, forever,
    /// on a log that cannot be pruned. Anything an agent is told to read at a
    /// fixed path therefore needs a home no caller can claim.
    #[test]
    fn reserved_prefix_is_closed_and_distinct_from_the_open_root() {
        assert!(namespace_is_reserved("/memories/.well-known/standard.md"));
        assert!(namespace_is_reserved("/memories/.well-known/nested/a.md"));

        // The squattable shapes the reservation is contrasted with.
        assert!(!namespace_is_reserved("/memories/standard.md"));
        assert!(!namespace_is_reserved(
            "/memories/by_attester/k572x7go/notes.md"
        ));

        // A near-miss must not be treated as reserved: the guard is a prefix
        // test, so a sibling directory that merely starts similarly is open.
        assert!(!namespace_is_reserved("/memories/.well-knownish/a.md"));
        assert!(!namespace_is_reserved("/memories/well-known/a.md"));

        // Reserved and owner-enforced are different questions, and the
        // reserved check must not depend on the by_attester rule.
        assert!(!namespace_requires_attester(RESERVED_PREFIX));
    }
}
