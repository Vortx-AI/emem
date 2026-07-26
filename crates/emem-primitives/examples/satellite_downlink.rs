//! The satellite-operator loop, end to end, offline: enroll a
//! constellation key, capture a downlink window's OS trace, write the
//! observation through the trace gate, and walk away holding three
//! citeable handles: the `emem:fact:` token, the `emem:trace:` token,
//! and the `emem:bundle:` handle over the batch.
//!
//! This is the self-hosted (embedded) path a manufacturer can run
//! today with the crates alone; the hosted path is the same loop over
//! `POST /v1/attest` once the REST surface for traces lands. Run it:
//!
//! ```bash
//! cargo run -p emem-primitives --example satellite_downlink
//! ```

use std::sync::Arc;

use blake3::Hasher;
use ed25519_dalek::{Signer, SigningKey};

use emem_core::key::{AttesterKey, KeyEpoch, Signature};
use emem_core::substrates::TraceLayerKind;
use emem_fact::{Attestation, Derivation, Fact, PrimaryFact, RegistryCid, SchemaCid, Source};
use emem_primitives::memory_bundle::{bundle_token, compute_bundle_cid, BundleCitation};
use emem_storage::MaterializingStorage;
use emem_trace::{payload_digest_of_value, DeviceIdentity, EmittedOutput, OsTrace, TraceSegment};

const PROFILE: &str = "orbital.satellite.v1";

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ── 1. The operator's storage (self-hosted). ────────────────────
    let storage = MaterializingStorage::ephemeral(
        Arc::new(emem_core::bands::DEFAULT.clone()),
        Arc::new(emem_core::FunctionRegistry::parse_default()?),
        Arc::new(emem_core::SourceRegistry::parse_default()?),
    )?;
    let gate = storage.trace_gate.clone().expect("trace gate");

    // ── 2. One key per spacecraft; enroll it under the profile. ─────
    let mut secret = [0u8; 32];
    secret[..7].copy_from_slice(b"SAT-042");
    let sk = SigningKey::from_bytes(&secret);
    let pubkey = sk.verifying_key().to_bytes();
    let pubkey_b32 = data_encoding::BASE32_NOPAD.encode(&pubkey).to_lowercase();
    gate.enroll(&pubkey_b32, PROFILE)?;
    println!("enrolled {pubkey_b32} under {PROFILE}");

    // ── 3. The downlink observation the operator wants admitted. ────
    let cell = "damO.zb000.xUti.zde78";
    let value = ciborium::Value::Float(0.6431);
    let payload_digest = payload_digest_of_value(&value)?;
    let fact = Fact::Primary(PrimaryFact {
        cell: cell.into(),
        band: "indices.ndvi".into(),
        tslot: 12,
        value,
        unit: None,
        confidence: 1.0,
        uncertainty: None,
        sources: vec![Source {
            scheme: "operator.downlink".into(),
            id: "SAT-042/pass-1881".into(),
            cid: None,
            hash: None,
            captured_at: None,
            url: None,
        }],
        derivation: Derivation {
            fn_key: "test@1".into(),
            args: None,
        },
        privacy_class: "public".into(),
        schema_cid: SchemaCid::new("test-schema"),
        signer: AttesterKey(pubkey),
        signed_at: "2026-07-26T00:00:00Z".into(),
        served_via: None,
    });

    // ── 4. The capture window's OS trace, payload bound inside. ─────
    let required = [
        TraceLayerKind::Syscall,
        TraceLayerKind::Scheduler,
        TraceLayerKind::Memory,
        TraceLayerKind::SensorBus,
        TraceLayerKind::Signal,
        TraceLayerKind::Energy,
        TraceLayerKind::Thermal,
        TraceLayerKind::Storage,
    ];
    let segments: Vec<TraceSegment> = required
        .iter()
        .enumerate()
        .map(|(i, layer)| {
            let raw = format!("ctf segment {i} of pass 1881");
            TraceSegment {
                layer: *layer,
                seq: 0,
                clock_start_ns: 1_000 + i as u64,
                clock_end_ns: 90_000,
                event_count: 512,
                log_digest: data_encoding::BASE32_NOPAD
                    .encode(blake3::hash(raw.as_bytes()).as_bytes())
                    .to_lowercase(),
                prev_digest: None,
                encoding: "zephyr.ctf.v1".into(),
            }
        })
        .collect();
    let trace = OsTrace::build_and_sign_v1(
        DeviceIdentity {
            device_key: AttesterKey(pubkey),
            key_epoch: KeyEpoch(0),
            substrate_profile: PROFILE.into(),
            platform: "payload-computer-a53".into(),
            os: "zephyr-3.7".into(),
            kernel: "zephyr-3.7.0".into(),
            boot_id: "pass-1881-boot".into(),
        },
        1_000,
        100_000,
        segments,
        vec![EmittedOutput {
            payload_digest: payload_digest.clone(),
            band: Some("indices.ndvi".into()),
            emitted_at_ns: 88_000,
            layer: TraceLayerKind::SensorBus,
        }],
        &sk,
    )?;

    // ── 5. Sign the attestation envelope and write through the gate. ─
    let att = sign_attestation(vec![fact], &sk);
    let (cids, admitted) = storage.put_attestation_gated(&att, Some(&trace)).await?;
    let admitted = admitted.expect("gated admission");

    // ── 6. The three handles the operator keeps. ────────────────────
    let fact_token = format!("emem:fact:{cell}:{}", cids[0].as_str());
    let citations = vec![BundleCitation {
        cell: cell.into(),
        band: "indices.ndvi".into(),
        resolved_tslot: 12,
        fact_cid: Some(cids[0].as_str().to_string()),
        miss_reason: None,
        memory_token: Some(fact_token.clone()),
    }];
    let bundle_cid = compute_bundle_cid(&citations, Some("pass 1881 downlink"));

    println!("fact token   {fact_token}");
    println!("trace token  {}", admitted.token);
    println!("bundle token {}", bundle_token(&bundle_cid));

    // ── 7. Anyone can re-check: resolve the trace, re-verify. ───────
    let stored = gate.get_trace(&admitted.trace_cid).expect("stored trace");
    let profile = emem_core::substrates::DEFAULT
        .lookup(PROFILE)
        .expect("profile");
    let report = emem_trace::verify_os_trace(&stored, profile, Some(&payload_digest));
    println!(
        "re-verified  {:?} (reasons: {})",
        report.verdict,
        report.reasons.len()
    );
    assert_eq!(report.verdict, emem_trace::Verdict::Admit);
    assert_eq!(
        gate.trace_for_fact(&cids[0]).as_deref(),
        Some(admitted.trace_cid.as_str())
    );
    Ok(())
}

