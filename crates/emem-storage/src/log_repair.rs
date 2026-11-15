//! Log entries that are on disk but are not leaves of the transparency tree:
//! find them, append them again at the head, and link their facts' proofs to
//! the new leaves.
//!
//! # How entries fell out of the tree
//!
//! Before the writer cut a failed write back (`write_whole_or_nothing` in
//! `merkle_log.rs`), a write that a full disk stopped part way left a partial
//! frame on disk, and the next append started at the end of the last block
//! written. The tree's length-driven walk then read the torn length. Where
//! the declared frame still fitted in the file, the walk took the 32 bytes at
//! its end as a leaf, which is the hash of no entry. In every case the walk
//! stopped reading that segment soon after, so the intact entries after the
//! tear are on disk and under no signed head.
//!
//! # The repair
//!
//! The tree is not read another way. A walk that resyncs would put leaves
//! into the middle of the tree, and no head signed since the first tear would
//! be a prefix of the result. Instead:
//!
//! 1. [`scan`] walks every segment, checks every frame and resyncs past each
//!    tear ([`crate::log_walk`]). It lists each tear, each leaf the tree took
//!    from a torn frame, and each entry that is not a leaf where it lies. An
//!    entry whose hash is a leaf anywhere in the tree counts as a leaf.
//! 2. [`relog`] appends each listed entry's exact CBOR again through the
//!    normal append path, so its leaf is `blake3` of the same bytes, and a
//!    receipt or a note that names its entry hash now finds it. An entry whose
//!    hash is already a leaf is skipped, so a second run appends nothing. Then
//!    the proof row of each fact in a relogged attestation is linked to that
//!    entry (`log_entry_hash`), as `put_attestation` links a new one.
//!
//! The new tree is the old tree with leaves appended, so every head signed
//! before the relog is a prefix of every head after it. Nothing in an old
//! segment is written, moved or removed.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use emem_cache::{KvTable, RedbFacts};
use emem_fact::MerkleProof;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::fact_locator::{self, entry_fact_spans, fact_spans_in_prefix, EntryFacts};
use crate::log_walk::{segment_indices, SegmentWalk, Step};
use crate::merkle_log::AttestationLog;
use crate::orphans::{
    cancelled, err, file_blake3, now_unix, pause, read_json, sleep_unless_cancelled,
    write_json_atomic, JobControl,
};
use crate::StorageError;

const REPORT: &str = "report.json";
const ENTRIES: &str = "entries.jsonl";
const JOURNAL: &str = "relog.jsonl";
const RELOCATIONS: &str = "relocations.json";
const RELOG_REPORT: &str = "relog.report.json";

/// What a repair reads and, for `relog`, writes.
pub struct RepairStore<'a> {
    pub log: &'a AttestationLog,
    /// Where proof rows are read and written. Absent on a node without it.
    pub redb: Option<&'a RedbFacts>,
    /// The sled store: proof rows still being copied to redb, and facts on a
    /// node without redb.
    pub sled: Option<&'a sled::Db>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct RepairOptions {
    /// Read rate cap for the scan, MB/s.
    pub log_mb_s: u64,
    /// Entries appended per group write, 1 to 512.
    pub batch: usize,
    /// Share of wall time the relog spends working (0.05 to 1).
    pub duty: f64,
}

impl Default for RepairOptions {
    fn default() -> Self {
        Self {
            log_mb_s: 50,
            batch: 64,
            duty: 0.25,
        }
    }
}

/// An entry on disk that is not a leaf where it lies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OffTreeEntry {
    pub segment: u64,
    pub offset: u64,
    pub cbor_len: u64,
    /// `blake3` of its CBOR, base32 lower case without padding.
    pub entry_hash: String,
    /// `attestation`, or the `kind` a memory entry names.
    pub kind: String,
    pub facts: u64,
    /// The note path of a memory entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The cid of each fact of an attestation, in its order: what a check
    /// of `/v1/log/inclusion?fact_cid=` after the relog asks with.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fact_cids: Vec<String>,
    /// Where its hash was a leaf when the scan ran, if anywhere.
    pub leaf_index: Option<u64>,
}

/// One line of `relocations.json`: an entry's place on disk and the leaf
/// that now holds it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Relocation {
    pub segment: u64,
    pub offset: u64,
    pub entry_hash: String,
    pub kind: String,
    pub facts: u64,
    pub leaf_index: u64,
    /// Appended by a relog; false when its hash was already a leaf.
    pub relogged: bool,
}

/// What linking the proofs of relogged facts did.
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize)]
pub struct LinkTally {
    pub facts: u64,
    /// A proof row of this batch now names the entry.
    pub linked: u64,
    pub already_linked: u64,
    /// No proof row: one was built from the entry and stored.
    pub created: u64,
    /// The proof row names another batch, which proves the fact on its own.
    pub other_batch: u64,
    /// No proof row and no stored fact, so nothing was written.
    pub fact_absent: u64,
    /// No proof row, and the facts alone do not rebuild the batch root (the
    /// batch also carried edges), so no proof was built.
    pub root_differs: u64,
    /// Fact locators pointed at the entry's new place.
    pub locators_moved: u64,
}

