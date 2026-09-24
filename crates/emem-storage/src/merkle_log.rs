//! Append-only Merkle attestation log.
//!
//! On-disk wire format:
//!
//! ```text
//! segment files: merkle.log.<u64-segment-index>
//! per record:    [u32 LE: cbor_len][cbor_bytes][32 bytes: blake3(cbor_bytes)]
//! per segment:   trailing 32-byte segment hash = blake3(all_records)
//! ```
//!
//! Segments rotate at 1 GiB. Replay-restore = "for each segment, re-hash
//! and verify trailing hash." Snapshots ship the segment file + the
//! per-segment hash to S3/IPFS every N segments.

use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::PathBuf;

use async_trait::async_trait;
use blake3::Hasher;
use tokio::sync::Mutex;

use emem_fact::Attestation;

/// Default segment size (1 GiB).
pub const SEGMENT_BYTES: u64 = 1 << 30;

/// Append-only attestation log.
pub struct AttestationLog {
    /// Root directory for segment files.
    pub root: PathBuf,
    state: std::sync::Arc<Mutex<LogState>>,
    /// Framed records waiting for the next group write, in arrival order.
    queue: std::sync::Mutex<Vec<Pending>>,
}

/// One framed record waiting to be written, and who to tell.
struct Pending {
    record: Vec<u8>,
    record_hash: [u8; 32],
    done: tokio::sync::oneshot::Sender<std::io::Result<AppendOutcome>>,
}

struct LogState {
    segment_index: u64,
    bytes_in_segment: u64,
    segment_hasher: Hasher,
    /// The segment index this process started at. Every segment strictly
    /// below it was written by an earlier process and can never be appended
    /// to again (open always starts a fresh segment), so those files are
    /// frozen and are exactly the ones `prior` counts.
    first_own_segment: u64,
    /// Records appended by THIS process.
    appended: u64,
    /// Records that were already on disk when this log was opened. `None`
    /// until somebody asks for it: the count is a length-driven walk over
    /// every byte of every segment, and open() is on the boot path.
    prior: Option<u64>,
}

impl AttestationLog {
    /// Open or create a log at the given root directory. Resumes from
    /// the last existing segment so appends after restart preserve
    /// the cumulative segment hash.
    pub fn open(root: impl Into<PathBuf>) -> std::io::Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root)?;
        let state = scan_existing(&root)?;
        Ok(Self {
            root,
            state: std::sync::Arc::new(Mutex::new(state)),
            queue: std::sync::Mutex::new(Vec::new()),
        })
    }

    /// Records already on disk when this log was opened, counted once and
    /// then remembered. Held apart from the append counter because it is the
    /// expensive half: a length-driven walk over every byte of every sealed
    /// segment. An IO error is not memoised, so a later call retries.
    /// Runs on the blocking pool for the same reason the append fsync does:
    /// on this responder it is a multi-second read of several GB, and an
    /// async worker parked on it stalls every other request that worker
    /// owned.
    async fn prior_records(&self, s: &mut LogState) -> u64 {
        if s.prior.is_none() {
            let root = self.root.clone();
            let first_own = s.first_own_segment;
            if let Ok(Ok(n)) =
                tokio::task::spawn_blocking(move || count_records_below(&root, first_own)).await
            {
                s.prior = Some(n);
            }
        }
        s.prior.unwrap_or(0)
    }

    /// Append an attestation. Bytes are flushed and fsynced before this
    /// returns — receipts depend on the cryptographic durability claim.
    pub async fn append(&self, att: &Attestation) -> Result<AppendOutcome, std::io::Error> {
        let mut buf = Vec::new();
        ciborium::ser::into_writer(att, &mut buf)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        self.append_cbor(buf).await
    }

    /// Append one record's CBOR bytes as they are. The framing, the fsync and
    /// the leaf (`blake3(cbor)`) are the same as for an attestation; this is
    /// how an entry that is not an attestation, such as a memory write, gets
    /// into the same tree under the same signed head.
    pub async fn append_cbor(&self, buf: Vec<u8>) -> Result<AppendOutcome, std::io::Error> {
        let len = u32::try_from(buf.len()).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "attestation > 4 GiB")
        })?;
        let mut record = Vec::with_capacity(4 + buf.len() + 32);
        record.extend_from_slice(&len.to_le_bytes());
        record.extend_from_slice(&buf);
        let mut record_hasher = Hasher::new();
        record_hasher.update(&buf);
        let record_hash = record_hasher.finalize();
        record.extend_from_slice(record_hash.as_bytes());

        let mut record_hash_arr = [0u8; 32];
        record_hash_arr.copy_from_slice(record_hash.as_bytes());

        // Group commit. Each append used to hold the lock for its own open,
        // write and fsync, so N writers paid N fsyncs in a row; under disk
        // load one fsync took up to a second and a cold write waited 11 s
        // behind the queue (measured 2026-09-24). Now a record joins the
        // queue, and whoever holds the lock next writes everything queued in
        // one write and one fsync per segment. Order, framing, the segment
        // hash chain and "durable before return" are unchanged.
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(Pending {
                record,
                record_hash: record_hash_arr,
                done: tx,
            });
        // The batch is written by a detached task holding the lock, so a
        // caller dropped by a timeout cannot stop a write halfway: the bytes
        // and the in-memory hash chain always advance together.
        let mut s = self.state.clone().lock_owned().await;
        let batch = std::mem::take(&mut *self.queue.lock().unwrap_or_else(|e| e.into_inner()));
        if !batch.is_empty() {
            let root = self.root.clone();
            tokio::spawn(async move { write_batch(&root, &mut s, batch).await });
        } else {
            drop(s);
        }
        rx.await
            .map_err(|_| std::io::Error::other("merkle log append: the group write was dropped"))?
    }
}

