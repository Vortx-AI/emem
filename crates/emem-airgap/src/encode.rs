//! The encoder: capture what this machine will actually show you, and claim
//! nothing else.
//!
//! # Why this is a separate binary from the decoder
//!
//! Privilege. Measured on a Jetson-class host:
//!
//! ```text
//! /sys/kernel/tracing   700 root   syscall, scheduler, memory   needs privilege
//! /sys/class/hwmon      755        energy, thermal              needs none
//! /proc/*               444        uptime, boot id              needs none
//! ```
//!
//! The decoder runs `--cap-drop ALL` and its whole claim is that it cannot do
//! anything. An encoder needs a tracefs mount. Putting both in one image would
//! mean an operator who copies the encoder's flags onto the decoder silently
//! throws away that claim, and nothing would complain. Two images, one crate:
//! the code is shared, the postures cannot be confused.
//!
//! # The rule
//!
//! A layer appears in a trace only if this process actually read it. Where a
//! source is unreadable the layer is ABSENT and the reason is reported. A
//! trace missing layers a substrate profile requires will be refused by
//! `verify_os_trace`, and that refusal is correct: it is the difference
//! between an honest partial capture and a fabricated complete one.

use std::path::{Path, PathBuf};

use ed25519_dalek::{Signer, SigningKey};
use emem_core::key::{AttesterKey, KeyEpoch, Signature};
use emem_core::substrates::TraceLayerKind;
use emem_trace::{DeviceIdentity, EmittedOutput, OsTrace, TraceSegment, OS_TRACE_SCHEMA_V1};
use serde::{Deserialize, Serialize};

use crate::b32;

/// Where a layer's evidence comes from, and what it costs to read.
struct Source {
    layer: TraceLayerKind,
    encoding: &'static str,
    /// Paths tried in order; the first readable one wins.
    roots: &'static [&'static str],
}

/// Every layer this encoder knows how to capture on Linux.
///
/// Deliberately short. An encoder that listed a source per layer and then
/// silently produced nothing for most of them would be worse than one that
/// admits its range: the reader of a trace cannot tell "captured nothing" from
/// "there was nothing to capture" unless the encoder says which.
const SOURCES: &[Source] = &[
    Source {
        layer: TraceLayerKind::Thermal,
        encoding: "linux.hwmon.v1",
        roots: &["/sys/class/thermal", "/sys/class/hwmon"],
    },
    Source {
        layer: TraceLayerKind::Energy,
        encoding: "linux.hwmon.v1",
        roots: &["/sys/class/powercap", "/sys/class/hwmon"],
    },
    Source {
        layer: TraceLayerKind::Syscall,
        encoding: "linux.ftrace.v1",
        roots: &["/sys/kernel/tracing", "/sys/kernel/debug/tracing"],
    },
    Source {
        layer: TraceLayerKind::Scheduler,
        encoding: "linux.ftrace.v1",
        roots: &["/sys/kernel/tracing", "/sys/kernel/debug/tracing"],
    },
    Source {
        layer: TraceLayerKind::Memory,
        encoding: "linux.ftrace.v1",
        roots: &["/sys/kernel/tracing", "/sys/kernel/debug/tracing"],
    },
];

/// A layer the encoder could not capture, and why. Reported, never hidden.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissedLayer {
    /// The layer, as the substrate profile names it.
    pub layer: String,
    /// What was tried.
    pub source: String,
    /// Why it did not work, in words an operator can act on.
    pub reason: String,
}

/// What one capture window produced.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureReport {
    /// Content id of the trace written, when one was.
    pub trace_cid: Option<String>,
    /// Layers actually read.
    pub captured: Vec<String>,
    /// Layers that could not be read, with the reason for each.
    pub missed: Vec<MissedLayer>,
    /// Payload digests bound into the trace as emitted outputs.
    pub outputs: usize,
    /// Plain-language statement of what this trace can and cannot support.
    pub admissibility: String,
}

/// How a capture run is configured.
pub struct CaptureSettings {
    /// Directory the trace is written to. The decoder reads this.
    pub out: PathBuf,
    /// Directory holding the payloads this window produced; their digests are
    /// bound into the trace as emitted outputs.
    pub payloads: Option<PathBuf>,
    /// Substrate profile the device writes under.
    pub profile: String,
    /// Hardware platform string, e.g. `jetson-orin-nx`.
    pub platform: String,
    /// Content id of the previous trace in this device's stream, chaining the
    /// windows so a dropped one is detectable.
    pub prev_trace_cid: Option<String>,
}

