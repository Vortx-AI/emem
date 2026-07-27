//! One satellite pass, end to end: the full operator loop a
//! manufacturer runs to make its constellation a member of the
//! multi-agent system on the ground.
//!
//! SAT-042 images three cells of Nile Delta cropland on pass 1881.
//! The story has five acts, and the refusals are the point as much as
//! the admission:
//!
//!   1. Enroll the spacecraft key under `orbital.satellite.v1`; the
//!      registry, not this example, dictates what must be captured.
//!   2. Try to write without a trace: refused. Respected, not believed.
//!   3. Capture the pass window, bind the three downlink payloads,
//!      then try to sneak in a fourth fact the execution never
//!      emitted: refused, by name.
//!   4. Write the honest batch: three signed facts, one admitted
//!      trace, and the handles that survive any context window:
//!      `emem:fact:`, `emem:trace:`, `emem:bundle:`.
//!   5. Score the claims against the open-archive drift anchor, then
//!      tamper with the stored trace and watch the verifier name the
//!      cut.
//!
//! Everything runs offline and deterministically (fixed key, fixed
//! clocks), which is what makes it CI-runnable proof rather than a
//! demo that needs someone's credentials:
//!
//! ```bash
//! cargo run -p emem-primitives --example satellite_downlink
//! ```

use std::sync::Arc;

use ed25519_dalek::SigningKey;

use emem_core::key::{AttesterKey, KeyEpoch};
use emem_core::substrates::{TraceLayerKind, DEFAULT as SUBSTRATES};
use emem_fact::{Attestation, Derivation, Fact, PrimaryFact, RegistryCid, SchemaCid, Source};
use emem_primitives::memory_bundle::{bundle_token, compute_bundle_cid, BundleCitation};
use emem_storage::{MaterializingStorage, Storage};
use emem_trace::{
    payload_digest_of_value, verify_os_trace, DeviceIdentity, DriftAnchorCheck, EmittedOutput,
    OsTrace, TraceSegment,
};

const PROFILE: &str = "orbital.satellite.v1";
const BAND: &str = "indices.ndvi";
const TSLOT: u64 = 12;

