//! Facts the transparency log never received: find them, back them up, and
//! remove them from `facts.redb`.
//!
//! `put_attestation` stores the facts (body and canonical index row, one
//! durable redb commit), then appends the attestation to the log, then
//! writes the proof, multi-attester and scope rows. A caller dropped after
//! the first step, or a log append that failed, left facts that recall serves
//! and that no log entry and no inclusion proof backs. These are orphans.
//!
//! # The rule
//!
//! A fact (a row of `emem.facts`, keyed by its cid) is an orphan when all of
//! these hold:
//!
//! 1. `emem.fact_proofs` has no row for its cid and `emem.fact_locators` has
//!    no row for its digest. Both are written only after the log append
//!    returned, so either one proves the fact is logged. This is a filter
//!    that keeps the next step small, not the decision.
//! 2. One pass over every record of every log segment finds no fact whose
//!    bytes hash to the cid. The pass hashes each element of each record's
//!    `facts` array exactly as the log stores it, which is the same bytes
//!    `put_many` hashed into the cid. This is the decision.
//! 3. The pass is trusted only if it also found every positive control (a
//!    sample of facts that have a proof row, so are certainly logged), and
//!    could walk every record whose stored hash matched. A place where a
//!    frame does not check (a tear: a stored hash that is wrong, or a length
//!    that runs past the end) must be one the operator accepted by position
//!    (`accept_log_problems`, each `"<segment>:<offset>"`), and each accepted
//!    position must be a tear the pass found. Otherwise the scan says the log
//!    check failed and the delete refuses. At a tear the pass resyncs to the
//!    next frame that checks and reads on, so a fact logged after a tear is
//!    found and stays. A fact whose bytes are whole in the surviving part of
//!    a torn record is held (`in_damaged_log_record`), never removed.
//!
//! Facts written before proofs existed (the log is older than the proof
//! table) fail step 1 and pass step 2: the log has them, so they stay. So
//! does a fact whose write was dropped after the log append, before its
//! proof was stored.
//!
//! A fact is exported but held, not removed, when its bytes are whole in the
//! surviving part of a torn record, its body does not decode, its
//! `signed_at` does not parse, or it was signed less than `min_age_hours`
//! before the scan began (a write still between its two steps).
//!
//! # What the delete changes
//!
//! Only `facts.redb`, and only rows that name an exported orphan. Each
//! removal runs its checks again in the write transaction that removes (see
//! `RedbFacts::remove_orphans`): a fact that has a proof or a locator by
//! then, or whose cid is in the multi-attester index, is left alone. An index
//! row that names the orphan is pointed back at the newest logged fact at the
//! same key by the same signer, or removed so the next read materialises
//! again. The log, sled and receipts are never written.

use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use emem_cache::redb_facts::{OrphanRemoval, RemovalTally};
use emem_cache::{KvTable, RedbFacts};
use emem_fact::Fact;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::fact_locator::{entry_fact_spans, fact_spans_in_prefix, EntryFacts};
use crate::log_walk::{segment_indices, SegmentWalk, Step};
use crate::StorageError;

const SCAN_STATE: &str = "scan.state.json";
const SPOOL: &str = "candidates.spool";
const EXPORT: &str = "orphans.jsonl";
const REPORT: &str = "report.json";
const DELETE_STATE: &str = "delete.state.json";
const DELETE_REPORT: &str = "delete.report.json";
const VERIFY_REPORT: &str = "verify.report.json";
/// Positive controls kept from the walk. Each is a fact with a proof row,
/// which the log pass must find.
const CONTROLS: usize = 256;
/// Index rows per read transaction in the index walk.
const INDEX_PAGE: usize = 5_000;
/// How many of the newest logged cids at a key are tried as the fact to
/// point an index row back at.
const REPLACEMENT_TRIES: usize = 8;

/// The store a pass reads and, for the delete, writes.
pub struct OrphanStore<'a> {
    pub redb: &'a RedbFacts,
    /// The hot sled store, read to learn whether it still holds a copy of a
    /// fact's body. Never written.
    pub sled: Option<&'a sled::Db>,
    /// The transparency log's directory (`merkle.log.<n>` segments).
    pub log_root: &'a Path,
}

/// Cancellation and progress, shared with whoever started the pass.
#[derive(Default)]
pub struct JobControl {
    cancel: AtomicBool,
    progress: Mutex<Value>,
    /// Stop the walk once it has saved this many pages, as a restart would.
    #[cfg(test)]
    stop_after_pages: Option<u64>,
}

