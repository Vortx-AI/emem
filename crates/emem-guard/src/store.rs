//! An append-only, ed25519-signed verdict log on local disk.
//!
//! One line of JSON per verdict, each carrying the record, the signature over
//! its preimage, and the hash chain linking it to the one before. A node with
//! no network still produces evidence anyone can check; anchoring that log
//! into a wider graph is a later step, and nothing here depends on it.
//!
//! # Why a hash chain and not just a list of signatures
//!
//! Signatures alone prove each entry was issued by this node. They do not
//! prove the FILE is complete: an operator could delete a line and every
//! remaining signature would still verify. Chaining each entry to the digest
//! of the previous one makes a deletion detectable, because the chain breaks
//! at the seam. That is the difference between "these verdicts are genuine"
//! and "these are all the verdicts", and only the second is an audit trail.
//!
//! The chain is deliberately simple rather than an RFC 6962 tree. This log is
//! the node's own record; the responder already runs a real transparency tree
//! for facts, and anchoring a guard head into it is the design. Building a
//! second Merkle implementation here would be two implementations to keep
//! correct for no gain today.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::log::{LogError, VerdictLog, VerdictRecord};

/// One line in the log.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LoggedVerdict {
    /// Monotonic index, starting at 0.
    pub seq: u64,
    /// The verdict.
    pub record: VerdictRecord,
    /// ed25519 over `record.preimage()`, base32-nopad-lowercase, matching
    /// how every other signature on this surface is rendered.
    pub signature_b32: String,
    /// The node key that signed, so a reader needs nothing from us to check.
    pub signer_b32: String,
    /// `blake3(prev_chain || preimage)`, hex. The link that makes a deletion
    /// visible.
    pub chain: String,
}

/// A file-backed log.
///
/// The mutex is held across the read-modify-append so two concurrent verdicts
/// cannot compute the same chain link. Verdict volume is bounded by the
/// checkpoint's own rate, and an append is a few hundred bytes, so this is
/// not the bottleneck the budget cares about.
pub struct FileLog {
    path: PathBuf,
    state: Mutex<Tail>,
    /// The PUBLIC key, stamped on each entry so a reader needs nothing from
    /// us to verify it.
    ///
    /// The private half is deliberately absent. Signing is injected into
    /// `seal`, so this type writes signatures it could never produce, and a
    /// component that cannot forge an entry is a smaller thing to trust.
    signer_b32: String,
}

#[derive(Default)]
struct Tail {
    seq: u64,
    chain: String,
}

impl FileLog {
    /// Open or create the log at `path`, recovering the tail so a restart
    /// continues the chain rather than starting a second one.
    pub fn open(
        path: impl AsRef<Path>,
        verifying: ed25519_dalek::VerifyingKey,
    ) -> std::io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut tail = Tail::default();
        if path.exists() {
            // Read to the end rather than trusting a sidecar: the file is the
            // record, and a cached tail that disagreed with it would be a
            // silent fork.
            let f = std::fs::File::open(&path)?;
            for line in BufReader::new(f).lines() {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                if let Ok(v) = serde_json::from_str::<LoggedVerdict>(&line) {
                    tail = Tail {
                        seq: v.seq + 1,
                        chain: v.chain,
                    };
                }
            }
        }
        let signer_b32 = data_encoding::BASE32_NOPAD
            .encode(verifying.as_bytes())
            .to_lowercase();
        Ok(Self {
            path,
            state: Mutex::new(tail),
            signer_b32,
        })
    }

    /// The public key this log signs with.
    pub fn signer_b32(&self) -> &str {
        &self.signer_b32
    }

    /// How many verdicts have been written.
    pub fn len(&self) -> u64 {
        self.state.lock().map(|t| t.seq).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Look up a previously logged verdict by the checkpoint's request id.
    ///
    /// Used for idempotency: the platform retries once on a connection
    /// failure with the SAME id, and re-evaluating would let a replay produce
    /// a different answer than the one already recorded and acted on.
    pub fn find_by_request_id(&self, request_id: &str) -> Option<LoggedVerdict> {
        let f = std::fs::File::open(&self.path).ok()?;
        BufReader::new(f)
            .lines()
            .map_while(Result::ok)
            .filter_map(|l| serde_json::from_str::<LoggedVerdict>(&l).ok())
            .find(|v| v.record.request_id == request_id)
    }

    /// Verify the whole file: every signature, and the chain.
    ///
    /// This is what makes the log worth having, so it ships in the library
    /// rather than in a test: an auditor runs it against a copy of the file
    /// and needs nothing from the node that wrote it.
    pub fn audit(path: impl AsRef<Path>) -> Result<AuditReport, std::io::Error> {
        let f = std::fs::File::open(path)?;
        let mut report = AuditReport::default();
        let mut prev_chain = String::new();
        for (i, line) in BufReader::new(f).lines().enumerate() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let Ok(v) = serde_json::from_str::<LoggedVerdict>(&line) else {
                report.unparseable.push(i);
                continue;
            };
            report.entries += 1;

            // Signature.
            let ok_sig = (|| {
                let pk = data_encoding::BASE32_NOPAD
                    .decode(v.signer_b32.to_uppercase().as_bytes())
                    .ok()?;
                let pk: [u8; 32] = pk.try_into().ok()?;
                let sig = data_encoding::BASE32_NOPAD
                    .decode(v.signature_b32.to_uppercase().as_bytes())
                    .ok()?;
                let sig: [u8; 64] = sig.try_into().ok()?;
                let vk = ed25519_dalek::VerifyingKey::from_bytes(&pk).ok()?;
                use ed25519_dalek::Verifier as _;
                vk.verify(
                    &v.record.preimage(),
                    &ed25519_dalek::Signature::from_bytes(&sig),
                )
                .ok()
            })()
            .is_some();
            if !ok_sig {
                report.bad_signature.push(v.seq);
            }

            // Chain.
            let want = chain_link(&prev_chain, &v.record.preimage());
            if want != v.chain {
                report.broken_chain.push(v.seq);
            }
            prev_chain = v.chain.clone();
        }
        Ok(report)
    }
}