/// Three along-track ground points in Nile Delta cropland, with the
/// NDVI the payload computer measured at each. Real coordinates; the
/// cell64 addresses are derived, not invented.
const PASS: [(f64, f64, f64); 3] = [
    (30.7901, 31.0007, 0.6431),
    (30.7899, 31.0021, 0.6512),
    (30.7897, 31.0035, 0.2103),
];

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let storage = MaterializingStorage::ephemeral(
        Arc::new(emem_core::bands::DEFAULT.clone()),
        Arc::new(emem_core::FunctionRegistry::parse_default()?),
        Arc::new(emem_core::SourceRegistry::parse_default()?),
    )?;
    let gate = storage.trace_gate.clone().expect("trace gate");

    // Real content-addressed registry pins, not placeholder strings:
    // the attestation cites the exact band ontology and schema bundle
    // in force when it was signed.
    let registry_cid = RegistryCid::new(emem_core::manifest_cid(&*emem_core::bands::DEFAULT)?);
    let schema_cid = SchemaCid::new(emem_core::manifest_cid(&*emem_core::schema::DEFAULT)?);

    // ── Act 1: enroll. The registry dictates the contract. ──────────
    let mut secret = [0u8; 32];
    secret[..7].copy_from_slice(b"SAT-042");
    let sk = SigningKey::from_bytes(&secret);
    let pubkey = sk.verifying_key().to_bytes();
    let pubkey_b32 = data_encoding::BASE32_NOPAD.encode(&pubkey).to_lowercase();

    let profile = SUBSTRATES.lookup(PROFILE).expect("profile in manifest");
    gate.enroll(&pubkey_b32, PROFILE)?;
    println!("act 1  enrolled spacecraft key {pubkey_b32}");
    println!(
        "       profile {PROFILE} requires {} trace layers:",
        profile.required_trace_layers.len()
    );
    println!("       {:?}", profile.required_trace_layers);

    // The three observations, at real cell64 addresses.
    let cells: Vec<String> = PASS
        .iter()
        .map(|(lat, lng, _)| emem_codec::cell64_from_latlng(*lat, *lng))
        .collect();
    let facts: Vec<Fact> = PASS
        .iter()
        .zip(&cells)
        .map(|((_, _, ndvi), cell)| mk_fact(cell, *ndvi, pubkey, &schema_cid))
        .collect();
    let payloads: Vec<String> = facts
        .iter()
        .map(|f| match f {
            Fact::Primary(p) => payload_digest_of_value(&p.value).expect("digest"),
            _ => unreachable!(),
        })
        .collect();
    let att = Attestation::build_and_sign_v1(
        facts.clone(),
        vec![],
        registry_cid.clone(),
        schema_cid.clone(),
        &sk,
        KeyEpoch(0),
        "2026-07-26T09:41:00Z".into(),
        None,
    )?;

    // ── Act 2: no trace, no entry. ──────────────────────────────────
    let refused = storage.put_attestation_gated(&att, None).await;
    println!("\nact 2  write without a trace:");
    println!("       {}", refused.expect_err("must refuse"));

    // ── Act 3: capture the pass; a smuggled fact is named. ──────────
    // One segment per required layer, digest-chained by the builder.
    // In production these digests come from the real capture files
    // (ftrace/eBPF on Linux payloads, CTF on RTOS flight software).
    let segments: Vec<TraceSegment> = profile
        .required_trace_layers
        .iter()
        .enumerate()
        .map(|(i, layer)| segment(*layer, i, &format!("pass-1881 {layer:?} capture")))
        .collect();
    let outputs: Vec<EmittedOutput> = payloads
        .iter()
        .enumerate()
        .map(|(i, digest)| EmittedOutput {
            payload_digest: digest.clone(),
            band: Some(BAND.into()),
            emitted_at_ns: 82_000 + i as u64 * 1_000,
            layer: TraceLayerKind::SensorBus,
        })
        .collect();
    let trace = OsTrace::build_and_sign_v1(device(pubkey), 1_000, 100_000, segments, outputs, &sk)?;

    // A fourth fact whose payload the execution never emitted, hidden
    // inside an otherwise honest batch.
    let mut smuggled_facts = facts.clone();
    smuggled_facts.push(mk_fact(&cells[0], 0.9999, pubkey, &schema_cid));
    let smuggled = Attestation::build_and_sign_v1(
        smuggled_facts,
        vec![],
        registry_cid.clone(),
        schema_cid.clone(),
        &sk,
        KeyEpoch(0),
        "2026-07-26T09:41:00Z".into(),
        None,
    )?;
    let caught = storage.put_attestation_gated(&smuggled, Some(&trace)).await;
    println!("\nact 3  smuggle one unemitted fact into a sound batch:");
    println!("       {}", caught.expect_err("must catch"));

    // ── Act 4: the honest batch admits. ─────────────────────────────
    let (cids, admitted) = storage.put_attestation_gated(&att, Some(&trace)).await?;
    let admitted = admitted.expect("gated admission");
    println!(
        "\nact 4  admitted: {} facts under one verified trace",
        cids.len()
    );
    let mut citations = Vec::new();
    for (cell, cid) in cells.iter().zip(&cids) {
        let token = format!("emem:fact:{cell}:{}", cid.as_str());
        println!("       {token}");
        citations.push(BundleCitation {
            cell: cell.clone(),
            band: BAND.into(),
            resolved_tslot: TSLOT,
            fact_cid: Some(cid.as_str().to_string()),
            miss_reason: None,
            memory_token: Some(token),
        });
    }
    println!("       {}", admitted.token);
    let bundle_cid = compute_bundle_cid(&citations, Some("SAT-042 pass 1881, Nile Delta"));
    println!("       {}", bundle_token(&bundle_cid));

    // ── Act 5: the anchor scores the claims; a tamper is named. ─────
    // The anchor values below are what the open archive (Sentinel-2
    // L2A over the same delta cropland, July) reads at these cells;
    // live, an operator recalls them with:
    //   curl -s -X POST https://emem.dev/v1/recall -H 'content-type: application/json' \
    //     -d '{"place":"Nile Delta","bands":["indices.ndvi"]}'
    // The third claim (0.21 against an anchor of 0.63) is the
    // interesting one: nine-plus sigma out, so it lands as
    // contradicted. If the field really was just harvested, the next
    // archive pass moves the anchor and the tension resolves toward
    // the device; the score surfaces the disagreement instead of
    // silently deciding who is right.
    println!("\nact 5  drift-anchor scoring against the open archive:");
    let anchor = (0.6402, 0.02);
    for ((_, _, claimed), cell) in PASS.iter().zip(&cells) {
        let check = DriftAnchorCheck::new(BAND, *claimed, anchor.0, anchor.1);
        println!(
            "       {cell}  device {claimed:.4} vs anchor {:.4}  score {:.2}  {:?}",
            anchor.0, check.score, check.verdict
        );
    }

    // Resolve the stored trace and re-verify it, then rewrite one
    // captured log and watch the verifier name the cut.
    let stored = gate.get_trace(&admitted.trace_cid).expect("stored trace");
    assert_eq!(
        verify_os_trace(&stored, profile, Some(&payloads[0])).verdict,
        emem_trace::Verdict::Admit
    );
    let mut tampered = stored.clone();
    tampered.segments[2].log_digest =
        payload_digest_of_value(&ciborium::Value::Text("scrubbed after the fact".into()))?;
    let report = verify_os_trace(&tampered, profile, Some(&payloads[0]));
    println!("\n       tampered segment 2 after signing; the verifier answers:");
    for reason in &report.reasons {
        println!("       reject: {reason}");
    }
    assert_eq!(report.verdict, emem_trace::Verdict::Reject);
    assert_eq!(
        gate.trace_for_fact(&cids[0]).as_deref(),
        Some(admitted.trace_cid.as_str()),
        "every admitted fact keeps its audit edge to the execution that produced it"
    );
    println!("\ndone   three facts, one trace, one bundle, and every forgery named.");
    Ok(())
}

