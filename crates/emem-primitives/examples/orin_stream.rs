//! Orin NX streaming, end to end: an NVIDIA Jetson Orin NX streams
//! OS-traced camera frames, and each frame becomes a 63-character token
//! that reconstructs the *verified provenance* of that frame on the
//! ground.
//!
//! This is the streaming counterpart to `satellite_downlink`. Instead of
//! one pass, a continuous stream: each frame is a signed `emem.os_trace.v1`
//! record chained to the previous one (`prev_trace_cid`), so a dropped or
//! reordered frame is caught at ingest, and each trace binds the frame's
//! image digest.
//!
//! The example is deliberately honest about the boundary it reveals. Run
//! it and you see the obvious flaw: **emem does not reconstruct the image.
//! It reconstructs the attested provenance of the image** — who captured
//! it, on what measured platform, that its execution verifies, and the
//! digest that lets you check the pixels wherever they actually live. The
//! frame bytes travel out of band; the token is 63 characters and carries
//! proof, not payload. That is the design working, not failing: a video
//! stream is not memory, but *what a frame means and that it is genuine*
//! is.
//!
//! The frames are real: four Sentinel-2 true-colour crops of the Nile
//! Delta cells this example grounds, committed under
//! `examples/data/orin_frames/` (provenance in its `SOURCE.md`). Set
//! `EMEM_FRAMES_DIR` to a directory of your own captures to stream those
//! instead; the tracing, chaining, refusal, and tokens are unchanged.
//! Everything still runs offline and deterministically (committed frames,
//! fixed key, fixed clocks), so it is CI-runnable proof rather than a demo:
//!
//! ```bash
//! cargo run -p emem-primitives --example orin_stream
//! ```

use std::sync::Arc;

use ed25519_dalek::SigningKey;

use emem_core::key::{AttesterKey, KeyEpoch};
use emem_core::substrates::{TraceLayerKind, DEFAULT as SUBSTRATES};
use emem_core::trace_encodings::DEFAULT as ENCODINGS;
use emem_fact::{Attestation, Derivation, Fact, PrimaryFact, RegistryCid, SchemaCid, Source};
use emem_storage::{MaterializingStorage, Storage};
use emem_trace::{
    parse_trace_token, payload_digest_of_value, verify_os_trace, DeviceIdentity, EmittedOutput,
    OsTrace, PlatformAttestation, TraceSegment, Verdict,
};

