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
    /// Unprivileged fallback: named files under /proc, read whole, tried only
    /// when no root above yielded anything.
    ///
    /// These exist because a real flight platform grants neither a tracefs
    /// mount nor CAP_DAC_READ_SEARCH, and without them the encoder captured
    /// one layer out of ten and wrote no trace at all. An encoder that can
    /// only work on a host willing to hand it kernel tracing is an encoder
    /// that does nothing on the hardware it was built for.
    ///
    /// They are NOT the same evidence and are never labelled as though they
    /// were: see [`PROCFS_ENCODING`].
    procfs: &'static [&'static str],
}

/// What a /proc-sourced segment is called, and why it is not `linux.ftrace.v1`.
///
/// ftrace gives an event log: what happened, in order. /proc gives counters:
/// totals since boot, sampled at two instants. A counter delta is real
/// evidence about a machine and is worth signing, but it is weaker, and a
/// reader has to be able to tell which one they are holding. Labelling a
/// counter snapshot as an event log would be the exact kind of quiet
/// overstatement this crate exists to avoid.
///
/// Note what this does NOT do: no profile in the registry is satisfied by it
/// where it was not satisfied before, because every trace-admitted profile
/// requires the syscall layer and syscall has no unprivileged source. A
/// /proc-only trace stays inadmissible; it is simply no longer empty.
pub const PROCFS_ENCODING: &str = "linux.procfs.v1";

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
        procfs: &[],
    },
    Source {
        layer: TraceLayerKind::Energy,
        encoding: "linux.hwmon.v1",
        roots: &["/sys/class/powercap", "/sys/class/hwmon"],
        procfs: &[],
    },
    Source {
        layer: TraceLayerKind::Syscall,
        encoding: "linux.ftrace.v1",
        roots: &["/sys/kernel/tracing", "/sys/kernel/debug/tracing"],
        procfs: &[],
    },
    Source {
        layer: TraceLayerKind::Scheduler,
        encoding: "linux.ftrace.v1",
        roots: &["/sys/kernel/tracing", "/sys/kernel/debug/tracing"],
        procfs: &["/proc/schedstat", "/proc/loadavg", "/proc/stat"],
    },
    Source {
        layer: TraceLayerKind::Memory,
        encoding: "linux.ftrace.v1",
        roots: &["/sys/kernel/tracing", "/sys/kernel/debug/tracing"],
        procfs: &["/proc/meminfo", "/proc/vmstat"],
    },
    Source {
        layer: TraceLayerKind::Storage,
        encoding: "linux.ftrace.v1",
        roots: &["/sys/kernel/tracing", "/sys/kernel/debug/tracing"],
        procfs: &["/proc/diskstats"],
    },
    Source {
        layer: TraceLayerKind::Network,
        encoding: "linux.ftrace.v1",
        roots: &["/sys/kernel/tracing", "/sys/kernel/debug/tracing"],
        procfs: &["/proc/net/dev", "/proc/net/snmp"],
    },
];

