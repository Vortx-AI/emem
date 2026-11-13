//! The HTTP surface a checkpoint calls.
//!
//! Nine routes, seven of which belong to no vendor. Every one of them lands in
//! the same engine, and a test asserts they reach the same verdict on the same
//! evidence, because a guard that answered differently by door would be
//! discriminating between agents on the basis of the client they happened to
//! hold.
//!
//! # The operational rules, and where they come from
//!
//! Each rule below is set to satisfy the STRICTEST checkpoint contract this
//! crate has been implemented against, so it also satisfies the looser ones.
//! That is deliberate: a server tuned to the median contract fails silently at
//! the demanding one, and the failure mode is always the same, a server that
//! appears to enforce and does not.
//!
//!   - **Always a success status with a parseable verdict.** A non-200, a
//!     redirect, an unparseable body or a timeout is a transport failure, not
//!     a deny: it hands the decision to the caller's failure policy and, under
//!     fail-open, lets the request through uninspected. Sustained failures
//!     trip a breaker that stops enforcement entirely. So there is no error
//!     path here: malformed bodies, unknown events and internal faults all
//!     answer 200 with an allow. Claude Code makes the same point from the
//!     other side, where blocking IS a 2xx carrying a deny in the body.
//!   - **10 MB bodies.** Transcripts arrive untruncated. A rejected body counts
//!     as a failure, and common proxy defaults (nginx 1 MB, Express 100 kB) sit
//!     far below the ceiling, so this is a thing you have to go and fix in
//!     front of the server as well as in it.
//!   - **Answer inside the budget.** The tightest floor we have seen an
//!     operator able to configure is 1000 ms for the whole exchange, so
//!     [`SELF_BUDGET`] is 800 ms and everything inside it is bounded.
//!   - **Deduplicate on the request id AND the input.** Retries reuse both,
//!     and re-evaluating could return a different answer than the one already
//!     acted on. The id alone is not enough: a Claude Code hook sends its
//!     session id for every input, so an id-only key replayed the session's
//!     first verdict for every later input.

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
use crate::interop::{
    BatchRequest, BatchResponse, CloudEvent, CloudEventGate, McpCall, McpGate, OpenAiGate,
    OpenAiRequest, PolicyPointGate, MAX_BATCH,
};
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
/// The share of the self-budget citations may consume.
///
/// Half, so signing and the log append always have room. A verdict that
/// resolved every token and then missed its deadline helps nobody: the
/// platform records a timeout, which is a webhook failure, which is worse
/// than an allow that checked less than it wanted to.
pub const RESOLVE_BUDGET: Duration = Duration::from_millis(400);

/// How many citations one transcript may have resolved.
///
/// A ceiling rather than a guess: a transcript can carry thousands of tokens,
/// and an unbounded loop over them is a denial-of-service against ourselves
/// that any caller could trigger. Beyond this the rest are unresolved, which
/// is not a denial, so the cap costs coverage and never correctness.
pub const MAX_RESOLVED_TOKENS: usize = 64;

pub struct Guard {
    pub config: Config,
    /// How citations are checked. A bare node uses [`NullResolver`] and
    /// allows everything; a node pointed at a responder verifies.
    pub resolver: Box<dyn crate::resolve::Resolver>,
    /// cell64 addresses the operator restricted. Exact match only.
    pub restricted_cells: std::collections::HashSet<String>,
    /// Loaded detection modules. Empty is the default and the common case.
    pub modules: crate::module::ModuleRegistry,
    /// Whether transcript text may be shown to modules at all.
    ///
    /// False in a split-verdict deployment where text does not cross the
    /// boundary. Every module then sees an empty transcript regardless of its
    /// declared data class, so a text-hungry module loaded here is inert
    /// rather than a leak.
    pub modules_may_read_text: bool,
    pub log: FileLog,
    pub signing: ed25519_dalek::SigningKey,
    /// The key for every digest that reaches the open log.
    ///
    /// A separate secret, never the signing key. `/log/entries` needs no
    /// auth, and an unkeyed BLAKE3 of a short prompt or a matched string is a
    /// guess test: hash the guess and look for it. Under this key only the
    /// operator can test a guess.
    pub digest_key: [u8; 32],
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
    /// A keyed BLAKE3 digest, hex, under [`Guard::digest_key`].
    ///
    /// `domain` keeps digests of different things apart, and every part is
    /// length-prefixed so `["ab", "c"]` and `["a", "bc"]` differ.
    fn keyed_digest(&self, domain: &str, parts: &[&[u8]]) -> String {
        let mut h = blake3::Hasher::new_keyed(&self.digest_key);
        h.update(&(domain.len() as u64).to_le_bytes());
        h.update(domain.as_bytes());
        for p in parts {
            h.update(&(p.len() as u64).to_le_bytes());
            h.update(p);
        }
        h.finalize().to_hex().to_string()
    }

    /// The keyed digest of the texts a verdict judges.
    ///
    /// The second half of the idempotency key, and signed into the record so
    /// the operator, who holds the key, can show which input a verdict was
    /// about.
    fn input_digest(&self, texts: &[&str]) -> String {
        let parts: Vec<&[u8]> = texts.iter().map(|t| t.as_bytes()).collect();
        self.keyed_digest("emem.guard.input.v1", &parts)
    }

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

        // Claims are scanned whenever the rule is configured at all, including
        // in shadow, because an org that cannot see the count has no basis on
        // which to decide whether to enforce. Scanning is pure string work
        // over text already in memory, so it costs nothing a resolver does not
        // already dwarf.
        let claims = if self.config.claim_gating {
            crate::claim::scan_claims(texts.iter().copied())
        } else {
            Vec::new()
        };

        // The budget is shared across every citation, not granted per token.
        // A transcript carrying forty of them must not take forty times as
        // long as one: the deadline belongs to the exchange, and a caller who
        // cites heavily should not be the one who trips the breaker.
        let per_token = if need.is_empty() {
            Duration::ZERO
        } else {
            RESOLVE_BUDGET / need.len().min(MAX_RESOLVED_TOKENS) as u32
        };

