//! The fact index and fact bodies on redb, beside the sled `Db` that keeps
//! the memory trees and the side indexes.
//!
//! Why a second engine for two trees: every wedge snapshot on 2026-09-03
//! paired sled readers pulling fact pages (`PageCache::get` waiting in
//! `make_stable`) with one explicit flush from a memory-tree write stuck in
//! the same call, sled's flusher threads parked, the disk idle. Taking the
//! fact reads out of that file removes one half of the pair. redb's readers
//! never wait on writers (MVCC), a commit is durable when it returns
//! (`Durability::Immediate`), and the file compacts.
//!
//! Cutover is lazy: new facts are written here only; a miss falls through to
//! the old sled trees until the background backfill has copied them; then
//! sled is never consulted for facts again. Rollback is `EMEM_HOT_BACKEND=sled`.
use crate::CacheError;
use redb::{Database, Durability, ReadableDatabase, ReadableTableMetadata, TableDefinition};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

/// canonical key bytes (`cell\0band\0tslot_be`) -> fact cid (base32 bytes)
const INDEX: TableDefinition<&[u8], &[u8]> = TableDefinition::new("emem.canonical_index");
/// fact cid (base32 bytes) -> canonical CBOR of the fact
const FACTS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("emem.facts");
/// backfill bookkeeping
const META: TableDefinition<&str, &[u8]> = TableDefinition::new("emem.meta");

/// Byte-to-byte tables that used to be sled trees.
///
/// Facts moved to redb in the cutover and the boot got no faster, because the
/// sled heap the server still opened was 43.7 GB of `emem.fact_proofs` (20M
/// rows) and `emem.multi_attester_index` (2.4M): measured 2026-09-14, and
/// the cold open is page-cache faulting on that file, ~21 minutes. These
/// three tables take the remaining bulk out of sled; the small live trees
/// (attesters, trace gate, entities) stay, and once the two backfills report
/// done the sled store can be slimmed to a few hundred thousand rows.
const PROOFS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("emem.fact_proofs");
const MULTI: TableDefinition<&[u8], &[u8]> = TableDefinition::new("emem.multi_attester_index");
/// `state_cid -> canonical CBOR of the StateRecord`. Written when an answer
/// emits its reasoning states, so `GET /v1/state/<cid>` can return the bytes
/// the address commits to. Content-addressed, so a repeat is a no-op.
const STATES: TableDefinition<&[u8], &[u8]> = TableDefinition::new("emem.state_records");

/// Which byte-to-byte table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KvTable {
    Proofs,
    Multi,
    States,
}

impl KvTable {
    fn def(self) -> TableDefinition<'static, &'static [u8], &'static [u8]> {
        match self {
            KvTable::Proofs => PROOFS,
            KvTable::Multi => MULTI,
            KvTable::States => STATES,
        }
    }
    /// The sled tree this table replaces, for the backfill. States never
    /// lived in sled.
    pub fn sled_tree(self) -> Option<&'static str> {
        match self {
            KvTable::Proofs => Some("emem.fact_proofs"),
            KvTable::Multi => Some("emem.multi_attester_index"),
            KvTable::States => None,
        }
    }
    fn table_name(self) -> &'static str {
        match self {
            KvTable::Proofs => "emem.fact_proofs",
            KvTable::Multi => "emem.multi_attester_index",
            KvTable::States => "emem.state_records",
        }
    }
    fn meta_done(self) -> String {
        format!("backfill_done:{}", self.table_name())
    }
    fn meta_cursor(self) -> String {
        format!("backfill_cursor:{}", self.table_name())
    }
}
const META_CURSOR: &str = "backfill_cursor";
const META_DONE: &str = "backfill_done";

