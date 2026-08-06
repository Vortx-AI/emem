//! The inbound prompt frame, and the verdict that answers it.
//!
//! Both shapes are the platform's, not ours, so this module's job is to
//! parse permissively and answer exactly. The spec is unusually explicit
//! about forward compatibility, and every rule it states is encoded here as
//! a type decision rather than a comment:
//!
//!   - Unknown TOP-LEVEL fields, unknown `metadata` keys, new
//!     `source.application` values, new `actor.type` values and unrecognised
//!     content-block types must all be tolerated. So nothing here is
//!     `deny_unknown_fields`, every open string stays a `String` rather than
//!     an enum, and an unknown block keeps its raw JSON instead of failing
//!     the parse.
//!   - An unknown top-level `type` must return ALLOW, not an error. A new
//!     event type still needs a verdict, and an error response is a webhook
//!     failure that counts toward the circuit breaker.
//!
//! The asymmetry is the thing to hold on to: a webhook failure is NOT a
//! deny. Under fail-open policy it means the prompt reaches the model
//! uninspected, so being strict here buys nothing and costs inspection.

use serde::{Deserialize, Serialize};

/// The hook event this frame carries.
///
/// `Other` exists because the spec promises more event types and requires an
/// allow rather than an error for them. Modelling it as an enum with a
/// catch-all keeps that decision in the type system instead of in whoever
/// writes the next match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventType {
    /// `"prompt"`: the only event today. Fires once per governed inference
    /// request, before inference begins.
    Prompt,
    /// Anything else the platform introduces later.
    Other(String),
}

impl EventType {
    /// Whether this responder knows how to form an opinion about the event.
    ///
    /// False means the caller must allow: we cannot evaluate what we do not
    /// understand, and refusing would be a webhook failure rather than a
    /// deny.
    pub fn is_evaluable(&self) -> bool {
        matches!(self, Self::Prompt)
    }
}

impl Serialize for EventType {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(match self {
            Self::Prompt => "prompt",
            Self::Other(s) => s.as_str(),
        })
    }
}

impl<'de> Deserialize<'de> for EventType {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ok(match s.as_str() {
            "prompt" => Self::Prompt,
            _ => Self::Other(s),
        })
    }
}

/// The principal a request is attributed to.
///
/// `actor` is a union discriminated on `type`, and `"user"` is the only kind
/// sent today; the spec says a future kind "guarantees only that `type` is
/// present", so every other field is optional.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Actor {
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub email_address: Option<String>,
}

/// The originating application.
///
/// `application` is documented as "an open string, not a closed enum", and
/// as advisory routing metadata rather than a trust boundary. It is a
/// `String` for the first reason and must not gate a security decision on
/// its own for the second.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Source {
    #[serde(default)]
    pub application: Option<String>,
}

/// One block of message content.
///
/// `Unknown` keeps the raw JSON rather than discarding it: the spec permits
/// a policy to "inspect whatever other fields are present" on a block type it
/// does not recognise, and a future block could well carry the very text a
/// grounding check wants to read.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        #[serde(default)]
        text: String,
    },
    ToolUse {
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        tool_name: Option<String>,
        #[serde(default)]
        input: Option<serde_json::Value>,
    },
    ToolResult {
        #[serde(default)]
        content: Option<String>,
        #[serde(default)]
        is_error: Option<bool>,
        #[serde(default)]
        tool_name: Option<String>,
        #[serde(default)]
        tool_use_id: Option<String>,
    },
    Attachment {
        #[serde(default)]
        file_name: Option<String>,
        #[serde(default)]
        media_type: Option<String>,
        #[serde(default)]
        size_bytes: Option<u64>,
        /// Extracted text. Raw bytes are never sent, so this is all a policy
        /// gets from an attachment; an image-only document yields nothing.
        #[serde(default)]
        text: Option<String>,
    },
    #[serde(other)]
    Unknown,
}

impl ContentBlock {
    /// Every piece of human-readable text this block contributes.
    ///
    /// Used by the token scanner. Tool RESULTS matter as much as user text
    /// here: an agent that recalled a fact and is now reasoning from it
    /// carries the token in a tool result, not in what the person typed.
    pub fn texts(&self) -> Vec<&str> {
        match self {
            Self::Text { text } => vec![text.as_str()],
            Self::ToolResult { content, .. } => content.iter().map(String::as_str).collect(),
            Self::Attachment { text, .. } => text.iter().map(String::as_str).collect(),
            // A tool_use carries arguments rather than prose; its input is
            // scanned separately by the caller when needed, since it is
            // arbitrary JSON rather than text.
            Self::ToolUse { .. } | Self::Unknown => Vec::new(),
        }
    }
}

