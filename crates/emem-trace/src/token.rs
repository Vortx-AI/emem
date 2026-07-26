//! The `emem:trace:` token — carrying execution evidence the way an
//! agent carries a fact.
//!
//! Grammar, matching the fact token's shape with one segment fewer
//! (a trace is not addressed at a cell; it is addressed at itself):
//!
//! ```text
//! emem:trace:<trace_cid>
//! ```
//!
//! where `<trace_cid>` is the 52-character base32-nopad lowercase
//! blake3 of the canonical CBOR of the signed [`crate::OsTrace`]
//! record. Resolving the token returns the byte-identical record;
//! re-running [`crate::verify_os_trace`] on it needs nothing else.

/// Token prefix, stable wire constant.
pub const TRACE_TOKEN_PREFIX: &str = "emem:trace:";

/// Compose the token for a trace CID.
pub fn trace_token(trace_cid: &str) -> String {
    format!("{TRACE_TOKEN_PREFIX}{trace_cid}")
}

/// Parse a trace token back to its CID. Returns `None` unless the
/// prefix matches and the remainder is a well-formed 52-character
/// lowercase base32 digest.
pub fn parse_trace_token(token: &str) -> Option<&str> {
    let cid = token.strip_prefix(TRACE_TOKEN_PREFIX)?;
    if cid.len() == 52
        && cid
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && crate::schema::decode_digest(cid).is_some()
    {
        Some(cid)
    } else {
        None
    }
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
    fn rejects_malformed() {
        assert_eq!(parse_trace_token("emem:fact:abc:def"), None);
        assert_eq!(parse_trace_token("emem:trace:"), None);
        assert_eq!(parse_trace_token("emem:trace:UPPERCASE"), None);
        assert_eq!(parse_trace_token("emem:trace:tooshort"), None);
    }
}