/// Layers this encoder has NO source for, and why.
///
/// Reported alongside the ones it tried and failed, because the two are
/// different problems and an operator needs to tell them apart. "The path was
/// unreadable, mount it" is a configuration fix. "Nothing here can produce
/// this" is not, and a report that showed only the first would leave someone
/// hunting for a permission they could never grant.
///
/// These are honest gaps rather than oversights: the trace-encodings registry
/// lists `ros2.bag.v2` as the only thing that captures the sensor bus and the
/// signal layer, and a ROS bag is produced by a robotics stack, not by reading
/// sysfs. Emitting a segment labelled `ros2.bag.v2` without one would be a lie
/// about provenance in a provenance record.
const UNSUPPORTED: &[(&str, &str)] = &[
    (
        "sensor_bus",
        "no source in this encoder: the registry's only encoding for it is ros2.bag.v2, which \
         a robotics stack produces and reading sysfs does not",
    ),
    (
        "signal",
        "no source in this encoder: as above, ros2.bag.v2 is the only registered encoding",
    ),
    (
        "inference",
        "no source in this encoder: registered encodings are linux.ebpf.raw and nvidia.nsys.v1, \
         both of which need a profiler attached to the workload rather than a directory read",
    ),
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
    /// Layers this encoder tried and could not read, with the reason.
    pub missed: Vec<MissedLayer>,
    /// Layers this encoder has no source for at all. A different problem from
    /// `missed`: no mount or capability will fix these, and saying so stops an
    /// operator hunting for a permission that does not exist.
    pub unsupported: Vec<MissedLayer>,
    /// Payload digests bound into the trace as emitted outputs.
    pub outputs: usize,
    /// Every substrate profile whose required layers this capture actually
    /// covers, computed against the registry rather than guessed.
    ///
    /// Without this an operator saw only a refusal: "required trace layer
    /// missing: Syscall, SensorBus, Signal", with nothing saying whether any
    /// profile could have accepted what was captured, or which. Reported from
    /// a real deployment, where the answer turned out to be none, and finding
    /// that out took a round trip.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accepted_by: Vec<String>,
    /// Plain-language statement of what this trace can and cannot support.
    pub admissibility: String,
}

/// Where a device's trace stream has got to, kept across restarts.
///
/// A chain is only worth having if it survives the thing that breaks chains.
/// The sidecar is stopped, the bus browns out, the container is rescheduled;
/// on the way back the encoder has to know which trace to chain from, or every
/// restart silently begins a new stream and the gap is invisible.
///
/// Keyed by boot id, and that is the load-bearing part. Two windows from one
/// boot belong to one chain. After a REBOOT the previous head refers to a
/// stream this kernel never ran, so chaining to it would assert a continuity
/// that did not happen: a fresh boot starts a fresh stream, which is exactly
/// what the trace gate expects.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamHead {
    /// Boot this head belongs to.
    pub boot_id: String,
    /// Content id of the most recent trace written under that boot.
    pub trace_cid: String,
    /// How many windows this stream has produced, for an operator reading the
    /// file rather than the traces.
    pub windows: u64,
}

impl StreamHead {
    /// Read the head, if it belongs to the boot we are in now.
    ///
    /// A head from an earlier boot is not an error and not corruption: it is a
    /// device that restarted, so it is ignored and a new stream begins.
    pub fn load_for_this_boot(path: &Path) -> Option<Self> {
        let raw = std::fs::read(path).ok()?;
        let head: StreamHead = serde_json::from_slice(&raw).ok()?;
        (head.boot_id == boot_id()).then_some(head)
    }

    /// Where the head lives: beside the traces it names.
    ///
    /// It used to live beside the identity, in `--data`, and `--data` defaults
    /// to the working directory. On a read-only rootfs that meant the encoder
    /// wrote its trace and then exited 1 trying to record where the chain had
    /// got to, which is the worst of both: the work was done and the run
    /// reported failure. Reported from a real deployment.
    ///
    /// The traces directory is the one place an encoder is guaranteed to be
    /// able to write, because it is where its output goes. The head is not
    /// secret either: it holds the previous trace's content id, which is
    /// public, and a boot id.
    ///
    /// In a `.state` subdirectory rather than loose among the traces, because
    /// the decoder scans that directory for traces and skips subdirectories
    /// without comment. Loose, it would be counted as debris on every run.
    pub fn path(out_dir: &Path) -> PathBuf {
        out_dir.join(".state").join("stream_head.json")
    }

    /// Record a new head.
    ///
    /// Written AFTER the trace it names, and atomically. If power is lost
    /// between the two, the head still points at the older trace and the next
    /// window chains from there: the just-written trace is left unreferenced,
    /// which an operator can see, rather than two windows claiming the same
    /// predecessor, which would be a fork nobody could tell from tampering.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        // The head lives in a .state subdirectory now, so making it is part of
        // saving. Without this the encoder wrote its trace and then failed on
        // the very last step, which is the one failure shape worth avoiding
        // above all others: work done, run reported as failed.
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| {
                std::io::Error::new(
                    e.kind(),
                    format!(
                        "cannot create {} for the stream head: {e}. This lives beside the \
                         traces, so the directory --out points at must be writable.",
                        dir.display()
                    ),
                )
            })?;
        }
        crate::run::write_atomic(path, &serde_json::to_vec_pretty(self)?)
    }
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

