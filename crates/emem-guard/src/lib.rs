//! emem-guard — a signed verdict server for claims about the physical world.
//!
//! What antivirus and DLP engines are for content, this is for grounding.
//! Input: a conversation transcript. Output: allow or deny, signed, logged,
//! with a machine-readable reason. Nothing else.
//!
//! # Why this exists
//!
//! Anthropic's Inference hooks hold every governed prompt for an allow/deny
//! verdict from an org-chosen server before inference runs. The named
//! destinations are DLP incumbents, and every one of them evaluates
//! CONTENT: does this text contain a card number, a secret, a classified
//! marking. None of them can evaluate whether a claim about the world is
//! still true, because none of them hold signed observations of it.
//!
//! The seam is exact: their verdict shape (allow/deny plus a reason plus a
//! join id) is our receipt shape. Requests to a security server are signed
//! on the way in, but verdicts come back as unsigned JSON into a mutable
//! audit database, so nobody can later PROVE what was allowed, denied, or
//! why. That is the half we already built.
//!
//! # What this crate is, and is not
//!
//! It is the checkpoint-agnostic core plus a thin adapter per provider.
//! Anthropic is the first adapter because it is the first checkpoint that
//! exists; nothing above [`webhook`] or [`tokens`] assumes it.
//!
//! It is NOT a DLP engine. Detection accuracy is a race against vendors with
//! hundreds of engineers maintaining thousands of patterns, and winning it
//! would grow nothing that matters here: a credit-card regex adds no token
//! adoption, no log age, no witnesses. The grounding sub-verdict is the part
//! nobody else can build.
//!
//! # The one asymmetry to hold on to
//!
//! A webhook failure is **not** a deny. Anything other than HTTP 200 with a
//! parseable verdict hands the decision to the org's failure policy, which
//! under fail-open means the prompt reaches the model uninspected. So this
//! crate answers 200 with a verdict in every reachable case, including ones
//! it does not understand: an unrecognised event type allows, a malformed
//! frame allows, an internal error allows. Refusing to answer does not block
//! anything; it just removes us from the path and, if sustained, trips the
//! circuit breaker that stops enforcement entirely.
//!
//! Denial is reserved for the case where we positively established that a
//! cited claim does not hold.

#![forbid(unsafe_code)]

pub mod checkpoint;
pub mod frame;
pub mod log;
pub mod policy;
pub mod server;
pub mod store;
pub mod tokens;
pub mod webhook;

pub use checkpoint::{Adapter, AnthropicHook, ClaudeCodeHook, Outcome, Transcript};
pub use frame::{Action, ContentBlock, EventType, Message, PromptFrame, Verdict};
pub use log::{seal, LogError, LogFailurePolicy, VerdictLog, VerdictRecord};
pub use policy::{evaluate, Config, Decision, DenyCode, Evidence, Fix, TokenStatus};
pub use tokens::{scan, scan_all, FoundToken, TokenKind};
pub use webhook::{verify, SignatureHeaders, VerifyError};

/// Why a verdict came out the way it did, in a form an agent can act on.
///
/// The `deny_reason` is shown to a human, but the audience that matters most
/// is the agent that will retry. The mission is explicit that the gate drives
/// token adoption: an agent that learns "refresh the token and retry" from a
/// denial starts carrying valid tokens, and a denial that says only "policy
/// violation" teaches it nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ground {
    /// No emem tokens in the transcript. Nothing to check, so nothing to
    /// object to.
    ///
    /// This is the common case and it must stay cheap: most prompts in a
    /// governed org have nothing to do with the physical world, and a
    /// grounding gate that taxes them is a gate that gets turned off.
    NoCitations,
    /// Citations found and every one verified.
    AllVerified {
        /// How many tokens were checked.
        checked: usize,
    },
    /// A cited token's signature did not verify.
    ///
    /// The strongest denial available: the bytes are not what was signed.
    SignatureFailed { token: String },
    /// A cited token resolved, but to different bytes than the transcript
    /// asserts.
    ByteMismatch { token: String },
    /// A cited token could not be resolved on this responder at all.
    ///
    /// Deliberately NOT a denial on its own: a token from another responder
    /// is unresolvable here and perfectly legitimate, and a self-hosted node
    /// holds a subset of the corpus by design. Treating "I do not have it"
    /// as "it is false" would deny honest agents for the crime of citing
    /// something we have not seen.
    Unresolvable { token: String },
}

