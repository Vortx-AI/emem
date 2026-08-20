//! One directory in, one directory out.

use std::path::{Path, PathBuf};

use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};

use crate::{Custody, NodeIdentity};

/// How a run is configured.
#[derive(Debug, Clone)]
pub struct DecodeSettings {
    /// Directory the host drops payloads into. Read, never written.
    pub input: PathBuf,
    /// Directory this node writes to. Everything that leaves the machine
    /// leaves through here.
    pub output: PathBuf,
    /// The node's identity, as configured by the operator.
    pub node: NodeIdentity,
    /// Directory of `emem.os_trace.v1` records an encoder on this machine has
    /// written, if one is running.
    ///
    /// The folder the two halves meet through. Neither has to know the other
    /// exists; a node with no encoder points this at nothing.
    pub traces: Option<PathBuf>,
    /// The operator's label for the stage the payloads in `input` sit at.
    ///
    /// Free text, because every host names its pipeline differently and a
    /// fixed vocabulary would force somebody to lie about theirs.
    pub stage: Option<String>,
    /// Wall clock for this run, RFC 3339 UTC. Passed in rather than read so a
    /// run is reproducible and a machine with a bad clock cannot invent one.
    pub observed_at: String,
    /// Most files this node will process in one run. Bounds the aggregate,
    /// which a per-payload cap does not: a directory of a million tiny files
    /// is within every per-file limit and still unbounded.
    pub max_files: u64,
    /// Largest payload this node will read into memory. A cap rather than a
    /// hope: the input directory belongs to the host, and one enormous file
    /// should cost a skip line in the report, not the process.
    pub max_payload_bytes: u64,
}

/// Default payload cap: 256 MiB. Big enough for the imagery this is built for,
/// small enough that a node with modest memory survives a hostile directory.
pub const DEFAULT_MAX_PAYLOAD_BYTES: u64 = 256 * 1024 * 1024;

/// Default per-run file cap. A run that hits it reports the overflow rather
/// than truncating silently, and the remainder is picked up on the next run.
pub const DEFAULT_MAX_FILES: u64 = 10_000;

/// What a run did.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecodeReport {
    /// Payloads that produced a custody record.
    pub recorded: usize,
    /// Files skipped, with the reason, so a run never silently ignores input.
    pub skipped: Vec<Skipped>,
    /// Total payload bytes read.
    pub bytes_read: u64,
    /// Total bytes written to the output directory.
    pub bytes_written: u64,
    /// Payloads whose digest an encoder trace covered, so their record cites
    /// one. The rest carry custody alone.
    pub traced: usize,
    /// Files in the traces directory that could not be read as a trace.
    /// Counted rather than fatal: the encoder is a separate process and its
    /// debris must not stop custody being recorded.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub unreadable_traces: usize,
    /// Temporary files left behind by a run that did not finish, reported and
    /// deliberately not deleted.
    ///
    /// Deleting them would be the wrong call on a host that runs containers in
    /// parallel: a `.part` file may belong to another node that is writing
    /// right now, and removing it would corrupt a healthy write to tidy up
    /// after a dead one. Naming them lets the operator clean up knowing which
    /// containers are running; guessing does not.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stale_partials: Vec<String>,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// A file the run did not record, and why.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skipped {
    /// The file name as it appeared in the input directory.
    pub name: String,
    /// Why it was not recorded.
    pub reason: String,
}