/// A monotonic clock with a since-boot anchor and nanosecond resolution.
///
/// Neither half alone is enough. `/proc/uptime` is since boot, which is what
/// the schema's window means, but it is written to two decimal places: every
/// reading is quantised to 10 ms. `Instant` has nanosecond resolution but no
/// absolute value at all, only differences. So the anchor is read once from
/// uptime and every later reading is that anchor plus an `Instant` delta.
///
/// The coarse clock alone produced traces that were byte-identical. Four
/// encoders capturing four different windows on a live machine all landed in
/// the same 10 ms slot, and because the recorded window was
/// `start .. start + 1` regardless of how long the capture ran, nothing else
/// in the record distinguished them. They shared a content id, so three of the
/// four silently overwrote the first. Worse than losing the files: a trace
/// whose bytes do not depend on when it was captured is a trace that can be
/// replayed as evidence of any later window, which is the one thing an
/// execution trace exists to prevent.
///
/// Wall clock is deliberately not used: a clock correction mid-window would
/// make a trace appear to end before it began.
struct MonotonicClock {
    anchor_ns: u64,
    from: std::time::Instant,
}

impl MonotonicClock {
    fn start() -> Self {
        Self {
            anchor_ns: uptime_ns(),
            from: std::time::Instant::now(),
        }
    }

    /// Nanoseconds since boot, to the resolution the platform actually offers.
    fn now(&self) -> u64 {
        self.anchor_ns
            .saturating_add(self.from.elapsed().as_nanos().min(u64::MAX as u128) as u64)
    }
}

/// Seconds since boot from /proc/uptime, in nanoseconds. Quantised to 10 ms by
/// the file's own format; used only as [`MonotonicClock`]'s anchor.
fn uptime_ns() -> u64 {
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
    // The richest source first, the unprivileged one after. A reason from the
    // richest source is kept so the report can say what the host would have to
    // grant for the stronger evidence, even when the fallback succeeded.
    let mut best_reason: Option<MissedLayer> = None;
    for root in src.roots {
        let p = Path::new(root);
        if !p.exists() {
            continue;
        }
        // The distinction an operator needs: absent versus forbidden.
        if std::fs::read_dir(p).is_err() {
            best_reason.get_or_insert(MissedLayer {
                layer: format!("{:?}", src.layer).to_lowercase(),
                source: (*root).to_string(),
                reason: format!(
                    "{root} exists but is not readable by uid {}. Mount it and grant the \
                     capability, or accept that this layer is absent.",
                    unsafe_uid()
                ),
            });
            continue;
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
            best_reason.get_or_insert(MissedLayer {
                layer: format!("{:?}", src.layer).to_lowercase(),
                source: (*root).to_string(),
                reason: format!("{root} is readable but exposed nothing for this layer"),
            });
            continue;
        }
        return Ok((bytes, count, src.encoding));
    }

    // Nothing richer was available. Counters, honestly labelled.
    let mut buf = Vec::new();
    let mut count = 0u64;
    for path in src.procfs {
        let Some(bytes) = read_small(Path::new(path)) else {
            continue;
        };
        if bytes.is_empty() {
            continue;
        }
        // The path is part of the capture, not just the contents: a digest
        // over bare numbers says nothing about where they came from.
        buf.extend_from_slice(path.as_bytes());
        buf.push(b'\n');
        count += bytes.iter().filter(|b| **b == b'\n').count() as u64;
        buf.extend_from_slice(&bytes);
        buf.push(b'\n');
    }
    if !buf.is_empty() {
        return Ok((buf, count, PROCFS_ENCODING));
    }

    Err(best_reason.unwrap_or(MissedLayer {
        layer: format!("{:?}", src.layer).to_lowercase(),
        source: src.roots.join(", "),
        reason: "none of these paths exist on this machine".into(),
    }))
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
pub fn boot_id() -> String {
    std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "unknown".into())
}

