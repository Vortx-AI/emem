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

use crate::{Cache, CacheError, CanonicalKey, Tier};
use emem_fact::{Fact, FactCid};

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
fn scan_cell_tree(
    idx: &sled::Tree,
    cell: &str,
    tslot: Option<u64>,
) -> Result<Vec<(CanonicalKey, FactCid)>, CacheError> {
    let limit: usize = std::env::var("EMEM_SCAN_CELL_LIMIT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10_000);
    let mut prefix = Vec::with_capacity(cell.len() + 1);
    prefix.extend_from_slice(cell.as_bytes());
    prefix.push(SEP);
    let mut out = Vec::new();
    let mut seen = 0usize;
    for kv in idx.scan_prefix(&prefix) {
        seen += 1;
        if out.len() >= limit {
            tracing::warn!(
                target: "emem::storage",
                scan_cell = %cell,
                scan_limit = limit,
                scan_seen = seen,
                "scan_cell_limit_hit",
            );
            break;
        }
        let (k, v) = kv?;
        let key = decode_key(&k).map_err(CacheError::Cbor)?;
        if let Some(t) = tslot {
            if key.tslot != t {
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

/// Shared body of [`SledHotCache::scan_cell_with_tslot_bound`].
fn scan_cell_bound_tree(
    idx: &sled::Tree,
    cell: &str,
    tslot_eq: Option<u64>,
    tslot_le: Option<u64>,
) -> Result<Vec<(CanonicalKey, FactCid)>, CacheError> {
    let limit: usize = std::env::var("EMEM_SCAN_CELL_LIMIT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10_000);
    let mut prefix = Vec::with_capacity(cell.len() + 1);
    prefix.extend_from_slice(cell.as_bytes());
    prefix.push(SEP);
    let mut out = Vec::new();
    let mut seen = 0usize;
    for kv in idx.scan_prefix(&prefix) {
        seen += 1;
        if out.len() >= limit {
            tracing::warn!(
                target: "emem::storage",
                scan_cell = %cell,
                scan_limit = limit,
                scan_seen = seen,
                "scan_cell_limit_hit",
            );
            break;
        }
        let (k, v) = kv?;
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

/// Hot tier on top of sled.
pub struct SledHotCache {
    db: sled::Db,
    idx: sled::Tree,
    facts: sled::Tree,
}

impl SledHotCache {
    /// Open or create a sled DB at the given path.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, CacheError> {
        let db = sled::open(path)?;
        let idx = db.open_tree(TREE_INDEX)?;
        let facts = db.open_tree(TREE_FACTS)?;
        Ok(Self { db, idx, facts })
    }

    /// Open an in-memory (temporary) cache. Useful for tests and the
    /// dev server's first-boot bootstrap.
    pub fn open_temporary() -> Result<Self, CacheError> {
        let db = sled::Config::new().temporary(true).open()?;
        let idx = db.open_tree(TREE_INDEX)?;
        let facts = db.open_tree(TREE_FACTS)?;
        Ok(Self { db, idx, facts })
    }

    /// Iterate every (canonical_key, fact_cid) in the index. Used by
    /// primitives like find_similar that need a corpus-wide scan.
    pub fn iter_index(
        &self,
    ) -> impl Iterator<Item = Result<(CanonicalKey, FactCid), CacheError>> + '_ {
        self.idx.iter().map(|kv| {
            let (k, v) = kv?;
            let key = decode_key(&k).map_err(CacheError::Cbor)?;
            let cid_s = std::str::from_utf8(&v)
                .map_err(|e| CacheError::Cbor(e.to_string()))?
                .to_string();
            Ok((key, FactCid::new(cid_s)))
        })
    }

    /// Prefix-scan the index by cell64 (and optional tslot equality filter).
    /// Caps iteration at `EMEM_SCAN_CELL_LIMIT` rows (default 10_000) so a
    /// pathologically dense cell can't tie up a request thread. The cap is
    /// well above any expected legitimate density (a single cell holds one
    /// fact per (band, tslot)); hitting it indicates either an attack or a
    /// schema mistake, both of which we want logged.
    pub fn scan_cell(
        &self,
        cell: &str,
        tslot: Option<u64>,
    ) -> Result<Vec<(CanonicalKey, FactCid)>, CacheError> {
        scan_cell_tree(&self.idx, cell, tslot)
    }

    /// [`SledHotCache::scan_cell`] run on the blocking pool, bounded by the
    /// global sled concurrency limit. This is the form the async recall
    /// path must use: the scan is a blocking sled operation and belongs off
    /// the reactor. Byte-identical result to the synchronous method.
    pub async fn scan_cell_off(
        &self,
        cell: &str,
        tslot: Option<u64>,
    ) -> Result<Vec<(CanonicalKey, FactCid)>, CacheError> {
        let idx = self.idx.clone();
        let cell = cell.to_string();
        off_thread(move || scan_cell_tree(&idx, &cell, tslot)).await
    }

    /// Bi-temporal sibling of [`SledHotCache::scan_cell`]. Pre-filters
    /// on `tslot` directly from the canonical key (decoded inline as
    /// the iterator walks — no CBOR body load required), then on
    /// `valid_time` (also a key-level predicate). This keeps the cold
    /// path index-bound when the caller only constrains valid-time —
    /// fact bodies are loaded only for entries that survived the
    /// valid-time filter, and only when the caller additionally pinned
    /// transaction-time. Returns the (key, fact_cid) pairs that survive
    /// both predicates; transaction-time filtering is left to the
    /// caller because resolving `signed_at` requires the body and the
    /// storage layer is the canonical loader.
    pub fn scan_cell_with_tslot_bound(
        &self,
        cell: &str,
        tslot_eq: Option<u64>,
        tslot_le: Option<u64>,
    ) -> Result<Vec<(CanonicalKey, FactCid)>, CacheError> {
        scan_cell_bound_tree(&self.idx, cell, tslot_eq, tslot_le)
    }

    /// [`SledHotCache::scan_cell_with_tslot_bound`] on the blocking pool,
    /// bounded by the global sled concurrency limit — the form the async
    /// as-of recall path must use.
    pub async fn scan_cell_with_tslot_bound_off(
        &self,
        cell: &str,
        tslot_eq: Option<u64>,
        tslot_le: Option<u64>,
    ) -> Result<Vec<(CanonicalKey, FactCid)>, CacheError> {
        let idx = self.idx.clone();
        let cell = cell.to_string();
        off_thread(move || scan_cell_bound_tree(&idx, &cell, tslot_eq, tslot_le)).await
    }

    /// Collect up to `limit` index entries on the blocking pool, bounded by
    /// the global sled concurrency limit. A full-corpus `iter_index` is a
    /// heavy blocking scan (find_similar, lance hydration); this keeps it
    /// off the async reactor. Same decoding as [`SledHotCache::iter_index`].
    pub async fn collect_index_off(
        &self,
        limit: Option<usize>,
    ) -> Result<Vec<(CanonicalKey, FactCid)>, CacheError> {
        let idx = self.idx.clone();
        off_thread(move || {
            let mut out = Vec::new();
            for kv in idx.iter() {
                let (k, v) = kv?;
                let key = decode_key(&k).map_err(CacheError::Cbor)?;
                let cid_s = std::str::from_utf8(&v)
                    .map_err(|e| CacheError::Cbor(e.to_string()))?
                    .to_string();
                out.push((key, FactCid::new(cid_s)));
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

    /// Approximate item count across the index tree.
    pub fn len(&self) -> usize {
        self.idx.len()
    }
    /// Whether the index has zero entries.
    pub fn is_empty(&self) -> bool {
        self.idx.is_empty()
    }
    /// Total bytes across both trees on disk (sled estimate).
    /// Borrow the underlying sled DB so callers (e.g., the attester
    /// reputation tracker) can open additional named trees alongside the
    /// canonical index + facts trees without re-opening the file.
    pub fn db(&self) -> &sled::Db {
        &self.db
    }

    pub fn size_on_disk(&self) -> Result<u64, CacheError> {
        Ok(self.db.size_on_disk()?)
    }
}

/// Compute the deterministic FactCid for a fact: base32-nopad-lowercase of
/// blake3(canonical_cbor(fact)). Always 52 chars (256 bits).
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
    // Every method below runs its sled work through `off_thread`: the sled
    // reads/writes are blocking and must not execute on the async workers
    // (see `sled_blocking_sem`). The sled `Tree` handles clone cheaply
    // (Arc-backed) and the inputs are copied into the closure so it owns
    // everything and stays `'static`.
    async fn lookup_many(&self, keys: &[CanonicalKey]) -> Result<Vec<Option<FactCid>>, CacheError> {
        let idx = self.idx.clone();
        let keys = keys.to_vec();
        off_thread(move || {
            let mut out = Vec::with_capacity(keys.len());
            for k in &keys {
                let kb = encode_key(k);
                match idx.get(&kb)? {
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
        let cids = cids.to_vec();
        off_thread(move || {
            let mut out = Vec::with_capacity(cids.len());
            for cid in &cids {
                match facts.get(cid.as_str().as_bytes())? {
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
        let facts_in = facts.to_vec();
        // The inserts are blocking sled writes → off the reactor. sled
        // buffers them in its log; the durability fsync is the async
        // `flush_async` below, which already yields.
        let out = off_thread(move || {
            let mut out = Vec::with_capacity(facts_in.len());
            for f in &facts_in {
                let cbor = fact_to_cbor(f)?;
                let mut h = Hasher::new();
                h.update(&cbor);
                let hash = h.finalize();
                let cid_s = BASE32_NOPAD.encode(hash.as_bytes()).to_lowercase();
                let cid = FactCid::new(cid_s);
                facts_tree.insert(cid.as_str().as_bytes(), cbor)?;
                if let Some(k) = fact_canonical_key(f) {
                    idx.insert(encode_key(&k), cid.as_str().as_bytes())?;
                }
                out.push(cid);
            }
            Ok(out)
        })
        .await?;
        // sled flushes at the Db level: one fsync of the shared log
        // persists writes to every tree. The insert loop above has already
        // committed to `facts` and `idx`, so a single flush here makes all
        // of them durable — flushing both trees separately just paid for
        // the fsync twice on every write batch.
        self.facts
            .flush_async()
            .await
            .map_err(|e| CacheError::Cbor(e.to_string()))?;
        Ok(out)
    }

    async fn tier_of(&self, cid: &FactCid) -> Result<Option<Tier>, CacheError> {
        let facts = self.facts.clone();
        let cid = cid.clone();
        off_thread(move || {
            Ok(if facts.contains_key(cid.as_str().as_bytes())? {
                Some(Tier::Hot)
            } else {
                None
            })
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