/// Read a small file, returning None rather than failing the run.
fn read_small(p: &Path) -> Option<Vec<u8>> {
    // symlink_metadata: a link under a sysfs path we walk should not be
    // followed to somewhere else, for the same reason it is refused in the
    // decoder's input directory.
    let meta = std::fs::symlink_metadata(p).ok()?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        return None;
    }
    if meta.len() > 8 * 1024 * 1024 {
        return None;
    }
    // A bounded read rather than read-to-end. Most sysfs files report length 0
    // whatever they contain, so the size check above cannot protect against a
    // file that keeps producing bytes; this stops at a fixed ceiling either
    // way.
    use std::io::Read;
    let mut f = std::fs::File::open(p).ok()?;
    let mut buf = Vec::new();
    f.by_ref()
        .take(8 * 1024 * 1024)
        .read_to_end(&mut buf)
        .ok()?;
    Some(buf)
}

/// Monotonic nanoseconds since boot, from /proc/uptime.
///
/// Not the wall clock. The schema wants a monotonic window, and uptime is
/// monotonic by construction where `SystemTime` is not: a clock correction
/// mid-window would otherwise make a trace look like it ended before it began.
fn monotonic_ns() -> u64 {
    std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| s.split_whitespace().next().map(str::to_string))
        .and_then(|s| s.parse::<f64>().ok())
        .map(|secs| (secs * 1e9) as u64)
        .unwrap_or(0)
}

/// Gather every readable leaf under a directory, one level deep per entry.
///
/// The bytes read ARE the capture. `log_digest` is taken over them, so the
/// digest commits to what this process actually saw rather than to a label
/// describing what it might have seen.
fn harvest(root: &Path, want: &[&str]) -> (Vec<u8>, u64) {
    let mut buf = Vec::new();
    let mut count = 0u64;
    let mut dirs: Vec<PathBuf> = vec![root.to_path_buf()];
    // One level of nesting: /sys/class/thermal/thermal_zone0/temp and friends.
    if let Ok(entries) = std::fs::read_dir(root) {
        for e in entries.filter_map(|e| e.ok()) {
            dirs.push(e.path());
        }
    }
    dirs.sort();
    for d in dirs {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        let mut names: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        names.sort();
        for f in names {
            let Some(name) = f.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !want.contains(&name) {
                continue;
            }
            // Belt and braces against the hang above: anything whose name says
            // "pipe" streams, and a streaming file has no end to read to.
            if name.contains("pipe") {
                continue;
            }
            if let Some(bytes) = read_small(&f) {
                buf.extend_from_slice(f.to_string_lossy().as_bytes());
                buf.push(b'=');
                buf.extend_from_slice(&bytes);
                buf.push(b'\n');
                count += 1;
            }
        }
    }
    (buf, count)
}

/// Capture one layer, or explain why not.
fn capture(src: &Source) -> Result<(Vec<u8>, u64, &'static str), MissedLayer> {
    for root in src.roots {
        let p = Path::new(root);
        if !p.exists() {
            continue;
        }
        // The distinction an operator needs: absent versus forbidden.
        if std::fs::read_dir(p).is_err() {
            return Err(MissedLayer {
                layer: format!("{:?}", src.layer).to_lowercase(),
                source: (*root).to_string(),
                reason: format!(
                    "{root} exists but is not readable by uid {}. Mount it and grant the \
                     capability, or accept that this layer is absent.",
                    unsafe_uid()
                ),
            });
        }
        // EXACT file names, never a substring match.
        //
        // This read `*trace*` once, which in tracefs matches `trace_pipe`. That
        // file blocks forever waiting for events, so the encoder hung the
        // moment it was finally run with enough privilege to open it: ten
        // minutes and still going. A blocking read in a sidecar is worse than
        // a missing layer, because the operator sees a process that is neither
        // working nor failing.
        //
        // Naming the files also documents what is actually being captured,
        // which a wildcard never did.
        let want: &[&str] = match src.layer {
            TraceLayerKind::Thermal => &["temp1_input", "temp2_input", "type"],
            TraceLayerKind::Energy => &["energy_uj", "power1_input", "curr1_input", "in1_input"],
            // `trace` is the static snapshot buffer and returns immediately.
            // `trace_pipe` is the streaming one and must never appear here.
            _ => &["trace", "current_tracer", "available_tracers", "tracing_on"],
        };
        let (bytes, count) = harvest(p, want);
        if bytes.is_empty() {
            return Err(MissedLayer {
                layer: format!("{:?}", src.layer).to_lowercase(),
                source: (*root).to_string(),
                reason: format!("{root} is readable but exposed nothing for this layer"),
            });
        }
        return Ok((bytes, count, src.encoding));
    }
    Err(MissedLayer {
        layer: format!("{:?}", src.layer).to_lowercase(),
        source: src.roots.join(", "),
        reason: "none of these paths exist on this machine".into(),
    })
}

/// The process uid, for an error message. No libc: /proc knows.
fn unsafe_uid() -> String {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Uid:"))
                .and_then(|l| l.split_whitespace().nth(1).map(str::to_string))
        })
        .unwrap_or_else(|| "?".into())
}