/// Write `batch` in order, one write and one fsync per segment it touches,
/// then tell each writer where its record landed.
async fn write_batch(root: &std::path::Path, s: &mut LogState, batch: Vec<Pending>) {
    let mut group: Vec<Pending> = Vec::new();
    let mut group_bytes = 0u64;
    for p in batch {
        let len = p.record.len() as u64;
        let used = s.bytes_in_segment + group_bytes;
        if used > 0 && used + len > SEGMENT_BYTES {
            write_group(root, s, std::mem::take(&mut group)).await;
            group_bytes = 0;
            if let Err(e) = seal_segment(root, s) {
                let _ = p.done.send(Err(e));
                continue;
            }
        }
        group_bytes += len;
        group.push(p);
    }
    write_group(root, s, group).await;
}

async fn write_group(root: &std::path::Path, s: &mut LogState, group: Vec<Pending>) {
    if group.is_empty() {
        return;
    }
    let path = root.join(format!("merkle.log.{}", s.segment_index));
    let mut bytes = Vec::with_capacity(group.iter().map(|p| p.record.len()).sum());
    for p in &group {
        bytes.extend_from_slice(&p.record);
    }
    // The open + write + fsync stay on the blocking pool so an fsync never
    // parks an async worker.
    let written = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        let mut f = OpenOptions::new().create(true).append(true).open(&path)?;
        f.write_all(&bytes)?;
        f.sync_all()
    })
    .await
    .map_err(|e| std::io::Error::other(format!("merkle log append task panicked: {e}")))
    .and_then(|r| r);
    match written {
        Ok(()) => {
            for p in group {
                s.segment_hasher.update(&p.record);
                let offset = s.bytes_in_segment;
                s.bytes_in_segment += p.record.len() as u64;
                s.appended += 1;
                let _ = p.done.send(Ok(AppendOutcome {
                    segment_index: s.segment_index,
                    offset_in_segment: offset,
                    record_hash: p.record_hash,
                }));
            }
        }
        Err(e) => {
            for p in group {
                let _ = p
                    .done
                    .send(Err(std::io::Error::new(e.kind(), e.to_string())));
            }
        }
    }
}

impl AttestationLog {
    /// Cumulative number of attestation records appended in this log's
    /// lifetime (including across restarts of the process).
    pub async fn record_count(&self) -> u64 {
        let mut s = self.state.lock().await;
        let prior = self.prior_records(&mut s).await;
        prior + s.appended
    }

    /// Collect every record's per-record hash (the trailing
    /// `blake3(attestation_cbor)` on disk) in global append order:
    /// segments in ascending index order, records in file order within
    /// each. These are the leaves of the RFC 6962 transparency tree
    /// ([`emem_attest::translog`]); the order is stable and append-only
    /// (new records extend the current segment; new segments take a higher
    /// index), which is what makes consistency proofs meaningful.
    ///
    /// `O(total_bytes)`. Prefer [`leaf_hashes_from`] on any path that runs
    /// more than once: this responder's log is 6.6 GB across 1,390 segments,
    /// and all but the highest-indexed one are sealed and immutable, so
    /// re-reading them to learn about one appended record is the expensive
    /// way to ask a cheap question.
    pub fn leaf_hashes(&self) -> std::io::Result<Vec<[u8; 32]>> {
        let mut leaves = Vec::new();
        for (_, seg) in self.leaf_hashes_from(0)? {
            leaves.extend(seg);
        }
        Ok(leaves)
    }