/// Which registered profiles this capture would satisfy.
///
/// Coverage only: the layers present against each profile's requirements. The
/// full verifier also checks the chain, the root, the window and the
/// signature, and the decoder runs it before citing anything. This answers the
/// one question a refusal does not: was any profile reachable from what this
/// machine can read?
fn profiles_satisfied_by(trace: &OsTrace) -> Vec<String> {
    let present: std::collections::HashSet<_> = trace.segments.iter().map(|s| s.layer).collect();
    emem_core::substrates::DEFAULT
        .substrates
        .iter()
        .filter(|p| p.admission == emem_core::substrates::AdmissionRule::OsTraceRequired)
        .filter(|p| p.required_trace_layers.iter().all(|l| present.contains(l)))
        .map(|p| p.id.clone())
        .collect()
}

/// What this trace supports, and what it does not, in a sentence an operator
/// can act on.
fn admissibility_line(
    trace: &OsTrace,
    missed: &[MissedLayer],
    accepts: &[String],
    configured: &str,
) -> String {
    let n = trace.segments.len();
    if accepts.is_empty() {
        return format!(
            "{n} layer(s) captured, {} absent, and NO registered profile accepts this \
             combination. The trace is signed, chained and binds its payload digests, but no \
             verifier will admit it. Every profile but one requires sensor_bus, signal or \
             inference, which this encoder has no source for on any machine; the exception \
             requires syscall, which needs a tracefs mount and the capability to read it. If this \
             host grants neither, host.counters.v1 is the profile built for it.",
            missed.len()
        );
    }
    let mine = accepts.iter().any(|p| p == configured);
    format!(
        "{n} layer(s) captured, {} absent. Accepted by: {}.{}",
        missed.len(),
        accepts.join(", "),
        if mine {
            String::new()
        } else {
            format!(
                " NOT by --profile {configured}, which this capture does not cover; a verifier \
                 will refuse it under that profile and be right to."
            )
        }
    )
}

