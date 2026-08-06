//! Checkpoints that belong to standards rather than to vendors.
//!
//! [`crate::open`] gives an agent a shape we designed. This module gives it
//! shapes it is probably already speaking, so adopting the guard costs a URL
//! rather than an integration.
//!
//! Four, each chosen because it is an interchange format with more than one
//! implementation behind it and no owner who can revoke it:
//!
//! | Adapter | Speaks to | Route |
//! |---|---|---|
//! | [`McpGate`] | any MCP host or proxy, gating a tool call | `POST /verdict/mcp` |
//! | [`OpenAiGate`] | anything holding an OpenAI-shaped client | `POST /verdict/openai` |
//! | [`CloudEventGate`] | CNCF CloudEvents producers and eventing meshes | `POST /verdict/cloudevent` |
//! | [`PolicyPointGate`] | OPA-compatible policy clients, Envoy ext_authz | `POST /verdict/policy` |
//!
//! # The rule these all obey
//!
//! Reading a shape is not conformance to the API that shape came from. Each
//! adapter here reads a body for its text and answers in a matching envelope.
//! None of them claims to implement the vendor API whose shape it borrowed,
//! and [`OpenAiGate`] says so in its own response, because that is the one
//! where a caller could plausibly mistake us for a drop-in replacement and
//! silently lose a control they were relying on.

use serde::{Deserialize, Serialize};

use crate::checkpoint::{collect_strings, Adapter, Outcome, Transcript};
use crate::open::NativeVerdict;
use crate::policy::{Decision, DenyCode, Fix};

/// How many transcripts one batch may carry.
///
/// A batch is the one route where a single request can ask for unbounded work,
/// and every verdict in it signs and appends to the log. The cap is on the
/// request rather than on a rate limiter because it has to hold on a node
/// nobody put a proxy in front of.
pub const MAX_BATCH: usize = 64;

// ── MCP: gate a tool call ────────────────────────────────────────────────

/// The Model Context Protocol tool-call checkpoint.
///
/// The most portable checkpoint that exists today. MCP is an open protocol
/// with many independent hosts, and every one of them has the same natural
/// pause: after the model asks for a tool and before the tool runs. A proxy
/// sitting there can ask this route whether to proceed and substitute the
/// returned result when the answer is no.
///
/// It also covers the direction the other checkpoints miss. A tool RESULT is
/// where a grounded agent's citations actually arrive, so gating results
/// catches a fabricated token at the moment it enters the context rather than
/// one turn later when the model has already reasoned on it.
#[derive(Debug, Clone, Copy, Default)]
pub struct McpGate;

/// A `tools/call`, either as a full JSON-RPC envelope or bare.
///
/// Both are accepted because a host that forwards the whole request and a
/// proxy that forwards only the params are both reasonable implementations,
/// and refusing one would make the route depend on an integration detail we
/// have no business having an opinion about.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct McpCall {
    /// JSON-RPC id, echoed back so a proxy can correlate.
    #[serde(default)]
    pub id: Option<serde_json::Value>,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub params: Option<McpParams>,
    /// The bare form: `name` and `arguments` at the top level.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<serde_json::Value>,
    /// A tool result being gated on the way back in.
    #[serde(default)]
    pub result: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct McpParams {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<serde_json::Value>,
    /// Present when the host is gating a result rather than a call.
    #[serde(default)]
    pub result: Option<serde_json::Value>,
    /// MCP carries `_meta` on requests; a host may put its own request id
    /// there, and using it keeps idempotency working across a retry.
    #[serde(default, rename = "_meta")]
    pub meta: Option<serde_json::Value>,
}

impl McpCall {
    /// The tool name, from whichever form carried it.
    pub fn tool_name(&self) -> Option<&str> {
        self.params
            .as_ref()
            .and_then(|p| p.name.as_deref())
            .or(self.name.as_deref())
    }

