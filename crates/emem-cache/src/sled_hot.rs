//! Hot-tier cache backed by sled.
//!
//! Two trees:
//!
//! - `emem.canonical_index` — `cell\0band\0tslot_be8` → `fact_cid_string_bytes`
//! - `emem.facts`           — `fact_cid_string_bytes` → canonical CBOR of the fact
//!
//! Fact CIDs are derived deterministically: `base32_nopad_lc(blake3(canonical_cbor(fact)))`.
//! Two implementations encoding the same fact converge on the same CID, so cache
//! lookups are content-addressed end to end.

use async_trait::async_trait;
use blake3::Hasher;
use data_encoding::BASE32_NOPAD;
use std::sync::OnceLock;

use crate::redb_facts::{FactRow, IndexRows, RedbFacts};
use crate::{Cache, CacheError, CanonicalKey, Tier};
use emem_fact::{Fact, FactCid};
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

const TREE_INDEX: &str = "emem.canonical_index";
const TREE_FACTS: &str = "emem.facts";

const SEP: u8 = 0u8;

/// Global bound on how many sled operations run on the blocking pool at
/// once, across the whole process.
///
/// sled is a synchronous, blocking store. Every read and write below is a
/// blocking syscall-heavy operation, and sled additionally spawns its own
/// I/O threadpool internally. Running those directly on the tokio async
/// workers (as this cache did before) means that under a burst of cold
/// recalls or a materialize storm, enough workers block inside sled that
/// the runtime stops making progress: /health times out, the accept loop
/// starves, and the watchdog SIGKILLs a process that is "alive" but wedged
/// (2026-05-31, -06-12, -06-15, and four times on 2026-07-03; the
/// symbolised backtrace showed all 30 workers parked in parking_lot
/// condvars beneath `sled::pagecache` / `sled::threadpool`). Moving the
/// blocking work to `spawn_blocking` keeps the async workers free to poll
/// the accept loop and the gateway timer; the semaphore then bounds how
/// many run at once so a storm can neither exhaust the 512-thread blocking
/// pool nor drive sled's own threadpool to spawn without limit.
///
/// Default = 2x available parallelism, clamped to [4, 64]; override with
/// `EMEM_SLED_BLOCKING_CONCURRENCY` (clamped 1..=512).
fn sled_blocking_sem() -> &'static tokio::sync::Semaphore {
    static S: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
    S.get_or_init(|| {
        let n = std::env::var("EMEM_SLED_BLOCKING_CONCURRENCY")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or_else(|| {
                let cores = std::thread::available_parallelism()
                    .map(|p| p.get())
                    .unwrap_or(4);
                (cores * 2).clamp(4, 64)
            })
            .clamp(1, 512);
        tokio::sync::Semaphore::new(n)
    })
}

/// Run a blocking sled closure on the blocking pool under the concurrency
/// bound above, off the async reactor. `f` owns everything it touches
/// (sled `Db`/`Tree` handles are cheap `Arc`-backed clones), so it is
/// `'static` and cannot borrow an async stack frame.
async fn off_thread<T, F>(f: F) -> Result<T, CacheError>
where
    F: FnOnce() -> Result<T, CacheError> + Send + 'static,
    T: Send + 'static,
{
    let _permit = sled_blocking_sem()
        .acquire()
        .await
        .expect("sled blocking semaphore is never closed");
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| CacheError::Cbor(format!("sled blocking task panicked: {e}")))?
}

/// Prefix-scan the canonical index for one cell, decoding keys inline.
/// Shared by the synchronous [`SledHotCache::scan_cell`] and its
/// off-thread async sibling so both stay byte-identical.
/// `EMEM_HOT_BACKEND`: `redb` (default) keeps the fact index and bodies in a
/// redb file beside the sled directory; `sled` is the old layout, and the
/// rollback. The sled `Db` stays open either way: the memory trees and the
/// side indexes live there.
fn redb_enabled() -> bool {
    std::env::var("EMEM_HOT_BACKEND")
        .map(|v| v.trim() != "sled")
        .unwrap_or(true)
}

fn redb_path_for(sled_path: &std::path::Path) -> std::path::PathBuf {
    sled_path
        .parent()
        .map(|d| d.join("facts.redb"))
        .unwrap_or_else(|| std::path::PathBuf::from("facts.redb"))
}

/// Where fact rows come from: redb when present, the sled index for rows the
/// backfill has not copied yet, never sled once the backfill is done.
#[derive(Clone)]
struct Source {
    idx: sled::Tree,
    redb: Option<Arc<RedbFacts>>,
}

impl Source {
    fn consult_sled(&self) -> bool {
        self.redb
            .as_ref()
            .map(|r| !r.backfill_done())
            .unwrap_or(true)
    }

    /// Index rows under `prefix`, key order, at most `limit`; the second
    /// value is how many rows were seen, for the limit-hit warning.
    fn rows_with_prefix(
        &self,
        prefix: &[u8],
        limit: usize,
    ) -> Result<(IndexRows, usize), CacheError> {
        let mut merged: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
        let mut seen = 0usize;
        if self.consult_sled() {
            for kv in self.idx.scan_prefix(prefix) {
                seen += 1;
                if merged.len() >= limit {
                    break;
                }
                let (k, v) = kv?;
                merged.insert(k.to_vec(), v.to_vec());
            }
        }
        if let Some(r) = &self.redb {
            let (rows, rseen) = r.scan_prefix(prefix, limit)?;
            seen = seen.max(rseen);
            for (k, v) in rows {
                merged.insert(k, v); // redb is authoritative for a key in both
            }
        }
        let out: IndexRows = merged.into_iter().take(limit).collect();
        Ok((out, seen))
    }
}

