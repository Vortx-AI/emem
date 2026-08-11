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
    state: Mutex<LogState>,
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
            state: Mutex::new(state),
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

        let mut s = self.state.lock().await;
        if s.bytes_in_segment > 0 && s.bytes_in_segment + record.len() as u64 > SEGMENT_BYTES {
            seal_segment(&self.root, &mut s)?;
        }
        let path = self.root.join(format!("merkle.log.{}", s.segment_index));
        // The open + write + fsync is the one blocking syscall on the write
        // hot path, and receipts depend on it completing before we return.
        // Run it on the blocking pool rather than the async worker: the
        // `state` lock is held across the await, so total append order and
        // the segment hash-chain stay byte-identical — only the syscall moves
        // off the runtime thread, so an fsync no longer parks a worker (the
        // failure mode when many cold writes land at once). The record is
        // moved into the closure and handed back, so the in-memory hasher
        // still advances only after a durable write (preserving the original
        // ordering: durable bytes first, then hasher).
        let record = tokio::task::spawn_blocking(move || -> std::io::Result<Vec<u8>> {
            let mut f = OpenOptions::new().create(true).append(true).open(&path)?;
            f.write_all(&record)?;
            f.sync_all()?;
            Ok(record)
        })
        .await
        .map_err(|e| std::io::Error::other(format!("merkle log append task panicked: {e}")))??;
        s.segment_hasher.update(&record);
        s.bytes_in_segment += record.len() as u64;
        s.appended += 1;
        let mut record_hash_arr = [0u8; 32];
        record_hash_arr.copy_from_slice(record_hash.as_bytes());
        Ok(AppendOutcome {
            segment_index: s.segment_index,
            offset_in_segment: s.bytes_in_segment - record.len() as u64,
            record_hash: record_hash_arr,
        })
    }

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
    /// `O(total_bytes)` — the caller (STH construction) caches the result
    /// by the log's record count so a rebuild only happens when the log
    /// has grown.
    pub fn leaf_hashes(&self) -> std::io::Result<Vec<[u8; 32]>> {
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
        let mut leaves: Vec<[u8; 32]> = Vec::new();
        for idx in indices {
            let path = self.root.join(format!("merkle.log.{idx}"));
            let mut bytes = Vec::new();
            std::fs::File::open(&path)?.read_to_end(&mut bytes)?;
            // Each record is [u32 LE len][len bytes cbor][32 bytes hash].
            // A sealed segment has a trailing 32-byte segment hash after
            // the last record; the length-driven walk below stops before
            // it (the leftover < a full record is ignored).
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
        }
        Ok(leaves)
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