    fn request_id(&self) -> Option<&str> {
        self.params
            .as_ref()
            .and_then(|p| p.meta.as_ref())
            .and_then(|m| m.get("requestId").or_else(|| m.get("request_id")))
            .and_then(|v| v.as_str())
    }
}

/// What a proxy does with the answer.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct McpGateResponse {
    /// Whether the call may run.
    pub allow: bool,
    /// Echoed JSON-RPC id, when one arrived.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<serde_json::Value>,
    /// On a deny, the `CallToolResult` the host should return INSTEAD of
    /// running the tool.
    ///
    /// Pre-built rather than described, so a proxy does not have to know our
    /// reason grammar to enforce correctly. `isError` is set, which is how MCP
    /// reports a tool-level failure to the model, so the model sees the deny
    /// and the remedy in the place it already reads errors.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    /// The full verdict, for a proxy that wants the evidence rather than the
    /// substitution.
    pub verdict: NativeVerdict,
}

impl Adapter for McpGate {
    type Request = McpCall;
    type Response = McpGateResponse;

    fn checkpoint_id(&self) -> &'static str {
        "mcp.tools_call.v1"
    }

    fn transcript<'a>(&self, body: &'a McpCall) -> Option<Transcript<'a>> {
        let mut texts: Vec<&str> = Vec::new();
        if let Some(n) = body.tool_name() {
            texts.push(n);
        }
        for v in [
            body.params.as_ref().and_then(|p| p.arguments.as_ref()),
            body.params.as_ref().and_then(|p| p.result.as_ref()),
            body.arguments.as_ref(),
            body.result.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            collect_strings(v, &mut texts);
        }
        Some(Transcript {
            texts,
            request_id: body.request_id(),
            surface: Some("mcp"),
        })
    }

    fn render(&self, d: &Decision) -> McpGateResponse {
        let blocked = d.outcome == Outcome::Block;
        McpGateResponse {
            allow: !blocked,
            id: None,
            result: blocked.then(|| {
                serde_json::json!({
                    "content": [{"type": "text", "text": d.reason_line()}],
                    "isError": true,
                })
            }),
            verdict: crate::open::NativeHook.render(d),
        }
    }
}

// ── OpenAI-shaped ────────────────────────────────────────────────────────

/// A checkpoint in the shape every OpenAI-compatible client already speaks.
///
/// Reach, not endorsement. The chat-completions message array and the
/// moderations request are the two most widely reimplemented request bodies in
/// the ecosystem: local runtimes, gateways, routers and half the agent
/// frameworks emit one or the other. Accepting them means an agent on any
/// model can call this with a client it already has.
///
/// # The one thing this is NOT
///
/// It is not a content-safety moderation endpoint and it must not be swapped
/// in for one. It checks grounding, and it detects none of the harm categories
/// a moderation API exists for. That is stated in every response it emits, in
/// [`OpenAiResults::note`], and the route is deliberately NOT `/v1/moderations`
/// so that adopting it takes a decision rather than a base-URL change. A
/// caller who replaced their moderation endpoint with this one would lose a
/// control and gain a different one, and would have no way to notice.
#[derive(Debug, Clone, Copy, Default)]
pub struct OpenAiGate;

/// Either shape, read for its text.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct OpenAiRequest {
    /// The moderations shape: a string or an array of strings.
    #[serde(default)]
    pub input: Option<OpenAiInput>,
    /// The chat-completions shape.
    #[serde(default)]
    pub messages: Vec<crate::open::NativeMessage>,
    /// Echoed back, as the moderations response carries it.
    #[serde(default)]
    pub model: Option<String>,
    /// Optional idempotency key. Not part of the OpenAI schema; read when
    /// present because a caller who has one deserves replay safety.
    #[serde(default)]
    pub request_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum OpenAiInput {
    One(String),
    Many(Vec<String>),
}

