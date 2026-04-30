//! emem-api-rest — HTTP surface for agents and humans.
//!
//! ```text
//! GET    /                                    → landing (Accept-aware)
//! GET    /agents                              → AGENTS.md (markdown by default)
//! GET    /llms.txt | /llms-full.txt           → LLM-friendly summaries
//! GET    /robots.txt                          → AI bots explicitly allowed
//! GET    /sitemap.xml                         → sitemap
//! GET    /health                              → liveness + identity
//! GET    /.well-known/emem.json               → manifest CIDs, responder pubkey
//! GET    /.well-known/ai-plugin.json          → OpenAI plugin manifest
//! GET    /.well-known/agent.json              → agent-platform metadata
//! GET    /openapi.json                        → OpenAPI 3.1
//! GET    /v1/manifests | /v1/bands | /v1/functions | /v1/sources
//! GET    /v1/errors | /v1/tools | /v1/agent_card | /v1/quickstart
//!
//! GET    /v1/cells/{cell64}                   → recall (no filter)
//! POST   /v1/recall | /v1/query_region | /v1/compare | /v1/find_similar
//! POST   /v1/diff | /v1/trajectory | /v1/verify | /v1/intent
//! POST   /v1/attest | /v1/attest_cbor         → submit signed attestation
//! POST   /v1/verify_receipt                   → offline-verify any responder's receipt
//! GET    /v1/facts/{cid}                      → fact dereference (immutable, ETag-tagged)
//!
//! POST   /mcp                                 → MCP JSON-RPC 2.0
//! ```
//!
//! Agent-friendly defaults:
//!  * CORS open for any origin
//!  * Content negotiation: `Accept: text/markdown` returns markdown for / and /agents
//!  * ETag + immutable on /v1/facts/{cid} (CIDs never change)
//!  * `traceparent` (W3C Trace Context) propagated into receipts via the
//!    `request_id` field when a caller provides one
//!  * Compression delegated to fronting proxy (Caddy / Cloudflare) — server
//!    emits identity bytes so byte-exact merkle agreement is preserved.

#![forbid(unsafe_code)]
#![recursion_limit = "256"]

use std::sync::{Arc, LazyLock};

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::header::{ACCEPT, CACHE_CONTROL, CONTENT_TYPE, ETAG, IF_NONE_MATCH};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as JsonValue};

use ed25519_dalek::Signer;
use emem_core::{manifest::manifest_cid, ErrorCode};
use emem_core::{KeyEpoch, Signature as EmCoreSignature};
use emem_fact::{
    Attestation, Derivation, Fact, NegativeFact, PrimaryFact, ReasonCid, RegistryCid, SchemaCid,
    Source,
};
use emem_intent::{plan, Intent};
use emem_primitives::{
    compare, compare_bands, diff, find_similar, query_region, recall, trajectory, verify,
    CompareBandsReq, CompareReq, DiffReq, FindSimilarReq, QueryRegionReq, RecallReq, RecallResp,
    TrajectoryReq, VerifyReq,
};
use emem_storage::{Server, StorageError};

/// Shared application state.
pub type AppState = Arc<Server>;

const LLMS_TXT: &str = include_str!("../../../web/llms.txt");
const LLMS_FULL_TXT: &str = include_str!("../../../web/llms-full.txt");
const AGENT_WALKTHROUGHS_MD: &str = include_str!("../../../examples/agent-walkthroughs.md");
const AGENT_TRIAL_MD: &str = include_str!("../../../docs/AGENT_TRIAL.md");
const ATTESTING_MD: &str = include_str!("../../../docs/ATTESTING.md");
const GLOBAL_TRIAL_MD: &str = include_str!("../../../docs/GLOBAL_TRIAL.md");
const MATERIALIZERS_MD: &str = include_str!("../../../docs/MATERIALIZERS.md");
const SPACES_MD: &str = include_str!("../../../docs/SPACES.md");
const TEMPORAL_MD: &str = include_str!("../../../docs/TEMPORAL.md");
const ROBOTS_TXT: &str = include_str!("../../../web/robots.txt");
const INDEX_HTML: &str = include_str!("../../../web/index.html");
const AI_PLUGIN_JSON: &str = include_str!("../../../web/ai-plugin.json");
const AGENT_JSON: &str = include_str!("../../../web/agent.json");
const AGENTS_MD: &str = include_str!("../../../docs/AGENTS.md");
const WHITEPAPER_MD: &str = include_str!("../../../docs/WHITEPAPER.md");
const SPEC_MD: &str = include_str!("../../../docs/SPEC.md");
const PRIVACY_MD: &str = include_str!("../../../PRIVACY.md");
const TERMS_MD: &str = include_str!("../../../TERMS.md");
const SUPPORT_MD: &str = include_str!("../../../SUPPORT.md");
const SITEMAP_XML: &str = include_str!("../../../web/sitemap.xml");
const FAVICON_SVG: &str = include_str!("../../../web/favicon.svg");
const OG_IMAGE_SVG: &str = include_str!("../../../web/og-image.svg");
const INDEXNOW_KEY: &str = include_str!("../../../web/indexnow.txt");

const EXAMPLE_CLAUDE_DESKTOP: &str = include_str!("../../../examples/claude-desktop.json");
const EXAMPLE_CLAUDE_CODE: &str = include_str!("../../../examples/claude-code.mcp.json");
const EXAMPLE_CURSOR: &str = include_str!("../../../examples/cursor.mcp.json");
const EXAMPLE_CLINE: &str = include_str!("../../../examples/cline.mcp.json");
const EXAMPLE_OPENAI: &str = include_str!("../../../examples/openai-gpt-action.json");
const EXAMPLE_LANGCHAIN: &str = include_str!("../../../examples/langchain.py");
const EXAMPLE_LLAMAINDEX: &str = include_str!("../../../examples/llamaindex.py");

/// Build the full HTTP router.
pub fn router(state: AppState) -> Router {
    // Force the metrics start-instant to align with router construction so
    // /metrics emem_uptime_seconds reflects real wall-clock uptime.
    let _ = START_INSTANT.set(std::time::Instant::now());

    // Boot sled-backed persistence for the agent-stats counters. Loads
    // the last snapshot synchronously, then spawns a periodic flush task.
    // The hot cache always holds a sled handle in production; if it's
    // missing (in-memory test backend), persistence is silently disabled.
    if let Some(db) = state.storage.hot_sled_db() {
        let db_arc = Arc::new(db.clone());
        agent_stats_init_persistence(db_arc);
    }
    Router::new()
        // Landing & agent-targeted pages
        .route("/", get(landing))
        .route("/agents", get(agents_page))
        .route("/agents.md", get(serve_agents_md))
        .route("/whitepaper", get(serve_whitepaper_md))
        .route("/whitepaper.md", get(serve_whitepaper_md))
        .route("/spec", get(serve_spec_md))
        .route("/spec.md", get(serve_spec_md))
        .route("/llms.txt", get(serve_llms_txt))
        .route("/llms-full.txt", get(serve_llms_full))
        .route("/robots.txt", get(serve_robots))
        .route("/sitemap.xml", get(serve_sitemap))
        .route("/favicon.svg", get(serve_favicon))
        .route("/favicon.ico", get(serve_favicon))
        .route("/og-image.svg", get(serve_og_image))
        .route(
            "/484b153b1031a5a89d8217c1efbe6fe91313e0b328e94b0f10446c6dbda8b10e.txt",
            get(serve_indexnow_key),
        )
        .route("/.well-known/security.txt", get(serve_security_txt))
        // Well-known
        .route("/health", get(health))
        .route("/.well-known/emem.json", get(well_known))
        .route("/.well-known/ai-plugin.json", get(ai_plugin))
        .route("/.well-known/agent.json", get(agent_manifest))
        .route("/agent.json", get(agent_manifest))
        .route("/openapi.json", get(openapi))
        // Examples
        .route(
            "/examples/claude-desktop.json",
            get(serve_example_claude_desktop),
        )
        .route(
            "/examples/claude-code.mcp.json",
            get(serve_example_claude_code),
        )
        .route("/examples/cursor.mcp.json", get(serve_example_cursor))
        .route("/examples/cline.mcp.json", get(serve_example_cline))
        .route(
            "/examples/openai-gpt-action.json",
            get(serve_example_openai),
        )
        .route("/examples/langchain.py", get(serve_example_langchain))
        .route("/examples/llamaindex.py", get(serve_example_llamaindex))
        .route(
            "/examples/agent-walkthroughs.md",
            get(serve_agent_walkthroughs),
        )
        .route("/agent-trial.md", get(serve_agent_trial))
        .route("/attesting.md", get(serve_attesting))
        .route("/docs/ATTESTING.md", get(serve_attesting))
        // Policy docs published under stable URLs so server.json's
        // privacyPolicyUrl / termsOfServiceUrl / supportUrl resolve.
        .route("/privacy", get(serve_privacy_md))
        .route("/privacy.md", get(serve_privacy_md))
        .route("/docs/PRIVACY.md", get(serve_privacy_md))
        .route("/terms", get(serve_terms_md))
        .route("/terms.md", get(serve_terms_md))
        .route("/docs/TERMS.md", get(serve_terms_md))
        .route("/support", get(serve_support_md))
        .route("/support.md", get(serve_support_md))
        .route("/docs/SUPPORT.md", get(serve_support_md))
        .route("/global-trial.md", get(serve_global_trial))
        .route("/docs/GLOBAL_TRIAL.md", get(serve_global_trial))
        .route("/materializers.md", get(serve_materializers_md))
        .route("/docs/MATERIALIZERS.md", get(serve_materializers_md))
        .route("/spaces.md", get(serve_spaces_md))
        .route("/docs/SPACES.md", get(serve_spaces_md))
        .route("/temporal.md", get(serve_temporal_md))
        .route("/docs/TEMPORAL.md", get(serve_temporal_md))
        .route("/api", get(api_alias))
        .route("/v1/discover", get(discover))
        .route("/v1/grid_info", get(grid_info))
        .route("/v1/elevation", post(post_elevation))
        .route("/v1/coverage_map.svg", get(coverage_map_svg))
        .route("/v1/coverage", get(coverage_json))
        .route("/v1/recall_many", post(post_recall_many))
        .route("/v1/recall_polygon", post(post_recall_polygon))
        // Introspection
        .route("/v1/manifests", get(manifests))
        .route("/v1/bands", get(bands))
        .route("/v1/materializers", get(materializers))
        .route("/v1/data_availability", get(data_availability))
        .route("/v1/fleet", get(fleet))
        .route("/v1/coverage_matrix", get(coverage_matrix))
        .route("/v1/functions", get(functions))
        .route("/v1/sources", get(sources))
        .route("/v1/algorithms", get(algorithms))
        .route("/v1/algorithms/:key", get(algorithm_detail))
        .route("/v1/errors", get(errors))
        .route("/v1/tools", get(tools))
        .route("/v1/agent_card", get(agent_card))
        .route("/v1/quickstart", get(quickstart))
        // Read primitives
        .route("/v1/cells/:cell64", get(get_cell))
        .route("/v1/cells/:cell64/info", get(get_cell_info))
        // Note: matchit (axum 0.7) doesn't accept `:cell64.geojson` next to
        // `:cell64/info`. We expose the GeoJSON variant under a plain
        // sub-path. The handler still strips a `.geojson` suffix for
        // backward-compatible URLs that include it.
        .route("/v1/cells/:cell64/geojson", get(get_cell_geojson))
        .route(
            "/v1/cells/:cell64/recall_geojson",
            get(get_cell_recall_geojson),
        )
        .route("/v1/cells/:cell64/scene.png", get(get_cell_scene_png))
        .route("/v1/locate", post(post_locate))
        .route("/v1/locate", get(get_locate))
        .route("/v1/ask", post(post_ask))
        .route("/v1/recall", post(post_recall))
        .route("/v1/query_region", post(post_query_region))
        .route("/v1/compare", post(post_compare))
        .route("/v1/compare_bands", post(post_compare_bands))
        .route("/v1/find_similar", post(post_find_similar))
        .route("/v1/diff", post(post_diff))
        .route("/v1/trajectory", post(post_trajectory))
        .route("/v1/backfill", post(post_backfill))
        .route("/v1/schema", get(get_schema))
        .route("/v1/verify", post(post_verify))
        .route("/v1/intent", post(post_intent))
        .route("/v1/attest", post(post_attest))
        .route("/v1/attest_cbor", post(post_attest_cbor))
        .route("/v1/verify_receipt", post(post_verify_receipt))
        .route("/v1/facts/:cid", get(get_fact))
        .route("/v1/demos", get(list_demos))
        .route("/v1/demos/:run", get(get_demo_index))
        .route("/v1/demos/:run/:file", get(get_demo_file))
        .route("/v1/contributors", get(list_contributors))
        .route("/v1/contributors/:pubkey_b32", get(get_contributor))
        .route("/v1/agent_stats", get(agent_stats_endpoint))
        .route("/v1/reviews", post(post_review).get(list_reviews))
        .route("/v1/reviews/:subject_id", get(reviews_for_subject))
        .route(
            "/v1/temporal_route",
            post(post_temporal_route).get(get_temporal_route),
        )
        .route("/metrics", get(metrics))
        .route("/mcp", get(mcp_discover).post(mcp_jsonrpc))
        // Order: outermost wraps innermost. Trace first so spans see everything.
        .layer(axum::middleware::from_fn(security_headers_layer))
        .layer(axum::middleware::from_fn(rate_limit_layer))
        .layer(axum::middleware::from_fn(cors_layer))
        .layer(axum::middleware::from_fn(cache_hint_layer))
        .layer(axum::middleware::from_fn(agent_access_log_layer))
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .layer(tower_http::timeout::TimeoutLayer::with_status_code(
            StatusCode::GATEWAY_TIMEOUT,
            std::time::Duration::from_secs(timeout_seconds()),
        ))
        .layer(tower_http::limit::RequestBodyLimitLayer::new(
            body_limit_bytes(),
        ))
        // gzip the response body when the client signals support. JSON
        // payloads compress ~10× — agents under rate limit pay 10× less
        // bandwidth without giving up cite-ability (the canonical bytes
        // for fact CIDs are decoded from the compressed wire by the
        // client before any verification step).
        .layer(tower_http::compression::CompressionLayer::new().gzip(true))
        .with_state(state)
}

/// Mean meridional metres per degree of latitude (WGS-84 mean radius
/// 6 371 008.8 m × π / 180). Used by `/v1/cells/{cell64}/info` to render
/// the cell's bbox in metres for agents that prefer SI over degrees.
/// Off by ≈0.5 % vs the proper geodetic distance — acceptable for the
/// "what scale is this cell" UX hint, not for survey-grade calculations.
const METERS_PER_DEGREE_LAT: f64 = 111_320.0;

/// Hard cap on POST bodies. Defaults to 16 MiB; tunable via
/// `EMEM_BODY_LIMIT_MB` (clamped to 1..=256 MiB).
fn body_limit_bytes() -> usize {
    let mb: usize = std::env::var("EMEM_BODY_LIMIT_MB")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(16);
    mb.clamp(1, 256) * 1024 * 1024
}

/// HTTP request gateway timeout. Defaults to 30s; tunable via
/// `EMEM_TIMEOUT_SECS` (clamped to 1..=600).
fn timeout_seconds() -> u64 {
    std::env::var("EMEM_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30u64)
        .clamp(1, 600)
}

/// Per-upstream materializer fetch timeout. Defaults to 15s; tunable via
/// `EMEM_MATERIALIZER_TIMEOUT_SECS` (clamped to 2..=120). Bounding the
/// upstream call here is what stops a slow MODIS / met.no / STAC peer
/// from dragging the recall request past the gateway timeout — the
/// previous behavior was an unbounded `reqwest::send().await`, which
/// surfaced to agents as a generic 504 with no per-band attribution.
fn materializer_timeout_secs() -> u64 {
    std::env::var("EMEM_MATERIALIZER_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(15u64)
        .clamp(2, 120)
}

/// Number of HTTP attempts a materializer makes before giving up.
/// Defaults to 2; tunable via `EMEM_MATERIALIZER_RETRIES` (clamped 1..=5).
fn materializer_retries() -> u32 {
    std::env::var("EMEM_MATERIALIZER_RETRIES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2u32)
        .clamp(1, 5)
}

// ── CORS layer (open for agents, Origin-allowlist when configured) ─────
//
// Default behavior is `Access-Control-Allow-Origin: *` so unauthenticated
// agents (Claude, Cursor, Cline, …) can call the API from any origin
// without preflight friction. Set `EMEM_ALLOWED_ORIGINS` to a comma-
// separated list of origins (e.g. `https://claude.ai,https://claude.com`)
// to switch to strict allowlist mode — when a request's Origin matches an
// allowlisted entry the server echoes it back with `Vary: Origin`; when
// it doesn't match (or no Origin header is present) no CORS headers are
// emitted, which the browser interprets as same-origin-only.
//
// Anthropic's connector review criteria require Origin validation for
// listed integrations; flipping the env var is the supported toggle.

fn allowed_origins() -> Vec<String> {
    std::env::var("EMEM_ALLOWED_ORIGINS")
        .ok()
        .map(|raw| {
            raw.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

async fn cors_layer(
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> Response {
    let is_preflight = req.method() == Method::OPTIONS;
    let origin_header = req
        .headers()
        .get("origin")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let mut response = if is_preflight {
        Response::builder()
            .status(StatusCode::NO_CONTENT)
            .body(axum::body::Body::empty())
            .unwrap_or_else(|_| StatusCode::NO_CONTENT.into_response())
    } else {
        next.run(req).await
    };
    let h = response.headers_mut();
    let allow = allowed_origins();
    if allow.is_empty() {
        h.insert("access-control-allow-origin", HeaderValue::from_static("*"));
    } else if let Some(origin) = origin_header {
        if allow.iter().any(|o| o.eq_ignore_ascii_case(&origin)) {
            if let Ok(v) = HeaderValue::from_str(&origin) {
                h.insert("access-control-allow-origin", v);
                h.insert("vary", HeaderValue::from_static("Origin"));
            }
        }
        // Origin present but not allowlisted → no allow-origin header,
        // browser blocks the response. Headless agents (no Origin
        // header) keep working.
    }
    h.insert(
        "access-control-allow-methods",
        HeaderValue::from_static("GET, POST, OPTIONS"),
    );
    h.insert(
        "access-control-allow-headers",
        // Includes the MCP Streamable-HTTP negotiation / session / SSE-resume
        // headers (mcp-protocol-version, mcp-session-id, last-event-id) so
        // browser-based MCP clients (Claude.ai, Inspector) can preflight the
        // /mcp endpoint without being blocked.
        HeaderValue::from_static(
            "content-type, authorization, traceparent, accept, if-none-match, \
             mcp-protocol-version, mcp-session-id, last-event-id",
        ),
    );
    h.insert(
        "access-control-expose-headers",
        // Expose the MCP session header so JS clients can read it after
        // initialize and echo it on subsequent requests.
        HeaderValue::from_static(
            "etag, x-emem-receipt-cid, traceparent, mcp-session-id, mcp-protocol-version",
        ),
    );
    h.insert("access-control-max-age", HeaderValue::from_static("86400"));
    response
}

// ── Cache-Control hint layer ─────────────────────────────────────────────
//
// Static-shape introspection endpoints change only on manifest rotation
// or version bump. Without explicit Cache-Control, every fresh agent
// re-bootstraps from emem.dev — a thousand Claude instances starting up
// is a thousand /v1/discover round trips that could have been one.
// This layer injects per-path Cache-Control on success only; per-path
// values reflect how often the response actually changes.

fn cache_ttl_for_path(path: &str) -> Option<&'static str> {
    match path {
        // Stable across deploys (build-pinned constants).
        "/v1/grid_info"
        | "/v1/agent_card"
        | "/v1/tools"
        | "/v1/bands"
        | "/v1/materializers"
        | "/v1/data_availability"
        | "/v1/functions"
        | "/v1/sources"
        | "/v1/manifests"
        | "/v1/errors"
        | "/v1/quickstart"
        | "/agents.md"
        | "/whitepaper.md"
        | "/spec.md"
        | "/llms.txt"
        | "/llms-full.txt"
        | "/agent-trial.md"
        | "/attesting.md"
        | "/privacy"
        | "/privacy.md"
        | "/docs/PRIVACY.md"
        | "/terms"
        | "/terms.md"
        | "/docs/TERMS.md"
        | "/support"
        | "/support.md"
        | "/docs/SUPPORT.md"
        | "/global-trial.md"
        | "/materializers.md"
        | "/spaces.md"
        | "/temporal.md"
        | "/openapi.json" => Some("public, max-age=86400, stale-while-revalidate=604800"),
        // Changes on manifest rotation (hours), not seconds.
        "/.well-known/emem.json"
        | "/.well-known/ai-plugin.json"
        | "/.well-known/agent.json"
        | "/v1/discover" => Some("public, max-age=3600, stale-while-revalidate=86400"),
        // Active operational data — bounded staleness OK.
        "/v1/contributors"
        | "/v1/coverage"
        | "/v1/coverage_map.svg"
        | "/v1/agent_stats"
        | "/v1/reviews" => Some("public, max-age=300, stale-while-revalidate=900"),
        // Static assets.
        "/favicon.svg" | "/favicon.ico" | "/og-image.svg" | "/robots.txt" | "/sitemap.xml" => {
            Some("public, max-age=86400")
        }
        _ => None,
    }
}

async fn cache_hint_layer(
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> Response {
    let path = req.uri().path().to_string();
    let method = req.method().clone();
    let mut resp = next.run(req).await;
    // GET and HEAD share routes in axum; both should advertise the same
    // cache hint so a HEAD probe sees what GET would deliver.
    if (method == Method::GET || method == Method::HEAD) && resp.status().is_success() {
        if let Some(ttl) = cache_ttl_for_path(&path) {
            // Don't clobber a more specific Cache-Control set by a handler
            // (e.g. /v1/recall sets `private, max-age=60` per-request).
            if !resp.headers().contains_key(CACHE_CONTROL) {
                if let Ok(v) = HeaderValue::from_str(ttl) {
                    resp.headers_mut().insert(CACHE_CONTROL, v);
                }
            }
        }
    }
    resp
}

// ── Agent access log ─────────────────────────────────────────────────────
//
// Every request is emitted as a structured `tracing` event with the fields
// an operator wants to see: how the agent identified itself, which MCP
// tool it called (if any), the latency, the status, and the bytes a
// downstream verifier would need (responder pubkey, traceparent, IP hash).
// Bodies are *not* logged — only metadata. This is the single source of
// truth for "how are agents approaching us?" without leaking memory.
//
// The User-Agent string is *not* PII for headless agents (it identifies
// software, not a person). For browsers we could add a privacy filter
// later; today the surface is API-only.

/// Family classifier applied to the User-Agent header. Lets us answer
/// "what fraction of traffic is Claude vs Cursor vs unknown agents?"
/// without doing string matching downstream.
fn classify_agent(ua: &str) -> &'static str {
    let ua_lc = ua.to_ascii_lowercase();
    // Order matters: more specific tokens first.
    if ua_lc.contains("claude-code") {
        "claude-code"
    } else if ua_lc.contains("claude") {
        "claude"
    } else if ua_lc.contains("cursor") {
        "cursor"
    } else if ua_lc.contains("cline") {
        "cline"
    } else if ua_lc.contains("openai") || ua_lc.contains("gpt") {
        "openai"
    } else if ua_lc.contains("perplexity") {
        "perplexity"
    } else if ua_lc.contains("anthropic") {
        "anthropic"
    } else if ua_lc.contains("langchain") {
        "langchain"
    } else if ua_lc.contains("llamaindex") {
        "llamaindex"
    } else if ua_lc.contains("python-requests")
        || ua_lc.contains("aiohttp")
        || ua_lc.contains("httpx")
    {
        "python"
    } else if ua_lc.contains("curl") || ua_lc.contains("wget") {
        "cli"
    } else if ua_lc.contains("mozilla") {
        "browser"
    } else if ua_lc.is_empty() {
        "anonymous"
    } else {
        "other"
    }
}

/// Hash the client IP so logs identify retries from the same caller without
/// storing the address. blake3 truncated to 8 bytes — enough for cross-line
/// correlation, not enough to reverse.
fn hashed_ip(headers: &HeaderMap) -> String {
    let raw = client_ip(headers).unwrap_or_default();
    if raw.is_empty() {
        return "anon".into();
    }
    let h = blake3::hash(raw.as_bytes());
    data_encoding::BASE32_NOPAD
        .encode(&h.as_bytes()[..8])
        .to_lowercase()
}

async fn agent_access_log_layer(
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> Response {
    let started = std::time::Instant::now();
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let query = req.uri().query().map(|q| q.to_string()).unwrap_or_default();
    let headers = req.headers().clone();
    let ua = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let agent_family = classify_agent(&ua);
    let traceparent = headers
        .get("traceparent")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let accept = headers
        .get("accept")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let ip_h = hashed_ip(&headers);

    let resp = next.run(req).await;

    let dur_ms = started.elapsed().as_secs_f64() * 1000.0;
    let status = resp.status().as_u16();
    let receipt_cid = resp
        .headers()
        .get("x-emem-receipt-cid")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    record_request(agent_family, status, dur_ms);

    // One line per request. tracing's `info!` lands in journald via systemd's
    // RUST_LOG=info default; operators can pipe to ClickHouse / Loki by
    // adding a tracing-subscriber JSON formatter at startup.
    tracing::info!(
        target: "emem::access",
        http_method = %method,
        http_path = %path,
        http_query = %query,
        http_status = status,
        http_duration_ms = dur_ms,
        agent_family = agent_family,
        agent_user_agent = %ua,
        agent_accept = %accept,
        agent_traceparent = %traceparent,
        agent_ip_hash = %ip_h,
        emem_receipt_cid = %receipt_cid,
        "access"
    );
    resp
}

// ── Security headers (always-on) ─────────────────────────────────────────

async fn security_headers_layer(
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> Response {
    // Optional HTTP → HTTPS redirect. When EMEM_REDIRECT_HTTPS=1 is set on
    // the responder process, any plain HTTP request that names a known TLS
    // host is 301-redirected to https. This keeps the alongside-5051 plain
    // listener for local agents, while pushing public traffic to TLS.
    if std::env::var("EMEM_REDIRECT_HTTPS").ok().as_deref() == Some("1") {
        let proto_https = req
            .headers()
            .get("x-forwarded-proto")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.eq_ignore_ascii_case("https"))
            .unwrap_or(false)
            || req.uri().scheme_str() == Some("https");
        if !proto_https {
            if let Some(host) = req.headers().get("host").and_then(|v| v.to_str().ok()) {
                let lower = host.to_ascii_lowercase();
                let host_only: String = lower.split(':').next().unwrap_or(&lower).to_string();
                let tls_hosts: Vec<String> = std::env::var("EMEM_TLS_DOMAINS")
                    .ok()
                    .map(|s| {
                        s.split(',')
                            .map(|x| x.trim().to_ascii_lowercase())
                            .collect()
                    })
                    .unwrap_or_default();
                if tls_hosts.iter().any(|h| h == &host_only) {
                    let path = req
                        .uri()
                        .path_and_query()
                        .map(|p| p.as_str().to_string())
                        .unwrap_or_else(|| "/".into());
                    let location = format!("https://{host_only}{path}");
                    return Response::builder()
                        .status(StatusCode::PERMANENT_REDIRECT)
                        .header("location", location)
                        .header(
                            "strict-transport-security",
                            "max-age=31536000; includeSubDomains; preload",
                        )
                        .body(axum::body::Body::empty())
                        .unwrap_or_else(|_| StatusCode::PERMANENT_REDIRECT.into_response());
                }
            }
        }
    }
    let mut response = next.run(req).await;
    let h = response.headers_mut();
    // HSTS: opt browsers into HTTPS for 1y, including subdomains. Safe to send
    // over plain HTTP too — browsers ignore it on http://.
    h.insert(
        "strict-transport-security",
        HeaderValue::from_static("max-age=31536000; includeSubDomains; preload"),
    );
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        "referrer-policy",
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    h.insert(
        "permissions-policy",
        HeaderValue::from_static("geolocation=(), microphone=(), camera=()"),
    );
    // Allow huggingface.co and *.hf.space to embed the landing page so the
    // HuggingFace Space iframe preview renders. Modern browsers ignore the
    // legacy X-Frame-Options when CSP frame-ancestors is set, so we drop it.
    h.insert(
        "content-security-policy",
        HeaderValue::from_static(
            "default-src 'self'; \
         script-src 'self' https://www.googletagmanager.com 'unsafe-inline'; \
         connect-src 'self' https://www.google-analytics.com; \
         img-src 'self' data: https:; \
         style-src 'self' 'unsafe-inline'; \
         frame-ancestors 'self' https://huggingface.co https://*.hf.space; \
         base-uri 'self'; \
         form-action 'self'",
        ),
    );
    h.insert(
        "x-emem-version",
        HeaderValue::from_static(env!("CARGO_PKG_VERSION")),
    );
    response
}

// ── In-process token-bucket rate limit ───────────────────────────────────
//
// 60 req/min per remote IP, 120 burst. Enough headroom for a busy agent
// but small enough that a misbehaving client can't drown the server. State
// is held in a single Mutex (cheap; entries are 24 bytes); GC-free since
// we only mutate on hit.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Default per-IP token-bucket refill rate, in tokens per second.
/// 1 tok/s ≈ 60 req/min sustained, with `RATE_LIMIT_BURST` as the ceiling.
/// Tunable via `EMEM_RATE_LIMIT_RPS` (clamped to 0.01..=1000.0).
fn rate_limit_rps() -> f64 {
    std::env::var("EMEM_RATE_LIMIT_RPS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1.0_f64)
        .clamp(0.01, 1000.0)
}

/// Default per-IP burst capacity (max tokens in the bucket).
/// Tunable via `EMEM_RATE_LIMIT_BURST` (clamped to 1.0..=100_000.0).
fn rate_limit_burst() -> f64 {
    std::env::var("EMEM_RATE_LIMIT_BURST")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(120.0_f64)
        .clamp(1.0, 100_000.0)
}

const RATE_LIMIT_GC_AFTER: Duration = Duration::from_secs(600);

#[derive(Clone, Copy)]
struct Bucket {
    tokens: f64,
    last: Instant,
}

static BUCKETS: std::sync::OnceLock<Mutex<std::collections::HashMap<String, Bucket>>> =
    std::sync::OnceLock::new();

fn buckets() -> &'static Mutex<std::collections::HashMap<String, Bucket>> {
    BUCKETS.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

async fn rate_limit_layer(
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> Response {
    metrics_inc(&REQ_TOTAL);
    // Skip read-only health & discovery — they need to stay always-up for
    // monitoring and for agent bootstrap.
    let path = req.uri().path();
    let bypass = matches!(
        path,
        "/health"
            | "/metrics"
            | "/.well-known/emem.json"
            | "/openapi.json"
            | "/robots.txt"
            | "/sitemap.xml"
            | "/favicon.svg"
            | "/favicon.ico"
    );
    if bypass {
        return next.run(req).await;
    }

    let ip = client_ip(req.headers()).unwrap_or_else(|| "unknown".to_string());
    let now = Instant::now();
    let rps = rate_limit_rps();
    let burst = rate_limit_burst();
    let allow = {
        let mut map = match buckets().lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        // Periodic GC: drop entries older than 10 min.
        if map.len() > 1024 {
            map.retain(|_, b| now.duration_since(b.last) < RATE_LIMIT_GC_AFTER);
        }
        let bucket = map.entry(ip.clone()).or_insert(Bucket {
            tokens: burst,
            last: now,
        });
        let elapsed = now.duration_since(bucket.last).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * rps).min(burst);
        bucket.last = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    };
    if allow {
        next.run(req).await
    } else {
        metrics_inc(&RATE_LIMITED_TOTAL);
        let body = serde_json::json!({
            "code": "rate_limited",
            "message": format!("rate limit: {} req/min, burst {}; backoff and retry", (rps * 60.0) as u64, burst as u64),
        });
        let mut resp = (StatusCode::TOO_MANY_REQUESTS, Json(body)).into_response();
        resp.headers_mut()
            .insert("retry-after", HeaderValue::from_static("60"));
        resp
    }
}

fn client_ip(h: &HeaderMap) -> Option<String> {
    // Trust X-Forwarded-For only when running behind our own reverse proxy.
    // EMEM_TRUST_FORWARDED=1 enables it; default is off so we don't get spoofed.
    if std::env::var("EMEM_TRUST_FORWARDED").ok().as_deref() == Some("1") {
        if let Some(v) = h.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
            if let Some(first) = v.split(',').next() {
                return Some(first.trim().to_string());
            }
        }
        if let Some(v) = h.get("x-real-ip").and_then(|v| v.to_str().ok()) {
            return Some(v.to_string());
        }
    }
    None
}

// ── Wire-error envelope ──────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct ErrorBody {
    code: ErrorCode,
    message: String,
}

pub(crate) struct ApiError(StatusCode, ErrorBody);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(self.1)).into_response()
    }
}

impl From<StorageError> for ApiError {
    fn from(e: StorageError) -> Self {
        let code = e.wire_code();
        let status = match code {
            ErrorCode::CidNotFound
            | ErrorCode::BandNotInRegistry
            | ErrorCode::FunctionNotInRegistry
            | ErrorCode::SchemaCidUnknown
            | ErrorCode::RegistryCidUnknown => StatusCode::NOT_FOUND,
            ErrorCode::BadSignature | ErrorCode::BadMerkleProof => StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::Unauthorized | ErrorCode::AttesterRevoked => StatusCode::UNAUTHORIZED,
            ErrorCode::PrivacyRefused | ErrorCode::LevelTooLow => StatusCode::FORBIDDEN,
            ErrorCode::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            ErrorCode::ComputeTimeout => StatusCode::GATEWAY_TIMEOUT,
            ErrorCode::ComputeQuotaExceeded => StatusCode::PAYMENT_REQUIRED,
            ErrorCode::SourceFetchFailed | ErrorCode::SourceFormatMismatch => {
                StatusCode::BAD_GATEWAY
            }
            ErrorCode::InvalidCell
            | ErrorCode::InvalidResolution
            | ErrorCode::TslotMismatch
            | ErrorCode::SourceSchemeUnknown
            | ErrorCode::ClaimUndecidable => StatusCode::BAD_REQUEST,
            ErrorCode::CanonicalEncodingDivergence
            | ErrorCode::CacheError
            | ErrorCode::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        };
        ApiError(
            status,
            ErrorBody {
                code,
                message: e.to_string(),
            },
        )
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

fn prefer_markdown(headers: &HeaderMap) -> bool {
    headers
        .get(ACCEPT)
        .and_then(|v| v.to_str().ok())
        .map(|a| {
            let a = a.to_lowercase();
            a.contains("text/markdown") || a.contains("text/plain") || a.contains("text/x-markdown")
        })
        .unwrap_or(false)
}

fn text_response(content_type: &'static str, body: &'static str) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, content_type)
        .body(axum::body::Body::from(body))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn html_or_md(headers: &HeaderMap, html: &'static str, md: &'static str) -> Response {
    if prefer_markdown(headers) {
        text_response("text/markdown; charset=utf-8", md)
    } else {
        text_response("text/html; charset=utf-8", html)
    }
}

// ── Static page routes ───────────────────────────────────────────────────

async fn landing(headers: HeaderMap) -> Response {
    // Content-negotiate three-way: JSON-asking agents get a pointer to
    // /v1/discover (the bootstrap), markdown-asking agents get llms.txt
    // (token-cheap summary), and HTML-asking browsers get the homepage.
    // This is the *first* request a fresh agent makes; getting it wrong
    // means an agent-only client never finds the protocol surface.
    let accept = headers
        .get(ACCEPT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let prefers_json = accept.contains("application/json") && !accept.contains("text/html");
    if prefers_json {
        let body = serde_json::to_vec(&serde_json::json!({
            "schema": "emem.landing.v1",
            "tagline": "Cite-able, content-addressed, signed memory of every place on Earth.",
            "bootstrap": "/v1/discover",
            "agent_card": "/v1/agent_card",
            "openapi": "/openapi.json",
            "mcp": "/mcp",
            "llms_txt": "/llms.txt",
            "agents_md": "/agents.md",
            "next": [
                "GET /v1/discover  — one-call bootstrap with manifests, tools, canonical places",
                "GET /v1/quickstart — three-call flow: discover → recall → verify_receipt",
                "GET /v1/grid_info — actual cell64 resolution + DGGS interop",
                "GET /v1/coverage  — what places have data attested today",
                "GET /v1/coverage_map.svg — visual gym (multimodal agents)"
            ]
        }))
        .unwrap_or_default();
        return Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "application/json; charset=utf-8")
            .body(axum::body::Body::from(body))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }
    html_or_md(&headers, INDEX_HTML, LLMS_TXT)
}

async fn agents_page(headers: HeaderMap) -> Response {
    // Agents always get markdown; browsers get markdown rendered as plain text.
    let _ = headers;
    text_response("text/markdown; charset=utf-8", AGENTS_MD)
}

async fn serve_agents_md() -> Response {
    text_response("text/markdown; charset=utf-8", AGENTS_MD)
}
async fn serve_whitepaper_md() -> Response {
    text_response("text/markdown; charset=utf-8", WHITEPAPER_MD)
}
async fn serve_spec_md() -> Response {
    text_response("text/markdown; charset=utf-8", SPEC_MD)
}
async fn serve_llms_txt() -> Response {
    text_response("text/plain; charset=utf-8", LLMS_TXT)
}
async fn serve_llms_full() -> Response {
    text_response("text/plain; charset=utf-8", LLMS_FULL_TXT)
}
async fn serve_agent_walkthroughs() -> Response {
    text_response("text/markdown; charset=utf-8", AGENT_WALKTHROUGHS_MD)
}
async fn serve_agent_trial() -> Response {
    text_response("text/markdown; charset=utf-8", AGENT_TRIAL_MD)
}
async fn serve_attesting() -> Response {
    text_response("text/markdown; charset=utf-8", ATTESTING_MD)
}
async fn serve_privacy_md() -> Response {
    text_response("text/markdown; charset=utf-8", PRIVACY_MD)
}
async fn serve_terms_md() -> Response {
    text_response("text/markdown; charset=utf-8", TERMS_MD)
}
async fn serve_support_md() -> Response {
    text_response("text/markdown; charset=utf-8", SUPPORT_MD)
}
async fn serve_global_trial() -> Response {
    text_response("text/markdown; charset=utf-8", GLOBAL_TRIAL_MD)
}
async fn serve_materializers_md() -> Response {
    text_response("text/markdown; charset=utf-8", MATERIALIZERS_MD)
}
async fn serve_spaces_md() -> Response {
    text_response("text/markdown; charset=utf-8", SPACES_MD)
}
async fn serve_temporal_md() -> Response {
    text_response("text/markdown; charset=utf-8", TEMPORAL_MD)
}
async fn serve_robots() -> Response {
    text_response("text/plain; charset=utf-8", ROBOTS_TXT)
}
async fn serve_sitemap() -> Response {
    text_response("application/xml; charset=utf-8", SITEMAP_XML)
}
async fn serve_favicon() -> Response {
    text_response("image/svg+xml; charset=utf-8", FAVICON_SVG)
}
async fn serve_og_image() -> Response {
    text_response("image/svg+xml; charset=utf-8", OG_IMAGE_SVG)
}
async fn serve_indexnow_key() -> Response {
    text_response("text/plain; charset=utf-8", INDEXNOW_KEY)
}
async fn serve_security_txt() -> Response {
    let body = build_security_txt();
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(axum::body::Body::from(body))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// Build a [security.txt](https://datatracker.ietf.org/doc/html/rfc9116) body
/// from environment configuration. Operators set:
///
/// - `EMEM_TLS_CONTACT` — `mailto:` or `https:` URI for security reports.
/// - `EMEM_PUBLIC_URL` — origin for the `Canonical:` field (e.g.
///   `https://emem.dev`); falls back to the first domain in
///   `EMEM_TLS_DOMAINS`. If neither is set, `Canonical:` is omitted.
/// - `EMEM_SECURITY_POLICY_URL` — optional URL for `Policy:`.
///
/// `Expires:` is computed at render time as now + 365d (RFC 9116 recommends
/// ≤1 year), so a long-running responder never serves a stale expiry.
fn build_security_txt() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let mut out = String::new();
    if let Ok(contact) = std::env::var("EMEM_TLS_CONTACT") {
        let c = contact.trim();
        if !c.is_empty() {
            out.push_str("Contact: ");
            out.push_str(c);
            out.push('\n');
        }
    }
    if let Some(origin) = public_origin() {
        out.push_str("Canonical: ");
        out.push_str(&origin);
        out.push_str("/.well-known/security.txt\n");
    }
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        + 365 * 24 * 60 * 60;
    out.push_str(&format!("Expires: {}\n", iso8601_utc(secs)));
    out.push_str("Preferred-Languages: en\n");
    if let Ok(policy) = std::env::var("EMEM_SECURITY_POLICY_URL") {
        let p = policy.trim();
        if !p.is_empty() {
            out.push_str("Policy: ");
            out.push_str(p);
            out.push('\n');
        }
    }
    out
}

/// Returns the configured public origin (no trailing slash), or `None` if
/// neither `EMEM_PUBLIC_URL` nor `EMEM_TLS_DOMAINS` is set.
fn public_origin() -> Option<String> {
    if let Ok(u) = std::env::var("EMEM_PUBLIC_URL") {
        let u = u.trim().trim_end_matches('/');
        if !u.is_empty() {
            return Some(u.to_string());
        }
    }
    if let Ok(domains) = std::env::var("EMEM_TLS_DOMAINS") {
        let first = domains.split(',').next().unwrap_or("").trim();
        if !first.is_empty() {
            return Some(format!("https://{first}"));
        }
    }
    None
}

/// Render `secs` (Unix epoch seconds) as RFC 3339 / ISO 8601 UTC.
fn iso8601_utc(secs: u64) -> String {
    // Days since epoch + civil-from-days (Howard Hinnant) — avoids pulling chrono
    // for one timestamp formatter.
    let days = (secs / 86_400) as i64;
    let sod = (secs % 86_400) as u32;
    let (y, m, d) = civil_from_days(days);
    let hh = sod / 3600;
    let mm = (sod / 60) % 60;
    let ss = sod % 60;
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

/// Convert civil (Y, M, D) → days since 1970-01-01 (the inverse of
/// `civil_from_days`). Same Hinnant reference; works for any year in the
/// proleptic Gregorian calendar without pulling chrono.
fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y } as i64;
    let m = m as i64;
    let d = d as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Unix epoch seconds for `YYYY-01-01T00:00:00Z`. Used as the canonical
/// per-year anchor for annual-snapshot bands like Tessera.
fn jan1_unix(year: i32) -> i64 {
    days_from_civil(year, 1, 1) * 86_400
}

/// Convert days since 1970-01-01 to civil (Y, M, D). Algorithm by Howard
/// Hinnant — overflow-safe for any plausible epoch second.
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}
async fn ai_plugin() -> Response {
    text_response("application/json; charset=utf-8", AI_PLUGIN_JSON)
}
async fn agent_manifest() -> Response {
    text_response("application/json; charset=utf-8", AGENT_JSON)
}

async fn serve_example_claude_desktop() -> Response {
    text_response("application/json", EXAMPLE_CLAUDE_DESKTOP)
}
async fn serve_example_claude_code() -> Response {
    text_response("application/json", EXAMPLE_CLAUDE_CODE)
}
async fn serve_example_cursor() -> Response {
    text_response("application/json", EXAMPLE_CURSOR)
}
async fn serve_example_cline() -> Response {
    text_response("application/json", EXAMPLE_CLINE)
}
async fn serve_example_openai() -> Response {
    text_response("application/json", EXAMPLE_OPENAI)
}
async fn serve_example_langchain() -> Response {
    text_response("text/x-python", EXAMPLE_LANGCHAIN)
}
async fn serve_example_llamaindex() -> Response {
    text_response("text/x-python", EXAMPLE_LLAMAINDEX)
}

// ── Introspection routes ─────────────────────────────────────────────────

async fn health(State(s): State<AppState>) -> Json<JsonValue> {
    // Cheap runtime stats — agents check this before relying on cache
    // hit rates or recently-attested facts. Distinct bands / total
    // facts come from one storage.scan_index pass capped at the same
    // limit coverage_matrix uses, so the call stays bounded even on a
    // large corpus.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let uptime_s = (now - s.started_at_unix_s).max(0);
    let limit: usize = 32_768;
    let mut total_facts: u64 = 0;
    let mut distinct_cells = std::collections::HashSet::<String>::new();
    let mut distinct_bands = std::collections::HashSet::<String>::new();
    if let Ok(rows) = s.storage.iter_index(Some(limit)).await {
        for (k, _) in &rows {
            total_facts += 1;
            distinct_cells.insert(k.cell.clone());
            distinct_bands.insert(k.band.clone());
        }
    }
    Json(json!({
        "ok": true,
        "name": "emem",
        "version": env!("CARGO_PKG_VERSION"),
        "responder_pubkey_b32": data_encoding::BASE32_NOPAD
            .encode(&s.identity.pubkey.0)
            .to_lowercase(),
        "responder_key_epoch": s.identity.epoch.0,
        "registry_cid": s.manifests.registry_cid.as_str(),
        "schema_cid": s.manifests.schema_cid.as_str(),
        "bands_cid": &s.manifests.bands_cid,
        "sources_cid": &s.manifests.sources_cid,
        "algorithms_cid": ALGORITHMS_CID.clone(),
        "started_at_unix_s": s.started_at_unix_s,
        "now_unix_s": now,
        "uptime_seconds": uptime_s,
        "corpus": {
            "facts_scanned": total_facts,
            "distinct_cells": distinct_cells.len(),
            "distinct_bands": distinct_bands.len(),
            "scan_index_limit": limit,
            "note": "facts_scanned reflects the index scan cap; corpora above the cap are paginated via /v1/coverage_matrix",
        },
    }))
}

async fn well_known(State(s): State<AppState>) -> Json<JsonValue> {
    Json(json!({
        "protocol": "emem",
        "version": "0.1",
        "manifests": {
            "bands_cid": &s.manifests.bands_cid,
            "sources_cid": &s.manifests.sources_cid,
            "registry_cid": s.manifests.registry_cid.as_str(),
            "schema_cid": s.manifests.schema_cid.as_str(),
            "algorithms_cid": ALGORITHMS_CID.clone(),
        },
        "responder": {
            "pubkey_b32": data_encoding::BASE32_NOPAD.encode(&s.identity.pubkey.0).to_lowercase(),
            "key_epoch": s.identity.epoch.0,
            "signature_alg": "ed25519",
            "hash_alg": "blake3",
            "cid_encoding": "base32-nopad-lowercase",
        },
        "tools_url": "/v1/tools",
        "openapi_url": "/openapi.json",
        "mcp_url": "/mcp",
        "agent_card_url": "/v1/agent_card",
        "quickstart_url": "/v1/quickstart",
        // Discovery hooks for connector-directory reviewers + offline
        // signature-verifying clients. The full text lives at /privacy,
        // /terms, /support; the canonical contact is the maintainer
        // email. `operator` carries the legal entity (Vortx AI Private
        // Limited, India); the flat `vendor`/`*_url` keys below are
        // kept as compatibility shims for tools that scrape the older
        // shape from earlier in the directory-prep work.
        "vendor": "Vortx-AI",
        "contact_email": "avijeet@vortx.ai",
        "privacy_url": "/privacy",
        "terms_url":   "/terms",
        "support_url": "/support",
        "operator": {
            "name":    "Vortx AI Private Limited",
            "country": "India",
            "url":     "https://vortx.ai",
            "contact": "avijeet@vortx.ai",
        },
        "policies": {
            "privacy_policy":   "/privacy",
            "terms_of_service": "/terms",
            "support":          "/support",
            "security":         "https://github.com/Vortx-AI/emem/blob/main/SECURITY.md",
        },
    }))
}

async fn manifests(State(s): State<AppState>) -> Json<JsonValue> {
    let algorithms_cid = ALGORITHMS_CID.clone();
    Json(json!({
        "bands_cid": &s.manifests.bands_cid,
        "functions_cid": s.manifests.registry_cid.as_str(),
        "sources_cid": &s.manifests.sources_cid,
        "schema_cid": s.manifests.schema_cid.as_str(),
        "algorithms_cid": algorithms_cid,
    }))
}

async fn bands() -> Json<JsonValue> {
    Json(serde_json::to_value(&*emem_core::bands::DEFAULT).unwrap_or(json!({})))
}

/// `GET /v1/materializers` — declare which bands this responder will
/// auto-materialize on a recall miss, what upstream source produces
/// the value, and what the resulting fact looks like (Primary or
/// Absence). Lets an agent discover, without trial-and-error, which
/// bands work cite-ably for any cell on Earth versus which require a
/// pre-existing attestation.
async fn materializers(State(s): State<AppState>) -> Json<JsonValue> {
    let pubkey_b32 = data_encoding::BASE32_NOPAD
        .encode(&s.identity.pubkey.0)
        .to_lowercase();
    let mut payload = json!({
        "schema": "emem.materializers.v1",
        "responder_pubkey_b32": pubkey_b32.clone(),
        "auto_materialize_enabled": auto_materialize_enabled(),
        "materializers": [
            {
                "band":              "modis.ndvi_mean",
                "unit":              null,
                "value_kind":        "primary",
                "coverage":          "global terrestrial; 250m native; latest valid 16-day composite within last 90 days",
                "upstream_scheme":   "ornl_modis",
                "upstream_endpoint": "https://modis.ornl.gov/rst/api/v1/MOD13Q1/subset",
                "derivation_fn_key": "modis_ornl_subset@1",
                "confidence":        0.9,
                "tempo":             "medium",
                "kernel_for_router": "heat_gaussian",
                "notes":             "MOD13Q1 16-day composite NDVI from MODIS Terra. ~10–15 s upstream latency. NDVI ∈ [-0.2, 1.0]; values <0 mean water/snow, >0.4 mean dense vegetation. Quality flag filtering uses fill_value=-3000 from the NASA QA layer."
            },
            {
                "band":              "gmrt.topobathy_mean",
                "unit":              "m",
                "value_kind":        "primary",
                "coverage":          "global; positive over land, negative over water",
                "upstream_scheme":   "gmrt",
                "upstream_endpoint": "https://www.gmrt.org/services/PointServer",
                "derivation_fn_key": "gmrt_pointserver@1",
                "confidence":        0.9,
                "notes":             "Lamont-Doherty Earth Observatory's Global Multi-Resolution Topography. Single dataset that fuses Cop-DEM, GEBCO, multibeam swaths, and high-res surveys into a globally-consistent topo-bathy raster. The right band when you need a number for any point on Earth, including ocean."
            },
            {
                "band":              "copdem30m.elevation_mean",
                "unit":              "m",
                "value_kind":        "primary_or_absence",
                "coverage":          "land surface only; |lat| < ~85°. Returns Fact::Absence over open water (Cop-DEM uses 0 m as no-data marker over ocean) or upstream no-coverage zones.",
                "upstream_scheme":   "open_meteo",
                "upstream_endpoint": "https://api.open-meteo.com/v1/elevation",
                "derivation_fn_key": "open_meteo_copdem90m@1",
                "confidence":        0.95,
                "notes":             "Use this when you specifically want a *land DEM* answer and want a signed absence over water rather than topo-bathy. For most general-purpose elevation queries, prefer gmrt.topobathy_mean."
            },
            {
                "band":              "geotessera",
                "unit":              null,
                "value_kind":        "primary",
                "value_shape":       [128],
                "coverage":          "global terrestrial; 0.1° tile grid; v1 vintage 2024",
                "upstream_scheme":   "geotessera",
                "upstream_endpoint": "https://dl2.geotessera.org/v1/global_0.1_degree_representation",
                "derivation_fn_key": "geotessera_v1@1",
                "confidence":        0.85,
                "tempo":             "slow",
                "kernel_for_router": "linear_ar1",
                "fetch_strategy":    "https_range",
                "fetch_bytes_per_cell": 640,
                "notes":             "Tessera 128-D foundation embedding (Cambridge/Clay-style; quantized int8 + float32 scales). Per-cell HTTPS range reads against the public bucket — ~640 B downloaded per recall instead of the full 91 MB tile. Native CRS is per-tile UTM; we sample by linear (lat,lng)→(row,col) within the 0.1° tile so corner samples have ~1–2 px UTM-vs-EPSG:4326 skew that's recorded in derivation.args. Default vintage 2024; use geotessera.YYYY for an explicit year, or geotessera.multi_year for the 8-year stack."
            },
            {
                "band":              "geotessera.multi_year",
                "unit":              null,
                "value_kind":        "primary",
                "value_shape":       [1024],
                "coverage":          "global terrestrial; 0.1° tile grid; 8-year stack 2017-2024",
                "upstream_scheme":   "geotessera",
                "upstream_endpoint": "https://dl2.geotessera.org/v1/global_0.1_degree_representation",
                "derivation_fn_key": "geotessera_multi_year@1",
                "confidence":        0.85,
                "tempo":             "slow",
                "kernel_for_router": "linear_ar1",
                "fetch_strategy":    "8x https_range (one per year)",
                "fetch_bytes_per_cell": 5120,
                "notes":             "1024-D = 128 × 8 years (2017,2018,2019,2020,2021,2022,2023,2024). Years with no tile coverage at this cell get zero-padded slices; derivation.args.years_covered records which slices are real. Use this for time-aware similarity search — the temporal trajectory of a place across the Tessera vintage."
            },
            {
                "band":              "weather.temperature_2m",
                "unit":              "degC",
                "value_kind":        "primary",
                "coverage":          "global; 15-minute updates from blended HRRR/ICON/GFS/ECMWF",
                "upstream_scheme":   "met_no",
                "upstream_endpoint": "https://api.met.no/weatherapi/locationforecast/2.0/compact",
                "derivation_fn_key": "met_no_locationforecast_compact@1",
                "confidence":        0.85,
                "tempo":             "ultra_fast",
                "kernel_for_router": "advection_linear",
                "notes":             "Current 2-m air temperature in °C. MET Norway's locationforecast/2.0/compact requires no API key and is not per-IP rate-limited; their TOS asks only for an identifying User-Agent. The data is sat-fed (ECMWF + EUMETSAT geostationary fleet)."
            },
            {
                "band":              "weather.cloud_cover",
                "unit":              "percent",
                "value_kind":        "primary",
                "coverage":          "global; 15-minute updates",
                "upstream_scheme":   "met_no",
                "upstream_endpoint": "https://api.met.no/weatherapi/locationforecast/2.0/compact",
                "derivation_fn_key": "met_no_locationforecast_compact@1",
                "confidence":        0.80,
                "tempo":             "ultra_fast",
                "kernel_for_router": "advection_linear",
                "notes":             "Total cloud-cover percentage from the geostationary-fed NWP blend. Use alongside indices.ndvi to gate vegetation queries — high cloud_cover means the latest optical composite is likely stale."
            },
            {
                "band":              "weather.precipitation_mm",
                "unit":              "mm",
                "value_kind":        "primary",
                "coverage":          "global; 15-minute updates",
                "upstream_scheme":   "met_no",
                "upstream_endpoint": "https://api.met.no/weatherapi/locationforecast/2.0/compact",
                "derivation_fn_key": "met_no_locationforecast_compact@1",
                "confidence":        0.75,
                "tempo":             "ultra_fast",
                "kernel_for_router": "advection_linear",
                "notes":             "Liquid-equivalent precipitation in the last 15-minute window."
            },
            {
                "band":              "weather.wind_speed_10m",
                "unit":              "m/s",
                "value_kind":        "primary",
                "coverage":          "global; 15-minute updates",
                "upstream_scheme":   "met_no",
                "upstream_endpoint": "https://api.met.no/weatherapi/locationforecast/2.0/compact",
                "derivation_fn_key": "met_no_locationforecast_compact@1",
                "confidence":        0.80,
                "tempo":             "ultra_fast",
                "kernel_for_router": "advection_linear",
                "notes":             "10-m wind speed in m/s. The advection kernel matches how wind itself transports the rest of the weather state."
            },
            {
                "band":              "indices.ndvi",
                "unit":              null,
                "value_kind":        "primary",
                "coverage":          "global; Sentinel-2 L2A 10 m; 5-day revisit at the equator",
                "upstream_scheme":   "sentinel_s2_l2a",
                "upstream_endpoint": "https://earth-search.aws.element84.com/v1/search → https://sentinel-cogs.s3.us-west-2.amazonaws.com/...",
                "derivation_fn_key": "sentinel2_l2a_ndvi@1",
                "confidence":        0.92,
                "tempo":             "fast",
                "kernel_for_router": "wave_seasonal",
                "fetch_strategy":    "stac_search + https_range_cog",
                "fetch_bytes_per_cell": "~600 KB (IFD + 1 tile per band × 2 bands)",
                "notes":             "Pure-Rust COG range read against AWS Open Data sentinel-cogs bucket. STAC search picks the latest scene <40% cloud that *contains* the point (intersects: Point); reflectance scale = 1e-4. NDVI = (B08 − B04) / (B08 + B04). No API key. Range read uses Predictor 2 (horizontal differencing) + Deflate decompression."
            },
            {
                "band":              "sentinel1_raw",
                "unit":              "dB",
                "value_kind":        "primary",
                "coverage":          "global; Sentinel-1 RTC (Radiometrically Terrain Corrected) C-band SAR; ~10 m; 6-12-day revisit",
                "upstream_scheme":   "sentinel_s1_rtc_mpc",
                "upstream_endpoint": "https://planetarycomputer.microsoft.com/api/stac/v1/search (collection: sentinel-1-rtc) → Azure Blob COG signed with anonymous SAS",
                "derivation_fn_key": "sentinel1_rtc_vv_db@1",
                "confidence":        0.85,
                "tempo":             "fast",
                "kernel_for_router": "wave_seasonal",
                "fetch_strategy":    "stac_search + https_range_cog (Azure SAS-signed)",
                "fetch_bytes_per_cell": "~700 KB (IFD + 1 tile)",
                "notes":             "VV polarisation gamma-naught backscatter in dB (10·log₁₀ of linear power). Radar — works at night and through cloud, complements indices.ndvi for all-weather monitoring. Source: Microsoft Planetary Computer's `sentinel-1-rtc` collection — proper UTM-projected COG (the upstream Element84 `sentinel-1-grd` mirror ships SAFE-format scenes with GCP-based georeferencing the pure-Rust COG sampler can't decode). Asset URLs are anonymous Azure Blobs signed with a free SAS token cached process-wide."
            },
            {
                "band":              "surface_water.recurrence",
                "unit":              "percent",
                "value_kind":        "primary_or_absence",
                "value_shape":       "scalar",
                "coverage":          "global ±60° lat from JRC observation footprint; 30 m native; static climatology over 1984-2021. Absence over permanent non-water and unmapped polar interiors (255 nodata).",
                "upstream_scheme":   "jrc.gsw.v1_4.recurrence",
                "upstream_endpoint": "https://storage.googleapis.com/global-surface-water/downloads2021/recurrence/recurrence_{lon_left10}_{lat_top10}v1_4_2021.tif",
                "derivation_fn_key": "jrc_gsw_recurrence_pixel@1",
                "confidence":        0.95,
                "tempo":             "static",
                "kernel_for_router": "linear_ar1",
                "fetch_strategy":    "https_range_cog",
                "fetch_bytes_per_cell": "~320 KB (IFD head + 1 tile)",
                "notes":             "Inter-annual water recurrence in percent: 0 = never water, 100 = water every year of the 1984-2021 record, intermediate = flood-prone or seasonal water. The canonical signed answer to 'has this place been wet historically?' Pure-Rust COG range read against the public GCS bucket; tiles are EPSG:4326 so no UTM projection step. License: JRC open."
            },
            {
                "band":              "overture.buildings.count",
                "unit":              null,
                "value_kind":        "primary",
                "coverage":          "global vector; Overture Maps Foundation 2026-04-15 release",
                "upstream_scheme":   "overture.maps.foundation.v1",
                "upstream_endpoint": "s3://overturemaps-us-west-2/release/2026-04-15.0/theme=buildings/type=building/",
                "derivation_fn_key": "overture_buildings_count@1",
                "confidence":        0.95,
                "tempo":             "slow",
                "kernel_for_router": "linear_ar1",
                "fetch_strategy":    "anonymous_s3 + parquet_row_group_pruning + wkb_polygon_centroid",
                "notes":             "Count of building footprints whose vertex-mean centroid falls inside the cell bbox. Pure-Rust path: object_store anonymous AWS S3 + parquet 55 async reader + WKB decode. No GDAL, no Python, no API key. Per-file parquet footers cached in process memory; first-call to a region warms the cache."
            },
            {
                "band":              "overture.places.count",
                "unit":              null,
                "value_kind":        "primary",
                "coverage":          "global vector; Overture Maps Foundation 2026-04-15 release",
                "upstream_scheme":   "overture.maps.foundation.v1",
                "upstream_endpoint": "s3://overturemaps-us-west-2/release/2026-04-15.0/theme=places/type=place/",
                "derivation_fn_key": "overture_places_count@1",
                "confidence":        0.90,
                "tempo":             "slow",
                "kernel_for_router": "linear_ar1",
                "fetch_strategy":    "anonymous_s3 + parquet_row_group_pruning + wkb_point_inside",
                "notes":             "Count of POIs (places) whose Point geometry falls inside the cell bbox. Same anonymous S3 + pure-Rust parquet path as buildings."
            },
            {
                "band":              "overture.transportation.road_length_m",
                "unit":              "m",
                "value_kind":        "primary",
                "coverage":          "global vector; Overture Maps Foundation 2026-04-15 release",
                "upstream_scheme":   "overture.maps.foundation.v1",
                "upstream_endpoint": "s3://overturemaps-us-west-2/release/2026-04-15.0/theme=transportation/type=segment/",
                "derivation_fn_key": "overture_road_length_m@1",
                "confidence":        0.85,
                "tempo":             "slow",
                "kernel_for_router": "linear_ar1",
                "fetch_strategy":    "anonymous_s3 + parquet_row_group_pruning + wkb_linestring_clip",
                "notes":             "Sum of road-segment length (metres) intersecting the cell bbox. Each WKB LineString is clipped to the bbox via Liang-Barsky and projected planar with a local-tangent-plane scale at the cell's mid-latitude. The cell is small (~305 m × ~190 m at 52° N) so planar approximation is within centimetres of haversine."
            }
        ],
        "agent_hint": {
            "how_it_works": "Call POST /v1/recall {cell, bands: [<band>]}. If the cell has no fact yet AND auto_materialize_enabled is true, the responder fetches the upstream value, signs the resulting fact under its identity, persists it, and returns it in the same response. The next call hits the hot cache (~10 ms instead of ~180 ms).",
            "trust_model":  "Materialized facts are signed by the responder pubkey above, NOT by the upstream provider. The fact's `derivation.fn_key` declares the function that produced the value; an external attester can run the same function and submit their own signed fact to corroborate or correct.",
            "absence_facts": "Fact::Absence (kind: \"absence\") records confirmed no-data with a content-addressed `reason_cid`. Treat it as a signed statement that the responder tried and got no answer — don't re-fetch on every call.",
            "history_bounds": "Each entry now carries `history_available_from_unix` / `history_available_to_unix` derived from the upstream provider's documented record. `null` means present-only (e.g. weather nowcast, Overture release snapshot, or static climatology). Pass these to `emem_backfill` to materialize and sign every per-tslot fact in the window — turns 'I want history' into 'history exists in the ledger'.",
        }
    });

    // Decorate every materializer entry with history bounds + a tempo
    // seconds value so an agent can size an `emem_backfill` window
    // without a second round-trip to /v1/coverage_matrix. We post-process
    // here rather than inlining per-entry to keep the inline JSON literal
    // above the single source of truth for connector text — `band_materializer_meta`
    // is the single source of truth for the bounds themselves.
    if let Some(arr) = payload
        .get_mut("materializers")
        .and_then(|v| v.as_array_mut())
    {
        for entry in arr.iter_mut() {
            let band = entry
                .get("band")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let Some(band) = band else { continue };
            let meta = band_materializer_meta(&band);
            if let Some(obj) = entry.as_object_mut() {
                obj.insert(
                    "history_available_from_unix".into(),
                    meta.as_ref()
                        .and_then(|m| m.history_from_unix)
                        .map(JsonValue::from)
                        .unwrap_or(JsonValue::Null),
                );
                obj.insert(
                    "history_available_to_unix".into(),
                    meta.as_ref()
                        .and_then(|m| m.history_to_unix)
                        .map(JsonValue::from)
                        .unwrap_or(JsonValue::Null),
                );
                if let Some(m) = meta.as_ref() {
                    obj.insert(
                        "tempo_seconds".into(),
                        JsonValue::from(m.tempo.slot_seconds()),
                    );
                    obj.insert("temporal_kind".into(), JsonValue::from(m.kind.as_str()));
                    obj.insert("upstream_wire_path".into(), JsonValue::from(m.wire_path));
                }
                obj.insert(
                    "responder_pubkey_b32".into(),
                    JsonValue::from(pubkey_b32.clone()),
                );
            }
        }
    }
    // Backfill every materializable band that the curated payload above
    // doesn't already mention, using `band_materializer_meta` +
    // `all_materializable_bands` as the registry. This guarantees the
    // /v1/materializers endpoint is never a partial view of the responder
    // — agents that introspect either /v1/materializers or
    // /v1/data_availability see the same band list. The auto-generated
    // entries have less editorial text but full machine-readable fields.
    if let Some(arr) = payload
        .get_mut("materializers")
        .and_then(|v| v.as_array_mut())
    {
        let curated: std::collections::HashSet<String> = arr
            .iter()
            .filter_map(|e| {
                e.get("band")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .collect();
        for band in all_materializable_bands() {
            if curated.contains(&band) {
                continue;
            }
            let Some(meta) = band_materializer_meta(&band) else {
                continue;
            };
            let mut obj = serde_json::Map::new();
            obj.insert("band".into(), JsonValue::from(band.clone()));
            obj.insert("value_kind".into(), JsonValue::from("primary"));
            obj.insert("auto_generated".into(), JsonValue::from(true));
            obj.insert(
                "notes".into(),
                JsonValue::from("auto-registered from band_materializer_meta — see /v1/data_availability for the full machine-readable catalog and any peer-reviewed citation in source code"),
            );
            obj.insert(
                "history_available_from_unix".into(),
                meta.history_from_unix
                    .map(JsonValue::from)
                    .unwrap_or(JsonValue::Null),
            );
            obj.insert(
                "history_available_to_unix".into(),
                meta.history_to_unix
                    .map(JsonValue::from)
                    .unwrap_or(JsonValue::Null),
            );
            obj.insert(
                "tempo_seconds".into(),
                JsonValue::from(meta.tempo.slot_seconds()),
            );
            obj.insert("temporal_kind".into(), JsonValue::from(meta.kind.as_str()));
            obj.insert("upstream_wire_path".into(), JsonValue::from(meta.wire_path));
            obj.insert(
                "responder_pubkey_b32".into(),
                JsonValue::from(pubkey_b32.clone()),
            );
            arr.push(JsonValue::Object(obj));
        }
    }
    Json(payload)
}

/// `GET /v1/data_availability` — temporal catalog: for every band this
/// responder can materialize, the upstream-of-record window the data
/// genuinely covers and the temporal shape an agent should expect from
/// `recall` / `backfill`. Driven entirely by `band_materializer_meta`
/// (single source of truth) and `all_materializable_bands` (concrete
/// enumeration including each Tessera vintage). No hardcoded windows
/// or duplicated lookups.
///
/// Shape (per entry):
/// ```json
/// {
///   "band": "geotessera.2020",
///   "kind": "annual_snapshot",
///   "tempo": "slow",
///   "tempo_seconds": 31556952,
///   "history_available_from_unix": 1577836800,
///   "history_available_to_unix":   1609459199,
///   "history_available_from_iso":  "2020-01-01T00:00:00Z",
///   "history_available_to_iso":    "2020-12-31T23:59:59Z",
///   "upstream_wire_path": "dl2.geotessera.org per-year .npy HTTPS-Range",
///   "backfill_supported": true,
///   "tslot_grid_seconds": 31556952
/// }
/// ```
///
/// `backfill_supported` is `true` iff the band has a `kind` other than
/// `now_only` AND `materialize_band_at` will accept a past `target_unix`
/// for it. Used by /v1/backfill clients to skip nowcast bands without a
/// trial-and-error 422.
async fn data_availability(State(s): State<AppState>) -> Json<JsonValue> {
    let pubkey_b32 = data_encoding::BASE32_NOPAD
        .encode(&s.identity.pubkey.0)
        .to_lowercase();
    let mut entries: Vec<JsonValue> = Vec::new();
    for band in all_materializable_bands() {
        let Some(meta) = band_materializer_meta(&band) else {
            continue;
        };
        let backfill_supported = !matches!(meta.kind, BandKind::NowOnly);
        let from_iso = meta
            .history_from_unix
            .and_then(|u| u64::try_from(u).ok())
            .map(iso8601_utc);
        let to_iso = meta
            .history_to_unix
            .and_then(|u| u64::try_from(u).ok())
            .map(iso8601_utc);
        entries.push(json!({
            "band": band,
            "kind": meta.kind.as_str(),
            "tempo": format!("{:?}", meta.tempo).to_lowercase(),
            "tempo_seconds": meta.tempo.slot_seconds(),
            "tslot_grid_seconds": meta.tempo.slot_seconds(),
            "history_available_from_unix": meta.history_from_unix,
            "history_available_to_unix": meta.history_to_unix,
            "history_available_from_iso": from_iso,
            "history_available_to_iso": to_iso,
            "upstream_wire_path": meta.wire_path,
            "backfill_supported": backfill_supported,
        }));
    }
    Json(json!({
        "schema": "emem.data_availability.v1",
        "responder_pubkey_b32": pubkey_b32,
        "auto_materialize_enabled": auto_materialize_enabled(),
        "tessera_vintages": TESSERA_YEARS_RANGE_PUBLIC.clone().collect::<Vec<_>>(),
        "kinds": [
            {"name": "static",          "meaning": "single signed fact valid for all time (Cop-DEM, GMRT, JRC GSW recurrence)"},
            {"name": "annual_snapshot", "meaning": "one fact per calendar year on Jan 1 UTC (Tessera per-year)"},
            {"name": "annual_stack",    "meaning": "stack of multiple annual snapshots fused into one fact (Tessera multi-year 1024-D)"},
            {"name": "time_series",     "meaning": "per-tslot historical series fetched on demand from a STAC-style archive (Sentinel-2 L2A, Sentinel-1 RTC, MODIS NDVI)"},
            {"name": "now_only",        "meaning": "provider only exposes current value plus short forecast — no historical record (met.no)"},
            {"name": "per_release",     "meaning": "versioned global snapshot — each release replaces the previous (Overture Maps)"}
        ],
        "entries": entries,
    }))
}

/// `GET /v1/fleet` — declare the satellite/sensor lineage that feeds each
/// materialized band. The protocol-level contract is the *band manifest*;
/// this endpoint is the editorial layer that names the actual hardware
/// platforms and their cadences, so an agent can decide "I need 15-min
/// observation rate" or "I need radar so it works at night" without
/// reading every band's documentation.
///
/// The mapping is keyed by upstream sensor (TerrA-MODIS, GMRT multibeam,
/// Cop-DEM 90m, etc.) → list of bands that derive from it. Geostationary
/// weather satellites (GOES-16/17/18, Himawari-9, Meteosat-9/11) are
/// declared as the *cadence source* for the weather bands even though
/// the wire path goes through Open-Meteo's NWP blend — this matches how
/// the temporal router classifies these bands as `ultra_fast`.
/// `GET /v1/coverage_matrix` — observability layer that lets agents (and
/// operators) see, in one call, which bands are *actually being answered*
/// by the responder right now. Combines the static manifest (what bands
/// exist), the materializer registry (what we know how to fetch), and the
/// live storage index (what we have).
///
/// For every band in the active manifest plus every materializer-only band
/// (modis.ndvi_mean, gmrt.topobathy_mean, copdem30m.elevation_mean,
/// indices.*, s2.*, sentinel1_raw, geotessera.*, weather.*) we surface:
///   - has_materializer: bool — wired into try_materialize_bands?
///   - facts_count       : u64 — distinct (cell, tslot) pairs cached
///   - last_attested_at  : ISO 8601 — when the responder last signed under
///     this band; helps an agent cite the freshest band for a query
///   - tempo / family    : from the manifest, for the temporal router
///   - sat_lineage       : satellite/sensor names from the fleet declaration
///
/// O(N) over the full storage index; capped by EMEM_COVERAGE_MATRIX_LIMIT
/// (default 50_000) so big deployments don't blow the response.
async fn coverage_matrix(State(s): State<AppState>) -> Json<JsonValue> {
    use std::collections::BTreeMap;
    let limit: usize = std::env::var("EMEM_COVERAGE_MATRIX_LIMIT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(50_000);

    // Aggregate over the storage index. For each entry we resolve the
    // actual fact and parse its `signed_at` to get a real wall-clock
    // last-attested per band. This is one cache get_many call per ~256
    // entries (chunked) so a 50k-entry deployment still answers in
    // sub-second range. Entries with non-zero tslots that *look* like
    // unix timestamps still update the bound (cheap path) so weather/
    // index bands surface freshness even when the get_many lookup is
    // slow or partial.
    let mut counts: BTreeMap<String, (u64, Option<i64>)> = BTreeMap::new();
    if let Ok(entries) = s.storage.iter_index(Some(limit)).await {
        // First pass: counts + tslot proxy (cheap, no fact lookups).
        let mut cids: Vec<emem_fact::FactCid> = Vec::with_capacity(entries.len());
        let mut keys: Vec<emem_cache::CanonicalKey> = Vec::with_capacity(entries.len());
        for (k, cid) in entries {
            let entry = counts.entry(k.band.clone()).or_insert((0, None));
            entry.0 += 1;
            if k.tslot >= 1_577_836_800 {
                let cur = entry.1.unwrap_or(0);
                if (k.tslot as i64) > cur {
                    entry.1 = Some(k.tslot as i64);
                }
            }
            cids.push(cid);
            keys.push(k);
        }
        // Second pass: chunked fact resolution to pick up signed_at on
        // bands whose tslot is 0 (static bands like copdem, koppen, dem)
        // or non-timestamp-shaped (foundation embedding tslots).
        const CHUNK: usize = 256;
        for (chunk_keys, chunk_cids) in keys.chunks(CHUNK).zip(cids.chunks(CHUNK)) {
            if let Ok(facts) = s.storage.get_facts_many(chunk_cids).await {
                for (k, fact) in chunk_keys.iter().zip(facts) {
                    let signed_at_unix = match fact.as_ref() {
                        Some(emem_fact::Fact::Primary(p)) => parse_iso8601_unix(&p.signed_at),
                        Some(emem_fact::Fact::Absence(n)) => parse_iso8601_unix(&n.signed_at),
                        _ => None,
                    };
                    if let Some(ts) = signed_at_unix {
                        let entry = counts.entry(k.band.clone()).or_insert((0, None));
                        let cur = entry.1.unwrap_or(0);
                        if ts > cur {
                            entry.1 = Some(ts);
                        }
                    }
                }
            }
        }
    }

    // Bands that ship a materializer in this build, with metadata for the agent.
    let materializer_bands: &[(&str, &str, &str, &[&str])] = &[
        ("modis.ndvi_mean", "medium", "vegetation",
            &["Terra (NASA EOS AM-1) MODIS"]),
        ("gmrt.topobathy_mean", "static", "terrain",
            &["GMRT (Lamont-Doherty multibeam fleet)"]),
        ("copdem30m.elevation_mean", "static", "terrain",
            &["TanDEM-X / Copernicus DEM"]),
        ("geotessera", "slow", "foundation",
            &["Tessera v1 (Cambridge AAILab + Clay-style composite)"]),
        ("geotessera.multi_year", "slow", "foundation",
            &["Tessera v1 multi-year (2017-2024 stacked, 1024-D)"]),
        ("geotessera.2017", "slow", "foundation", &["Tessera v1 2017"]),
        ("geotessera.2018", "slow", "foundation", &["Tessera v1 2018"]),
        ("geotessera.2019", "slow", "foundation", &["Tessera v1 2019"]),
        ("geotessera.2020", "slow", "foundation", &["Tessera v1 2020"]),
        ("geotessera.2021", "slow", "foundation", &["Tessera v1 2021"]),
        ("geotessera.2022", "slow", "foundation", &["Tessera v1 2022"]),
        ("geotessera.2023", "slow", "foundation", &["Tessera v1 2023"]),
        ("geotessera.2024", "slow", "foundation", &["Tessera v1 2024"]),
        ("indices.ndvi", "fast", "vegetation", &["Sentinel-2A/B/C MSI (NDVI)"]),
        ("indices.ndwi", "fast", "water",      &["Sentinel-2A/B/C MSI (NDWI Gao)"]),
        ("indices.mndwi", "fast", "water",     &["Sentinel-2A/B/C MSI (MNDWI McFeeters)"]),
        ("indices.evi", "fast", "vegetation",  &["Sentinel-2A/B/C MSI (EVI)"]),
        ("indices.nbr", "fast", "fire",        &["Sentinel-2A/B/C MSI (NBR)"]),
        ("indices.ndmi", "fast", "vegetation", &["Sentinel-2A/B/C MSI (NDMI)"]),
        ("indices.savi", "fast", "vegetation", &["Sentinel-2A/B/C MSI (SAVI)"]),
        ("indices.bsi",  "fast", "soil",       &["Sentinel-2A/B/C MSI (BSI)"]),
        ("indices.ndbi", "fast", "human",      &["Sentinel-2A/B/C MSI (NDBI built-up)"]),
        ("s2.B01", "fast", "optical", &["Sentinel-2A/B/C MSI B01 60m coastal aerosol"]),
        ("s2.B02", "fast", "optical", &["Sentinel-2A/B/C MSI B02 10m blue"]),
        ("s2.B03", "fast", "optical", &["Sentinel-2A/B/C MSI B03 10m green"]),
        ("s2.B04", "fast", "optical", &["Sentinel-2A/B/C MSI B04 10m red"]),
        ("s2.B05", "fast", "optical", &["Sentinel-2A/B/C MSI B05 20m red-edge 1"]),
        ("s2.B06", "fast", "optical", &["Sentinel-2A/B/C MSI B06 20m red-edge 2"]),
        ("s2.B07", "fast", "optical", &["Sentinel-2A/B/C MSI B07 20m red-edge 3"]),
        ("s2.B08", "fast", "optical", &["Sentinel-2A/B/C MSI B08 10m wide NIR"]),
        ("s2.B8A", "fast", "optical", &["Sentinel-2A/B/C MSI B8A 20m narrow NIR"]),
        ("s2.B09", "fast", "optical", &["Sentinel-2A/B/C MSI B09 60m water vapor"]),
        ("s2.B11", "fast", "optical", &["Sentinel-2A/B/C MSI B11 20m SWIR-1"]),
        ("s2.B12", "fast", "optical", &["Sentinel-2A/B/C MSI B12 20m SWIR-2"]),
        ("s2.scl", "fast", "vision",  &["Sentinel-2 L2A scene-classification (uint8 0..11)"]),
        ("sentinel1_raw", "fast", "radar",
            &["Sentinel-1A/C C-band SAR (RTC γ0 VV in dB) via Microsoft Planetary Computer"]),
        ("surface_water.recurrence", "static", "water",
            &["JRC Global Surface Water v1.4 (Pekel et al. 2016, Landsat-derived 1984-2021 inter-annual recurrence climatology)"]),
        ("weather.temperature_2m", "ultra_fast", "climate",
            &["MET Norway api.met.no — sat-fed via ECMWF + EUMETSAT geostationary fleet"]),
        ("weather.cloud_cover", "ultra_fast", "climate",
            &["MET Norway api.met.no"]),
        ("weather.precipitation_mm", "ultra_fast", "climate",
            &["MET Norway api.met.no"]),
        ("weather.wind_speed_10m", "ultra_fast", "climate",
            &["MET Norway api.met.no"]),
        ("weather.relative_humidity_2m", "ultra_fast", "climate",
            &["MET Norway api.met.no — instant.details.relative_humidity"]),
        ("weather.dew_point_2m", "ultra_fast", "climate",
            &["MET Norway api.met.no — instant.details.dew_point_temperature"]),
        ("weather.air_pressure_msl", "ultra_fast", "climate",
            &["MET Norway api.met.no — instant.details.air_pressure_at_sea_level"]),
        ("weather.wind_direction_10m", "ultra_fast", "climate",
            &["MET Norway api.met.no — instant.details.wind_from_direction"]),
        ("overture.buildings.count", "slow", "human",
            &["Overture Maps Foundation buildings parquet (anonymous S3)"]),
        ("overture.places.count", "slow", "human",
            &["Overture Maps Foundation places parquet (anonymous S3)"]),
        ("overture.transportation.road_length_m", "slow", "human",
            &["Overture Maps Foundation transportation parquet (anonymous S3)"]),
    ];
    let mat_set: std::collections::HashSet<&str> = materializer_bands.iter().map(|t| t.0).collect();

    let registry = &*emem_core::bands::DEFAULT;
    let mut bands_json: Vec<JsonValue> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Cube placeholder slots are family aliases (`dem`, `cop_dem`,
    // `climate`, `indices`, `landcover`, …) — they reserve cube offsets
    // for a *family* of attestable sub-keys and are not themselves
    // directly materialized. Without this map, the coverage matrix
    // shows `cop_dem mat=False facts=0` next to a working
    // `copdem30m.elevation_mean mat=True`, which makes agents conclude
    // "DEM is offline" when in fact it's wired under a different key.
    let cube_aliases: &[(&str, &[&str])] = &[
        ("dem", &["copdem30m.elevation_mean", "gmrt.topobathy_mean"]),
        ("cop_dem", &["copdem30m.elevation_mean"]),
        (
            "climate",
            &[
                "weather.temperature_2m",
                "weather.cloud_cover",
                "weather.precipitation_mm",
                "weather.wind_speed_10m",
                "weather.relative_humidity_2m",
                "weather.dew_point_2m",
                "weather.air_pressure_msl",
                "weather.wind_direction_10m",
            ],
        ),
        (
            "indices",
            &[
                "indices.ndvi",
                "indices.ndwi",
                "indices.mndwi",
                "indices.evi",
                "indices.nbr",
                "indices.ndmi",
                "indices.savi",
                "indices.bsi",
                "indices.ndbi",
            ],
        ),
        (
            "sentinel2_raw",
            &[
                "s2.B01", "s2.B02", "s2.B03", "s2.B04", "s2.B05", "s2.B06", "s2.B07", "s2.B08",
                "s2.B8A", "s2.B09", "s2.B11", "s2.B12", "s2.scl",
            ],
        ),
        (
            "geotessera",
            &[
                "geotessera",
                "geotessera.multi_year",
                "geotessera.2017",
                "geotessera.2018",
                "geotessera.2019",
                "geotessera.2020",
                "geotessera.2021",
                "geotessera.2022",
                "geotessera.2023",
                "geotessera.2024",
            ],
        ),
        (
            "overture",
            &[
                "overture.buildings.count",
                "overture.places.count",
                "overture.transportation.road_length_m",
            ],
        ),
    ];
    let alias_for = |k: &str| -> &'static [&'static str] {
        cube_aliases
            .iter()
            .find(|(name, _)| *name == k)
            .map(|(_, v)| *v)
            .unwrap_or(&[])
    };
    let aggregate_subkey_facts = |subkeys: &[&str]| -> (u64, Option<i64>) {
        let mut n_total = 0u64;
        let mut latest: Option<i64> = None;
        for k in subkeys {
            if let Some((n, last)) = counts.get(*k) {
                n_total += *n;
                if let Some(t) = last {
                    latest = Some(latest.map_or(*t, |x| x.max(*t)));
                }
            }
        }
        (n_total, latest)
    };

    let pubkey_b32 = data_encoding::BASE32_NOPAD
        .encode(&s.identity.pubkey.0)
        .to_lowercase();

    // Cube bands first — every entry from bands-v0.json. For family
    // alias slots we surface the wired sub-keys so an agent reading
    // this view is never told a family is offline when its sub-keys
    // are live.
    for b in &registry.bands {
        seen.insert(b.key.clone());
        let (n_self, last_self) = counts.get(&b.key).copied().unwrap_or((0, None));
        let direct_mat = mat_set.contains(b.key.as_str());
        let subkeys = alias_for(b.key.as_str());
        let is_family_alias = !subkeys.is_empty();
        let (n_sub, last_sub) = if is_family_alias {
            aggregate_subkey_facts(subkeys)
        } else {
            (0, None)
        };
        let n = n_self + n_sub;
        let last = match (last_self, last_sub) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (Some(a), None) | (None, Some(a)) => Some(a),
            (None, None) => None,
        };
        let has_mat = direct_mat || is_family_alias && subkeys.iter().any(|k| mat_set.contains(k));
        let sat_lineage: Vec<&str> = materializer_bands
            .iter()
            .find(|t| t.0 == b.key.as_str())
            .map(|t| t.3.to_vec())
            .unwrap_or_default();
        let meta = band_materializer_meta(b.key.as_str());
        let history_from = meta.as_ref().and_then(|m| m.history_from_unix);
        let history_to = meta.as_ref().and_then(|m| m.history_to_unix);
        let mut row = json!({
            "band":             b.key,
            "family":           format!("{:?}", b.family).to_ascii_lowercase(),
            "tempo":            format!("{:?}", b.tempo).to_ascii_lowercase(),
            "tempo_seconds":    b.tempo.slot_seconds(),
            "in_cube":          true,
            "cube_offset":      b.offset,
            "cube_dims":        b.dims,
            "has_materializer": has_mat,
            "facts_count":      n,
            "last_attested_unix_s": last,
            "sat_lineage":      sat_lineage,
            "history_available_from_unix": history_from,
            "history_available_to_unix":   history_to,
            "responder_pubkey_b32": pubkey_b32,
        });
        if is_family_alias {
            row.as_object_mut()
                .unwrap()
                .insert("is_family_alias".into(), JsonValue::Bool(true));
            row.as_object_mut().unwrap().insert(
                "wired_subkeys".into(),
                JsonValue::Array(
                    subkeys
                        .iter()
                        .map(|k| JsonValue::String((*k).into()))
                        .collect(),
                ),
            );
        }
        bands_json.push(row);
    }
    // Auxiliary materializer-only bands (not in the cube).
    for (key, tempo, family, sats) in materializer_bands {
        if seen.contains(*key) {
            continue;
        }
        let (n, last) = counts.get(*key).copied().unwrap_or((0, None));
        let meta = band_materializer_meta(key);
        let tempo_seconds = meta.as_ref().map(|m| m.tempo.slot_seconds()).unwrap_or(0);
        let history_from = meta.as_ref().and_then(|m| m.history_from_unix);
        let history_to = meta.as_ref().and_then(|m| m.history_to_unix);
        bands_json.push(json!({
            "band":             key,
            "family":           family,
            "tempo":            tempo,
            "tempo_seconds":    tempo_seconds,
            "in_cube":          false,
            "has_materializer": true,
            "facts_count":      n,
            "last_attested_unix_s": last,
            "sat_lineage":      sats,
            "history_available_from_unix": history_from,
            "history_available_to_unix":   history_to,
            "responder_pubkey_b32": pubkey_b32,
        }));
    }

    // Roll-ups for the agent's at-a-glance view.
    let total_bands = bands_json.len();
    let total_facts: u64 = counts.values().map(|(n, _)| *n).sum();
    let with_materializer = bands_json
        .iter()
        .filter(|b| {
            b.get("has_materializer")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        })
        .count();
    let with_facts = bands_json
        .iter()
        .filter(|b| b.get("facts_count").and_then(|v| v.as_u64()).unwrap_or(0) > 0)
        .count();

    Json(json!({
        "schema": "emem.coverage_matrix.v1",
        "totals": {
            "bands_declared": total_bands,
            "bands_with_materializer": with_materializer,
            "bands_with_facts": with_facts,
            "total_facts": total_facts,
            "iter_index_limit": limit,
        },
        "responder_pubkey_b32": data_encoding::BASE32_NOPAD
            .encode(&s.identity.pubkey.0).to_lowercase(),
        "bands": bands_json,
        "agent_hint": {
            "use_when": "Decide which band to /v1/recall before paying for materialization. has_materializer=false bands need a third-party signed Attestation; in_cube=false bands ride alongside the 1792-D layout but aren't byte-addressable in the cube.",
            "family_aliases": "Some cube slots (`dem`, `cop_dem`, `climate`, `indices`, `sentinel2_raw`, `geotessera`, `overture`) reserve byte ranges for a *family* of attestable sub-keys; recall calls go to the granular subkey, not the family slot. When `is_family_alias=true`, the row reports rolled-up `facts_count` and `wired_subkeys` so the agent can pick the actual band to materialize.",
            "freshness": "facts_count>0 with a recent last_attested_unix_s means the band has been answered globally somewhere recently; per-cell freshness still needs /v1/temporal_route.",
            "history_bounds": "history_available_from_unix / history_available_to_unix bound what an `emem_backfill` call can materialize on this responder. `null` = the band has no historical materializer here (e.g. weather nowcast, Overture snapshot, or static climatology where one fact answers for all time).",
            "tempo_seconds": "Slot duration in seconds. 0 = static (one fact). Use it to convert between Unix epoch and tslot when planning a backfill window.",
            "federation": "responder_pubkey_b32 is the ed25519 key that signs every fact for this band at this responder. When the directory grows beyond one responder, the same band may be signed by different pubkeys across responders — agents should treat (responder_pubkey, fact_cid) as the unit of trust.",
            "fleet": "/v1/fleet for per-platform sensor lineage (cadence, swath, native_res_m).",
        },
    }))
}

async fn fleet() -> Json<JsonValue> {
    Json(json!({
        "schema": "emem.fleet.v1",
        "platforms": [
            {
                "platform":      "Terra (NASA EOS AM-1)",
                "sensor":        "MODIS",
                "swath_km":      2330,
                "revisit":       "1–2 days global; 16-day composite for indices",
                "native_res_m":  250,
                "tempo":         "medium",
                "bands":         ["modis.ndvi_mean"],
                "wire_path":     "ORNL DAAC TESViS REST → Primary fact",
                "notes":         "Optical multi-spectral; NDVI 16-day composite is the canonical vegetation signal at 250 m. Aqua (PM-1) provides the same MOD13 family with a 90-min phase shift; not yet wired."
            },
            {
                "platform":      "GMRT (Lamont-Doherty multibeam fleet)",
                "sensor":        "Acoustic + DEM fusion",
                "swath_km":      null,
                "revisit":       "static (peer-reviewed updates ~quarterly)",
                "native_res_m":  null,
                "tempo":         "static",
                "bands":         ["gmrt.topobathy_mean"],
                "wire_path":     "GMRT PointServer → Primary fact",
                "notes":         "Single peer-reviewed topo-bathy raster fusing Cop-DEM, GEBCO, multibeam, and high-res sounders. The right band for any-point-on-Earth elevation including ocean."
            },
            {
                "platform":      "Sentinel-1 (Copernicus / ESA TanDEM-X derived Cop-DEM)",
                "sensor":        "Cop-DEM 90 m",
                "swath_km":      null,
                "revisit":       "static",
                "native_res_m":  90,
                "tempo":         "static",
                "bands":         ["copdem30m.elevation_mean"],
                "wire_path":     "Open-Meteo Elevation REST → Primary or Absence fact",
                "notes":         "Land DEM. Returns Fact::Absence over water (Cop-DEM uses 0 m as no-data marker over ocean) so downstream models can distinguish 'sea level over water' from 'land at 0 m'."
            },
            {
                "platform":      "Tessera v1 (Cambridge AAILab + Clay-style)",
                "sensor":        "Multi-source self-supervised composite",
                "swath_km":      null,
                "revisit":       "annual",
                "native_res_m":  10,
                "tempo":         "slow",
                "bands":         ["geotessera"],
                "wire_path":     "dl2.geotessera.org HTTPS range → Primary fact",
                "notes":         "128-D foundation embedding ingested via HTTPS range reads against the public bucket; ~640 B per cell instead of 91 MiB per tile. Native CRS is per-tile UTM."
            },
            {
                "platform":      "GOES-16/17/18 + Himawari-9 + Meteosat-9/11 (geostationary fleet)",
                "sensor":        "ABI / AHI / SEVIRI imagers, blended through HRRR/ICON/GFS/ECMWF NWP",
                "swath_km":      "full disk",
                "revisit":       "10–15 minutes",
                "native_res_m":  500,
                "tempo":         "ultra_fast",
                "bands":         [
                    "weather.temperature_2m",
                    "weather.cloud_cover",
                    "weather.precipitation_mm",
                    "weather.wind_speed_10m"
                ],
                "wire_path":     "Open-Meteo Forecast `current` endpoint → Primary fact",
                "notes":         "Open-Meteo blends NWP runs that ingest the geostationary fleet every ~15 min. We declare the geostationary cadence as the temporal class so the router scores `wave_seasonal` / `advection_linear` against actual orbital physics rather than against the JSON wire format."
            },
            {
                "platform":      "Sentinel-2A/B/C",
                "sensor":        "MSI",
                "swath_km":      290,
                "revisit":       "5 days at equator",
                "native_res_m":  10,
                "tempo":         "fast",
                "bands":         ["indices.ndvi"],
                "wire_path":     "Element84 STAC search → AWS Open Data sentinel-cogs bucket → pure-Rust COG range read (TIFF IFD + Deflate + Predictor 2 decode) → Primary fact",
                "notes":         "Optical multi-spectral. Per-cell call touches ~600 KB across two bands (B04 + B08) instead of the 200 MB native scene; STAC search uses intersects:Point so we always get a tile that *contains* the cell."
            },
            {
                "platform":      "Sentinel-1A/C",
                "sensor":        "C-band SAR (RTC)",
                "swath_km":      250,
                "revisit":       "6-12 days",
                "native_res_m":  10,
                "tempo":         "fast",
                "bands":         ["sentinel1_raw"],
                "wire_path":     "MPC STAC (sentinel-1-rtc) → Azure Blob COG (anonymous SAS) → range read → Primary fact (dB-scaled VV gamma0)",
                "notes":         "All-weather radar; complements Sentinel-2 when cloudy. dB = 10·log₁₀(linear γ0). Source: Microsoft Planetary Computer's Radiometrically Terrain Corrected mirror — proper UTM-projected COG. Anonymous SAS token cached process-wide."
            },
            {
                "platform":      "Overture Maps Foundation (open vector basemap consortium)",
                "sensor":        "Crowdsourced + commercial vector aggregation (OSM, Microsoft, Meta, Esri, TomTom contributions)",
                "swath_km":      "global vector",
                "revisit":       "monthly snapshots",
                "native_res_m":  null,
                "tempo":         "slow",
                "bands":         [
                    "overture.buildings.count",
                    "overture.places.count",
                    "overture.transportation.road_length_m"
                ],
                "wire_path":     "Anonymous S3 (overturemaps-us-west-2) → ListObjectsV2 → parquet 55 async reader (row-group pruning on bbox struct stats) → WKB decode (Point/LineString/Polygon centroid) → Primary fact",
                "notes":         "Vector basemap, not a satellite — but the cadence and routing class match `slow` foundation embeddings (annual-ish cadence + linear AR(1) kernel). Each cell touches one or two row groups across one or two parquet files; per-file footers are cached so adjacent cells are cheap. No API key, no auth header."
            }
        ],
        "agent_hint": {
            "by_cadence": {
                "ultra_fast (<= 1 hour)": ["weather.temperature_2m","weather.cloud_cover","weather.precipitation_mm","weather.wind_speed_10m"],
                "fast (~5–12 days)":      ["indices.ndvi","sentinel1_raw","s2.B04","s2.B08"],
                "medium (~16–30 days)":   ["modis.ndvi_mean"],
                "slow (annual or monthly)": ["geotessera","geotessera.multi_year","overture.buildings.count","overture.places.count","overture.transportation.road_length_m"],
                "static":                 ["copdem30m.elevation_mean","gmrt.topobathy_mean"]
            },
            "by_capability": {
                "all_weather":   ["sentinel1_raw"],
                "night":         ["sentinel1_raw"],
                "vegetation":    ["modis.ndvi_mean","indices.ndvi","geotessera"],
                "weather":       ["weather.temperature_2m","weather.cloud_cover","weather.precipitation_mm","weather.wind_speed_10m"],
                "elevation":     ["gmrt.topobathy_mean","copdem30m.elevation_mean"],
                "embedding":     ["geotessera","geotessera.multi_year"],
                "human_geography": ["overture.buildings.count","overture.places.count","overture.transportation.road_length_m"]
            },
            "router":            "POST /v1/temporal_route to score these bands against your query time and intent.",
            "live_observability": "GET /v1/coverage_matrix for live per-band facts_count, has_materializer, last_attested_at."
        }
    }))
}

async fn functions() -> Json<JsonValue> {
    Json(serde_json::to_value(&*emem_core::functions::DEFAULT).unwrap_or(json!({})))
}

async fn sources() -> Json<JsonValue> {
    Json(serde_json::to_value(&*emem_core::sources::DEFAULT).unwrap_or(json!({})))
}

/// `GET /v1/algorithms` — content-addressed algorithm registry.
///
/// Composition recipes that fuse already-attested band facts (and
/// embeddings) into derived scores, classifications, and similarity
/// metrics. Distinct from `/v1/functions` (which derives a single band
/// value from raw upstream sources) and from `/v1/bands` (the data slot
/// itself). Receipts that quote a derived score should cite
/// `algorithm_cid` alongside the input `fact_cids` so a downstream
/// verifier can replay the same composition.
///
/// Three kinds: `solo` (one band → derived), `combined` (multi-band
/// composite), `embedding` (operations on the geotessera vector).
/// Process-wide cached algorithms_cid — the registry is content-addressed
/// and immutable for the life of the process, so we hash it once and
/// reuse the string everywhere it's surfaced (/health, /v1/manifests,
/// /v1/discover, /v1/agent_card, /v1/algorithms, /v1/intent, /.well-known).
static ALGORITHMS_CID: LazyLock<Option<String>> =
    LazyLock::new(|| emem_core::manifest::manifest_cid(&*emem_core::algorithms::DEFAULT).ok());

async fn algorithms() -> Json<JsonValue> {
    let reg = &*emem_core::algorithms::DEFAULT;
    let cid = ALGORITHMS_CID.clone();
    Json(json!({
        "manifest": reg.manifest,
        "version":  reg.version,
        "algorithms_cid": cid,
        "_note": reg.note,
        "agent_hint": {
            "what_this_is": "A content-addressed dictionary of recipes that combine attested band facts into composite answers (e.g. flood_risk@1, water_consensus@1, embedding_novelty@1). Receipts citing one of these names + the algorithm_cid let any other operator replay the same composition deterministically.",
            "how_to_use":    "1) Pick the entry whose `when_to_use` matches the agent's query. 2) Read its `inputs[].band` list and assemble a single `/v1/recall` (or `/v1/recall_many`) body. 3) Apply the `formula` in-process. 4) Cite the algorithm key + algorithms_cid alongside the fact_cids in the agent's reply.",
            "kinds": {
                "solo":      "single band → derived classification or scalar",
                "combined":  "multi-band weighted composite",
                "embedding": "cosine / novelty / change over geotessera vectors",
            },
        },
        "algorithms": reg.algorithms,
    }))
}

/// `GET /v1/algorithms/:key` — one algorithm entry by key (URL-encode
/// the `@` in keys like `flood_risk@1` if your client cares — most
/// don't). Returns 404 if the key is unknown.
async fn algorithm_detail(
    axum::extract::Path(key): axum::extract::Path<String>,
) -> Result<Json<JsonValue>, ApiError> {
    let reg = &*emem_core::algorithms::DEFAULT;
    match reg.lookup(&key) {
        Some(a) => Ok(Json(serde_json::to_value(a).unwrap_or(json!({})))),
        None => {
            let known: Vec<&str> = reg.algorithms.iter().map(|a| a.key.as_str()).collect();
            Err(ApiError(
                StatusCode::NOT_FOUND,
                ErrorBody {
                    code: emem_core::error::ErrorCode::CidNotFound,
                    message: format!(
                        "no algorithm with key {key:?}; see /v1/algorithms — known keys: {}",
                        known.join(", "),
                    ),
                },
            ))
        }
    }
}

/// `GET /v1/errors` — error catalogue with action hints. Just listing the
/// codes (which is what we did before) doesn't tell an agent what to DO
/// when it hits one. Each entry now has a `recover` field naming the
/// concrete next step.
async fn errors() -> Json<JsonValue> {
    let entries: &[(&str, &str, &str)] = &[
        ("invalid_cell",                 "cell64 string did not parse",
         "Re-encode (lat,lng) via POST /v1/locate, or check the bigram boundaries (must be exactly four base-1024 bigrams joined by '.')."),
        ("invalid_resolution",           "resolution bits out of range",
         "Use the cell64 returned by /v1/locate; that's always at the active resolution. See /v1/grid_info."),
        ("tslot_mismatch",               "tslot in request doesn't match band's expected tempo",
         "Check /v1/bands for the band's `tempo` field and round your tslot to that grid."),
        ("band_not_in_registry",         "band key isn't in the active manifest",
         "GET /v1/bands for the canonical list. Common typos: missing schema namespace (need `copdem30m.elevation_mean`, not `elevation_mean`)."),
        ("function_not_in_registry",     "derivation fn_key not registered",
         "Functions ship via /v1/functions. To use a private function, register it in your registry_cid before attesting."),
        ("source_scheme_unknown",        "source.scheme not in /v1/sources",
         "GET /v1/sources for the active connector list. Self-hosters can add via the source manifest."),
        ("cid_not_found",                "fact CID exists but no matching object on disk",
         "The fact may have been pruned, or the CID is from a different responder. Verify `responder_pubkey_b32` matches between the receipt and /health."),
        ("registry_cid_unknown",         "the bound registry_cid isn't recognised",
         "Receipts pin manifest CIDs at issue time; if the responder rotated manifests, check /v1/manifests for the current `registry_cid`."),
        ("schema_cid_unknown",           "fact's schema_cid isn't loaded",
         "GET /v1/manifests for the current `schema_cid`. Re-attest with the active schema, or pin the older manifest if the responder still has it."),
        ("privacy_refused",              "band's privacy_class blocks this query at the requested resolution",
         "Drop the resolution (snap to a coarser cell) or request via an authorised attester. Check /v1/bands for `privacy_class`."),
        ("level_too_low",                "operation requires a higher protocol level (L1/L2)",
         "L0/L1 are anonymous; L2 (write/challenge) requires an attester key. Generate ed25519, post to /v1/attest_cbor."),
        ("attester_revoked",             "the attester pubkey is in the revocation set",
         "Check /v1/contributors for `revoked` flag. Submit a fresh keypair if revoked legitimately, or contest via /v1/challenge."),
        ("unauthorized",                 "missing or invalid attester signature on a write",
         "Re-sign with the same keypair that issued the registered attester pubkey; signature must be over blake3(batch_root || registry_cid || schema_cid)."),
        ("claim_undecidable",            "verify input doesn't yield true/false (e.g. NaN, type mismatch)",
         "Inspect the fact via /v1/recall — the value may be missing, the wrong type, or NaN. Tighten your claim or pre-filter before /v1/verify."),
        ("bad_signature",                "ed25519 signature failed to verify",
         "Re-sign with the correct preimage. /v1/verify_receipt's response includes `preimage_blake3_hex` so you can debug locally."),
        ("bad_merkle_proof",             "Merkle inclusion proof is malformed or doesn't reach the batch_root",
         "Re-build the proof from the canonical-sorted leaf list; verify `leaf_index` is correct."),
        ("canonical_encoding_divergence","CBOR you sent isn't deterministic per RFC 8949 §4.2.1",
         "Use a CBOR library with `canonical=true`; serde-derived structs are usually fine, freeform maps must have sorted keys."),
        ("source_fetch_failed",          "upstream open-data provider returned non-2xx",
         "Retry with backoff; check /v1/sources to confirm the URL template still resolves. May indicate provider-side outage."),
        ("source_format_mismatch",       "fetched bytes don't match the declared scheme (e.g. GeoTIFF magic missing)",
         "Re-fetch with explicit content-type. Could indicate provider migrated to a new format; consult their docs."),
        ("compute_timeout",              "derivation function exceeded EMEM_TIMEOUT_SECS",
         "Submit smaller inputs, or run the computation client-side and attest the derivative directly."),
        ("compute_quota_exceeded",       "function call hit per-attester quota",
         "Throttle, or request quota increase via /v1/contributors leaderboard (high-score attesters get larger quotas)."),
        ("rate_limited",                 "per-IP rate limit hit",
         "Backoff per the `Retry-After` header (default 60 s). Operators tune via EMEM_RATE_LIMIT_RPS."),
        ("cache_error",                  "responder's hot cache (sled) had an internal error",
         "Retry; if persistent, the responder's storage may need recovery — operators see the cause in the journald log."),
        ("internal",                     "responder-side bug",
         "Capture the response and the request that produced it; file at https://github.com/Vortx-AI/emem/issues."),
    ];
    let codes: Vec<JsonValue> = entries
        .iter()
        .map(|(c, m, r)| {
            json!({
                "code": c, "meaning": m, "recover": r,
            })
        })
        .collect();
    Json(json!({
        "schema": "emem.errors.v1",
        "codes": codes,
        "next": [
            "GET /v1/manifests  — current registry/schema/bands/sources CIDs",
            "GET /v1/bands      — band catalogue with tempo + privacy_class",
            "POST /v1/verify_receipt — debug a bad_signature with `preimage_blake3_hex`"
        ],
    }))
}

async fn tools() -> Json<JsonValue> {
    let descriptors: Vec<JsonValue> = emem_mcp::TOOLS.iter().map(|t| json!({
        "name": t.name,
        "title": t.title,
        "description": t.description,
        "when_to_use": t.when_to_use,
        "input_schema": serde_json::from_str::<JsonValue>(t.input_schema).unwrap_or(json!({})),
        "example_args": serde_json::from_str::<JsonValue>(t.example_args).unwrap_or(json!({})),
        "level": t.level,
        "category": t.category,
        "annotations": {
            "title":           t.title,
            "readOnlyHint":    t.read_only_hint,
            "destructiveHint": t.destructive_hint,
            "idempotentHint":  t.idempotent_hint,
            "openWorldHint":   t.open_world_hint,
        },
    })).collect();
    Json(json!({ "tools": descriptors }))
}

async fn agent_card(State(s): State<AppState>) -> Json<JsonValue> {
    let descriptors: Vec<JsonValue> = emem_mcp::TOOLS.iter().map(|t| json!({
        "name": t.name,
        "title": t.title,
        "description": t.description,
        "when_to_use": t.when_to_use,
        "level": t.level,
        "category": t.category,
        "input_schema": serde_json::from_str::<JsonValue>(t.input_schema).unwrap_or(json!({})),
        "example_args": serde_json::from_str::<JsonValue>(t.example_args).unwrap_or(json!({})),
        "annotations": {
            "title":           t.title,
            "readOnlyHint":    t.read_only_hint,
            "destructiveHint": t.destructive_hint,
            "idempotentHint":  t.idempotent_hint,
            "openWorldHint":   t.open_world_hint,
        },
    })).collect();
    Json(json!({
        "name": "emem",
        "version": env!("CARGO_PKG_VERSION"),
        "purpose": "Agent-native, content-addressed memory of every place on Earth. Cite-able answers about places, signed receipts, token-economical addressing.",
        "trigger_phrases": [
            "what is at <place>",
            "tell me about this cell",
            "compare X and Y",
            "find places like X",
            "did <band> change between t1 and t2",
            "is <claim> true at <place>",
            "average <band> over this region"
        ],
        "primary_tools": ["emem_recall", "emem_compare", "emem_find_similar", "emem_diff", "emem_verify"],
        // Order matters: bands → materializers → coverage_matrix builds
        // the agent's mental model in the right order. Bands declare
        // what *can* exist; materializers declare what auto-fetches on
        // recall miss (so the agent knows it can recall any cell on
        // Earth without seeding); coverage_matrix declares what's
        // already cached + freshness. Manifests pin the CIDs for
        // citation. Without `emem_materializers` here, agents miss the
        // auto-materialize contract and assume empty recall = no data.
        "discover_first": ["emem_bands", "emem_materializers", "emem_algorithms", "emem_coverage_matrix", "emem_manifests"],
        "authentication": "none for L0/L1; ed25519 attester for L2 writes",
        // Required by the Anthropic Software Directory submission flow:
        // every connector must expose a privacy URL, a terms URL, and a
        // support contact. These point to the markdown docs served from
        // /privacy and /terms; vendor + contact email are explicit so the
        // reviewer can reach the maintainer without scraping the repo.
        "vendor": "Vortx-AI",
        "contact_email": "avijeet@vortx.ai",
        "privacy_url": "https://emem.dev/privacy",
        "terms_url": "https://emem.dev/terms",
        "support_url": "https://github.com/Vortx-AI/emem/issues",
        "documentation_url": "https://github.com/Vortx-AI/emem#readme",
        "license": "Apache-2.0",
        "license_url": "https://github.com/Vortx-AI/emem/blob/main/LICENSE",
        "responder": {
            "pubkey_b32": data_encoding::BASE32_NOPAD.encode(&s.identity.pubkey.0).to_lowercase(),
            "key_epoch": s.identity.epoch.0,
        },
        "manifests": {
            "bands_cid": &s.manifests.bands_cid,
            "functions_cid": s.manifests.registry_cid.as_str(),
            "sources_cid": &s.manifests.sources_cid,
            "schema_cid": s.manifests.schema_cid.as_str(),
            "algorithms_cid": ALGORITHMS_CID.clone(),
        },
        "surfaces": {
            "rest_openapi":     "/openapi.json",
            "mcp_jsonrpc":      "/mcp",
            "well_known":       "/.well-known/emem.json",
            "ai_plugin":        "/.well-known/ai-plugin.json",
            "agents_md":        "/agents.md",
            "whitepaper_md":    "/whitepaper.md",
            "llms_txt":         "/llms.txt",
            "privacy":          "/privacy",
            "terms":            "/terms",
            "fleet":            "/v1/fleet",
            "coverage_matrix":  "/v1/coverage_matrix",
            "materializers":    "/v1/materializers",
            "data_availability":"/v1/data_availability",
            "algorithms":       "/v1/algorithms",
            "temporal_route":   "/v1/temporal_route",
            "reviews":          "/v1/reviews",
        },
        // Honest declaration of which non-JSON surfaces exist and which
        // protocol channels actually expose them. The infrastructure
        // pre-dates wide MCP image-block support, so today: SVG/GeoJSON
        // are reachable via REST but get text-wrapped by MCP. Documented
        // here so agents discover the gap rather than assuming emem
        // doesn't ship multimodal at all.
        "multimodal_surfaces": {
            "_status_summary": "REST DELIVERS images (image/svg+xml + image/png) and geo+json today. MCP delivers the coverage map as a native EmbeddedResource via `emem_coverage_map`, per-cell true-colour Sentinel-2 thumbnails as native ImageContent via `emem_cell_scene_rgb`, and cell polygons as native EmbeddedResource (geo+json) via `emem_cell_geojson`. INGESTION is JSON/CBOR only — no image upload or binary-vector-as-payload endpoint yet.",
            "deliver": {
                "coverage_map_svg": {
                    "url": "/v1/coverage_map.svg",
                    "mime": "image/svg+xml",
                    "wired_via": "REST + MCP",
                    "mcp_tool": "emem_coverage_map",
                    "mcp_content_block": "EmbeddedResource (text resource, mimeType image/svg+xml) + a text summary block",
                    "agent_use":  "Multimodal MCP agents call `emem_coverage_map` and receive a native EmbeddedResource block. Plain REST callers GET /v1/coverage_map.svg and receive the bare 1440×720 Plate-Carrée SVG. Both paths share the same renderer."
                },
                "cell_geojson": {
                    "url_template": "/v1/cells/{cell64}/geojson",
                    "mime": "application/geo+json",
                    "wired_via": "REST + MCP",
                    "mcp_tool": "emem_cell_geojson",
                    "mcp_content_block": "EmbeddedResource (text resource, mimeType application/geo+json) + a text summary block",
                    "agent_use":  "Polygon hexagon + bbox/lat/lng/neighbours. Drop straight into Mapbox/Leaflet/Deck.gl/QGIS without a GIS pipeline."
                },
                "cell_scene_rgb": {
                    "url_template": "/v1/cells/{cell64}/scene.png?max_cloud=20",
                    "mime": "image/png",
                    "wired_via": "REST + MCP",
                    "mcp_tool": "emem_cell_scene_rgb",
                    "mcp_content_block": "ImageContent (base64 PNG, mimeType image/png) + a text summary block with the STAC item id / capture time / stretch values",
                    "agent_use":  "True-colour Sentinel-2 L2A thumbnail centred on the cell, 256×256 px (~2.56 km × ~2.56 km at S2's 10 m native). Auto-picks the latest scene with `eo:cloud_cover < max_cloud` (default 20 %, override via query/arg). Pure-Rust pipeline — STAC search + HTTPS-Range COG reads + 2-98 percentile stretch + PNG encode. The STAC item id is emitted in `x-emem-scene-item-id` header (REST) and structuredContent (MCP) so the receipt is reproducible."
                },
                "cell_recall_geojson": {
                    "url_template": "/v1/cells/{cell64}/recall.geojson?bands={bands}",
                    "mime": "application/geo+json",
                    "wired_via": "REST",
                    "mcp_wrapped_as": "text",
                    "agent_use":  "Polygon properties carry every fact value the responder has at the cell — one call to render a styled choropleth."
                },
                "embedding_vectors": {
                    "transport": "JSON arrays inside Fact.value",
                    "wired_via": "REST + MCP",
                    "agent_use":  "Geotessera 128-D and multi_year 1024-D vectors travel as JSON-encoded float arrays inside the standard recall envelope. There is no separate binary fp16 / Arrow / msgpack vector channel today."
                }
            },
            "ingest": {
                "_state": "no image / vector ingestion path is wired today",
                "attestations": {
                    "url": "/v1/attest_cbor",
                    "mime": "application/cbor",
                    "carries": "PrimaryFact / DerivativeFact / Absence as canonical CBOR — structured fact data, NOT image bytes. Submit ed25519-signed batches to grow the corpus."
                },
                "image_input_for_similarity": {
                    "status": "not_wired",
                    "note":   "There is no /v1/find_similar_image (or equivalent multipart upload) for 'find places that look like this satellite preview'. Workaround today: have the agent map the image to a known cell64 (e.g. by user-supplied location), then /v1/find_similar over `geotessera`."
                },
                "vector_payload_input": {
                    "status": "supported_via_inline_literal",
                    "note":   "/v1/find_similar accepts `key: \"inline:[v0,v1,...]\"` — a JSON-array literal. There is no binary fp16 ingestion path."
                }
            },
            "what_to_build_next": [
                "MCP `emem_coverage_map` tool that returns the SVG as a `resource` content block with mime + text — so multimodal MCP agents see the image natively.",
                "Server-side SVG → PNG raster (resvg crate) so Anthropic / OpenAI image-input clients can ingest the coverage map.",
                "/v1/find_similar_image — multipart upload → geotessera lookup via a small image encoder (Tessera v1 has a published encoder).",
                "Binary vector channel: application/octet-stream fp16 for k-NN payloads (saves ~6× over JSON arrays for the 1024-D multi_year vector)."
            ]
        },
        // Curated signposts grouped by use-case so agents skimming the
        // card can see at a glance which family of question maps to
        // which band(s). Only WIRED materializers appear here — empty
        // groups are omitted rather than stubbed. For the canonical
        // per-band roster (with upstream URLs and absence semantics),
        // call `emem_materializers`; for live counts and freshness
        // call `emem_coverage_matrix`.
        "live_bands": {
            "vegetation_fast":      ["indices.ndvi","indices.evi","indices.savi","indices.ndmi"],
            "vegetation_medium":    ["modis.ndvi_mean"],
            "water_history_long":   ["surface_water.recurrence"],
            "water_event_window":   ["indices.ndwi","indices.mndwi"],
            "fire_burn_scar":       ["indices.nbr"],
            "soil_bare":            ["indices.bsi"],
            "built_up":             ["indices.ndbi","overture.buildings.count","overture.places.count","overture.transportation.road_length_m"],
            "weather_15min":        ["weather.temperature_2m","weather.cloud_cover","weather.precipitation_mm","weather.wind_speed_10m"],
            "elevation_global":     ["gmrt.topobathy_mean"],
            "elevation_land_only":  ["copdem30m.elevation_mean"],
            "optical_raw":          ["s2.B02","s2.B03","s2.B04","s2.B08","s2.B11","s2.B12"],
            "scene_classification": ["s2.scl"],
            "foundation_embedding": ["geotessera","geotessera.multi_year"],
        },
        "live_bands_discovery_hint": "live_bands is a curated subset; call `emem_materializers` for the full registry with upstream URLs, licenses, and value-kind (primary | absence | primary_or_absence). Call `emem_coverage_matrix` for per-band facts_count and last-attested timestamps.",
        "runtime": {
            "language":       "Rust",
            "no_python_at_request_path": true,
            "cog_reader":     "pure-Rust HTTPS-range TIFF/IFD parser + Deflate + Predictor 2 (no GDAL, no rasterio)",
            "weather_source": "MET Norway api.met.no (no API key, no per-IP rate limit)",
            "stac_search":    "Element84 earth-search (anonymous; AWS Open Data backed)",
        },
        "tools": descriptors,
    }))
}

async fn quickstart() -> Json<JsonValue> {
    Json(json!({
        "schema": "emem.quickstart.v1",
        "purpose": "Six-call flow proving agent integration: discover → fleet/coverage → locate → recall (multi-band) → audit → contribute.",
        "steps": [
            {
                "n": 1,
                "name": "Discover the surface",
                "method": "GET",
                "path": "/v1/discover",
                "why": "One call returns the agent_card, manifest CIDs, primary tools, canonical_places list, and next_call hints. Cheaper than reading the spec."
            },
            {
                "n": 2,
                "name": "Pick a band",
                "method": "GET",
                "path": "/v1/coverage_matrix",
                "why": "Per-band live status: which materializers are wired, how many facts are cached, when the band was last attested. Combine with /v1/fleet to map bands → satellites and /v1/temporal_route for cadence-aware ranking."
            },
            {
                "n": 3,
                "name": "Locate a place",
                "method": "POST",
                "path": "/v1/locate",
                "body": { "place": "Mount Fuji" },
                "expect": "200 with { cell64, neighborhood_cells (9), polygon_bbox?, polygon_sample_cells?, agent_hint, advice, via }. `via` declares whether the centroid came from the embedded gazetteer, the cache, or live Nominatim."
            },
            {
                "n": 4,
                "name": "Recall — fan out across bands and the neighborhood",
                "method": "POST",
                "path": "/v1/recall_many",
                "body": { "cells": ["damO.zb000.xUti.zde79","damO.zb000.xUti.zde78"], "bands": ["indices.ndvi","weather.temperature_2m","copdem30m.elevation_mean","geotessera"] },
                "expect": "200 with { by_cell: {<cell>: {facts, receipt, bands_available?}} }. NDVI from Sentinel-2 L2A via pure-Rust COG range read; weather from MET Norway (no API key); foundation embedding from Tessera v1; elevation from Cop-DEM."
            },
            {
                "n": 5,
                "name": "Audit one of the receipts offline",
                "method": "POST",
                "path": "/v1/verify_receipt",
                "body": { "receipt": "<paste any per-cell receipt object from step 4>" },
                "expect": "200 with { valid: true, signer_pubkey_b32, preimage_blake3_hex }. Anyone with the responder pubkey can recompute the preimage; this is what makes the answer cite-able."
            },
            {
                "n": 6,
                "name": "Contribute (optional)",
                "method": "POST",
                "path": "/v1/attest_cbor",
                "body": "<canonical CBOR Attestation>",
                "expect": "200 on accept. Generate ed25519, build canonical CBOR over PrimaryFact(s), Merkle-root the leaves, sign blake3(batch_root||registry_cid||schema_cid). See crates/emem-cli/src/bin/emem-realdemo.rs for a full Rust example. Increases your score on /v1/contributors."
            }
        ],
        "example_chat_to_paste_into_a_system_prompt":
            "When the user asks about a place (lat/lng, region name, or cell64), call emem.recall first via the cell64 returned by emem.locate's neighborhood_cells. Cite the response receipt's fact_cids (truncated cid64 form) in your reply. If the user asks 'what changed', use emem.diff. If 'find similar', use emem.find_similar.",
        "see_also": {
            "first_person_trial":   "/agent-trial.md",
            "agent_walkthroughs":   "/examples/agent-walkthroughs.md",
            "errors_with_recovery": "/v1/errors",
            "grid_resolution":      "/v1/grid_info",
            "coverage_map_visual":  "/v1/coverage_map.svg"
        }
    }))
}

// ── Primitive routes ─────────────────────────────────────────────────────

/// Recall + lazy materialization. If the cache miss and the agent asked
/// for specific bands, fall through to the registered upstream connector,
/// sign the resulting fact under the responder identity, persist it, and
/// re-query.
///
/// Used by both `POST /v1/recall` and `POST /v1/recall_polygon` so the
/// fan-out path picks up the same auto-materialize behaviour as the
/// single-cell path. Without this shared helper, recall_polygon would
/// return zero facts at any cell that hasn't been seeded yet — defeating
/// the whole point of the polygon fan-out (which exists to fix
/// place-name drift, not to expose a different read shape).
async fn recall_with_auto_materialize(
    req: &RecallReq,
    s: &AppState,
) -> Result<(RecallResp, Vec<JsonValue>), ApiError> {
    use std::collections::HashSet;
    let mut resp = recall(req, s).await?;
    let mut materialize_notes: Vec<JsonValue> = Vec::new();

    // Per-band, on-demand materialization. The corpus grows as agents
    // demand: every (cell, band) the agent reaches for that we know how
    // to fetch is materialized, signed under the responder identity,
    // persisted, and then returned in the same response.
    //
    // Three cases drive what we try to materialize:
    //
    // (1) Request specifies bands: every band missing from the response
    //     is candidate. Lets an agent ask for [elevation, NDVI] and get
    //     both, even if NDVI was never seen at this cell before.
    // (2) Request specifies no bands AND cell is empty: materialize a
    //     small cheap default set (elevation + current temperature) so
    //     a bare `recall {"cell":"…"}` surfaces *something* instead of
    //     an empty array.
    // (3) Request specifies no bands AND cell has facts: leave alone.
    //     The agent didn't ask for anything new; don't pay an upstream
    //     fetch on every recall.
    let owned_default: Vec<String>;
    let candidates: &[String] = match req.bands.as_ref() {
        Some(req_bands) if !req_bands.is_empty() => {
            let present: HashSet<&str> = resp
                .facts
                .iter()
                .filter_map(|f| match f {
                    emem_fact::Fact::Primary(p) => Some(p.band.as_str()),
                    _ => None,
                })
                .collect();
            owned_default = req_bands
                .iter()
                .filter(|b| !present.contains(b.as_str()))
                .cloned()
                .collect();
            owned_default.as_slice()
        }
        _ if resp.facts.is_empty() => {
            owned_default = vec![
                "copdem30m.elevation_mean".into(),
                "weather.temperature_2m".into(),
            ];
            owned_default.as_slice()
        }
        _ => &[],
    };

    if !candidates.is_empty() {
        let outcomes = try_materialize_bands(&req.cell, candidates, s).await;
        let materialized_any = outcomes.iter().any(|o| o.fact_cid.is_some());
        for o in &outcomes {
            if let Some(reason) = &o.skip_reason {
                materialize_notes.push(json!({
                    "band":   o.band,
                    "status": "skipped",
                    "reason": reason,
                }));
            } else if o.fact_cid.is_some() {
                materialize_notes.push(json!({
                    "band":   o.band,
                    "status": "materialized",
                    "fact_cid": o.fact_cid.as_deref(),
                }));
            }
        }
        if materialized_any {
            resp = recall(req, s).await?;
        }
    }
    Ok((resp, materialize_notes))
}

async fn get_cell(
    State(s): State<AppState>,
    Path(cell64): Path<String>,
) -> Result<Json<JsonValue>, ApiError> {
    let req = RecallReq {
        cell: cell64,
        bands: None,
        tslot: None,
    };
    let resp = recall(&req, &s).await?;
    Ok(Json(serde_json::to_value(resp).unwrap_or(json!({}))))
}

/// REST-side wrapper for `RecallReq` that accepts a singular `band` field
/// in addition to `bands`. Agents tend to write `{"band": "modis.ndvi_mean"}`
/// — the spec defines the field as `bands: [...]` but a singular alias
/// is the most common first-call shape, so silently rejecting it (recall
/// then matches against an empty filter) is the worst possible UX. We
/// normalise to `bands: [band]` here so the auto-materializer downstream
/// sees the requested band.
#[derive(Deserialize)]
struct RecallApiReq {
    #[serde(alias = "cell64")]
    cell: String,
    #[serde(default)]
    band: Option<String>,
    #[serde(default)]
    bands: Option<Vec<String>>,
    #[serde(default)]
    tslot: Option<u64>,
}

impl From<RecallApiReq> for RecallReq {
    fn from(api: RecallApiReq) -> Self {
        // If both `band` and `bands` are supplied we merge — the singular
        // is treated as one more entry on the plural list rather than
        // silently winning.
        let bands = match (api.band, api.bands) {
            (None, None) => None,
            (Some(b), None) => Some(vec![b]),
            (None, Some(v)) => Some(v),
            (Some(b), Some(mut v)) => {
                if !v.iter().any(|x| x == &b) {
                    v.insert(0, b);
                }
                Some(v)
            }
        };
        RecallReq {
            cell: api.cell,
            bands,
            tslot: api.tslot,
        }
    }
}

async fn post_recall(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(api_req): Json<RecallApiReq>,
) -> Result<Response, ApiError> {
    metrics_inc(&RECALL_TOTAL);
    let mut req: RecallReq = api_req.into();
    // Accept place names: `recall {"cell":"Mount Everest"}` is just
    // `locate` + `recall` from the agent's POV, no reason to make
    // them do two round-trips.
    let (cell, resolved) = resolve_cell_field(&req.cell).await?;
    req.cell = cell;
    let (resp, materialize_notes) = recall_with_auto_materialize(&req, &s).await?;

    // ETag derivation: blake3 of the sorted fact_cids list. Facts are
    // content-addressed and immutable, so the same recall on the same cell
    // with the same band/tslot filter returns the same ETag bit-exactly.
    // The receipt's `served_at` differs each call, but the cite-able
    // payload (the facts) doesn't — agents repeating a recall during a
    // session can short-circuit on 304 Not Modified.
    let mut cids: Vec<String> = resp.receipt.fact_cids.iter().map(|c| c.0.clone()).collect();
    cids.sort();
    let mut hasher = blake3::Hasher::new();
    for c in &cids {
        hasher.update(c.as_bytes());
        hasher.update(b"\n");
    }
    let etag = format!("\"{}\"", &hasher.finalize().to_hex().to_string()[..16]);

    if let Some(inm) = headers.get(IF_NONE_MATCH).and_then(|v| v.to_str().ok()) {
        if inm.split(',').map(|s| s.trim()).any(|s| s == etag) {
            return Ok(Response::builder()
                .status(StatusCode::NOT_MODIFIED)
                .header(ETAG, &etag)
                .body(axum::body::Body::empty())
                .unwrap_or_else(|_| StatusCode::NOT_MODIFIED.into_response()));
        }
    }

    // Attach materialize_notes to the body when any band was attempted
    // and skipped. Cheap to skip when the array is empty (no key emitted).
    let resolved_env = resolved_envelope(vec![("cell".into(), resolved)]);
    let body = {
        let mut v = serde_json::to_value(&resp).unwrap_or(json!({}));
        if let Some(map) = v.as_object_mut() {
            if !materialize_notes.is_empty() {
                map.insert(
                    "materialize_notes".into(),
                    JsonValue::Array(materialize_notes),
                );
            }
            if let Some(env) = resolved_env {
                map.insert("resolved_from".into(), env);
            }
        }
        serde_json::to_vec(&v).unwrap_or_else(|_| b"{}".to_vec())
    };
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "application/json; charset=utf-8")
        .header(ETAG, &etag)
        // 60s — short, because new facts may land on this cell from any
        // other agent and the immutable cite-ability is per-fact, not
        // per-recall. Long enough to dedupe within an agent's reasoning
        // loop, short enough that contributions become visible quickly.
        .header(CACHE_CONTROL, "private, max-age=60")
        .body(axum::body::Body::from(body))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()))
}

#[derive(Deserialize)]
struct RecallManyReq {
    /// Cells to fan out across (alias `cell64s` accepted).
    #[serde(alias = "cell64s")]
    cells: Vec<String>,
    /// Optional band filter, applied uniformly to every cell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bands: Option<Vec<String>>,
    /// Optional tslot, applied uniformly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tslot: Option<u64>,
}

/// `POST /v1/recall_many` — bulk recall in one round trip. The
/// `polygon_sample_cells` field on `/v1/locate` returns up to 64 cells,
/// so a Grand-Canyon agent doing fan-out used to make 64 round trips.
/// This collapses them to one. The response is a per-cell map keyed by
/// cell64; each entry has the same shape as `/v1/recall`.
async fn post_recall_many(
    State(s): State<AppState>,
    Json(mut req): Json<RecallManyReq>,
) -> Result<Json<JsonValue>, ApiError> {
    if req.cells.is_empty() {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            ErrorBody {
                code: ErrorCode::Internal,
                message: "recall_many: `cells` cannot be empty".into(),
            },
        ));
    }
    if req.cells.len() > 256 {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            ErrorBody {
                code: ErrorCode::Internal,
                message: format!(
                    "recall_many: max 256 cells per call (got {}); split client-side",
                    req.cells.len()
                ),
            },
        ));
    }
    // Resolve any place names in the cells array. The result-by-cell
    // map is keyed by the resolved cell64 so a downstream agent can
    // dedupe across heterogeneous inputs ("Tokyo" + "damO.zb000.xUto.sisA"
    // resolve to the same cell and merge into one entry).
    let mut resolved_inputs: Vec<(String, String)> = Vec::with_capacity(req.cells.len()); // (input, cell64)
    for raw in &req.cells {
        let (cell64, _) = resolve_cell_field(raw).await?;
        resolved_inputs.push((raw.clone(), cell64));
    }
    req.cells = resolved_inputs.iter().map(|(_, c)| c.clone()).collect();
    metrics_inc(&RECALL_TOTAL);
    let mut by_cell = serde_json::Map::with_capacity(req.cells.len());
    let mut total_facts = 0usize;
    for cell in &req.cells {
        let r = RecallReq {
            cell: cell.clone(),
            bands: req.bands.clone(),
            tslot: req.tslot,
        };
        match recall(&r, &s).await {
            Ok(resp) => {
                total_facts += resp.facts.len();
                by_cell.insert(
                    cell.clone(),
                    serde_json::to_value(&resp).unwrap_or(json!({})),
                );
            }
            Err(e) => {
                by_cell.insert(
                    cell.clone(),
                    json!({
                        "error": e.to_string(),
                        "code": format!("{:?}", e.wire_code()),
                    }),
                );
            }
        }
    }
    let resolved_map: serde_json::Map<String, JsonValue> = resolved_inputs
        .iter()
        .filter(|(input, c)| input != c)
        .map(|(input, c)| (input.clone(), JsonValue::String(c.clone())))
        .collect();
    let mut out = json!({
        "schema": "emem.recall_many.v1",
        "cells_requested": req.cells.len(),
        "facts_returned": total_facts,
        "by_cell": JsonValue::Object(by_cell),
        "note": "Each cell carries its own signed receipt under by_cell.<cell>.receipt. There is no aggregate receipt — verifying any one cell verifies that cell only. To audit the bulk call, verify each cell's receipt independently via /v1/verify_receipt.",
    });
    if !resolved_map.is_empty() {
        if let Some(map) = out.as_object_mut() {
            map.insert("resolved_from".into(), JsonValue::Object(resolved_map));
        }
    }
    Ok(Json(out))
}

/// `POST /v1/recall_polygon` — single call from "place name" or
/// "polygon_bbox" to a fully merged set of facts. Solves the
/// place-name-drift problem at the API level: the agent doesn't have to
/// `/v1/locate` first, read `polygon_sample_cells`, then `/v1/recall_many`
/// — it just hands us a name (or a bbox) plus the bands it wants. We
/// resolve the polygon (via embedded gazetteer → cache → Nominatim,
/// same path as `/v1/locate`), sample up to `max_cells` cells inside
/// the bbox, fan out, and return the per-cell facts plus a flat
/// `merged_facts` array convenient for region-level reasoning.
#[derive(Debug, Clone, Deserialize)]
struct RecallPolygonReq {
    /// Free-text place name (resolved through the same layered geocoder
    /// as `/v1/locate`). One of `place` or `polygon_bbox` is required.
    #[serde(default, alias = "q", alias = "query", alias = "name")]
    place: Option<String>,
    /// Explicit polygon bbox `{min_lat, max_lat, min_lng, max_lng}`. Used
    /// when the caller already has coordinates (e.g. from a polygon
    /// returned by an earlier `/v1/locate`).
    #[serde(default)]
    polygon_bbox: Option<RecallPolygonBbox>,
    /// Bands to recall at every cell in the polygon. Empty / omitted means
    /// "every band attested at the cell" (same as `/v1/recall`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bands: Option<Vec<String>>,
    /// Optional uniform tslot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tslot: Option<u64>,
    /// Cap on cells sampled from the polygon. Default 64; max 256 (the
    /// recall_many ceiling).
    #[serde(default)]
    max_cells: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
struct RecallPolygonBbox {
    min_lat: f64,
    max_lat: f64,
    min_lng: f64,
    max_lng: f64,
}

async fn post_recall_polygon(
    State(s): State<AppState>,
    Json(req): Json<RecallPolygonReq>,
) -> Result<Json<JsonValue>, ApiError> {
    // Resolve polygon_bbox: explicit → place lookup → error.
    let (bbox, polygon_source, place_label, via): (
        (f64, f64, f64, f64),
        &'static str,
        Option<String>,
        &'static str,
    ) = if let Some(b) = req.polygon_bbox.as_ref() {
        (
            (b.min_lat, b.max_lat, b.min_lng, b.max_lng),
            "request_polygon_bbox",
            None,
            "direct",
        )
    } else if let Some(p) = req.place.as_deref() {
        // Reuse locate_inner so we get the exact same geocoder layering
        // (embedded → cache → Nominatim) including the wide-bbox table.
        let lr = LocateReq {
            lat: None,
            lng: None,
            place: Some(p.into()),
        };
        let resp = locate_inner(lr).await?;
        // Pull polygon_bbox out; if Nominatim didn't return one, fall back
        // to the centre cell's bbox (single-cell fan-out, basically
        // /v1/recall behaviour but still OK as a degenerate case).
        let pb = resp.0.get("polygon_bbox").cloned();
        let lab = resp
            .0
            .get("place_label")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let v = resp
            .0
            .get("via")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let via_static: &'static str = match v {
            "embedded" => "embedded",
            "cache" => "cache",
            "nominatim" => "nominatim",
            "direct" => "direct",
            _ => "unknown",
        };
        if let Some(JsonValue::Object(m)) = pb {
            let g = |k: &str| m.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0);
            let src = m
                .get("source")
                .and_then(|v| v.as_str())
                .unwrap_or("nominatim_boundingbox");
            let src_static: &'static str = match src {
                "wide_bbox_table" => "wide_bbox_table",
                "nominatim_boundingbox" => "nominatim_boundingbox",
                _ => "geocoder",
            };
            (
                (g("min_lat"), g("max_lat"), g("min_lng"), g("max_lng")),
                src_static,
                lab,
                via_static,
            )
        } else {
            // No polygon — fall back to the locate centre + a tiny epsilon
            // so sample_cells_in_bbox still returns the centre cell.
            let bbox_centre = resp.0.get("bbox_deg").cloned();
            if let Some(JsonValue::Object(m)) = bbox_centre {
                let g = |k: &str| m.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0);
                (
                    (g("min_lat"), g("max_lat"), g("min_lng"), g("max_lng")),
                    "centre_cell_bbox",
                    lab,
                    via_static,
                )
            } else {
                return Err(ApiError(StatusCode::BAD_REQUEST, ErrorBody {
                        code: ErrorCode::Internal,
                        message: format!("recall_polygon: place '{p}' resolved without bbox or centre — try passing polygon_bbox explicitly"),
                    }));
            }
        }
    } else {
        return Err(ApiError(StatusCode::BAD_REQUEST, ErrorBody {
                code: ErrorCode::Internal,
                message: "recall_polygon: supply either {place: \"...\"} or {polygon_bbox: {min_lat, max_lat, min_lng, max_lng}}".into(),
            }));
    };

    // Sanity-check the bbox.
    if !(bbox.0.is_finite() && bbox.1.is_finite() && bbox.2.is_finite() && bbox.3.is_finite())
        || bbox.0 > bbox.1
        || bbox.2 > bbox.3
    {
        return Err(ApiError(StatusCode::BAD_REQUEST, ErrorBody {
            code: ErrorCode::Internal,
            message: format!("recall_polygon: degenerate bbox {bbox:?}; need min_lat ≤ max_lat and min_lng ≤ max_lng with finite values"),
        }));
    }

    let max_cells = req.max_cells.unwrap_or(64).clamp(1, 256);
    let cells = sample_cells_in_bbox(bbox, max_cells);
    if cells.is_empty() {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            ErrorBody {
                code: ErrorCode::Internal,
                message: "recall_polygon: bbox sampled to zero cells".into(),
            },
        ));
    }

    // Fan out. Reuse the same lazy-materialize helper as POST /v1/recall
    // so a cell that hasn't been seeded for a requested band still picks
    // up a fresh signed fact from the registered connector. Without this,
    // recall_polygon at a never-touched region returns zero facts even
    // for bands whose materializer is alive — defeating the whole point.
    metrics_inc(&RECALL_TOTAL);
    let mut by_cell = serde_json::Map::with_capacity(cells.len());
    let mut merged_facts: Vec<JsonValue> = Vec::new();
    let mut total_facts = 0usize;
    let mut materialize_notes_all: Vec<JsonValue> = Vec::new();
    for cell in &cells {
        let r = RecallReq {
            cell: cell.clone(),
            bands: req.bands.clone(),
            tslot: req.tslot,
        };
        match recall_with_auto_materialize(&r, &s).await {
            Ok((resp, notes)) => {
                total_facts += resp.facts.len();
                materialize_notes_all.extend(notes);
                if let Ok(j) = serde_json::to_value(&resp) {
                    if let Some(JsonValue::Array(facts)) = j.get("facts").cloned() {
                        merged_facts.extend(facts);
                    }
                    by_cell.insert(cell.clone(), j);
                }
            }
            Err(e) => {
                by_cell.insert(
                    cell.clone(),
                    json!({
                        "error":  e.1.message,
                        "code":   format!("{:?}", e.1.code),
                        "status": e.0.as_u16(),
                    }),
                );
            }
        }
    }

    let mut out = json!({
        "schema": "emem.recall_polygon.v1",
        "polygon_bbox": {
            "min_lat": bbox.0, "max_lat": bbox.1,
            "min_lng": bbox.2, "max_lng": bbox.3,
            "source": polygon_source,
        },
        "place": req.place,
        "place_label": place_label,
        "via": via,
        "cells_sampled": cells.len(),
        "cells": cells,
        "facts_returned": total_facts,
        "merged_facts": merged_facts,
        "by_cell": JsonValue::Object(by_cell),
        "next": [
            "Each cell.receipt is independently signed; verify any cell's receipt via POST /v1/verify_receipt.",
            "For region statistics over the merged_facts, POST /v1/query_region with `geometry: \"cells:c1,c2,...\"` and the same bands.",
            "If you need a finer scan, raise `max_cells` (capped at 256) or split the bbox client-side.",
        ],
        "agent_hint": "POST /v1/recall_polygon collapses /v1/locate → polygon_sample_cells → /v1/recall_many into one call. The bbox source is declared so an agent can detect when the geocoder fell back from polygon (Nominatim bbox) to a single-cell centroid (place-name drift mitigation: fail loud, not silent).",
    });
    if !materialize_notes_all.is_empty() {
        if let Some(m) = out.as_object_mut() {
            m.insert(
                "materialize_notes".into(),
                JsonValue::Array(materialize_notes_all),
            );
        }
    }
    Ok(Json(out))
}

async fn post_query_region(
    State(s): State<AppState>,
    Json(req): Json<QueryRegionReq>,
) -> Result<Json<JsonValue>, ApiError> {
    let resp = query_region(&req, &s).await?;
    Ok(Json(serde_json::to_value(resp).unwrap_or(json!({}))))
}

/// Build a `resolved_from` JSON object surfacing every place name we
/// substituted for a cell64. Empty when every input was already a cell64.
fn resolved_envelope(entries: Vec<(String, ResolvedRef)>) -> Option<JsonValue> {
    let mut out = serde_json::Map::new();
    for (field, r) in entries {
        if matches!(r, ResolvedRef::Place { .. }) {
            out.insert(field, serde_json::to_value(&r).unwrap_or(JsonValue::Null));
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(JsonValue::Object(out))
    }
}

fn attach_resolved(mut body: JsonValue, env: Option<JsonValue>) -> JsonValue {
    if let (Some(env), Some(map)) = (env, body.as_object_mut()) {
        map.insert("resolved_from".into(), env);
    }
    body
}

async fn post_compare(
    State(s): State<AppState>,
    Json(mut req): Json<CompareReq>,
) -> Result<Json<JsonValue>, ApiError> {
    // Accept place names in either side. `compare {a:"Tokyo", b:"Mumbai"}`
    // now works without the agent having to call `emem_locate` twice first.
    let (a_cell, ra) = resolve_cell_field(&req.a).await?;
    let (b_cell, rb) = resolve_cell_field(&req.b).await?;
    req.a = a_cell;
    req.b = b_cell;
    let resp = compare(&req, &s).await?;
    let env = resolved_envelope(vec![("a".into(), ra), ("b".into(), rb)]);
    Ok(Json(attach_resolved(
        serde_json::to_value(resp).unwrap_or(json!({})),
        env,
    )))
}

async fn post_compare_bands(
    State(s): State<AppState>,
    Json(mut req): Json<CompareBandsReq>,
) -> Result<Json<JsonValue>, ApiError> {
    let (cell, rc) = resolve_cell_field(&req.cell).await?;
    req.cell = cell;
    let resp = compare_bands(&req, &s).await?;
    let env = resolved_envelope(vec![("cell".into(), rc)]);
    Ok(Json(attach_resolved(
        serde_json::to_value(resp).unwrap_or(json!({})),
        env,
    )))
}

async fn post_find_similar(
    State(s): State<AppState>,
    Json(mut req): Json<FindSimilarReq>,
) -> Result<Json<JsonValue>, ApiError> {
    // `key` may be `inline:[...]`, a cell64, or a place name. Only the
    // last needs resolution. Inline literals carry their own vector.
    let mut env_entries: Vec<(String, ResolvedRef)> = Vec::new();
    if !req.key.starts_with("inline:") {
        let (cell, rc) = resolve_cell_field(&req.key).await?;
        req.key = cell;
        env_entries.push(("key".into(), rc));
    }
    let resp = find_similar(&req, &s).await?;
    let env = resolved_envelope(env_entries);
    Ok(Json(attach_resolved(
        serde_json::to_value(resp).unwrap_or(json!({})),
        env,
    )))
}

async fn post_diff(
    State(s): State<AppState>,
    Json(mut req): Json<DiffReq>,
) -> Result<Json<JsonValue>, ApiError> {
    let (cell, rc) = resolve_cell_field(&req.cell).await?;
    req.cell = cell;
    let resp = diff(&req, &s).await?;
    let env = resolved_envelope(vec![("cell".into(), rc)]);
    Ok(Json(attach_resolved(
        serde_json::to_value(resp).unwrap_or(json!({})),
        env,
    )))
}

async fn post_trajectory(
    State(s): State<AppState>,
    Json(mut req): Json<TrajectoryReq>,
) -> Result<Json<JsonValue>, ApiError> {
    let (cell, rc) = resolve_cell_field(&req.cell).await?;
    req.cell = cell;
    let resp = trajectory(&req, &s).await?;
    let env = resolved_envelope(vec![("cell".into(), rc)]);
    Ok(Json(attach_resolved(
        serde_json::to_value(resp).unwrap_or(json!({})),
        env,
    )))
}

/// Request body for `POST /v1/backfill`. Mirrors the MCP `emem_backfill`
/// schema declared in `emem-mcp/src/lib.rs`. `deny_unknown_fields` so
/// agents that send `from_unix`/`to_unix` (or any other typo) get a 400
/// instead of a silently-defaulted call that backfills the entire
/// history of the band.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BackfillReq {
    cell: String,
    band: String,
    #[serde(default)]
    start_unix: Option<i64>,
    #[serde(default)]
    end_unix: Option<i64>,
    #[serde(default)]
    max_facts: Option<usize>,
}

async fn post_backfill(
    State(s): State<AppState>,
    Json(mut req): Json<BackfillReq>,
) -> Result<Json<JsonValue>, ApiError> {
    let (cell, rc) = resolve_cell_field(&req.cell).await?;
    req.cell = cell;
    let resp = backfill_inner(req, &s).await?;
    let env = resolved_envelope(vec![("cell".into(), rc)]);
    Ok(Json(attach_resolved(resp, env)))
}

/// `GET /v1/schema` — serve the active CDDL/JSON schema bundle. REST
/// mirror of the MCP `emem_schema` tool. Closes the parity gap reported
/// where curl-to-/v1/schema returned 404 while the MCP tool worked.
async fn get_schema() -> Json<JsonValue> {
    Json(serde_json::to_value(&*emem_core::schema::DEFAULT).unwrap_or(JsonValue::Null))
}

async fn post_verify(
    State(s): State<AppState>,
    Json(mut req): Json<VerifyReq>,
) -> Result<Json<JsonValue>, ApiError> {
    let (cell, rc) = resolve_cell_field(&req.cell).await?;
    req.cell = cell;
    let resp = verify(&req, &s).await?;
    let env = resolved_envelope(vec![("cell".into(), rc)]);
    Ok(Json(attach_resolved(
        serde_json::to_value(resp).unwrap_or(json!({})),
        env,
    )))
}

async fn post_intent(State(s): State<AppState>, Json(intent): Json<Intent>) -> Json<JsonValue> {
    // The planner maps each Intent variant to a sequence of primitive
    // ToolCalls; we then execute every call in order and return both the
    // plan and the per-step results so the agent has everything in one
    // round-trip. Without execution the agent gets a paper plan and has
    // to do another tool round-trip itself — which in practice means it
    // *doesn't*, and ends up guessing cell64 strings.
    let plan = plan(&intent);
    let mut results: Vec<JsonValue> = Vec::with_capacity(plan.calls.len());
    for call in &plan.calls {
        let args_json = ciborium_to_json(&call.args);
        match mcp_tool_call(&call.primitive, args_json.clone(), &s).await {
            Ok(v) => results.push(json!({
                "primitive": call.primitive,
                "args": args_json,
                "ok": true,
                "result": v,
            })),
            Err((code, msg)) => results.push(json!({
                "primitive": call.primitive,
                "args": args_json,
                "ok": false,
                "error": { "code": code, "message": msg },
            })),
        }
    }
    Json(json!({
        "plan": serde_json::to_value(&plan).unwrap_or(json!({})),
        "results": results,
        "composite_suggestions": suggest_algorithms_for_intent(&intent),
    }))
}

/// Variant-aware algorithm hints. The planner emits raw primitive
/// ToolCalls; this helper surfaces the named composition recipes from
/// `/v1/algorithms` that are typically applied AFTER the plan executes.
/// Computed from the live registry — no hardcoded names — so newly
/// registered algorithms become discoverable through `/v1/intent`
/// without touching this function.
fn suggest_algorithms_for_intent(intent: &Intent) -> JsonValue {
    let reg = &*emem_core::algorithms::DEFAULT;
    use emem_core::algorithms::AlgorithmKind;

    // Variant → set of algorithm-key prefixes that semantically fit.
    // Prefix-match is intentional — e.g. `embedding_` covers cosine,
    // l2_distance, change_score, neighborhood_consistency, etc.
    let pattern: &[&str] = match intent {
        Intent::WhereIs { .. } => &[],
        Intent::WhatIsHere { .. } | Intent::Ask { .. } => &[
            "vegetation_class_from_ndvi",
            "flood_history_class",
            "flood_risk",
            "water_consensus",
            "built_up_from_ndbi",
            "urban_density_score",
            "livability_index",
            "outdoor_comfort_score",
            "place_archetype_match",
            "embedding_neighborhood_consistency",
        ],
        Intent::IsLike { .. } => &[
            "embedding_cosine",
            "embedding_l2_distance",
            "region_similarity",
            "place_archetype_match",
            "embedding_weighted_blend",
        ],
        Intent::DidChange { .. } => &[
            "embedding_change_score",
            "trend_strength",
            "burn_severity_from_dnbr",
            "anomaly_zscore",
        ],
        Intent::FindLike { .. } => &[
            "visual_search_match",
            "embedding_novelty",
            "embedding_diversity_score",
            "embedding_corridor_consistency",
            "place_archetype_match",
        ],
        Intent::Confirm { .. } => &[
            "anomaly_zscore",
            "trend_strength",
            "flood_history_class",
            "vegetation_class_from_ndvi",
        ],
    };
    let mut hits: Vec<&emem_core::algorithms::Algorithm> = reg
        .algorithms
        .iter()
        .filter(|a| pattern.iter().any(|p| a.key.starts_with(p)))
        .collect();
    // Stable order: prefix order, then key alpha. Helps agent prompt
    // determinism — same intent always returns suggestions in the same
    // order, so cached responses stay valid.
    hits.sort_by(|a, b| {
        let ai = pattern
            .iter()
            .position(|p| a.key.starts_with(p))
            .unwrap_or(usize::MAX);
        let bi = pattern
            .iter()
            .position(|p| b.key.starts_with(p))
            .unwrap_or(usize::MAX);
        ai.cmp(&bi).then_with(|| a.key.cmp(&b.key))
    });

    let alg_cid = emem_core::manifest::manifest_cid(reg).ok();
    json!({
        "_purpose": "Composition recipes the agent should apply AFTER `plan` executes — the planner returns raw fact-fetching primitives; these algorithms turn those facts into derived scores / classifications / similarities. Cite the chosen algorithm key + algorithms_cid alongside the input fact_cids in the receipt.",
        "algorithms_cid": alg_cid,
        "applicable": hits.iter().map(|a| json!({
            "key":         a.key,
            "kind":        a.kind,
            "domain":      a.domain,
            "input_bands": a.inputs.iter().filter_map(|i| i.band.clone()).collect::<Vec<_>>(),
            "when_to_use": a.when_to_use.chars().take(180).collect::<String>(),
            "fetch_url":   format!("/v1/algorithms/{}", a.key),
        })).collect::<Vec<_>>(),
        "_note_for_more_specific": match intent {
            Intent::WhatIsHere { .. } | Intent::Ask { .. } => "If the user's question is COMPOSITE (flood risk, walkability, climate exposure), look up `data_at_this_cell.algorithms_for_topic` from a fresh /v1/locate at this cell — that is the topic→recipe map.",
            _ => "GET /v1/algorithms for the full registry of 68 recipes across 17 domains.",
        },
        "_emit_ai_kind_distribution": json!({
            "solo_count":      reg.by_kind(AlgorithmKind::Solo).count(),
            "combined_count":  reg.by_kind(AlgorithmKind::Combined).count(),
            "embedding_count": reg.by_kind(AlgorithmKind::Embedding).count(),
        }),
    })
}

/// Convert a ciborium::Value into serde_json::Value so primitive args
/// emitted by the planner can be re-used as JSON request bodies.
fn ciborium_to_json(v: &ciborium::Value) -> JsonValue {
    use ciborium::Value as C;
    match v {
        C::Null => JsonValue::Null,
        C::Bool(b) => JsonValue::Bool(*b),
        C::Integer(i) => {
            let i: i128 = (*i).into();
            if let Ok(i64v) = i64::try_from(i) {
                JsonValue::from(i64v)
            } else {
                JsonValue::from(i.to_string())
            }
        }
        C::Float(f) => serde_json::Number::from_f64(*f)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null),
        C::Text(s) => JsonValue::String(s.clone()),
        C::Bytes(_) => JsonValue::Null,
        C::Array(xs) => JsonValue::Array(xs.iter().map(ciborium_to_json).collect()),
        C::Map(pairs) => {
            let mut m = serde_json::Map::new();
            for (k, val) in pairs {
                if let C::Text(ks) = k {
                    m.insert(ks.clone(), ciborium_to_json(val));
                }
            }
            JsonValue::Object(m)
        }
        C::Tag(_, inner) => ciborium_to_json(inner),
        _ => JsonValue::Null,
    }
}

async fn post_attest(
    State(s): State<AppState>,
    Json(att): Json<Attestation>,
) -> Result<Json<JsonValue>, ApiError> {
    match s.storage.put_attestation(&att).await {
        Ok(cids) => {
            metrics_inc(&ATTEST_TOTAL);
            let cid_strs: Vec<&str> = cids.iter().map(|c| c.as_str()).collect();
            Ok(Json(json!({ "cids": cid_strs, "count": cids.len() })))
        }
        Err(e) => {
            metrics_inc(&ATTEST_FAIL_TOTAL);
            Err(ApiError::from(e))
        }
    }
}

async fn post_attest_cbor(
    State(s): State<AppState>,
    body: Bytes,
) -> Result<Json<JsonValue>, ApiError> {
    let att: Attestation = ciborium::de::from_reader(body.as_ref()).map_err(|e| {
        metrics_inc(&ATTEST_FAIL_TOTAL);
        ApiError(
            StatusCode::BAD_REQUEST,
            ErrorBody {
                code: ErrorCode::CanonicalEncodingDivergence,
                message: format!("cbor decode: {e}"),
            },
        )
    })?;
    match s.storage.put_attestation(&att).await {
        Ok(cids) => {
            metrics_inc(&ATTEST_TOTAL);
            let cid_strs: Vec<&str> = cids.iter().map(|c| c.as_str()).collect();
            Ok(Json(json!({ "cids": cid_strs, "count": cids.len() })))
        }
        Err(e) => {
            metrics_inc(&ATTEST_FAIL_TOTAL);
            Err(ApiError::from(e))
        }
    }
}

#[derive(Deserialize)]
struct VerifyReceiptReq {
    /// The receipt object to audit.
    receipt: emem_fact::Receipt,
    /// Optional override pubkey (32-byte base32-nopad-lc). When unset, uses
    /// the receipt's `responder` field — this lets agents verify any
    /// responder's receipt without trusting this server.
    #[serde(default)]
    pubkey_b32: Option<String>,
}

async fn post_verify_receipt(
    Json(req): Json<VerifyReceiptReq>,
) -> Result<Json<JsonValue>, ApiError> {
    let r = &req.receipt;
    let pk_bytes: [u8; 32] = if let Some(b32) = req.pubkey_b32 {
        let raw = data_encoding::BASE32_NOPAD
            .decode(b32.to_uppercase().as_bytes())
            .map_err(|e| {
                ApiError(
                    StatusCode::BAD_REQUEST,
                    ErrorBody {
                        code: ErrorCode::Internal,
                        message: format!("pubkey_b32 decode: {e}"),
                    },
                )
            })?;
        if raw.len() != 32 {
            return Err(ApiError(
                StatusCode::BAD_REQUEST,
                ErrorBody {
                    code: ErrorCode::Internal,
                    message: format!("pubkey_b32 must decode to 32 bytes, got {}", raw.len()),
                },
            ));
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&raw);
        arr
    } else {
        r.responder.0
    };

    let mut h = blake3::Hasher::new();
    h.update(r.request_id.as_bytes());
    h.update(b"|");
    h.update(r.served_at.as_bytes());
    h.update(b"|");
    h.update(r.primitive.as_bytes());
    h.update(b"|");
    for c in &r.cells {
        h.update(c.as_bytes());
        h.update(b",");
    }
    h.update(b"|");
    for c in &r.fact_cids {
        h.update(c.as_str().as_bytes());
        h.update(b",");
    }
    let msg = h.finalize();

    let pk = ed25519_dalek::VerifyingKey::from_bytes(&pk_bytes).map_err(|e| {
        ApiError(
            StatusCode::UNPROCESSABLE_ENTITY,
            ErrorBody {
                code: ErrorCode::BadSignature,
                message: format!("bad pubkey: {e}"),
            },
        )
    })?;
    let sig = ed25519_dalek::Signature::from_bytes(&r.signature.0);
    let valid = pk.verify_strict(msg.as_bytes(), &sig).is_ok();

    Ok(Json(json!({
        "valid": valid,
        "signer_pubkey_b32": data_encoding::BASE32_NOPAD.encode(&pk_bytes).to_lowercase(),
        "preimage_blake3_hex": msg.to_hex().to_string(),
        "primitive": r.primitive,
        "served_at": r.served_at,
        "fact_cids_count": r.fact_cids.len(),
    })))
}

async fn get_fact(
    State(s): State<AppState>,
    Path(cid): Path<String>,
    headers: HeaderMap,
) -> Response {
    // Immutable: the CID *is* the validator. Return 304 on If-None-Match match.
    let etag_value = format!("\"{}\"", &cid);
    if let Some(if_none) = headers.get(IF_NONE_MATCH).and_then(|v| v.to_str().ok()) {
        if if_none.contains(&etag_value) {
            return Response::builder()
                .status(StatusCode::NOT_MODIFIED)
                .header(ETAG, &etag_value)
                .header(CACHE_CONTROL, "public, max-age=31536000, immutable")
                .body(axum::body::Body::empty())
                .unwrap_or_else(|_| StatusCode::NOT_MODIFIED.into_response());
        }
    }
    let cid_obj = emem_fact::FactCid::new(cid.clone());
    let facts = match s.storage.get_facts_many(&[cid_obj]).await {
        Ok(v) => v,
        Err(e) => return ApiError::from(e).into_response(),
    };
    let Some(Some(fact)) = facts.into_iter().next() else {
        return ApiError(
            StatusCode::NOT_FOUND,
            ErrorBody {
                code: ErrorCode::CidNotFound,
                message: format!("no fact for cid={cid}"),
            },
        )
        .into_response();
    };
    let body = serde_json::to_string(&fact).unwrap_or_else(|_| "{}".into());
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "application/json; charset=utf-8")
        .header(ETAG, &etag_value)
        .header(CACHE_CONTROL, "public, max-age=31536000, immutable")
        .body(axum::body::Body::from(body))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

// ── MCP JSON-RPC 2.0 ──────────────────────────────────────────────────────

#[derive(Deserialize)]
struct JsonRpcReq {
    #[serde(default)]
    _jsonrpc: Option<String>,
    method: String,
    #[serde(default)]
    params: Option<JsonValue>,
    #[serde(default)]
    id: Option<JsonValue>,
}

/// `GET /mcp` — discovery document.
///
/// MCP transport is JSON-RPC 2.0 over POST. Plain GET would otherwise 405.
/// We return a self-describing payload so a human or agent that hits the
/// URL learns the transport, the protocol version, the tool names, and a
/// pasteable `initialize` body — without having to read source.
async fn mcp_discover(State(s): State<AppState>) -> Json<JsonValue> {
    let pubkey = data_encoding::BASE32_NOPAD
        .encode(&s.identity.pubkey.0)
        .to_lowercase();
    let tools: Vec<JsonValue> = emem_mcp::TOOLS
        .iter()
        .map(|t| json!({"name": t.name, "description": t.description}))
        .collect();
    let origin = public_origin().unwrap_or_else(|| "<your-emem-origin>".into());
    let curl_example = format!(
        "curl -s -X POST {origin}/mcp -H 'content-type: application/json' -d '{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{{}}}}'",
    );
    Json(json!({
        "schema": "emem.mcp.discover.v1",
        "transport": "MCP Streamable HTTP (2025-03-26) over HTTPS on the hosted instance: single endpoint, POST for client→server JSON-RPC, GET for this discovery doc",
        "endpoint": "/mcp",
        "method": "POST",
        "content_type": "application/json",
        "accept": ["application/json", "text/event-stream"],
        "protocolVersion": "2025-03-26",
        "serverInfo": { "name": "emem", "version": env!("CARGO_PKG_VERSION") },
        "responder_pubkey_b32": pubkey,
        "tools": tools,
        "examples": {
            "initialize": {
                "request": {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}},
                "curl": curl_example,
            },
            "tools_list": {
                "request": {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
            },
            "tools_call_recall": {
                "request": {
                    "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                    "params": {
                        "name": "emem_recall",
                        "arguments": {"cell": "damO.zb000.xUti.zde78", "bands": ["copdem30m.elevation_mean"]}
                    }
                }
            }
        },
        "client_configs": {
            "claude_desktop": "/examples/claude-desktop.json",
            "claude_code": "/examples/claude-code.mcp.json",
            "cursor": "/examples/cursor.mcp.json",
            "cline": "/examples/cline.mcp.json"
        },
        "see_also": {
            "agent_card": "/v1/agent_card",
            "discover": "/v1/discover",
            "openapi": "/openapi.json",
            "agents_md": "/agents.md"
        }
    }))
}

async fn mcp_jsonrpc(
    State(s): State<AppState>,
    Json(req): Json<JsonRpcReq>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    metrics_inc(&MCP_TOTAL);
    let started = std::time::Instant::now();
    // Per JSON-RPC 2.0 §4.1 + MCP Streamable-HTTP spec, a request without
    // `id` is a Notification: the server MUST NOT reply with a Response
    // object. The Streamable-HTTP transport recommends HTTP 202 Accepted
    // with empty body in that case. Detect-and-short-circuit before any
    // dispatch so notifications never accidentally produce a response.
    let is_notification = req.id.is_none();
    let id = req.id.clone().unwrap_or(JsonValue::Null);
    let method = req.method.clone();
    // Pre-extract the tool name for tracing — without this, every /mcp POST
    // looks identical in access logs and we can't tell `emem.recall` calls
    // apart from `emem.find_similar` calls.
    let tool_name = if method == "tools/call" {
        req.params
            .as_ref()
            .and_then(|p| p.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    } else {
        String::new()
    };
    let result: Result<JsonValue, (i64, String)> = match method.as_str() {
        "initialize" => {
            // Spec-correct version negotiation: if the client requested a
            // version we support, echo it back; otherwise default to our
            // highest. We support 2024-11-05 (initial Streamable-HTTP),
            // 2025-03-26 (annotations, structuredContent), and 2025-06-18
            // (tool titles + content-block resource refresh) — all three
            // share the same wire shape for tools/list and tools/call,
            // and emem's tools haven't required behavioural changes
            // across them.
            const SUPPORTED: &[&str] = &["2024-11-05", "2025-03-26", "2025-06-18"];
            let requested = req
                .params
                .as_ref()
                .and_then(|p| p.get("protocolVersion"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let negotiated = if SUPPORTED.contains(&requested) {
                requested
            } else {
                "2025-06-18"
            };
            Ok(json!({
                "protocolVersion": negotiated,
                "serverInfo": { "name": "emem", "version": env!("CARGO_PKG_VERSION") },
                // Declare every MCP feature this responder serves so spec-
                // compliant clients (Claude Desktop, Claude.ai connector
                // picker, MCP Inspector, Cursor, Cline) know to call
                // resources/list and prompts/list. `listChanged: false`
                // because emem's resource and prompt sets are compiled in
                // — they only change on a redeploy.
                "capabilities": {
                    "tools":     { "listChanged": false },
                    "resources": { "listChanged": false, "subscribe": false },
                    "prompts":   { "listChanged": false },
                },
            }))
        }
        "tools/list" => Ok(json!({
            // MCP `description` is the only natural-language field the host
            // LLM sees when picking a tool, so we fold `when_to_use` into it.
            // Without this, agents miss strong guidance like "ALWAYS call
            // emem_locate first" and end up guessing cell64 strings.
            //
            // `annotations` carries the five behavioural hints the Anthropic
            // Software Directory expects (`title`, `readOnlyHint`,
            // `destructiveHint`, `idempotentHint`, `openWorldHint`). Hosts
            // (Claude Desktop, Claude.ai connector picker) use these to
            // group tools, gate auto-execution, and label them. Per-tool
            // hints are explicit fields on `ToolDescriptor`; the per-category
            // helpers on `ToolCategory` are kept as a fallback derivation
            // for clients that read the descriptor crate directly.
            "tools": emem_mcp::TOOLS.iter().map(|t| json!({
                "name": t.name,
                "title": t.title,
                "description": format!("{}\n\nWhen to use: {}", t.description, t.when_to_use),
                "inputSchema": serde_json::from_str::<JsonValue>(t.input_schema).unwrap_or(json!({})),
                "annotations": {
                    "title":           t.title,
                    "readOnlyHint":    t.read_only_hint,
                    "destructiveHint": t.destructive_hint,
                    "idempotentHint":  t.idempotent_hint,
                    "openWorldHint":   t.open_world_hint,
                },
            })).collect::<Vec<_>>(),
        })),
        "tools/call" => {
            // The MCP spec (2025-03-26 and later) requires `tools/call`
            // results to be a `CallToolResult` envelope:
            //   { content: [{type:"text", text: "..."}],
            //     structuredContent?: {...},
            //     isError?: bool }
            //
            // Returning the bare inner JSON works for some clients but the
            // Anthropic-hosted MCP frontend silently drops it ("completed
            // with no output"), so we wrap every result here. For tool
            // failures we return a *successful* JSON-RPC envelope with
            // `isError: true` inside — that's the spec-correct way to
            // surface tool-runtime errors so the agent sees them as a
            // tool result, not a protocol fault.
            let p = req.params.unwrap_or(JsonValue::Null);
            let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args = p.get("arguments").cloned().unwrap_or(JsonValue::Null);
            match mcp_tool_call(name, args, &s).await {
                Ok(inner) => {
                    // Multimodal escape hatch. A tool that needs to emit
                    // native MCP content blocks (image / resource /
                    // multi-block) sets `_mcp_content` on the inner JSON
                    // — that array becomes the literal `content` field
                    // of the CallToolResult instead of the default
                    // text-wrap. `_mcp_structured` (optional) carries
                    // the structured-content sibling. This keeps the
                    // dispatch signature uniform while letting
                    // `emem_coverage_map` ship a real EmbeddedResource.
                    let raw_content = inner
                        .get("_mcp_content")
                        .and_then(|v| v.as_array())
                        .cloned();
                    let raw_structured = inner.get("_mcp_structured").cloned();
                    if let Some(content) = raw_content {
                        Ok(json!({
                            "content": content,
                            "structuredContent": raw_structured.unwrap_or(JsonValue::Null),
                            "isError": false,
                        }))
                    } else {
                        let text =
                            serde_json::to_string(&inner).unwrap_or_else(|_| "{}".to_string());
                        Ok(json!({
                            "content": [{"type": "text", "text": text}],
                            "structuredContent": inner,
                            "isError": false,
                        }))
                    }
                }
                Err((code, msg)) => {
                    // Unknown-method (-32601) is a protocol error, propagate
                    // as JSON-RPC error. Everything else is a tool runtime
                    // error and should land in CallToolResult with isError.
                    if code == -32601 {
                        Err((code, msg))
                    } else {
                        Ok(json!({
                            "content": [{
                                "type": "text",
                                "text": format!("tool error ({}): {}", code, msg),
                            }],
                            "isError": true,
                        }))
                    }
                }
            }
        }
        // ---- MCP Resources ----------------------------------------------
        //
        // Static, content-addressed-compatible documentation surfaces
        // exposed as MCP `resources`. Compliant clients (Claude Desktop,
        // Inspector) auto-attach them as context, so an agent's first
        // turn already carries our agent_card / spec / whitepaper without
        // a separate tool call. URIs use the `emem://` scheme and are
        // dereferenced server-side from compile-time `include_str!`
        // constants — no I/O, no network round-trip.
        "resources/list" => Ok(json!({
            "resources": mcp_static_resources(),
        })),
        "resources/templates/list" => Ok(json!({
            "resourceTemplates": mcp_resource_templates(),
        })),
        "resources/read" => {
            let p = req.params.unwrap_or(JsonValue::Null);
            match p.get("uri").and_then(|v| v.as_str()) {
                Some(uri) => mcp_read_resource(uri).map(|c| json!({ "contents": [c] })),
                None => Err((-32602, "missing `uri`".to_string())),
            }
        }
        // ---- MCP Prompts ------------------------------------------------
        //
        // Canned prompt templates that wrap the most-asked place questions
        // (flood, air quality, urban heat, place summary, compare, forest
        // loss, coastal eutrophication, carbon-uptake anomaly). Each names
        // its required arguments so hosts can render a form / slot-fill,
        // and `prompts/get` returns a single user-message that already
        // wires the right tool sequence (emem_locate → emem_recall +
        // algorithm key) so the agent doesn't have to re-derive it.
        "prompts/list" => Ok(json!({
            "prompts": mcp_prompts(),
        })),
        "prompts/get" => {
            let p = req.params.unwrap_or(JsonValue::Null);
            match p.get("name").and_then(|v| v.as_str()) {
                Some(name) => {
                    let args = p.get("arguments").cloned().unwrap_or(json!({}));
                    mcp_render_prompt(name, &args)
                }
                None => Err((-32602, "missing `name`".to_string())),
            }
        }
        // MCP lifecycle: client signals it's done with `initialize` by
        // sending `notifications/initialized` (JSON-RPC notification — no
        // id, no response expected). Per spec we MUST accept it; the
        // empty result here is harmless because the dispatch layer below
        // suppresses notifications when `id` is null.
        "notifications/initialized" => Ok(json!({})),
        // Cancellation notifications — we don't track in-flight ids
        // (every tool call is awaited synchronously here), so accept and
        // discard.
        "notifications/cancelled" => Ok(json!({})),
        // Optional health-ping; MCP spec leaves the response shape free
        // beyond it being a successful result.
        "ping" => Ok(json!({})),
        other => Err((-32601, format!("method not found: {other}"))),
    };
    let dur_ms = started.elapsed().as_secs_f64() * 1000.0;
    let (ok, err_code) = match &result {
        Ok(_) => (true, 0i64),
        Err((c, _)) => (false, *c),
    };
    record_mcp_tool(&tool_name, ok, dur_ms);
    tracing::info!(
        target: "emem::mcp",
        mcp_method = %method,
        mcp_tool = %tool_name,
        mcp_ok = ok,
        mcp_error_code = err_code,
        mcp_duration_ms = dur_ms,
        "mcp_call"
    );
    if is_notification {
        // Spec: 202 Accepted with empty body. We still ran the dispatch
        // (`notifications/initialized` is a no-op anyway) so nothing
        // observable changes; only the wire-level response shape does.
        return (axum::http::StatusCode::ACCEPTED, ()).into_response();
    }
    match result {
        Ok(v) => Json(json!({"jsonrpc":"2.0","id":id,"result":v})).into_response(),
        Err((code, msg)) => Json(json!({
            "jsonrpc":"2.0","id":id,
            "error": { "code": code, "message": msg },
        }))
        .into_response(),
    }
}

// ── MCP Resources ────────────────────────────────────────────────────────
//
// Per MCP spec a Resource has `uri`, `name`, optional `description` and
// `mimeType`. The static set below is content-addressed at compile time
// (`include_str!`), so resources/list is constant-time and resources/read
// can never produce stale bytes for a given binary build.

fn mcp_static_resources() -> Vec<JsonValue> {
    // Tuples of (uri, name, mimeType, description) — kept in a single
    // place so resources/list and resources/read share a single source
    // of truth. The bodies are looked up in `mcp_read_resource` below
    // by matching on the uri suffix.
    let entries: &[(&str, &str, &str, &str)] = &[
        (
            "emem://docs/agents.md",
            "agents.md",
            "text/markdown",
            "Full integration guide: REST + MCP setup for Claude Desktop/Code, Cursor, Cline, OpenAI GPT, plus tool reference and worked examples.",
        ),
        (
            "emem://docs/spec.md",
            "spec",
            "text/markdown",
            "Authoritative protocol spec: cell64, tslot, content-addressing, ed25519 receipts, lazy materialization, attestation merkle root.",
        ),
        (
            "emem://docs/whitepaper.md",
            "whitepaper",
            "text/markdown",
            "Architecture and math: 1792-D voxel layout, BLAKE3 fact CIDs, ed25519 attestation, agent-native invariants.",
        ),
        (
            "emem://docs/llms.txt",
            "llms.txt",
            "text/markdown",
            "LLM-optimised summary: when to call which primitive, which bands answer which question, 30-second curl examples.",
        ),
        (
            "emem://docs/llms-full.txt",
            "llms-full.txt",
            "text/markdown",
            "Long-form LLM-optimised guide: full band catalogue, every primitive with example payload, error catalogue.",
        ),
        (
            "emem://docs/agent_walkthroughs.md",
            "agent_walkthroughs",
            "text/markdown",
            "Agent walkthroughs: end-to-end flows for flood-history, urban-heat, similarity, and trajectory questions.",
        ),
        (
            "emem://docs/temporal.md",
            "temporal",
            "text/markdown",
            "Temporal model: Tempo (Static/Slow/Medium/Fast/UltraFast), tslot mapping, backfill semantics, history bounds.",
        ),
        (
            "emem://docs/materializers.md",
            "materializers",
            "text/markdown",
            "Materializer playbook: how a band's upstream connector turns into a signed fact, error envelope, no-fallback rule.",
        ),
        (
            "emem://docs/agent.json",
            "agent.json",
            "application/json",
            "Discovery manifest: operator, surfaces, primitives, policies — what a host platform reads to wire emem in.",
        ),
        (
            "emem://docs/privacy.md",
            "privacy",
            "text/markdown",
            "Privacy policy of the responder operator (Vortx AI). No PII collected from inbound MCP/REST requests.",
        ),
        (
            "emem://docs/terms.md",
            "terms",
            "text/markdown",
            "Terms of service of the responder operator. Apache-2.0 protocol; hosted-instance terms.",
        ),
    ];
    entries
        .iter()
        .map(|(uri, name, mime, desc)| {
            json!({
                "uri":         uri,
                "name":        name,
                "mimeType":    mime,
                "description": desc,
            })
        })
        .collect()
}

fn mcp_resource_templates() -> Vec<JsonValue> {
    // Templated URIs that the host can fill in (cell64 / fact CID) and
    // dereference via resources/read. Useful for "show me this place's
    // polygon" / "show me the bytes behind this CID" without a tool call.
    vec![
        json!({
            "uriTemplate": "emem://cell/{cell64}/geojson",
            "name":        "cell.geojson",
            "mimeType":    "application/geo+json",
            "description": "Cell polygon as GeoJSON Feature with bbox + neighbours, ready for Mapbox/Leaflet/Deck.gl.",
        }),
        json!({
            "uriTemplate": "emem://cell/{cell64}/scene.png",
            "name":        "cell.scene.png",
            "mimeType":    "image/png",
            "description": "Sentinel-2 L2A true-colour 256×256 thumbnail centred on the cell. Use the emem_cell_scene_rgb tool to actually fetch it (resources/read returns a pointer; the tool returns the bytes).",
        }),
    ]
}

fn mcp_read_resource(uri: &str) -> Result<JsonValue, (i64, String)> {
    // Match the docs/{slug} family by suffix. For markdown / plain text
    // resources we return the body via the spec's `text` field; for
    // application/json resources we still return as `text` (the spec
    // permits JSON to be embedded as text), so any client can render
    // them without a separate JSON parser.
    let body: Option<(&str, &str)> = match uri {
        "emem://docs/agents.md" => Some((AGENTS_MD, "text/markdown")),
        "emem://docs/spec.md" => Some((SPEC_MD, "text/markdown")),
        "emem://docs/whitepaper.md" => Some((WHITEPAPER_MD, "text/markdown")),
        "emem://docs/llms.txt" => Some((LLMS_TXT, "text/markdown")),
        "emem://docs/llms-full.txt" => Some((LLMS_FULL_TXT, "text/markdown")),
        "emem://docs/agent_walkthroughs.md" => Some((AGENT_WALKTHROUGHS_MD, "text/markdown")),
        "emem://docs/temporal.md" => Some((TEMPORAL_MD, "text/markdown")),
        "emem://docs/materializers.md" => Some((MATERIALIZERS_MD, "text/markdown")),
        "emem://docs/agent.json" => Some((AGENT_JSON, "application/json")),
        "emem://docs/privacy.md" => Some((PRIVACY_MD, "text/markdown")),
        "emem://docs/terms.md" => Some((TERMS_MD, "text/markdown")),
        _ => None,
    };
    match body {
        Some((text, mime)) => Ok(json!({
            "uri":      uri,
            "mimeType": mime,
            "text":     text,
        })),
        None => Err((
            -32602,
            format!(
                "unknown resource uri '{uri}': call resources/list for the catalog. Templated URIs (emem://cell/...) need to go through the matching tool: emem_cell_geojson / emem_cell_scene_rgb."
            ),
        )),
    }
}

// ── MCP Prompts ──────────────────────────────────────────────────────────
//
// Canned prompts that wrap emem's most-asked place questions. Each prompt
// names the arguments it needs, and `prompts/get` returns a fully-rendered
// user-message that already names the tools the agent should call. Hosts
// (Claude Desktop, Cursor) render them as slash-commands or pickable
// templates, so a non-technical user can ask "is this place flooded?"
// without knowing the band names.

fn mcp_prompts() -> Vec<JsonValue> {
    vec![
        json!({
            "name":        "flood_history",
            "title":       "Has this place flooded historically?",
            "description": "Long-term flood/inundation history at a place, classified from JRC Global Surface Water v1.4 (1984–2021).",
            "arguments": [{
                "name":        "place",
                "description": "Place name, address, or 'lat,lng' string. Resolved via emem_locate.",
                "required":    true,
            }],
        }),
        json!({
            "name":        "air_quality_now",
            "title":       "What's the air quality at this place right now?",
            "description": "Current PM2.5 / PM10 / NO2 / O3 / SO2 / CO at a place, classified against WHO 2021 AQG ladder.",
            "arguments": [{
                "name":        "place",
                "description": "Place name, address, or 'lat,lng'.",
                "required":    true,
            }],
        }),
        json!({
            "name":        "urban_heat",
            "title":       "Is this neighbourhood hot for an urban area?",
            "description": "Urban-heat-island assessment from MODIS LST day/night and indices.urban_canopy_index, with cooling-potential note.",
            "arguments": [{ "name": "place", "description": "Place name or 'lat,lng'.", "required": true }],
        }),
        json!({
            "name":        "place_summary",
            "title":       "Summarize a place",
            "description": "Quick characterisation: landcover (ESA WorldCover), elevation (Cop-DEM), greenness (NDVI), current temperature.",
            "arguments": [{ "name": "place", "description": "Place name or 'lat,lng'.", "required": true }],
        }),
        json!({
            "name":        "compare_places",
            "title":       "How similar are two places?",
            "description": "Cosine similarity over the geotessera 128-D embedding, with dominant-band rationale.",
            "arguments": [
                { "name": "place_a", "description": "First place.",  "required": true },
                { "name": "place_b", "description": "Second place.", "required": true },
            ],
        }),
        json!({
            "name":        "forest_loss",
            "title":       "Has this place lost forest?",
            "description": "Hansen Global Forest Change v1.11 layers: tree cover 2000, year of loss (2001–2023), gain mask.",
            "arguments": [{ "name": "place", "description": "Place name or 'lat,lng'.", "required": true }],
        }),
        json!({
            "name":        "coastal_eutrophication",
            "title":       "Are coastal waters here algal?",
            "description": "SDG 14.1.1a coastal-eutrophication first-pass: floating-algae index + chlorophyll proxy + turbidity + SST.",
            "arguments": [{ "name": "place", "description": "Coastal place name or 'lat,lng'.", "required": true }],
        }),
        json!({
            "name":        "carbon_uptake_anomaly",
            "title":       "Is carbon uptake unusual at this place?",
            "description": "GPP anomaly z-score from MOD17A2H trajectory (current 8-day vs same-DOY climatology).",
            "arguments": [{ "name": "place", "description": "Place name or 'lat,lng'.", "required": true }],
        }),
    ]
}

fn mcp_render_prompt(name: &str, args: &JsonValue) -> Result<JsonValue, (i64, String)> {
    // Argument helpers — return a clear JSON-RPC -32602 if a required
    // arg is missing so hosts can surface the correct field.
    let s = |key: &str| -> Result<String, (i64, String)> {
        args.get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| (-32602, format!("missing or empty argument `{key}`")))
    };

    let (description, text): (&str, String) = match name {
        "flood_history" => {
            let place = s("place")?;
            (
                "Long-term flood/inundation history",
                format!(
                    "Has {place} flooded historically? \
                     Step 1: call emem_locate with q={place:?} to get a cell64. \
                     Step 2: call emem_recall on that cell with bands=['surface_water.recurrence']. \
                     Step 3: classify with the algorithm key `flood_history_class@1` (never/occasional/seasonal/perennial/none). \
                     Cite the receipt's first fact_cid and the responder pubkey in your reply."
                ),
            )
        }
        "air_quality_now" => {
            let place = s("place")?;
            (
                "Current air quality vs WHO 2021 AQG",
                format!(
                    "What is the air quality at {place} right now? \
                     emem_locate then emem_recall bands=['cams.pm25','cams.pm10','cams.no2','cams.o3','cams.so2','cams.co']. \
                     Classify the PM2.5 reading against the WHO 2021 ladder \
                     (≤5 / ≤10 / ≤15 / ≤25 / ≤35 µg/m³ → AQG / IT-4 / IT-3 / IT-2 / IT-1 / exceeds-IT-1). \
                     Surface the materialised_at timestamp so the user knows the cadence (CAMS hourly)."
                ),
            )
        }
        "urban_heat" => {
            let place = s("place")?;
            (
                "Urban heat-island assessment",
                format!(
                    "Is {place} hot for an urban area? \
                     emem_locate then emem_recall bands=['modis.lst_day_8day','modis.lst_night_8day','indices.urban_canopy_index','weather.temperature_2m']. \
                     Use the algorithm `carbon.urban_canopy_cooling_potential@1` for the headline number and surface the diurnal amplitude (LST day − LST night)."
                ),
            )
        }
        "place_summary" => {
            let place = s("place")?;
            (
                "Quick place characterisation",
                format!(
                    "Summarize {place}. \
                     emem_locate then emem_recall bands=['esa_worldcover.lc_2021','copdem30m.elevation_mean','indices.ndvi','weather.temperature_2m','overture.places.count','overture.transportation.road_length_m']. \
                     Convert the WorldCover class to a human label (10=tree cover, 20=shrub, 30=grass, 40=crop, 50=built-up, 60=bare, 80=water, 90=wet, 95=mangroves, 100=moss/lichen). \
                     Cite all six fact_cids."
                ),
            )
        }
        "compare_places" => {
            let a = s("place_a")?;
            let b = s("place_b")?;
            (
                "Embedding-cosine similarity",
                format!(
                    "How similar are {a} and {b}? \
                     emem_locate on both → call emem_compare with the two cells and band='geotessera'. \
                     Report the cosine similarity (0.0 = orthogonal physical types, 1.0 = identical), \
                     and call emem_recall on each cell with bands=['esa_worldcover.lc_2021','copdem30m.elevation_mean','indices.ndvi'] to narrate WHY they are/aren't similar."
                ),
            )
        }
        "forest_loss" => {
            let place = s("place")?;
            (
                "Hansen Global Forest Change v1.11 read",
                format!(
                    "Has {place} lost forest since 2000? \
                     emem_locate then emem_recall bands=['hansen.tree_cover_2000','hansen.loss_year','hansen.gain']. \
                     If loss_year > 0, the year of loss is 2000 + loss_year (1=2001 … 23=2023). \
                     Pair with esa_worldcover.lc_2021 to confirm the current state. \
                     Use the algorithm `carbon.deforestation_alert_proxy@1` if you want a composite score across years."
                ),
            )
        }
        "coastal_eutrophication" => {
            let place = s("place")?;
            (
                "SDG 14.1.1a coastal eutrophication first-pass",
                format!(
                    "Are coastal waters at {place} algal? \
                     emem_locate then emem_recall bands=['indices.fai','indices.gndvi','indices.tss','marine.sst']. \
                     Apply the algorithm `sdg.14_1_1a.coastal_eutrophication_index@1` (FAI + GNDVI + TSS + SST modifier). \
                     Flag CEI > 0.6 as anomalous; recommend confirmation via Sentinel-3 OLCI Chl-a once that connector lands."
                ),
            )
        }
        "carbon_uptake_anomaly" => {
            let place = s("place")?;
            (
                "GPP anomaly z-score",
                format!(
                    "Is carbon uptake unusual at {place}? \
                     emem_locate then emem_trajectory band='modis.gpp_8day' across the past 5 years. \
                     Apply the algorithm `carbon.gpp_anomaly_zscore@1`: group by DOY, compute z = (current − mean) / sd. \
                     |z| > 2 is significant; sign indicates direction (negative = below baseline, positive = above)."
                ),
            )
        }
        _ => {
            return Err((
                -32602,
                format!("unknown prompt '{name}': call prompts/list for the catalog"),
            ));
        }
    };

    Ok(json!({
        "description": description,
        "messages": [{
            "role": "user",
            "content": { "type": "text", "text": text },
        }],
    }))
}

async fn mcp_tool_call(
    name: &str,
    mut args: JsonValue,
    s: &AppState,
) -> Result<JsonValue, (i64, String)> {
    // Pre-flight: rewrite cell-typed args from place names to cell64
    // strings so MCP clients get the same place-name UX as REST. The
    // map below names the (tool → field-list) routes that take cell64
    // values. A field is rewritten only when its current value is a
    // string that does not pass `is_cell64_shape` — already-canonical
    // cell64 strings cost zero locate calls.
    let canon = name.replace('.', "_");
    let cell_fields: &[&str] = match canon.as_str() {
        "emem_recall"
        | "emem_compare_bands"
        | "emem_diff"
        | "emem_trajectory"
        | "emem_verify"
        | "emem_cell_scene_rgb"
        | "emem_cell_geojson" => &["cell"],
        "emem_compare" => &["a", "b"],
        "emem_find_similar" => &["key"],
        _ => &[],
    };
    let mut resolved_envelope_map = serde_json::Map::new();
    if !cell_fields.is_empty() {
        if let Some(obj) = args.as_object_mut() {
            for field in cell_fields {
                if let Some(v) = obj.get(*field).and_then(|v| v.as_str()) {
                    // Skip inline:[...] vector literal in find_similar.key
                    if v.starts_with("inline:") {
                        continue;
                    }
                    if !emem_codec::is_cell64_shape(v) {
                        let (cell64, rref) = resolve_cell_field(v)
                            .await
                            .map_err(|e| (-(e.1.code as i64), e.1.message))?;
                        obj.insert((*field).to_string(), JsonValue::String(cell64));
                        if matches!(rref, ResolvedRef::Place { .. }) {
                            resolved_envelope_map.insert(
                                (*field).to_string(),
                                serde_json::to_value(rref).unwrap_or(JsonValue::Null),
                            );
                        }
                    }
                }
            }
        }
    }
    macro_rules! call {
        ($t:ty, $f:ident) => {{
            let req: $t = serde_json::from_value(args).map_err(|e| (-32602, e.to_string()))?;
            let resp = $f(&req, s).await.map_err(|e| {
                let code = e.wire_code();
                (-(code as i64), e.to_string())
            })?;
            serde_json::to_value(resp).map_err(|e| (-32603, e.to_string()))
        }};
    }
    let attach_resolved_env = |mut v: JsonValue| -> JsonValue {
        if !resolved_envelope_map.is_empty() {
            if let Some(map) = v.as_object_mut() {
                map.insert(
                    "resolved_from".into(),
                    JsonValue::Object(resolved_envelope_map.clone()),
                );
            }
        }
        v
    };
    // Accept both the wire-stable underscore form (`emem_recall` — required
    // by Anthropic's hosted MCP frontend whose tool-name validator is
    // `^[a-zA-Z0-9_-]{1,64}$`) and the legacy dotted form (`emem.recall`)
    // so existing integrations keep working. New clients should always
    // see the underscore form via /v1/mcp tools/list.
    match canon.as_str() {
        "emem_locate" => {
            // The geocoder layer is HTTP-side, not a primitive. Reuse the
            // exact `locate_inner` path so the response shape is byte-
            // identical to GET/POST /v1/locate.
            #[derive(serde::Deserialize)]
            struct LocateArgs {
                #[serde(default)]
                place: Option<String>,
                #[serde(default)]
                lat: Option<f64>,
                #[serde(default)]
                lng: Option<f64>,
            }
            let a: LocateArgs =
                serde_json::from_value(args).map_err(|e| (-32602, e.to_string()))?;
            let lreq = LocateReq {
                lat: a.lat,
                lng: a.lng,
                place: a.place,
            };
            match locate_inner(lreq).await {
                Ok(Json(v)) => Ok(v),
                Err(e) => Err((-(e.1.code as i64), e.1.message)),
            }
        }
        "emem_ask" => {
            // Single-shot free-text answer. Same routing as POST /v1/ask;
            // returns the full envelope with topic_routing + facts +
            // algorithms_for_question + scene + caveats so the LLM at
            // the top of the call stack can answer without a second
            // round-trip.
            let req: AskReq = serde_json::from_value(args).map_err(|e| (-32602, e.to_string()))?;
            ask_inner(s.clone(), req)
                .await
                .map_err(|e| (-(e.1.code as i64), e.1.message))
        }
        "emem_recall_polygon" => {
            let req: RecallPolygonReq =
                serde_json::from_value(args).map_err(|e| (-32602, e.to_string()))?;
            match post_recall_polygon(State(s.clone()), Json(req)).await {
                Ok(Json(v)) => Ok(v),
                Err(e) => Err((-(e.1.code as i64), e.1.message)),
            }
        }
        "emem_grid_info" => Ok(grid_info().await.0),
        "emem_coverage_matrix" => Ok(coverage_matrix(State(s.clone())).await.0),
        "emem_materializers" => Ok(materializers(State(s.clone())).await.0),
        "emem_data_availability" => Ok(data_availability(State(s.clone())).await.0),
        "emem_recall" => {
            // Route through the auto-materialize wrapper so MCP gets the
            // same on-demand corpus growth as REST. Without this,
            // emem_recall via MCP returns empty for any cell that hasn't
            // been seeded — even though the materializer is registered.
            // Use the wrapper that accepts singular `band` too — agents
            // call this both ways, and silently dropping `band` makes the
            // recall match every band at the cell instead of the asked-for
            // one (we saw this with `band: "geotessera.2020"` returning a
            // cached weather fact instead of materialising Tessera 2020).
            let api_req: RecallApiReq =
                serde_json::from_value(args).map_err(|e| (-32602, e.to_string()))?;
            let req: RecallReq = api_req.into();
            let (resp, materialize_notes) = recall_with_auto_materialize(&req, s)
                .await
                .map_err(|e| (-(e.1.code as i64), e.1.message))?;
            let mut v = serde_json::to_value(resp).map_err(|e| (-32603, e.to_string()))?;
            if !materialize_notes.is_empty() {
                if let Some(map) = v.as_object_mut() {
                    map.insert(
                        "materialize_notes".into(),
                        JsonValue::Array(materialize_notes),
                    );
                }
            }
            Ok(attach_resolved_env(v))
        }
        "emem_query_region" => call!(QueryRegionReq, query_region),
        "emem_compare" => call!(CompareReq, compare).map(attach_resolved_env),
        "emem_compare_bands" => call!(CompareBandsReq, compare_bands).map(attach_resolved_env),
        "emem_find_similar" => call!(FindSimilarReq, find_similar).map(attach_resolved_env),
        "emem_diff" => call!(DiffReq, diff).map(attach_resolved_env),
        "emem_trajectory" => call!(TrajectoryReq, trajectory).map(attach_resolved_env),
        "emem_verify" => call!(VerifyReq, verify).map(attach_resolved_env),
        "emem_intent" => {
            // Execute the plan in-process so the agent gets the geocoded
            // cell64 (or compare/diff/verify result) in the same call. We
            // re-enter `mcp_tool_call` for each step — Box::pin keeps the
            // future Sized despite the self-recursion. The planner never
            // emits another `emem_intent` step, so there is no infinite
            // recursion in practice.
            let intent: Intent =
                serde_json::from_value(args).map_err(|e| (-32602, e.to_string()))?;
            let p = plan(&intent);
            let mut results: Vec<JsonValue> = Vec::with_capacity(p.calls.len());
            for call in &p.calls {
                let args_json = ciborium_to_json(&call.args);
                let r = Box::pin(mcp_tool_call(&call.primitive, args_json.clone(), s)).await;
                match r {
                    Ok(v) => results.push(json!({
                        "primitive": call.primitive,
                        "args": args_json,
                        "ok": true,
                        "result": v,
                    })),
                    Err((code, msg)) => results.push(json!({
                        "primitive": call.primitive,
                        "args": args_json,
                        "ok": false,
                        "error": { "code": code, "message": msg },
                    })),
                }
            }
            Ok(json!({
                "plan": serde_json::to_value(&p).unwrap_or(JsonValue::Null),
                "results": results,
            }))
        }
        "emem_bands" => {
            Ok(serde_json::to_value(&*emem_core::bands::DEFAULT).unwrap_or(JsonValue::Null))
        }
        "emem_functions" => {
            Ok(serde_json::to_value(&*emem_core::functions::DEFAULT).unwrap_or(JsonValue::Null))
        }
        "emem_sources" => {
            Ok(serde_json::to_value(&*emem_core::sources::DEFAULT).unwrap_or(JsonValue::Null))
        }
        "emem_algorithms" => Ok(algorithms().await.0),
        "emem_cell_geojson" => {
            let cell = args
                .get("cell")
                .and_then(|v| v.as_str())
                .ok_or((-32602i64, "missing `cell`".to_string()))?;
            let feat = build_cell_geojson(cell).map_err(|e| (-32000i64, e))?;
            let geojson_text = serde_json::to_string(&feat).unwrap_or_else(|_| "{}".to_string());
            let origin = public_origin().unwrap_or_else(|| "urn:emem".into());
            let uri = format!("{origin}/v1/cells/{cell}/geojson");
            let summary = format!(
                "Cell polygon GeoJSON for {cell} — Feature with Polygon geometry; properties.centre + bbox + neighbours included. Render the EmbeddedResource above in Mapbox/Leaflet/Deck.gl/QGIS."
            );
            Ok(json!({
                "_mcp_content": [
                    {
                        "type": "resource",
                        "resource": {
                            "uri": uri,
                            "mimeType": "application/geo+json",
                            "text": geojson_text,
                        }
                    },
                    { "type": "text", "text": summary },
                ],
                "_mcp_structured": feat,
            }))
        }
        "emem_cell_scene_rgb" => {
            let cell = args
                .get("cell")
                .and_then(|v| v.as_str())
                .ok_or((-32602i64, "missing `cell`".to_string()))?;
            let max_cloud = args
                .get("max_cloud")
                .and_then(|v| v.as_f64())
                .unwrap_or(20.0);
            let datetime = args
                .get("datetime")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let scene = build_cell_scene_rgb(cell, max_cloud, datetime.as_deref())
                .await
                .map_err(|e| (-32000i64, e))?;
            let b64 = data_encoding::BASE64.encode(&scene.png);
            let summary = format!(
                "Sentinel-2 L2A true-colour at {cell} ({}×{}) · scene {} · {} · cloud_cover {} · EPSG {} · 2-98 % percentile stretch (R: {:.0}-{:.0}, G: {:.0}-{:.0}, B: {:.0}-{:.0})",
                scene.w, scene.h,
                scene.item_id,
                scene.item_datetime,
                scene.cloud_cover.map(|c| format!("{c:.1}%")).unwrap_or_else(|| "n/a".into()),
                scene.epsg,
                scene.stretch_p2_p98.0.0, scene.stretch_p2_p98.0.1,
                scene.stretch_p2_p98.1.0, scene.stretch_p2_p98.1.1,
                scene.stretch_p2_p98.2.0, scene.stretch_p2_p98.2.1,
            );
            Ok(json!({
                "_mcp_content": [
                    { "type": "image", "data": b64, "mimeType": "image/png" },
                    { "type": "text",  "text": summary },
                ],
                "_mcp_structured": {
                    "kind": "cell_scene_rgb",
                    "cell": cell,
                    "width": scene.w,
                    "height": scene.h,
                    "stac_item_id": scene.item_id,
                    "stac_item_datetime": scene.item_datetime,
                    "cloud_cover": scene.cloud_cover,
                    "epsg": scene.epsg,
                    "stretch": {
                        "red":   { "p2": scene.stretch_p2_p98.0.0, "p98": scene.stretch_p2_p98.0.1 },
                        "green": { "p2": scene.stretch_p2_p98.1.0, "p98": scene.stretch_p2_p98.1.1 },
                        "blue":  { "p2": scene.stretch_p2_p98.2.0, "p98": scene.stretch_p2_p98.2.1 },
                    },
                    "rest_url": format!("{}/v1/cells/{cell}/scene.png?max_cloud={max_cloud}",
                        public_origin().unwrap_or_else(|| "urn:emem".into())),
                },
            }))
        }
        "emem_coverage_map" => {
            // Multimodal MCP tool: return the live coverage SVG as a
            // proper EmbeddedResource content block (text + uri +
            // mimeType) so multimodal-aware MCP clients can render the
            // image natively, plus a small text-summary block so
            // text-only clients still get the cell/fact counts.
            let (svg, cell_count, total_facts) = build_coverage_map_svg(s).await;
            let pubkey = data_encoding::BASE32_NOPAD
                .encode(&s.identity.pubkey.0)
                .to_lowercase();
            let origin = public_origin().unwrap_or_else(|| "urn:emem".into());
            let map_url = format!("{origin}/v1/coverage_map.svg");
            let summary = format!(
                "Coverage map · {cell_count} attested cells · {total_facts} facts · responder {}… · render the EmbeddedResource above to view the 1440×720 Plate-Carrée SVG (1° × 1° bins, log-scale colour, continent envelopes for orientation).",
                &pubkey[..32.min(pubkey.len())],
            );
            Ok(json!({
                "_mcp_content": [
                    {
                        "type": "resource",
                        "resource": {
                            "uri": map_url,
                            "mimeType": "image/svg+xml",
                            "text": svg,
                        }
                    },
                    { "type": "text", "text": summary },
                ],
                "_mcp_structured": {
                    "kind": "coverage_map_svg",
                    "cell_count": cell_count,
                    "total_facts": total_facts,
                    "responder_pubkey_b32": pubkey,
                    "rest_url": map_url,
                },
            }))
        }
        "emem_schema" => {
            Ok(serde_json::to_value(&*emem_core::schema::DEFAULT).unwrap_or(JsonValue::Null))
        }
        "emem_manifests" => Ok(json!({
            "bands_cid": &s.manifests.bands_cid,
            "functions_cid": s.manifests.registry_cid.as_str(),
            "sources_cid": &s.manifests.sources_cid,
            "schema_cid": s.manifests.schema_cid.as_str(),
        })),
        "emem_errors" => Ok(serde_json::to_value(emem_mcp::TOOLS.len()).unwrap_or(JsonValue::Null)),
        "emem_fetch" => {
            #[derive(serde::Deserialize)]
            struct FetchArgs {
                cid: String,
            }
            let a: FetchArgs = serde_json::from_value(args).map_err(|e| (-32602, e.to_string()))?;
            let trimmed = a.cid.trim();
            if trimmed.is_empty() {
                return Err((
                    -(ErrorCode::CidNotFound as i64),
                    "cid must be a non-empty string".into(),
                ));
            }
            // Cheap shape check — emem CIDs are blake3 base32-nopad lowercase
            // (52 chars). Anything else cannot resolve, and rejecting early
            // saves a storage round-trip on obviously-malformed input.
            let shape_ok = trimmed.len() >= 32
                && trimmed.len() <= 96
                && trimmed.bytes().all(|c| c.is_ascii_alphanumeric());
            if !shape_ok {
                return Err((
                    -(ErrorCode::CidNotFound as i64),
                    format!("cid '{trimmed}' is not a well-formed content address"),
                ));
            }
            let cid = emem_fact::FactCid::new(trimmed.to_string());
            let facts = s
                .storage
                .get_facts_many(&[cid])
                .await
                .map_err(|e| (-(ErrorCode::Internal as i64), e.to_string()))?;
            let fact = facts.into_iter().next().flatten().ok_or_else(|| {
                (
                    -(ErrorCode::CidNotFound as i64),
                    format!("no fact for cid={trimmed}"),
                )
            })?;
            Ok(json!({
                "schema": "emem.fetch.v1",
                "cid": trimmed,
                "fact": serde_json::to_value(&fact).unwrap_or(JsonValue::Null),
                "rest_url": format!("/v1/facts/{trimmed}"),
                "agent_hint": "Fact bytes are byte-identical across responders for the same CID; the CID itself is the validator. Verify the responder's signature with /v1/verify_receipt.",
            }))
        }
        "emem_backfill" => {
            let req: BackfillReq =
                serde_json::from_value(args).map_err(|e| (-32602, e.to_string()))?;
            backfill_inner(req, s)
                .await
                .map_err(|e| (-(e.1.code as i64), e.1.message))
        }
        other => Err((-32601, format!("unknown tool: {other}"))),
    }
}

// ── OpenAPI 3.1 (hand-rolled, agent-discoverable) ────────────────────────

async fn openapi() -> Json<JsonValue> {
    Json(json!({
        "openapi": "3.1.0",
        "info": {
            "title": "emem",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Agent-native, content-addressed Earth memory protocol. Every read returns a signed receipt. cell × band × tslot.",
            "license": { "name": "Apache-2.0" }
        },
        // Servers list is REQUIRED for OpenAI Custom GPTs and several
        // other agent platforms that won't pick up a tool spec without
        // an absolute base URL. We declare both the hosted instance and
        // a relative-base form so self-hosted deployments still work.
        "servers": [
            {"url": "https://emem.dev",  "description": "Hosted instance (HTTPS-only)"},
            {"url": "/",                 "description": "Same-origin (self-hosted or local)"}
        ],
        "paths": {
            "/health":               {"get":{"summary":"liveness","responses":{"200":{"description":"ok"}}}},
            "/.well-known/emem.json":{"get":{"summary":"protocol discovery","responses":{"200":{"description":"ok"}}}},
            "/v1/agent_card":        {"get":{"summary":"rich tool catalog with when-to-use","responses":{"200":{"description":"ok"}}}},
            "/v1/quickstart":        {"get":{"summary":"6-step playbook","responses":{"200":{"description":"ok"}}}},
            "/v1/manifests":         {"get":{"summary":"active manifest CIDs","responses":{"200":{"description":"ok"}}}},
            "/v1/bands":             {"get":{"summary":"band ontology","responses":{"200":{"description":"ok"}}}},
            "/v1/materializers":     {"get":{"summary":"per-band auto-fetch registry (which bands the responder will materialize on a recall miss)","responses":{"200":{"description":"ok"}}}},
            "/v1/data_availability": {"get":{"summary":"per-band temporal coverage catalog (window + tempo + kind + upstream wire path)","responses":{"200":{"description":"ok"}}}},
            "/v1/functions":         {"get":{"summary":"function registry","responses":{"200":{"description":"ok"}}}},
            "/v1/sources":           {"get":{"summary":"source registry","responses":{"200":{"description":"ok"}}}},
            "/v1/errors":            {"get":{"summary":"error code catalog","responses":{"200":{"description":"ok"}}}},
            "/v1/tools":             {"get":{"summary":"MCP tool descriptors with schemas","responses":{"200":{"description":"ok"}}}},
            "/v1/cells/{cell64}":    {"get":{"summary":"recall facts at a cell","parameters":[{"name":"cell64","in":"path","required":true,"schema":{"type":"string"}}],"responses":{"200":{"description":"ok"}}}},
            "/v1/recall":            {"post":{"summary":"recall facts","operationId":"emem_recall","requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/RecallReq"}}}},"responses":{"200":{"description":"ok"}}}},
            "/v1/query_region":      {"post":{"summary":"query region","operationId":"emem_query_region","requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/QueryRegionReq"}}}},"responses":{"200":{"description":"ok"}}}},
            "/v1/compare":           {"post":{"summary":"compare two cells","operationId":"emem_compare","requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/CompareReq"}}}},"responses":{"200":{"description":"ok"}}}},
            "/v1/find_similar":      {"post":{"summary":"k-NN over band vectors","operationId":"emem_find_similar","requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/FindSimilarReq"}}}},"responses":{"200":{"description":"ok"}}}},
            "/v1/diff":              {"post":{"summary":"derivative fact between two tslots","operationId":"emem_diff","requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/DiffReq"}}}},"responses":{"200":{"description":"ok"}}}},
            "/v1/trajectory":        {"post":{"summary":"time series","operationId":"emem_trajectory","requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/TrajectoryReq"}}}},"responses":{"200":{"description":"ok"}}}},
            "/v1/backfill":          {"post":{"summary":"materialize history in a window","operationId":"emem_backfill","requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/BackfillReq"}}}},"responses":{"200":{"description":"ok"}}}},
            "/v1/schema":            {"get":{"summary":"active CDDL/JSON schema bundle (REST mirror of emem_schema)","operationId":"emem_schema","responses":{"200":{"description":"ok"}}}},
            "/v1/facts/{cid}":       {"get":{"summary":"fetch a fact by CID (REST mirror of emem_fetch)","operationId":"emem_fetch","parameters":[{"name":"cid","in":"path","required":true,"schema":{"type":"string"}}],"responses":{"200":{"description":"ok"},"404":{"description":"no fact for cid"}}}},
            "/v1/verify":            {"post":{"summary":"verify a structured claim","operationId":"emem_verify","requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/VerifyReq"}}}},"responses":{"200":{"description":"ok"}}}},
            "/v1/verify_receipt":    {"post":{"summary":"offline-verify any responder's receipt","operationId":"emem_verify_receipt","responses":{"200":{"description":"ok"}}}},
            "/v1/intent":            {"post":{"summary":"intent → plan","operationId":"emem_intent","responses":{"200":{"description":"ok"}}}},
            "/v1/ask":               {"post":{"summary":"single-shot free-text answer with signed evidence","operationId":"emem_ask","requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/AskReq"}}}},"responses":{"200":{"description":"ok"}}}},
            "/v1/attest":            {"post":{"summary":"submit signed attestation (JSON)","operationId":"emem_attest","responses":{"200":{"description":"ok"}}}},
            "/v1/attest_cbor":       {"post":{"summary":"submit signed attestation (canonical CBOR)","operationId":"emem_attest_cbor","responses":{"200":{"description":"ok"}}}},
            "/v1/facts/{cid}":       {"get":{"summary":"fact dereference (immutable, ETag-tagged)","operationId":"emem_get_fact","parameters":[{"name":"cid","in":"path","required":true,"schema":{"type":"string"}}],"responses":{"200":{"description":"ok"},"304":{"description":"unchanged"},"404":{"description":"not found"}}}},
            "/mcp":                  {"post":{"summary":"MCP JSON-RPC 2.0","operationId":"mcp_jsonrpc","responses":{"200":{"description":"ok"}}}},
            // High-traffic endpoints that were previously discoverable
            // only via /v1/discover or the agent_card. OpenAI Custom GPT
            // and ChatGPT plugin pickers ignore endpoints not in
            // `paths`, so any tool we want them to route to MUST appear
            // here.
            "/v1/locate":            {"post":{"summary":"resolve a place name (or lat/lng) to a cell64","operationId":"emem_locate","requestBody":{"required":true,"content":{"application/json":{"schema":{"type":"object","properties":{"query":{"type":"string"},"lat":{"type":"number"},"lng":{"type":"number"}}}}}},"responses":{"200":{"description":"ok"}}}},
            "/v1/recall_many":       {"post":{"summary":"bulk recall over up to 256 cells per call","operationId":"emem_recall_many","requestBody":{"required":true,"content":{"application/json":{"schema":{"type":"object","required":["cells"],"properties":{"cells":{"type":"array","items":{"type":"string"}},"bands":{"type":"array","items":{"type":"string"}}}}}}},"responses":{"200":{"description":"ok"}}}},
            "/v1/recall_polygon":    {"post":{"summary":"recall facts inside a GeoJSON polygon","operationId":"emem_recall_polygon","responses":{"200":{"description":"ok"}}}},
            "/v1/grid_info":         {"get":{"summary":"declare the active spatial grid (cell64 / Hilbert / future H3)","operationId":"emem_grid_info","responses":{"200":{"description":"ok"}}}},
            "/v1/algorithms":        {"get":{"summary":"composition recipe registry (formulas that fuse band facts)","operationId":"emem_algorithms","responses":{"200":{"description":"ok"}}}},
            "/v1/algorithms/{key}":  {"get":{"summary":"single algorithm detail","parameters":[{"name":"key","in":"path","required":true,"schema":{"type":"string"}}],"responses":{"200":{"description":"ok"}}}},
            "/v1/compare_bands":     {"post":{"summary":"per-band diff between two cells (delta + percent change)","operationId":"emem_compare_bands","responses":{"200":{"description":"ok"}}}},
            "/v1/coverage_matrix":   {"get":{"summary":"per-band facts_count + has_materializer + last_attested_at","operationId":"emem_coverage_matrix","responses":{"200":{"description":"ok"}}}},
            "/v1/coverage":          {"get":{"summary":"JSON snapshot of where data lives (cells + lat/lng + counts)","responses":{"200":{"description":"ok"}}}},
            "/v1/coverage_map.svg":  {"get":{"summary":"SVG render of corpus density (image/svg+xml)","responses":{"200":{"description":"ok"}}}},
            "/v1/fleet":             {"get":{"summary":"satellite/sensor lineage feeding each band","responses":{"200":{"description":"ok"}}}},
            "/v1/cells/{cell64}/info":     {"get":{"summary":"cell64 introspection (centroid, bbox, neighbors)","parameters":[{"name":"cell64","in":"path","required":true,"schema":{"type":"string"}}],"responses":{"200":{"description":"ok"}}}},
            "/v1/cells/{cell64}/geojson":  {"get":{"summary":"cell polygon as GeoJSON","parameters":[{"name":"cell64","in":"path","required":true,"schema":{"type":"string"}}],"responses":{"200":{"description":"ok"}}}},
            "/v1/cells/{cell64}/scene.png":{"get":{"summary":"Sentinel-2 true-colour thumbnail (256×256 PNG)","parameters":[{"name":"cell64","in":"path","required":true,"schema":{"type":"string"}}],"responses":{"200":{"description":"ok"}}}},
            "/v1/elevation":         {"post":{"summary":"convenience elevation lookup (uses copdem30m / gmrt fallback)","responses":{"200":{"description":"ok"}}}},
            "/v1/discover":          {"get":{"summary":"machine-readable index of all surfaces","responses":{"200":{"description":"ok"}}}},
            "/v1/reviews":           {"post":{"summary":"submit task-outcome review keyed by subject","responses":{"200":{"description":"ok"}}}},
            "/v1/reviews/{subject_id}":{"get":{"summary":"list reviews for a subject","parameters":[{"name":"subject_id","in":"path","required":true,"schema":{"type":"string"}}],"responses":{"200":{"description":"ok"}}}},
            "/v1/contributors":      {"get":{"summary":"list of contributing pubkeys + per-band fact counts","responses":{"200":{"description":"ok"}}}},
            "/v1/contributors/{pubkey_b32}": {"get":{"summary":"contributor profile by pubkey","parameters":[{"name":"pubkey_b32","in":"path","required":true,"schema":{"type":"string"}}],"responses":{"200":{"description":"ok"}}}},
            "/v1/agent_stats":       {"get":{"summary":"per-tool MCP latency + error counts","responses":{"200":{"description":"ok"}}}},
            "/v1/demos":             {"get":{"summary":"index of pre-recorded demo runs (live signed receipts)","responses":{"200":{"description":"ok"}}}}
        },
        "components": {
            "schemas": {
                "RecallReq":       {"type":"object","required":["cell"],"properties":{"cell":{"type":"string","description":"cell64 string"},"bands":{"type":"array","items":{"type":"string"}},"tslot":{"type":"integer"}}},
                "QueryRegionReq":  {"type":"object","required":["geometry"],"properties":{"geometry":{"type":"string"},"bands":{"type":"array","items":{"type":"string"}},"agg":{"type":"string","enum":["mean","median","p90","vector_centroid"]}}},
                "CompareReq":      {"type":"object","required":["a","b"],"properties":{"a":{"type":"string"},"b":{"type":"string"},"family":{"type":"string"}}},
                "FindSimilarReq":  {"type":"object","required":["key"],"properties":{"key":{"type":"string"},"k":{"type":"integer","minimum":1,"maximum":1000},"band":{"type":"string"}}},
                "DiffReq":         {"type":"object","required":["cell","band","tslot_a","tslot_b"],"properties":{"cell":{"type":"string"},"band":{"type":"string"},"tslot_a":{"type":"integer"},"tslot_b":{"type":"integer"}}},
                "TrajectoryReq":   {"type":"object","required":["cell","band","window"],"properties":{"cell":{"type":"string"},"band":{"type":"string"},"window":{"type":"array","items":{"type":"integer"},"minItems":2,"maxItems":2}}},
                "VerifyReq":       {"type":"object","required":["claim","cell"],"properties":{"cell":{"type":"string"},"mode":{"type":"string","enum":["fast","resolve","zk"]},"claim":{"type":"object"}}},
                "AskReq":          {"type":"object","required":["q"],"properties":{"q":{"type":"string"},"place":{"type":"string"},"cell":{"type":"string"},"lat":{"type":"number"},"lng":{"type":"number"},"include_image":{"type":"boolean","default":false}}}
            }
        }
    }))
}

/// Compute the bands_cid + sources_cid for the default in-process registries.
pub fn default_manifest_cids() -> (String, String) {
    let bands = manifest_cid(&*emem_core::bands::DEFAULT).unwrap_or_default();
    let sources = manifest_cid(&*emem_core::sources::DEFAULT).unwrap_or_default();
    (bands, sources)
}

// ── Demo trace browsing (var/demos/) ─────────────────────────────────────

fn demos_root() -> std::path::PathBuf {
    std::env::var("EMEM_DEMOS_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("var/demos"))
}

/// List every demo run directory under `EMEM_DEMOS_DIR` (default `var/demos`).
async fn list_demos() -> Json<JsonValue> {
    let root = demos_root();
    let mut runs: Vec<JsonValue> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&root) {
        for ent in rd.flatten() {
            let name = ent.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            if !ent.path().is_dir() {
                continue;
            }
            let mut steps = 0usize;
            let mut bytes = 0u64;
            if let Ok(rd2) = std::fs::read_dir(ent.path()) {
                for f in rd2.flatten() {
                    if let Ok(md) = f.metadata() {
                        bytes += md.len();
                        let fname = f.file_name().to_string_lossy().into_owned();
                        if fname.ends_with(".resp.json") {
                            steps += 1;
                        }
                    }
                }
            }
            let trace_exists = ent.path().join("trace.json").exists();
            runs.push(serde_json::json!({
                "id": name,
                "steps": steps,
                "bytes": bytes,
                "has_trace": trace_exists,
                "url": format!("/v1/demos/{name}"),
            }));
        }
    }
    runs.sort_by(|a, b| {
        b["id"]
            .as_str()
            .unwrap_or("")
            .cmp(a["id"].as_str().unwrap_or(""))
    });
    Json(serde_json::json!({
        "root": root.to_string_lossy(),
        "count": runs.len(),
        "runs": runs,
    }))
}

/// Return a single run's `trace.json` (the primary index for that run).
async fn get_demo_index(Path(run): Path<String>) -> Response {
    if !is_safe_id(&run) {
        return not_found("invalid run id");
    }
    let p = demos_root().join(&run).join("trace.json");
    serve_demo_path(p, "application/json")
}

/// Return a single per-step file from a run.
async fn get_demo_file(Path((run, file)): Path<(String, String)>) -> Response {
    if !is_safe_id(&run) || !is_safe_id(&file) {
        return not_found("invalid path");
    }
    let p = demos_root().join(&run).join(&file);
    let mime = if file.ends_with(".cbor") {
        "application/cbor"
    } else if file.ends_with(".md") {
        "text/markdown; charset=utf-8"
    } else {
        "application/json"
    };
    serve_demo_path(p, mime)
}

fn is_safe_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() < 200
        && !s.contains("..")
        && !s.contains('/')
        && !s.contains('\\')
        && !s.starts_with('.')
}

fn serve_demo_path(p: std::path::PathBuf, mime: &'static str) -> Response {
    match std::fs::read(&p) {
        Ok(bytes) => Response::builder()
            .status(StatusCode::OK)
            .header("content-type", mime)
            .header("cache-control", "public, max-age=300")
            .body(axum::body::Body::from(bytes))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()),
        Err(_) => not_found("file not found"),
    }
}

// ── /api alias + /v1/discover (one-call agent bootstrap) ────────────────
//
// `/api` is a 308 redirect to `/v1/agent_card` — agents that probe the
// conventional `/api` slug land in the right place.
//
// `/v1/discover` is the *one* call a new agent should make. It returns
// the agent_card, manifests, a curated set of canonical "famous places"
// the agent can recall against to verify the loop works, and a list of
// active contributors. Replaces a five-call onboarding dance with one.

async fn api_alias() -> Response {
    Response::builder()
        .status(StatusCode::PERMANENT_REDIRECT)
        .header("location", "/v1/agent_card")
        .body(axum::body::Body::empty())
        .unwrap_or_else(|_| StatusCode::PERMANENT_REDIRECT.into_response())
}

/// Curated landmarks an agent can use to bootstrap and verify the protocol
/// without leaving the page. Each entry maps a globally-recognised name to
/// the canonical cell64 produced by `emem-codec::cell_from_latlng`. This
/// list is editorial; new entries should ship with at least one attested
/// fact so `recall` returns provenance, not silence.
fn canonical_places() -> Vec<(&'static str, f64, f64)> {
    vec![
        ("Mount Fuji", 35.3606, 138.7274),
        ("Mount Everest", 27.9881, 86.9250),
        ("Grand Canyon", 36.1069, -112.1129),
        ("Tokyo, Japan", 35.6762, 139.6503),
        ("New York, USA", 40.7128, -74.0060),
        ("São Paulo, Brazil", -23.5505, -46.6333),
        ("Lagos, Nigeria", 6.5244, 3.3792),
        ("Sydney, Australia", -33.8688, 151.2093),
        ("Reykjavík, Iceland", 64.1466, -21.9426),
    ]
}

async fn discover(State(s): State<AppState>) -> Json<JsonValue> {
    let card = agent_card(State(s.clone())).await;
    let manifests = manifests(State(s.clone())).await;
    let bands = bands().await;
    // Surface a slim algorithm-registry summary inline so the cold-start
    // bootstrap is genuinely one-call. Full bodies live at /v1/algorithms.
    let alg_reg = &*emem_core::algorithms::DEFAULT;
    let alg_summary: Vec<JsonValue> = alg_reg
        .algorithms
        .iter()
        .map(|a| {
            json!({
                "key":    a.key,
                "kind":   a.kind,
                "domain": a.domain,
                "when_to_use": a.when_to_use.chars().take(140).collect::<String>(),
            })
        })
        .collect();
    let alg_cid = emem_core::manifest::manifest_cid(alg_reg).ok();
    let mut places: Vec<JsonValue> = Vec::new();
    for (name, lat, lng) in canonical_places() {
        let cell = emem_codec::cell64_from_latlng(lat, lng);
        places.push(json!({
            "name": name,
            "lat": lat,
            "lng": lng,
            "cell64": cell,
            "recall": format!("/v1/recall body {{\"cell\":\"{cell}\"}}"),
        }));
    }
    let pubkey = data_encoding::BASE32_NOPAD
        .encode(&s.identity.pubkey.0)
        .to_lowercase();
    Json(json!({
        "schema": "emem.discover.v1",
        "tagline": "Cite-able, content-addressed, signed memory of every place on Earth.",
        "responder_pubkey_b32": pubkey,
        "responder_key_epoch": s.identity.epoch.0,
        "manifests": manifests.0,
        "bands": bands.0,
        "algorithms": {
            "_purpose": "Composition recipes (flood_risk, walkability, embedding_novelty, …) that fuse attested band facts into derived scores. Cite algorithm_cid + fact_cids for reproducibility. Full bodies at GET /v1/algorithms.",
            "algorithms_cid": alg_cid,
            "count": alg_reg.algorithms.len(),
            "summary": alg_summary,
        },
        "agent_card": card.0,
        "canonical_places": places,
        "next_calls": [
            {"call":"POST /v1/locate", "use":"map a place name or lat/lng to a cell64; returns data_at_this_cell with bands AND algorithms grouped by topic"},
            {"call":"POST /v1/recall", "use":"read the facts known about a cell; auto-materializes on miss for any wired band"},
            {"call":"GET  /v1/algorithms/:key", "use":"fetch one composition recipe (formula + inputs + citation); apply in-process and cite algorithm_cid in the receipt"},
            {"call":"POST /v1/compare", "use":"score two cells against a band family"},
            {"call":"POST /v1/find_similar", "use":"k-NN over the corpus"},
            {"call":"GET  /v1/cells/:cell64/scene.png", "use":"true-colour Sentinel-2 RGB thumbnail (multimodal); also via MCP `emem_cell_scene_rgb`"},
            {"call":"POST /v1/attest_cbor", "use":"contribute facts to the shared memory"},
            {"call":"POST /v1/verify_receipt", "use":"audit any responder's signed receipt"},
            {"call":"GET  /v1/grid_info", "use":"declared resolution, DGGS lineage, H3/S2 interop"},
            {"call":"GET  /v1/agent_stats", "use":"by-family request counts, latency p50/p95/p99"},
        ],
        "human_readable_doc": "/agents.md",
        "llm_optimised_doc":  "/llms-full.txt",
        "openapi":            "/openapi.json",
        "mcp":                "/mcp",
        "metrics":            "/metrics",
        "leaderboard":        "/v1/contributors",
        "source":             "https://github.com/Vortx-AI/emem",
        "license":            "Apache-2.0",
    }))
}

// ── /v1/locate + /v1/cells/:cell64/info (agent address-book) ────────────
//
// Agents typically have a lat/lng or a place name; the protocol's address
// space is cell64. These two endpoints make the bridge so an agent can go
// from "what's at Mt. Fuji?" → cell64 → recall, or back from cell64 →
// lat/lng/bbox for a human-readable answer.

#[derive(Deserialize)]
struct LocateReq {
    /// (lat, lng) in WGS-84 degrees. Either this or `place` required.
    #[serde(default)]
    lat: Option<f64>,
    #[serde(default)]
    lng: Option<f64>,
    /// Free-text place name; resolved via OSM Nominatim (open data).
    /// Accepts `q` as an alias because that's the de-facto convention
    /// across OSM, Google Geocoding, Mapbox, etc. Agents transferring
    /// patterns from those APIs land on the right field either way.
    #[serde(default, alias = "q", alias = "query", alias = "name")]
    place: Option<String>,
}

async fn post_locate(Json(req): Json<LocateReq>) -> Result<Json<JsonValue>, ApiError> {
    locate_inner(req).await
}

/// One cell-typed field of a primitive request after geocoding. Either
/// the input was already a cell64 (`Cell`) or the responder ran a
/// locate to resolve it (`Place`). Carried into the per-handler
/// response under `resolved_from` so the agent can see exactly which
/// place name produced the cell64 it just queried.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResolvedRef {
    /// Already a cell64.
    Cell,
    /// Resolved from a free-text place name.
    Place {
        input: String,
        label: Option<String>,
        lat: f64,
        lng: f64,
        via: String,
    },
}

/// Resolve a cell-typed field. If the string is already shaped like a
/// cell64 we keep it; otherwise we treat it as a place name and run
/// `locate_inner`. Geocoding failures bubble up as 400 / 502 from
/// locate so the agent can correct the call without a second round-trip.
pub(crate) async fn resolve_cell_field(s: &str) -> Result<(String, ResolvedRef), ApiError> {
    if emem_codec::is_cell64_shape(s) {
        return Ok((s.to_string(), ResolvedRef::Cell));
    }
    let lr = LocateReq {
        lat: None,
        lng: None,
        place: Some(s.to_string()),
    };
    let resp = locate_inner(lr).await?;
    let body = &resp.0;
    let cell = body.get("cell64").and_then(|v| v.as_str())
        .ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, ErrorBody {
            code: ErrorCode::Internal,
            message: format!("could not resolve '{s}' to a cell64; pass a cell64 string or a recognisable place name"),
        }))?
        .to_string();
    let label = body
        .get("place_label")
        .and_then(|v| v.as_str())
        .map(String::from);
    let lat = body
        .get("lat_input")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let lng = body
        .get("lng_input")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let via = body
        .get("via")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    Ok((
        cell,
        ResolvedRef::Place {
            input: s.to_string(),
            label,
            lat,
            lng,
            via,
        },
    ))
}

/// Same as `resolve_cell_field` but only returns the cell64 — convenient
/// for handlers that want the substitution but don't surface the
/// resolved-from envelope.
#[allow(dead_code)]
pub(crate) async fn resolve_cell_only(s: &str) -> Result<String, ApiError> {
    Ok(resolve_cell_field(s).await?.0)
}

async fn get_locate(
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Json<JsonValue>, ApiError> {
    let lat = q.get("lat").and_then(|s| s.parse::<f64>().ok());
    let lng = q.get("lng").and_then(|s| s.parse::<f64>().ok());
    // Same alias-set as the POST body deserialization: `place`, `q`,
    // `query`, `name` are all accepted so an agent doesn't fail merely
    // because they used a synonym from a different geocoder API.
    let place = q
        .get("place")
        .or_else(|| q.get("q"))
        .or_else(|| q.get("query"))
        .or_else(|| q.get("name"))
        .cloned();
    locate_inner(LocateReq { lat, lng, place }).await
}

/// `GET /v1/grid_info` — declares the active grid's actual resolution,
/// the spec target, and the DGGS lineage so an agent that already knows
/// H3 / S2 can decide whether to convert or pre-snap. Honest about the
/// gap between current build and spec.
async fn grid_info() -> Json<JsonValue> {
    Json(json!({
        "schema": "emem.grid_info.v1",
        "active_encoding": {
            "name": "cell64-hilbert16",
            "kind": "raster, locality-preserving Hilbert curve",
            "lat_lng_bits": 16,
            "encoded_string_form": "four base-1024 bigrams joined by '.', e.g. damO.zb000.xUti.zde79",
            "string_length_chars": 18,
            "ground_resolution": {
                "lat_axis_deg":   0.00275,
                "lng_axis_deg":   0.00549,
                "lat_axis_metres_at_equator":  305.0,
                "lng_axis_metres_at_equator":  611.0,
                "lat_axis_metres_at_lat_60":   305.0,
                "lng_axis_metres_at_lat_60":   305.0,
                "comment": "Latitude is uniform; longitude varies with cos(lat) so high-latitude cells are physically smaller in the lng axis."
            },
            "domain": {
                "lat_deg": [-90.0, 90.0],
                "lng_deg": [-180.0, 180.0],
                "antimeridian_handled": true,
                "poles_handled": true
            }
        },
        "spec_target": {
            "name": "aperture-7 hex DGGS (per docs/SPEC.md §3)",
            "default_resolution": 13,
            "edge_length_m": 3.41,
            "cell_area_m2": 30.2,
            "h3_compatibility": "Reference implementations MAY use Uber H3 ≥4.0 as a backend if outputs pass the cell.* test vectors (SPEC §19). H3 is not normatively cited in the wire format.",
            "s2_compatibility": "Not declared. Conversion to S2 cell IDs is straightforward via lat/lng but not built in.",
            "status": "Spec target not yet active in this build. Migration to hex H3 backend is planned; today the responder serves cells at the cell64-hilbert16 resolution above."
        },
        "interop": {
            "to_h3":  "Decode cell64 → (lat, lng) via /v1/cells/{cell64}/info, then call h3.geo_to_h3(lat, lng, res) client-side.",
            "to_s2":  "Decode cell64 → (lat, lng), then S2.CellId.from_lat_lng(...) at the desired level.",
            "from_h3":"Use h3.h3_to_geo(h3_id) for the centre, then POST /v1/locate with {lat, lng}."
        },
        "honest_warnings": [
            "Cell granularity is ~305 m, not 30 m and not 10 m. Earlier docs and the locate `advice` string used ~30 m loosely; that has been corrected.",
            "Two callers asking about the same place from slightly different (lat, lng) inputs will land in adjacent cells. Use /v1/locate's `neighborhood_cells` for fan-out before concluding a place is empty.",
            "When the migration to hex H3 lands, current cell64 strings remain valid but new strings will be issued under a new mode prefix; receipts pin the active manifest CIDs so historical answers don't drift."
        ],
        "next": [
            "GET  /v1/cells/{cell64}/info  — lat/lng/bbox for any cell",
            "POST /v1/locate                — (lat,lng) or place name → cell64",
            "GET  /v1/discover              — full bootstrap"
        ]
    }))
}

// ── /v1/elevation — read-through to Open-Meteo (Copernicus DEM 90m) ───
//
// The honest answer to "this protocol returns empty for any cell where no
// agent has attested" is lazy materialization: when an agent asks about
// elevation, fetch from an open-data provider and serve. This is the
// minimal viable form — a read-through pass-through that returns a real
// value for any (lat, lng) on Earth without requiring an attester to
// have walked there first.
//
// Open-Meteo wraps Copernicus DEM 90 m and is rate-limit-free for
// reasonable use. For agents that need cite-able receipts (not just a
// number), recall a real attested fact via /v1/recall first; this
// endpoint is the convenience layer for "I just need a number now."

#[derive(Deserialize)]
struct ElevationReq {
    #[serde(default)]
    lat: Option<f64>,
    #[serde(default)]
    lng: Option<f64>,
    /// Either `cell64` or `cell` accepted; decoded via emem_codec.
    #[serde(default, alias = "cell")]
    cell64: Option<String>,
}

async fn post_elevation(Json(req): Json<ElevationReq>) -> Result<Json<JsonValue>, ApiError> {
    let (lat, lng, source_kind) = match (req.lat, req.lng, req.cell64.as_deref()) {
        (Some(la), Some(lo), _) => (la, lo, "input_latlng"),
        (_, _, Some(c)) => {
            let info = emem_codec::latlng_from_cell64(c).map_err(|e| {
                ApiError(
                    StatusCode::BAD_REQUEST,
                    ErrorBody {
                        code: ErrorCode::InvalidCell,
                        message: format!("cell64 decode: {e}"),
                    },
                )
            })?;
            (info.lat_deg, info.lng_deg, "cell64_centre")
        }
        _ => {
            return Err(ApiError(
                StatusCode::BAD_REQUEST,
                ErrorBody {
                    code: ErrorCode::Internal,
                    message: "supply (lat,lng) or cell64".into(),
                },
            ))
        }
    };
    let url =
        format!("https://api.open-meteo.com/v1/elevation?latitude={lat:.6}&longitude={lng:.6}",);
    let cli = reqwest_client();
    let resp = cli.get(&url).send().await.map_err(|e| {
        ApiError(
            StatusCode::BAD_GATEWAY,
            ErrorBody {
                code: ErrorCode::SourceFetchFailed,
                message: format!("open-meteo https: {e}"),
            },
        )
    })?;
    if !resp.status().is_success() {
        return Err(ApiError(
            StatusCode::BAD_GATEWAY,
            ErrorBody {
                code: ErrorCode::SourceFetchFailed,
                message: format!("open-meteo status {}", resp.status()),
            },
        ));
    }
    let body: JsonValue = resp.json().await.map_err(|e| {
        ApiError(
            StatusCode::BAD_GATEWAY,
            ErrorBody {
                code: ErrorCode::SourceFormatMismatch,
                message: format!("open-meteo json: {e}"),
            },
        )
    })?;
    let elev_m = body
        .get("elevation")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|v| v.as_f64())
        .ok_or_else(|| {
            ApiError(
                StatusCode::BAD_GATEWAY,
                ErrorBody {
                    code: ErrorCode::SourceFormatMismatch,
                    message: "open-meteo response missing elevation[0]".into(),
                },
            )
        })?;
    let cell64 = emem_codec::cell64_from_latlng(lat, lng);
    Ok(Json(json!({
        "schema": "emem.elevation.v1",
        "lat": lat,
        "lng": lng,
        "lat_lng_source": source_kind,
        "cell64": cell64,
        "elevation_m": elev_m,
        "unit": "m",
        "source": {
            "scheme": "open_meteo",
            "wraps":  "Copernicus DEM 90 m",
            "url":    url,
            "license": "Copernicus DEM is free under Copernicus Data Licence; Open-Meteo redistributes under their terms (see open-meteo.com/license).",
        },
        "honest_caveat": "This is a read-through pass-through, not a signed attestation. The value is not in our hot cache and is not cite-able via /v1/verify_receipt. For cite-able answers, use /v1/recall on an attested cell. To make this value cite-able, attest it yourself via /v1/attest_cbor — see /docs/ATTESTING.md.",
        "next": [
            "POST /v1/locate    — get the cell64 + neighborhood for this place",
            "POST /v1/recall    — see if any agent has attested this cell",
            "POST /v1/attest_cbor — promote this read-through into a signed fact"
        ]
    })))
}

// ── Lazy materialization (the read → attest → cache loop) ───────────────
//
// When an agent calls /v1/recall for `copdem30m.elevation_mean` on a cell
// no one has attested yet, the responder fetches Open-Meteo, signs a
// Primary fact under its own identity, persists it via storage layer,
// then returns it. Future calls hit the sled hot cache. Net effect: ANY
// geo cell on Earth answers elevation cite-ably, without an external
// attester having walked there first.
//
// Honesty:
// - The fact's `derivation.fn_key = "open_meteo_copdem90m@1"` declares the
//   exact function used. A skeptical verifier downweights non-deterministic
//   derivations; this one is deterministic up to the upstream provider's
//   stability.
// - The signer is the responder's pubkey (the same one that signs receipts).
//   Agents already trust this key for receipts; an attestation under the
//   same key extends that trust to the materialized fact.
// - Operators disable via `EMEM_AUTO_MATERIALIZE=0`. Off by default in
//   isolated test environments; on for the public emem.dev responder.

fn auto_materialize_enabled() -> bool {
    std::env::var("EMEM_AUTO_MATERIALIZE")
        .map(|v| v != "0" && v.to_lowercase() != "false")
        .unwrap_or(true)
}

/// Outcome of an elevation materialization attempt:
///   - `Primary(cid)` — Cop-DEM returned a non-zero land elevation; a
///     signed Primary fact landed in storage.
///   - `Absence(cid)` — Cop-DEM returned 0 m (no-data marker over water)
///     OR Open-Meteo errored; a signed *NegativeFact* now records the
///     absence with a content-addressed reason. Future recalls on this
///     cell hit the hot cache and short-circuit, no upstream re-fetch.
enum ElevationMaterialization {
    Primary(emem_fact::FactCid),
    Absence(emem_fact::FactCid),
}

/// Build a content-addressed `ReasonCid` over a canonical UTF-8
/// reason string. Same reason → same CID — that's the whole point of
/// content addressing applied to absence justifications. The 16-byte
/// truncation matches FactCid's wire form (base32-nopad-lowercase).
fn reason_cid_for(reason: &str) -> ReasonCid {
    let h = blake3::hash(reason.as_bytes());
    let cid = data_encoding::BASE32_NOPAD
        .encode(&h.as_bytes()[..16])
        .to_lowercase();
    ReasonCid::new(cid)
}

/// Fetch elevation from Open-Meteo (Cop-DEM 90 m wrap), build a signed
/// attestation under the responder's identity, store via the storage
/// layer, return what was materialized. On a successful land query a
/// Primary fact is signed; on Cop-DEM no-data (value=0 over water) or
/// upstream error a NegativeFact is signed with a content-addressed
/// reason. Either way the next /v1/recall on this cell hits the hot
/// cache and serves the result from sled without re-fetching upstream.
async fn materialize_elevation_mean(
    cell64: &str,
    s: &AppState,
) -> Result<ElevationMaterialization, String> {
    // 1. Decode cell → centre lat/lng.
    let info = emem_codec::latlng_from_cell64(cell64).map_err(|e| format!("cell decode: {e}"))?;
    let lat = info.lat_deg;
    let lng = info.lng_deg;

    // 2. Call Open-Meteo. (Same client + UA as /v1/elevation.)
    let url =
        format!("https://api.open-meteo.com/v1/elevation?latitude={lat:.6}&longitude={lng:.6}",);
    let resp_result = reqwest_client().get(&url).send().await;
    let signed_at = chrono_iso8601_utc();

    let elev_m = match resp_result {
        Ok(resp) if resp.status().is_success() => {
            let body: JsonValue = resp
                .json()
                .await
                .map_err(|e| format!("open-meteo json: {e}"))?;
            body.get("elevation")
                .and_then(|v| v.as_array())
                .and_then(|a| a.first())
                .and_then(|v| v.as_f64())
                .ok_or_else(|| "open-meteo response missing elevation[0]".to_string())?
        }
        Ok(resp) => {
            // Upstream error (e.g. 502 above the polar circle). Sign an
            // Absence fact so subsequent recalls short-circuit.
            let reason = format!(
                "upstream_no_coverage: copdem30m.elevation_mean lookup at ({lat:.6},{lng:.6}) returned HTTP {} from open-meteo; Cop-DEM 90m has no global coverage above |lat|≈85° and at certain Antarctic interiors, so this cell is recorded as a confirmed absence rather than re-fetched on every call.",
                resp.status(),
            );
            let cid = sign_elevation_absence(cell64, s, &url, &signed_at, &reason).await?;
            return Ok(ElevationMaterialization::Absence(cid));
        }
        Err(e) => {
            // Network error — DON'T persist an absence (we don't know
            // if the cell genuinely has no coverage; might be transient).
            return Err(format!("open-meteo https: {e}"));
        }
    };

    // Cop-DEM is a *land* digital elevation model: ocean cells return
    // exactly 0 m by design. Signing 0 m as elevation_mean would be
    // verifiable but factually wrong by up to 11 km (Mariana Trench).
    // Materialize a NegativeFact instead so the agent gets a signed,
    // cite-able absence-of-land-elevation here, and a future
    // gebco.bathymetry_mean materializer can supply real depth.
    if elev_m == 0.0 {
        let reason = format!(
            "ocean_or_no_land_dem: open-meteo cop-dem 90m returned exactly 0 m at ({lat:.6},{lng:.6}). Cop-DEM is a land DEM and uses 0 as its no-data marker over water. This cell is recorded as a confirmed absence for band copdem30m.elevation_mean; a future gebco.bathymetry_mean materializer will provide bathymetric depth here."
        );
        let cid = sign_elevation_absence(cell64, s, &url, &signed_at, &reason).await?;
        return Ok(ElevationMaterialization::Absence(cid));
    }

    // 3. Build the Primary fact. The schema_cid binds the meaning of this
    //    fact to the responder's active manifest at materialization time.
    let fact = Fact::Primary(PrimaryFact {
        cell: cell64.to_string(),
        band: "copdem30m.elevation_mean".into(),
        tslot: 0,
        value: ciborium::Value::Float(elev_m),
        unit: Some("m".into()),
        confidence: 0.95,
        uncertainty: None,
        sources: vec![Source {
            scheme: "open_meteo".into(),
            id: url.clone(),
            cid: None,
            hash: None,
            captured_at: Some(signed_at.clone()),
            url: None,
        }],
        derivation: Derivation {
            fn_key: "open_meteo_copdem90m@1".into(),
            args: Some(ciborium::Value::Array(vec![
                ciborium::Value::Float(lat),
                ciborium::Value::Float(lng),
            ])),
        },
        privacy_class: "public".into(),
        schema_cid: SchemaCid::new(s.manifests.schema_cid.as_str()),
        signer: s.identity.pubkey,
        signed_at: signed_at.clone(),
    });

    // 4. Compute the merkle root via emem_attest::merkle_root over the
    //    sorted leaf hashes (verify_attestation re-runs this exact path).
    let mut buf = Vec::new();
    ciborium::ser::into_writer(&fact, &mut buf).map_err(|e| format!("cbor encode: {e}"))?;
    let leaf_hash = blake3::hash(&buf);
    let mut leaf = [0u8; 32];
    leaf.copy_from_slice(leaf_hash.as_bytes());
    let batch_root = emem_attest::merkle_root(&[leaf]);

    // 5. Sign blake3(batch_root || registry_cid || schema_cid) with the
    //    responder's identity key.
    let mut h = blake3::Hasher::new();
    h.update(&batch_root);
    h.update(s.manifests.registry_cid.as_str().as_bytes());
    h.update(s.manifests.schema_cid.as_str().as_bytes());
    let signed_digest = h.finalize();
    let sig = s.identity.signing.sign(signed_digest.as_bytes());
    let mut sig_bytes = [0u8; 64];
    sig_bytes.copy_from_slice(&sig.to_bytes());

    let att = Attestation {
        facts: vec![fact],
        batch_root,
        attester: s.identity.pubkey,
        attester_key_epoch: KeyEpoch(s.identity.epoch.0),
        registry_cid: RegistryCid::new(s.manifests.registry_cid.as_str()),
        schema_cid: SchemaCid::new(s.manifests.schema_cid.as_str()),
        stake: None,
        signature: EmCoreSignature(sig_bytes),
        attested_at: signed_at,
    };

    // 6. Persist. Storage layer recomputes the root, validates the
    //    signature, and commits to sled. Future recalls hit hot cache.
    let cids = s
        .storage
        .put_attestation(&att)
        .await
        .map_err(|e| format!("put_attestation: {e}"))?;

    let cid = cids
        .into_iter()
        .next()
        .ok_or_else(|| "put_attestation returned no fact_cid".to_string())?;
    Ok(ElevationMaterialization::Primary(cid))
}

/// Fetch topo-bathymetric elevation from GMRT (Global Multi-Resolution
/// Topography), build a signed Primary fact under the responder's
/// identity, and persist it. GMRT returns negative values over water
/// (real bathymetric depth) and positive values over land in a single
/// scientific dataset, so this band answers cite-ably for any cell on
/// Earth — including the Mariana Trench.
///
/// Reference: <https://www.gmrt.org/services/PointServer> — peer-
/// reviewed, no auth, maintained by Lamont-Doherty Earth Observatory.
async fn materialize_gmrt_topobathy(
    cell64: &str,
    s: &AppState,
) -> Result<emem_fact::FactCid, String> {
    let info = emem_codec::latlng_from_cell64(cell64).map_err(|e| format!("cell decode: {e}"))?;
    let lat = info.lat_deg;
    let lng = info.lng_deg;
    let url = format!(
        "https://www.gmrt.org/services/PointServer?latitude={lat:.6}&longitude={lng:.6}&format=text/plain",
    );
    let resp = reqwest_client()
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("gmrt https: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("gmrt status {}", resp.status()));
    }
    let body = resp.text().await.map_err(|e| format!("gmrt body: {e}"))?;
    let elev_m: f64 = body
        .trim()
        .parse()
        .map_err(|e| format!("gmrt non-numeric body {body:?}: {e}"))?;

    let signed_at = chrono_iso8601_utc();
    let fact = Fact::Primary(PrimaryFact {
        cell: cell64.to_string(),
        band: "gmrt.topobathy_mean".into(),
        tslot: 0,
        value: ciborium::Value::Float(elev_m),
        unit: Some("m".into()),
        confidence: 0.9,
        uncertainty: None,
        sources: vec![Source {
            scheme: "gmrt".into(),
            id: url.clone(),
            cid: None,
            hash: None,
            captured_at: Some(signed_at.clone()),
            url: None,
        }],
        derivation: Derivation {
            fn_key: "gmrt_pointserver@1".into(),
            args: Some(ciborium::Value::Array(vec![
                ciborium::Value::Float(lat),
                ciborium::Value::Float(lng),
            ])),
        },
        privacy_class: "public".into(),
        schema_cid: SchemaCid::new(s.manifests.schema_cid.as_str()),
        signer: s.identity.pubkey,
        signed_at: signed_at.clone(),
    });

    let mut buf = Vec::new();
    ciborium::ser::into_writer(&fact, &mut buf).map_err(|e| format!("cbor encode: {e}"))?;
    let leaf_hash = blake3::hash(&buf);
    let mut leaf = [0u8; 32];
    leaf.copy_from_slice(leaf_hash.as_bytes());
    let batch_root = emem_attest::merkle_root(&[leaf]);

    let mut h = blake3::Hasher::new();
    h.update(&batch_root);
    h.update(s.manifests.registry_cid.as_str().as_bytes());
    h.update(s.manifests.schema_cid.as_str().as_bytes());
    let signed_digest = h.finalize();
    let sig = s.identity.signing.sign(signed_digest.as_bytes());
    let mut sig_bytes = [0u8; 64];
    sig_bytes.copy_from_slice(&sig.to_bytes());

    let att = Attestation {
        facts: vec![fact],
        batch_root,
        attester: s.identity.pubkey,
        attester_key_epoch: KeyEpoch(s.identity.epoch.0),
        registry_cid: RegistryCid::new(s.manifests.registry_cid.as_str()),
        schema_cid: SchemaCid::new(s.manifests.schema_cid.as_str()),
        stake: None,
        signature: EmCoreSignature(sig_bytes),
        attested_at: signed_at,
    };

    let cids = s
        .storage
        .put_attestation(&att)
        .await
        .map_err(|e| format!("put_attestation (gmrt): {e}"))?;
    cids.into_iter()
        .next()
        .ok_or_else(|| "put_attestation (gmrt) returned no fact_cid".to_string())
}

/// Parse an ORNL MODIS calendar date `YYYY-MM-DD` to Unix epoch seconds
/// (UTC midnight). Returns `None` if the string isn't well-formed.
fn modis_calendar_to_unix(s: &str) -> Option<i64> {
    // Format guaranteed by ORNL: "YYYY-MM-DD".
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 3 {
        return None;
    }
    let y: i64 = parts[0].parse().ok()?;
    let m: i64 = parts[1].parse().ok()?;
    let d: i64 = parts[2].parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // Civil date → days since Unix epoch (Howard Hinnant date algorithm).
    let yy = if m <= 2 { y - 1 } else { y };
    let era = if yy >= 0 { yy / 400 } else { (yy - 399) / 400 };
    let yoe = yy - era * 400; // 0..=399
    let mp = if m > 2 { m - 3 } else { m + 9 }; // 0..=11
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days * 86400)
}

/// Fetch a MODIS MOD13Q1 16-day NDVI composite for the cell's centroid
/// at a target Unix epoch. When `target_unix` is `None`, picks the most
/// recent valid composite in the last 90 days (legacy "current" behavior
/// used by the on-recall materializer). When `Some(t)`, opens a ±32-day
/// window around `t` and selects the composite whose `calendar_date` is
/// closest to `t` — that's the historical-backfill path used by
/// `emem_backfill`.
///
/// Reference: <https://modis.ornl.gov/rst/api/v1/MOD13Q1/subset> —
/// ORNL DAAC, no auth, free, supports up to 160 days per call.
async fn materialize_modis_ndvi_window(
    cell64: &str,
    target_unix: Option<i64>,
    s: &AppState,
) -> Result<emem_fact::FactCid, String> {
    let info = emem_codec::latlng_from_cell64(cell64).map_err(|e| format!("cell decode: {e}"))?;
    let lat = info.lat_deg;
    let lng = info.lng_deg;

    // Window: target ±32d (covers ≥4 16-day composites) or "last 90d".
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (start_unix, end_unix) = match target_unix {
        Some(t) => {
            let lo = (t - 32 * 86400).max(0);
            let hi = (t + 32 * 86400).min(now);
            (lo, hi.max(lo + 86400))
        }
        None => (now - 90 * 86400, now),
    };
    let start_str = unix_to_modis_date(start_unix);
    let end_str = unix_to_modis_date(end_unix);
    let url = format!(
        "https://modis.ornl.gov/rst/api/v1/MOD13Q1/subset?latitude={lat:.6}&longitude={lng:.6}&band=250m_16_days_NDVI&startDate={start_str}&endDate={end_str}&kmAboveBelow=0&kmLeftRight=0",
    );

    // Bounded fetch with explicit retry. Total wall-clock cap is
    // `materializer_timeout_secs() * materializer_retries()`, well under
    // the 30s gateway timeout at default config.
    let timeout = std::time::Duration::from_secs(materializer_timeout_secs());
    let retries = materializer_retries();
    let mut last_err: String = String::new();
    let mut body: Option<JsonValue> = None;
    for attempt in 1..=retries {
        let send = reqwest_client()
            .get(&url)
            .header("accept", "application/json")
            .send();
        match tokio::time::timeout(timeout, send).await {
            Err(_) => {
                last_err = format!(
                    "modis timeout after {}s on attempt {attempt}/{retries}",
                    timeout.as_secs()
                );
                continue;
            }
            Ok(Err(e)) => {
                last_err = format!("modis https on attempt {attempt}/{retries}: {e}");
                continue;
            }
            Ok(Ok(resp)) => {
                let status = resp.status();
                if !status.is_success() {
                    last_err = format!("modis status {status} on attempt {attempt}/{retries}");
                    if status.is_client_error() {
                        break;
                    } // 4xx won't change on retry
                    continue;
                }
                match tokio::time::timeout(timeout, resp.json::<JsonValue>()).await {
                    Err(_) => {
                        last_err = format!(
                            "modis body timeout after {}s on attempt {attempt}/{retries}",
                            timeout.as_secs()
                        );
                        continue;
                    }
                    Ok(Err(e)) => {
                        last_err = format!("modis json on attempt {attempt}/{retries}: {e}");
                        continue;
                    }
                    Ok(Ok(b)) => {
                        body = Some(b);
                        break;
                    }
                }
            }
        }
    }
    let body = body.ok_or(last_err)?;

    // Pick the entry closest to target_unix (or the latest valid one for
    // the current-mode call).
    let scale: f64 = body
        .get("scale")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0001);
    let subset = body
        .get("subset")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "modis response missing `subset` array".to_string())?;
    let mut best: Option<(i64, f64, String)> = None; // (priority, ndvi, cal_date)
    for entry in subset.iter() {
        let raw = entry
            .get("data")
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())
            .and_then(|v| v.as_i64());
        let cal = entry
            .get("calendar_date")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_default();
        let Some(r) = raw else { continue };
        if r == -3000 {
            continue;
        } // MOD13 fill value
        let ndvi = (r as f64) * scale;
        if !(-0.2..=1.0).contains(&ndvi) {
            continue;
        }
        let entry_unix = modis_calendar_to_unix(&cal).unwrap_or(0);
        let priority = match target_unix {
            Some(t) => (entry_unix - t).abs(),
            // Current mode: prefer most recent → priority is age (lower = better).
            None => i64::MAX - entry_unix,
        };
        if best.as_ref().map(|(p, _, _)| priority < *p).unwrap_or(true) {
            best = Some((priority, ndvi, cal));
        }
    }
    let (_, ndvi, cal_date) = best.ok_or_else(||
        if let Some(t) = target_unix {
            format!("no valid NDVI observation in 64-day window around {t}; cell may be permanently cloudy or off-coverage")
        } else {
            "no valid NDVI observation in last 90 days; cell may be permanently cloudy or off-coverage".to_string()
        }
    )?;

    // Tslot is derived from the actual capture date — the MOD13Q1
    // calendar_date the responder picked above, parsed back to Unix
    // seconds. Both backfill and on-recall paths converge on the same
    // tslot when they pick the same composite, so there's exactly one
    // address per (cell, band, composite) regardless of how it was
    // requested. Falls back to the request target (or now in current
    // mode) when calendar parsing fails.
    let cal_unix = modis_calendar_to_unix(&cal_date)
        .or(target_unix)
        .unwrap_or(now);
    let tslot = emem_core::tslot::Tslot::from_unix(cal_unix, emem_core::tslot::Tempo::Medium).0;

    let signed_at = chrono_iso8601_utc();
    let fact = Fact::Primary(PrimaryFact {
        cell: cell64.to_string(),
        band: "modis.ndvi_mean".into(),
        tslot,
        value: ciborium::Value::Float(ndvi),
        unit: None, // NDVI is dimensionless
        confidence: 0.9,
        uncertainty: None,
        sources: vec![Source {
            scheme: "ornl_modis".into(),
            id: url.clone(),
            cid: None,
            hash: None,
            captured_at: Some(cal_date),
            url: None,
        }],
        derivation: Derivation {
            fn_key: "modis_ornl_subset@1".into(),
            args: Some(ciborium::Value::Array(vec![
                ciborium::Value::Float(lat),
                ciborium::Value::Float(lng),
                ciborium::Value::Text("MOD13Q1".into()),
                ciborium::Value::Text("250m_16_days_NDVI".into()),
            ])),
        },
        privacy_class: "public".into(),
        schema_cid: SchemaCid::new(s.manifests.schema_cid.as_str()),
        signer: s.identity.pubkey,
        signed_at: signed_at.clone(),
    });

    let mut buf = Vec::new();
    ciborium::ser::into_writer(&fact, &mut buf).map_err(|e| format!("cbor encode: {e}"))?;
    let leaf_hash = blake3::hash(&buf);
    let mut leaf = [0u8; 32];
    leaf.copy_from_slice(leaf_hash.as_bytes());
    let batch_root = emem_attest::merkle_root(&[leaf]);

    let mut h = blake3::Hasher::new();
    h.update(&batch_root);
    h.update(s.manifests.registry_cid.as_str().as_bytes());
    h.update(s.manifests.schema_cid.as_str().as_bytes());
    let signed_digest = h.finalize();
    let sig = s.identity.signing.sign(signed_digest.as_bytes());
    let mut sig_bytes = [0u8; 64];
    sig_bytes.copy_from_slice(&sig.to_bytes());

    let att = Attestation {
        facts: vec![fact],
        batch_root,
        attester: s.identity.pubkey,
        attester_key_epoch: KeyEpoch(s.identity.epoch.0),
        registry_cid: RegistryCid::new(s.manifests.registry_cid.as_str()),
        schema_cid: SchemaCid::new(s.manifests.schema_cid.as_str()),
        stake: None,
        signature: EmCoreSignature(sig_bytes),
        attested_at: signed_at,
    };

    let cids = s
        .storage
        .put_attestation(&att)
        .await
        .map_err(|e| format!("put_attestation (modis): {e}"))?;
    cids.into_iter()
        .next()
        .ok_or_else(|| "put_attestation (modis) returned no fact_cid".to_string())
}

/// On-recall materializer: latest 16-day composite. Wraps the windowed
/// fetcher in current-mode for backwards compatibility (existing call
/// sites pass no target).
async fn materialize_modis_ndvi(cell64: &str, s: &AppState) -> Result<emem_fact::FactCid, String> {
    materialize_modis_ndvi_window(cell64, None, s).await
}

/// Sample a single 128-D Tessera embedding pixel for the cell's centroid
/// from the GeoTessera v1 public bucket using HTTP range reads — no GDAL,
/// no rasterio, no full-tile download. The full tile is ~91 MiB and we
/// fetch ~640 bytes per cell instead.
///
/// Math:
/// 1. Snap (lat, lng) to the 0.1° tile centred at (tile_lon, tile_lat) where
///    `tile_lon = floor(lng*10)/10 + 0.05`, same for tile_lat.
/// 2. Range-read the .npy header (first 256 B) to get `(H, W)`.
/// 3. Map (lat, lng) → (row, col) by linear interpolation across the tile
///    extent. The native CRS is UTM; for a 0.1° tile this introduces sub-
///    pixel error near the tile centre and ~1–2 px near the corners. We
///    record this in `derivation.args` so the recipe is reproducible.
/// 4. Range-read the 128 int8 bytes for that pixel and the matching
///    float32 scale, dequantize via `f32 = i8 * scale` (per the GeoTessera
///    public dequantize_embedding function).
/// 5. Sign as Primary fact carrying the 128-D vector under band `geotessera`.
async fn materialize_geotessera_embedding(
    cell64: &str,
    s: &AppState,
) -> Result<emem_fact::FactCid, String> {
    materialize_geotessera_for_year(cell64, s, 2024, "geotessera").await
}

/// Multi-year Tessera: stack 2017..=2024 → 8 × 128 = 1024-D vector signed
/// as one Primary fact for `geotessera.multi_year`. Each year is a
/// per-cell range read against the public bucket; we serialise the calls
/// and skip years that 404 (some tiles only exist for some years) so the
/// signed value reflects the actually-available temporal slice.
async fn materialize_geotessera_multi_year(
    cell64: &str,
    s: &AppState,
) -> Result<emem_fact::FactCid, String> {
    let info = emem_codec::latlng_from_cell64(cell64).map_err(|e| format!("cell decode: {e}"))?;
    let lat = info.lat_deg;
    let lng = info.lng_deg;

    let years: [i32; 8] = [2017, 2018, 2019, 2020, 2021, 2022, 2023, 2024];
    let mut full = Vec::with_capacity(128 * years.len());
    let mut covered: Vec<i32> = Vec::new();
    for y in years.iter() {
        match fetch_geotessera_pixel(lat, lng, *y).await {
            Ok(v) => {
                full.extend(v.into_iter().map(ciborium::Value::Float));
                covered.push(*y);
            }
            Err(e) => {
                tracing::debug!(target: "emem::materialize",
                    year = *y, error = %e, "geotessera multi-year skipped a year");
                // Pad with zeros for byte-stable layout — the year list in args
                // tells the agent which slices are real vs absent.
                for _ in 0..128 {
                    full.push(ciborium::Value::Float(0.0));
                }
            }
        }
    }
    if covered.is_empty() {
        return Err("geotessera multi-year: no year had coverage at this cell".into());
    }

    let signed_at = chrono_iso8601_utc();
    let fact = Fact::Primary(PrimaryFact {
        cell: cell64.to_string(),
        band: "geotessera.multi_year".into(),
        tslot: 0,
        value: ciborium::Value::Array(full),
        unit: None,
        confidence: 0.85,
        uncertainty: None,
        sources: vec![Source {
            scheme: "geotessera".into(),
            id: "https://dl2.geotessera.org/v1/global_0.1_degree_representation/{2017..2024}"
                .into(),
            cid: None,
            hash: None,
            captured_at: Some(signed_at.clone()),
            url: None,
        }],
        derivation: Derivation {
            fn_key: "geotessera_multi_year@1".into(),
            args: Some(ciborium::Value::Array(vec![
                ciborium::Value::Float(lat),
                ciborium::Value::Float(lng),
                ciborium::Value::Array(
                    years
                        .iter()
                        .map(|y| ciborium::Value::Integer((*y as i64).into()))
                        .collect(),
                ),
                ciborium::Value::Array(
                    covered
                        .iter()
                        .map(|y| ciborium::Value::Integer((*y as i64).into()))
                        .collect(),
                ),
                ciborium::Value::Text("zero_pad_missing_years".into()),
            ])),
        },
        privacy_class: "public".into(),
        schema_cid: SchemaCid::new(s.manifests.schema_cid.as_str()),
        signer: s.identity.pubkey,
        signed_at: signed_at.clone(),
    });
    sign_and_persist(s, fact, &signed_at).await
}

/// Per-year Tessera pixel fetch — pure HTTP-range NumPy reader against the
/// public dl2.geotessera.org bucket. Returns the 128-D dequantised
/// embedding for the exact (lat, lng) tile pixel, or an error string.
async fn fetch_geotessera_pixel(lat: f64, lng: f64, year: i32) -> Result<Vec<f64>, String> {
    let tile_lon = (lng * 10.0).floor() / 10.0 + 0.05;
    let tile_lat = (lat * 10.0).floor() / 10.0 + 0.05;
    let tile_lon_r = (tile_lon * 100.0).round() / 100.0;
    let tile_lat_r = (tile_lat * 100.0).round() / 100.0;
    let grid_name = format!("grid_{:.2}_{:.2}", tile_lon_r, tile_lat_r);
    let base = format!(
        "https://dl2.geotessera.org/v1/global_0.1_degree_representation/{year}/{grid_name}"
    );
    let emb_url = format!("{base}/{grid_name}.npy");
    let scales_url = format!("{base}/{grid_name}_scales.npy");

    let cli = reqwest_client();
    let emb_hdr = cli
        .get(&emb_url)
        .header("range", "bytes=0-511")
        .send()
        .await
        .map_err(|e| format!("emb head https: {e}"))?;
    if !(emb_hdr.status() == reqwest::StatusCode::PARTIAL_CONTENT
        || emb_hdr.status() == reqwest::StatusCode::OK)
    {
        return Err(format!(
            "emb head status {} for year {year}",
            emb_hdr.status()
        ));
    }
    let emb_hdr_bytes = emb_hdr
        .bytes()
        .await
        .map_err(|e| format!("emb head body: {e}"))?;
    let (emb_shape, emb_dtype, emb_data_off) =
        parse_npy_header(&emb_hdr_bytes).map_err(|e| format!("emb npy: {e}"))?;
    if emb_shape.len() != 3 || emb_shape[2] != 128 || emb_dtype != "|i1" {
        return Err(format!(
            "emb shape/dtype unexpected {emb_shape:?} {emb_dtype:?}"
        ));
    }
    let h = emb_shape[0];
    let w = emb_shape[1];

    let sc_hdr = cli
        .get(&scales_url)
        .header("range", "bytes=0-511")
        .send()
        .await
        .map_err(|e| format!("scales head https: {e}"))?;
    if !(sc_hdr.status() == reqwest::StatusCode::PARTIAL_CONTENT
        || sc_hdr.status() == reqwest::StatusCode::OK)
    {
        return Err(format!(
            "scales head status {} for year {year}",
            sc_hdr.status()
        ));
    }
    let sc_hdr_bytes = sc_hdr
        .bytes()
        .await
        .map_err(|e| format!("scales head body: {e}"))?;
    let (sc_shape, sc_dtype, sc_data_off) =
        parse_npy_header(&sc_hdr_bytes).map_err(|e| format!("scales npy: {e}"))?;
    if sc_dtype != "<f4" {
        return Err(format!("scales dtype unexpected {sc_dtype:?}"));
    }
    let scales_per_pixel: usize = match sc_shape.len() {
        2 if sc_shape[0] == h && sc_shape[1] == w => 1,
        3 if sc_shape[0] == h && sc_shape[1] == w && sc_shape[2] == 128 => 128,
        _ => {
            return Err(format!(
                "scales shape mismatch {sc_shape:?} vs {emb_shape:?}"
            ))
        }
    };

    let north_lat = tile_lat + 0.05;
    let west_lng = tile_lon - 0.05;
    let frac_y = ((north_lat - lat) / 0.1).clamp(0.0, 1.0 - 1e-9);
    let frac_x = ((lng - west_lng) / 0.1).clamp(0.0, 1.0 - 1e-9);
    let row = (frac_y * (h as f64)) as usize;
    let col = (frac_x * (w as f64)) as usize;
    let pixel_idx = row * w + col;

    let emb_off = emb_data_off + pixel_idx * 128;
    let emb_resp = cli
        .get(&emb_url)
        .header("range", format!("bytes={}-{}", emb_off, emb_off + 127))
        .send()
        .await
        .map_err(|e| format!("emb pixel https: {e}"))?;
    let emb_pixel = emb_resp
        .bytes()
        .await
        .map_err(|e| format!("emb pixel body: {e}"))?;
    if emb_pixel.len() != 128 {
        return Err(format!("emb pixel got {} bytes", emb_pixel.len()));
    }

    let scale_bytes_n = scales_per_pixel * 4;
    let sc_off = sc_data_off + pixel_idx * scale_bytes_n;
    let sc_resp = cli
        .get(&scales_url)
        .header(
            "range",
            format!("bytes={}-{}", sc_off, sc_off + scale_bytes_n - 1),
        )
        .send()
        .await
        .map_err(|e| format!("scale pixel https: {e}"))?;
    let sc_bytes = sc_resp
        .bytes()
        .await
        .map_err(|e| format!("scale pixel body: {e}"))?;
    if sc_bytes.len() != scale_bytes_n {
        return Err(format!("scale got {} bytes", sc_bytes.len()));
    }

    let mut out = Vec::with_capacity(128);
    if scales_per_pixel == 1 {
        let sc = f32::from_le_bytes([sc_bytes[0], sc_bytes[1], sc_bytes[2], sc_bytes[3]]);
        for i in 0..128 {
            let q = emb_pixel[i] as i8;
            out.push((q as f32 * sc) as f64);
        }
    } else {
        for i in 0..128 {
            let off = i * 4;
            let sc = f32::from_le_bytes([
                sc_bytes[off],
                sc_bytes[off + 1],
                sc_bytes[off + 2],
                sc_bytes[off + 3],
            ]);
            let q = emb_pixel[i] as i8;
            out.push((q as f32 * sc) as f64);
        }
    }
    Ok(out)
}

/// Per-year Tessera band materializer. `band` is `geotessera.YYYY` for
/// YYYY in 2017..=2024.
async fn materialize_geotessera_year_band(
    cell64: &str,
    s: &AppState,
    band: &str,
) -> Result<emem_fact::FactCid, String> {
    let suffix = band
        .strip_prefix("geotessera.")
        .ok_or_else(|| format!("not a geotessera year band: {band}"))?;
    let year: i32 = suffix
        .parse()
        .map_err(|_| format!("invalid year in {band}"))?;
    if !(2017..=2024).contains(&year) {
        return Err(format!(
            "year {year} outside Tessera v1 vintage [2017,2024]"
        ));
    }
    materialize_geotessera_for_year(cell64, s, year, band).await
}

/// Shared core for per-year Tessera. Signs as the given band name so `geotessera`
/// (default 2024) and `geotessera.YYYY` both work without code duplication.
async fn materialize_geotessera_for_year(
    cell64: &str,
    s: &AppState,
    year: i32,
    band_name: &str,
) -> Result<emem_fact::FactCid, String> {
    let info = emem_codec::latlng_from_cell64(cell64).map_err(|e| format!("cell decode: {e}"))?;
    let lat = info.lat_deg;
    let lng = info.lng_deg;
    let v = fetch_geotessera_pixel(lat, lng, year).await?;
    let signed_at = chrono_iso8601_utc();
    let fact = Fact::Primary(PrimaryFact {
        cell: cell64.to_string(),
        band: band_name.to_string(),
        tslot: 0,
        value: ciborium::Value::Array(v.into_iter().map(ciborium::Value::Float).collect()),
        unit: None,
        confidence: 0.85,
        uncertainty: None,
        sources: vec![Source {
            scheme: "geotessera".into(),
            id: format!(
                "https://dl2.geotessera.org/v1/global_0.1_degree_representation/{year}/..."
            ),
            cid: None,
            hash: None,
            captured_at: Some(signed_at.clone()),
            url: None,
        }],
        derivation: Derivation {
            fn_key: "geotessera_v1@1".into(),
            args: Some(ciborium::Value::Array(vec![
                ciborium::Value::Float(lat),
                ciborium::Value::Float(lng),
                ciborium::Value::Integer((year as i64).into()),
            ])),
        },
        privacy_class: "public".into(),
        schema_cid: SchemaCid::new(s.manifests.schema_cid.as_str()),
        signer: s.identity.pubkey,
        signed_at: signed_at.clone(),
    });
    sign_and_persist(s, fact, &signed_at).await
}

// Legacy 2024-only Tessera fetch was inlined into `materialize_geotessera_for_year`.

/// Materialize one of the `weather.*` bands from Open-Meteo's free public
/// `forecast?current=...` endpoint. Tempo is `ultra_fast` — Open-Meteo
/// updates this surface every ~15 minutes from a blend of HRRR, ICON, GFS,
/// and ECMWF runs, mirroring how geostationary satellites (GOES, Himawari,
/// Meteosat) feed the upstream NWP. We persist exactly one Primary fact
/// per requested band per call so the caller gets one cite-able value;
/// repeated calls within the 15-minute slot hit hot cache.
///
/// Mapping:
///   weather.temperature_2m   → temperature_2m   (°C)
///   weather.cloud_cover      → cloud_cover      (%)
///   weather.precipitation_mm → precipitation    (mm in last 15 min)
///   weather.wind_speed_10m   → wind_speed_10m   (m/s)
async fn materialize_weather_current(
    cell64: &str,
    s: &AppState,
    band: &str,
) -> Result<emem_fact::FactCid, String> {
    let info = emem_codec::latlng_from_cell64(cell64).map_err(|e| format!("cell decode: {e}"))?;
    let lat = info.lat_deg;
    let lng = info.lng_deg;

    // Map our band → MET Norway's `details` field name + unit + confidence.
    // MET Norway's locationforecast/2.0/compact requires no API key and is
    // not rate-limited at the per-IP level; their TOS asks only for an
    // identifying User-Agent. The data is sat-fed (ECMWF + EUMETSAT
    // geostationary) so the temporal-class assignment (ultra_fast → advection
    // kernel) still matches the underlying physics.
    let (met_field, unit, confidence): (&str, Option<&str>, f32) = match band {
        "weather.temperature_2m" => ("air_temperature", Some("degC"), 0.85),
        "weather.cloud_cover" => ("cloud_area_fraction", Some("percent"), 0.80),
        "weather.precipitation_mm" => ("precipitation_amount", Some("mm"), 0.75),
        "weather.wind_speed_10m" => ("wind_speed", Some("m/s"), 0.80),
        "weather.relative_humidity_2m" => ("relative_humidity", Some("percent"), 0.80),
        "weather.dew_point_2m" => ("dew_point_temperature", Some("degC"), 0.80),
        "weather.air_pressure_msl" => ("air_pressure_at_sea_level", Some("hPa"), 0.85),
        "weather.wind_direction_10m" => ("wind_from_direction", Some("deg"), 0.80),
        _ => return Err(format!("weather band not wired: {band}")),
    };

    let url = format!(
        "https://api.met.no/weatherapi/locationforecast/2.0/compact?lat={lat:.4}&lon={lng:.4}",
    );
    let resp = reqwest_client()
        .get(&url)
        .header("user-agent", "emem.dev/0.0.2 (avijeet@vortx.ai)")
        .send()
        .await
        .map_err(|e| format!("met.no https: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("met.no status {} for {band}", resp.status()));
    }
    let body: JsonValue = resp.json().await.map_err(|e| format!("met.no json: {e}"))?;
    // MET Norway returns the *current* observation as `properties.timeseries[0]`.
    // `instant.details` carries point-in-time fields; `next_1_hours.details`
    // carries accumulated values like precipitation_amount.
    let ts0 = body
        .pointer("/properties/timeseries/0")
        .ok_or_else(|| "met.no missing properties.timeseries[0]".to_string())?;
    let captured = ts0
        .get("time")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let value: f64 = if band == "weather.precipitation_mm" {
        ts0.pointer("/data/next_1_hours/details/precipitation_amount")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| "met.no missing next_1_hours.precipitation_amount".to_string())?
    } else {
        ts0.pointer(&format!("/data/instant/details/{met_field}"))
            .and_then(|v| v.as_f64())
            .ok_or_else(|| format!("met.no missing instant.details.{met_field}"))?
    };

    let signed_at = chrono_iso8601_utc();
    let fact = Fact::Primary(PrimaryFact {
        cell: cell64.to_string(),
        band: band.to_string(),
        tslot: 0,
        value: ciborium::Value::Float(value),
        unit: unit.map(|u| u.to_string()),
        confidence,
        uncertainty: None,
        sources: vec![Source {
            scheme: "met_no".into(),
            id: url.clone(),
            cid: None,
            hash: None,
            captured_at: captured,
            url: None,
        }],
        derivation: Derivation {
            fn_key: "met_no_locationforecast_compact@1".into(),
            args: Some(ciborium::Value::Array(vec![
                ciborium::Value::Float(lat),
                ciborium::Value::Float(lng),
                ciborium::Value::Text(met_field.to_string()),
            ])),
        },
        privacy_class: "public".into(),
        schema_cid: SchemaCid::new(s.manifests.schema_cid.as_str()),
        signer: s.identity.pubkey,
        signed_at: signed_at.clone(),
    });
    sign_and_persist(s, fact, &signed_at).await
}

/// Convert a Unix epoch second to a `YYYYMMDD` string for upstream APIs that
/// expect calendar dates. Uses `civil_from_days` (defined elsewhere in this
/// module) — no chrono dependency.
fn unix_to_yyyymmdd(unix: i64) -> String {
    let z = unix.div_euclid(86_400);
    let (y, m, d) = civil_from_days(z);
    format!("{y:04}{m:02}{d:02}")
}

/// Materialize a NASA POWER reanalysis fact at a single lat/lng. Free, no
/// auth, public-domain (US Gov), global, 0.5° MERRA-2 grid downscaled.
///
/// `target_unix = None` materializes the *current* day from the latest
/// available POWER cycle (~2-day lag); `Some(t)` materializes the daily
/// value covering that Unix second. Both modes hit the same daily REST
/// endpoint — POWER returns one row per requested calendar day.
///
/// Reference: https://power.larc.nasa.gov/docs/services/api/temporal/daily/
async fn materialize_power_band(
    cell64: &str,
    s: &AppState,
    band: &str,
    target_unix: Option<i64>,
) -> Result<emem_fact::FactCid, String> {
    // Map emem band → POWER `parameters=` field + unit + confidence.
    let (param, unit, confidence) = match band {
        "power.t2m" => ("T2M", Some("degC"), 0.85),
        "power.t2m_min" => ("T2M_MIN", Some("degC"), 0.85),
        "power.t2m_max" => ("T2M_MAX", Some("degC"), 0.85),
        "power.precip" => ("PRECTOTCORR", Some("mm/day"), 0.80),
        "power.rh2m" => ("RH2M", Some("percent"), 0.80),
        "power.allsky_sw" => ("ALLSKY_SFC_SW_DWN", Some("MJ/m^2/day"), 0.85),
        "power.ws10m" => ("WS10M", Some("m/s"), 0.80),
        _ => return Err(format!("power band not wired: {band}")),
    };
    let info = emem_codec::latlng_from_cell64(cell64).map_err(|e| format!("cell decode: {e}"))?;
    let lat = info.lat_deg;
    let lng = info.lng_deg;
    // POWER's daily product publishes with ~3-5 day latency depending on
    // which native source (GEOSIT, MERRA-2) is fronting the request. Pick
    // 5 days back for the "current" mode so we always land inside the
    // published window — 422/-999 sentinels make this brittle otherwise.
    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let target = target_unix.unwrap_or(now_unix - 5 * 86_400);
    let date = unix_to_yyyymmdd(target);
    let url = format!(
        "https://power.larc.nasa.gov/api/temporal/daily/point?parameters={param}&latitude={lat:.4}&longitude={lng:.4}&start={date}&end={date}&community=ag&format=JSON"
    );
    let timeout = std::time::Duration::from_secs(materializer_timeout_secs());
    let resp = tokio::time::timeout(
        timeout,
        reqwest_client()
            .get(&url)
            .header("user-agent", "emem.dev/0.0.2 (avijeet@vortx.ai)")
            .send(),
    )
    .await
    .map_err(|_| format!("nasa power timeout after {}s", timeout.as_secs()))?
    .map_err(|e| format!("nasa power https: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("nasa power status {} for {band}", resp.status()));
    }
    let body: JsonValue = resp.json().await.map_err(|e| format!("power json: {e}"))?;
    // Response shape: properties.parameter.<PARAM>.<YYYYMMDD> = number.
    let v = body
        .pointer(&format!("/properties/parameter/{param}/{date}"))
        .and_then(|v| v.as_f64())
        .ok_or_else(|| format!("nasa power missing properties.parameter.{param}.{date}"))?;
    if v <= -990.0 {
        // POWER's nodata sentinel is -999.
        return Err(format!(
            "nasa power nodata at lat={lat:.3} lng={lng:.3} on {date} for {param}"
        ));
    }
    let signed_at = chrono_iso8601_utc();
    let tslot = emem_core::tslot::Tslot::from_unix(target, emem_core::tslot::Tempo::Fast).0;
    let fact = Fact::Primary(PrimaryFact {
        cell: cell64.to_string(),
        band: band.to_string(),
        tslot,
        value: ciborium::Value::Float(v),
        unit: unit.map(|u| u.to_string()),
        confidence,
        uncertainty: None,
        sources: vec![Source {
            scheme: "nasa_power".into(),
            id: url.clone(),
            cid: None,
            hash: None,
            captured_at: Some(format!(
                "{}-{}-{}T00:00:00Z",
                &date[0..4],
                &date[4..6],
                &date[6..8]
            )),
            url: None,
        }],
        derivation: Derivation {
            fn_key: "nasa_power_daily_point@1".into(),
            args: Some(ciborium::Value::Array(vec![
                ciborium::Value::Float(lat),
                ciborium::Value::Float(lng),
                ciborium::Value::Text(param.to_string()),
                ciborium::Value::Text(date.clone()),
            ])),
        },
        privacy_class: "public".into(),
        schema_cid: SchemaCid::new(s.manifests.schema_cid.as_str()),
        signer: s.identity.pubkey,
        signed_at: signed_at.clone(),
    });
    sign_and_persist(s, fact, &signed_at).await
}

/// Materialize an Open-Meteo CAMS air-quality fact at a single lat/lng.
/// Free, no API key, CC BY 4.0; CAMS surface-level pollutant blend updates
/// hourly with ~30-minute latency.
///
/// `target_unix = None` materializes the *current* hour. `Some(t)` returns
/// the hour-bucket containing `t`. Open-Meteo's hourly archive runs from
/// 2013-08-01 forward.
///
/// Reference: https://open-meteo.com/en/docs/air-quality-api
async fn materialize_cams_band(
    cell64: &str,
    s: &AppState,
    band: &str,
    target_unix: Option<i64>,
) -> Result<emem_fact::FactCid, String> {
    let (om_field, unit, confidence) = match band {
        "cams.pm25" => ("pm2_5", Some("ug/m^3"), 0.80),
        "cams.pm10" => ("pm10", Some("ug/m^3"), 0.80),
        "cams.no2" => ("nitrogen_dioxide", Some("ug/m^3"), 0.80),
        "cams.o3" => ("ozone", Some("ug/m^3"), 0.80),
        "cams.so2" => ("sulphur_dioxide", Some("ug/m^3"), 0.80),
        "cams.co" => ("carbon_monoxide", Some("ug/m^3"), 0.80),
        "cams.aod_550" => ("aerosol_optical_depth", None, 0.75),
        _ => return Err(format!("cams band not wired: {band}")),
    };
    let info = emem_codec::latlng_from_cell64(cell64).map_err(|e| format!("cell decode: {e}"))?;
    let lat = info.lat_deg;
    let lng = info.lng_deg;
    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let target = target_unix.unwrap_or(now_unix);
    let date = unix_to_yyyymmdd(target);
    // Air-quality API accepts start_date/end_date as YYYY-MM-DD.
    let date_iso = format!("{}-{}-{}", &date[0..4], &date[4..6], &date[6..8]);
    let url = format!(
        "https://air-quality-api.open-meteo.com/v1/air-quality?latitude={lat:.4}&longitude={lng:.4}&hourly={om_field}&start_date={date_iso}&end_date={date_iso}&timezone=UTC"
    );
    let timeout = std::time::Duration::from_secs(materializer_timeout_secs());
    let resp = tokio::time::timeout(
        timeout,
        reqwest_client()
            .get(&url)
            .header("user-agent", "emem.dev/0.0.2 (avijeet@vortx.ai)")
            .send(),
    )
    .await
    .map_err(|_| format!("cams timeout after {}s", timeout.as_secs()))?
    .map_err(|e| format!("cams https: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("cams status {} for {band}", resp.status()));
    }
    let body: JsonValue = resp.json().await.map_err(|e| format!("cams json: {e}"))?;
    let times = body
        .pointer("/hourly/time")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "cams response missing hourly.time".to_string())?;
    let values = body
        .pointer(&format!("/hourly/{om_field}"))
        .and_then(|v| v.as_array())
        .ok_or_else(|| format!("cams response missing hourly.{om_field}"))?;
    // Pick the hour bucket containing `target` (Open-Meteo returns
    // ISO-format strings like "2024-03-15T07:00").
    let target_hour_unix = (target / 3600) * 3600;
    let mut chosen_idx: Option<usize> = None;
    let mut chosen_iso = String::new();
    for (i, t) in times.iter().enumerate() {
        let Some(s) = t.as_str() else { continue };
        // Append :00Z and parse as Unix via days_from_civil.
        if let Some(u) = parse_iso_utc_hour(s) {
            if u == target_hour_unix {
                chosen_idx = Some(i);
                chosen_iso = s.to_string();
                break;
            }
        }
    }
    let idx = chosen_idx.ok_or_else(|| {
        format!(
            "cams response had no hour matching target {target_hour_unix} ({})",
            unix_to_yyyymmdd(target_hour_unix)
        )
    })?;
    let v = values
        .get(idx)
        .and_then(|v| v.as_f64())
        .ok_or_else(|| format!("cams hourly.{om_field}[{idx}] missing or null"))?;
    let signed_at = chrono_iso8601_utc();
    let tslot = emem_core::tslot::Tslot::from_unix(target, emem_core::tslot::Tempo::UltraFast).0;
    let fact = Fact::Primary(PrimaryFact {
        cell: cell64.to_string(),
        band: band.to_string(),
        tslot,
        value: ciborium::Value::Float(v),
        unit: unit.map(|u| u.to_string()),
        confidence,
        uncertainty: None,
        sources: vec![Source {
            scheme: "open_meteo_cams".into(),
            id: url.clone(),
            cid: None,
            hash: None,
            captured_at: Some(format!("{chosen_iso}:00Z")),
            url: None,
        }],
        derivation: Derivation {
            fn_key: "open_meteo_cams_hourly@1".into(),
            args: Some(ciborium::Value::Array(vec![
                ciborium::Value::Float(lat),
                ciborium::Value::Float(lng),
                ciborium::Value::Text(om_field.to_string()),
                ciborium::Value::Text(chosen_iso.clone()),
            ])),
        },
        privacy_class: "public".into(),
        schema_cid: SchemaCid::new(s.manifests.schema_cid.as_str()),
        signer: s.identity.pubkey,
        signed_at: signed_at.clone(),
    });
    sign_and_persist(s, fact, &signed_at).await
}

/// Materialize an Open-Meteo ERA5 reanalysis fact at a single lat/lng.
/// Free, no API key, CC BY 4.0; ECMWF ERA5 (~9 km downscaled), hourly,
/// 1940-present.
///
/// `target_unix` is required (this is a historical archive — there's no
/// "current" mode). For backfill, the slot is anchored to the hour
/// containing `target_unix`.
///
/// Reference: https://open-meteo.com/en/docs/historical-weather-api
async fn materialize_era5_band(
    cell64: &str,
    s: &AppState,
    band: &str,
    target_unix: i64,
) -> Result<emem_fact::FactCid, String> {
    let (om_field, unit, confidence) = match band {
        "era5.t2m" => ("temperature_2m", Some("degC"), 0.90),
        "era5.precip" => ("precipitation", Some("mm"), 0.85),
        "era5.rh2m" => ("relative_humidity_2m", Some("percent"), 0.85),
        "era5.windspeed_10m" => ("wind_speed_10m", Some("km/h"), 0.85),
        "era5.cloudcover" => ("cloud_cover", Some("percent"), 0.80),
        "era5.surface_pressure" => ("surface_pressure", Some("hPa"), 0.85),
        "era5.dewpoint_2m" => ("dew_point_2m", Some("degC"), 0.85),
        _ => return Err(format!("era5 band not wired: {band}")),
    };
    let info = emem_codec::latlng_from_cell64(cell64).map_err(|e| format!("cell decode: {e}"))?;
    let lat = info.lat_deg;
    let lng = info.lng_deg;
    let date = unix_to_yyyymmdd(target_unix);
    let date_iso = format!("{}-{}-{}", &date[0..4], &date[4..6], &date[6..8]);
    let url = format!(
        "https://archive-api.open-meteo.com/v1/archive?latitude={lat:.4}&longitude={lng:.4}&hourly={om_field}&start_date={date_iso}&end_date={date_iso}&timezone=UTC"
    );
    let timeout = std::time::Duration::from_secs(materializer_timeout_secs());
    let resp = tokio::time::timeout(
        timeout,
        reqwest_client()
            .get(&url)
            .header("user-agent", "emem.dev/0.0.2 (avijeet@vortx.ai)")
            .send(),
    )
    .await
    .map_err(|_| format!("era5 timeout after {}s", timeout.as_secs()))?
    .map_err(|e| format!("era5 https: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("era5 status {} for {band}", resp.status()));
    }
    let body: JsonValue = resp.json().await.map_err(|e| format!("era5 json: {e}"))?;
    let times = body
        .pointer("/hourly/time")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "era5 response missing hourly.time".to_string())?;
    let values = body
        .pointer(&format!("/hourly/{om_field}"))
        .and_then(|v| v.as_array())
        .ok_or_else(|| format!("era5 response missing hourly.{om_field}"))?;
    let target_hour_unix = (target_unix / 3600) * 3600;
    let mut chosen: Option<(usize, String)> = None;
    for (i, t) in times.iter().enumerate() {
        let Some(s) = t.as_str() else { continue };
        if let Some(u) = parse_iso_utc_hour(s) {
            if u == target_hour_unix {
                chosen = Some((i, s.to_string()));
                break;
            }
        }
    }
    let (idx, iso) = chosen.ok_or_else(|| {
        format!(
            "era5 response had no hour matching target {target_hour_unix}; ERA5 archive starts 1940-01-01"
        )
    })?;
    let v = values
        .get(idx)
        .and_then(|v| v.as_f64())
        .ok_or_else(|| format!("era5 hourly.{om_field}[{idx}] missing or null"))?;
    let signed_at = chrono_iso8601_utc();
    let tslot =
        emem_core::tslot::Tslot::from_unix(target_unix, emem_core::tslot::Tempo::UltraFast).0;
    let fact = Fact::Primary(PrimaryFact {
        cell: cell64.to_string(),
        band: band.to_string(),
        tslot,
        value: ciborium::Value::Float(v),
        unit: unit.map(|u| u.to_string()),
        confidence,
        uncertainty: None,
        sources: vec![Source {
            scheme: "open_meteo_era5".into(),
            id: url.clone(),
            cid: None,
            hash: None,
            captured_at: Some(format!("{iso}:00Z")),
            url: None,
        }],
        derivation: Derivation {
            fn_key: "open_meteo_era5_archive@1".into(),
            args: Some(ciborium::Value::Array(vec![
                ciborium::Value::Float(lat),
                ciborium::Value::Float(lng),
                ciborium::Value::Text(om_field.to_string()),
                ciborium::Value::Text(iso.clone()),
            ])),
        },
        privacy_class: "public".into(),
        schema_cid: SchemaCid::new(s.manifests.schema_cid.as_str()),
        signer: s.identity.pubkey,
        signed_at: signed_at.clone(),
    });
    sign_and_persist(s, fact, &signed_at).await
}

/// Materialize an Open-Meteo Marine fact (ECMWF WAM wave model) at a single
/// lat/lng. Free, no API key, CC BY 4.0. Coverage: global oceans (NaN over
/// continents), hourly, 2022-08-01 forward.
///
/// `target_unix = None` materializes the current hour; `Some(t)` returns
/// the hour bucket containing t. Use this for coastal hazard / shipping
/// memory and SST as fallback.
///
/// Reference: https://open-meteo.com/en/docs/marine-weather-api
async fn materialize_marine_band(
    cell64: &str,
    s: &AppState,
    band: &str,
    target_unix: Option<i64>,
) -> Result<emem_fact::FactCid, String> {
    let (om_field, unit, confidence) = match band {
        "marine.wave_height" => ("wave_height", Some("m"), 0.80),
        "marine.swell_period" => ("swell_wave_period", Some("s"), 0.80),
        "marine.swell_height" => ("swell_wave_height", Some("m"), 0.80),
        "marine.sst" => ("sea_surface_temperature", Some("degC"), 0.85),
        "marine.wave_direction" => ("wave_direction", Some("deg"), 0.75),
        _ => return Err(format!("marine band not wired: {band}")),
    };
    let info = emem_codec::latlng_from_cell64(cell64).map_err(|e| format!("cell decode: {e}"))?;
    let lat = info.lat_deg;
    let lng = info.lng_deg;
    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let target = target_unix.unwrap_or(now_unix);
    let date = unix_to_yyyymmdd(target);
    let date_iso = format!("{}-{}-{}", &date[0..4], &date[4..6], &date[6..8]);
    let url = format!(
        "https://marine-api.open-meteo.com/v1/marine?latitude={lat:.4}&longitude={lng:.4}&hourly={om_field}&start_date={date_iso}&end_date={date_iso}&timezone=UTC"
    );
    let timeout = std::time::Duration::from_secs(materializer_timeout_secs());
    let resp = tokio::time::timeout(
        timeout,
        reqwest_client()
            .get(&url)
            .header("user-agent", "emem.dev/0.0.2 (avijeet@vortx.ai)")
            .send(),
    )
    .await
    .map_err(|_| format!("marine timeout after {}s", timeout.as_secs()))?
    .map_err(|e| format!("marine https: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!(
            "marine status {} for {band} (this is a coastal-only model — inland points always fail)",
            resp.status()
        ));
    }
    let body: JsonValue = resp.json().await.map_err(|e| format!("marine json: {e}"))?;
    let times = body
        .pointer("/hourly/time")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "marine response missing hourly.time".to_string())?;
    let values = body
        .pointer(&format!("/hourly/{om_field}"))
        .and_then(|v| v.as_array())
        .ok_or_else(|| format!("marine response missing hourly.{om_field}"))?;
    let target_hour_unix = (target / 3600) * 3600;
    let mut chosen: Option<(usize, String)> = None;
    for (i, t) in times.iter().enumerate() {
        let Some(s) = t.as_str() else { continue };
        if let Some(u) = parse_iso_utc_hour(s) {
            if u == target_hour_unix {
                chosen = Some((i, s.to_string()));
                break;
            }
        }
    }
    let (idx, iso) = chosen
        .ok_or_else(|| format!("marine response had no hour matching target {target_hour_unix}"))?;
    let v = values
        .get(idx)
        .and_then(|v| v.as_f64())
        .ok_or_else(|| format!("marine hourly.{om_field}[{idx}] is null (likely inland point)"))?;
    let signed_at = chrono_iso8601_utc();
    let tslot = emem_core::tslot::Tslot::from_unix(target, emem_core::tslot::Tempo::UltraFast).0;
    let fact = Fact::Primary(PrimaryFact {
        cell: cell64.to_string(),
        band: band.to_string(),
        tslot,
        value: ciborium::Value::Float(v),
        unit: unit.map(|u| u.to_string()),
        confidence,
        uncertainty: None,
        sources: vec![Source {
            scheme: "open_meteo_marine".into(),
            id: url.clone(),
            cid: None,
            hash: None,
            captured_at: Some(format!("{iso}:00Z")),
            url: None,
        }],
        derivation: Derivation {
            fn_key: "open_meteo_marine_hourly@1".into(),
            args: Some(ciborium::Value::Array(vec![
                ciborium::Value::Float(lat),
                ciborium::Value::Float(lng),
                ciborium::Value::Text(om_field.to_string()),
                ciborium::Value::Text(iso.clone()),
            ])),
        },
        privacy_class: "public".into(),
        schema_cid: SchemaCid::new(s.manifests.schema_cid.as_str()),
        signer: s.identity.pubkey,
        signed_at: signed_at.clone(),
    });
    sign_and_persist(s, fact, &signed_at).await
}

/// Parse an Open-Meteo `YYYY-MM-DDTHH:MM` (always UTC when timezone=UTC)
/// timestamp into Unix epoch seconds. Returns `None` for malformed input.
/// Strict: must match exactly 16 chars, the literal `T`, and `:00` minutes.
fn parse_iso_utc_hour(s: &str) -> Option<i64> {
    let bytes = s.as_bytes();
    if bytes.len() != 16
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
    {
        return None;
    }
    let y: i32 = std::str::from_utf8(&bytes[0..4]).ok()?.parse().ok()?;
    let m: u32 = std::str::from_utf8(&bytes[5..7]).ok()?.parse().ok()?;
    let d: u32 = std::str::from_utf8(&bytes[8..10]).ok()?.parse().ok()?;
    let hh: i64 = std::str::from_utf8(&bytes[11..13]).ok()?.parse().ok()?;
    let mm: i64 = std::str::from_utf8(&bytes[14..16]).ok()?.parse().ok()?;
    let day = days_from_civil(y, m, d);
    Some(day * 86_400 + hh * 3600 + mm * 60)
}

/// Materialize an additional ORNL DAAC MODIS/VIIRS subset product. Same
/// REST pipeline as `materialize_modis_ndvi_window` (`MOD13Q1`) but with
/// per-product band names, scale factors, and quality filters.
///
/// Products wired (all anonymous, public-domain):
/// - `modis.lst_day_8day`   → MOD11A2.061 LST_Day_1km    (K, scale 0.02)
/// - `modis.lst_night_8day` → MOD11A2.061 LST_Night_1km  (K, scale 0.02)
/// - `modis.et_8day`        → MOD16A2.061 ET             (kg/m^2, scale 0.1)
/// - `modis.gpp_8day`       → MOD17A2H.061 Gpp           (kg C/m^2, scale 1e-4)
/// - `modis.lai_8day`       → MOD15A2H.061 Lai_500m      (m^2/m^2, scale 0.1)
/// - `modis.burned_area`    → MCD64A1.061 Burn_Date      (DOY of burn, no scale)
///
/// Reference: https://modis.ornl.gov/data/modis_webservice.html
async fn materialize_ornl_modis_band(
    cell64: &str,
    s: &AppState,
    band: &str,
    target_unix: Option<i64>,
) -> Result<emem_fact::FactCid, String> {
    // Map our band → (MODIS product, MODIS variable, default scale, unit, ±day window).
    // ORNL DAAC requires the product's actual variable name (e.g. `LST_Day_1km`),
    // not the emem alias.
    let (product, variable, default_scale, unit, half_window_days, fn_key) = match band {
        "modis.lst_day_8day" => (
            "MOD11A2",
            "LST_Day_1km",
            0.02_f64,
            Some("K"),
            8_i64,
            "modis_ornl_mod11a2_lstday@1",
        ),
        "modis.lst_night_8day" => (
            "MOD11A2",
            "LST_Night_1km",
            0.02_f64,
            Some("K"),
            8_i64,
            "modis_ornl_mod11a2_lstnight@1",
        ),
        "modis.et_8day" => (
            "MOD16A2",
            "ET_500m",
            0.1_f64,
            Some("kg/m^2"),
            8_i64,
            "modis_ornl_mod16a2_et@1",
        ),
        "modis.gpp_8day" => (
            "MOD17A2H",
            "Gpp_500m",
            1e-4_f64,
            Some("kg C/m^2"),
            8_i64,
            "modis_ornl_mod17a2h_gpp@1",
        ),
        "modis.lai_8day" => (
            "MOD15A2H",
            "Lai_500m",
            0.1_f64,
            Some("m^2/m^2"),
            8_i64,
            "modis_ornl_mod15a2h_lai@1",
        ),
        "modis.burned_area_monthly" => (
            "MCD64A1",
            "Burn_Date",
            1.0_f64,
            Some("doy"),
            32_i64,
            "modis_ornl_mcd64a1_burndate@1",
        ),
        _ => return Err(format!("ornl modis band not wired: {band}")),
    };
    let info = emem_codec::latlng_from_cell64(cell64).map_err(|e| format!("cell decode: {e}"))?;
    let lat = info.lat_deg;
    let lng = info.lng_deg;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    // Window selection: ±half_window around an explicit target_unix, OR a
    // 12·half_window lookback in "current" mode. The wider lookback covers
    // the publication-latency tail of slow MODIS products (MOD16A2 ET and
    // MOD17A2H GPP routinely lag 30+ days; MOD15A2H LAI similar).
    let (start_unix, end_unix) = match target_unix {
        Some(t) => {
            let lo = (t - half_window_days * 86_400).max(0);
            let hi = (t + half_window_days * 86_400).min(now);
            (lo, hi.max(lo + 86_400))
        }
        None => (now - 12 * half_window_days * 86_400, now),
    };
    let start_str = unix_to_modis_date(start_unix);
    let end_str = unix_to_modis_date(end_unix);
    let url = format!(
        "https://modis.ornl.gov/rst/api/v1/{product}/subset?latitude={lat:.6}&longitude={lng:.6}&band={variable}&startDate={start_str}&endDate={end_str}&kmAboveBelow=0&kmLeftRight=0",
    );
    let timeout = std::time::Duration::from_secs(materializer_timeout_secs());
    let retries = materializer_retries();
    let mut last_err = String::new();
    let mut body: Option<JsonValue> = None;
    for attempt in 1..=retries {
        let send = reqwest_client()
            .get(&url)
            .header("accept", "application/json")
            .send();
        match tokio::time::timeout(timeout, send).await {
            Err(_) => {
                last_err = format!(
                    "{product} timeout after {}s on attempt {attempt}/{retries}",
                    timeout.as_secs()
                );
                continue;
            }
            Ok(Err(e)) => {
                last_err = format!("{product} https on attempt {attempt}/{retries}: {e}");
                continue;
            }
            Ok(Ok(resp)) => {
                let status = resp.status();
                if !status.is_success() {
                    // ORNL DAAC returns 400 with a body like "No data
                    // available for time period A2026087 to A2026119 for
                    // MOD16A2 19.0767 72.8762 combination." when the
                    // upstream simply hasn't published data in this
                    // range yet. Treat this as soft no-data (empty
                    // subset) rather than a hard transport failure so
                    // the caller gets a clean "no valid observation"
                    // message instead of "status 400".
                    if status == reqwest::StatusCode::BAD_REQUEST {
                        if let Ok(text) = resp.text().await {
                            if text.contains("No data available for time period") {
                                body = Some(json!({"subset": []}));
                                break;
                            }
                        }
                    }
                    last_err = format!("{product} status {status} on attempt {attempt}/{retries}");
                    if status.is_client_error() {
                        break;
                    }
                    continue;
                }
                match tokio::time::timeout(timeout, resp.json::<JsonValue>()).await {
                    Err(_) => {
                        last_err = format!("{product} body timeout on attempt {attempt}/{retries}");
                        continue;
                    }
                    Ok(Err(e)) => {
                        last_err = format!("{product} json on attempt {attempt}/{retries}: {e}");
                        continue;
                    }
                    Ok(Ok(b)) => {
                        body = Some(b);
                        break;
                    }
                }
            }
        }
    }
    let body = body.ok_or(last_err)?;
    // Scale factor: prefer upstream-reported, fall back to product default.
    let scale: f64 = body
        .get("scale")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok())
        .unwrap_or(default_scale);
    let subset = body
        .get("subset")
        .and_then(|v| v.as_array())
        .ok_or_else(|| format!("{product} response missing `subset` array"))?;
    let mut best: Option<(i64, f64, String)> = None;
    for entry in subset.iter() {
        let raw = entry
            .get("data")
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())
            .and_then(|v| v.as_i64());
        let cal = entry
            .get("calendar_date")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_default();
        let Some(r) = raw else { continue };
        // Per-product nodata sentinels:
        //   MOD11A2 LST: 0 (raw uint16; 0 = unfilled QC).
        //   MOD16A2 ET:  32760+ (fill ranges). >= 32700 → fill.
        //   MOD17A2H GPP: 32760+ fill, also negative gpp on water.
        //   MOD15A2H LAI: 249..255 reserved (water/cloud/etc).
        //   MCD64A1 Burn_Date: 0 = no burn that month, -1 unmapped/water.
        let is_nodata = match product {
            "MOD11A2" => r == 0,
            "MOD16A2" | "MOD17A2H" => r >= 32700,
            "MOD15A2H" => !(0..=100).contains(&r),
            "MCD64A1" => r < 1, // 0 = unburned, treat as no event but emit value
            _ => false,
        };
        if product != "MCD64A1" && is_nodata {
            continue;
        }
        let scaled = (r as f64) * scale;
        let entry_unix = modis_calendar_to_unix(&cal).unwrap_or(0);
        let priority = match target_unix {
            Some(t) => (entry_unix - t).abs(),
            None => i64::MAX - entry_unix,
        };
        if best.as_ref().map(|(p, _, _)| priority < *p).unwrap_or(true) {
            best = Some((priority, scaled, cal));
        }
    }
    let (_, value, cal_date) = best.ok_or_else(|| match target_unix {
        Some(t) => {
            format!("no valid {product} {variable} observation in ±{half_window_days}d around {t}")
        }
        None => format!(
            "no valid {product} {variable} observation in last {} days",
            4 * half_window_days
        ),
    })?;
    let cal_unix = modis_calendar_to_unix(&cal_date)
        .or(target_unix)
        .unwrap_or(now);
    let tslot = emem_core::tslot::Tslot::from_unix(cal_unix, emem_core::tslot::Tempo::Medium).0;
    let signed_at = chrono_iso8601_utc();
    let fact = Fact::Primary(PrimaryFact {
        cell: cell64.to_string(),
        band: band.to_string(),
        tslot,
        value: ciborium::Value::Float(value),
        unit: unit.map(|u| u.to_string()),
        confidence: 0.85,
        uncertainty: None,
        sources: vec![Source {
            scheme: "ornl_modis".into(),
            id: url.clone(),
            cid: None,
            hash: None,
            captured_at: Some(cal_date.clone()),
            url: None,
        }],
        derivation: Derivation {
            fn_key: fn_key.to_string(),
            args: Some(ciborium::Value::Array(vec![
                ciborium::Value::Float(lat),
                ciborium::Value::Float(lng),
                ciborium::Value::Text(product.to_string()),
                ciborium::Value::Text(variable.to_string()),
                ciborium::Value::Text(cal_date.clone()),
                ciborium::Value::Float(scale),
            ])),
        },
        privacy_class: "public".into(),
        schema_cid: SchemaCid::new(s.manifests.schema_cid.as_str()),
        signer: s.identity.pubkey,
        signed_at: signed_at.clone(),
    });
    sign_and_persist(s, fact, &signed_at).await
}

/// Map an emem band key to the set of Sentinel-2 STAC asset names it depends on.
/// Returns (asset_aliases_per_input, kind, formula_note). The asset aliases use
/// Element84's STAC naming (`red`, `nir`, …) with B-number fallbacks; the COG
/// reader sees whichever is present.
fn s2_band_plan(band: &str) -> Option<(Vec<&'static [&'static str]>, &'static str, &'static str)> {
    static B01: &[&str] = &["coastal", "B01"];
    static B02: &[&str] = &["blue", "B02"];
    static B03: &[&str] = &["green", "B03"];
    static B04: &[&str] = &["red", "B04"];
    static B05: &[&str] = &["rededge1", "B05"];
    static B06: &[&str] = &["rededge2", "B06"];
    static B07: &[&str] = &["rededge3", "B07"];
    static B08: &[&str] = &["nir", "B08"];
    static B8A: &[&str] = &["nir08", "B8A"];
    static B09: &[&str] = &["nir09", "B09"];
    static B11: &[&str] = &["swir16", "B11"];
    static B12: &[&str] = &["swir22", "B12"];
    static SCL: &[&str] = &["scl"];
    match band {
        // Raw L2A surface reflectance per band (uint16, scale 1e-4 → reflectance ∈ [0,1]).
        "s2.B01" => Some((vec![B01], "raw_reflectance", "B01 60m coastal aerosol")),
        "s2.B02" => Some((vec![B02], "raw_reflectance", "B02 10m blue")),
        "s2.B03" => Some((vec![B03], "raw_reflectance", "B03 10m green")),
        "s2.B04" => Some((vec![B04], "raw_reflectance", "B04 10m red")),
        "s2.B05" => Some((
            vec![B05],
            "raw_reflectance",
            "B05 20m vegetation red-edge 1",
        )),
        "s2.B06" => Some((
            vec![B06],
            "raw_reflectance",
            "B06 20m vegetation red-edge 2",
        )),
        "s2.B07" => Some((
            vec![B07],
            "raw_reflectance",
            "B07 20m vegetation red-edge 3",
        )),
        "s2.B08" => Some((vec![B08], "raw_reflectance", "B08 10m wide NIR")),
        "s2.B8A" => Some((vec![B8A], "raw_reflectance", "B8A 20m narrow NIR")),
        "s2.B09" => Some((vec![B09], "raw_reflectance", "B09 60m water vapor")),
        "s2.B11" => Some((vec![B11], "raw_reflectance", "B11 20m SWIR-1")),
        "s2.B12" => Some((vec![B12], "raw_reflectance", "B12 20m SWIR-2")),
        // Scene Classification Layer — uint8 categorical 0..11.
        "s2.scl" => Some((vec![SCL], "scl_categorical", "SCL 20m scene class (0..11)")),
        // Indices computed deterministically from raw reflectance.
        "indices.ndvi" => Some((
            vec![B08, B04],
            "index_ndvi",
            "NDVI = (B08 − B04) / (B08 + B04). Vegetation greenness.",
        )),
        "indices.ndwi" => Some((
            vec![B03, B08],
            "index_ndwi",
            "NDWI (Gao) = (B03 − B08) / (B03 + B08). Open water; positive over water.",
        )),
        "indices.mndwi" => Some((
            vec![B03, B11],
            "index_mndwi",
            "MNDWI (McFeeters) = (B03 − B11) / (B03 + B11). Stronger water mask using SWIR.",
        )),
        "indices.evi" => Some((
            vec![B08, B04, B02],
            "index_evi",
            "EVI = 2.5·(B08 − B04) / (B08 + 6·B04 − 7.5·B02 + 1). Saturation-resistant veg.",
        )),
        "indices.nbr" => Some((
            vec![B08, B12],
            "index_nbr",
            "NBR = (B08 − B12) / (B08 + B12). Burn-severity ratio.",
        )),
        "indices.ndmi" => Some((
            vec![B08, B11],
            "index_ndmi",
            "NDMI = (B08 − B11) / (B08 + B11). Canopy moisture.",
        )),
        "indices.savi" => Some((
            vec![B08, B04],
            "index_savi",
            "SAVI (L=0.5) = (1+L)·(B08 − B04) / (B08 + B04 + L). Soil-adjusted veg.",
        )),
        "indices.bsi" => Some((
            vec![B11, B04, B08, B02],
            "index_bsi",
            "BSI = ((B11+B04) − (B08+B02)) / ((B11+B04) + (B08+B02)). Bare-soil index.",
        )),
        "indices.ndbi" => Some((
            vec![B11, B08],
            "index_ndbi",
            "NDBI = (B11 − B08) / (B11 + B08). Built-up index.",
        )),
        // Health-relevant indices. Each formula is from a peer-reviewed paper
        // or recognized public-health authority — see the kind dispatcher in
        // `materialize_sentinel2_band` for the full citation per index.
        "indices.ndti" => Some((
            vec![B04, B03],
            "index_ndti",
            "NDTI (Lacaux 2007) = (B04 − B03) / (B04 + B03). Water turbidity proxy; higher → diarrheal-disease risk and mosquito breeding.",
        )),
        "indices.gndvi" => Some((
            vec![B08, B03],
            "index_gndvi",
            "GNDVI (Gitelson 1996) = (B08 − B03) / (B08 + B03). Pasture/crop chlorophyll proxy; higher → better local nutrition.",
        )),
        "indices.ndre" => Some((
            vec![B8A, B05],
            "index_ndre",
            "NDRE (Gitelson 1996) = (B8A − B05) / (B8A + B05). Red-edge chlorophyll; crop nitrogen → dietary protein supply.",
        )),
        "indices.fai" => Some((
            vec![B08, B04, B11],
            "index_fai",
            "FAI (Hu 2009) = B08 − [B04 + (842−665)/(1610−665)·(B11 − B04)]. Floating algae / scum surface signature; harmful algal bloom risk.",
        )),
        "indices.tss" => Some((
            vec![B04, B02],
            "index_tss",
            "TSS (Ouma 2020) = 14.464·(B04/B02) + 16.336 mg/L. Total suspended solids; pathogen-laden runoff after storms.",
        )),
        "indices.ndsi" => Some((
            vec![B03, B11],
            "index_ndsi",
            "NDSI (Hall 1995) = (B03 − B11) / (B03 + B11). Snow cover; high values → tick-season delay, snow-reflected UV burn at altitude.",
        )),
        "indices.afri1600" => Some((
            vec![B08, B11],
            "index_afri1600",
            "AFRI1.6 (Karnieli 2001) = (B08 − 0.66·B11) / (B08 + 0.66·B11). Aerosol-resistant veg index — usable through smoke/dust where NDVI is biased.",
        )),
        "indices.savi_l1" => Some((
            vec![B08, B04],
            "index_savi_l1",
            "SAVI L=1 (Huete 1988) = 2·(B08 − B04) / (B08 + B04 + 1). Soil-adjusted veg in arid pastures — feeds drought-nutrition signal.",
        )),
        "indices.surface_dryness" => Some((
            vec![B08, B11],
            "index_surface_dryness",
            "SDI = 1 − NDMI = 1 − (B08 − B11)/(B08 + B11), clamped to [0,1]. Compound heat-stress signal; no evaporative cooling when high.",
        )),
        "indices.urban_canopy_index" => Some((
            vec![B08, B04, B11],
            "index_urban_canopy",
            "UCI = NDVI · (1 − NDBI) where NDBI = (B11 − B08)/(B11 + B08). Tree-canopy density in urban grid; ↑ = mental-health and mortality benefit (3-30-300 rule).",
        )),
        _ => None,
    }
}

/// Generic Sentinel-2 L2A point sampler. Handles every `s2.*` raw-reflectance
/// band and every `indices.*` derived index from the same one-scene path:
///
/// 1. STAC search Element84 → latest scene <40% cloud that *contains* the
///    point (intersects: Point, never bbox).
/// 2. For each STAC asset the band needs (1..4 of them), open the COG profile
///    via HTTP range read and sample one pixel.
/// 3. Compute the band value (raw scale 1e-4 reflectance, SCL category, or
///    a deterministic index formula).
/// 4. Sign one Primary fact under the responder identity. The
///    `derivation.fn_key` records the formula so external attesters can
///    re-execute and corroborate.
///
/// One STAC search per call, so per-band cost is dominated by the per-asset
/// COG reads (~600 KB each: IFD head + 1 tile).
async fn materialize_sentinel2_band(
    cell64: &str,
    s: &AppState,
    band: &str,
    target_unix: Option<i64>,
) -> Result<emem_fact::FactCid, String> {
    let plan = s2_band_plan(band).ok_or_else(|| format!("unknown s2 band {band}"))?;
    let (asset_lists, kind, formula_note) = plan;
    let info = emem_codec::latlng_from_cell64(cell64).map_err(|e| format!("cell decode: {e}"))?;
    let lat = info.lat_deg;
    let lng = info.lng_deg;

    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    // Backfill window: ±30d around target_unix (covers ≥4 revisits at most
    // latitudes); current-mode is "last 30d up to now".
    let (lo_unix, hi_unix) = match target_unix {
        Some(t) => {
            let lo = (t - 30 * 86400).max(0);
            let hi = (t + 30 * 86400).min(now_unix);
            (lo, hi.max(lo + 86400))
        }
        None => (now_unix - 30 * 86400, now_unix),
    };
    let datetime = format!(
        "{}/{}",
        iso8601_utc(lo_unix as u64),
        iso8601_utc(hi_unix as u64)
    );

    let cli = s2_http_client();
    let item =
        emem_fetch::stac::search_one(&cli, "sentinel-2-l2a", lng, lat, &datetime, Some(40.0))
            .await
            .map_err(|e| format!("stac: {e}"))?
            .ok_or_else(|| match target_unix {
                Some(t) => format!("no Sentinel-2 L2A scene under 40% cloud within ±30d of {t}"),
                None => "no Sentinel-2 L2A scene under 40% cloud in last 30 days".to_string(),
            })?;
    let epsg = item
        .epsg
        .ok_or_else(|| "stac item missing proj:epsg".to_string())?;
    let utm = emem_fetch::proj::latlng_to_utm_with_epsg(lat, lng, epsg)
        .ok_or_else(|| format!("epsg {epsg} not a UTM code"))?;

    // Resolve each asset alias chain to a URL, open the profile, sample.
    let mut samples = Vec::with_capacity(asset_lists.len());
    let mut asset_urls: Vec<String> = Vec::with_capacity(asset_lists.len());
    for aliases in &asset_lists {
        let mut url: Option<String> = None;
        for alias in *aliases {
            if let Some(u) = item.assets.get(*alias) {
                url = Some(u.clone());
                break;
            }
        }
        let url = url.ok_or_else(|| format!("stac item missing any of {:?}", aliases))?;
        let prof = emem_fetch::cog::open_profile(&cli, &url)
            .await
            .map_err(|e| format!("open COG {url}: {e}"))?;
        let v = emem_fetch::cog::sample_pixel(&cli, &url, &prof, utm.easting, utm.northing)
            .await
            .map_err(|e| format!("sample {url}: {e}"))?;
        samples.push(v);
        asset_urls.push(url);
    }

    // Compute the band value.
    let (value, fact_unit) = match kind {
        "raw_reflectance" => {
            let raw = samples[0];
            if raw == 0.0 {
                return Err(format!(
                    "nodata at scene={} (band={band} raw={raw})",
                    item.id
                ));
            }
            let refl = raw * 1e-4;
            (refl, None)
        }
        "scl_categorical" => {
            // SCL is uint8 0..11; treat 0 as no-data per ESA spec.
            let raw = samples[0];
            if raw == 0.0 {
                return Err(format!("scl=0 (no-data) at scene={}", item.id));
            }
            (raw, Some("class_index".to_string()))
        }
        "index_ndvi" => {
            let nir = samples[0] * 1e-4;
            let red = samples[1] * 1e-4;
            if nir + red < 1e-6 {
                return Err("ndvi denom ≈ 0".to_string());
            }
            ((nir - red) / (nir + red), None)
        }
        "index_ndwi" => {
            let g = samples[0] * 1e-4;
            let nir = samples[1] * 1e-4;
            if g + nir < 1e-6 {
                return Err("ndwi denom ≈ 0".to_string());
            }
            ((g - nir) / (g + nir), None)
        }
        "index_mndwi" => {
            let g = samples[0] * 1e-4;
            let swir = samples[1] * 1e-4;
            if g + swir < 1e-6 {
                return Err("mndwi denom ≈ 0".to_string());
            }
            ((g - swir) / (g + swir), None)
        }
        "index_evi" => {
            let nir = samples[0] * 1e-4;
            let red = samples[1] * 1e-4;
            let blue = samples[2] * 1e-4;
            let denom = nir + 6.0 * red - 7.5 * blue + 1.0;
            if denom.abs() < 1e-6 {
                return Err("evi denom ≈ 0".to_string());
            }
            (2.5 * (nir - red) / denom, None)
        }
        "index_nbr" => {
            let nir = samples[0] * 1e-4;
            let swir2 = samples[1] * 1e-4;
            if nir + swir2 < 1e-6 {
                return Err("nbr denom ≈ 0".to_string());
            }
            ((nir - swir2) / (nir + swir2), None)
        }
        "index_ndmi" => {
            let nir = samples[0] * 1e-4;
            let swir1 = samples[1] * 1e-4;
            if nir + swir1 < 1e-6 {
                return Err("ndmi denom ≈ 0".to_string());
            }
            ((nir - swir1) / (nir + swir1), None)
        }
        "index_savi" => {
            let nir = samples[0] * 1e-4;
            let red = samples[1] * 1e-4;
            let l = 0.5;
            if nir + red + l < 1e-6 {
                return Err("savi denom ≈ 0".to_string());
            }
            ((1.0 + l) * (nir - red) / (nir + red + l), None)
        }
        "index_bsi" => {
            let swir1 = samples[0] * 1e-4;
            let red = samples[1] * 1e-4;
            let nir = samples[2] * 1e-4;
            let blue = samples[3] * 1e-4;
            let num = (swir1 + red) - (nir + blue);
            let den = (swir1 + red) + (nir + blue);
            if den.abs() < 1e-6 {
                return Err("bsi denom ≈ 0".to_string());
            }
            (num / den, None)
        }
        "index_ndbi" => {
            let swir1 = samples[0] * 1e-4;
            let nir = samples[1] * 1e-4;
            if swir1 + nir < 1e-6 {
                return Err("ndbi denom ≈ 0".to_string());
            }
            ((swir1 - nir) / (swir1 + nir), None)
        }
        "index_ndti" => {
            // NDTI (Lacaux et al. 2007, RSE 109:66-77): turbidity proxy used
            // for waterborne-disease vector mapping in the Senegal Valley.
            let red = samples[0] * 1e-4;
            let green = samples[1] * 1e-4;
            if red + green < 1e-6 {
                return Err("ndti denom ≈ 0".to_string());
            }
            ((red - green) / (red + green), None)
        }
        "index_gndvi" => {
            // GNDVI (Gitelson, Kaufman & Merzlyak 1996, RSE 58:289-298):
            // chlorophyll-sensitive vegetation index — saturates later than
            // NDVI over dense canopies, useful for crop nitrogen + pasture.
            let nir = samples[0] * 1e-4;
            let green = samples[1] * 1e-4;
            if nir + green < 1e-6 {
                return Err("gndvi denom ≈ 0".to_string());
            }
            ((nir - green) / (nir + green), None)
        }
        "index_ndre" => {
            // NDRE (Gitelson & Merzlyak 1996, J Plant Physiol 148:494-500):
            // red-edge chlorophyll. Sentinel-2's red-edge band B05 (704 nm)
            // is uniquely mid-range and saturates much later than NIR,
            // exposing nitrogen status during peak biomass.
            let nirn = samples[0] * 1e-4;
            let re1 = samples[1] * 1e-4;
            if nirn + re1 < 1e-6 {
                return Err("ndre denom ≈ 0".to_string());
            }
            ((nirn - re1) / (nirn + re1), None)
        }
        "index_fai" => {
            // FAI (Hu 2009, RSE 113:2118-2129): floating algae index.
            // Linear baseline drawn between red (665 nm) and SWIR1 (1610 nm),
            // measured at NIR (842 nm). Surface scum reflects strongly in
            // NIR but not in SWIR (water absorbs SWIR), so positive FAI
            // identifies HAB scums, sargassum, oil sheens, plastics.
            let nir = samples[0] * 1e-4;
            let red = samples[1] * 1e-4;
            let swir1 = samples[2] * 1e-4;
            // S2 band centers (nm): B04=665, B08=842, B11=1610.
            let lambda_baseline = red + (842.0 - 665.0) / (1610.0 - 665.0) * (swir1 - red);
            (nir - lambda_baseline, None)
        }
        "index_tss" => {
            // TSS (Ouma et al. 2020, J Sensors): empirical regression on
            // East African reservoirs. Formula: TSS [mg/L] = 14.464·(R/B)
            // + 16.336. Valid for moderately turbid inland water; clip
            // to 0 at clearest pixels. Pre-storm vs post-storm Δ is the
            // pathogen-runoff signal.
            let red = samples[0] * 1e-4;
            let blue = samples[1] * 1e-4;
            if blue.abs() < 1e-6 {
                return Err("tss denom (blue) ≈ 0".to_string());
            }
            let tss = 14.464_f64 * (red / blue) + 16.336;
            (tss.max(0.0), Some("mg/L".to_string()))
        }
        "index_ndsi" => {
            // NDSI (Hall, Riggs & Salomonson 1995, RSE 54:127-140): the
            // canonical snow-cover index. Snow is bright in green and dark
            // in SWIR. Threshold ~0.4 separates snow from cloud over land.
            let green = samples[0] * 1e-4;
            let swir1 = samples[1] * 1e-4;
            if green + swir1 < 1e-6 {
                return Err("ndsi denom ≈ 0".to_string());
            }
            ((green - swir1) / (green + swir1), None)
        }
        "index_afri1600" => {
            // AFRI1.6 (Karnieli et al. 2001, RSE 77:10-21): aerosol-free
            // vegetation index for use under smoke/dust/haze. Uses SWIR
            // (which is largely transparent to aerosol) instead of red.
            let nir = samples[0] * 1e-4;
            let swir1 = samples[1] * 1e-4;
            let denom = nir + 0.66 * swir1;
            if denom.abs() < 1e-6 {
                return Err("afri1600 denom ≈ 0".to_string());
            }
            ((nir - 0.66 * swir1) / denom, None)
        }
        "index_savi_l1" => {
            // SAVI with L=1 (Huete 1988, RSE 25:295-309). Heavier soil
            // correction than the canonical L=0.5 version — useful for
            // pasture/desert conditions where bare-soil background is
            // dominant.
            let nir = samples[0] * 1e-4;
            let red = samples[1] * 1e-4;
            let l = 1.0;
            if nir + red + l < 1e-6 {
                return Err("savi_l1 denom ≈ 0".to_string());
            }
            ((1.0 + l) * (nir - red) / (nir + red + l), None)
        }
        "index_surface_dryness" => {
            // SDI = 1 − NDMI, clamped to [0,1]. NDMI > 0 over wet canopy
            // means SDI < 1; bare/dry surfaces produce SDI ≈ 1. Used as
            // the multiplicative compound term in heat-stress: high LST ×
            // high SDI = no evaporative cooling = ER-visit risk.
            let nir = samples[0] * 1e-4;
            let swir1 = samples[1] * 1e-4;
            if nir + swir1 < 1e-6 {
                return Err("sdi denom ≈ 0".to_string());
            }
            let ndmi = (nir - swir1) / (nir + swir1);
            (1.0 - ndmi.clamp(0.0, 1.0), None)
        }
        "index_urban_canopy" => {
            // UCI = NDVI · (1 − NDBI). NDVI captures vegetation; (1−NDBI)
            // dampens the score on built-up pixels so it specifically
            // surfaces TREE canopy in urban grids — supports the WHO
            // "3-30-300" rule (3 trees from every window, 30% canopy
            // cover per neighborhood, 300 m to nearest green space).
            let nir = samples[0] * 1e-4;
            let red = samples[1] * 1e-4;
            let swir1 = samples[2] * 1e-4;
            if nir + red < 1e-6 || swir1 + nir < 1e-6 {
                return Err("uci denom ≈ 0".to_string());
            }
            let ndvi = (nir - red) / (nir + red);
            let ndbi = (swir1 - nir) / (swir1 + nir);
            (ndvi * (1.0 - ndbi), None)
        }
        other => return Err(format!("s2 band kind {other} not implemented")),
    };

    let signed_at = chrono_iso8601_utc();
    let fn_key = format!("sentinel2_l2a_{}@1", band.replace('.', "_"));
    let fact = Fact::Primary(PrimaryFact {
        cell: cell64.to_string(),
        band: band.to_string(),
        tslot: 0,
        value: ciborium::Value::Float(value),
        unit: fact_unit,
        confidence: 0.90,
        uncertainty: None,
        sources: vec![Source {
            scheme: "sentinel_s2_l2a".into(),
            id: asset_urls.join(" ; "),
            cid: None,
            hash: None,
            captured_at: Some(item.datetime.clone()),
            url: None,
        }],
        derivation: Derivation {
            fn_key,
            args: Some(ciborium::Value::Array(vec![
                ciborium::Value::Float(lat),
                ciborium::Value::Float(lng),
                ciborium::Value::Text(item.id.clone()),
                ciborium::Value::Integer((epsg as i64).into()),
                ciborium::Value::Text(formula_note.into()),
                ciborium::Value::Array(
                    samples.iter().map(|v| ciborium::Value::Float(*v)).collect(),
                ),
                ciborium::Value::Float(item.cloud_cover.unwrap_or(-1.0)),
            ])),
        },
        privacy_class: "public".into(),
        schema_cid: SchemaCid::new(s.manifests.schema_cid.as_str()),
        signer: s.identity.pubkey,
        signed_at: signed_at.clone(),
    });
    sign_and_persist(s, fact, &signed_at).await
}

/// Sample Sentinel-1 RTC VV polarisation at the cell centroid via the
/// Microsoft Planetary Computer's `sentinel-1-rtc` collection. RTC =
/// Radiometrically Terrain Corrected gamma-naught backscatter, projected
/// to UTM as proper Cloud-Optimized GeoTIFFs (the upstream Sentinel-1 GRD
/// catalogue on Element84 ships SAFE-format scenes with GCP-based
/// georeferencing that the pure-Rust COG sampler can't decode).
///
/// MPC asset URLs are anonymous Azure Blobs that require an anonymous SAS
/// token — fetched once per process and cached for ~50 min. Output is
/// dB-scaled radar backscatter (10·log10 of the linear gamma0 power).
async fn materialize_sentinel1_vv(
    cell64: &str,
    s: &AppState,
    target_unix: Option<i64>,
) -> Result<emem_fact::FactCid, String> {
    let info = emem_codec::latlng_from_cell64(cell64).map_err(|e| format!("cell decode: {e}"))?;
    let lat = info.lat_deg;
    let lng = info.lng_deg;

    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    // Backfill window: ±15d around target_unix (S1A+B revisit ~6d at most
    // latitudes); current-mode is "last 30d up to now".
    let (lo_unix, hi_unix) = match target_unix {
        Some(t) => {
            let lo = (t - 15 * 86400).max(0);
            let hi = (t + 15 * 86400).min(now_unix);
            (lo, hi.max(lo + 86400))
        }
        None => (now_unix - 30 * 86400, now_unix),
    };
    let datetime = format!(
        "{}/{}",
        iso8601_utc(lo_unix as u64),
        iso8601_utc(hi_unix as u64)
    );

    let cli = s2_http_client();
    let item = emem_fetch::stac::search_one_at(
        &cli,
        emem_fetch::stac::STAC_MPC_V1,
        "sentinel-1-rtc",
        lng,
        lat,
        &datetime,
        None,
    )
    .await
    .map_err(|e| format!("stac: {e}"))?
    .ok_or_else(|| match target_unix {
        Some(t) => format!("no Sentinel-1 RTC scene within ±15d of {t}"),
        None => "no Sentinel-1 RTC scene in last 30 days".to_string(),
    })?;
    let vv_url_raw = item
        .assets
        .get("vv")
        .cloned()
        .or_else(|| item.assets.get("VV").cloned())
        .ok_or_else(|| "stac item missing vv asset".to_string())?;
    // MPC asset URLs are Azure Blob HTTPS. Append the anonymous SAS
    // token as a query string so range reads authenticate.
    let sas = emem_fetch::stac::mpc_sas_token(&cli, "sentinel-1-rtc")
        .await
        .map_err(|e| format!("mpc sas: {e}"))?;
    let sep = if vv_url_raw.contains('?') { '&' } else { '?' };
    let vv_url = format!("{vv_url_raw}{sep}{sas}");
    let epsg = item
        .epsg
        .ok_or_else(|| "stac item missing proj:epsg".to_string())?;
    let prof = emem_fetch::cog::open_profile(&cli, &vv_url)
        .await
        .map_err(|e| format!("open vv COG: {e}"))?;
    let utm = emem_fetch::proj::latlng_to_utm_with_epsg(lat, lng, epsg)
        .ok_or_else(|| format!("epsg {epsg} not a UTM code"))?;
    let vv_lin = emem_fetch::cog::sample_pixel(&cli, &vv_url, &prof, utm.easting, utm.northing)
        .await
        .map_err(|e| format!("sample vv: {e}"))?;
    if !vv_lin.is_finite() || vv_lin <= 0.0 {
        return Err(format!(
            "vv non-positive {vv_lin} (likely water mask or nodata)"
        ));
    }
    let vv_db = 10.0 * vv_lin.log10();

    let signed_at = chrono_iso8601_utc();
    let fact = Fact::Primary(PrimaryFact {
        cell: cell64.to_string(),
        band: "sentinel1_raw".into(),
        tslot: 0,
        value: ciborium::Value::Float(vv_db),
        unit: Some("dB".into()),
        confidence: 0.85,
        uncertainty: None,
        sources: vec![Source {
            scheme: "sentinel_s1_rtc_mpc".into(),
            // Persist the SAS-less URL — the token is short-lived and
            // would invalidate the receipt within an hour. Anyone
            // verifying the source can re-mint a SAS via the same
            // public MPC token endpoint.
            id: vv_url_raw.clone(),
            cid: None,
            hash: None,
            captured_at: Some(item.datetime.clone()),
            url: None,
        }],
        derivation: Derivation {
            fn_key: "sentinel1_rtc_vv_db@1".into(),
            args: Some(ciborium::Value::Array(vec![
                ciborium::Value::Float(lat),
                ciborium::Value::Float(lng),
                ciborium::Value::Text(item.id.clone()),
                ciborium::Value::Integer((epsg as i64).into()),
                ciborium::Value::Float(vv_lin),
            ])),
        },
        privacy_class: "public".into(),
        schema_cid: SchemaCid::new(s.manifests.schema_cid.as_str()),
        signer: s.identity.pubkey,
        signed_at: signed_at.clone(),
    });
    sign_and_persist(s, fact, &signed_at).await
}

/// JRC Global Surface Water v1.4 recurrence at a single pixel.
///
/// Recurrence is the inter-annual variability of water — a u8 percentage
/// 0..=100 saying how often water was present at this 30m pixel during
/// the JRC observation period (1984-2021, Landsat-derived). 0 = never
/// water, 100 = water every year, intermediate values map to flood-prone
/// terrain that is wet in some years and dry in others. nodata=255 means
/// permanent non-water or unmapped — signed as `Fact::Absence`.
///
/// Tile naming: `recurrence_<lon_left10>_<lat_top10>v1_4_2021.tif` over
/// 10° × 10° tiles in EPSG:4326 (lat/lng directly), so the pure-Rust COG
/// sampler skips the UTM projection step the Sentinel-2 path needs.
async fn materialize_jrc_gsw_recurrence(
    cell64: &str,
    s: &AppState,
) -> Result<emem_fact::FactCid, String> {
    let info = emem_codec::latlng_from_cell64(cell64).map_err(|e| format!("cell decode: {e}"))?;
    let lat = info.lat_deg;
    let lng = info.lng_deg;

    // 10° tile grid: lon_left = floor(lng/10)*10 with E/W suffix; lat_top
    // = ceil(lat/10)*10 with N/S suffix. Matches emem_core::Bbox helpers
    // and the URL pattern verified live (recurrence_80E_30Nv1_4_2021.tif
    // → 200 OK at storage.googleapis.com).
    let lon_edge = (lng / 10.0).floor() as i32 * 10;
    let lon_left = if lon_edge >= 0 {
        format!("{}E", lon_edge)
    } else {
        format!("{}W", lon_edge.abs())
    };
    let lat_edge = (lat / 10.0).ceil() as i32 * 10;
    let lat_top = if lat_edge >= 0 {
        format!("{}N", lat_edge)
    } else {
        format!("{}S", lat_edge.abs())
    };
    let url = format!(
        "https://storage.googleapis.com/global-surface-water/downloads2021/recurrence/recurrence_{lon_left}_{lat_top}v1_4_2021.tif",
    );

    let cli = s2_http_client();
    let prof = emem_fetch::cog::open_profile(&cli, &url)
        .await
        .map_err(|e| format!("open jrc gsw cog {url}: {e}"))?;
    // GSW tiles are EPSG:4326 — sample_pixel takes (world_x, world_y),
    // which for geographic CRS means (lng, lat).
    let raw = emem_fetch::cog::sample_pixel(&cli, &url, &prof, lng, lat)
        .await
        .map_err(|e| format!("sample jrc gsw {url}: {e}"))?;

    let signed_at = chrono_iso8601_utc();

    // 255 is JRC's no-data marker (permanent non-water or pixel outside
    // the observation footprint — high-latitude polar interiors, etc.).
    // Sign Absence so subsequent recalls short-circuit.
    if raw == 255.0 || !raw.is_finite() {
        let reason = format!(
            "jrc_gsw_no_data: surface_water.recurrence at ({lat:.6},{lng:.6}) returned 255 or non-finite. \
             JRC Global Surface Water v1.4 uses 255 for permanent non-water and unmapped pixels \
             (high-latitude polar interiors, etc.). This cell is recorded as a confirmed absence \
             — neither measurable inter-annual water recurrence nor part of the observation footprint."
        );
        let reason_cid = reason_cid_for(&reason);
        let fact = Fact::Absence(NegativeFact {
            cell: cell64.to_string(),
            band: "surface_water.recurrence".into(),
            tslot: 0,
            reason_cid,
            confidence: 1.0,
            sources: vec![Source {
                scheme: "jrc.gsw.v1_4.recurrence".into(),
                id: url.clone(),
                cid: None,
                hash: None,
                captured_at: Some(signed_at.clone()),
                url: None,
            }],
            schema_cid: SchemaCid::new(s.manifests.schema_cid.as_str()),
            signer: s.identity.pubkey,
            signed_at: signed_at.clone(),
        });
        return sign_and_persist(s, fact, &signed_at).await;
    }

    // Bound-check: any value outside [0,100] (and not 255) is a parse
    // error or corrupt tile. The protocol's no-fallback rule applies.
    if !(0.0..=100.0).contains(&raw) {
        return Err(format!(
            "jrc gsw recurrence pixel out of range: raw={raw} at ({lat:.6},{lng:.6}) tile {lon_left}_{lat_top}; \
             expected 0..=100 (% inter-annual recurrence) or 255 (no_data)"
        ));
    }

    let fact = Fact::Primary(PrimaryFact {
        cell: cell64.to_string(),
        band: "surface_water.recurrence".into(),
        tslot: 0,
        value: ciborium::Value::Float(raw),
        unit: Some("percent".into()),
        confidence: 0.95,
        uncertainty: None,
        sources: vec![Source {
            scheme: "jrc.gsw.v1_4.recurrence".into(),
            id: url.clone(),
            cid: None,
            hash: None,
            captured_at: Some(signed_at.clone()),
            url: None,
        }],
        derivation: Derivation {
            fn_key: "jrc_gsw_recurrence_pixel@1".into(),
            args: Some(ciborium::Value::Array(vec![
                ciborium::Value::Float(lat),
                ciborium::Value::Float(lng),
                ciborium::Value::Text(format!("{lon_left}_{lat_top}")),
            ])),
        },
        privacy_class: "public".into(),
        schema_cid: SchemaCid::new(s.manifests.schema_cid.as_str()),
        signer: s.identity.pubkey,
        signed_at: signed_at.clone(),
    });
    sign_and_persist(s, fact, &signed_at).await
}

// ---------------- Overture Maps materializers ----------------
//
// Three scalar bands derived from anonymous-S3 GeoParquet (Overture Maps
// Foundation). Pure-Rust path: object_store anonymous AWS S3 → parquet
// row-group pruning over `bbox` struct stats → Arrow record batches →
// minimal WKB decode (Point / LineString / Polygon centroid). No Python,
// no GDAL, no auth header.
//
// All three call into emem_fetch::overture::OvertureClient::shared(),
// which lazily lists files for the configured release and caches per-file
// parquet footers in process memory so a second cell in the same area is
// cheap.

async fn materialize_overture_buildings_count(
    cell64: &str,
    s: &AppState,
) -> Result<emem_fact::FactCid, String> {
    let info = emem_codec::latlng_from_cell64(cell64).map_err(|e| format!("cell decode: {e}"))?;
    let bb = info.bbox_deg;
    let cli = emem_fetch::overture::OvertureClient::shared();
    let n = cli
        .buildings_count_in_bbox(bb.min_lat, bb.max_lat, bb.min_lng, bb.max_lng)
        .await
        .map_err(|e| format!("overture buildings: {e}"))?;
    let signed_at = chrono_iso8601_utc();
    let release = cli.release().to_string();
    let upstream =
        format!("s3://overturemaps-us-west-2/release/{release}/theme=buildings/type=building/");
    let upstream_url = format!(
        "https://overturemaps-us-west-2.s3.amazonaws.com/release/{release}/theme=buildings/type=building/"
    );
    let fact = Fact::Primary(PrimaryFact {
        cell: cell64.to_string(),
        band: "overture.buildings.count".into(),
        tslot: 0,
        value: ciborium::Value::Integer((n as i64).into()),
        unit: None,
        confidence: 0.95,
        uncertainty: None,
        sources: vec![Source {
            scheme: "overture.maps.foundation.v1".into(),
            id: upstream,
            cid: None,
            hash: None,
            captured_at: Some(release.clone()),
            url: Some(upstream_url),
        }],
        derivation: Derivation {
            fn_key: "overture_buildings_count@1".into(),
            args: Some(ciborium::Value::Array(vec![
                ciborium::Value::Float(bb.min_lat),
                ciborium::Value::Float(bb.max_lat),
                ciborium::Value::Float(bb.min_lng),
                ciborium::Value::Float(bb.max_lng),
                ciborium::Value::Text(release),
            ])),
        },
        privacy_class: "public".into(),
        schema_cid: SchemaCid::new(s.manifests.schema_cid.as_str()),
        signer: s.identity.pubkey,
        signed_at: signed_at.clone(),
    });
    sign_and_persist(s, fact, &signed_at).await
}

async fn materialize_overture_places_count(
    cell64: &str,
    s: &AppState,
) -> Result<emem_fact::FactCid, String> {
    let info = emem_codec::latlng_from_cell64(cell64).map_err(|e| format!("cell decode: {e}"))?;
    let bb = info.bbox_deg;
    let cli = emem_fetch::overture::OvertureClient::shared();
    let n = cli
        .places_count_in_bbox(bb.min_lat, bb.max_lat, bb.min_lng, bb.max_lng)
        .await
        .map_err(|e| format!("overture places: {e}"))?;
    let signed_at = chrono_iso8601_utc();
    let release = cli.release().to_string();
    let upstream =
        format!("s3://overturemaps-us-west-2/release/{release}/theme=places/type=place/");
    let upstream_url = format!(
        "https://overturemaps-us-west-2.s3.amazonaws.com/release/{release}/theme=places/type=place/"
    );
    let fact = Fact::Primary(PrimaryFact {
        cell: cell64.to_string(),
        band: "overture.places.count".into(),
        tslot: 0,
        value: ciborium::Value::Integer((n as i64).into()),
        unit: None,
        confidence: 0.90,
        uncertainty: None,
        sources: vec![Source {
            scheme: "overture.maps.foundation.v1".into(),
            id: upstream,
            cid: None,
            hash: None,
            captured_at: Some(release.clone()),
            url: Some(upstream_url),
        }],
        derivation: Derivation {
            fn_key: "overture_places_count@1".into(),
            args: Some(ciborium::Value::Array(vec![
                ciborium::Value::Float(bb.min_lat),
                ciborium::Value::Float(bb.max_lat),
                ciborium::Value::Float(bb.min_lng),
                ciborium::Value::Float(bb.max_lng),
                ciborium::Value::Text(release),
            ])),
        },
        privacy_class: "public".into(),
        schema_cid: SchemaCid::new(s.manifests.schema_cid.as_str()),
        signer: s.identity.pubkey,
        signed_at: signed_at.clone(),
    });
    sign_and_persist(s, fact, &signed_at).await
}

async fn materialize_overture_road_length_m(
    cell64: &str,
    s: &AppState,
) -> Result<emem_fact::FactCid, String> {
    let info = emem_codec::latlng_from_cell64(cell64).map_err(|e| format!("cell decode: {e}"))?;
    let bb = info.bbox_deg;
    let cli = emem_fetch::overture::OvertureClient::shared();
    let length_m = cli
        .road_length_m_in_bbox(bb.min_lat, bb.max_lat, bb.min_lng, bb.max_lng)
        .await
        .map_err(|e| format!("overture transportation: {e}"))?;
    let signed_at = chrono_iso8601_utc();
    let release = cli.release().to_string();
    let upstream =
        format!("s3://overturemaps-us-west-2/release/{release}/theme=transportation/type=segment/");
    let upstream_url = format!(
        "https://overturemaps-us-west-2.s3.amazonaws.com/release/{release}/theme=transportation/type=segment/"
    );
    let fact = Fact::Primary(PrimaryFact {
        cell: cell64.to_string(),
        band: "overture.transportation.road_length_m".into(),
        tslot: 0,
        value: ciborium::Value::Float(length_m),
        unit: Some("m".into()),
        confidence: 0.85,
        uncertainty: None,
        sources: vec![Source {
            scheme: "overture.maps.foundation.v1".into(),
            id: upstream,
            cid: None,
            hash: None,
            captured_at: Some(release.clone()),
            url: Some(upstream_url),
        }],
        derivation: Derivation {
            fn_key: "overture_road_length_m@1".into(),
            args: Some(ciborium::Value::Array(vec![
                ciborium::Value::Float(bb.min_lat),
                ciborium::Value::Float(bb.max_lat),
                ciborium::Value::Float(bb.min_lng),
                ciborium::Value::Float(bb.max_lng),
                ciborium::Value::Text(release),
            ])),
        },
        privacy_class: "public".into(),
        schema_cid: SchemaCid::new(s.manifests.schema_cid.as_str()),
        signer: s.identity.pubkey,
        signed_at: signed_at.clone(),
    });
    sign_and_persist(s, fact, &signed_at).await
}

// ---------------- ESA WorldCover 2021 materializer ----------------
//
// 11-class global landcover at 10 m, 3°×3° tile grid named by the SW
// corner. Anonymous AWS S3 (CC BY 4.0). Tiles only exist over land +
// coastal strips; ocean cells return Absence.
//
// Class values per ESA WorldCover product user manual v2.0 (2022):
//   10  Tree cover            20  Shrubland           30  Grassland
//   40  Cropland              50  Built-up            60  Bare/sparse
//   70  Snow/ice              80  Permanent water     90  Herbaceous wet
//   95  Mangroves            100  Moss & lichen
async fn materialize_esa_worldcover_2021(
    cell64: &str,
    s: &AppState,
) -> Result<emem_fact::FactCid, String> {
    let info = emem_codec::latlng_from_cell64(cell64).map_err(|e| format!("cell decode: {e}"))?;
    let lat = info.lat_deg;
    let lng = info.lng_deg;
    // SW-corner naming on a 3° grid; lat is N/S, lng is E/W.
    let lat_floor = (lat / 3.0).floor() as i32 * 3;
    let lng_floor = (lng / 3.0).floor() as i32 * 3;
    let lat_tag = if lat_floor >= 0 {
        format!("N{:02}", lat_floor)
    } else {
        format!("S{:02}", lat_floor.abs())
    };
    let lng_tag = if lng_floor >= 0 {
        format!("E{:03}", lng_floor)
    } else {
        format!("W{:03}", lng_floor.abs())
    };
    let url = format!(
        "https://esa-worldcover.s3.eu-central-1.amazonaws.com/v200/2021/map/ESA_WorldCover_10m_2021_v200_{lat_tag}{lng_tag}_Map.tif",
    );

    let cli = s2_http_client();
    let signed_at = chrono_iso8601_utc();

    let prof = match emem_fetch::cog::open_profile(&cli, &url).await {
        Ok(p) => p,
        Err(e) => {
            // 404 over open ocean is the documented gap in coverage; sign
            // Absence rather than a transport error so subsequent recalls
            // short-circuit.
            let es = e.to_string();
            if es.contains("404") || es.contains("Not Found") {
                let reason = format!(
                    "esa_worldcover_no_tile: ESA WorldCover 2021 v200 publishes no tile {lat_tag}{lng_tag} ({}). \
                     This is the documented behaviour over open ocean and unmapped polar interiors. \
                     Cell ({lat:.6},{lng:.6}) lies outside the land+coastal mask.",
                    url
                );
                let reason_cid = reason_cid_for(&reason);
                let fact = Fact::Absence(NegativeFact {
                    cell: cell64.to_string(),
                    band: "esa_worldcover.lc_2021".into(),
                    tslot: 0,
                    reason_cid,
                    confidence: 1.0,
                    sources: vec![Source {
                        scheme: "esa.worldcover.v200.2021".into(),
                        id: url.clone(),
                        cid: None,
                        hash: None,
                        captured_at: Some(signed_at.clone()),
                        url: None,
                    }],
                    schema_cid: SchemaCid::new(s.manifests.schema_cid.as_str()),
                    signer: s.identity.pubkey,
                    signed_at: signed_at.clone(),
                });
                return sign_and_persist(s, fact, &signed_at).await;
            }
            return Err(format!("open esa_worldcover cog {url}: {e}"));
        }
    };
    let raw = emem_fetch::cog::sample_pixel(&cli, &url, &prof, lng, lat)
        .await
        .map_err(|e| format!("sample esa_worldcover {url}: {e}"))?;

    if !raw.is_finite() || raw == 0.0 {
        let reason = format!(
            "esa_worldcover_no_class: ESA WorldCover pixel at ({lat:.6},{lng:.6}) returned 0/non-finite. \
             0 is the WorldCover no-data marker (mostly over the open-ocean tile borders that v200 still ships)."
        );
        let reason_cid = reason_cid_for(&reason);
        let fact = Fact::Absence(NegativeFact {
            cell: cell64.to_string(),
            band: "esa_worldcover.lc_2021".into(),
            tslot: 0,
            reason_cid,
            confidence: 1.0,
            sources: vec![Source {
                scheme: "esa.worldcover.v200.2021".into(),
                id: url.clone(),
                cid: None,
                hash: None,
                captured_at: Some(signed_at.clone()),
                url: None,
            }],
            schema_cid: SchemaCid::new(s.manifests.schema_cid.as_str()),
            signer: s.identity.pubkey,
            signed_at: signed_at.clone(),
        });
        return sign_and_persist(s, fact, &signed_at).await;
    }
    let class_int = raw.round() as i64;
    if !matches!(
        class_int,
        10 | 20 | 30 | 40 | 50 | 60 | 70 | 80 | 90 | 95 | 100
    ) {
        return Err(format!(
            "esa_worldcover unexpected class {class_int} at ({lat:.6},{lng:.6}); \
             documented values are {{10,20,30,40,50,60,70,80,90,95,100}}"
        ));
    }
    let fact = Fact::Primary(PrimaryFact {
        cell: cell64.to_string(),
        band: "esa_worldcover.lc_2021".into(),
        tslot: 0,
        value: ciborium::Value::Integer(class_int.into()),
        unit: Some("lccs_class".into()),
        confidence: 0.92,
        uncertainty: None,
        sources: vec![Source {
            scheme: "esa.worldcover.v200.2021".into(),
            id: url.clone(),
            cid: None,
            hash: None,
            captured_at: Some(signed_at.clone()),
            url: None,
        }],
        derivation: Derivation {
            fn_key: "esa_worldcover_2021_pixel@1".into(),
            args: Some(ciborium::Value::Array(vec![
                ciborium::Value::Float(lat),
                ciborium::Value::Float(lng),
                ciborium::Value::Text(format!("{lat_tag}{lng_tag}")),
            ])),
        },
        privacy_class: "public".into(),
        schema_cid: SchemaCid::new(s.manifests.schema_cid.as_str()),
        signer: s.identity.pubkey,
        signed_at: signed_at.clone(),
    });
    sign_and_persist(s, fact, &signed_at).await
}

// ---------------- Hansen Global Forest Change materializer ----------------
//
// Three layers from the GFC v1.11 (2023) release:
//   - hansen.tree_cover_2000  (byte 0..100, % canopy cover at 30 m)
//   - hansen.loss_year        (byte 0..23, year of forest loss; 0 = no loss,
//                              1 = 2001, ..., 23 = 2023)
//   - hansen.gain             (byte 0/1, 2000–2012 gain mask)
//
// 10°×10° tiles on storage.googleapis.com (public-fetch HTTPS), named by
// the NW corner: Hansen_GFC-2023-v1.11_<layer>_<lat_top>_<lng_left>.tif.
async fn materialize_hansen_band(
    cell64: &str,
    s: &AppState,
    band: &str,
) -> Result<emem_fact::FactCid, String> {
    let layer = match band {
        "hansen.tree_cover_2000" => "treecover2000",
        "hansen.loss_year" => "lossyear",
        "hansen.gain" => "gain",
        _ => return Err(format!("hansen band {band} not registered")),
    };
    let info = emem_codec::latlng_from_cell64(cell64).map_err(|e| format!("cell decode: {e}"))?;
    let lat = info.lat_deg;
    let lng = info.lng_deg;
    // NW-corner naming on a 10° grid.
    let lat_top = (lat / 10.0).ceil() as i32 * 10;
    let lng_left = (lng / 10.0).floor() as i32 * 10;
    let lat_tag = if lat_top >= 0 {
        format!("{:02}N", lat_top)
    } else {
        format!("{:02}S", lat_top.abs())
    };
    let lng_tag = if lng_left >= 0 {
        format!("{:03}E", lng_left)
    } else {
        format!("{:03}W", lng_left.abs())
    };
    let url = format!(
        "https://storage.googleapis.com/earthenginepartners-hansen/GFC-2023-v1.11/Hansen_GFC-2023-v1.11_{layer}_{lat_tag}_{lng_tag}.tif"
    );

    let cli = s2_http_client();
    let signed_at = chrono_iso8601_utc();

    let prof = emem_fetch::cog::open_profile(&cli, &url)
        .await
        .map_err(|e| format!("open hansen cog {url}: {e}"))?;
    let raw = emem_fetch::cog::sample_pixel(&cli, &url, &prof, lng, lat)
        .await
        .map_err(|e| format!("sample hansen {url}: {e}"))?;

    if !raw.is_finite() {
        return Err(format!(
            "hansen pixel non-finite at ({lat:.6},{lng:.6}) {url}"
        ));
    }
    let v = raw.round() as i64;
    let (unit, lo, hi) = match band {
        "hansen.tree_cover_2000" => ("percent_canopy_cover", 0i64, 100i64),
        "hansen.loss_year" => ("year_offset_from_2000", 0i64, 23i64),
        "hansen.gain" => ("binary", 0i64, 1i64),
        _ => unreachable!(),
    };
    if v < lo || v > hi {
        return Err(format!(
            "hansen {band} pixel out of range: raw={raw} (rounded={v}), expected [{lo},{hi}] at ({lat:.6},{lng:.6}) tile {lat_tag}_{lng_tag}"
        ));
    }
    let fact = Fact::Primary(PrimaryFact {
        cell: cell64.to_string(),
        band: band.to_string(),
        tslot: 0,
        value: ciborium::Value::Integer(v.into()),
        unit: Some(unit.into()),
        confidence: 0.93,
        uncertainty: None,
        sources: vec![Source {
            scheme: "hansen.gfc.v1_11.2023".into(),
            id: url.clone(),
            cid: None,
            hash: None,
            captured_at: Some(signed_at.clone()),
            url: None,
        }],
        derivation: Derivation {
            fn_key: "hansen_gfc_v1_11_pixel@1".into(),
            args: Some(ciborium::Value::Array(vec![
                ciborium::Value::Float(lat),
                ciborium::Value::Float(lng),
                ciborium::Value::Text(format!("{lat_tag}_{lng_tag}")),
                ciborium::Value::Text(layer.into()),
            ])),
        },
        privacy_class: "public".into(),
        schema_cid: SchemaCid::new(s.manifests.schema_cid.as_str()),
        signer: s.identity.pubkey,
        signed_at: signed_at.clone(),
    });
    sign_and_persist(s, fact, &signed_at).await
}

/// Build the merkle-rooted Attestation around one fact, sign it under the
/// responder's identity, persist it, and return the fact CID. Centralises
/// the boilerplate that all materializers share.
async fn sign_and_persist(
    s: &AppState,
    fact: Fact,
    signed_at: &str,
) -> Result<emem_fact::FactCid, String> {
    let mut buf = Vec::new();
    ciborium::ser::into_writer(&fact, &mut buf).map_err(|e| format!("cbor encode: {e}"))?;
    let leaf_hash = blake3::hash(&buf);
    let mut leaf = [0u8; 32];
    leaf.copy_from_slice(leaf_hash.as_bytes());
    let batch_root = emem_attest::merkle_root(&[leaf]);
    let mut h = blake3::Hasher::new();
    h.update(&batch_root);
    h.update(s.manifests.registry_cid.as_str().as_bytes());
    h.update(s.manifests.schema_cid.as_str().as_bytes());
    let signed_digest = h.finalize();
    let sig = s.identity.signing.sign(signed_digest.as_bytes());
    let mut sig_bytes = [0u8; 64];
    sig_bytes.copy_from_slice(&sig.to_bytes());
    let att = Attestation {
        facts: vec![fact],
        batch_root,
        attester: s.identity.pubkey,
        attester_key_epoch: KeyEpoch(s.identity.epoch.0),
        registry_cid: RegistryCid::new(s.manifests.registry_cid.as_str()),
        schema_cid: SchemaCid::new(s.manifests.schema_cid.as_str()),
        stake: None,
        signature: EmCoreSignature(sig_bytes),
        attested_at: signed_at.to_string(),
    };
    let cids = s
        .storage
        .put_attestation(&att)
        .await
        .map_err(|e| format!("put_attestation: {e}"))?;
    cids.into_iter()
        .next()
        .ok_or_else(|| "put_attestation returned no fact_cid".to_string())
}

/// Long-timeout HTTP client for STAC + COG range reads. The default
/// `reqwest_client()` uses an 8-s timeout that is too tight for the
/// multi-step COG path (STAC POST + IFD head + tile range read).
fn s2_http_client() -> reqwest::Client {
    static C: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(45))
            .build()
            .unwrap_or_default()
    })
    .clone()
}

/// Parse a NumPy `.npy` header from the leading bytes of the file.
/// Returns (shape, dtype, data_offset) where data_offset is the byte offset
/// of the first data element. Supports v1 and v2 headers; only the small
/// subset we need (no record arrays, no object dtypes).
fn parse_npy_header(buf: &[u8]) -> Result<(Vec<usize>, String, usize), String> {
    if buf.len() < 10 || &buf[..6] != b"\x93NUMPY" {
        return Err("not a .npy file (magic missing)".into());
    }
    let major = buf[6];
    let (hdr_len, body_off) = match major {
        1 => {
            let n = u16::from_le_bytes([buf[8], buf[9]]) as usize;
            (n, 10usize)
        }
        2 | 3 => {
            if buf.len() < 12 {
                return Err("v2 header too short".into());
            }
            let n = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]) as usize;
            (n, 12usize)
        }
        v => return Err(format!("unsupported npy version {v}")),
    };
    let end = body_off + hdr_len;
    if buf.len() < end {
        return Err(format!(
            "npy header truncated: have {} need {}",
            buf.len(),
            end
        ));
    }
    let hdr = std::str::from_utf8(&buf[body_off..end])
        .map_err(|e| format!("npy header utf8: {e}"))?
        .trim_matches(|c: char| c == ' ' || c == '\n');
    // Header is a Python literal dict. Extract by string surgery — tiny
    // and known-shape so a real Python parser is overkill.
    let dtype = extract_str_field(hdr, "descr").ok_or("no descr")?;
    let shape_s = extract_paren_field(hdr, "shape").ok_or("no shape")?;
    let mut shape = Vec::new();
    for tok in shape_s.split(',') {
        let t = tok.trim();
        if t.is_empty() {
            continue;
        }
        let n: usize = t.parse().map_err(|e| format!("shape parse {t:?}: {e}"))?;
        shape.push(n);
    }
    Ok((shape, dtype, end))
}

fn extract_str_field(hdr: &str, key: &str) -> Option<String> {
    // Looks for `'key': '<value>'`
    let needle = format!("'{key}':");
    let i = hdr.find(&needle)?;
    let rest = &hdr[i + needle.len()..];
    let q1 = rest.find('\'')?;
    let after = &rest[q1 + 1..];
    let q2 = after.find('\'')?;
    Some(after[..q2].to_string())
}

fn extract_paren_field(hdr: &str, key: &str) -> Option<String> {
    let needle = format!("'{key}':");
    let i = hdr.find(&needle)?;
    let rest = &hdr[i + needle.len()..];
    let p1 = rest.find('(')?;
    let after = &rest[p1 + 1..];
    let p2 = after.find(')')?;
    Some(after[..p2].to_string())
}

/// Convert a Unix epoch second to an ORNL MODIS date code `A<YYYY><DOY>`.
fn unix_to_modis_date(unix_s: i64) -> String {
    // Compute year + day-of-year from days-since-1970 using inverse of
    // Howard Hinnant's date algorithm.
    let days = unix_s.div_euclid(86400);
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy0 = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy0 + 2) / 153;
    let d = doy0 - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let yy = if m <= 2 { y + 1 } else { y };
    // Day of year (1-indexed).
    let is_leap = (yy % 4 == 0 && yy % 100 != 0) || yy % 400 == 0;
    let cum: [i64; 13] = if is_leap {
        [0, 31, 60, 91, 121, 152, 182, 213, 244, 274, 305, 335, 366]
    } else {
        [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334, 365]
    };
    let doy = cum[(m - 1) as usize] + d;
    format!("A{:04}{:03}", yy, doy)
}

/// Build + persist a signed `Fact::Absence` for `copdem30m.elevation_mean`
/// at this cell. The reason text is hashed to a `ReasonCid` so anyone
/// reading the fact can verify the responder asserted *this exact*
/// reason at attestation time.
async fn sign_elevation_absence(
    cell64: &str,
    s: &AppState,
    upstream_url: &str,
    signed_at: &str,
    reason_text: &str,
) -> Result<emem_fact::FactCid, String> {
    let reason_cid = reason_cid_for(reason_text);
    let fact = Fact::Absence(NegativeFact {
        cell: cell64.to_string(),
        band: "copdem30m.elevation_mean".into(),
        tslot: 0,
        reason_cid,
        confidence: 1.0,
        sources: vec![Source {
            scheme: "open_meteo".into(),
            id: upstream_url.to_string(),
            cid: None,
            hash: None,
            captured_at: Some(signed_at.to_string()),
            url: None,
        }],
        schema_cid: SchemaCid::new(s.manifests.schema_cid.as_str()),
        signer: s.identity.pubkey,
        signed_at: signed_at.to_string(),
    });

    let mut buf = Vec::new();
    ciborium::ser::into_writer(&fact, &mut buf).map_err(|e| format!("cbor encode: {e}"))?;
    let leaf_hash = blake3::hash(&buf);
    let mut leaf = [0u8; 32];
    leaf.copy_from_slice(leaf_hash.as_bytes());
    let batch_root = emem_attest::merkle_root(&[leaf]);

    let mut h = blake3::Hasher::new();
    h.update(&batch_root);
    h.update(s.manifests.registry_cid.as_str().as_bytes());
    h.update(s.manifests.schema_cid.as_str().as_bytes());
    let signed_digest = h.finalize();
    let sig = s.identity.signing.sign(signed_digest.as_bytes());
    let mut sig_bytes = [0u8; 64];
    sig_bytes.copy_from_slice(&sig.to_bytes());

    let att = Attestation {
        facts: vec![fact],
        batch_root,
        attester: s.identity.pubkey,
        attester_key_epoch: KeyEpoch(s.identity.epoch.0),
        registry_cid: RegistryCid::new(s.manifests.registry_cid.as_str()),
        schema_cid: SchemaCid::new(s.manifests.schema_cid.as_str()),
        stake: None,
        signature: EmCoreSignature(sig_bytes),
        attested_at: signed_at.to_string(),
    };

    let cids = s
        .storage
        .put_attestation(&att)
        .await
        .map_err(|e| format!("put_attestation (absence): {e}"))?;
    cids.into_iter()
        .next()
        .ok_or_else(|| "put_attestation (absence) returned no fact_cid".to_string())
}

/// Outcome of one materialization attempt.
struct MaterializeOutcome {
    band: String,
    /// `Some(fact_cid_str)` on success.
    fact_cid: Option<String>,
    /// `Some(reason)` on skip/failure — surfaced to the agent so they
    /// know *why* this band is not available, not just that it isn't.
    skip_reason: Option<String>,
}

/// Resolve the tempo class for any band the responder knows about. Cube
/// bands are sourced from the canonical bands manifest; pure-materializer
/// bands (weather, modis, gmrt, copdem, …) declare their tempo via the
/// `band_materializer_meta` table below — kept in sync with the live
/// materializer registry exposed via `/v1/materializers`.
fn tempo_for_band(band: &str) -> Option<emem_core::tslot::Tempo> {
    if let Some(b) = emem_core::bands::DEFAULT.lookup(band) {
        return Some(b.tempo);
    }
    band_materializer_meta(band).map(|m| m.tempo)
}

/// Per-band metadata for materializer-only bands (the ones not in the
/// 1792-D cube layout). The tempo + history window come from the upstream
/// provider's actual coverage, not editorial guesses.
#[derive(Clone)]
struct MaterializerMeta {
    tempo: emem_core::tslot::Tempo,
    /// Shape of the data — what an agent should expect when calling
    /// recall/backfill on this band.
    kind: BandKind,
    /// Earliest Unix epoch the upstream provider can serve. `None` for
    /// bands whose provider is now-only (e.g. met.no nowcast).
    history_from_unix: Option<i64>,
    /// Latest Unix epoch the upstream can serve. `None` defaults to
    /// "now" at request time (most providers).
    history_to_unix: Option<i64>,
    /// Short upstream identifier — wire path / provider key. Surfaced in
    /// `/v1/data_availability` so a reviewer can tell at a glance which
    /// dataset a band actually fetches from.
    wire_path: &'static str,
}

/// Editorial classification of a materializable band's temporal shape.
/// The agent uses this to decide whether `emem_backfill` is meaningful
/// (`time_series` / `annual_snapshot`) or whether one recall is the only
/// answer (`static`, `now_only`, `per_release`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BandKind {
    /// Single content-addressed fact valid for all time (Cop-DEM, GMRT,
    /// JRC GSW recurrence climatology).
    Static,
    /// One fact per calendar year on Jan 1 UTC (Tessera per-year).
    AnnualSnapshot,
    /// Stack of multiple annual snapshots fused into one fact (Tessera
    /// multi-year 1024-D).
    AnnualStack,
    /// Per-tslot historical series fetched on demand from a STAC-style
    /// archive (Sentinel-2 L2A, Sentinel-1 RTC, MODIS NDVI).
    TimeSeries,
    /// Provider only exposes the current value plus a short forecast
    /// window — no historical record. Backfill on past tslots returns
    /// `present_only`.
    NowOnly,
    /// Provider ships a versioned global snapshot. Each release replaces
    /// the previous; not a per-tslot series (Overture Maps, ESA
    /// WorldCover, future global landcover products).
    PerRelease,
}

impl BandKind {
    fn as_str(self) -> &'static str {
        match self {
            BandKind::Static => "static",
            BandKind::AnnualSnapshot => "annual_snapshot",
            BandKind::AnnualStack => "annual_stack",
            BandKind::TimeSeries => "time_series",
            BandKind::NowOnly => "now_only",
            BandKind::PerRelease => "per_release",
        }
    }
}

/// Authoritative per-band history bounds. The Unix epoch values come from
/// the upstream provider's documented start of record:
///
/// - MOD13Q1 first acquisition: 2000-02-18 (`Terra` launch + commissioning).
/// - Sentinel-2 L2A operational: 2018-12-04 (Microsoft Planetary Computer
///   coverage start; mission start was 2015-06-23 but L2A reprocessing
///   begins late-2018).
/// - Sentinel-1 RTC: 2014-10-03 (S1A operational; 2014-04 was launch).
/// - Tessera v1 vintages: 2017–2024 (one fact per year on Jan 1 UTC).
/// - JRC GSW recurrence: 1984-03-01 to 2021-12-31 climatology, but the
///   product itself is a single static fact (no per-tslot backfill).
/// - Cop-DEM / GMRT / weather nowcasts: static or now-only.
///
/// `geotessera.YYYY` is parsed structurally: any year in the supported
/// range gets a single-year window `[YYYY-01-01, YYYY-12-31T23:59:59]`.
/// Adding a new vintage is a one-line edit to `TESSERA_YEARS_RANGE`.
fn band_materializer_meta(band: &str) -> Option<MaterializerMeta> {
    use emem_core::tslot::Tempo;
    // Provider-of-record start dates, computed via Hinnant civil → days
    // so the constants stay self-checking (no magic-number drift).
    // - MOD13Q1 first granule "A2000049" = day-of-year 49 of 2000 = 2000-02-18.
    // - Sentinel-2 L2A: Microsoft Planetary Computer coverage begins 2018-12-04.
    // - Sentinel-1 RTC: S1A operational + RTC reprocessing from 2014-10-03.
    let modis_start: i64 = days_from_civil(2000, 2, 18) * 86_400;
    let s2_l2a_start: i64 = days_from_civil(2018, 12, 4) * 86_400;
    let s1_start: i64 = days_from_civil(2014, 10, 3) * 86_400;

    // Single source of truth for which Tessera vintages exist. Recall +
    // materialize + catalog all read from this range.
    const TESSERA_YEARS_RANGE: std::ops::RangeInclusive<i32> = 2017..=2024;
    let tessera_window_start = jan1_unix(*TESSERA_YEARS_RANGE.start());
    let tessera_window_end = jan1_unix(*TESSERA_YEARS_RANGE.end() + 1) - 1;

    let m = match band {
        "modis.ndvi_mean" => MaterializerMeta {
            tempo: Tempo::Medium,
            kind: BandKind::TimeSeries,
            history_from_unix: Some(modis_start),
            history_to_unix: None,
            wire_path: "ORNL DAAC MOD13Q1 16-day 250 m NDVI subset",
        },
        // Every Sentinel-2 reflectance band and every derived spectral
        // index — `s2_band_plan` is the single source of truth for which
        // bands are wired here, so the registry can't drift away from
        // what `materialize_sentinel2_band` actually computes.
        b if s2_band_plan(b).is_some() => MaterializerMeta {
            tempo: Tempo::Fast,
            kind: BandKind::TimeSeries,
            history_from_unix: Some(s2_l2a_start),
            history_to_unix: None,
            wire_path: "Element84/MPC Sentinel-2 L2A STAC + COG HTTPS-Range",
        },
        "sentinel1_raw" => MaterializerMeta {
            tempo: Tempo::Fast,
            kind: BandKind::TimeSeries,
            history_from_unix: Some(s1_start),
            history_to_unix: None,
            wire_path: "Microsoft Planetary Computer Sentinel-1 RTC STAC + SAS-signed COG",
        },
        "geotessera" => MaterializerMeta {
            // The bare `geotessera` band is an alias for the latest
            // available vintage; tempo is `Slow` (annual cadence) and
            // history bounds match the latest year only.
            tempo: Tempo::Slow,
            kind: BandKind::AnnualSnapshot,
            history_from_unix: Some(jan1_unix(*TESSERA_YEARS_RANGE.end())),
            history_to_unix: Some(tessera_window_end),
            wire_path: "dl2.geotessera.org per-year .npy HTTPS-Range",
        },
        "geotessera.multi_year" => MaterializerMeta {
            tempo: Tempo::Slow,
            kind: BandKind::AnnualStack,
            history_from_unix: Some(tessera_window_start),
            history_to_unix: Some(tessera_window_end),
            wire_path: "dl2.geotessera.org × 8 vintages, fused 1024-D",
        },
        b if parse_geotessera_year(b)
            .map(|y| TESSERA_YEARS_RANGE.contains(&y))
            .unwrap_or(false) =>
        {
            // SAFETY: just verified it parses + is in range above.
            let y = parse_geotessera_year(b).unwrap();
            MaterializerMeta {
                tempo: Tempo::Slow,
                kind: BandKind::AnnualSnapshot,
                history_from_unix: Some(jan1_unix(y)),
                history_to_unix: Some(jan1_unix(y + 1) - 1),
                wire_path: "dl2.geotessera.org per-year .npy HTTPS-Range",
            }
        }
        // Static climatologies / single-snapshot products — no per-tslot
        // history; one fact answers for all time.
        "copdem30m.elevation_mean" => MaterializerMeta {
            tempo: Tempo::Static,
            kind: BandKind::Static,
            history_from_unix: None,
            history_to_unix: None,
            wire_path: "Open-Meteo Elevation REST (Cop-DEM 90 m derived)",
        },
        "gmrt.topobathy_mean" => MaterializerMeta {
            tempo: Tempo::Static,
            kind: BandKind::Static,
            history_from_unix: None,
            history_to_unix: None,
            wire_path: "GMRT PointServer (multibeam-fused topobathy)",
        },
        "surface_water.recurrence" => MaterializerMeta {
            tempo: Tempo::Static,
            kind: BandKind::Static,
            // JRC GSW v1.4 climatology: 1984-03-16 to 2021-12-31. The
            // product itself is a single signed value per cell (no
            // per-tslot series), but the underlying observation window is
            // surfaced so an agent can cite "decades of Landsat record".
            history_from_unix: Some(days_from_civil(1984, 3, 16) * 86_400),
            history_to_unix: Some(days_from_civil(2022, 1, 1) * 86_400 - 1),
            wire_path: "JRC Global Surface Water v1.4 (Landsat 1984-2021)",
        },
        // Met.no's locationforecast is a nowcast + 9-day forecast — no
        // historical record. Backfill is honest about this.
        b if b.starts_with("weather.") => MaterializerMeta {
            tempo: Tempo::UltraFast,
            kind: BandKind::NowOnly,
            history_from_unix: None,
            history_to_unix: None,
            wire_path: "api.met.no locationforecast/2.0/compact (now + 9-day forecast)",
        },
        // Overture Maps releases are versioned snapshots, not per-tslot
        // historical series — treat as slow + present-only.
        b if b.starts_with("overture.") => MaterializerMeta {
            tempo: Tempo::Slow,
            kind: BandKind::PerRelease,
            history_from_unix: None,
            history_to_unix: None,
            wire_path: "Overture Maps S3 (anonymous), latest release only",
        },
        // NASA POWER: MERRA-2 + GEOS daily reanalysis. Public-domain (US
        // Gov), no auth, ~2-day publication latency. Record begins 1981.
        b if b.starts_with("power.") => MaterializerMeta {
            tempo: Tempo::Fast,
            kind: BandKind::TimeSeries,
            history_from_unix: Some(days_from_civil(1981, 1, 1) * 86_400),
            history_to_unix: None,
            wire_path: "NASA POWER daily/point REST (MERRA-2 + GEOS)",
        },
        // Open-Meteo CAMS: surface-level air pollutants from CAMS reanalysis.
        // Hourly cadence, archive runs from 2013-08-01.
        b if b.starts_with("cams.") => MaterializerMeta {
            tempo: Tempo::UltraFast,
            kind: BandKind::TimeSeries,
            history_from_unix: Some(days_from_civil(2013, 8, 1) * 86_400),
            history_to_unix: None,
            wire_path: "Open-Meteo CAMS air-quality REST (ECMWF CAMS reanalysis)",
        },
        // Open-Meteo Archive: ECMWF ERA5 retrospective 1940-present, hourly.
        b if b.starts_with("era5.") => MaterializerMeta {
            tempo: Tempo::UltraFast,
            kind: BandKind::TimeSeries,
            history_from_unix: Some(days_from_civil(1940, 1, 1) * 86_400),
            history_to_unix: None,
            wire_path: "Open-Meteo Archive REST (ECMWF ERA5)",
        },
        // Open-Meteo Marine: ECMWF WAM wave model. Coastal/oceanic only,
        // hourly, 2022-08-01 onward.
        b if b.starts_with("marine.") => MaterializerMeta {
            tempo: Tempo::UltraFast,
            kind: BandKind::TimeSeries,
            history_from_unix: Some(days_from_civil(2022, 8, 1) * 86_400),
            history_to_unix: None,
            wire_path: "Open-Meteo Marine REST (ECMWF WAM wave model)",
        },
        // ORNL DAAC additional MODIS subset products. Per-product start of
        // record below; all 8-day or 16-day or 30-day composites depending
        // on the upstream product's natural cadence.
        "modis.lst_day_8day" | "modis.lst_night_8day" => MaterializerMeta {
            tempo: Tempo::Medium,
            kind: BandKind::TimeSeries,
            // MOD11A2 first granule: 2000-03-05.
            history_from_unix: Some(days_from_civil(2000, 3, 5) * 86_400),
            history_to_unix: None,
            wire_path: "ORNL DAAC MOD11A2 8-day 1 km LST subset",
        },
        "modis.et_8day" => MaterializerMeta {
            tempo: Tempo::Medium,
            kind: BandKind::TimeSeries,
            // MOD16A2 first valid granule: 2001-01-01.
            history_from_unix: Some(days_from_civil(2001, 1, 1) * 86_400),
            history_to_unix: None,
            wire_path: "ORNL DAAC MOD16A2 8-day 500 m ET subset",
        },
        "modis.gpp_8day" => MaterializerMeta {
            tempo: Tempo::Medium,
            kind: BandKind::TimeSeries,
            // MOD17A2H first granule: 2000-02-18.
            history_from_unix: Some(days_from_civil(2000, 2, 18) * 86_400),
            history_to_unix: None,
            wire_path: "ORNL DAAC MOD17A2H 8-day 500 m GPP subset",
        },
        "modis.lai_8day" => MaterializerMeta {
            tempo: Tempo::Medium,
            kind: BandKind::TimeSeries,
            // MOD15A2H first granule: 2002-07-04.
            history_from_unix: Some(days_from_civil(2002, 7, 4) * 86_400),
            history_to_unix: None,
            wire_path: "ORNL DAAC MOD15A2H 8-day 500 m LAI subset",
        },
        "modis.burned_area_monthly" => MaterializerMeta {
            tempo: Tempo::Medium,
            kind: BandKind::TimeSeries,
            // MCD64A1 first granule: 2000-11-01.
            history_from_unix: Some(days_from_civil(2000, 11, 1) * 86_400),
            history_to_unix: None,
            wire_path: "ORNL DAAC MCD64A1 monthly 500 m burned-area subset",
        },
        // ESA WorldCover 2021 v200 — 11-class global landcover at 10 m, one
        // signed fact per cell (single 2021 release).
        "esa_worldcover.lc_2021" => MaterializerMeta {
            tempo: Tempo::Slow,
            kind: BandKind::PerRelease,
            history_from_unix: Some(days_from_civil(2021, 1, 1) * 86_400),
            history_to_unix: Some(days_from_civil(2022, 1, 1) * 86_400 - 1),
            wire_path: "esa-worldcover s3 (anonymous): v200 2021 10 m LCCS map",
        },
        // Hansen Global Forest Change v1.11 (2023 release). Three layers:
        // tree_cover_2000 (static climatology of 2000), loss_year (cumulative
        // 2001..=2023), gain (single 2000–2012 mask). All three are signed
        // as static-per-release facts (the 2023 release is the canonical
        // serving snapshot until the next annual update).
        "hansen.tree_cover_2000" | "hansen.loss_year" | "hansen.gain" => MaterializerMeta {
            tempo: Tempo::Slow,
            kind: BandKind::PerRelease,
            history_from_unix: Some(days_from_civil(2000, 1, 1) * 86_400),
            history_to_unix: Some(days_from_civil(2024, 1, 1) * 86_400 - 1),
            wire_path:
                "storage.googleapis.com/earthenginepartners-hansen GFC-2023-v1.11 30 m tiles",
        },
        _ => return None,
    };
    Some(m)
}

/// Parse `geotessera.YYYY` → `Some(YYYY)`, anything else → `None`. Pulled
/// out of `band_materializer_meta` so the materializer dispatch can reuse
/// it without duplicating the suffix-strip logic.
fn parse_geotessera_year(band: &str) -> Option<i32> {
    let suffix = band.strip_prefix("geotessera.")?;
    suffix.parse::<i32>().ok()
}

/// Authoritative range of Tessera v1 vintages this responder ships. Mirror
/// of the constant in `band_materializer_meta` and `materialize_band_at`;
/// kept here so `/v1/data_availability` can enumerate per-year entries
/// programmatically rather than hardcoding a list.
const TESSERA_YEARS_RANGE_PUBLIC: std::ops::RangeInclusive<i32> = 2017..=2024;

/// Concrete enumeration of every band this responder will materialize on
/// demand — used to drive `/v1/data_availability`. Includes the bare
/// `geotessera` alias, the multi-year stack, and one entry per supported
/// vintage. The order is the order the catalog will be reported in.
fn all_materializable_bands() -> Vec<String> {
    let mut out: Vec<String> = vec![
        // Static climatologies.
        "copdem30m.elevation_mean".into(),
        "gmrt.topobathy_mean".into(),
        "surface_water.recurrence".into(),
        // Per-tslot historical archives.
        "modis.ndvi_mean".into(),
        // Sentinel-2 reflectance bands.
        "s2.B01".into(),
        "s2.B02".into(),
        "s2.B03".into(),
        "s2.B04".into(),
        "s2.B05".into(),
        "s2.B06".into(),
        "s2.B07".into(),
        "s2.B08".into(),
        "s2.B8A".into(),
        "s2.B09".into(),
        "s2.B11".into(),
        "s2.B12".into(),
        "s2.scl".into(),
        // Sentinel-2 derived spectral indices.
        "indices.ndvi".into(),
        "indices.ndwi".into(),
        "indices.mndwi".into(),
        "indices.evi".into(),
        "indices.nbr".into(),
        "indices.ndmi".into(),
        "indices.savi".into(),
        "indices.bsi".into(),
        "indices.ndbi".into(),
        // Sentinel-2 derived health indices.
        "indices.ndti".into(),
        "indices.gndvi".into(),
        "indices.ndre".into(),
        "indices.fai".into(),
        "indices.tss".into(),
        "indices.ndsi".into(),
        "indices.afri1600".into(),
        "indices.savi_l1".into(),
        "indices.surface_dryness".into(),
        "indices.urban_canopy_index".into(),
        // Sentinel-1 RTC.
        "sentinel1_raw".into(),
        // Tessera vintages.
        "geotessera".into(),
        "geotessera.multi_year".into(),
    ];
    for y in TESSERA_YEARS_RANGE_PUBLIC.clone() {
        out.push(format!("geotessera.{y}"));
    }
    // Overture Maps.
    out.push("overture.buildings.count".into());
    out.push("overture.places.count".into());
    out.push("overture.transportation.road_length_m".into());
    // Met.no nowcast bands.
    out.push("weather.temperature_2m".into());
    out.push("weather.cloud_cover".into());
    out.push("weather.precipitation_mm".into());
    out.push("weather.wind_speed_10m".into());
    out.push("weather.relative_humidity_2m".into());
    out.push("weather.dew_point_2m".into());
    out.push("weather.air_pressure_msl".into());
    out.push("weather.wind_direction_10m".into());
    // NASA POWER reanalysis (daily, 1981-present).
    out.push("power.t2m".into());
    out.push("power.t2m_min".into());
    out.push("power.t2m_max".into());
    out.push("power.precip".into());
    out.push("power.rh2m".into());
    out.push("power.allsky_sw".into());
    out.push("power.ws10m".into());
    // Open-Meteo CAMS air-quality (hourly, 2013-08-01 onward).
    out.push("cams.pm25".into());
    out.push("cams.pm10".into());
    out.push("cams.no2".into());
    out.push("cams.o3".into());
    out.push("cams.so2".into());
    out.push("cams.co".into());
    out.push("cams.aod_550".into());
    // Open-Meteo Archive ERA5 (hourly, 1940-present).
    out.push("era5.t2m".into());
    out.push("era5.precip".into());
    out.push("era5.rh2m".into());
    out.push("era5.windspeed_10m".into());
    out.push("era5.cloudcover".into());
    out.push("era5.surface_pressure".into());
    out.push("era5.dewpoint_2m".into());
    // Open-Meteo Marine (hourly, 2022-08-01 onward, ocean only).
    out.push("marine.wave_height".into());
    out.push("marine.swell_period".into());
    out.push("marine.swell_height".into());
    out.push("marine.sst".into());
    out.push("marine.wave_direction".into());
    // ORNL DAAC additional MODIS subsets.
    out.push("modis.lst_day_8day".into());
    out.push("modis.lst_night_8day".into());
    out.push("modis.et_8day".into());
    out.push("modis.gpp_8day".into());
    out.push("modis.lai_8day".into());
    out.push("modis.burned_area_monthly".into());
    // ESA WorldCover 2021 (single release).
    out.push("esa_worldcover.lc_2021".into());
    // Hansen Global Forest Change v1.11 (2023 release).
    out.push("hansen.tree_cover_2000".into());
    out.push("hansen.loss_year".into());
    out.push("hansen.gain".into());
    out
}

/// Tslot Unix range covered by the calendar year `y` (inclusive lo, exclusive hi).
fn year_unix_range(y: i32) -> (i64, i64) {
    (jan1_unix(y), jan1_unix(y + 1))
}

/// Map a target Unix epoch to the Tessera vintage that "owns" it: the year
/// `y` such that `jan1_unix(y) <= t < jan1_unix(y+1)`. Returns `None` when
/// the target falls outside the supported Tessera year range.
fn tessera_year_for_unix(t: i64, range: std::ops::RangeInclusive<i32>) -> Option<i32> {
    for y in range {
        let (lo, hi) = year_unix_range(y);
        if t >= lo && t < hi {
            return Some(y);
        }
    }
    None
}

/// Per-band materialization for a target Unix epoch. Returns Ok(cid) on
/// success, Err(reason) when the band has no historical materializer at
/// this responder. The reason string is wired to the per-step `reason`
/// field of `BackfillResp` so an agent can distinguish "upstream down"
/// from "this band is now-only at this responder".
async fn materialize_band_at(
    cell64: &str,
    band: &str,
    target_unix: i64,
    s: &AppState,
) -> Result<emem_fact::FactCid, String> {
    // Static products: tslot is meaningless, the canonical fact lives at
    // tslot=0 regardless of target.
    match band {
        "modis.ndvi_mean" => {
            return materialize_modis_ndvi_window(cell64, Some(target_unix), s).await
        }
        "copdem30m.elevation_mean" => {
            return match materialize_elevation_mean(cell64, s).await {
                Ok(ElevationMaterialization::Primary(c))
                | Ok(ElevationMaterialization::Absence(c)) => Ok(c),
                Err(e) => Err(e),
            };
        }
        "gmrt.topobathy_mean" => return materialize_gmrt_topobathy(cell64, s).await,
        "surface_water.recurrence" => return materialize_jrc_gsw_recurrence(cell64, s).await,
        "geotessera" => {
            // Bare `geotessera` resolves to the Tessera vintage that
            // contains target_unix; agents asking for backfill across
            // years should use `geotessera.YYYY` directly.
            const TESSERA_YEARS_RANGE: std::ops::RangeInclusive<i32> = 2017..=2024;
            let y = tessera_year_for_unix(target_unix, TESSERA_YEARS_RANGE.clone()).ok_or_else(
                || {
                    format!(
                        "no Tessera vintage covers target_unix={target_unix}; supported range is {}..={} (Jan 1 UTC)",
                        TESSERA_YEARS_RANGE.start(),
                        TESSERA_YEARS_RANGE.end()
                    )
                },
            )?;
            return materialize_geotessera_for_year(cell64, s, y, "geotessera").await;
        }
        "geotessera.multi_year" => return materialize_geotessera_multi_year(cell64, s).await,
        "sentinel1_raw" => return materialize_sentinel1_vv(cell64, s, Some(target_unix)).await,
        // Overture is a versioned global snapshot, not a per-tslot series;
        // the canonical fact is "latest release" — backfill on a past
        // target_unix surfaces the same fact (signed at request time).
        "overture.buildings.count" => return materialize_overture_buildings_count(cell64, s).await,
        "overture.places.count" => return materialize_overture_places_count(cell64, s).await,
        "overture.transportation.road_length_m" => {
            return materialize_overture_road_length_m(cell64, s).await
        }
        _ => {}
    }

    // Sentinel-2 reflectance bands and derived spectral indices.
    if s2_band_plan(band).is_some() {
        return materialize_sentinel2_band(cell64, s, band, Some(target_unix)).await;
    }

    // Per-year Tessera vintages: `geotessera.YYYY` → fixed year, ignoring
    // target_unix (the year IS the target).
    if let Some(y) = parse_geotessera_year(band) {
        const TESSERA_YEARS_RANGE: std::ops::RangeInclusive<i32> = 2017..=2024;
        if !TESSERA_YEARS_RANGE.contains(&y) {
            return Err(format!(
                "Tessera vintage {y} not in supported range {}..={}",
                TESSERA_YEARS_RANGE.start(),
                TESSERA_YEARS_RANGE.end()
            ));
        }
        return materialize_geotessera_for_year(cell64, s, y, band).await;
    }

    // ORNL DAAC additional MODIS subset products.
    if matches!(
        band,
        "modis.lst_day_8day"
            | "modis.lst_night_8day"
            | "modis.et_8day"
            | "modis.gpp_8day"
            | "modis.lai_8day"
            | "modis.burned_area_monthly"
    ) {
        return materialize_ornl_modis_band(cell64, s, band, Some(target_unix)).await;
    }

    // NASA POWER reanalysis (1981-present, daily).
    if band.starts_with("power.") {
        return materialize_power_band(cell64, s, band, Some(target_unix)).await;
    }

    // Open-Meteo CAMS air-quality (2013-08-01+, hourly).
    if band.starts_with("cams.") {
        return materialize_cams_band(cell64, s, band, Some(target_unix)).await;
    }

    // Open-Meteo Archive ERA5 (1940-present, hourly).
    if band.starts_with("era5.") {
        return materialize_era5_band(cell64, s, band, target_unix).await;
    }

    // Open-Meteo Marine ECMWF WAM (2022-08-01+, hourly).
    if band.starts_with("marine.") {
        return materialize_marine_band(cell64, s, band, Some(target_unix)).await;
    }

    // ESA WorldCover 2021 — per-release static fact, ignores target_unix.
    if band == "esa_worldcover.lc_2021" {
        return materialize_esa_worldcover_2021(cell64, s).await;
    }

    // Hansen GFC v1.11 layers — single release, static per cell.
    if matches!(
        band,
        "hansen.tree_cover_2000" | "hansen.loss_year" | "hansen.gain"
    ) {
        return materialize_hansen_band(cell64, s, band).await;
    }

    // Met.no nowcast — no historical record. Surface this honestly so the
    // backfill response can distinguish "upstream down" from "this band is
    // now-only at this responder".
    if band.starts_with("weather.") {
        return Err(format!(
            "present_only: '{band}' is a met.no nowcast — backfill only meaningful for the current tslot; recall without a tslot to fetch the latest value"
        ));
    }

    Err(format!(
        "no historical materializer registered for band '{band}'; call /v1/data_availability for the catalog of materializable bands"
    ))
}

/// Result of a `POST /v1/backfill` call. Symmetrical with the MCP
/// `emem_backfill` response shape.
async fn backfill_inner(req: BackfillReq, s: &AppState) -> Result<JsonValue, ApiError> {
    use emem_core::tslot::{Tempo, Tslot};
    let tempo = tempo_for_band(&req.band).ok_or_else(|| {
        ApiError(
            StatusCode::NOT_FOUND,
            ErrorBody {
                code: ErrorCode::BandNotInRegistry,
                message: format!(
                    "unknown band '{}': call /v1/bands or emem_bands first",
                    req.band
                ),
            },
        )
    })?;
    let meta = band_materializer_meta(&req.band);
    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    // Bucket anchor for "no start_unix given": the band's documented
    // upstream-of-record Unix second, falling back to the Unix epoch.
    let default_start = meta.as_ref().and_then(|m| m.history_from_unix).unwrap_or(0);
    let default_end = meta
        .as_ref()
        .and_then(|m| m.history_to_unix)
        .unwrap_or(now_unix);
    let mut start = req.start_unix.unwrap_or(default_start);
    let mut end = req.end_unix.unwrap_or(default_end);
    if start > end {
        std::mem::swap(&mut start, &mut end);
    }
    let max_facts = req.max_facts.unwrap_or(64).clamp(1, 1024);

    let slot_secs = tempo.slot_seconds();
    let mut steps: Vec<JsonValue> = Vec::new();
    let mut materialized = 0usize;
    let mut cached = 0usize;
    let mut skipped = 0usize;
    let mut notes: Vec<String> = Vec::new();

    // Fast path for static bands: one tslot 0, one fact total.
    if matches!(tempo, Tempo::Static) {
        // Already have it?
        let existing = s.storage.scan_cell(&req.cell, None).await.map_err(|e| {
            ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorBody {
                    code: ErrorCode::Internal,
                    message: e.to_string(),
                },
            )
        })?;
        let already = existing
            .into_iter()
            .find(|(k, _)| k.band == req.band && k.tslot == 0)
            .map(|(_, c)| c);
        match already {
            Some(cid) => {
                cached += 1;
                steps.push(json!({
                    "tslot": 0u64,
                    "target_unix": 0i64,
                    "status": "cached",
                    "fact_cid": cid.as_str(),
                }));
            }
            None => match materialize_band_at(&req.cell, &req.band, now_unix, s).await {
                Ok(cid) => {
                    materialized += 1;
                    steps.push(json!({
                        "tslot": 0u64,
                        "target_unix": now_unix,
                        "status": "materialized",
                        "fact_cid": cid.as_str(),
                    }));
                }
                Err(e) => {
                    skipped += 1;
                    let status = if e.starts_with("present_only:") {
                        "present_only"
                    } else {
                        "error"
                    };
                    steps.push(json!({
                        "tslot": 0u64,
                        "target_unix": now_unix,
                        "status": status,
                        "reason": e,
                    }));
                }
            },
        }
    } else {
        let start_t = Tslot::from_unix(start, tempo).0;
        let end_t = Tslot::from_unix(end, tempo).0;
        // Pre-load existing facts for this (cell, band) once, so we don't
        // do N sled scans for an N-step backfill.
        let existing = s.storage.scan_cell(&req.cell, None).await.map_err(|e| {
            ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorBody {
                    code: ErrorCode::Internal,
                    message: e.to_string(),
                },
            )
        })?;
        let mut have: std::collections::HashMap<u64, emem_fact::FactCid> = existing
            .into_iter()
            .filter(|(k, _)| k.band == req.band)
            .map(|(k, c)| (k.tslot, c))
            .collect();

        for t in start_t..=end_t {
            if steps.len() >= max_facts {
                notes.push(format!(
                    "max_facts={max_facts} reached at tslot {t}; partial backfill — call again with start_unix > {} to continue",
                    (t as i64) * (slot_secs as i64),
                ));
                break;
            }
            let target_unix = (t as i64) * (slot_secs as i64);
            if target_unix > now_unix {
                steps.push(json!({
                    "tslot": t,
                    "target_unix": target_unix,
                    "status": "future",
                    "reason": "tslot is in the future relative to wall-clock now",
                }));
                skipped += 1;
                continue;
            }
            if let Some(cid) = have.remove(&t) {
                cached += 1;
                steps.push(json!({
                    "tslot": t,
                    "target_unix": target_unix,
                    "status": "cached",
                    "fact_cid": cid.as_str(),
                }));
                continue;
            }
            match materialize_band_at(&req.cell, &req.band, target_unix, s).await {
                Ok(cid) => {
                    materialized += 1;
                    steps.push(json!({
                        "tslot": t,
                        "target_unix": target_unix,
                        "status": "materialized",
                        "fact_cid": cid.as_str(),
                    }));
                }
                Err(e) => {
                    skipped += 1;
                    let status = if e.starts_with("present_only:") {
                        "present_only"
                    } else {
                        "error"
                    };
                    steps.push(json!({
                        "tslot": t,
                        "target_unix": target_unix,
                        "status": status,
                        "reason": e,
                    }));
                    // If the reason is "no historical materializer", every
                    // remaining tslot will fail the same way — short-circuit
                    // and tell the agent honestly.
                    if status == "present_only" {
                        notes.push(format!(
                            "band '{}' is now-only at this responder; further tslots in this window will return the same status — stopping after first probe",
                            req.band));
                        break;
                    }
                }
            }
        }
    }

    let pubkey = data_encoding::BASE32_NOPAD
        .encode(&s.identity.pubkey.0)
        .to_lowercase();
    Ok(json!({
        "schema": "emem.backfill.v1",
        "cell": req.cell,
        "band": req.band,
        "tempo": format!("{:?}", tempo).to_ascii_lowercase(),
        "slot_seconds": slot_secs,
        "window_start_unix": start,
        "window_end_unix": end,
        "history_available_from_unix": meta.as_ref().and_then(|m| m.history_from_unix),
        "history_available_to_unix": meta.as_ref().and_then(|m| m.history_to_unix),
        "total_steps": steps.len(),
        "materialized_count": materialized,
        "cached_count": cached,
        "skipped_count": skipped,
        "responder_pubkey_b32": pubkey,
        "steps": steps,
        "notes": notes,
        "next": [
            "POST /v1/trajectory with the same cell+band+window to read back the now-attested series.",
            "POST /v1/diff between any two materialized tslots for a signed delta.",
            "Each step.fact_cid is independently citable via emem_fetch / GET /v1/facts/{cid}.",
        ],
        "agent_hint": "emem_backfill is the bridge between 'I want history' and 'history exists in the ledger'. Each materialized fact is signed by the responder above; replay across responders by content-addressing the same upstream URLs."
    }))
}

/// Try to materialize each requested band on the given cell. Returns
/// per-band outcomes so the recall handler can surface why a band was
/// skipped (ocean cell, upstream down, no materializer registered).
async fn try_materialize_bands(
    cell64: &str,
    bands: &[String],
    s: &AppState,
) -> Vec<MaterializeOutcome> {
    let mut out = Vec::with_capacity(bands.len());
    if !auto_materialize_enabled() {
        return out;
    }
    for b in bands {
        match b.as_str() {
            "modis.ndvi_mean" => match materialize_modis_ndvi(cell64, s).await {
                Ok(cid) => {
                    tracing::info!(
                        target: "emem::materialize",
                        materialize_cell = %cell64, materialize_band = %b,
                        materialize_fact_cid = %cid.as_str(),
                        materialize_kind = "primary",
                        "materialize_ok"
                    );
                    out.push(MaterializeOutcome {
                        band: b.clone(),
                        fact_cid: Some(cid.as_str().to_string()),
                        skip_reason: None,
                    });
                }
                Err(e) => {
                    tracing::warn!(
                        target: "emem::materialize",
                        materialize_cell = %cell64, materialize_band = %b,
                        materialize_error = %e,
                        "materialize_failed"
                    );
                    out.push(MaterializeOutcome {
                        band: b.clone(),
                        fact_cid: None,
                        skip_reason: Some(e),
                    });
                }
            },
            "gmrt.topobathy_mean" => match materialize_gmrt_topobathy(cell64, s).await {
                Ok(cid) => {
                    tracing::info!(
                        target: "emem::materialize",
                        materialize_cell = %cell64, materialize_band = %b,
                        materialize_fact_cid = %cid.as_str(),
                        materialize_kind = "primary",
                        "materialize_ok"
                    );
                    out.push(MaterializeOutcome {
                        band: b.clone(),
                        fact_cid: Some(cid.as_str().to_string()),
                        skip_reason: None,
                    });
                }
                Err(e) => {
                    tracing::warn!(
                        target: "emem::materialize",
                        materialize_cell = %cell64, materialize_band = %b,
                        materialize_error = %e,
                        "materialize_failed"
                    );
                    out.push(MaterializeOutcome {
                        band: b.clone(),
                        fact_cid: None,
                        skip_reason: Some(e),
                    });
                }
            },
            "copdem30m.elevation_mean" => match materialize_elevation_mean(cell64, s).await {
                Ok(ElevationMaterialization::Primary(cid)) => {
                    tracing::info!(
                        target: "emem::materialize",
                        materialize_cell = %cell64, materialize_band = %b,
                        materialize_fact_cid = %cid.as_str(),
                        materialize_kind = "primary",
                        "materialize_ok"
                    );
                    out.push(MaterializeOutcome {
                        band: b.clone(),
                        fact_cid: Some(cid.as_str().to_string()),
                        skip_reason: None,
                    });
                }
                Ok(ElevationMaterialization::Absence(cid)) => {
                    tracing::info!(
                        target: "emem::materialize",
                        materialize_cell = %cell64, materialize_band = %b,
                        materialize_fact_cid = %cid.as_str(),
                        materialize_kind = "absence",
                        "materialize_ok"
                    );
                    out.push(MaterializeOutcome {
                        band: b.clone(),
                        fact_cid: Some(cid.as_str().to_string()),
                        skip_reason: None,
                    });
                }
                Err(e) => {
                    tracing::warn!(
                        target: "emem::materialize",
                        materialize_cell = %cell64, materialize_band = %b,
                        materialize_error = %e,
                        "materialize_failed"
                    );
                    out.push(MaterializeOutcome {
                        band: b.clone(),
                        fact_cid: None,
                        skip_reason: Some(e),
                    });
                }
            },
            "geotessera"
            | "geotessera.multi_year"
            | "geotessera.2017"
            | "geotessera.2018"
            | "geotessera.2019"
            | "geotessera.2020"
            | "geotessera.2021"
            | "geotessera.2022"
            | "geotessera.2023"
            | "geotessera.2024" => {
                let result = if b == "geotessera" {
                    materialize_geotessera_embedding(cell64, s).await
                } else if b == "geotessera.multi_year" {
                    materialize_geotessera_multi_year(cell64, s).await
                } else {
                    materialize_geotessera_year_band(cell64, s, b).await
                };
                match result {
                    Ok(cid) => {
                        tracing::info!(target: "emem::materialize",
                            materialize_cell = %cell64, materialize_band = %b,
                            materialize_fact_cid = %cid.as_str(),
                            materialize_kind = "primary", "materialize_ok");
                        out.push(MaterializeOutcome {
                            band: b.clone(),
                            fact_cid: Some(cid.as_str().to_string()),
                            skip_reason: None,
                        });
                    }
                    Err(e) => {
                        tracing::warn!(target: "emem::materialize",
                            materialize_cell = %cell64, materialize_band = %b,
                            materialize_error = %e, "materialize_failed");
                        out.push(MaterializeOutcome {
                            band: b.clone(),
                            fact_cid: None,
                            skip_reason: Some(e),
                        });
                    }
                }
            }
            "weather.temperature_2m"
            | "weather.cloud_cover"
            | "weather.precipitation_mm"
            | "weather.wind_speed_10m"
            | "weather.relative_humidity_2m"
            | "weather.dew_point_2m"
            | "weather.air_pressure_msl"
            | "weather.wind_direction_10m" => {
                match materialize_weather_current(cell64, s, b).await {
                    Ok(cid) => {
                        tracing::info!(
                            target: "emem::materialize",
                            materialize_cell = %cell64, materialize_band = %b,
                            materialize_fact_cid = %cid.as_str(),
                            materialize_kind = "primary",
                            "materialize_ok"
                        );
                        out.push(MaterializeOutcome {
                            band: b.clone(),
                            fact_cid: Some(cid.as_str().to_string()),
                            skip_reason: None,
                        });
                    }
                    Err(e) => {
                        tracing::warn!(
                            target: "emem::materialize",
                            materialize_cell = %cell64, materialize_band = %b,
                            materialize_error = %e,
                            "materialize_failed"
                        );
                        out.push(MaterializeOutcome {
                            band: b.clone(),
                            fact_cid: None,
                            skip_reason: Some(e),
                        });
                    }
                }
            }
            b_name if s2_band_plan(b_name).is_some() => {
                match materialize_sentinel2_band(cell64, s, b, None).await {
                    Ok(cid) => {
                        tracing::info!(
                            target: "emem::materialize",
                            materialize_cell = %cell64, materialize_band = %b,
                            materialize_fact_cid = %cid.as_str(),
                            materialize_kind = "primary",
                            "materialize_ok"
                        );
                        out.push(MaterializeOutcome {
                            band: b.clone(),
                            fact_cid: Some(cid.as_str().to_string()),
                            skip_reason: None,
                        });
                    }
                    Err(e) => {
                        tracing::warn!(
                            target: "emem::materialize",
                            materialize_cell = %cell64, materialize_band = %b,
                            materialize_error = %e,
                            "materialize_failed"
                        );
                        out.push(MaterializeOutcome {
                            band: b.clone(),
                            fact_cid: None,
                            skip_reason: Some(e),
                        });
                    }
                }
            }
            "overture.buildings.count" => {
                match materialize_overture_buildings_count(cell64, s).await {
                    Ok(cid) => {
                        tracing::info!(
                            target: "emem::materialize",
                            materialize_cell = %cell64, materialize_band = %b,
                            materialize_fact_cid = %cid.as_str(),
                            materialize_kind = "primary",
                            "materialize_ok"
                        );
                        out.push(MaterializeOutcome {
                            band: b.clone(),
                            fact_cid: Some(cid.as_str().to_string()),
                            skip_reason: None,
                        });
                    }
                    Err(e) => {
                        tracing::warn!(
                            target: "emem::materialize",
                            materialize_cell = %cell64, materialize_band = %b,
                            materialize_error = %e,
                            "materialize_failed"
                        );
                        out.push(MaterializeOutcome {
                            band: b.clone(),
                            fact_cid: None,
                            skip_reason: Some(e),
                        });
                    }
                }
            }
            "overture.places.count" => match materialize_overture_places_count(cell64, s).await {
                Ok(cid) => {
                    tracing::info!(
                        target: "emem::materialize",
                        materialize_cell = %cell64, materialize_band = %b,
                        materialize_fact_cid = %cid.as_str(),
                        materialize_kind = "primary",
                        "materialize_ok"
                    );
                    out.push(MaterializeOutcome {
                        band: b.clone(),
                        fact_cid: Some(cid.as_str().to_string()),
                        skip_reason: None,
                    });
                }
                Err(e) => {
                    tracing::warn!(
                        target: "emem::materialize",
                        materialize_cell = %cell64, materialize_band = %b,
                        materialize_error = %e,
                        "materialize_failed"
                    );
                    out.push(MaterializeOutcome {
                        band: b.clone(),
                        fact_cid: None,
                        skip_reason: Some(e),
                    });
                }
            },
            "overture.transportation.road_length_m" => {
                match materialize_overture_road_length_m(cell64, s).await {
                    Ok(cid) => {
                        tracing::info!(
                            target: "emem::materialize",
                            materialize_cell = %cell64, materialize_band = %b,
                            materialize_fact_cid = %cid.as_str(),
                            materialize_kind = "primary",
                            "materialize_ok"
                        );
                        out.push(MaterializeOutcome {
                            band: b.clone(),
                            fact_cid: Some(cid.as_str().to_string()),
                            skip_reason: None,
                        });
                    }
                    Err(e) => {
                        tracing::warn!(
                            target: "emem::materialize",
                            materialize_cell = %cell64, materialize_band = %b,
                            materialize_error = %e,
                            "materialize_failed"
                        );
                        out.push(MaterializeOutcome {
                            band: b.clone(),
                            fact_cid: None,
                            skip_reason: Some(e),
                        });
                    }
                }
            }
            "sentinel1_raw" => match materialize_sentinel1_vv(cell64, s, None).await {
                Ok(cid) => {
                    tracing::info!(
                        target: "emem::materialize",
                        materialize_cell = %cell64, materialize_band = %b,
                        materialize_fact_cid = %cid.as_str(),
                        materialize_kind = "primary",
                        "materialize_ok"
                    );
                    out.push(MaterializeOutcome {
                        band: b.clone(),
                        fact_cid: Some(cid.as_str().to_string()),
                        skip_reason: None,
                    });
                }
                Err(e) => {
                    tracing::warn!(
                        target: "emem::materialize",
                        materialize_cell = %cell64, materialize_band = %b,
                        materialize_error = %e,
                        "materialize_failed"
                    );
                    out.push(MaterializeOutcome {
                        band: b.clone(),
                        fact_cid: None,
                        skip_reason: Some(e),
                    });
                }
            },
            "surface_water.recurrence" => match materialize_jrc_gsw_recurrence(cell64, s).await {
                Ok(cid) => {
                    tracing::info!(
                        target: "emem::materialize",
                        materialize_cell = %cell64, materialize_band = %b,
                        materialize_fact_cid = %cid.as_str(),
                        materialize_kind = "primary_or_absence",
                        "materialize_ok"
                    );
                    out.push(MaterializeOutcome {
                        band: b.clone(),
                        fact_cid: Some(cid.as_str().to_string()),
                        skip_reason: None,
                    });
                }
                Err(e) => {
                    tracing::warn!(
                        target: "emem::materialize",
                        materialize_cell = %cell64, materialize_band = %b,
                        materialize_error = %e,
                        "materialize_failed"
                    );
                    out.push(MaterializeOutcome {
                        band: b.clone(),
                        fact_cid: None,
                        skip_reason: Some(e),
                    });
                }
            },
            // ORNL DAAC additional MODIS subset products (LST/ET/GPP/LAI/burn).
            // Recall path: target_unix=None → "latest valid composite within
            // last 4·half_window" — same heuristic as MOD13Q1 NDVI.
            "modis.lst_day_8day"
            | "modis.lst_night_8day"
            | "modis.et_8day"
            | "modis.gpp_8day"
            | "modis.lai_8day"
            | "modis.burned_area_monthly" => {
                match materialize_ornl_modis_band(cell64, s, b, None).await {
                    Ok(cid) => {
                        tracing::info!(
                            target: "emem::materialize",
                            materialize_cell = %cell64, materialize_band = %b,
                            materialize_fact_cid = %cid.as_str(),
                            materialize_kind = "primary",
                            "materialize_ok"
                        );
                        out.push(MaterializeOutcome {
                            band: b.clone(),
                            fact_cid: Some(cid.as_str().to_string()),
                            skip_reason: None,
                        });
                    }
                    Err(e) => {
                        tracing::warn!(
                            target: "emem::materialize",
                            materialize_cell = %cell64, materialize_band = %b,
                            materialize_error = %e,
                            "materialize_failed"
                        );
                        out.push(MaterializeOutcome {
                            band: b.clone(),
                            fact_cid: None,
                            skip_reason: Some(e),
                        });
                    }
                }
            }
            // NASA POWER reanalysis. Recall path: latest available daily
            // value (target_unix=None → 2-day-ago).
            b_name if b_name.starts_with("power.") => {
                match materialize_power_band(cell64, s, b, None).await {
                    Ok(cid) => {
                        tracing::info!(
                            target: "emem::materialize",
                            materialize_cell = %cell64, materialize_band = %b,
                            materialize_fact_cid = %cid.as_str(),
                            materialize_kind = "primary",
                            "materialize_ok"
                        );
                        out.push(MaterializeOutcome {
                            band: b.clone(),
                            fact_cid: Some(cid.as_str().to_string()),
                            skip_reason: None,
                        });
                    }
                    Err(e) => {
                        tracing::warn!(
                            target: "emem::materialize",
                            materialize_cell = %cell64, materialize_band = %b,
                            materialize_error = %e,
                            "materialize_failed"
                        );
                        out.push(MaterializeOutcome {
                            band: b.clone(),
                            fact_cid: None,
                            skip_reason: Some(e),
                        });
                    }
                }
            }
            // Open-Meteo CAMS air-quality. Recall path: current hour.
            b_name if b_name.starts_with("cams.") => {
                match materialize_cams_band(cell64, s, b, None).await {
                    Ok(cid) => {
                        tracing::info!(
                            target: "emem::materialize",
                            materialize_cell = %cell64, materialize_band = %b,
                            materialize_fact_cid = %cid.as_str(),
                            materialize_kind = "primary",
                            "materialize_ok"
                        );
                        out.push(MaterializeOutcome {
                            band: b.clone(),
                            fact_cid: Some(cid.as_str().to_string()),
                            skip_reason: None,
                        });
                    }
                    Err(e) => {
                        tracing::warn!(
                            target: "emem::materialize",
                            materialize_cell = %cell64, materialize_band = %b,
                            materialize_error = %e,
                            "materialize_failed"
                        );
                        out.push(MaterializeOutcome {
                            band: b.clone(),
                            fact_cid: None,
                            skip_reason: Some(e),
                        });
                    }
                }
            }
            // Open-Meteo Archive ERA5. Recall path: 5 days ago (ERA5 publishes
            // with 5-day lag; "current" mode cannot return a value newer than that).
            b_name if b_name.starts_with("era5.") => {
                let now_unix = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                match materialize_era5_band(cell64, s, b, now_unix - 5 * 86_400).await {
                    Ok(cid) => {
                        tracing::info!(
                            target: "emem::materialize",
                            materialize_cell = %cell64, materialize_band = %b,
                            materialize_fact_cid = %cid.as_str(),
                            materialize_kind = "primary",
                            "materialize_ok"
                        );
                        out.push(MaterializeOutcome {
                            band: b.clone(),
                            fact_cid: Some(cid.as_str().to_string()),
                            skip_reason: None,
                        });
                    }
                    Err(e) => {
                        tracing::warn!(
                            target: "emem::materialize",
                            materialize_cell = %cell64, materialize_band = %b,
                            materialize_error = %e,
                            "materialize_failed"
                        );
                        out.push(MaterializeOutcome {
                            band: b.clone(),
                            fact_cid: None,
                            skip_reason: Some(e),
                        });
                    }
                }
            }
            // Open-Meteo Marine ECMWF WAM. Recall path: current hour.
            b_name if b_name.starts_with("marine.") => {
                match materialize_marine_band(cell64, s, b, None).await {
                    Ok(cid) => {
                        tracing::info!(
                            target: "emem::materialize",
                            materialize_cell = %cell64, materialize_band = %b,
                            materialize_fact_cid = %cid.as_str(),
                            materialize_kind = "primary",
                            "materialize_ok"
                        );
                        out.push(MaterializeOutcome {
                            band: b.clone(),
                            fact_cid: Some(cid.as_str().to_string()),
                            skip_reason: None,
                        });
                    }
                    Err(e) => {
                        tracing::warn!(
                            target: "emem::materialize",
                            materialize_cell = %cell64, materialize_band = %b,
                            materialize_error = %e,
                            "materialize_failed"
                        );
                        out.push(MaterializeOutcome {
                            band: b.clone(),
                            fact_cid: None,
                            skip_reason: Some(e),
                        });
                    }
                }
            }
            // ESA WorldCover 2021 (single global release).
            "esa_worldcover.lc_2021" => match materialize_esa_worldcover_2021(cell64, s).await {
                Ok(cid) => {
                    tracing::info!(
                        target: "emem::materialize",
                        materialize_cell = %cell64, materialize_band = %b,
                        materialize_fact_cid = %cid.as_str(),
                        materialize_kind = "primary_or_absence",
                        "materialize_ok"
                    );
                    out.push(MaterializeOutcome {
                        band: b.clone(),
                        fact_cid: Some(cid.as_str().to_string()),
                        skip_reason: None,
                    });
                }
                Err(e) => {
                    tracing::warn!(
                        target: "emem::materialize",
                        materialize_cell = %cell64, materialize_band = %b,
                        materialize_error = %e,
                        "materialize_failed"
                    );
                    out.push(MaterializeOutcome {
                        band: b.clone(),
                        fact_cid: None,
                        skip_reason: Some(e),
                    });
                }
            },
            // Hansen Global Forest Change v1.11.
            "hansen.tree_cover_2000" | "hansen.loss_year" | "hansen.gain" => {
                match materialize_hansen_band(cell64, s, b).await {
                    Ok(cid) => {
                        tracing::info!(
                            target: "emem::materialize",
                            materialize_cell = %cell64, materialize_band = %b,
                            materialize_fact_cid = %cid.as_str(),
                            materialize_kind = "primary",
                            "materialize_ok"
                        );
                        out.push(MaterializeOutcome {
                            band: b.clone(),
                            fact_cid: Some(cid.as_str().to_string()),
                            skip_reason: None,
                        });
                    }
                    Err(e) => {
                        tracing::warn!(
                            target: "emem::materialize",
                            materialize_cell = %cell64, materialize_band = %b,
                            materialize_error = %e,
                            "materialize_failed"
                        );
                        out.push(MaterializeOutcome {
                            band: b.clone(),
                            fact_cid: None,
                            skip_reason: Some(e),
                        });
                    }
                }
            }
            _ => {
                let e = format!("no_auto_materializer_registered: no upstream connector wired for band={b}; submit a signed Attestation via /v1/attest_cbor to seed it");
                out.push(MaterializeOutcome {
                    band: b.clone(),
                    fact_cid: None,
                    skip_reason: Some(e),
                });
            }
        }
    }
    out
}

fn chrono_iso8601_utc() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    iso8601_utc(secs)
}

/// `GET /v1/coverage` — JSON snapshot of where data lives. Returns a list
/// of (cell64, lat, lng, fact_count) up to `limit` (default 1000, max
/// 10000) so agents can cluster, sort, or inspect the actual data
/// distribution programmatically.
async fn coverage_json(
    State(s): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Json<JsonValue> {
    let limit: usize = q
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1000)
        .min(10_000);
    let storage = s.storage.as_ref();
    let entries = storage
        .iter_index(Some(limit * 8))
        .await
        .unwrap_or_default();
    // Bin by cell64 and count facts per cell.
    let mut by_cell: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    for (k, _) in &entries {
        *by_cell.entry(k.cell.clone()).or_default() += 1;
    }
    let mut cells: Vec<JsonValue> = by_cell
        .into_iter()
        .filter_map(|(c, n)| {
            let info = emem_codec::latlng_from_cell64(&c).ok()?;
            Some(json!({
                "cell64": c,
                "lat_deg": info.lat_deg,
                "lng_deg": info.lng_deg,
                "fact_count": n,
            }))
        })
        .collect();
    cells.sort_by(|a, b| {
        b["fact_count"]
            .as_u64()
            .unwrap_or(0)
            .cmp(&a["fact_count"].as_u64().unwrap_or(0))
    });
    cells.truncate(limit);
    Json(json!({
        "schema": "emem.coverage.v1",
        "total_cells": cells.len(),
        "limit": limit,
        "cells": cells,
        "next": [
            "GET /v1/coverage_map.svg  — same data as a 1440×720 raster (multimodal agents)",
            "POST /v1/recall            — read facts at any cell64 listed here"
        ]
    }))
}

/// `GET /v1/coverage_map.svg` — visual gym for multimodal agents.
/// Renders a 1440×720 Plate Carrée world map with a coloured rectangle
/// for every attested cell. Agents that can ingest images (Claude,
/// GPT-4V, Gemini) can answer "show me where data lives" with one call,
/// no client-side rendering needed.
/// Render the coverage map and return `(svg_text, cell_count, total_facts)`.
/// Shared between the REST handler [`coverage_map_svg`] and the MCP
/// `emem_coverage_map` tool so both surfaces produce the same image.
async fn build_coverage_map_svg(s: &AppState) -> (String, usize, u64) {
    let storage = s.storage.as_ref();
    let entries = storage.iter_index(Some(50_000)).await.unwrap_or_default();

    // Bin into 1° × 1° cells for the map render. Each bin gets a count
    // that drives the colour saturation. 360 columns × 180 rows = 64,800
    // bins — comfortably under SVG element budgets.
    let mut by_bin: std::collections::HashMap<(i32, i32), u64> = std::collections::HashMap::new();
    for (k, _) in &entries {
        if let Ok(info) = emem_codec::latlng_from_cell64(&k.cell) {
            let bin_lat = info.lat_deg.floor() as i32;
            let bin_lng = info.lng_deg.floor() as i32;
            *by_bin.entry((bin_lat, bin_lng)).or_default() += 1;
        }
    }
    let max_count = by_bin.values().copied().max().unwrap_or(1).max(1);

    // Plate Carrée: 1440 px wide (= 4 px / lng degree), 720 px tall
    // (= 4 px / lat degree). Bottom-left = (-180, -90), top-right = (180, 90).
    const W: i32 = 1440;
    const H: i32 = 720;
    let mut rects = String::new();
    for ((bin_lat, bin_lng), count) in &by_bin {
        let x = (bin_lng + 180) * W / 360;
        let y = (90 - bin_lat - 1) * H / 180; // top-down, 1° tall
        let w = W / 360;
        let h = H / 180;
        // Colour: cool teal at low density → warm yellow at high density
        // on log scale, so the eye doesn't flatten with one outlier cell.
        let t = ((*count as f64).ln_1p() / (max_count as f64).ln_1p()).clamp(0.0, 1.0);
        let r = (40.0 + 215.0 * t) as u8;
        let g = (180.0 - 60.0 * t) as u8;
        let b = (200.0 - 180.0 * t) as u8;
        rects.push_str(&format!(
            "<rect x='{x}' y='{y}' width='{w}' height='{h}' fill='#{:02x}{:02x}{:02x}'/>",
            r, g, b
        ));
    }
    let cell_count = by_bin.len();
    let total_facts: u64 = by_bin.values().sum();
    let pubkey = data_encoding::BASE32_NOPAD
        .encode(&s.identity.pubkey.0)
        .to_lowercase();
    let pubkey_short = &pubkey[..32.min(pubkey.len())];
    // Coastline / latitude reference. Without this the map is a black
    // rectangle with three coloured dots — a multimodal agent has no way
    // to anchor "where is that". Lat/lng grid lines every 30° + an
    // approximate landmass envelope (rough rectangles for the continents,
    // not real coastline) give enough geographic context to reason.
    let mut grid = String::new();
    for lat in (-60..=60).step_by(30) {
        let y = (90 - lat) * H / 180;
        grid.push_str(&format!(
            "<line x1='0' y1='{y}' x2='{W}' y2='{y}' stroke='#ffffff' stroke-width='1' opacity='0.12'/>"
        ));
        grid.push_str(&format!(
            "<text x='8' y='{}' font-size='10' opacity='0.5'>{lat}°</text>",
            y - 4
        ));
    }
    for lng in (-150..=150).step_by(60) {
        let x = (lng + 180) * W / 360;
        grid.push_str(&format!(
            "<line x1='{x}' y1='0' x2='{x}' y2='{H}' stroke='#ffffff' stroke-width='1' opacity='0.12'/>"
        ));
        grid.push_str(&format!(
            "<text x='{}' y='{}' font-size='10' opacity='0.5'>{lng}°</text>",
            x + 4,
            H - 6
        ));
    }
    // Hand-traced continent envelopes — *approximations* of land mass
    // bounding regions, accurate enough to anchor the eye, not a coastline.
    // Each path is a closed polyline in lat/lng, converted to pixel-space
    // by the same Plate Carrée transform used for cells.
    let landmasses: &[(&str, &[(f64, f64)])] = &[
        (
            "North America",
            &[
                (71.0, -160.0),
                (60.0, -141.0),
                (50.0, -126.0),
                (31.0, -117.0),
                (15.0, -95.0),
                (8.0, -78.0),
                (25.0, -80.0),
                (45.0, -66.0),
                (60.0, -55.0),
                (78.0, -72.0),
                (82.0, -95.0),
                (80.0, -130.0),
                (71.0, -160.0),
            ],
        ),
        (
            "South America",
            &[
                (12.0, -72.0),
                (0.0, -50.0),
                (-15.0, -39.0),
                (-35.0, -58.0),
                (-55.0, -67.0),
                (-50.0, -72.0),
                (-30.0, -71.0),
                (0.0, -80.0),
                (12.0, -72.0),
            ],
        ),
        (
            "Africa",
            &[
                (37.0, -9.0),
                (35.0, 11.0),
                (31.0, 33.0),
                (15.0, 42.0),
                (10.0, 52.0),
                (-12.0, 42.0),
                (-34.0, 18.0),
                (-30.0, 15.0),
                (0.0, 9.0),
                (15.0, -17.0),
                (37.0, -9.0),
            ],
        ),
        (
            "Europe",
            &[
                (71.0, 30.0),
                (60.0, 60.0),
                (45.0, 40.0),
                (36.0, 28.0),
                (36.0, -9.0),
                (50.0, -10.0),
                (58.0, -8.0),
                (71.0, 5.0),
                (71.0, 30.0),
            ],
        ),
        (
            "Asia",
            &[
                (78.0, 60.0),
                (75.0, 140.0),
                (60.0, 170.0),
                (40.0, 140.0),
                (20.0, 108.0),
                (8.0, 98.0),
                (8.0, 80.0),
                (20.0, 60.0),
                (30.0, 48.0),
                (45.0, 40.0),
                (60.0, 50.0),
                (78.0, 60.0),
            ],
        ),
        (
            "Oceania",
            &[
                (-10.0, 113.0),
                (-10.0, 153.0),
                (-25.0, 153.0),
                (-39.0, 146.0),
                (-35.0, 118.0),
                (-22.0, 114.0),
                (-10.0, 113.0),
            ],
        ),
        (
            "Antarctica",
            &[
                (-65.0, -180.0),
                (-78.0, -90.0),
                (-85.0, 0.0),
                (-78.0, 90.0),
                (-65.0, 180.0),
                (-65.0, -180.0),
            ],
        ),
    ];
    let mut land = String::new();
    for (_name, pts) in landmasses {
        let mut d = String::new();
        for (i, (lat, lng)) in pts.iter().enumerate() {
            let x = ((lng + 180.0) / 360.0 * W as f64) as i32;
            let y = ((90.0 - lat) / 180.0 * H as f64) as i32;
            d.push_str(&format!("{}{x},{y}", if i == 0 { "M" } else { " L" }));
        }
        d.push_str(" Z");
        land.push_str(&format!(
            "<path d='{d}' fill='#1a3a4a' fill-opacity='0.55' stroke='#3a6a7a' stroke-width='1' stroke-opacity='0.7'/>"
        ));
    }
    let svg = format!(
        r##"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}">
<defs><style>text{{font-family:ui-monospace,SF Mono,Menlo,Consolas,monospace;fill:#cfd8dc;}}</style></defs>
<rect width="{W}" height="{H}" fill="#06121a"/>
{land}
{grid}
<line x1="0" y1="{half_h}" x2="{W}" y2="{half_h}" stroke="#ffd166" stroke-width="1" opacity="0.4" stroke-dasharray="4,4"/>
<line x1="{half_w}" y1="0" x2="{half_w}" y2="{H}" stroke="#ffd166" stroke-width="1" opacity="0.4" stroke-dasharray="4,4"/>
{rects}
<rect x="14" y="14" width="520" height="64" fill="#000000" fill-opacity="0.55" rx="4"/>
<text x="24" y="40"  font-size="18" font-weight="700">emem.dev coverage map</text>
<text x="24" y="62"  font-size="12" opacity="0.85">{cell_count} attested cells · {total_facts} facts · responder {pubkey_short}…</text>
<text x="20" y="{bottom_l}" font-size="11" opacity="0.7">Plate Carrée · 1° × 1° bins · log-scale colour · grid every 30°</text>
<text x="{right_x}" y="{bottom_l}" font-size="11" opacity="0.7" text-anchor="end">cool=sparse  warm=dense</text>
</svg>
"##,
        half_h = H / 2,
        half_w = W / 2,
        bottom_l = H - 14,
        right_x = W - 20,
    );
    (svg, cell_count, total_facts)
}

/// `GET /v1/coverage_map.svg` — image/svg+xml render of the corpus
/// density (1° × 1° bins, log-scale colour, continent envelopes for
/// orientation). The MCP equivalent is the `emem_coverage_map` tool,
/// which returns the same SVG as an EmbeddedResource content block.
async fn coverage_map_svg(State(s): State<AppState>) -> Response {
    let (svg, _cells, _facts) = build_coverage_map_svg(&s).await;
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "image/svg+xml; charset=utf-8")
        .header(CACHE_CONTROL, "public, max-age=300")
        .body(axum::body::Body::from(svg))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// Result of a Sentinel-2 RGB scene render at a cell.
struct SceneRgb {
    /// PNG bytes — RGB 8-bit, default 256×256.
    png: Vec<u8>,
    /// Image width in pixels.
    w: u32,
    /// Image height in pixels.
    h: u32,
    /// STAC item id of the scene the pixels came from.
    item_id: String,
    /// ISO 8601 capture time.
    item_datetime: String,
    /// `eo:cloud_cover` from the STAC item.
    cloud_cover: Option<f64>,
    /// EPSG of the COG.
    epsg: u32,
    /// Per-channel `(p2, p98)` reflectance values used for the stretch
    /// (×10000 to recover S2's stored DN).
    stretch_p2_p98: ((f64, f64), (f64, f64), (f64, f64)),
}

/// Build a Sentinel-2 L2A true-colour PNG centred on the cell. Picks
/// the latest scene with `eo:cloud_cover < max_cloud` that intersects
/// the cell centroid (defaults to 20 %). Window size is fixed at 256
/// pixels = 2.56 km on the ground at S2's 10 m native resolution —
/// roughly 8 cell-widths, enough to anchor the eye.
async fn build_cell_scene_rgb(
    cell64: &str,
    max_cloud_pct: f64,
    datetime_window: Option<&str>,
) -> Result<SceneRgb, String> {
    let info = emem_codec::latlng_from_cell64(cell64).map_err(|e| format!("cell decode: {e}"))?;
    let lat = info.lat_deg;
    let lng = info.lng_deg;

    let datetime = match datetime_window {
        Some(d) => d.to_string(),
        None => {
            let now_unix = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let start_unix = now_unix - 90 * 86400;
            format!(
                "{}/{}",
                iso8601_utc(start_unix as u64),
                iso8601_utc(now_unix as u64)
            )
        }
    };

    let cli = s2_http_client();
    let item = emem_fetch::stac::search_one(
        &cli, "sentinel-2-l2a", lng, lat, &datetime, Some(max_cloud_pct),
    ).await
        .map_err(|e| format!("stac: {e}"))?
        .ok_or_else(|| format!(
            "no Sentinel-2 L2A scene with cloud_cover < {max_cloud_pct}% in the last 90 days at this cell"
        ))?;

    let red_url = item
        .assets
        .get("red")
        .or_else(|| item.assets.get("B04"))
        .cloned()
        .ok_or_else(|| "stac item missing red/B04 asset".to_string())?;
    let green_url = item
        .assets
        .get("green")
        .or_else(|| item.assets.get("B03"))
        .cloned()
        .ok_or_else(|| "stac item missing green/B03 asset".to_string())?;
    let blue_url = item
        .assets
        .get("blue")
        .or_else(|| item.assets.get("B02"))
        .cloned()
        .ok_or_else(|| "stac item missing blue/B02 asset".to_string())?;
    let epsg = item
        .epsg
        .ok_or_else(|| "stac item missing proj:epsg".to_string())?;
    let utm = emem_fetch::proj::latlng_to_utm_with_epsg(lat, lng, epsg)
        .ok_or_else(|| format!("epsg {epsg} not a UTM code"))?;

    const W: u32 = 256;
    const H: u32 = 256;

    let red_prof = emem_fetch::cog::open_profile(&cli, &red_url)
        .await
        .map_err(|e| format!("open red COG: {e}"))?;
    let green_prof = emem_fetch::cog::open_profile(&cli, &green_url)
        .await
        .map_err(|e| format!("open green COG: {e}"))?;
    let blue_prof = emem_fetch::cog::open_profile(&cli, &blue_url)
        .await
        .map_err(|e| format!("open blue COG: {e}"))?;
    let red_pix =
        emem_fetch::cog::sample_window(&cli, &red_url, &red_prof, utm.easting, utm.northing, W, H)
            .await
            .map_err(|e| format!("sample red: {e}"))?;
    let green_pix = emem_fetch::cog::sample_window(
        &cli,
        &green_url,
        &green_prof,
        utm.easting,
        utm.northing,
        W,
        H,
    )
    .await
    .map_err(|e| format!("sample green: {e}"))?;
    let blue_pix = emem_fetch::cog::sample_window(
        &cli,
        &blue_url,
        &blue_prof,
        utm.easting,
        utm.northing,
        W,
        H,
    )
    .await
    .map_err(|e| format!("sample blue: {e}"))?;

    // Per-channel 2nd–98th percentile stretch, then gamma 1/2.2.
    fn percentile(values: &[f64], p: f64) -> f64 {
        let mut v: Vec<f64> = values
            .iter()
            .copied()
            .filter(|x| x.is_finite() && *x > 0.0)
            .collect();
        if v.is_empty() {
            return 0.0;
        }
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let idx = ((v.len() as f64 - 1.0) * p).round() as usize;
        v[idx.min(v.len() - 1)]
    }
    let r_lo = percentile(&red_pix, 0.02);
    let r_hi = percentile(&red_pix, 0.98).max(r_lo + 1.0);
    let g_lo = percentile(&green_pix, 0.02);
    let g_hi = percentile(&green_pix, 0.98).max(g_lo + 1.0);
    let b_lo = percentile(&blue_pix, 0.02);
    let b_hi = percentile(&blue_pix, 0.98).max(b_lo + 1.0);

    let mut rgb = vec![0u8; (W as usize) * (H as usize) * 3];
    for i in 0..(W * H) as usize {
        let r = ((red_pix[i] - r_lo) / (r_hi - r_lo)).clamp(0.0, 1.0);
        let g = ((green_pix[i] - g_lo) / (g_hi - g_lo)).clamp(0.0, 1.0);
        let b = ((blue_pix[i] - b_lo) / (b_hi - b_lo)).clamp(0.0, 1.0);
        let r8 = (r.powf(1.0 / 2.2) * 255.0) as u8;
        let g8 = (g.powf(1.0 / 2.2) * 255.0) as u8;
        let b8 = (b.powf(1.0 / 2.2) * 255.0) as u8;
        rgb[i * 3] = r8;
        rgb[i * 3 + 1] = g8;
        rgb[i * 3 + 2] = b8;
    }

    let mut png_bytes: Vec<u8> = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_bytes, W, H);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| format!("png header: {e}"))?;
        writer
            .write_image_data(&rgb)
            .map_err(|e| format!("png write: {e}"))?;
    }

    Ok(SceneRgb {
        png: png_bytes,
        w: W,
        h: H,
        item_id: item.id,
        item_datetime: item.datetime,
        cloud_cover: item.cloud_cover,
        epsg,
        stretch_p2_p98: ((r_lo, r_hi), (g_lo, g_hi), (b_lo, b_hi)),
    })
}

/// `GET /v1/cells/{cell64}/scene.png` — true-colour Sentinel-2 RGB
/// thumbnail centred on the cell. Picks the latest scene with
/// `eo:cloud_cover < max_cloud` (default 20 %, override via
/// `?max_cloud=N` query). Pure-Rust pipeline: STAC search +
/// HTTP-Range COG reads + 2-98 percentile stretch + PNG encode.
/// 256×256 px = ~2.56 km × ~2.56 km at S2's 10 m native resolution.
async fn get_cell_scene_png(
    axum::extract::Path(cell64): axum::extract::Path<String>,
    axum::extract::Query(qs): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let cell = cell64.trim_end_matches(".png").to_string();
    let max_cloud = qs
        .get("max_cloud")
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(20.0);
    let datetime_window = qs.get("datetime").cloned();
    let scene = match build_cell_scene_rgb(&cell, max_cloud, datetime_window.as_deref()).await {
        Ok(s) => s,
        Err(e) => {
            return (StatusCode::NOT_FOUND, format!("scene unavailable: {e}")).into_response()
        }
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "image/png")
        .header(CACHE_CONTROL, "public, max-age=3600")
        .header("x-emem-scene-item-id", &scene.item_id)
        .header("x-emem-scene-datetime", &scene.item_datetime)
        .header(
            "x-emem-scene-cloud-cover",
            scene
                .cloud_cover
                .map(|c| format!("{c:.2}"))
                .unwrap_or_default(),
        )
        .header("x-emem-scene-epsg", scene.epsg.to_string())
        .body(axum::body::Body::from(scene.png))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

// ── /v1/ask — single-shot free-text answer routing ─────────────────────
//
// `/v1/ask` collapses the locate → topic-route → recall → algorithm chain
// into one round-trip so an LLM with emem connected can forward a
// free-text place question and receive packaged signed evidence — instead
// of refusing with "I can't access live data". The routing here is
// keyword-based and structural (deterministic, reproducible), not LLM-
// generated: the LLM at the top of the call stack does no protocol-
// reasoning, it just relays the user's question.
//
// `TOPIC_BANDS` and `TOPIC_ALGORITHMS` mirror what /v1/locate's
// `data_at_this_cell` block exposes today; the duplication is editorial
// (kept here so /v1/ask is self-contained at code review). The unit
// test `topic_route_matches_locate_inventory` keeps them in sync.

const TOPIC_BANDS: &[(&str, &[&str])] = &[
    ("flood_history_long_term", &["surface_water.recurrence"]),
    (
        "flood_water_event_window",
        &["indices.ndwi", "indices.mndwi", "sentinel1_raw"],
    ),
    (
        "vegetation_condition",
        &[
            "indices.ndvi",
            "indices.evi",
            "indices.savi",
            "indices.ndmi",
            "modis.ndvi_mean",
        ],
    ),
    ("fire_burn_severity", &["indices.nbr"]),
    ("soil_bare", &["indices.bsi"]),
    (
        "built_up_human_geography",
        &[
            "indices.ndbi",
            "overture.buildings.count",
            "overture.places.count",
            "overture.transportation.road_length_m",
        ],
    ),
    (
        "weather_now",
        &[
            "weather.temperature_2m",
            "weather.cloud_cover",
            "weather.precipitation_mm",
            "weather.wind_speed_10m",
        ],
    ),
    ("elevation_global_topobathy", &["gmrt.topobathy_mean"]),
    ("elevation_land_only", &["copdem30m.elevation_mean"]),
    (
        "optical_raw_reflectance",
        &["s2.B02", "s2.B03", "s2.B04", "s2.B08", "s2.B11", "s2.B12"],
    ),
    ("scene_classification", &["s2.scl"]),
    (
        "foundation_embedding",
        &["geotessera", "geotessera.multi_year"],
    ),
    ("radar_all_weather_sar", &["sentinel1_raw"]),
];

const TOPIC_ALGORITHMS: &[(&str, &[&str])] = &[
    ("flood_history_long_term", &["flood_history_class@1"]),
    (
        "flood_water_event_window",
        &["water_consensus@1", "water_likelihood_from_vv@1"],
    ),
    (
        "flood_risk_composite",
        &["flood_risk@1", "route_flood_exposure@1"],
    ),
    (
        "vegetation_condition",
        &["vegetation_class_from_ndvi@1", "crop_stress_score@1"],
    ),
    (
        "fire_burn_severity",
        &[
            "burn_likelihood_from_nbr@1",
            "burn_severity_from_dnbr@1",
            "wildfire_exposure_score@1",
        ],
    ),
    ("soil_bare", &["bare_soil_class@1"]),
    ("snow", &["snow_likelihood_from_ndsi@1"]),
    (
        "built_up_human_geography",
        &[
            "built_up_from_ndbi@1",
            "urban_density_score@1",
            "noise_exposure_proxy@1",
        ],
    ),
    (
        "weather_now",
        &[
            "heat_index@2",
            "heat_health_risk@2",
            "wind_chill@1",
            "outdoor_comfort_score@1",
            "precip_intensity_class@1",
        ],
    ),
    (
        "topography",
        &[
            "slope_from_dem_neighborhood@1",
            "ruggedness_index@1",
            "topo_position_index@1",
            "coastal_proximity@1",
        ],
    ),
    (
        "foundation_embedding",
        &[
            "embedding_cosine@1",
            "embedding_l2_distance@1",
            "embedding_change_score@1",
            "region_similarity@1",
            "place_archetype_match@1",
        ],
    ),
    (
        "real_estate",
        &[
            "property_climate_risk_score@1",
            "insurance_premium_proxy@1",
            "coastal_erosion_proxy@1",
            "multi_peril_score@1",
        ],
    ),
    (
        "esg",
        &[
            "carbon_sink_score@1",
            "biodiversity_proxy@1",
            "physical_climate_risk_index@1",
        ],
    ),
    (
        "agriculture",
        &["crop_yield_proxy@1", "vineyard_terroir_score@1"],
    ),
    ("public_health", &["heat_vulnerability_index@1"]),
    (
        "urban_livability",
        &[
            "walkability_score@1",
            "bikeability_score@1",
            "green_space_access@1",
            "outdoor_comfort_score@1",
            "livability_index@1",
        ],
    ),
    (
        "analytics",
        &[
            "spatial_volatility_index@1",
            "trend_strength@1",
            "anomaly_zscore@1",
        ],
    ),
];

// Keyword routing. Order matters: composite/lifestyle topics come first
// so a question like "buy a flat in Ashok Nagar Ranchi, is it flood-
// prone" routes to flood_risk_composite (which composes
// surface_water.recurrence + Cop-DEM + S1 via flood_risk@1) instead of
// stopping at the single-band flood_history_long_term.
const TOPIC_KEYWORDS: &[(&str, &[&str])] = &[
    (
        "flood_risk_composite",
        &[
            "flood-prone",
            "flood prone",
            "floodprone",
            "flood risk",
            // Word-ordering variants — agents and users alternate between
            // "buy a flat" / "purchase a flat" / "purchasing a flat" / "buying
            // an apartment" freely; match any of them so a real-estate
            // question always routes to the composite recipe.
            "buy a flat",
            "buying a flat",
            "purchase a flat",
            "purchasing a flat",
            "flat purchase",
            "flat to buy",
            "buy an apartment",
            "purchase an apartment",
            "buy a house",
            "buying a house",
            "purchase a house",
            "purchasing a house",
            "buy a home",
            "buying a home",
            "purchase a home",
            "buy property",
            "buying property",
            "purchase property",
            "invest in property",
            "real estate purchase",
            "safe to live",
            "safe to buy",
            "should i live",
            "should i buy",
            "is it safe",
            "is this safe",
            "monsoon flooding",
            "monsoon water",
            "monsoon waterlogging",
            "drainage",
            "floodplain",
        ],
    ),
    (
        "real_estate",
        &[
            "insurance premium",
            "property risk",
            "real estate risk",
            "climate risk score",
            "physical climate risk",
        ],
    ),
    (
        "urban_livability",
        &[
            "walkable",
            "walkability",
            "bikeability",
            "livable",
            "livability",
            "heat island",
            "green space",
            "outdoor comfort",
            "quality of life",
        ],
    ),
    (
        "flood_history_long_term",
        &[
            "flood history",
            "historical flood",
            "ever flooded",
            "past flood",
            "flooded before",
            "flooded in",
            "long-term flood",
        ],
    ),
    (
        "flood_water_event_window",
        &[
            "water now",
            "standing water",
            "puddle",
            "waterlogged",
            "is there water",
            "wet right now",
            "current water",
        ],
    ),
    (
        "vegetation_condition",
        &[
            "vegetation",
            "ndvi",
            "greenness",
            "green cover",
            "tree cover",
            "forest cover",
            "crop health",
            "biomass",
        ],
    ),
    (
        "fire_burn_severity",
        &[
            "fire",
            "burn",
            "wildfire",
            "burned",
            "scorched",
            "burn severity",
        ],
    ),
    (
        "built_up_human_geography",
        &[
            "urban density",
            "developed",
            "city density",
            "building",
            "road length",
            "built up",
            "built-up",
            "ndbi",
        ],
    ),
    (
        "weather_now",
        &[
            "weather",
            "temperature now",
            "rain now",
            "precipitation",
            "wind speed",
            "humidity",
            "current heat",
        ],
    ),
    (
        "elevation_land_only",
        &[
            "elevation",
            "altitude",
            "how high",
            "metres above",
            "meters above",
            "above sea level",
        ],
    ),
    (
        "topography",
        &["slope", "terrain", "ruggedness", "ridge", "valley"],
    ),
    (
        "agriculture",
        &[
            "crop yield",
            "farm yield",
            "agricultural",
            "wheat",
            "rice yield",
            "maize",
            "vineyard",
        ],
    ),
    (
        "esg",
        &[
            "carbon sink",
            "biodiversity",
            "esg",
            "environmental pressure",
            "transition risk",
        ],
    ),
    (
        "public_health",
        &[
            "air quality",
            "air pollution",
            "vector-borne",
            "mosquito",
            "heat vulnerability",
        ],
    ),
    (
        "foundation_embedding",
        &[
            "similar to",
            "like this place",
            "find places like",
            "compare to",
            "embedding",
        ],
    ),
    (
        "analytics",
        &["volatility", "trend", "anomaly", "outlier", "z-score"],
    ),
];

fn live_bands_for_topic(topic: &str) -> &'static [&'static str] {
    TOPIC_BANDS
        .iter()
        .find(|(k, _)| *k == topic)
        .map(|(_, v)| *v)
        .unwrap_or(&[])
}

fn algorithms_keys_for_topic(topic: &str) -> &'static [&'static str] {
    TOPIC_ALGORITHMS
        .iter()
        .find(|(k, _)| *k == topic)
        .map(|(_, v)| *v)
        .unwrap_or(&[])
}

/// Map the user's free-text question to the set of topic keys that
/// match. Pure structural routing (lower-case substring containment),
/// not LLM-generated — so the same question always routes the same way
/// and the receipt is reproducible. Topics are returned in TOPIC_KEYWORDS
/// declaration order, which puts composite/lifestyle topics first.
pub fn route_question_to_topics(q: &str) -> Vec<&'static str> {
    let q_lc = q.to_ascii_lowercase();
    let mut hits: Vec<&'static str> = Vec::new();
    for (topic, kws) in TOPIC_KEYWORDS {
        if kws.iter().any(|k| q_lc.contains(*k)) {
            hits.push(*topic);
        }
    }
    hits
}

/// Per-topic matched keyword breakdown for the response envelope.
fn matched_keywords(q: &str) -> Vec<JsonValue> {
    let q_lc = q.to_ascii_lowercase();
    TOPIC_KEYWORDS
        .iter()
        .filter_map(|(topic, kws)| {
            let m: Vec<&str> = kws.iter().copied().filter(|k| q_lc.contains(*k)).collect();
            if m.is_empty() {
                None
            } else {
                Some(json!({"topic": topic, "matched": m}))
            }
        })
        .collect()
}

#[derive(Deserialize)]
struct AskReq {
    /// User's natural-language question.
    q: String,
    /// Free-text place name (resolved via /v1/locate). One of `place`,
    /// `cell`, or both `lat`+`lng` is required.
    #[serde(default)]
    place: Option<String>,
    /// cell64 string (alternative to `place`).
    #[serde(default)]
    cell: Option<String>,
    /// WGS-84 latitude (paired with `lng`).
    #[serde(default)]
    lat: Option<f64>,
    #[serde(default)]
    lng: Option<f64>,
    /// Bundle a Sentinel-2 RGB scene URL with the response.
    #[serde(default)]
    include_image: bool,
}

async fn post_ask(
    State(s): State<AppState>,
    Json(req): Json<AskReq>,
) -> Result<Json<JsonValue>, ApiError> {
    ask_inner(s, req).await.map(Json)
}

async fn ask_inner(s: AppState, req: AskReq) -> Result<JsonValue, ApiError> {
    if req.q.trim().is_empty() {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            ErrorBody {
                code: ErrorCode::Internal,
                message: "ask: `q` cannot be empty".into(),
            },
        ));
    }

    // Resolve cell64 from whichever locator the caller provided. We
    // prefer explicit cell64 → lat/lng → place name, in that order, so
    // an LLM that has the cell from a previous /v1/locate doesn't pay a
    // second geocoder round-trip.
    let (cell, place_resolved): (String, JsonValue) = if let Some(c) = req.cell.as_ref() {
        if !emem_codec::is_cell64_shape(c) {
            return Err(ApiError(
                StatusCode::BAD_REQUEST,
                ErrorBody {
                    code: ErrorCode::Internal,
                    message: format!("ask: `cell` must be a cell64 string, got '{c}'"),
                },
            ));
        }
        let centre = emem_codec::latlng_from_cell64(c).ok();
        (
            c.clone(),
            json!({
                "cell64": c,
                "lat":    centre.as_ref().map(|p| p.lat_deg),
                "lng":    centre.as_ref().map(|p| p.lng_deg),
                "via":    "direct_cell",
            }),
        )
    } else if let (Some(la), Some(lo)) = (req.lat, req.lng) {
        let body = locate_inner(LocateReq {
            lat: Some(la),
            lng: Some(lo),
            place: None,
        })
        .await?
        .0;
        let cell = body
            .get("cell64")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        (
            cell.clone(),
            json!({
                "cell64": cell, "lat": la, "lng": lo, "via": "direct_latlng",
            }),
        )
    } else if let Some(p) = req.place.as_ref() {
        let body = locate_inner(LocateReq {
            lat: None,
            lng: None,
            place: Some(p.clone()),
        })
        .await?
        .0;
        let cell = body
            .get("cell64")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let label = body.get("place_label").cloned().unwrap_or(JsonValue::Null);
        let lat = body
            .get("lat_input")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let lng = body
            .get("lng_input")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let via = body.get("via").cloned().unwrap_or(json!("unknown"));
        let polygon_bbox = body.get("polygon_bbox").cloned().unwrap_or(JsonValue::Null);
        (
            cell.clone(),
            json!({
                "cell64": cell, "input": p, "label": label, "lat": lat, "lng": lng,
                "via": via, "polygon_bbox": polygon_bbox,
            }),
        )
    } else {
        return Err(ApiError(StatusCode::BAD_REQUEST, ErrorBody {
            code: ErrorCode::Internal,
            message: "ask: requires one of `place` (free text), `cell` (cell64), or both `lat` and `lng`".into(),
        }));
    };

    let topics = route_question_to_topics(&req.q);
    let alg_reg = &*emem_core::algorithms::DEFAULT;

    // Union of every band needed to answer the matched topics. For
    // single-band topics this is `live_bands_for_topic`; for composite
    // topics (flood_risk_composite, urban_livability, real_estate) the
    // bands come from the algorithm registry's declared `inputs`.
    let mut want_bands: std::collections::BTreeSet<String> = Default::default();
    for t in &topics {
        for b in live_bands_for_topic(t) {
            want_bands.insert((*b).into());
        }
        for alg_key in algorithms_keys_for_topic(t) {
            for b in alg_reg.input_bands(alg_key) {
                want_bands.insert(b.into());
            }
        }
    }
    let bands_vec: Vec<String> = want_bands.iter().cloned().collect();

    let recall_req = RecallReq {
        cell: cell.clone(),
        bands: if bands_vec.is_empty() {
            None
        } else {
            Some(bands_vec.clone())
        },
        tslot: None,
    };
    let (recall_resp, materialize_notes) = recall_with_auto_materialize(&recall_req, &s).await?;

    // Algorithm hints — for each matched topic, surface every recipe key
    // the agent should apply, with input bands, formula, output range,
    // and citation. The agent applies the formula in-process and cites
    // algorithm key + algorithms_cid alongside the input fact_cids.
    let alg_cid = emem_core::manifest::manifest_cid(alg_reg).ok();
    let mut algorithms_for_question: Vec<JsonValue> = Vec::new();
    for t in &topics {
        for alg_key in algorithms_keys_for_topic(t) {
            if let Some(a) = alg_reg.lookup(alg_key) {
                let inputs: Vec<&str> = a.inputs.iter().filter_map(|i| i.band.as_deref()).collect();
                algorithms_for_question.push(json!({
                    "topic":       t,
                    "key":         a.key,
                    "kind":        a.kind,
                    "input_bands": inputs,
                    "formula":     a.formula,
                    "output":      a.output,
                    "citation":    a.citation,
                    "fetch_url":   format!("/v1/algorithms/{}", a.key),
                }));
            }
        }
    }

    // Optional Sentinel-2 RGB scene. Default off — the scene fetch is
    // ~1-2 s on first call (STAC search + COG range reads + PNG encode)
    // and most ask flows are decision-making, not visual.
    let scene_url = format!(
        "{}/v1/cells/{cell}/scene.png?max_cloud=40",
        public_origin().unwrap_or_else(|| "urn:emem".into())
    );
    let scene = if req.include_image {
        match build_cell_scene_rgb(&cell, 40.0, None).await {
            Ok(s) => json!({
                "fetched":            true,
                "url":                scene_url,
                "stac_item_id":       s.item_id,
                "stac_item_datetime": s.item_datetime,
                "cloud_cover":        s.cloud_cover,
                "epsg":               s.epsg,
            }),
            Err(e) => json!({
                "fetched": false,
                "url":     scene_url,
                "error":   e,
            }),
        }
    } else {
        json!({
            "fetched": false,
            "url":     scene_url,
            "_note":   "set include_image=true to fetch the latest cloud-free Sentinel-2 thumbnail in this call",
        })
    };

    let caveats = json!({
        "grid_resolution":               "cell64 is ~305 m × ~611 m at the equator. Sub-cell phenomena (street-level waterlogging, single-building damage) are below this grid. Spec target is 3.4 m H3 — declared via /v1/grid_info.",
        "satellite_revisit_typical":     "Sentinel-1 6-12 d; Sentinel-2 5 d combined. Short-duration urban flash events between revisits are not captured by satellite bands; check `weather.precipitation_mm` for the now-cast.",
        "out_of_scope_when_topic_null":  "If `topic_routing.matched_topics` is empty, the question class isn't covered by this responder. Topics not listed in `data_at_this_cell.live_bands_by_topic` (real-time air quality, traffic, indoor data) are out of scope today — say so honestly instead of refusing or web-searching.",
    });

    let topics_empty = topics.is_empty();
    let mut body = json!({
        "schema":         "emem.ask.v1",
        "question":       req.q,
        "place_resolved": place_resolved,
        "topic_routing": {
            "matched_topics":   topics,
            "matched_keywords": matched_keywords(&req.q),
            "out_of_scope":     topics_empty,
            "_explanation":     "Topics are matched by lower-cased substring presence against TOPIC_KEYWORDS. The match is structural, not LLM-generated, so the routing is reproducible. Composite topics (flood_risk_composite, urban_livability, real_estate) get priority over single-band topics so a lifestyle question routes to the algorithm recipe, not just one band.",
        },
        "facts":                   serde_json::to_value(&recall_resp).unwrap_or(json!({})),
        "algorithms_for_question": algorithms_for_question,
        "algorithms_cid":          alg_cid,
        "scene":                   scene,
        "caveats":                 caveats,
    });
    if let Some(map) = body.as_object_mut() {
        if !materialize_notes.is_empty() {
            map.insert(
                "materialize_notes".into(),
                JsonValue::Array(materialize_notes),
            );
        }
        if topics_empty {
            map.insert("inventory".into(), json!({
                "_purpose":              "Topic-grouped roster of every band/algorithm at this cell. Use this when no topic auto-routed — pick the topic whose name best matches the user's question and call /v1/recall directly.",
                "live_bands_by_topic":   TOPIC_BANDS.iter().map(|(k,v)| (*k, *v)).collect::<std::collections::BTreeMap<&str, &[&str]>>(),
                "algorithms_for_topic":  TOPIC_ALGORITHMS.iter().map(|(k,v)| (*k, *v)).collect::<std::collections::BTreeMap<&str, &[&str]>>(),
            }));
        }
    }
    Ok(body)
}

async fn locate_inner(req: LocateReq) -> Result<Json<JsonValue>, ApiError> {
    // Provenance of the (lat,lng) returned to the agent. "direct" — caller
    // supplied coordinates; "embedded" — hit our compiled-in gazetteer
    // (no upstream network call); "cache" — Nominatim TTL cache hit;
    // "nominatim" — live Nominatim call.
    let mut via = "direct";
    let mut polygon_bbox: Option<(f64, f64, f64, f64)> = None;
    let mut polygon_source: Option<&'static str> = None;
    // Alternative candidates surfaced when Nominatim returned multiple
    // matches for an ambiguous place name. Empty for non-ambiguous lookups
    // and for cache/embedded paths (those are deterministic single-result).
    let mut alternatives: Vec<JsonValue> = Vec::new();
    let (lat, lng, label) = match (req.lat, req.lng, req.place.as_deref()) {
        (Some(la), Some(lo), _) => (la, lo, None),
        (_, _, Some(p)) if !p.is_empty() => {
            // First check our wide-bbox table — if the name is a known wide
            // feature, the bbox here is *better* than any centroid because
            // it tells the agent to fan out instead of trusting one point.
            if let Some(bbox) = wide_bbox_lookup(p) {
                polygon_bbox = Some(bbox);
                polygon_source = Some("wide_bbox_table");
            }
            // Layer 1: embedded gazetteer (no network).
            if let Some((la, lo, lab)) = embedded_gazetteer_lookup(p) {
                via = "embedded";
                (la, lo, Some(lab))
            } else if let Some((la, lo, lab, bb_cached)) = nominatim_cache_get(p) {
                // Layer 2: persistent cache hit. Recover the polygon_bbox
                // from cache if we stored one, so recall_polygon at a
                // cached place doesn't lose the polygon and degrade to a
                // single-cell fan-out. (Pre-fix: cache stored only lat/lng,
                // forcing every cached recall_polygon to centre_cell_bbox.)
                via = "cache";
                if polygon_bbox.is_none() {
                    if let Some(arr) = bb_cached {
                        polygon_bbox = Some((arr[0], arr[1], arr[2], arr[3]));
                        polygon_source = Some("nominatim_boundingbox");
                    }
                }
                (la, lo, Some(lab))
            } else {
                via = "nominatim";
                // Fetch up to 5 candidates so we can surface alternatives
                // for ambiguous names ("Springfield", "San José"). Picking
                // the first match silently is what produced the worst kind
                // of place-name drift in earlier trials.
                let hits = nominatim_lookup_candidates(p, 5).await.map_err(|e| ApiError(
                    StatusCode::BAD_GATEWAY,
                    ErrorBody { code: ErrorCode::Internal, message: format!("place lookup failed: {e}") },
                ))?;
                let hit = hits.first().cloned().ok_or_else(|| ApiError(
                    StatusCode::NOT_FOUND,
                    ErrorBody { code: ErrorCode::Internal, message: format!("no geocoder match for '{p}'") },
                ))?;
                let hit_bbox_arr = hit.bbox.map(|(a, b, c, d)| [a, b, c, d]);
                nominatim_cache_put(p, hit.lat, hit.lng, &hit.label, hit_bbox_arr);
                if polygon_bbox.is_none() {
                    if let Some(bb) = hit.bbox {
                        polygon_bbox = Some(bb);
                        polygon_source = Some("nominatim_boundingbox");
                    }
                }
                // Build alternatives from hits[1..] so the agent can
                // disambiguate without a second HTTP round-trip.
                for alt in hits.iter().skip(1) {
                    let alt_cell = emem_codec::to_cell64(emem_codec::cell_from_latlng(alt.lat, alt.lng));
                    alternatives.push(json!({
                        "cell64":     alt_cell,
                        "lat":        alt.lat,
                        "lng":        alt.lng,
                        "label":      alt.label,
                        "osm_type":   alt.osm_type,
                        "class":      alt.class_,
                        "type":       alt.type_,
                        "importance": alt.importance,
                    }));
                }
                (hit.lat, hit.lng, Some(hit.label))
            }
        }
        _ => return Err(ApiError(
            StatusCode::BAD_REQUEST,
            ErrorBody {
                code: ErrorCode::Internal,
                message: "locate requires either {lat, lng} (WGS-84 degrees) or {place: \"...\"} (free-text place name; aliases accepted: q, query, name). Examples: {\"lat\":35.36,\"lng\":138.73} or {\"place\":\"Mt Fuji\"} or {\"q\":\"東京\"}.".into(),
            },
        )),
    };
    let cell = emem_codec::cell_from_latlng(lat, lng);
    let cell_str = emem_codec::to_cell64(cell);
    let center = emem_codec::latlng_from_cell64(&cell_str).ok();
    // 9-cell neighborhood (centre + 8 corners). Two callers asking about
    // "Mt. Fuji" from slightly different lat/lngs can land in adjacent cells;
    // an agent that recalls only the centre will miss data attested under a
    // neighbor. Returning the surrounding cells lets the caller fan out
    // recall and union the results — without us guessing the right radius.
    let neighborhood = if let Some(c) = center.as_ref() {
        let dlat = c.bbox_deg.max_lat - c.bbox_deg.min_lat;
        let dlng = c.bbox_deg.max_lng - c.bbox_deg.min_lng;
        let mut seen = std::collections::BTreeSet::new();
        let mut out = Vec::with_capacity(9);
        for (sa, sb) in [
            (0.0, 0.0),
            (1.0, 0.0),
            (-1.0, 0.0),
            (0.0, 1.0),
            (0.0, -1.0),
            (1.0, 1.0),
            (1.0, -1.0),
            (-1.0, 1.0),
            (-1.0, -1.0),
        ] {
            let s = emem_codec::to_cell64(emem_codec::cell_from_latlng(
                c.lat_deg + sa * dlat,
                c.lng_deg + sb * dlng,
            ));
            if seen.insert(s.clone()) {
                out.push(s);
            }
        }
        out
    } else {
        vec![cell_str.clone()]
    };
    let mut body = json!({
        "cell64": cell_str,
        "lat_input": lat,
        "lng_input": lng,
        "place_label": label,
        "via": via,
        "centre": center.as_ref().map(|c| json!({"lat_deg": c.lat_deg, "lng_deg": c.lng_deg})),
        "bbox_deg": center.as_ref().map(|c| json!({
            "min_lat": c.bbox_deg.min_lat, "max_lat": c.bbox_deg.max_lat,
            "min_lng": c.bbox_deg.min_lng, "max_lng": c.bbox_deg.max_lng,
        })),
        "neighborhood_cells": neighborhood,
        "polygon_bbox": polygon_bbox.map(|(s, n, w, e)| json!({
            "min_lat": s, "max_lat": n, "min_lng": w, "max_lng": e,
            "source": polygon_source,
        })),
        "polygon_sample_cells": polygon_bbox.map(|bb| sample_cells_in_bbox(bb, 64)),
        "advice": "Place names map to a single cell at ~305 m / ~610 m resolution (lat × lng axis at equator; lng narrows with latitude). For point features (peaks, towers), use `neighborhood_cells` to fan out across the immediate ~9 cells. For wide features (canyons, basins, regions, countries), `polygon_bbox` carries the actual extent and `polygon_sample_cells` is a 64-cell grid sample inside it — query those to find the data. **`data_at_this_cell` lists every band you can recall here, grouped by topic — read it BEFORE concluding emem can't answer.**",
        // Topic-grouped roster of every band the responder can answer at
        // this cell, plus the cube placeholders that have no materializer
        // wired (so the agent reports them as honest "not connected" rather
        // than concluding emem doesn't carry that data class). This is the
        // first-pass discovery surface: every geospatial query starts with
        // locate, so the agent sees the full inventory before picking
        // bands. Drift risk vs `materializer_bands` is mitigated by the
        // unit test in tests::locate_inventory_matches_coverage_matrix.
        "data_at_this_cell": json!({
            "_purpose": "Topic-grouped list of every band you can recall at this cell, plus the algorithm recipes that compose those bands into named scores. Read this BEFORE concluding the responder doesn't have data on a topic, and BEFORE inventing a synthesis formula in your own reasoning.",
            "_pointer_to_algorithms": "GET /v1/algorithms — full content-addressed registry of 68 composition recipes (flood_risk, walkability, embedding_novelty, etc.) used by `algorithms_for_topic` below. Cite the algorithm key + algorithms_cid in the receipt.",
            "live_bands_by_topic": {
                "flood_history_long_term":    ["surface_water.recurrence"],
                "flood_water_event_window":   ["indices.ndwi","indices.mndwi","sentinel1_raw"],
                "vegetation_condition":       ["indices.ndvi","indices.evi","indices.savi","indices.ndmi","modis.ndvi_mean"],
                "fire_burn_severity":         ["indices.nbr"],
                "soil_bare":                  ["indices.bsi"],
                "built_up_human_geography":   ["indices.ndbi","overture.buildings.count","overture.places.count","overture.transportation.road_length_m"],
                "weather_now":                ["weather.temperature_2m","weather.cloud_cover","weather.precipitation_mm","weather.wind_speed_10m"],
                "elevation_global_topobathy": ["gmrt.topobathy_mean"],
                "elevation_land_only":        ["copdem30m.elevation_mean"],
                "optical_raw_reflectance":    ["s2.B02","s2.B03","s2.B04","s2.B08","s2.B11","s2.B12"],
                "scene_classification":       ["s2.scl"],
                "foundation_embedding":       ["geotessera","geotessera.multi_year"],
                "radar_all_weather_sar":      ["sentinel1_raw"],
            },
            // For each topic above, the algorithm recipe(s) that compose
            // its bands into a derived answer. Agents should prefer the
            // named recipe over inventing thresholds — receipts citing
            // an algorithm_cid replay deterministically.
            "algorithms_for_topic": {
                "flood_history_long_term":    ["flood_history_class@1"],
                "flood_water_event_window":   ["water_consensus@1", "water_likelihood_from_vv@1"],
                "flood_risk_composite":       ["flood_risk@1", "route_flood_exposure@1"],
                "vegetation_condition":       ["vegetation_class_from_ndvi@1", "crop_stress_score@1", "agb_ndvi_powerlaw@1"],
                "fire_burn_severity":         ["burn_likelihood_from_nbr@1", "burn_severity_from_dnbr@1", "wildfire_exposure_score@1", "fosberg_fire_weather_index@1"],
                "soil_bare":                  ["bare_soil_class@1"],
                "snow":                       ["snow_likelihood_from_ndsi@1"],
                "built_up_human_geography":   ["built_up_from_ndbi@1", "urban_density_score@1", "population_ghsl_dasymetric@1", "noise_exposure_proxy@1"],
                "weather_now":                ["heat_index@2", "heat_health_risk@2", "wind_chill@1", "outdoor_comfort_score@1", "precip_intensity_class@1", "vapor_pressure_deficit@1"],
                "topography":                 ["slope_from_dem_neighborhood@1", "ruggedness_index@1", "topo_position_index@1", "coastal_proximity@1"],
                "foundation_embedding":       ["embedding_cosine@1", "embedding_l2_distance@1", "embedding_change_score@1", "embedding_novelty@1", "embedding_neighborhood_consistency@1", "embedding_centroid@1", "region_similarity@1", "place_archetype_match@1", "region_outlier_score@1", "embedding_corridor_consistency@1", "embedding_diversity_score@1"],
                "real_estate":                ["property_climate_risk_score@1", "insurance_premium_proxy@1", "coastal_erosion_proxy@1", "multi_peril_score@1"],
                "esg":                        ["carbon_sink_score@1", "biodiversity_proxy@1", "esg_environmental_pressure@1", "physical_climate_risk_index@1", "transition_risk_proxy@1"],
                "agriculture":                ["crop_yield_proxy@1", "vineyard_terroir_score@1"],
                "energy":                     ["wind_power_density@1", "hydro_theoretical_power@1", "ghi_clearsky_haurwitz@1"],
                "public_health":              ["air_stagnation_wang_angell@1", "aedes_thermal_suitability_mordecai@1", "heat_vulnerability_index@1"],
                "urban_livability":           ["walkability_score@1", "bikeability_score@1", "urban_heat_island_imhoff@1", "green_space_access@1", "outdoor_comfort_score@1", "construction_site_exposure@1", "livability_index@1"],
                "analytics":                  ["spatial_volatility_index@1", "trend_strength@1", "anomaly_zscore@1", "visual_search_match@1"],
            },
            // Multimodal (visual / image) surfaces available at this cell.
            "visual_surfaces": {
                "rgb_scene_png": "GET /v1/cells/{cell64}/scene.png?max_cloud=20  (or MCP `emem_cell_scene_rgb`) — true-colour Sentinel-2 L2A 256×256 thumbnail",
                "cell_geojson":  "GET /v1/cells/{cell64}/geojson — polygon hexagon for any GIS / map renderer",
                "cell_recall_geojson": "GET /v1/cells/{cell64}/recall_geojson?bands=... — properties carry every fact value the responder has, ready to style",
            },
            "declared_but_no_materializer_at_this_responder": {
                "_meaning": "These bands are reserved in the cube manifest but have no live connector. Recall returns empty. Tell the user honestly: 'this responder doesn't have a connector for X' — don't web-search until you've reported the gap.",
                "_note_on_surface_water_vector": "The 12-d cube key `surface_water` is unfilled (no responder has agreed on the slot allocation yet). The scalar `surface_water.recurrence` IS live (see live_bands_by_topic.flood_history_long_term) and answers the historical-flood question.",
                "deforestation_canopy_loss":   ["forest_change"],
                "landcover_classes":           ["landcover","ecoregions","mangrove","protected"],
                "human_population":            ["nightlights","ghsl","population"],
                "climate_long_term":           ["koppen","terraclimate"],
                "soil_properties":             ["soilgrids"],
                "ocean_chemistry":             ["ocean_chl"],
            },
            "how_to_use": "Pick the topic that matches the user's question. (1) If the user wants ONE band's value, look up `live_bands_by_topic` and call `emem_recall` with those bands — they auto-fetch on miss. (2) If the user wants a COMPOSITE answer (flood risk, walkability, climate exposure, similarity, change), look up `algorithms_for_topic` and call `emem_algorithms` for the recipe — apply its `formula` over a single `emem_recall` body that fetches every input band, then cite the algorithm key + algorithms_cid alongside the input fact_cids. (3) For a VISUAL answer, hit `visual_surfaces.rgb_scene_png` (or MCP `emem_cell_scene_rgb`). (4) If the topic only appears under `declared_but_no_materializer_at_this_responder`, tell the user this responder has the slot reserved but no live connector (don't claim emem has no flood/water/etc. data — be precise). Topics not listed at all (e.g. real-time air quality, traffic) are genuinely out of scope for this protocol today.",
            "for_temporal_questions": "For 'last N years' questions, materializers return one fact at the latest available tslot. To get a series, call `emem_recall` repeatedly for past tslots only if the band's tempo is `slow`/`static` (which means one fact covers the period). For `fast`/`medium` tempo bands, history requires the responder to have already seeded past tslots — call `emem_trajectory` to enumerate what's there, do NOT assume historical lookback materializes on demand.",
        }),
        "agent_hint": {
            "request_field_name": "cell",
            "alias_accepted":     "cell64",
            "value_format":       "cell64 string (four base-1024 bigrams joined by '.')",
            "explanation":        "Field name in request bodies is `cell` (or `cell64` as serde alias). The string format is named cell64. Two different things: `cell` is the slot, cell64 is what goes in it — like a `mode: String` field where strings are UTF-8."
        },
        "next": [
            "POST /v1/recall  {\"cell\": \"<cell64>\", \"bands\": [...]}",
            "POST /v1/find_similar",
            "POST /v1/compare",
            "GET  /v1/cells/{cell64}/info",
            "GET  /v1/grid_info  — actual vs spec-target resolution",
        ],
    });
    // Surface alternatives only when the geocoder layer produced more
    // than one candidate; keep the response minimal on the common
    // unambiguous path so agents don't pay tokens for empty arrays.
    if !alternatives.is_empty() {
        if let Some(m) = body.as_object_mut() {
            m.insert("alternatives".into(), JsonValue::Array(alternatives));
            m.insert("disambiguation_hint".into(), json!(
                "Multiple matches for this name. The chosen result is in `cell64`/`place_label`/`centre`; \
                 ranked alternatives (by Nominatim importance) are in `alternatives`. \
                 If the chosen one isn't what the user meant, re-query with a more specific \
                 string (add country, region, or feature type) or call /v1/locate with \
                 lat/lng of the alternative you want."
            ));
        }
    }
    Ok(Json(body))
}

// ── Geocoder: layered ────────────────────────────────────────────────────
//
// Public Nominatim's usage policy is hard: 1 req/sec absolute, "systematic
// queries" forbidden. For an agent-native protocol that expects to be hit
// many times per agent per minute, that's a non-starter. Three layers in
// order:
//
// 1. **Embedded gazetteer** — a compiled-in table of well-known places
//    (capitals, peaks, regions). Zero network. Resolves the long tail of
//    questions agents actually ask.
// 2. **TTL cache** — second-call resolution lands in-memory for 24 h, so
//    a hot place hits Nominatim once, ever.
// 3. **Live Nominatim** — only as the last resort, with the operator's
//    `EMEM_PUBLIC_URL` / `EMEM_TLS_CONTACT` in the User-Agent so the OSM
//    operators can reach us if we exceed policy.
//
// Operators wanting unconditional self-host should set
// `EMEM_NOMINATIM_BASE` to their own instance; that's a one-line change
// in `nominatim_lookup` and the same layered cache applies.

/// Static gazetteer of places agents ask about most often. Source: public
/// authoritative coordinates (Wikipedia summit boxes, capital city
/// coordinates). Not exhaustive — a fallback for the obvious queries.
/// Format: (key, centre_lat, centre_lng, label).
const GAZETTEER: &[(&str, f64, f64, &str)] = &[
    // World capitals (subset).
    ("tokyo", 35.6764, 139.6500, "Tokyo, Japan"),
    ("london", 51.5074, -0.1278, "London, United Kingdom"),
    ("paris", 48.8566, 2.3522, "Paris, France"),
    ("new york", 40.7128, -74.0060, "New York City, USA"),
    ("new york city", 40.7128, -74.0060, "New York City, USA"),
    ("nyc", 40.7128, -74.0060, "New York City, USA"),
    ("delhi", 28.6139, 77.2090, "Delhi, India"),
    ("new delhi", 28.6139, 77.2090, "New Delhi, India"),
    ("mumbai", 19.0760, 72.8777, "Mumbai, India"),
    ("bangalore", 12.9716, 77.5946, "Bengaluru, India"),
    ("bengaluru", 12.9716, 77.5946, "Bengaluru, India"),
    ("chennai", 13.0827, 80.2707, "Chennai, India"),
    ("kolkata", 22.5726, 88.3639, "Kolkata, India"),
    ("beijing", 39.9042, 116.4074, "Beijing, China"),
    ("shanghai", 31.2304, 121.4737, "Shanghai, China"),
    ("seoul", 37.5665, 126.9780, "Seoul, South Korea"),
    ("singapore", 1.3521, 103.8198, "Singapore"),
    ("bangkok", 13.7563, 100.5018, "Bangkok, Thailand"),
    ("jakarta", -6.2088, 106.8456, "Jakarta, Indonesia"),
    ("sydney", -33.8688, 151.2093, "Sydney, Australia"),
    ("são paulo", -23.5505, -46.6333, "São Paulo, Brazil"),
    ("sao paulo", -23.5505, -46.6333, "São Paulo, Brazil"),
    (
        "rio de janeiro",
        -22.9068,
        -43.1729,
        "Rio de Janeiro, Brazil",
    ),
    (
        "buenos aires",
        -34.6037,
        -58.3816,
        "Buenos Aires, Argentina",
    ),
    ("mexico city", 19.4326, -99.1332, "Mexico City, Mexico"),
    ("lagos", 6.5244, 3.3792, "Lagos, Nigeria"),
    ("nairobi", -1.2864, 36.8172, "Nairobi, Kenya"),
    ("cairo", 30.0444, 31.2357, "Cairo, Egypt"),
    (
        "johannesburg",
        -26.2041,
        28.0473,
        "Johannesburg, South Africa",
    ),
    ("cape town", -33.9249, 18.4241, "Cape Town, South Africa"),
    ("istanbul", 41.0082, 28.9784, "Istanbul, Türkiye"),
    ("dubai", 25.2048, 55.2708, "Dubai, UAE"),
    ("moscow", 55.7558, 37.6173, "Moscow, Russia"),
    ("berlin", 52.5200, 13.4050, "Berlin, Germany"),
    ("madrid", 40.4168, -3.7038, "Madrid, Spain"),
    ("rome", 41.9028, 12.4964, "Rome, Italy"),
    ("amsterdam", 52.3676, 4.9041, "Amsterdam, Netherlands"),
    ("toronto", 43.6532, -79.3832, "Toronto, Canada"),
    ("vancouver", 49.2827, -123.1207, "Vancouver, Canada"),
    ("san francisco", 37.7749, -122.4194, "San Francisco, USA"),
    ("los angeles", 34.0522, -118.2437, "Los Angeles, USA"),
    ("seattle", 47.6062, -122.3321, "Seattle, USA"),
    ("chicago", 41.8781, -87.6298, "Chicago, USA"),
    ("reykjavík", 64.1466, -21.9426, "Reykjavík, Iceland"),
    ("reykjavik", 64.1466, -21.9426, "Reykjavík, Iceland"),
    // Iconic peaks & landmarks.
    ("mount fuji", 35.3606, 138.7274, "Mount Fuji, Japan"),
    ("mt fuji", 35.3606, 138.7274, "Mount Fuji, Japan"),
    ("mt. fuji", 35.3606, 138.7274, "Mount Fuji, Japan"),
    ("fuji", 35.3606, 138.7274, "Mount Fuji, Japan"),
    (
        "mount everest",
        27.9881,
        86.9250,
        "Mount Everest, Nepal/Tibet",
    ),
    ("mt everest", 27.9881, 86.9250, "Mount Everest, Nepal/Tibet"),
    (
        "mt. everest",
        27.9881,
        86.9250,
        "Mount Everest, Nepal/Tibet",
    ),
    ("everest", 27.9881, 86.9250, "Mount Everest, Nepal/Tibet"),
    ("k2", 35.8825, 76.5133, "K2, Karakoram"),
    (
        "kilimanjaro",
        -3.0674,
        37.3556,
        "Mount Kilimanjaro, Tanzania",
    ),
    ("denali", 63.0692, -151.0070, "Denali, Alaska"),
    (
        "mount kosciuszko",
        -36.4558,
        148.2640,
        "Mount Kosciuszko, Australia",
    ),
    ("aconcagua", -32.6532, -70.0109, "Aconcagua, Argentina"),
    (
        "matterhorn",
        45.9763,
        7.6586,
        "Matterhorn, Swiss/Italian Alps",
    ),
    ("mount blanc", 45.8326, 6.8652, "Mont Blanc, France/Italy"),
    ("mont blanc", 45.8326, 6.8652, "Mont Blanc, France/Italy"),
    // Iconic wide features (centroids — agents should fan out).
    ("grand canyon", 36.0544, -112.1401, "Grand Canyon, USA"),
    ("amazon", -3.4653, -62.2159, "Amazon Basin, Brazil"),
    ("sahara", 23.4162, 25.6628, "Sahara Desert"),
    ("antarctica", -82.8628, 35.0000, "Antarctica"),
    ("arctic", 80.0000, -0.0000, "Arctic Ocean"),
    (
        "great barrier reef",
        -18.2871,
        147.6992,
        "Great Barrier Reef, Australia",
    ),
];

/// Bounding boxes for places that span enough of Earth that a single
/// centroid-cell query would miss most of the data. Format:
/// (key, min_lat, max_lat, min_lng, max_lng). Agents asking about these
/// names should fan out a /v1/recall over cells inside the box, not
/// trust the centroid.
const WIDE_BBOXES: &[(&str, f64, f64, f64, f64)] = &[
    ("grand canyon", 35.95, 36.30, -113.00, -111.60),
    ("amazon", -10.00, 5.00, -75.00, -50.00),
    ("amazon basin", -10.00, 5.00, -75.00, -50.00),
    ("sahara", 12.00, 35.00, -17.00, 40.00),
    ("antarctica", -90.00, -60.00, -180.00, 180.00),
    ("arctic", 66.50, 90.00, -180.00, 180.00),
    ("great barrier reef", -24.00, -10.00, 142.00, 154.00),
    ("himalayas", 26.00, 36.00, 72.00, 97.00),
    ("alps", 43.00, 48.50, 5.00, 17.00),
    ("rockies", 30.00, 60.00, -123.00, -103.00),
    ("rocky mountains", 30.00, 60.00, -123.00, -103.00),
    ("andes", -55.00, 12.00, -82.00, -62.00),
    ("greenland", 59.00, 84.00, -73.00, -11.00),
    ("siberia", 50.00, 78.00, 60.00, 180.00),
];

/// Return up to `target_n` distinct cell64 strings sampled inside a
/// (min_lat, max_lat, min_lng, max_lng) box on a square grid. Used for
/// fan-out recall over wide features. The grid step is √target_n × √target_n,
/// floored so two adjacent samples are always on different cells.
///
/// Honest caveat: this is a *sample*, not coverage. A 64-sample fan over
/// the Sahara still leaves vast cells unread; agents looking for
/// completeness should use `query_region` against the bbox, not this.
fn sample_cells_in_bbox(bbox: (f64, f64, f64, f64), target_n: usize) -> Vec<String> {
    let (mn_la, mx_la, mn_ln, mx_ln) = bbox;
    let n_side = (target_n as f64).sqrt().floor().max(2.0) as usize;
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::with_capacity(n_side * n_side);
    for i in 0..n_side {
        for j in 0..n_side {
            // (i+0.5)/n centers each sample inside its sub-cell — avoids
            // the corner case where i=0 lands exactly on a boundary and
            // collides with a neighbour due to floating-point quantisation.
            let la = mn_la + (mx_la - mn_la) * (i as f64 + 0.5) / n_side as f64;
            let ln = mn_ln + (mx_ln - mn_ln) * (j as f64 + 0.5) / n_side as f64;
            let s = emem_codec::to_cell64(emem_codec::cell_from_latlng(la, ln));
            if seen.insert(s.clone()) {
                out.push(s);
                if out.len() >= target_n {
                    return out;
                }
            }
        }
    }
    out
}

fn wide_bbox_lookup(query: &str) -> Option<(f64, f64, f64, f64)> {
    let q = query.trim().to_ascii_lowercase();
    if q.is_empty() {
        return None;
    }
    for (key, mn_la, mx_la, mn_ln, mx_ln) in WIDE_BBOXES {
        if q == *key || q.starts_with(key) || q.contains(&format!(" {key} ")) {
            return Some((*mn_la, *mx_la, *mn_ln, *mx_ln));
        }
    }
    None
}

fn embedded_gazetteer_lookup(query: &str) -> Option<(f64, f64, String)> {
    let q = query.trim().to_ascii_lowercase();
    if q.is_empty() {
        return None;
    }
    // Exact match first (cheaper than a scan).
    for (key, lat, lng, label) in GAZETTEER {
        if q == *key {
            return Some((*lat, *lng, (*label).to_string()));
        }
    }
    // Substring tolerance: "tokyo, japan" still hits "tokyo".
    for (key, lat, lng, label) in GAZETTEER {
        if q.starts_with(key) || q.contains(&format!(" {key} ")) {
            return Some((*lat, *lng, (*label).to_string()));
        }
    }
    None
}

#[derive(Clone, Serialize, Deserialize)]
struct CachedPlace {
    lat: f64,
    lng: f64,
    label: String,
    /// Polygon bbox returned by Nominatim, when present. Stored so a
    /// cache hit can still feed `recall_polygon` — without this, the
    /// agent would resolve "Yellowstone" once with a polygon, then on
    /// every subsequent call get a single-cell fallback because the
    /// hot cache forgot the polygon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    polygon_bbox: Option<[f64; 4]>, // [min_lat, max_lat, min_lng, max_lng]
    /// Wall-clock seconds since epoch when this entry was inserted.
    /// Unix-time rather than Instant so the cache is durable across
    /// process restarts.
    inserted_unix_s: i64,
}

/// 30 d TTL — place-name → centroid is stable. Nominatim's caching
/// policy explicitly allows long retention. Override via
/// `EMEM_GEOCODER_TTL_SECS` for testing.
const NOMINATIM_CACHE_TTL_SECS_DEFAULT: i64 = 30 * 24 * 60 * 60;

fn nominatim_cache_ttl_secs() -> i64 {
    std::env::var("EMEM_GEOCODER_TTL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(NOMINATIM_CACHE_TTL_SECS_DEFAULT)
}

fn geocoder_db_path() -> std::path::PathBuf {
    let dir = std::env::var("EMEM_DATA").unwrap_or_else(|_| "/home/ubuntu/emem/var/emem".into());
    std::path::Path::new(&dir).join("geocoder.sled")
}

/// Sled-backed geocoder cache. Replaces the previous JSON-file
/// implementation: O(1) per-key writes (no full-file rewrite on every
/// put), zero-copy lookups, durable across restart, and the same
/// process-exclusive lock semantics as our main cache. Stored in its
/// own sled DB at `$EMEM_DATA/geocoder.sled` so it doesn't share a
/// lock with the fact cache (the two have different access patterns —
/// the geocoder is read-heavy, the fact cache is write-heavy).
fn geocoder_db() -> &'static sled::Tree {
    static T: std::sync::OnceLock<sled::Tree> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        // Open lazily on first cache access. Sled is forgiving: missing
        // dir is created, missing tree is created, corrupt segments are
        // recovered — we just propagate any unrecoverable error to a
        // panic on first call rather than silently swallowing data loss.
        let path = geocoder_db_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let db = sled::open(&path).expect("open geocoder sled DB");
        let tree = db
            .open_tree("emem.geocoder")
            .expect("open geocoder sled tree");
        // Leak the Db so the Tree handle stays alive for process lifetime.
        // Sled's Db drops close all trees; OnceLock retains only the Tree.
        // We intentionally Box::leak the Db to keep it open as long as the
        // process runs — this is a one-shot 8 KB leak amortised over the
        // process lifetime.
        let _: &'static sled::Db = Box::leak(Box::new(db));
        let n = tree.len();
        if n > 0 {
            tracing::info!(
                target: "emem::geocoder",
                geocoder_cache_warm_start = n,
                geocoder_cache_path = %path.display(),
                "geocoder_cache_warm_start"
            );
        }
        tree
    })
}

fn now_unix_s() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn nominatim_cache_get(query: &str) -> Option<(f64, f64, String, Option<[f64; 4]>)> {
    let q = query.trim().to_ascii_lowercase();
    let raw = geocoder_db().get(q.as_bytes()).ok()??;
    let entry: CachedPlace = serde_json::from_slice(&raw).ok()?;
    let age_s = now_unix_s().saturating_sub(entry.inserted_unix_s);
    if age_s > nominatim_cache_ttl_secs() {
        // Lazy expiry: drop the stale row so the next miss takes the
        // Nominatim path cleanly. No need to fsync — sled flushes on
        // its own cadence.
        let _ = geocoder_db().remove(q.as_bytes());
        return None;
    }
    Some((entry.lat, entry.lng, entry.label, entry.polygon_bbox))
}

fn nominatim_cache_put(
    query: &str,
    lat: f64,
    lng: f64,
    label: &str,
    polygon_bbox: Option<[f64; 4]>,
) {
    let q = query.trim().to_ascii_lowercase();
    let entry = CachedPlace {
        lat,
        lng,
        label: label.to_string(),
        polygon_bbox,
        inserted_unix_s: now_unix_s(),
    };
    let bytes = match serde_json::to_vec(&entry) {
        Ok(b) => b,
        Err(_) => return,
    };
    let _ = geocoder_db().insert(q.as_bytes(), bytes);
    // Sled flushes on its own background cadence (default ~500 ms).
    // We don't force-flush per put because the geocoder cache is
    // refillable from upstream — losing the last few ms of writes on
    // crash is fine, the cost of fsync-per-put isn't.
}

#[allow(dead_code)]
async fn nominatim_lookup(q: &str) -> Result<NominatimHit, String> {
    // Single-best-match wrapper around the multi-candidate fetch. Keeps
    // the callsite simple while the candidate variant is exposed via
    // `nominatim_lookup_candidates` for disambiguation use.
    let hits = nominatim_lookup_candidates(q, 1).await?;
    hits.into_iter()
        .next()
        .ok_or_else(|| "no results".to_string())
}

/// Fetch up to `limit` candidates from Nominatim. Used by `/v1/locate`
/// to return ranked alternatives when a place name is ambiguous
/// ("Springfield", "San José", "Bristol") so the agent can disambiguate
/// instead of silently accepting whichever match Nominatim ranked first.
async fn nominatim_lookup_candidates(q: &str, limit: usize) -> Result<Vec<NominatimHit>, String> {
    let base = std::env::var("EMEM_NOMINATIM_BASE")
        .unwrap_or_else(|_| "https://nominatim.openstreetmap.org".into());
    let base = base.trim_end_matches('/');
    let limit = limit.clamp(1, 10);
    let url = format!(
        "{base}/search?q={}&format=json&limit={limit}&addressdetails=0&extratags=0",
        urlencoding(q),
    );
    let body = nominatim_get(&url).await?;
    let v: JsonValue = serde_json::from_str(&body).map_err(|e| format!("nominatim json: {e}"))?;
    let arr = v.as_array().ok_or("nominatim returned non-array")?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let lat: f64 = match item["lat"].as_str().and_then(|s| s.parse().ok()) {
            Some(v) => v,
            None => continue,
        };
        let lng: f64 = match item["lon"].as_str().and_then(|s| s.parse().ok()) {
            Some(v) => v,
            None => continue,
        };
        let label = item["display_name"].as_str().unwrap_or("").to_string();
        let osm_type = item["osm_type"].as_str().unwrap_or("").to_string();
        let class_ = item["class"].as_str().unwrap_or("").to_string();
        let type_ = item["type"].as_str().unwrap_or("").to_string();
        let importance = item["importance"].as_f64().unwrap_or(0.0);
        // boundingbox: [south, north, west, east] as strings.
        let bbox = item["boundingbox"].as_array().and_then(|a| {
            if a.len() != 4 {
                return None;
            }
            let s: f64 = a[0].as_str()?.parse().ok()?;
            let n: f64 = a[1].as_str()?.parse().ok()?;
            let w: f64 = a[2].as_str()?.parse().ok()?;
            let e: f64 = a[3].as_str()?.parse().ok()?;
            Some((s, n, w, e))
        });
        out.push(NominatimHit {
            lat,
            lng,
            label,
            bbox,
            osm_type,
            class_,
            type_,
            importance,
        });
    }
    if out.is_empty() {
        return Err("no results".into());
    }
    Ok(out)
}

#[derive(Clone)]
struct NominatimHit {
    lat: f64,
    lng: f64,
    label: String,
    /// (min_lat, max_lat, min_lng, max_lng) when Nominatim provides a
    /// boundingbox (it almost always does).
    bbox: Option<(f64, f64, f64, f64)>,
    /// "node" / "way" / "relation" — feature kind in OSM.
    osm_type: String,
    /// OSM tag class (boundary, place, natural, leisure, …).
    class_: String,
    /// OSM tag type (administrative, peak, water, park, …).
    type_: String,
    /// Nominatim's relevance score in [0, 1]. Useful as a disambiguation
    /// signal when multiple candidates share a name.
    importance: f64,
}

/// User-agent for Nominatim requests. Nominatim's usage policy *requires* a
/// "valid HTTP Referer or User-Agent identifying the application" so anonymous
/// `reqwest/x.y.z` requests get blocked. We attach the operator's
/// `EMEM_PUBLIC_URL` (if set) so the upstream operator can contact them; if
/// not set we send a generic identifier with a `mailto:` derived from
/// `EMEM_TLS_CONTACT` so private deployments still identify themselves.
fn nominatim_user_agent() -> String {
    let base = concat!("emem-server/", env!("CARGO_PKG_VERSION"));
    let contact = std::env::var("EMEM_PUBLIC_URL")
        .ok()
        .map(|u| format!(" (+{})", u.trim().trim_end_matches('/')))
        .or_else(|| {
            std::env::var("EMEM_TLS_CONTACT")
                .ok()
                .map(|c| format!(" ({})", c.trim()))
        })
        .unwrap_or_default();
    format!("{base}{contact}")
}

async fn nominatim_get(url: &str) -> Result<String, String> {
    let cli = reqwest_client();
    // accept-language: the wildcard `*` tells Nominatim "return whatever
    // matches the query string itself" — it preserves the script the
    // agent asked in (Cyrillic in, Cyrillic out) instead of forcing
    // English. Without this header Nominatim defaults to the upstream
    // server locale, which can mangle non-Latin labels.
    let resp = cli
        .get(url)
        .header("user-agent", &nominatim_user_agent())
        .header("accept", "application/json")
        .header("accept-language", "*")
        .send()
        .await
        .map_err(|e| format!("nominatim https: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("nominatim status {}", resp.status()));
    }
    resp.text()
        .await
        .map_err(|e| format!("nominatim body: {e}"))
}

fn reqwest_client() -> reqwest::Client {
    static C: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(8))
            .build()
            .unwrap_or_default()
    })
    .clone()
}

fn urlencoding(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{:02X}", b),
        })
        .collect()
}

async fn get_cell_info(Path(cell64): Path<String>) -> Result<Json<JsonValue>, ApiError> {
    let info = emem_codec::latlng_from_cell64(&cell64).map_err(|e| {
        ApiError(
            StatusCode::BAD_REQUEST,
            ErrorBody {
                code: ErrorCode::InvalidCell,
                message: format!("{e}"),
            },
        )
    })?;
    let lat_span = info.bbox_deg.max_lat - info.bbox_deg.min_lat;
    let lng_span = info.bbox_deg.max_lng - info.bbox_deg.min_lng;
    let lat_m = lat_span * METERS_PER_DEGREE_LAT;
    let lng_m = lng_span * METERS_PER_DEGREE_LAT * info.lat_deg.to_radians().cos();
    Ok(Json(json!({
        "cell64": cell64,
        "centre": {"lat_deg": info.lat_deg, "lng_deg": info.lng_deg},
        "bbox_deg": {
            "min_lat": info.bbox_deg.min_lat, "max_lat": info.bbox_deg.max_lat,
            "min_lng": info.bbox_deg.min_lng, "max_lng": info.bbox_deg.max_lng,
        },
        "approx_size_m": {
            "lat": lat_m,
            "lng": lng_m.abs(),
        },
    })))
}

/// `GET /v1/cells/{cell64}.geojson` — cell as a GeoJSON Feature with
/// Polygon geometry (the cell's WGS-84 bbox as a closed 5-vertex ring) and
/// `properties` carrying centre lat/lng, bbox, neighbour cell64s, and
/// approximate ground size in metres. The "multimodal-lite" handoff:
/// agents that want to overlay emem facts on Mapbox / Leaflet / Deck.gl /
/// QGIS get a feed-ready GeoJSON without running their own GIS pipeline.
/// Build the cell-polygon GeoJSON Feature in-memory. Shared between
/// the REST handler [`get_cell_geojson`] and the MCP `emem_cell_geojson`
/// tool so both surfaces serialise an identical Feature.
fn build_cell_geojson(cell64: &str) -> Result<JsonValue, String> {
    let info = emem_codec::latlng_from_cell64(cell64).map_err(|e| format!("cell64 decode: {e}"))?;
    let s_lat = info.bbox_deg.min_lat;
    let n_lat = info.bbox_deg.max_lat;
    let w_lng = info.bbox_deg.min_lng;
    let e_lng = info.bbox_deg.max_lng;
    let lat_span = n_lat - s_lat;
    let lng_span = e_lng - w_lng;
    let lat_m = lat_span * METERS_PER_DEGREE_LAT;
    let lng_m = lng_span * METERS_PER_DEGREE_LAT * info.lat_deg.to_radians().cos();

    let mut neighbours: Vec<String> = Vec::with_capacity(8);
    let mut seen = std::collections::BTreeSet::new();
    seen.insert(cell64.to_string());
    for (sa, sb) in [
        (1.0, 0.0),
        (-1.0, 0.0),
        (0.0, 1.0),
        (0.0, -1.0),
        (1.0, 1.0),
        (1.0, -1.0),
        (-1.0, 1.0),
        (-1.0, -1.0),
    ] {
        let s = emem_codec::to_cell64(emem_codec::cell_from_latlng(
            info.lat_deg + sa * lat_span,
            info.lng_deg + sb * lng_span,
        ));
        if seen.insert(s.clone()) {
            neighbours.push(s);
        }
    }

    let ring = json!([
        [w_lng, s_lat],
        [e_lng, s_lat],
        [e_lng, n_lat],
        [w_lng, n_lat],
        [w_lng, s_lat],
    ]);

    Ok(json!({
        "type": "Feature",
        "geometry": {
            "type": "Polygon",
            "coordinates": [ring],
        },
        "properties": {
            "cell64":  cell64,
            "centre":  {"lat": info.lat_deg, "lng": info.lng_deg},
            "bbox":    {"min_lat": s_lat, "max_lat": n_lat, "min_lng": w_lng, "max_lng": e_lng},
            "approx_size_m": {"lat": lat_m, "lng": lng_m.abs()},
            "neighbours": neighbours,
            "schema":  "emem.cell_geojson.v1",
        },
    }))
}

async fn get_cell_geojson(Path(cell64_with_ext): Path<String>) -> Result<Response, ApiError> {
    // axum captures `cell64.geojson` as a single param; strip the suffix.
    let cell64 = cell64_with_ext
        .strip_suffix(".geojson")
        .unwrap_or(&cell64_with_ext);
    let feat = build_cell_geojson(cell64).map_err(|e| {
        ApiError(
            StatusCode::BAD_REQUEST,
            ErrorBody {
                code: ErrorCode::InvalidCell,
                message: e,
            },
        )
    })?;
    let body = serde_json::to_string(&feat).unwrap_or_else(|_| "{}".into());
    let resp = Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/geo+json; charset=utf-8")
        .header("cache-control", "public, max-age=86400, immutable")
        .body(body.into())
        .unwrap();
    Ok(resp)
}

/// `GET /v1/cells/{cell64}/recall.geojson` — facts at the cell as a
/// GeoJSON FeatureCollection. Each fact becomes a Feature with the
/// cell's polygon geometry and properties carrying band, value, unit,
/// confidence, derivation.fn_key, signed_at, fact_cid (truncated cid64).
/// Multimodal-lite: pipe directly into Mapbox/Leaflet/Deck.gl/QGIS.
async fn get_cell_recall_geojson(
    State(s): State<AppState>,
    Path(cell64): Path<String>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Response, ApiError> {
    let info = emem_codec::latlng_from_cell64(&cell64).map_err(|e| {
        ApiError(
            StatusCode::BAD_REQUEST,
            ErrorBody {
                code: ErrorCode::InvalidCell,
                message: format!("cell64 decode: {e}"),
            },
        )
    })?;
    let s_lat = info.bbox_deg.min_lat;
    let n_lat = info.bbox_deg.max_lat;
    let w_lng = info.bbox_deg.min_lng;
    let e_lng = info.bbox_deg.max_lng;
    let ring = json!([
        [w_lng, s_lat],
        [e_lng, s_lat],
        [e_lng, n_lat],
        [w_lng, n_lat],
        [w_lng, s_lat],
    ]);
    let band_filter: Option<Vec<String>> = q.get("bands").map(|v| {
        v.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    });
    let tslot: Option<u64> = q.get("tslot").and_then(|v| v.parse().ok());
    let req = RecallReq {
        cell: cell64.clone(),
        bands: band_filter,
        tslot,
    };
    let resp = recall(&req, &s).await.map_err(|e| {
        ApiError(
            StatusCode::BAD_GATEWAY,
            ErrorBody {
                code: ErrorCode::Internal,
                message: format!("recall failed: {e}"),
            },
        )
    })?;
    let mut features: Vec<JsonValue> = Vec::new();
    let resp_json = serde_json::to_value(&resp).unwrap_or(json!({}));
    let fact_cids: Vec<String> = resp_json
        .get("receipt")
        .and_then(|r| r.get("fact_cids"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    if let Some(facts) = resp_json.get("facts").and_then(|v| v.as_array()) {
        for (i, f) in facts.iter().enumerate() {
            let cid = fact_cids.get(i).cloned().unwrap_or_default();
            let mut props = serde_json::Map::new();
            props.insert("cell64".into(), json!(cell64));
            props.insert(
                "kind".into(),
                f.get("kind").cloned().unwrap_or(json!("primary")),
            );
            for k in &[
                "band",
                "tslot",
                "value",
                "unit",
                "confidence",
                "signed_at",
                "signer",
                "schema_cid",
            ] {
                if let Some(v) = f.get(*k) {
                    props.insert((*k).into(), v.clone());
                }
            }
            if let Some(d) = f.get("derivation") {
                props.insert("derivation".into(), d.clone());
            }
            if !cid.is_empty() {
                props.insert("fact_cid".into(), json!(cid));
            }
            features.push(json!({
                "type": "Feature",
                "geometry": {"type": "Polygon", "coordinates": [ring.clone()]},
                "properties": props,
            }));
        }
    }
    let receipt = resp_json.get("receipt").cloned().unwrap_or(json!(null));
    let coll = json!({
        "type": "FeatureCollection",
        "features": features,
        "metadata": {
            "cell64": cell64,
            "centre": {"lat": info.lat_deg, "lng": info.lng_deg},
            "bbox":   {"min_lat": s_lat, "max_lat": n_lat, "min_lng": w_lng, "max_lng": e_lng},
            "schema": "emem.cell_recall_geojson.v1",
            "receipt": receipt,
        },
    });
    let body = serde_json::to_string(&coll).unwrap_or_else(|_| "{}".into());
    let resp = Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/geo+json; charset=utf-8")
        .header("cache-control", "public, max-age=60")
        .body(body.into())
        .unwrap();
    Ok(resp)
}

// ── /v1/contributors (CoIL — Contributor-of-Intelligence Layer) ─────────

async fn list_contributors(State(s): State<AppState>) -> Json<JsonValue> {
    let limit = 50;
    let mut rows: Vec<JsonValue> = Vec::new();
    let mut total: u64 = 0;
    if let Some(reg) = s.storage_attesters() {
        if let Ok(top) = reg.top(limit) {
            for st in top {
                rows.push(stat_to_json(&st));
            }
        }
        if let Ok(c) = reg.count() {
            total = c;
        }
    }
    Json(json!({
        "schema": "emem.contributors.v1",
        "count": rows.len(),
        "total_known": total,
        "scoring": "score = citations + 8·ln(1+facts) + 4·ln(1+attestations)",
        "leaderboard": rows,
    }))
}

async fn get_contributor(
    State(s): State<AppState>,
    Path(pubkey_b32): Path<String>,
) -> Result<Json<JsonValue>, ApiError> {
    let Some(reg) = s.storage_attesters() else {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            ErrorBody {
                code: ErrorCode::Internal,
                message: "attester registry unavailable on this responder".into(),
            },
        ));
    };
    match reg.get(&pubkey_b32) {
        Ok(Some(stat)) => Ok(Json(stat_to_json(&stat))),
        Ok(None) => Err(ApiError(
            StatusCode::NOT_FOUND,
            ErrorBody {
                code: ErrorCode::Internal,
                message: format!("no contributor record for {pubkey_b32}"),
            },
        )),
        Err(e) => Err(ApiError(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorBody {
                code: ErrorCode::Internal,
                message: format!("attester lookup failed: {e}"),
            },
        )),
    }
}

fn stat_to_json(s: &emem_storage::AttesterStats) -> JsonValue {
    json!({
        "pubkey_b32":        s.pubkey_b32,
        "attestations":      s.attestations,
        "facts":             s.facts,
        "citations":         s.citations,
        "unique_cells":      s.unique_cells,
        "first_seen_unix_s": s.first_seen_unix_s,
        "last_seen_unix_s":  s.last_seen_unix_s,
        "last_cited_unix_s": s.last_cited_unix_s,
        "score":             s.score(),
    })
}

// ── /metrics (Prometheus text format) ───────────────────────────────────

use std::sync::atomic::{AtomicU64, Ordering};

static REQ_TOTAL: AtomicU64 = AtomicU64::new(0);
static RATE_LIMITED_TOTAL: AtomicU64 = AtomicU64::new(0);
static ATTEST_TOTAL: AtomicU64 = AtomicU64::new(0);
static ATTEST_FAIL_TOTAL: AtomicU64 = AtomicU64::new(0);
static RECALL_TOTAL: AtomicU64 = AtomicU64::new(0);
static MCP_TOTAL: AtomicU64 = AtomicU64::new(0);
static START_INSTANT: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

// ── Agent stats aggregator ───────────────────────────────────────────────
//
// Materialises the structured access log into in-memory counters so we can
// answer "who's using us, what tools, how fast" without grepping journald.
// Fields are atomic / Mutex<small map>; the maps stay tiny because the
// keyspace is small (≤ ~14 agent families, 8 MCP tools, 12 latency buckets).

/// Latency-bucket boundaries in milliseconds (cumulative-LE histogram).
/// Twelve log-spaced buckets cover sub-1 ms hot-cache reads through 5 s
/// worst-case S3 range fetches, which spans ≈3.5 orders of magnitude.
const LATENCY_BUCKETS_MS: [f64; 12] = [
    1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0,
];

#[derive(Default)]
struct FamilyCounters {
    requests: u64,
    errors: u64,
    last_seen_unix_s: u64,
}

#[derive(Default)]
struct ToolCounters {
    calls: u64,
    errors: u64,
    total_dur_ms: f64,
}

struct AgentStatsState {
    by_family: std::sync::Mutex<std::collections::HashMap<&'static str, FamilyCounters>>,
    by_tool: std::sync::Mutex<std::collections::HashMap<String, ToolCounters>>,
    status_2xx: AtomicU64,
    status_3xx: AtomicU64,
    status_4xx: AtomicU64,
    status_5xx: AtomicU64,
    /// Cumulative-LE histogram. `latency[i]` counts requests with
    /// `duration_ms <= LATENCY_BUCKETS_MS[i]`. Last bucket is +Inf.
    latency: [AtomicU64; 13],
}

impl AgentStatsState {
    fn new() -> Self {
        Self {
            by_family: Default::default(),
            by_tool: Default::default(),
            status_2xx: AtomicU64::new(0),
            status_3xx: AtomicU64::new(0),
            status_4xx: AtomicU64::new(0),
            status_5xx: AtomicU64::new(0),
            latency: std::array::from_fn(|_| AtomicU64::new(0)),
        }
    }
}

static AGENT_STATS: std::sync::OnceLock<AgentStatsState> = std::sync::OnceLock::new();

fn agent_stats() -> &'static AgentStatsState {
    AGENT_STATS.get_or_init(AgentStatsState::new)
}

/// Serializable mirror of `AgentStatsState`. Survives restart by being
/// written to a sled tree every `AGENT_STATS_FLUSH_SECS` seconds and
/// loaded once at boot via `agent_stats_init_persistence`.
#[derive(Default, Serialize, Deserialize)]
struct AgentStatsSnapshot {
    status_2xx: u64,
    status_3xx: u64,
    status_4xx: u64,
    status_5xx: u64,
    latency: [u64; 13],
    by_family: Vec<(String, u64, u64, u64)>, // (family, requests, errors, last_seen_unix_s)
    by_tool: Vec<(String, u64, u64, f64)>,   // (tool, calls, errors, total_dur_ms)
}

const AGENT_STATS_TREE: &str = "emem.agent_stats";
const AGENT_STATS_KEY: &[u8] = b"snapshot";
const AGENT_STATS_FLUSH_SECS: u64 = 60;

fn snapshot_agent_stats() -> AgentStatsSnapshot {
    let st = agent_stats();
    let mut snap = AgentStatsSnapshot {
        status_2xx: st.status_2xx.load(Ordering::Relaxed),
        status_3xx: st.status_3xx.load(Ordering::Relaxed),
        status_4xx: st.status_4xx.load(Ordering::Relaxed),
        status_5xx: st.status_5xx.load(Ordering::Relaxed),
        latency: std::array::from_fn(|i| st.latency[i].load(Ordering::Relaxed)),
        by_family: vec![],
        by_tool: vec![],
    };
    if let Ok(m) = st.by_family.lock() {
        snap.by_family = m
            .iter()
            .map(|(k, c)| ((*k).to_string(), c.requests, c.errors, c.last_seen_unix_s))
            .collect();
    }
    if let Ok(m) = st.by_tool.lock() {
        snap.by_tool = m
            .iter()
            .map(|(k, c)| (k.clone(), c.calls, c.errors, c.total_dur_ms))
            .collect();
    }
    snap
}

fn restore_agent_stats(snap: AgentStatsSnapshot) {
    let st = agent_stats();
    st.status_2xx.store(snap.status_2xx, Ordering::Relaxed);
    st.status_3xx.store(snap.status_3xx, Ordering::Relaxed);
    st.status_4xx.store(snap.status_4xx, Ordering::Relaxed);
    st.status_5xx.store(snap.status_5xx, Ordering::Relaxed);
    for (i, v) in snap.latency.iter().enumerate() {
        st.latency[i].store(*v, Ordering::Relaxed);
    }
    if let Ok(mut m) = st.by_tool.lock() {
        for (k, c, e, d) in snap.by_tool {
            let entry = m.entry(k).or_default();
            entry.calls = c;
            entry.errors = e;
            entry.total_dur_ms = d;
        }
    }
    // by_family keys are &'static str, so we can only restore those that
    // map to a known family literal. Unknown families are dropped on
    // purpose — they may be from a binary that registered different
    // family names, and resurrecting them would mis-key the next access.
    if let Ok(mut m) = st.by_family.lock() {
        for (k, requests, errors, last_seen_unix_s) in snap.by_family {
            if let Some(static_key) = known_family_static(&k) {
                let entry = m.entry(static_key).or_default();
                entry.requests = requests;
                entry.errors = errors;
                entry.last_seen_unix_s = last_seen_unix_s;
            }
        }
    }
}

/// Map a runtime family string back to the `&'static str` `classify_agent`
/// returns. Used by the snapshot restorer; if `classify_agent` ever
/// returns a new family, add it here too.
fn known_family_static(s: &str) -> Option<&'static str> {
    const KNOWN: &[&str] = &[
        "claude-code",
        "claude",
        "cursor",
        "cline",
        "openai",
        "perplexity",
        "anthropic",
        "langchain",
        "llamaindex",
        "python",
        "cli",
        "browser",
        "anonymous",
        "other",
    ];
    KNOWN.iter().copied().find(|k| *k == s)
}

/// Initialise sled-backed persistence for `agent_stats`: load the last
/// snapshot if one exists, then spawn a periodic flush task. Idempotent
/// — calling twice is a no-op (the OnceLock blocks re-init).
fn agent_stats_init_persistence(db: Arc<sled::Db>) {
    static INIT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    if INIT.set(()).is_err() {
        return;
    }

    // Load.
    if let Ok(tree) = db.open_tree(AGENT_STATS_TREE) {
        if let Ok(Some(bytes)) = tree.get(AGENT_STATS_KEY) {
            let mut buf: &[u8] = bytes.as_ref();
            if let Ok(snap) = ciborium::de::from_reader::<AgentStatsSnapshot, _>(&mut buf) {
                restore_agent_stats(snap);
                tracing::info!(target: "emem::agent_stats", "agent_stats_loaded_from_sled");
            }
        }
    }

    // Flush task.
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(AGENT_STATS_FLUSH_SECS));
        // First tick fires immediately; skip it so we don't write empty
        // counters over a fresh restore.
        tick.tick().await;
        loop {
            tick.tick().await;
            let snap = snapshot_agent_stats();
            let mut buf = Vec::with_capacity(1024);
            if ciborium::ser::into_writer(&snap, &mut buf).is_err() {
                continue;
            }
            if let Ok(tree) = db.open_tree(AGENT_STATS_TREE) {
                let _ = tree.insert(AGENT_STATS_KEY, buf);
                let _ = tree.flush_async().await;
            }
        }
    });
}

fn record_request(family: &'static str, status: u16, dur_ms: f64) {
    let st = agent_stats();
    match status / 100 {
        2 => {
            st.status_2xx.fetch_add(1, Ordering::Relaxed);
        }
        3 => {
            st.status_3xx.fetch_add(1, Ordering::Relaxed);
        }
        4 => {
            st.status_4xx.fetch_add(1, Ordering::Relaxed);
        }
        5 => {
            st.status_5xx.fetch_add(1, Ordering::Relaxed);
        }
        _ => {}
    }
    let bucket = LATENCY_BUCKETS_MS
        .iter()
        .position(|b| dur_ms <= *b)
        .unwrap_or(12);
    st.latency[bucket].fetch_add(1, Ordering::Relaxed);
    if let Ok(mut map) = st.by_family.lock() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let e = map.entry(family).or_default();
        e.requests += 1;
        if status >= 400 {
            e.errors += 1;
        }
        e.last_seen_unix_s = now;
    }
}

fn record_mcp_tool(tool: &str, ok: bool, dur_ms: f64) {
    if tool.is_empty() {
        return;
    }
    if let Ok(mut map) = agent_stats().by_tool.lock() {
        let e = map.entry(tool.to_string()).or_default();
        e.calls += 1;
        if !ok {
            e.errors += 1;
        }
        e.total_dur_ms += dur_ms;
    }
}

/// Approximate p-th percentile from the cumulative-LE histogram by linear
/// interpolation within the bucket that crosses the target rank. Returns
/// the bucket upper bound when interpolation isn't possible. Honest
/// approximation — not for SLA enforcement.
fn latency_percentile(p: f64) -> Option<f64> {
    let st = agent_stats();
    let counts: Vec<u64> = st
        .latency
        .iter()
        .map(|a| a.load(Ordering::Relaxed))
        .collect();
    let total: u64 = counts.iter().sum();
    if total == 0 {
        return None;
    }
    let target = ((total as f64) * p).ceil() as u64;
    let mut cum = 0u64;
    for (i, c) in counts.iter().enumerate() {
        cum += *c;
        if cum >= target {
            return Some(if i < LATENCY_BUCKETS_MS.len() {
                LATENCY_BUCKETS_MS[i]
            } else {
                LATENCY_BUCKETS_MS[LATENCY_BUCKETS_MS.len() - 1] * 2.0
            });
        }
    }
    None
}

// ── Temporal routing middleware ─────────────────────────────────────────
//
// Given a query for "what's the state at cell C at time τ", which band
// should the agent ask for? The answer depends on:
//   1. How fast the underlying phenomenon changes (the band's tempo class).
//   2. How fresh the latest attestation is for this cell+band.
//   3. The temporal distance between τ and that latest attestation.
//
// The PDE classes mirror the agri TDM (`/home/ubuntu/agri/training/tdm.py`):
//   - Static (DEM, soil-class, Köppen): identity, Q = 1 forever once
//     attested.
//   - Slow / AR-1 (annual embeddings, land cover): linear-with-clamped-
//     decay over the band's natural year.
//   - Medium / heat (monthly NDVI composites, surface water occurrence):
//     ∂u/∂t = D∇²u → fundamental Gaussian decay kernel
//     Q(τ, t_obs) = exp(-((τ - t_obs)/σ)²) with σ ≈ slot_duration.
//   - Fast / wave + seasonal (raw S2 NDVI, daily SAR): one full Sentinel-2
//     revisit cycle (~5 d) repeats; Q = max(0, 0.5 + 0.5·cos(2π·Δt/T)).
//   - Ultra-fast / advection (hourly weather, vehicle counts): linear
//     decay over a few hours, Q = max(0, 1 - Δt/horizon).
//
// All four kernels live in pure Rust, deterministic, no ML model. The
// router is mathematics, not heuristic.

#[derive(Deserialize, Debug)]
struct TemporalRouteReq {
    /// cell64 string (alias `cell64` accepted) — optional, used to
    /// look up cell-local attestation freshness when present.
    #[serde(default, alias = "cell64", skip_serializing_if = "Option::is_none")]
    cell: Option<String>,
    /// ISO-8601 UTC of the query time. If omitted, "now".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    query_time: Option<String>,
    /// Optional intent string (e.g., "monitor flood risk this week").
    /// Routes the band families that match the intent up the ranking.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    intent: Option<String>,
    /// Optional shortlist of bands to score. If omitted, scores every
    /// band declared in the active manifest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bands: Option<Vec<String>>,
    /// Limit on returned candidates. Default 8, max 64.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    limit: Option<usize>,
}

/// Quality kernel families. Maps directly to the PDE class associated
/// with each Tempo bucket. Kept in this file because the math is the
/// protocol-level contract — the band registry just declares the
/// tempo, and this router decides the kernel.
fn quality_kernel(tempo: emem_core::tslot::Tempo, dt_s: f64) -> (f64, &'static str, &'static str) {
    use emem_core::tslot::Tempo;
    let dt = dt_s.abs();
    match tempo {
        // Static bands never decay; one good attestation is forever.
        Tempo::Static => (
            1.0,
            "identity",
            "Q = 1 (static phenomenon, no temporal decay)",
        ),
        // Slow (annual): linear decay over slot_seconds, then clamped.
        Tempo::Slow => {
            let horizon = Tempo::Slow.slot_seconds() as f64;
            let q = (1.0 - dt / horizon).max(0.0);
            (
                q,
                "linear_ar1",
                "Q = max(0, 1 - Δt/T_slot); AR-1 process step",
            )
        }
        // Medium (monthly): heat-equation Gaussian. σ = slot duration so
        // ~38% of the score remains at exactly one slot's worth of lag.
        Tempo::Medium => {
            let sigma = Tempo::Medium.slot_seconds() as f64;
            let q = (-((dt / sigma).powi(2))).exp();
            (
                q,
                "heat_gaussian",
                "Q = exp(-(Δt/σ)²); fundamental solution of ∂u/∂t = D∇²u with σ = slot duration",
            )
        }
        // Fast (daily): wave + seasonal. Sentinel-2 5-day revisit ≈
        // T_seasonal here. Half-cosine gives exactly 1.0 at Δt=0,
        // 0.5 at Δt=T/4, 0 at Δt=T/2.
        Tempo::Fast => {
            let period = (Tempo::Fast.slot_seconds() as f64) * 5.0; // ≈ S2 revisit
            let phase = 2.0 * std::f64::consts::PI * dt / period;
            let q = (0.5 + 0.5 * phase.cos()).clamp(0.0, 1.0);
            // Hard cutoff once we're past one full period.
            let q = if dt > period { 0.0 } else { q };
            (
                q,
                "wave_seasonal",
                "Q = max(0, 0.5 + 0.5·cos(2π·Δt/T)); ∂²u/∂t² = c²∇²u with T ≈ Sentinel-2 revisit",
            )
        }
        // Ultra-fast (hourly): advection. After ~6 h, the value is moot.
        Tempo::UltraFast => {
            let horizon = Tempo::UltraFast.slot_seconds() as f64 * 6.0;
            let q = (1.0 - dt / horizon).max(0.0);
            (
                q,
                "advection_linear",
                "Q = max(0, 1 - Δt/horizon); ∂u/∂t + v·∇u = 0 with horizon ≈ 6 slots",
            )
        }
    }
}

/// Score a band given the query time, the band's last-known
/// observation time at the cell (or None if we have nothing), and
/// the band's tempo class. Returns (score, kernel, derivation).
fn score_band(
    tempo: emem_core::tslot::Tempo,
    query_unix_s: i64,
    last_obs_unix_s: Option<i64>,
) -> (f64, &'static str, String) {
    let dt_s: f64 = match last_obs_unix_s {
        Some(t) => (query_unix_s - t).abs() as f64,
        None => {
            // No observation → the kernel has no anchor. We score by
            // the *opportunity*: a static band still rates 1 because a
            // single future attestation will satisfy any query; a
            // fast band rates 0.1 because by the time we attest, the
            // value is already stale.
            use emem_core::tslot::Tempo;
            let q = match tempo {
                Tempo::Static => 1.0,
                Tempo::Slow => 0.5,
                Tempo::Medium => 0.3,
                Tempo::Fast => 0.1,
                Tempo::UltraFast => 0.05,
            };
            return (
                q,
                "no_observation",
                format!(
                    "Q = {q:.3}; floor score for {:?} when no attestation exists yet",
                    tempo
                ),
            );
        }
    };
    let (q, kernel, deriv) = quality_kernel(tempo, dt_s);
    (q, kernel, format!("{deriv}; Δt = {dt_s:.0} s"))
}

/// Adjust a score by intent affinity: bands whose families match the
/// intent's keyword get a small multiplicative boost. Pure heuristic
/// layer above the math, surfaced separately so an agent can ignore
/// it. The math score is the protocol contract; intent affinity is
/// editorial.
fn intent_affinity(band_key: &str, family: &str, intent: &str) -> f64 {
    let intent_lc = intent.to_ascii_lowercase();
    let bf = format!("{band_key} {family}").to_ascii_lowercase();
    let mut score = 1.0;
    let pairs: &[(&[&str], &[&str], f64)] = &[
        (
            &["flood", "water", "wet", "river"],
            &["surface_water", "ocean_chl", "water"],
            1.5,
        ),
        (
            &["forest", "deforest", "tree", "logging"],
            &["forest_change", "mangrove", "vegetation"],
            1.5,
        ),
        (
            &["crop", "farm", "agri", "harvest", "yield"],
            &["indices", "phenology", "ndvi", "vegetation"],
            1.5,
        ),
        (
            &["urban", "city", "population", "human"],
            &["nightlights", "ghsl", "population", "human"],
            1.5,
        ),
        (
            &["climate", "weather", "temperature", "rainfall"],
            &["climate", "terraclimate", "koppen"],
            1.5,
        ),
        (
            &["terrain", "elevation", "mountain", "depth", "bathymetry"],
            &["dem", "cop_dem", "topobathy", "terrain"],
            1.5,
        ),
        (
            &["radar", "all-weather", "cloud", "night"],
            &["sentinel1"],
            1.4,
        ),
        (
            &["foundation", "embedding", "latent", "general"],
            &["geotessera", "foundation"],
            1.3,
        ),
    ];
    for (kws, fams, boost) in pairs {
        if kws.iter().any(|k| intent_lc.contains(k)) && fams.iter().any(|f| bf.contains(f)) {
            score *= boost;
        }
    }
    score
}

async fn temporal_route_inner(
    State(s): State<AppState>,
    req: TemporalRouteReq,
) -> Result<Json<JsonValue>, ApiError> {
    let limit = req.limit.unwrap_or(8).min(64);
    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let query_unix: i64 = match req.query_time.as_deref() {
        Some(qt) => parse_iso8601_unix(qt).unwrap_or(now_unix),
        None => now_unix,
    };

    let registry = &*emem_core::bands::DEFAULT;
    let candidates: Vec<&emem_core::bands::Band> = match &req.bands {
        Some(want) => {
            let want_set: std::collections::HashSet<&str> =
                want.iter().map(|s| s.as_str()).collect();
            registry
                .bands
                .iter()
                .filter(|b| {
                    want_set.contains(b.key.as_str())
                        || want.iter().any(|w| w.starts_with(&format!("{}.", b.key)))
                })
                .collect()
        }
        None => registry.bands.iter().collect(),
    };

    // Look up cell-local last-observation timestamps from the storage
    // layer. We don't have a per-band scan API at this resolution, so
    // we use scan_cell to get every fact, group by band-prefix, and
    // pick the latest signed_at. Cheap because cells hold ≤ tens of
    // facts in practice.
    let last_obs_by_band: std::collections::HashMap<String, i64> = match req.cell.as_deref() {
        Some(cell) => {
            let pairs = s.storage.scan_cell(cell, None).await.unwrap_or_default();
            let cids: Vec<emem_fact::FactCid> = pairs.into_iter().map(|(_k, c)| c).collect();
            let mut map = std::collections::HashMap::new();
            if !cids.is_empty() {
                if let Ok(facts) = s.storage.get_facts_many(&cids).await {
                    for f in facts.into_iter().flatten() {
                        let (band, signed_at) = match f {
                            Fact::Primary(p) => (p.band, p.signed_at),
                            Fact::Absence(n) => (n.band, n.signed_at),
                            Fact::Derivative(_) => continue,
                        };
                        if let Some(t) = parse_iso8601_unix(&signed_at) {
                            let prefix = band.split('.').next().unwrap_or(&band).to_string();
                            let entry = map.entry(prefix).or_insert(t);
                            if t > *entry {
                                *entry = t;
                            }
                        }
                    }
                }
            }
            map
        }
        None => Default::default(),
    };

    let intent = req.intent.as_deref().unwrap_or("");
    let mut scored: Vec<(f64, JsonValue, bool)> = Vec::with_capacity(candidates.len());
    for band in candidates {
        let last_obs = last_obs_by_band.get(&band.key).copied();
        let (q_math, kernel, derivation) = score_band(band.tempo, query_unix, last_obs);
        let family_str = format!("{:?}", band.family).to_ascii_lowercase();
        let affinity = if intent.is_empty() {
            1.0
        } else {
            intent_affinity(&band.key, &family_str, intent)
        };
        let intent_matched = affinity > 1.0;
        let q_total = (q_math * affinity).min(1.5);
        scored.push((
            q_total,
            json!({
                "band":             band.key,
                "family":           family_str,
                "tempo":            format!("{:?}", band.tempo).to_ascii_lowercase(),
                "score":            (q_total * 1000.0).round() / 1000.0,
                "score_math":       (q_math * 1000.0).round() / 1000.0,
                "intent_affinity":  (affinity * 1000.0).round() / 1000.0,
                "intent_matched":   intent_matched,
                "kernel":           kernel,
                "derivation":       derivation,
                "last_obs_unix_s":  last_obs,
                "last_obs_age_s":   last_obs.map(|t| (query_unix - t).abs()),
            }),
            intent_matched,
        ));
    }
    // Two ranked lists:
    //   - cite_now:        highest-Q-now (static and recently-attested
    //                      bands dominate; agent gets a verifiable
    //                      answer immediately).
    //   - fetch_for_intent: intent-matched bands ranked by their
    //                      kernel score; agent should /v1/recall these
    //                      to trigger materialization.
    let mut cite_now = scored.clone();
    cite_now.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let cite_now_json: Vec<JsonValue> = cite_now
        .into_iter()
        .take(limit)
        .map(|(_, v, _)| v)
        .collect();

    let mut fetch: Vec<(f64, JsonValue)> = scored
        .into_iter()
        .filter(|(_, _, matched)| *matched)
        .map(|(s, j, _)| (s, j))
        .collect();
    fetch.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let fetch_json: Vec<JsonValue> = fetch.into_iter().take(limit).map(|(_, v)| v).collect();
    let candidates_json = cite_now_json;

    Ok(Json(json!({
        "schema":      "emem.temporal_route.v1",
        "query_time":  iso8601_utc(query_unix as u64),
        "cell":        req.cell,
        "intent":      req.intent,
        "cite_now":    candidates_json,
        "fetch_for_intent": fetch_json,
        "math_note": {
            "kernels": {
                "static":           "Q = 1 (no decay)",
                "slow_ar1":         "Q = max(0, 1 - Δt/T_slot); annual cadence; T_slot = 1 year",
                "heat_gaussian":    "Q = exp(-(Δt/σ)²); ∂u/∂t = D∇²u; σ = monthly slot",
                "wave_seasonal":    "Q = max(0, 0.5 + 0.5·cos(2π·Δt/T)); ∂²u/∂t² = c²∇²u; T ≈ Sentinel-2 revisit",
                "advection_linear": "Q = max(0, 1 - Δt/horizon); ∂u/∂t + v·∇u = 0; horizon ≈ 6 slots",
            },
            "reference": "Inspired by /home/ubuntu/agri/training/tdm.py (Temporal Dynamics Module): physics-informed PDE operators per band class. ReJEPA / V-JEPA (arxiv.org/abs/2504.03169, arxiv.org/abs/2301.08243) provide the embedding-prediction extension once we add a learned predictor for missing (cell, time) pairs.",
            "intent_affinity_disclaimer": "intent_affinity is a heuristic family-match multiplier, NOT part of the PDE math. Strip it (use score_math) for protocol-level reasoning.",
        }
    })))
}

async fn post_temporal_route(
    State(s): State<AppState>,
    Json(req): Json<TemporalRouteReq>,
) -> Result<Json<JsonValue>, ApiError> {
    temporal_route_inner(State(s), req).await
}

async fn get_temporal_route(
    State(s): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Json<JsonValue>, ApiError> {
    let req = TemporalRouteReq {
        cell: q.get("cell").or_else(|| q.get("cell64")).cloned(),
        query_time: q.get("query_time").cloned(),
        intent: q.get("intent").cloned(),
        bands: q
            .get("bands")
            .map(|s| s.split(',').map(|x| x.trim().to_string()).collect()),
        limit: q.get("limit").and_then(|s| s.parse().ok()),
    };
    temporal_route_inner(State(s), req).await
}

/// Best-effort ISO-8601 UTC parser. Accepts "Z" or "+00:00". Returns
/// None on any parse failure — callers fall back to "now".
fn parse_iso8601_unix(s: &str) -> Option<i64> {
    // Forms: "2026-04-27T01:23:45Z", "2026-04-27T01:23:45+00:00",
    //        "2026-04-27" (day boundary).
    let s = s.trim();
    let (date_part, time_part) = if let Some((d, t)) = s.split_once('T') {
        (d, t)
    } else {
        (s, "00:00:00Z")
    };
    let mut date_iter = date_part.splitn(3, '-');
    let y: i64 = date_iter.next()?.parse().ok()?;
    let m: i64 = date_iter.next()?.parse().ok()?;
    let d: i64 = date_iter.next()?.parse().ok()?;
    let time_clean = time_part.trim_end_matches('Z').trim_end_matches("+00:00");
    let mut time_iter = time_clean.splitn(3, ':');
    let h: i64 = time_iter.next()?.parse().ok()?;
    let mi: i64 = time_iter.next()?.parse().ok()?;
    let se: i64 = time_iter
        .next()
        .and_then(|s| s.split('.').next())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    // Days since civil 1970-01-01 (Howard Hinnant's date algorithm).
    let yy = if m <= 2 { y - 1 } else { y };
    let mm = if m <= 2 { m + 12 } else { m };
    let era = if yy >= 0 { yy } else { yy - 399 } / 400;
    let yoe = yy - era * 400;
    let doy = (153 * (mm - 3) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days * 86400 + h * 3600 + mi * 60 + se)
}

// ── Agent reviews loop ──────────────────────────────────────────────────
//
// AI agents post structured reviews tied to specific things they used:
// a fact_cid (was the value useful?), a cell64 (does this region have
// the data I needed?), a request_id (did this primitive call answer
// my question?), a band, an endpoint, or a session.
//
// Reviews are persisted in a dedicated sled tree as content-addressed
// blobs. Their `review_cid = blake3(canonical_cbor(payload))[..16]
// base32-nopad-lowercase` matches the FactCid algebra so the agent can
// cite a review the same way they cite a fact. An optional ed25519
// signature ties the review to the agent's pubkey for verifiable
// provenance; unsigned reviews still land but list as anonymous.
//
// Listing is by subject (so an agent can ask "what do other agents
// think about fact X?") or globally paginated by submitted_at.

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ReviewRecord {
    /// Content-addressed CID over the canonical CBOR of every other
    /// field in this struct. Computed server-side; the input struct
    /// `ReviewIn` deliberately has no field for it, so clients cannot
    /// set it on POST. Always populated when reading from sled.
    #[serde(default)]
    review_cid: String,
    /// What the review is about — one of: "fact", "cell", "request_id",
    /// "session", "band", "endpoint", "other". Kept open so future
    /// subject types don't require a schema migration.
    subject_kind: String,
    /// The string identifier of the subject (fact_cid, cell64, request_id,
    /// band key, etc). The pair (subject_kind, subject_id) is the index.
    subject_id: String,
    /// What the agent was trying to do — short imperative, like "find
    /// elevation at Mt Fuji" or "verify a polygon-fan-out query".
    task: String,
    /// "success" | "partial" | "failed" | "noisy" — free-text but
    /// these four are the canonical values an agent should pick from.
    outcome: String,
    /// 1..=5 quality rating. 0 means unrated.
    #[serde(default)]
    rating: u8,
    /// Free-form comment. Limit to 4 KiB on the wire to keep the index
    /// scannable; longer reviews should be uploaded as a Fact and
    /// referenced via subject_kind="fact".
    #[serde(default)]
    comment: String,
    /// ISO-8601 UTC. Server stamps on POST (clients have no input
    /// field for it, so backdating is impossible). Always populated
    /// when reading from sled.
    #[serde(default)]
    submitted_at: String,
    /// Optional agent pubkey (32-byte ed25519, base32-nopad-lowercase).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    agent_pubkey_b32: Option<String>,
    /// Optional ed25519 signature over `blake3(canonical_cbor(payload))`,
    /// where `payload` is everything in this struct EXCEPT review_cid,
    /// submitted_at, and agent_signature. Hex-encoded 128 chars.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    agent_signature_hex: Option<String>,
}

const REVIEWS_TREE: &str = "emem.reviews";
const REVIEWS_INDEX_TREE: &str = "emem.reviews_by_subject";
const REVIEW_COMMENT_MAX: usize = 4096;
const REVIEW_TASK_MAX: usize = 512;

fn known_subject_kind(s: &str) -> bool {
    matches!(
        s,
        "fact" | "cell" | "request_id" | "session" | "band" | "endpoint" | "other"
    )
}

fn known_outcome(s: &str) -> bool {
    matches!(s, "success" | "partial" | "failed" | "noisy")
}

/// Compute a deterministic 16-byte review_cid (matches FactCid's
/// 80-bit truncation). Hashes the canonical-CBOR encoding of every
/// non-derived field so the same review payload always gets the same
/// CID — the same content-addressing property as facts.
fn compute_review_cid(rec: &ReviewRecord) -> String {
    let mut clean = rec.clone();
    clean.review_cid = String::new();
    clean.submitted_at = String::new();
    let mut buf = Vec::with_capacity(256);
    let _ = ciborium::ser::into_writer(&clean, &mut buf);
    let h = blake3::hash(&buf);
    data_encoding::BASE32_NOPAD
        .encode(&h.as_bytes()[..16])
        .to_lowercase()
}

#[derive(Deserialize)]
struct ReviewIn {
    subject_kind: String,
    subject_id: String,
    task: String,
    outcome: String,
    #[serde(default)]
    rating: u8,
    #[serde(default)]
    comment: String,
    #[serde(default)]
    agent_pubkey_b32: Option<String>,
    #[serde(default)]
    agent_signature_hex: Option<String>,
}

async fn post_review(
    State(s): State<AppState>,
    Json(req): Json<ReviewIn>,
) -> Result<Json<JsonValue>, ApiError> {
    if !known_subject_kind(&req.subject_kind) {
        return Err(ApiError(StatusCode::BAD_REQUEST, ErrorBody {
            code: ErrorCode::Internal,
            message: format!("subject_kind must be one of: fact, cell, request_id, session, band, endpoint, other. got: {:?}", req.subject_kind),
        }));
    }
    if !known_outcome(&req.outcome) {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            ErrorBody {
                code: ErrorCode::Internal,
                message: format!(
                    "outcome must be one of: success, partial, failed, noisy. got: {:?}",
                    req.outcome
                ),
            },
        ));
    }
    if req.task.is_empty() {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            ErrorBody {
                code: ErrorCode::Internal,
                message: "task is required and must be non-empty (1..=512 chars).".into(),
            },
        ));
    }
    if req.task.len() > REVIEW_TASK_MAX || req.comment.len() > REVIEW_COMMENT_MAX {
        return Err(ApiError(StatusCode::PAYLOAD_TOO_LARGE, ErrorBody {
            code: ErrorCode::Internal,
            message: format!("task ≤ {REVIEW_TASK_MAX} chars; comment ≤ {REVIEW_COMMENT_MAX} chars. For longer notes, upload as a Fact and reference via subject_kind=fact."),
        }));
    }
    if req.rating > 5 {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            ErrorBody {
                code: ErrorCode::Internal,
                message: "rating must be 0..=5 (0 = unrated).".into(),
            },
        ));
    }

    let mut rec = ReviewRecord {
        review_cid: String::new(),
        subject_kind: req.subject_kind,
        subject_id: req.subject_id,
        task: req.task,
        outcome: req.outcome,
        rating: req.rating,
        comment: req.comment,
        submitted_at: String::new(),
        agent_pubkey_b32: req.agent_pubkey_b32,
        agent_signature_hex: req.agent_signature_hex,
    };
    rec.review_cid = compute_review_cid(&rec);
    rec.submitted_at = chrono_iso8601_utc();

    let db = match s.storage.hot_sled_db() {
        Some(db) => db,
        None => return Err(ApiError(StatusCode::SERVICE_UNAVAILABLE, ErrorBody {
            code: ErrorCode::Internal,
            message: "reviews persistence requires a sled-backed hot cache; this responder is running ephemeral storage.".into(),
        })),
    };

    let tree = db.open_tree(REVIEWS_TREE).map_err(|e| {
        ApiError(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorBody {
                code: ErrorCode::Internal,
                message: format!("open reviews tree: {e}"),
            },
        )
    })?;
    let idx = db.open_tree(REVIEWS_INDEX_TREE).map_err(|e| {
        ApiError(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorBody {
                code: ErrorCode::Internal,
                message: format!("open reviews index: {e}"),
            },
        )
    })?;

    let mut buf = Vec::with_capacity(512);
    ciborium::ser::into_writer(&rec, &mut buf).map_err(|e| {
        ApiError(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorBody {
                code: ErrorCode::Internal,
                message: format!("cbor: {e}"),
            },
        )
    })?;

    // Primary tree: key = review_cid → value = full record.
    tree.insert(rec.review_cid.as_bytes(), buf.clone())
        .map_err(|e| {
            ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorBody {
                    code: ErrorCode::Internal,
                    message: format!("insert: {e}"),
                },
            )
        })?;
    // Index tree: key = "<subject_kind>:<subject_id>:<submitted_at>:<review_cid>"
    // (sorts by recency within a subject) → value = review_cid.
    let idx_key = format!(
        "{}:{}:{}:{}",
        rec.subject_kind, rec.subject_id, rec.submitted_at, rec.review_cid
    );
    idx.insert(idx_key.as_bytes(), rec.review_cid.as_bytes())
        .map_err(|e| {
            ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorBody {
                    code: ErrorCode::Internal,
                    message: format!("index insert: {e}"),
                },
            )
        })?;

    tracing::info!(
        target: "emem::reviews",
        review_cid = %rec.review_cid,
        review_subject_kind = %rec.subject_kind,
        review_subject_id = %rec.subject_id,
        review_outcome = %rec.outcome,
        review_rating = rec.rating,
        review_signed = rec.agent_signature_hex.is_some(),
        "review_submitted"
    );

    Ok(Json(json!({
        "ok": true,
        "review_cid": rec.review_cid,
        "submitted_at": rec.submitted_at,
        "subject": { "kind": rec.subject_kind, "id": rec.subject_id },
    })))
}

async fn list_reviews(
    State(s): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Json<JsonValue>, ApiError> {
    let limit: usize = q
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(50)
        .min(500);
    let db = match s.storage.hot_sled_db() {
        Some(db) => db,
        None => {
            return Ok(Json(
                json!({"reviews": [], "note": "ephemeral storage; reviews disabled"}),
            ))
        }
    };
    let tree = db.open_tree(REVIEWS_TREE).map_err(|e| {
        ApiError(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorBody {
                code: ErrorCode::Internal,
                message: format!("open: {e}"),
            },
        )
    })?;
    let mut out = Vec::with_capacity(limit);
    for (_k, v) in tree.iter().take(limit).flatten() {
        if let Ok(rec) = ciborium::de::from_reader::<ReviewRecord, _>(v.as_ref()) {
            if let Ok(j) = serde_json::to_value(&rec) {
                out.push(j);
            }
        }
    }
    // Recent-first.
    out.sort_by(|a, b| {
        b.get("submitted_at")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .cmp(a.get("submitted_at").and_then(|x| x.as_str()).unwrap_or(""))
    });
    Ok(Json(json!({
        "reviews": out,
        "count": out.len(),
        "schema": "emem.reviews.v1",
    })))
}

async fn reviews_for_subject(
    State(s): State<AppState>,
    Path(subject_id): Path<String>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Json<JsonValue>, ApiError> {
    let kind = q.get("kind").cloned().unwrap_or_else(|| "fact".to_string());
    if !known_subject_kind(&kind) {
        return Err(ApiError(StatusCode::BAD_REQUEST, ErrorBody {
            code: ErrorCode::Internal,
            message: format!("kind query param must be one of: fact, cell, request_id, session, band, endpoint, other. got: {kind:?}"),
        }));
    }
    let limit: usize = q
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(50)
        .min(500);
    let db = match s.storage.hot_sled_db() {
        Some(db) => db,
        None => {
            return Ok(Json(
                json!({"reviews": [], "note": "ephemeral storage; reviews disabled"}),
            ))
        }
    };
    let tree = db.open_tree(REVIEWS_TREE).map_err(|e| {
        ApiError(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorBody {
                code: ErrorCode::Internal,
                message: format!("open: {e}"),
            },
        )
    })?;
    let idx = db.open_tree(REVIEWS_INDEX_TREE).map_err(|e| {
        ApiError(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorBody {
                code: ErrorCode::Internal,
                message: format!("open idx: {e}"),
            },
        )
    })?;

    let prefix = format!("{kind}:{subject_id}:");
    let mut review_cids: Vec<String> = Vec::with_capacity(limit);
    for kv in idx.scan_prefix(prefix.as_bytes()) {
        if review_cids.len() >= limit {
            break;
        }
        if let Ok((_k, v)) = kv {
            if let Ok(cid) = std::str::from_utf8(v.as_ref()) {
                review_cids.push(cid.to_string());
            }
        }
    }
    let mut out = Vec::with_capacity(review_cids.len());
    for cid in &review_cids {
        if let Ok(Some(v)) = tree.get(cid.as_bytes()) {
            if let Ok(rec) = ciborium::de::from_reader::<ReviewRecord, _>(v.as_ref()) {
                if let Ok(j) = serde_json::to_value(&rec) {
                    out.push(j);
                }
            }
        }
    }
    out.sort_by(|a, b| {
        b.get("submitted_at")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .cmp(a.get("submitted_at").and_then(|x| x.as_str()).unwrap_or(""))
    });

    let mut sum_rating = 0u32;
    let mut rated = 0u32;
    let mut by_outcome = std::collections::HashMap::<String, u32>::new();
    for r in &out {
        if let Some(rt) = r.get("rating").and_then(|v| v.as_u64()) {
            if rt > 0 {
                sum_rating += rt as u32;
                rated += 1;
            }
        }
        if let Some(oc) = r.get("outcome").and_then(|v| v.as_str()) {
            *by_outcome.entry(oc.to_string()).or_insert(0) += 1;
        }
    }
    let mean = if rated == 0 {
        None
    } else {
        Some(sum_rating as f64 / rated as f64)
    };

    Ok(Json(json!({
        "subject": { "kind": kind, "id": subject_id },
        "reviews": out,
        "count": out.len(),
        "rating_mean": mean,
        "by_outcome": by_outcome,
        "schema": "emem.reviews.v1",
    })))
}

async fn agent_stats_endpoint() -> Json<JsonValue> {
    let st = agent_stats();
    let by_family: Vec<JsonValue> = match st.by_family.lock() {
        Ok(m) => {
            let mut v: Vec<_> = m
                .iter()
                .map(|(k, c)| (*k, c.requests, c.errors, c.last_seen_unix_s))
                .collect();
            v.sort_by_key(|(_, requests, _, _)| std::cmp::Reverse(*requests));
            v.into_iter()
                .map(|(k, r, e, t)| {
                    json!({
                        "family": k, "requests": r, "errors": e, "last_seen_unix_s": t,
                    })
                })
                .collect()
        }
        Err(_) => vec![],
    };
    let by_tool: Vec<JsonValue> = match st.by_tool.lock() {
        Ok(m) => {
            let mut v: Vec<_> = m
                .iter()
                .map(|(k, c)| (k.clone(), c.calls, c.errors, c.total_dur_ms))
                .collect();
            v.sort_by_key(|(_, calls, _, _)| std::cmp::Reverse(*calls));
            v.into_iter()
                .map(|(k, c, e, d)| {
                    json!({
                        "tool": k, "calls": c, "errors": e,
                        "mean_duration_ms": if c == 0 { 0.0 } else { d / c as f64 },
                    })
                })
                .collect()
        }
        Err(_) => vec![],
    };
    let buckets: Vec<JsonValue> = LATENCY_BUCKETS_MS
        .iter()
        .enumerate()
        .map(|(i, b)| {
            json!({
                "le_ms": b, "count": st.latency[i].load(Ordering::Relaxed),
            })
        })
        .chain(std::iter::once(json!({
            "le_ms": "+Inf", "count": st.latency[12].load(Ordering::Relaxed),
        })))
        .collect();
    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let uptime = START_INSTANT.get_or_init(Instant::now).elapsed().as_secs();
    Json(json!({
        "schema": "emem.agent_stats.v1",
        "since_unix_s": now_unix.saturating_sub(uptime),
        "uptime_seconds": uptime,
        "status_classes": {
            "2xx": st.status_2xx.load(Ordering::Relaxed),
            "3xx": st.status_3xx.load(Ordering::Relaxed),
            "4xx": st.status_4xx.load(Ordering::Relaxed),
            "5xx": st.status_5xx.load(Ordering::Relaxed),
        },
        "by_family": by_family,
        "by_tool":   by_tool,
        "latency_histogram_ms": buckets,
        "latency_p50_ms": latency_percentile(0.50),
        "latency_p95_ms": latency_percentile(0.95),
        "latency_p99_ms": latency_percentile(0.99),
        "next": [
            "GET /metrics  (Prometheus text format, includes responder pubkey)",
            "GET /v1/contributors  (CoIL leaderboard)",
            "GET /health  (responder identity + uptime)"
        ]
    }))
}

fn metrics_inc(c: &AtomicU64) {
    c.fetch_add(1, Ordering::Relaxed);
}

async fn metrics(State(s): State<AppState>) -> Response {
    let start = *START_INSTANT.get_or_init(Instant::now);
    let uptime = start.elapsed().as_secs();
    let bands_count = emem_core::bands::DEFAULT.bands.len();
    let pubkey = data_encoding::BASE32_NOPAD
        .encode(&s.identity.pubkey.0)
        .to_lowercase();
    let body = format!(
        "# HELP emem_uptime_seconds Process uptime in seconds.
# TYPE emem_uptime_seconds counter
emem_uptime_seconds {uptime}
# HELP emem_request_total Total HTTP requests handled (excludes preflight).
# TYPE emem_request_total counter
emem_request_total {req}
# HELP emem_rate_limited_total Total HTTP requests rejected by the rate limiter.
# TYPE emem_rate_limited_total counter
emem_rate_limited_total {rl}
# HELP emem_attest_total Total successful attestations accepted.
# TYPE emem_attest_total counter
emem_attest_total {att}
# HELP emem_attest_fail_total Total attestations rejected (sig/merkle/dup).
# TYPE emem_attest_fail_total counter
emem_attest_fail_total {attf}
# HELP emem_recall_total Total recall calls served.
# TYPE emem_recall_total counter
emem_recall_total {rec}
# HELP emem_mcp_total Total MCP JSON-RPC calls served.
# TYPE emem_mcp_total counter
emem_mcp_total {mcp}
# HELP emem_bands Number of bands registered in the active manifest.
# TYPE emem_bands gauge
emem_bands {bands_count}
# HELP emem_responder_pubkey Responder identity (label only).
# TYPE emem_responder_pubkey gauge
emem_responder_pubkey{{pubkey_b32=\"{pubkey}\"}} 1
",
        req = REQ_TOTAL.load(Ordering::Relaxed),
        rl = RATE_LIMITED_TOTAL.load(Ordering::Relaxed),
        att = ATTEST_TOTAL.load(Ordering::Relaxed),
        attf = ATTEST_FAIL_TOTAL.load(Ordering::Relaxed),
        rec = RECALL_TOTAL.load(Ordering::Relaxed),
        mcp = MCP_TOTAL.load(Ordering::Relaxed),
    );
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")
        .body(axum::body::Body::from(body))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn not_found(msg: &str) -> Response {
    let body = serde_json::to_vec(&serde_json::json!({
        "code": "not_found", "message": msg,
    }))
    .unwrap_or_else(|_| br#"{"code":"not_found"}"#.to_vec());
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body))
        .unwrap_or_else(|_| StatusCode::NOT_FOUND.into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The structural-routing eval corpus. Each prompt is a free-text
    /// question that an LLM with emem connected SHOULD route into the
    /// protocol — historically these are the questions where Claude
    /// would have refused with "I can't access live data" or fallen
    /// back to web search. The first matched topic is asserted; if the
    /// router fails, descriptions / keyword sets need to widen.
    const SHOULD_BE_EMEM_ASK: &[(&str, &str)] = &[
        // Lifestyle / decision-making — the original Ashok-Nagar shape.
        (
            "is this neighbourhood flood-prone for a flat purchase",
            "flood_risk_composite",
        ),
        (
            "should I buy a house here, is the area safe to live",
            "flood_risk_composite",
        ),
        (
            "is it safe to invest in property in this area",
            "flood_risk_composite",
        ),
        (
            "does this area have monsoon waterlogging issues",
            "flood_risk_composite",
        ),
        // Word-ordering variants (the bug the live test caught:
        // "purchase a flat" should route same as "flat purchase").
        (
            "I want to purchase a flat here, has it ever flooded",
            "flood_risk_composite",
        ),
        (
            "buying a home in this area, is it safe to buy",
            "flood_risk_composite",
        ),
        (
            "purchasing an apartment, monsoon waterlogging concerns",
            "flood_risk_composite",
        ),
        // Insurance / property risk.
        ("what's the property risk for this address", "real_estate"),
        (
            "estimate the insurance premium for this neighbourhood",
            "real_estate",
        ),
        // Livability.
        ("how walkable is this area", "urban_livability"),
        ("urban heat island intensity here", "urban_livability"),
        (
            "does this neighbourhood have green space access",
            "urban_livability",
        ),
        // Direct flood / water.
        ("flood history of this place", "flood_history_long_term"),
        ("has this area ever flooded", "flood_history_long_term"),
        (
            "is there standing water right now at this site",
            "flood_water_event_window",
        ),
        // Vegetation.
        ("what's the NDVI here", "vegetation_condition"),
        ("crop health in this region", "vegetation_condition"),
        // Built-up.
        ("is this area densely built up", "built_up_human_geography"),
        (
            "road length and building count in this neighbourhood",
            "built_up_human_geography",
        ),
        // Topography.
        (
            "elevation of this place above sea level",
            "elevation_land_only",
        ),
        ("how rugged is the terrain here", "topography"),
        // Energy / agri / esg / health.
        ("crop yield potential of this farm", "agriculture"),
        ("carbon sink potential of this forest", "esg"),
        ("heat vulnerability of this neighbourhood", "public_health"),
        (
            "similar to this place, find lookalikes",
            "foundation_embedding",
        ),
    ];

    #[test]
    fn route_question_corpus_hits_expected_topic() {
        let mut misses: Vec<(&str, &str, Vec<&'static str>)> = Vec::new();
        for (q, expected) in SHOULD_BE_EMEM_ASK {
            let hits = route_question_to_topics(q);
            // The first matched topic is the highest-priority match —
            // composite topics declared early in TOPIC_KEYWORDS win
            // over single-band topics, which is the desired behaviour.
            if hits.first().copied() != Some(*expected) {
                misses.push((q, expected, hits));
            }
        }
        assert!(
            misses.is_empty(),
            "router missed {} of {} corpus questions:\n{}",
            misses.len(),
            SHOULD_BE_EMEM_ASK.len(),
            misses
                .iter()
                .map(|(q, exp, got)| format!("  q={q:?} expected={exp:?} got={got:?}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    #[test]
    fn topic_keys_all_have_either_bands_or_algorithms() {
        // Every topic the router can match must resolve to at least one
        // band-or-algorithm — otherwise /v1/ask would return an empty
        // recall and the agent would have nothing to cite.
        let alg_reg = &*emem_core::algorithms::DEFAULT;
        for (topic, _) in TOPIC_KEYWORDS {
            let bands_count = live_bands_for_topic(topic).len();
            let alg_count = algorithms_keys_for_topic(topic).len();
            // Walk the algorithm registry to count bands those algorithms read.
            let alg_band_count: usize = algorithms_keys_for_topic(topic)
                .iter()
                .map(|k| alg_reg.input_bands(k).len())
                .sum();
            assert!(
                bands_count > 0 || alg_count > 0,
                "topic {topic} has no live_bands AND no algorithms — router would route to nothing"
            );
            if bands_count == 0 {
                assert!(
                    alg_band_count > 0,
                    "topic {topic} has algorithms {:?} but the registry knows no input bands for any of them",
                    algorithms_keys_for_topic(topic)
                );
            }
        }
    }

    #[test]
    fn empty_question_routes_to_nothing() {
        assert!(route_question_to_topics("").is_empty());
        assert!(route_question_to_topics("hello world").is_empty());
    }

    #[test]
    fn ask_req_deserialises_minimal_form() {
        let r: AskReq = serde_json::from_value(serde_json::json!({
            "q": "is this flood-prone", "place": "Ashok Nagar, Ranchi"
        }))
        .unwrap();
        assert_eq!(r.q, "is this flood-prone");
        assert_eq!(r.place.as_deref(), Some("Ashok Nagar, Ranchi"));
        assert!(r.cell.is_none());
        assert!(!r.include_image);
    }

    #[test]
    fn intent_ask_plans_to_emem_ask() {
        use emem_intent::{plan, Intent};
        let p = plan(&Intent::Ask {
            description: "is this flood-prone".into(),
            place: Some("Ashok Nagar, Ranchi".into()),
            cell: None,
            lat: None,
            lng: None,
        });
        assert_eq!(p.calls.len(), 1);
        assert_eq!(p.calls[0].primitive, "emem_ask");
    }

    #[test]
    fn intent_what_is_here_without_cell_routes_to_ask() {
        use emem_intent::{plan, Intent};
        let p = plan(&Intent::WhatIsHere {
            cell: None,
            place: Some("Ashok Nagar, Ranchi".into()),
            description: Some("flood risk and waterlogging".into()),
        });
        assert_eq!(
            p.calls.len(),
            1,
            "expected single call to emem_ask, got {p:?}"
        );
        assert_eq!(p.calls[0].primitive, "emem_ask");
    }
}