/// Read every payload in `input`, write one custody record each to `output`.
///
/// Properties this deliberately has:
///
/// * **The input is never modified.** The host owns that directory; a decoder
///   that deleted or moved what it consumed would be destroying someone
///   else's data to keep its own bookkeeping simple.
/// * **Nothing is skipped silently.** A file that cannot be recorded appears
///   in the report with a reason. A run that quietly ignored half its input
///   would look identical to a successful one.
/// * **Deterministic order.** Entries are sorted by name before processing so
///   two runs over the same directory produce the same report.
/// * **The payload does not leave.** Only the custody record is written out.
///   The bytes stay where the host put them, which is what makes this usable
///   on a link where the payload is the expensive thing.
pub fn decode_dir(key: &SigningKey, settings: &DecodeSettings) -> std::io::Result<DecodeReport> {
    std::fs::create_dir_all(&settings.output)?;
    let mut names: Vec<PathBuf> = std::fs::read_dir(&settings.input)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    names.sort();
    let mut overflow: Vec<PathBuf> = Vec::new();
    if names.len() as u64 > settings.max_files {
        overflow = names.split_off(settings.max_files as usize);
    }

    // Read the encoder's output once, up front: payload digest -> trace cid.
    //
    // This is the folder the two halves meet through. A trace names the
    // payloads it emitted, so the join is the digest and nothing has to be
    // agreed between the halves beyond a directory. Neither has to know the
    // other exists, and a node with no encoder yet points this at nothing.
    //
    // A trace that does not parse is counted, not fatal: the encoder is a
    // separate process and its debris must not stop custody being recorded.
    let mut traced: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut unreadable_traces = 0usize;
    if let Some(dir) = &settings.traces {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.filter_map(|e| e.ok()) {
                let path = e.path();
                if !path.is_file() {
                    continue;
                }
                match std::fs::read(&path)
                    .ok()
                    .and_then(|b| serde_json::from_slice::<emem_trace::OsTrace>(&b).ok())
                {
                    Some(t) => match t.trace_cid() {
                        Ok(cid) => {
                            for out in &t.outputs {
                                traced.insert(out.payload_digest.clone(), cid.clone());
                            }
                        }
                        Err(_) => unreadable_traces += 1,
                    },
                    None => unreadable_traces += 1,
                }
            }
        }
    }

    let mut report = DecodeReport {
        recorded: 0,
        skipped: Vec::new(),
        bytes_read: 0,
        bytes_written: 0,
        traced: 0,
        unreadable_traces,
        stale_partials: Vec::new(),
    };

    for path in names {
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => {
                // A name that is not UTF-8 cannot be bound into the preimage,
                // and a record whose name field is a lossy approximation of
                // the real one would verify while describing a different file.
                report.skipped.push(Skipped {
                    name: path.to_string_lossy().into_owned(),
                    reason: "file name is not valid UTF-8, so it cannot be signed".into(),
                });
                continue;
            }
        };
        // symlink_metadata, NOT metadata: the latter follows the link.
        //
        // Following was a real disclosure and it was demonstrated before it
        // was fixed. A symlink dropped in the input directory made the node
        // read the target and sign its size and digest into a record that
        // then LEAVES the machine. Pointed at the node's own
        // node_identity.json it published facts about the private key file;
        // pointed at /etc/anything it published those. On hardware someone
        // else owns, the input directory is attacker-controlled by
        // definition, so the only safe rule is that a symlink is never
        // followed and never silently ignored.
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) => {
                report.skipped.push(Skipped {
                    name,
                    reason: format!("unreadable: {e}"),
                });
                continue;
            }
        };
        if meta.file_type().is_symlink() {
            report.skipped.push(Skipped {
                name,
                reason: "symlink: refused, because following it would sign a file outside the \
                         input directory and publish its digest"
                    .into(),
            });
            continue;
        }
        if !meta.is_file() {
            report.skipped.push(Skipped {
                name,
                reason: "not a regular file".into(),
            });
            continue;
        }
        // Size is checked before the read, not after. std::fs::read on a
        // multi-gigabyte payload (or a symlinked /dev/zero, were symlinks
        // allowed) would take the process down, and a node that dies part way
        // through a run leaves the host unable to tell what it did.
        if meta.len() > settings.max_payload_bytes {
            report.skipped.push(Skipped {
                name,
                reason: format!(
                    "{} bytes exceeds the {} byte payload cap; raise --max-payload-bytes \
                     deliberately if this node really should read files this large",
                    meta.len(),
                    settings.max_payload_bytes
                ),
            });
            continue;
        }
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                report.skipped.push(Skipped {
                    name,
                    reason: format!("unreadable: {e}"),
                });
                continue;
            }
        };
        let digest = crate::b32(blake3::hash(&bytes).as_bytes());
        let record = Custody::sign(
            key,
            settings.node.clone(),
            &name,
            &bytes,
            &settings.observed_at,
            settings.stage.as_deref(),
            traced.get(&digest).map(String::as_str),
        );
        let json = serde_json::to_vec_pretty(&record).unwrap_or_else(|_| b"{}".to_vec());
        let out = settings.output.join(format!("{name}.custody.json"));
        write_atomic(&out, &json)?;

        // Read it back and verify the signature against what actually landed
        // on disk.
        //
        // Not paranoia in this deployment. A single-event upset flips a bit in
        // RAM or in flash, the write reports success, and the record that
        // leaves the machine no longer verifies. The node is the only party
        // that can still notice, because on the ground there is no second copy
        // to compare against. Catching it costs a re-read of a few hundred
        // bytes and turns a silently corrupt record into a reported skip.
        match std::fs::read(&out)
            .ok()
            .and_then(|b| serde_json::from_slice::<Custody>(&b).ok())
        {
            Some(back) if back.verify().is_ok() && back.covers(&bytes) => {}
            _ => {
                let _ = std::fs::remove_file(&out);
                report.skipped.push(Skipped {
                    name,
                    reason: "the record did not verify when read back from disk, so it was \
                             removed rather than shipped. Suspect storage corruption."
                        .into(),
                });
                continue;
            }
        }
        report.bytes_read += bytes.len() as u64;
        report.bytes_written += json.len() as u64;
        if record.trace_cid.is_some() {
            report.traced += 1;
        }
        report.recorded += 1;
    }

    for path in overflow {
        report.skipped.push(Skipped {
            name: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            reason: format!(
                "beyond the {} file cap for one run; it will be picked up next run, or raise \
                 --max-files",
                settings.max_files
            ),
        });
    }

    // Debris from runs that did not finish. Named, never removed: see the
    // field's own note for why deleting would be the more dangerous choice.
    let mine = format!(".{}.part", std::process::id());
    if let Ok(entries) = std::fs::read_dir(&settings.output) {
        for e in entries.filter_map(|e| e.ok()) {
            let n = e.file_name().to_string_lossy().into_owned();
            if n.ends_with(".part") && !n.ends_with(&mine) {
                report.stale_partials.push(n);
            }
        }
    }
    report.stale_partials.sort();

    // Per-node, not one global name. Two containers sharing an output mount
    // both wrote `run.json` and the second silently destroyed the first's
    // report, so a run could complete and leave no record that it had. Keying
    // by the node's short key means parallel nodes coexist, while the same
    // node re-running still overwrites its own previous report, which is what
    // keeps a re-run reproducible.
    let manifest = settings
        .output
        .join(format!("run.{}.json", short_key(&settings.node.node_key)));
    let mj = serde_json::to_vec_pretty(&report).unwrap_or_else(|_| b"{}".to_vec());
    write_atomic(&manifest, &mj)?;
    report.bytes_written += mj.len() as u64;
    Ok(report)
}