const PROFILE: &str = "robot.fleet.v1";
const PLATFORM: &str = "nvidia.jetson-orin";
const BAND: &str = "indices.ndvi";
/// A raw 1920x1080 NV12 capture is 1080 * 1920 * 3 / 2 bytes; the second
/// yardstick in act 5, next to the committed frame's actual file size.
const NV12_1080P_BYTES: usize = 1920 * 1080 * 3 / 2;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let storage = MaterializingStorage::ephemeral(
        Arc::new(emem_core::bands::DEFAULT.clone()),
        Arc::new(emem_core::FunctionRegistry::parse_default()?),
        Arc::new(emem_core::SourceRegistry::parse_default()?),
    )?;
    let gate = storage.trace_gate.clone().expect("trace gate");
    let registry_cid = RegistryCid::new(emem_core::manifest_cid(&*emem_core::bands::DEFAULT)?);
    let schema_cid = SchemaCid::new(emem_core::manifest_cid(&*emem_core::schema::DEFAULT)?);

    // ── Act 1: the device asks to be believed. ─────────────────────────
    let mut secret = [0u8; 32];
    secret[..11].copy_from_slice(b"ORIN-NX-042");
    let sk = SigningKey::from_bytes(&secret);
    let pubkey = sk.verifying_key().to_bytes();
    let pubkey_b32 = data_encoding::BASE32_NOPAD.encode(&pubkey).to_lowercase();
    let profile = SUBSTRATES.lookup(PROFILE).expect("profile in manifest");
    let platform = emem_core::device_platforms::DEFAULT
        .lookup(PLATFORM)
        .expect("platform in whitelist");

    println!("act 1  an NVIDIA Jetson Orin NX wants to write to emem");
    println!("       device key   {pubkey_b32}");
    println!(
        "       platform     {} ({:?} root of trust, {} family)",
        platform.id, platform.root_of_trust, platform.family
    );
    println!(
        "       profile      {PROFILE} requires {} trace layers: {:?}",
        profile.required_trace_layers.len(),
        profile.required_trace_layers
    );

    // The right path first: present a platform attestation and let a
    // whitelisted anchor vouch. Today every anchor is provisional, so this
    // is refused by name — the honest "device ingest is not open" state.
    let endorser = SigningKey::from_bytes(&[9u8; 32]);
    let attestation = PlatformAttestation::build_and_sign_v0(
        PLATFORM,
        AttesterKey(pubkey),
        "jetson-orin-nx",
        "nvidia",
        "boot-nonce-1881",
        vec![],
        &endorser,
    );
    match storage.gate_enroll_attested(&pubkey_b32, PROFILE, PLATFORM, &attestation) {
        Ok(_) => println!("\n       attested enrolment admitted"),
        Err(e) => println!("\n       attested enrolment refused (as expected): {e}"),
    }

    // Fall back to the operator's assertion so the rest of the stream can
    // run. The enrolment records assurance=operator_asserted, and a reader
    // can tell it apart from a hardware-attested one.
    gate.enroll(&pubkey_b32, PROFILE)?;
    let record = gate.enrollment_of(&pubkey_b32).expect("enrolled");
    println!(
        "       enrolled operator-asserted (assurance = {})",
        record.assurance()
    );

    // ── Act 2: stream the frames, each chained to the last. ────────────
    let frames = load_frames()?;
    assert!(
        frames.len() >= 2,
        "need at least two frames to stream a chain"
    );
    println!(
        "\nact 2  streaming {} camera frames, each an OS-traced window:",
        frames.len()
    );
    let cell = emem_codec::cell64_from_latlng(30.7901, 31.0007);
    let mut prev_trace: Option<String> = None;
    let mut tokens: Vec<(String, String)> = Vec::new(); // (trace_token, fact_token)
    let mut image_digests: Vec<String> = Vec::new();

    for (f, (frame_name, image)) in frames.iter().enumerate() {
        let window_start = 1_000 + (f as u64) * 100_000;

        // The Orin's camera + ISP produced this frame — real bytes from
        // the frames directory. The trace binds their digest, not the
        // bytes themselves.
        let image_digest = render(&blake3::hash(image));
        image_digests.push(image_digest.clone());

        // The fact is the compact on-device readout for this frame: the
        // scene NDVI and a pointer to the image by digest. The image bytes
        // are NOT the fact value — that is the whole point of act 4.
        let ndvi = 0.61 + (f as f64) * 0.004;
        let fact = mk_fact(&cell, f as u64, ndvi, &image_digest, pubkey, &schema_cid);
        let descriptor_digest = match &fact {
            Fact::Primary(p) => payload_digest_of_value(&p.value)?,
            _ => unreachable!(),
        };

        // A realistic Orin trace: one segment per required layer, each
        // with the encoding that actually produces it (kernel ftrace for
        // syscalls/scheduling/memory/storage, the camera/RF recorder for
        // the sensor bus, Nsight Systems for the on-GPU model, the hardware
        // monitor for the tegra thermal zones and INA3221 power rails).
        let segments: Vec<TraceSegment> = profile
            .required_trace_layers
            .iter()
            .enumerate()
            .map(|(i, layer)| segment(*layer, i, window_start, f))
            .collect();

        // Two emissions: the raw frame (from the camera) and the model
        // readout (from inference). The device attests both; only the
        // readout becomes a fact.
        let outputs = vec![
            EmittedOutput {
                payload_digest: image_digest.clone(),
                band: None,
                emitted_at_ns: window_start + 40_000,
                layer: TraceLayerKind::SensorBus,
            },
            EmittedOutput {
                payload_digest: descriptor_digest.clone(),
                band: Some(BAND.into()),
                emitted_at_ns: window_start + 80_000,
                layer: TraceLayerKind::Inference,
            },
        ];
        let trace = OsTrace::build_and_sign_chained_v1(
            device(pubkey, BOOT),
            window_start,
            window_start + 100_000,
            segments,
            outputs,
            prev_trace.clone(),
            &sk,
        )?;

        let att = Attestation::build_and_sign_v1(
            vec![fact],
            vec![],
            registry_cid.clone(),
            schema_cid.clone(),
            &sk,
            KeyEpoch(0),
            "2026-07-27T00:00:00Z".into(),
            None,
        )?;

        let (cids, admitted) = storage.put_attestation_gated(&att, Some(&trace)).await?;
        let admitted = admitted.expect("gated admission");
        let fact_token = format!("emem:fact:{cell}:{}", cids[0].as_str());
        println!(
            "       frame {f} ({frame_name}): {}  chained<-{}",
            admitted.token,
            prev_trace.as_deref().unwrap_or("(stream head)")
        );
        prev_trace = Some(admitted.trace_cid.clone());
        tokens.push((admitted.token, fact_token));
    }

    // ── Act 3: a dropped frame is caught. ──────────────────────────────
    // Skip a frame: build a window that does NOT chain to the current
    // head. The gate refuses it — a gap in a stream is not a private loss,
    // it is a detectable break.
    println!("\nact 3  drop a frame (a window that does not chain to the head):");
    let orphan_image = &frames[frames.len() - 1].1; // captured, but its window skipped the chain
    let orphan_digest = render(&blake3::hash(orphan_image));
    let orphan_fact = mk_fact(&cell, 99, 0.5, &orphan_digest, pubkey, &schema_cid);
    let orphan_desc = match &orphan_fact {
        Fact::Primary(p) => payload_digest_of_value(&p.value)?,
        _ => unreachable!(),
    };
    let orphan_segments: Vec<TraceSegment> = profile
        .required_trace_layers
        .iter()
        .enumerate()
        .map(|(i, layer)| segment(*layer, i, 9_000_000, 99))
        .collect();
    let orphan_trace = OsTrace::build_and_sign_chained_v1(
        device(pubkey, BOOT),
        9_000_000,
        9_100_000,
        orphan_segments,
        vec![EmittedOutput {
            payload_digest: orphan_desc,
            band: Some(BAND.into()),
            emitted_at_ns: 9_080_000,
            layer: TraceLayerKind::Inference,
        }],
        None, // no prev — a fresh head mid-stream
        &sk,
    )?;
    let orphan_att = Attestation::build_and_sign_v1(
        vec![orphan_fact],
        vec![],
        registry_cid.clone(),
        schema_cid.clone(),
        &sk,
        KeyEpoch(0),
        "2026-07-27T00:00:00Z".into(),
        None,
    )?;
    let refused = storage
        .put_attestation_gated(&orphan_att, Some(&orphan_trace))
        .await;
    println!(
        "       {}",
        refused.expect_err("must refuse a broken chain")
    );

    // ── Act 3b: the Orin reboots, and starts a clean stream. ───────────
    // A rebooted device has lost the CID of its last frame, so it cannot
    // chain to it. Because the session is keyed by (device_key, boot_id)
    // and a reboot mints a fresh boot_id in the signed trace, the first
    // frame of the new boot legitimately has no prev and is admitted — the
    // device is not wedged.
    println!("\nact 3b the Orin reboots (fresh boot_id) and streams again:");
    let reboot_image = &frames[0].1; // the camera re-captures the same scene after reboot
    let reboot_digest = render(&blake3::hash(reboot_image));
    let reboot_fact = mk_fact(&cell, 0, 0.61, &reboot_digest, pubkey, &schema_cid);
    let reboot_desc = match &reboot_fact {
        Fact::Primary(p) => payload_digest_of_value(&p.value)?,
        _ => unreachable!(),
    };
    let reboot_segments: Vec<TraceSegment> = profile
        .required_trace_layers
        .iter()
        .enumerate()
        .map(|(i, layer)| segment(*layer, i, 1_000, 0))
        .collect();
    let reboot_trace = OsTrace::build_and_sign_chained_v1(
        device(pubkey, "orin-042-boot-a0"), // a NEW boot
        1_000,
        101_000,
        reboot_segments,
        vec![
            EmittedOutput {
                payload_digest: reboot_digest.clone(),
                band: None,
                emitted_at_ns: 41_000,
                layer: TraceLayerKind::SensorBus,
            },
            EmittedOutput {
                payload_digest: reboot_desc,
                band: Some(BAND.into()),
                emitted_at_ns: 81_000,
                layer: TraceLayerKind::Inference,
            },
        ],
        None, // no prev — the first frame of the new boot
        &sk,
    )?;
    let reboot_att = Attestation::build_and_sign_v1(
        vec![reboot_fact],
        vec![],
        registry_cid.clone(),
        schema_cid.clone(),
        &sk,
        KeyEpoch(0),
        "2026-07-27T00:05:00Z".into(),
        None,
    )?;
    let (_c, reboot_admitted) = storage
        .put_attestation_gated(&reboot_att, Some(&reboot_trace))
        .await?;
    println!(
        "       new-boot head admitted: {}",
        reboot_admitted.expect("reboot admits").token
    );

    // ── Act 4: reconstruct on the ground, and see what actually returns.
    println!("\nact 4  hand off only the tokens; reconstruct and re-verify:");
    for (i, (trace_token, _fact_token)) in tokens.iter().enumerate() {
        let cid = parse_trace_token(trace_token).expect("well-formed token");
        let reconstructed = storage
            .resolve_os_trace(cid)
            .expect("stored trace resolves");
        let report = verify_os_trace(&reconstructed, profile, None);
        assert_eq!(report.verdict, Verdict::Admit, "re-verified offline");
        let has_image = reconstructed
            .outputs
            .iter()
            .any(|o| o.payload_digest == image_digests[i]);
        println!(
            "       {trace_token} -> verified {:?}, binds image digest: {has_image}, prev: {}",
            report.verdict,
            reconstructed.prev_trace_cid.as_deref().unwrap_or("(head)")
        );
    }

    // Walk the chain backward from the head, token by token.
    print!("\n       chain (head -> tail): ");
    let mut walk = prev_trace.clone();
    let mut hops = 0;
    while let Some(cid) = walk {
        print!("{}..", &cid[..8]);
        hops += 1;
        walk = storage
            .resolve_os_trace(&cid)
            .and_then(|t| t.prev_trace_cid);
    }
    println!("(head)  {hops} frames linked");

    // ── Act 5: the flaw, stated plainly. ───────────────────────────────
    let token_bytes = tokens[0].0.len();
    let frame_bytes = frames[0].1.len();
    println!("\nact 5  what reconstruction returns — the honest boundary:");
    println!(
        "       the token is {token_bytes} bytes; frame 0 ({}) is {frame_bytes} bytes",
        frames[0].0
    );
    println!("       on disk, and a raw 1080p NV12 capture is {NV12_1080P_BYTES} bytes.");
    println!(
        "       ratio ~{}x against this file, ~{}x against raw NV12.",
        frame_bytes / token_bytes,
        NV12_1080P_BYTES / token_bytes
    );
    println!("       The token does not carry the frame; it carries");
    println!("       PROOF: the verified execution trace and the frame's digest.");
    println!("       Resolving it reconstructs the provenance, not the pixels.");
    println!("       The image travels out of band; blake3 == the bound digest");
    println!("       proves it was not altered. emem is memory, not a blob store:");
    println!("       putting the frame bytes in as a fact value would make every");
    println!("       fact_cid a hash of megabytes and defeat the 63-byte handoff.");

    // Prove the integrity check an out-of-band consumer would run: pull
    // the pixels from where they actually live (here, the frames
    // directory again) and check them against the digest the trace bound.
    let refetched = load_frames()?;
    assert_eq!(render(&blake3::hash(&refetched[0].1)), image_digests[0]);
    println!("\n       out-of-band frame 0 re-fetched: blake3 matches the bound");
    println!("       digest, so the pixels are provably the ones the Orin emitted.");

    // What each tracer's own integrity rests on (the trace of the trace).
    println!("\n       trace-of-the-trace: each capture encoding's own integrity:");
    for layer in &profile.required_trace_layers {
        let enc_id = encoding_for(*layer);
        if let Some(enc) = ENCODINGS.lookup(enc_id) {
            println!("       {:?} <- {} ({:?})", layer, enc_id, enc.integrity);
        }
    }

    println!(
        "\ndone   {} frames streamed, chained, and reconstructed; a dropped",
        frames.len()
    );
    println!("       frame refused; the image bound by digest, not swallowed.");
    Ok(())
}