        let mut tokens = Vec::with_capacity(need.len());
        // The numeric reading behind each citation that resolved to one, so
        // the value-agreement rule can compare prose against fact without a
        // second round trip.
        let mut values: Vec<(String, crate::resolve::FactValue)> = Vec::new();
        let deadline = Instant::now() + RESOLVE_BUDGET;
        for t in need {
            // Past the deadline everything remaining is unresolved rather
            // than unchecked-and-silent. The count still reaches the log, so
            // a shadow report can show how often the budget ran out.
            let resolved = if Instant::now() >= deadline || tokens.len() >= MAX_RESOLVED_TOKENS {
                crate::resolve::Resolution::bare(policy::TokenStatus::Unresolved)
            } else {
                // The resolver does BLOCKING http, and this runs inside an
                // async handler. Calling it straight through panicked the
                // worker with "Cannot drop a runtime in a context where
                // blocking is not allowed" the moment a transcript carried a
                // citation, so a node started with `--responder` answered
                // every cited request by dropping the connection. Nothing
                // caught it because every test here uses `NullResolver`,
                // which does no IO: the resolving path had no coverage at all.
                //
                // `block_in_place` tells the scheduler this worker is about
                // to block, which both unblocks the runtime and makes the
                // drop legal. It requires a multi-thread runtime and a
                // runtime context, so both are checked rather than assumed;
                // outside one, a direct call is already correct.
                let resolve_one = || self.resolver.resolve(t, per_token);
                match tokio::runtime::Handle::try_current() {
                    Ok(h) if h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
                        tokio::task::block_in_place(resolve_one)
                    }
                    _ => resolve_one(),
                }
            };
            if let Some(v) = resolved.value.clone() {
                values.push((t.token.clone(), v));
            }
            tokens.push((t.clone(), resolved.status));
        }

        // Modules run over the evidence gathered so far, on the same thread and
        // inside the same budget. `enforcing` is what keeps a slow module off
        // the blocking path; the registry, not this call site, decides.
        //
        // The text digest is keyed. A `digests_only` module is not shown the
        // text, and an unkeyed hash of a short prompt would show it anyway to
        // a module that hashes guesses. Its own domain, so it never equals the
        // `input_digest` in the open log.
        let parts: Vec<&[u8]> = texts.iter().map(|t| t.as_bytes()).collect();
        let text_digest = self.keyed_digest("emem.guard.module_text.v1", &parts);
        let no_text: [&str; 0] = [];
        let module_ctx = crate::module::ModuleContext {
            texts: if self.modules_may_read_text {
                texts
            } else {
                &no_text
            },
            tokens: &tokens,
            claims: &claims,
            text_digest: &text_digest,
        };
        let modules = if self.modules.is_empty() {
            Vec::new()
        } else {
            self.modules
                .run(&module_ctx, self.config.mode == policy::Mode::Enforce)
                .into_iter()
                .map(|(man, mut v)| {
                    // A module hashes its own evidence with plain BLAKE3, as it
                    // holds no key. Re-keyed here, before it can reach a
                    // decision or the open log.
                    v.evidence_digest = v
                        .evidence_digest
                        .map(|d| self.keyed_digest("emem.guard.evidence.v1", &[d.as_bytes()]));
                    (man, v)
                })
                .collect()
        };

        Evidence {
            tokens,
            values,
            // Scanned whenever provenance is on, since the value-agreement
            // rule lives there. Pure string work over text already in memory.
            cited_numbers: if self.config.provenance {
                crate::claim::scan_cited_numbers(texts.iter().copied())
            } else {
                Vec::new()
            },
            // A cell64 reference is restricted when the operator listed it.
            // Exact match against a configured set, never a geocode: fuzzy
            // resolution on the verdict path is how a guard confidently
            // blocks innocent work, and a wrong deny is the worst failure
            // this system has.
            restricted_cells: self.restricted_in(&found),
            claims,
            modules,
        }
    }

    /// The restricted cells that `found` cites, in order.
    fn restricted_in(&self, found: &[tokens::FoundToken]) -> Vec<String> {
        found
            .iter()
            .filter_map(|t| cell_of(&t.token))
            .filter(|c| self.restricted_cells.contains(*c))
            .map(str::to_string)
            .collect()
    }

    /// The keyed digest that stands in for a restricted cell in the open log.
    fn cell_digest(&self, cell: &str) -> String {
        self.keyed_digest("emem.guard.restricted_cell.v1", &[cell.as_bytes()])
    }

    /// The restricted cell that a token would show in the log, if any.
    ///
    /// A GEO_ZONE token is the bare cell. Any other token can carry a cell in
    /// its address (`emem:fact:<cell>:<cid>`), so a PROV_SIG on a restricted
    /// cell would publish the cell as surely as a GEO_ZONE would.
    fn restricted_cell_of(&self, token: &str) -> Option<String> {
        if self.restricted_cells.contains(token) {
            return Some(token.to_string());
        }
        cell_of(token)
            .filter(|c| self.restricted_cells.contains(*c))
            .map(str::to_string)
    }

    /// The token that a logged record held back, read again from the retry.
    ///
    /// A retry carries the same input, so the token is in it. The logged
    /// digest binds the cell, not the token. When two different tokens in the
    /// input name that cell, this answers `None`: naming the wrong token would
    /// send an agent to remove a citation that was fine.
    fn withheld_token(
        &self,
        code: policy::DenyCode,
        texts: &[&str],
        logged: &str,
    ) -> Option<String> {
        let candidates: std::collections::BTreeSet<String> =
            tokens::scan_all(texts.iter().copied())
                .into_iter()
                .filter_map(|t| {
                    let cell = cell_of(&t.token).filter(|c| self.restricted_cells.contains(*c))?;
                    if self.cell_digest(cell) != logged {
                        return None;
                    }
                    match code {
                        policy::DenyCode::GeoZone => Some(cell.to_string()),
                        // A cell token is never resolved, so it is never the token
                        // a provenance rule names.
                        _ if t.kind == tokens::TokenKind::Cell => None,
                        _ => Some(t.token),
                    }
                })
                .collect();
        match candidates.len() {
            1 => candidates.into_iter().next(),
            _ => None,
        }
    }

    /// Decide, sign, log, and return what the caller should send.
    fn decide(&self, checkpoint: &str, request_id: &str, texts: &[&str]) -> Decision {
        // A replay must get the answer already recorded, not a fresh one. But
        // only a replay of the SAME input: the id alone is not enough. A
        // Claude Code hook sends its session id for every input, and a caller
        // can reuse an id. Keyed on the id only, the first verdict of a
        // session answered every later input, so a clean first message let a
        // restricted cell through.
        let input = self.input_digest(texts);
        if let Some(prior) = self.log.find_replay(request_id, Some(&input)) {
            let mut d = if prior.record.code.is_some() {
                // The log holds no token that names a restricted cell, whatever
                // the code. The retry carries the same input, so the token is
                // read back from it and confirmed against the logged digest. A
                // module denial also has no token, but its digest is its own.
                let token = match (
                    prior.record.code,
                    &prior.record.token,
                    &prior.record.module,
                    &prior.record.evidence_digest,
                ) {
                    (Some(code), None, None, Some(logged)) => {
                        self.withheld_token(code, texts, logged)
                    }
                    _ => prior.record.token.clone(),
                };
                Decision::block(
                    prior.record.code.unwrap_or(policy::DenyCode::ProvSig),
                    token,
                    prior.record.fix.unwrap_or(policy::Fix::ContactAdmin),
                )
            } else {
                Decision::proceed()
            };
            d = d
                .with_checked(prior.record.checked)
                .with_leaf(format!("leaf_{}", prior.seq));
            // Replay the MODE the original was decided under, not the current
            // one. A node switched from shadow to enforcing between a request
            // and its retry must not turn a verdict the caller was already
            // told to allow into a block.
            return d.under(prior.record.mode);
        }

        let evidence = self.gather(texts);
        let mut decision = policy::evaluate(&self.config, &evidence);
        // The caller is told which token it cited; the open log is not told
        // when that token names a restricted cell, whatever rule fired. A list
        // of restricted cells is the operator's to publish.
        if let Some(cell) = decision
            .token
            .as_deref()
            .and_then(|t| self.restricted_cell_of(t))
        {
            decision = decision.with_token_digest(self.cell_digest(&cell));
        }
        // Shadow is applied BEFORE sealing, so the record holds the answer the
        // caller actually received in `outcome` and the answer the rules
        // reached in `evaluated`. Sealing the enforcing decision and
        // downgrading afterwards would log a block that never happened.
        let returned = decision.under(self.config.mode);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        // The append fsyncs before it returns. Run directly on an async
        // worker, it held that worker for the whole fsync, so concurrent
        // verdicts were capped at the worker count and could not share one
        // group commit. `block_in_place`, as for the resolver above.
        let seal_one = || {
            seal(
                checkpoint,
                request_id,
                Some(&input),
                returned,
                now,
                self.config.mode,
                &self.modules.config_digest(),
                signer(&self.signing),
                &self.log,
                self.log_failure_policy,
            )
        };
        let (sealed, log_err) = match tokio::runtime::Handle::try_current() {
            Ok(h) if h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
                tokio::task::block_in_place(seal_one)
            }
            _ => seal_one(),
        };
        if let Some(e) = log_err {
            // Visible, because an unlogged verdict is the one condition that
            // silently weakens the product's whole claim.
            eprintln!("emem-guard: verdict not logged: {e}");
        }
        sealed
    }

    /// Run one adapter end to end: parse, evaluate, render.
    ///
    /// Every route is this function with a different adapter. Written once so
    /// a new checkpoint cannot accidentally skip the signature check, the
    /// idempotency lookup or the log, which are the three things that make a
    /// verdict worth anything and the three easiest to leave out of a
    /// hand-written handler.
    fn run<A: Adapter>(&self, adapter: &A, body: &[u8]) -> A::Response
    where
        A::Request: serde::de::DeserializeOwned,
    {
        let Ok(req) = serde_json::from_slice::<A::Request>(body) else {
            // Unparseable is never a failure of the exchange. See the module
            // docs: a non-verdict is worse than a permissive one.
            return adapter.render(&Decision::proceed());
        };
        let decision = match adapter.transcript(&req) {
            None => Decision::proceed(),
            Some(t) => self.decide(
                adapter.checkpoint_id(),
                &request_id_or_new(t.request_id),
                &t.texts,
            ),
        };
        adapter.render(&decision)
    }
}