/// One turn of the transcript.
///
/// The spec warns that "a turn whose every block is excluded is omitted
/// entirely, so don't assume strict user and assistant alternation".
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Message {
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub content: Vec<ContentBlock>,
}

/// The request body an Anthropic Inference hook POSTs.
///
/// One adapter's wire shape. Nothing above [`crate::checkpoint`] parses it,
/// and the open routes never see it.
///
/// Deliberately NOT `deny_unknown_fields`: the spec requires unknown
/// top-level fields to be ignored, and it warns that requests "currently also
/// carry deprecated legacy aliases of some of these fields", which a strict
/// parser would choke on.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PromptFrame {
    #[serde(rename = "type")]
    pub event_type: EventType,
    /// Opaque per-inference-call id. Equals the `webhook-id` header, and is
    /// the idempotency key for deduplication.
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub tenant_id: Option<String>,
    #[serde(default)]
    pub actor: Option<Actor>,
    #[serde(default)]
    pub source: Option<Source>,
    #[serde(default)]
    pub messages: Vec<Message>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// "Reserved extension map ... sent empty today. Require nothing from it,
    /// and tolerate its absence, its presence, and any keys that appear."
    #[serde(default)]
    pub metadata: std::collections::BTreeMap<String, String>,
}

impl PromptFrame {
    /// Every readable string in the transcript, in order.
    pub fn texts(&self) -> Vec<&str> {
        self.messages
            .iter()
            .flat_map(|m| m.content.iter().flat_map(ContentBlock::texts))
            .collect()
    }
}

/// What we send back.
///
/// Serialized with `deny_reason` and `reference_id` omitted when absent,
/// because an allow verdict is documented as exactly `{"action": "allow"}`
/// and there is no reason to send fields the platform will ignore on that
/// path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Verdict {
    pub action: Action,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deny_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_id: Option<String>,
}

/// `allow` or `deny`. Nothing else: the spec states that "any `action` value
/// other than `allow` or `deny` is treated as a webhook failure".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Allow,
    Deny,
}

/// `deny_reason` is shown to the end user and truncated at 500 characters.
pub const DENY_REASON_MAX: usize = 500;
/// `reference_id` is capped at 50 characters from `[A-Za-z0-9._:/-]`.
pub const REFERENCE_ID_MAX: usize = 50;

impl Verdict {
    /// Let the request through.
    pub fn allow() -> Self {
        Self {
            action: Action::Allow,
            deny_reason: None,
            reference_id: None,
        }
    }

    /// Block the request, telling the USER what to change.
    ///
    /// The spec's guidance is explicit: "Tell them what to change ... rather
    /// than emitting a scanner code that only your team can interpret." The
    /// reason is truncated here rather than at the platform, on a character
    /// boundary, so we control where the sentence breaks instead of having it
    /// cut mid-word.
    pub fn deny(reason: impl Into<String>) -> Self {
        let mut r: String = reason.into();
        if r.chars().count() > DENY_REASON_MAX {
            r = r.chars().take(DENY_REASON_MAX - 1).collect::<String>() + "…";
        }
        Self {
            action: Action::Deny,
            deny_reason: Some(r),
            reference_id: None,
        }
    }

