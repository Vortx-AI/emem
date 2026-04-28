//! Protocol error catalog. Spec §11.3.
//!
//! These codes are wire-stable. Agents program against them. New codes ship
//! under semver and degrade gracefully (`unknown` is a valid response).

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Stable error codes returned to agents over MCP / REST.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    // ── Address / lookup ──────────────────────────────────────────────
    /// Cell ID was malformed (cell64 round-trip failed).
    InvalidCell,
    /// Resolution out of [0, 15].
    InvalidResolution,
    /// Tslot did not match the band's tempo class grain.
    TslotMismatch,
    /// Band key not present in the active registry.
    BandNotInRegistry,
    /// Function key not present in the active registry.
    FunctionNotInRegistry,
    /// Source scheme not present in the active sources manifest.
    SourceSchemeUnknown,
    /// CID could not be dereferenced.
    CidNotFound,
    /// The referenced registry CID is unknown to this responder.
    RegistryCidUnknown,
    /// The referenced schema CID is unknown to this responder.
    SchemaCidUnknown,

    // ── Privacy / auth ────────────────────────────────────────────────
    /// Privacy class refuses serving at the requested resolution.
    PrivacyRefused,
    /// Conformance level required for the operation is higher than this server's.
    LevelTooLow,
    /// Attester key has been revoked at the cited epoch.
    AttesterRevoked,
    /// Caller lacks authorization for an L2 / staked operation.
    Unauthorized,

    // ── Verification / consistency ────────────────────────────────────
    /// Claim could not be decided (insufficient facts; agent may switch to mode=resolve).
    ClaimUndecidable,
    /// Signature verification failed.
    BadSignature,
    /// Merkle inclusion proof did not validate.
    BadMerkleProof,
    /// Two implementations produced byte-different canonical CBOR (protocol violation upstream).
    CanonicalEncodingDivergence,

    // ── Compute / fetch ───────────────────────────────────────────────
    /// Upstream source fetch failed (network, auth, or rate-limit).
    SourceFetchFailed,
    /// Source response did not match expected format (CRS, dtype, dims).
    SourceFormatMismatch,
    /// Compute deadline exceeded.
    ComputeTimeout,
    /// Per-caller compute quota exhausted.
    ComputeQuotaExceeded,
    /// Per-caller QPS rate limit exceeded.
    RateLimited,

    // ── Internal ──────────────────────────────────────────────────────
    /// Cache backend reported an error.
    CacheError,
    /// Any other failure (responder MUST include a free-form message).
    Internal,
}

/// Structured top-level error. Wraps the code + a human-readable message +
/// optional pointer to the offending CID/cell/etc.
#[derive(Debug, Error, Serialize, Deserialize)]
#[error("{code:?}: {message}")]
pub struct Error {
    /// Stable error code.
    pub code: ErrorCode,
    /// Free-form message; safe to log; not parsed by agents.
    pub message: String,
    /// Optional reference (CID, cell64, function key) for the offending object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offending: Option<String>,
}

impl Error {
    /// Build a new error.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), offending: None }
    }
    /// Builder: attach an offending reference.
    pub fn with_offending(mut self, offending: impl Into<String>) -> Self {
        self.offending = Some(offending.into());
        self
    }
}
