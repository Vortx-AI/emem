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

pub mod attesters;
pub mod merkle_log;
pub mod server;

pub use attesters::{AttesterRegistry, AttesterStats};
pub use merkle_log::{AppendOutcome, AttestationLog, VerifyReport};
pub use server::Server;

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
    leaves_with_orig.sort_by(|a, b| a.0.cmp(&b.0));
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
