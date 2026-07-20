//! emem-scorecard — a MemoryAgentBench-style scorecard for an emem responder.
//!
//! Scores a local emem responder across four memory-system axes
//! (retrieval accuracy, test-time learning, long-range understanding,
//! conflict resolution) plus a LongMemEval-S-style topline, and emits a
//! JSON scorecard.
//!
//! Because the full public benchmark datasets need large downloads /
//! network, this binary ships a SMALL built-in synthetic fixture corpus
//! (see [`fixture`]) for `--self-test`, and a committed SAMPLE dataset
//! (`data/sample-longmemeval.jsonl`) so `--live` runs end-to-end with no
//! external download. See `docs/benchmarks.md`.
//!
//! Modes (mutually exclusive; `--self-test` is the default):
//!   --self-test  Grade the in-memory stub responder against the built-in
//!                fixture. No network. Writes var/benchmarks/scorecard-self-test.json.
//!   --live       Drive a RUNNING responder at --url / EMEM_URL (default
//!                http://127.0.0.1:5051): load the `--dataset` corpus over
//!                the write API (memory_create), answer every query over
//!                the read API (memory_search, or a labelled recall
//!                fallback when the embedder is absent), compute all four
//!                axes + topline from the responder's own output, embed its
//!                signed receipt, and write var/benchmarks/scorecard-live.json.
//!
//! Honesty: the scorecard labels SAMPLE (committed ~15-item set,
//! illustrative only) vs FULL (user-supplied dataset) so a sample score is
//! never mistaken for the published benchmark.

mod dataset;
mod fixture;
mod live;
mod responder;
mod score;

use std::path::PathBuf;

use clap::Parser;
use serde::Serialize;

use live::{LiveDriver, RetrievalPath};
use responder::StubResponder;
use score::Scorecard;

const DEFAULT_URL: &str = "http://127.0.0.1:5051";

#[derive(Parser, Debug)]
#[command(
    name = "emem-scorecard",
    about = "MemoryAgentBench-style scorecard for an emem responder"
)]
struct Args {
    /// Base URL of a live emem responder. Falls back to $EMEM_URL, then
    /// the default, when not passed (clap `env` feature is not enabled in
    /// the workspace, so the env fallback is wired manually in `main`).
    #[arg(long)]
    url: Option<String>,

    /// Run the offline fixture path against the in-memory stub (no network).
    #[arg(long)]
    self_test: bool,

    /// Score a RUNNING responder: load the dataset corpus over the write
    /// API, answer every query over the read API, and compute the four
    /// axes + topline from the responder's own output.
    #[arg(long)]
    live: bool,

    /// Path to a LongMemEval-S / MemoryAgentBench-format JSONL dataset
    /// ({id, content, query, expected_answer}). Defaults to the committed
    /// SAMPLE so `--live` runs end-to-end with no external download. Point
    /// this at the full public dataset (see docs/benchmarks.md) for a real
    /// benchmark run.
    #[arg(long)]
    dataset: Option<PathBuf>,

    /// Directory to write the scorecard JSON into (created if missing).
    #[arg(long, default_value = "var/benchmarks")]
    out_dir: PathBuf,
}

/// Resolve the dataset path: explicit `--dataset`, else the committed
/// sample (tried at the crate-relative path, then the workspace-relative
/// path so it works from either cwd).
fn resolve_dataset_path(arg: &Option<PathBuf>) -> anyhow::Result<String> {
    if let Some(p) = arg {
        return Ok(p.to_string_lossy().into_owned());
    }
    for cand in [
        dataset::SAMPLE_RELATIVE_PATH,
        concat!(env!("CARGO_MANIFEST_DIR"), "/data/sample-longmemeval.jsonl"),
    ] {
        if std::path::Path::new(cand).exists() {
            return Ok(cand.to_string());
        }
    }
    anyhow::bail!(
        "no --dataset given and the committed sample was not found at {}",
        dataset::SAMPLE_RELATIVE_PATH
    )
}

