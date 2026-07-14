//! Caller-registered derivations: the canonical body a `/v1/derive`
//! signature covers.
//!
//! `DerivativeFact` has always been able to express "this value is a
//! function over these parent fact CIDs". What was missing is a way for
//! someone other than the responder to register one. This module defines
//! the bytes such a caller signs, and nothing else: the responder-side
//! policy (which provenance classes are acceptable, whether the parents
//! resolve) lives at the API layer.
//!
//! The binding reuses the memory-write attester scheme verbatim, with
//! `verb = "derive"` and `path = "/v1/derive"`:
//!
//! ```text
//! sig = ed25519(blake3("emem.memory_write|derive|/v1/derive|" || body_hash))
//! body_hash = blake3(cbor(DeriveBody))
//! ```
//!
//! `cbor(DeriveBody)` is ciborium's struct encoding: a definite-length
//! 10-entry map whose keys appear in DECLARATION order, not sorted. It is
//! deterministic, but it is deliberately not RFC 8949 §4.2.1 key-sorted,
//! so a generic canonical-CBOR encoder pointed at this struct will sort
//! the keys and compute the wrong digest. `confidence` rides as a CBOR
//! float32 because [`emem_fact::DerivativeFact::confidence`] is an `f32`
//! and the two have to agree bit-for-bit; `value` is float-canonicalized
//! and follows the §4.2 deterministic rules within its own subtree. The
//! responder publishes all of this per-request in
//! `details.how_to_sign.how_it_was_built.body_cbor_rules`, and a caller
//! who would rather not implement it can send the request unattested and
//! sign the digest the refusal hands back.
//!
//! One scheme, one verifier ([`crate::memory_acl::verify_attester`]). The
//! verb is what separates the domains: no memory-file verb is spelled
//! `derive`, so a signature minted for a file write can never be replayed
//! as a derivation, and vice versa. `/v1/derive` sits outside
//! [`crate::memory_acl::BY_ATTESTER_PREFIX`], so the namespace rule is a
//! no-op here and the signature binds content alone.
//!
//! [`DeriveBody`] carries only fields that survive onto the stored fact,
//! so a third party holding the fact can rebuild these bytes and re-check
//! the caller's signature without the original HTTP request. That is the
//! difference between recording "this key signed something" and proving
//! "this key signed THIS derivation".

#![forbid(unsafe_code)]

use serde::Serialize;

/// Verb component of the derive attester preimage.
pub const DERIVE_VERB: &str = "derive";

/// Path component of the derive attester preimage. Fixed: the derivation's
/// content is bound through `body_hash`, so the path only has to
/// domain-separate, and a constant does that without inviting a caller to
/// guess at spelling.
pub const DERIVE_PATH: &str = "/v1/derive";

/// The provenance classes a caller may declare on a derivation.
///
/// A caller-registered derivative is untrusted input. The responder did
/// not compute the value and cannot re-derive it, so it must not be filed
/// under a class that asserts otherwise: `direct_sensor` claims a sensor
/// produced it, and `deterministic_index` claims anyone can recompute it
/// from the cited raw source. Both would be the responder vouching for a
/// stranger's arithmetic. The caller declares which of these two honest
/// classes applies; the responder validates membership and nothing more.
pub const CALLER_PROVENANCE_CLASSES: &[&str] = &["model_output", "human_curated"];

/// Whether `class` is one a caller may declare on a derivation.
pub fn is_caller_provenance_class(class: &str) -> bool {
    CALLER_PROVENANCE_CLASSES.contains(&class)
}

/// The canonical body a derive signature covers.
///
/// Field order below IS the wire order: serde emits struct fields in
/// declaration order, so moving a field here silently changes every
/// future digest. Every field is recoverable from the stored
/// `DerivativeFact`, which is what makes the caller's signature
/// re-checkable by a third party holding only the fact:
///
/// | field              | where it lives on the fact          |
/// |--------------------|-------------------------------------|
/// | `fn_key`           | `derivation.fn_key`                 |
/// | `inputs`           | `derivation.args.inputs`            |
/// | `cell`             | `cell`                              |
/// | `band`             | `band`                              |
/// | `tslot_window`     | `tslot_window`                      |
/// | `op`               | `op`                                |
/// | `value`            | `value`                             |
/// | `confidence`       | `confidence`                        |
/// | `provenance_class` | `derivation.args.provenance_class`  |
/// | `code_cid`         | `derivation.args.code_cid`          |
#[derive(Debug, Clone, Serialize)]
pub struct DeriveBody {
    /// Derivation recipe key the caller is registering under, e.g.
    /// `"same_doy_ndvi_delta@1"`. Free-form: this is the caller's recipe,
    /// not an entry in the responder's function registry.
    pub fn_key: String,
    /// Parent tokens in caller order, each `emem:fact:<cell64>:<fact_cid>`.
    /// Order is significant and signed: a derivation over `[a, b]` is not
    /// the one over `[b, a]` for any non-commutative `op`.
    pub inputs: Vec<String>,
    /// cell64 the derivative is anchored at.
    pub cell: String,
    /// Band key the derivative pertains to.
    pub band: String,
    /// Inclusive `[start, end]` tslot window the derivation spans.
    pub tslot_window: [u64; 2],
    /// Operator: `"delta"`, `"mean"`, `"trend"`, ...
    pub op: String,
    /// The derived value, already float-canonicalized (see
    /// [`emem_fact::cbor::canonicalize_value`]). Canonicalizing before
    /// hashing is what keeps the caller's digest and the stored fact's
    /// bytes describing the same number.
    pub value: ciborium::Value,
    /// Caller's confidence in the derived value, 0..1.
    pub confidence: f32,
    /// One of [`CALLER_PROVENANCE_CLASSES`].
    pub provenance_class: String,
    /// Optional blake3 of the code that computed the value. The responder
    /// never fetches or runs it; it is recorded so a verifier can ask the
    /// caller for the named artifact.
    pub code_cid: Option<String>,
}