/// `blake3(prev || preimage)`, hex.
fn chain_link(prev: &str, preimage: &[u8]) -> String {
    let mut h = blake3::Hasher::new();
    h.update(prev.as_bytes());
    h.update(preimage);
    h.finalize().to_hex().to_string()
}

/// What an audit found.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct AuditReport {
    pub entries: usize,
    /// Sequence numbers whose signature did not verify.
    pub bad_signature: Vec<u64>,
    /// Sequence numbers whose chain link does not follow the previous entry.
    /// A deletion shows up here, at the seam.
    pub broken_chain: Vec<u64>,
    /// Line numbers that were not valid JSON.
    pub unparseable: Vec<usize>,
}

impl AuditReport {
    /// Whether the log is intact: every signature verifies and no entry is
    /// missing.
    pub fn is_intact(&self) -> bool {
        self.bad_signature.is_empty() && self.broken_chain.is_empty() && self.unparseable.is_empty()
    }
}

impl VerdictLog for FileLog {
    fn append(&self, record: &VerdictRecord, signature: &[u8]) -> Result<String, LogError> {
        let mut tail = self
            .state
            .lock()
            .map_err(|_| LogError::Unavailable("log mutex poisoned".into()))?;
        let preimage = record.preimage();
        let chain = chain_link(&tail.chain, &preimage);
        let entry = LoggedVerdict {
            seq: tail.seq,
            record: record.clone(),
            signature_b32: data_encoding::BASE32_NOPAD.encode(signature).to_lowercase(),
            signer_b32: self.signer_b32.clone(),
            chain: chain.clone(),
        };
        let line = serde_json::to_string(&entry)
            .map_err(|e| LogError::Unavailable(format!("serialise: {e}")))?;

        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| LogError::Unavailable(format!("open {}: {e}", self.path.display())))?;
        writeln!(f, "{line}").map_err(|e| LogError::Unavailable(format!("write: {e}")))?;
        // Durable before we report success. `seal` returns to the caller the
        // moment this function does, so an unflushed write would be a verdict
        // that was acted on and is not on disk.
        f.sync_all()
            .map_err(|e| LogError::Unavailable(format!("fsync: {e}")))?;

        let leaf = format!("leaf_{}", entry.seq);
        tail.seq += 1;
        tail.chain = chain;
        Ok(leaf)
    }
}