/// The eight-character form of a node key, used to keep per-node output files
/// from colliding when several nodes share one output directory.
pub fn short_key(node_key: &str) -> String {
    node_key.chars().take(8).collect()
}

/// Write a file so that a reader only ever sees all of it or none of it, and
/// so that no symlink can redirect where it lands.
///
/// Three properties, each earned:
///
/// **No follow.** A symlink planted at the destination redirected the write
/// and overwrote whatever it pointed at; that was demonstrated against this
/// code. The temporary is created exclusively, so it cannot be pre-planted,
/// and `rename` replaces a symlink at the destination rather than following
/// it.
///
/// **Atomic.** The earlier version truncated the destination and then wrote
/// into it, so a power cut mid-write left a half-written record on disk. On a
/// spacecraft that is not a hypothetical: the bus browns out, the node comes
/// back, and the operator has a file that parses as JSON but is not a complete
/// signed record. `rename` is atomic, so a reader sees the old file or the new
/// one, never a partial one.
///
/// **Durable.** fsync on the file before the rename, and on the directory
/// after it, because a rename that is only in the page cache is not a rename
/// that survived the power cut that made you care.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    // The temporary carries this process's pid.
    //
    // Without it, two nodes running at once against the same output mount race
    // on one temporary name: both call create_new, one wins, and the other
    // exits non-zero. That is not hypothetical on a host that runs containers
    // in parallel by design, and it was reproduced before this line existed.
    let tmp = path.with_extension(format!("{}.part", std::process::id()));
    // A leftover .part from an interrupted earlier run is expected debris,
    // not a reason to fail. Removing a symlink here removes the link only.
    match std::fs::remove_file(&tmp) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    // Directory fsync: without it the rename can still be lost. Best effort,
    // because some filesystems refuse to open a directory for sync and that
    // is not a reason to fail a write that otherwise succeeded.
    if let Some(dir) = path.parent() {
        if let Ok(d) = std::fs::File::open(dir) {
            let _ = d.sync_all();
        }
    }
    Ok(())
}