fn b32(h: &[u8]) -> String {
    data_encoding::BASE32_NOPAD.encode(h).to_lowercase()
}

fn unb32(s: &str) -> Option<[u8; 32]> {
    data_encoding::BASE32_NOPAD
        .decode(s.to_ascii_uppercase().as_bytes())
        .ok()?
        .try_into()
        .ok()
}

/// The run directory for `run` under `maint_root`. A run name becomes a
/// path, so it must be a plain token.
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
    Ok(maint_root.join("log-repair").join(run))
}

/// `(kind, fact cids, note path)` of one entry.
fn describe(cbor: &[u8]) -> (String, Vec<String>, Option<String>) {
    let mut kind = None;
    let mut path = None;
    if let Ok(ciborium::Value::Map(m)) = ciborium::de::from_reader::<ciborium::Value, _>(cbor) {
        for (k, v) in m {
            match (k.as_text(), v) {
                (Some("kind"), ciborium::Value::Text(t)) => kind = Some(t),
                (Some("path"), ciborium::Value::Text(t)) => path = Some(t),
                _ => {}
            }
        }
    }
    let facts = match entry_fact_spans(cbor) {
        EntryFacts::Spans(s) => s
            .iter()
            .map(|&(a, b)| b32(blake3::hash(&cbor[a..b]).as_bytes()))
            .collect(),
        EntryFacts::NoFacts | EntryFacts::Malformed => Vec::new(),
    };
    (
        kind.unwrap_or_else(|| "attestation".to_string()),
        facts,
        path,
    )
}

/// For each hash, the first leaf index where the tree holds it, and the
/// tree's size.
fn leaf_positions(
    log: &AttestationLog,
    hashes: &[[u8; 32]],
) -> std::io::Result<(Vec<Option<u64>>, u64)> {
    let mut want: HashMap<[u8; 32], Vec<usize>> = HashMap::with_capacity(hashes.len());
    for (i, h) in hashes.iter().enumerate() {
        want.entry(*h).or_default().push(i);
    }
    let mut out = vec![None; hashes.len()];
    let mut index = 0u64;
    for (_, leaves) in log.leaf_hashes_from(0)? {
        for leaf in leaves {
            if let Some(v) = want.remove(&leaf) {
                for i in v {
                    out[i] = Some(index);
                }
            }
            index += 1;
        }
    }
    Ok((out, index))
}

// ── scan ─────────────────────────────────────────────────────────────────