    /// Leaf hashes of every segment with index >= `from`, as
    /// `(segment_index, leaves)` in ascending index order.
    ///
    /// Exists so a caller holding leaves from a previous read can refresh
    /// without re-reading the whole log. Only the HIGHEST-indexed segment can
    /// still grow: a segment is sealed when it passes `SEGMENT_BYTES` or when
    /// the process that owned it exits, and a sealed segment's bytes never
    /// change again. So a caller that remembers the highest index it saw can
    /// pass it here and re-read one segment instead of 1,390.
    ///
    /// Returns per-segment rather than flattened precisely so the caller can
    /// tell where the immutable prefix ends; flattening throws that away.
    pub fn leaf_hashes_from(&self, from: u64) -> std::io::Result<Vec<(u64, Vec<[u8; 32]>)>> {
        let mut indices: Vec<u64> = Vec::new();
        for entry in std::fs::read_dir(&self.root)? {
            let entry = entry?;
            if let Some(rest) = entry
                .file_name()
                .to_str()
                .and_then(|n| n.strip_prefix("merkle.log.").map(|s| s.to_string()))
            {
                if let Ok(n) = rest.parse::<u64>() {
                    if n >= from {
                        indices.push(n);
                    }
                }
            }
        }
        indices.sort_unstable();
        let mut out: Vec<(u64, Vec<[u8; 32]>)> = Vec::with_capacity(indices.len());
        for idx in indices {
            let path = self.root.join(format!("merkle.log.{idx}"));
            let mut bytes = Vec::new();
            std::fs::File::open(&path)?.read_to_end(&mut bytes)?;
            // Each record is [u32 LE len][len bytes cbor][32 bytes hash].
            // A sealed segment has a trailing 32-byte segment hash after
            // the last record; the length-driven walk below stops before
            // it (the leftover < a full record is ignored).
            let mut leaves: Vec<[u8; 32]> = Vec::new();
            let mut i = 0usize;
            while i + 4 <= bytes.len() {
                let len = u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]])
                    as usize;
                let needed = 4 + len + 32;
                if i + needed > bytes.len() {
                    break;
                }
                let mut leaf = [0u8; 32];
                leaf.copy_from_slice(&bytes[i + 4 + len..i + needed]);
                leaves.push(leaf);
                i += needed;
            }
            out.push((idx, leaves));
        }
        Ok(out)
    }

    /// Return the raw attestation CBOR for the half-open global index range
    /// `[start, end)`, in append order, as `(global_index, cbor_bytes)`.
    ///
    /// This is RFC 6962 §4.6 `get-entries`, and without it the log is only
    /// half a transparency log. `/v1/log/inclusion` lets a party prove a cid
    /// they ALREADY HOLD is in the tree; only enumeration lets them audit what
    /// else is in it. A log nobody can read is a log nobody can catch.
    ///
    /// Indices are global and stable: segments ascend, records follow file
    /// order within a segment, and the log is append-only, which is the same
    /// ordering [`leaf_hashes`] builds the tree from. So entry `i` here is the
    /// preimage of leaf `i` there, and a caller can check
    /// `blake3(cbor) == leaf_hashes()[i]` themselves.
    ///
    /// The caller bounds the range; this refuses nothing and truncates nothing
    /// silently. It returns what exists in `[start, end)`, so a short result
    /// means the log ended, exactly as RFC 6962 permits.
    ///
    /// `O(bytes in the touched segments)`: it skips whole segments before
    /// `start`, but does not index within one, so a range near the end of a
    /// 1 GiB segment still walks that segment.
    pub fn entries(&self, start: u64, end: u64) -> std::io::Result<Vec<(u64, Vec<u8>)>> {
        if end <= start {
            return Ok(Vec::new());
        }
        let mut indices: Vec<u64> = Vec::new();
        for entry in std::fs::read_dir(&self.root)? {
            let entry = entry?;
            if let Some(rest) = entry
                .file_name()
                .to_str()
                .and_then(|n| n.strip_prefix("merkle.log.").map(|s| s.to_string()))
            {
                if let Ok(n) = rest.parse::<u64>() {
                    indices.push(n);
                }
            }
        }
        indices.sort_unstable();

        let mut out: Vec<(u64, Vec<u8>)> = Vec::new();
        let mut global = 0u64;
        for idx in indices {
            if global >= end {
                break;
            }
            let path = self.root.join(format!("merkle.log.{idx}"));
            // Skip whole segments that end before `start` by their record
            // count, without reading them: a leaf near the head used to read
            // every earlier segment (4.7 GB, ~5.5 s), and witnesses sampling
            // leaves for a custody audit timed out on it.
            let n = segment_record_count(&path)?;
            if global + n <= start {
                global += n;
                continue;
            }
            let mut bytes = Vec::new();
            std::fs::File::open(&path)?.read_to_end(&mut bytes)?;
            // Same length-driven walk as `leaf_hashes`: [u32 LE len][cbor][32 hash].
            // A sealed segment's trailing 32-byte hash is left over and ignored.
            let mut i = 0usize;
            while i + 4 <= bytes.len() {
                let len = u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]])
                    as usize;
                let needed = 4 + len + 32;
                if i + needed > bytes.len() {
                    break;
                }
                if global >= start && global < end {
                    out.push((global, bytes[i + 4..i + 4 + len].to_vec()));
                }
                global += 1;
                i += needed;
                if global >= end {
                    break;
                }
            }
        }
        Ok(out)
    }

    /// Verify the on-disk integrity of every sealed segment. Open
    /// (current) segment is not verified because it has no trailing
    /// hash yet.
    pub fn verify(&self) -> std::io::Result<VerifyReport> {
        let mut sealed = 0u64;
        let mut bad: Vec<(u64, String)> = Vec::new();
        for entry in std::fs::read_dir(&self.root)? {
            let entry = entry?;
            let name = match entry.file_name().to_str() {
                Some(s) => s.to_string(),
                None => continue,
            };
            let n = match name
                .strip_prefix("merkle.log.")
                .and_then(|s| s.parse::<u64>().ok())
            {
                Some(n) => n,
                None => continue,
            };
            let mut bytes = Vec::new();
            std::fs::File::open(entry.path())?.read_to_end(&mut bytes)?;
            if bytes.len() < 32 {
                continue;
            }
            let (body, trailer) = bytes.split_at(bytes.len() - 32);
            let mut h = Hasher::new();
            h.update(body);
            if h.finalize().as_bytes() == trailer {
                sealed += 1;
            } else {
                bad.push((n, "trailing hash mismatch".into()));
            }
        }
        Ok(VerifyReport {
            sealed_ok: sealed,
            bad,
        })
    }
}

