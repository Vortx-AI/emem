//! emem-storage — the composite storage layer.
//!
//! Composes the multi-tier cache with the fetch dispatcher and attestation
//! log into the **lazy materializer**: agent-facing primitives call
//! `Storage::materialize_many(...)` and the layer:
//!
//! 1. Looks up canonical fact CID in cache (`Hot → Warm → Cold`).
//! 2. On hit: returns the cached CID.
//! 3. On miss: looks up the function in the registry, fetches required
//!    upstream sources via `emem-fetch`, computes the band value via the
//!    function executor, attests, writes to the cache + Merkle log,
//!    returns the new CID.
//!
//! Bootstrap is "warm the cache by pre-recalling popular cells" — exactly
//! the same code path as agent recall, just driven by an offline workload.

#![forbid(unsafe_code)]

use std::sync::Arc;

use async_trait::async_trait;
use blake3::Hasher;

use emem_cache::{Cache, CanonicalKey, SledHotCache};
use emem_core::{BandRegistry, ErrorCode, FunctionRegistry, SourceRegistry};
use emem_fact::{Attestation, Fact, FactCid, MerkleProof};
use emem_fetch::Dispatcher;

/// Sled tree storing per-fact merkle inclusion proofs. Populated by
/// [`MaterializingStorage::put_attestation`]; read at receipt-sign time
/// so every cited fact carries the path back to the batch root that
/// signed it. Tree value: canonical CBOR of [`MerkleProof`].
const TREE_FACT_PROOFS: &str = "emem.fact_proofs";

/// Sled tree mapping `(cell, band, tslot)` (encoded the same way as the
/// canonical index: `cell\0band\0tslot_be8`) → canonical CBOR of
/// `Vec<FactCid>`. Every attested Primary / Absence fact that produces a
/// canonical-keyable triple appends its CID to the slot here, even when
/// the canonical index has already been overwritten by a later
/// last-write-wins. This is the substrate the memory-contradictions
/// primitive scans: when two attesters disagree about the same (cell,
/// band, tslot) we want BOTH CIDs to live and be addressable, not just
/// the latest writer's.
pub const TREE_MULTI_ATTESTER_INDEX: &str = "emem.multi_attester_index";

/// Sled tree storing memory-bundle envelopes keyed by `bundle_cid`.
/// Value: canonical CBOR of the bundle response shape (defined in
/// `emem-primitives::memory_bundle`). Lookups serve the
/// `GET /v1/memory_bundle/<token>` and MCP `resources/read` paths.
pub const TREE_MEMORY_BUNDLES: &str = "emem.memory_bundles";

/// Sled tree mapping LLM-managed memory-tool paths (Anthropic
/// `context-management-2025-06-27` spec) to their current file CID.
/// Key: UTF-8 path string (e.g. `/memories/foo.md`). Value: file_cid
/// bytes (base32-nopad-lc).
pub const TREE_MEMORY_FILES: &str = "emem.memory_files";

/// Sled tree mapping `file_cid → bytes`. Content-addressed blob store
/// for the memory-tool surface — independent of the `path → file_cid`
/// index so the same blob shared across paths costs storage once.
pub const TREE_MEMORY_FILE_BLOBS: &str = "emem.memory_file_blobs";

/// Sled tree mapping `path → CBOR(Vec<file_cid>)` — append-only edit
/// history per path. Lets an audit replay every str_replace / insert /
/// create touch in order. Most recent CID at the back.
pub const TREE_MEMORY_FILE_HISTORY: &str = "emem.memory_file_history";

/// Sled tree mapping `file_cid → CBOR(MemoryFileMeta)` — the
/// receipt + sign timestamp for each write so an audit can reconstruct
/// who-signed-what-when without re-signing.
pub const TREE_MEMORY_FILE_META: &str = "emem.memory_file_meta";

/// Sled tree indexing memory files by typed `kind` (episodic / semantic /
/// procedural / resource — see `emem_primitives::memory_typing`). Key:
/// `b"<kind>|<path>"`. Value: file_cid bytes. Powers
/// `memory_list_by_kind` without scanning the global path index.
pub const TREE_MEMORY_FILES_BY_KIND: &str = "emem.memory_files_by_kind";

/// Sled tree mapping expired memory paths (TTL pass moved them out of
/// the live index) to their last-known `file_cid`. Key: `path` bytes.
/// Value: file_cid bytes. Blob retention stays in
/// `TREE_MEMORY_FILE_BLOBS` so expired files are still dereferenceable
/// by CID — only the live path index forgets.
pub const TREE_MEMORY_FILES_EXPIRED: &str = "emem.memory_files_expired";

pub mod attesters;
pub mod merkle_log;
pub mod server;

pub use attesters::{AttesterRegistry, AttesterStats};
pub use merkle_log::{AppendOutcome, AttestationLog, VerifyReport};
pub use server::Server;

