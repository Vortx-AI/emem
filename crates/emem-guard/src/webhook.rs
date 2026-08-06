//! Standard Webhooks signature verification for inbound inference-hook
//! requests.
//!
//! This is the only thing standing between "Anthropic asked us for a verdict"
//! and "anyone on the internet asked us for a verdict", so it is written to
//! the spec's letter and tested against its own stated failure modes rather
//! than against our own output.
//!
//! The spec (platform.claude.com/docs/en/manage-claude/inference-hooks-endpoint)
//! names two details that "cause most verification bugs", and both are
//! encoded here as behaviour plus a test:
//!
//!   1. The HMAC covers the body EXACTLY as received. Parsing JSON and
//!      re-encoding it changes bytes (key order, whitespace, unicode escapes)
//!      and the signature stops matching, so this module never sees a parsed
//!      value: it takes `&[u8]`.
//!   2. The secret is STANDARD base64, not URL-safe. A URL-safe decoder
//!      derives the wrong key whenever the secret contains `+` or `/`, which
//!      is most of the time, and the failure looks like "Anthropic sent a bad
//!      signature" rather than "we decoded the key wrong".

use base64::Engine as _;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

/// How far the `webhook-timestamp` may sit from our clock, in seconds.
///
/// The spec says to reject beyond five minutes "in either direction": ahead
/// as well as behind, because a clock that disagrees is as much a problem as
/// a replay.
pub const TOLERANCE_SECONDS: i64 = 300;

/// Why a request was not accepted as signed by Anthropic.
///
/// Distinct variants rather than a bool because the operator response differs:
/// a missing header is an unsigned caller, a stale timestamp is a replay or a
/// clock problem, and a bad secret is our own misconfiguration. Collapsing
/// them to "invalid" is how a config error gets investigated as an attack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyError {
    /// One of the three `webhook-*` headers is absent. Per the spec this is
    /// simply not a request from Anthropic.
    Unsigned,
    /// `webhook-timestamp` is not a decimal integer.
    MalformedTimestamp,
    /// Outside [`TOLERANCE_SECONDS`] of our clock, in either direction.
    StaleTimestamp {
        /// Seconds between the signature and our clock; negative means the
        /// signature is in the future.
        skew_s: i64,
    },
    /// The configured secret is not decodable standard base64. Ours to fix,
    /// not the caller's.
    MalformedSecret,
    /// Headers present and fresh, but no candidate signature matched.
    NoSignatureMatched,
}

impl core::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unsigned => write!(
                f,
                "request carries no webhook-id / webhook-timestamp / webhook-signature headers"
            ),
            Self::MalformedTimestamp => write!(f, "webhook-timestamp is not a decimal integer"),
            Self::StaleTimestamp { skew_s } => write!(
                f,
                "webhook-timestamp is {skew_s}s from this server's clock, outside the {TOLERANCE_SECONDS}s tolerance"
            ),
            Self::MalformedSecret => write!(
                f,
                "the configured signing secret is not standard base64 after the whsec_ prefix"
            ),
            Self::NoSignatureMatched => write!(f, "no candidate signature matched"),
        }
    }
}

/// The three signature headers, looked up case-insensitively by the caller.
///
/// Borrowed rather than owned: the caller already holds them, and copying a
/// signature into a new allocation on every request buys nothing on a path
/// with a 5-second budget for the entire exchange.
#[derive(Debug, Clone, Copy)]
pub struct SignatureHeaders<'a> {
    /// `webhook-id`. Equals the body's `request_id`; also the idempotency key.
    pub id: &'a str,
    /// `webhook-timestamp`, unix seconds as a decimal string.
    pub timestamp: &'a str,
    /// `webhook-signature`: one or more space-separated `v1,<base64>` values.
    pub signature: &'a str,
}

impl<'a> SignatureHeaders<'a> {
    /// Pull the three headers out of anything that can look one up.
    ///
    /// Case-insensitive because the spec says Anthropic sends them lowercase
    /// "and proxies are free to re-case them". A server that matches exactly
    /// works until it is put behind a proxy that title-cases headers, and
    /// then rejects every request as unsigned.
    pub fn from_lookup(get: impl Fn(&str) -> Option<&'a str>) -> Option<Self> {
        Some(Self {
            id: get("webhook-id")?,
            timestamp: get("webhook-timestamp")?,
            signature: get("webhook-signature")?,
        })
    }
}