/// Sign the v0 attestation envelope the storage tests use; a real
/// operator uses `Attestation::build_and_sign_v1`.
fn sign_attestation(facts: Vec<Fact>, sk: &SigningKey) -> Attestation {
    let registry_cid = "test-registry";
    let schema_cid = "test-schema";
    let mut leaves: Vec<[u8; 32]> = facts
        .iter()
        .map(|f| {
            let mut buf = Vec::new();
            ciborium::ser::into_writer(f, &mut buf).expect("cbor");
            *blake3::hash(&buf).as_bytes()
        })
        .collect();
    leaves.sort();
    let root = emem_attest::merkle_root(&leaves);
    let mut h = Hasher::new();
    h.update(&root);
    h.update(registry_cid.as_bytes());
    h.update(schema_cid.as_bytes());
    let sig = sk.sign(h.finalize().as_bytes());
    let mut sig_bytes = [0u8; 64];
    sig_bytes.copy_from_slice(&sig.to_bytes());
    Attestation {
        facts,
        edges: vec![],
        batch_root: root,
        attester: AttesterKey(sk.verifying_key().to_bytes()),
        attester_key_epoch: KeyEpoch(0),
        registry_cid: RegistryCid::new(registry_cid),
        schema_cid: SchemaCid::new(schema_cid),
        signature: Signature(sig_bytes),
        attested_at: "2026-07-26T00:00:00Z".into(),
        scope: None,
        preimage_version: 0,
    }
}
