//! End-to-end: build a robot's trace, verify it admits, then tamper
//! with every part of the evidence and check each forgery is named.

use ed25519_dalek::SigningKey;

use emem_core::key::{AttesterKey, KeyEpoch};
use emem_core::substrates::{AdmissionRule, SubstrateRegistry, TraceLayerKind, DEFAULT};
use emem_trace::{
    verify_os_trace, DeviceIdentity, EmittedOutput, OsTrace, RejectReason, TraceSegment, Verdict,
};

fn signing_key() -> SigningKey {
    let mut bytes = [0u8; 32];
    bytes[0] = 42;
    SigningKey::from_bytes(&bytes)
}

fn device(sk: &SigningKey, profile: &str) -> DeviceIdentity {
    DeviceIdentity {
        device_key: AttesterKey(sk.verifying_key().to_bytes()),
        key_epoch: KeyEpoch(0),
        substrate_profile: profile.to_string(),
        platform: "jetson-orin-nx".into(),
        os: "ubuntu-24.04".into(),
        kernel: "6.8.0-tegra".into(),
        boot_id: "b7c1e2d3".into(),
    }
}

fn segment(layer: TraceLayerKind, start: u64, end: u64, raw: &[u8]) -> TraceSegment {
    TraceSegment {
        layer,
        seq: 0,
        clock_start_ns: start,
        clock_end_ns: end,
        event_count: raw.len() as u64,
        log_digest: data_digest(raw),
        prev_digest: None,
        encoding: "linux.ftrace.v1".into(),
    }
}

fn data_digest(raw: &[u8]) -> String {
    use data_encoding::BASE32_NOPAD;
    BASE32_NOPAD
        .encode(blake3::hash(raw).as_bytes())
        .to_lowercase()
}

/// A full trace covering every layer robot.fleet.v1 requires, with one
/// emitted lidar payload.
fn robot_trace(sk: &SigningKey) -> (OsTrace, String) {
    let payload = data_digest(b"lidar frame 881");
    let layers = [
        TraceLayerKind::Syscall,
        TraceLayerKind::Scheduler,
        TraceLayerKind::Memory,
        TraceLayerKind::SensorBus,
        TraceLayerKind::Energy,
        TraceLayerKind::Thermal,
        TraceLayerKind::Inference,
    ];
    let segments: Vec<TraceSegment> = layers
        .iter()
        .enumerate()
        .map(|(i, l)| segment(*l, 1_000 + i as u64, 9_000, format!("log {i}").as_bytes()))
        .collect();
    let outputs = vec![EmittedOutput {
        payload_digest: payload.clone(),
        band: Some("robot.lidar_occupancy".into()),
        emitted_at_ns: 8_500,
        layer: TraceLayerKind::SensorBus,
    }];
    let trace = OsTrace::build_and_sign_v1(
        device(sk, "robot.fleet.v1"),
        1_000,
        10_000,
        segments,
        outputs,
        sk,
    )
    .expect("build");
    (trace, payload)
}

#[test]
fn sound_trace_admits_and_cid_is_stable() {
    let sk = signing_key();
    let (trace, payload) = robot_trace(&sk);
    let profile = DEFAULT.lookup("robot.fleet.v1").expect("profile");
    let report = verify_os_trace(&trace, profile, Some(&payload));
    assert_eq!(report.verdict, Verdict::Admit, "{:?}", report.reasons);
    assert!(report.coverage.missing.is_empty());

    // The record round-trips through canonical CBOR to the same CID.
    let mut buf = Vec::new();
    ciborium::ser::into_writer(&trace, &mut buf).expect("encode");
    let back: OsTrace = ciborium::de::from_reader(buf.as_slice()).expect("decode");
    assert_eq!(trace.trace_cid().unwrap(), back.trace_cid().unwrap());
    let report2 = verify_os_trace(&back, profile, Some(&payload));
    assert_eq!(report2.verdict, Verdict::Admit);
}

