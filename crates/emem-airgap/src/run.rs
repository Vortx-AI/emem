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
    /// Wall clock for this run, RFC 3339 UTC. Passed in rather than read so a
    /// run is reproducible and a machine with a bad clock cannot invent one.
    pub observed_at: String,
    /// Largest payload this node will read into memory. A cap rather than a
    /// hope: the input directory belongs to the host, and one enormous file
    /// should cost a skip line in the report, not the process.
    pub max_payload_bytes: u64,
}

/// Default payload cap: 256 MiB. Big enough for the imagery this is built for,
/// small enough that a node with modest memory survives a hostile directory.
pub const DEFAULT_MAX_PAYLOAD_BYTES: u64 = 256 * 1024 * 1024;

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

    let mut report = DecodeReport {
        recorded: 0,
        skipped: Vec::new(),
        bytes_read: 0,
        bytes_written: 0,
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
        let record = Custody::sign(
            key,
            settings.node.clone(),
            &name,
            &bytes,
            &settings.observed_at,
        );
        let json = serde_json::to_vec_pretty(&record).unwrap_or_else(|_| b"{}".to_vec());
        let out = settings.output.join(format!("{name}.custody.json"));
        write_no_follow(&out, &json)?;
        report.bytes_read += bytes.len() as u64;
        report.bytes_written += json.len() as u64;
        report.recorded += 1;
    }

    let manifest = settings.output.join("run.json");
    let mj = serde_json::to_vec_pretty(&report).unwrap_or_else(|_| b"{}".to_vec());
    write_no_follow(&manifest, &mj)?;
    report.bytes_written += mj.len() as u64;
    Ok(report)
}

/// Write a file without ever following a symlink at the destination.
///
/// The output directory is shared with the host, so it is attacker-controlled
/// too. A symlink planted there named `<payload>.custody.json` redirected the
/// write and overwrote whatever it pointed at; that was demonstrated against
/// this code before it was fixed.
///
/// Unlinking first and then creating exclusively removes the follow entirely:
/// `remove_file` on a symlink removes the link and not its target, and
/// `create_new` refuses to open anything that already exists, so a racing
/// writer gets an error rather than silently winning. Re-running the node
/// still overwrites its own previous output, which is the behaviour that
/// makes a run reproducible.
pub(crate) fn write_no_follow(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    // NotFound is the ordinary case on a first run; anything else is real.
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    f.write_all(bytes)
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
            observed_at: "2026-08-20T09:00:00Z".into(),
            max_payload_bytes: DEFAULT_MAX_PAYLOAD_BYTES,
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
            observed_at: "2026-08-20T09:00:00Z".into(),
            max_payload_bytes: DEFAULT_MAX_PAYLOAD_BYTES,
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