/// Decode a signing secret into HMAC key bytes.
///
/// The `whsec_` prefix is stripped and the remainder decoded with the
/// STANDARD base64 alphabet. See the module docs for why the alphabet
/// matters.
fn key_bytes(secret: &str) -> Result<Vec<u8>, VerifyError> {
    let raw = secret.strip_prefix("whsec_").unwrap_or(secret);
    base64::engine::general_purpose::STANDARD
        .decode(raw)
        .map_err(|_| VerifyError::MalformedSecret)
}

/// The signature we expect for this (id, timestamp, body) under `secret`,
/// rendered as the `v1,<base64>` form that appears in the header.
fn expected_signature(
    secret: &str,
    id: &str,
    timestamp: &str,
    body: &[u8],
) -> Result<String, VerifyError> {
    let key = key_bytes(secret)?;
    // `new_from_slice` on Hmac accepts any key length, so this cannot fail
    // for a decodable secret.
    let mut mac = <Hmac<Sha256>>::new_from_slice(&key).map_err(|_| VerifyError::MalformedSecret)?;
    mac.update(id.as_bytes());
    mac.update(b".");
    mac.update(timestamp.as_bytes());
    mac.update(b".");
    // The body goes in as received. See the module docs.
    mac.update(body);
    Ok(format!(
        "v1,{}",
        base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
    ))
}