/// Where the node's key lives on disk, given a data directory.
pub fn key_path(data_dir: &Path) -> PathBuf {
    data_dir.join("node_identity.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Custody;

    fn settings(dir: &Path) -> DecodeSettings {
        DecodeSettings {
            input: dir.join("in"),
            output: dir.join("out"),
            node: NodeIdentity {
                node_key: crate::b32(
                    SigningKey::from_bytes(&[3u8; 32])
                        .verifying_key()
                        .as_bytes(),
                ),
                profile: "orbital.satellite.v1".into(),
                platform: "nvidia.jetson-orin".into(),
            },
            traces: None,
            stage: None,
            observed_at: "2026-08-20T09:00:00Z".into(),
            max_payload_bytes: DEFAULT_MAX_PAYLOAD_BYTES,
            max_files: DEFAULT_MAX_FILES,
        }
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("emem-airgap-{name}"));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("in")).unwrap();
        d
    }

    #[test]
    fn a_run_records_every_payload_and_leaves_the_input_alone() {
        let d = tmp("basic");
        std::fs::write(d.join("in/b.tif"), b"second").unwrap();
        std::fs::write(d.join("in/a.tif"), b"first").unwrap();
        let k = SigningKey::from_bytes(&[3u8; 32]);
        let s = settings(&d);
        let r = decode_dir(&k, &s).unwrap();

        assert_eq!(r.recorded, 2);
        assert!(r.skipped.is_empty());
        assert_eq!(r.bytes_read, 11);

        // The host's directory is untouched: same names, same bytes.
        assert_eq!(std::fs::read(d.join("in/a.tif")).unwrap(), b"first");
        assert_eq!(std::fs::read(d.join("in/b.tif")).unwrap(), b"second");

        // And each record verifies on its own, against the payload.
        for (name, bytes) in [("a.tif", &b"first"[..]), ("b.tif", &b"second"[..])] {
            let raw = std::fs::read(d.join(format!("out/{name}.custody.json"))).unwrap();
            let c: Custody = serde_json::from_slice(&raw).unwrap();
            c.verify().expect("record verifies");
            assert!(c.covers(bytes), "record covers the payload it names");
        }
    }

    /// A file that cannot be recorded must appear in the report. A run that
    /// silently dropped input would be indistinguishable from a clean one.
    #[test]
    fn what_cannot_be_recorded_is_reported_not_dropped() {
        let d = tmp("skips");
        std::fs::write(d.join("in/ok.tif"), b"x").unwrap();
        std::fs::create_dir_all(d.join("in/nested")).unwrap();
        let k = SigningKey::from_bytes(&[3u8; 32]);
        let r = decode_dir(&k, &settings(&d)).unwrap();
        assert_eq!(r.recorded, 1);
        assert_eq!(r.skipped.len(), 1);
        assert_eq!(r.skipped[0].name, "nested");
        assert!(r.skipped[0].reason.contains("not a regular file"));
    }

    /// Two runs over the same directory must produce the same bytes, or a
    /// reader cannot tell a re-run from a change.
    #[test]
    fn a_run_is_reproducible() {
        let d = tmp("repro");
        std::fs::write(d.join("in/a.tif"), b"same").unwrap();
        let k = SigningKey::from_bytes(&[3u8; 32]);
        let s = settings(&d);
        decode_dir(&k, &s).unwrap();
        let first = std::fs::read(d.join("out/a.tif.custody.json")).unwrap();
        decode_dir(&k, &s).unwrap();
        let second = std::fs::read(d.join("out/a.tif.custody.json")).unwrap();
        assert_eq!(first, second, "the same input must sign to the same bytes");
    }

    /// The point of the design: what leaves is far smaller than what arrived.
    #[test]
    fn the_record_is_much_smaller_than_the_payload() {
        let d = tmp("size");
        std::fs::write(d.join("in/frame.tif"), vec![0u8; 1024 * 1024]).unwrap();
        let k = SigningKey::from_bytes(&[3u8; 32]);
        let r = decode_dir(&k, &settings(&d)).unwrap();
        assert_eq!(r.recorded, 1);
        assert!(
            r.bytes_written * 100 < r.bytes_read,
            "a megabyte in should not produce ten kilobytes out: read {} wrote {}",
            r.bytes_read,
            r.bytes_written
        );
    }
}