#[derive(Serialize)]
struct Output {
    /// "self-test" or "live".
    mode: &'static str,
    /// emem-scorecard crate version.
    version: &'static str,
    /// Base URL when live; null in self-test.
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    /// Dataset provenance: "sample" (committed ~15-item illustrative set,
    /// NOT the published benchmark) or "full" (user-supplied dataset).
    /// Null in self-test (which uses the built-in synthetic fixture).
    #[serde(skip_serializing_if = "Option::is_none")]
    dataset_provenance: Option<&'static str>,
    /// Dataset source path (live only).
    #[serde(skip_serializing_if = "Option::is_none")]
    dataset_source: Option<String>,
    /// Number of corpus items loaded (live only).
    #[serde(skip_serializing_if = "Option::is_none")]
    corpus_items: Option<usize>,
    /// Which read path produced the scores: "memory_search" (BGE semantic)
    /// or "recall_fallback" (lexical, embedder absent). Live only.
    #[serde(skip_serializing_if = "Option::is_none")]
    retrieval_path: Option<&'static str>,
    /// How the conflict axis reached its verdict: "contradiction_scan"
    /// (responder's EO-fact multi-attester index) or
    /// "stored_distinct_fallback" (text-surface non-destructive-storage
    /// check). Live only, present when the dataset has conflict items.
    #[serde(skip_serializing_if = "Option::is_none")]
    conflict_method: Option<&'static str>,
    /// Human-readable caveat string surfaced in the JSON so the scorecard
    /// is self-describing.
    #[serde(skip_serializing_if = "Option::is_none")]
    caveat: Option<String>,
    /// The four-axis scorecard + topline.
    scorecard: Scorecard,
    /// Signed receipt from the live responder (null in self-test).
    #[serde(skip_serializing_if = "Option::is_none")]
    receipt: Option<serde_json::Value>,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let base_url = args
        .url
        .clone()
        .or_else(|| std::env::var("EMEM_URL").ok())
        .unwrap_or_else(|| DEFAULT_URL.to_string());

    // `--live` is the real path; `--self-test` (the default when neither is
    // passed) grades the offline stub. They are mutually exclusive.
    if args.live && args.self_test {
        anyhow::bail!("pass at most one of --live / --self-test");
    }

    let out = if args.live {
        run_live(&base_url, &args).await?
    } else {
        if !args.self_test {
            eprintln!("[scorecard] no mode flag given — defaulting to --self-test (pass --live to score a running responder)");
        }
        run_self_test().await?
    };

    let json = serde_json::to_string_pretty(&out)?;

    // Persist the scorecard to var/benchmarks/ for artifact capture.
    std::fs::create_dir_all(&args.out_dir)?;
    let fname = if out.mode == "live" {
        "scorecard-live.json"
    } else {
        "scorecard-self-test.json"
    };
    let path = args.out_dir.join(fname);
    std::fs::write(&path, &json)?;
    eprintln!("[scorecard] wrote scorecard to {}", path.display());

    println!("{json}");

    // The always-present axes (retrieval + long-range) MUST have graded
    // items — a zero there means a broken fixture / unreachable responder /
    // empty dataset, which we fail on so CI catches a hollow scorecard. The
    // learning + conflict axes depend on optional dataset fields (`update`,
    // `conflicts_with`); a dataset without them legitimately grades 0 items
    // there, so we only warn.
    let c = &out.scorecard;
    if c.retrieval_accuracy.items == 0 || c.long_range_understanding.items == 0 {
        eprintln!("[scorecard] ERROR: retrieval/long-range axis evaluated 0 items");
        std::process::exit(1);
    }
    if c.test_time_learning.items == 0 {
        eprintln!("[scorecard] WARN: test-time-learning axis evaluated 0 items (dataset has no `update`/`update_answer` items)");
    }
    if c.conflict_resolution.items == 0 {
        eprintln!("[scorecard] WARN: conflict-resolution axis evaluated 0 items (dataset has no `conflicts_with` items)");
    }
    Ok(())
}

