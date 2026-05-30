//! Runtime configuration for the sleep-time agent, read from environment
//! variables (the deployment convention everywhere else in emem) plus CLI
//! overrides.

use std::time::Duration;

use crate::interval_from_secs;

/// Default time between passes (10 minutes).
pub const DEFAULT_INTERVAL_SECS: u64 = 600;
/// Default number of candidates considered per pass.
pub const DEFAULT_TOP_N: usize = 8;
/// Default per-pass spend cap in USD.
pub const DEFAULT_BUDGET_USD: f64 = 0.50;
/// Minimum contradiction severity for a path to be a candidate.
pub const DEFAULT_MIN_SEVERITY: f32 = 0.3;
/// A memory path is "high-churn" once its history length reaches this.
pub const DEFAULT_CHURN_THRESHOLD: usize = 3;

/// Where the merge LLM lives.
#[derive(Debug, Clone)]
pub enum LlmRoute {
    /// No LLM configured. The agent runs dry and never fabricates.
    None,
    /// An OpenAI-compatible chat-completions endpoint.
    OpenAiCompatible {
        url: String,
        model: String,
        api_key: String,
    },
    /// Route rewrites through the responder's own `/v1/ask` gateway, so the
    /// model choice follows the operator's responder configuration rather
    /// than a separately-pinned key.
    ResponderAsk,
}

/// Fully-resolved configuration for a run.
#[derive(Debug, Clone)]
pub struct SleepAgentConfig {
    /// Base URL of the responder, e.g. `http://127.0.0.1:5088`.
    pub responder_url: String,
    /// Time between passes.
    pub interval: Duration,
    /// Top-N candidates per pass.
    pub top_n: usize,
    /// Per-pass USD spend cap.
    pub budget_usd: f64,
    /// Minimum contradiction severity to treat a path as a candidate.
    pub min_severity: f32,
    /// History length at/after which a path counts as high-churn.
    pub churn_threshold: usize,
    /// Memory kinds scanned for churn candidates.
    pub churn_kinds: Vec<String>,
    /// When true: select + plan only, never call the LLM, never write.
    pub dry_run: bool,
    /// Where the merge LLM lives.
    pub llm: LlmRoute,
    /// Attester label the merged memory is written under. Matches the
    /// `sleep_agent_v1` attester registered in the responder's
    /// `attesters-v0.json`.
    pub attester_label: String,
}

impl SleepAgentConfig {
    /// Build a config from environment + CLI overrides.
    ///
    /// `EMEM_URL` (or `EMEM_SLEEP_AGENT_URL`) — responder base URL.
    /// `EMEM_SLEEP_AGENT_INTERVAL_SECS`, `_TOPN`, `_BUDGET_USD`,
    /// `_MIN_SEVERITY`, `_CHURN_THRESHOLD` — loop caps.
    /// `EMEM_SLEEP_AGENT_LLM_URL` + `_LLM_MODEL` + `OPENAI_API_KEY` — an
    /// OpenAI-compatible endpoint. `EMEM_SLEEP_AGENT_USE_ASK=1` — route
    /// through the responder's `/v1/ask` gateway instead.
    pub fn from_env(dry_run_cli: bool, url_cli: Option<String>) -> Self {
        let responder_url = url_cli
            .or_else(|| std::env::var("EMEM_SLEEP_AGENT_URL").ok())
            .or_else(|| std::env::var("EMEM_URL").ok())
            .unwrap_or_else(|| "http://127.0.0.1:5051".to_string());

        let interval = interval_from_secs(env_parse(
            "EMEM_SLEEP_AGENT_INTERVAL_SECS",
            DEFAULT_INTERVAL_SECS,
        ));
        let top_n = env_parse("EMEM_SLEEP_AGENT_TOPN", DEFAULT_TOP_N).max(1);
        let budget_usd = env_parse("EMEM_SLEEP_AGENT_BUDGET_USD", DEFAULT_BUDGET_USD);
        let min_severity = env_parse("EMEM_SLEEP_AGENT_MIN_SEVERITY", DEFAULT_MIN_SEVERITY);
        let churn_threshold =
            env_parse("EMEM_SLEEP_AGENT_CHURN_THRESHOLD", DEFAULT_CHURN_THRESHOLD).max(2);

        let churn_kinds = std::env::var("EMEM_SLEEP_AGENT_CHURN_KINDS")
            .ok()
            .map(|s| {
                s.split(',')
                    .map(|x| x.trim().to_string())
                    .filter(|x| !x.is_empty())
                    .collect::<Vec<_>>()
            })
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| {
                vec![
                    "semantic".to_string(),
                    "episodic".to_string(),
                    "resource".to_string(),
                ]
            });

        let llm = resolve_llm_route();

        SleepAgentConfig {
            responder_url,
            interval,
            top_n,
            budget_usd,
            min_severity,
            churn_threshold,
            churn_kinds,
            dry_run: dry_run_cli,
            llm,
            attester_label: std::env::var("EMEM_SLEEP_AGENT_ATTESTER")
                .unwrap_or_else(|_| "sleep_agent_v1".to_string()),
        }
    }
}

fn resolve_llm_route() -> LlmRoute {
    if std::env::var("EMEM_SLEEP_AGENT_USE_ASK")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        return LlmRoute::ResponderAsk;
    }
    let url = std::env::var("EMEM_SLEEP_AGENT_LLM_URL").ok();
    let model = std::env::var("EMEM_SLEEP_AGENT_LLM_MODEL").ok();
    let key = std::env::var("OPENAI_API_KEY")
        .ok()
        .or_else(|| std::env::var("EMEM_SLEEP_AGENT_LLM_API_KEY").ok());
    match (url, model, key) {
        (Some(url), Some(model), Some(api_key))
            if !url.is_empty() && !model.is_empty() && !api_key.is_empty() =>
        {
            LlmRoute::OpenAiCompatible {
                url,
                model,
                api_key,
            }
        }
        _ => LlmRoute::None,
    }
}

fn env_parse<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_env_absent() {
        // Build with explicit dry-run + url so the test does not depend on
        // the ambient environment.
        let cfg = SleepAgentConfig::from_env(true, Some("http://localhost:9999".into()));
        assert!(cfg.dry_run);
        assert_eq!(cfg.responder_url, "http://localhost:9999");
        assert!(cfg.top_n >= 1);
        assert!(cfg.budget_usd > 0.0);
    }
}