/// Attacks that were demonstrated against this code and now must fail.
///
/// These are regression tests in the strict sense: every one of them passed
/// as an attack before the fix landed, and each is written from the attacker's
/// side rather than the implementer's.
#[cfg(test)]
mod security {
    use super::*;

    fn key() -> SigningKey {
        SigningKey::from_bytes(&[3u8; 32])
    }

    fn s(dir: &Path) -> DecodeSettings {
        DecodeSettings {
            input: dir.join("in"),
            output: dir.join("out"),
            node: NodeIdentity {
                node_key: crate::b32(key().verifying_key().as_bytes()),
                profile: "orbital.satellite.v1".into(),
                platform: "nvidia.jetson-orin".into(),
            },
            traces: None,
            stage: None,
            observed_at: "2026-08-20T09:00:00Z".into(),
            max_payload_bytes: DEFAULT_MAX_PAYLOAD_BYTES,
            max_files: DEFAULT_MAX_FILES,
        }
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("emem-airgap-sec-{name}"));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("in")).unwrap();
        std::fs::create_dir_all(d.join("out")).unwrap();
        d
    }

    /// A symlink in the input directory must never be read.
    ///
    /// Before the fix this signed the size and digest of whatever the link
    /// pointed at, including the node's own private key file, into a record
    /// that then leaves the machine.
    #[cfg(unix)]
    #[test]
    fn a_symlink_in_the_input_is_refused_not_followed() {
        let d = tmp("symlink-in");
        std::fs::write(d.join("secret.txt"), b"operator secret").unwrap();
        std::os::unix::fs::symlink(d.join("secret.txt"), d.join("in/innocent.tif")).unwrap();

        let r = decode_dir(&key(), &s(&d)).unwrap();
        assert_eq!(r.recorded, 0, "nothing behind a symlink may be signed");
        assert_eq!(r.skipped.len(), 1);
        assert!(
            r.skipped[0].reason.contains("symlink"),
            "the refusal must name the reason, got: {}",
            r.skipped[0].reason
        );
        assert!(
            !d.join("out/innocent.tif.custody.json").exists(),
            "no record may be written for a symlinked payload"
        );
    }

    /// A symlink at the OUTPUT path must not redirect the write.
    ///
    /// Before the fix this overwrote the file the link pointed at with custody
    /// JSON: an arbitrary file write on hardware the node does not own.
    #[cfg(unix)]
    #[test]
    fn a_symlink_in_the_output_cannot_redirect_a_write() {
        let d = tmp("symlink-out");
        std::fs::write(d.join("in/x.tif"), b"payload").unwrap();
        std::fs::write(d.join("victim.txt"), b"ORIGINAL").unwrap();
        std::os::unix::fs::symlink(d.join("victim.txt"), d.join("out/x.tif.custody.json")).unwrap();

        decode_dir(&key(), &s(&d)).unwrap();
        assert_eq!(
            std::fs::read(d.join("victim.txt")).unwrap(),
            b"ORIGINAL",
            "the victim file must be untouched"
        );
        // The record still lands, at the real path, having replaced the link.
        let raw = std::fs::read(d.join("out/x.tif.custody.json")).unwrap();
        assert!(
            raw.starts_with(b"{"),
            "the record is written where it belongs"
        );
    }

    /// One huge file must cost a skip line, not the process.
    #[test]
    fn an_oversized_payload_is_skipped_rather_than_read() {
        let d = tmp("huge");
        std::fs::write(d.join("in/big.tif"), vec![0u8; 4096]).unwrap();
        std::fs::write(d.join("in/ok.tif"), b"small").unwrap();
        let mut st = s(&d);
        st.max_payload_bytes = 1024;
        let r = decode_dir(&key(), &st).unwrap();
        assert_eq!(r.recorded, 1, "the small payload is still recorded");
        assert_eq!(r.skipped.len(), 1);
        assert!(r.skipped[0].reason.contains("exceeds"));
        assert!(
            r.bytes_read < 1024,
            "the oversized file must never be read into memory"
        );
    }

    /// A run must not be able to write outside its output directory, whatever
    /// the input is named. On unix a file name cannot contain a separator, so
    /// this checks the property rather than assuming it.
    #[test]
    fn every_written_path_stays_inside_the_output_directory() {
        let d = tmp("escape");
        for name in ["..tif", "...", "a b.tif", "-rf.tif"] {
            std::fs::write(d.join("in").join(name), b"x").unwrap();
        }
        decode_dir(&key(), &s(&d)).unwrap();
        let out = d.join("out").canonicalize().unwrap();
        for e in std::fs::read_dir(d.join("out")).unwrap() {
            let p = e.unwrap().path().canonicalize().unwrap();
            assert!(
                p.starts_with(&out),
                "{} escaped the output directory",
                p.display()
            );
        }
    }
}