/// The caller's request id, or a fresh one when it sent none.
///
/// A caller with no id still gets an answer; it simply cannot claim to be a
/// replay of an earlier one, which is the honest behaviour when nothing
/// identifies the earlier one. An empty id counts as none. The two vendor
/// routes once mapped a missing id to `""`, so every request without one
/// shared that id.
fn request_id_or_new(id: Option<&str>) -> String {
    match id.map(str::trim) {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => ulid::Ulid::new().to_string(),
    }
}

/// The cell64 a token names, for the restricted-zone check.
///
/// Only the address form. The spoken form carries coordinates, and turning
/// those into an address would be a geocode, which the verdict path forbids.
fn cell_of(token: &str) -> Option<&str> {
    let rest = token
        .strip_prefix("emem:fact:")
        .or_else(|| token.strip_prefix("emem:cell:"))?;
    let cell = rest.split(':').next()?;
    (cell.contains('.') && !cell.contains(',') && !cell.contains('@')).then_some(cell)
}

/// Build the router.
///
/// The route table is the interoperability commitment made concrete. Two
/// vendor checkpoints, one native, four standards-based, and a batch shape,
/// all landing in the same engine. Nothing about the decision changes with the
/// door it came through, which is asserted rather than assumed in
/// [`tests::every_route_reaches_the_same_verdict_on_the_same_evidence`].
pub fn router(guard: Arc<Guard>) -> Router {
    Router::new()
        .route("/health", get(health))
        // Self-description, so an agent learns the whole contract from one GET
        // rather than from documentation it has to be told to read.
        .route("/.well-known/emem-guard.json", get(descriptor))
        // ── vendor checkpoints ──
        .route("/verdict/anthropic-hook", post(anthropic))
        .route("/verdict/claude-code", post(claude_code))
        // ── the open route ──
        // No vendor, no account, no installed client: any agent on any model
        // can ask the same question and get the same answer, with the log leaf
        // attached so it need not take our word.
        .route("/verdict", post(native))
        .route("/verdict/native", post(native))
        // ── standards-based checkpoints ──
        .route("/verdict/mcp", post(mcp))
        .route("/verdict/openai", post(openai))
        .route("/verdict/cloudevent", post(cloudevent))
        .route("/verdict/policy", post(policy_point))
        .route("/verdict/batch", post(batch))
        // ── the evidence half ──
        // A denial names a leaf. Without these it would be naming something
        // nobody could fetch, which is a promise of verifiability rather than
        // verifiability.
        .route("/log/head", get(log_head))
        .route("/log/entry/:leaf", get(log_entry))
        .route("/log/entries", get(log_entries))
        .route("/log/report", get(log_report))
        // What detection is loaded, what class it declared, and what it has
        // actually cost. An operator who cannot see the second and third has
        // only the first, which is a promise.
        .route("/modules", get(modules))
        // Transcripts arrive untruncated up to 10 MB and a rejected body is a
        // transport failure, so the limit is the ceiling rather than a
        // comfortable default.
        //
        // BOTH layers are required and neither is redundant. Axum applies its
        // own DefaultBodyLimit of 2 MB to every extractor, and it wins
        // regardless of what tower-http is configured with: this server
        // documented 10 MB, carried the tower-http layer, and still answered
        // 413 on anything past 2 MB. Nothing caught it, because a unit test
        // that drives the router directly never sends a body that large. The
        // conformance suite found it on the first run against a real socket.
        .layer(axum::extract::DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(tower_http::limit::RequestBodyLimitLayer::new(
            MAX_BODY_BYTES,
        ))
        .with_state(guard)
}

async fn health(State(g): State<Arc<Guard>>) -> impl IntoResponse {
    let (seq, chain) = g.log.head();
    Json(serde_json::json!({
        "ok": true,
        "signer_b32": g.log.signer_b32(),
        "verdicts_logged": seq,
        "log_head": chain,
        "require_signature": g.require_signature,
        "mode": g.config.mode.as_str(),
        // Which resolver is attached, because a bare node's allow and a
        // corpus-backed node's allow are different claims and an operator
        // should never have to guess which one they are running.
        "resolver": g.resolver.describe(),
        "verifies_citations": g.resolver.describe() != "null",
        "rules": {
            "provenance": g.config.provenance,
            "freshness": g.config.freshness,
            "geo_restriction": g.config.geo_restriction,
            "claim_gating": g.config.claim_gating,
        },
    }))
}

/// Everything an agent needs to use this node, in one document.
///
/// The alternative is an agent that has to be handed a README by a person,
/// which is the coupling this whole crate exists to remove. A cold agent can
/// GET this, learn every route, every deny code, every remedy and the exact
/// reason grammar, and integrate without reading a word of prose.
async fn descriptor(State(g): State<Arc<Guard>>) -> impl IntoResponse {
    let codes: Vec<_> = crate::interop::deny_codes()
        .into_iter()
        .map(|(code, means)| serde_json::json!({"code": code, "means": means}))
        .collect();
    let fixes: Vec<_> = crate::interop::fixes()
        .into_iter()
        .map(|(fix, action)| serde_json::json!({"fix": fix, "action": action}))
        .collect();
    Json(serde_json::json!({
        "service": "emem-guard",
        "version": env!("CARGO_PKG_VERSION"),
        "what_it_checks": "whether cited observations verify, and whether \
            measurable claims about the physical world carry a citation",
        "what_it_does_not_check": "content safety, personal data, secrets. \
            emem-guard is a chassis for those, not an implementation of them.",
        "signer_b32": g.log.signer_b32(),
        "mode": g.config.mode.as_str(),
        "resolver": g.resolver.describe(),
        "verifies_citations": g.resolver.describe() != "null",
        "checkpoints": [
            {"route": "/verdict", "shape": "emem native", "checkpoint_id": "emem.native.v1",
             "open": true},
            {"route": "/verdict/native", "shape": "emem native", "checkpoint_id": "emem.native.v1",
             "open": true},
            {"route": "/verdict/mcp", "shape": "MCP tools/call", "checkpoint_id": "mcp.tools_call.v1",
             "open": true},
            {"route": "/verdict/openai", "shape": "OpenAI-compatible chat or moderations",
             "checkpoint_id": "openai.compatible.v1", "open": true},
            {"route": "/verdict/cloudevent", "shape": "CloudEvents 1.0",
             "checkpoint_id": "cloudevents.v1", "open": true},
            {"route": "/verdict/policy", "shape": "OPA-style input/result",
             "checkpoint_id": "policy_point.v1", "open": true},
            {"route": "/verdict/batch", "shape": "array of emem native",
             "checkpoint_id": "emem.native.v1", "open": true,
             "max_items": MAX_BATCH},
            {"route": "/verdict/anthropic-hook", "shape": "Anthropic Inference hooks",
             "checkpoint_id": "anthropic.inference_hook.v1", "open": false},
            {"route": "/verdict/claude-code", "shape": "Claude Code hook input",
             "checkpoint_id": "anthropic.claude_code_hook.v1", "open": false}
        ],
        "modules": {
            "route": "/modules",
            "loaded": g.modules.len(),
            "config_digest": g.modules.config_digest(),
            "note": "detection is pluggable; grounding is native. A module's verdict is signed and logged like any other, and the log records its id and a keyed evidence digest, never what it matched.",
        },
        "evidence": {
            "head": "/log/head",
            "entry": "/log/entry/{leaf}",
            "entries": "/log/entries?start=0&count=50",
            "report": "/log/report",
            "preimage_domain": crate::log::PREIMAGE_DOMAIN,
            "signature": "ed25519 over the record preimage, base32-nopad-lowercase",
            "digests": "input_digest and evidence_digest are keyed BLAKE3 under this node's digest key. Without the key, nobody can test a guess against them.",
            "restricted_cells": "a record whose token names a restricted cell holds no token, whatever the code. Its evidence_digest is a keyed digest of the cell. The token is in the caller's reply only.",
            "replay": "a retry gets the recorded verdict only for the same request_id and the same input_digest"
        },
        "reason_grammar": "EMEM-GUARD DENY <CODE> token=<token|-> fix=<fix> leaf=<leaf|->",
        "deny_codes": codes,
        "fixes": fixes,
        "rules": {
            "provenance": g.config.provenance,
            "freshness": g.config.freshness,
            "geo_restriction": g.config.geo_restriction,
            "claim_gating": g.config.claim_gating,
        },
        "never_denies_on": "a citation this node does not hold. It is \
            indistinguishable from one minted by another responder.",
        "self_host": "https://github.com/Vortx-AI/emem/blob/main/crates/emem-guard/SKILL.md",
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
        // A DISTINCT, greppable level. This is the one line that means "we
        // answered allow without having authenticated the asker", and an
        // operator has to be able to alert on it without parsing every allow
        // out of the verdict log. Sustained UNVERIFIED_ALLOW is either a
        // misconfigured secret or somebody else talking to your endpoint, and
        // both deserve a page rather than a grep.
        eprintln!("emem-guard: UNVERIFIED_ALLOW inbound signature not verified: {why}");
        // Allow anyway: see the module docs. We are not the only control, and
        // a refusal here blocks nothing while counting against the breaker.
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
        Some(t) => g.decide(
            adapter.checkpoint_id(),
            &request_id_or_new(t.request_id),
            &t.texts,
        ),
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
    // The request id is the hook's session id, which is the same for every
    // input in a session. `decide` pairs it with a digest of the input, so
    // each input is judged on its own and only a true retry is replayed.
    let decision = match adapter.transcript(&input) {
        None => Decision::proceed(),
        Some(t) => g.decide(
            adapter.checkpoint_id(),
            &request_id_or_new(t.request_id),
            &t.texts,
        ),
    };
    (StatusCode::OK, Json(adapter.render(&decision))).into_response()
}

/// The open, vendor-neutral verdict route.
async fn native(State(g): State<Arc<Guard>>, body: axum::body::Bytes) -> impl IntoResponse {
    (StatusCode::OK, Json(g.run(&crate::open::NativeHook, &body)))
}

/// Gate an MCP tool call, or a tool result on its way back in.
async fn mcp(State(g): State<Arc<Guard>>, body: axum::body::Bytes) -> impl IntoResponse {
    let mut out = g.run(&McpGate, &body);
    // Echo the JSON-RPC id so a proxy can correlate without re-parsing the
    // request it just sent.
    out.id = serde_json::from_slice::<McpCall>(&body)
        .ok()
        .and_then(|c| c.id);
    (StatusCode::OK, Json(out))
}

/// The OpenAI-shaped route. Reach, not endorsement: see [`OpenAiGate`].
async fn openai(State(g): State<Arc<Guard>>, body: axum::body::Bytes) -> impl IntoResponse {
    let mut out = g.run(&OpenAiGate, &body);
    // Echo the caller's model name where the shape expects one, so a client
    // that logs the field sees what it asked for rather than our name for
    // ourselves.
    if let Some(m) = serde_json::from_slice::<OpenAiRequest>(&body)
        .ok()
        .and_then(|r| r.model)
    {
        out.model = m;
    }
    (StatusCode::OK, Json(out))
}

/// A CloudEvent in, a CloudEvent out.
///
/// Binary mode too: the spec puts the attributes in `ce-*` headers and the
/// data in the body, so a body that is not a structured event is read as the
/// data with the id taken from the header. Both modes reach the same engine.
async fn cloudevent(
    State(g): State<Arc<Guard>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let header = |name: &str| -> Option<String> {
        headers
            .iter()
            .find(|(k, _)| k.as_str().eq_ignore_ascii_case(name))
            .and_then(|(_, v)| v.to_str().ok())
            .map(str::to_string)
    };
    let structured = serde_json::from_slice::<CloudEvent>(&body)
        .ok()
        .filter(|e| e.specversion.is_some());
    let event = match structured {
        Some(e) => e,
        None => CloudEvent {
            specversion: header("ce-specversion"),
            id: header("ce-id"),
            event_type: header("ce-type"),
            source: header("ce-source"),
            data: serde_json::from_slice(&body).ok(),
        },
    };
    let subject = event.id.clone();
    // Not `Guard::run`: the event was assembled here from two possible
    // sources, so there is no single body left to parse. Everything after the
    // parse is the same path every other route takes.
    let decision = match CloudEventGate.transcript(&event) {
        None => Decision::proceed(),
        Some(t) => g.decide(
            CloudEventGate.checkpoint_id(),
            &request_id_or_new(t.request_id),
            &t.texts,
        ),
    };
    let mut out = CloudEventGate.render(&decision);
    out.subject = subject;
    (StatusCode::OK, Json(out))
}

/// The OPA-style policy decision point.
async fn policy_point(State(g): State<Arc<Guard>>, body: axum::body::Bytes) -> impl IntoResponse {
    (StatusCode::OK, Json(g.run(&PolicyPointGate, &body)))
}

/// Many transcripts, one request. The offline shape.
async fn batch(State(g): State<Arc<Guard>>, body: axum::body::Bytes) -> impl IntoResponse {
    let req = serde_json::from_slice::<BatchRequest>(&body).unwrap_or_default();
    let total = req.items.len();
    let adapter = crate::open::NativeHook;
    let verdicts: Vec<_> = req
        .items
        .iter()
        .take(MAX_BATCH)
        .map(|item| {
            let decision = match adapter.transcript(item) {
                None => Decision::proceed(),
                Some(t) => g.decide(
                    adapter.checkpoint_id(),
                    &request_id_or_new(t.request_id),
                    &t.texts,
                ),
            };
            adapter.render(&decision)
        })
        .collect();
    (
        StatusCode::OK,
        Json(BatchResponse {
            dropped: total.saturating_sub(verdicts.len()),
            verdicts,
        }),
    )
}

// ── the evidence half ────────────────────────────────────────────────────

/// The loaded module set, as declared and as measured.
async fn modules(State(g): State<Arc<Guard>>) -> impl IntoResponse {
    let loaded: Vec<_> = g
        .modules
        .manifests()
        .into_iter()
        .map(|m| {
            let h = g.modules.health(&m.id);
            serde_json::json!({
                "id": m.id,
                "version": m.version,
                "build_hash": m.build_hash,
                "latency_class": m.latency_class,
                "data_class": m.data_class,
                "deny_code": m.deny_code.as_str(),
                "fix": m.fix.as_str(),
                "description": m.description,
                // Measured, not declared. This is the half an operator cannot
                // get from a README.
                "observed": {
                    "calls": h.calls,
                    "last_ms": h.last_ms,
                    "max_ms": h.max_ms,
                    "budget_overruns": h.overruns,
                    "demoted": h.demoted,
                },
                "can_block": m.latency_class == crate::module::LatencyClass::Fast && !h.demoted,
            })
        })
        .collect();
    Json(serde_json::json!({
        "config_digest": g.modules.config_digest(),
        "count": loaded.len(),
        "text_visible_to_modules": g.modules_may_read_text,
        "fast_budget_ms": crate::module::FAST_BUDGET.as_millis(),
        "modules": loaded,
        "rules": {
            "slow_never_blocks": "a module declaring latency_class slow is evaluated in shadow only and can never deny",
            "fast_is_measured": format!(
                "a module declaring fast that exceeds {} ms {} times is demoted to shadow and stops being able to block",
                crate::module::FAST_BUDGET.as_millis(),
                crate::module::FAST_STRIKES
            ),
            "abstain_never_blocks": "a module that timed out or could not decide has not told us to deny",
            "no_content_logged": "the log records module id, version and a keyed evidence digest, never what matched",
        },
    }))
}

/// The chain head: what a witness or a mirror pins.
async fn log_head(State(g): State<Arc<Guard>>) -> impl IntoResponse {
    let (seq, chain) = g.log.head();
    Json(serde_json::json!({
        "entries": seq,
        "head": chain,
        "signer_b32": g.log.signer_b32(),
        "preimage_domain": crate::log::PREIMAGE_DOMAIN,
    }))
}

/// One entry, by the leaf id a denial named.
///
/// Accepts `leaf_7` and `7`, because the reason line carries the first form
/// and a person reading a report has the second.
async fn log_entry(
    State(g): State<Arc<Guard>>,
    axum::extract::Path(leaf): axum::extract::Path<String>,
) -> impl IntoResponse {
    let Some(seq) = leaf
        .strip_prefix("leaf_")
        .unwrap_or(&leaf)
        .parse::<u64>()
        .ok()
    else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "leaf must be leaf_<n> or <n>"})),
        )
            .into_response();
    };
    match g.log.entry(seq) {
        Some(v) => (StatusCode::OK, Json(serde_json::json!(v))).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "no such leaf", "leaf": leaf})),
        )
            .into_response(),
    }
}

