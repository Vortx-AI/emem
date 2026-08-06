//! Checkpoints: the places a verdict can be asked for.
//!
//! A checkpoint is any system that pauses before doing something and asks an
//! external server whether to proceed. This module holds the two whose wire
//! shapes belong to a vendor, because that vendor published a spec and shipped
//! a product. **They are not the protocol and they are not the default.** The
//! shapes anyone can call live in [`crate::open`] and [`crate::interop`], and
//! they outnumber these two more than three to one.
//!
//! They are here for a specific reason: they reach agents the open routes do
//! not, and reach is the whole argument for a guardrail. An org that governs
//! its models centrally will never call a route it has to write code for, and
//! an agent inside a coding tool is gated by that tool's hook or not at all.
//! Serving them costs one adapter each and no privilege anywhere else.
//!
//! # Why this is a trait and not an if-statement
//!
//! The grounding decision has nothing to do with who asked. Whether a cited
//! observation still verifies is a fact about the observation, not about the
//! vendor whose checkpoint noticed it. So the policy engine takes a
//! [`Transcript`] and returns a [`Decision`], and every vendor-specific shape
//! lives in one adapter that translates inward and outward.
//!
//! That boundary is also the interoperability commitment. An agent running on
//! any model, through any checkpoint, gets the same verdict on the same
//! evidence. A guard that answered differently depending on which vendor's
//! webhook arrived would be discriminating between agents on the basis of
//! their host, which is the opposite of what a shared memory layer is for.
//!
//! # The rule every adapter must honour
//!
//! Each checkpoint has its own way of saying "I could not decide", and in
//! every one of them that is NOT a block. Anthropic treats a non-200 as a
//! webhook failure and hands the decision to the org's failure policy;
//! Claude Code treats a non-2xx as non-blocking. An adapter that signals
//! refusal by erroring does not block anything, it just removes itself from
//! the path. So [`Adapter::render`] is total: every decision, including the
//! ones we could not form, becomes a well-formed success response.

use crate::frame::{PromptFrame, Verdict};
use crate::policy::Decision;
use serde::Serialize;

/// A transcript, normalised away from any one vendor's wire shape.
///
/// Borrowed rather than owned because an adapter has already parsed the
/// request and holds it; a verdict path with a 5-second budget for the whole
/// exchange should not be copying megabytes of transcript to change its type.
#[derive(Debug, Clone, Default)]
pub struct Transcript<'a> {
    /// Every readable string, in transcript order: user text, tool results,
    /// extracted attachment text.
    ///
    /// Tool results are in here deliberately. A grounded agent carries its
    /// citations in what it read back from a tool, not in what the human
    /// typed, so a scanner that only saw user text would miss the entire
    /// population it exists to check.
    pub texts: Vec<&'a str>,
    /// Opaque per-request id, used as the idempotency key.
    pub request_id: Option<&'a str>,
    /// Which surface this came from, when the checkpoint says. Advisory
    /// routing metadata only: the spec is explicit that it is not a trust
    /// boundary, so no rule may branch on it for a security decision.
    pub surface: Option<&'a str>,
}

/// What a checkpoint is entitled to be told.
///
/// Deliberately smaller than any vendor's response type. An adapter may add
/// vendor-specific fields on the way out, but nothing upstream of this
/// enum may depend on them, or the policy engine grows a vendor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Proceed.
    Proceed,
    /// Stop, and tell the caller why in terms it can act on.
    Block,
}

/// One vendor's translation layer.
///
/// `parse` is fallible because a body can be malformed. `render` is NOT:
/// see the module docs. Once a decision exists, producing the wire form of it
/// cannot fail, because the alternative to a well-formed answer is silence,
/// and silence is not a block anywhere.
pub trait Adapter {
    /// The wire response this checkpoint expects.
    type Response: Serialize;

    /// A stable identifier for the checkpoint class, used in the log so a
    /// verdict can be attributed to the surface that asked for it.
    fn checkpoint_id(&self) -> &'static str;

    /// Pull a normalised transcript out of a parsed request body.
    ///
    /// `None` means the body was structurally valid but carries no event this
    /// adapter evaluates. The caller must PROCEED on `None`, never error: an
    /// event type we do not recognise still needs an answer, and refusing
    /// would count against the circuit breaker.
    fn transcript<'a>(&self, body: &'a Self::Request) -> Option<Transcript<'a>>;

    /// The request body type this checkpoint POSTs.
    type Request;

    /// Turn a decision into this checkpoint's wire form. Total by design.
    fn render(&self, decision: &Decision) -> Self::Response;
}

// ── Anthropic Inference hooks ────────────────────────────────────────────

/// Anthropic's Inference hooks (Claude Enterprise).
///
/// Governs claude.ai, Cowork and Claude Code sessions through one org-level
/// config, and is the checkpoint that holds the request before inference.
/// Out of scope on their side: the Platform API, Bedrock, Vertex.
#[derive(Debug, Clone, Copy, Default)]
pub struct AnthropicHook;

