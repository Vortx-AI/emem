//! emem-primitives — the read primitives.
//!
//! Each primitive is an async function over `&Server`. Each returns a
//! response that includes a Receipt with cost+latency+freshness self-declared
//! and signed with the responder's epoch-pubkey.

#![forbid(unsafe_code)]

pub mod recall;
pub mod query_region;
pub mod compare;
pub mod compare_bands;
pub mod find_similar;
pub mod verify;
pub mod diff;
pub mod trajectory;
pub mod refinement;
pub mod cbor_ops;

pub use recall::{recall, RecallReq, RecallResp};
pub use query_region::{query_region, QueryRegionReq, QueryRegionResp};
pub use compare::{compare, CompareReq, CompareResp};
pub use compare_bands::{compare_bands, CompareBandsReq, CompareBandsResp};
pub use find_similar::{find_similar, FindSimilarReq, FindSimilarResp, Neighbor};
pub use verify::{verify, VerifyReq, VerifyResp, Mode};
pub use diff::{diff, DiffReq, DiffResp};
pub use trajectory::{trajectory, TrajectoryReq, TrajectoryResp, Point};