impl OpenAiRequest {
    fn texts(&self) -> Vec<&str> {
        let mut v: Vec<&str> = match &self.input {
            Some(OpenAiInput::One(s)) => vec![s.as_str()],
            Some(OpenAiInput::Many(ss)) => ss.iter().map(String::as_str).collect(),
            None => Vec::new(),
        };
        v.extend(self.messages.iter().flat_map(|m| m.content.texts()));
        v
    }
}

/// The moderations-shaped answer.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct OpenAiResponse {
    pub id: String,
    pub model: String,
    pub results: Vec<OpenAiResults>,
}

/// One result, carrying our categories rather than a moderation vendor's.
///
/// Not `Eq`: the borrowed schema has a float field. Every value it can hold is
/// exactly 0 or 1, so the derived `PartialEq` is total in practice, but
/// asserting `Eq` over an `f64` would be claiming a property the type does not
/// have.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct OpenAiResults {
    /// True when a rule fired. The field a moderation client branches on, so
    /// an existing integration works without change.
    pub flagged: bool,
    /// Namespaced under `emem/` so nobody can mistake one of ours for one of
    /// theirs, and so a client reading `categories.violence` gets an honest
    /// absence rather than a false negative dressed as a pass.
    pub categories: std::collections::BTreeMap<String, bool>,
    /// Present because the shape has it. Every value is 0 or 1: a grounding
    /// verdict is established or it is not, and emitting a fabricated
    /// confidence to fill a field would be the one dishonest thing in this
    /// crate.
    pub category_scores: std::collections::BTreeMap<String, f64>,
    /// What this endpoint does and does not check. Extension field, always
    /// present, so the limitation travels with the answer rather than living
    /// only in documentation nobody re-reads.
    pub note: &'static str,
    /// The full verdict, for a caller that wants the evidence.
    pub emem: NativeVerdict,
}

/// The sentence that keeps this route honest.
const OPENAI_NOTE: &str = "emem-guard checks grounding: whether cited observations verify and \
     whether measurable claims about the physical world carry a citation. It \
     detects no content-safety category and is not a replacement for a \
     moderation endpoint.";

/// Every category this route can report, with the deny code it comes from.
fn openai_categories() -> [(&'static str, Option<DenyCode>); 4] {
    [
        ("emem/unverified_citation", Some(DenyCode::ProvSig)),
        ("emem/altered_citation", Some(DenyCode::ProvBytes)),
        ("emem/stale_observation", Some(DenyCode::ProvDrift)),
        ("emem/ungrounded_claim", Some(DenyCode::ClaimUngrounded)),
    ]
}

impl Adapter for OpenAiGate {
    type Request = OpenAiRequest;
    type Response = OpenAiResponse;

    fn checkpoint_id(&self) -> &'static str {
        "openai.compatible.v1"
    }

    fn transcript<'a>(&self, body: &'a OpenAiRequest) -> Option<Transcript<'a>> {
        Some(Transcript {
            texts: body.texts(),
            request_id: body.request_id.as_deref(),
            surface: Some("openai-compatible"),
        })
    }

    fn render(&self, d: &Decision) -> OpenAiResponse {
        let mut categories = std::collections::BTreeMap::new();
        let mut scores = std::collections::BTreeMap::new();
        for (name, code) in openai_categories() {
            let hit = d.code.is_some() && d.code == code;
            categories.insert(name.to_string(), hit);
            scores.insert(name.to_string(), if hit { 1.0 } else { 0.0 });
        }
        OpenAiResponse {
            id: d
                .leaf
                .clone()
                .map(|l| format!("modr-{l}"))
                .unwrap_or_else(|| "modr-unlogged".to_string()),
            model: "emem-guard".to_string(),
            results: vec![OpenAiResults {
                flagged: d.outcome == Outcome::Block,
                categories,
                category_scores: scores,
                note: OPENAI_NOTE,
                emem: crate::open::NativeHook.render(d),
            }],
        }
    }
}

