//! The LLM gateway. The merge call is REAL: an OpenAI-compatible
//! chat-completions request, or a round-trip through the responder's own
//! `/v1/ask` gateway. The transport is an injectable trait so the
//! merge-application logic can be unit-tested with a fixed mock response —
//! tests never touch a real API.

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::candidates::Candidate;
use crate::config::{LlmRoute, SleepAgentConfig};
use crate::ResponderClient;

/// The most completion tokens a merge call may use. Sent as `max_tokens`
/// so the cost of one call has an upper bound before it starts.
pub const MERGE_MAX_COMPLETION_TOKENS: u32 = 4096;

// Nominal blended small-model rates, USD per million tokens. They price
// both the reported usage and the worst case of a call not yet made.
const PROMPT_USD_PER_MTOK: f64 = 0.5;
const COMPLETION_USD_PER_MTOK: f64 = 1.5;

// Chat formatting adds a few tokens per message and for the reply primer.
const CHAT_OVERHEAD_TOKENS: f64 = 32.0;

/// What the agent hands the LLM: the system instruction plus the rendered
/// cluster of memory files to reconcile.
#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub system: String,
    pub user: String,
}

impl LlmRequest {
    /// Build a merge prompt from a candidate. Deterministic given the
    /// candidate (so a mock transport in tests sees a stable prompt).
    pub fn for_merge(cand: &Candidate, _cfg: &SleepAgentConfig) -> Self {
        let system = "You reconcile near-duplicate or contradictory agent memory entries into a \
             single, accurate, non-redundant note. Preserve every distinct fact; when sources \
             disagree, state the disagreement explicitly rather than silently picking one. \
             Output ONLY the merged memory text, no preamble."
            .to_string();
        let mut user = String::new();
        user.push_str("Merge the following memory entries into one:\n\n");
        if cand.cluster.is_empty() {
            user.push_str(&format!(
                "(No memory file is attached; reconcile the contradiction at {})\n",
                cand.canonical_path
            ));
        }
        for (i, f) in cand.cluster.iter().enumerate() {
            user.push_str(&format!(
                "--- entry {} (path: {}) ---\n{}\n\n",
                i + 1,
                f.path,
                f.content
            ));
        }
        LlmRequest { system, user }
    }

    /// The most a chat-completions call with this request can cost at the
    /// nominal rates. A byte-level BPE token covers at least one byte, so
    /// the prompt has no more tokens than bytes; the completion is capped
    /// by [`MERGE_MAX_COMPLETION_TOKENS`].
    pub fn nominal_max_cost_usd(&self) -> f64 {
        let prompt_tokens = (self.system.len() + self.user.len()) as f64 + CHAT_OVERHEAD_TOKENS;
        let completion_tokens = f64::from(MERGE_MAX_COMPLETION_TOKENS);
        (prompt_tokens * PROMPT_USD_PER_MTOK + completion_tokens * COMPLETION_USD_PER_MTOK)
            / 1_000_000.0
    }
}

/// The LLM's proposed merge.
#[derive(Debug, Clone)]
pub struct MergeProposal {
    /// The reconciled memory text to write back.
    pub merged_text: String,
    /// Estimated USD cost of the call (0.0 when the gateway does not
    /// report token usage, e.g. `/v1/ask`).
    pub cost_usd: f64,
}

/// Errors a transport can raise.
#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("transport: {0}")]
    Transport(String),
    #[error("malformed LLM response: {0}")]
    Malformed(String),
}

/// Injectable LLM transport. Implemented by the real HTTP gateway and by a
/// mock in tests.
// async_trait marks each declared method #[must_use] on a future that
// already is; the lint fires inside the macro's expansion, not our code.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait LlmTransport: Send + Sync {
    async fn propose_merge(&self, req: LlmRequest) -> Result<MergeProposal, LlmError>;

    /// The most `propose_merge(req)` can add to the pass spend. The budget
    /// gate refuses a call when spend plus this value passes the cap.
    fn max_cost_usd(&self, req: &LlmRequest) -> f64 {
        req.nominal_max_cost_usd()
    }
}

