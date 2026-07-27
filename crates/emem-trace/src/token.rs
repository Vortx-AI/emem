//! The OS-tracing token grammar — carrying execution evidence and
//! enrolment evidence the way an agent carries a fact.
//!
//! Two content-addressed tokens sit beside the fact / bundle / entity /
//! cell tokens, each `emem:<type>:<cid>` where the CID is the
//! 52-character base32-nopad lowercase blake3 of the canonical CBOR of
//! the signed record it names:
//!
//! ```text
//! emem:trace:<trace_cid>              the signed OsTrace record
//! emem:attestation:<attestation_cid>  the signed PlatformAttestation
//! ```
//!
//! A trace is addressed at itself, not at a cell, so the grammar carries
//! one segment fewer than a fact token. Resolving `emem:trace:` returns
//! the byte-identical [`crate::OsTrace`]; re-running
//! [`crate::verify_os_trace`] on it needs nothing else. Resolving
//! `emem:attestation:` returns the byte-identical
//! [`crate::PlatformAttestation`]; re-running
//! [`crate::verify_platform_attestation`] against the named platform's
//! whitelist needs nothing else. Together they let a device fact cite,
//! in two tokens, both the execution that produced it and the hardware
//! root of trust that vouches for the device.

/// Trace token prefix, stable wire constant.
pub const TRACE_TOKEN_PREFIX: &str = "emem:trace:";

/// Platform-attestation token prefix, stable wire constant.
pub const ATTESTATION_TOKEN_PREFIX: &str = "emem:attestation:";

/// Compose the token for a trace CID.
pub fn trace_token(trace_cid: &str) -> String {
    format!("{TRACE_TOKEN_PREFIX}{trace_cid}")
}

/// Compose the token for a platform-attestation CID.
pub fn attestation_token(attestation_cid: &str) -> String {
    format!("{ATTESTATION_TOKEN_PREFIX}{attestation_cid}")
}

/// Whether `cid` is a well-formed emem CID: 52 lowercase base32 digits
/// that decode to a 32-byte digest.
fn is_cid(cid: &str) -> bool {
    cid.len() == 52
        && cid
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && crate::schema::decode_digest(cid).is_some()
}

/// Parse a trace token back to its CID. Returns `None` unless the prefix
/// matches and the remainder is a well-formed CID.
pub fn parse_trace_token(token: &str) -> Option<&str> {
    let cid = token.strip_prefix(TRACE_TOKEN_PREFIX)?;
    is_cid(cid).then_some(cid)
}

/// Parse a platform-attestation token back to its CID. Returns `None`
/// unless the prefix matches and the remainder is a well-formed CID.
pub fn parse_attestation_token(token: &str) -> Option<&str> {
    let cid = token.strip_prefix(ATTESTATION_TOKEN_PREFIX)?;
    is_cid(cid).then_some(cid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let cid = crate::schema::render_digest(blake3::hash(b"trace").as_bytes());
        let tok = trace_token(&cid);
        assert_eq!(parse_trace_token(&tok), Some(cid.as_str()));
    }

    #[test]
    fn attestation_round_trip() {
        let cid = crate::schema::render_digest(blake3::hash(b"attestation").as_bytes());
        let tok = attestation_token(&cid);
        assert_eq!(parse_attestation_token(&tok), Some(cid.as_str()));
        // The two token types never parse as one another.
        assert_eq!(parse_trace_token(&tok), None);
        assert_eq!(parse_attestation_token(&trace_token(&cid)), None);
    }

    #[test]
    fn rejects_malformed() {
        assert_eq!(parse_trace_token("emem:fact:abc:def"), None);
        assert_eq!(parse_trace_token("emem:trace:"), None);
        assert_eq!(parse_trace_token("emem:trace:UPPERCASE"), None);
        assert_eq!(parse_trace_token("emem:trace:tooshort"), None);
        assert_eq!(parse_attestation_token("emem:attestation:"), None);
        assert_eq!(parse_attestation_token("emem:attestation:tooshort"), None);
    }
}