// ── CloudEvents ──────────────────────────────────────────────────────────

/// A CNCF CloudEvents checkpoint.
///
/// The envelope an eventing mesh already puts around everything. A guard
/// reachable this way drops into Knative, Dapr, Argo Events and anything else
/// that speaks the spec, with no adapter written on the caller's side.
///
/// Structured mode only: the event is a JSON object with `specversion` and a
/// `data` member. Binary mode puts the attributes in `ce-*` headers and the
/// data in the body, which the server handles by reading the body as the data
/// and taking the id from the header, so both modes reach the same code.
#[derive(Debug, Clone, Copy, Default)]
pub struct CloudEventGate;

/// A CloudEvent in structured mode.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CloudEvent {
    #[serde(default)]
    pub specversion: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default, rename = "type")]
    pub event_type: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    /// The payload. Read for its string leaves at any depth, because a
    /// producer's data schema is its own business.
    #[serde(default)]
    pub data: Option<serde_json::Value>,
}

/// A CloudEvent carrying the verdict.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CloudEventResponse {
    pub specversion: &'static str,
    pub id: String,
    #[serde(rename = "type")]
    pub event_type: &'static str,
    pub source: &'static str,
    pub datacontenttype: &'static str,
    /// The id of the event this answers, so a mesh can correlate without
    /// inspecting the payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    pub data: NativeVerdict,
}

impl Adapter for CloudEventGate {
    type Request = CloudEvent;
    type Response = CloudEventResponse;

    fn checkpoint_id(&self) -> &'static str {
        "cloudevents.v1"
    }

    fn transcript<'a>(&self, body: &'a CloudEvent) -> Option<Transcript<'a>> {
        let mut texts: Vec<&str> = Vec::new();
        if let Some(d) = body.data.as_ref() {
            collect_strings(d, &mut texts);
        }
        Some(Transcript {
            texts,
            // The CloudEvents id is required to be unique per source, which is
            // exactly the property an idempotency key needs.
            request_id: body.id.as_deref(),
            surface: body.source.as_deref(),
        })
    }

    fn render(&self, d: &Decision) -> CloudEventResponse {
        CloudEventResponse {
            specversion: "1.0",
            id: d.leaf.clone().unwrap_or_else(|| "unlogged".to_string()),
            event_type: match d.outcome {
                Outcome::Proceed => "dev.emem.guard.verdict.allow",
                Outcome::Block => "dev.emem.guard.verdict.deny",
            },
            source: "/emem-guard",
            datacontenttype: "application/json",
            subject: None,
            data: crate::open::NativeHook.render(d),
        }
    }
}

// ── Policy decision point ────────────────────────────────────────────────

/// The `{input} -> {result}` shape every OPA-compatible client speaks.
///
/// The lingua franca of authorisation in cloud-native infrastructure: OPA
/// itself, Envoy's external authorisation filter, Gatekeeper, and a long tail
/// of gateways all send an `input` object and read a `result`. An org that
/// already has a policy decision point in the path can add this as one more
/// without teaching anything a new protocol.
///
/// `result.allow` is the boolean, and `result.deny` is an array of reason
/// strings, which is the Rego convention: an empty `deny` set means allowed.
/// Both are emitted so a client written against either style works.
#[derive(Debug, Clone, Copy, Default)]
pub struct PolicyPointGate;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PolicyPointRequest {
    /// Whatever the caller's policy input is. Read for its string leaves.
    #[serde(default)]
    pub input: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PolicyPointResponse {
    pub result: PolicyPointResult,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PolicyPointResult {
    pub allow: bool,
    /// Rego convention: a non-empty set means denied.
    pub deny: Vec<String>,
    pub verdict: NativeVerdict,
}

impl Adapter for PolicyPointGate {
    type Request = PolicyPointRequest;
    type Response = PolicyPointResponse;

    fn checkpoint_id(&self) -> &'static str {
        "policy_point.v1"
    }

    fn transcript<'a>(&self, body: &'a PolicyPointRequest) -> Option<Transcript<'a>> {
        let mut texts: Vec<&str> = Vec::new();
        if let Some(i) = body.input.as_ref() {
            collect_strings(i, &mut texts);
        }
        let request_id = body
            .input
            .as_ref()
            .and_then(|i| i.get("request_id").or_else(|| i.get("requestId")))
            .and_then(|v| v.as_str());
        Some(Transcript {
            texts,
            request_id,
            surface: Some("policy-point"),
        })
    }

    fn render(&self, d: &Decision) -> PolicyPointResponse {
        let blocked = d.outcome == Outcome::Block;
        PolicyPointResponse {
            result: PolicyPointResult {
                allow: !blocked,
                deny: if blocked {
                    vec![d.reason_line()]
                } else {
                    Vec::new()
                },
                verdict: crate::open::NativeHook.render(d),
            },
        }
    }
}