impl JobControl {
    pub fn new() -> Self {
        Self::default()
    }
    /// Ask the pass to stop at its next batch boundary. Its state on disk
    /// stays where the last committed batch left it.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }
    pub fn progress(&self) -> Value {
        self.progress
            .lock()
            .map(|p| p.clone())
            .unwrap_or(Value::Null)
    }
    pub(crate) fn set(&self, v: Value) {
        if let Ok(mut p) = self.progress.lock() {
            *p = v;
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ScanOptions {
    /// Walk only cids that start with this. One base32 character is about
    /// 1/32 of the store: a sample to time the walk and see the shape.
    pub prefix: String,
    /// Facts per read transaction.
    pub page: usize,
    /// Share of wall time the redb walks may spend reading (0.05 to 1).
    pub duty: f64,
    /// Facts signed less than this many hours before the scan began are
    /// exported but held.
    pub min_age_hours: u64,
    /// Read rate cap for the log pass, MB/s.
    pub log_mb_s: u64,
    /// Tears in the log the operator knows of, each `"<segment>:<offset>"`.
    /// Any other tear fails the log check, and so does a position here that
    /// is not a tear.
    pub accept_log_problems: Vec<String>,
    /// The scan's reference time; the clock when absent. For tests.
    #[serde(skip)]
    pub now_unix: Option<i64>,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            prefix: String::new(),
            page: 2_000,
            duty: 0.25,
            min_age_hours: 1,
            log_mb_s: 50,
            accept_log_problems: Vec::new(),
            now_unix: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct DeleteOptions {
    /// False (the default) reads and counts, and changes nothing.
    pub apply: bool,
    /// Facts per write transaction, at most 2,000.
    pub batch: usize,
    /// Share of wall time spent in write transactions (0.05 to 1).
    pub duty: f64,
    /// The export's BLAKE3 in hex, as `report.json` states it. Required to
    /// apply, so a delete runs only against an export someone has read.
    pub confirm: String,
    /// Remove only facts signed at or after this (`YYYY-MM-DD` or RFC 3339).
    pub signed_from: Option<String>,
    /// Remove only facts signed before this.
    pub signed_until: Option<String>,
}

impl Default for DeleteOptions {
    fn default() -> Self {
        Self {
            apply: false,
            batch: 1_000,
            duty: 0.25,
            confirm: String::new(),
            signed_from: None,
            signed_until: None,
        }
    }
}

pub(crate) fn err(msg: impl Into<String>) -> StorageError {
    StorageError::Io(std::io::Error::other(msg.into()))
}

fn cache_err(e: emem_cache::CacheError) -> StorageError {
    StorageError::Cache(e)
}

pub(crate) fn cancelled() -> StorageError {
    err("stopped on request; run the same action again to resume")
}

/// The pause that holds a step that took `took` to `duty` of wall time.
pub(crate) fn pause(took: Duration, duty: f64) -> Duration {
    let duty = duty.clamp(0.05, 1.0);
    took.mul_f64((1.0 - duty) / duty)
}

pub(crate) fn sleep_unless_cancelled(d: Duration, ctl: &JobControl) {
    let until = Instant::now() + d;
    while Instant::now() < until && !ctl.cancelled() {
        std::thread::sleep((until - Instant::now()).min(Duration::from_millis(200)));
    }
}

fn hex(b: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(b)
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    data_encoding::HEXLOWER.decode(s.as_bytes()).ok()
}

fn digest_of_cid(cid: &[u8]) -> Option<[u8; 32]> {
    data_encoding::BASE32_NOPAD
        .decode(&cid.to_ascii_uppercase())
        .ok()?
        .try_into()
        .ok()
}

pub(crate) fn write_json_atomic(path: &Path, v: &impl Serialize) -> Result<(), StorageError> {
    let tmp = path.with_extension("tmp");
    let bytes = serde_json::to_vec_pretty(v).map_err(|e| err(e.to_string()))?;
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub(crate) fn read_json<T: for<'de> Deserialize<'de>>(
    path: &Path,
) -> Result<Option<T>, StorageError> {
    match std::fs::read(path) {
        Ok(b) => serde_json::from_slice(&b)
            .map(Some)
            .map_err(|e| err(format!("{}: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub(crate) fn file_blake3(path: &Path) -> Result<(String, u64), StorageError> {
    let mut f = std::fs::File::open(path)?;
    let mut h = blake3::Hasher::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut n = 0u64;
    loop {
        let k = f.read(&mut buf)?;
        if k == 0 {
            break;
        }
        h.update(&buf[..k]);
        n += k as u64;
    }
    Ok((h.finalize().to_hex().to_string(), n))
}

/// The scan needs every proof row and multi-attester row in redb. While a
/// sled-to-redb copy is still running, "no proof row in redb" says nothing.
fn preflight(redb: &RedbFacts) -> Result<(), StorageError> {
    if !redb.backfill_done()
        || !redb.table_backfill_done(KvTable::Proofs)
        || !redb.table_backfill_done(KvTable::Multi)
    {
        return Err(err(
            "a sled-to-redb backfill has not finished, so redb does not yet hold every proof and \
             multi-attester row; the orphan scan would be guessing",
        ));
    }
    Ok(())
}

// ── scan ─────────────────────────────────────────────────────────────────

#[derive(Debug, Default, Serialize, Deserialize)]
struct WalkState {
    prefix: String,
    started_unix: i64,
    cursor_hex: Option<String>,
    done: bool,
    seen: u64,
    unproven: u64,
    proven: u64,
    pages: u64,
    spool_bytes: u64,
    controls: Vec<String>,
    walk_ms: u64,
}

#[derive(Debug, Default, Serialize)]
struct LogPass {
    segments: u64,
    records: u64,
    bytes: u64,
    attestations: u64,
    facts: u64,
    no_facts: u64,
    malformed: u64,
    /// Tears whose declared frame fits and whose stored hash is wrong.
    hash_mismatch: u64,
    /// Tears whose declared frame runs past the end of the segment.
    torn_tails: u64,
    /// Records read after a tear, by resyncing to the next frame that checks.
    after_a_tear: u64,
    /// Facts whole in the surviving bytes of a torn record.
    in_damaged_records: u64,
    ms: u64,
}

/// One place where a frame does not check.
#[derive(Debug, Clone, Serialize)]
struct LogProblem {
    /// `"<segment>:<offset>"`, the form `accept_log_problems` takes.
    position: String,
    segment: u64,
    offset: u64,
    kind: &'static str,
    declared_len: Option<u64>,
    resume_offset: Option<u64>,
    /// The leaf the tree's walk took from this frame, when it took one: that
    /// leaf is the hash of no entry.
    leaf_index: Option<u64>,
    accepted: bool,
    facts_named: u64,
}

/// `"<segment>:<offset>"`.
fn parse_position(s: &str) -> Option<(u64, u64)> {
    let (a, b) = s.trim().split_once(':')?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

fn spool_records(path: &Path) -> Result<Vec<(Vec<u8>, u64)>, StorageError> {
    let mut out = Vec::new();
    let mut r = BufReader::with_capacity(1 << 20, std::fs::File::open(path)?);
    let mut pos = 0u64;
    loop {
        let mut len = [0u8; 4];
        match r.read_exact(&mut len) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.into()),
        }
        let n = u32::from_le_bytes(len) as usize;
        let mut cid = vec![0u8; n];
        r.read_exact(&mut cid)?;
        r.read_exact(&mut len)?;
        let m = u32::from_le_bytes(len) as u64;
        std::io::copy(&mut (&mut r).take(m), &mut std::io::sink())?;
        out.push((cid, pos));
        pos += 4 + n as u64 + 4 + m;
    }
    Ok(out)
}

fn spool_body(f: &mut std::fs::File, at: u64) -> Result<(Vec<u8>, Vec<u8>), StorageError> {
    f.seek(SeekFrom::Start(at))?;
    let mut len = [0u8; 4];
    f.read_exact(&mut len)?;
    let mut cid = vec![0u8; u32::from_le_bytes(len) as usize];
    f.read_exact(&mut cid)?;
    f.read_exact(&mut len)?;
    let mut body = vec![0u8; u32::from_le_bytes(len) as usize];
    f.read_exact(&mut body)?;
    Ok((cid, body))
}

fn walk(
    store: &OrphanStore<'_>,
    dir: &Path,
    opts: &ScanOptions,
    ctl: &JobControl,
) -> Result<WalkState, StorageError> {
    let state_path = dir.join(SCAN_STATE);
    let mut st: WalkState = match read_json::<WalkState>(&state_path)? {
        Some(s) if s.prefix == opts.prefix => s,
        Some(s) => {
            return Err(err(format!(
                "this run was started with prefix {:?}; use a new run name for prefix {:?}",
                s.prefix, opts.prefix
            )))
        }
        None => WalkState {
            prefix: opts.prefix.clone(),
            started_unix: opts.now_unix.unwrap_or_else(now_unix),
            ..WalkState::default()
        },
    };
    let mut spool = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(dir.join(SPOOL))?;
    // A page appended after the last saved state is dropped and read again.
    spool.set_len(st.spool_bytes)?;
    spool.seek(SeekFrom::End(0))?;
    let page_rows = opts.page.clamp(1, 5_000);
    while !st.done {
        if ctl.cancelled() {
            return Err(cancelled());
        }
        #[cfg(test)]
        if ctl.stop_after_pages.is_some_and(|k| st.pages >= k) {
            return Err(cancelled());
        }
        let t = Instant::now();
        let cursor = match &st.cursor_hex {
            Some(h) => Some(unhex(h).ok_or_else(|| err("scan state cursor is not hex"))?),
            None => None,
        };
        let page = store
            .redb
            .facts_proof_page(cursor.as_deref(), opts.prefix.as_bytes(), page_rows)
            .map_err(cache_err)?;
        let mut w = BufWriter::new(&mut spool);
        for (cid, body) in &page.unproven {
            w.write_all(&(cid.len() as u32).to_le_bytes())?;
            w.write_all(cid)?;
            w.write_all(&(body.len() as u32).to_le_bytes())?;
            w.write_all(body)?;
            st.spool_bytes += 8 + cid.len() as u64 + body.len() as u64;
        }
        w.flush()?;
        drop(w);
        spool.sync_data()?;
        if let Some(first) = page.proven.first() {
            let c = String::from_utf8_lossy(first).into_owned();
            if st.controls.len() < CONTROLS {
                st.controls.push(c);
            } else if st.pages.is_multiple_of(16) {
                let slot = (st.pages / 16) as usize % CONTROLS;
                st.controls[slot] = c;
            }
        }
        st.seen += page.seen as u64;
        st.unproven += page.unproven.len() as u64;
        st.proven += page.proven.len() as u64;
        st.pages += 1;
        if let Some(l) = &page.last {
            st.cursor_hex = Some(hex(l));
        }
        st.done = page.done;
        st.walk_ms += t.elapsed().as_millis() as u64;
        write_json_atomic(&state_path, &st)?;
        ctl.set(json!({
            "phase": "walk",
            "facts_seen": st.seen,
            "without_proof": st.unproven,
            "pages": st.pages,
            "cursor": st.cursor_hex.as_deref().and_then(unhex).map(|c| String::from_utf8_lossy(&c).into_owned()),
        }));
        if !st.done {
            sleep_unless_cancelled(pause(t.elapsed(), opts.duty), ctl);
        }
    }
    Ok(st)
}

pub(crate) fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Every record of every segment, in segment order. Marks `found[i]` for
/// each fact whose digest is `want`'s key `i`, and `damaged[i]` for each
/// one whole in the surviving bytes of a torn record. Resyncs past a tear
/// and lists each one.
fn log_pass(
    log_root: &Path,
    want: &HashMap<[u8; 32], u32>,
    found: &mut [bool],
    damaged: &mut [bool],
    mb_s: u64,
    ctl: &JobControl,
) -> Result<(LogPass, Vec<LogProblem>), StorageError> {
    let t0 = Instant::now();
    let segs = segment_indices(log_root)?;
    let newest = segs.last().copied();
    let rate = (mb_s.clamp(5, 500) * 1_000_000) as f64;
    let mut p = LogPass::default();
    let mut problems = Vec::new();
    let mut base = 0u64;
    for seg in segs {
        if ctl.cancelled() {
            return Err(cancelled());
        }
        // The open segment grows while this reads it; what was appended after
        // this moment is a write too new for the scan to judge anyway.
        let mut w = SegmentWalk::open(&log_root.join(format!("merkle.log.{seg}")))?;
        let mut paced = 0u64;
        while let Some(step) = w.next_step()? {
            match step {
                Step::Entry {
                    cbor, tree_ordinal, ..
                } => {
                    p.records += 1;
                    if tree_ordinal.is_none() {
                        p.after_a_tear += 1;
                    }
                    match entry_fact_spans(cbor) {
                        EntryFacts::Spans(spans) => {
                            p.attestations += 1;
                            for (a, b) in spans {
                                p.facts += 1;
                                let d = *blake3::hash(&cbor[a..b]).as_bytes();
                                if let Some(&i) = want.get(&d) {
                                    found[i as usize] = true;
                                }
                            }
                        }
                        EntryFacts::NoFacts => p.no_facts += 1,
                        EntryFacts::Malformed => p.malformed += 1,
                    }
                }
                // A record the writer is still writing at the end of the
                // newest segment is not a tear.
                Step::Tear { tear, .. } if Some(seg) == newest && tear.resume.is_none() => {}
                Step::Tear { tear, partial } => {
                    match tear.kind {
                        crate::log_walk::TearKind::BadHash => p.hash_mismatch += 1,
                        _ => p.torn_tails += 1,
                    }
                    let mut named = 0u64;
                    for (a, b) in fact_spans_in_prefix(partial) {
                        named += 1;
                        let d = *blake3::hash(&partial[a..b]).as_bytes();
                        if let Some(&i) = want.get(&d) {
                            damaged[i as usize] = true;
                        }
                    }
                    p.in_damaged_records += named;
                    problems.push(LogProblem {
                        position: format!("{seg}:{}", tear.offset),
                        segment: seg,
                        offset: tear.offset,
                        kind: tear.kind.as_str(),
                        declared_len: tear.declared_len,
                        resume_offset: tear.resume,
                        leaf_index: tear.tree_ordinal.map(|k| base + k),
                        accepted: false,
                        facts_named: named,
                    });
                }
            }
            if w.pos() - paced > 16 << 20 {
                paced = w.pos();
                let due = Duration::from_secs_f64((p.bytes + paced) as f64 / rate);
                if due > t0.elapsed() {
                    sleep_unless_cancelled(due - t0.elapsed(), ctl);
                }
            }
        }
        base += w.tree_leaves();
        p.segments += 1;
        p.bytes += w.len();
        ctl.set(json!({
            "phase": "log",
            "segment": seg,
            "records": p.records,
            "bytes": p.bytes,
        }));
        let due = Duration::from_secs_f64(p.bytes as f64 / rate);
        let spent = t0.elapsed();
        if due > spent {
            sleep_unless_cancelled(due - spent, ctl);
        }
    }
    p.ms = t0.elapsed().as_millis() as u64;
    Ok((p, problems))
}

struct Decoded {
    kind: &'static str,
    band: String,
    cell: String,
    tslot: u64,
    signed_at: String,
    signer: [u8; 32],
    key: Option<Vec<u8>>,
}

fn decode(body: &[u8]) -> Option<Decoded> {
    let f: Fact = ciborium::de::from_reader(body).ok()?;
    let key = crate::fact_canonical_key_bytes(&f);
    Some(match f {
        Fact::Primary(p) => Decoded {
            kind: "primary",
            band: p.band,
            cell: p.cell,
            tslot: p.tslot,
            signed_at: p.signed_at,
            signer: p.signer.0,
            key,
        },
        Fact::Absence(n) => Decoded {
            kind: "absence",
            band: n.band,
            cell: n.cell,
            tslot: n.tslot,
            signed_at: n.signed_at,
            signer: n.signer.0,
            key,
        },
        Fact::Derivative(d) => Decoded {
            kind: "derivative",
            band: d.band,
            cell: d.cell,
            tslot: d.tslot_window[0],
            signed_at: d.signed_at,
            signer: d.signer.0,
            key,
        },
    })
}

/// One line of the export: the backup of one orphan and the plan for it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportLine {
    pub cid: String,
    /// The fact's canonical CBOR, base64. BLAKE3 of these bytes is the cid.
    pub cbor_b64: String,
    pub kind: Option<String>,
    pub band: Option<String>,
    pub cell: Option<String>,
    pub tslot: Option<u64>,
    pub signed_at: Option<String>,
    pub signer: Option<String>,
    /// The fact's own canonical key, hex.
    pub key_hex: Option<String>,
    /// Index rows that named this cid at scan time.
    pub index_rows: Vec<ExportIndexRow>,
    /// The hot sled store still holds this body; a read by cid would copy it
    /// back, so the delete fixes the index rows and keeps the body.
    pub sled_copy: bool,
    /// Whether the delete may remove it.
    pub delete: bool,
    /// Why not, when `delete` is false.
    pub hold: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportIndexRow {
    pub key_hex: String,
    /// The logged fact this row is pointed back at; none removes the row.
    pub replace_with: Option<String>,
}

/// Index keys that name a cid, by that cid's ordinal in the pass.
type RowsByOrdinal = HashMap<u32, Vec<Vec<u8>>>;

/// The keys of every index row whose cid `is_target` maps to an ordinal,
/// grouped by it. One read transaction per page.
fn index_rows_naming(
    redb: &RedbFacts,
    is_target: impl Fn(&[u8]) -> Option<u32>,
    duty: f64,
    ctl: &JobControl,
    phase: &str,
) -> Result<(RowsByOrdinal, u64), StorageError> {
    let mut rows: RowsByOrdinal = HashMap::new();
    let mut after: Option<Vec<u8>> = None;
    let mut walked = 0u64;
    loop {
        if ctl.cancelled() {
            return Err(cancelled());
        }
        let t = Instant::now();
        let page = redb
            .index_page(after.as_deref(), INDEX_PAGE)
            .map_err(cache_err)?;
        walked += page.len() as u64;
        for (k, v) in &page {
            if let Some(i) = is_target(v) {
                rows.entry(i).or_default().push(k.clone());
            }
        }
        ctl.set(json!({"phase": phase, "index_rows_walked": walked}));
        if page.len() < INDEX_PAGE {
            break;
        }
        after = page.last().map(|(k, _)| k.clone());
        sleep_unless_cancelled(pause(t.elapsed(), duty), ctl);
    }
    Ok((rows, walked))
}

/// For each index key, the newest logged fact at that key by `signer` that
/// is stored and has a proof, among the last few the multi-attester index
/// lists there.
fn replacements(
    redb: &RedbFacts,
    keys: &[(Vec<u8>, [u8; 32])],
    is_orphan: &dyn Fn(&[u8]) -> bool,
) -> Result<Vec<Option<String>>, StorageError> {
    let key_refs: Vec<&[u8]> = keys.iter().map(|(k, _)| k.as_slice()).collect();
    let lists = redb
        .kv_get_many(KvTable::Multi, &key_refs)
        .map_err(cache_err)?;
    let mut tries: Vec<Vec<String>> = Vec::with_capacity(keys.len());
    for l in &lists {
        let cids: Vec<String> = match l {
            Some(b) => ciborium::de::from_reader(b.as_slice())
                .map_err(|e| err(format!("multi_attester decode: {e}")))?,
            None => Vec::new(),
        };
        tries.push(
            cids.into_iter()
                .rev()
                .filter(|c| !is_orphan(c.as_bytes()))
                .take(REPLACEMENT_TRIES)
                .collect(),
        );
    }
    let flat: Vec<&[u8]> = tries.iter().flatten().map(|c| c.as_bytes()).collect();
    let bodies = redb.get_facts(&flat).map_err(cache_err)?;
    let proven = redb.proven_many(&flat).map_err(cache_err)?;
    let mut at = 0usize;
    let mut out = Vec::with_capacity(keys.len());
    for ((key, signer), cands) in keys.iter().zip(&tries) {
        let mut pick = None;
        for (j, c) in cands.iter().enumerate() {
            let fits = proven[at + j]
                && bodies[at + j]
                    .as_deref()
                    .and_then(decode)
                    .is_some_and(|d| &d.signer == signer && d.key.as_deref() == Some(key));
            if fits {
                pick = Some(c.clone());
                break;
            }
        }
        at += cands.len();
        out.push(pick);
    }
    Ok(out)
}

fn instant_of(s: &str) -> Option<i64> {
    crate::rfc3339_instant(s).map(|(secs, _)| secs)
}

/// A window bound: `YYYY-MM-DD` (midnight UTC) or RFC 3339.
fn bound_of(s: &str) -> Option<i64> {
    if s.len() == 10 {
        return instant_of(&format!("{s}T00:00:00Z"));
    }
    instant_of(s)
}

/// Walk `emem.facts`, check the facts without a proof against the log, plan
/// the index rows, and write the export and the report into `dir`. Changes
/// nothing in the store. Resumable: the walk continues from its saved
/// cursor; the log pass and the plan run again in full.
pub fn scan(
    store: &OrphanStore<'_>,
    dir: &Path,
    opts: &ScanOptions,
    ctl: &JobControl,
) -> Result<Value, StorageError> {
    preflight(store.redb)?;
    std::fs::create_dir_all(dir)?;
    let st = walk(store, dir, opts, ctl)?;

    // Candidates then controls share one digest map; ordinals below
    // `candidates.len()` are candidates.
    let spool_path = dir.join(SPOOL);
    let candidates = spool_records(&spool_path)?;
    let n = candidates.len();
    let mut want: HashMap<[u8; 32], u32> = HashMap::with_capacity(n + st.controls.len());
    for (i, (cid, _)) in candidates.iter().enumerate() {
        let d = digest_of_cid(cid).ok_or_else(|| err("a spooled cid is not base32"))?;
        want.insert(d, i as u32);
    }
    let mut controls = 0usize;
    for c in &st.controls {
        if let Some(d) = digest_of_cid(c.as_bytes()) {
            if let std::collections::hash_map::Entry::Vacant(e) = want.entry(d) {
                e.insert((n + controls) as u32);
                controls += 1;
            }
        }
    }
    let mut found = vec![false; n + controls];
    let mut damaged = vec![false; n + controls];
    let (lp, mut problems) = log_pass(
        store.log_root,
        &want,
        &mut found,
        &mut damaged,
        opts.log_mb_s,
        ctl,
    )?;
    let controls_found = found[n..].iter().filter(|f| **f).count();
    let mut log_problems = Vec::new();
    let mut accepted: Vec<(u64, u64)> = Vec::new();
    for a in &opts.accept_log_problems {
        match parse_position(a) {
            Some(pos) => accepted.push(pos),
            None => log_problems.push(format!(
                "accept_log_problems entry {a:?} is not <segment>:<offset>"
            )),
        }
    }
    for pr in problems.iter_mut() {
        pr.accepted = accepted.contains(&(pr.segment, pr.offset));
        if !pr.accepted {
            log_problems.push(format!(
                "log segment {} at offset {}: {}{}; not in accept_log_problems",
                pr.segment,
                pr.offset,
                pr.kind,
                pr.leaf_index
                    .map(|l| format!(", the tree took leaf {l} from it"))
                    .unwrap_or_default()
            ));
        }
    }
    for (seg, off) in &accepted {
        if !problems
            .iter()
            .any(|pr| pr.segment == *seg && pr.offset == *off)
        {
            log_problems.push(format!(
                "accept_log_problems names {seg}:{off}, where the log has no problem"
            ));
        }
    }
    if controls == 0 {
        log_problems.push("no fact with a proof was seen, so the log pass has no control".into());
    }
    if controls_found < controls {
        log_problems.push(format!(
            "{} of {controls} controls (facts with a proof row) were not found in the log",
            controls - controls_found
        ));
    }
    if lp.malformed > 0 {
        log_problems.push(format!("{} log records could not be walked", lp.malformed));
    }
    let log_check_ok = log_problems.is_empty();

    let unlogged: Vec<usize> = (0..n).filter(|i| !found[*i]).collect();
    let is_unlogged = |cid: &[u8]| -> Option<u32> {
        let d = digest_of_cid(cid)?;
        let &i = want.get(&d)?;
        ((i as usize) < n && !found[i as usize]).then_some(i)
    };
    let (rows_by, index_walked) =
        index_rows_naming(store.redb, is_unlogged, opts.duty, ctl, "index")?;

    let sled_facts = match store.sled {
        Some(db) if db.tree_names().iter().any(|t| t.as_ref() == b"emem.facts") => {
            Some(db.open_tree("emem.facts").map_err(|e| err(e.to_string()))?)
        }
        _ => None,
    };
    let min_age = (opts.min_age_hours as i64).saturating_mul(3600);
    let export_tmp = dir.join(format!("{EXPORT}.tmp"));
    let mut out = BufWriter::new(std::fs::File::create(&export_tmp)?);
    let mut spool = std::fs::File::open(&spool_path)?;
    let mut by_band: BTreeMap<String, u64> = BTreeMap::new();
    let mut by_day: BTreeMap<String, u64> = BTreeMap::new();
    let mut by_kind: BTreeMap<String, u64> = BTreeMap::new();
    let mut held: BTreeMap<String, u64> = BTreeMap::new();
    let (mut deletable, mut sled_copies, mut rows_total, mut rows_back, mut lines) =
        (0u64, 0u64, 0u64, 0u64, 0u64);
    for chunk in unlogged.chunks(512) {
        if ctl.cancelled() {
            return Err(cancelled());
        }
        let t = Instant::now();
        let mut items: Vec<(Vec<u8>, Vec<u8>, Option<Decoded>)> = Vec::with_capacity(chunk.len());
        for &i in chunk {
            let (cid, body) = spool_body(&mut spool, candidates[i].1)?;
            let d = decode(&body);
            items.push((cid, body, d));
        }
        let mut row_keys: Vec<(Vec<u8>, [u8; 32])> = Vec::new();
        for (&i, (_, _, d)) in chunk.iter().zip(&items) {
            if let (Some(rows), Some(d)) = (rows_by.get(&(i as u32)), d) {
                for k in rows {
                    row_keys.push((k.clone(), d.signer));
                }
            }
        }
        let is_orphan = |c: &[u8]| is_unlogged(c).is_some();
        let repl = replacements(store.redb, &row_keys, &is_orphan)?;
        let mut repl_at: HashMap<&[u8], Option<String>> = HashMap::new();
        for ((k, _), r) in row_keys.iter().zip(repl) {
            repl_at.insert(k.as_slice(), r);
        }
        for (&i, (cid, body, d)) in chunk.iter().zip(&items) {
            let index_rows: Vec<ExportIndexRow> = rows_by
                .get(&(i as u32))
                .map(|rows| {
                    rows.iter()
                        .map(|k| ExportIndexRow {
                            key_hex: hex(k),
                            replace_with: repl_at.get(k.as_slice()).cloned().flatten(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            rows_total += index_rows.len() as u64;
            rows_back += index_rows
                .iter()
                .filter(|r| r.replace_with.is_some())
                .count() as u64;
            let sled_copy = match &sled_facts {
                Some(t) => t.contains_key(cid).map_err(|e| err(e.to_string()))?,
                None => false,
            };
            sled_copies += u64::from(sled_copy);
            let hold = match d {
                _ if damaged[i] => Some("in_damaged_log_record"),
                None => Some("body_does_not_decode"),
                Some(d) => match instant_of(&d.signed_at) {
                    None => Some("signed_at_does_not_parse"),
                    Some(t) if t > st.started_unix - min_age => Some("signed_too_recently"),
                    Some(_) => None,
                },
            };
            if let Some(h) = hold {
                *held.entry(h.to_string()).or_default() += 1;
            } else {
                deletable += 1;
            }
            if let Some(d) = d {
                *by_band.entry(d.band.clone()).or_default() += 1;
                *by_kind.entry(d.kind.to_string()).or_default() += 1;
                let day = d.signed_at.get(..10).unwrap_or("unknown").to_string();
                *by_day.entry(day).or_default() += 1;
            }
            let line = ExportLine {
                cid: String::from_utf8_lossy(cid).into_owned(),
                cbor_b64: data_encoding::BASE64.encode(body),
                kind: d.as_ref().map(|d| d.kind.to_string()),
                band: d.as_ref().map(|d| d.band.clone()),
                cell: d.as_ref().map(|d| d.cell.clone()),
                tslot: d.as_ref().map(|d| d.tslot),
                signed_at: d.as_ref().map(|d| d.signed_at.clone()),
                signer: d
                    .as_ref()
                    .map(|d| data_encoding::BASE32_NOPAD.encode(&d.signer).to_lowercase()),
                key_hex: d.as_ref().and_then(|d| d.key.as_deref().map(hex)),
                index_rows,
                sled_copy,
                delete: hold.is_none() && log_check_ok,
                hold: hold
                    .map(str::to_string)
                    .or_else(|| (!log_check_ok).then(|| "log_check_failed".to_string())),
            };
            serde_json::to_writer(&mut out, &line).map_err(|e| err(e.to_string()))?;
            out.write_all(b"\n")?;
            lines += 1;
        }
        ctl.set(json!({"phase": "export", "lines": lines, "of": unlogged.len()}));
        sleep_unless_cancelled(pause(t.elapsed(), opts.duty), ctl);
    }
    out.flush()?;
    out.get_ref().sync_all()?;
    drop(out);
    let export_path = dir.join(EXPORT);
    std::fs::rename(&export_tmp, &export_path)?;
    let (export_blake3, export_bytes) = file_blake3(&export_path)?;

    let deletable = if log_check_ok { deletable } else { 0 };
    let report = json!({
        "schema": "emem.orphan_scan.v1",
        "dir": dir.display().to_string(),
        "prefix": st.prefix,
        "started_unix": st.started_unix,
        "min_age_hours": opts.min_age_hours,
        "walk": {
            "facts_seen": st.seen,
            "with_proof": st.proven,
            "without_proof": st.unproven,
            "pages": st.pages,
            "ms": st.walk_ms,
        },
        "log": lp,
        "controls": {"checked": controls, "found_in_log": controls_found},
        "log_check_ok": log_check_ok,
        "log_problems": log_problems,
        "accept_log_problems": opts.accept_log_problems,
        "log_problems_at": problems,
        "without_proof_found_in_log": n - unlogged.len(),
        "unlogged": unlogged.len(),
        "unlogged_by_band": by_band,
        "unlogged_by_signed_day": by_day,
        "unlogged_by_kind": by_kind,
        "held": held,
        "deletable": deletable,
        "sled_copy": sled_copies,
        "index": {
            "rows_walked": index_walked,
            "rows_naming_unlogged": rows_total,
            "to_point_back": rows_back,
            "to_remove": rows_total - rows_back,
        },
        "export": {
            "path": export_path.display().to_string(),
            "lines": lines,
            "bytes": export_bytes,
            "blake3": export_blake3,
        },
        "ok_to_delete": log_check_ok,
    });
    write_json_atomic(&dir.join(REPORT), &report)?;
    ctl.set(json!({"phase": "done"}));
    Ok(report)
}

// ── delete ───────────────────────────────────────────────────────────────

#[derive(Debug, Default, Serialize, Deserialize)]
struct DeleteState {
    export_blake3: String,
    next_line: u64,
    tally: Tally,
    batches: u64,
}

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize)]
struct Tally {
    removed: u64,
    already_gone: u64,
    kept_body: u64,
    skipped_has_proof: u64,
    skipped_in_multi: u64,
    skipped_body_differs: u64,
    index_repointed: u64,
    index_removed: u64,
    index_moved_on: u64,
    held: u64,
    outside_window: u64,
}

impl Tally {
    fn add(&mut self, t: &RemovalTally) {
        self.removed += t.removed;
        self.already_gone += t.already_gone;
        self.kept_body += t.kept_body;
        self.skipped_has_proof += t.skipped_has_proof;
        self.skipped_in_multi += t.skipped_in_multi;
        self.skipped_body_differs += t.skipped_body_differs;
        self.index_repointed += t.index_repointed;
        self.index_removed += t.index_removed;
        self.index_moved_on += t.index_moved_on;
    }
}

fn removal_of(line: &ExportLine) -> Result<OrphanRemoval, StorageError> {
    let body = data_encoding::BASE64
        .decode(line.cbor_b64.as_bytes())
        .map_err(|e| err(format!("export line {}: {e}", line.cid)))?;
    // The export is the backup. A line whose bytes are not the cid it names
    // is a damaged backup, and nothing is removed on its word.
    if Some(*blake3::hash(&body).as_bytes()) != digest_of_cid(line.cid.as_bytes()) {
        return Err(err(format!(
            "export line {}: the bytes do not hash to the cid; the export is damaged",
            line.cid
        )));
    }
    let mut index_rows = Vec::with_capacity(line.index_rows.len());
    for r in &line.index_rows {
        let k = unhex(&r.key_hex).ok_or_else(|| err("export index key is not hex"))?;
        index_rows.push((k, r.replace_with.as_ref().map(|c| c.as_bytes().to_vec())));
    }
    Ok(OrphanRemoval {
        cid: line.cid.as_bytes().to_vec(),
        body,
        key: match &line.key_hex {
            Some(h) => Some(unhex(h).ok_or_else(|| err("export key is not hex"))?),
            None => None,
        },
        index_rows,
        keep_body: line.sled_copy,
    })
}

/// Remove the facts a scan exported, in small write transactions, or with
/// `apply` false count what that would do and change nothing. Resumable: an
/// applied run saves the next export line after every committed batch.
/// Idempotent: a fact already gone is counted and its index rows checked.
pub fn delete(
    store: &OrphanStore<'_>,
    dir: &Path,
    opts: &DeleteOptions,
    ctl: &JobControl,
) -> Result<Value, StorageError> {
    preflight(store.redb)?;
    let report: Value = read_json(&dir.join(REPORT))?
        .ok_or_else(|| err("no report.json in this run; scan first"))?;
    if report.get("ok_to_delete").and_then(Value::as_bool) != Some(true) {
        return Err(err(
            "the scan's log check did not pass (see log_problems in report.json); nothing is removed",
        ));
    }
    let expected = report
        .pointer("/export/blake3")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let export_path = dir.join(EXPORT);
    let (actual, _) = file_blake3(&export_path)?;
    if actual != expected {
        return Err(err(format!(
            "orphans.jsonl hashes to {actual}, report.json says {expected}; the export changed"
        )));
    }
    if opts.apply && opts.confirm.trim() != expected {
        return Err(err(
            "apply needs confirm set to the export's blake3 from report.json",
        ));
    }
    let from = match &opts.signed_from {
        Some(s) => Some(bound_of(s).ok_or_else(|| err("signed_from does not parse"))?),
        None => None,
    };
    let until = match &opts.signed_until {
        Some(s) => Some(bound_of(s).ok_or_else(|| err("signed_until does not parse"))?),
        None => None,
    };
    let state_path = dir.join(DELETE_STATE);
    let mut st = if opts.apply {
        match read_json::<DeleteState>(&state_path)? {
            Some(s) if s.export_blake3 == expected => s,
            Some(_) | None => DeleteState {
                export_blake3: expected.clone(),
                ..DeleteState::default()
            },
        }
    } else {
        DeleteState::default()
    };
    let batch_size = opts.batch.clamp(1, 2_000);
    let t0 = Instant::now();
    let mut lines = BufReader::new(std::fs::File::open(&export_path)?).lines();
    let mut line_no = 0u64;
    let mut done = false;
    while !done {
        if ctl.cancelled() {
            return Err(cancelled());
        }
        let mut batch: Vec<OrphanRemoval> = Vec::with_capacity(batch_size);
        while batch.len() < batch_size {
            let Some(l) = lines.next() else {
                done = true;
                break;
            };
            let l = l?;
            line_no += 1;
            if line_no <= st.next_line {
                continue;
            }
            let line: ExportLine =
                serde_json::from_str(&l).map_err(|e| err(format!("export line {line_no}: {e}")))?;
            if !line.delete {
                st.tally.held += 1;
                continue;
            }
            let at = line.signed_at.as_deref().and_then(instant_of);
            let inside = match at {
                Some(t) => from.is_none_or(|f| t >= f) && until.is_none_or(|u| t < u),
                None => from.is_none() && until.is_none(),
            };
            if !inside {
                st.tally.outside_window += 1;
                continue;
            }
            batch.push(removal_of(&line)?);
        }
        let t = Instant::now();
        if !batch.is_empty() {
            let got = store
                .redb
                .remove_orphans(&batch, opts.apply)
                .map_err(cache_err)?;
            st.tally.add(&got);
            st.batches += 1;
        }
        st.next_line = line_no;
        if opts.apply {
            write_json_atomic(&state_path, &st)?;
        }
        ctl.set(json!({
            "phase": if opts.apply { "delete" } else { "dry_run" },
            "lines": line_no,
            "tally": st.tally,
        }));
        if !done {
            sleep_unless_cancelled(pause(t.elapsed(), opts.duty), ctl);
        }
    }
    let out = json!({
        "schema": "emem.orphan_delete.v1",
        "applied": opts.apply,
        "export_blake3": expected,
        "lines": line_no,
        "batches": st.batches,
        "tally": st.tally,
        "ms": t0.elapsed().as_millis() as u64,
    });
    if opts.apply {
        write_json_atomic(&dir.join(DELETE_REPORT), &out)?;
    }
    ctl.set(json!({"phase": "done"}));
    Ok(out)
}

// ── verify ───────────────────────────────────────────────────────────────

/// After a delete: how many exported facts are still stored, and how many
/// index rows still name one. Read-only.
pub fn verify(
    store: &OrphanStore<'_>,
    dir: &Path,
    duty: f64,
    ctl: &JobControl,
) -> Result<Value, StorageError> {
    let export_path = dir.join(EXPORT);
    // One digest map for both kinds of line, so the index is walked once:
    // ordinals below `removable` are lines the delete removes.
    let mut lines: Vec<(Vec<u8>, bool)> = Vec::new();
    for l in BufReader::new(std::fs::File::open(&export_path)?).lines() {
        let line: ExportLine = serde_json::from_str(&l?).map_err(|e| err(e.to_string()))?;
        lines.push((line.cid.into_bytes(), line.delete && !line.sled_copy));
    }
    lines.sort_by_key(|(_, removable)| !*removable);
    let removable = lines.iter().filter(|(_, r)| *r).count();
    let mut ordinal: HashMap<[u8; 32], u32> = HashMap::with_capacity(lines.len());
    for (i, (cid, _)) in lines.iter().enumerate() {
        if let Some(d) = digest_of_cid(cid) {
            ordinal.insert(d, i as u32);
        }
    }
    let mut still_stored = 0u64;
    for chunk in lines[..removable].chunks(2_000) {
        if ctl.cancelled() {
            return Err(cancelled());
        }
        let t = Instant::now();
        let refs: Vec<&[u8]> = chunk.iter().map(|(c, _)| c.as_slice()).collect();
        still_stored += store
            .redb
            .facts_present_many(&refs)
            .map_err(cache_err)?
            .into_iter()
            .filter(|p| *p)
            .count() as u64;
        sleep_unless_cancelled(pause(t.elapsed(), duty), ctl);
    }
    let (naming, walked) = index_rows_naming(
        store.redb,
        |v| digest_of_cid(v).and_then(|d| ordinal.get(&d).copied()),
        duty,
        ctl,
        "verify_index",
    )?;
    let (mut rows_removable, mut rows_held) = (0usize, 0usize);
    for (i, rows) in &naming {
        if (*i as usize) < removable {
            rows_removable += rows.len();
        } else {
            rows_held += rows.len();
        }
    }
    let out = json!({
        "schema": "emem.orphan_verify.v1",
        "removable_exported": removable,
        "removable_still_stored": still_stored,
        "index_rows_walked": walked,
        "index_rows_naming_removable": rows_removable,
        "held_exported": lines.len() - removable,
        "index_rows_naming_held": rows_held,
    });
    write_json_atomic(&dir.join(VERIFY_REPORT), &out)?;
    ctl.set(json!({"phase": "done"}));
    Ok(out)
}

/// The run directory for `run` under `maint_root`, refusing a name that is
/// not a plain token (it becomes a path).
pub fn run_dir(maint_root: &Path, run: &str) -> Result<PathBuf, StorageError> {
    let ok = !run.is_empty()
        && run.len() <= 64
        && run
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if !ok {
        return Err(err(
            "run must be 1 to 64 characters of letters, digits, '-' or '_'",
        ));
    }
    Ok(maint_root.join("orphans").join(run))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{FactPlanePolicy, MaterializingStorage, Storage};
    use ed25519_dalek::SigningKey;
    use emem_core::{AttesterKey, KeyEpoch};
    use emem_fact::{
        Attestation, Derivation, FactCid, PrimaryFact, RegistryCid, SchemaCid, Source,
    };
    use std::sync::Arc;

    const CELL: &str = "damO.zb000.xUti.zde78";
    const BAND: &str = "indices.ndvi";
    const NOW: &str = "2026-11-14T12:00:00Z";

    fn key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn storage() -> MaterializingStorage {
        let bands = Arc::new(emem_core::bands::DEFAULT.clone());
        let functions =
            Arc::new(emem_core::FunctionRegistry::parse_default().expect("default functions"));
        let sources =
            Arc::new(emem_core::SourceRegistry::parse_default().expect("default sources"));
        let s = MaterializingStorage::ephemeral(bands, functions, sources).expect("storage");
        s.set_fact_plane_policy(FactPlanePolicy {
            open: true,
            ..Default::default()
        });
        let r = s.redb().expect("redb");
        r.mark_table_backfill_done(KvTable::Proofs).unwrap();
        r.mark_table_backfill_done(KvTable::Multi).unwrap();
        s
    }

    pub(crate) fn fact(tslot: u64, value: f64, signed_at: &str) -> Fact {
        Fact::Primary(PrimaryFact {
            cell: CELL.into(),
            band: BAND.into(),
            tslot,
            value: ciborium::Value::Float(value),
            unit: None,
            confidence: 1.0,
            uncertainty: None,
            sources: vec![Source {
                scheme: "test".into(),
                id: format!("scene-{tslot}-{value}"),
                cid: None,
                hash: None,
                captured_at: None,
                url: None,
            }],
            derivation: Derivation {
                fn_key: "test@1".into(),
                args: None,
            },
            privacy_class: "public".into(),
            schema_cid: SchemaCid::new("test-schema"),
            signer: AttesterKey(key().verifying_key().to_bytes()),
            signed_at: signed_at.into(),
            served_via: None,
        })
    }

    pub(crate) fn att(facts: Vec<Fact>) -> Attestation {
        Attestation::build_and_sign_v1(
            facts,
            vec![],
            RegistryCid::new("test-registry"),
            SchemaCid::new("test-schema"),
            &key(),
            KeyEpoch(0),
            "2026-11-13T00:00:00Z".into(),
            None,
        )
        .expect("attestation")
    }

    /// The whole write path: body, index, log, proof, multi-attester row.
    async fn logged(s: &MaterializingStorage, facts: Vec<Fact>) -> Vec<FactCid> {
        s.put_attestation(&att(facts)).await.expect("logged write")
    }

    /// What a dropped caller left: body and index row, nothing after.
    async fn orphaned(s: &MaterializingStorage, facts: Vec<Fact>) -> Vec<FactCid> {
        s.cache.put_many(&facts).await.expect("put_many")
    }

    /// A fact the log has but the proof table never got, as every fact
    /// written before proofs existed.
    async fn logged_without_proof(s: &MaterializingStorage, facts: Vec<Fact>) -> Vec<FactCid> {
        let cids = s.cache.put_many(&facts).await.expect("put_many");
        s.log.append(&att(facts)).await.expect("log append");
        cids
    }

    fn key_bytes(tslot: u64) -> Vec<u8> {
        let mut k = format!("{CELL}\0{BAND}\0").into_bytes();
        k.extend_from_slice(&tslot.to_be_bytes());
        k
    }

    struct World {
        s: MaterializingStorage,
        dir: tempfile::TempDir,
        older: FactCid,
        over_older: FactCid,
        fresh: FactCid,
        displaced: FactCid,
        displacer: FactCid,
        no_proof: FactCid,
        recent: FactCid,
        plain: Vec<FactCid>,
    }

    impl World {
        fn store(&self) -> OrphanStore<'_> {
            OrphanStore {
                redb: self.r(),
                sled: self.s.hot_sled_db(),
                log_root: &self.s.log.root,
            }
        }
        fn r(&self) -> &RedbFacts {
            self.s.hot.as_ref().and_then(|h| h.redb()).expect("redb")
        }
        fn run(&self) -> PathBuf {
            run_dir(self.dir.path(), "t1").unwrap()
        }
        fn all(&self) -> Vec<Vec<u8>> {
            let mut v = vec![
                &self.older,
                &self.over_older,
                &self.fresh,
                &self.displaced,
                &self.displacer,
                &self.no_proof,
                &self.recent,
            ];
            v.extend(self.plain.iter());
            v.into_iter()
                .map(|c| c.as_str().as_bytes().to_vec())
                .collect()
        }
        fn index_at(&self, tslot: u64) -> Option<String> {
            self.r()
                .lookup(&key_bytes(tslot))
                .unwrap()
                .map(|v| String::from_utf8(v).unwrap())
        }
        fn present(&self, c: &FactCid) -> bool {
            self.r().contains_fact(c.as_str().as_bytes()).unwrap()
        }
    }

    async fn world() -> World {
        let s = storage();
        let older = logged(&s, vec![fact(1, 0.1, "2026-11-12T10:00:00Z")]).await[0].clone();
        let over_older = orphaned(&s, vec![fact(1, 0.2, "2026-11-13T16:00:00Z")]).await[0].clone();
        let fresh = orphaned(&s, vec![fact(2, 0.3, "2026-11-13T16:00:00Z")]).await[0].clone();
        let displaced = orphaned(&s, vec![fact(3, 0.4, "2026-11-13T16:00:00Z")]).await[0].clone();
        let displacer = logged(&s, vec![fact(3, 0.5, "2026-11-13T17:00:00Z")]).await[0].clone();
        let no_proof =
            logged_without_proof(&s, vec![fact(4, 0.6, "2026-05-01T00:00:00Z")]).await[0].clone();
        let recent = orphaned(&s, vec![fact(5, 0.7, "2026-11-14T11:30:00Z")]).await[0].clone();
        let plain = logged(
            &s,
            (6..=10)
                .map(|t| fact(t, t as f64 / 100.0, "2026-11-10T00:00:00Z"))
                .collect(),
        )
        .await;
        World {
            s,
            dir: tempfile::tempdir().unwrap(),
            older,
            over_older,
            fresh,
            displaced,
            displacer,
            no_proof,
            recent,
            plain,
        }
    }

    fn opts() -> ScanOptions {
        ScanOptions {
            duty: 1.0,
            log_mb_s: 500,
            now_unix: crate::rfc3339_instant(NOW).map(|(s, _)| s),
            ..ScanOptions::default()
        }
    }

    fn export(w: &World) -> Vec<ExportLine> {
        std::fs::read_to_string(w.run().join(EXPORT))
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    /// Every index row, which facts are stored, and the proof and
    /// multi-attester row counts.
    type Snapshot = (emem_cache::redb_facts::IndexRows, Vec<bool>, u64, u64);

    fn snapshot(w: &World) -> Snapshot {
        let r = w.r();
        let all = w.all();
        let refs: Vec<&[u8]> = all.iter().map(Vec::as_slice).collect();
        (
            r.index_page(None, 100_000).unwrap(),
            r.facts_present_many(&refs).unwrap(),
            r.kv_len(KvTable::Proofs).unwrap(),
            r.kv_len(KvTable::Multi).unwrap(),
        )
    }

    fn log_bytes(w: &World) -> Vec<(String, Vec<u8>)> {
        let mut v: Vec<(String, Vec<u8>)> = std::fs::read_dir(&w.s.log.root)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| {
                (
                    e.file_name().to_string_lossy().into_owned(),
                    std::fs::read(e.path()).unwrap(),
                )
            })
            .filter(|(n, _)| n.starts_with("merkle.log."))
            .collect();
        v.sort();
        v
    }

    #[tokio::test]
    async fn the_scan_finds_the_unlogged_and_only_them() {
        let w = world().await;
        let rep = scan(&w.store(), &w.run(), &opts(), &JobControl::new()).unwrap();
        assert_eq!(rep["log_check_ok"], true, "{rep:#}");
        assert_eq!(rep["walk"]["facts_seen"], 12);
        assert_eq!(
            rep["walk"]["without_proof"], 5,
            "four orphans and the old fact"
        );
        assert_eq!(
            rep["without_proof_found_in_log"], 1,
            "the fact with no proof row that the log has is not an orphan"
        );
        assert_eq!(rep["unlogged"], 4);
        assert_eq!(rep["deletable"], 3);
        assert_eq!(rep["held"]["signed_too_recently"], 1);
        assert_eq!(rep["unlogged_by_band"][BAND], 4);
        assert_eq!(rep["unlogged_by_signed_day"]["2026-11-13"], 3);
        assert!(rep["controls"]["checked"].as_u64().unwrap() > 0);
        assert_eq!(rep["controls"]["checked"], rep["controls"]["found_in_log"]);
        assert_eq!(rep["index"]["rows_naming_unlogged"], 3);
        assert_eq!(rep["index"]["to_point_back"], 1);
        assert_eq!(rep["index"]["to_remove"], 2);

        let lines = export(&w);
        let mut cids: Vec<&str> = lines.iter().map(|l| l.cid.as_str()).collect();
        cids.sort();
        let mut want = vec![
            w.over_older.as_str(),
            w.fresh.as_str(),
            w.displaced.as_str(),
            w.recent.as_str(),
        ];
        want.sort();
        assert_eq!(cids, want);
        let over = lines
            .iter()
            .find(|l| l.cid == w.over_older.as_str())
            .unwrap();
        assert_eq!(over.index_rows.len(), 1);
        assert_eq!(
            over.index_rows[0].replace_with.as_deref(),
            Some(w.older.as_str()),
            "the slot goes back to the logged fact it displaced"
        );
        let displaced = lines
            .iter()
            .find(|l| l.cid == w.displaced.as_str())
            .unwrap();
        assert!(
            displaced.index_rows.is_empty(),
            "a later logged write already holds its slot"
        );
        let recent = lines.iter().find(|l| l.cid == w.recent.as_str()).unwrap();
        assert!(!recent.delete);
        assert_eq!(recent.hold.as_deref(), Some("signed_too_recently"));
    }

    #[tokio::test]
    async fn the_export_round_trips_to_the_stored_bytes() {
        let w = world().await;
        scan(&w.store(), &w.run(), &opts(), &JobControl::new()).unwrap();
        let lines = export(&w);
        assert_eq!(lines.len(), 4);
        for l in &lines {
            let bytes = data_encoding::BASE64.decode(l.cbor_b64.as_bytes()).unwrap();
            assert_eq!(
                Some(*blake3::hash(&bytes).as_bytes()),
                digest_of_cid(l.cid.as_bytes()),
                "the backup's bytes are what the cid addresses"
            );
            assert_eq!(
                w.r().get_fact(l.cid.as_bytes()).unwrap().as_deref(),
                Some(bytes.as_slice()),
                "and they are the stored body, byte for byte"
            );
            let f: Fact = ciborium::de::from_reader(bytes.as_slice()).unwrap();
            assert_eq!(
                emem_cache::fact_cid_of(&f).unwrap().as_str(),
                l.cid,
                "and they decode to a fact that re-encodes to the same cid"
            );
        }
        let rep: Value = read_json(&w.run().join(REPORT)).unwrap().unwrap();
        let (h, _) = file_blake3(&w.run().join(EXPORT)).unwrap();
        assert_eq!(rep["export"]["blake3"], h.as_str());
    }

    #[tokio::test]
    async fn a_dry_run_counts_and_changes_nothing() {
        let w = world().await;
        scan(&w.store(), &w.run(), &opts(), &JobControl::new()).unwrap();
        let before = snapshot(&w);
        let logs = log_bytes(&w);
        let dry = DeleteOptions {
            duty: 1.0,
            ..DeleteOptions::default()
        };
        let out = delete(&w.store(), &w.run(), &dry, &JobControl::new()).unwrap();
        assert_eq!(out["applied"], false);
        assert_eq!(out["tally"]["removed"], 3);
        assert_eq!(out["tally"]["index_repointed"], 1);
        assert_eq!(out["tally"]["index_removed"], 1);
        assert_eq!(out["tally"]["held"], 1);
        assert_eq!(snapshot(&w), before, "a dry run wrote to the store");
        assert_eq!(log_bytes(&w), logs);
        assert!(!w.run().join(DELETE_STATE).exists());
    }

    #[tokio::test]
    async fn the_delete_removes_orphans_and_keeps_every_logged_fact() {
        let w = world().await;
        let rep = scan(&w.store(), &w.run(), &opts(), &JobControl::new()).unwrap();
        let logs = log_bytes(&w);
        let proofs = w.r().kv_len(KvTable::Proofs).unwrap();
        let multi = w.r().kv_len(KvTable::Multi).unwrap();
        let mut o = DeleteOptions {
            apply: true,
            duty: 1.0,
            batch: 2,
            ..DeleteOptions::default()
        };
        assert!(
            delete(&w.store(), &w.run(), &o, &JobControl::new()).is_err(),
            "apply without confirm must refuse"
        );
        o.confirm = rep["export"]["blake3"].as_str().unwrap().to_string();
        let out = delete(&w.store(), &w.run(), &o, &JobControl::new()).unwrap();
        assert_eq!(out["tally"]["removed"], 3, "{out:#}");
        assert_eq!(out["tally"]["skipped_has_proof"], 0);

        for c in [&w.older, &w.displacer, &w.no_proof, &w.recent] {
            assert!(w.present(c), "{} is gone", c.as_str());
        }
        for c in &w.plain {
            assert!(w.present(c));
        }
        for c in [&w.over_older, &w.fresh, &w.displaced] {
            assert!(!w.present(c), "orphan {} survived", c.as_str());
        }
        assert_eq!(
            w.index_at(1).as_deref(),
            Some(w.older.as_str()),
            "pointed back"
        );
        assert_eq!(
            w.index_at(2),
            None,
            "removed, so the next read materialises"
        );
        assert_eq!(w.index_at(3).as_deref(), Some(w.displacer.as_str()));
        assert_eq!(w.index_at(4).as_deref(), Some(w.no_proof.as_str()));
        assert_eq!(w.index_at(5).as_deref(), Some(w.recent.as_str()), "held");
        assert_eq!(w.r().kv_len(KvTable::Proofs).unwrap(), proofs);
        assert_eq!(w.r().kv_len(KvTable::Multi).unwrap(), multi);
        assert_eq!(log_bytes(&w), logs, "the log is never written");

        // Idempotent: a second full pass finds everything done.
        std::fs::remove_file(w.run().join(DELETE_STATE)).unwrap();
        let again = delete(&w.store(), &w.run(), &o, &JobControl::new()).unwrap();
        assert_eq!(again["tally"]["removed"], 0);
        assert_eq!(again["tally"]["already_gone"], 3);
        assert_eq!(again["tally"]["index_moved_on"], 2);
        assert_eq!(w.index_at(1).as_deref(), Some(w.older.as_str()));

        let v = verify(&w.store(), &w.run(), 1.0, &JobControl::new()).unwrap();
        assert_eq!(v["removable_still_stored"], 0);
        assert_eq!(v["index_rows_naming_removable"], 0);
        assert_eq!(v["held_exported"], 1);
        assert_eq!(v["index_rows_naming_held"], 1);
    }

    #[tokio::test]
    async fn a_stopped_walk_resumes_without_counting_twice() {
        let w = world().await;
        let o = ScanOptions { page: 2, ..opts() };
        let stop = JobControl {
            stop_after_pages: Some(3),
            ..JobControl::default()
        };
        assert!(scan(&w.store(), &w.run(), &o, &stop).is_err());
        let mid: WalkState = read_json(&w.run().join(SCAN_STATE)).unwrap().unwrap();
        assert_eq!(mid.pages, 3);
        assert!(!mid.done);
        let rep = scan(&w.store(), &w.run(), &o, &JobControl::new()).unwrap();
        assert_eq!(rep["walk"]["facts_seen"], 12);
        assert_eq!(rep["walk"]["without_proof"], 5);
        assert_eq!(rep["unlogged"], 4);
        assert_eq!(spool_records(&w.run().join(SPOOL)).unwrap().len(), 5);
    }

    #[tokio::test]
    async fn a_log_pass_that_finds_no_control_fails_closed() {
        let w = world().await;
        let empty = tempfile::tempdir().unwrap();
        let store = OrphanStore {
            redb: w.r(),
            sled: None,
            log_root: empty.path(),
        };
        let rep = scan(&store, &w.run(), &opts(), &JobControl::new()).unwrap();
        assert_eq!(rep["log_check_ok"], false);
        assert_eq!(rep["deletable"], 0);
        assert!(export(&w).iter().all(|l| !l.delete));
        let o = DeleteOptions {
            duty: 1.0,
            ..DeleteOptions::default()
        };
        assert!(delete(&w.store(), &w.run(), &o, &JobControl::new()).is_err());
    }

    #[tokio::test]
    async fn a_fact_with_a_proof_is_never_removed_whatever_the_export_says() {
        let w = world().await;
        let body = w
            .r()
            .get_fact(w.older.as_str().as_bytes())
            .unwrap()
            .unwrap();
        let item = OrphanRemoval {
            cid: w.older.as_str().as_bytes().to_vec(),
            body,
            key: Some(key_bytes(1)),
            index_rows: vec![],
            keep_body: false,
        };
        let t = w.r().remove_orphans(&[item], true).unwrap();
        assert_eq!(t.skipped_has_proof, 1);
        assert_eq!(t.removed, 0);
        assert!(w.present(&w.older));
    }

    #[tokio::test]
    async fn a_damaged_export_stops_the_delete() {
        let w = world().await;
        let rep = scan(&w.store(), &w.run(), &opts(), &JobControl::new()).unwrap();
        let path = w.run().join(EXPORT);
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(
            &path,
            text.replacen("\"delete\":true", "\"delete\":false", 1),
        )
        .unwrap();
        let o = DeleteOptions {
            apply: true,
            duty: 1.0,
            confirm: rep["export"]["blake3"].as_str().unwrap().to_string(),
            ..DeleteOptions::default()
        };
        let e = delete(&w.store(), &w.run(), &o, &JobControl::new()).unwrap_err();
        assert!(e.to_string().contains("export changed"), "{e}");
    }

    fn framed(cbor: &[u8]) -> Vec<u8> {
        let mut v = (cbor.len() as u32).to_le_bytes().to_vec();
        v.extend_from_slice(cbor);
        v.extend_from_slice(blake3::hash(cbor).as_bytes());
        v
    }

    /// A segment as the production tears left them: an attestation cut at a
    /// 4 KiB boundary, then whole records appended from that boundary.
    /// Returns the fact in the cut record, the fact logged after the cut, and
    /// the position of the tear.
    async fn torn_segment(w: &World) -> (FactCid, FactCid, String) {
        let mut many: Vec<Fact> = (40..70)
            .map(|t| fact(t, t as f64, "2026-11-13T16:00:00Z"))
            .collect();
        let cut_cids = orphaned(&w.s, many.drain(..1).collect()).await;
        let mut cut_facts = vec![fact(40, 40.0, "2026-11-13T16:00:00Z")];
        cut_facts.extend(many);
        let mut cut = Vec::new();
        ciborium::ser::into_writer(&att(cut_facts), &mut cut).unwrap();
        let after = vec![fact(80, 0.8, "2026-11-13T16:00:00Z")];
        let after_cids = orphaned(&w.s, after.clone()).await;
        let mut whole = Vec::new();
        ciborium::ser::into_writer(&att(after), &mut whole).unwrap();
        let cut_frame = framed(&cut);
        assert!(cut_frame.len() > 4096, "{}", cut_frame.len());
        let mut seg = cut_frame[..4096].to_vec();
        seg.extend(framed(&whole));
        let other = att(vec![fact(81, 0.9, "2026-11-13T16:00:00Z")]);
        let mut o = Vec::new();
        ciborium::ser::into_writer(&other, &mut o).unwrap();
        seg.extend(framed(&o));
        std::fs::write(w.s.log.root.join("merkle.log.7"), &seg).unwrap();
        (
            cut_cids[0].clone(),
            after_cids[0].clone(),
            "7:0".to_string(),
        )
    }

    /// A tear fails the log check until the operator accepts its position.
    /// The pass reads on past it, so a fact logged after the tear is found
    /// and never exported, and a fact whole in the cut record is held.
    #[tokio::test]
    async fn a_torn_log_is_read_past_its_tear_and_checked_by_position() {
        let w = world().await;
        let (in_cut, after, pos) = torn_segment(&w).await;
        let run = |name: &str, accept: Vec<String>| {
            let o = ScanOptions {
                accept_log_problems: accept,
                ..opts()
            };
            scan(
                &w.store(),
                &run_dir(w.dir.path(), name).unwrap(),
                &o,
                &JobControl::new(),
            )
            .unwrap()
        };

        let rep = run("t-none", vec![]);
        assert_eq!(rep["log_check_ok"], false, "{rep:#}");
        assert_eq!(rep["log_problems_at"][0]["position"], pos.as_str());
        assert_eq!(rep["log_problems_at"][0]["accepted"], false);
        assert_eq!(rep["log"]["after_a_tear"], 2);
        assert_eq!(rep["deletable"], 0);

        let rep = run("t-accept", vec![pos.clone()]);
        assert_eq!(rep["log_check_ok"], true, "{rep:#}");
        assert_eq!(rep["log_problems_at"][0]["accepted"], true);
        assert!(rep["log_problems_at"][0]["facts_named"].as_u64().unwrap() >= 1);
        assert_eq!(rep["held"]["in_damaged_log_record"], 1);
        let lines: Vec<ExportLine> =
            std::fs::read_to_string(run_dir(w.dir.path(), "t-accept").unwrap().join(EXPORT))
                .unwrap()
                .lines()
                .map(|l| serde_json::from_str(l).unwrap())
                .collect();
        assert!(
            !lines.iter().any(|l| l.cid == after.as_str()),
            "a fact logged after the tear is found, so never exported"
        );
        let cut = lines.iter().find(|l| l.cid == in_cut.as_str()).unwrap();
        assert!(!cut.delete);
        assert_eq!(cut.hold.as_deref(), Some("in_damaged_log_record"));
        assert_eq!(rep["deletable"], 3, "the three orphans of the world");

        let rep = run("t-extra", vec![pos, "7:99".into()]);
        assert_eq!(rep["log_check_ok"], false);
        assert!(rep["log_problems"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p.as_str().unwrap().contains("7:99")));
    }

    #[test]
    fn the_record_walk_tells_no_facts_from_unreadable() {
        let a = att(vec![
            fact(1, 0.1, "2026-11-12T10:00:00Z"),
            fact(2, 0.2, "2026-11-12T10:00:00Z"),
        ]);
        let mut buf = Vec::new();
        ciborium::ser::into_writer(&a, &mut buf).unwrap();
        match entry_fact_spans(&buf) {
            EntryFacts::Spans(s) => {
                assert_eq!(s.len(), 2);
                for ((x, y), f) in s.iter().zip(&a.facts) {
                    assert_eq!(
                        blake3::hash(&buf[*x..*y]).as_bytes(),
                        &digest_of_cid(emem_cache::fact_cid_of(f).unwrap().as_str().as_bytes())
                            .unwrap()
                    );
                }
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            entry_fact_spans(&buf[..buf.len() / 2]),
            EntryFacts::Malformed
        );
        let mut note = Vec::new();
        ciborium::ser::into_writer(&json!({"kind": "memory_write", "path": "/x"}), &mut note)
            .unwrap();
        assert_eq!(entry_fact_spans(&note), EntryFacts::NoFacts);
        let mut arr = Vec::new();
        ciborium::ser::into_writer(&vec![1u8, 2, 3], &mut arr).unwrap();
        assert_eq!(entry_fact_spans(&arr), EntryFacts::NoFacts);
    }

    #[test]
    fn run_names_cannot_leave_the_maintenance_directory() {
        let root = Path::new("/data/maint");
        assert!(run_dir(root, "2026-11-14a").is_ok());
        for bad in ["", "../x", "a/b", "a b", &"x".repeat(65)] {
            assert!(run_dir(root, bad).is_err(), "{bad:?}");
        }
    }
}