#[test]
fn rewritten_segment_is_caught_at_every_escalation() {
    let sk = signing_key();
    let profile = DEFAULT.lookup("robot.fleet.v1").expect("profile");

    // Step 1: edit one captured log after signing. The next segment's
    // prev_digest no longer matches: chain broken.
    let (mut trace, payload) = robot_trace(&sk);
    trace.segments[2].log_digest = data_digest(b"scrubbed log");
    let report = verify_os_trace(&trace, profile, Some(&payload));
    assert_eq!(report.verdict, Verdict::Reject);
    assert!(report
        .reasons
        .iter()
        .any(|r| matches!(r, RejectReason::ChainBroken { seq: 3 })));

    // Step 2: the forger also relinks the chain. Now the chain is
    // internally consistent but its root no longer matches the signed
    // trace_root.
    let mut prev: Option<String> = None;
    for seg in &mut trace.segments {
        seg.prev_digest = prev.take();
        prev = Some(data_digest_of_segment(seg));
    }
    let report = verify_os_trace(&trace, profile, Some(&payload));
    assert_eq!(report.verdict, Verdict::Reject);
    assert!(report
        .reasons
        .iter()
        .any(|r| matches!(r, RejectReason::RootMismatch)));

    // Step 3: the forger also rewrites trace_root. Now the device
    // signature no longer covers the record.
    use data_encoding::BASE32_NOPAD;
    let digests: Vec<[u8; 32]> = trace
        .segments
        .iter()
        .map(|s| s.digest().expect("digest"))
        .collect();
    let root = emem_attest::merkle_root_v1(&digests);
    trace.trace_root = BASE32_NOPAD.encode(&root).to_lowercase();
    let report = verify_os_trace(&trace, profile, Some(&payload));
    assert_eq!(report.verdict, Verdict::Reject);
    assert!(report
        .reasons
        .iter()
        .any(|r| matches!(r, RejectReason::SignatureInvalid)));
}

fn data_digest_of_segment(seg: &TraceSegment) -> String {
    use data_encoding::BASE32_NOPAD;
    BASE32_NOPAD
        .encode(&seg.digest().expect("digest"))
        .to_lowercase()
}

#[test]
fn dropped_layer_is_missing_coverage() {
    let sk = signing_key();
    let (mut trace, _) = robot_trace(&sk);
    // Remove the energy layer, relink honestly, re-sign honestly: the
    // trace is internally consistent but incomplete for the profile.
    let mut segments = trace.segments.clone();
    segments.retain(|s| s.layer != TraceLayerKind::Energy);
    for s in &mut segments {
        s.seq = 0;
        s.prev_digest = None;
    }
    trace = OsTrace::build_and_sign_v1(
        trace.device.clone(),
        trace.window_start_ns,
        trace.window_end_ns,
        segments,
        trace.outputs.clone(),
        &sk,
    )
    .expect("rebuild");
    let profile = DEFAULT.lookup("robot.fleet.v1").expect("profile");
    let report = verify_os_trace(&trace, profile, None);
    assert_eq!(report.verdict, Verdict::Reject);
    assert_eq!(report.coverage.missing, vec![TraceLayerKind::Energy]);
}

#[test]
fn unbound_payload_is_rejected_even_with_a_sound_trace() {
    let sk = signing_key();
    let (trace, _) = robot_trace(&sk);
    let profile = DEFAULT.lookup("robot.fleet.v1").expect("profile");
    let forged = data_digest(b"payload the execution never emitted");
    let report = verify_os_trace(&trace, profile, Some(&forged));
    assert_eq!(report.verdict, Verdict::Reject);
    assert!(report
        .reasons
        .iter()
        .any(|r| matches!(r, RejectReason::OutputUnbound { .. })));
}

#[test]
fn archive_substrate_never_admits_device_output() {
    let sk = signing_key();
    let (mut trace, payload) = robot_trace(&sk);
    trace.device.substrate_profile = "earth.satellite.v0".into();
    let profile = DEFAULT.lookup("earth.satellite.v0").expect("profile");
    let report = verify_os_trace(&trace, profile, Some(&payload));
    assert_eq!(report.verdict, Verdict::Reject);
    assert!(report
        .reasons
        .iter()
        .any(|r| matches!(r, RejectReason::AdmissionNotTraceBased { .. })));
}