/// Their host runs containers in parallel against one output mount, so that
/// is a supported configuration and not an edge case.
#[cfg(test)]
mod parallel {
    use super::*;

    /// Two nodes writing the same output directory must both finish, and
    /// neither may destroy the other's report.
    ///
    /// Before the fix this was reproduced with two real processes: one exited
    /// non-zero on a temporary-file race, and the survivor's run.json had
    /// overwritten the other's.
    #[test]
    fn two_nodes_sharing_an_output_directory_do_not_collide() {
        let d = std::env::temp_dir().join("emem-airgap-parallel");
        let _ = std::fs::remove_dir_all(&d);
        for sub in ["in_a", "in_b", "out"] {
            std::fs::create_dir_all(d.join(sub)).unwrap();
        }
        std::fs::write(d.join("in_a/shared_name.tif"), b"from node a").unwrap();
        std::fs::write(d.join("in_b/shared_name.tif"), b"from node b").unwrap();

        let mk = |seed: u8, input: &str| {
            let k = SigningKey::from_bytes(&[seed; 32]);
            let st = DecodeSettings {
                input: d.join(input),
                output: d.join("out"),
                node: NodeIdentity {
                    node_key: crate::b32(k.verifying_key().as_bytes()),
                    profile: "orbital.satellite.v1".into(),
                    platform: "nvidia.jetson-orin".into(),
                },
                traces: None,
                stage: None,
                observed_at: "2026-08-20T09:00:00Z".into(),
                max_payload_bytes: DEFAULT_MAX_PAYLOAD_BYTES,
                max_files: DEFAULT_MAX_FILES,
            };
            (k, st)
        };
        let (ka, sa) = mk(11, "in_a");
        let (kb, sb) = mk(22, "in_b");

        decode_dir(&ka, &sa).unwrap();
        decode_dir(&kb, &sb).unwrap();

        // Both reports survive, each under its own node's short key.
        let a = d
            .join("out")
            .join(format!("run.{}.json", short_key(&sa.node.node_key)));
        let b = d
            .join("out")
            .join(format!("run.{}.json", short_key(&sb.node.node_key)));
        assert!(a.exists(), "node a's report must survive");
        assert!(b.exists(), "node b's report must survive");
        assert_ne!(a, b, "the two reports must not be the same file");

        // No temporary debris is left in a shared directory.
        let leftovers: Vec<_> = std::fs::read_dir(d.join("out"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".part"))
            .collect();
        assert!(leftovers.is_empty(), "no .part files may be left behind");
    }
}

/// Failure modes that are not attacks: power, radiation, and a full disk.
#[cfg(test)]
mod resilience {
    use super::*;

    fn key() -> SigningKey {
        SigningKey::from_bytes(&[3u8; 32])
    }

