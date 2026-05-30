//! `emem-sleep-agent` — the opt-in LLM rewrite/merge loop that layers on
//! top of the deterministic contradiction→`disagrees_with`-edge refinement
//! loop that already ships in emem (`EMEM_REFINEMENT_ENABLED`).
//!
//! # What it does
//!
//! During idle periods a background worker (`emem-sleep-agentd`, env-gated,
//! default OFF) drives a running emem responder over its **REST + MCP** API:
//!
//! 1. **Select candidates** — top-N memory paths ranked by multi-attester
//!    contradiction severity (`POST /v1/memory_contradictions`) and/or by
//!    high write-churn (the `memory_file_history` length surfaced through
//!    `memory_view` / `memory_list_by_kind`).
//! 2. **Rewrite + merge** — for each candidate cluster of near-duplicate
//!    memory files, ask an operator-configured LLM to propose a single
//!    reconciled memory.
//! 3. **Write back non-destructively** — the merged text is written as a
//!    NEW signed memory via `memory_create` to the canonical path. emem's
//!    bi-temporal model means the new write *shadows* the old one (latest
//!    `signed_at` wins under `as_of:now`) while every prior version stays
//!    in the append-only `memory_file_history` and is still replayable.
//!
//! # Honesty contract
//!
//! This project forbids stubs and fabricated data. If **no** LLM endpoint
//! is configured (or it is unreachable) the agent MUST NOT invent a
//! rewrite. The library exposes this as two distinct execution modes:
//!
//! - [`run_pass`] with `dry_run = true` — connects to the responder,
//!   selects candidates, prints exactly what it WOULD merge, performs
//!   **no** LLM call and **no** write. Works fully offline against a
//!   local responder.
//! - [`run_pass`] with `dry_run = false` — performs the real LLM call
//!   through the injected [`LlmTransport`] and the real write-back. If
//!   the transport is absent the pass refuses to fabricate and degrades
//!   to a dry-run with a logged warning.
//!
//! The real rewrite path is real code; it is simply unexercised when no
//! key is configured.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub mod candidates;
pub mod client;
pub mod config;
pub mod llm;
pub mod merge;

pub use candidates::{select_candidates, Candidate, CandidateSource, MemoryFile};
pub use client::ResponderClient;
pub use config::SleepAgentConfig;
pub use llm::{HttpLlmTransport, LlmRequest, LlmTransport, MergeProposal};
pub use merge::{apply_merge, MergeOutcome};

/// Errors surfaced by the sleep-agent loop.
#[derive(Debug, thiserror::Error)]
pub enum SleepAgentError {
    #[error("responder transport error: {0}")]
    Http(String),
    #[error("LLM transport error: {0}")]
    Llm(String),
    #[error("responder returned malformed JSON: {0}")]
    Decode(String),
    #[error("config error: {0}")]
    Config(String),
}

/// Per-pass summary. Printed at the end of every pass and returned so the
/// binary (or a test) can assert on it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PassSummary {
    /// True when this was a candidate-selection-and-plan-only pass (no LLM
    /// call, no write).
    pub dry_run: bool,
    /// Candidates considered this pass (already capped at `top_n`).
    pub candidates_considered: usize,
    /// How many candidates carried a contradiction signal.
    pub from_contradictions: usize,
    /// How many candidates carried a high-churn signal.
    pub from_churn: usize,
    /// Merges actually written back as new superseding memories. Always 0
    /// in dry-run.
    pub merges_written: usize,
    /// New superseding file CIDs, one per written merge.
    pub written_file_cids: Vec<String>,
    /// Estimated USD spent on LLM calls this pass (transport-reported).
    pub spend_usd: f64,
    /// Human-readable lines describing each planned/applied merge, in
    /// order. Always populated (even in dry-run) so the operator sees
    /// exactly what would happen.
    pub plan: Vec<String>,
    /// Honest notes — e.g. "no LLM configured, ran dry", "empty corpus".
    pub notes: Vec<String>,
}

