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
    /// `request_id -> byte offset of its line`.
    ///
    /// Without this, a replay check reads the WHOLE log: O(n) per request and
    /// O(n squared) over the life of the file. A node answering 100 verdicts
    /// a second would be re-reading a growing file on every one of them, and
    /// the verdict path has an 800 ms self-budget for the entire exchange.
    ///
    /// Offsets rather than records: the key is the only thing held in memory,
    /// so a million verdicts cost about 30 MB of ids instead of gigabytes of
    /// bodies, and a hit is one seek and one line.
    index: std::collections::HashMap<String, u64>,
    /// Byte offset of each entry, indexed by sequence number.
    ///
    /// A `Vec` rather than a second map because sequence numbers are dense and
    /// start at zero. This is what makes a leaf id fetchable: a denial names
    /// `leaf=leaf_7`, and whoever holds that reason has to be able to pull
    /// entry 7 and check its signature, or the leaf is a promise with no
    /// mechanism behind it.
    offsets: Vec<u64>,
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
            // One pass at open, rebuilding both the chain tail and the
            // dedupe index. Read to the end rather than trusting a sidecar:
            // the file is the record, and a cached tail that disagreed with
            // it would be a silent fork.
            let f = std::fs::File::open(&path)?;
            let mut reader = BufReader::new(f);
            let mut offset: u64 = 0;
            let mut line = String::new();
            loop {
                line.clear();
                let n = reader.read_line(&mut line)?;
                if n == 0 {
                    break;
                }
                let here = offset;
                offset += n as u64;
                if line.trim().is_empty() {
                    continue;
                }
                if let Ok(v) = serde_json::from_str::<LoggedVerdict>(&line) {
                    tail.index.insert(v.record.request_id.clone(), here);
                    // Recorded by position rather than by the entry's own seq,
                    // so a file whose sequence numbers were tampered with
                    // cannot make a lookup return the wrong line. The seq is
                    // re-checked on read.
                    tail.offsets.push(here);
                    tail.seq = v.seq + 1;
                    tail.chain = v.chain;
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

    /// Where the log lives, for the report and for an operator copying it.
    pub fn path(&self) -> &Path {
        &self.path
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
        // Index hit or nothing: an id we have never seen costs a hash lookup
        // rather than a file read, which is the common case by far since most
        // requests are not replays.
        let offset = *self.state.lock().ok()?.index.get(request_id)?;
        let v = self.read_at(offset)?;
        // The offset came from our own index, but a truncated or rewritten
        // file could point it at the wrong line. Confirm rather than trust:
        // returning another request's verdict would be worse than re-deciding.
        (v.record.request_id == request_id).then_some(v)
    }

    /// Fetch one entry by its sequence number.
    ///
    /// This is what makes `leaf=leaf_7` in a deny reason mean something. The
    /// caller gets the record, the signature and the signing key, and can
    /// verify the verdict offline against a node it does not trust, which is
    /// the difference between an audit trail and an assertion.
    pub fn entry(&self, seq: u64) -> Option<LoggedVerdict> {
        let offset = {
            let t = self.state.lock().ok()?;
            *t.offsets.get(usize::try_from(seq).ok()?)?
        };
        let v = self.read_at(offset)?;
        (v.seq == seq).then_some(v)
    }

    /// A window of entries, oldest first.
    ///
    /// Bounded by the caller AND by `max`, because an unbounded range over a
    /// long-lived log is a way to make a node read its whole history into
    /// memory on request.
    pub fn range(&self, start: u64, count: usize, max: usize) -> Vec<LoggedVerdict> {
        let count = count.min(max);
        (start..start.saturating_add(count as u64))
            .filter_map(|s| self.entry(s))
            .collect()
    }

    /// The head of the chain: how many entries, and the last link.
    ///
    /// Two nodes holding the same head hold the same history, so this is what
    /// a witness or a mirror pins. Publishing it costs nothing and lets anyone
    /// notice a fork without reading the log.
    pub fn head(&self) -> (u64, String) {
        match self.state.lock() {
            Ok(t) => (t.seq, t.chain.clone()),
            Err(_) => (0, String::new()),
        }
    }

    /// Read and parse the entry whose line starts at `offset`.
    fn read_at(&self, offset: u64) -> Option<LoggedVerdict> {
        use std::io::{Seek as _, SeekFrom};
        let mut f = std::fs::File::open(&self.path).ok()?;
        f.seek(SeekFrom::Start(offset)).ok()?;
        // One line, not the file. The entry is a few hundred bytes.
        let mut line = String::new();
        BufReader::new(&mut f).read_line(&mut line).ok()?;
        serde_json::from_str(line.trim_end()).ok()
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

/// What a node did, counted from the log rather than from memory.
///
/// The instrument an organisation uses to decide whether to enforce a rule.
/// It reads the same file an auditor reads, so the numbers an operator quotes
/// internally and the numbers an outsider can verify are the same numbers.
///
/// Deliberately not a metrics endpoint. A counter in process memory resets on
/// restart, cannot be checked by anyone else, and is exactly the mutable
/// testimony this whole design exists to replace.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ShadowReport {
    pub entries: usize,
    /// Verdicts returned as deny, which blocked something.
    pub enforced_denies: usize,
    /// Verdicts where a rule fired but the node was observing, so nothing was
    /// blocked. This is the number that answers "what would enforcing cost".
    pub observed_denies: usize,
    /// Per deny code, `(enforced, observed)`.
    pub by_code: std::collections::BTreeMap<String, (usize, usize)>,
    /// Per checkpoint, how many verdicts arrived through it.
    pub by_checkpoint: std::collections::BTreeMap<String, usize>,
    /// Citations examined across every verdict.
    pub citations_checked: u64,
    /// Unix seconds of the first and last entry, so a rate has a denominator.
    pub first_unix_s: Option<i64>,
    pub last_unix_s: Option<i64>,
}

impl ShadowReport {
    /// Read a log and count it.
    pub fn read(path: impl AsRef<Path>) -> Result<Self, std::io::Error> {
        let f = std::fs::File::open(path)?;
        let mut r = Self::default();
        for line in BufReader::new(f).lines() {
            let line = line?;
            let Ok(v) = serde_json::from_str::<LoggedVerdict>(&line) else {
                continue;
            };
            r.entries += 1;
            r.citations_checked += v.record.checked as u64;
            *r.by_checkpoint
                .entry(v.record.checkpoint.clone())
                .or_default() += 1;
            let t = v.record.decided_at_unix_s;
            r.first_unix_s = Some(r.first_unix_s.map_or(t, |a: i64| a.min(t)));
            r.last_unix_s = Some(r.last_unix_s.map_or(t, |a: i64| a.max(t)));

            if v.record.evaluated != "deny" {
                continue;
            }
            let acted = v.record.outcome == "deny";
            if acted {
                r.enforced_denies += 1;
            } else {
                r.observed_denies += 1;
            }
            if let Some(c) = v.record.code {
                let e = r.by_code.entry(c.as_str().to_string()).or_default();
                if acted {
                    e.0 += 1;
                } else {
                    e.1 += 1;
                }
            }
        }
        Ok(r)
    }

    /// The share of verdicts where a rule fired, enforced or not.
    ///
    /// The number an org is actually deciding on: turn this rule on and this
    /// fraction of traffic stops. Zero entries returns zero rather than a
    /// division by nothing, because "no data" and "no denials" are different
    /// statements and the caller can tell them apart from `entries`.
    pub fn fired_rate(&self) -> f64 {
        if self.entries == 0 {
            return 0.0;
        }
        (self.enforced_denies + self.observed_denies) as f64 / self.entries as f64
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
        // Where this line starts, so the dedupe index can seek straight to it.
        let byte_offset = f
            .metadata()
            .map_err(|e| LogError::Unavailable(format!("stat: {e}")))?
            .len();
        writeln!(f, "{line}").map_err(|e| LogError::Unavailable(format!("write: {e}")))?;
        // Durable before we report success. `seal` returns to the caller the
        // moment this function does, so an unflushed write would be a verdict
        // that was acted on and is not on disk.
        f.sync_all()
            .map_err(|e| LogError::Unavailable(format!("fsync: {e}")))?;

        let leaf = format!("leaf_{}", entry.seq);
        tail.index.insert(record.request_id.clone(), byte_offset);
        tail.offsets.push(byte_offset);
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
            crate::policy::Mode::Enforce,
            "cfg-test",
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