/// (fact cid bytes, canonical CBOR, canonical key bytes if the fact has one)
pub type FactRow = (Vec<u8>, Vec<u8>, Option<Vec<u8>>);
/// index rows as (key bytes, cid bytes)
pub type IndexRows = Vec<(Vec<u8>, Vec<u8>)>;
/// Raw rows of a byte-to-byte table.
pub type KvRows = Vec<(Vec<u8>, Vec<u8>)>;

fn rb<E: std::fmt::Display>(e: E) -> CacheError {
    CacheError::Backend(e.to_string())
}

pub struct RedbFacts {
    /// `None` after `close()`. redb writes its "shut down cleanly" header in
    /// `Database::drop`, and this process ends in `std::process::exit`, which
    /// skips destructors: every boot since the redb cutover therefore opened a
    /// database still marked `recovery_required` and paid a full
    /// `rebuild_allocator_state` walk of the file. Holding the handle in an
    /// Option is what lets the shutdown path actually drop it.
    db: std::sync::RwLock<Option<Database>>,
    path: PathBuf,
    /// Set once the sled trees have been copied in full; after that no read
    /// consults sled.
    pub(crate) backfill_done: AtomicBool,
    pub(crate) backfilled: AtomicU64,
    /// Set when redb told us, through the repair callback, that it had to
    /// rebuild the allocator state. That happens only when the previous
    /// process did not drop the `Database`.
    repaired: AtomicBool,
    /// Write transactions currently alive, counted by [`WriteTxn`].
    ///
    /// `Database::drop` does NOT close the database when a write transaction
    /// is live: it parks a deferred close on the transaction tracker, returns
    /// instantly, and the close runs when the last transaction guard drops.
    /// A shutdown that drops the handle and then calls `std::process::exit`
    /// lands in that gap, so the file stays marked `recovery_required` and the
    /// next open walks it. Nothing in redb's API reports the deferral, and the
    /// symptom is a boot tens of minutes long an hour later. This counter is
    /// the only way the shutdown path can tell the two outcomes apart.
    live_writes: AtomicUsize,
}

/// A redb write transaction that counts itself while it lives.
///
/// Dereferences to [`redb::WriteTransaction`], so `set_durability` and
/// `open_table` read exactly as before; `commit` is inherent and consuming,
/// which shadows the inner one. The count drops after the transaction is
/// dropped, never before, so a zero means redb's tracker has released it and
/// any deferred close has already run.
pub struct WriteTxn<'a> {
    txn: Option<redb::WriteTransaction>,
    live: &'a AtomicUsize,
}

impl std::ops::Deref for WriteTxn<'_> {
    type Target = redb::WriteTransaction;
    fn deref(&self) -> &Self::Target {
        self.txn
            .as_ref()
            .expect("write transaction taken only by commit")
    }
}

impl std::ops::DerefMut for WriteTxn<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.txn
            .as_mut()
            .expect("write transaction taken only by commit")
    }
}

impl WriteTxn<'_> {
    fn commit(mut self) -> Result<(), redb::CommitError> {
        let txn = self
            .txn
            .take()
            .expect("write transaction taken only by commit");
        txn.commit()
    }
}

impl Drop for WriteTxn<'_> {
    fn drop(&mut self) {
        // Order matters: the transaction goes first, because dropping it is
        // what runs redb's deferred close. Decrementing before that would
        // publish a zero while the close had not happened.
        drop(self.txn.take());
        self.live.fetch_sub(1, Ordering::AcqRel);
    }
}

impl RedbFacts {
    /// `EMEM_REDB_CACHE_BYTES`: redb's read cache. Default 2 GiB.
    fn cache_bytes() -> usize {
        std::env::var("EMEM_REDB_CACHE_BYTES")
            .ok()
            .and_then(|v| crate::sled_hot::parse_bytes(v.trim()))
            .unwrap_or(2 << 30)
            .clamp(64 << 20, 32 << 30) as usize
    }

