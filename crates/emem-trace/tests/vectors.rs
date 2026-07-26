//! Conformance vectors for `emem.os_trace.v1` (spec/test_vectors/os_trace/).
//!
//! The committed JSON vectors are authoritative: this test replays each
//! one through [`emem_trace::verify_os_trace`] and asserts the verdict,
//! the reason codes, and the trace CID. Regenerate after an intentional
//! schema change with:
//!
//! ```bash
//! EMEM_REGEN_VECTORS=1 cargo test -p emem-trace --test vectors
//! ```
//!
//! then review the diff like any other wire change: a vector that moved
//! without a spec bump is a bug, not an update.

use std::path::PathBuf;

use ed25519_dalek::SigningKey;
use serde_json::{json, Value};

use emem_core::key::{AttesterKey, KeyEpoch};
use emem_core::substrates::{TraceLayerKind, DEFAULT};
use emem_trace::{verify_os_trace, DeviceIdentity, EmittedOutput, OsTrace, TraceSegment, Verdict};

fn vectors_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../spec/test_vectors/os_trace")
}

fn signing_key() -> SigningKey {
    let mut bytes = [0u8; 32];
    bytes[0] = 42;
    SigningKey::from_bytes(&bytes)
}

fn digest_of(raw: &[u8]) -> String {
    use data_encoding::BASE32_NOPAD;
    BASE32_NOPAD
        .encode(blake3::hash(raw).as_bytes())
        .to_lowercase()
}

