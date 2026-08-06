//! Turning a citation into a verdict input.
//!
//! This is the seam that decides whether the guard verifies anything or
//! merely logs. A citation is a claim of the form "this observation exists,
//! was signed, and says X". Resolving it means asking a responder that holds
//! the observation whether all three are true.
//!
//! # The constraint the design has to respect
//!
//! The verdict path may touch warm state only. No materialisation, no
//! upstream fetch, no geocoding, ever. That is not a preference: the whole
//! exchange has an 800 ms self-budget under an org floor of 1000 ms, and a
//! rule that reached upstream would both blow it and make the verdict depend
//! on a third party that can be slow, down, or wrong.
//!
//! So a resolver is allowed exactly one thing: ask a responder for something
//! it already holds, under a hard timeout, and treat a miss as a miss. A
//! responder is a cache lookup away from an answer it has; it is a satellite
//! fetch away from one it does not, and the second is what the deadline
//! forbids.
//!
//! # Why a miss is not a failure
//!
//! [`TokenStatus::Unresolved`] is the answer for anything this node does not
//! hold, and the policy engine never denies on it. That is deliberate and it
//! is what makes federation possible: a token minted by another responder is
//! unresolvable here and completely legitimate. A guard that treated "I do
//! not have it" as "it is false" would punish an agent for citing across
//! nodes, which is the behaviour the token format exists to enable.

use std::time::Duration;

use crate::policy::TokenStatus;
use crate::tokens::{FoundToken, TokenKind};

/// Something that can say what a citation turned out to be.
pub trait Resolver: Send + Sync {
    /// Resolve one citation. Must return within `budget` or answer
    /// [`TokenStatus::Unresolved`]; a resolver that blocks past the deadline
    /// converts a verdict into a webhook failure.
    fn resolve(&self, token: &FoundToken, budget: Duration) -> TokenStatus;

    /// A short name for the log, so an auditor can tell which kind of node
    /// produced a verdict. A bare node and a corpus-backed one reach
    /// different conclusions from the same transcript, and the record should
    /// say which one this was.
    fn describe(&self) -> String;
}

/// A node with no corpus attached.
///
/// Every citation is unresolved, so every verdict is an allow. That is the
/// correct behaviour for a node that cannot check rather than a degraded one:
/// it still signs and logs every decision, which is the half of the product
/// that does not need a corpus. An operator who wants verification points it
/// at a responder.
#[derive(Debug, Clone, Copy, Default)]
pub struct NullResolver;

impl Resolver for NullResolver {
    fn resolve(&self, _token: &FoundToken, _budget: Duration) -> TokenStatus {
        TokenStatus::Unresolved
    }
    fn describe(&self) -> String {
        "null".to_string()
    }
}

/// Resolve against an emem responder over HTTP.
///
/// Points at a node that already holds the corpus, normally on localhost or
/// inside the same network. The responder's own warm cache is what makes this
/// fast enough to sit on a verdict path.
pub struct ResponderResolver {
    base: String,
    client: reqwest::blocking::Client,
}

impl ResponderResolver {
    /// `base` is the responder root, e.g. `http://127.0.0.1:5051`.
    ///
    /// The client carries its own connect and total timeouts as a floor, so a
    /// responder that hangs cannot hold the verdict past its deadline even if
    /// a caller passes a generous budget.
    pub fn new(base: impl Into<String>, budget: Duration) -> Result<Self, reqwest::Error> {
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_millis(200).min(budget))
            .timeout(budget)
            // Connection reuse matters here: a TLS handshake per citation
            // would dominate the budget on a transcript carrying several.
            .pool_max_idle_per_host(8)
            .build()?;
        Ok(Self {
            base: base.into().trim_end_matches('/').to_string(),
            client,
        })
    }

    /// The fact cid a token names, if its grammar carries one.
    ///
    /// Only the forms whose last segment is a content id. An entity token
    /// names an object rather than an observation, and a cell token names an
    /// address, so neither has bytes to check.
    fn fact_cid(token: &FoundToken) -> Option<&str> {
        match token.kind {
            TokenKind::Fact | TokenKind::Bundle | TokenKind::Field => {
                token.token.rsplit(':').next().filter(|s| !s.is_empty())
            }
            TokenKind::Entity | TokenKind::Cell => None,
        }
    }
}

impl Resolver for ResponderResolver {
    fn resolve(&self, token: &FoundToken, budget: Duration) -> TokenStatus {
        let Some(cid) = Self::fact_cid(token) else {
            return TokenStatus::Unresolved;
        };
        // A cid is used to build a URL, so it must be shape-checked first.
        // Anything else is an injection surface and, worse, a way to make the
        // guard issue arbitrary requests on a caller's behalf.
        if !cid
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            || cid.len() < 16
            || cid.len() > 64
        {
            return TokenStatus::Unresolved;
        }

        let url = format!("{}/v1/facts/{cid}", self.base);
        let Ok(resp) = self.client.get(&url).timeout(budget).send() else {
            // Unreachable, slow, or refused. Not evidence about the claim.
            return TokenStatus::Unresolved;
        };
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            // The responder is healthy and does not hold it. Still not a
            // statement that the observation is false: this node may simply
            // not have cached what another minted.
            return TokenStatus::Unresolved;
        }
        if !resp.status().is_success() {
            return TokenStatus::Unresolved;
        }
        let Ok(body) = resp.json::<serde_json::Value>() else {
            return TokenStatus::Unresolved;
        };
        // A typed error body carries `code`, not a fact.
        if body.get("code").is_some() && body.get("cell").is_none() {
            return TokenStatus::Unresolved;
        }