fn scan_limit() -> usize {
    std::env::var("EMEM_SCAN_CELL_LIMIT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10_000)
}

fn cell_prefix(cell: &str) -> Vec<u8> {
    let mut prefix = Vec::with_capacity(cell.len() + 1);
    prefix.extend_from_slice(cell.as_bytes());
    prefix.push(SEP);
    prefix
}

fn scan_cell_tree(
    src: &Source,
    cell: &str,
    tslot: Option<u64>,
) -> Result<Vec<(CanonicalKey, FactCid)>, CacheError> {
    scan_cell_bound_tree(src, cell, tslot, None)
}

fn scan_cell_bound_tree(
    src: &Source,
    cell: &str,
    tslot_eq: Option<u64>,
    tslot_le: Option<u64>,
) -> Result<Vec<(CanonicalKey, FactCid)>, CacheError> {
    let limit = scan_limit();
    let (rows, seen) = src.rows_with_prefix(&cell_prefix(cell), limit)?;
    if rows.len() >= limit && seen > limit {
        tracing::warn!(
            target: "emem::storage",
            scan_cell = %cell,
            scan_limit = limit,
            scan_seen = seen,
            "scan_cell_limit_hit",
        );
    }
    let mut out = Vec::with_capacity(rows.len());
    for (k, v) in rows {
        let key = decode_key(&k).map_err(CacheError::Cbor)?;
        if let Some(t) = tslot_eq {
            if key.tslot != t {
                continue;
            }
        }
        if let Some(t) = tslot_le {
            if key.tslot > t {
                continue;
            }
        }
        let cid_s = std::str::from_utf8(&v)
            .map_err(|e| CacheError::Cbor(e.to_string()))?
            .to_string();
        out.push((key, FactCid::new(cid_s)));
    }
    Ok(out)
}

/// The whole index in key order: redb by pages (a fresh read transaction
/// per page, so no guard outlives a page), then, while the backfill is
/// still running, the sled rows redb does not have yet.
fn index_iter(
    src: Source,
) -> Box<dyn Iterator<Item = Result<(CanonicalKey, FactCid), CacheError>> + Send> {
    fn row(k: &[u8], v: &[u8]) -> Result<(CanonicalKey, FactCid), CacheError> {
        let key = decode_key(k).map_err(CacheError::Cbor)?;
        let cid_s = std::str::from_utf8(v)
            .map_err(|e| CacheError::Cbor(e.to_string()))?
            .to_string();
        Ok((key, FactCid::new(cid_s)))
    }
    match src.redb.clone() {
        None => Box::new(src.idx.iter().map(|kv| {
            let (k, v) = kv?;
            row(&k, &v)
        })),
        Some(r) => {
            let pages = RedbIndexIter {
                r: r.clone(),
                after: None,
                buf: VecDeque::new(),
                exhausted: false,
            };
            if src.consult_sled() {
                let r2 = r.clone();
                let tail = src.idx.iter().filter_map(move |kv| match kv {
                    Err(e) => Some(Err(CacheError::from(e))),
                    Ok((k, v)) => match r2.contains_index(&k) {
                        Ok(true) => None,
                        Ok(false) => Some(row(&k, &v)),
                        Err(e) => Some(Err(e)),
                    },
                });
                Box::new(pages.chain(tail))
            } else {
                Box::new(pages)
            }
        }
    }
}

struct RedbIndexIter {
    r: Arc<RedbFacts>,
    after: Option<Vec<u8>>,
    buf: VecDeque<(Vec<u8>, Vec<u8>)>,
    exhausted: bool,
}

impl Iterator for RedbIndexIter {
    type Item = Result<(CanonicalKey, FactCid), CacheError>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.buf.is_empty() && !self.exhausted {
            match self.r.index_page(self.after.as_deref(), 2048) {
                Ok(page) => {
                    if page.is_empty() {
                        self.exhausted = true;
                    } else {
                        self.after = page.last().map(|(k, _)| k.clone());
                        self.buf.extend(page);
                    }
                }
                Err(e) => {
                    self.exhausted = true;
                    return Some(Err(e));
                }
            }
        }
        let (k, v) = self.buf.pop_front()?;
        Some(decode_key(&k).map_err(CacheError::Cbor).and_then(|key| {
            let cid_s = std::str::from_utf8(&v)
                .map_err(|e| CacheError::Cbor(e.to_string()))?
                .to_string();
            Ok((key, FactCid::new(cid_s)))
        }))
    }
}

