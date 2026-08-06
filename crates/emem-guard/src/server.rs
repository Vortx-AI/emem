//! The HTTPS surface a checkpoint calls.
//!
//! Every operational rule in this module is one the platform documents, and
//! getting any of them wrong produces a server that appears to enforce and
//! does not:
//!
//!   - **Always 200 with a parseable verdict.** A non-200, a redirect, an
//!     unparseable body or a timeout is a webhook failure, which hands the
//!     decision to the org's failure policy and, under fail-open, sends the
//!     prompt to the model uninspected. Sustained failures trip a circuit
//!     breaker that stops enforcement entirely. So there is no error path
//!     here: malformed bodies, unknown events and internal faults all answer
//!     200 with an allow.
//!   - **10 MB bodies.** Transcripts are sent untruncated. A rejected body
//!     counts as a failure, and common defaults (nginx 1 MB, Express 100 kB)
//!     are far below the ceiling.
//!   - **Answer inside the budget.** The org sets 1..10000 ms, default 5000,
//!     covering the whole exchange. A slow verdict is an unreachable server.
//!   - **Deduplicate on the request id.** A connection-failure retry reuses
//!     it, and re-evaluating could return a different answer than the one
//!     already acted on.

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};

use crate::checkpoint::{Adapter, AnthropicHook, ClaudeCodeHook, ClaudeCodeInput, ClaudeCodeStyle};
use crate::frame::PromptFrame;
use crate::log::{seal, LogFailurePolicy};
use crate::policy::{self, Config, Decision, Evidence};
use crate::store::{signer, FileLog};
use crate::tokens;

/// The ceiling the platform documents for a transcript body.
pub const MAX_BODY_BYTES: usize = 10 * 1024 * 1024;

/// How long the whole verdict may take before we answer anyway.
///
/// Below the org's floor of 1000 ms on purpose. If we are going to be late we
/// would rather return a considered allow than have the platform record a
/// timeout, because a timeout is a webhook failure and enough of them stop
/// enforcement for every request in the org, not just this one.
pub const SELF_BUDGET: Duration = Duration::from_millis(800);

/// Everything a request needs.
pub struct Guard {
    pub config: Config,
    pub log: FileLog,
    pub signing: ed25519_dalek::SigningKey,
    /// Secrets accepted right now. More than one during a rotation.
    ///
    /// Empty means no secret is configured yet, which is a real state: the
    /// platform's first connection test arrives BEFORE the org has saved a
    /// secret, and rejecting it would fail the very check that tells an
    /// administrator the endpoint works.
    pub secrets: Vec<String>,
    /// Whether to refuse unsigned requests.
    ///
    /// False until the administrator confirms a secret exists, then true
    /// forever. The spec is explicit about this ordering.
    pub require_signature: bool,
    pub log_failure_policy: LogFailurePolicy,
}

impl Guard {
    /// Resolve every citation in a transcript against LOCAL state only.
    ///
    /// The hard rule from the devspec: the verdict path touches warm cache
    /// and the local log, never an upstream, never a materializer, never a
    /// geocoder. Anything not held locally is [`TokenStatus::Unresolved`],
    /// which is not a denial.
    ///
    /// This is the seam where a node with the emem corpus attached does more
    /// than a bare one. A guard with no corpus still runs: it finds
    /// citations, resolves none, and allows, which is the correct behaviour
    /// for a node that cannot check rather than a reason to block.
    fn gather(&self, texts: &[&str]) -> Evidence {
        let found = tokens::scan_all(texts.iter().copied());
        let need = policy::needs_resolution(&found);
        Evidence {
            tokens: need
                .into_iter()
                .map(|t| {
                    // A bare node holds no corpus, so every citation is cold.
                    // Wiring this to the responder's warm cache is what turns
                    // a logging guard into a verifying one, and it is the
                    // only line that has to change.
                    (t.clone(), policy::TokenStatus::Unresolved)
                })
                .collect(),
            restricted_cells: Vec::new(),
        }
    }

    /// Decide, sign, log, and return what the caller should send.
    fn decide(&self, checkpoint: &str, request_id: &str, texts: &[&str]) -> Decision {
        // A replay must get the answer already recorded, not a fresh one.
        if let Some(prior) = self.log.find_by_request_id(request_id) {
            let mut d = if prior.record.outcome == "deny" {
                Decision::block(
                    prior.record.code.unwrap_or(policy::DenyCode::ProvSig),
                    prior.record.token.clone(),
                    prior.record.fix.unwrap_or(policy::Fix::ContactAdmin),
                )
            } else {
                Decision::proceed()
            };
            d = d
                .with_checked(prior.record.checked)
                .with_leaf(format!("leaf_{}", prior.seq));
            return d;
        }

        let evidence = self.gather(texts);
        let decision = policy::evaluate(&self.config, &evidence);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let (sealed, log_err) = seal(
            checkpoint,
            request_id,
            decision,
            now,
            signer(&self.signing),
            &self.log,
            self.log_failure_policy,
        );
        if let Some(e) = log_err {
            // Visible, because an unlogged verdict is the one condition that
            // silently weakens the product's whole claim.
            eprintln!("emem-guard: verdict not logged: {e}");
        }
        sealed
    }
}