/// Result of a successful append: where the record landed and its
/// per-record hash. Callers use this to construct downstream Merkle
/// inclusion proofs.
#[derive(Debug, Clone)]
pub struct AppendOutcome {
    /// Segment index the record was appended to.
    pub segment_index: u64,
    /// Byte offset of the record within the segment.
    pub offset_in_segment: u64,
    /// blake3(attestation_cbor) — the per-record hash on disk.
    pub record_hash: [u8; 32],
}

/// Output of [`AttestationLog::verify`].
#[derive(Debug, Clone)]
pub struct VerifyReport {
    /// Sealed segments whose trailing hash matched.
    pub sealed_ok: u64,
    /// Sealed segments that failed verification, with reason.
    pub bad: Vec<(u64, String)>,
}

fn seal_segment(root: &std::path::Path, s: &mut LogState) -> std::io::Result<()> {
    let segment_hash_bytes = s.segment_hasher.finalize();
    let path = root.join(format!("merkle.log.{}", s.segment_index));
    let mut f = OpenOptions::new().append(true).open(&path)?;
    f.write_all(segment_hash_bytes.as_bytes())?;
    f.sync_all()?;
    s.segment_index += 1;
    s.bytes_in_segment = 0;
    s.segment_hasher = Hasher::new();
    Ok(())
}