/// Walk every segment, list each tear, each leaf the tree took from a torn
/// frame, and each entry that is not a leaf, and write `report.json` and
/// `entries.jsonl` into `dir`. Reads only.
pub fn scan(
    store: &RepairStore<'_>,
    dir: &Path,
    opts: &RepairOptions,
    ctl: &JobControl,
) -> Result<Value, StorageError> {
    std::fs::create_dir_all(dir)?;
    let t0 = Instant::now();
    let root = &store.log.root;
    let segs = segment_indices(root)?;
    let newest = segs.last().copied();
    let walked = segs.len();
    let rate = (opts.log_mb_s.clamp(5, 500) * 1_000_000) as f64;
    let (mut bytes, mut entries_on_disk, mut base) = (0u64, 0u64, 0u64);
    let mut tears: Vec<Value> = Vec::new();
    let mut misframed: Vec<Value> = Vec::new();
    let mut in_flight: Vec<Value> = Vec::new();
    let mut segments: Vec<Value> = Vec::new();
    let mut off: Vec<OffTreeEntry> = Vec::new();
    for seg in segs {
        if ctl.cancelled() {
            return Err(cancelled());
        }
        let mut w = SegmentWalk::open(&root.join(format!("merkle.log.{seg}")))?;
        let mut seg_tears = Vec::new();
        let (mut seg_entries, mut seg_off, mut paced) = (0u64, 0u64, 0u64);
        while let Some(step) = w.next_step()? {
            match step {
                Step::Entry {
                    offset,
                    cbor,
                    hash,
                    tree_ordinal,
                } => {
                    seg_entries += 1;
                    if tree_ordinal.is_none() {
                        let (kind, fact_cids, path) = describe(cbor);
                        off.push(OffTreeEntry {
                            segment: seg,
                            offset,
                            cbor_len: cbor.len() as u64,
                            entry_hash: b32(&hash),
                            kind,
                            facts: fact_cids.len() as u64,
                            path,
                            fact_cids,
                            leaf_index: None,
                        });
                        seg_off += 1;
                    }
                }
                Step::Tear { tear, partial } => {
                    // Facts whole in the torn record's surviving bytes: never
                    // logged whole, so the orphan pass holds them.
                    let named: Vec<String> = fact_spans_in_prefix(partial)
                        .iter()
                        .map(|&(a, b)| b32(blake3::hash(&partial[a..b]).as_bytes()))
                        .collect();
                    seg_tears.push((tear, named));
                }
            }
            if w.pos() - paced > 16 << 20 {
                paced = w.pos();
                let due = Duration::from_secs_f64((bytes + paced) as f64 / rate);
                if due > t0.elapsed() {
                    sleep_unless_cancelled(due - t0.elapsed(), ctl);
                }
                if ctl.cancelled() {
                    return Err(cancelled());
                }
            }
        }
        for (ord, f) in w.misframed() {
            misframed.push(json!({
                "leaf_index": base + ord,
                "segment": seg,
                "offset": f.offset,
                "declared_len": f.cbor_len,
                "tree_leaf_b32": b32(&f.leaf),
            }));
        }
        for (t, named) in &seg_tears {
            let row = json!({
                "segment": seg,
                "offset": t.offset,
                "kind": t.kind.as_str(),
                "declared_len": t.declared_len,
                "resume_offset": t.resume,
                "gap": t.resume.unwrap_or(w.len()) - t.offset,
                "resume_block_aligned": t.resume.map(|r| r % 4096 == 0),
                "leaf_index": t.tree_ordinal.map(|k| base + k),
                "tree": if t.tree_ordinal.is_some() {
                    "misframed_leaf"
                } else if t.tree_stops_here {
                    "tree_walk_ends"
                } else {
                    "after_tree_walk_end"
                },
                "surviving_fact_cids": named,
            });
            // The newest segment can end in a write still in flight.
            if Some(seg) == newest && t.resume.is_none() {
                in_flight.push(row);
            } else {
                tears.push(row);
            }
        }
        if !seg_tears.is_empty() || seg_off > 0 {
            segments.push(json!({
                "segment": seg,
                "bytes": w.len(),
                "entries": seg_entries,
                "tree_leaves": w.tree_leaves(),
                "trailer": w.has_trailer(),
                "first_leaf_index": base,
                "off_tree_entries": seg_off,
            }));
        }
        entries_on_disk += seg_entries;
        bytes += w.len();
        base += w.tree_leaves();
        ctl.set(json!({"phase": "scan", "segment": seg, "bytes": bytes, "off_tree": off.len()}));
    }

    ctl.set(json!({"phase": "leaf_set", "off_tree": off.len()}));
    let hashes: Vec<[u8; 32]> = off
        .iter()
        .map(|e| unb32(&e.entry_hash).unwrap_or_default())
        .collect();
    let (at, tree_now) = leaf_positions(store.log, &hashes)?;
    for (e, a) in off.iter_mut().zip(&at) {
        e.leaf_index = *a;
    }

    let tmp = dir.join(format!("{ENTRIES}.tmp"));
    {
        let mut f = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
        for e in &off {
            serde_json::to_writer(&mut f, e).map_err(|e| err(e.to_string()))?;
            f.write_all(b"\n")?;
        }
        f.flush()?;
        f.get_ref().sync_all()?;
    }
    let entries_path = dir.join(ENTRIES);
    std::fs::rename(&tmp, &entries_path)?;
    let (entries_blake3, entries_bytes) = file_blake3(&entries_path)?;

    let to_relog: Vec<&OffTreeEntry> = off.iter().filter(|e| e.leaf_index.is_none()).collect();
    let mut by_kind: BTreeMap<String, u64> = BTreeMap::new();
    for e in &to_relog {
        *by_kind.entry(e.kind.clone()).or_default() += 1;
    }
    let report = json!({
        "schema": "emem.log_repair.scan.v1",
        "dir": dir.display().to_string(),
        "scanned_unix": now_unix(),
        "segments": {"walked": walked, "with_tears_or_off_tree_entries": segments},
        "bytes": bytes,
        "entries_on_disk": entries_on_disk,
        "tree_leaves_walked": base,
        "tree_size_after_scan": tree_now,
        "tears": tears,
        "misframed_leaves": misframed,
        "in_flight_tail": in_flight,
        "off_tree": {
            "entries": off.len(),
            "already_a_leaf": off.len() - to_relog.len(),
            "to_relog": to_relog.len(),
            "facts_to_relog": to_relog.iter().map(|e| e.facts).sum::<u64>(),
            "to_relog_by_kind": by_kind,
            "entries_file": {
                "path": entries_path.display().to_string(),
                "lines": off.len(),
                "bytes": entries_bytes,
                "blake3": entries_blake3,
            },
        },
        "ms": t0.elapsed().as_millis() as u64,
    });
    write_json_atomic(&dir.join(REPORT), &report)?;
    ctl.set(json!({"phase": "done"}));
    Ok(report)
}

// ── relog ────────────────────────────────────────────────────────────────

fn read_entries(dir: &Path) -> Result<Vec<OffTreeEntry>, StorageError> {
    let report: Value = read_json(&dir.join(REPORT))?
        .ok_or_else(|| err("no report.json in this run; scan first"))?;
    let expected = report
        .pointer("/off_tree/entries_file/blake3")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let path = dir.join(ENTRIES);
    let (actual, _) = file_blake3(&path)?;
    if actual != expected {
        return Err(err(format!(
            "entries.jsonl hashes to {actual} and report.json says {expected}; scan again"
        )));
    }
    let mut out = Vec::new();
    for l in BufReader::new(std::fs::File::open(&path)?).lines() {
        out.push(serde_json::from_str(&l?).map_err(|e| err(e.to_string()))?);
    }
    Ok(out)
}