    /// Attach our own evaluation id, for joining a denial in the Activity
    /// Feed back to the signed record in our log.
    ///
    /// Silently drops a value that violates the platform's charset or length
    /// rule, because a malformed `reference_id` is "silently dropped" there
    /// anyway and the deny is still honoured. Sending one we know is invalid
    /// would only make the join fail further away from the cause.
    pub fn with_reference_id(mut self, id: impl Into<String>) -> Self {
        let id: String = id.into();
        let ok = !id.is_empty()
            && id.len() <= REFERENCE_ID_MAX
            && id.bytes().all(|b| {
                b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'/' | b'-')
            });
        self.reference_id = ok.then_some(id);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The spec's own example request body must parse.
    #[test]
    fn the_documented_example_frame_parses() {
        let raw = r#"{
          "type": "prompt",
          "request_id": "req_abc123",
          "tenant_id": "11111111-1111-1111-1111-111111111111",
          "actor": {"type":"user","id":"user_01AbCd","email_address":"alice@example.com"},
          "source": {"application": "claude-ai"},
          "session_id": "22222222-2222-2222-2222-222222222222",
          "model": "claude-sonnet-4-5",
          "messages": [{"role":"user","content":[
            {"type":"text","text":"Summarize the attached report."},
            {"type":"attachment","file_name":"q2-report.pdf","media_type":"application/pdf",
             "size_bytes":48213,"text":"Q2 revenue grew 14% quarter over quarter..."}
          ]}],
          "metadata": {}
        }"#;
        let f: PromptFrame = serde_json::from_str(raw).expect("the documented example must parse");
        assert_eq!(f.event_type, EventType::Prompt);
        assert_eq!(f.request_id.as_deref(), Some("req_abc123"));
        assert_eq!(f.texts().len(), 2, "attachment text is transcript text");
        assert!(f.texts()[1].contains("Q2 revenue"));
    }

    /// Every forward-compatibility rule the spec lists, in one frame.
    ///
    /// A parser that rejects any of these turns a protocol addition into a
    /// webhook failure, and sustained failures trip the circuit breaker,
    /// which stops enforcement entirely. Being strict here is how a server
    /// takes itself offline.
    #[test]
    fn additions_the_spec_promises_do_not_break_the_parse() {
        let raw = r#"{
          "type": "prompt",
          "request_id": "req_1",
          "brand_new_top_level_field": {"anything": true},
          "actor": {"type":"service_account","id":"svc_1"},
          "source": {"application": "some-future-surface"},
          "metadata": {"future_key": "value"},
          "messages": [{"role":"user","content":[
            {"type":"text","text":"hello"},
            {"type":"holographic_projection","frames":[1,2,3]}
          ]}]
        }"#;
        let f: PromptFrame = serde_json::from_str(raw).expect("additions must not break the parse");
        assert_eq!(
            f.metadata.get("future_key").map(String::as_str),
            Some("value")
        );
        assert_eq!(
            f.actor.as_ref().unwrap().kind.as_deref(),
            Some("service_account")
        );
        assert_eq!(
            f.source.as_ref().unwrap().application.as_deref(),
            Some("some-future-surface")
        );
        assert!(matches!(f.messages[0].content[1], ContentBlock::Unknown));
        assert_eq!(f.texts(), vec!["hello"], "the known block is still read");
    }

    /// An unrecognised EVENT still needs a verdict, and the spec says that
    /// verdict is allow. Returning an error would be a webhook failure.
    #[test]
    fn an_unknown_event_type_is_evaluable_by_nobody_and_must_allow() {
        let f: PromptFrame =
            serde_json::from_str(r#"{"type":"response","request_id":"r"}"#).unwrap();
        assert_eq!(f.event_type, EventType::Other("response".into()));
        assert!(!f.event_type.is_evaluable());
    }

    /// The wire shape of an allow is exactly the documented object.
    #[test]
    fn an_allow_serializes_to_the_documented_object() {
        assert_eq!(
            serde_json::to_string(&Verdict::allow()).unwrap(),
            r#"{"action":"allow"}"#
        );
    }

    #[test]
    fn a_deny_carries_its_reason_and_is_truncated_on_a_char_boundary() {
        let v = Verdict::deny("nope").with_reference_id("emem:v:01HXPT");
        let s = serde_json::to_string(&v).unwrap();
        assert!(s.contains(r#""action":"deny""#));
        assert!(s.contains(r#""deny_reason":"nope""#));
        assert!(s.contains(r#""reference_id":"emem:v:01HXPT""#));

        // Multi-byte input must not be cut mid-character.
        let long = "é".repeat(DENY_REASON_MAX + 50);
        let v = Verdict::deny(long);
        let r = v.deny_reason.unwrap();
        assert!(r.chars().count() <= DENY_REASON_MAX);
        assert!(r.ends_with('…'));
    }

    /// A reference_id the platform would silently drop is dropped here, so
    /// the join fails at the source rather than mysteriously downstream.
    #[test]
    fn an_invalid_reference_id_is_not_sent() {
        assert!(Verdict::allow()
            .with_reference_id("has spaces")
            .reference_id
            .is_none());
        assert!(Verdict::allow()
            .with_reference_id("a".repeat(51))
            .reference_id
            .is_none());
        assert!(Verdict::allow()
            .with_reference_id("")
            .reference_id
            .is_none());
        assert_eq!(
            Verdict::allow()
                .with_reference_id("ok.-_:/09Az")
                .reference_id
                .as_deref(),
            Some("ok.-_:/09Az")
        );
    }
}