/// The real gateway. Either an OpenAI-compatible chat-completions endpoint
/// or the responder's `/v1/ask`. Constructed only when a route is actually
/// configured — `from_config` returns `None` for [`LlmRoute::None`], which
/// is what enforces the no-fabrication contract upstream.
pub struct HttpLlmTransport {
    inner: Inner,
    http: reqwest::Client,
}

enum Inner {
    OpenAi {
        url: String,
        model: String,
        api_key: String,
    },
    Ask {
        client: ResponderClient,
    },
}

impl HttpLlmTransport {
    /// Build a transport for the configured route. Returns `None` when no
    /// LLM is configured (the caller then runs dry instead of fabricating).
    pub fn from_config(cfg: &SleepAgentConfig) -> Option<Self> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("emem-sleep-agent/", env!("CARGO_PKG_VERSION")))
            .build()
            .ok()?;
        let inner = match &cfg.llm {
            LlmRoute::None => return None,
            LlmRoute::OpenAiCompatible {
                url,
                model,
                api_key,
            } => Inner::OpenAi {
                url: url.clone(),
                model: model.clone(),
                api_key: api_key.clone(),
            },
            LlmRoute::ResponderAsk => Inner::Ask {
                client: ResponderClient::new(cfg.responder_url.clone()),
            },
        };
        Some(HttpLlmTransport { inner, http })
    }

    async fn openai_chat(
        &self,
        url: &str,
        model: &str,
        api_key: &str,
        req: &LlmRequest,
    ) -> Result<MergeProposal, LlmError> {
        let body = json!({
            "model": model,
            "messages": [
                { "role": "system", "content": req.system },
                { "role": "user", "content": req.user },
            ],
            "temperature": 0.2,
            "max_tokens": MERGE_MAX_COMPLETION_TOKENS,
        });
        let resp = self
            .http
            .post(url)
            .bearer_auth(api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::Transport(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(LlmError::Transport(format!("{url} -> {}", resp.status())));
        }
        let v: Value = resp
            .json()
            .await
            .map_err(|e| LlmError::Malformed(e.to_string()))?;
        parse_chat_completion(&v)
    }
}

/// Read a merge proposal from a chat-completions response. A reply cut at
/// the token cap is an error: writing half a merge would supersede the
/// originals with a truncated note.
fn parse_chat_completion(v: &Value) -> Result<MergeProposal, LlmError> {
    if v.pointer("/choices/0/finish_reason")
        .and_then(|r| r.as_str())
        == Some("length")
    {
        return Err(LlmError::Malformed(format!(
            "merge reply stopped at the {MERGE_MAX_COMPLETION_TOKENS}-token cap; not written"
        )));
    }
    let merged_text = v
        .pointer("/choices/0/message/content")
        .and_then(|c| c.as_str())
        .ok_or_else(|| LlmError::Malformed("no choices[0].message.content".into()))?
        .trim()
        .to_string();
    if merged_text.is_empty() {
        return Err(LlmError::Malformed("LLM returned empty content".into()));
    }
    // Best-effort cost estimate from reported token usage. When the
    // endpoint does not report usage we leave it at 0.0 rather than
    // inventing a number.
    let cost_usd = estimate_cost(v);
    Ok(MergeProposal {
        merged_text,
        cost_usd,
    })
}

/// Very rough cost estimate from OpenAI-style `usage` totals at a nominal
/// blended rate. Honest about being an estimate; the budget cap is the
/// real guardrail.
fn estimate_cost(v: &Value) -> f64 {
    let prompt = v
        .pointer("/usage/prompt_tokens")
        .and_then(|t| t.as_f64())
        .unwrap_or(0.0);
    let completion = v
        .pointer("/usage/completion_tokens")
        .and_then(|t| t.as_f64())
        .unwrap_or(0.0);
    (prompt * PROMPT_USD_PER_MTOK + completion * COMPLETION_USD_PER_MTOK) / 1_000_000.0
}