/// Bi-temporal filter (Zep / Graphiti edge model, arXiv 2501.13956).
///
/// `valid_time` = the planet-side observation time (a fact's `tslot`).
/// `transaction_time` = the system-side learning time (a fact's
/// `signed_at` ISO-8601 wall clock). A scan with both fields `None` is
/// unbounded and behaves like the historical recall path.
///
/// When both fields are `Some`, both predicates must hold simultaneously
/// (`fact.tslot <= valid_time` AND `fact.signed_at <= transaction_time`).
/// The single-axis cases pin only one side.
#[derive(Debug, Clone, Default)]
pub struct AsOfBound {
    /// Upper bound on a fact's `tslot` (valid-time). When `Some(t)`, only
    /// facts with `tslot <= t` survive the filter.
    pub valid_time: Option<u64>,
    /// Upper bound on a fact's `signed_at` (transaction-time). RFC 3339
    /// string; lexicographic comparison is safe because RFC 3339 / ISO
    /// 8601 strings sort correctly when truncated to the same precision.
    /// The primitive layer parses the caller's input with [`chrono`] /
    /// the time crate to reject malformed strings BEFORE constructing
    /// this bound — by the time a value lands here it has been validated.
    pub transaction_time: Option<String>,
}

impl AsOfBound {
    /// Construct an unbounded bound (no valid-time, no transaction-time
    /// cap). Callers that never expose an `as_of` knob — e.g. polygon
    /// aggregators that always read "latest known" — use this so the
    /// receipt signer can take `&AsOfBound` uniformly without each call
    /// site re-typing the `AsOfBound { valid_time: None, transaction_time:
    /// None }` literal.
    pub fn unbounded() -> Self {
        Self::default()
    }

    /// True when neither bound was set — the historical "latest"
    /// behaviour applies and no `as_of` block is added to the receipt.
    pub fn is_unbounded(&self) -> bool {
        self.valid_time.is_none() && self.transaction_time.is_none()
    }

    /// Whether the fact passes both predicates. `valid_time` is checked
    /// purely on `fact.tslot`; `transaction_time` is checked on the
    /// fact's `signed_at` string via lexicographic comparison, which
    /// is order-correct for normalised RFC 3339 timestamps.
    pub fn fact_passes(&self, fact: &Fact) -> bool {
        let (tslot, signed_at) = match fact {
            Fact::Primary(p) => (p.tslot, p.signed_at.as_str()),
            Fact::Absence(a) => (a.tslot, a.signed_at.as_str()),
            Fact::Derivative(d) => {
                // Derivative facts use a tslot window; treat the upper
                // bound as the effective "valid time" of the derivative
                // so a derivative computed across [t0, t1] is excluded
                // from an as_of query that pre-dates t1.
                (d.tslot_window[1], d.signed_at.as_str())
            }
        };
        if let Some(t) = self.valid_time {
            if tslot > t {
                return false;
            }
        }
        if let Some(tt) = self.transaction_time.as_deref() {
            if signed_at > tt {
                return false;
            }
        }
        true
    }
}

/// The lazy-materialization storage facade. Composes cache + fetch + log.
pub struct MaterializingStorage {
    /// Multi-tier fact cache.
    pub cache: Arc<dyn Cache>,
    /// Optional concrete handle to the hot cache when callers need
    /// scan-style access (find_similar, query_region) that the trait
    /// surface does not expose.
    pub hot: Option<Arc<SledHotCache>>,
    /// Source-fetch dispatcher.
    pub fetch: Dispatcher,
    /// Active band registry.
    pub bands: Arc<BandRegistry>,
    /// Active function registry.
    pub functions: Arc<FunctionRegistry>,
    /// Active sources manifest.
    pub sources: Arc<SourceRegistry>,
    /// Append-only attestation log.
    pub log: Arc<AttestationLog>,
    /// Per-attester reputation registry. Optional — `None` for ephemeral
    /// (in-memory) deploys; populated when storage is `rooted` to disk.
    pub attesters: Option<AttesterRegistry>,
}

/// The protocol-level storage trait. All primitives program against this
/// surface. Async + batch-shaped from day one.
#[async_trait]
pub trait Storage: Send + Sync {
    /// Look up canonical fact CIDs for many keys.
    async fn lookup_canonical_many(
        &self,
        keys: &[CanonicalKey],
    ) -> Result<Vec<Option<FactCid>>, StorageError>;

    /// Fetch many facts by CID.
    async fn get_facts_many(&self, cids: &[FactCid]) -> Result<Vec<Option<Fact>>, StorageError>;

    /// Persist an attestation. Verifies the merkle root + ed25519
    /// signature before committing. Returns CIDs of stored facts.
    async fn put_attestation(&self, att: &Attestation) -> Result<Vec<FactCid>, StorageError>;

    /// Lazy materialization entry point: ensure facts exist for these keys,
    /// fetching + computing + attesting on miss. Returns the resolved CIDs
    /// in the same order as inputs.
    async fn materialize_many(&self, keys: &[CanonicalKey]) -> Result<Vec<FactCid>, StorageError>;

    /// Scan all (canonical_key, fact_cid) pairs whose key shares the given
    /// cell, optionally filtered by tslot. Returned order is index order.
    async fn scan_cell(
        &self,
        cell: &str,
        tslot: Option<u64>,
    ) -> Result<Vec<(CanonicalKey, FactCid)>, StorageError>;