    /// Timed in three parts, because the whole open is minutes on a large file
    /// and "storage_open took 34 minutes" names no cause. On 2026-09-14 the
    /// responder spent 2,051,951 ms here and in the sled open together, with
    /// no way to tell which, while a standalone run of emem-sled-slim put ~30
    /// minutes before its first redb line and then opened sled in 28.2 s. The
    /// two stores total ~95 GB against a 61 GB page cache, so each open evicts
    /// the other and the attribution flips with whatever ran last. These three
    /// numbers end the guessing: file open, the table-creating commit, and the
    /// meta read are separately reported.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, CacheError> {
        let path = path.as_ref().to_path_buf();
        let repaired = std::sync::Arc::new(AtomicBool::new(false));
        let t0 = std::time::Instant::now();
        // A repair says so, with progress. Before this, a boot that was
        // rebuilding the allocator state looked identical to a hang: no line
        // between "opening persistent storage" and the store being ready,
        // tens of minutes apart, which is how the cost got blamed on page-cache
        // faulting for weeks.
        // redb calls the repair callback when IT decides to, and on a file this
        // size that was TWICE in fifty minutes: 0.0 at the start and 0.6 at
        // minute forty-nine. A line every forty-nine minutes does not
        // distinguish a repair from a hang, which is the thing the callback was
        // added for, so the callback is paired with a heartbeat that says how
        // long this has been running and what the last value redb gave was.
        // The heartbeat never computes a progress of its own: two generators
        // must not own one number, and an interpolated percentage on a walk
        // whose rate is unknown is a number that would be believed.
        let last_progress = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let stop_beat = std::sync::Arc::new(AtomicBool::new(false));
        let beat = std::thread::spawn({
            let repairing = repaired.clone();
            let progress = last_progress.clone();
            let stop = stop_beat.clone();
            let every = std::time::Duration::from_secs(
                std::env::var("EMEM_REDB_REPAIR_HEARTBEAT_S")
                    .ok()
                    .and_then(|v| v.trim().parse::<u64>().ok())
                    .unwrap_or(60)
                    .clamp(5, 3600),
            );
            move || {
                let mut since = std::time::Instant::now();
                while !stop.load(Ordering::Relaxed) {
                    std::thread::sleep(std::time::Duration::from_millis(250));
                    if since.elapsed() < every || !repairing.load(Ordering::Relaxed) {
                        continue;
                    }
                    since = std::time::Instant::now();
                    tracing::warn!(
                        target: "emem::boot",
                        elapsed_s = t0.elapsed().as_secs(),
                        last_progress_redb_reported =
                            progress.load(Ordering::Relaxed) as f64 / 1000.0,
                        "still rebuilding the allocator state; this is work, not a hang"
                    );
                }
            }
        });
        let db = Database::builder()
            .set_cache_size(Self::cache_bytes())
            .set_repair_callback({
                let seen = repaired.clone();
                let progress = last_progress.clone();
                move |s| {
                    seen.store(true, Ordering::Relaxed);
                    progress.store((s.progress() * 1000.0) as u64, Ordering::Relaxed);
                    tracing::warn!(
                        target: "emem::boot",
                        progress = s.progress(),
                        elapsed_s = t0.elapsed().as_secs(),
                        "redb was NOT closed cleanly; rebuilding the allocator state (this walks the file)"
                    );
                }
            })
            .create(&path);
        stop_beat.store(true, Ordering::Relaxed);
        let _ = beat.join();
        let db = db.map_err(rb)?;
        let create_ms = t0.elapsed().as_millis();
        let t1 = std::time::Instant::now();
        {
            let w = db.begin_write().map_err(rb)?;
            w.open_table(INDEX).map_err(rb)?;
            w.open_table(FACTS).map_err(rb)?;
            w.open_table(META).map_err(rb)?;
            w.open_table(PROOFS).map_err(rb)?;
            w.open_table(MULTI).map_err(rb)?;
            w.open_table(STATES).map_err(rb)?;
            w.commit().map_err(rb)?;
        }
        let tables_ms = t1.elapsed().as_millis();
        let t2 = std::time::Instant::now();
        let done = {
            let r = db.begin_read().map_err(rb)?;
            let m = r.open_table(META).map_err(rb)?;
            let hit = m.get(META_DONE).map_err(rb)?;
            hit.is_some()
        };
        tracing::info!(
            target: "emem::boot",
            path = %path.display(),
            bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
            create_ms,
            tables_ms,
            meta_ms = t2.elapsed().as_millis(),
            "redb open"
        );
        Ok(Self {
            db: std::sync::RwLock::new(Some(db)),
            path,
            live_writes: AtomicUsize::new(0),
            backfill_done: AtomicBool::new(done),
            backfilled: AtomicU64::new(0),
            repaired: AtomicBool::new(repaired.load(Ordering::Relaxed)),
        })
    }

    /// A read transaction, or the error a closed store owes its caller.
    fn begin_read(&self) -> Result<redb::ReadTransaction, CacheError> {
        let g = self.db.read().map_err(|_| Self::poisoned())?;
        let db = g.as_ref().ok_or_else(Self::closed)?;
        db.begin_read().map_err(rb)
    }

    /// A write transaction. Transactions hold their own `Arc`s and do not
    /// borrow the `Database`, so the guard is released as this returns.
    fn begin_write(&self) -> Result<WriteTxn<'_>, CacheError> {
        let g = self.db.read().map_err(|_| Self::poisoned())?;
        let db = g.as_ref().ok_or_else(Self::closed)?;
        let txn = db.begin_write().map_err(rb)?;
        self.live_writes.fetch_add(1, Ordering::AcqRel);
        Ok(WriteTxn {
            txn: Some(txn),
            live: &self.live_writes,
        })
    }

    fn closed() -> CacheError {
        CacheError::Backend("the redb store is closed; the process is shutting down".into())
    }

    fn poisoned() -> CacheError {
        CacheError::Backend("the redb store lock is poisoned".into())
    }

    /// Drop the database so redb writes its clean-shutdown header, then wait
    /// for any write transaction that deferred the close.
    ///
    /// Without the drop the next open finds `recovery_required` set and
    /// rebuilds the allocator state by walking the file: 29 s for a 43.8 GB
    /// sled store against tens of minutes for a 56 GB redb, measured on the
    /// 2026-09-14 boots.
    ///
    /// The drop alone is not enough, and the gap is exactly the case that
    /// matters. `Database::drop` checks for a live write transaction and, if
    /// there is one, parks a deferred close and returns in microseconds. The
    /// old version of this function logged "closed cleanly" on that path too,
    /// because it had nothing to check. On 2026-09-15 the watchdog restarted a
    /// responder whose storage flush had been stuck for three consecutive
    /// checks -- the one state in which a write transaction is certainly live
    /// -- the shutdown logged a clean close in 0 ms, and the next boot spent
    /// the afternoon walking 56 GB. A quiet deploy closes cleanly; the restart
    /// that needs a fast boot never does.
    ///
    /// So: drop, then wait for [`Self::live_writes`] to reach zero, which is
    /// the last transaction guard dropping and therefore the deferred close
    /// running. `EMEM_REDB_CLOSE_WAIT_S` bounds the wait, default 20 s.
    /// Whichever way it ends, the log says which one happened.
    ///
    /// Idempotent, and safe to call while other handles still exist: they will
    /// get `closed()` rather than a panic.
    pub fn close(&self) {
        self.close_within(Self::close_wait())
    }

    /// The env-configured ceiling on the close wait, for a caller that needs
    /// to take the minimum of it and its own remaining budget.
    pub fn close_wait_default() -> std::time::Duration {
        Self::close_wait()
    }

    fn close_wait() -> std::time::Duration {
        std::time::Duration::from_secs(
            std::env::var("EMEM_REDB_CLOSE_WAIT_S")
                .ok()
                .and_then(|v| v.trim().parse::<u64>().ok())
                .unwrap_or(20)
                .clamp(0, 300),
        )
    }

    /// [`Self::close`] with the wait supplied rather than read from the
    /// environment.
    ///
    /// The shutdown path uses this to bound the wait by what is LEFT of the
    /// container's stop grace. `ExecStop=docker stop -t 60` sends SIGKILL 60 s
    /// after SIGTERM, and a kill at second 61 undoes the whole point of
    /// waiting: the deferred close never runs and the next open walks the
    /// file. The drain and the sled flush spend that budget first, so the
    /// figure that matters is the remainder, not a constant.
    pub fn close_with_budget(&self, budget: std::time::Duration) {
        self.close_within(budget)
    }

    fn close_within(&self, budget: std::time::Duration) {
        let taken = match self.db.write() {
            Ok(mut g) => g.take(),
            Err(e) => e.into_inner().take(),
        };
        if taken.is_none() {
            return;
        }
        let t = std::time::Instant::now();
        let at_drop = self.live_writes.load(Ordering::Acquire);
        drop(taken);
        while self.live_writes.load(Ordering::Acquire) > 0 && t.elapsed() < budget {
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        let left = self.live_writes.load(Ordering::Acquire);
        if left == 0 {
            tracing::info!(
                target: "emem::boot",
                elapsed_ms = t.elapsed().as_millis(),
                writes_live_at_drop = at_drop,
                "redb closed cleanly; the next open skips the repair walk"
            );
        } else {
            tracing::error!(
                target: "emem::boot",
                elapsed_ms = t.elapsed().as_millis(),
                writes_live_at_drop = at_drop,
                writes_still_live = left,
                "redb close was deferred and the write transactions did not finish; the file stays marked for recovery and the next open rebuilds the allocator state by walking it"
            );
        }
    }

    /// True when this open had to rebuild the allocator state, meaning the
    /// previous process left the database dirty.
    pub fn repaired(&self) -> bool {
        self.repaired.load(Ordering::Relaxed)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn backfill_done(&self) -> bool {
        self.backfill_done.load(Ordering::Acquire)
    }

    /// Write facts (and their index entries) in one transaction. `durable`
    /// makes the commit an fsync; the backfill batches without it and
    /// closes with one durable commit.
    pub fn put_batch(&self, items: &[FactRow], durable: bool) -> Result<(), CacheError> {
        let mut w = self.begin_write()?;
        w.set_durability(if durable {
            Durability::Immediate
        } else {
            Durability::None
        })
        .map_err(rb)?;
        {
            let mut f = w.open_table(FACTS).map_err(rb)?;
            let mut i = w.open_table(INDEX).map_err(rb)?;
            for (cid, cbor, key) in items {
                f.insert(cid.as_slice(), cbor.as_slice()).map_err(rb)?;
                if let Some(k) = key {
                    i.insert(k.as_slice(), cid.as_slice()).map_err(rb)?;
                }
            }
        }
        w.commit().map_err(rb)?;
        Ok(())
    }

    /// An fsync with nothing new in it: closes a run of non-durable batches.
    // ── byte-to-byte tables ─────────────────────────────────────────────
    pub fn kv_get(&self, t: KvTable, key: &[u8]) -> Result<Option<Vec<u8>>, CacheError> {
        let r = self.begin_read()?;
        let tb = r.open_table(t.def()).map_err(rb)?;
        Ok(tb.get(key).map_err(rb)?.map(|v| v.value().to_vec()))
    }

    pub fn kv_put_batch(
        &self,
        t: KvTable,
        items: &[(Vec<u8>, Vec<u8>)],
        durable: bool,
    ) -> Result<(), CacheError> {
        if items.is_empty() {
            return Ok(());
        }
        let mut w = self.begin_write()?;
        w.set_durability(if durable {
            Durability::Immediate
        } else {
            Durability::None
        })
        .map_err(rb)?;
        {
            let mut tb = w.open_table(t.def()).map_err(rb)?;
            for (k, v) in items {
                tb.insert(k.as_slice(), v.as_slice()).map_err(rb)?;
            }
        }
        w.commit().map_err(rb)?;
        Ok(())
    }

    /// Rows whose key starts with `prefix`, key order, at most `limit`.
    pub fn kv_scan_prefix(
        &self,
        t: KvTable,
        prefix: &[u8],
        limit: usize,
    ) -> Result<KvRows, CacheError> {
        let r = self.begin_read()?;
        let tb = r.open_table(t.def()).map_err(rb)?;
        let mut out = Vec::new();
        for item in tb.range(prefix..).map_err(rb)? {
            let (k, v) = item.map_err(rb)?;
            if !k.value().starts_with(prefix) || out.len() >= limit {
                break;
            }
            out.push((k.value().to_vec(), v.value().to_vec()));
        }
        Ok(out)
    }

    pub fn kv_len(&self, t: KvTable) -> Result<u64, CacheError> {
        let r = self.begin_read()?;
        r.open_table(t.def()).map_err(rb)?.len().map_err(rb)
    }

    pub fn table_backfill_done(&self, t: KvTable) -> bool {
        let Ok(r) = self.begin_read() else {
            return false;
        };
        let Ok(m) = r.open_table(META) else {
            return false;
        };
        matches!(m.get(t.meta_done().as_str()), Ok(Some(_)))
    }

    pub fn mark_table_backfill_done(&self, t: KvTable) -> Result<(), CacheError> {
        let mut w = self.begin_write()?;
        w.set_durability(Durability::Immediate).map_err(rb)?;
        {
            let mut m = w.open_table(META).map_err(rb)?;
            m.insert(t.meta_done().as_str(), b"1".as_slice())
                .map_err(rb)?;
        }
        w.commit().map_err(rb)?;
        Ok(())
    }

    pub fn table_cursor(&self, t: KvTable) -> Result<Option<Vec<u8>>, CacheError> {
        let r = self.begin_read()?;
        let m = r.open_table(META).map_err(rb)?;
        Ok(m.get(t.meta_cursor().as_str())
            .map_err(rb)?
            .map(|v| v.value().to_vec()))
    }

    pub fn set_table_cursor(&self, t: KvTable, cursor: &[u8]) -> Result<(), CacheError> {
        let mut w = self.begin_write()?;
        w.set_durability(Durability::None).map_err(rb)?;
        {
            let mut m = w.open_table(META).map_err(rb)?;
            m.insert(t.meta_cursor().as_str(), cursor).map_err(rb)?;
        }
        w.commit().map_err(rb)?;
        Ok(())
    }

    pub fn sync(&self) -> Result<(), CacheError> {
        let mut w = self.begin_write()?;
        w.set_durability(Durability::Immediate).map_err(rb)?;
        w.commit().map_err(rb)?;
        Ok(())
    }

    pub fn get_fact(&self, cid: &[u8]) -> Result<Option<Vec<u8>>, CacheError> {
        let r = self.begin_read()?;
        let t = r.open_table(FACTS).map_err(rb)?;
        Ok(t.get(cid).map_err(rb)?.map(|g| g.value().to_vec()))
    }

    pub fn contains_fact(&self, cid: &[u8]) -> Result<bool, CacheError> {
        let r = self.begin_read()?;
        let t = r.open_table(FACTS).map_err(rb)?;
        Ok(t.get(cid).map_err(rb)?.is_some())
    }

    pub fn lookup(&self, key: &[u8]) -> Result<Option<Vec<u8>>, CacheError> {
        let r = self.begin_read()?;
        let t = r.open_table(INDEX).map_err(rb)?;
        Ok(t.get(key).map_err(rb)?.map(|g| g.value().to_vec()))
    }

    pub fn contains_index(&self, key: &[u8]) -> Result<bool, CacheError> {
        let r = self.begin_read()?;
        let t = r.open_table(INDEX).map_err(rb)?;
        Ok(t.get(key).map_err(rb)?.is_some())
    }

    /// Index entries whose key starts with `prefix`, in key order, at most
    /// `limit`. Returns the rows and how many were seen (the callers log a
    /// hit limit the way the sled scan did).
    pub fn scan_prefix(
        &self,
        prefix: &[u8],
        limit: usize,
    ) -> Result<(IndexRows, usize), CacheError> {
        let r = self.begin_read()?;
        let t = r.open_table(INDEX).map_err(rb)?;
        let mut out = Vec::new();
        let mut seen = 0usize;
        for item in t.range(prefix..).map_err(rb)? {
            let (k, v) = item.map_err(rb)?;
            let kb = k.value();
            if !kb.starts_with(prefix) {
                break;
            }
            seen += 1;
            if out.len() >= limit {
                break;
            }
            out.push((kb.to_vec(), v.value().to_vec()));
        }
        Ok((out, seen))
    }

    /// One page of the index in key order, strictly after `after`.
    pub fn index_page(&self, after: Option<&[u8]>, max: usize) -> Result<IndexRows, CacheError> {
        let r = self.begin_read()?;
        let t = r.open_table(INDEX).map_err(rb)?;
        let mut out = Vec::with_capacity(max.min(4096));
        let iter = match after {
            Some(a) => t.range(a..).map_err(rb)?,
            None => t.range::<&[u8]>(..).map_err(rb)?,
        };
        for item in iter {
            let (k, v) = item.map_err(rb)?;
            if let Some(a) = after {
                if k.value() == a {
                    continue;
                }
            }
            out.push((k.value().to_vec(), v.value().to_vec()));
            if out.len() >= max {
                break;
            }
        }
        Ok(out)
    }

    pub fn index_len(&self) -> Result<u64, CacheError> {
        let r = self.begin_read()?;
        let t = r.open_table(INDEX).map_err(rb)?;
        t.len().map_err(rb)
    }

    pub fn size_on_disk(&self) -> u64 {
        std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0)
    }

    pub fn backfill_cursor(&self) -> Result<Option<Vec<u8>>, CacheError> {
        let r = self.begin_read()?;
        let m = r.open_table(META).map_err(rb)?;
        Ok(m.get(META_CURSOR).map_err(rb)?.map(|g| g.value().to_vec()))
    }

    pub fn set_backfill_cursor(&self, cursor: &[u8], durable: bool) -> Result<(), CacheError> {
        let mut w = self.begin_write()?;
        w.set_durability(if durable {
            Durability::Immediate
        } else {
            Durability::None
        })
        .map_err(rb)?;
        {
            let mut m = w.open_table(META).map_err(rb)?;
            m.insert(META_CURSOR, cursor).map_err(rb)?;
        }
        w.commit().map_err(rb)?;
        Ok(())
    }

    pub fn mark_backfill_done(&self) -> Result<(), CacheError> {
        let mut w = self.begin_write()?;
        w.set_durability(Durability::Immediate).map_err(rb)?;
        {
            let mut m = w.open_table(META).map_err(rb)?;
            m.insert(META_DONE, &b"1"[..]).map_err(rb)?;
        }
        w.commit().map_err(rb)?;
        self.backfill_done.store(true, Ordering::Release);
        Ok(())
    }
}