/// Copy the sled fact index and bodies into redb in the background: small
/// batches, a pause between them, a free-disk guard, a resumable cursor, a
/// durable commit every twenty batches. Rows already in redb (new writes)
/// are skipped. When the sled index is exhausted the store is marked done
/// and sled is never consulted for facts again.
/// One backfill batch: up to `batch` sled index rows strictly after
/// `cursor`, copied into redb unless already there. Returns (copied,
/// skipped, last key seen); `None` for the last key means the sled index is
/// exhausted. Non-durable commits; the caller syncs periodically.
fn backfill_step(
    r: &RedbFacts,
    idx: &sled::Tree,
    facts: &sled::Tree,
    cursor: Option<&[u8]>,
    batch: usize,
) -> Result<(u64, u64, Option<Vec<u8>>), CacheError> {
    let mut items = Vec::with_capacity(batch);
    let (mut n, mut skip, mut last) = (0usize, 0u64, None);
    let iter: Box<dyn Iterator<Item = sled::Result<(sled::IVec, sled::IVec)>>> = match cursor {
        Some(c) => Box::new(idx.range(c.to_vec()..)),
        None => Box::new(idx.iter()),
    };
    for kv in iter {
        let (k, v) = kv?;
        if let Some(c) = cursor {
            if k.as_ref() == c {
                continue;
            }
        }
        n += 1;
        last = Some(k.to_vec());
        if r.contains_index(&k)? {
            skip += 1;
        } else if let Some(body) = facts.get(&v)? {
            items.push((v.to_vec(), body.to_vec(), Some(k.to_vec())));
        } else {
            skip += 1;
        }
        if n >= batch {
            break;
        }
    }
    let n_copied = items.len() as u64;
    if !items.is_empty() {
        r.put_batch(&items, false)?;
    }
    if let Some(l) = &last {
        r.set_backfill_cursor(l, false)?;
    }
    Ok((n_copied, skip, last))
}