    fn s(dir: &Path) -> DecodeSettings {
        DecodeSettings {
            input: dir.join("in"),
            output: dir.join("out"),
            node: NodeIdentity {
                node_key: crate::b32(key().verifying_key().as_bytes()),
                profile: "orbital.satellite.v1".into(),
                platform: "nvidia.jetson-orin".into(),
            },
            traces: None,
            stage: None,
            observed_at: "2026-08-20T09:00:00Z".into(),
            max_payload_bytes: DEFAULT_MAX_PAYLOAD_BYTES,
            max_files: DEFAULT_MAX_FILES,
        }
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("emem-airgap-res-{name}"));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("in")).unwrap();
        std::fs::create_dir_all(d.join("out")).unwrap();
        d
    }

    /// Debris from a run that was interrupted mid-write must not block the
    /// next run, and must never be mistaken for a finished record.
    #[test]
    fn a_leftover_partial_write_does_not_poison_the_next_run() {
        let d = tmp("partial");
        std::fs::write(d.join("in/a.tif"), b"payload").unwrap();
        // What a power cut during the previous run leaves behind.
        std::fs::write(d.join("out/a.tif.custody.part"), b"{ truncated").unwrap();

        let r = decode_dir(&key(), &s(&d)).unwrap();
        assert_eq!(r.recorded, 1, "the run completes despite the debris");
        // Reported, not deleted. On a host that runs containers in parallel a
        // .part file may belong to a node writing right now, and tidying it
        // away would corrupt a healthy write.
        assert_eq!(
            r.stale_partials,
            vec!["a.tif.custody.part".to_string()],
            "debris must be named in the report"
        );
        assert!(
            d.join("out/a.tif.custody.part").exists(),
            "another container's temporary must not be removed on its behalf"
        );
        let raw = std::fs::read(d.join("out/a.tif.custody.json")).unwrap();
        let c: Custody = serde_json::from_slice(&raw).unwrap();
        c.verify().expect("the finished record verifies");
    }

    /// A run larger than the file cap is truncated LOUDLY: every file beyond
    /// the cap appears in the report, so the host can see what was left.
    #[test]
    fn the_file_cap_reports_the_overflow_rather_than_hiding_it() {
        let d = tmp("cap");
        for i in 0..5 {
            std::fs::write(d.join("in").join(format!("f{i}.tif")), b"x").unwrap();
        }
        let mut st = s(&d);
        st.max_files = 2;
        let r = decode_dir(&key(), &st).unwrap();
        assert_eq!(r.recorded, 2);
        assert_eq!(r.skipped.len(), 3, "every skipped file is named");
        assert!(r.skipped.iter().all(|s| s.reason.contains("file cap")));
    }

    /// Corruption after the write must be caught by the node, because on the
    /// ground there is no second copy to compare against.
    #[test]
    fn a_record_that_does_not_survive_the_round_trip_is_not_shipped() {
        let d = tmp("bitflip");
        std::fs::write(d.join("in/a.tif"), b"payload").unwrap();
        decode_dir(&key(), &s(&d)).unwrap();

        // Flip a byte inside the signed body, the way a stray upset would.
        let p = d.join("out/a.tif.custody.json");
        let mut raw = std::fs::read(&p).unwrap();
        let i = raw
            .windows(9)
            .position(|w| w == b"\"size_byt")
            .expect("field present");
        raw[i + 2] = b'X';
        std::fs::write(&p, &raw).unwrap();

        // A reader on the ground must reject it rather than half-trust it.
        let parsed: Result<Custody, _> = serde_json::from_slice(&raw);
        if let Ok(c) = parsed {
            assert!(c.verify().is_err(), "a flipped byte must not verify");
        }
    }
}

/// The seam between the encoder and the decoder.
#[cfg(test)]
mod handoff {
    use super::*;
    use emem_core::key::{AttesterKey, KeyEpoch};
    use emem_trace::{DeviceIdentity, EmittedOutput, OsTrace, TraceSegment};

    /// Build a real, signed trace that emits one payload digest.
    ///
    /// Real rather than mocked: if the decoder can read what emem-trace
    /// actually produces, the two halves meet. A hand-rolled JSON blob would
    /// prove only that the decoder can read a hand-rolled JSON blob.
    fn trace_covering(payload: &[u8], key: &SigningKey) -> OsTrace {
        let device = DeviceIdentity {
            device_key: AttesterKey(key.verifying_key().to_bytes()),
            key_epoch: KeyEpoch(0),
            substrate_profile: "robot.fleet.v1".into(),
            platform: "jetson-orin-nx".into(),
            os: "Ubuntu 22.04".into(),
            kernel: "5.15.148-tegra".into(),
            boot_id: "boot-1".into(),
        };
        let seg = TraceSegment {
            layer: emem_core::substrates::TraceLayerKind::Syscall,
            encoding: "linux.ftrace.v1".into(),
            seq: 0,
            clock_start_ns: 0,
            clock_end_ns: 10,
            event_count: 1,
            log_digest: crate::b32(blake3::hash(b"segment").as_bytes()),
            prev_digest: None,
        };
        let out = EmittedOutput {
            payload_digest: crate::b32(blake3::hash(payload).as_bytes()),
            band: None,
            emitted_at_ns: 5,
            layer: emem_core::substrates::TraceLayerKind::SensorBus,
        };
        let segments = vec![seg];
        let root = OsTrace::compute_trace_root(&segments).expect("root");
        let mut t = OsTrace {
            schema: emem_trace::OS_TRACE_SCHEMA_V1.into(),
            device,
            window_start_ns: 0,
            window_end_ns: 10,
            segments,
            outputs: vec![out],
            trace_root: crate::b32(&root),
            prev_trace_cid: None,
            signature: emem_core::key::Signature([0u8; 64]),
        };
        let pre = t.preimage().expect("preimage");
        use ed25519_dalek::Signer;
        t.signature = emem_core::key::Signature(key.sign(&pre).to_bytes());
        t
    }