/// Verify an inbound request against one or more accepted secrets.
///
/// `secrets` is a slice because secret rotation is an immediate cutover on
/// Anthropic's side, and the spec warns that requests signed with the
/// PREVIOUS secret keep arriving "for about a minute afterwards, plus
/// anything already in flight". A server that holds one secret drops those
/// on the floor, and a dropped request is a webhook failure, which under
/// fail-open policy means an uninspected prompt reaches the model. Accepting
/// both during the switchover is what makes rotation a non-event.
///
/// `now_unix_s` is a parameter rather than read from the clock so the
/// tolerance window is testable without sleeping or mocking time.
pub fn verify(
    secrets: &[String],
    headers: SignatureHeaders<'_>,
    body: &[u8],
    now_unix_s: i64,
) -> Result<(), VerifyError> {
    let signed_at: i64 = headers
        .timestamp
        .parse()
        .map_err(|_| VerifyError::MalformedTimestamp)?;
    let skew = now_unix_s - signed_at;
    if skew.abs() > TOLERANCE_SECONDS {
        return Err(VerifyError::StaleTimestamp { skew_s: skew });
    }

    // Every secret is tried against every candidate, and the result is
    // accumulated rather than returned early, so the work does not depend on
    // WHICH secret or WHICH candidate matched. Returning on first match would
    // leak, through timing, how far down the list the right secret sits.
    let mut matched = subtle::Choice::from(0u8);
    let mut malformed_secret = true;
    for secret in secrets {
        let Ok(expected) = expected_signature(secret, headers.id, headers.timestamp, body) else {
            continue;
        };
        malformed_secret = false;
        for candidate in headers.signature.split_whitespace() {
            // ct_eq over unequal lengths is false without comparing content,
            // which is the correct answer and not a leak: the length of a
            // base64 HMAC is fixed and public.
            if candidate.len() == expected.len() {
                matched |= candidate.as_bytes().ct_eq(expected.as_bytes());
            }
        }
    }
    if malformed_secret {
        return Err(VerifyError::MalformedSecret);
    }
    if bool::from(matched) {
        Ok(())
    } else {
        Err(VerifyError::NoSignatureMatched)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The spec's own worked example: a secret containing `+` and `/`.
    ///
    /// This is the case the docs single out, because a URL-safe decoder
    /// derives different key bytes and every signature then fails to match.
    /// The bug presents as "Anthropic is sending bad signatures", so it is
    /// pinned rather than left to review.
    const SECRET_WITH_URLSAFE_CHARS: &str =
        "whsec_YWJjK2RlZi9naGlqa2xtbm9wcXJzdHV2d3h5ejAxMjM0NTY3ODk=";

    fn sign(secret: &str, id: &str, ts: &str, body: &[u8]) -> String {
        expected_signature(secret, id, ts, body).expect("secret decodes")
    }

    #[test]
    fn a_genuine_signature_verifies() {
        let body = br#"{"type":"prompt","request_id":"req_abc123"}"#;
        let sig = sign(SECRET_WITH_URLSAFE_CHARS, "req_abc123", "1000", body);
        let h = SignatureHeaders {
            id: "req_abc123",
            timestamp: "1000",
            signature: &sig,
        };
        assert_eq!(
            verify(&[SECRET_WITH_URLSAFE_CHARS.to_string()], h, body, 1000),
            Ok(())
        );
    }

    /// The signed payload is `{id}.{timestamp}.{body}`, so changing any of
    /// the three must break it. Asserting only "a good signature passes"
    /// would also pass for an implementation that ignores its inputs.
    #[test]
    fn every_component_is_bound_into_the_signature() {
        let body = br#"{"type":"prompt"}"#;
        let sig = sign(SECRET_WITH_URLSAFE_CHARS, "req_1", "1000", body);
        let secrets = vec![SECRET_WITH_URLSAFE_CHARS.to_string()];

        // Wrong id.
        let h = SignatureHeaders {
            id: "req_2",
            timestamp: "1000",
            signature: &sig,
        };
        assert_eq!(
            verify(&secrets, h, body, 1000),
            Err(VerifyError::NoSignatureMatched)
        );

        // Wrong timestamp (still inside tolerance, so this is the signature
        // failing rather than the freshness check).
        let h = SignatureHeaders {
            id: "req_1",
            timestamp: "1001",
            signature: &sig,
        };
        assert_eq!(
            verify(&secrets, h, body, 1000),
            Err(VerifyError::NoSignatureMatched)
        );

        // Body altered by one byte.
        let h = SignatureHeaders {
            id: "req_1",
            timestamp: "1000",
            signature: &sig,
        };
        assert_eq!(
            verify(&secrets, h, br#"{"type":"prompts"}"#, 1000),
            Err(VerifyError::NoSignatureMatched)
        );
    }

    /// Re-encoding the body before hashing is the other bug the spec names.
    /// A JSON round trip is semantically identical and byte-different, and
    /// the signature must follow the bytes.
    #[test]
    fn a_semantically_identical_reencoding_does_not_verify() {
        let received = br#"{"b":1,"a":2}"#;
        let sig = sign(SECRET_WITH_URLSAFE_CHARS, "req_1", "1000", received);
        let reencoded =
            serde_json::to_vec(&serde_json::from_slice::<serde_json::Value>(received).unwrap())
                .unwrap();
        assert_ne!(
            received.as_slice(),
            reencoded.as_slice(),
            "the round trip must actually differ"
        );
        let h = SignatureHeaders {
            id: "req_1",
            timestamp: "1000",
            signature: &sig,
        };
        assert_eq!(
            verify(
                &[SECRET_WITH_URLSAFE_CHARS.to_string()],
                h,
                &reencoded,
                1000
            ),
            Err(VerifyError::NoSignatureMatched),
            "hashing a re-encoded body is the classic failure; it must not silently pass"
        );
    }

    /// Rotation is an immediate cutover with stragglers signed by the old
    /// secret. Both must verify while both are configured, or rotation drops
    /// requests, and a dropped request is a webhook failure rather than a
    /// deny.
    #[test]
    fn both_secrets_verify_during_a_rotation() {
        let old = "whsec_b2xkc2VjcmV0MDEyMzQ1Njc4OWFiY2RlZmdoaWprbG1ub3A=".to_string();
        let new = "whsec_bmV3c2VjcmV0MDEyMzQ1Njc4OWFiY2RlZmdoaWprbG1ub3A=".to_string();
        let body = br#"{"type":"prompt"}"#;
        let both = vec![new.clone(), old.clone()];

        for secret in [&old, &new] {
            let sig = sign(secret, "req_1", "1000", body);
            let h = SignatureHeaders {
                id: "req_1",
                timestamp: "1000",
                signature: &sig,
            };
            assert_eq!(
                verify(&both, h, body, 1000),
                Ok(()),
                "straggler on {secret} must verify"
            );
        }

        // And a third, unrelated secret still does not.
        let other = "whsec_b3RoZXJzZWNyZXQwMTIzNDU2Nzg5YWJjZGVmZ2hpamtsbW4=";
        let sig = sign(other, "req_1", "1000", body);
        let h = SignatureHeaders {
            id: "req_1",
            timestamp: "1000",
            signature: &sig,
        };
        assert_eq!(
            verify(&both, h, body, 1000),
            Err(VerifyError::NoSignatureMatched)
        );
    }

    /// The header may carry several space-separated candidates; any match
    /// accepts.
    #[test]
    fn any_of_several_candidate_signatures_may_match() {
        let body = br#"{"type":"prompt"}"#;
        let good = sign(SECRET_WITH_URLSAFE_CHARS, "req_1", "1000", body);
        let joined = format!("v1,AAAA {good} v1,BBBB");
        let h = SignatureHeaders {
            id: "req_1",
            timestamp: "1000",
            signature: &joined,
        };
        assert_eq!(
            verify(&[SECRET_WITH_URLSAFE_CHARS.to_string()], h, body, 1000),
            Ok(())
        );
    }

    /// Stale in either direction, and the error carries the skew so an
    /// operator can tell a replay from a clock that drifted.
    #[test]
    fn the_freshness_window_is_symmetric() {
        let body = br#"{"type":"prompt"}"#;
        let secrets = vec![SECRET_WITH_URLSAFE_CHARS.to_string()];
        for (ts, now, want_skew) in [("1000", 1000 + 301, 301), ("1000", 1000 - 301, -301)] {
            let sig = sign(SECRET_WITH_URLSAFE_CHARS, "req_1", ts, body);
            let h = SignatureHeaders {
                id: "req_1",
                timestamp: ts,
                signature: &sig,
            };
            assert_eq!(
                verify(&secrets, h, body, now),
                Err(VerifyError::StaleTimestamp { skew_s: want_skew })
            );
        }
        // Exactly at the boundary is still accepted.
        let sig = sign(SECRET_WITH_URLSAFE_CHARS, "req_1", "1000", body);
        let h = SignatureHeaders {
            id: "req_1",
            timestamp: "1000",
            signature: &sig,
        };
        assert_eq!(verify(&secrets, h, body, 1000 + TOLERANCE_SECONDS), Ok(()));
    }

    /// A missing header is an unsigned caller, not a bad signature, and the
    /// two get different operator responses.
    #[test]
    fn missing_headers_read_as_unsigned() {
        let hdrs = |k: &str| -> Option<&'static str> {
            match k {
                "webhook-id" => Some("req_1"),
                _ => None,
            }
        };
        assert!(SignatureHeaders::from_lookup(hdrs).is_none());
    }

    /// Header names arrive lowercase but "proxies are free to re-case them".
    #[test]
    fn header_lookup_is_case_insensitive_at_the_boundary() {
        let raw = [
            ("Webhook-Id", "req_1"),
            ("WEBHOOK-TIMESTAMP", "1000"),
            ("webhook-signature", "v1,x"),
        ];
        let got = SignatureHeaders::from_lookup(|want| {
            raw.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(want))
                .map(|(_, v)| *v)
        });
        assert!(got.is_some(), "a re-cased header must still be found");
    }

    /// Our own misconfiguration is reported as ours.
    #[test]
    fn an_undecodable_secret_is_named_as_our_problem() {
        let body = br#"{}"#;
        let h = SignatureHeaders {
            id: "req_1",
            timestamp: "1000",
            signature: "v1,AAAA",
        };
        assert_eq!(
            verify(&["whsec_!!!not base64!!!".to_string()], h, body, 1000),
            Err(VerifyError::MalformedSecret)
        );
    }
}