impl Adapter for AnthropicHook {
    type Request = PromptFrame;
    type Response = Verdict;

    fn checkpoint_id(&self) -> &'static str {
        "anthropic.inference_hook.v1"
    }

    fn transcript<'a>(&self, body: &'a PromptFrame) -> Option<Transcript<'a>> {
        // An event type we do not know is not ours to judge. Returning None
        // sends the caller down the proceed path, which is what the spec
        // requires: "return an allow verdict rather than an error status".
        if !body.event_type.is_evaluable() {
            return None;
        }
        Some(Transcript {
            texts: body.texts(),
            request_id: body.request_id.as_deref(),
            surface: body.source.as_ref().and_then(|s| s.application.as_deref()),
        })
    }

    fn render(&self, decision: &Decision) -> Verdict {
        match decision.outcome {
            Outcome::Proceed => Verdict::allow(),
            Outcome::Block => {
                let v = Verdict::deny(decision.reason_line());
                match decision.reference_id() {
                    Some(id) => v.with_reference_id(id),
                    None => v,
                }
            }
        }
    }
}

// ── Claude Code client-side hooks ────────────────────────────────────────

/// Claude Code's client-side hooks.
///
/// The second checkpoint class, and the one that matters for reach: it runs
/// in the client, so it covers agents talking to the Platform API, Bedrock
/// and Vertex, none of which Inference hooks can see. Enterprise is the
/// revenue path; this is the adoption path.
///
/// Its blocking semantics are the inverse of an HTTP intuition: you block by
/// returning **2xx with a deny decision in the body**. A non-2xx is
/// non-blocking, so an adapter that signalled refusal with a 500 would let
/// everything through while appearing to enforce.
#[derive(Debug, Clone, Copy, Default)]
pub struct ClaudeCodeHook {
    /// Which of the two body shapes to emit.
    pub style: ClaudeCodeStyle,
}

/// Claude Code hooks come in two response shapes depending on the event.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ClaudeCodeStyle {
    /// `PreToolUse`: carries `hookSpecificOutput.permissionDecision`, one of
    /// `allow` / `deny` / `ask`.
    #[default]
    PreToolUse,
    /// Prompt-style events, which expect `{"ok": bool, "reason": str}`.
    Prompt,
}

/// The request body a Claude Code hook posts.
///
/// Permissive for the same reason the Anthropic frame is: an unknown field is
/// an addition, and rejecting it would turn a client upgrade into a hook that
/// stops enforcing.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct ClaudeCodeInput {
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub hook_event_name: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub tool_input: Option<serde_json::Value>,
    #[serde(default)]
    pub tool_response: Option<serde_json::Value>,
}

/// The two wire shapes, as one serializable type.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum ClaudeCodeResponse {
    /// `PreToolUse` shape.
    PreToolUse {
        #[serde(rename = "hookSpecificOutput")]
        hook_specific_output: PermissionOutput,
    },
    /// Prompt shape.
    Prompt {
        ok: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
}

/// `hookSpecificOutput` for `PreToolUse`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PermissionOutput {
    #[serde(rename = "hookEventName")]
    pub hook_event_name: &'static str,
    #[serde(rename = "permissionDecision")]
    pub permission_decision: &'static str,
    #[serde(
        rename = "permissionDecisionReason",
        skip_serializing_if = "Option::is_none"
    )]
    pub permission_decision_reason: Option<String>,
}

impl ClaudeCodeInput {
    /// Every readable string this hook input carries.
    ///
    /// Tool responses are included for the same reason they are on the
    /// Anthropic side: that is where a grounded agent's citations live.
    fn texts(&self) -> Vec<&str> {
        let mut v: Vec<&str> = Vec::new();
        if let Some(p) = self.prompt.as_deref() {
            v.push(p);
        }
        // Tool input and response are arbitrary JSON. Only their string
        // leaves can carry a citation, and walking them is cheap next to the
        // budget, so both are scanned rather than guessed at.
        for val in [self.tool_input.as_ref(), self.tool_response.as_ref()]
            .into_iter()
            .flatten()
        {
            collect_strings(val, &mut v);
        }
        v
    }
}

/// Every string leaf in a JSON value, in document order.
///
/// Shared with the interop adapters: an MCP tool call, a CloudEvent and an OPA
/// input are all arbitrary JSON whose citations live at unknown depth, and
/// each one having its own walker would be three chances to miss a nesting.
pub(crate) fn collect_strings<'a>(v: &'a serde_json::Value, out: &mut Vec<&'a str>) {
    match v {
        serde_json::Value::String(s) => out.push(s.as_str()),
        serde_json::Value::Array(a) => a.iter().for_each(|x| collect_strings(x, out)),
        serde_json::Value::Object(o) => o.values().for_each(|x| collect_strings(x, out)),
        _ => {}
    }
}

impl Adapter for ClaudeCodeHook {
    type Request = ClaudeCodeInput;
    type Response = ClaudeCodeResponse;