/// Sign with the node key.
pub fn signer(key: &ed25519_dalek::SigningKey) -> impl Fn(&[u8]) -> Vec<u8> + '_ {
    move |preimage| {
        use ed25519_dalek::Signer as _;
        key.sign(preimage).to_bytes().to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::{Decision, DenyCode, Fix};

    fn key() -> ed25519_dalek::SigningKey {
        ed25519_dalek::SigningKey::from_bytes(&[7u8; 32])
    }

    fn tmp(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "emem-guard-test-{name}-{}.jsonl",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&p);
        p
    }

    fn rec(id: &str) -> VerdictRecord {
        VerdictRecord::new(
            "cp",
            id,
            &Decision::block(
                DenyCode::ProvSig,
                Some("emem:fact:a:b".into()),
                Fix::RefreshToken,
            ),
            1_700_000_000,
        )
    }

    /// An auditor with a copy of the file and nothing else can check it.
    #[test]
    fn a_written_log_audits_clean_without_the_node() {
        let path = tmp("clean");
        let log = FileLog::open(&path, key().verifying_key()).unwrap();
        let k = key();
        let sign = signer(&k);
        for i in 0..5 {
            let r = rec(&format!("req_{i}"));
            log.append(&r, &sign(&r.preimage())).unwrap();
        }
        let report = FileLog::audit(&path).unwrap();
        assert_eq!(report.entries, 5);
        assert!(report.is_intact(), "{report:?}");
        std::fs::remove_file(&path).ok();
    }

    /// The point of the chain: signatures alone would still verify after a
    /// deletion, so removing an entry must break something.
    #[test]
    fn deleting_an_entry_is_detected_even_though_every_signature_still_verifies() {
        let path = tmp("deleted");
        let log = FileLog::open(&path, key().verifying_key()).unwrap();
        let k = key();
        let sign = signer(&k);
        for i in 0..4 {
            let r = rec(&format!("req_{i}"));
            log.append(&r, &sign(&r.preimage())).unwrap();
        }
        // Excise the second entry, exactly as an operator hiding a verdict
        // would.
        let kept: Vec<String> = std::fs::read_to_string(&path)
            .unwrap()
            .lines()
            .enumerate()
            .filter(|(i, _)| *i != 1)
            .map(|(_, l)| l.to_string())
            .collect();
        std::fs::write(&path, kept.join("\n") + "\n").unwrap();

        let report = FileLog::audit(&path).unwrap();
        assert_eq!(report.entries, 3);
        assert!(
            report.bad_signature.is_empty(),
            "every remaining signature is genuine, which is exactly why the chain is needed"
        );
        assert!(
            !report.broken_chain.is_empty(),
            "the deletion must show at the seam"
        );
        assert!(!report.is_intact());
        std::fs::remove_file(&path).ok();
    }

    /// Editing a logged verdict must invalidate its signature.
    #[test]
    fn altering_a_verdict_after_the_fact_breaks_it() {
        let path = tmp("altered");
        let log = FileLog::open(&path, key().verifying_key()).unwrap();
        let k = key();
        let sign = signer(&k);
        let r = rec("req_0");
        log.append(&r, &sign(&r.preimage())).unwrap();

        let raw = std::fs::read_to_string(&path).unwrap();
        let mut v: LoggedVerdict = serde_json::from_str(raw.trim()).unwrap();
        v.record.outcome = "allow".to_string(); // flip a deny to an allow
        std::fs::write(&path, serde_json::to_string(&v).unwrap() + "\n").unwrap();

        let report = FileLog::audit(&path).unwrap();
        assert_eq!(report.bad_signature, vec![0]);
        assert!(!report.is_intact());
        std::fs::remove_file(&path).ok();
    }

    /// A restart must continue the chain, not fork it.
    #[test]
    fn reopening_continues_the_existing_chain() {
        let path = tmp("reopen");
        let k = key();
        let sign = signer(&k);
        {
            let log = FileLog::open(&path, key().verifying_key()).unwrap();
            let r = rec("req_0");
            log.append(&r, &sign(&r.preimage())).unwrap();
        }
        {
            let log = FileLog::open(&path, key().verifying_key()).unwrap();
            assert_eq!(log.len(), 1, "the tail was recovered from the file");
            let r = rec("req_1");
            let leaf = log.append(&r, &sign(&r.preimage())).unwrap();
            assert_eq!(leaf, "leaf_1", "sequence continues");
        }
        assert!(
            FileLog::audit(&path).unwrap().is_intact(),
            "the chain survived the restart"
        );
        std::fs::remove_file(&path).ok();
    }

    /// The platform retries once with the SAME id on a connection failure.
    /// Re-evaluating could produce a different answer than the one already
    /// acted on, so the recorded verdict is what a replay gets.
    #[test]
    fn a_replay_finds_the_verdict_already_recorded() {
        let path = tmp("replay");
        let log = FileLog::open(&path, key().verifying_key()).unwrap();
        let k = key();
        let sign = signer(&k);
        let r = rec("req_dedupe");
        log.append(&r, &sign(&r.preimage())).unwrap();

        let found = log
            .find_by_request_id("req_dedupe")
            .expect("the replay must find it");
        assert_eq!(found.record.outcome, "deny");
        assert!(log.find_by_request_id("never_seen").is_none());
        std::fs::remove_file(&path).ok();
    }
}
