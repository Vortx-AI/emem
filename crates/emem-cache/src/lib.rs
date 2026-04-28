//! emem-cache — multi-tier cache for emem facts.
//!
//! Spec §15. Three tiers, each addressed by `(cell, band, tslot)` →
//! `FactCid`, plus a CID → bytes resolver.
//!
//! ```text
//! Hot   (sled, ~30 days, sub-ms point lookups)
//!   ↓ evict on age + LRU
//! Warm  (parquet, ~90 days, columnar scans)
//!   ↓ evict on age + tier capacity
//! Cold  (IPLD/IPFS, forever, retrievable)
//! ```
//!
//! All tiers are content-addressed: the same fact CID resolves through any
//! tier. Eviction never deletes; it only changes which tier holds the bytes.
//! `Storage::get_fact(cid)` walks Hot → Warm → Cold until hit.
//!
//! The hot tier is the live, content-addressed sled store backing recall.
//! Warm (parquet) and cold (IPLD/IPFS) tiers extend the same trait surface
//! and may be layered on with `Cache` impls that delegate to lower tiers
//! on miss.

#![forbid(unsafe_code)]

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use emem_fact::{Fact, FactCid};

/// A cache tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier { Hot, Warm, Cold }

/// Lookup key for the canonical-fact index.
#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalKey {
    /// cell64 string.
    pub cell: String,
    /// Band key.
    pub band: String,
    /// Time slot.
    pub tslot: u64,
}

/// The composite multi-tier cache surface.
///
/// Implementations are async to play with axum/rmcp runtimes. Methods are
/// batch-shaped to support bootstrap throughput targets.
#[async_trait]
pub trait Cache: Send + Sync {
    /// Look up canonical fact CIDs for many keys at once. Returns one slot
    /// per input (Some on hit, None on miss).
    async fn lookup_many(&self, keys: &[CanonicalKey]) -> Result<Vec<Option<FactCid>>, CacheError>;

    /// Fetch many facts by CID. Walks Hot → Warm → Cold internally.
    async fn get_many(&self, cids: &[FactCid]) -> Result<Vec<Option<Fact>>, CacheError>;

    /// Insert many facts. Always lands in Hot tier; promoter task
    /// (background) demotes to Warm/Cold over time.
    async fn put_many(&self, facts: &[Fact]) -> Result<Vec<FactCid>, CacheError>;

    /// Tier hint for a CID without fetching it (for cost estimation in
    /// receipts).
    async fn tier_of(&self, cid: &FactCid) -> Result<Option<Tier>, CacheError>;
}

/// Cache errors.
#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    /// Underlying KV error.
    #[error("kv: {0}")]
    Kv(#[from] sled::Error),
    /// Serialization failure.
    #[error("cbor: {0}")]
    Cbor(String),
    /// Disk I/O.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

pub mod sled_hot;
pub use sled_hot::SledHotCache;