        // The fact resolved. That the responder returned it under this cid is
        // itself the content-address check: the cid IS the hash of the
        // canonical bytes, so a mismatch cannot survive the lookup.
        //
        // What remains is whether the token's own cell agrees with the fact's,
        // which is the case a transcript can get wrong by pairing a real cid
        // with the wrong address.
        if token.kind == TokenKind::Fact {
            if let (Some(claimed), Some(actual)) = (
                cell_of_token(&token.token),
                body.get("cell").and_then(|c| c.as_str()),
            ) {
                if claimed != actual {
                    return TokenStatus::ByteMismatch;
                }
            }
        }
        TokenStatus::Verified
    }

    fn describe(&self) -> String {
        format!("responder:{}", self.base)
    }
}

/// The cell64 an `emem:fact:<cell>:<cid>` token claims.
///
/// `None` for the spoken form, which carries coordinates and a date instead
/// of an address and so has nothing to compare directly.
fn cell_of_token(token: &str) -> Option<&str> {
    let rest = token.strip_prefix("emem:fact:")?;
    let cell = rest.split(':').next()?;
    // The address form only. The spoken form contains a comma or an @.
    (!cell.contains(',') && !cell.contains('@') && cell.contains('.')).then_some(cell)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tok(s: &str, kind: TokenKind) -> FoundToken {
        FoundToken {
            token: s.to_string(),
            kind,
        }
    }

    /// A bare node allows everything and says so in the log, rather than
    /// pretending it checked.
    #[test]
    fn the_null_resolver_is_honest_about_checking_nothing() {
        let r = NullResolver;
        assert_eq!(
            r.resolve(
                &tok("emem:fact:a.b.c.d:cid", TokenKind::Fact),
                Duration::from_millis(50)
            ),
            TokenStatus::Unresolved
        );
        assert_eq!(r.describe(), "null");
    }

    /// Only the token families that name an observation have bytes to check.
    #[test]
    fn only_observation_tokens_carry_a_content_id() {
        let cases = [
            (
                "emem:fact:defi.zb493.xuqA.zcb5f:abc123",
                TokenKind::Fact,
                Some("abc123"),
            ),
            ("emem:bundle:bcid987", TokenKind::Bundle, Some("bcid987")),
            ("emem:entity:ecid", TokenKind::Entity, None),
            ("emem:cell:defi.zb493.xuqA.zcb5f", TokenKind::Cell, None),
        ];
        for (s, kind, want) in cases {
            assert_eq!(ResponderResolver::fact_cid(&tok(s, kind)), want, "{s}");
        }
    }

    /// A cid goes into a URL, so a malformed one must never reach the wire.
    /// Otherwise the guard becomes a way to make requests on a caller's
    /// behalf, aimed by whatever text arrives in a transcript.
    #[test]
    fn a_cid_that_could_escape_a_url_is_refused_before_any_request() {
        let r = ResponderResolver::new("http://127.0.0.1:1", Duration::from_millis(5)).unwrap();
        for bad in [
            "emem:fact:a.b.c.d:../../etc/passwd",
            "emem:fact:a.b.c.d:abc?x=1",
            "emem:fact:a.b.c.d:abc#frag",
            "emem:fact:a.b.c.d:ABCDEF0123456789",
            "emem:fact:a.b.c.d:short",
        ] {
            assert_eq!(
                r.resolve(&tok(bad, TokenKind::Fact), Duration::from_millis(5)),
                TokenStatus::Unresolved,
                "{bad} must not produce a request"
            );
        }
    }

    /// An unreachable responder is not evidence about the claim. A guard that
    /// denied when its corpus was down would turn our outage into the
    /// customer's.
    #[test]
    fn an_unreachable_responder_yields_unresolved_not_a_denial() {
        // Port 1 is reserved and refuses immediately.
        let r = ResponderResolver::new("http://127.0.0.1:1", Duration::from_millis(50)).unwrap();
        let st = r.resolve(
            &tok(
                "emem:fact:defi.zb493.xuqA.zcb5f:abcdefghijklmnop",
                TokenKind::Fact,
            ),
            Duration::from_millis(50),
        );
        assert_eq!(st, TokenStatus::Unresolved);
    }

    /// The address half of a fact token is what a transcript can get wrong by
    /// pairing a real content id with the wrong place.
    #[test]
    fn the_claimed_cell_is_extracted_only_from_the_address_form() {
        assert_eq!(
            cell_of_token("emem:fact:defi.zb493.xuqA.zcb5f:abc"),
            Some("defi.zb493.xuqA.zcb5f")
        );
        // The spoken form carries coordinates and a date, not an address.
        assert_eq!(
            cell_of_token("emem:fact:12.97,77.59@2026-08-05@indices.ndvi:abc"),
            None
        );
        assert_eq!(cell_of_token("emem:entity:abc"), None);
    }

    /// The describe string reaches the log, so an auditor can tell a bare
    /// node's allow from a verifying node's allow. They are different claims.
    #[test]
    fn a_resolver_names_itself_for_the_record() {
        let r =
            ResponderResolver::new("http://127.0.0.1:5051/", Duration::from_millis(10)).unwrap();
        assert_eq!(r.describe(), "responder:http://127.0.0.1:5051");
    }
}
