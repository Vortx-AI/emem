//! emem-primitives — the read primitives.
//!
//! Each primitive is an async function over `&Server`. Each returns a
//! response that includes a Receipt with cost+latency+freshness self-declared
//! and signed with the responder's epoch-pubkey.

#![forbid(unsafe_code)]

pub mod binary_embedding;
pub mod cbor_ops;
pub mod compare;
pub mod compare_bands;
pub mod derive;
pub mod diff;
pub mod edges;
pub mod entity;
pub mod find_similar;
pub mod lance_index;
pub mod memory_acl;
pub mod memory_bundle;
pub mod memory_consolidation;
pub mod memory_contradictions;
pub mod memory_events;
pub mod memory_search;
pub mod memory_typing;
pub mod query_region;
pub mod recall;
pub mod refinement;
pub mod trajectory;
pub mod verify;

pub use compare::{compare, CompareReq, CompareResp};
pub use compare_bands::{compare_bands, CompareBandsReq, CompareBandsResp};
pub use derive::{
    is_caller_provenance_class, DeriveBody, CALLER_PROVENANCE_CLASSES, DERIVE_PATH, DERIVE_VERB,
};
pub use diff::{diff, DiffReq, DiffResp};
pub use edges::{edges_recall, EdgesRecallReq, EdgesRecallResp};
pub use find_similar::{find_similar, FindSimilarReq, FindSimilarResp, Neighbor};
pub use lance_index::{IndexStats, LanceIndex, PerDimStats};
pub use memory_acl::{
    attester_preimage, body_hash, namespace_is_reserved, namespace_ownership_ok,
    namespace_requires_attester, pubkey_short_from_b32, rename_body_hash, verify_attester,
    AttestationVerdict, MemoryAttester, BY_ATTESTER_PREFIX, PUBKEY_SHORT_LEN, RESERVED_PREFIX,
};
pub use memory_contradictions::{
    memory_contradictions, AttesterDisagreement, Contradiction, ContradictionsReq,
    ContradictionsResp,
};
pub use memory_events::{MemoryEvent, MemoryEventFilter};
pub use memory_search::{
    memory_search, MemoryFileSource, MemoryFileSummary, MemoryIndexStats, MemorySearchError,
    MemorySearchHit, MemorySearchReq, MemorySearchResp, MemoryTextIndex,
};
pub use memory_typing::{ttl_days_for_kind, MemoryKind};
pub use query_region::{query_region, QueryRegionReq, QueryRegionResp};
pub use recall::{recall, RecallReq, RecallResp};
pub use trajectory::{trajectory, Point, TrajectoryReq, TrajectoryResp};
pub use verify::{verify, Mode, VerifyReq, VerifyResp};