    fn checkpoint_id(&self) -> &'static str {
        "anthropic.claude_code_hook.v1"
    }

    fn transcript<'a>(&self, body: &'a ClaudeCodeInput) -> Option<Transcript<'a>> {
        Some(Transcript {
            texts: body.texts(),
            request_id: body.session_id.as_deref(),
            surface: Some("claude-code"),
        })
    }

    fn render(&self, decision: &Decision) -> ClaudeCodeResponse {
        let blocked = decision.outcome == Outcome::Block;
        match self.style {
            ClaudeCodeStyle::PreToolUse => ClaudeCodeResponse::PreToolUse {
                hook_specific_output: PermissionOutput {
                    hook_event_name: "PreToolUse",
                    // `ask` is deliberately never emitted. A grounding
                    // verdict is a statement about evidence, and handing a
                    // human a prompt that says "this citation does not
                    // verify, proceed?" converts a checkable fact into a
                    // judgement call under time pressure.
                    permission_decision: if blocked { "deny" } else { "allow" },
                    permission_decision_reason: blocked.then(|| decision.reason_line()),
                },
            },
            ClaudeCodeStyle::Prompt => ClaudeCodeResponse::Prompt {
                ok: !blocked,
                reason: blocked.then(|| decision.reason_line()),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::{Decision, DenyCode, Fix};

    fn blocked() -> Decision {
        Decision::block(
            DenyCode::ProvSig,
            Some("emem:fact:cell:cid".to_string()),
            Fix::RefreshToken,
        )
        .with_leaf("leaf_01HX")
    }

    /// The same evidence yields the same outcome through every checkpoint.
    ///
    /// This is the interoperability commitment, asserted rather than assumed:
    /// a guard that answered differently by vendor would be discriminating
    /// between agents on the basis of their host.
    #[test]
    fn every_checkpoint_reaches_the_same_outcome_on_the_same_evidence() {
        let d = blocked();
        let anthropic = AnthropicHook.render(&d);
        assert_eq!(anthropic.action, crate::frame::Action::Deny);

        let pre = ClaudeCodeHook {
            style: ClaudeCodeStyle::PreToolUse,
        }
        .render(&d);
        match pre {
            ClaudeCodeResponse::PreToolUse {
                hook_specific_output,
            } => {
                assert_eq!(hook_specific_output.permission_decision, "deny")
            }
            _ => panic!("wrong shape"),
        }

        let prompt = ClaudeCodeHook {
            style: ClaudeCodeStyle::Prompt,
        }
        .render(&d);
        assert_eq!(
            prompt,
            ClaudeCodeResponse::Prompt {
                ok: false,
                reason: Some(d.reason_line())
            }
        );

        // And the reason text is identical everywhere, so an agent that
        // learned to parse it on one checkpoint can parse it on all of them.
        assert_eq!(anthropic.deny_reason.unwrap(), d.reason_line());
    }

    /// Blocking in Claude Code is 2xx-with-a-deny-body, so the deny must be
    /// IN the body. A response that only carried a status would enforce
    /// nothing.
    #[test]
    fn a_claude_code_block_is_expressed_in_the_body() {
        let r = ClaudeCodeHook::default().render(&blocked());
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains(r#""permissionDecision":"deny""#), "{json}");
        assert!(
            json.contains("EMEM-GUARD DENY"),
            "the reason must travel with it: {json}"
        );
    }

    /// `ask` is never emitted: see the comment at the call site.
    #[test]
    fn a_grounding_verdict_never_asks_a_human_to_arbitrate() {
        for d in [blocked(), Decision::proceed()] {
            let r = ClaudeCodeHook::default().render(&d);
            let json = serde_json::to_string(&r).unwrap();
            assert!(!json.contains("\"ask\""), "{json}");
        }
    }

    /// An unevaluable event proceeds rather than erroring, on every adapter.
    #[test]
    fn an_event_we_cannot_judge_yields_no_transcript_and_so_proceeds() {
        let f: PromptFrame =
            serde_json::from_str(r#"{"type":"some_future_event","request_id":"r"}"#).unwrap();
        assert!(AnthropicHook.transcript(&f).is_none());
    }

    /// Citations in Claude Code live in tool input and response, which are
    /// arbitrary JSON, so string leaves at any depth must be reachable.
    #[test]
    fn claude_code_citations_are_found_at_any_json_depth() {
        let input = ClaudeCodeInput {
            prompt: Some("check this".into()),
            tool_response: Some(serde_json::json!({
                "facts": [{"nested": {"deep": "emem:fact:cell:cid"}}]
            })),
            ..Default::default()
        };
        let t = ClaudeCodeHook::default().transcript(&input).unwrap();
        let found = crate::tokens::scan_all(t.texts.iter().copied());
        assert_eq!(
            found.len(),
            1,
            "a citation nested in a tool response must be found"
        );
        assert_eq!(found[0].token, "emem:fact:cell:cid");
    }
}
