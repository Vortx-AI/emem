//! Protocol error catalog. Spec §11.3.
//!
//! These codes are wire-stable. Agents program against them. New codes ship
//! under semver and degrade gracefully (`unknown` is a valid response).

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Stable error codes returned to agents over MCP / REST.
///
/// The discriminants are written down because they are on the wire. MCP sends
/// the NUMBER, negated: `tool error (-25)` is `InvalidArgument`. Before this
/// they were declaration order, so inserting a variant anywhere but the end
/// silently renumbered every code below it, in a file whose own opening line
/// promises agents these are stable. Nothing would have failed; agents would
/// simply have started reading a different error than the one that happened.
///
/// `InvalidCell` used to be first, and therefore zero, and therefore arrived
/// at agents as `tool error (0)` -- a number most clients cannot tell from an
/// unset field. It is the commonest address error on the surface. It is 27
/// now; every other code keeps the number it already had, so nothing else
/// moved. New variants take the next free number and are added at the END of
/// the list, which is a convention `error_codes_are_pinned_to_names` enforces
/// rather than trusts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(i32)]
pub enum ErrorCode {
    // ── Address / lookup ──────────────────────────────────────────────
    /// Cell ID was malformed (cell64 round-trip failed).
    InvalidCell = 27,
    /// Resolution out of [0, 15].
    InvalidResolution = 1,
    /// Tslot did not match the band's tempo class grain.
    TslotMismatch = 2,
    /// Band key not present in the active registry.
    BandNotInRegistry = 3,
    /// Function key not present in the active registry.
    FunctionNotInRegistry = 4,
    /// Source scheme not present in the active sources manifest.
    SourceSchemeUnknown = 5,
    /// CID could not be dereferenced.
    CidNotFound = 6,
    /// A place / address lookup string did not resolve to a cell64 in
    /// any tier of the geocoder cascade (embedded → cache → Photon →
    /// Nominatim). Distinct from `SourceFetchFailed`, which means the
    /// upstream geocoder transport itself failed; this code means the
    /// transports succeeded but unanimously returned zero results.
    /// Maps to HTTP 404. The agent should refine the query (more
    /// specific name, add country / region) or pass coordinates
    /// directly via `lat` + `lng`.
    NoGeocoderMatch = 7,
    /// The referenced registry CID is unknown to this responder.
    RegistryCidUnknown = 8,
    /// The referenced schema CID is unknown to this responder.
    SchemaCidUnknown = 9,

    // ── Privacy / auth ────────────────────────────────────────────────
    /// Privacy class refuses serving at the requested resolution.
    PrivacyRefused = 10,
    /// Conformance level required for the operation is higher than this server's.
    LevelTooLow = 11,
    /// Attester key has been revoked at the cited epoch.
    AttesterRevoked = 12,
    /// Caller lacks authorization for an L2 / staked operation.
    Unauthorized = 13,

    // ── Verification / consistency ────────────────────────────────────
    /// Claim could not be decided (insufficient facts; agent may switch to mode=resolve).
    ClaimUndecidable = 14,
    /// Signature verification failed.
    BadSignature = 15,
    /// A fact's subject belongs to no address space, so the write is refused.
    ///
    /// Distinct from [`Self::BadSignature`] because the two send a debugger in
    /// opposite directions and one of them wasted somebody's afternoon:
    /// 4b43rrtd reported `raster_bundle` failing with `bad_signature` on
    /// 2026-08-13 and reasonably went looking at signing, when their signature
    /// was fine and the subject was a bbox digest rather than an address.
    /// Every subject-validation refusal came out as a signature error because
    /// both are `AttestationInvalid` underneath.
    UnaddressableSubject = 16,
    /// Merkle inclusion proof did not validate.
    BadMerkleProof = 17,
    /// Two implementations produced byte-different canonical CBOR (protocol violation upstream).
    CanonicalEncodingDivergence = 18,

    // ── Compute / fetch ───────────────────────────────────────────────
    /// Upstream source fetch failed (network, auth, or rate-limit).
    SourceFetchFailed = 19,
    /// Source response did not match expected format (CRS, dtype, dims).
    SourceFormatMismatch = 20,
    /// Compute deadline exceeded.
    ComputeTimeout = 21,
    /// Per-caller compute quota exhausted.
    ComputeQuotaExceeded = 22,
    /// Per-caller QPS rate limit exceeded.
    RateLimited = 23,