fn device(pubkey: [u8; 32]) -> DeviceIdentity {
    DeviceIdentity {
        device_key: AttesterKey(pubkey),
        key_epoch: KeyEpoch(0),
        substrate_profile: PROFILE.into(),
        platform: "jetson-orin-nx".into(),
        os: "jetpack-6.0 (ubuntu-22.04)".into(),
        kernel: "5.15.148-tegra".into(),
        boot_id: "pass-1881-boot".into(),
    }
}

/// A registered capture encoding that can produce `layer`. A real payload
/// trace is multi-source: the kernel tracer for syscalls/scheduling/memory
/// and storage, the sensor-bus recorder for the instrument and RF path,
/// and the hardware monitor for power and temperature.
fn encoding_for(layer: TraceLayerKind) -> &'static str {
    match layer {
        TraceLayerKind::SensorBus | TraceLayerKind::Signal => "ros2.bag.v2",
        TraceLayerKind::Energy | TraceLayerKind::Thermal => "linux.hwmon.v1",
        TraceLayerKind::Inference => "nvidia.nsys.v1",
        _ => "linux.ftrace.v1",
    }
}

fn segment(layer: TraceLayerKind, i: usize, raw: &str) -> TraceSegment {
    TraceSegment {
        layer,
        seq: 0,
        clock_start_ns: 1_000 + i as u64,
        clock_end_ns: 90_000,
        event_count: 512,
        log_digest: data_encoding::BASE32_NOPAD
            .encode(blake3::hash(raw.as_bytes()).as_bytes())
            .to_lowercase(),
        prev_digest: None,
        encoding: encoding_for(layer).into(),
    }
}

fn mk_fact(cell: &str, ndvi: f64, pubkey: [u8; 32], schema_cid: &SchemaCid) -> Fact {
    Fact::Primary(PrimaryFact {
        cell: cell.into(),
        band: BAND.into(),
        tslot: TSLOT,
        value: ciborium::Value::Float(ndvi),
        unit: None,
        confidence: 0.98,
        uncertainty: None,
        sources: vec![Source {
            scheme: "operator.downlink".into(),
            id: "SAT-042/pass-1881".into(),
            cid: None,
            hash: None,
            captured_at: Some("2026-07-26T09:38:12Z".into()),
            url: None,
        }],
        derivation: Derivation {
            fn_key: "ndvi@1".into(),
            args: None,
        },
        privacy_class: "public".into(),
        schema_cid: schema_cid.clone(),
        signer: AttesterKey(pubkey),
        signed_at: "2026-07-26T09:41:00Z".into(),
        served_via: None,
    })
}
