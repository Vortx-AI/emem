//! `emem-sleep-agentd` — the sleep-time agent daemon.
//!
//! Default OFF. Enable the loop with `EMEM_SLEEP_AGENT=1`. Without that
//! flag the binary runs a single pass and exits (handy for cron / one-shot
//! ops). `--dry-run` forces select-and-plan only: no LLM call, no write,
//! works fully offline against a local responder.
//!
//! Env (see `docs/sleep-agent.md`):
//!   EMEM_URL / EMEM_SLEEP_AGENT_URL        responder base URL
//!   EMEM_SLEEP_AGENT=1                      enable the long-running loop
//!   EMEM_SLEEP_AGENT_INTERVAL_SECS          seconds between passes
//!   EMEM_SLEEP_AGENT_TOPN                   candidates per pass
//!   EMEM_SLEEP_AGENT_BUDGET_USD             per-pass spend cap
//!   EMEM_SLEEP_AGENT_LLM_URL/_LLM_MODEL     OpenAI-compatible endpoint
//!   OPENAI_API_KEY                          key for the above
//!   EMEM_SLEEP_AGENT_USE_ASK=1              route via responder /v1/ask

use std::sync::Arc;

use clap::Parser;

use emem_sleep_agent::{
    config::LlmRoute, run_loop, run_pass, HttpLlmTransport, LlmTransport, ResponderClient,
    SleepAgentConfig,
};

#[derive(Parser, Debug)]
#[command(
    name = "emem-sleep-agentd",
    about = "Sleep-time agent: LLM rewrite/merge of high-churn or contradicted emem memory, written back as a NEW signed memory that supersedes the originals bi-temporally (non-destructive)."
)]
struct Args {
    /// Select + print candidates and the merge plan only. No LLM call, no
    /// write. Works fully offline against a local responder.
    #[arg(long)]
    dry_run: bool,

    /// Responder base URL (overrides EMEM_URL / EMEM_SLEEP_AGENT_URL).
    #[arg(long)]
    url: Option<String>,

    /// Run a single pass and exit, even when EMEM_SLEEP_AGENT=1.
    #[arg(long)]
    once: bool,
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let args = Args::parse();
    let cfg = SleepAgentConfig::from_env(args.dry_run, args.url);

    let client = ResponderClient::new(cfg.responder_url.clone());

    // Connectivity check up front so the operator gets a clear error rather
    // than per-pass failures.
    if let Err(e) = client.ping().await {
        eprintln!("error: cannot reach responder at {}: {e}", cfg.responder_url);
        return std::process::ExitCode::from(2);
    }
    println!("connected to responder at {}", cfg.responder_url);

    // Resolve the LLM transport. None means "run dry, never fabricate".
    let transport: Option<Arc<dyn LlmTransport>> = if cfg.dry_run {
        if !matches!(cfg.llm, LlmRoute::None) {
            println!("note: --dry-run set; LLM transport is configured but will not be called.");
        }
        None
    } else {
        match HttpLlmTransport::from_config(&cfg) {
            Some(t) => {
                match &cfg.llm {
                    LlmRoute::OpenAiCompatible { model, url, .. } => {
                        println!("LLM gateway: OpenAI-compatible model={model} url={url}");
                    }
                    LlmRoute::ResponderAsk => {
                        println!("LLM gateway: responder /v1/ask");
                    }
                    LlmRoute::None => {}
                }
                Some(Arc::new(t) as Arc<dyn LlmTransport>)
            }
            None => {
                println!(
                    "note: no LLM endpoint configured; running dry (select + plan, no writes). \
                     Set EMEM_SLEEP_AGENT_LLM_URL + EMEM_SLEEP_AGENT_LLM_MODEL + OPENAI_API_KEY, \
                     or EMEM_SLEEP_AGENT_USE_ASK=1, to enable the real rewrite path."
                );
                None
            }
        }
    };

    let loop_enabled = std::env::var("EMEM_SLEEP_AGENT")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    if loop_enabled && !args.once {
        println!(
            "sleep-agent loop enabled (interval={}s, top_n={}, budget=${:.2}/pass)",
            cfg.interval.as_secs(),
            cfg.top_n,
            cfg.budget_usd
        );
        match run_loop(client, transport, cfg).await {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("fatal: {e}");
                std::process::ExitCode::from(1)
            }
        }
    } else {
        // Single pass.
        match run_pass(&client, transport.as_deref(), &cfg).await {
            Ok(summary) => {
                print!("{}", summary.render());
                std::process::ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error: {e}");
                std::process::ExitCode::from(1)
            }
        }
    }
}