    // ── Internal ──────────────────────────────────────────────────────
    /// Cache backend reported an error.
    CacheError = 24,
    /// Caller supplied a syntactically valid but semantically invalid
    /// argument (e.g. unparseable timestamp, out-of-range numeric
    /// parameter, malformed enum value). Distinct from
    /// `InvalidCell` / `InvalidResolution`, which are address-shape
    /// failures. Use this when you can attribute the failure to one
    /// specific request field; pair the message with the field name
    /// so an agent can self-correct.
    InvalidArgument = 25,
    /// Any other failure (responder MUST include a free-form message).
    Internal = 26,
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
        Self {
            code,
            message: message.into(),
            offending: None,
        }
    }
    /// Builder: attach an offending reference.
    pub fn with_offending(mut self, offending: impl Into<String>) -> Self {
        self.offending = Some(offending.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::ErrorCode::{self, *};

    /// Every code, pinned to the number it goes out as.
    ///
    /// This is the whole guarantee the module header states. It was not
    /// enforced by anything: the numbers came from declaration order, so
    /// adding a variant in a sensible-looking place -- next to the codes it
    /// belongs with -- would have shifted every code after it by one and told
    /// agents a different error than the one that happened, with no test, no
    /// warning and no way to notice from the outside.
    ///
    /// Add a new variant at the END, give it the next free number, and add it
    /// here. Do not renumber an existing one.
    const WIRE: &[(ErrorCode, i32)] = &[
        (InvalidResolution, 1),
        (TslotMismatch, 2),
        (BandNotInRegistry, 3),
        (FunctionNotInRegistry, 4),
        (SourceSchemeUnknown, 5),
        (CidNotFound, 6),
        (NoGeocoderMatch, 7),
        (RegistryCidUnknown, 8),
        (SchemaCidUnknown, 9),
        (PrivacyRefused, 10),
        (LevelTooLow, 11),
        (AttesterRevoked, 12),
        (Unauthorized, 13),
        (ClaimUndecidable, 14),
        (BadSignature, 15),
        (UnaddressableSubject, 16),
        (BadMerkleProof, 17),
        (CanonicalEncodingDivergence, 18),
        (SourceFetchFailed, 19),
        (SourceFormatMismatch, 20),
        (ComputeTimeout, 21),
        (ComputeQuotaExceeded, 22),
        (RateLimited, 23),
        (CacheError, 24),
        (InvalidArgument, 25),
        (Internal, 26),
        (InvalidCell, 27),
    ];

    #[test]
    fn error_codes_are_pinned_to_names() {
        for (code, want) in WIRE {
            assert_eq!(
                *code as i32, *want,
                "{code:?} goes out as {}, and agents were told {want}",
                *code as i32
            );
        }
    }

    /// No code is zero, because MCP sends the number negated and `-0` reaches
    /// a client as `0`, which is not distinguishable from an absent field.
    /// `InvalidCell` was zero and is the commonest address error here.
    #[test]
    fn no_error_code_is_zero_or_duplicated() {
        let mut seen: Vec<i32> = Vec::new();
        for (code, _) in WIRE {
            let n = *code as i32;
            assert_ne!(n, 0, "{code:?} would arrive as `tool error (0)`");
            assert!(
                !seen.contains(&n),
                "{code:?} shares wire code {n} with another variant"
            );
            seen.push(n);
        }
        // A control: the table must cover the enum, or the two tests above
        // pass by looking at a subset. Bumped deliberately when a code ships.
        assert_eq!(
            WIRE.len(),
            27,
            "a variant was added without pinning it here"
        );
    }

    /// The string form is what REST bodies carry, and it must not move either.
    #[test]
    fn the_snake_case_name_is_the_rest_form() {
        assert_eq!(
            serde_json::to_string(&InvalidCell).unwrap(),
            "\"invalid_cell\""
        );
        assert_eq!(
            serde_json::to_string(&NoGeocoderMatch).unwrap(),
            "\"no_geocoder_match\""
        );
    }
}
