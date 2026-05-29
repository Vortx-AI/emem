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
use emem_fact::{Attestation, EdgeCid, EdgeFact, Fact, FactCid, MerkleProof};
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

/// Sled tree for AEAD-sealed Vault memory entries (v0.0.8). Key: the
/// memory path (UTF-8, e.g. `/memories/secrets/openai.key`). Value:
/// canonical CBOR of the sealed envelope (ciphertext + nonce + aad +
/// attester pubkey that derived the key). Vault entries live ONLY here —
/// never in [`TREE_MEMORY_FILES`] / [`TREE_MEMORY_FILE_BLOBS`] /
/// [`TREE_MEMORY_FILES_BY_KIND`] — so they are structurally invisible to
/// the BGE search indexer (which scans the plaintext file trees) and to
/// the contradiction scanner. Decryption requires a per-call ed25519
/// capability; see the `emem-api-rest` vault module.
pub const TREE_MEMORY_VAULT: &str = "emem.memory_vault";

/// Sled tree mapping `edge_cid` (base32-nopad-lc) → canonical CBOR of the
/// [`emem_fact::EdgeFact`] body. The content-addressed edge store; the SPO
/// / OPS index trees point back here to hydrate bodies. (v0.0.9 temporal
/// knowledge-graph edges.)
pub const TREE_EDGES: &str = "emem.edges";

/// Sled tree indexing edges by `(subject, predicate, valid_from)` for
/// ascending range scans. Key:
/// `subj_bytes \0 pred_bytes \0 valid_from.to_be_bytes() \0 edge_cid_bytes`.
/// Big-endian `valid_from` so a `scan_prefix(subj\0pred\0)` walks edges in
/// ascending valid-time order. Value: the object fact CID bytes.
pub const TREE_EDGE_SPO: &str = "emem.edge_spo";

/// Reverse of [`TREE_EDGE_SPO`], keyed by object: `obj_bytes \0 pred_bytes
/// \0 valid_from_be8 \0 edge_cid_bytes` → subject fact CID bytes. Lets a
/// future "what points at this fact" query range-scan without walking the
/// forward index.
pub const TREE_EDGE_OPS: &str = "emem.edge_ops";

/// Sled tree mapping `fact_cid` (base32-nopad-lc) → canonical CBOR of a
/// [`FactContestedRecord`]. Written by the contradiction-fed refinement
/// loop when a fact loses a `disagrees_with` pairing: it marks the
/// lower-confidence fact as contested. The fact BODY is never touched —
/// this is a non-destructive overlay row keyed by the fact's CID, so a
/// recall path can surface "this observation is contested by edge X"
/// without re-signing or mutating the content-addressed fact. Idempotent
/// overwrite-by-key. (v0.0.9 refinement loop.)
pub const TREE_FACT_CONTESTED: &str = "emem.fact_contested";

/// Sled tree indexing every scoped fact by its multi-tenant
/// [`emem_fact::Scope`] so a scoped recall can range-scan a prefix
/// without walking every fact at the cell. Populated by
/// [`MaterializingStorage::put_attestation`] ONLY when the attestation
/// carries a non-empty `scope` (additive serde-default field on
/// [`Attestation`]). Without a scope no rows are written and recall
/// falls back to the global canonical index — the pre-v0.0.8 behaviour
/// is byte-identical.
///
/// Key layout (NUL-separated, empty-string sentinel for absent fields):
/// `scope_user \0 scope_agent \0 scope_run \0 scope_org \0 cell64 \0 band \0 tslot_be8`.
/// Value: `fact_cid` bytes (UTF-8 base32-nopad-lc). The four scope
/// fields lead the key so `scan_prefix(user \0 agent \0 run \0 org \0)`
/// walks exactly the facts visible to that tenant in O(matches). The
/// `tslot_be8` suffix keeps multiple tslots at the same (cell, band)
/// distinct so last-write-wins per (scope, cell, band, tslot) holds the
/// same shape as the canonical index.
pub const TREE_SCOPE_INDEX: &str = "emem.scope_index";