/// An entry's CBOR, read again from where the scan found it, and checked
/// against the hash the scan recorded.
fn read_entry(root: &Path, e: &OffTreeEntry, hash: &[u8; 32]) -> Result<Vec<u8>, StorageError> {
    let f = std::fs::File::open(root.join(format!("merkle.log.{}", e.segment)))?;
    let mut frame = vec![0u8; 4 + e.cbor_len as usize + 32];
    f.read_exact_at(&mut frame, e.offset)?;
    let n = u64::from(u32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]));
    let (cbor, stored) = frame[4..].split_at(e.cbor_len as usize);
    if n != e.cbor_len || blake3::hash(cbor).as_bytes() != hash || stored != hash {
        return Err(err(format!(
            "segment {} at offset {} no longer holds entry {}; nothing more is appended",
            e.segment, e.offset, e.entry_hash
        )));
    }
    Ok(cbor.to_vec())
}

#[derive(Debug, Serialize, Deserialize)]
struct JournalLine {
    entry_hash: String,
    segment: u64,
    offset: u64,
    new_segment: u64,
    new_offset: u64,
}

/// Append every entry the scan listed that is not yet a leaf, link the
/// proofs of their facts, and write `relocations.json` and
/// `relog.report.json`. Resumable and idempotent: what is already a leaf is
/// never appended again, and a linked proof is left as it is.
pub fn relog(
    store: &RepairStore<'_>,
    dir: &Path,
    opts: &RepairOptions,
    ctl: &JobControl,
    rt: &tokio::runtime::Handle,
) -> Result<Value, StorageError> {
    let t0 = Instant::now();
    let entries = read_entries(dir)?;
    let mut hashes = Vec::with_capacity(entries.len());
    for e in &entries {
        hashes.push(unb32(&e.entry_hash).ok_or_else(|| err("entries.jsonl holds a bad hash"))?);
    }
    ctl.set(json!({"phase": "leaf_set", "entries": entries.len()}));
    let (at_start, _) = leaf_positions(store.log, &hashes)?;
    let mut seen: HashSet<[u8; 32]> = HashSet::new();
    let todo: Vec<usize> = (0..entries.len())
        .filter(|&i| at_start[i].is_none() && seen.insert(hashes[i]))
        .collect();

    let mut journal = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(JOURNAL))?;
    let batch = opts.batch.clamp(1, 512);
    let mut appended = 0u64;
    for chunk in todo.chunks(batch) {
        if ctl.cancelled() {
            return Err(cancelled());
        }
        let t = Instant::now();
        let mut bufs = Vec::with_capacity(chunk.len());
        for &i in chunk {
            bufs.push(read_entry(&store.log.root, &entries[i], &hashes[i])?);
        }
        let results = rt.block_on(store.log.append_cbor_many(bufs));
        let mut failed = None;
        let mut lines = Vec::new();
        for (&i, r) in chunk.iter().zip(results) {
            match r {
                Ok(out) if out.record_hash == hashes[i] => {
                    appended += 1;
                    serde_json::to_writer(
                        &mut lines,
                        &JournalLine {
                            entry_hash: entries[i].entry_hash.clone(),
                            segment: entries[i].segment,
                            offset: entries[i].offset,
                            new_segment: out.segment_index,
                            new_offset: out.offset_in_segment,
                        },
                    )
                    .map_err(|e| err(e.to_string()))?;
                    lines.push(b'\n');
                }
                Ok(_) => {
                    failed.get_or_insert_with(|| err("an appended entry hashed differently"));
                }
                Err(e) => {
                    failed.get_or_insert(StorageError::Io(e));
                }
            }
        }
        journal.write_all(&lines)?;
        journal.sync_data()?;
        ctl.set(json!({"phase": "append", "appended": appended, "of": todo.len()}));
        if let Some(e) = failed {
            return Err(e);
        }
        sleep_unless_cancelled(pause(t.elapsed(), opts.duty), ctl);
    }

    let mut moved: HashMap<String, (u64, u64)> = HashMap::new();
    for l in BufReader::new(std::fs::File::open(dir.join(JOURNAL))?).lines() {
        let j: JournalLine = serde_json::from_str(&l?).map_err(|e| err(e.to_string()))?;
        moved.insert(j.entry_hash, (j.new_segment, j.new_offset));
    }
    ctl.set(json!({"phase": "leaf_set_after", "appended": appended}));
    let (at, tree_after) = leaf_positions(store.log, &hashes)?;

    let mut tally = LinkTally::default();
    let with_facts: Vec<usize> = (0..entries.len())
        .filter(|&i| entries[i].facts > 0 && at[i].is_some())
        .collect();
    for chunk in with_facts.chunks(batch) {
        if ctl.cancelled() {
            return Err(cancelled());
        }
        let t = Instant::now();
        let mut rows = Vec::new();
        for &i in chunk {
            let cbor = read_entry(&store.log.root, &entries[i], &hashes[i])?;
            let new_place = moved.get(&entries[i].entry_hash).copied();
            link_proofs(store, &cbor, &hashes[i], new_place, &mut tally, &mut rows)?;
        }
        put_proofs(store, &rows)?;
        ctl.set(json!({"phase": "link_proofs", "tally": tally}));
        sleep_unless_cancelled(pause(t.elapsed(), opts.duty), ctl);
    }

    let relocations: Vec<Relocation> = entries
        .iter()
        .zip(&at)
        .filter_map(|(e, a)| {
            Some(Relocation {
                segment: e.segment,
                offset: e.offset,
                entry_hash: e.entry_hash.clone(),
                kind: e.kind.clone(),
                facts: e.facts,
                leaf_index: (*a)?,
                // Not a leaf when the scan ran and a leaf now: a relog put
                // it there, even one whose journal line was lost.
                relogged: e.leaf_index.is_none(),
            })
        })
        .collect();
    let missing = entries.len() - relocations.len();
    let mut list = serde_json::to_vec(&relocations).map_err(|e| err(e.to_string()))?;
    list.push(b'\n');
    let list_path = dir.join(RELOCATIONS);
    let tmp = list_path.with_extension("json.tmp");
    std::fs::write(&tmp, &list)?;
    std::fs::rename(&tmp, &list_path)?;

    let out = json!({
        "schema": "emem.log_repair.relog.v1",
        "entries": entries.len(),
        "already_leaves_at_start": at_start.iter().filter(|a| a.is_some()).count(),
        "appended_this_run": appended,
        "relogged_in_all_runs": moved.len(),
        "not_leaves_after": missing,
        "complete": missing == 0,
        "proofs": tally,
        "relocations": {
            "path": list_path.display().to_string(),
            "entries": relocations.len(),
            "bytes": list.len(),
            "blake3_b32": b32(blake3::hash(&list).as_bytes()),
            "first_leaf_index": relocations.iter().map(|r| r.leaf_index).min(),
            "last_leaf_index": relocations.iter().map(|r| r.leaf_index).max(),
        },
        "tree_size_after": tree_after,
        "ms": t0.elapsed().as_millis() as u64,
    });
    write_json_atomic(&dir.join(RELOG_REPORT), &out)?;
    ctl.set(json!({"phase": "done"}));
    Ok(out)
}