/// Grade the in-memory stub from the built-in synthetic fixture. No network.
async fn run_self_test() -> anyhow::Result<Output> {
    eprintln!("[scorecard] mode=self-test — grading in-memory stub from built-in fixture");
    eprintln!(
        "[scorecard] fixture: {} items, {} retrieval queries, {} learning episodes, {} long-range items, {} conflict cases",
        fixture::ITEMS.len(),
        fixture::RETRIEVAL_QUERIES.len(),
        fixture::LEARNING_EPISODES.len(),
        fixture::LONG_RANGE_ITEMS.len(),
        fixture::CONFLICT_CASES.len(),
    );
    eprintln!(
        "[scorecard] NOT run: live HTTP, server-signed receipt (self-test is unsigned by design)"
    );
    let stub = StubResponder::from_fixture();
    let card = score::run_all(&stub).await?;
    Ok(Output {
        mode: "self-test",
        version: env!("CARGO_PKG_VERSION"),
        url: None,
        dataset_provenance: None,
        dataset_source: None,
        corpus_items: None,
        retrieval_path: None,
        conflict_method: None,
        caveat: Some(
            "self-test grades an in-memory stub over the built-in synthetic fixture; not a benchmark number".into(),
        ),
        scorecard: card,
        receipt: None,
    })
}