#[test]
fn wrong_key_cannot_speak_for_the_device() {
    let sk = signing_key();
    let (mut trace, payload) = robot_trace(&sk);
    let mut other = [0u8; 32];
    other[0] = 99;
    let other_sk = SigningKey::from_bytes(&other);
    // Re-sign the same evidence with a different key while still
    // claiming the original device identity.
    use ed25519_dalek::Signer;
    let preimage = trace.preimage().expect("preimage");
    trace.signature = emem_core::key::Signature(other_sk.sign(&preimage).to_bytes());
    let profile = DEFAULT.lookup("robot.fleet.v1").expect("profile");
    let report = verify_os_trace(&trace, profile, Some(&payload));
    assert_eq!(report.verdict, Verdict::Reject);
    assert!(report
        .reasons
        .iter()
        .any(|r| matches!(r, RejectReason::SignatureInvalid)));
}

#[test]
fn every_candidate_profile_in_the_registry_is_verifiable() {
    // The registry and the verifier agree on the layer vocabulary: a
    // trace covering exactly a profile's required layers admits.
    let sk = signing_key();
    let registry: &SubstrateRegistry = &DEFAULT;
    let mut trace_admitted = 0;
    let mut archive_admitted = 0;
    for profile in &registry.substrates {
        // Iterate on the PROPERTY, not on an id.
        //
        // This skipped `earth.satellite.v0` by name, which was the right
        // invariant written as an exemption and held only while exactly one
        // profile was archive-admitted. Adding a codebase at a commit, a table
        // at a schema version and a model at a checkpoint, all recomputable
        // and none of them device-borne, made the loop build a trace out of
        // zero required layers and panic on `build: Empty`.
        //
        // A profile admitted by re-fetchability has no execution trace to
        // verify, by design. Asserting that positively is better than skipping
        // it, because "no layers" is exactly what the registry's own
        // validation demands of an archive profile and a test that skipped
        // them would not notice if one grew some.
        if profile.admission != AdmissionRule::OsTraceRequired {
            archive_admitted += 1;
            assert!(
                profile.required_trace_layers.is_empty(),
                "{}: archive-admitted profiles must require no trace layers",
                profile.id
            );
            continue;
        }
        trace_admitted += 1;
        let segments: Vec<TraceSegment> = profile
            .required_trace_layers
            .iter()
            .enumerate()
            .map(|(i, l)| segment(*l, 1_000 + i as u64, 9_000, format!("{i}").as_bytes()))
            .collect();
        let trace = OsTrace::build_and_sign_v1(
            device(&sk, &profile.id),
            1_000,
            10_000,
            segments,
            vec![],
            &sk,
        )
        .expect("build");
        let report = verify_os_trace(&trace, profile, None);
        assert_eq!(
            report.verdict,
            Verdict::Admit,
            "{}: {:?}",
            profile.id,
            report.reasons
        );
    }
    // Both arms must actually run. If the registry ever drifts to all-one-kind
    // this test would keep passing while covering half of what it claims.
    assert!(
        trace_admitted > 0,
        "no trace-admitted profile was exercised"
    );
    assert!(
        archive_admitted > 0,
        "no archive-admitted profile was exercised"
    );
}

