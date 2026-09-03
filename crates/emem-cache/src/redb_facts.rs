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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// canonical key bytes (`cell\0band\0tslot_be`) -> fact cid (base32 bytes)
const INDEX: TableDefinition<&[u8], &[u8]> = TableDefinition::new("emem.canonical_index");
/// fact cid (base32 bytes) -> canonical CBOR of the fact
const FACTS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("emem.facts");
/// backfill bookkeeping
const META: TableDefinition<&str, &[u8]> = TableDefinition::new("emem.meta");
const META_CURSOR: &str = "backfill_cursor";
const META_DONE: &str = "backfill_done";

/// (fact cid bytes, canonical CBOR, canonical key bytes if the fact has one)
pub type FactRow = (Vec<u8>, Vec<u8>, Option<Vec<u8>>);
/// index rows as (key bytes, cid bytes)
pub type IndexRows = Vec<(Vec<u8>, Vec<u8>)>;

fn rb<E: std::fmt::Display>(e: E) -> CacheError {
    CacheError::Backend(e.to_string())
}

pub struct RedbFacts {
    db: Database,
    path: PathBuf,
    /// Set once the sled trees have been copied in full; after that no read
    /// consults sled.
    pub(crate) backfill_done: AtomicBool,
    pub(crate) backfilled: AtomicU64,
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

    pub fn open(path: impl AsRef<Path>) -> Result<Self, CacheError> {
        let path = path.as_ref().to_path_buf();
        let db = Database::builder()
            .set_cache_size(Self::cache_bytes())
            .create(&path)
            .map_err(rb)?;
        {
            let w = db.begin_write().map_err(rb)?;
            w.open_table(INDEX).map_err(rb)?;
            w.open_table(FACTS).map_err(rb)?;
            w.open_table(META).map_err(rb)?;
            w.commit().map_err(rb)?;
        }
        let done = {
            let r = db.begin_read().map_err(rb)?;
            let m = r.open_table(META).map_err(rb)?;
            let hit = m.get(META_DONE).map_err(rb)?;
            hit.is_some()
        };
        Ok(Self {
            db,
            path,
            backfill_done: AtomicBool::new(done),
            backfilled: AtomicU64::new(0),
        })
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
        let mut w = self.db.begin_write().map_err(rb)?;
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
    pub fn sync(&self) -> Result<(), CacheError> {
        let mut w = self.db.begin_write().map_err(rb)?;
        w.set_durability(Durability::Immediate).map_err(rb)?;
        w.commit().map_err(rb)?;
        Ok(())
    }

    pub fn get_fact(&self, cid: &[u8]) -> Result<Option<Vec<u8>>, CacheError> {
        let r = self.db.begin_read().map_err(rb)?;
        let t = r.open_table(FACTS).map_err(rb)?;
        Ok(t.get(cid).map_err(rb)?.map(|g| g.value().to_vec()))
    }

    pub fn contains_fact(&self, cid: &[u8]) -> Result<bool, CacheError> {
        let r = self.db.begin_read().map_err(rb)?;
        let t = r.open_table(FACTS).map_err(rb)?;
        Ok(t.get(cid).map_err(rb)?.is_some())
    }

    pub fn lookup(&self, key: &[u8]) -> Result<Option<Vec<u8>>, CacheError> {
        let r = self.db.begin_read().map_err(rb)?;
        let t = r.open_table(INDEX).map_err(rb)?;
        Ok(t.get(key).map_err(rb)?.map(|g| g.value().to_vec()))
    }

    pub fn contains_index(&self, key: &[u8]) -> Result<bool, CacheError> {
        let r = self.db.begin_read().map_err(rb)?;
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
        let r = self.db.begin_read().map_err(rb)?;
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
        let r = self.db.begin_read().map_err(rb)?;
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
        let r = self.db.begin_read().map_err(rb)?;
        let t = r.open_table(INDEX).map_err(rb)?;
        t.len().map_err(rb)
    }

    pub fn size_on_disk(&self) -> u64 {
        std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0)
    }

    pub fn backfill_cursor(&self) -> Result<Option<Vec<u8>>, CacheError> {
        let r = self.db.begin_read().map_err(rb)?;
        let m = r.open_table(META).map_err(rb)?;
        Ok(m.get(META_CURSOR).map_err(rb)?.map(|g| g.value().to_vec()))
    }

    pub fn set_backfill_cursor(&self, cursor: &[u8], durable: bool) -> Result<(), CacheError> {
        let mut w = self.db.begin_write().map_err(rb)?;
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
        let mut w = self.db.begin_write().map_err(rb)?;
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