/// Where the frames come from: `EMEM_FRAMES_DIR` if set (point it at a
/// directory of your own captures), else the committed sample frames —
/// real Sentinel-2 true-colour crops of the cells this example grounds
/// (provenance in `examples/data/orin_frames/SOURCE.md`). Files are read
/// as bytes, sorted by name; the format never matters because only the
/// digest enters the trace.
fn load_frames() -> Result<Vec<(String, Vec<u8>)>, std::io::Error> {
    let dir = std::env::var("EMEM_FRAMES_DIR").unwrap_or_else(|_| {
        concat!(env!("CARGO_MANIFEST_DIR"), "/examples/data/orin_frames").into()
    });
    let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x != "md"))
        .collect();
    paths.sort();
    let mut frames = Vec::with_capacity(paths.len());
    for p in paths {
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        frames.push((name, std::fs::read(&p)?));
    }
    Ok(frames)
}

fn render(h: &blake3::Hash) -> String {
    data_encoding::BASE32_NOPAD
        .encode(h.as_bytes())
        .to_lowercase()
}

const BOOT: &str = "orin-042-boot-9f";

fn device(pubkey: [u8; 32], boot_id: &str) -> DeviceIdentity {
    DeviceIdentity {
        device_key: AttesterKey(pubkey),
        key_epoch: KeyEpoch(0),
        substrate_profile: PROFILE.into(),
        platform: "jetson-orin-nx".into(),
        os: "jetpack-6.0 (ubuntu-22.04)".into(),
        kernel: "5.15.148-tegra".into(),
        boot_id: boot_id.into(),
    }
}