/// Decide which segment this process will write to, by name only.
///
/// This used to also total the records in every segment, which meant reading
/// every byte of the log to produce a number. On this responder that was
/// 1,046 segments and 4.7 GB, because a fresh segment is opened per process
/// start and the log therefore accumulates one file per restart: the cost
/// grew with the number of deploys, not with the amount of data. Nothing on
/// the boot path consumes the total (`leaf_hashes`, `entries` and `verify`
/// each re-walk the disk themselves), so it moved to [`record_count`], which
/// pays for it on demand and remembers the answer.
fn scan_existing(root: &std::path::Path) -> std::io::Result<LogState> {
    let mut max: Option<u64> = None;
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let name = match entry.file_name().to_str() {
            Some(s) => s.to_string(),
            None => continue,
        };
        if let Some(rest) = name.strip_prefix("merkle.log.") {
            if let Ok(n) = rest.parse::<u64>() {
                max = Some(max.map(|m| m.max(n)).unwrap_or(n));
            }
        }
    }
    let segment_index = max.map(|m| m + 1).unwrap_or(0);
    // We always start a new segment on open, so the previous one is
    // implicitly considered sealed (or in-progress without a trailer
    // — verifying that on each open is a future enhancement).
    Ok(LogState {
        segment_index,
        bytes_in_segment: 0,
        segment_hasher: Hasher::new(),
        first_own_segment: segment_index,
        appended: 0,
        prior: None,
    })
}

/// Total the records in every segment with an index below `first_own`, i.e.
/// everything that existed before this process opened the log. Bounding it
/// that way is what keeps it composable with the append counter: this
/// process's own segments are counted by `appended`, never here, so no
/// record is counted twice however many segments we seal while running.
/// Records in one segment file, remembered by (path, length). A sealed
/// segment never changes; the open one is recounted when it has grown. The
/// walk reads only each record's 4-byte length and seeks past the body.
fn segment_record_count(path: &std::path::Path) -> std::io::Result<u64> {
    use std::io::{Seek, SeekFrom};
    type Counts = std::sync::Mutex<std::collections::HashMap<PathBuf, (u64, u64)>>;
    static COUNTS: std::sync::OnceLock<Counts> = std::sync::OnceLock::new();
    let len = std::fs::metadata(path)?.len();
    let memo = COUNTS.get_or_init(Default::default);
    if let Some((l, n)) = memo.lock().unwrap_or_else(|e| e.into_inner()).get(path) {
        if *l == len {
            return Ok(*n);
        }
    }
    let mut f = std::io::BufReader::new(std::fs::File::open(path)?);
    let (mut pos, mut n) = (0u64, 0u64);
    let mut hdr = [0u8; 4];
    while pos + 4 <= len {
        f.seek(SeekFrom::Start(pos))?;
        f.read_exact(&mut hdr)?;
        let needed = 4 + u32::from_le_bytes(hdr) as u64 + 32;
        if pos + needed > len {
            break;
        }
        n += 1;
        pos += needed;
    }
    memo.lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(path.to_path_buf(), (len, n));
    Ok(n)
}

fn count_records_below(root: &std::path::Path, first_own: u64) -> std::io::Result<u64> {
    let mut total = 0u64;
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let name = match entry.file_name().to_str() {
            Some(s) => s.to_string(),
            None => continue,
        };
        if let Some(rest) = name.strip_prefix("merkle.log.") {
            if let Ok(n) = rest.parse::<u64>() {
                if n < first_own {
                    total += count_records_in(&entry.path())?;
                }
            }
        }
    }
    Ok(total)
}

fn count_records_in(path: &std::path::Path) -> std::io::Result<u64> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?.read_to_end(&mut bytes)?;
    let mut count = 0u64;
    let mut i = 0usize;
    while i + 4 <= bytes.len() {
        let len = u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]) as usize;
        let needed = 4 + len + 32;
        if i + needed > bytes.len() {
            break;
        }
        i += needed;
        count += 1;
    }
    Ok(count)
}

/// Segment manifest for snapshot/replication. Published to the coverage
/// manifest CID so any replica can replay-restore from upstream snapshots.
#[derive(Debug, Clone)]
pub struct SegmentManifest {
    /// Segment index.
    pub index: u64,
    /// Trailing 32-byte segment hash.
    pub hash: [u8; 32],
    /// Byte length of the segment file.
    pub bytes: u64,
}

/// A trait alias for backup/replication backends (S3, IPFS, etc.).
#[async_trait]
pub trait SegmentBackup: Send + Sync {
    /// Push a sealed segment file + its manifest to remote storage.
    async fn push_segment(
        &self,
        path: &std::path::Path,
        manifest: &SegmentManifest,
    ) -> std::io::Result<()>;

    /// Pull a segment by index for replay-restore.
    async fn pull_segment(
        &self,
        index: u64,
        dst: &std::path::Path,
    ) -> std::io::Result<SegmentManifest>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use emem_core::{AttesterKey, KeyEpoch, Signature};
    use emem_fact::{RegistryCid, SchemaCid};

