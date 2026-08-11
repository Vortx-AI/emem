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

pub mod artifacts;
pub mod attesters;
pub mod merkle_log;
pub mod server;
pub mod trace_gate;

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
    /// Device enrollment + OS-trace gate (`docs/plans/encoder-substrates.md`).
    /// Optional for the same reason `attesters` is; when present,
    /// [`MaterializingStorage::put_attestation_gated`] enforces the
    /// trace-admission rule for enrolled device keys.
    pub trace_gate: Option<trace_gate::TraceGate>,
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

    /// The durable append-only attestation log backing this storage, when
    /// present. Used by the transparency-log surface to build signed tree
    /// heads and inclusion / consistency proofs. Default `None` for
    /// backends without a durable log (in-memory test mocks).
    fn transparency_log(&self) -> Option<&AttestationLog> {
        None
    }

    /// Resolve a stored OS trace by its content ID to the byte-identical
    /// signed record — what an `emem:trace:` token names. Default `None`
    /// for backends without a trace gate.
    fn resolve_os_trace(&self, _trace_cid: &str) -> Option<emem_trace::OsTrace> {
        None
    }

    /// Resolve a stored platform attestation by its content ID — what an
    /// `emem:attestation:` token names. Default `None`.
    fn resolve_platform_attestation(
        &self,
        _attestation_cid: &str,
    ) -> Option<emem_trace::PlatformAttestation> {
        None
    }

    /// The trace-gated write path: for an enrolled device key the
    /// attestation must arrive with its OS execution trace, which must
    /// verify against the enrolled profile with every fact's payload bound
    /// in it. For a key that was never enrolled this is exactly
    /// [`Storage::put_attestation`], so the gate cannot break an existing
    /// writer. Backends without a gate get the default: ungated, `None`.
    async fn put_attestation_gated(
        &self,
        att: &Attestation,
        _trace: Option<&emem_trace::OsTrace>,
    ) -> Result<(Vec<FactCid>, Option<trace_gate::AdmittedTrace>), StorageError> {
        Ok((self.put_attestation(att).await?, None))
    }

    /// Enrol a device key by presenting its platform attestation. Default:
    /// unsupported (no gate). See [`trace_gate::TraceGate::enroll_attested`].
    fn gate_enroll_attested(
        &self,
        _pubkey_b32: &str,
        _profile_id: &str,
        _platform_id: &str,
        _attestation: &emem_trace::PlatformAttestation,
    ) -> Result<trace_gate::EnrollmentRecord, StorageError> {
        Err(StorageError::AttestationInvalid(
            "os_trace enrolment is not supported by this storage backend".into(),
        ))
    }

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
        resolve_as_of_transaction_time(self, pairs, bound).await
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

    /// Every fact CID ever written at each canonical key, in append order,
    /// as a batched point lookup on the same index `scan_multi_attester`
    /// walks.
    ///
    /// The canonical index is last-write-wins, so it answers "what is true
    /// now" and cannot answer "what did this responder know on date Y". A
    /// third-party benchmark measured the consequence on 2026-08-11:
    /// `as_of_signed_at` bounded to a date the superseded fact was already
    /// inside returned ZERO facts, because the only candidate offered to the
    /// transaction-time filter was the CURRENT fact, whose `signed_at` is
    /// later than any bound in the past. The history was in this tree the
    /// whole time. Transaction-time history was reachable only if you already
    /// held the cid, which is exactly when you do not need a query.
    ///
    /// Returns one vector per input key, index-aligned, empty where the key
    /// has no recorded history. Default impl returns empties so backends
    /// without the index keep compiling and keep today's behaviour.
    async fn history_many(
        &self,
        keys: &[emem_cache::CanonicalKey],
    ) -> Result<Vec<Vec<FactCid>>, StorageError> {
        Ok(vec![Vec::new(); keys.len()])
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

    /// Recall edges TERMINATING at `obj` ("what points at this fact")
    /// under predicate `pred`, bi-temporally filtered by `as_of`. The
    /// mirror of [`Storage::recall_edges`]: same `pred=""` → all-predicates
    /// rule, same closed-interval `as_of` + `valid_to` filter, but
    /// supersession collapses per SUBJECT (the newest edge from each
    /// distinct subject under a predicate wins) and the scan walks the
    /// reverse [`TREE_EDGE_OPS`] index. Default impl returns `[]` so
    /// in-memory mocks and ephemeral backends keep compiling. (v0.0.9.)
    async fn recall_edges_by_obj(
        &self,
        _obj: &FactCid,
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
        let trace_gate = trace_gate::TraceGate::open(hot.db()).ok();
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
            trace_gate,
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
        let trace_gate = trace_gate::TraceGate::open(hot.db()).ok();
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
            trace_gate,
        })
    }

    /// Enrol a device key by presenting its platform attestation. Thin
    /// wrapper over [`trace_gate::TraceGate::enroll_attested`] so the trait
    /// method (and thus a REST handler) can reach it; errors if this
    /// storage has no gate. Self-service: the attestation must endorse the
    /// key and be signed by a whitelisted anchor, so a caller cannot enrol
    /// a key it has no valid evidence for.
    pub fn enroll_attested_device(
        &self,
        pubkey_b32: &str,
        profile_id: &str,
        platform_id: &str,
        attestation: &emem_trace::PlatformAttestation,
    ) -> Result<trace_gate::EnrollmentRecord, StorageError> {
        let gate = self.trace_gate.as_ref().ok_or_else(|| {
            StorageError::AttestationInvalid("os_trace gate is not enabled on this storage".into())
        })?;
        gate.enroll_attested(pubkey_b32, profile_id, platform_id, attestation)
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
    fn transparency_log(&self) -> Option<&AttestationLog> {
        Some(&self.log)
    }

    fn resolve_os_trace(&self, trace_cid: &str) -> Option<emem_trace::OsTrace> {
        self.trace_gate.as_ref()?.get_trace(trace_cid)
    }

    fn resolve_platform_attestation(
        &self,
        attestation_cid: &str,
    ) -> Option<emem_trace::PlatformAttestation> {
        self.trace_gate.as_ref()?.get_attestation(attestation_cid)
    }

    async fn put_attestation_gated(
        &self,
        att: &Attestation,
        trace: Option<&emem_trace::OsTrace>,
    ) -> Result<(Vec<FactCid>, Option<trace_gate::AdmittedTrace>), StorageError> {
        let admitted_profile = match &self.trace_gate {
            Some(gate) => gate.check(att, trace)?,
            None => None,
        };
        let cids = self.put_attestation(att).await?;
        let admitted = match (admitted_profile, trace, &self.trace_gate) {
            (Some(profile), Some(trace), Some(gate)) => {
                Some(gate.persist(trace, &cids, &profile.id)?)
            }
            _ => None,
        };
        Ok((cids, admitted))
    }

    fn gate_enroll_attested(
        &self,
        pubkey_b32: &str,
        profile_id: &str,
        platform_id: &str,
        attestation: &emem_trace::PlatformAttestation,
    ) -> Result<trace_gate::EnrollmentRecord, StorageError> {
        self.enroll_attested_device(pubkey_b32, profile_id, platform_id, attestation)
    }

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
            // The three best-effort index writes below (proof, multi-
            // attester, scope) are blocking sled operations. During a cold
            // materialize storm they run on every attestation, so — like
            // the reads and the cache writes — they must go on the blocking
            // pool rather than the async workers, or they starve the
            // runtime (the recurring wedge). sled `Db` clones cheaply; the
            // facts/cids/scope are copied into the task. The durability
            // fsync stays the async `flush_async` that follows.
            //
            // The multi-attester index preserves every distinct CID
            // attested at a (cell, band, tslot) key (the canonical index is
            // last-write-wins) so the contradictions primitive can surface
            // disagreement. The scope index (v0.0.8) writes one row per
            // keyable fact only when the attestation carries a non-empty
            // multi-tenant scope; without one, recall falls back to the
            // global canonical index and stays byte-identical to the
            // pre-v0.0.8 path.
            let db = hot.db().clone();
            let facts = att.facts.clone();
            let cids_c = cids.clone();
            let pv = att.preimage_version;
            let scope = att.scope.clone();
            let idx_writes = tokio::task::spawn_blocking(move || {
                if let Err(e) = persist_fact_proofs(&db, &facts, &cids_c, pv) {
                    tracing::warn!(error=%e, "fact proof persistence error (ignored)");
                }
                if let Err(e) = append_multi_attester(&db, &facts, &cids_c) {
                    tracing::warn!(error=%e, "multi-attester index append error (ignored)");
                }
                if let Some(scope) = scope.as_ref().filter(|sc| !sc.is_empty()) {
                    if let Err(e) = append_scope_index(&db, scope, &facts, &cids_c) {
                        tracing::warn!(error=%e, "scope index append error (ignored)");
                    }
                }
            })
            .await;
            if let Err(e) = idx_writes {
                tracing::warn!(error=%e, "index-write task join error (ignored)");
            }
            // One fsync makes the proof + multi-attester + scope rows above
            // durable. sled flushes the whole Db, so the per-helper flushes
            // were three redundant fsyncs per cold write; this single async
            // flush replaces them off the runtime thread. Best-effort, like
            // the index writes it backs — the facts themselves are already
            // durable via the cache + merkle log above.
            if let Err(e) = hot.db().flush_async().await {
                tracing::warn!(error=%e, "index flush error (ignored)");
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
        // Off the reactor: this index scan is a blocking sled operation and
        // was a prime mover of the runtime-wedge under recall storms.
        Ok(hot.scan_cell_off(cell, tslot).await?)
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
        let pairs = hot
            .scan_cell_with_tslot_bound_off(cell, tslot, bound.valid_time)
            .await?;
        if bound.transaction_time.is_none() {
            return Ok(pairs);
        }
        // Transaction-time predicate requires loading bodies for `signed_at`,
        // and must consider each key's history rather than only the fact that
        // is current — see `resolve_as_of_transaction_time`.
        resolve_as_of_transaction_time(self, pairs, bound).await
    }

    async fn iter_index(
        &self,
        limit: Option<usize>,
    ) -> Result<Vec<(CanonicalKey, FactCid)>, StorageError> {
        let hot = self.hot.as_ref().ok_or_else(|| StorageError::Protocol {
            code: ErrorCode::Internal,
            message: "iter_index requires a SledHotCache handle".into(),
        })?;
        // A corpus-wide index scan is a heavy blocking sled operation; run
        // it on the blocking pool rather than the async workers.
        Ok(hot.collect_index_off(limit).await?)
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

    async fn history_many(
        &self,
        keys: &[emem_cache::CanonicalKey],
    ) -> Result<Vec<Vec<FactCid>>, StorageError> {
        let Some(hot) = self.hot.as_ref() else {
            return Ok(vec![Vec::new(); keys.len()]);
        };
        let tree = hot
            .db()
            .open_tree(TREE_MULTI_ATTESTER_INDEX)
            .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
        let mut out = Vec::with_capacity(keys.len());
        for k in keys {
            let key_bytes = canonical_key_bytes(k);
            let cids: Vec<FactCid> = match tree
                .get(&key_bytes)
                .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?
            {
                Some(b) => ciborium::de::from_reader::<Vec<String>, _>(&*b)
                    .map_err(|e| StorageError::Cbor(format!("history decode: {e}")))?
                    .into_iter()
                    .map(FactCid::new)
                    .collect(),
                None => Vec::new(),
            };
            out.push(cids);
        }
        Ok(out)
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

    async fn recall_edges_by_obj(
        &self,
        obj: &FactCid,
        pred: &str,
        as_of: Option<u64>,
        limit: usize,
    ) -> Result<Vec<EdgeFact>, StorageError> {
        let hot = self.hot.as_ref().ok_or_else(|| StorageError::Protocol {
            code: ErrorCode::Internal,
            message: "recall_edges_by_obj requires a SledHotCache handle".into(),
        })?;
        recall_edges_by_obj_tree(hot.db(), obj, pred, as_of, limit)
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
    // One flush persists all three edge trees: sled fsyncs the whole Db,
    // so the separate spo/ops flushes were two redundant fsyncs on every
    // edge write. add_edges still returns only after a durable write, so
    // its contract (callers rely on it) is unchanged.
    bodies
        .flush()
        .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
    Ok(out)
}

/// Which end of the edge anchors a scan. `Subj` walks [`TREE_EDGE_SPO`]
/// ("what does this fact point at"); `Obj` walks [`TREE_EDGE_OPS`] ("what
/// points at this fact"). The two share the SAME bi-temporal `as_of` +
/// `valid_to` filter and supersession discipline (factored into
/// [`scan_edges_anchored`]) so the forward and reverse reads can never
/// drift apart — they differ only in (a) which index tree they prefix-scan
/// and (b) the group key supersession collapses on.
#[derive(Clone, Copy)]
enum EdgeAnchor {
    /// Scan by subject — forward index, group/supersede per object.
    Subj,
    /// Scan by object — reverse index, group/supersede per subject.
    Obj,
}

impl EdgeAnchor {
    fn tree(&self) -> &'static str {
        match self {
            EdgeAnchor::Subj => TREE_EDGE_SPO,
            EdgeAnchor::Obj => TREE_EDGE_OPS,
        }
    }
    /// The supersession group key for a hydrated edge: forward collapses
    /// per `(pred, obj)`, reverse per `(pred, subj)`. Either way the key
    /// fixes the predicate and the *other* end so the newest `valid_from`
    /// wins among edges that mean the same relation to the same neighbour.
    fn group_key(&self, edge: &EdgeFact) -> String {
        match self {
            EdgeAnchor::Subj => format!("{}\0{}", edge.pred, edge.obj.as_str()),
            EdgeAnchor::Obj => format!("{}\0{}", edge.pred, edge.subj.as_str()),
        }
    }
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
    scan_edges_anchored(db, EdgeAnchor::Subj, subj.as_str(), pred, as_of, limit)
}

/// Recall edges TERMINATING at `obj` from the reverse index
/// ([`TREE_EDGE_OPS`]). See [`Storage::recall_edges_by_obj`] for the
/// contract. Shares [`scan_edges_anchored`] with the forward path so the
/// `as_of` / `valid_to` boundary and supersession rule are byte-for-byte
/// the same filter — only the anchor end differs.
fn recall_edges_by_obj_tree(
    db: &sled::Db,
    obj: &FactCid,
    pred: &str,
    as_of: Option<u64>,
    limit: usize,
) -> Result<Vec<EdgeFact>, StorageError> {
    scan_edges_anchored(db, EdgeAnchor::Obj, obj.as_str(), pred, as_of, limit)
}

/// Shared scan: prefix-walk the index tree named by `anchor` for
/// `anchor_cid` (optionally narrowed to one `pred`, `""` = all preds),
/// hydrate each edge body from [`TREE_EDGES`], apply the bi-temporal
/// `as_of` + `valid_to` filter, collapse by the anchor's supersession
/// group key keeping the largest `valid_from`, and return up to `limit`
/// edges in ascending `(valid_from, obj_cid)` order.
fn scan_edges_anchored(
    db: &sled::Db,
    anchor: EdgeAnchor,
    anchor_cid: &str,
    pred: &str,
    as_of: Option<u64>,
    limit: usize,
) -> Result<Vec<EdgeFact>, StorageError> {
    let bodies = db
        .open_tree(TREE_EDGES)
        .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
    let index = db
        .open_tree(anchor.tree())
        .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;

    // Collect the candidate edge CIDs from the index. The index keys are
    // laid out identically for both trees (`anchor \0 pred \0 vf_be8 \0
    // edge_cid`), so the same decoders work for the SPO and OPS scans.
    let mut candidate_cids: Vec<String> = Vec::new();
    if pred.is_empty() {
        let mut prefix = Vec::with_capacity(anchor_cid.len() + 1);
        prefix.extend_from_slice(anchor_cid.as_bytes());
        prefix.push(0u8);
        for row in index.scan_prefix(prefix) {
            let (k, _v) =
                row.map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
            if let Some((_p, _vf, cid)) = decode_edge_spo_key_anypred(&k, anchor_cid) {
                candidate_cids.push(cid);
            }
        }
    } else {
        let mut prefix = Vec::with_capacity(anchor_cid.len() + pred.len() + 2);
        prefix.extend_from_slice(anchor_cid.as_bytes());
        prefix.push(0u8);
        prefix.extend_from_slice(pred.as_bytes());
        prefix.push(0u8);
        for row in index.scan_prefix(prefix) {
            let (k, _v) =
                row.map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
            if let Some((_vf, cid)) = decode_edge_spo_key(&k, anchor_cid, pred) {
                candidate_cids.push(cid);
            }
        }
    }

    // Hydrate bodies and apply the bi-temporal filter + supersession. We
    // group by the anchor-specific key (read off the hydrated body so the
    // any-predicate path groups correctly) and keep the edge with the
    // largest valid_from that satisfies the as_of bound.
    use std::collections::HashMap;
    let mut best: HashMap<String, EdgeFact> = HashMap::new();
    for cid in candidate_cids {
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
        let group = anchor.group_key(&edge);
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
    // Durability is forced once by `put_attestation` (this fn's only
    // caller) after all index writes — sled fsyncs the whole Db, so a
    // flush here too was a redundant fsync on every write.
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
    // Durability is forced once by `put_attestation` (this fn's only
    // caller) after all index writes — sled fsyncs the whole Db, so a
    // flush here too was a redundant fsync on every write.
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


/// Resolve each `(key, current_cid)` pair to the latest fact satisfying a
/// transaction-time bound, considering the key's WHOLE recorded history.
///
/// Three call sites used to run their own copy of "load the current fact,
/// keep it if `signed_at` passes". That answers the wrong question. The
/// canonical index is last-write-wins, so the only candidate it offers is the
/// NEWEST fact, whose `signed_at` is later than any bound in the past: a
/// bi-temporal query therefore returned nothing at every bound, including
/// bounds the superseded fact was comfortably inside. Measured by a
/// third-party benchmark on 2026-08-11 (D6). The history was in
/// `TREE_MULTI_ATTESTER_INDEX` the whole time.
///
/// The current cid is always a candidate, so a backend whose `history_many`
/// returns empties (the trait default) keeps exactly its previous behaviour.
/// Ties on `signed_at` break on the lexicographic cid, which is deterministic
/// across responders.
pub async fn resolve_as_of_transaction_time<S: Storage + ?Sized>(
    storage: &S,
    pairs: Vec<(CanonicalKey, FactCid)>,
    bound: &AsOfBound,
) -> Result<Vec<(CanonicalKey, FactCid)>, StorageError> {
    if pairs.is_empty() {
        return Ok(pairs);
    }
    let keys: Vec<CanonicalKey> = pairs.iter().map(|(k, _)| k.clone()).collect();
    let histories = storage.history_many(&keys).await?;

    let mut candidates: Vec<(usize, FactCid)> = Vec::new();
    for (i, ((_, current), history)) in pairs.iter().zip(histories.iter()).enumerate() {
        let mut seen: Vec<&str> = Vec::with_capacity(history.len() + 1);
        for cid in history.iter().chain(std::iter::once(current)) {
            if seen.contains(&cid.as_str()) {
                continue;
            }
            seen.push(cid.as_str());
            candidates.push((i, cid.clone()));
        }
    }
    let cids: Vec<FactCid> = candidates.iter().map(|(_, c)| c.clone()).collect();
    let facts = storage.get_facts_many(&cids).await?;

    let mut best: Vec<Option<(String, FactCid)>> = vec![None; pairs.len()];
    for ((i, cid), fact) in candidates.into_iter().zip(facts) {
        let Some(f) = fact else { continue };
        if !bound.fact_passes(&f) {
            continue;
        }
        let signed_at = match &f {
            Fact::Primary(p) => p.signed_at.clone(),
            Fact::Absence(a) => a.signed_at.clone(),
            Fact::Derivative(_) => continue,
        };
        let replace = match &best[i] {
            Some((s, c)) => signed_at > *s || (signed_at == *s && cid.as_str() > c.as_str()),
            None => true,
        };
        if replace {
            best[i] = Some((signed_at, cid));
        }
    }
    Ok(pairs
        .into_iter()
        .zip(best)
        .filter_map(|((k, _), b)| b.map(|(_, cid)| (k, cid)))
        .collect())
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

/// Encode a `CanonicalKey` the same way, for point lookups into the index
/// that `fact_canonical_key_bytes` writes. The two must agree byte for byte
/// or a history lookup silently misses.
fn canonical_key_bytes(k: &emem_cache::CanonicalKey) -> Vec<u8> {
    let mut buf = Vec::with_capacity(k.cell.len() + k.band.len() + 10);
    buf.extend_from_slice(k.cell.as_bytes());
    buf.push(0u8);
    buf.extend_from_slice(k.band.as_bytes());
    buf.push(0u8);
    buf.extend_from_slice(&k.tslot.to_be_bytes());
    buf
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
    preimage_version: u8,
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
    // Build proofs under the same merkle rule the attestation's root was
    // computed with, so a verifier re-deriving the root from the proof
    // path lands on `att.batch_root`.
    let (root, paths) = if preimage_version >= emem_attest::PREIMAGE_V1 {
        emem_attest::merkle_root_and_paths_v1(&leaves)
    } else {
        emem_attest::merkle_root_and_paths(&leaves)
    };
    let proof_version = if preimage_version >= emem_attest::PREIMAGE_V1 {
        emem_attest::PREIMAGE_V1
    } else {
        0
    };
    let tree = db
        .open_tree(TREE_FACT_PROOFS)
        .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
    for (sorted_idx, (_, orig_idx)) in leaves_with_orig.iter().enumerate() {
        let cid = &cids[*orig_idx];
        let proof = MerkleProof {
            leaf_index: sorted_idx as u32,
            path: paths[sorted_idx].clone(),
            root,
            version: proof_version,
        };
        let mut buf = Vec::new();
        ciborium::ser::into_writer(&proof, &mut buf)
            .map_err(|e| StorageError::Cbor(format!("fact_proofs cbor: {e}")))?;
        tree.insert(cid.as_str().as_bytes(), buf)
            .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
    }
    // Durability is forced once by `put_attestation` (this fn's only
    // caller) after all index writes — sled fsyncs the whole Db, so a
    // flush here too was a redundant fsync on every write.
    Ok(())
}

/// Verify an attestation envelope. The hashing rule is selected by
/// `att.preimage_version`:
///
/// * `0` (legacy): leaves and nodes hashed without prefixes; signature
///   over `blake3(batch_root || registry_cid || schema_cid)`.
/// * `1`: RFC 6962-style 0x00/0x01 leaf/node prefixes
///   ([`emem_attest::merkle_root_v1`]); duplicate leaves rejected
///   (root-equivocation guard); signature over
///   [`emem_attest::attestation_preimage_v1`].
///
/// Both paths recompute the leaf set the same way (canonical-CBOR blake3
/// per fact, plus each edge digest, bytewise-sorted) and confirm the
/// recomputed root matches `att.batch_root` before checking the
/// signature — so a forged root or a tampered fact is caught regardless
/// of version.
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

    let root = if att.preimage_version >= emem_attest::PREIMAGE_V1 {
        // v1 rejects duplicate leaves: with the duplicate-last fold,
        // root([A,B,C]) == root([A,B,C,C]), so a duplicate would let an
        // attester equivocate over which fact set a root commits to.
        if emem_attest::has_adjacent_duplicate(&leaves) {
            return Err(StorageError::AttestationInvalid(
                "duplicate fact/edge leaf in attestation batch".into(),
            ));
        }
        emem_attest::merkle_root_v1(&leaves)
    } else {
        emem_attest::merkle_root(&leaves)
    };
    if root != att.batch_root {
        return Err(StorageError::AttestationInvalid(format!(
            "merkle root mismatch: computed={} declared={}",
            hex32(&root),
            hex32(&att.batch_root)
        )));
    }

    let msg: [u8; 32] = if att.preimage_version >= emem_attest::PREIMAGE_V1 {
        emem_attest::attestation_preimage_v1(
            &att.batch_root,
            att.registry_cid.as_str(),
            att.schema_cid.as_str(),
        )
    } else {
        let mut h = Hasher::new();
        h.update(&att.batch_root);
        h.update(att.registry_cid.as_str().as_bytes());
        h.update(att.schema_cid.as_str().as_bytes());
        *h.finalize().as_bytes()
    };

    let pk = ed25519_dalek::VerifyingKey::from_bytes(&att.attester.0)
        .map_err(|e| StorageError::AttestationInvalid(format!("bad attester key: {e}")))?;
    let sig = ed25519_dalek::Signature::from_bytes(&att.signature.0);
    pk.verify_strict(&msg, &sig)
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
            preimage_version: 0,
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
            preimage_version: 0,
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

    /// Reverse lookup (`obj -> subj`): write `A relates_to B`, then ask
    /// "what points at B" via the OPS index. The forward and reverse reads
    /// must agree on the same edge, and the reverse path must honour the
    /// SAME bi-temporal `as_of` + supersession contract.
    #[tokio::test]
    async fn reverse_lookup_round_trip() {
        let storage = ephemeral();
        let e = mk_edge("subj-a", "relates_to", "obj-b", 10, None);
        storage
            .add_edges(std::slice::from_ref(&e))
            .await
            .expect("add");

        // recall_edges_by_obj(B) returns the edge with subj=A.
        let got = storage
            .recall_edges_by_obj(&FactCid::new("obj-b"), "relates_to", None, 100)
            .await
            .expect("reverse recall");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0], e);
        assert_eq!(got[0].subj.as_str(), "subj-a");

        // Empty-predicate reverse scan finds it too.
        let any = storage
            .recall_edges_by_obj(&FactCid::new("obj-b"), "", None, 100)
            .await
            .unwrap();
        assert_eq!(any.len(), 1);
        assert_eq!(any[0], e);

        // A different object yields nothing (honest empty, not a leak).
        let none = storage
            .recall_edges_by_obj(&FactCid::new("obj-other"), "", None, 100)
            .await
            .unwrap();
        assert!(none.is_empty());
    }

    /// Reverse-path supersession honours `as_of` the same way the forward
    /// path does: two edges from the SAME subject to the same object under
    /// the same predicate, newer `valid_from` shadows the older when both
    /// are in-window; only the older is visible before the newer begins.
    #[tokio::test]
    async fn reverse_lookup_supersession_as_of() {
        let storage = ephemeral();
        let early = mk_edge("subj-a", "state", "obj-x", 10, None);
        let late = mk_edge("subj-a", "state", "obj-x", 20, None);
        storage
            .add_edges(&[early.clone(), late.clone()])
            .await
            .expect("add");

        // as_of=15 → only vf=10 is in-window.
        let at15 = storage
            .recall_edges_by_obj(&FactCid::new("obj-x"), "state", Some(15), 100)
            .await
            .unwrap();
        assert_eq!(at15.len(), 1);
        assert_eq!(at15[0].valid_from, 10);

        // as_of=25 → both in-window; supersession (per subject) keeps vf=20.
        let at25 = storage
            .recall_edges_by_obj(&FactCid::new("obj-x"), "state", Some(25), 100)
            .await
            .unwrap();
        assert_eq!(at25.len(), 1);
        assert_eq!(at25[0].valid_from, 20);

        // Non-destructive: both rows still live in the reverse OPS index.
        let ops = storage
            .hot
            .as_ref()
            .unwrap()
            .db()
            .open_tree(TREE_EDGE_OPS)
            .unwrap();
        assert_eq!(ops.len(), 2, "reverse supersession must not delete rows");
    }

    /// Forward `recall_edges` bi-temporal boundary at `as_of == valid_to`.
    /// The impl (`scan_edges_anchored`) drops an edge only when
    /// `valid_to Some(vt)` with `vt < as_of` (line ~1156), so the closed
    /// interval is INCLUSIVE of `valid_to`: an edge with `valid_to = Some(18)`
    /// is still visible at `as_of == 18`, visible at 17, and gone at 19.
    /// This pins the exact boundary the storage layer implements (the mock
    /// the audit flagged diverged here).
    #[tokio::test]
    async fn forward_recall_edges_valid_to_boundary() {
        let storage = ephemeral();
        let e = mk_edge("subj-bound", "rel", "obj-bound", 5, Some(18));
        storage.add_edges(std::slice::from_ref(&e)).await.unwrap();

        // as_of = 17 (< valid_to) → included.
        let at17 = storage
            .recall_edges(&FactCid::new("subj-bound"), "rel", Some(17), 100)
            .await
            .unwrap();
        assert_eq!(at17.len(), 1, "as_of < valid_to is in-window");

        // as_of == valid_to == 18 → INCLUDED (closed interval, vt < as_of is
        // the drop rule, so vt == as_of survives).
        let at18 = storage
            .recall_edges(&FactCid::new("subj-bound"), "rel", Some(18), 100)
            .await
            .unwrap();
        assert_eq!(
            at18.len(),
            1,
            "as_of == valid_to must be INCLUDED (drop rule is vt < as_of)"
        );

        // as_of = 19 (> valid_to) → closed → excluded.
        let at19 = storage
            .recall_edges(&FactCid::new("subj-bound"), "rel", Some(19), 100)
            .await
            .unwrap();
        assert!(at19.is_empty(), "as_of > valid_to drops the closed edge");
    }

    /// Forward supersession: two edges with the SAME (subj,pred,obj) but
    /// different `valid_from`. At a given `as_of` only the latest in-window
    /// `valid_from` survives the group-by-collapse, while the earlier edge is
    /// the only one visible before the newer one begins. (Complements the
    /// reverse-path `reverse_lookup_supersession_as_of` and the existing
    /// forward `edge_supersession_as_of`, exercising the boundary at exactly
    /// the newer `valid_from`.)
    #[tokio::test]
    async fn forward_supersession_boundary_at_valid_from() {
        let storage = ephemeral();
        let early = mk_edge("subj-s", "state", "obj-s", 10, None);
        let late = mk_edge("subj-s", "state", "obj-s", 20, None);
        storage
            .add_edges(&[early.clone(), late.clone()])
            .await
            .unwrap();

        // as_of == 19 → late (vf=20) not yet begun → only early visible.
        let at19 = storage
            .recall_edges(&FactCid::new("subj-s"), "state", Some(19), 100)
            .await
            .unwrap();
        assert_eq!(at19.len(), 1);
        assert_eq!(at19[0].valid_from, 10, "before vf=20 begins, early wins");

        // as_of == 20 → both in-window (vf <= as_of) → supersession keeps late.
        let at20 = storage
            .recall_edges(&FactCid::new("subj-s"), "state", Some(20), 100)
            .await
            .unwrap();
        assert_eq!(at20.len(), 1);
        assert_eq!(
            at20[0].valid_from, 20,
            "at exactly vf=20, the newer edge supersedes"
        );
    }

    /// Edge self-loop: subj == obj. Pins the impl's ACTUAL behavior — the
    /// storage layer treats subject and object as opaque CIDs and applies no
    /// self-loop guard, so a self-loop is stored and recallable from both the
    /// forward (SPO) and reverse (OPS) indexes. This is intentional: the edge
    /// layer is a general temporal KG and self-edges (e.g. a fact that
    /// supersedes a prior version of itself) are legitimate.
    #[tokio::test]
    async fn edge_self_loop_is_stored_and_recallable() {
        let storage = ephemeral();
        let e = mk_edge("node-x", "supersedes", "node-x", 3, None);
        let cids = storage
            .add_edges(std::slice::from_ref(&e))
            .await
            .expect("add self-loop");
        assert_eq!(cids.len(), 1, "self-loop is accepted, not rejected");
        assert!(storage.has_edge(&e.cid()).await.unwrap());

        // Recallable in the forward direction.
        let fwd = storage
            .recall_edges(&FactCid::new("node-x"), "supersedes", None, 100)
            .await
            .unwrap();
        assert_eq!(fwd.len(), 1, "self-loop visible via SPO");
        assert_eq!(fwd[0], e);

        // Recallable in the reverse direction (same node as object).
        let rev = storage
            .recall_edges_by_obj(&FactCid::new("node-x"), "supersedes", None, 100)
            .await
            .unwrap();
        assert_eq!(rev.len(), 1, "self-loop visible via OPS");
        assert_eq!(rev[0], e);
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

#[cfg(test)]
mod trace_gate_tests {
    //! End-to-end: an enrolled device key cannot write without its OS
    //! trace, writes with a sound trace that binds its payload, and
    //! never-enrolled keys keep the ungated path byte-for-byte.

    use super::*;
    use blake3::Hasher;
    use ed25519_dalek::{Signer, SigningKey};
    use emem_attest::merkle_root;
    use emem_core::substrates::TraceLayerKind;
    use emem_core::{AttesterKey, KeyEpoch, Signature};
    use emem_fact::{Attestation, Derivation, Fact, PrimaryFact, RegistryCid, SchemaCid, Source};
    use emem_trace::{DeviceIdentity, EmittedOutput, OsTrace, TraceSegment};

    fn ephemeral() -> MaterializingStorage {
        let bands = Arc::new(emem_core::bands::DEFAULT.clone());
        let functions =
            Arc::new(emem_core::FunctionRegistry::parse_default().expect("default functions"));
        let sources =
            Arc::new(emem_core::SourceRegistry::parse_default().expect("default sources"));
        MaterializingStorage::ephemeral(bands, functions, sources).expect("ephemeral storage")
    }

    fn mk_fact(value: f64, signer_pk: [u8; 32]) -> Fact {
        Fact::Primary(PrimaryFact {
            cell: "damO.zb000.xUti.zde78".into(),
            band: "indices.ndvi".into(),
            tslot: 12,
            value: ciborium::Value::Float(value),
            unit: None,
            confidence: 1.0,
            uncertainty: None,
            sources: vec![Source {
                scheme: "test".into(),
                id: "robot-obs".into(),
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
            signed_at: "2026-07-26T00:00:00Z".into(),
            served_via: None,
        })
    }

    fn build_signed(facts: Vec<Fact>, secret: [u8; 32]) -> Attestation {
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
        leaves.sort();
        let root = merkle_root(&leaves);
        let mut h = Hasher::new();
        h.update(&root);
        h.update(registry_cid.as_bytes());
        h.update(schema_cid.as_bytes());
        let sig = signing.sign(h.finalize().as_bytes());
        let mut sig_bytes = [0u8; 64];
        sig_bytes.copy_from_slice(&sig.to_bytes());
        Attestation {
            facts,
            edges: vec![],
            batch_root: root,
            attester: AttesterKey(pk),
            attester_key_epoch: KeyEpoch(0),
            registry_cid: RegistryCid::new(registry_cid),
            schema_cid: SchemaCid::new(schema_cid),
            signature: Signature(sig_bytes),
            attested_at: "2026-07-26T00:00:00Z".into(),
            scope: None,
            preimage_version: 0,
        }
    }

    /// A registered encoding that can capture `layer` — so a fixture trace
    /// passes the gate's layer-consistency check.
    fn enc_for_layer(layer: TraceLayerKind) -> &'static str {
        match layer {
            TraceLayerKind::SensorBus | TraceLayerKind::Signal => "ros2.bag.v2",
            TraceLayerKind::Energy | TraceLayerKind::Thermal => "linux.hwmon.v1",
            TraceLayerKind::Inference => "nvidia.nsys.v1",
            _ => "linux.ftrace.v1",
        }
    }

    /// A robot.fleet.v1 trace whose emitted output binds `payload`.
    fn mk_trace(sk: &SigningKey, payload_digest: &str) -> OsTrace {
        mk_trace_chained(sk, payload_digest, None, 1_000)
    }

    /// Like `mk_trace` but links `prev` (the previous window's trace CID)
    /// and takes a distinct window start so consecutive windows differ.
    fn mk_trace_chained(
        sk: &SigningKey,
        payload_digest: &str,
        prev: Option<String>,
        window_start: u64,
    ) -> OsTrace {
        let layers = [
            TraceLayerKind::Syscall,
            TraceLayerKind::Scheduler,
            TraceLayerKind::Memory,
            TraceLayerKind::SensorBus,
            TraceLayerKind::Energy,
            TraceLayerKind::Thermal,
            TraceLayerKind::Inference,
        ];
        let segments: Vec<TraceSegment> = layers
            .iter()
            .enumerate()
            .map(|(i, l)| TraceSegment {
                layer: *l,
                seq: 0,
                clock_start_ns: window_start + i as u64,
                clock_end_ns: window_start + 9_000,
                event_count: 7,
                log_digest: data_encoding::BASE32_NOPAD
                    .encode(blake3::hash(format!("log {i}").as_bytes()).as_bytes())
                    .to_lowercase(),
                prev_digest: None,
                encoding: enc_for_layer(*l).into(),
            })
            .collect();
        OsTrace::build_and_sign_chained_v1(
            DeviceIdentity {
                device_key: AttesterKey(sk.verifying_key().to_bytes()),
                key_epoch: KeyEpoch(0),
                substrate_profile: "robot.fleet.v1".into(),
                platform: "jetson-orin-nx".into(),
                os: "ubuntu-24.04".into(),
                kernel: "6.8.0-tegra".into(),
                boot_id: "b7c1e2d3".into(),
            },
            window_start,
            window_start + 10_000,
            segments,
            vec![EmittedOutput {
                payload_digest: payload_digest.into(),
                band: Some("indices.ndvi".into()),
                emitted_at_ns: window_start + 8_500,
                layer: TraceLayerKind::SensorBus,
            }],
            prev,
            sk,
        )
        .expect("build trace")
    }

    #[tokio::test]
    async fn enrolled_device_needs_a_binding_trace() {
        let storage = ephemeral();
        let gate = storage.trace_gate.as_ref().expect("gate").clone();

        let mut sec = [0u8; 32];
        sec[0] = 7;
        let sk = SigningKey::from_bytes(&sec);
        let pk = sk.verifying_key().to_bytes();
        let pk_b32 = data_encoding::BASE32_NOPAD.encode(&pk).to_lowercase();
        gate.enroll(&pk_b32, "robot.fleet.v1").expect("enroll");

        let fact = mk_fact(0.42, pk);
        let att = build_signed(vec![fact.clone()], sec);

        // 1. No trace: rejected, nothing stored.
        let err = storage
            .put_attestation_gated(&att, None)
            .await
            .expect_err("must reject");
        assert!(err.to_string().contains("none was presented"), "{err}");

        // 2. Sound trace that never emitted this payload: rejected.
        let other = mk_trace(&sk, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let err = storage
            .put_attestation_gated(&att, Some(&other))
            .await
            .expect_err("must reject unbound payload");
        assert!(err.to_string().contains("not bound"), "{err}");

        // 3. Trace binding the fact's payload digest: admitted, stored,
        //    and audit-linked.
        let payload = match &fact {
            Fact::Primary(p) => emem_trace::payload_digest_of_value(&p.value).unwrap(),
            _ => unreachable!(),
        };
        let trace = mk_trace(&sk, &payload);
        let (cids, admitted) = storage
            .put_attestation_gated(&att, Some(&trace))
            .await
            .expect("admit");
        let admitted = admitted.expect("admitted trace record");
        assert_eq!(cids.len(), 1);
        assert_eq!(admitted.profile_id, "robot.fleet.v1");
        assert!(admitted.token.starts_with("emem:trace:"));
        assert_eq!(
            gate.trace_for_fact(&cids[0]),
            Some(admitted.trace_cid.clone())
        );
        let stored = gate.get_trace(&admitted.trace_cid).expect("stored trace");
        assert_eq!(stored.trace_cid().unwrap(), admitted.trace_cid);
        // The Storage-trait resolver (what /v1/trace_resolve calls) sees it too.
        let via_trait =
            Storage::resolve_os_trace(&storage, &admitted.trace_cid).expect("resolve via trait");
        assert_eq!(via_trait.trace_cid().unwrap(), admitted.trace_cid);
    }

    #[tokio::test]
    async fn unenrolled_writers_are_untouched() {
        let storage = ephemeral();
        let mut sec = [0u8; 32];
        sec[0] = 9;
        let pk = SigningKey::from_bytes(&sec).verifying_key().to_bytes();
        let att = build_signed(vec![mk_fact(0.85, pk)], sec);
        let (cids, admitted) = storage
            .put_attestation_gated(&att, None)
            .await
            .expect("ungated write");
        assert_eq!(cids.len(), 1);
        assert!(admitted.is_none());
    }

    #[tokio::test]
    async fn enrollment_refuses_the_archive_profile() {
        let storage = ephemeral();
        let gate = storage.trace_gate.as_ref().expect("gate");
        let err = gate
            .enroll("somekey", "earth.satellite.v0")
            .expect_err("archive profile is not enrollable");
        assert!(err.to_string().contains("not trace-admitted"), "{err}");
        assert_eq!(gate.enrolled_count(), 0);
    }

    #[tokio::test]
    async fn derivative_facts_are_not_a_side_door() {
        // An enrolled device presenting a perfectly sound trace still
        // cannot write a derivative fact: a derivative is a claim about
        // facts, not an emission of a sensor, and no traced-derivation
        // rule exists yet.
        let storage = ephemeral();
        let gate = storage.trace_gate.as_ref().expect("gate");
        let mut sec = [0u8; 32];
        sec[0] = 11;
        let sk = SigningKey::from_bytes(&sec);
        let pk = sk.verifying_key().to_bytes();
        let pk_b32 = data_encoding::BASE32_NOPAD.encode(&pk).to_lowercase();
        gate.enroll(&pk_b32, "robot.fleet.v1").expect("enroll");

        let deriv = Fact::Derivative(emem_fact::DerivativeFact {
            cell: "damO.zb000.xUti.zde78".into(),
            band: "indices.ndvi".into(),
            tslot_window: [10, 12],
            op: "mean".into(),
            parents: vec![],
            value: ciborium::Value::Float(0.5),
            confidence: 1.0,
            derivation: Derivation {
                fn_key: "test@1".into(),
                args: None,
            },
            schema_cid: SchemaCid::new("test-schema"),
            signer: AttesterKey(pk),
            signed_at: "2026-07-26T00:00:00Z".into(),
        });
        let att = build_signed(vec![deriv], sec);
        let payload = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let trace = mk_trace(&sk, payload);
        let err = storage
            .put_attestation_gated(&att, Some(&trace))
            .await
            .expect_err("derivative must be refused");
        assert!(err.to_string().contains("primary facts only"), "{err}");
    }

    #[tokio::test]
    async fn operator_asserted_enrollment_is_labeled_as_such() {
        // The migration-safe legacy path still works, and a reader can see
        // it was the operator's assertion, not a hardware attestation.
        let storage = ephemeral();
        let gate = storage.trace_gate.as_ref().expect("gate");
        gate.enroll("somekey", "robot.fleet.v1").expect("enroll");
        let rec = gate.enrollment_of("somekey").expect("record");
        assert_eq!(rec.profile_id, "robot.fleet.v1");
        assert_eq!(rec.platform_id, None);
        assert_eq!(rec.assurance(), "operator_asserted");
        assert!(gate.evidence_of("somekey").is_none());
    }

    #[tokio::test]
    async fn attested_enrollment_is_refused_while_anchors_are_provisional() {
        // The safe property of this stage end to end: enroll_attested with
        // the real (all-provisional) whitelist admits nothing, and the
        // refusal names why. The device is never enrolled.
        let storage = ephemeral();
        let gate = storage.trace_gate.as_ref().expect("gate");

        let device = SigningKey::from_bytes(&[7u8; 32]);
        let dk = AttesterKey(device.verifying_key().to_bytes());
        let dk_b32 = data_encoding::BASE32_NOPAD.encode(&dk.0).to_lowercase();
        let endorser = SigningKey::from_bytes(&[3u8; 32]);
        let att = emem_trace::PlatformAttestation::build_and_sign_v0(
            "nvidia.jetson-orin",
            dk,
            "jetson-orin-nx",
            "nvidia",
            "nonce-1",
            vec![],
            &endorser,
        );
        let err = gate
            .enroll_attested(&dk_b32, "robot.fleet.v1", "nvidia.jetson-orin", &att)
            .expect_err("provisional anchors admit nothing");
        assert!(
            err.to_string().contains("no effective trust anchor"),
            "{err}"
        );
        assert!(gate.enrollment_of(&dk_b32).is_none());
    }

    /// Like `mk_trace` but every segment carries `encoding`, for exercising
    /// the gate's trace-encodings enforcement.
    fn mk_trace_enc(sk: &SigningKey, payload_digest: &str, encoding: &str) -> OsTrace {
        let layers = [
            TraceLayerKind::Syscall,
            TraceLayerKind::Scheduler,
            TraceLayerKind::Memory,
            TraceLayerKind::SensorBus,
            TraceLayerKind::Energy,
            TraceLayerKind::Thermal,
            TraceLayerKind::Inference,
        ];
        let segments: Vec<TraceSegment> = layers
            .iter()
            .enumerate()
            .map(|(i, l)| TraceSegment {
                layer: *l,
                seq: 0,
                clock_start_ns: 1_000 + i as u64,
                clock_end_ns: 9_000,
                event_count: 7,
                log_digest: data_encoding::BASE32_NOPAD
                    .encode(blake3::hash(format!("log {i}").as_bytes()).as_bytes())
                    .to_lowercase(),
                prev_digest: None,
                // First segment carries the encoding under test; the rest
                // use a per-layer valid encoding so only the injected one
                // is exercised against the gate.
                encoding: if i == 0 {
                    encoding.into()
                } else {
                    enc_for_layer(*l).into()
                },
            })
            .collect();
        OsTrace::build_and_sign_v1(
            DeviceIdentity {
                device_key: AttesterKey(sk.verifying_key().to_bytes()),
                key_epoch: KeyEpoch(0),
                substrate_profile: "robot.fleet.v1".into(),
                platform: "jetson-orin-nx".into(),
                os: "ubuntu-24.04".into(),
                kernel: "6.8.0-tegra".into(),
                boot_id: "b7c1e2d3".into(),
            },
            1_000,
            10_000,
            segments,
            vec![EmittedOutput {
                payload_digest: payload_digest.into(),
                band: Some("indices.ndvi".into()),
                emitted_at_ns: 8_500,
                layer: TraceLayerKind::SensorBus,
            }],
            sk,
        )
        .expect("build trace")
    }

    #[tokio::test]
    async fn trace_naming_an_unregistered_encoding_is_refused() {
        // The trace-of-the-trace check: an enrolled device presenting a
        // sound, signed trace whose segments name a capture encoding the
        // registry does not define is refused, even though the signature
        // and chain are perfect.
        let storage = ephemeral();
        let gate = storage.trace_gate.as_ref().expect("gate").clone();
        let mut sec = [0u8; 32];
        sec[0] = 21;
        let sk = SigningKey::from_bytes(&sec);
        let pk = sk.verifying_key().to_bytes();
        let pk_b32 = data_encoding::BASE32_NOPAD.encode(&pk).to_lowercase();
        gate.enroll(&pk_b32, "robot.fleet.v1").expect("enroll");

        let fact = mk_fact(0.42, pk);
        let att = build_signed(vec![fact.clone()], sec);
        let payload = match &fact {
            Fact::Primary(p) => emem_trace::payload_digest_of_value(&p.value).unwrap(),
            _ => unreachable!(),
        };
        // Rejections first, while the device has no stream head yet (a
        // rejected trace never advances it), so each is evaluated on the
        // encoding, not on chain continuity.
        let bad = mk_trace_enc(&sk, &payload, "totally.made.up.v9");
        let err = storage
            .put_attestation_gated(&att, Some(&bad))
            .await
            .expect_err("unregistered encoding must be refused");
        assert!(
            err.to_string().contains("unregistered capture encoding"),
            "{err}"
        );

        // A registered encoding used for a layer it cannot produce is also
        // refused: the first (Syscall) segment gets hwmon, which only
        // captures energy/thermal.
        let wrong_layer = mk_trace_enc(&sk, &payload, "linux.hwmon.v1");
        let err = storage
            .put_attestation_gated(&att, Some(&wrong_layer))
            .await
            .expect_err("encoding-layer mismatch must be refused");
        assert!(err.to_string().contains("cannot capture"), "{err}");

        // A registered, layer-consistent encoding admits.
        let good = mk_trace_enc(&sk, &payload, "linux.ftrace.v1");
        storage
            .put_attestation_gated(&att, Some(&good))
            .await
            .expect("registered encoding admits");
    }

    #[tokio::test]
    async fn device_trace_stream_must_chain() {
        // Streaming: consecutive per-window traces from a device form a
        // chain the gate enforces. The first has no prev; each next must
        // name the previous; a gap or a wrong link is refused.
        let storage = ephemeral();
        let gate = storage.trace_gate.as_ref().expect("gate").clone();
        let mut sec = [0u8; 32];
        sec[0] = 31;
        let sk = SigningKey::from_bytes(&sec);
        let pk = sk.verifying_key().to_bytes();
        let pk_b32 = data_encoding::BASE32_NOPAD.encode(&pk).to_lowercase();
        gate.enroll(&pk_b32, "robot.fleet.v1").expect("enroll");

        let fact = mk_fact(0.42, pk);
        let att = build_signed(vec![fact.clone()], sec);
        let payload = match &fact {
            Fact::Primary(p) => emem_trace::payload_digest_of_value(&p.value).unwrap(),
            _ => unreachable!(),
        };

        // Window 0: the stream head (no prev). Admitted; advances the head.
        let a = mk_trace_chained(&sk, &payload, None, 1_000);
        let (_c, admitted_a) = storage
            .put_attestation_gated(&att, Some(&a))
            .await
            .expect("head admits");
        let a_cid = admitted_a.expect("admitted a").trace_cid;
        assert_eq!(
            gate.stream_head(&pk_b32, "b7c1e2d3").as_deref(),
            Some(a_cid.as_str())
        );

        // Window 1: chains to window 0. Admitted; advances the head.
        let b = mk_trace_chained(&sk, &payload, Some(a_cid.clone()), 20_000);
        let (_c, admitted_b) = storage
            .put_attestation_gated(&att, Some(&b))
            .await
            .expect("chained window admits");
        let b_cid = admitted_b.expect("admitted b").trace_cid;
        assert_eq!(
            gate.stream_head(&pk_b32, "b7c1e2d3").as_deref(),
            Some(b_cid.as_str())
        );

        // A gap: a fresh head (no prev) after the stream started is refused.
        let gap = mk_trace_chained(&sk, &payload, None, 30_000);
        let err = storage
            .put_attestation_gated(&att, Some(&gap))
            .await
            .expect_err("a headless window mid-stream must be refused");
        assert!(err.to_string().contains("continuity broken"), "{err}");

        // A wrong link: naming a prev that is not the current head is refused.
        let wrong = mk_trace_chained(&sk, &payload, Some(a_cid.clone()), 40_000);
        let err = storage
            .put_attestation_gated(&att, Some(&wrong))
            .await
            .expect_err("a wrong prev link must be refused");
        assert!(err.to_string().contains("continuity broken"), "{err}");
        // The head did not move on either refusal.
        assert_eq!(
            gate.stream_head(&pk_b32, "b7c1e2d3").as_deref(),
            Some(b_cid.as_str())
        );
    }

    #[tokio::test]
    async fn attested_enrollment_refuses_a_platform_that_does_not_serve_the_class() {
        // A microscope profile cannot be enrolled through an Orin: the
        // platform does not serve that contributor class. This is refused
        // before the attestation is even appraised.
        let storage = ephemeral();
        let gate = storage.trace_gate.as_ref().expect("gate");
        let device = SigningKey::from_bytes(&[8u8; 32]);
        let dk = AttesterKey(device.verifying_key().to_bytes());
        let dk_b32 = data_encoding::BASE32_NOPAD.encode(&dk.0).to_lowercase();
        let endorser = SigningKey::from_bytes(&[3u8; 32]);
        let att = emem_trace::PlatformAttestation::build_and_sign_v0(
            "nvidia.jetson-orin",
            dk,
            "jetson-orin-nx",
            "nvidia",
            "n",
            vec![],
            &endorser,
        );
        let err = gate
            .enroll_attested(&dk_b32, "lab.microscope.v1", "nvidia.jetson-orin", &att)
            .expect_err("orin does not serve microscope");
        assert!(err.to_string().contains("does not serve"), "{err}");
    }
}