impl DeriveBody {
    /// blake3 over the CBOR encoding of this body: the `body_hash`
    /// component of the attester preimage. See the module docs for the
    /// exact encoding a third party must reproduce.
    pub fn body_hash(&self) -> Result<[u8; 32], String> {
        let bytes = emem_fact::cbor::to_canonical_cbor(self)
            .map_err(|e| format!("derive body canonical cbor: {e}"))?;
        Ok(*blake3::hash(&bytes).as_bytes())
    }

    /// The 32-byte digest a caller signs to register this derivation.
    pub fn preimage(&self) -> Result<[u8; 32], String> {
        Ok(crate::memory_acl::attester_preimage(
            DERIVE_VERB,
            DERIVE_PATH,
            &self.body_hash()?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_acl::{verify_attester, AttestationVerdict, MemoryAttester};
    use ed25519_dalek::{Signer, SigningKey};
    use rand::rngs::OsRng;
    use rand::RngCore;

    fn body() -> DeriveBody {
        DeriveBody {
            fn_key: "same_doy_ndvi_delta@1".into(),
            inputs: vec!["emem:fact:defi.zb4d9.pefa.zf619:aaaa".into()],
            cell: "defi.zb4d9.pefa.zf619".into(),
            band: "indices.ndvi".into(),
            tslot_window: [10, 20],
            op: "delta".into(),
            value: ciborium::Value::Float(0.25),
            confidence: 0.9,
            provenance_class: "model_output".into(),
            code_cid: None,
        }
    }

    fn fresh_signer() -> SigningKey {
        let mut sec = [0u8; 32];
        OsRng.fill_bytes(&mut sec);
        SigningKey::from_bytes(&sec)
    }

    fn b32(b: &[u8]) -> String {
        data_encoding::BASE32_NOPAD.encode(b).to_lowercase()
    }

    #[test]
    fn body_hash_is_stable_across_encodings() {
        assert_eq!(body().body_hash().unwrap(), body().body_hash().unwrap());
    }

    /// The wire encoding is a published contract: an independent client
    /// signs these exact bytes, and a verifier rebuilds them from a stored
    /// fact years later. Pin them. Reordering a `DeriveBody` field or
    /// widening `confidence` to f64 would silently invalidate every
    /// signature ever issued, and only a byte-level assertion catches it.
    #[test]
    fn body_cbor_wire_form_is_pinned() {
        let b = DeriveBody {
            fn_key: "f@1".into(),
            inputs: vec!["a".into()],
            cell: "c".into(),
            band: "b".into(),
            tslot_window: [10, 20],
            op: "delta".into(),
            value: ciborium::Value::Float(0.14),
            confidence: 0.9,
            provenance_class: "model_output".into(),
            code_cid: None,
        };
        let hex = data_encoding::HEXLOWER.encode(&emem_fact::cbor::to_canonical_cbor(&b).unwrap());
        assert_eq!(
            hex,
            concat!(
                "aa", // map(10), definite, NOT key-sorted
                "66666e5f6b6579",
                "63664031", // "fn_key": "f@1"
                "66696e70757473",
                "816161", // "inputs": ["a"]
                "6463656c6c",
                "6163", // "cell": "c"
                "6462616e64",
                "6162", // "band": "b"
                "6c74736c6f745f77696e646f77",
                "820a14", // "tslot_window": [10, 20]
                "626f70",
                "6564656c7461", // "op": "delta"
                "6576616c7565",
                "fb3fc1eb851eb851ec", // "value": f64 0.14
                "6a636f6e666964656e6365",
                "fa3f666666", // "confidence": f32 0.9
                "7070726f76656e616e63655f636c617373",
                "6c6d6f64656c5f6f7574707574",
                "68636f64655f636964",
                "f6", // "code_cid": null
            ),
            "the derive body's CBOR wire form changed; every previously issued \
             caller signature just became unverifiable"
        );
    }

    /// The outer map is in declaration order, which is NOT what RFC 8949
    /// §4.2.1 key sorting would produce. Callers reaching for a generic
    /// canonical-CBOR encoder get a wrong digest, so the distinction is
    /// documented and pinned rather than left to be discovered.
    #[test]
    fn outer_map_is_declaration_ordered_not_key_sorted() {
        let bytes = emem_fact::cbor::to_canonical_cbor(&body()).unwrap();
        let fn_key_at = bytes
            .windows(6)
            .position(|w| w == b"fn_key")
            .expect("fn_key present");
        let band_at = bytes
            .windows(4)
            .position(|w| w == b"band")
            .expect("band present");
        assert!(
            fn_key_at < band_at,
            "fn_key must precede band (declaration order); a key-sorted map would put `band` first"
        );
    }

    /// Every semantic field enters the hash, so no part of a derivation
    /// can be swapped after signing.
    #[test]
    fn every_field_moves_the_body_hash() {
        let base = body().body_hash().unwrap();

        let mut b = body();
        b.fn_key = "other@1".into();
        assert_ne!(base, b.body_hash().unwrap(), "fn_key");

        let mut b = body();
        b.inputs.push("emem:fact:defi.zb4d9.pefa.zf619:bbbb".into());
        assert_ne!(base, b.body_hash().unwrap(), "inputs");

        let mut b = body();
        b.cell = "damO.zb000.xUti.zde78".into();
        assert_ne!(base, b.body_hash().unwrap(), "cell");

        let mut b = body();
        b.band = "indices.evi".into();
        assert_ne!(base, b.body_hash().unwrap(), "band");

        let mut b = body();
        b.tslot_window = [10, 21];
        assert_ne!(base, b.body_hash().unwrap(), "tslot_window");

        let mut b = body();
        b.op = "mean".into();
        assert_ne!(base, b.body_hash().unwrap(), "op");

        let mut b = body();
        b.value = ciborium::Value::Float(0.26);
        assert_ne!(base, b.body_hash().unwrap(), "value");

        let mut b = body();
        b.confidence = 0.8;
        assert_ne!(base, b.body_hash().unwrap(), "confidence");

        let mut b = body();
        b.provenance_class = "human_curated".into();
        assert_ne!(base, b.body_hash().unwrap(), "provenance_class");

        let mut b = body();
        b.code_cid = Some("deadbeef".into());
        assert_ne!(base, b.body_hash().unwrap(), "code_cid");
    }

    /// Input ORDER is signed: `delta(a, b)` and `delta(b, a)` are
    /// different derivations and must not share a signature.
    #[test]
    fn input_order_is_bound() {
        let mut a = body();
        a.inputs = vec!["emem:fact:c:one".into(), "emem:fact:c:two".into()];
        let mut b = body();
        b.inputs = vec!["emem:fact:c:two".into(), "emem:fact:c:one".into()];
        assert_ne!(a.body_hash().unwrap(), b.body_hash().unwrap());
    }

    /// The whole point of the reuse: the derive preimage verifies through
    /// the SAME `verify_attester` the memory-write path uses.
    #[test]
    fn derive_signature_verifies_through_the_shared_verifier() {
        let sk = fresh_signer();
        let b = body();
        let bh = b.body_hash().unwrap();
        let att = MemoryAttester {
            pubkey_b32: b32(sk.verifying_key().as_bytes()),
            sig_b32: b32(&sk.sign(&b.preimage().unwrap()).to_bytes()),
        };
        assert_eq!(
            verify_attester(DERIVE_VERB, DERIVE_PATH, &bh, &att),
            AttestationVerdict::Ok
        );
    }

    /// A signature minted for a memory-file `create` must not register a
    /// derivation, and a derive signature must not write a file. The verb
    /// is what keeps the two domains apart.
    #[test]
    fn derive_and_memory_write_signatures_do_not_cross() {
        let sk = fresh_signer();
        let b = body();
        let bh = b.body_hash().unwrap();

        // A derive signature, replayed as a file create at the same hash.
        let derive_att = MemoryAttester {
            pubkey_b32: b32(sk.verifying_key().as_bytes()),
            sig_b32: b32(&sk.sign(&b.preimage().unwrap()).to_bytes()),
        };
        assert_eq!(
            verify_attester("create", DERIVE_PATH, &bh, &derive_att),
            AttestationVerdict::BadSignature
        );

        // A create signature, replayed as a derivation.
        let create_att = MemoryAttester {
            pubkey_b32: b32(sk.verifying_key().as_bytes()),
            sig_b32: b32(&sk
                .sign(&crate::memory_acl::attester_preimage(
                    "create",
                    DERIVE_PATH,
                    &bh,
                ))
                .to_bytes()),
        };
        assert_eq!(
            verify_attester(DERIVE_VERB, DERIVE_PATH, &bh, &create_att),
            AttestationVerdict::BadSignature
        );
    }

    #[test]
    fn caller_classes_exclude_the_responder_only_ones() {
        assert!(is_caller_provenance_class("model_output"));
        assert!(is_caller_provenance_class("human_curated"));
        assert!(!is_caller_provenance_class("direct_sensor"));
        assert!(!is_caller_provenance_class("deterministic_index"));
        assert!(!is_caller_provenance_class("unclassified"));
    }
}