#[async_trait]
impl LlmTransport for HttpLlmTransport {
    async fn propose_merge(&self, req: LlmRequest) -> Result<MergeProposal, LlmError> {
        match &self.inner {
            Inner::OpenAi {
                url,
                model,
                api_key,
            } => self.openai_chat(url, model, api_key, &req).await,
            Inner::Ask { client } => {
                // Route the merge through the responder's own gateway. The
                // prompt is the system + user concatenation; the gateway's
                // configured model does the rewrite.
                let question = format!("{}\n\n{}", req.system, req.user);
                let v = client
                    .ask(&question)
                    .await
                    .map_err(|e| LlmError::Transport(e.to_string()))?;
                // `/v1/ask` is structured; the merged text lives in the
                // answer/summary field depending on responder version. Try
                // the common shapes; if none carry free text, that is a
                // malformed response (no fabrication).
                let merged_text = v
                    .get("answer")
                    .and_then(|a| a.as_str())
                    .or_else(|| v.get("summary").and_then(|a| a.as_str()))
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        LlmError::Malformed(
                            "responder /v1/ask returned no free-text answer to use as a merge"
                                .into(),
                        )
                    })?;
                Ok(MergeProposal {
                    merged_text,
                    cost_usd: 0.0,
                })
            }
        }
    }

    fn max_cost_usd(&self, req: &LlmRequest) -> f64 {
        match &self.inner {
            Inner::OpenAi { .. } => req.nominal_max_cost_usd(),
            // The responder gateway reports no usage, so its calls add 0.0.
            Inner::Ask { .. } => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_cost_from_usage() {
        let v = json!({ "usage": { "prompt_tokens": 1000, "completion_tokens": 500 }});
        let c = estimate_cost(&v);
        // 1000*0.5 + 500*1.5 = 1250 / 1e6
        assert!((c - 0.00125).abs() < 1e-9);
    }

    #[test]
    fn estimate_cost_zero_when_no_usage() {
        assert_eq!(estimate_cost(&json!({})), 0.0);
    }

    #[test]
    fn nominal_max_cost_bounds_the_reported_cost() {
        let req = LlmRequest {
            system: "s".repeat(400),
            user: "u".repeat(600),
        };
        let bound = req.nominal_max_cost_usd();
        // Most tokens the call can report: one per prompt byte plus
        // overhead, and the completion cap.
        let worst = json!({ "usage": {
            "prompt_tokens": 1000 + CHAT_OVERHEAD_TOKENS as u64,
            "completion_tokens": MERGE_MAX_COMPLETION_TOKENS,
        }});
        assert!((bound - estimate_cost(&worst)).abs() < 1e-12);
        assert!(bound > 0.0);
    }

    #[test]
    fn a_reply_cut_at_the_token_cap_is_not_a_merge() {
        let v = json!({ "choices": [{
            "message": { "content": "half a mer" },
            "finish_reason": "length",
        }]});
        let err = parse_chat_completion(&v).unwrap_err();
        assert!(matches!(err, LlmError::Malformed(_)), "{err}");

        let ok = json!({ "choices": [{
            "message": { "content": " merged " },
            "finish_reason": "stop",
        }]});
        assert_eq!(parse_chat_completion(&ok).unwrap().merged_text, "merged");
    }

    #[test]
    fn from_config_none_when_no_llm() {
        let cfg = SleepAgentConfig::from_env(false, Some("http://x".into()));
        // With no LLM env vars set, route resolves to None → no transport.
        // (Test environment may have OPENAI_API_KEY; only assert the None
        // route maps to None.)
        if matches!(cfg.llm, LlmRoute::None) {
            assert!(HttpLlmTransport::from_config(&cfg).is_none());
        }
    }

    #[test]
    fn merge_prompt_includes_all_cluster_entries() {
        use crate::candidates::{Candidate, CandidateSource, MemoryFile};
        let cand = Candidate {
            source: CandidateSource::Churn {
                stem: "notes/x".into(),
                total_versions: 2,
            },
            cluster: vec![
                MemoryFile::stub("notes/x-1", "first fact"),
                MemoryFile::stub("notes/x-2", "second fact"),
            ],
            canonical_path: "notes/x-1".into(),
        };
        let cfg = SleepAgentConfig::from_env(true, Some("http://x".into()));
        let req = LlmRequest::for_merge(&cand, &cfg);
        assert!(req.user.contains("first fact"));
        assert!(req.user.contains("second fact"));
        assert!(req.system.contains("reconcile"));
    }
}