    /// Bi-temporal sibling of [`Storage::scan_cell`]. Pre-filters on
    /// `tslot` directly from the canonical index (key carries the
    /// `tslot` bytes — no body load required for the valid-time half of
    /// the bound), then for any caller-set `transaction_time` it loads
    /// each fact and filters by `signed_at`. Default implementation
    /// delegates to `scan_cell` + `get_facts_many`; backend-specific
    /// implementations may push the valid-time filter into the index
    /// scan for a faster cold path.
    ///
    /// When `bound.is_unbounded()`, this is equivalent to
    /// `scan_cell(cell, tslot)`. When `tslot` is `Some(t)` the exact-
    /// match takes precedence over the bound's `valid_time` ceiling —
    /// the caller is expected to have rejected the conflict at the
    /// surface (`as_of_tslot < tslot`) before reaching the storage
    /// layer.
    async fn scan_cell_as_of(
        &self,
        cell: &str,
        tslot: Option<u64>,
        bound: &AsOfBound,
    ) -> Result<Vec<(CanonicalKey, FactCid)>, StorageError> {
        let mut pairs = self.scan_cell(cell, tslot).await?;
        if bound.is_unbounded() {
            return Ok(pairs);
        }
        // Cheap half: valid-time predicate is fully decidable from the
        // index key — no body load needed.
        if let Some(vt) = bound.valid_time {
            pairs.retain(|(k, _)| k.tslot <= vt);
        }
        if bound.transaction_time.is_none() {
            return Ok(pairs);
        }
        // Expensive half: transaction-time predicate reads `signed_at`
        // off the body. Batch-load every surviving fact so the cost is
        // one get-facts-many round-trip, not N.
        let cids: Vec<FactCid> = pairs.iter().map(|(_, c)| c.clone()).collect();
        let facts = self.get_facts_many(&cids).await?;
        let mut filtered: Vec<(CanonicalKey, FactCid)> = Vec::with_capacity(pairs.len());
        for ((k, c), fact) in pairs.into_iter().zip(facts) {
            if let Some(f) = fact {
                if bound.fact_passes(&f) {
                    filtered.push((k, c));
                }
            }
        }
        Ok(filtered)
    }

    /// Iterate every (canonical_key, fact_cid) in the index. Used by
    /// corpus-wide scans (find_similar). Bounded by the optional `limit`
    /// to keep responses tractable.
    async fn iter_index(
        &self,
        limit: Option<usize>,
    ) -> Result<Vec<(CanonicalKey, FactCid)>, StorageError>;

    /// Borrow the per-attester reputation tracker, if this storage backend
    /// runs one. Optional because ephemeral / read-only deploys may skip it.
    fn attesters(&self) -> Option<&AttesterRegistry> {
        None
    }

    /// Look up the merkle inclusion proof persisted for `cid` at
    /// attestation-write time. Returns None when no proof was ever
    /// persisted (ephemeral storage that didn't open the
    /// `emem.fact_proofs` tree, or a fact written before this surface
    /// existed). Default impl returns None so backends that don't track
    /// proofs are still valid `Storage`.
    fn proof_for_cid(&self, _cid: &FactCid) -> Option<MerkleProof> {
        None
    }

    /// Borrow the hot-cache sled DB if one is mounted, so callers (e.g.
    /// the API layer's agent-stats persistence) can open auxiliary trees
    /// alongside the canonical index. Optional: ephemeral or non-sled
    /// backends return `None`.
    fn hot_sled_db(&self) -> Option<&sled::Db> {
        None
    }

    /// Scan the multi-attester index for every `(cell, band, tslot)` key
    /// that has at least TWO distinct fact CIDs recorded (which is the
    /// minimum for a contradiction to be possible). Returns
    /// `(CanonicalKey, Vec<FactCid>)` pairs in the iteration order of
    /// the underlying tree.
    ///
    /// `cell_prefix` filters at the iterator boundary so a regional
    /// scan never loads the whole index — `prefix = "defi.zb"` only
    /// walks the matching sled key range.
    ///
    /// Default impl returns `Ok(vec![])` so backends that don't
    /// implement the multi-attester index (in-memory mocks, ephemeral
    /// stores opened before this tree existed) continue to compile and
    /// answer with "no contradictions known".
    async fn scan_multi_attester(
        &self,
        _cell_prefix: Option<&str>,
        _limit: usize,
    ) -> Result<Vec<(emem_cache::CanonicalKey, Vec<FactCid>)>, StorageError> {
        Ok(Vec::new())
    }
}