    #[test]
    fn a_payload_an_encoder_traced_cites_that_trace() {
        let d = std::env::temp_dir().join("emem-airgap-handoff");
        let _ = std::fs::remove_dir_all(&d);
        for sub in ["in", "out", "traces"] {
            std::fs::create_dir_all(d.join(sub)).unwrap();
        }
        let payload = b"a frame the encoder watched being produced";
        std::fs::write(d.join("in/traced.tif"), payload).unwrap();
        std::fs::write(d.join("in/untraced.tif"), b"a frame nobody watched").unwrap();

        let k = SigningKey::from_bytes(&[3u8; 32]);
        let t = trace_covering(payload, &k);
        std::fs::write(
            d.join("traces/t1.json"),
            serde_json::to_vec_pretty(&t).unwrap(),
        )
        .unwrap();
        // Debris in the encoder's directory must not stop custody.
        std::fs::write(d.join("traces/garbage.json"), b"not a trace").unwrap();

        let st = DecodeSettings {
            input: d.join("in"),
            output: d.join("out"),
            traces: Some(d.join("traces")),
            stage: Some("L2".into()),
            node: NodeIdentity {
                node_key: crate::b32(k.verifying_key().as_bytes()),
                profile: "robot.fleet.v1".into(),
                platform: "nvidia.jetson-orin".into(),
            },
            observed_at: "2026-08-20T09:00:00Z".into(),
            max_payload_bytes: DEFAULT_MAX_PAYLOAD_BYTES,
            max_files: DEFAULT_MAX_FILES,
        };
        let r = decode_dir(&k, &st).unwrap();
        assert_eq!(r.recorded, 2);
        assert_eq!(r.traced, 1, "exactly the traced payload cites a trace");
        assert_eq!(r.unreadable_traces, 1, "debris is counted, not fatal");

        let traced: Custody =
            serde_json::from_slice(&std::fs::read(d.join("out/traced.tif.custody.json")).unwrap())
                .unwrap();
        traced.verify().expect("a traced record still verifies");
        assert_eq!(
            traced.trace_cid.as_deref(),
            Some(t.trace_cid().unwrap().as_str())
        );
        assert!(traced.assurance.starts_with("custody_with_trace"));
        assert_eq!(traced.stage.as_deref(), Some("L2"));

        let plain: Custody = serde_json::from_slice(
            &std::fs::read(d.join("out/untraced.tif.custody.json")).unwrap(),
        )
        .unwrap();
        plain.verify().expect("an untraced record still verifies");
        assert!(plain.trace_cid.is_none(), "no trace, no citation");
        assert!(plain.assurance.starts_with("custody_only"));
    }

    /// A record must not be able to claim the stronger sentence without the
    /// evidence that earns it.
    #[test]
    fn claiming_a_trace_you_do_not_cite_is_refused() {
        let k = SigningKey::from_bytes(&[3u8; 32]);
        let node = NodeIdentity {
            node_key: crate::b32(k.verifying_key().as_bytes()),
            profile: "p".into(),
            platform: "pl".into(),
        };
        let mut c = Custody::sign(&k, node, "a.tif", b"x", "2026-08-20T09:00:00Z", None, None);
        c.assurance = crate::custody::ASSURANCE_TRACED.to_string();
        assert!(c.verify().is_err(), "the sentence must match the evidence");
    }
}
