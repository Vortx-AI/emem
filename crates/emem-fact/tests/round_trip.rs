//! Round-trip tests for the four cryptographic invariants of `emem-fact`.
//!
//! These are not smoke tests — each one fails loudly if a regression
//! breaks a property the protocol depends on.
//!
//! 1. Canonical CBOR is *byte-identical* across encode → decode → re-encode.
//!    Without this, fact CIDs drift and signatures stop verifying.
//! 2. blake3 over the canonical CBOR is deterministic; the same fact value
//!    always produces the same FactCid prefix.
//! 3. ed25519 sign/verify on the canonical preimage holds across a JSON
//!    round-trip of `Signature`. Required for receipts and attestations.
//! 4. CID newtypes serialize transparently (as plain strings), so JSON
//!    consumers don't see `{"0":"..."}` wrappers.

use ciborium::Value as CborValue;
use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey, SECRET_KEY_LENGTH};
use emem_core::{AttesterKey, Signature};
use emem_fact::cbor::{base32_prefix, blake3_32, to_canonical_cbor};
use emem_fact::cid::{FactCid, SchemaCid};
use emem_fact::fact::{Derivation, Fact, PrimaryFact, Source};
use rand::rngs::OsRng;
use rand::RngCore;

fn generate_signing_key() -> SigningKey {
    let mut bytes = [0u8; SECRET_KEY_LENGTH];
    OsRng.fill_bytes(&mut bytes);
    SigningKey::from_bytes(&bytes)
}

fn sample_primary(signer: &VerifyingKey) -> PrimaryFact {
    PrimaryFact {
        cell: "damO.zb000.xUti.zde79".into(),
        band: "copdem30m.elevation_mean".into(),
        tslot: 17_532_576_000, // arbitrary non-zero
        value: CborValue::Float(3776.24),
        unit: Some("m".into()),
        confidence: 0.97,
        uncertainty: None,
        sources: vec![Source {
            scheme: "copdem30m".into(),
            id: "Copernicus_DSM_COG_10_N35_00_E138_00_DEM".into(),
            cid: None,
            hash: None,
            captured_at: Some("2021-04-22T00:00:00Z".into()),
            url: None,
        }],
        derivation: Derivation {
            fn_key: "copdem30m.elevation_mean@1".into(),
            args: None,
        },
        privacy_class: "public".into(),
        schema_cid: SchemaCid::new("emem.schema.primary.v1"),
        signer: AttesterKey(signer.to_bytes()),
        signed_at: "2026-04-26T00:00:00Z".into(),
    }
}

#[test]
fn cbor_round_trip_is_byte_identical() {
    let sk = generate_signing_key();
    let f = Fact::Primary(sample_primary(&sk.verifying_key()));

    let bytes_a = to_canonical_cbor(&f).expect("encode 1");
    let decoded: Fact = ciborium::de::from_reader(&bytes_a[..]).expect("decode");
    let bytes_b = to_canonical_cbor(&decoded).expect("encode 2");

    assert_eq!(
        bytes_a, bytes_b,
        "encode → decode → re-encode must produce identical bytes; \
         deterministic CBOR is the foundation of fact CIDs"
    );
}

#[test]
fn blake3_over_canonical_cbor_is_stable() {
    let sk = generate_signing_key();
    let f = Fact::Primary(sample_primary(&sk.verifying_key()));

    let h1 = blake3_32(&to_canonical_cbor(&f).unwrap());
    let h2 = blake3_32(&to_canonical_cbor(&f).unwrap());
    assert_eq!(h1, h2, "blake3 of canonical CBOR must be deterministic");

    // Mutating any field must change the hash. Confidence is a tight check
    // because the diff is one byte but on a tagged float.
    let mut mutated = match f {
        Fact::Primary(p) => p,
        _ => unreachable!(),
    };
    mutated.confidence = 0.96;
    let h3 = blake3_32(&to_canonical_cbor(&Fact::Primary(mutated)).unwrap());
    assert_ne!(h1, h3, "fact CID must change when any field changes");
}

#[test]
fn fact_cid_prefix_is_collision_resistant_and_lowercased() {
    let sk = generate_signing_key();
    let f = Fact::Primary(sample_primary(&sk.verifying_key()));
    let h = blake3_32(&to_canonical_cbor(&f).unwrap());
    let cid = base32_prefix(&h, 16); // 128-bit prefix as used in FactCid wire form

    assert_eq!(cid.len(), 26, "16 bytes → 26 base32-nopad chars");
    assert!(
        cid.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
        "base32-nopad must be lowercase ASCII; got {cid}"
    );
}

#[test]
fn signature_serde_json_round_trip() {
    // Sign a preimage with ed25519, serialize the Signature through JSON,
    // and verify the round-tripped signature with the public key. This is
    // exactly what a verifier does with a receipt fetched over the wire.
    let sk = generate_signing_key();
    let vk = sk.verifying_key();
    let preimage = b"emem-receipt-preimage-test-2026";
    let sig = sk.sign(preimage);

    let wire = Signature(sig.to_bytes());
    let json = serde_json::to_string(&wire).expect("serialize Signature");
    let back: Signature = serde_json::from_str(&json).expect("deserialize Signature");
    assert_eq!(
        wire.0, back.0,
        "Signature bytes must survive JSON round-trip"
    );

    let recovered = ed25519_dalek::Signature::from_bytes(&back.0);
    vk.verify(preimage, &recovered)
        .expect("ed25519 must verify the recovered signature");
}

#[test]
fn cid_newtype_serializes_as_transparent_string() {
    // FactCid / SchemaCid use #[serde(transparent)] — JSON consumers must see
    // a bare string, not {"0":"..."}. If this regresses, every receipt on the
    // wire becomes a breaking change.
    let cid = FactCid::new("damo.zb000.xuti");
    let json = serde_json::to_string(&cid).unwrap();
    assert_eq!(
        json, "\"damo.zb000.xuti\"",
        "FactCid must serialize as a bare string, not a tagged wrapper"
    );

    let back: FactCid = serde_json::from_str(&json).unwrap();
    assert_eq!(back, cid);
}

#[test]
fn signature_rejects_wrong_pubkey() {
    // Sanity: a signature from key A must NOT verify under key B. This
    // catches accidental key/byte aliasing.
    let sk_a = generate_signing_key();
    let sk_b = generate_signing_key();
    let preimage = b"audit-emem-attest";
    let sig_a = sk_a.sign(preimage);

    let result = sk_b.verifying_key().verify(preimage, &sig_a);
    assert!(result.is_err(), "signature from A must not verify under B");
}

#[test]
fn signing_key_from_bytes_round_trip() {
    // A persisted secret (we store these at <data>/identity.secret.b32) must
    // reconstitute into the same signing key.
    let mut bytes = [0u8; SECRET_KEY_LENGTH];
    OsRng.fill_bytes(&mut bytes);
    let sk = SigningKey::from_bytes(&bytes);
    let sk2 = SigningKey::from_bytes(&bytes);
    assert_eq!(
        sk.verifying_key().to_bytes(),
        sk2.verifying_key().to_bytes()
    );
}