/// A window of entries, so a mirror can pull the log without the node's help.
#[derive(serde::Deserialize)]
pub struct EntriesQuery {
    #[serde(default)]
    start: u64,
    #[serde(default = "default_count")]
    count: usize,
}

fn default_count() -> usize {
    50
}

/// The largest window one request may pull.
///
/// A cap rather than a page size: an unbounded range over a long-lived log
/// would let any caller make the node read its whole history into memory.
pub const MAX_ENTRIES_PER_REQUEST: usize = 500;

async fn log_entries(
    State(g): State<Arc<Guard>>,
    axum::extract::Query(q): axum::extract::Query<EntriesQuery>,
) -> impl IntoResponse {
    let entries = g.log.range(q.start, q.count, MAX_ENTRIES_PER_REQUEST);
    let next = entries.last().map(|e| e.seq + 1);
    Json(serde_json::json!({
        "start": q.start,
        "returned": entries.len(),
        "next_start": next,
        "max_per_request": MAX_ENTRIES_PER_REQUEST,
        "entries": entries,
    }))
}

/// What this node has actually done, counted from the log.
///
/// The number an organisation decides on before enforcing a rule, and it comes
/// from the same file an auditor would read rather than from a counter in
/// process memory that resets on restart and nobody else can check.
async fn log_report(State(g): State<Arc<Guard>>) -> impl IntoResponse {
    match crate::store::ShadowReport::read(g.log.path()) {
        Ok(r) => Json(serde_json::json!({
            "entries": r.entries,
            "enforced_denies": r.enforced_denies,
            "observed_denies": r.observed_denies,
            "fired_rate": r.fired_rate(),
            "citations_checked": r.citations_checked,
            "by_code": r.by_code,
            "by_checkpoint": r.by_checkpoint,
            "first_unix_s": r.first_unix_s,
            "last_unix_s": r.last_unix_s,
            "mode": g.config.mode.as_str(),
        }))
        .into_response(),
        // An empty log is not an error, it is a node that has answered
        // nothing yet, and saying so is more useful than a 500.
        Err(_) => Json(serde_json::json!({
            "entries": 0,
            "enforced_denies": 0,
            "observed_denies": 0,
            "fired_rate": 0.0,
            "mode": g.config.mode.as_str(),
        }))
        .into_response(),
    }
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
            resolver: Box::new(crate::resolve::NullResolver),
            restricted_cells: Default::default(),
            modules: crate::module::ModuleRegistry::new(),
            modules_may_read_text: true,
            log: FileLog::open(&p, key.verifying_key()).unwrap(),
            signing: key,
            digest_key: [3u8; 32],
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

    /// The interoperability commitment, asserted at the level that matters:
    /// not that the adapters agree in a unit test, but that the SERVER answers
    /// the same way through every door.
    ///
    /// A guard that denied through one route and allowed through another would
    /// be discriminating between agents on the basis of the client they
    /// happened to hold.
    #[tokio::test]
    async fn every_route_reaches_the_same_verdict_on_the_same_evidence() {
        // A bare node resolves nothing, so this asserts the allow path across
        // every shape. What matters is that all of them answer, in their own
        // envelope, with a verdict that reads as allow.
        /// A route, a body to send it, and how to read an allow out of its
        /// own envelope.
        type RouteCase<'a> = (&'a str, &'a str, &'a dyn Fn(&serde_json::Value) -> bool);
        let cases: [RouteCase; 7] = [
            (
                "/verdict",
                r#"{"texts":["per emem:fact:a:b it is 918 m"]}"#,
                &|v| v["action"] == "allow",
            ),
            (
                "/verdict/native",
                r#"{"messages":[{"role":"user","content":"emem:fact:a:b"}]}"#,
                &|v| v["action"] == "allow",
            ),
            (
                "/verdict/mcp",
                r#"{"method":"tools/call","params":{"name":"t","arguments":{"q":"emem:fact:a:b"}}}"#,
                &|v| v["allow"] == true && v["verdict"]["action"] == "allow",
            ),
            ("/verdict/openai", r#"{"input":"emem:fact:a:b"}"#, &|v| {
                v["results"][0]["flagged"] == false
            }),
            (
                "/verdict/cloudevent",
                r#"{"specversion":"1.0","id":"e1","type":"t","source":"/a","data":{"t":"emem:fact:a:b"}}"#,
                &|v| v["type"] == "dev.emem.guard.verdict.allow",
            ),
            (
                "/verdict/policy",
                r#"{"input":{"prompt":"emem:fact:a:b"}}"#,
                &|v| {
                    v["result"]["allow"] == true
                        && v["result"]["deny"].as_array().unwrap().is_empty()
                },
            ),
            (
                "/verdict/batch",
                r#"{"items":[{"texts":["emem:fact:a:b"]},{"texts":["nothing"]}]}"#,
                &|v| v["verdicts"].as_array().unwrap().len() == 2 && v["dropped"] == 0,
            ),
        ];
        for (path, body, ok) in cases {
            let (status, v) = post(router(guard("routes")), path, body).await;
            assert_eq!(status, StatusCode::OK, "{path}");
            assert!(ok(&v), "{path} answered {v}");
        }
    }

    /// No route may produce a webhook failure, whatever arrives. The rule was
    /// written for the Anthropic checkpoint and it holds for all of them,
    /// because an unparseable body is our lag far more often than an attack.
    #[tokio::test]
    async fn no_route_fails_on_a_body_it_cannot_read() {
        for path in [
            "/verdict",
            "/verdict/mcp",
            "/verdict/openai",
            "/verdict/cloudevent",
            "/verdict/policy",
            "/verdict/batch",
            "/verdict/anthropic-hook",
            "/verdict/claude-code",
        ] {
            for body in [
                "}{ not json",
                "",
                "[1,2,3]",
                r#"{"unexpected":{"deep":[1]}}"#,
            ] {
                let (status, v) = post(router(guard("robust")), path, body).await;
                assert_eq!(status, StatusCode::OK, "{path} on {body:?}");
                assert!(!v.is_null(), "{path} on {body:?} returned no verdict");
            }
        }
    }

    /// A batch cannot be used to make one request do unbounded work, and the
    /// truncation is stated rather than silent: a caller who sent more than the
    /// cap must not read the short answer as a clean run.
    #[tokio::test]
    async fn an_oversized_batch_says_what_it_dropped() {
        let items: Vec<String> = (0..MAX_BATCH + 10)
            .map(|i| format!(r#"{{"texts":["item {i}"]}}"#))
            .collect();
        let body = format!(r#"{{"items":[{}]}}"#, items.join(","));
        let (status, v) = post(router(guard("batchcap")), "/verdict/batch", &body).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["verdicts"].as_array().unwrap().len(), MAX_BATCH);
        assert_eq!(v["dropped"], 10);
    }

    /// A denial names a leaf. Without a route to fetch it, that name is a
    /// promise of verifiability rather than verifiability.
    #[tokio::test]
    async fn the_leaf_a_denial_names_can_actually_be_fetched_and_checked() {
        let g = guard("leaf");
        // Any verdict writes a leaf; an allow is enough to test the retrieval
        // path, and the signature check below is the part that matters.
        let (_, v) = post(
            router(g.clone()),
            "/verdict",
            r#"{"texts":["hello"],"request_id":"r-leaf"}"#,
        )
        .await;
        let leaf = v["leaf"].as_str().expect("every verdict is logged");

        let app = router(g.clone());
        let res = app
            .oneshot(
                Request::builder()
                    .uri(format!("/log/entry/{leaf}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let entry: crate::store::LoggedVerdict = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(entry.record.request_id, "r-leaf");

        // And the fetched entry verifies against the key it names, with
        // nothing taken from the node that served it.
        use ed25519_dalek::Verifier as _;
        let pk: [u8; 32] = data_encoding::BASE32_NOPAD
            .decode(entry.signer_b32.to_uppercase().as_bytes())
            .unwrap()
            .try_into()
            .unwrap();
        let sig: [u8; 64] = data_encoding::BASE32_NOPAD
            .decode(entry.signature_b32.to_uppercase().as_bytes())
            .unwrap()
            .try_into()
            .unwrap();
        ed25519_dalek::VerifyingKey::from_bytes(&pk)
            .unwrap()
            .verify(
                &entry.record.preimage(),
                &ed25519_dalek::Signature::from_bytes(&sig),
            )
            .expect("the leaf a denial names must verify offline");

        // A leaf that does not exist is a 404, not a fabricated entry.
        let app = router(g);
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/log/entry/leaf_9999")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    /// The descriptor is how a cold agent integrates without being handed a
    /// README by a person. Every route it advertises has to exist.
    #[tokio::test]
    async fn the_descriptor_advertises_only_routes_that_answer() {
        let g = guard("descriptor");
        let app = router(g.clone());
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/.well-known/emem-guard.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let d: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        let checkpoints = d["checkpoints"].as_array().unwrap();
        assert!(checkpoints.len() >= 9, "{} advertised", checkpoints.len());
        for cp in checkpoints {
            let route = cp["route"].as_str().unwrap();
            let (status, _) = post(router(g.clone()), route, "{}").await;
            assert_eq!(
                status,
                StatusCode::OK,
                "{route} is advertised but does not answer"
            );
        }
        // At least as many open routes as vendor ones, which is the posture
        // this crate is committing to rather than merely describing.
        let open = checkpoints.iter().filter(|c| c["open"] == true).count();
        assert!(
            open > checkpoints.len() - open,
            "{open} open of {} total",
            checkpoints.len()
        );
        // The grammar an agent parses is published, not folklore.
        assert!(d["reason_grammar"]
            .as_str()
            .unwrap()
            .starts_with("EMEM-GUARD DENY"));
        // Counted off the enum, not off a number someone typed. Both of these
        // were literals until 2026-08-09, and POLICY_MODULE and
        // redact_and_retry had gone unpublished behind them the whole time.
        assert_eq!(
            d["deny_codes"].as_array().unwrap().len(),
            crate::policy::DenyCode::ALL.len()
        );
        assert_eq!(
            d["fixes"].as_array().unwrap().len(),
            crate::policy::Fix::ALL.len()
        );
    }

    /// Shadow: every rule runs, every verdict is signed and logged with what
    /// it would have done, and nobody is blocked. The report reads it back.
    #[tokio::test]
    async fn a_shadow_node_records_what_it_would_have_blocked_and_blocks_nothing() {
        let mut g = guard("shadow");
        {
            let m = Arc::get_mut(&mut g).unwrap();
            m.config.claim_gating = true;
            m.config.mode = policy::Mode::Shadow;
        }

        let (status, v) = post(
            router(g.clone()),
            "/verdict",
            r#"{"texts":["Elevation in Leh is 3500 m."],"request_id":"r-shadow"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["action"], "allow", "shadow blocks nobody");
        assert_eq!(v["shadowed"], true, "and says it would have");
        assert_eq!(v["code"], "CLAIM_UNGROUNDED");
        assert_eq!(v["fix"], "cite_observation");
        assert_eq!(
            v["claim"]["source_band"], "copdem30m.elevation_mean",
            "and names a band recall will actually serve, not the `dem` family root"
        );

        // The count comes off disk, from the same file an auditor reads.
        let r = crate::store::ShadowReport::read(g.log.path()).unwrap();
        assert_eq!(r.entries, 1);
        assert_eq!(r.enforced_denies, 0, "nothing was blocked");
        assert_eq!(r.observed_denies, 1, "and one would have been");
        assert_eq!(r.by_code["CLAIM_UNGROUNDED"], (0, 1));
    }

    /// The same node with the same rule, enforcing. Same evidence, same code,
    /// different answer, which is the only difference shadow is allowed to
    /// make.
    #[tokio::test]
    async fn the_same_transcript_is_denied_once_the_node_enforces() {
        let mut g = guard("enforce");
        Arc::get_mut(&mut g).unwrap().config.claim_gating = true;
        let (status, v) = post(
            router(g.clone()),
            "/verdict",
            r#"{"texts":["Elevation in Leh is 3500 m."],"request_id":"r-enf"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "a deny is still a 200");
        assert_eq!(v["action"], "deny");
        assert!(v["shadowed"].is_null(), "not shadowed: it was acted on");
        let reason = v["reason"].as_str().unwrap();
        assert!(
            reason.starts_with("EMEM-GUARD DENY CLAIM_UNGROUNDED"),
            "{reason}"
        );
        assert!(reason.contains("fix=cite_observation"), "{reason}");

        let r = crate::store::ShadowReport::read(g.log.path()).unwrap();
        assert_eq!((r.enforced_denies, r.observed_denies), (1, 0));
    }

    /// A retry must get the answer already acted on, even across a mode
    /// change. A node switched from shadow to enforcing between a request and
    /// its retry must not turn an allow the caller already received into a
    /// block.
    #[tokio::test]
    async fn a_replay_gets_the_mode_it_was_originally_decided_under() {
        let mut g = guard("replaymode");
        {
            let m = Arc::get_mut(&mut g).unwrap();
            m.config.claim_gating = true;
            m.config.mode = policy::Mode::Shadow;
        }
        let body = r#"{"texts":["Elevation in Leh is 3500 m."],"request_id":"r-mode"}"#;
        let (_, first) = post(router(g.clone()), "/verdict", body).await;
        assert_eq!(first["action"], "allow");

        // Flip to enforcing and replay the same id.
        Arc::get_mut(&mut g).unwrap().config.mode = policy::Mode::Enforce;
        let (_, again) = post(router(g.clone()), "/verdict", body).await;
        assert_eq!(
            again["action"], "allow",
            "the recorded verdict wins over the current config"
        );
        assert_eq!(again["leaf"], first["leaf"], "and it is the same entry");
    }

    /// A Claude Code hook sends its session id as the request id, so every
    /// input in a session arrives under one id. The second input must be
    /// judged on its own. Until 2026-11-12 the session's first verdict was
    /// replayed for every later input, so a clean first message let a
    /// restricted cell through.
    #[tokio::test]
    async fn a_session_id_never_replays_a_verdict_for_a_different_input() {
        let cell = "defi.zb493.xuqA.zcb5f";
        let mut g = guard("cc-session");
        {
            let m = Arc::get_mut(&mut g).unwrap();
            m.restricted_cells.insert(cell.to_string());
            m.config.geo_restriction = true;
        }
        let clean = r#"{"hook_event_name":"UserPromptSubmit","session_id":"s-1","prompt":"hello"}"#;
        let restricted = format!(
            r#"{{"hook_event_name":"UserPromptSubmit","session_id":"s-1","prompt":"go to emem:cell:{cell}"}}"#
        );

        let (_, first) = post(router(g.clone()), "/verdict/claude-code", clean).await;
        assert_eq!(first["ok"], true, "{first}");
        let (_, second) = post(router(g.clone()), "/verdict/claude-code", &restricted).await;
        assert_eq!(second["ok"], false, "judged on its own: {second}");
        let reason = second["reason"].as_str().unwrap();
        assert!(reason.starts_with("EMEM-GUARD DENY GEO_ZONE"), "{reason}");
        assert_eq!(g.log.len(), 2, "two inputs, two verdicts");

        // The same input again is a retry, and gets the recorded verdict.
        let (_, again) = post(router(g.clone()), "/verdict/claude-code", &restricted).await;
        assert_eq!(again, second);
        assert_eq!(g.log.len(), 2, "a retry writes no new verdict");

        // Same for a PreToolUse event in the same session.
        let tool = format!(
            r#"{{"hook_event_name":"PreToolUse","session_id":"s-1","tool_name":"Bash","tool_input":{{"command":"echo emem:cell:{cell}"}}}}"#
        );
        let (_, v) = post(router(g.clone()), "/verdict/claude-code", &tool).await;
        assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny", "{v}");
    }

    /// Any route: one request id with a different input is a new request.
    #[tokio::test]
    async fn a_reused_request_id_with_new_text_is_judged_again() {
        let mut g = guard("reused-id");
        Arc::get_mut(&mut g).unwrap().config.claim_gating = true;
        let (_, a) = post(
            router(g.clone()),
            "/verdict",
            r#"{"texts":["hello"],"request_id":"r-reused"}"#,
        )
        .await;
        assert_eq!(a["action"], "allow");
        let (_, b) = post(
            router(g.clone()),
            "/verdict",
            r#"{"texts":["Elevation in Leh is 3500 m."],"request_id":"r-reused"}"#,
        )
        .await;
        assert_eq!(b["action"], "deny", "{b}");
        assert_ne!(a["leaf"], b["leaf"]);
    }

    /// A request with no id, or an empty one, never shares a verdict with
    /// another request. The vendor routes once mapped a missing id to `""`,
    /// so every such request shared one id.
    #[tokio::test]
    async fn requests_without_an_id_never_share_a_verdict() {
        let cell = "defi.zb493.xuqA.zcb5f";
        let mut g = guard("no-id");
        {
            let m = Arc::get_mut(&mut g).unwrap();
            m.restricted_cells.insert(cell.to_string());
            m.config.geo_restriction = true;
        }
        let frame = |id: &str, text: &str| {
            format!(
                r#"{{"type":"prompt",{id}"messages":[{{"role":"user","content":[{{"type":"text","text":"{text}"}}]}}]}}"#
            )
        };
        let clean = frame("", "hello");
        let restricted = frame("", &format!("go to emem:cell:{cell}"));
        let (_, a) = post(router(g.clone()), "/verdict/anthropic-hook", &clean).await;
        assert_eq!(a["action"], "allow");
        let (_, b) = post(router(g.clone()), "/verdict/anthropic-hook", &restricted).await;
        assert_eq!(b["action"], "deny", "judged on its own: {b}");

        // Not even the same text without an id is a replay: nothing names
        // the earlier request, so each one gets its own entry.
        post(router(g.clone()), "/verdict/anthropic-hook", &clean).await;
        let empty_id = frame(r#""request_id":"","#, "hello");
        post(router(g.clone()), "/verdict/anthropic-hook", &empty_id).await;
        assert_eq!(g.log.len(), 4);
        let ids: std::collections::HashSet<_> = (0..4)
            .map(|i| g.log.entry(i).unwrap().record.request_id)
            .collect();
        assert_eq!(ids.len(), 4, "every request got its own id: {ids:?}");
        assert!(!ids.contains(""));

        // And on the Claude Code route, an input with no session id.
        let no_session = r#"{"hook_event_name":"UserPromptSubmit","prompt":"hello"}"#;
        post(router(g.clone()), "/verdict/claude-code", no_session).await;
        post(router(g.clone()), "/verdict/claude-code", no_session).await;
        assert_eq!(g.log.len(), 6);
        assert_ne!(
            g.log.entry(4).unwrap().record.request_id,
            g.log.entry(5).unwrap().record.request_id
        );
    }

    /// The claim gate must be off unless an operator turned it on, because it
    /// is the one rule that denies on absence.
    #[tokio::test]
    async fn a_default_node_never_denies_an_uncited_claim() {
        let (status, v) = post(
            router(guard("claimoff")),
            "/verdict",
            r#"{"texts":["Elevation in Leh is 3500 m."]}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["action"], "allow");
        assert!(v["code"].is_null(), "no rule fired at all");
    }

    /// The chassis working end to end: a loaded module denies, the verdict
    /// carries POLICY_MODULE and the module's declared fix, and the log
    /// records who found it without recording what they found.
    #[tokio::test]
    async fn a_loaded_module_denies_and_the_log_names_it_without_the_content() {
        let mut g = guard("modrun");
        {
            let m = Arc::get_mut(&mut g).unwrap();
            m.modules
                .load(Box::new(crate::modules::SecretPatterns::new()))
                .unwrap();
        }
        // AWS's documented example key, split so secret scanners skip the source.
        let secret = concat!("AKIA", "IOSFODNN7EXAMPLE");
        let body = format!(r#"{{"texts":["deploy with {secret} please"],"request_id":"r-mod"}}"#);
        let (status, v) = post(router(g.clone()), "/verdict", &body).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["action"], "deny");
        assert_eq!(v["code"], "POLICY_MODULE");
        assert_eq!(v["fix"], "redact_and_retry", "the module's declared remedy");

        // The reason line is the same fixed grammar every other denial uses:
        // no new field was grown for modules.
        let reason = v["reason"].as_str().unwrap();
        assert!(
            reason.starts_with("EMEM-GUARD DENY POLICY_MODULE token=-"),
            "{reason}"
        );

        // The log names the module and hashes the finding.
        let entry = g.log.entry(0).expect("the verdict was logged");
        assert_eq!(
            entry
                .record
                .module
                .as_deref()
                .map(|m| m.split('@').next().unwrap()),
            Some("secret-patterns")
        );
        assert_eq!(entry.record.evidence_digest.as_ref().unwrap().len(), 64);
        let whole = serde_json::to_string(&entry).unwrap();
        assert!(
            !whole.contains(secret) && !whole.contains("AKIA"),
            "the credential reached the log: {whole}"
        );
        // And the verdict names the module set that produced it, or it would
        // not be reproducible.
        assert_eq!(entry.record.config_digest, g.modules.config_digest());
        assert!(!entry.record.config_digest.is_empty());

        // The digest is keyed. `/log/entries` needs no auth, and an unkeyed
        // BLAKE3 of the match let anyone confirm a guessed secret: hash the
        // guess, look for it. A node with another key logs another digest.
        let logged = entry.record.evidence_digest.clone().unwrap();
        let plain = blake3::hash(secret.as_bytes()).to_hex().to_string();
        assert_ne!(logged, plain, "an unkeyed digest is a guess test");
        let mut other = guard("modrun-otherkey");
        {
            let m = Arc::get_mut(&mut other).unwrap();
            m.digest_key = [4u8; 32];
            m.modules
                .load(Box::new(crate::modules::SecretPatterns::new()))
                .unwrap();
        }
        post(router(other.clone()), "/verdict", &body).await;
        let elsewhere = other.log.entry(0).unwrap().record.evidence_digest;
        assert_ne!(elsewhere.as_deref(), Some(logged.as_str()));
    }

    /// The open log must not list the operator's restricted cells. Until
    /// 2026-11-12 a GEO_ZONE record held the cell in `token`, so
    /// `/log/entries` published the restricted list one hit at a time. The
    /// caller is still told which cell it cited, and a retry still gets it.
    #[tokio::test]
    async fn a_geo_zone_log_entry_holds_no_cell() {
        let cell = "defi.zb493.xuqA.zcb5f";
        let mut g = guard("geo-redact");
        {
            let m = Arc::get_mut(&mut g).unwrap();
            m.restricted_cells.insert(cell.to_string());
            m.config.geo_restriction = true;
        }
        let body = format!(r#"{{"texts":["survey emem:cell:{cell} today"],"request_id":"r-geo"}}"#);
        let (_, v) = post(router(g.clone()), "/verdict", &body).await;
        assert_eq!(v["code"], "GEO_ZONE");
        assert_eq!(v["token"], cell, "the caller is told");

        let entry = g.log.entry(0).unwrap();
        assert!(entry.record.token.is_none());
        assert_eq!(
            entry.record.evidence_digest.as_deref(),
            Some(g.cell_digest(cell).as_str()),
            "the operator, who holds the key, can map the digest to the cell"
        );
        assert!(
            !serde_json::to_string(&entry).unwrap().contains(cell),
            "the restricted cell reached the open log"
        );
        // The signed record still verifies with the cell left out.
        assert!(FileLog::audit(g.log.path()).unwrap().is_intact());

        // A retry carries the same input, so it is told the cell again.
        let (_, again) = post(router(g.clone()), "/verdict", &body).await;
        assert_eq!(again, v);
        assert_eq!(g.log.len(), 1);
    }

    /// The input digest in the open log is keyed too: a short prompt cannot
    /// be confirmed by hashing a guess.
    #[tokio::test]
    async fn the_logged_input_digest_is_keyed() {
        let g = guard("input-keyed");
        post(
            router(g.clone()),
            "/verdict",
            r#"{"texts":["hello"],"request_id":"r-in"}"#,
        )
        .await;
        let logged = g.log.entry(0).unwrap().record.input_digest.unwrap();
        assert_eq!(logged, g.input_digest(&["hello"]));
        assert_ne!(logged, blake3::hash(b"hello").to_hex().to_string());
    }

    /// A resolver that gives every citation one status.
    struct FixedStatus(policy::TokenStatus);

    impl crate::resolve::Resolver for FixedStatus {
        fn resolve(
            &self,
            _token: &tokens::FoundToken,
            _budget: Duration,
        ) -> crate::resolve::Resolution {
            crate::resolve::Resolution::bare(self.0.clone())
        }
        fn describe(&self) -> String {
            "fixed-status".into()
        }
    }

    /// A guard whose resolver answers `status`, with `cell` restricted.
    fn guard_resolving(dir: &str, status: policy::TokenStatus, cell: &str) -> Arc<Guard> {
        let mut g = guard(dir);
        {
            let m = Arc::get_mut(&mut g).unwrap();
            m.resolver = Box::new(FixedStatus(status));
            m.restricted_cells.insert(cell.to_string());
            m.config.geo_restriction = true;
        }
        g
    }

    /// Any code, not only GEO_ZONE, can carry a restricted cell in its token:
    /// `emem:fact:<cell>:<cid>`. PROV_SIG and PROV_BYTES outrank GEO_ZONE, so
    /// they are the code the log gets for a bad citation in a restricted
    /// cell. The log must still not name the cell, and a retry must still
    /// get the token.
    #[tokio::test]
    async fn a_provenance_deny_in_a_restricted_cell_logs_no_cell() {
        let cell = "defi.zb493.xuqA.zcb5f";
        let token =
            format!("emem:fact:{cell}:yqbolgeoycqkvj3zkxukb4bjw4odhpwvfzqo3fbgwf4spk45zala");
        let body = format!(r#"{{"texts":["per {token} it is 918 m"],"request_id":"r-prov"}}"#);
        for (status, code) in [
            (policy::TokenStatus::SignatureFailed, "PROV_SIG"),
            (policy::TokenStatus::ByteMismatch, "PROV_BYTES"),
        ] {
            let g = guard_resolving(&format!("prov-redact-{code}"), status, cell);
            let (_, v) = post(router(g.clone()), "/verdict", &body).await;
            assert_eq!(v["code"], code, "{v}");
            assert_eq!(v["token"], token, "the caller is told the token");

            let entry = g.log.entry(0).unwrap();
            assert!(entry.record.token.is_none(), "{code}");
            assert_eq!(
                entry.record.evidence_digest.as_deref(),
                Some(g.cell_digest(cell).as_str())
            );
            assert!(
                !serde_json::to_string(&entry).unwrap().contains(cell),
                "{code}: the restricted cell reached the open log"
            );
            assert!(FileLog::audit(g.log.path()).unwrap().is_intact());

            let (_, again) = post(router(g.clone()), "/verdict", &body).await;
            assert_eq!(again, v, "{code}: a retry gets the same reply");
            assert_eq!(g.log.len(), 1);
        }

        // Control: the same deny on a cell nobody restricted keeps its token.
        let open = guard_resolving(
            "prov-redact-open",
            policy::TokenStatus::SignatureFailed,
            "defi.zb493.hufo.zccc6",
        );
        post(router(open.clone()), "/verdict", &body).await;
        let entry = open.log.entry(0).unwrap();
        assert_eq!(entry.record.token.as_deref(), Some(token.as_str()));
        assert!(entry.record.evidence_digest.is_none());

        // Two different tokens in the one restricted cell: the digest cannot
        // say which one the rule named, so a retry names none rather than a
        // wrong one. The rest of the verdict is the same.
        let g = guard_resolving(
            "prov-redact-two",
            policy::TokenStatus::SignatureFailed,
            cell,
        );
        let two = format!(
            r#"{{"texts":["{token} and emem:fact:{cell}:cj4ccplttk6oyp5sr7rn76igvqq7mno2b6z4kqxbztcjemsisdya"],"request_id":"r-two"}}"#
        );
        let (_, first) = post(router(g.clone()), "/verdict", &two).await;
        let (_, again) = post(router(g.clone()), "/verdict", &two).await;
        assert_eq!(first["token"], token);
        assert!(again["token"].is_null(), "{again}");
        assert_eq!(
            (&again["code"], &again["fix"], &again["leaf"]),
            (&first["code"], &first["fix"], &first["leaf"])
        );
    }

    /// A cryptographic failure is a stronger statement than a pattern match,
    /// so it is the one reported when both fire.
    #[tokio::test]
    async fn an_established_provenance_failure_outranks_a_module_finding() {
        let ev = Evidence {
            tokens: vec![(
                crate::tokens::FoundToken {
                    token: String::from("emem:fact:a:b"),
                    kind: crate::tokens::TokenKind::Fact,
                },
                policy::TokenStatus::SignatureFailed,
            )],
            modules: vec![(
                crate::module::PolicyModule::manifest(&crate::modules::SecretPatterns::new())
                    .clone(),
                crate::module::ModuleVerdict::deny("x", b"y", 1.0),
            )],
            ..Default::default()
        };
        assert_eq!(
            policy::evaluate(&Config::default(), &ev).code,
            Some(policy::DenyCode::ProvSig)
        );
    }

    /// The descriptor and /modules have to agree with what is loaded, or an
    /// operator is reading a promise rather than a state.
    #[tokio::test]
    async fn the_modules_route_reports_what_is_loaded_and_what_it_cost() {
        let mut g = guard("modlist");
        Arc::get_mut(&mut g)
            .unwrap()
            .modules
            .load(Box::new(crate::modules::SecretPatterns::new()))
            .unwrap();
        // One call, so there is something measured to report.
        post(
            router(g.clone()),
            "/verdict",
            r#"{"texts":["nothing here"]}"#,
        )
        .await;

        let app = router(g.clone());
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/modules")
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
        assert_eq!(v["count"], 1);
        assert_eq!(v["config_digest"], g.modules.config_digest());
        let m = &v["modules"][0];
        assert_eq!(m["id"], "secret-patterns");
        assert_eq!(m["latency_class"], "fast");
        assert_eq!(m["can_block"], true);
        assert_eq!(m["observed"]["calls"], 1, "measured, not declared");
        assert_eq!(m["observed"]["demoted"], false);
    }

    /// A node with no modules is the default, and it must still produce a
    /// config digest: "nothing loaded" has to be a signed statement rather
    /// than an absence somebody could introduce later.
    #[tokio::test]
    async fn a_node_with_no_modules_still_binds_its_empty_pipeline() {
        let g = guard("nomod");
        post(
            router(g.clone()),
            "/verdict",
            r#"{"texts":["hi"],"request_id":"r-none"}"#,
        )
        .await;
        let entry = g.log.entry(0).unwrap();
        assert!(!entry.record.config_digest.is_empty());
        assert_eq!(entry.record.config_digest, g.modules.config_digest());
        assert!(entry.record.module.is_none());
    }

    /// The regression the conformance suite found: this server documented
    /// 10 MB, carried a tower-http limit layer set to it, and still answered
    /// 413 on anything past 2 MB, because axum's own DefaultBodyLimit wins.
    ///
    /// Under a fail-open policy that turns every oversized transcript into an
    /// uninspected request, which is the exact failure the whole crate exists
    /// to avoid, produced by a default nobody set.
    #[tokio::test]
    async fn a_body_up_to_the_documented_ceiling_is_accepted() {
        // Just under the ceiling, and far past every default that would
        // silently truncate it.
        let filler = "x".repeat(9 * 1024 * 1024);
        let body = format!(r#"{{"texts":["{filler}"]}}"#);
        assert!(body.len() > 9_000_000 && body.len() < MAX_BODY_BYTES);
        let (status, v) = post(router(guard("bigbody")), "/verdict", &body).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "a {} byte body was rejected under a {} byte ceiling",
            body.len(),
            MAX_BODY_BYTES
        );
        assert_eq!(v["action"], "allow");
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
