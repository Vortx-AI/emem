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
pub mod diff;
pub mod find_similar;
pub mod query_region;
pub mod recall;
pub mod refinement;
pub mod trajectory;
pub mod verify;

pub use compare::{compare, CompareReq, CompareResp};
pub use compare_bands::{compare_bands, CompareBandsReq, CompareBandsResp};
pub use diff::{diff, DiffReq, DiffResp};
pub use find_similar::{find_similar, FindSimilarReq, FindSimilarResp, Neighbor};
pub use query_region::{query_region, QueryRegionReq, QueryRegionResp};
pub use recall::{recall, RecallReq, RecallResp};
pub use trajectory::{trajectory, Point, TrajectoryReq, TrajectoryResp};
pub use verify::{verify, Mode, VerifyReq, VerifyResp};