#[cfg(test)]
mod close_tests {
    use super::*;

    /// A store that was closed reopens without a repair, and a closed handle
    /// refuses rather than panics.
    ///
    /// Only the clean arm runs in-process. The dirty arm is what
    /// `std::process::exit` does, and simulating it with `std::mem::forget`
    /// would keep redb's file lock held, so the reopen inside the same test
    /// could not happen at all. The dirty behaviour is the one measured in
    /// production on 2026-09-14 and the reason this exists.
    #[test]
    fn a_closed_store_reopens_without_a_repair() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("facts.redb");

        let r = RedbFacts::open(&path).unwrap();
        // The control. A brand-new file goes through the same rebuild path,
        // so this is `true`, and that is what makes the flag observable here at
        // all: a test where both arms read `false` would prove nothing.
        assert!(r.repaired(), "a fresh create is initialised through repair");
        r.kv_put_batch(KvTable::Proofs, &[(b"k".to_vec(), b"v".to_vec())], true)
            .unwrap();
        r.close();

        // Every operation after close reports it; nothing unwraps a None.
        let err = r.kv_get(KvTable::Proofs, b"k").unwrap_err().to_string();
        assert!(err.contains("closed"), "{err}");
        r.close(); // idempotent

        let again = RedbFacts::open(&path).unwrap();
        // The property under test: after an explicit close, the reopen does
        // NOT rebuild the allocator state. In production that walk is the whole
        // boot, tens of minutes on a 53.7 GB file.
        assert!(
            !again.repaired(),
            "a cleanly closed database must not rebuild its allocator state"
        );
        assert_eq!(
            again.kv_get(KvTable::Proofs, b"k").unwrap().as_deref(),
            Some(&b"v"[..]),
            "the row written before the close survived it"
        );
        again.close();
    }

    /// The case the test above cannot see, and the one production hits.
    ///
    /// `Database::drop` does not close a database with a live write
    /// transaction: it parks a deferred close and returns immediately. The
    /// close then runs when the last transaction guard drops -- which, in the
    /// server, is after `std::process::exit` has already ended the process.
    /// Nothing in redb reports the deferral, so before the counter the
    /// shutdown logged "closed cleanly" in 0 ms and the next boot walked
    /// 56 GB anyway.
    #[test]
    fn a_live_write_defers_the_close_and_the_counter_is_what_notices() {
        let dir = tempfile::tempdir().unwrap();
        let r = RedbFacts::open(dir.path().join("facts.redb")).unwrap();

        let w = r.begin_write().unwrap();
        assert_eq!(r.live_writes.load(Ordering::Acquire), 1);

        // Zero budget: return at once, the way the old close did, and report
        // what is true rather than what is hoped.
        r.close_within(std::time::Duration::ZERO);
        assert_eq!(
            r.live_writes.load(Ordering::Acquire),
            1,
            "the transaction outlived the close, so redb deferred it"
        );

        drop(w);
        assert_eq!(
            r.live_writes.load(Ordering::Acquire),
            0,
            "the guard decrements only after the transaction is dropped"
        );
    }

    /// The wait does its job: a write that finishes just after the shutdown
    /// starts is waited for, and the close ends with nothing outstanding.
    #[test]
    fn the_close_waits_out_a_write_that_finishes_after_it_begins() {
        let dir = tempfile::tempdir().unwrap();
        let r = RedbFacts::open(dir.path().join("facts.redb")).unwrap();
        let t0 = std::time::Instant::now();

        std::thread::scope(|sc| {
            let w = r.begin_write().unwrap();
            sc.spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(250));
                w.commit().unwrap();
            });
            // Give the spawn a moment so the close genuinely starts with the
            // transaction live; without this the test can pass on a path where
            // there was nothing to wait for.
            std::thread::sleep(std::time::Duration::from_millis(20));
            assert_eq!(r.live_writes.load(Ordering::Acquire), 1);
            r.close_within(std::time::Duration::from_secs(5));
        });

        assert_eq!(r.live_writes.load(Ordering::Acquire), 0);
        assert!(
            t0.elapsed() >= std::time::Duration::from_millis(250),
            "the close returned before the write it was waiting for: {:?}",
            t0.elapsed()
        );
    }
}