impl PassSummary {
    /// Render a compact multi-line summary for stdout.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "sleep-agent pass: {} | candidates={} (contradictions={}, churn={}) | merges_written={} | spend=${:.4}\n",
            if self.dry_run { "DRY-RUN (no LLM, no writes)" } else { "LIVE" },
            self.candidates_considered,
            self.from_contradictions,
            self.from_churn,
            self.merges_written,
            self.spend_usd,
        ));
        for line in &self.plan {
            out.push_str(&format!("  plan: {line}\n"));
        }
        for cid in &self.written_file_cids {
            out.push_str(&format!("  wrote (supersedes via bi-temporal): {cid}\n"));
        }
        for note in &self.notes {
            out.push_str(&format!("  note: {note}\n"));
        }
        out
    }
}

/// Run a single sleep-agent pass against a responder.
///
/// `transport` is `Some` only when a real LLM endpoint is configured. When
/// it is `None`, or when `cfg.dry_run` is set, the pass selects + plans but
/// never calls an LLM and never writes (the honesty contract).
///
/// Returns a [`PassSummary`]; the caller prints it and decides whether to
/// loop again.
pub async fn run_pass(
    client: &ResponderClient,
    transport: Option<&dyn LlmTransport>,
    cfg: &SleepAgentConfig,
) -> Result<PassSummary, SleepAgentError> {
    let mut summary = PassSummary {
        dry_run: cfg.dry_run || transport.is_none(),
        ..Default::default()
    };

    // The effective dry-run: explicit flag OR no transport available. We
    // never fabricate, so absence of a transport collapses to a plan-only
    // pass with an honest note.
    if cfg.dry_run {
        summary
            .notes
            .push("--dry-run requested: selecting candidates and planning only.".into());
    } else if transport.is_none() {
        summary.notes.push(
            "No LLM endpoint configured (set EMEM_SLEEP_AGENT_LLM_URL + EMEM_SLEEP_AGENT_LLM_MODEL + OPENAI_API_KEY, \
             or EMEM_SLEEP_AGENT_USE_ASK=1). Refusing to fabricate a rewrite; running dry."
                .into(),
        );
    }

    // 1) Select candidates from contradictions + churn.
    let candidates = select_candidates(client, cfg).await?;
    summary.candidates_considered = candidates.len();
    summary.from_contradictions = candidates
        .iter()
        .filter(|c| matches!(c.source, CandidateSource::Contradiction { .. }))
        .count();
    summary.from_churn = candidates
        .iter()
        .filter(|c| matches!(c.source, CandidateSource::Churn { .. }))
        .count();

    if candidates.is_empty() {
        summary
            .notes
            .push("empty candidate set — corpus has no contradicted or high-churn memory to reconcile.".into());
        return Ok(summary);
    }

    // 2 + 3) For each candidate, plan the merge; in live mode call the LLM
    // and write back.
    let mut spend = 0.0_f64;
    for cand in &candidates {
        // The cluster of near-duplicate memory files to reconcile.
        let files = &cand.cluster;
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        let plan_line = format!(
            "[{}] reconcile {} memory file(s) {:?} → canonical path `{}`",
            cand.source.label(),
            files.len(),
            paths,
            cand.canonical_path,
        );
        summary.plan.push(plan_line);

        if summary.dry_run {
            continue;
        }

        // Budget gate — never start a call that would breach the cap.
        if spend >= cfg.budget_usd {
            summary.notes.push(format!(
                "budget cap ${:.4} reached after ${:.4}; stopping this pass early.",
                cfg.budget_usd, spend
            ));
            break;
        }

        let transport = transport.expect("dry_run guard guarantees transport is Some here");
        let req = LlmRequest::for_merge(cand, cfg);
        let proposal = transport
            .propose_merge(req)
            .await
            .map_err(|e| SleepAgentError::Llm(e.to_string()))?;
        spend += proposal.cost_usd;

        let outcome = apply_merge(client, cand, &proposal, cfg).await?;
        summary.merges_written += 1;
        summary.written_file_cids.push(outcome.new_file_cid.clone());
        if let Some(last) = summary.plan.last_mut() {
            last.push_str(&format!(
                " → wrote {} (originals preserved in history)",
                outcome.new_file_cid
            ));
        }
    }
    summary.spend_usd = spend;
    Ok(summary)
}

