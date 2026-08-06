//! emem-guard — a signed verdict server for claims about the physical world.
//!
//! What antivirus and DLP engines are for content, this is for grounding.
//! Input: a conversation transcript. Output: allow or deny, signed, logged,
//! with a machine-readable reason. Nothing else.
//!
//! # Why this exists
//!
//! Agents assert things about the physical world constantly, and almost
//! nothing in the stack can tell a measured claim from a fluent one. Every
//! guardrail that ships today evaluates CONTENT: does this text contain a card
//! number, a secret, a classified marking, a jailbreak. None of them can
//! evaluate whether a claim about the world is still true, because none of
//! them hold signed observations of it. That is not a gap in their engineering;
//! it is a gap in what they have to check against.
//!
//! A **checkpoint** is any place a system pauses before doing something and
//! asks whether to proceed. There are many, they belong to different people,
//! and more arrive every month:
//!
//!   - a model gateway holding a prompt before inference,
//!   - an MCP host about to dispatch a tool call, or about to admit a tool
//!     result into the context,
//!   - a policy decision point in front of an action,
//!   - an eventing mesh routing an agent's output,
//!   - a client-side hook inside a coding agent.
//!
//! The question is the same at every one of them and it has nothing to do with
//! who asked: does this cited observation still verify. So this crate answers
//! it once and adapts outward. [`checkpoint`] holds the two vendor-shaped
//! adapters, [`open`] holds the shape we designed for anyone, and [`interop`]
//! holds the standards-based ones. The engine in [`policy`] has never heard of
//! any of them.
//!
//! # What this crate is, and is not
//!
//! It is the checkpoint-agnostic core plus one thin adapter per wire shape. No
//! vendor is privileged: a test asserts the same evidence yields the same
//! outcome and the same reason text through every adapter, because a guard
//! that answered differently by host would be discriminating between agents on
//! the basis of whose product they happened to be running inside.
//!
//! It is NOT a DLP engine. Detection accuracy is a race against vendors with
//! hundreds of engineers maintaining thousands of patterns, and winning it
//! would grow nothing that matters here: a credit-card regex adds no token
//! adoption, no log age, no witnesses. The grounding sub-verdict is the part
//! nobody else can build, and everyone else's detection should plug in beside
//! it rather than be reimplemented here.
//!
//! # The one asymmetry to hold on to
//!
//! **A transport failure is not a deny.** Every checkpoint contract this crate
//! has been written against agrees on that and none of them make it obvious.
//! Anything other than a success status with a parseable verdict hands the
//! decision to the caller's failure policy, which under fail-open means the
//! request proceeds uninspected; sustained failures trip a breaker that stops
//! enforcement entirely. Claude Code inverts the usual HTTP intuition the same
//! way: you block by returning 2xx with a deny in the BODY, and a 500 enforces
//! nothing at all.
//!
//! So this crate answers success with a verdict in every reachable case,
//! including ones it does not understand: an unrecognised event type allows, a
//! malformed body allows, an internal fault allows. Refusing to answer does not
//! block anything; it removes us from the path.
//!
//! Denial is reserved for the case where we positively established that a
//! cited claim does not hold.

#![forbid(unsafe_code)]

pub mod checkpoint;
pub mod claim;
pub mod conformance;
pub mod frame;
pub mod interop;
pub mod log;
pub mod module;
pub mod modules;
pub mod open;
pub mod policy;
pub mod resolve;
pub mod server;
pub mod store;
pub mod tokens;
pub mod webhook;

pub use checkpoint::{Adapter, AnthropicHook, ClaudeCodeHook, Outcome, Transcript};
pub use claim::{scan_claims, Anchor, Claim, Quantity};
pub use frame::{Action, ContentBlock, EventType, Message, PromptFrame, Verdict};
pub use interop::{CloudEventGate, McpGate, OpenAiGate, PolicyPointGate};
pub use log::{seal, LogError, LogFailurePolicy, VerdictLog, VerdictRecord};
pub use module::{
    DataClass, LatencyClass, ModuleContext, ModuleError, ModuleManifest, ModuleOutcome,
    ModuleRegistry, ModuleVerdict, PolicyModule, SignedManifest,
};
#[cfg(unix)]
pub use modules::SidecarModule;
pub use modules::{OrgWebhook, SecretPatterns};
pub use open::{ClaimDetail, NativeHook, NativeRequest, NativeVerdict};
pub use policy::{
    evaluate, Config, Decision, DenyCode, Evidence, Fix, Mode, ModuleAttribution, TokenStatus,
};
pub use resolve::{NullResolver, Resolver, ResponderResolver};
pub use store::{AuditReport, FileLog, ShadowReport};
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