/// Every field of an emitted output is bound by the device signature.
///
/// It was not. Only `payload_digest` reached the preimage, so `band`, `layer`
/// and `emitted_at_ns` could be edited on a signed trace and the trace still
/// admitted. Found by mutating one field at a time and asking the verifier,
/// rather than by reading the preimage and assuming.
///
/// The two that matter beyond tidiness: `layer` separates a payload that came
/// off a sensor bus from one that came out of an inference pass, which is the
/// direct-sensor versus model-output distinction the provenance class rests
/// on; and `band` is the label the fact is filed under. An unsigned label on a
/// signed record invites exactly the trust it has not earned.
#[test]
fn every_field_of_an_emitted_output_is_signed() {
    let sk = signing_key();
    let (trace, _) = robot_trace(&sk);
    let registry = &*DEFAULT;
    let profile = registry.lookup("robot.fleet.v1").expect("profile");

    assert_eq!(
        verify_os_trace(&trace, profile, None).verdict,
        Verdict::Admit,
        "the control must admit, or the mutations below prove nothing"
    );
    assert!(
        !trace.outputs.is_empty(),
        "this test needs an output to mutate"
    );

    type Edit = (&'static str, fn(&mut EmittedOutput));
    let edits: Vec<Edit> = vec![
        ("band", |o| {
            o.band = Some(match o.band.as_deref() {
                Some("surface_water") => "ndvi".to_string(),
                _ => "surface_water".to_string(),
            })
        }),
        ("layer", |o| {
            o.layer = if o.layer == TraceLayerKind::Syscall {
                TraceLayerKind::Network
            } else {
                TraceLayerKind::Syscall
            }
        }),
        ("emitted_at_ns", |o| o.emitted_at_ns += 1),
        ("payload_digest", |o| {
            o.payload_digest = data_digest(b"a payload this device never emitted")
        }),
    ];

    for (field, edit) in edits {
        let mut forged = trace.clone();
        edit(&mut forged.outputs[0]);
        let report = verify_os_trace(&forged, profile, None);
        assert_eq!(
            report.verdict,
            Verdict::Reject,
            "editing output.{field} left the trace verifying"
        );
        assert!(
            report
                .reasons
                .iter()
                .any(|r| matches!(r, RejectReason::SignatureInvalid)),
            "editing output.{field} was caught, but not as a broken signature: {:?}",
            report.reasons
        );
    }
}

/// Verification cost must stay linear in segment count.
///
/// Ignored by default: it is a timing measurement, and a timing assertion on a
/// loaded CI box is a flaky assertion. Run it deliberately:
///
/// ```bash
/// cargo test -p emem-trace --release --test trace_round_trip -- --ignored --nocapture
/// ```
///
/// What it guards. The duplicate-segment check was `digests.contains(&d)`
/// inside the segment loop, which is quadratic, on an unauthenticated write
/// path with a 16 MB body limit. Release measurements before and after:
///
/// | segments | json    | before   | after  |
/// |----------|---------|----------|--------|
/// |    1,000 |  232 KB |     2 ms |   1 ms |
/// |    5,000 |  1.1 MB |    17 ms |   7 ms |
/// |   20,000 |  4.6 MB |   189 ms |  29 ms |
/// |   50,000 | 11.6 MB | 1,086 ms |  77 ms |
///
/// Roughly 50 KB of request bought a second of CPU. The ratio is what matters:
/// a tenfold rise in segments cost 543x before and 77x after.
#[test]
#[ignore = "timing measurement; run deliberately in release"]
fn verification_cost_is_linear_in_segment_count() {
    fn timed(n: usize) -> u128 {
        let sk = signing_key();
        let mut segments = Vec::with_capacity(n);
        for i in 0..n {
            let layer = match i % 3 {
                0 => TraceLayerKind::Syscall,
                1 => TraceLayerKind::Scheduler,
                _ => TraceLayerKind::Memory,
            };
            segments.push(segment(
                layer,
                i as u64,
                i as u64 + 1,
                format!("segment {i}").as_bytes(),
            ));
        }
        let trace = OsTrace::build_and_sign_v1(
            device(&sk, "exec.trace.v1"),
            0,
            n as u64 + 1,
            segments,
            vec![],
            &sk,
        )
        .expect("build");
        let registry = &*DEFAULT;
        let profile = registry.lookup("exec.trace.v1").expect("profile");
        let start = std::time::Instant::now();
        let report = verify_os_trace(&trace, profile, None);
        let ms = start.elapsed().as_millis();
        assert_eq!(report.verdict, Verdict::Admit, "control at {n} segments");
        println!("  {n:>6} segments  verify {ms:>5} ms");
        ms
    }

    let small = timed(5_000).max(1);
    let large = timed(50_000);
    let ratio = large as f64 / small as f64;
    println!("  tenfold segments cost {ratio:.0}x");
    assert!(
        ratio < 40.0,
        "a tenfold rise in segments cost {ratio:.0}x, which is superlinear: the duplicate-segment \
         check is scanning again"
    );
}
