//! Checkpoints that belong to nobody.
//!
//! The two adapters in [`crate::checkpoint`] speak one vendor's wire shapes,
//! because that vendor shipped a checkpoint and published a spec. Neither is
//! the protocol, and a guard reachable only through them would be a guard for
//! that vendor's customers.
//!
//! This module is the open half: shapes any agent can call, on any model,
//! through any framework, with nothing installed and nobody's account.
//!
//! # Why an open route is a correctness requirement, not generosity
//!
//! The claim this whole system rests on is that a cited observation verifies
//! or it does not, independent of who asks. If the only way to get that answer
//! were a proprietary hook, the answer would in practice be available to some
//! agents and not others, and "verifiable by anyone" would quietly mean
//! "verifiable by Enterprise customers". The open route is what keeps the
//! claim true.
//!
//! It also matters for the shape of the verdict. A vendor's checkpoint decides
//! what it is willing to be told: an inference hook accepts allow or deny and ignores
//! unknown fields, and Claude Code accepts a permission decision. Neither can
//! carry a signature. The native shape can, so an agent calling the open route
//! gets the evidence rather than a verdict it has to take on faith.

use serde::{Deserialize, Serialize};

use crate::checkpoint::{Adapter, Outcome, Transcript};
use crate::policy::Decision;

// ── The native shape ─────────────────────────────────────────────────────

/// What any agent can post to ask the question directly.
///
/// Deliberately minimal. A caller that already has a transcript sends it; a
/// caller that only has a paragraph of its own reasoning sends that. Nothing
/// here is required except something to read, because a schema that demanded
/// a vendor's envelope would reintroduce the coupling this module removes.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct NativeRequest {
    /// Free text to scan. Any number of pieces, in any order.
    #[serde(default)]
    pub texts: Vec<String>,
    /// A chat-completions-shaped transcript, accepted because it is the de
    /// facto interchange format across frameworks and models.
    ///
    /// This is a SHAPE, not a claim of conformance to any vendor's API: it is
    /// read for its text and nothing else, so a framework that produces
    /// something close enough gets a useful answer without us asserting
    /// compatibility we have not verified.
    #[serde(default)]
    pub messages: Vec<NativeMessage>,
    /// Caller's own id for this evaluation, used for idempotency. Generated
    /// when absent, because a caller with no id still deserves an answer.
    #[serde(default)]
    pub request_id: Option<String>,
    /// Free-text label for the log: which agent, which framework, which run.
    /// Advisory only, and never a trust boundary.
    #[serde(default)]
    pub agent: Option<String>,
}

/// One turn, in the shape most frameworks already emit.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct NativeMessage {
    #[serde(default)]
    pub role: Option<String>,
    /// Either a string or an array of blocks: both are common, and rejecting
    /// one would make the route useless to half its callers.
    #[serde(default)]
    pub content: NativeContent,
}

/// String or blocks.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum NativeContent {
    Text(String),
    Blocks(Vec<serde_json::Value>),
}

impl Default for NativeContent {
    fn default() -> Self {
        Self::Text(String::new())
    }
}

impl NativeContent {
    pub(crate) fn texts(&self) -> Vec<&str> {
        match self {
            Self::Text(s) => vec![s.as_str()],
            Self::Blocks(bs) => bs
                .iter()
                .filter_map(|b| {
                    // `{"type":"text","text":...}` and a bare string are both
                    // widespread; read either rather than insisting.
                    b.get("text")
                        .and_then(|t| t.as_str())
                        .or_else(|| b.as_str())
                })
                .collect(),
        }
    }
}

impl NativeRequest {
    /// Everything readable, in order.
    pub fn all_texts(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.texts.iter().map(String::as_str).collect();
        v.extend(self.messages.iter().flat_map(|m| m.content.texts()));
        v
    }
}

/// What the open route answers with.
///
/// Richer than any vendor verdict, because nothing here is constrained by
/// someone else's schema. The caller gets the decision, the reason, the
/// machine-readable parts already split out so nothing has to parse the
/// string, and the log leaf that makes it checkable.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct NativeVerdict {
    /// `allow` or `deny`.
    pub action: &'static str,
    /// The same line every other checkpoint receives, so an agent that
    /// learned the grammar on one route reads it identically here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Pre-split, because making every caller parse a string to find the
    /// remedy is how a machine-readable contract becomes a human one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// The transparency-log entry for this verdict. Fetch it and check the
    /// signature; you do not have to believe this response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub leaf: Option<String>,
    /// How many citations were examined. An allow with `checked: 0` means
    /// there was nothing to check, which is a different statement from an
    /// allow with `checked: 4`, and a caller deserves to tell them apart.
    pub checked: usize,
    /// True when a rule fired and the node was observing rather than
    /// enforcing.
    ///
    /// An allow that says `shadowed: true` is telling the caller it would have
    /// been blocked on an enforcing node. That is the most useful thing a
    /// shadow deployment can give an agent, and no vendor schema has a field
    /// for it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub shadowed: bool,
    /// The ungrounded claim behind a `CLAIM_UNGROUNDED` verdict.
    ///
    /// The fixed reason line has no field for this, and changing that grammar
    /// would break every agent already parsing it. Here the schema is ours, so
    /// the caller gets the sentence, the magnitude and the band that would
    /// have answered it, which is the difference between "cite something" and
    /// "resolve elevation at this place and cite that".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim: Option<ClaimDetail>,
}