#[derive(Deserialize)]
struct BatchHead {
    batch_root: [u8; 32],
    #[serde(default)]
    preimage_version: u8,
}

fn stored_proofs(
    store: &RepairStore<'_>,
    cids: &[&[u8]],
) -> Result<Vec<Option<MerkleProof>>, StorageError> {
    let decode = |b: &[u8]| ciborium::de::from_reader::<MerkleProof, _>(b).ok();
    let mut out: Vec<Option<MerkleProof>> = vec![None; cids.len()];
    if let Some(r) = store.redb {
        let rows = r
            .kv_get_many(KvTable::Proofs, cids)
            .map_err(StorageError::Cache)?;
        for (o, v) in out.iter_mut().zip(rows) {
            *o = v.as_deref().and_then(decode);
        }
        if r.table_backfill_done(KvTable::Proofs) {
            return Ok(out);
        }
    }
    if let Some(db) = store.sled {
        let tree = db
            .open_tree(crate::TREE_FACT_PROOFS)
            .map_err(|e| err(e.to_string()))?;
        for (o, c) in out.iter_mut().zip(cids) {
            if o.is_none() {
                *o = tree
                    .get(c)
                    .map_err(|e| err(e.to_string()))?
                    .as_deref()
                    .and_then(decode);
            }
        }
    }
    Ok(out)
}

fn facts_present(store: &RepairStore<'_>, cids: &[&[u8]]) -> Result<Vec<bool>, StorageError> {
    if let Some(r) = store.redb {
        return r.facts_present_many(cids).map_err(StorageError::Cache);
    }
    match store.sled {
        Some(db) => {
            let t = db.open_tree("emem.facts").map_err(|e| err(e.to_string()))?;
            cids.iter()
                .map(|c| t.contains_key(c).map_err(|e| err(e.to_string())))
                .collect()
        }
        None => Ok(vec![false; cids.len()]),
    }
}

fn put_proofs(store: &RepairStore<'_>, rows: &[(Vec<u8>, Vec<u8>)]) -> Result<(), StorageError> {
    if rows.is_empty() {
        return Ok(());
    }
    if let Some(r) = store.redb {
        return r
            .kv_put_batch(KvTable::Proofs, rows, true)
            .map_err(StorageError::Cache);
    }
    if let Some(db) = store.sled {
        let tree = db
            .open_tree(crate::TREE_FACT_PROOFS)
            .map_err(|e| err(e.to_string()))?;
        for (k, v) in rows {
            tree.insert(k, v.as_slice())
                .map_err(|e| err(e.to_string()))?;
        }
        db.flush().map_err(|e| err(e.to_string()))?;
    }
    Ok(())
}