// ── Batch ────────────────────────────────────────────────────────────────

/// Many transcripts, one request.
///
/// Not a checkpoint. This is the offline shape: an org pointing the guard at
/// an archive of conversations to find out what a rule would have done before
/// putting it anywhere near live traffic. Every item is evaluated, signed and
/// logged exactly as a live verdict is, so the numbers a shadow run produces
/// come from the same path that will run in production.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct BatchRequest {
    #[serde(default)]
    pub items: Vec<crate::open::NativeRequest>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BatchResponse {
    pub verdicts: Vec<NativeVerdict>,
    /// How many items were dropped for exceeding [`MAX_BATCH`].
    ///
    /// Said out loud rather than silently truncated: a caller who sent 200 and
    /// got 64 answers must not be able to read that as 200 clean transcripts.
    pub dropped: usize,
}

/// The parts of a verdict that a caller can act on without our reason grammar.
///
/// Used by the descriptor so an agent can learn the whole contract from one
/// GET rather than from prose.
pub fn deny_codes() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            DenyCode::ProvSig.as_str(),
            "a cited token's signature did not verify",
        ),
        (
            DenyCode::ProvBytes.as_str(),
            "a cited token resolved to different content than claimed",
        ),
        (
            DenyCode::ProvDrift.as_str(),
            "a cited reading has drifted past its band's threshold",
        ),
        (
            DenyCode::GeoZone.as_str(),
            "a cited address falls in an operator-restricted zone",
        ),
        (
            DenyCode::ClaimUngrounded.as_str(),
            "a measurable claim about a place carried no citation",
        ),
    ]
}