    fn sample_attestation() -> Attestation {
        Attestation {
            facts: vec![],
            edges: vec![],
            batch_root: [9u8; 32],
            attester: AttesterKey([1u8; 32]),
            attester_key_epoch: KeyEpoch(0),
            registry_cid: RegistryCid::new("r"),
            schema_cid: SchemaCid::new("s"),
            signature: Signature([0u8; 64]),
            attested_at: "2026-01-01T00:00:00Z".into(),
            scope: None,
            preimage_version: 0,
        }
    }

    fn distinct_attestation(i: u64) -> Attestation {
        let mut a = sample_attestation();
        a.batch_root = [i as u8; 32];
        a.attested_at = format!("2026-01-01T00:00:{i:02}Z");
        a
    }

    /// The property that makes `entries` an audit rather than a data dump:
    /// entry `i` must be the PREIMAGE of leaf `i`. If the two orderings ever
    /// diverge, a caller who re-hashes an entry and compares it to the tree
    /// gets a false mismatch, and every inclusion proof they build is noise.
    /// Nothing else in this file couples the two walks, so pin it.
    /// A raw entry of another kind shares the framing, the order and the leaf
    /// rule with attestations, so one tree covers both.
    #[tokio::test]
    async fn a_raw_entry_is_a_leaf_like_any_attestation() {
        let tmp = tempfile::tempdir().unwrap();
        let log = AttestationLog::open(tmp.path()).unwrap();
        log.append(&distinct_attestation(1)).await.unwrap();
        let raw = b"\xa1dkindtemem.memory_write.v1".to_vec();
        let out = log.append_cbor(raw.clone()).await.unwrap();
        log.append(&distinct_attestation(2)).await.unwrap();
        assert_eq!(&out.record_hash, blake3::hash(&raw).as_bytes());
        let leaves = log.leaf_hashes().unwrap();
        let all = log.entries(0, u64::MAX).unwrap();
        assert_eq!(leaves.len(), 3);
        assert_eq!(all[1].1, raw, "the bytes come back as written");
        assert_eq!(leaves[1], out.record_hash);
        assert_eq!(log.record_count().await, 3);
    }

    #[tokio::test]
    async fn entries_are_the_preimages_of_leaf_hashes_in_the_same_order() {
        let tmp = tempfile::tempdir().unwrap();
        let log = AttestationLog::open(tmp.path()).unwrap();
        for i in 0..7u64 {
            log.append(&distinct_attestation(i)).await.unwrap();
        }
        let leaves = log.leaf_hashes().unwrap();
        let all = log.entries(0, u64::MAX).unwrap();
        assert_eq!(all.len(), 7, "entries must return every record");
        assert_eq!(leaves.len(), 7);
        for (i, (idx, cbor)) in all.iter().enumerate() {
            assert_eq!(*idx as usize, i, "global index must be dense and ordered");
            assert_eq!(
                blake3::hash(cbor).as_bytes(),
                &leaves[i],
                "entry {i} must hash to leaf {i}: a third party re-hashing this entry \
                 must land on the same leaf the tree was built from"
            );
        }
    }