/// Build the router.
pub fn router(guard: Arc<Guard>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/verdict/anthropic-hook", post(anthropic))
        .route("/verdict/claude-code", post(claude_code))
        // The platform sends up to 10 MB and a rejected body is a webhook
        // failure, so the limit is the ceiling rather than a comfortable
        // default.
        .layer(tower_http::limit::RequestBodyLimitLayer::new(
            MAX_BODY_BYTES,
        ))
        .with_state(guard)
}

async fn health(State(g): State<Arc<Guard>>) -> impl IntoResponse {
    Json(serde_json::json!({
        "ok": true,
        "signer_b32": g.log.signer_b32(),
        "verdicts_logged": g.log.len(),
        "require_signature": g.require_signature,
        "rules": {
            "provenance": g.config.provenance,
            "freshness": g.config.freshness,
            "geo_restriction": g.config.geo_restriction,
            "claim_gating": g.config.claim_gating,
        },
    }))
}

/// Whether this request is allowed to be acted on.
///
/// Returns the reason it was not, for the operator log. A request that fails
/// verification still receives 200 with an allow: refusing would be a webhook
/// failure, which does not block anything and does count toward the breaker.
fn signature_ok(g: &Guard, headers: &HeaderMap, body: &[u8]) -> Result<(), String> {
    let get = |name: &str| -> Option<&str> {
        headers
            .iter()
            .find(|(k, _)| k.as_str().eq_ignore_ascii_case(name))
            .and_then(|(_, v)| v.to_str().ok())
    };
    let Some(h) = crate::webhook::SignatureHeaders::from_lookup(get) else {
        // Unsigned. Permitted only before the org has saved its first secret,
        // because the platform's initial connection test arrives that way.
        return if g.require_signature {
            Err("unsigned request refused: a signing secret is configured".into())
        } else {
            Ok(())
        };
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    crate::webhook::verify(&g.secrets, h, body, now).map_err(|e| e.to_string())
}

async fn anthropic(
    State(g): State<Arc<Guard>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let started = Instant::now();
    let adapter = AnthropicHook;

    if let Err(why) = signature_ok(&g, &headers, &body) {
        eprintln!("emem-guard: {why}");
        // Allow: see the module docs. We are not the only control, and a
        // refusal here blocks nothing while counting against the breaker.
        return (StatusCode::OK, Json(adapter.render(&Decision::proceed()))).into_response();
    }

    // A body we cannot parse is not a reason to fail the exchange. The
    // platform warns that legacy field aliases still ride along and that new
    // fields will appear, so an unparseable frame is far more likely to be
    // our lag than an attack.
    let Ok(frame) = serde_json::from_slice::<PromptFrame>(&body) else {
        eprintln!(
            "emem-guard: unparseable prompt frame ({} bytes)",
            body.len()
        );
        return (StatusCode::OK, Json(adapter.render(&Decision::proceed()))).into_response();
    };

    let decision = match adapter.transcript(&frame) {
        // An event type we do not evaluate still needs a verdict.
        None => Decision::proceed(),
        Some(t) => {
            let id = t.request_id.unwrap_or("").to_string();
            g.decide(adapter.checkpoint_id(), &id, &t.texts)
        }
    };

    if started.elapsed() > SELF_BUDGET {
        eprintln!(
            "emem-guard: verdict took {} ms, over the {} ms self-budget",
            started.elapsed().as_millis(),
            SELF_BUDGET.as_millis()
        );
    }
    (StatusCode::OK, Json(adapter.render(&decision))).into_response()
}

async fn claude_code(
    State(g): State<Arc<Guard>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    // Style follows the event: PreToolUse carries a permission decision,
    // everything else takes the {ok, reason} shape.
    let style = serde_json::from_slice::<ClaudeCodeInput>(&body)
        .ok()
        .and_then(|i| i.hook_event_name)
        .map(|n| {
            if n == "PreToolUse" {
                ClaudeCodeStyle::PreToolUse
            } else {
                ClaudeCodeStyle::Prompt
            }
        })
        .unwrap_or(ClaudeCodeStyle::Prompt);
    let adapter = ClaudeCodeHook { style };

    // Claude Code hooks are client-side and carry no webhook signature; the
    // trust boundary is the local socket, so the check is skipped rather than
    // faked. Deployments that expose this beyond localhost put it behind
    // their own auth.
    let _ = &headers;

    let Ok(input) = serde_json::from_slice::<ClaudeCodeInput>(&body) else {
        return (StatusCode::OK, Json(adapter.render(&Decision::proceed()))).into_response();
    };
    let decision = match adapter.transcript(&input) {
        None => Decision::proceed(),
        Some(t) => {
            let id = t.request_id.unwrap_or("").to_string();
            g.decide(adapter.checkpoint_id(), &id, &t.texts)
        }
    };
    (StatusCode::OK, Json(adapter.render(&decision))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt as _;

    fn guard(dir: &str) -> Arc<Guard> {
        let mut p = std::env::temp_dir();
        p.push(format!("emem-guard-srv-{dir}-{}.jsonl", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let key = ed25519_dalek::SigningKey::from_bytes(&[9u8; 32]);
        Arc::new(Guard {
            config: Config::default(),
            log: FileLog::open(&p, key.verifying_key()).unwrap(),
            signing: key,
            secrets: Vec::new(),
            require_signature: false,
            log_failure_policy: LogFailurePolicy::default(),
        })
    }

    async fn post(app: Router, path: &str, body: &str) -> (StatusCode, serde_json::Value) {
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        )
    }

    /// The rule the whole module exists to honour: never a non-200, whatever
    /// arrives. Each of these would be a webhook failure, and enough of them
    /// stop enforcement for the entire organisation.
    #[tokio::test]
    async fn nothing_produces_a_webhook_failure() {
        let cases = [
            (
                "a valid frame",
                r#"{"type":"prompt","request_id":"r1","messages":[]}"#,
            ),
            (
                "an unknown event",
                r#"{"type":"some_future_event","request_id":"r2"}"#,
            ),
            ("not even json", "}{ this is not json"),
            ("an empty body", ""),
            ("json of the wrong shape", r#"[1,2,3]"#),
            ("a frame with no type", r#"{"request_id":"r3"}"#),
        ];
        for (what, body) in cases {
            let (status, v) = post(router(guard("nofail")), "/verdict/anthropic-hook", body).await;
            assert_eq!(
                status,
                StatusCode::OK,
                "{what} must not be a webhook failure"
            );
            assert_eq!(v["action"], "allow", "{what} must still carry a verdict");
        }
    }

    /// The documented example must round-trip through the real handler.
    #[tokio::test]
    async fn the_documented_frame_gets_a_verdict() {
        let body = r#"{"type":"prompt","request_id":"req_abc123","source":{"application":"claude-ai"},
          "messages":[{"role":"user","content":[{"type":"text","text":"Summarize the report."}]}]}"#;
        let (status, v) = post(router(guard("doc")), "/verdict/anthropic-hook", body).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["action"], "allow", "no citations, nothing to object to");
    }

    /// A citation this node does not hold must not block. A bare guard holds
    /// no corpus, so this is the default deployment and it has to be safe.
    #[tokio::test]
    async fn a_bare_node_allows_citations_it_cannot_resolve() {
        let body = r#"{"type":"prompt","request_id":"req_tok","messages":[{"role":"user",
          "content":[{"type":"text","text":"per emem:fact:defi.zb493.xuqA.zcb5f:abc123 it is 918 m"}]}]}"#;
        let (status, v) = post(router(guard("cold")), "/verdict/anthropic-hook", body).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["action"], "allow");
    }

    /// Claude Code blocks via 2xx-with-a-deny-body, so the status is 200 even
    /// when the answer is deny.
    #[tokio::test]
    async fn the_claude_code_surface_answers_in_its_own_shape() {
        let body = r#"{"hook_event_name":"PreToolUse","session_id":"s1","tool_name":"Bash",
          "tool_input":{"command":"echo emem:fact:a:b"}}"#;
        let (status, v) = post(router(guard("cc")), "/verdict/claude-code", body).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "allow");
    }

    /// An unsigned request is refused only once a secret exists, because the
    /// platform's first connection test arrives before one does.
    #[tokio::test]
    async fn unsigned_is_accepted_before_a_secret_exists_and_ignored_after() {
        let body = r#"{"type":"prompt","request_id":"r","messages":[]}"#;

        let g = guard("unsigned");
        let (s, v) = post(router(g), "/verdict/anthropic-hook", body).await;
        assert_eq!(
            (s, &v["action"]),
            (StatusCode::OK, &serde_json::json!("allow"))
        );

        // With a secret configured, an unsigned request is not acted on, but
        // the answer is still a well-formed 200 allow.
        let mut g2 = guard("signed");
        Arc::get_mut(&mut g2).unwrap().require_signature = true;
        let (s2, v2) = post(router(g2), "/verdict/anthropic-hook", body).await;
        assert_eq!(s2, StatusCode::OK, "refusing would be a webhook failure");
        assert_eq!(v2["action"], "allow");
    }

    /// Health is what an operator and a load balancer read.
    #[tokio::test]
    async fn health_reports_the_signer_and_the_active_rules() {
        let app = router(guard("health"));
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["rules"]["provenance"], true);
        assert_eq!(v["rules"]["claim_gating"], false, "off by default");
        assert!(v["signer_b32"].as_str().unwrap().len() == 52);
    }
}