/// Every remedy, with what the caller should do about it.
pub fn fixes() -> Vec<(&'static str, &'static str)> {
    vec![
        (Fix::RefreshToken.as_str(), "re-resolve the token and retry"),
        (
            Fix::RemoveReference.as_str(),
            "the citation cannot be made to verify; drop it",
        ),
        (
            Fix::ContactAdmin.as_str(),
            "a person restricted this, not the evidence",
        ),
        (
            Fix::CiteObservation.as_str(),
            "resolve the observation through emem and cite the token",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::{Decision, DenyCode, Fix};

    fn blocked() -> Decision {
        Decision::block(
            DenyCode::ProvSig,
            Some("emem:fact:cell:cid".into()),
            Fix::RefreshToken,
        )
        .with_leaf("leaf_9")
        .with_checked(2)
    }

    /// The commitment the whole crate rests on, extended to every new shape:
    /// the same evidence reaches the same outcome regardless of which envelope
    /// carried the question.
    #[test]
    fn every_interop_shape_reaches_the_same_outcome() {
        let d = blocked();
        assert!(!McpGate.render(&d).allow);
        assert!(OpenAiGate.render(&d).results[0].flagged);
        assert_eq!(
            CloudEventGate.render(&d).event_type,
            "dev.emem.guard.verdict.deny"
        );
        assert!(!PolicyPointGate.render(&d).result.allow);

        let a = Decision::proceed();
        assert!(McpGate.render(&a).allow);
        assert!(!OpenAiGate.render(&a).results[0].flagged);
        assert_eq!(
            CloudEventGate.render(&a).event_type,
            "dev.emem.guard.verdict.allow"
        );
        assert!(PolicyPointGate.render(&a).result.allow);
    }

    /// And the reason text is byte-identical everywhere, so an agent needs one
    /// parser however it reached us.
    #[test]
    fn the_reason_line_is_the_same_through_every_envelope() {
        let d = blocked();
        let want = d.reason_line();
        let mcp = McpGate.render(&d);
        assert_eq!(
            mcp.result.unwrap()["content"][0]["text"].as_str().unwrap(),
            want
        );
        assert_eq!(PolicyPointGate.render(&d).result.deny, vec![want.clone()]);
        assert_eq!(
            CloudEventGate.render(&d).data.reason.as_deref(),
            Some(want.as_str())
        );
        assert_eq!(
            OpenAiGate.render(&d).results[0].emem.reason.as_deref(),
            Some(want.as_str())
        );
    }

    /// A JSON-RPC `tools/call` and a bare params object are both accepted,
    /// because both are reasonable proxy implementations.
    #[test]
    fn an_mcp_call_is_read_in_either_form() {
        let full: McpCall = serde_json::from_str(
            r#"{"jsonrpc":"2.0","id":7,"method":"tools/call",
                "params":{"name":"emem_recall","arguments":{"q":"see emem:fact:a:b"}}}"#,
        )
        .unwrap();
        let t = McpGate.transcript(&full).unwrap();
        assert!(t.texts.contains(&"emem_recall"));
        assert_eq!(crate::tokens::scan_all(t.texts.iter().copied()).len(), 1);

        let bare: McpCall =
            serde_json::from_str(r#"{"name":"read","arguments":{"path":"emem:fact:a:b"}}"#)
                .unwrap();
        assert_eq!(
            crate::tokens::scan_all(McpGate.transcript(&bare).unwrap().texts.iter().copied()).len(),
            1
        );
    }

    /// A tool RESULT is where a grounded agent's citations arrive, so gating
    /// results is the direction that catches a fabricated token as it enters
    /// the context rather than a turn later.
    #[test]
    fn an_mcp_tool_result_is_scanned_at_any_depth() {
        let call: McpCall = serde_json::from_str(
            r#"{"method":"tools/call","params":{"name":"t",
                "result":{"content":[{"type":"text","text":"per emem:fact:x:y"}]}}}"#,
        )
        .unwrap();
        let t = McpGate.transcript(&call).unwrap();
        let found = crate::tokens::scan_all(t.texts.iter().copied());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].token, "emem:fact:x:y");
    }

    /// A denied MCP call gets the substitution pre-built, so a proxy does not
    /// have to know our grammar to enforce correctly.
    #[test]
    fn a_denied_mcp_call_carries_the_result_to_substitute() {
        let r = McpGate.render(&blocked());
        let result = r.result.expect("a deny must carry the substitution");
        assert_eq!(result["isError"], true);
        assert!(result["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("EMEM-GUARD DENY"));
        // An allow substitutes nothing: the real tool runs.
        assert!(McpGate.render(&Decision::proceed()).result.is_none());
    }

    /// Both OpenAI-shaped bodies are read, because callers emit both.
    #[test]
    fn both_openai_request_shapes_are_read() {
        let mod_one: OpenAiRequest = serde_json::from_str(r#"{"input":"emem:fact:a:b"}"#).unwrap();
        assert_eq!(mod_one.texts(), vec!["emem:fact:a:b"]);

        let mod_many: OpenAiRequest = serde_json::from_str(r#"{"input":["one","two"]}"#).unwrap();
        assert_eq!(mod_many.texts(), vec!["one", "two"]);

        let chat: OpenAiRequest = serde_json::from_str(
            r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hello"}]}"#,
        )
        .unwrap();
        assert_eq!(chat.texts(), vec!["hello"]);
    }

    /// The limitation travels with the answer. A caller who mistook this for a
    /// content-safety endpoint would lose a control silently, so the response
    /// itself says what it does not do, on every result including allows.
    #[test]
    fn the_openai_shape_states_what_it_does_not_check() {
        for d in [blocked(), Decision::proceed()] {
            let r = OpenAiGate.render(&d);
            assert!(r.results[0].note.contains("not a replacement"));
            // And no category can be mistaken for a moderation vendor's.
            for k in r.results[0].categories.keys() {
                assert!(k.starts_with("emem/"), "{k} is not namespaced");
            }
        }
    }

    /// Scores are 0 or 1 only. A grounding verdict is established or it is
    /// not, and inventing a confidence to fill a field in someone else's
    /// schema would be a fabricated number on a signed surface.
    #[test]
    fn no_confidence_is_invented_to_fill_a_borrowed_field() {
        let r = OpenAiGate.render(&blocked());
        for (k, v) in &r.results[0].category_scores {
            assert!(*v == 0.0 || *v == 1.0, "{k} = {v}");
            assert_eq!(*v == 1.0, r.results[0].categories[k], "{k} disagrees");
        }
    }

    /// A CloudEvent's payload schema is the producer's business, so citations
    /// are found wherever they are.
    #[test]
    fn a_cloudevent_payload_is_scanned_wherever_the_producer_put_it() {
        let ev: CloudEvent = serde_json::from_str(
            r#"{"specversion":"1.0","id":"e-1","type":"com.example.turn","source":"/agent",
                "data":{"turn":{"blocks":[{"text":"emem:fact:a:b"}]}}}"#,
        )
        .unwrap();
        let t = CloudEventGate.transcript(&ev).unwrap();
        assert_eq!(crate::tokens::scan_all(t.texts.iter().copied()).len(), 1);
        // And the id doubles as the idempotency key, which the spec already
        // requires to be unique per source.
        assert_eq!(t.request_id, Some("e-1"));
    }

    /// Rego convention: an empty deny set means allowed. A client written for
    /// either style has to work.
    #[test]
    fn the_policy_point_answers_in_both_rego_styles() {
        let allow = PolicyPointGate.render(&Decision::proceed());
        assert!(allow.result.allow);
        assert!(
            allow.result.deny.is_empty(),
            "an empty deny set is the allow"
        );
        let deny = PolicyPointGate.render(&blocked());
        assert!(!deny.result.allow);
        assert_eq!(deny.result.deny.len(), 1);
    }

    /// Every code and every fix is documented in the machine-readable list, or
    /// an agent reading the descriptor would meet one it had never been told
    /// about.
    #[test]
    fn the_published_contract_covers_every_code_and_fix() {
        let codes: Vec<&str> = deny_codes().into_iter().map(|(c, _)| c).collect();
        for c in [
            DenyCode::ProvSig,
            DenyCode::ProvBytes,
            DenyCode::ProvDrift,
            DenyCode::GeoZone,
            DenyCode::ClaimUngrounded,
        ] {
            assert!(codes.contains(&c.as_str()), "{} undocumented", c.as_str());
        }
        let fs: Vec<&str> = fixes().into_iter().map(|(f, _)| f).collect();
        for f in [
            Fix::RefreshToken,
            Fix::RemoveReference,
            Fix::ContactAdmin,
            Fix::CiteObservation,
        ] {
            assert!(fs.contains(&f.as_str()), "{} undocumented", f.as_str());
        }
        // And every entry says what it means, not just what it is called.
        for (name, desc) in deny_codes().into_iter().chain(fixes()) {
            assert!(desc.len() > 20, "{name} has no useful description");
        }
    }
}