fn spawn_backfill(r: Arc<RedbFacts>, idx: sled::Tree, facts: sled::Tree) {
    let Ok(h) = tokio::runtime::Handle::try_current() else {
        tracing::info!("hot backfill not started: no async runtime (CLI use)");
        return;
    };
    h.spawn(async move {
        let batch: usize = std::env::var("EMEM_HOT_BACKFILL_BATCH")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1000)
            .clamp(50, 20_000);
        let pause = std::time::Duration::from_millis(
            std::env::var("EMEM_HOT_BACKFILL_PAUSE_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(250)
                .clamp(0, 60_000),
        );
        let min_free: u64 = std::env::var("EMEM_HOT_BACKFILL_MIN_FREE_BYTES")
            .ok()
            .and_then(|v| parse_bytes(v.trim()))
            .unwrap_or(4 << 30);
        let dir = r
            .path()
            .parent()
            .map(|d| d.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let mut cursor = r.backfill_cursor().ok().flatten();
        let (mut batches, mut copied, mut skipped) = (0u64, 0u64, 0u64);
        let t0 = std::time::Instant::now();
        tracing::info!(resume_from = cursor.is_some(), "hot backfill started: sled facts -> redb");
        loop {
            if let Ok(free) = fs2::available_space(&dir) {
                if free < min_free {
                    tracing::warn!(free_bytes = free, min_free_bytes = min_free, "hot backfill paused: low disk");
                    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                    continue;
                }
            }
            let (r2, idx2, facts2, cur) = (r.clone(), idx.clone(), facts.clone(), cursor.clone());
            let step = off_thread(move || backfill_step(&r2, &idx2, &facts2, cur.as_deref(), batch)).await;
            match step {
                Ok((n_copied, n_skip, last)) => {
                    copied += n_copied;
                    skipped += n_skip;
                    batches += 1;
                    r.backfilled.store(copied, std::sync::atomic::Ordering::Relaxed);
                    if last.is_none() {
                        if let Err(e) = r.mark_backfill_done() {
                            tracing::warn!(error = %e, "hot backfill: could not mark done; retrying");
                            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                            continue;
                        }
                        tracing::info!(copied, skipped, batches, secs = t0.elapsed().as_secs(), redb_bytes = r.size_on_disk(),
                            "hot backfill complete: sled is no longer consulted for facts");
                        return;
                    }
                    cursor = last;
                    if batches % 20 == 0 {
                        if let Err(e) = r.sync() {
                            tracing::warn!(error = %e, "hot backfill: durable commit failed");
                        }
                    }
                    if batches % 200 == 0 {
                        tracing::info!(copied, skipped, batches, secs = t0.elapsed().as_secs(), redb_bytes = r.size_on_disk(), "hot backfill progress");
                    }
                    tokio::time::sleep(pause).await;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "hot backfill step failed; retrying in 5 s");
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
            }
        }
    });
}

pub struct SledHotCache {
    db: sled::Db,
    idx: sled::Tree,
    facts: sled::Tree,
    redb: Option<Arc<RedbFacts>>,
    _tmp: Option<tempfile::TempDir>,
}

/// `EMEM_SLED_CACHE_BYTES`: the sled pagecache budget for the hot store.
/// Default 8 GiB; clamped to [256 MiB, 64 GiB]. Bytes, or a number with a
/// `g`/`m` suffix.
fn sled_cache_bytes() -> u64 {
    const MIN: u64 = 256 << 20;
    const MAX: u64 = 64 << 30;
    const DEFAULT: u64 = 8 << 30;
    std::env::var("EMEM_SLED_CACHE_BYTES")
        .ok()
        .and_then(|v| parse_bytes(v.trim()))
        .unwrap_or(DEFAULT)
        .clamp(MIN, MAX)
}

/// `EMEM_SLED_FLUSH_MS`: how often sled's flusher makes the log stable.
/// Default 200 ms (sled's own is 500); clamped to [50, 5000].
fn sled_flush_every_ms() -> u64 {
    std::env::var("EMEM_SLED_FLUSH_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(200)
        .clamp(50, 5000)
}

pub(crate) fn parse_bytes(v: &str) -> Option<u64> {
    let lower = v.to_ascii_lowercase();
    let (num, mult) = if let Some(n) = lower.strip_suffix('g') {
        (n, 1u64 << 30)
    } else if let Some(n) = lower.strip_suffix('m') {
        (n, 1u64 << 20)
    } else {
        (lower.as_str(), 1u64)
    };
    num.trim().parse::<u64>().ok()?.checked_mul(mult)
}

#[cfg(test)]
mod sled_config_tests {
    use super::*;

    #[test]
    fn byte_sizes_parse_with_and_without_suffixes() {
        assert_eq!(parse_bytes("8g"), Some(8 << 30));
        assert_eq!(parse_bytes("512M"), Some(512 << 20));
        assert_eq!(parse_bytes("1048576"), Some(1 << 20));
        assert_eq!(parse_bytes("lots"), None);
    }

    #[test]
    fn defaults_and_clamps_hold_without_env() {
        // The env is process-global; these assert only the unset path and
        // the clamp arithmetic, never a value set by another test.
        assert!(sled_cache_bytes() >= 256 << 20 && sled_cache_bytes() <= 64 << 30);
        assert!((50..=5000).contains(&sled_flush_every_ms()));
    }
}

impl SledHotCache {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, CacheError> {
        // Not `sled::open` with its defaults. A 1 GiB pagecache in front of
        // a store that reached 58 GB meant most reads pulled pages from the
        // log, and in sled 0.34 a pull of a page written inside the current
        // flush window waits for that buffer's fsync (`PageCache::get` ->
        // `make_stable`). The wedge snapshots of 2026-09-02 (var/wedge)
        // show 147 threads, twelve of them tokio core workers, parked in
        // exactly that wait while the disk sat idle: eleven watchdog
        // restarts in one day. A larger cache cuts the pulls; a shorter
        // flush interval shortens the wait. Both are operator knobs.
        let path = path.as_ref();
        let db = sled::Config::new()
            .path(path)
            .cache_capacity(sled_cache_bytes())
            .flush_every_ms(Some(sled_flush_every_ms()))
            .open()?;
        let idx = db.open_tree(TREE_INDEX)?;
        let facts = db.open_tree(TREE_FACTS)?;
        let redb = if redb_enabled() {
            let rp = redb_path_for(path);
            let r = Arc::new(RedbFacts::open(&rp)?);
            tracing::info!(path = %rp.display(), backfill_done = r.backfill_done(), "hot facts on redb");
            if !r.backfill_done() {
                spawn_backfill(r.clone(), idx.clone(), facts.clone());
            }
            Some(r)
        } else {
            tracing::warn!("EMEM_HOT_BACKEND=sled: facts on sled 0.34, the layout that wedged");
            None
        };
        Ok(Self {
            db,
            idx,
            facts,
            redb,
            _tmp: None,
        })
    }

    /// Open an in-memory (temporary) cache. Useful for tests and the
    /// dev server's first-boot bootstrap.
    pub fn open_temporary() -> Result<Self, CacheError> {
        let db = sled::Config::new().temporary(true).open()?;
        let idx = db.open_tree(TREE_INDEX)?;
        let facts = db.open_tree(TREE_FACTS)?;
        let (redb, tmp) = if redb_enabled() {
            let tmp = tempfile::tempdir()?;
            let r = RedbFacts::open(tmp.path().join("facts.redb"))?;
            r.mark_backfill_done()?; // nothing to copy from an empty sled
            (Some(Arc::new(r)), Some(tmp))
        } else {
            (None, None)
        };
        Ok(Self {
            db,
            idx,
            facts,
            redb,
            _tmp: tmp,
        })
    }

    fn source(&self) -> Source {
        Source {
            idx: self.idx.clone(),
            redb: self.redb.clone(),
        }
    }

    /// Iterate every (canonical_key, fact_cid) in the index. Used by
    /// primitives like find_similar that need a corpus-wide scan.
    pub fn iter_index(
        &self,
    ) -> impl Iterator<Item = Result<(CanonicalKey, FactCid), CacheError>> + '_ {
        index_iter(self.source())
    }

    /// Prefix-scan the index by cell64 (and optional tslot equality filter).
    /// Caps iteration at `EMEM_SCAN_CELL_LIMIT` rows (default 10_000) so a
    /// pathologically dense cell can't tie up a request thread.
    pub fn scan_cell(
        &self,
        cell: &str,
        tslot: Option<u64>,
    ) -> Result<Vec<(CanonicalKey, FactCid)>, CacheError> {
        scan_cell_tree(&self.source(), cell, tslot)
    }

    pub async fn scan_cell_off(
        &self,
        cell: &str,
        tslot: Option<u64>,
    ) -> Result<Vec<(CanonicalKey, FactCid)>, CacheError> {
        let src = self.source();
        let cell = cell.to_string();
        off_thread(move || scan_cell_tree(&src, &cell, tslot)).await
    }

    pub fn scan_cell_with_tslot_bound(
        &self,
        cell: &str,
        tslot_eq: Option<u64>,
        tslot_le: Option<u64>,
    ) -> Result<Vec<(CanonicalKey, FactCid)>, CacheError> {
        scan_cell_bound_tree(&self.source(), cell, tslot_eq, tslot_le)
    }

    pub async fn scan_cell_with_tslot_bound_off(
        &self,
        cell: &str,
        tslot_eq: Option<u64>,
        tslot_le: Option<u64>,
    ) -> Result<Vec<(CanonicalKey, FactCid)>, CacheError> {
        let src = self.source();
        let cell = cell.to_string();
        off_thread(move || scan_cell_bound_tree(&src, &cell, tslot_eq, tslot_le)).await
    }

    pub async fn collect_index_off(
        &self,
        limit: Option<usize>,
    ) -> Result<Vec<(CanonicalKey, FactCid)>, CacheError> {
        let src = self.source();
        off_thread(move || {
            let mut out = Vec::new();
            for item in index_iter(src) {
                out.push(item?);
                if let Some(n) = limit {
                    if out.len() >= n {
                        break;
                    }
                }
            }
            Ok(out)
        })
        .await
    }

    pub fn len(&self) -> usize {
        match &self.redb {
            Some(r) if r.backfill_done() => r.index_len().unwrap_or(0) as usize,
            Some(r) => self.idx.len().max(r.index_len().unwrap_or(0) as usize),
            None => self.idx.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The sled `Db`: the memory trees, the attester registry, the trace
    /// gate and the side indexes still live there.
    pub fn db(&self) -> &sled::Db {
        &self.db
    }

    /// The redb fact store, when the backend is redb.
    pub fn redb(&self) -> Option<&Arc<RedbFacts>> {
        self.redb.as_ref()
    }

    pub fn size_on_disk(&self) -> Result<u64, CacheError> {
        Ok(self.db.size_on_disk()? + self.redb.as_ref().map(|r| r.size_on_disk()).unwrap_or(0))
    }
}

pub fn fact_cid_of(fact: &Fact) -> Result<FactCid, CacheError> {
    let cbor = fact_to_cbor(fact)?;
    let mut h = Hasher::new();
    h.update(&cbor);
    let hash = h.finalize();
    let s = BASE32_NOPAD.encode(hash.as_bytes()).to_lowercase();
    Ok(FactCid::new(s))
}

/// The exact bytes a [`FactCid`] commits to: `ciborium(fact)`.
///
/// Public because a caller who cannot obtain these bytes cannot check that the
/// value a responder shows is the value its content id addresses. The receipt
/// signature proves the responder attested to a LIST OF CIDS; it says nothing
/// about the numbers printed beside them. Without a way to recompute
/// `blake3(these bytes)` and compare it to the cid, "content-addressed" is a
/// property of our storage rather than a claim a reader can check.
///
/// Reconstructing them from the JSON form is not possible in practice: the
/// document cannot carry CBOR type widths (`confidence` is an f32 written as
/// CBOR float32, which JSON widens), nor the serialization of the nested
/// newtypes. Serving them is the difference between a verifiable claim and one
/// that has to be taken on trust.
pub fn fact_canonical_cbor(fact: &Fact) -> Result<Vec<u8>, CacheError> {
    fact_to_cbor(fact)
}

fn fact_to_cbor(fact: &Fact) -> Result<Vec<u8>, CacheError> {
    let mut buf = Vec::new();
    ciborium::ser::into_writer(fact, &mut buf).map_err(|e| CacheError::Cbor(e.to_string()))?;
    Ok(buf)
}

fn cbor_to_fact(bytes: &[u8]) -> Result<Fact, CacheError> {
    ciborium::de::from_reader(bytes).map_err(|e| CacheError::Cbor(e.to_string()))
}

fn encode_key(k: &CanonicalKey) -> Vec<u8> {
    let mut buf = Vec::with_capacity(k.cell.len() + k.band.len() + 10);
    buf.extend_from_slice(k.cell.as_bytes());
    buf.push(SEP);
    buf.extend_from_slice(k.band.as_bytes());
    buf.push(SEP);
    buf.extend_from_slice(&k.tslot.to_be_bytes());
    buf
}

fn decode_key(b: &[u8]) -> Result<CanonicalKey, String> {
    let mut parts = b.splitn(3, |c| *c == SEP);
    let cell = parts.next().ok_or("missing cell")?;
    let band = parts.next().ok_or("missing band")?;
    let rest = parts.next().ok_or("missing tslot")?;
    if rest.len() != 8 {
        return Err(format!("tslot must be 8 BE bytes, got {}", rest.len()));
    }
    let mut t = [0u8; 8];
    t.copy_from_slice(rest);
    Ok(CanonicalKey {
        cell: std::str::from_utf8(cell)
            .map_err(|e| e.to_string())?
            .to_string(),
        band: std::str::from_utf8(band)
            .map_err(|e| e.to_string())?
            .to_string(),
        tslot: u64::from_be_bytes(t),
    })
}

/// The canonical key derived from a fact's storage tuple. Returns None for
/// derivative facts (which are keyed by parent CIDs, not cell/band/tslot).
fn fact_canonical_key(fact: &Fact) -> Option<CanonicalKey> {
    match fact {
        Fact::Primary(p) => Some(CanonicalKey {
            cell: p.cell.clone(),
            band: p.band.clone(),
            tslot: p.tslot,
        }),
        Fact::Absence(n) => Some(CanonicalKey {
            cell: n.cell.clone(),
            band: n.band.clone(),
            tslot: n.tslot,
        }),
        Fact::Derivative(_) => None,
    }
}

#[async_trait]
impl Cache for SledHotCache {
    async fn lookup_many(&self, keys: &[CanonicalKey]) -> Result<Vec<Option<FactCid>>, CacheError> {
        let src = self.source();
        let keys = keys.to_vec();
        off_thread(move || {
            let mut out = Vec::with_capacity(keys.len());
            for k in &keys {
                let kb = encode_key(k);
                let hit: Option<Vec<u8>> = match &src.redb {
                    Some(r) => match r.lookup(&kb)? {
                        Some(v) => Some(v),
                        None if src.consult_sled() => src.idx.get(&kb)?.map(|v| v.to_vec()),
                        None => None,
                    },
                    None => src.idx.get(&kb)?.map(|v| v.to_vec()),
                };
                match hit {
                    Some(v) => {
                        let s = std::str::from_utf8(&v)
                            .map_err(|e| CacheError::Cbor(e.to_string()))?
                            .to_string();
                        out.push(Some(FactCid::new(s)));
                    }
                    None => out.push(None),
                }
            }
            Ok(out)
        })
        .await
    }

    async fn get_many(&self, cids: &[FactCid]) -> Result<Vec<Option<Fact>>, CacheError> {
        let facts = self.facts.clone();
        let src = self.source();
        let cids = cids.to_vec();
        off_thread(move || {
            let mut out = Vec::with_capacity(cids.len());
            for cid in &cids {
                let body: Option<Vec<u8>> = match &src.redb {
                    Some(r) => match r.get_fact(cid.as_str().as_bytes())? {
                        Some(b) => Some(b),
                        None if src.consult_sled() => {
                            facts.get(cid.as_str().as_bytes())?.map(|b| b.to_vec())
                        }
                        None => None,
                    },
                    None => facts.get(cid.as_str().as_bytes())?.map(|b| b.to_vec()),
                };
                match body {
                    Some(b) => out.push(Some(cbor_to_fact(&b)?)),
                    None => out.push(None),
                }
            }
            Ok(out)
        })
        .await
    }

    async fn put_many(&self, facts: &[Fact]) -> Result<Vec<FactCid>, CacheError> {
        let facts_tree = self.facts.clone();
        let idx = self.idx.clone();
        let redb = self.redb.clone();
        let facts_in = facts.to_vec();
        let out = off_thread(move || {
            let mut out = Vec::with_capacity(facts_in.len());
            let mut items: Vec<FactRow> = Vec::with_capacity(facts_in.len());
            for f in &facts_in {
                let cbor = fact_to_cbor(f)?;
                let mut h = Hasher::new();
                h.update(&cbor);
                let hash = h.finalize();
                let cid_s = BASE32_NOPAD.encode(hash.as_bytes()).to_lowercase();
                let cid = FactCid::new(cid_s);
                let key = fact_canonical_key(f).map(|k| encode_key(&k));
                match &redb {
                    // New facts go to redb only: durable when the commit
                    // returns, and no explicit sled flush from this path.
                    Some(_) => items.push((cid.as_str().as_bytes().to_vec(), cbor, key)),
                    None => {
                        facts_tree.insert(cid.as_str().as_bytes(), cbor)?;
                        if let Some(k) = key {
                            idx.insert(k, cid.as_str().as_bytes())?;
                        }
                    }
                }
                out.push(cid);
            }
            if let Some(r) = &redb {
                r.put_batch(&items, true)?;
            }
            Ok(out)
        })
        .await?;
        if self.redb.is_none() {
            self.facts
                .flush_async()
                .await
                .map_err(|e| CacheError::Cbor(e.to_string()))?;
        }
        Ok(out)
    }

    async fn tier_of(&self, cid: &FactCid) -> Result<Option<Tier>, CacheError> {
        let facts = self.facts.clone();
        let src = self.source();
        let cid = cid.clone();
        off_thread(move || {
            let present = match &src.redb {
                Some(r) => {
                    r.contains_fact(cid.as_str().as_bytes())?
                        || (src.consult_sled() && facts.contains_key(cid.as_str().as_bytes())?)
                }
                None => facts.contains_key(cid.as_str().as_bytes())?,
            };
            Ok(if present { Some(Tier::Hot) } else { None })
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use emem_core::AttesterKey;
    use emem_fact::{Derivation, PrimaryFact, SchemaCid, Source};

    pub(super) fn sample_fact(cell: &str, band: &str, tslot: u64) -> Fact {
        Fact::Primary(PrimaryFact {
            cell: cell.into(),
            band: band.into(),
            tslot,
            value: ciborium::Value::Float(0.42),
            unit: None,
            confidence: 1.0,
            uncertainty: None,
            sources: vec![Source {
                scheme: "test".into(),
                id: "t1".into(),
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
            schema_cid: SchemaCid::new("test"),
            signer: AttesterKey([0u8; 32]),
            signed_at: "2026-01-01T00:00:00Z".into(),
            served_via: None,
        })
    }

    #[tokio::test]
    async fn put_then_lookup_roundtrips() {
        let c = SledHotCache::open_temporary().unwrap();
        let f = sample_fact("ento.bria.calo.tris", "indices.ndvi", 7);
        let cids = c.put_many(std::slice::from_ref(&f)).await.unwrap();
        assert_eq!(cids.len(), 1);

        let key = CanonicalKey {
            cell: "ento.bria.calo.tris".into(),
            band: "indices.ndvi".into(),
            tslot: 7,
        };
        let hits = c.lookup_many(&[key]).await.unwrap();
        assert_eq!(hits[0], Some(cids[0].clone()));

        let facts = c.get_many(&cids).await.unwrap();
        assert!(facts[0].is_some());
    }

    #[tokio::test]
    async fn scan_cell_filters_by_tslot() {
        let c = SledHotCache::open_temporary().unwrap();
        c.put_many(&[
            sample_fact("ento.bria.calo.tris", "indices.ndvi", 7),
            sample_fact("ento.bria.calo.tris", "indices.evi", 7),
            sample_fact("ento.bria.calo.tris", "indices.ndvi", 8),
        ])
        .await
        .unwrap();

        let only_t7 = c.scan_cell("ento.bria.calo.tris", Some(7)).unwrap();
        assert_eq!(only_t7.len(), 2);
        let all = c.scan_cell("ento.bria.calo.tris", None).unwrap();
        assert_eq!(all.len(), 3);
    }

    /// Bi-temporal valid-time fast path: pre-filter on `tslot <= bound`
    /// directly from the canonical-key bytes — no body load required.
    #[tokio::test]
    async fn scan_cell_with_tslot_bound_filters_in_index() {
        let c = SledHotCache::open_temporary().unwrap();
        c.put_many(&[
            sample_fact("ento.bria.calo.tris", "indices.ndvi", 5),
            sample_fact("ento.bria.calo.tris", "indices.ndvi", 7),
            sample_fact("ento.bria.calo.tris", "indices.ndvi", 9),
        ])
        .await
        .unwrap();

        // Cap at 7 → keep tslots 5 + 7, drop 9.
        let bounded = c
            .scan_cell_with_tslot_bound("ento.bria.calo.tris", None, Some(7))
            .unwrap();
        let mut tslots: Vec<u64> = bounded.iter().map(|(k, _)| k.tslot).collect();
        tslots.sort_unstable();
        assert_eq!(tslots, vec![5, 7], "tslot_le=7 must drop the tslot=9 entry");

        // Same as scan_cell when both bounds are None.
        let all = c
            .scan_cell_with_tslot_bound("ento.bria.calo.tris", None, None)
            .unwrap();
        assert_eq!(all.len(), 3);

        // Exact + ceiling: tslot_eq wins precedence over the ceiling
        // (exact match excludes everything else regardless of bound).
        let exact = c
            .scan_cell_with_tslot_bound("ento.bria.calo.tris", Some(7), Some(100))
            .unwrap();
        assert_eq!(exact.len(), 1);
        assert_eq!(exact[0].0.tslot, 7);
    }
}

#[cfg(test)]
mod cid_preimage_tests {
    use super::*;

    /// The bytes we serve MUST hash to the cid that addresses them.
    ///
    /// This is the claim `/v1/facts/{cid}` with `Accept: application/cbor` makes
    /// by existing: a reader recomputes `blake3` over the body, base32-encodes
    /// it, and compares to the id they asked for. If those two ever come apart,
    /// every caller who checked would see it -- so this asserts the property
    /// here, where it is cheap, rather than waiting for someone outside to find
    /// it.
    ///
    /// Written as round-tripping the two public functions against each other
    /// because they are the two halves a caller uses: the id we publish and the
    /// preimage we publish. A test that recomputed the hash by calling the same
    /// helper twice would prove nothing.
    #[test]
    fn the_canonical_bytes_hash_to_the_cid_that_addresses_them() {
        use data_encoding::BASE32_NOPAD;

        let fact = super::tests::sample_fact("defi.zb64a.cAzU.zfa27", "indices.ndvi", 20367);
        let cid = fact_cid_of(&fact).expect("cid");
        let bytes = fact_canonical_cbor(&fact).expect("bytes");

        let mut h = Hasher::new();
        h.update(&bytes);
        let recomputed = BASE32_NOPAD.encode(h.finalize().as_bytes()).to_lowercase();

        assert_eq!(
            recomputed,
            cid.as_str(),
            "the served preimage does not hash to the id it is served under"
        );
        assert_eq!(cid.as_str().len(), 52, "256 bits, base32-nopad");

        // A CHANGED VALUE MUST CHANGE THE ADDRESS. Without this the assertion
        // above passes for a function that returns a constant.
        let mut other = fact.clone();
        if let Fact::Primary(p) = &mut other {
            p.value = ciborium::Value::Float(0.123_456_f64);
        }
        let other_cid = fact_cid_of(&other).expect("cid");
        assert_ne!(
            other_cid.as_str(),
            cid.as_str(),
            "a different value must have a different address, or the address \
             is not addressing the content"
        );
        let other_bytes = fact_canonical_cbor(&other).expect("bytes");
        assert_ne!(other_bytes, bytes, "and different content, different bytes");
    }
}

#[cfg(test)]
mod redb_cutover_tests {
    use super::*;

    fn sample(cell: &str, band: &str, tslot: u64) -> Fact {
        tests::sample_fact(cell, band, tslot)
    }

    /// Rows written straight into the old sled trees (the state on disk the
    /// day of the cutover) are found through redb's read-through until the
    /// backfill has copied them, and from redb alone afterwards.
    #[tokio::test]
    async fn sled_rows_are_read_through_then_copied_then_owned_by_redb() {
        let c = SledHotCache::open_temporary().unwrap();
        let Some(r) = c.redb.clone() else {
            return; // EMEM_HOT_BACKEND=sled in this environment
        };
        // open_temporary marks the backfill done (nothing to copy); undo
        // that to model a real cutover with an existing sled store.
        r.backfill_done
            .store(false, std::sync::atomic::Ordering::Release);
        // Write two facts the OLD way, into sled only.
        let f1 = sample("ento.bria.calo.tris", "indices.ndvi", 7);
        let f2 = sample("ento.bria.calo.tris", "indices.ndvi", 9);
        let mut cids = Vec::new();
        for f in [&f1, &f2] {
            let cbor = fact_to_cbor(f).unwrap();
            let cid = fact_cid_of(f).unwrap();
            c.facts.insert(cid.as_str().as_bytes(), cbor).unwrap();
            c.idx
                .insert(
                    encode_key(&fact_canonical_key(f).unwrap()),
                    cid.as_str().as_bytes(),
                )
                .unwrap();
            cids.push(cid);
        }
        // Read-through: redb has nothing yet, sled answers.
        assert_eq!(r.index_len().unwrap(), 0);
        let keys: Vec<CanonicalKey> = [&f1, &f2]
            .iter()
            .map(|f| fact_canonical_key(f).unwrap())
            .collect();
        let hits = c.lookup_many(&keys).await.unwrap();
        assert_eq!(hits, vec![Some(cids[0].clone()), Some(cids[1].clone())]);
        assert!(c.get_many(&cids).await.unwrap().iter().all(|f| f.is_some()));
        assert_eq!(c.scan_cell("ento.bria.calo.tris", None).unwrap().len(), 2);
        assert_eq!(c.iter_index().count(), 2, "union of redb (empty) and sled");
        // A new write goes to redb only.
        let f3 = sample("ento.bria.calo.tris", "indices.ndvi", 11);
        let new = c.put_many(std::slice::from_ref(&f3)).await.unwrap();
        assert!(r.contains_fact(new[0].as_str().as_bytes()).unwrap());
        assert!(
            !c.facts.contains_key(new[0].as_str().as_bytes()).unwrap(),
            "sled must not receive new facts"
        );
        assert_eq!(
            c.scan_cell("ento.bria.calo.tris", None).unwrap().len(),
            3,
            "union scan sees both stores"
        );
        assert_eq!(c.iter_index().count(), 3);
        // Backfill in batches of one until exhausted.
        let mut cursor: Option<Vec<u8>> = None;
        let mut copied = 0;
        loop {
            let (n, _skip, last) =
                backfill_step(&r, &c.idx, &c.facts, cursor.as_deref(), 1).unwrap();
            copied += n;
            match last {
                Some(l) => cursor = Some(l),
                None => break,
            }
        }
        assert_eq!(
            copied, 2,
            "the two sled-only rows were copied, the redb-only row was not re-copied"
        );
        r.mark_backfill_done().unwrap();
        // Sled is no longer consulted: remove everything from it and the
        // answers must not change.
        c.idx.clear().unwrap();
        c.facts.clear().unwrap();
        let hits = c.lookup_many(&keys).await.unwrap();
        assert_eq!(hits, vec![Some(cids[0].clone()), Some(cids[1].clone())]);
        assert_eq!(c.scan_cell("ento.bria.calo.tris", None).unwrap().len(), 3);
        assert_eq!(c.iter_index().count(), 3);
        assert_eq!(c.len(), 3);
        assert!(c.get_many(&cids).await.unwrap().iter().all(|f| f.is_some()));
    }

    /// The cursor resumes: a second pass over an already-copied store copies
    /// nothing and still reaches the end.
    #[tokio::test]
    async fn backfill_resumes_from_its_cursor_and_copies_nothing_twice() {
        let c = SledHotCache::open_temporary().unwrap();
        let Some(r) = c.redb.clone() else { return };
        for t in [1u64, 2, 3] {
            let f = sample("ento.bria.calo.tris", "indices.ndvi", t);
            let cbor = fact_to_cbor(&f).unwrap();
            let cid = fact_cid_of(&f).unwrap();
            c.facts.insert(cid.as_str().as_bytes(), cbor).unwrap();
            c.idx
                .insert(
                    encode_key(&fact_canonical_key(&f).unwrap()),
                    cid.as_str().as_bytes(),
                )
                .unwrap();
        }
        let (n1, _, last1) = backfill_step(&r, &c.idx, &c.facts, None, 2).unwrap();
        assert_eq!(n1, 2);
        let resumed = r.backfill_cursor().unwrap();
        assert_eq!(resumed, last1, "the cursor is persisted after each batch");
        let (n2, _, last2) = backfill_step(&r, &c.idx, &c.facts, resumed.as_deref(), 2).unwrap();
        assert_eq!(n2, 1);
        let (n3, _, last3) = backfill_step(&r, &c.idx, &c.facts, last2.as_deref(), 2).unwrap();
        assert_eq!((n3, last3), (0, None), "exhausted");
        let (n4, skip4, _) = backfill_step(&r, &c.idx, &c.facts, None, 10).unwrap();
        assert_eq!(
            (n4, skip4),
            (0, 3),
            "a fresh pass skips every row redb already has"
        );
    }
}