/// The machine-readable half of a claim denial.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ClaimDetail {
    /// The sentence that asserted it, truncated to
    /// [`crate::claim::SENTENCE_MAX`].
    pub sentence: String,
    /// The magnitude as written, e.g. `918 m`.
    pub magnitude: String,
    /// What kind of quantity it is.
    pub quantity: &'static str,
    /// A recallable band key that would answer this claim, or `null` when
    /// this responder observes no band in that quantity.
    ///
    /// The actionable field, so it must name a key `POST /v1/recall` accepts
    /// and a materializer can serve, never a family root from
    /// `GET /v1/bands`. A `null` here is informative rather than missing: the
    /// claim is ungrounded and this node cannot ground it either, so the
    /// agent should stop looking rather than chase a band that does not exist.
    pub source_band: Option<&'static str>,
    /// What tied the claim to a place or a time.
    pub anchor: &'static str,
}

impl From<&crate::claim::Claim> for ClaimDetail {
    fn from(c: &crate::claim::Claim) -> Self {
        Self {
            sentence: c.sentence.clone(),
            magnitude: c.magnitude.clone(),
            quantity: c.quantity.as_str(),
            source_band: c.source_band,
            anchor: c.anchor.as_str(),
        }
    }
}

/// The open, vendor-neutral checkpoint.
#[derive(Debug, Clone, Copy, Default)]
pub struct NativeHook;

impl Adapter for NativeHook {
    type Request = NativeRequest;
    type Response = NativeVerdict;

    fn checkpoint_id(&self) -> &'static str {
        "emem.native.v1"
    }

    fn transcript<'a>(&self, body: &'a NativeRequest) -> Option<Transcript<'a>> {
        Some(Transcript {
            texts: body.all_texts(),
            request_id: body.request_id.as_deref(),
            surface: body.agent.as_deref(),
        })
    }

    fn render(&self, d: &Decision) -> NativeVerdict {
        NativeVerdict {
            action: match d.outcome {
                Outcome::Proceed => "allow",
                Outcome::Block => "deny",
            },
            // The reason line is emitted whenever a rule fired, including on a
            // shadowed allow. An agent that wants to self-correct before it is
            // ever blocked can only do that if the observing node tells it
            // what it saw.
            reason: d.would_block().then(|| d.reason_line()),
            code: d.code.map(crate::policy::DenyCode::as_str),
            fix: d.fix.map(crate::policy::Fix::as_str),
            token: d.token.clone(),
            leaf: d.leaf.clone(),
            checked: d.checked,
            shadowed: d.is_shadowed(),
            claim: d.claim.as_ref().map(ClaimDetail::from),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::{DenyCode, Fix};

    #[test]
    fn the_open_route_reads_the_shapes_frameworks_actually_emit() {
        // Bare text.
        let r: NativeRequest = serde_json::from_str(r#"{"texts":["see emem:fact:a:b"]}"#).unwrap();
        assert_eq!(r.all_texts(), vec!["see emem:fact:a:b"]);

        // Chat-completions with string content.
        let r: NativeRequest =
            serde_json::from_str(r#"{"messages":[{"role":"user","content":"hello"}]}"#).unwrap();
        assert_eq!(r.all_texts(), vec!["hello"]);

        // Chat-completions with block content.
        let r: NativeRequest = serde_json::from_str(
            r#"{"messages":[{"role":"assistant","content":[{"type":"text","text":"blocks"}]}]}"#,
        )
        .unwrap();
        assert_eq!(r.all_texts(), vec!["blocks"]);

        // An empty request is answerable, not an error: a caller with nothing
        // to check still gets a verdict.
        let r: NativeRequest = serde_json::from_str("{}").unwrap();
        assert!(r.all_texts().is_empty());
    }

    /// An unknown field must not break the route. Frameworks decorate.
    #[test]
    fn unrecognised_fields_do_not_break_the_open_route() {
        let r: NativeRequest = serde_json::from_str(
            r#"{"texts":["x"],"framework":"langgraph","trace_id":"t1","nested":{"a":1}}"#,
        )
        .unwrap();
        assert_eq!(r.all_texts(), vec!["x"]);
    }

    /// The native verdict carries the evidence, not just the answer. A vendor
    /// schema cannot hold a log leaf; ours can, so an agent does not have to
    /// take the verdict on faith.
    #[test]
    fn the_native_verdict_is_checkable_rather_than_asserted() {
        let d = Decision::block(
            DenyCode::ProvSig,
            Some("emem:fact:a:b".into()),
            Fix::RefreshToken,
        )
        .with_leaf("leaf_7")
        .with_checked(3);
        let v = NativeHook.render(&d);
        assert_eq!(v.action, "deny");
        assert_eq!(v.code, Some("PROV_SIG"));
        assert_eq!(v.fix, Some("refresh_token"));
        assert_eq!(v.leaf.as_deref(), Some("leaf_7"));
        assert_eq!(v.checked, 3);
        // And the reason is the same line every other checkpoint gets, so an
        // agent needs one parser.
        assert_eq!(v.reason.as_deref(), Some(d.reason_line().as_str()));
    }

    /// An allow that checked nothing is a different statement from an allow
    /// that checked four things, and a caller must be able to tell.
    #[test]
    fn an_allow_says_how_much_was_actually_examined() {
        let nothing = NativeHook.render(&Decision::proceed());
        assert_eq!((nothing.action, nothing.checked), ("allow", 0));
        assert!(
            nothing.reason.is_none(),
            "an allow carries no reason to explain away"
        );

        let checked = NativeHook.render(&Decision::proceed().with_checked(4));
        assert_eq!(checked.checked, 4);
    }
}