impl Ground {
    /// Whether this outcome justifies blocking the request.
    pub fn is_denial(&self) -> bool {
        matches!(
            self,
            Self::SignatureFailed { .. } | Self::ByteMismatch { .. }
        )
    }

    /// The user-facing reason, written to tell them what to change.
    pub fn deny_reason(&self) -> Option<String> {
        Some(match self {
            Self::SignatureFailed { token } => format!(
                "A cited emem token in this conversation does not verify: its signature does not \
                 match the content it claims. Token {token}. Re-resolve it and retry; if it still \
                 fails, the citation has been altered since it was issued and should not be relied on."
            ),
            Self::ByteMismatch { token } => format!(
                "A cited emem token resolves to different content than this conversation states. \
                 Token {token}. Re-resolve it and quote the current value, or drop the claim."
            ),
            _ => return None,
        })
    }
}

/// Turn a grounding outcome into the verdict that answers the hook.
///
/// Allows everything that is not a positive failure, for the reason in the
/// module docs: a verdict we are unsure about costs inspection if we get it
/// wrong in the deny direction, and costs nothing if we get it wrong in the
/// allow direction, because we were never the only control.
pub fn verdict_for(ground: &Ground) -> Verdict {
    match ground.deny_reason() {
        Some(reason) => Verdict::deny(reason),
        None => Verdict::allow(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only a positive failure denies. Everything else allows, including the
    /// cases we could not evaluate.
    #[test]
    fn uncertainty_allows_and_only_proven_failure_denies() {
        let allow = [
            Ground::NoCitations,
            Ground::AllVerified { checked: 3 },
            Ground::Unresolvable {
                token: "emem:fact:a:b".into(),
            },
        ];
        for g in &allow {
            assert!(!g.is_denial(), "{g:?} must not deny");
            assert_eq!(verdict_for(g).action, Action::Allow, "{g:?}");
        }

        let deny = [
            Ground::SignatureFailed {
                token: "emem:fact:a:b".into(),
            },
            Ground::ByteMismatch {
                token: "emem:fact:a:b".into(),
            },
        ];
        for g in &deny {
            assert!(g.is_denial(), "{g:?} must deny");
            assert_eq!(verdict_for(g).action, Action::Deny, "{g:?}");
        }
    }

    /// A token this responder simply does not hold is not evidence of a lie.
    /// Self-hosted nodes hold a subset by design, and cross-responder
    /// citations are the point of the token format.
    #[test]
    fn a_token_we_do_not_hold_is_not_a_denial() {
        let g = Ground::Unresolvable {
            token: "emem:fact:elsewhere:cid".into(),
        };
        assert!(!g.is_denial());
        assert!(g.deny_reason().is_none());
    }

    /// The reason is for a human AND for the agent that will retry, so it has
    /// to name the token and the next action.
    #[test]
    fn a_denial_names_the_token_and_the_remedy() {
        let g = Ground::SignatureFailed {
            token: "emem:fact:cell:cid".into(),
        };
        let r = g.deny_reason().unwrap();
        assert!(
            r.contains("emem:fact:cell:cid"),
            "the agent must know WHICH token"
        );
        assert!(r.contains("retry"), "and what to do next");
        // And it fits the platform's cap without being truncated by it.
        assert!(r.chars().count() <= frame::DENY_REASON_MAX);
    }
}
