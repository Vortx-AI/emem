//! HTTP client that drives a running emem responder over its public
//! surface. The sleep-agent is a STANDALONE crate: it never links emem
//! internals, it only speaks the same REST + MCP wire the responder
//! advertises to any agent.
//!
//! Three surfaces are used:
//!
//! - `POST /v1/memory_contradictions` — multi-attester contradiction
//!   scan (REST, returns `ContradictionsResp` directly).
//! - `POST /mcp` (`tools/call`) — the Anthropic memory tools
//!   `memory_view`, `memory_list_by_kind`, `memory_create`. The MCP
//!   `CallToolResult` envelope carries the inner JSON under
//!   `structuredContent`.

use serde_json::{json, Value};

use crate::SleepAgentError;

/// Thin async client over a responder base URL.
#[derive(Clone)]
pub struct ResponderClient {
    base: String,
    http: reqwest::Client,
}

impl ResponderClient {
    /// Construct a client. `base` is e.g. `http://127.0.0.1:5088`.
    pub fn new(base: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(concat!("emem-sleep-agent/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("reqwest client");
        ResponderClient {
            base: base.into().trim_end_matches('/').to_string(),
            http,
        }
    }

    /// For tests / advanced callers that want to inject a pre-built client.
    pub fn with_http(base: impl Into<String>, http: reqwest::Client) -> Self {
        ResponderClient {
            base: base.into().trim_end_matches('/').to_string(),
            http,
        }
    }

    /// Liveness probe — GET `/v1/health` (falls back to `/` for older
    /// responders). Returns Ok(()) when the responder answers 2xx.
    pub async fn ping(&self) -> Result<(), SleepAgentError> {
        for path in ["/v1/health", "/health", "/"] {
            let url = format!("{}{}", self.base, path);
            if let Ok(resp) = self.http.get(&url).send().await {
                if resp.status().is_success() {
                    return Ok(());
                }
            }
        }
        Err(SleepAgentError::Http(format!(
            "responder at {} did not answer a health probe",
            self.base
        )))
    }

    /// `POST /v1/memory_contradictions` — returns the decoded response.
    pub async fn memory_contradictions(
        &self,
        cell_prefix: Option<&str>,
        min_severity: f32,
        limit: usize,
    ) -> Result<Value, SleepAgentError> {
        let mut body = json!({ "min_severity": min_severity, "limit": limit });
        if let Some(p) = cell_prefix {
            body["cell_prefix"] = json!(p);
        }
        let url = format!("{}/v1/memory_contradictions", self.base);
        let resp = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| SleepAgentError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(SleepAgentError::Http(format!(
                "POST /v1/memory_contradictions -> {}",
                resp.status()
            )));
        }
        resp.json::<Value>()
            .await
            .map_err(|e| SleepAgentError::Decode(e.to_string()))
    }

    /// `POST /mcp` `tools/call`. Unwraps the `CallToolResult` envelope and
    /// returns the inner `structuredContent` JSON. A tool-runtime error
    /// (`isError:true`) is surfaced as [`SleepAgentError::Http`].
    pub async fn mcp_call(&self, tool: &str, args: Value) -> Result<Value, SleepAgentError> {
        let url = format!("{}/mcp", self.base);
        let rpc = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": tool, "arguments": args },
        });
        let resp = self
            .http
            .post(&url)
            .json(&rpc)
            .send()
            .await
            .map_err(|e| SleepAgentError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(SleepAgentError::Http(format!(
                "POST /mcp ({tool}) -> {}",
                resp.status()
            )));
        }
        let env: Value = resp
            .json()
            .await
            .map_err(|e| SleepAgentError::Decode(e.to_string()))?;
        if let Some(err) = env.get("error") {
            return Err(SleepAgentError::Http(format!("mcp error: {err}")));
        }
        let result = env
            .get("result")
            .ok_or_else(|| SleepAgentError::Decode("mcp response missing `result`".into()))?;
        if result
            .get("isError")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            let text = result
                .get("content")
                .and_then(|c| c.get(0))
                .and_then(|b| b.get("text"))
                .and_then(|t| t.as_str())
                .unwrap_or("unknown tool error");
            return Err(SleepAgentError::Http(format!("{tool}: {text}")));
        }
        // Prefer structuredContent; fall back to parsing the text block.
        if let Some(sc) = result.get("structuredContent") {
            if !sc.is_null() {
                return Ok(sc.clone());
            }
        }
        let text = result
            .get("content")
            .and_then(|c| c.get(0))
            .and_then(|b| b.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("{}");
        serde_json::from_str(text).map_err(|e| SleepAgentError::Decode(e.to_string()))
    }

    /// List memory files of a given kind, newest-first. Wraps the
    /// `memory_list_by_kind` MCP tool.
    pub async fn list_by_kind(
        &self,
        kind: &str,
        limit: usize,
    ) -> Result<Vec<Value>, SleepAgentError> {
        let out = self
            .mcp_call(
                "emem_memory_list_by_kind",
                json!({ "kind": kind, "limit": limit }),
            )
            .await?;
        Ok(out
            .get("files")
            .and_then(|f| f.as_array())
            .cloned()
            .unwrap_or_default())
    }

    /// Read a single memory file (content + meta). Wraps `memory_view`.
    pub async fn view(&self, path: &str) -> Result<Value, SleepAgentError> {
        self.mcp_call("emem_memory_view", json!({ "path": path }))
            .await
    }

    /// Number of versions a path has held: the `count` of
    /// `memory_view {path, view: "history"}`.
    pub async fn history_len(&self, path: &str) -> Result<usize, SleepAgentError> {
        let out = self
            .mcp_call(
                "emem_memory_view",
                json!({ "path": path, "view": "history" }),
            )
            .await?;
        history_count(&out)
    }

    /// Create / overwrite a memory file. A write to an existing `path`
    /// supersedes the previous version bi-temporally (latest `signed_at`
    /// wins; the prior version stays in `memory_file_history`). Returns
    /// the new `file_cid`.
    pub async fn create(
        &self,
        path: &str,
        file_text: &str,
        kind: &str,
    ) -> Result<Value, SleepAgentError> {
        self.mcp_call(
            "emem_memory_create",
            json!({ "path": path, "file_text": file_text, "kind": kind }),
        )
        .await
    }

    /// `POST /v1/ask` — used only when the operator routes the merge LLM
    /// through the responder's own gateway (`EMEM_SLEEP_AGENT_USE_ASK=1`).
    pub async fn ask(&self, question: &str) -> Result<Value, SleepAgentError> {
        let url = format!("{}/v1/ask", self.base);
        let resp = self
            .http
            .post(&url)
            .json(&json!({ "question": question }))
            .send()
            .await
            .map_err(|e| SleepAgentError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(SleepAgentError::Http(format!(
                "POST /v1/ask -> {}",
                resp.status()
            )));
        }
        resp.json::<Value>()
            .await
            .map_err(|e| SleepAgentError::Decode(e.to_string()))
    }

    /// Base URL accessor (used by the ask-gateway LLM transport).
    pub fn base(&self) -> &str {
        &self.base
    }
}

/// The version count of a `view: "history"` response. A response without
/// `kind: "history"` and a numeric `count` is a decode error, not 1: a
/// default of 1 made every path look never rewritten.
fn history_count(view: &Value) -> Result<usize, SleepAgentError> {
    if view.get("kind").and_then(|k| k.as_str()) != Some("history") {
        return Err(SleepAgentError::Decode(
            "memory_view history: response kind is not `history`".into(),
        ));
    }
    view.get("count")
        .and_then(|c| c.as_u64())
        .map(|c| c as usize)
        .ok_or_else(|| SleepAgentError::Decode("memory_view history: no numeric `count`".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_count_reads_the_count_of_a_history_view() {
        let v = json!({ "kind": "history", "path": "/memories/a", "count": 4, "versions": [] });
        assert_eq!(history_count(&v).unwrap(), 4);
    }

    #[test]
    fn history_count_rejects_a_view_that_is_not_a_history() {
        let file = json!({ "kind": "file", "content": "x" });
        assert!(history_count(&file).is_err());
        let no_count = json!({ "kind": "history" });
        assert!(history_count(&no_count).is_err());
    }
}