/// The registered capture encoding that produces `layer` on an Orin.
fn encoding_for(layer: TraceLayerKind) -> &'static str {
    match layer {
        TraceLayerKind::SensorBus | TraceLayerKind::Signal => "ros2.bag.v2",
        TraceLayerKind::Energy | TraceLayerKind::Thermal => "linux.hwmon.v1",
        TraceLayerKind::Inference => "nvidia.nsys.v1",
        _ => "linux.ftrace.v1",
    }
}

/// One trace segment. `raw` stands for the real capture file bytes; in
/// production the digest is over the actual ftrace / rosbag / nsys / hwmon
/// capture for the window.
fn segment(layer: TraceLayerKind, i: usize, window_start: u64, frame: usize) -> TraceSegment {
    // Realistic-looking capture labels for an Orin NX frame window.
    let raw = match layer {
        TraceLayerKind::Thermal => {
            format!("tegra thermal frame {frame}: CPU-therm 46.5C GPU-therm 44.0C SOC0-therm 45.2C")
        }
        TraceLayerKind::Energy => {
            format!("INA3221 frame {frame}: VDD_IN 7180mW VDD_CPU_GPU_CV 2210mW VDD_SOC 1560mW")
        }
        TraceLayerKind::Inference => {
            format!("nsys frame {frame}: trt_engine detectnet_v2 inferences=1 gpu_ms=6.4")
        }
        TraceLayerKind::SensorBus => {
            format!("argus frame {frame}: /cam0/image_raw 1920x1080 NV12 seq={frame}")
        }
        other => format!("ftrace frame {frame} {other:?}"),
    };
    TraceSegment {
        layer,
        seq: 0,
        clock_start_ns: window_start + i as u64,
        clock_end_ns: window_start + 90_000,
        event_count: 512,
        log_digest: render(&blake3::hash(raw.as_bytes())),
        prev_digest: None,
        encoding: encoding_for(layer).into(),
    }
}

fn mk_fact(
    cell: &str,
    frame: u64,
    ndvi: f64,
    image_digest: &str,
    pubkey: [u8; 32],
    schema_cid: &SchemaCid,
) -> Fact {
    Fact::Primary(PrimaryFact {
        cell: cell.into(),
        band: BAND.into(),
        tslot: frame,
        value: ciborium::Value::Float(ndvi),
        unit: None,
        confidence: 0.98,
        uncertainty: None,
        sources: vec![Source {
            scheme: "orin.camera".into(),
            id: format!("ORIN-NX-042/frame-{frame}/image:{image_digest}"),
            cid: None,
            hash: None,
            captured_at: Some("2026-07-27T00:00:00Z".into()),
            url: None,
        }],
        derivation: Derivation {
            fn_key: "ndvi@1".into(),
            args: None,
        },
        privacy_class: "public".into(),
        schema_cid: schema_cid.clone(),
        signer: AttesterKey(pubkey),
        signed_at: "2026-07-27T00:00:00Z".into(),
        served_via: None,
    })
}