/// Non-destructive overlay record marking a content-addressed fact as
/// contested by a refinement-loop `disagrees_with` edge. Stored in
/// [`TREE_FACT_CONTESTED`] keyed by the fact's CID; the fact body is
/// never mutated. (v0.0.9 refinement loop.)
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FactContestedRecord {
    /// Edge CID (base32-nopad-lc) of the `disagrees_with` edge that
    /// contested this fact.
    pub by_edge: String,
    /// Severity of the contradiction in `[0, 1]` that produced the edge.
    pub severity: f32,
    /// ISO 8601 wall-clock when the marker was written.
    pub marked_at: String,
    /// `true` when this fact is the LOWER-confidence side of the
    /// disagreeing pair (the side the refinement loop down-weights).
    pub lower_confidence: bool,
}

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

    /// Scope-filtered sibling of [`Storage::scan_cell`] (v0.0.8). Returns
    /// only the `(canonical_key, fact_cid)` pairs at `cell` (optionally
    /// pinned to `tslot`) that were written under a [`Scope`] matching
    /// `scope` exactly (field-for-field, with `None` matching only the
    /// empty-string sentinel — i.e. a recall scoped to `{user_id:"u1"}`
    /// sees only facts written under `{user_id:"u1"}`, not facts written
    /// globally or under a different user).
    ///
    /// Default impl: when `scope` is `None` or empty, OR the backend has
    /// no scope index, this delegates to [`Storage::scan_cell`] so every
    /// existing call site compiles unchanged and unscoped reads keep the
    /// pre-v0.0.8 behaviour. Only [`MaterializingStorage`] overrides this
    /// to range-scan the dedicated scope-index tree.
    async fn scan_cell_in_scope(
        &self,
        cell: &str,
        tslot: Option<u64>,
        scope: Option<&emem_fact::Scope>,
    ) -> Result<Vec<(CanonicalKey, FactCid)>, StorageError> {
        match scope {
            Some(sc) if !sc.is_empty() => self.scan_cell(cell, tslot).await,
            _ => self.scan_cell(cell, tslot).await,
        }
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

    /// Persist temporal knowledge-graph edges. Idempotent: an edge whose
    /// CID already lives in [`TREE_EDGES`] is a no-op. Returns the CIDs in
    /// input order. Default impl is a no-op returning `[]` so in-memory
    /// mocks and ephemeral backends keep compiling. (v0.0.9.)
    async fn add_edges(
        &self,
        _edges: &[emem_fact::EdgeFact],
    ) -> Result<Vec<emem_fact::EdgeCid>, StorageError> {
        Ok(Vec::new())
    }

    /// Recall edges originating at `subj` under predicate `pred`,
    /// bi-temporally filtered by `as_of`. When `pred` is the empty string
    /// `""`, scan across every predicate for the subject. `as_of = None`
    /// returns the latest edge per object regardless of valid-time;
    /// `as_of = Some(t)` keeps edges with `valid_from <= t` and drops any
    /// whose `valid_to` is `Some(vt)` with `vt < t` (closed intervals).
    /// Supersession: among surviving edges to the same object, the one
    /// with the largest `valid_from` wins. Default impl returns `[]`.
    async fn recall_edges(
        &self,
        _subj: &FactCid,
        _pred: &str,
        _as_of: Option<u64>,
        _limit: usize,
    ) -> Result<Vec<emem_fact::EdgeFact>, StorageError> {
        Ok(Vec::new())
    }

    /// `true` when an edge with this CID has been persisted. Default impl
    /// returns `false`.
    async fn has_edge(&self, _cid: &emem_fact::EdgeCid) -> Result<bool, StorageError> {
        Ok(false)
    }

    /// Mark a content-addressed fact as contested by a refinement-loop
    /// `disagrees_with` edge. Writes a [`FactContestedRecord`] into
    /// [`TREE_FACT_CONTESTED`] keyed by `fact_cid`. The fact body is NEVER
    /// mutated — this is a non-destructive overlay row. Idempotent:
    /// overwrite-by-key is fine. Default impl is a no-op so in-memory
    /// mocks and ephemeral backends keep compiling. (v0.0.9.)
    async fn mark_fact_contested(
        &self,
        _fact_cid: &FactCid,
        _record: &FactContestedRecord,
    ) -> Result<(), StorageError> {
        Ok(())
    }

    /// Read the contested overlay record for a fact, if any was ever
    /// written. Default impl returns `None`.
    async fn get_fact_contested(
        &self,
        _fact_cid: &FactCid,
    ) -> Result<Option<FactContestedRecord>, StorageError> {
        Ok(None)
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
            // Scope index (v0.0.8): when the attestation carries a
            // non-empty multi-tenant scope, write one scope-index row per
            // keyable fact so a later scoped recall can range-scan exactly
            // this tenant's facts. Without a scope (or an empty one) NO
            // rows are written and recall falls back to the global
            // canonical index — the pre-v0.0.8 path is byte-identical.
            // Best-effort: an index-write error never fails the write.
            if let Some(scope) = att.scope.as_ref().filter(|sc| !sc.is_empty()) {
                if let Err(e) = append_scope_index(hot.db(), scope, &att.facts, &cids) {
                    tracing::warn!(error=%e, "scope index append error (ignored)");
                }
            }
        }
        if let Some(reg) = &self.attesters {
            if let Err(e) = reg.record_attestation(&att.attester.0, &att.facts) {
                tracing::warn!(error=%e, "attester reputation tracker error (ignored)");
            }
        }
        // Persist any temporal knowledge-graph edges carried by this
        // attestation. The signature already committed to them (the edge
        // leaves were folded into the verified merkle root above), so
        // persisting here is the canonical write path. Best-effort: an
        // index-write error is logged but never fails the attestation —
        // the facts are already durable.
        if !att.edges.is_empty() {
            if let Err(e) = self.add_edges(&att.edges).await {
                tracing::warn!(error=%e, "edge persistence error (ignored)");
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

    async fn scan_cell_in_scope(
        &self,
        cell: &str,
        tslot: Option<u64>,
        scope: Option<&emem_fact::Scope>,
    ) -> Result<Vec<(CanonicalKey, FactCid)>, StorageError> {
        // No scope (or an empty one) → identical to the global per-cell
        // scan. This keeps an unscoped recall byte-for-byte the same as
        // the pre-v0.0.8 path even on a backend that HAS the scope tree.
        let scope = match scope {
            Some(sc) if !sc.is_empty() => sc,
            _ => return self.scan_cell(cell, tslot).await,
        };
        let hot = self.hot.as_ref().ok_or_else(|| StorageError::Protocol {
            code: ErrorCode::Internal,
            message: "scan_cell_in_scope requires a SledHotCache handle".into(),
        })?;
        scan_scope_index(hot.db(), scope, cell, tslot)
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

    async fn add_edges(&self, edges: &[EdgeFact]) -> Result<Vec<EdgeCid>, StorageError> {
        if edges.is_empty() {
            return Ok(Vec::new());
        }
        let hot = self.hot.as_ref().ok_or_else(|| StorageError::Protocol {
            code: ErrorCode::Internal,
            message: "add_edges requires a SledHotCache handle".into(),
        })?;
        add_edges_tree(hot.db(), edges)
    }

    async fn recall_edges(
        &self,
        subj: &FactCid,
        pred: &str,
        as_of: Option<u64>,
        limit: usize,
    ) -> Result<Vec<EdgeFact>, StorageError> {
        let hot = self.hot.as_ref().ok_or_else(|| StorageError::Protocol {
            code: ErrorCode::Internal,
            message: "recall_edges requires a SledHotCache handle".into(),
        })?;
        recall_edges_tree(hot.db(), subj, pred, as_of, limit)
    }

    async fn has_edge(&self, cid: &EdgeCid) -> Result<bool, StorageError> {
        let hot = self.hot.as_ref().ok_or_else(|| StorageError::Protocol {
            code: ErrorCode::Internal,
            message: "has_edge requires a SledHotCache handle".into(),
        })?;
        let tree = hot
            .db()
            .open_tree(TREE_EDGES)
            .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
        Ok(tree
            .contains_key(cid.as_str().as_bytes())
            .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?)
    }

    async fn mark_fact_contested(
        &self,
        fact_cid: &FactCid,
        record: &FactContestedRecord,
    ) -> Result<(), StorageError> {
        let hot = self.hot.as_ref().ok_or_else(|| StorageError::Protocol {
            code: ErrorCode::Internal,
            message: "mark_fact_contested requires a SledHotCache handle".into(),
        })?;
        let tree = hot
            .db()
            .open_tree(TREE_FACT_CONTESTED)
            .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
        let mut buf = Vec::with_capacity(128);
        ciborium::into_writer(record, &mut buf)
            .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
        // Idempotent overwrite-by-key: the fact body is untouched; only
        // this overlay row is (re)written.
        tree.insert(fact_cid.as_str().as_bytes(), buf)
            .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
        Ok(())
    }

    async fn get_fact_contested(
        &self,
        fact_cid: &FactCid,
    ) -> Result<Option<FactContestedRecord>, StorageError> {
        let hot = self.hot.as_ref().ok_or_else(|| StorageError::Protocol {
            code: ErrorCode::Internal,
            message: "get_fact_contested requires a SledHotCache handle".into(),
        })?;
        let tree = hot
            .db()
            .open_tree(TREE_FACT_CONTESTED)
            .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
        let Some(bytes) = tree
            .get(fact_cid.as_str().as_bytes())
            .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?
        else {
            return Ok(None);
        };
        let rec: FactContestedRecord = ciborium::de::from_reader(&*bytes)
            .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
        Ok(Some(rec))
    }
}

/// Build the `(subj|obj) \0 pred \0 valid_from_be8 \0 edge_cid` index key.
fn edge_index_key(anchor: &str, pred: &str, valid_from: u64, edge_cid: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(anchor.len() + pred.len() + edge_cid.len() + 11);
    buf.extend_from_slice(anchor.as_bytes());
    buf.push(0u8);
    buf.extend_from_slice(pred.as_bytes());
    buf.push(0u8);
    buf.extend_from_slice(&valid_from.to_be_bytes());
    buf.push(0u8);
    buf.extend_from_slice(edge_cid.as_bytes());
    buf
}

/// Decode the `valid_from` (big-endian u64) and `edge_cid` out of a key
/// emitted by [`edge_index_key`]. Returns `None` on a malformed key. The
/// `pred` and `anchor` are already known by the caller (they prefix-scoped
/// the scan), so only the trailing `valid_from \0 edge_cid` is recovered.
fn decode_edge_spo_key(key: &[u8], anchor: &str, pred: &str) -> Option<(u64, String)> {
    // Skip the `anchor \0 pred \0` prefix.
    let prefix_len = anchor.len() + 1 + pred.len() + 1;
    let rest = key.get(prefix_len..)?;
    if rest.len() < 9 {
        return None;
    }
    let mut vf = [0u8; 8];
    vf.copy_from_slice(&rest[..8]);
    let valid_from = u64::from_be_bytes(vf);
    if rest[8] != 0u8 {
        return None;
    }
    let cid = std::str::from_utf8(&rest[9..]).ok()?.to_string();
    Some((valid_from, cid))
}

/// When `pred` is empty we cannot rely on a fixed-length prefix because the
/// predicate bytes are part of the key. Decode `(pred, valid_from,
/// edge_cid)` from a key scoped only by `anchor \0`. Splits on the NUL
/// bytes: `anchor \0 pred \0 valid_from_be8 \0 edge_cid`.
fn decode_edge_spo_key_anypred(key: &[u8], anchor: &str) -> Option<(String, u64, String)> {
    let after_anchor = key.get(anchor.len() + 1..)?;
    // Find the predicate terminator (first NUL).
    let pred_end = after_anchor.iter().position(|b| *b == 0u8)?;
    let pred = std::str::from_utf8(&after_anchor[..pred_end])
        .ok()?
        .to_string();
    let rest = after_anchor.get(pred_end + 1..)?;
    if rest.len() < 9 || rest[8] != 0u8 {
        return None;
    }
    let mut vf = [0u8; 8];
    vf.copy_from_slice(&rest[..8]);
    let valid_from = u64::from_be_bytes(vf);
    let cid = std::str::from_utf8(&rest[9..]).ok()?.to_string();
    Some((pred, valid_from, cid))
}

/// Persist edges idempotently. Skips any edge whose CID already lives in
/// [`TREE_EDGES`]. Writes the body to `TREE_EDGES`, a forward index row to
/// `TREE_EDGE_SPO` (`subj \0 pred \0 vf_be8 \0 cid -> obj_cid`) and a
/// reverse row to `TREE_EDGE_OPS` (`obj \0 pred \0 vf_be8 \0 cid ->
/// subj_cid`).
fn add_edges_tree(db: &sled::Db, edges: &[EdgeFact]) -> Result<Vec<EdgeCid>, StorageError> {
    let bodies = db
        .open_tree(TREE_EDGES)
        .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
    let spo = db
        .open_tree(TREE_EDGE_SPO)
        .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
    let ops = db
        .open_tree(TREE_EDGE_OPS)
        .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
    let mut out = Vec::with_capacity(edges.len());
    for e in edges {
        let cid = e.cid();
        out.push(cid.clone());
        // Idempotent: a re-submitted edge is a no-op.
        if bodies
            .contains_key(cid.as_str().as_bytes())
            .map_err(|err| StorageError::Io(std::io::Error::other(err.to_string())))?
        {
            continue;
        }
        let body = e.to_canonical_cbor();
        bodies
            .insert(cid.as_str().as_bytes(), body)
            .map_err(|err| StorageError::Io(std::io::Error::other(err.to_string())))?;
        let fkey = edge_index_key(e.subj.as_str(), &e.pred, e.valid_from, cid.as_str());
        spo.insert(fkey, e.obj.as_str().as_bytes())
            .map_err(|err| StorageError::Io(std::io::Error::other(err.to_string())))?;
        let rkey = edge_index_key(e.obj.as_str(), &e.pred, e.valid_from, cid.as_str());
        ops.insert(rkey, e.subj.as_str().as_bytes())
            .map_err(|err| StorageError::Io(std::io::Error::other(err.to_string())))?;
    }
    bodies
        .flush()
        .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
    spo.flush()
        .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
    ops.flush()
        .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
    Ok(out)
}

/// Recall edges from the forward index. See
/// [`Storage::recall_edges`] for the bi-temporal + supersession contract.
fn recall_edges_tree(
    db: &sled::Db,
    subj: &FactCid,
    pred: &str,
    as_of: Option<u64>,
    limit: usize,
) -> Result<Vec<EdgeFact>, StorageError> {
    let bodies = db
        .open_tree(TREE_EDGES)
        .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
    let spo = db
        .open_tree(TREE_EDGE_SPO)
        .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;

    // Collect (obj, valid_from, edge_cid) candidates from the index.
    let mut candidates: Vec<(String, u64, String)> = Vec::new();
    if pred.is_empty() {
        // Scan every predicate for this subject.
        let mut prefix = Vec::with_capacity(subj.as_str().len() + 1);
        prefix.extend_from_slice(subj.as_str().as_bytes());
        prefix.push(0u8);
        for row in spo.scan_prefix(prefix) {
            let (k, _obj) =
                row.map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
            if let Some((p, vf, cid)) = decode_edge_spo_key_anypred(&k, subj.as_str()) {
                let _ = p;
                candidates.push(("".into(), vf, cid));
            }
        }
    } else {
        let mut prefix = Vec::with_capacity(subj.as_str().len() + pred.len() + 2);
        prefix.extend_from_slice(subj.as_str().as_bytes());
        prefix.push(0u8);
        prefix.extend_from_slice(pred.as_bytes());
        prefix.push(0u8);
        for row in spo.scan_prefix(prefix) {
            let (k, _obj) =
                row.map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
            if let Some((vf, cid)) = decode_edge_spo_key(&k, subj.as_str(), pred) {
                candidates.push(("".into(), vf, cid));
            }
        }
    }

    // Hydrate bodies and apply the bi-temporal filter + supersession. We
    // group by the object fact CID (read off the hydrated body so the
    // any-predicate path groups correctly) and keep the edge with the
    // largest valid_from that satisfies the as_of bound.
    use std::collections::HashMap;
    let mut best: HashMap<String, EdgeFact> = HashMap::new();
    for (_obj_placeholder, _vf, cid) in candidates {
        let body = match bodies
            .get(cid.as_bytes())
            .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?
        {
            Some(b) => b,
            None => continue,
        };
        let edge: EdgeFact = ciborium::de::from_reader(&*body)
            .map_err(|e| StorageError::Cbor(format!("edge decode: {e}")))?;
        if let Some(t) = as_of {
            // valid_from must be <= as_of.
            if edge.valid_from > t {
                continue;
            }
            // Drop edges already closed at as_of: valid_to Some(vt) with vt < as_of.
            if let Some(vt) = edge.valid_to {
                if vt < t {
                    continue;
                }
            }
        }
        let group = format!("{}\0{}", edge.pred, edge.obj.as_str());
        match best.get(&group) {
            Some(existing) if existing.valid_from >= edge.valid_from => {}
            _ => {
                best.insert(group, edge);
            }
        }
    }

    let mut out: Vec<EdgeFact> = best.into_values().collect();
    // Deterministic order: ascending valid_from, then object CID.
    out.sort_by(|a, b| {
        a.valid_from
            .cmp(&b.valid_from)
            .then_with(|| a.obj.as_str().cmp(b.obj.as_str()))
    });
    out.truncate(limit);
    Ok(out)
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

/// Encode the four-field scope prefix for [`TREE_SCOPE_INDEX`] keys:
/// `user \0 agent \0 run \0 org \0`. Absent fields use the empty-string
/// sentinel (a bare NUL), consistent with how [`Scope`] canonicalises —
/// a recall scoped to `{user_id:"u1"}` produces the prefix
/// `u1 \0 \0 \0 \0` and range-scans exactly the facts written under that
/// same four-tuple.
fn scope_prefix_bytes(scope: &emem_fact::Scope) -> Vec<u8> {
    let u = scope.user_id.as_deref().unwrap_or("");
    let a = scope.agent_id.as_deref().unwrap_or("");
    let r = scope.run_id.as_deref().unwrap_or("");
    let o = scope.org_id.as_deref().unwrap_or("");
    let mut buf = Vec::with_capacity(u.len() + a.len() + r.len() + o.len() + 4);
    for part in [u, a, r, o] {
        buf.extend_from_slice(part.as_bytes());
        buf.push(0u8);
    }
    buf
}

/// Append one [`TREE_SCOPE_INDEX`] row per keyable fact:
/// `scope_prefix || cell \0 band \0 tslot_be8 -> fact_cid`. Skips
/// derivative facts (no canonical (cell, band, tslot) triple). Idempotent
/// overwrite-by-key. The scope fields lead the key so a scoped recall can
/// `scan_prefix(scope_prefix)` and walk only this tenant's facts.
fn append_scope_index(
    db: &sled::Db,
    scope: &emem_fact::Scope,
    facts: &[Fact],
    cids: &[FactCid],
) -> Result<(), StorageError> {
    if facts.is_empty() || cids.len() != facts.len() {
        return Ok(());
    }
    let tree = db
        .open_tree(TREE_SCOPE_INDEX)
        .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
    let prefix = scope_prefix_bytes(scope);
    for (f, cid) in facts.iter().zip(cids.iter()) {
        let Some(triple) = fact_canonical_key_bytes(f) else {
            continue; // derivative facts have no canonical key
        };
        let mut key = Vec::with_capacity(prefix.len() + triple.len());
        key.extend_from_slice(&prefix);
        key.extend_from_slice(&triple);
        tree.insert(key, cid.as_str().as_bytes())
            .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
    }
    tree.flush()
        .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
    Ok(())
}

/// Range-scan [`TREE_SCOPE_INDEX`] for every fact written under `scope`
/// at `cell` (optionally pinned to `tslot`). The scope prefix selects the
/// tenant; the `cell \0` segment then narrows to the cell. Returns the
/// decoded `(CanonicalKey, FactCid)` pairs in index order — same shape as
/// [`SledHotCache::scan_cell`] so callers are interchangeable.
fn scan_scope_index(
    db: &sled::Db,
    scope: &emem_fact::Scope,
    cell: &str,
    tslot: Option<u64>,
) -> Result<Vec<(CanonicalKey, FactCid)>, StorageError> {
    let tree = db
        .open_tree(TREE_SCOPE_INDEX)
        .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
    // Prefix = scope four-tuple || cell \0 — this pins both the tenant
    // and the cell so the scan touches only matching rows.
    let scope_prefix = scope_prefix_bytes(scope);
    let scope_prefix_len = scope_prefix.len();
    let mut prefix = scope_prefix;
    prefix.extend_from_slice(cell.as_bytes());
    prefix.push(0u8);
    let mut out: Vec<(CanonicalKey, FactCid)> = Vec::new();
    for kv in tree.scan_prefix(&prefix) {
        let (k, v) = kv.map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
        // Everything after the scope four-tuple is exactly the
        // `cell \0 band \0 tslot_be8` triple `fact_canonical_key_bytes`
        // emitted; decode it with the shared canonical-key decoder.
        let triple = match k.get(scope_prefix_len..) {
            Some(t) => t,
            None => continue,
        };
        let Some(key) = decode_key_bytes(triple) else {
            continue;
        };
        if let Some(t) = tslot {
            if key.tslot != t {
                continue;
            }
        }
        let cid_s = match std::str::from_utf8(&v) {
            Ok(s) => s.to_string(),
            Err(_) => continue,
        };
        out.push((key, FactCid::new(cid_s)));
    }
    Ok(out)
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
    let mut leaves: Vec<[u8; 32]> = Vec::with_capacity(att.facts.len() + att.edges.len());
    for f in &att.facts {
        let mut buf = Vec::new();
        ciborium::ser::into_writer(f, &mut buf)
            .map_err(|e| StorageError::AttestationInvalid(format!("fact cbor: {e}")))?;
        let h = blake3::hash(&buf);
        let mut a = [0u8; 32];
        a.copy_from_slice(h.as_bytes());
        leaves.push(a);
    }
    // v0.0.9 additive: fold each edge's blake3(canonical_cbor(edge)) into
    // the leaf set BEFORE sorting, but ONLY when edges are present. An
    // attestation with no edges produces the exact same leaf set, sort,
    // and root as a pre-v0.0.9 attestation, so legacy attestations verify
    // byte-identically.
    for e in &att.edges {
        leaves.push(e.blake3_digest());
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
            edges: vec![],
            batch_root: root,
            attester: AttesterKey(pk),
            attester_key_epoch: KeyEpoch(0),
            registry_cid: RegistryCid::new(registry_cid),
            schema_cid: SchemaCid::new(schema_cid),
            signature: Signature(sig_bytes),
            attested_at: "2026-05-28T00:00:00Z".into(),
            scope: None,
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

#[cfg(test)]
mod edge_tests {
    //! Temporal knowledge-graph edge persistence + bi-temporal recall,
    //! plus the back-compat guarantees: an attestation with no edges
    //! produces a byte-identical leaf set / root / signature, and edge
    //! folding only changes the root when edges are present.

    use super::*;
    use blake3::Hasher;
    use ed25519_dalek::{Signer, SigningKey};
    use emem_attest::merkle_root;
    use emem_core::{AttesterKey, KeyEpoch, Signature};
    use emem_fact::{Attestation, EdgeFact, FactCid, RegistryCid, SchemaCid};

    fn ephemeral() -> MaterializingStorage {
        let bands = Arc::new(emem_core::bands::DEFAULT.clone());
        let functions =
            Arc::new(emem_core::FunctionRegistry::parse_default().expect("default functions"));
        let sources =
            Arc::new(emem_core::SourceRegistry::parse_default().expect("default sources"));
        MaterializingStorage::ephemeral(bands, functions, sources).expect("ephemeral storage")
    }

    fn mk_edge(subj: &str, pred: &str, obj: &str, vf: u64, vt: Option<u64>) -> EdgeFact {
        EdgeFact {
            subj: FactCid::new(subj),
            pred: pred.into(),
            obj: FactCid::new(obj),
            valid_from: vf,
            valid_to: vt,
            confidence: 1.0,
            signer: AttesterKey([3u8; 32]),
            signed_at: "2026-05-29T00:00:00Z".into(),
            schema_cid: None,
            note: None,
        }
    }

    /// Sign an attestation over the given facts AND edges, folding the
    /// edge leaves into the merkle root exactly as the responder does.
    fn build_signed_with_edges(
        facts: Vec<Fact>,
        edges: Vec<EdgeFact>,
        secret: [u8; 32],
    ) -> Attestation {
        let registry_cid = "test-registry";
        let schema_cid = "test-schema";
        let signing = SigningKey::from_bytes(&secret);
        let mut pk = [0u8; 32];
        pk.copy_from_slice(signing.verifying_key().as_bytes());
        let mut leaves: Vec<[u8; 32]> = facts
            .iter()
            .map(|f| {
                let mut buf = Vec::new();
                ciborium::ser::into_writer(f, &mut buf).unwrap();
                *blake3::hash(&buf).as_bytes()
            })
            .collect();
        for e in &edges {
            leaves.push(e.blake3_digest());
        }
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
        Attestation {
            facts,
            edges,
            batch_root: root,
            attester: AttesterKey(pk),
            attester_key_epoch: KeyEpoch(0),
            registry_cid: RegistryCid::new(registry_cid),
            schema_cid: SchemaCid::new(schema_cid),
            signature: Signature(sig_bytes),
            attested_at: "2026-05-29T00:00:00Z".into(),
            scope: None,
        }
    }

    #[tokio::test]
    async fn edge_round_trip() {
        let storage = ephemeral();
        let e = mk_edge("subj-a", "replaced_by", "obj-b", 10, None);
        // CID stable across two encodings.
        assert_eq!(e.cid(), e.cid());
        let cids = storage
            .add_edges(std::slice::from_ref(&e))
            .await
            .expect("add");
        assert_eq!(cids.len(), 1);
        assert_eq!(cids[0], e.cid());
        assert!(storage.has_edge(&e.cid()).await.unwrap());
        let got = storage
            .recall_edges(&FactCid::new("subj-a"), "replaced_by", None, 100)
            .await
            .expect("recall");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0], e);
        // Empty-predicate scan finds it too.
        let any = storage
            .recall_edges(&FactCid::new("subj-a"), "", None, 100)
            .await
            .unwrap();
        assert_eq!(any.len(), 1);
        assert_eq!(any[0], e);
    }

    #[tokio::test]
    async fn edge_supersession_as_of() {
        let storage = ephemeral();
        let early = mk_edge("subj-a", "state", "obj-x", 10, None);
        let late = mk_edge("subj-a", "state", "obj-x", 20, None);
        storage
            .add_edges(&[early.clone(), late.clone()])
            .await
            .expect("add");

        // as_of=15 → only the vf=10 edge is in-window → returns vf=10.
        let at15 = storage
            .recall_edges(&FactCid::new("subj-a"), "state", Some(15), 100)
            .await
            .unwrap();
        assert_eq!(at15.len(), 1);
        assert_eq!(at15[0].valid_from, 10);

        // as_of=25 → both in-window, supersession keeps the newest vf=20.
        let at25 = storage
            .recall_edges(&FactCid::new("subj-a"), "state", Some(25), 100)
            .await
            .unwrap();
        assert_eq!(at25.len(), 1);
        assert_eq!(at25[0].valid_from, 20);

        // Non-destructive: BOTH rows still live in the SPO index.
        let spo = storage
            .hot
            .as_ref()
            .unwrap()
            .db()
            .open_tree(TREE_EDGE_SPO)
            .unwrap();
        assert_eq!(spo.len(), 2, "supersession must not delete the older row");
    }

    #[tokio::test]
    async fn edge_valid_to_closes() {
        let storage = ephemeral();
        let e = mk_edge("subj-a", "rel", "obj-y", 5, Some(18));
        storage.add_edges(&[e]).await.expect("add");
        // as_of=15 < valid_to=18 → included.
        let at15 = storage
            .recall_edges(&FactCid::new("subj-a"), "rel", Some(15), 100)
            .await
            .unwrap();
        assert_eq!(at15.len(), 1);
        // as_of=20 > valid_to=18 → closed → excluded.
        let at20 = storage
            .recall_edges(&FactCid::new("subj-a"), "rel", Some(20), 100)
            .await
            .unwrap();
        assert!(at20.is_empty(), "edge closed at valid_to must drop out");
    }

    #[tokio::test]
    async fn add_edges_is_idempotent() {
        let storage = ephemeral();
        let e = mk_edge("subj-a", "rel", "obj-z", 1, None);
        storage.add_edges(std::slice::from_ref(&e)).await.unwrap();
        storage.add_edges(std::slice::from_ref(&e)).await.unwrap();
        let spo = storage
            .hot
            .as_ref()
            .unwrap()
            .db()
            .open_tree(TREE_EDGE_SPO)
            .unwrap();
        assert_eq!(spo.len(), 1, "re-submitting the same edge is a no-op");
    }

    #[tokio::test]
    async fn legacy_attestation_still_verifies() {
        // An attestation with NO edges: its leaf set, root, and signature
        // are computed exactly as a pre-v0.0.9 attestation. verify must
        // pass and the JSON must round-trip without an `edges` key.
        let storage = ephemeral();
        let fact = mk_fact("damO.zb000.xUti.zde78", 1);
        let att = build_signed_with_edges(vec![fact], vec![], [9u8; 32]);
        // No edges → round-trips through canonical CBOR with edges == [].
        let mut buf = Vec::new();
        ciborium::ser::into_writer(&att, &mut buf).unwrap();
        let back: Attestation = ciborium::de::from_reader(buf.as_slice()).unwrap();
        assert!(back.edges.is_empty());
        // verify_attestation passes (root + signature unchanged) for both
        // the original and the round-tripped envelope.
        storage
            .put_attestation(&att)
            .await
            .expect("legacy verifies");
        verify_attestation(&back).expect("round-tripped legacy verifies");
    }

    #[tokio::test]
    async fn attestation_with_edges_verifies_and_persists() {
        let storage = ephemeral();
        let fact = mk_fact("damO.zb000.xUti.zde79", 2);
        let edge = mk_edge("subj-e", "links", "obj-f", 7, None);
        let att = build_signed_with_edges(vec![fact], vec![edge.clone()], [11u8; 32]);
        storage
            .put_attestation(&att)
            .await
            .expect("edge attestation verifies");
        // The edge was persisted via the put_attestation hook.
        assert!(storage.has_edge(&edge.cid()).await.unwrap());
        let got = storage
            .recall_edges(&FactCid::new("subj-e"), "links", None, 100)
            .await
            .unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0], edge);
    }

    /// Helper: a minimal Primary fact for attestation tests.
    fn mk_fact(cell: &str, tslot: u64) -> Fact {
        use emem_fact::{Derivation, PrimaryFact, Source};
        Fact::Primary(PrimaryFact {
            cell: cell.into(),
            band: "indices.ndvi".into(),
            tslot,
            value: ciborium::Value::Float(0.5),
            unit: None,
            confidence: 1.0,
            uncertainty: None,
            sources: vec![Source {
                scheme: "test".into(),
                id: "x".into(),
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
            signer: AttesterKey([5u8; 32]),
            signed_at: "2026-05-29T00:00:00Z".into(),
            served_via: None,
        })
    }
}