    /// Ranges are half-open and a short result means the log ended, which is
    /// what RFC 6962 permits. A range past the end must be empty, not an error,
    /// and must never wrap or panic.
    /// Entries deep in a many-segment log come from skipping whole segments
    /// by count, and match what a full walk returns, including after the
    /// open segment grows between calls.
    #[tokio::test]
    async fn entries_skip_segments_and_stay_exact() {
        let tmp = tempfile::tempdir().unwrap();
        let mut n = 0u64;
        for _ in 0..4 {
            let log = AttestationLog::open(tmp.path()).unwrap();
            for _ in 0..3 {
                log.append(&distinct_attestation(n)).await.unwrap();
                n += 1;
            }
        }
        let log = AttestationLog::open(tmp.path()).unwrap();
        let all = log.entries(0, n).unwrap();
        assert_eq!(all.len() as u64, n);
        for start in [0, 4, 7, 11] {
            let got = log.entries(start, start + 1).unwrap();
            assert_eq!(got, vec![all[start as usize].clone()], "start {start}");
        }
        log.append(&distinct_attestation(n)).await.unwrap();
        let tail = log.entries(n, n + 1).unwrap();
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0].0, n);
    }

    #[tokio::test]
    async fn entries_range_is_half_open_and_clamps_past_the_end() {
        let tmp = tempfile::tempdir().unwrap();
        let log = AttestationLog::open(tmp.path()).unwrap();
        for i in 0..5u64 {
            log.append(&distinct_attestation(i)).await.unwrap();
        }
        let ids = |v: Vec<(u64, Vec<u8>)>| v.into_iter().map(|(i, _)| i).collect::<Vec<_>>();
        assert_eq!(ids(log.entries(0, 5).unwrap()), vec![0, 1, 2, 3, 4]);
        assert_eq!(
            ids(log.entries(1, 3).unwrap()),
            vec![1, 2],
            "end is exclusive"
        );
        assert_eq!(
            ids(log.entries(3, 99).unwrap()),
            vec![3, 4],
            "clamps at the end"
        );
        assert!(
            log.entries(5, 9).unwrap().is_empty(),
            "past the end is empty"
        );
        assert!(log.entries(2, 2).unwrap().is_empty(), "empty range");
        assert!(
            log.entries(4, 1).unwrap().is_empty(),
            "inverted range must not wrap"
        );
    }

    /// Concurrent appends share writes but not places: every record lands
    /// once, at its own offset, the records tile the segment with no gap,
    /// and the leaves are exactly the records written.
    #[tokio::test]
    async fn concurrent_appends_group_into_one_intact_segment() {
        let tmp = tempfile::tempdir().unwrap();
        let log = std::sync::Arc::new(AttestationLog::open(tmp.path()).unwrap());
        let mut tasks = Vec::new();
        for i in 0..64u64 {
            let log = log.clone();
            tasks.push(tokio::spawn(async move {
                log.append(&distinct_attestation(i)).await.unwrap()
            }));
        }
        let mut outs = Vec::new();
        for t in tasks {
            outs.push(t.await.unwrap());
        }
        assert_eq!(log.record_count().await, 64);
        let mut offsets: Vec<u64> = outs.iter().map(|o| o.offset_in_segment).collect();
        offsets.sort_unstable();
        offsets.dedup();
        assert_eq!(offsets.len(), 64);
        let seg = outs[0].segment_index;
        assert!(outs.iter().all(|o| o.segment_index == seg));
        let mut leaves = log.leaf_hashes().unwrap();
        let mut want: Vec<[u8; 32]> = outs.iter().map(|o| o.record_hash).collect();
        leaves.sort_unstable();
        want.sort_unstable();
        assert_eq!(leaves, want);
    }

    /// Appends abandoned by their callers still land whole: what the log
    /// counts is what the disk holds, and the next append lands after them.
    #[tokio::test]
    async fn a_dropped_append_still_advances_the_chain_with_the_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let log = std::sync::Arc::new(AttestationLog::open(tmp.path()).unwrap());
        for i in 0..32u64 {
            let l = log.clone();
            let _ = tokio::time::timeout(std::time::Duration::from_micros(1), async move {
                l.append(&distinct_attestation(i)).await
            })
            .await;
        }
        let last = log.append(&distinct_attestation(99)).await.unwrap();
        let on_disk = log.leaf_hashes().unwrap();
        assert_eq!(on_disk.last(), Some(&last.record_hash));
        assert_eq!(log.record_count().await, on_disk.len() as u64);
    }

    #[tokio::test]
    async fn append_then_count() {
        let tmp = tempfile::tempdir().unwrap();
        let log = AttestationLog::open(tmp.path()).unwrap();
        let _ = log.append(&sample_attestation()).await.unwrap();
        let _ = log.append(&sample_attestation()).await.unwrap();
        // append() opens a fresh segment per process start, so previous
        // process's records do not appear in this run's `record_count`,
        // but the existing-on-disk total is reflected through the scan.
        assert_eq!(log.record_count().await, 2);
    }

    /// `record_count` is now two halves: what was on disk at open (counted
    /// lazily, so it is off the boot path) plus what this process appended.
    /// The way that split can go wrong is double counting — the lazy count
    /// running after we have already written our own segment and totalling
    /// it too. Reopen, append, and check the sum, in that order.
    #[tokio::test]
    async fn reopening_counts_prior_records_without_double_counting_our_own() {
        let tmp = tempfile::tempdir().unwrap();
        {
            let first = AttestationLog::open(tmp.path()).unwrap();
            for _ in 0..3 {
                let _ = first.append(&sample_attestation()).await.unwrap();
            }
            assert_eq!(first.record_count().await, 3);
        }
        let second = AttestationLog::open(tmp.path()).unwrap();
        // Asked before we write anything: prior only.
        assert_eq!(second.record_count().await, 3, "prior records on reopen");
        for _ in 0..2 {
            let _ = second.append(&sample_attestation()).await.unwrap();
        }
        assert_eq!(second.record_count().await, 5, "prior + our own");

        // And the same total when the FIRST question comes after our own
        // writes, which is the ordering that would double count if the lazy
        // walk did not stop below `first_own_segment`.
        let third = AttestationLog::open(tmp.path()).unwrap();
        let _ = third.append(&sample_attestation()).await.unwrap();
        assert_eq!(third.record_count().await, 6);
    }

    /// The incremental read has to be the same read.
    ///
    /// `leaf_hashes_from` exists so a caller can refresh without re-reading
    /// 6.6 GB, which is only safe if the pieces compose back to exactly what
    /// `leaf_hashes` returns, in the same order. Segments are rolled the way
    /// production rolls them — every `open` starts a new one, which is why
    /// this responder has 1,390 of them — so the boundaries here are real
    /// boundaries and not a fixture's idea of one.
    #[tokio::test]
    async fn leaf_hashes_from_composes_to_the_whole_log_across_segments() {
        let tmp = tempfile::tempdir().unwrap();
        let mut appended: Vec<[u8; 32]> = Vec::new();
        let mut seq = 0u64;
        for count in [3u64, 4, 2] {
            let log = AttestationLog::open(tmp.path()).unwrap();
            for _ in 0..count {
                appended.push(
                    log.append(&distinct_attestation(seq))
                        .await
                        .unwrap()
                        .record_hash,
                );
                seq += 1;
            }
        }
        let log = AttestationLog::open(tmp.path()).unwrap();

        let whole = log.leaf_hashes().unwrap();
        assert_eq!(whole, appended, "the flat read lost or reordered records");

        let segs = log.leaf_hashes_from(0).unwrap();
        assert_eq!(
            segs.iter().map(|(i, _)| *i).collect::<Vec<_>>(),
            vec![0, 1, 2],
            "segments must come back in ascending index order"
        );
        assert_eq!(
            segs.iter().map(|(_, v)| v.len()).collect::<Vec<_>>(),
            vec![3, 4, 2]
        );
        let flat: Vec<[u8; 32]> = segs.into_iter().flat_map(|(_, v)| v).collect();
        assert_eq!(
            flat, whole,
            "per-segment read did not compose to the flat one"
        );

        // Resuming from a segment returns that segment and everything above
        // it, and nothing below: this is the property the cache relies on to
        // keep its settled prefix.
        let tail = log.leaf_hashes_from(2).unwrap();
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0].0, 2);
        assert_eq!(tail[0].1, whole[7..].to_vec());

        // Past the end is empty, not an error: a caller that has already read
        // the highest segment asks this every time the log has not grown.
        assert!(log.leaf_hashes_from(99).unwrap().is_empty());
    }

    #[tokio::test]
    async fn leaf_hashes_match_appended_records_and_prove_out() {
        use emem_attest::translog;
        let tmp = tempfile::tempdir().unwrap();
        let log = AttestationLog::open(tmp.path()).unwrap();
        // Append N distinct attestations; the per-record hash returned by
        // append() must equal the corresponding leaf read back off disk,
        // in the same append order.
        let n = 6u64;
        let mut appended: Vec<[u8; 32]> = Vec::new();
        for i in 0..n {
            let out = log.append(&distinct_attestation(i)).await.unwrap();
            appended.push(out.record_hash);
        }
        let leaves = log.leaf_hashes().unwrap();
        assert_eq!(leaves, appended, "leaf order/contents must match appends");

        // Every leaf proves inclusion under the RFC 6962 root.
        let root = translog::merkle_tree_hash(&leaves);
        for (m, _) in leaves.iter().enumerate() {
            let path = translog::inclusion_path(m, &leaves).unwrap();
            assert!(translog::verify_inclusion(
                &translog::leaf_hash(&leaves[m]),
                m,
                leaves.len(),
                &path,
                &root
            ));
        }

        // A pinned earlier size is provably a prefix of the whole log.
        let m = 4usize;
        let old_root = translog::merkle_tree_hash(&leaves[..m]);
        let proof = translog::consistency_proof(m, &leaves).unwrap();
        assert!(translog::verify_consistency(
            m,
            &old_root,
            leaves.len(),
            &root,
            &proof
        ));
    }
}