/// Capture one window and write a signed trace.
pub fn capture_window(
    key: &SigningKey,
    settings: &CaptureSettings,
) -> std::io::Result<CaptureReport> {
    let clock = MonotonicClock::start();
    let window_start_ns = clock.now();

    let mut segments: Vec<TraceSegment> = Vec::new();
    let mut captured = Vec::new();
    let mut missed = Vec::new();
    // seq counts EMITTED segments, not attempted sources.
    //
    // It was the source index, which left holes whenever a layer could not be
    // read: capture five of seven and the sequence ran 0, 2, 3, 5, 6. The
    // verifier requires seq contiguous from zero and rejected every trace this
    // encoder produced. Nothing local caught it, because nothing local ran the
    // verifier; the deployed one found it on the first trace it was shown.
    let mut seq = 0u64;
    // And each segment must name the previous segment's digest. Every one of
    // these was None, so the chain was broken at every link. Same story: a
    // chain nothing checked is a chain nobody noticed was missing.
    let mut prev_digest: Option<String> = None;
    for src in SOURCES.iter() {
        // Per source, so a segment's clocks describe reading that source
        // rather than the whole window. Every segment used to carry the
        // window's own start and a fabricated one-nanosecond end.
        let seg_start = clock.now();
        match capture(src) {
            Ok((bytes, events, encoding)) => {
                let seg = TraceSegment {
                    layer: src.layer,
                    seq,
                    clock_start_ns: seg_start,
                    clock_end_ns: clock.now(),
                    event_count: events,
                    log_digest: b32(blake3::hash(&bytes).as_bytes()),
                    prev_digest: prev_digest.clone(),
                    encoding: encoding.to_string(),
                };
                prev_digest = Some(b32(&seg
                    .digest()
                    .map_err(|e| std::io::Error::other(format!("segment digest: {e}")))?));
                segments.push(seg);
                seq += 1;
                captured.push(format!("{:?}", src.layer).to_lowercase());
            }
            Err(m) => missed.push(m),
        }
    }

    let unsupported: Vec<MissedLayer> = UNSUPPORTED
        .iter()
        .map(|(layer, reason)| MissedLayer {
            layer: (*layer).to_string(),
            source: "none".into(),
            reason: (*reason).to_string(),
        })
        .collect();

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
                    // When this encoder saw the payload, which is what it can
                    // honestly say. It did not witness the write, so it does
                    // not claim to. Every output used to carry the window's
                    // start, which reads as a measurement and was not one.
                    emitted_at_ns: clock.now(),
                    layer: TraceLayerKind::SensorBus,
                });
            }
        }
    }

    // Closed after the outputs are read, so the window genuinely covers
    // everything the trace describes. Reading them after the window closed put
    // their timestamps outside it.
    let window_end_ns = clock.now();
    if window_end_ns <= window_start_ns {
        // Reported, never papered over. The previous code wrote
        // `start.max(start + 1)`, which satisfies the verifier's requirement
        // that a window be non-empty by inventing the one nanosecond it could
        // not measure. A window is the trace's central claim about when it
        // happened; a node that cannot measure one must say so rather than
        // supply a plausible number.
        return Err(std::io::Error::other(format!(
            "the monotonic clock did not advance across the capture ({window_start_ns} to \
             {window_end_ns}). No trace was written: a window is what a trace claims about \
             when it ran, and this node cannot measure one."
        )));
    }

    if segments.is_empty() {
        // No fabrication, and no empty trace either: verify_os_trace rejects
        // one, so writing it would only move the failure downstream.
        return Ok(CaptureReport {
            trace_cid: None,
            captured,
            missed,
            unsupported,
            outputs: outputs.len(),
            accepted_by: Vec::new(),
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

    let accepts = profiles_satisfied_by(&trace);
    let admissibility = admissibility_line(&trace, &missed, &accepts, &settings.profile);
    Ok(CaptureReport {
        trace_cid: Some(cid),
        captured,
        missed,
        unsupported,
        outputs: trace.outputs.len(),
        accepted_by: accepts,
        admissibility,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("emem-encode-{name}"));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A head from THIS boot is the chain to continue.
    #[test]
    fn a_head_from_this_boot_is_resumed() {
        let d = tmp("resume");
        let p = StreamHead::path(&d);
        let head = StreamHead {
            boot_id: boot_id(),
            trace_cid: "abc".into(),
            windows: 7,
        };
        head.save(&p).unwrap();
        let loaded = StreamHead::load_for_this_boot(&p).expect("same boot resumes");
        assert_eq!(loaded.trace_cid, "abc");
        assert_eq!(loaded.windows, 7);
    }

    /// A head from a PREVIOUS boot must not be chained to.
    ///
    /// Chaining across a reboot would assert a continuity that did not happen:
    /// the kernel that ran the earlier window is gone. A fresh boot is a fresh
    /// stream, and the gate on the ground expects exactly that.
    #[test]
    fn a_head_from_an_earlier_boot_starts_a_fresh_stream() {
        let d = tmp("reboot");
        let p = StreamHead::path(&d);
        let stale = StreamHead {
            boot_id: "a-boot-that-has-since-ended".into(),
            trace_cid: "abc".into(),
            windows: 99,
        };
        stale.save(&p).unwrap();
        assert!(
            StreamHead::load_for_this_boot(&p).is_none(),
            "a head from another boot must be ignored, not chained to"
        );
        // And the file is left alone rather than deleted: an operator
        // reconstructing what a device did wants the old head, not a gap.
        assert!(p.exists(), "the stale head is kept for forensics");
    }

    /// Debris must not be mistaken for a chain.
    #[test]
    fn an_unreadable_head_is_not_a_chain() {
        let d = tmp("garbage");
        let p = StreamHead::path(&d);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, b"{ not json").unwrap();
        assert!(StreamHead::load_for_this_boot(&p).is_none());
    }

    /// Run the REAL verifier over what the encoder actually produces.
    ///
    /// This test exists because its absence cost us. Every trace this encoder
    /// wrote was structurally invalid, in two ways at once: seq was the source
    /// index so it had holes whenever a layer could not be read, and every
    /// segment's prev_digest was None so the chain was broken at every link.
    /// Nothing local noticed, because nothing local ran the verifier. The
    /// deployed one rejected the first trace it was ever shown.
    ///
    /// So the encoder's output now meets its own verifier in the test suite,
    /// which is where that should have been checked from the beginning.
    #[test]
    fn what_the_encoder_produces_survives_the_real_verifier() {
        use emem_core::substrates::DEFAULT as SUBSTRATES;
        use emem_trace::{verify_os_trace, Verdict};

        let d = tmp("verify");
        std::fs::create_dir_all(d.join("out")).unwrap();
        std::fs::write(d.join("payload.bin"), b"an output of this window").unwrap();

        let key = SigningKey::from_bytes(&[13u8; 32]);
        let settings = CaptureSettings {
            out: d.join("out"),
            payloads: Some(d.clone()),
            // Three layers this machine can genuinely read, so the test turns
            // on structure rather than on what the CI host happens to expose.
            profile: "exec.trace.v1".into(),
            platform: "generic.linux-host".into(),
            prev_trace_cid: None,
        };
        let report = capture_window(&key, &settings).expect("capture runs");
        let Some(cid) = report.trace_cid else {
            // A host exposing nothing at all is a real possibility in CI, and
            // the encoder correctly writes no trace. Nothing to verify.
            return;
        };

        let raw = std::fs::read(d.join("out").join(format!("{cid}.trace.json"))).unwrap();
        let trace: OsTrace = serde_json::from_slice(&raw).unwrap();

        // Structure first, independent of which layers this host allowed.
        for (i, seg) in trace.segments.iter().enumerate() {
            assert_eq!(seg.seq, i as u64, "seq must be contiguous from zero");
            if i == 0 {
                assert!(
                    seg.prev_digest.is_none(),
                    "the first segment starts the chain"
                );
            } else {
                let want = b32(&trace.segments[i - 1].digest().unwrap());
                assert_eq!(
                    seg.prev_digest.as_deref(),
                    Some(want.as_str()),
                    "segment {i} must name the previous segment's digest"
                );
            }
        }

        // Then the verifier itself, on whatever profile the capture can meet.
        let profile = SUBSTRATES
            .lookup("exec.trace.v1")
            .expect("exec.trace.v1 is registered");
        let report = verify_os_trace(&trace, profile, None);
        let structural: Vec<_> = report
            .reasons
            .iter()
            .filter(|r| !format!("{r:?}").contains("MissingLayer"))
            .collect();
        assert!(
            structural.is_empty(),
            "the only acceptable rejection is an absent layer, got: {structural:?}"
        );
        if report.coverage.missing.is_empty() {
            assert_eq!(report.verdict, Verdict::Admit, "full coverage must admit");
        }
    }

    /// The capture rule, stated as a test: a layer with no readable source is
    /// absent and explained, never invented.
    /// A capture must be able to satisfy at least one profile.
    ///
    /// Before host.counters.v1 existed, none of them could be satisfied by any
    /// Linux host: every other trace profile requires sensor_bus, signal or
    /// inference, for which this encoder has no source anywhere, and the one
    /// exception requires syscall, which needs a tracefs mount. A hosted
    /// payload with neither produced a signed, chained, payload-binding trace
    /// that no verifier would admit. Reported from a real deployment.
    #[test]
    fn what_this_encoder_captures_satisfies_a_real_profile() {
        let d = tmp("accepts");
        std::fs::create_dir_all(d.join("out")).unwrap();
        let key = SigningKey::from_bytes(&[23u8; 32]);
        let settings = CaptureSettings {
            out: d.join("out"),
            payloads: None,
            profile: "host.counters.v1".into(),
            platform: "generic.linux-host".into(),
            prev_trace_cid: None,
        };
        let report = capture_window(&key, &settings).expect("capture runs");
        if report.trace_cid.is_none() {
            return; // a host exposing nothing writes no trace, correctly
        }
        assert!(
            report.accepted_by.contains(&"host.counters.v1".to_string()),
            "the profile built for a plain Linux host did not accept a plain Linux capture. \
             captured {:?}, accepted by {:?}",
            report.captured,
            report.accepted_by
        );
        assert!(
            report.admissibility.contains("Accepted by"),
            "the report must name the profiles that would take this: {}",
            report.admissibility
        );
    }

    /// Capturing under a profile the capture cannot cover says so, by name.
    #[test]
    fn a_capture_that_misses_its_own_profile_says_which_ones_would_take_it() {
        let d = tmp("mismatch");
        std::fs::create_dir_all(d.join("out")).unwrap();
        let key = SigningKey::from_bytes(&[29u8; 32]);
        let settings = CaptureSettings {
            out: d.join("out"),
            payloads: None,
            // Requires sensor_bus and signal, which have no source anywhere.
            profile: "orbital.satellite.v1".into(),
            platform: "nvidia.jetson-orin".into(),
            prev_trace_cid: None,
        };
        let report = capture_window(&key, &settings).expect("capture runs");
        if report.trace_cid.is_none() {
            return;
        }
        assert!(
            !report
                .accepted_by
                .contains(&"orbital.satellite.v1".to_string()),
            "a capture missing sensor_bus must not claim that profile"
        );
        assert!(
            report
                .admissibility
                .contains("NOT by --profile orbital.satellite.v1"),
            "the operator must be told their profile is the wrong one: {}",
            report.admissibility
        );
    }

    /// The stream head goes beside the traces, so a read-only rootfs is fine.
    #[test]
    fn the_stream_head_lives_with_the_traces_not_the_identity() {
        let d = tmp("headloc");
        let out = d.join("out");
        let p = StreamHead::path(&out);
        assert!(
            p.starts_with(&out),
            "the head must sit under --out, not --data: {}",
            p.display()
        );
        // save() makes its own directory, because the encoder used to write
        // the trace and then fail on this last step.
        StreamHead {
            boot_id: boot_id(),
            trace_cid: "abc".into(),
            windows: 1,
        }
        .save(&p)
        .expect("save must create .state itself");
        assert!(p.exists());
        // And it is in a subdirectory, which the decoder's trace scan skips,
        // so it is never counted as debris.
        assert_eq!(p.parent().unwrap().file_name().unwrap(), ".state");
    }

    /// Two captures in a row must produce two different traces.
    ///
    /// They produced one. Four encoders capturing four different windows on a
    /// live machine wrote byte-identical records and three silently overwrote
    /// the first, because the only clock was `/proc/uptime`, which is written
    /// to two decimal places, and the recorded window was
    /// `start .. start + 1` however long the capture actually ran.
    ///
    /// The lost files were the smaller half. A trace whose bytes do not depend
    /// on when it was captured can be replayed as evidence of any later
    /// window, which is the one thing an execution trace exists to prevent.
    #[test]
    fn two_captures_in_a_row_are_two_different_traces() {
        let d = tmp("distinct");
        std::fs::create_dir_all(d.join("out")).unwrap();
        let key = SigningKey::from_bytes(&[17u8; 32]);
        let settings = CaptureSettings {
            out: d.join("out"),
            payloads: None,
            profile: "exec.trace.v1".into(),
            platform: "generic.linux-host".into(),
            prev_trace_cid: None,
        };

        let a = capture_window(&key, &settings).expect("first capture");
        let b = capture_window(&key, &settings).expect("second capture");
        let (Some(ca), Some(cb)) = (a.trace_cid, b.trace_cid) else {
            // A host exposing no layer at all writes no trace, correctly.
            return;
        };
        assert_ne!(
            ca, cb,
            "two captures produced one content id, so the record does not depend on when it \
             was taken and can be replayed as evidence of a window it did not observe"
        );
    }

    /// The recorded window must be the window that was measured.
    #[test]
    fn the_recorded_window_is_measured_not_fabricated() {
        let d = tmp("window");
        std::fs::create_dir_all(d.join("out")).unwrap();
        std::fs::write(d.join("payload.bin"), b"an output of this window").unwrap();
        let key = SigningKey::from_bytes(&[19u8; 32]);
        let settings = CaptureSettings {
            out: d.join("out"),
            payloads: Some(d.clone()),
            profile: "exec.trace.v1".into(),
            platform: "generic.linux-host".into(),
            prev_trace_cid: None,
        };
        let Some(cid) = capture_window(&key, &settings).expect("capture").trace_cid else {
            return;
        };
        let raw = std::fs::read(d.join("out").join(format!("{cid}.trace.json"))).unwrap();
        let t: OsTrace = serde_json::from_slice(&raw).unwrap();

        let span = t.window_end_ns - t.window_start_ns;
        assert!(
            span > 1,
            "the window is {span} ns wide, which is the fabricated minimum rather than a \
             measurement: reading /proc takes longer than that"
        );

        for seg in &t.segments {
            assert!(
                seg.clock_start_ns >= t.window_start_ns && seg.clock_end_ns <= t.window_end_ns,
                "segment {} runs outside the window it belongs to",
                seg.seq
            );
            assert!(
                seg.clock_end_ns >= seg.clock_start_ns,
                "segment {} ends before it starts",
                seg.seq
            );
        }
        // Outputs are read before the window closes, so their timestamps fall
        // inside it. Reading them afterwards put every one of them outside.
        for out in &t.outputs {
            assert!(
                out.emitted_at_ns >= t.window_start_ns && out.emitted_at_ns <= t.window_end_ns,
                "an output is stamped outside the window"
            );
        }
    }

    #[test]
    fn an_unreadable_source_yields_an_explained_absence_not_a_segment() {
        let missing = Source {
            layer: TraceLayerKind::Syscall,
            encoding: "linux.ftrace.v1",
            roots: &["/definitely/not/a/path/on/any/machine"],
            procfs: &[],
        };
        let err = capture(&missing).expect_err("no source means no segment");
        assert_eq!(err.layer, "syscall");
        assert!(
            err.reason.contains("exist"),
            "the reason must say what was wrong: {}",
            err.reason
        );
    }

    /// A layer with an unprivileged fallback captures where the rich source is
    /// unavailable, and says which one it used.
    ///
    /// The whole encoder produced nothing on a host granting no tracefs and no
    /// capabilities: one layer captured out of ten, so no trace was written at
    /// all. Reported from a real deployment as dead weight in orbit.
    #[test]
    fn a_layer_with_a_counter_fallback_captures_and_labels_it_honestly() {
        let fallback = Source {
            layer: TraceLayerKind::Memory,
            encoding: "linux.ftrace.v1",
            roots: &["/definitely/not/a/path/on/any/machine"],
            procfs: &["/proc/meminfo", "/proc/vmstat"],
        };
        let (bytes, events, encoding) = capture(&fallback).expect("/proc/meminfo exists on Linux");
        assert!(!bytes.is_empty());
        assert!(events > 0, "a counter file has lines");
        assert_eq!(
            encoding, PROCFS_ENCODING,
            "a counter snapshot must never be labelled as an event log"
        );
        // The path is part of what was captured: a digest over bare numbers
        // says nothing about where they came from.
        assert!(
            String::from_utf8_lossy(&bytes).contains("/proc/meminfo"),
            "the capture must name its source"
        );

        // And a layer with no fallback still fails, so this is not a blanket
        // "always find something" change: syscall stays absent without
        // tracing, which is what keeps a /proc-only trace inadmissible.
        let no_fallback = Source {
            layer: TraceLayerKind::Syscall,
            encoding: "linux.ftrace.v1",
            roots: &["/definitely/not/a/path/on/any/machine"],
            procfs: &[],
        };
        assert!(capture(&no_fallback).is_err());
    }
}