/// Read the OS and kernel identity strings the trace records.
fn os_and_kernel() -> (String, String) {
    let os = std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|s| {
            s.lines().find(|l| l.starts_with("PRETTY_NAME=")).map(|l| {
                l.trim_start_matches("PRETTY_NAME=")
                    .trim_matches('"')
                    .to_string()
            })
        })
        .unwrap_or_else(|| "unknown".into());
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "unknown".into());
    (os, kernel)
}

/// The boot identifier, so two windows from one boot are linkable and a trace
/// replayed from an older boot is not silently treated as current.
fn boot_id() -> String {
    std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "unknown".into())
}

/// Capture one window and write a signed trace.
pub fn capture_window(
    key: &SigningKey,
    settings: &CaptureSettings,
) -> std::io::Result<CaptureReport> {
    let window_start_ns = monotonic_ns();

    let mut segments = Vec::new();
    let mut captured = Vec::new();
    let mut missed = Vec::new();
    for (i, src) in SOURCES.iter().enumerate() {
        match capture(src) {
            Ok((bytes, events, encoding)) => {
                segments.push(TraceSegment {
                    layer: src.layer,
                    seq: i as u64,
                    clock_start_ns: window_start_ns,
                    clock_end_ns: monotonic_ns().max(window_start_ns + 1),
                    event_count: events,
                    log_digest: b32(blake3::hash(&bytes).as_bytes()),
                    prev_digest: None,
                    encoding: encoding.to_string(),
                });
                captured.push(format!("{:?}", src.layer).to_lowercase());
            }
            Err(m) => missed.push(m),
        }
    }

    let window_end_ns = monotonic_ns().max(window_start_ns + 1);

    // Bind the payloads this window produced. Their digests are what lets the
    // decoder join a custody record to this trace.
    let mut outputs = Vec::new();
    if let Some(dir) = &settings.payloads {
        if let Ok(entries) = std::fs::read_dir(dir) {
            let mut paths: Vec<PathBuf> =
                entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
            paths.sort();
            for p in paths {
                // symlink_metadata, for the same reason the decoder uses it:
                // a link here would bind a digest of a file outside the
                // directory into a signed record.
                let Ok(meta) = std::fs::symlink_metadata(&p) else {
                    continue;
                };
                if meta.file_type().is_symlink() || !meta.is_file() {
                    continue;
                }
                let Some(bytes) = read_small(&p) else {
                    continue;
                };
                outputs.push(EmittedOutput {
                    payload_digest: b32(blake3::hash(&bytes).as_bytes()),
                    band: None,
                    emitted_at_ns: window_start_ns,
                    layer: TraceLayerKind::SensorBus,
                });
            }
        }
    }

    if segments.is_empty() {
        // No fabrication, and no empty trace either: verify_os_trace rejects
        // one, so writing it would only move the failure downstream.
        return Ok(CaptureReport {
            trace_cid: None,
            captured,
            missed,
            outputs: outputs.len(),
            admissibility: "no trace written: not one layer could be read on this machine. \
                            An empty trace is rejected by the verifier, so emitting one would \
                            only move the failure downstream."
                .into(),
        });
    }

    let (os, kernel) = os_and_kernel();
    let device = DeviceIdentity {
        device_key: AttesterKey(key.verifying_key().to_bytes()),
        key_epoch: KeyEpoch(0),
        substrate_profile: settings.profile.clone(),
        platform: settings.platform.clone(),
        os,
        kernel,
        boot_id: boot_id(),
    };

    let root = OsTrace::compute_trace_root(&segments)
        .map_err(|e| std::io::Error::other(format!("trace root: {e}")))?;
    let mut trace = OsTrace {
        schema: OS_TRACE_SCHEMA_V1.to_string(),
        device,
        window_start_ns,
        window_end_ns,
        segments,
        outputs,
        trace_root: b32(&root),
        prev_trace_cid: settings.prev_trace_cid.clone(),
        signature: Signature([0u8; 64]),
    };
    let pre = trace
        .preimage()
        .map_err(|e| std::io::Error::other(format!("preimage: {e}")))?;
    trace.signature = Signature(key.sign(&pre).to_bytes());

    let cid = trace
        .trace_cid()
        .map_err(|e| std::io::Error::other(format!("cid: {e}")))?;
    std::fs::create_dir_all(&settings.out)?;
    let path = settings.out.join(format!("{cid}.trace.json"));
    crate::run::write_atomic(&path, &serde_json::to_vec_pretty(&trace)?)?;

    let admissibility = if missed.is_empty() {
        "every layer this encoder knows how to read was captured".to_string()
    } else {
        format!(
            "{} layer(s) captured, {} absent. A substrate profile requiring an absent layer will \
             REFUSE this trace, and that refusal is correct: a partial capture is not a complete \
             one, and the alternative would have been to invent the difference.",
            trace.segments.len(),
            missed.len()
        )
    };
    Ok(CaptureReport {
        trace_cid: Some(cid),
        captured,
        missed,
        outputs: trace.outputs.len(),
        admissibility,
    })
}