/// Link each fact of one relogged attestation to its entry: a proof row of
/// this batch gets `log_entry_hash`; a fact with no row gets the proof
/// `put_attestation` would have stored; a row of another batch is left
/// alone. The rows to write are added to `rows`. Locators, where the node
/// keeps them, move to the new place.
fn link_proofs(
    store: &RepairStore<'_>,
    cbor: &[u8],
    entry_hash: &[u8; 32],
    new_place: Option<(u64, u64)>,
    tally: &mut LinkTally,
    rows: &mut Vec<(Vec<u8>, Vec<u8>)>,
) -> Result<(), StorageError> {
    let EntryFacts::Spans(spans) = entry_fact_spans(cbor) else {
        return Ok(());
    };
    if spans.is_empty() {
        return Ok(());
    }
    let head: BatchHead =
        ciborium::de::from_reader(cbor).map_err(|e| err(format!("entry head: {e}")))?;
    let digests: Vec<[u8; 32]> = spans
        .iter()
        .map(|&(a, b)| *blake3::hash(&cbor[a..b]).as_bytes())
        .collect();
    let cids: Vec<String> = digests.iter().map(|d| b32(d)).collect();
    let keys: Vec<&[u8]> = cids.iter().map(|c| c.as_bytes()).collect();
    let entry_b32 = b32(entry_hash);
    let n = digests.len() as u32;
    let (root, built) = crate::batch_proofs(&digests, head.preimage_version, Some(*entry_hash));
    let existing = stored_proofs(store, &keys)?;
    let present = facts_present(store, &keys)?;
    let mut put = |cid: &str, p: &MerkleProof| -> Result<(), StorageError> {
        let mut buf = Vec::new();
        ciborium::ser::into_writer(p, &mut buf).map_err(|e| err(e.to_string()))?;
        rows.push((cid.as_bytes().to_vec(), buf));
        Ok(())
    };
    for (orig, proof) in &built {
        tally.facts += 1;
        let cid = cids[*orig].as_str();
        match &existing[*orig] {
            Some(p) if p.root == head.batch_root => {
                if p.log_entry_hash.as_deref() == Some(entry_b32.as_str()) && p.leaf_count.is_some()
                {
                    tally.already_linked += 1;
                } else {
                    let mut p = p.clone();
                    p.log_entry_hash = Some(entry_b32.clone());
                    p.leaf_count = p.leaf_count.or(Some(n));
                    put(cid, &p)?;
                    tally.linked += 1;
                }
            }
            Some(_) => tally.other_batch += 1,
            None if root != head.batch_root => tally.root_differs += 1,
            None if !present[*orig] => tally.fact_absent += 1,
            None => {
                put(cid, proof)?;
                tally.created += 1;
            }
        }
    }

    if let (Some(r), Some((segment, offset))) = (store.redb, new_place) {
        if fact_locator::any_locators(r) {
            let digest_keys: Vec<&[u8]> = digests.iter().map(|d| d.as_slice()).collect();
            let located = r
                .kv_get_many(KvTable::Locators, &digest_keys)
                .map(|v| v.iter().filter(|x| x.is_some()).count() as u64)
                .unwrap_or(0);
            if located > 0 {
                let mut fspans = Vec::with_capacity(spans.len());
                for (&(a, b), d) in spans.iter().zip(&digests) {
                    fspans.push(fact_locator::FactSpan {
                        start: u32::try_from(a).map_err(|_| err("fact span past 4 GiB"))?,
                        len: u32::try_from(b - a).map_err(|_| err("fact span past 4 GiB"))?,
                        digest: *d,
                    });
                }
                let inp = fact_locator::LocatorInput {
                    segment,
                    offset,
                    entry_hash: *entry_hash,
                    spans: fspans,
                };
                fact_locator::write(r, &inp, head.preimage_version).map_err(StorageError::Cache)?;
                tally.locators_moved += located;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log_walk::tests::{frame, map_entry, torn_segment};

    fn opts() -> RepairOptions {
        RepairOptions {
            log_mb_s: 500,
            batch: 2,
            duty: 1.0,
        }
    }

    fn store(log: &AttestationLog) -> RepairStore<'_> {
        RepairStore {
            log,
            redb: None,
            sled: None,
        }
    }

    /// Segments 0 and 2 each hold a cut frame with records after it, and
    /// the newest segment ends in a write in flight. Returns every entry,
    /// in log order.
    fn torn_log(root: &Path) -> Vec<Vec<u8>> {
        let (s0, e0, _) = torn_segment(2, 10, 0);
        std::fs::write(root.join("merkle.log.0"), &s0).unwrap();
        let e1: Vec<Vec<u8>> = (0..3).map(|i| map_entry(50 + i, 300)).collect();
        let s1: Vec<u8> = e1.iter().flat_map(|e| frame(e)).collect();
        std::fs::write(root.join("merkle.log.1"), &s1).unwrap();
        let (s2, e2, _) = torn_segment(1, 1, 1000);
        std::fs::write(root.join("merkle.log.2"), &s2).unwrap();
        let e3 = map_entry(70, 100);
        let mut s3 = frame(&e3);
        s3.extend_from_slice(&frame(&map_entry(71, 100))[..40]);
        std::fs::write(root.join("merkle.log.3"), &s3).unwrap();
        e0.into_iter().chain(e1).chain(e2).chain([e3]).collect()
    }

    /// The scan finds what the tree missed; the relog appends exactly that,
    /// in log order, once; every head from before the relog is a prefix of
    /// the tree after it; and the relocation list names each new leaf.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_relog_appends_what_the_tree_missed_and_old_heads_stay_prefixes() {
        use emem_attest::translog;
        let tmp = tempfile::tempdir().unwrap();
        let all = torn_log(tmp.path());
        let run = tempfile::tempdir().unwrap();
        let rt = tokio::runtime::Handle::current();
        let root = tmp.path().to_path_buf();
        let dir = run.path().to_path_buf();
        let (old, rep, r1, r2, new) = tokio::task::spawn_blocking(move || {
            let log = AttestationLog::open(&root).unwrap();
            let old = log.leaf_hashes().unwrap();
            let rep = scan(&store(&log), &dir, &opts(), &JobControl::new()).unwrap();
            let r1 = relog(&store(&log), &dir, &opts(), &JobControl::new(), &rt).unwrap();
            let r2 = relog(&store(&log), &dir, &opts(), &JobControl::new(), &rt).unwrap();
            let new = log.leaf_hashes().unwrap();
            (old, rep, r1, r2, new)
        })
        .await
        .unwrap();
        let hidden: Vec<&Vec<u8>> = all
            .iter()
            .filter(|e| !old.contains(blake3::hash(e).as_bytes()))
            .collect();
        assert_eq!(
            hidden.len(),
            11,
            "ten after the misframe, one after the stop"
        );

        assert_eq!(rep["tears"].as_array().unwrap().len(), 2, "{rep:#}");
        assert_eq!(rep["in_flight_tail"].as_array().unwrap().len(), 1);
        assert_eq!(rep["off_tree"]["to_relog"], hidden.len());
        assert_eq!(rep["tree_leaves_walked"], old.len());
        let misframed = rep["misframed_leaves"].as_array().unwrap();
        assert_eq!(misframed.len(), 1, "{rep:#}");
        for m in misframed {
            let i = m["leaf_index"].as_u64().unwrap() as usize;
            assert_eq!(m["tree_leaf_b32"].as_str().unwrap(), b32(&old[i]));
        }

        assert_eq!(r1["appended_this_run"], hidden.len());
        assert_eq!(r1["complete"], true);
        assert_eq!(r2["appended_this_run"], 0, "a second run appends nothing");
        assert_eq!(new.len(), old.len() + hidden.len());
        assert_eq!(&new[..old.len()], &old[..], "the old tree is a prefix");
        for (k, e) in hidden.iter().enumerate() {
            assert_eq!(&new[old.len() + k], blake3::hash(e).as_bytes(), "log order");
        }
        for m in [1, old.len() / 2, old.len()] {
            let proof = translog::consistency_proof(m, &new).unwrap();
            assert!(translog::verify_consistency(
                m,
                &translog::merkle_tree_hash(&old[..m]),
                new.len(),
                &translog::merkle_tree_hash(&new),
                &proof
            ));
        }

        let list: Vec<Relocation> =
            serde_json::from_slice(&std::fs::read(run.path().join(RELOCATIONS)).unwrap()).unwrap();
        assert_eq!(list.len(), hidden.len());
        for r in &list {
            assert_eq!(b32(&new[r.leaf_index as usize]), r.entry_hash);
            assert!(r.relogged);
        }
    }

    /// A write retried with the same bytes left two copies after the tear.
    /// Both places map to one new leaf, appended once.
    #[tokio::test(flavor = "multi_thread")]
    async fn entries_with_the_same_bytes_are_appended_once() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut seg, entries, _) = torn_segment(1, 1, 0);
        let again = frame(&entries[1]);
        seg.extend_from_slice(&again);
        std::fs::write(tmp.path().join("merkle.log.0"), &seg).unwrap();
        std::fs::write(tmp.path().join("merkle.log.1"), frame(&map_entry(9, 50))).unwrap();
        let run = tempfile::tempdir().unwrap();
        let rt = tokio::runtime::Handle::current();
        let root = tmp.path().to_path_buf();
        let dir = run.path().to_path_buf();
        let (old, r, new) = tokio::task::spawn_blocking(move || {
            let log = AttestationLog::open(&root).unwrap();
            let old = log.leaf_hashes().unwrap();
            scan(&store(&log), &dir, &opts(), &JobControl::new()).unwrap();
            let r = relog(&store(&log), &dir, &opts(), &JobControl::new(), &rt).unwrap();
            (old, r, log.leaf_hashes().unwrap())
        })
        .await
        .unwrap();
        assert_eq!(r["entries"], 2);
        assert_eq!(r["appended_this_run"], 1);
        assert_eq!(r["complete"], true);
        assert_eq!(new.len(), old.len() + 1);
        let list: Vec<Relocation> =
            serde_json::from_slice(&std::fs::read(run.path().join(RELOCATIONS)).unwrap()).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].leaf_index, list[1].leaf_index);
        assert_eq!(list[0].leaf_index as usize, old.len());
    }

    /// A segment whose bytes changed after the scan stops the relog before
    /// anything is appended for it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_relog_refuses_bytes_that_changed_since_the_scan() {
        let tmp = tempfile::tempdir().unwrap();
        torn_log(tmp.path());
        let run = tempfile::tempdir().unwrap();
        let rt = tokio::runtime::Handle::current();
        let root = tmp.path().to_path_buf();
        let dir = run.path().to_path_buf();
        let (before, after, e) = tokio::task::spawn_blocking(move || {
            let log = AttestationLog::open(&root).unwrap();
            scan(&store(&log), &dir, &opts(), &JobControl::new()).unwrap();
            let entries = read_entries(&dir).unwrap();
            let seg = root.join(format!("merkle.log.{}", entries[0].segment));
            let mut b = std::fs::read(&seg).unwrap();
            b[entries[0].offset as usize + 10] ^= 1;
            std::fs::write(&seg, &b).unwrap();
            let before = log.leaf_hashes().unwrap().len();
            let e = relog(&store(&log), &dir, &opts(), &JobControl::new(), &rt).unwrap_err();
            (before, log.leaf_hashes().unwrap().len(), e)
        })
        .await
        .unwrap();
        assert!(e.to_string().contains("no longer holds"), "{e}");
        assert_eq!(before, after);
    }

    /// Facts of a relogged attestation get proofs that name its entry: a
    /// row of the batch is linked, a fact with no row gets one, and a second
    /// pass changes nothing.
    #[tokio::test(flavor = "multi_thread")]
    async fn relogged_facts_get_proofs_that_name_their_entry() {
        use crate::orphans::tests::{att, fact};
        use crate::{FactPlanePolicy, MaterializingStorage, Storage};
        let bands = std::sync::Arc::new(emem_core::bands::DEFAULT.clone());
        let functions =
            std::sync::Arc::new(emem_core::FunctionRegistry::parse_default().expect("functions"));
        let sources =
            std::sync::Arc::new(emem_core::SourceRegistry::parse_default().expect("sources"));
        let s = MaterializingStorage::ephemeral(bands, functions, sources).expect("storage");
        s.set_fact_plane_policy(FactPlanePolicy {
            open: true,
            ..Default::default()
        });
        let a = att(vec![
            fact(1, 0.1, "2026-05-30T03:00:00Z"),
            fact(2, 0.2, "2026-05-30T03:00:00Z"),
        ]);
        let cids = s.cache.put_many(&a.facts).await.unwrap();
        let mut cbor = Vec::new();
        ciborium::ser::into_writer(&a, &mut cbor).unwrap();
        let entry = *blake3::hash(&cbor).as_bytes();
        let digests: Vec<[u8; 32]> = cids.iter().map(|c| unb32(c.as_str()).unwrap()).collect();
        // The first fact has a row from before the log link existed, which
        // names no entry; the second has none.
        let (root, proofs) = crate::batch_proofs(&digests, a.preimage_version, None);
        assert_eq!(root, a.batch_root);
        let first = proofs.iter().find(|(o, _)| *o == 0).unwrap().1.clone();
        let mut b = Vec::new();
        ciborium::ser::into_writer(&first, &mut b).unwrap();
        let redb = s.redb().unwrap();
        redb.kv_put_batch(
            KvTable::Proofs,
            &[(cids[0].as_str().as_bytes().to_vec(), b)],
            true,
        )
        .unwrap();
        redb.mark_table_backfill_done(KvTable::Proofs).unwrap();
        assert!(s.proof_for_cid(&cids[1]).is_none());

        let rs = RepairStore {
            log: &s.log,
            redb: Some(&*redb),
            sled: s.hot_sled_db(),
        };
        let mut tally = LinkTally::default();
        let mut rows = Vec::new();
        link_proofs(&rs, &cbor, &entry, None, &mut tally, &mut rows).unwrap();
        put_proofs(&rs, &rows).unwrap();
        assert_eq!((tally.linked, tally.created), (1, 1), "{tally:?}");
        for c in &cids {
            let p = s.proof_for_cid(c).unwrap();
            assert_eq!(p.log_entry_hash.as_deref(), Some(b32(&entry).as_str()));
            assert_eq!(p.root, a.batch_root);
            assert_eq!(p.leaf_count, Some(2));
        }
        let mut again = LinkTally::default();
        let mut none = Vec::new();
        link_proofs(&rs, &cbor, &entry, None, &mut again, &mut none).unwrap();
        assert!(none.is_empty(), "nothing left to write");
        assert_eq!(again.already_linked, 2, "{again:?}");
    }
}
