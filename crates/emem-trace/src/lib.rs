//! emem-trace — the OS execution trace record and its verifier.
//!
//! The trust rule this crate enforces: **emem respects a device as an
//! observer of the physical world, but the protocol never accepts the
//! device's output on its own.** A satellite downlink station, a
//! telescope, a microscope, a CCTV head, a phone, a robot: each is an
//! encoder of the world, and each must present its complete, unaltered
//! OS execution trace for the capture window. Only output whose digest
//! is bound inside a verified trace is admitted as a fact.
//!
//! What "complete and unaltered" means operationally:
//!
//! - **Complete**: every trace layer the device's substrate profile
//!   requires ([`emem_core::substrates::SubstrateProfile::required_trace_layers`])
//!   is present, and the layers cover the whole capture window.
//! - **Unaltered**: segments form a digest chain (`prev_digest`), the
//!   chain's merkle root is signed by the device key together with the
//!   device identity, the window, and the emitted output digests, so no
//!   segment can be dropped, reordered, or rewritten after signing.
//!
//! The founding Earth substrate does not go through this path: its
//! admission rule is recomputability from free public archives, and it
//! serves as the drift anchor device claims are contradiction-scored
//! against ([`drift`]).
//!
//! Layering: this crate is pure (no I/O, no async), sitting beside
//! `emem-attest` in the trust stack. Ingest wiring (an attest-time gate
//! in `emem-storage`, a `/v1/trace_verify` route in `emem-api-rest`)
//! consumes it.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod drift;
pub mod schema;
pub mod verify;

pub use drift::{drift_contradiction_score, DriftAnchorCheck, DriftVerdict};
pub use schema::{
    DeviceIdentity, EmittedOutput, OsTrace, TraceBuildError, TraceSegment, OS_TRACE_SCHEMA_V1,
};
pub use verify::{verify_os_trace, Coverage, RejectReason, Verdict, VerificationReport};