/// Storage errors.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    /// Cache subsystem error.
    #[error("cache: {0}")]
    Cache(#[from] emem_cache::CacheError),
    /// Fetch subsystem error.
    #[error("fetch: {0}")]
    Fetch(#[from] emem_fetch::FetchError),
    /// Function key not found in active registry.
    #[error("function not in registry: {0}")]
    UnknownFunction(String),
    /// Band key not found in active registry.
    #[error("band not in registry: {0}")]
    UnknownBand(String),
    /// CBOR encode/decode failure.
    #[error("cbor: {0}")]
    Cbor(String),
    /// Disk I/O.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// Attestation rejected by verification (root mismatch, bad sig, etc.).
    #[error("attestation verification failed: {0}")]
    AttestationInvalid(String),
    /// Materialization required upstream fetch but no provider was registered
    /// for the source-scheme implied by the function recipe.
    #[error("materialize miss: {0}")]
    MaterializeMiss(String),
    /// Generic protocol error mapped to a wire-stable [`ErrorCode`].
    #[error("{code:?}: {message}")]
    Protocol { code: ErrorCode, message: String },
}

impl StorageError {
    /// Map this error to the wire-stable [`ErrorCode`] for transport-layer
    /// envelopes (REST / MCP).
    pub fn wire_code(&self) -> ErrorCode {
        match self {
            StorageError::Cache(_) => ErrorCode::CacheError,
            StorageError::Fetch(_) => ErrorCode::SourceFetchFailed,
            StorageError::UnknownFunction(_) => ErrorCode::FunctionNotInRegistry,
            StorageError::UnknownBand(_) => ErrorCode::BandNotInRegistry,
            StorageError::Cbor(_) => ErrorCode::CanonicalEncodingDivergence,
            StorageError::Io(_) => ErrorCode::Internal,
            StorageError::AttestationInvalid(_) => ErrorCode::BadSignature,
            StorageError::MaterializeMiss(_) => ErrorCode::CidNotFound,
            StorageError::Protocol { code, .. } => *code,
        }
    }
}

impl MaterializingStorage {
    /// Build a storage layer that uses an in-memory hot cache and a
    /// fetch dispatcher with the public open-data HTTPS connectors
    /// pre-registered. No on-disk persistence — for tests and ephemeral
    /// dev runs.
    pub fn ephemeral(
        bands: Arc<BandRegistry>,
        functions: Arc<FunctionRegistry>,
        sources: Arc<SourceRegistry>,
    ) -> Result<Self, StorageError> {
        let hot = Arc::new(SledHotCache::open_temporary()?);
        let attesters = AttesterRegistry::open(hot.db()).ok();
        let log_dir = tempdir_for_log()?;
        let log = Arc::new(AttestationLog::open(log_dir)?);
        let mut fetch = Dispatcher::new();
        emem_fetch::connectors::register_default_https(&mut fetch);
        Ok(Self {
            cache: hot.clone(),
            hot: Some(hot),
            fetch,
            bands,
            functions,
            sources,
            log,
            attesters,
        })
    }

    /// Build a storage layer rooted at `root`: `<root>/cache.sled` for the
    /// hot cache, `<root>/log/` for merkle log segments.
    pub fn rooted(
        root: impl AsRef<std::path::Path>,
        bands: Arc<BandRegistry>,
        functions: Arc<FunctionRegistry>,
        sources: Arc<SourceRegistry>,
    ) -> Result<Self, StorageError> {
        let root = root.as_ref();
        std::fs::create_dir_all(root)?;
        let hot = Arc::new(SledHotCache::open(root.join("cache.sled"))?);
        let attesters = AttesterRegistry::open(hot.db()).ok();
        let log = Arc::new(AttestationLog::open(root.join("log"))?);
        let mut fetch = Dispatcher::new();
        emem_fetch::connectors::register_default_https(&mut fetch);
        Ok(Self {
            cache: hot.clone(),
            hot: Some(hot),
            fetch,
            bands,
            functions,
            sources,
            log,
            attesters,
        })
    }
}

fn tempdir_for_log() -> std::io::Result<std::path::PathBuf> {
    let mut p = std::env::temp_dir();
    p.push(format!("emem-log-{}", std::process::id()));
    std::fs::create_dir_all(&p)?;
    Ok(p)
}

#[async_trait]
impl Storage for MaterializingStorage {
    async fn lookup_canonical_many(
        &self,
        keys: &[CanonicalKey],
    ) -> Result<Vec<Option<FactCid>>, StorageError> {
        Ok(self.cache.lookup_many(keys).await?)
    }

    async fn get_facts_many(&self, cids: &[FactCid]) -> Result<Vec<Option<Fact>>, StorageError> {
        let facts = self.cache.get_many(cids).await?;
        // Citation rollup — increments per-attester citation counters for
        // facts that were actually served. Best-effort: a tracker error
        // must never fail a read.
        if let Some(reg) = &self.attesters {
            let served: Vec<Fact> = facts.iter().flatten().cloned().collect();
            if !served.is_empty() {
                if let Err(e) = reg.record_citations(&served) {
                    tracing::debug!(error=%e, "attester citation tracker error (ignored)");
                }
            }
        }
        Ok(facts)
    }

    async fn put_attestation(&self, att: &Attestation) -> Result<Vec<FactCid>, StorageError> {
        verify_attestation(att)?;
        let cids = self.cache.put_many(&att.facts).await?;
        self.log.append(att).await?;
        // Persist a per-fact merkle inclusion proof so receipts citing
        // any of these CIDs can ship a verifier-ready proof. Best-effort:
        // a tree-write error never fails the attestation itself.
        if let Some(hot) = &self.hot {
            if let Err(e) = persist_fact_proofs(hot.db(), &att.facts, &cids) {
                tracing::warn!(error=%e, "fact proof persistence error (ignored)");
            }
            // Append every keyable fact's CID to the multi-attester
            // index. The canonical index above is last-write-wins;
            // this parallel index preserves every distinct CID
            // attested at a (cell, band, tslot) key so the
            // contradictions primitive can surface disagreement.
            // Best-effort: errors here never fail the attestation.
            if let Err(e) = append_multi_attester(hot.db(), &att.facts, &cids) {
                tracing::warn!(error=%e, "multi-attester index append error (ignored)");
            }
        }
        if let Some(reg) = &self.attesters {
            if let Err(e) = reg.record_attestation(&att.attester.0, &att.facts) {
                tracing::warn!(error=%e, "attester reputation tracker error (ignored)");
            }
        }
        Ok(cids)
    }

    async fn materialize_many(&self, keys: &[CanonicalKey]) -> Result<Vec<FactCid>, StorageError> {
        let hits = self.cache.lookup_many(keys).await?;
        let mut out: Vec<FactCid> = Vec::with_capacity(keys.len());
        for (key, hit) in keys.iter().zip(hits) {
            match hit {
                Some(cid) => out.push(cid),
                None => {
                    return Err(StorageError::MaterializeMiss(format!(
                        "no fact for cell={}, band={}, tslot={}; submit a signed Attestation via /v1/attest before recall, or operator must register an upstream connector for the function recipe that produces band '{}'",
                        key.cell, key.band, key.tslot, key.band)));
                }
            }
        }
        Ok(out)
    }

    async fn scan_cell(
        &self,
        cell: &str,
        tslot: Option<u64>,
    ) -> Result<Vec<(CanonicalKey, FactCid)>, StorageError> {
        let hot = self.hot.as_ref().ok_or_else(|| StorageError::Protocol {
            code: ErrorCode::Internal,
            message: "scan_cell requires a SledHotCache handle".into(),
        })?;
        Ok(hot.scan_cell(cell, tslot)?)
    }

    async fn scan_cell_as_of(
        &self,
        cell: &str,
        tslot: Option<u64>,
        bound: &AsOfBound,
    ) -> Result<Vec<(CanonicalKey, FactCid)>, StorageError> {
        // Index-bound fast path: when the caller did not pin transaction
        // time, we can satisfy the whole bound from the canonical index
        // without loading a single CBOR body.
        let hot = self.hot.as_ref().ok_or_else(|| StorageError::Protocol {
            code: ErrorCode::Internal,
            message: "scan_cell_as_of requires a SledHotCache handle".into(),
        })?;
        let pairs = hot.scan_cell_with_tslot_bound(cell, tslot, bound.valid_time)?;
        if bound.transaction_time.is_none() {
            return Ok(pairs);
        }
        // Transaction-time predicate requires loading the body for
        // `signed_at`. Batch the loads so the cost is one round-trip.
        let cids: Vec<FactCid> = pairs.iter().map(|(_, c)| c.clone()).collect();
        let facts = self.get_facts_many(&cids).await?;
        let mut filtered: Vec<(CanonicalKey, FactCid)> = Vec::with_capacity(pairs.len());
        for ((k, c), fact) in pairs.into_iter().zip(facts) {
            if let Some(f) = fact {
                if bound.fact_passes(&f) {
                    filtered.push((k, c));
                }
            }
        }
        Ok(filtered)
    }

    async fn iter_index(
        &self,
        limit: Option<usize>,
    ) -> Result<Vec<(CanonicalKey, FactCid)>, StorageError> {
        let hot = self.hot.as_ref().ok_or_else(|| StorageError::Protocol {
            code: ErrorCode::Internal,
            message: "iter_index requires a SledHotCache handle".into(),
        })?;
        let mut out = Vec::new();
        for entry in hot.iter_index() {
            out.push(entry?);
            if let Some(n) = limit {
                if out.len() >= n {
                    break;
                }
            }
        }
        Ok(out)
    }

    fn attesters(&self) -> Option<&AttesterRegistry> {
        self.attesters.as_ref()
    }

    fn hot_sled_db(&self) -> Option<&sled::Db> {
        self.hot.as_ref().map(|h| h.db())
    }

    fn proof_for_cid(&self, cid: &FactCid) -> Option<MerkleProof> {
        let hot = self.hot.as_ref()?;
        let tree = hot.db().open_tree(TREE_FACT_PROOFS).ok()?;
        let bytes = tree.get(cid.as_str().as_bytes()).ok()??;
        ciborium::de::from_reader::<MerkleProof, _>(&*bytes).ok()
    }

    async fn scan_multi_attester(
        &self,
        cell_prefix: Option<&str>,
        limit: usize,
    ) -> Result<Vec<(emem_cache::CanonicalKey, Vec<FactCid>)>, StorageError> {
        let hot = self.hot.as_ref().ok_or_else(|| StorageError::Protocol {
            code: ErrorCode::Internal,
            message: "scan_multi_attester requires a SledHotCache handle".into(),
        })?;
        scan_multi_attester_tree(hot.db(), cell_prefix, limit)
    }
}

/// Append every keyable fact's CID to the multi-attester index. CBOR
/// payload at each key is `Vec<FactCid>` (the encoded `String` form so
/// readers don't need the `FactCid` type to deserialize). Dedupe is
/// done in-memory before write so a redundant attestation (same fact
/// re-signed by the same attester) is a no-op. Best-effort: a write
/// error here is logged but never fails the attestation.
fn append_multi_attester(
    db: &sled::Db,
    facts: &[Fact],
    cids: &[FactCid],
) -> Result<(), StorageError> {
    if facts.is_empty() || cids.len() != facts.len() {
        return Ok(());
    }
    let tree = db
        .open_tree(TREE_MULTI_ATTESTER_INDEX)
        .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
    for (f, cid) in facts.iter().zip(cids.iter()) {
        let key_bytes = match fact_canonical_key_bytes(f) {
            Some(k) => k,
            None => continue, // derivative facts have no canonical key
        };
        let existing: Vec<String> = match tree
            .get(&key_bytes)
            .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?
        {
            Some(b) => ciborium::de::from_reader::<Vec<String>, _>(&*b)
                .map_err(|e| StorageError::Cbor(format!("multi_attester decode: {e}")))?,
            None => Vec::new(),
        };
        if existing.iter().any(|s| s == cid.as_str()) {
            continue;
        }
        let mut updated = existing;
        updated.push(cid.as_str().to_string());
        let mut buf = Vec::new();
        ciborium::ser::into_writer(&updated, &mut buf)
            .map_err(|e| StorageError::Cbor(format!("multi_attester encode: {e}")))?;
        tree.insert(&key_bytes, buf)
            .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
    }
    tree.flush()
        .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
    Ok(())
}

/// Encode `(cell, band, tslot)` exactly as `emem-cache` does so the
/// multi-attester index keys are bytewise-comparable with the canonical
/// index for prefix scans by cell64.
fn fact_canonical_key_bytes(fact: &Fact) -> Option<Vec<u8>> {
    let (cell, band, tslot) = match fact {
        Fact::Primary(p) => (p.cell.as_str(), p.band.as_str(), p.tslot),
        Fact::Absence(n) => (n.cell.as_str(), n.band.as_str(), n.tslot),
        Fact::Derivative(_) => return None,
    };
    let mut buf = Vec::with_capacity(cell.len() + band.len() + 10);
    buf.extend_from_slice(cell.as_bytes());
    buf.push(0u8);
    buf.extend_from_slice(band.as_bytes());
    buf.push(0u8);
    buf.extend_from_slice(&tslot.to_be_bytes());
    Some(buf)
}

/// Decode a `(cell, band, tslot)` key emitted by
/// [`fact_canonical_key_bytes`].
fn decode_key_bytes(b: &[u8]) -> Option<emem_cache::CanonicalKey> {
    let mut parts = b.splitn(3, |c| *c == 0u8);
    let cell = parts.next()?;
    let band = parts.next()?;
    let rest = parts.next()?;
    if rest.len() != 8 {
        return None;
    }
    let mut t = [0u8; 8];
    t.copy_from_slice(rest);
    Some(emem_cache::CanonicalKey {
        cell: std::str::from_utf8(cell).ok()?.to_string(),
        band: std::str::from_utf8(band).ok()?.to_string(),
        tslot: u64::from_be_bytes(t),
    })
}

/// Walk the multi-attester index. Returns only entries with ≥ 2
/// distinct CIDs (where a contradiction is possible). Stops once
/// `limit` such entries have been collected so a corpus-wide scan
/// remains O(scanned_keys) rather than O(all_keys).
fn scan_multi_attester_tree(
    db: &sled::Db,
    cell_prefix: Option<&str>,
    limit: usize,
) -> Result<Vec<(emem_cache::CanonicalKey, Vec<FactCid>)>, StorageError> {
    let tree = db
        .open_tree(TREE_MULTI_ATTESTER_INDEX)
        .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
    let mut out: Vec<(emem_cache::CanonicalKey, Vec<FactCid>)> = Vec::new();
    let iter: Box<dyn Iterator<Item = sled::Result<(sled::IVec, sled::IVec)>>> = match cell_prefix {
        Some(p) if !p.is_empty() => {
            // Iterator at the sled level over the byte prefix. We
            // include the SEP byte only when the caller passed the
            // full cell64 path; for a partial prefix we keep it as
            // a raw bytewise prefix.
            let prefix_bytes = p.as_bytes().to_vec();
            Box::new(tree.scan_prefix(prefix_bytes))
        }
        _ => Box::new(tree.iter()),
    };
    for kv in iter {
        if out.len() >= limit {
            break;
        }
        let (k, v) = kv.map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
        let Some(key) = decode_key_bytes(&k) else {
            continue;
        };
        let cids: Vec<String> = ciborium::de::from_reader::<Vec<String>, _>(&*v)
            .map_err(|e| StorageError::Cbor(format!("multi_attester decode: {e}")))?;
        if cids.len() < 2 {
            continue;
        }
        let cids: Vec<FactCid> = cids.into_iter().map(FactCid::new).collect();
        out.push((key, cids));
    }
    Ok(out)
}

/// Compute the per-fact merkle inclusion proof for every fact in the
/// attestation and write it to the dedicated sled tree, keyed by
/// `FactCid` string. The tree is opened on demand so attestations that
/// pre-date this surface continue to round-trip without it.
///
/// The leaves are ordered exactly as they are inside [`verify_attestation`]:
/// CBOR-encode each fact, blake3 the bytes, sort the leaves bytewise.
/// `MerkleProof.leaf_index` is the leaf's position in that sorted order.
fn persist_fact_proofs(
    db: &sled::Db,
    facts: &[Fact],
    cids: &[FactCid],
) -> Result<(), StorageError> {
    if facts.is_empty() || cids.len() != facts.len() {
        return Ok(());
    }
    let mut leaves_with_orig: Vec<([u8; 32], usize)> = Vec::with_capacity(facts.len());
    for (i, f) in facts.iter().enumerate() {
        let mut buf = Vec::new();
        ciborium::ser::into_writer(f, &mut buf)
            .map_err(|e| StorageError::Cbor(format!("fact_proofs cbor: {e}")))?;
        let h = blake3::hash(&buf);
        let mut a = [0u8; 32];
        a.copy_from_slice(h.as_bytes());
        leaves_with_orig.push((a, i));
    }
    leaves_with_orig.sort_by_key(|a| a.0);
    let leaves: Vec<[u8; 32]> = leaves_with_orig.iter().map(|(l, _)| *l).collect();
    let (root, paths) = emem_attest::merkle_root_and_paths(&leaves);
    let tree = db
        .open_tree(TREE_FACT_PROOFS)
        .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
    for (sorted_idx, (_, orig_idx)) in leaves_with_orig.iter().enumerate() {
        let cid = &cids[*orig_idx];
        let proof = MerkleProof {
            leaf_index: sorted_idx as u32,
            path: paths[sorted_idx].clone(),
            root,
        };
        let mut buf = Vec::new();
        ciborium::ser::into_writer(&proof, &mut buf)
            .map_err(|e| StorageError::Cbor(format!("fact_proofs cbor: {e}")))?;
        tree.insert(cid.as_str().as_bytes(), buf)
            .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
    }
    tree.flush()
        .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
    Ok(())
}

/// Verify an attestation envelope:
///
/// 1. Recompute each fact's CBOR + blake3 hash; sort the leaves canonically;
///    confirm the merkle root matches `att.batch_root`.
/// 2. Verify the ed25519 signature over `blake3(batch_root || registry_cid_bytes || schema_cid_bytes)`.
fn verify_attestation(att: &Attestation) -> Result<(), StorageError> {
    let mut leaves: Vec<[u8; 32]> = Vec::with_capacity(att.facts.len());
    for f in &att.facts {
        let mut buf = Vec::new();
        ciborium::ser::into_writer(f, &mut buf)
            .map_err(|e| StorageError::AttestationInvalid(format!("fact cbor: {e}")))?;
        let h = blake3::hash(&buf);
        let mut a = [0u8; 32];
        a.copy_from_slice(h.as_bytes());
        leaves.push(a);
    }
    leaves.sort();
    let root = emem_attest::merkle_root(&leaves);
    if root != att.batch_root {
        return Err(StorageError::AttestationInvalid(format!(
            "merkle root mismatch: computed={} declared={}",
            hex32(&root),
            hex32(&att.batch_root)
        )));
    }

    let mut h = Hasher::new();
    h.update(&att.batch_root);
    h.update(att.registry_cid.as_str().as_bytes());
    h.update(att.schema_cid.as_str().as_bytes());
    let msg = h.finalize();

    let pk = ed25519_dalek::VerifyingKey::from_bytes(&att.attester.0)
        .map_err(|e| StorageError::AttestationInvalid(format!("bad attester key: {e}")))?;
    let sig = ed25519_dalek::Signature::from_bytes(&att.signature.0);
    pk.verify_strict(msg.as_bytes(), &sig)
        .map_err(|e| StorageError::AttestationInvalid(format!("bad signature: {e}")))?;
    Ok(())
}

fn hex32(b: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for x in b {
        s.push_str(&format!("{:02x}", x));
    }
    s
}

#[cfg(test)]
mod multi_attester_tests {
    //! End-to-end smoke that two attesters writing to the same
    //! `(cell, band, tslot)` triple end up with BOTH CIDs in the
    //! multi-attester index (even though the canonical index is
    //! last-write-wins). Drives `MaterializingStorage::put_attestation`
    //! directly with fully-signed attestations so the test exercises
    //! the production code path.

    use super::*;
    use blake3::Hasher;
    use ed25519_dalek::{Signer, SigningKey};
    use emem_attest::merkle_root;
    use emem_core::{AttesterKey, KeyEpoch, Signature};
    use emem_fact::{Attestation, Derivation, Fact, PrimaryFact, RegistryCid, SchemaCid, Source};

    fn build_signed(facts: Vec<Fact>, secret: [u8; 32]) -> (Attestation, [u8; 32]) {
        let registry_cid = "test-registry";
        let schema_cid = "test-schema";
        let signing = SigningKey::from_bytes(&secret);
        let vk = signing.verifying_key();
        let mut pk = [0u8; 32];
        pk.copy_from_slice(vk.as_bytes());
        let mut leaves: Vec<[u8; 32]> = facts
            .iter()
            .map(|f| {
                let mut buf = Vec::new();
                ciborium::ser::into_writer(f, &mut buf).unwrap();
                let h = blake3::hash(&buf);
                let mut a = [0u8; 32];
                a.copy_from_slice(h.as_bytes());
                a
            })
            .collect();
        leaves.sort();
        let root = merkle_root(&leaves);
        let mut h = Hasher::new();
        h.update(&root);
        h.update(registry_cid.as_bytes());
        h.update(schema_cid.as_bytes());
        let msg = h.finalize();
        let sig = signing.sign(msg.as_bytes());
        let mut sig_bytes = [0u8; 64];
        sig_bytes.copy_from_slice(&sig.to_bytes());
        let att = Attestation {
            facts,
            batch_root: root,
            attester: AttesterKey(pk),
            attester_key_epoch: KeyEpoch(0),
            registry_cid: RegistryCid::new(registry_cid),
            schema_cid: SchemaCid::new(schema_cid),
            signature: Signature(sig_bytes),
            attested_at: "2026-05-28T00:00:00Z".into(),
        };
        (att, pk)
    }

    fn mk_ndvi_fact(cell: &str, tslot: u64, ndvi: f64, signer_pk: [u8; 32]) -> Fact {
        Fact::Primary(PrimaryFact {
            cell: cell.into(),
            band: "indices.ndvi".into(),
            tslot,
            value: ciborium::Value::Float(ndvi),
            unit: None,
            confidence: 1.0,
            uncertainty: None,
            sources: vec![Source {
                scheme: "test".into(),
                id: format!("ndvi-{ndvi}"),
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
            signer: AttesterKey(signer_pk),
            signed_at: "2026-05-28T00:00:00Z".into(),
            served_via: None,
        })
    }

    #[tokio::test]
    async fn put_attestation_populates_multi_attester_index() {
        // Build an ephemeral MaterializingStorage so the test exercises
        // the hot-cache sled tree wiring end to end. The function
        // registry has nothing wired for "indices.ndvi" but we never
        // call materialize_many in this test, only put_attestation —
        // which doesn't touch the registry.
        let bands = Arc::new(emem_core::bands::DEFAULT.clone());
        let functions =
            Arc::new(emem_core::FunctionRegistry::parse_default().expect("default functions"));
        let sources =
            Arc::new(emem_core::SourceRegistry::parse_default().expect("default sources"));
        let storage =
            MaterializingStorage::ephemeral(bands, functions, sources).expect("ephemeral storage");

        let cell = "damO.zb000.xUti.zde78";
        let tslot = 12u64;
        // Attester A signs NDVI = 0.85.
        let mut sec_a = [0u8; 32];
        sec_a[0] = 1;
        let f_a = mk_ndvi_fact(
            cell,
            tslot,
            0.85,
            SigningKey::from_bytes(&sec_a).verifying_key().to_bytes(),
        );
        let (att_a, _pk_a) = build_signed(vec![f_a.clone()], sec_a);
        let cids_a = storage.put_attestation(&att_a).await.expect("put A");
        assert_eq!(cids_a.len(), 1);

        // Attester B signs NDVI = 0.10 at the SAME (cell, band, tslot).
        let mut sec_b = [0u8; 32];
        sec_b[0] = 2;
        let f_b = mk_ndvi_fact(
            cell,
            tslot,
            0.10,
            SigningKey::from_bytes(&sec_b).verifying_key().to_bytes(),
        );
        let (att_b, _pk_b) = build_signed(vec![f_b.clone()], sec_b);
        let cids_b = storage.put_attestation(&att_b).await.expect("put B");
        assert_eq!(cids_b.len(), 1);
        assert_ne!(
            cids_a[0].as_str(),
            cids_b[0].as_str(),
            "two attesters at the same triple MUST produce different CIDs (they sign different content)"
        );

        // Canonical index is last-write-wins → only B's CID should
        // live there. (Confirming our motivation for the multi-attester
        // index.)
        let canonical = storage
            .lookup_canonical_many(&[emem_cache::CanonicalKey {
                cell: cell.into(),
                band: "indices.ndvi".into(),
                tslot,
            }])
            .await
            .unwrap();
        assert_eq!(
            canonical[0].as_ref().map(|c| c.as_str()),
            Some(cids_b[0].as_str())
        );

        // Multi-attester index MUST carry both CIDs.
        let multi = storage.scan_multi_attester(None, 1024).await.expect("scan");
        let entry = multi
            .iter()
            .find(|(k, _)| k.cell == cell && k.band == "indices.ndvi" && k.tslot == tslot)
            .expect("multi-attester entry for the disputed triple");
        let cid_strs: std::collections::BTreeSet<&str> =
            entry.1.iter().map(|c| c.as_str()).collect();
        assert!(cid_strs.contains(cids_a[0].as_str()));
        assert!(cid_strs.contains(cids_b[0].as_str()));
        assert_eq!(entry.1.len(), 2, "exactly two distinct CIDs preserved");
    }

    #[tokio::test]
    async fn cell_prefix_scan_filters_at_iterator() {
        let bands = Arc::new(emem_core::bands::DEFAULT.clone());
        let functions =
            Arc::new(emem_core::FunctionRegistry::parse_default().expect("default functions"));
        let sources =
            Arc::new(emem_core::SourceRegistry::parse_default().expect("default sources"));
        let storage = MaterializingStorage::ephemeral(bands, functions, sources).unwrap();

        // Two contradictions in different cell prefixes — only the
        // matching prefix should surface.
        for (cell, ndvi_a, ndvi_b, sec_seed_a, sec_seed_b) in [
            ("alfa.zb000.aaaa.aaaa", 0.85_f64, 0.10_f64, 11u8, 12u8),
            ("bravo.zb000.aaaa.aaaa", 0.90_f64, 0.05_f64, 21u8, 22u8),
        ] {
            for (ndvi, sec_seed) in [(ndvi_a, sec_seed_a), (ndvi_b, sec_seed_b)] {
                let mut sec = [0u8; 32];
                sec[0] = sec_seed;
                let pk = SigningKey::from_bytes(&sec).verifying_key().to_bytes();
                let f = mk_ndvi_fact(cell, 0, ndvi, pk);
                let (att, _) = build_signed(vec![f], sec);
                storage.put_attestation(&att).await.unwrap();
            }
        }

        let only_alfa = storage
            .scan_multi_attester(Some("alfa"), 1024)
            .await
            .unwrap();
        assert_eq!(only_alfa.len(), 1);
        assert!(only_alfa[0].0.cell.starts_with("alfa"));

        let all = storage.scan_multi_attester(None, 1024).await.unwrap();
        assert_eq!(all.len(), 2, "no prefix → both contradictions");
    }
}