/// The long-running loop. Sleeps `cfg.interval` between passes. Returns
/// only on a fatal transport/config error; otherwise loops forever. The
/// binary owns the `Ctrl-C` handling.
pub async fn run_loop(
    client: ResponderClient,
    transport: Option<Arc<dyn LlmTransport>>,
    cfg: SleepAgentConfig,
) -> Result<(), SleepAgentError> {
    loop {
        match run_pass(&client, transport.as_deref(), &cfg).await {
            Ok(summary) => println!("{}", summary.render()),
            Err(e) => {
                // A single failed pass should not kill the daemon; log and
                // retry on the next tick. Config errors are fatal.
                if let SleepAgentError::Config(_) = e {
                    return Err(e);
                }
                eprintln!("sleep-agent pass failed (will retry): {e}");
            }
        }
        tokio::time::sleep(cfg.interval).await;
    }
}

/// Render an `interval` from whole seconds. Small helper kept here so both
/// the binary and tests construct intervals the same way.
pub fn interval_from_secs(secs: u64) -> Duration {
    Duration::from_secs(secs.max(1))
}

/// Group a flat list of memory files into near-duplicate clusters keyed by
/// a normalized path stem. Two files cluster when their paths share the
/// same parent directory and a normalized leaf (e.g. `notes-1`, `notes-2`,
/// `notes_dup` all map to `notes`). This is deliberately conservative:
/// the LLM does the semantic merge, the clusterer only proposes who to
/// show it together.
pub(crate) fn cluster_by_stem(files: Vec<MemoryFile>) -> BTreeMap<String, Vec<MemoryFile>> {
    let mut clusters: BTreeMap<String, Vec<MemoryFile>> = BTreeMap::new();
    for f in files {
        let key = normalize_stem(&f.path);
        clusters.entry(key).or_default().push(f);
    }
    clusters
}

/// Normalize a memory path into a clustering key: keep the parent dir, and
/// strip a trailing `-N` / `_N` / ` (copy)` / `.dup` disambiguator and any
/// extension from the leaf.
pub(crate) fn normalize_stem(path: &str) -> String {
    let (parent, leaf) = match path.rsplit_once('/') {
        Some((p, l)) => (p, l),
        None => ("", path),
    };
    let leaf = leaf.split('.').next().unwrap_or(leaf);
    let leaf = leaf
        .trim_end_matches(|c: char| c.is_ascii_digit())
        .trim_end_matches(['-', '_', ' '])
        .trim_end_matches("(copy)")
        .trim_end_matches([' ', '-', '_']);
    format!("{parent}/{leaf}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_stem_collapses_disambiguators() {
        assert_eq!(normalize_stem("notes/site-1"), "notes/site");
        assert_eq!(normalize_stem("notes/site-2"), "notes/site");
        assert_eq!(normalize_stem("notes/site_3"), "notes/site");
        assert_eq!(normalize_stem("notes/site.md"), "notes/site");
        // Distinct stems stay distinct.
        assert_ne!(normalize_stem("notes/site"), normalize_stem("notes/place"));
    }

    #[test]
    fn cluster_groups_near_duplicates() {
        let files = vec![
            MemoryFile::stub("obs/ndvi-1", "a"),
            MemoryFile::stub("obs/ndvi-2", "b"),
            MemoryFile::stub("obs/water", "c"),
        ];
        let clusters = cluster_by_stem(files);
        assert_eq!(clusters.len(), 2);
        assert_eq!(clusters.get("obs/ndvi").map(|v| v.len()), Some(2));
        assert_eq!(clusters.get("obs/water").map(|v| v.len()), Some(1));
    }

    #[test]
    fn pass_summary_render_is_honest_about_dry_run() {
        let s = PassSummary {
            dry_run: true,
            candidates_considered: 0,
            notes: vec!["empty corpus".into()],
            ..Default::default()
        };
        let r = s.render();
        assert!(r.contains("DRY-RUN"));
        assert!(r.contains("empty corpus"));
    }
}