/// Drive a RUNNING responder: load the corpus, pick a read path, compute
/// every axis from the responder's own output.
async fn run_live(base_url: &str, args: &Args) -> anyhow::Result<Output> {
    let ds_path = resolve_dataset_path(&args.dataset)?;
    let ds = dataset::load(&ds_path)?;
    eprintln!(
        "[scorecard] mode=live — driving responder at {base_url}\n[scorecard] dataset: {} ({} items, provenance={})",
        ds.source,
        ds.items.len(),
        ds.provenance.as_str()
    );

    let driver = LiveDriver::new(base_url)?;

    // 1) Load the corpus over the real write API (memory_create).
    eprintln!("[scorecard] loading corpus via POST /mcp memory_create ...");
    let loaded = driver.load_corpus(&ds).await?;
    let signed = loaded.iter().filter(|f| f.file_cid.is_some()).count();
    eprintln!(
        "[scorecard] loaded {} files, {} returned a signed file_cid",
        loaded.len(),
        signed
    );

    // 2) Pick the read path: semantic search if the BGE model is loaded,
    //    else the honest lexical recall fallback.
    let sample_q = ds.items.first().map(|i| i.query.as_str()).unwrap_or("test");
    let (model_loaded, idx_size) = driver.probe_search(sample_q).await;
    let path = if model_loaded {
        eprintln!(
            "[scorecard] read path = memory_search (BGE model loaded, index size {idx_size})"
        );
        RetrievalPath::MemorySearch
    } else {
        eprintln!("[scorecard] read path = recall_fallback (embedder model NOT loaded offline — listing memory_view + lexical ranker over /v1/recall-class reads). Scores reflect lexical retrieval, NOT BGE semantic search.");
        RetrievalPath::RecallFallback
    };

    // 3) Compute the four axes from real responder output.
    eprintln!("[scorecard] running: memory_create (write), {} (read), /v1/memory_contradictions (conflict)", path.as_str());
    let (card, conflict_method) = score::run_all_live(&driver, &ds, path).await?;
    if let Some(m) = conflict_method {
        eprintln!("[scorecard] conflict axis graded via {}", m.as_str());
    }

    // 4) Embed a server-signed receipt where available.
    let receipt = if let Some(first) = loaded.first() {
        match driver.signed_receipt(&first.path).await {
            Ok(Some(r)) => {
                eprintln!("[scorecard] embedded signed receipt from responder");
                Some(r)
            }
            Ok(None) => {
                eprintln!("[scorecard] WARN: responder returned no signed receipt");
                None
            }
            Err(e) => {
                eprintln!("[scorecard] WARN: receipt fetch failed: {e}");
                None
            }
        }
    } else {
        None
    };

    let caveat = match ds.provenance {
        dataset::Provenance::Sample => format!(
            "SAMPLE dataset ({} items) — illustrative only, NOT the published LongMemEval/MemoryAgentBench score. Run with --dataset <full.jsonl> for a real benchmark. Read path: {}.",
            ds.items.len(),
            path.as_str()
        ),
        dataset::Provenance::Full => format!(
            "FULL user-supplied dataset ({} items). Read path: {}.",
            ds.items.len(),
            path.as_str()
        ),
    };

    Ok(Output {
        mode: "live",
        version: env!("CARGO_PKG_VERSION"),
        url: Some(base_url.to_string()),
        dataset_provenance: Some(ds.provenance.as_str()),
        dataset_source: Some(ds.source.clone()),
        corpus_items: Some(loaded.len()),
        retrieval_path: Some(path.as_str()),
        conflict_method: conflict_method.map(|m| m.as_str()),
        caveat: Some(caveat),
        scorecard: card,
        receipt,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use responder::Responder;

    /// A stub seeded with exactly the fixture truth scores 1.0 on the
    /// value-driven axes (retrieval, learning, long-range) and 1.0 on
    /// conflict (the StubResponder detects conflicts by value inequality).
    #[tokio::test]
    async fn perfect_corpus_scores_one() {
        let stub = StubResponder::from_fixture();
        let card = score::run_all(&stub).await.unwrap();
        assert_eq!(card.retrieval_accuracy.score, 1.0, "retrieval");
        assert_eq!(card.test_time_learning.score, 1.0, "learning");
        assert_eq!(card.long_range_understanding.score, 1.0, "long-range");
        assert_eq!(card.conflict_resolution.score, 1.0, "conflict");
        assert_eq!(card.longmemeval_topline, 1.0, "topline");
        // Every axis must have evaluated a non-zero number of items.
        assert!(card.retrieval_accuracy.items > 0);
        assert!(card.test_time_learning.items > 0);
        assert!(card.long_range_understanding.items > 0);
        assert!(card.conflict_resolution.items > 0);
    }

    /// A retrieval corpus where half the seeded values are wrong scores
    /// ~0.5 on the retrieval axis. Five fixture queries; we corrupt enough
    /// seeds that exactly 2 of 5 remain correct → 0.4, and a 4-item
    /// variant where 2 are wrong → 0.5. We assert the 0.4 case precisely
    /// and that it is strictly between 0 and 1.
    #[tokio::test]
    async fn half_wrong_corpus_scores_about_half() {
        // Seed: keep 2 of the 5 retrieval queries correct, corrupt 3.
        let stub = StubResponder::from_seed(&[
            // correct
            ("defi.aa111.bbbb.cccc", "copdem30m.elevation_mean", "812.0"),
            ("defi.aa111.bbbb.cccc", "esa_worldcover.class", "tree_cover"),
            // wrong values
            ("defi.dd222.eeee.ffff", "s2.ndvi", "0.99"),
            ("defi.dd222.eeee.ffff", "jrc_gsw.occurrence", "9"),
            ("defi.gg333.hhhh.iiii", "s2.ndvi", "0.99"),
        ]);
        let ra = score::retrieval_accuracy(&stub).await.unwrap();
        assert_eq!(ra.items, 5);
        assert_eq!(ra.correct, 2);
        assert!(
            (ra.score - 0.4).abs() < 1e-9,
            "expected 0.4, got {}",
            ra.score
        );
        assert!(ra.score > 0.0 && ra.score < 1.0);
    }

    /// An exactly-half (2 of 4) conflict corpus scores 0.5 — proves the
    /// scorer rewards correct true-negatives, not just positives. We build
    /// a stub that always reports "conflict" regardless of input, so it is
    /// right on the two should_conflict=true cases and wrong on the
    /// should_conflict=false case in the fixture.
    #[tokio::test]
    async fn always_conflict_stub_is_partial() {
        struct AlwaysConflict;
        impl Responder for AlwaysConflict {
            async fn recall(&self, _c: &str, _b: &str) -> anyhow::Result<Option<String>> {
                Ok(None)
            }
            async fn learn_then_recall(
                &self,
                _c: &str,
                _b: &str,
                _w: &[&str],
            ) -> anyhow::Result<Option<String>> {
                Ok(None)
            }
            async fn contradiction(
                &self,
                _c: &str,
                _b: &str,
                _a: &str,
                _bv: &str,
            ) -> anyhow::Result<responder::ConflictReport> {
                Ok(responder::ConflictReport { conflict: true })
            }
        }
        let cr = score::conflict_resolution(&AlwaysConflict).await.unwrap();
        // Fixture has 2 should_conflict=true and 1 should_conflict=false.
        assert_eq!(cr.items, fixture::CONFLICT_CASES.len());
        let expected_correct = fixture::CONFLICT_CASES
            .iter()
            .filter(|c| c.should_conflict)
            .count();
        assert_eq!(cr.correct, expected_correct);
        assert!(cr.score > 0.0 && cr.score < 1.0);
    }
}