fn segment(layer: TraceLayerKind, start: u64, raw: &[u8]) -> TraceSegment {
    TraceSegment {
        layer,
        seq: 0,
        clock_start_ns: start,
        clock_end_ns: 9_000,
        event_count: raw.len() as u64,
        log_digest: digest_of(raw),
        prev_digest: None,
        encoding: "linux.ftrace.v1".into(),
    }
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

fn robot_trace(sk: &SigningKey, payload_digest: &str, profile: &str) -> OsTrace {
    let profile_entry = DEFAULT.lookup(profile).expect("profile");
    let layers: &[TraceLayerKind] = if profile_entry.required_trace_layers.is_empty() {
        &[TraceLayerKind::Syscall, TraceLayerKind::SensorBus]
    } else {
        &profile_entry.required_trace_layers
    };
    let segments: Vec<TraceSegment> = layers
        .iter()
        .enumerate()
        .map(|(i, l)| segment(*l, 1_000 + i as u64, format!("log {i}").as_bytes()))
        .collect();
    OsTrace::build_and_sign_v1(
        device(sk, profile),
        1_000,
        10_000,
        segments,
        vec![EmittedOutput {
            payload_digest: payload_digest.into(),
            band: Some("indices.ndvi".into()),
            emitted_at_ns: 8_500,
            layer: TraceLayerKind::SensorBus,
        }],
        sk,
    )
    .expect("build")
}

/// The four vectors, built deterministically (fixed key, fixed clocks).
fn build_vectors() -> Vec<(String, Value)> {
    let sk = signing_key();
    let payload = digest_of(b"lidar frame 881");
    let mut out = Vec::new();

    // 1. Sound trace, bound payload: admit.
    let sound = robot_trace(&sk, &payload, "robot.fleet.v1");
    out.push(vector(
        "trace.os_trace.admit.0001",
        "robot.fleet.v1",
        Some(&payload),
        &sound,
        "A complete robot.fleet.v1 trace whose emitted output binds the claimed payload digest.",
    ));

    // 2. One captured log edited after signing: the chain is cut.
    let mut broken = sound.clone();
    broken.segments[2].log_digest = digest_of(b"scrubbed log");
    out.push(vector(
        "trace.os_trace.chain_broken.0002",
        "robot.fleet.v1",
        Some(&payload),
        &broken,
        "Segment 2's log digest was rewritten after signing; segment 3's prev_digest no longer matches.",
    ));

    // 3. Sound trace, but the write path claims a payload the execution
    //    never emitted.
    let forged = digest_of(b"payload the execution never emitted");
    out.push(vector(
        "trace.os_trace.output_unbound.0003",
        "robot.fleet.v1",
        Some(&forged),
        &sound,
        "The trace verifies but the claimed payload digest is not among its emitted outputs.",
    ));

    // 4. A device trying to write under the archive profile: the
    //    archive path never admits device output.
    let archive = robot_trace(&sk, &payload, "earth.satellite.v0");
    out.push(vector(
        "trace.os_trace.archive_refused.0004",
        "earth.satellite.v0",
        Some(&payload),
        &archive,
        "earth.satellite.v0 is archive_recomputable; device output cannot enter through the trace path.",
    ));

    out
}

fn vector(
    id: &str,
    profile: &str,
    claimed: Option<&str>,
    trace: &OsTrace,
    notes: &str,
) -> (String, Value) {
    let p = DEFAULT.lookup(profile).expect("profile");
    let report = verify_os_trace(trace, p, claimed);
    let verdict = match report.verdict {
        Verdict::Admit => "admit",
        Verdict::Reject => "reject",
    };
    let mut codes: Vec<String> = report
        .reasons
        .iter()
        .map(|r| {
            serde_json::to_value(r).expect("reason json")["code"]
                .as_str()
                .expect("code")
                .to_string()
        })
        .collect();
    codes.dedup();
    let v = json!({
        "id": id,
        "kind": "os_trace",
        "spec": "v1.2.1",
        "level": "L1",
        "input": {
            "profile": profile,
            "claimed_payload_digest": claimed,
            "trace": serde_json::to_value(trace).expect("trace json"),
        },
        "expected": {
            "verdict": verdict,
            "reason_codes": codes,
            "trace_cid": trace.trace_cid().expect("cid"),
        },
        "notes": notes,
    });
    (format!("{id}.json"), v)
}

#[test]
fn committed_vectors_replay() {
    if std::env::var("EMEM_REGEN_VECTORS").is_ok() {
        let dir = vectors_dir();
        std::fs::create_dir_all(&dir).expect("mkdir");
        for (name, v) in build_vectors() {
            let mut s = serde_json::to_string_pretty(&v).expect("json");
            s.push('\n');
            std::fs::write(dir.join(&name), s).expect("write vector");
        }
    }

    let dir = vectors_dir();
    let mut count = 0;
    for entry in
        std::fs::read_dir(&dir).expect("os_trace vector dir (regen with EMEM_REGEN_VECTORS=1)")
    {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        count += 1;
        let raw = std::fs::read_to_string(&path).expect("read vector");
        let v: Value = serde_json::from_str(&raw).expect("vector json");
        let profile_id = v["input"]["profile"].as_str().expect("profile");
        let profile = DEFAULT.lookup(profile_id).expect("profile in manifest");
        let claimed = v["input"]["claimed_payload_digest"].as_str();
        let trace: OsTrace =
            serde_json::from_value(v["input"]["trace"].clone()).expect("trace record");
        let report = verify_os_trace(&trace, profile, claimed);

        let expect_verdict = v["expected"]["verdict"].as_str().expect("verdict");
        let got_verdict = match report.verdict {
            Verdict::Admit => "admit",
            Verdict::Reject => "reject",
        };
        assert_eq!(got_verdict, expect_verdict, "{path:?}: verdict");

        let mut got_codes: Vec<String> = report
            .reasons
            .iter()
            .map(|r| {
                serde_json::to_value(r).expect("reason json")["code"]
                    .as_str()
                    .expect("code")
                    .to_string()
            })
            .collect();
        got_codes.dedup();
        let expect_codes: Vec<String> = v["expected"]["reason_codes"]
            .as_array()
            .expect("codes")
            .iter()
            .map(|c| c.as_str().expect("code").to_string())
            .collect();
        assert_eq!(got_codes, expect_codes, "{path:?}: reason codes");

        assert_eq!(
            trace.trace_cid().expect("cid"),
            v["expected"]["trace_cid"].as_str().expect("cid"),
            "{path:?}: trace_cid"
        );
    }
    assert_eq!(count, 4, "expected the four committed os_trace vectors");
}
