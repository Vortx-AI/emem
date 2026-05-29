//! emem-membench — a MemoryAgentBench-style scorecard for an emem responder.
//!
//! Scores a local emem responder across four memory-system axes
//! (retrieval accuracy, test-time learning, long-range understanding,
//! conflict resolution) plus a LongMemEval-S-style topline, and emits a
//! JSON scorecard.
//!
//! Because the full public benchmark datasets need large downloads /
//! network, this binary ships a SMALL built-in synthetic fixture corpus
//! (see [`fixture`]) with known answers, so it produces non-empty,
//! genuinely-computed scores offline.
//!
//! Modes:
//!   --self-test  Grade the in-memory stub responder. No network. Writes
//!                the scorecard to var/benchmarks/ and prints it. Exits 0
//!                iff all four axes + topline produced a score.
//!   (default)    Drive a live responder over REST at --url / EMEM_URL
//!                (default http://127.0.0.1:5051), grade it, embed the
//!                responder's signed receipt, and print the scorecard.

mod fixture;
mod responder;
mod score;

use std::path::PathBuf;

use clap::Parser;
use serde::Serialize;

use responder::{HttpResponder, Responder, StubResponder};
use score::Scorecard;

const DEFAULT_URL: &str = "http://127.0.0.1:5051";

#[derive(Parser, Debug)]
#[command(
    name = "emem-membench",
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

    /// Directory to write the scorecard JSON into (created if missing).
    #[arg(long, default_value = "var/benchmarks")]
    out_dir: PathBuf,
}

#[derive(Serialize)]
struct Output {
    /// "self-test" or "live".
    mode: &'static str,
    /// emem-membench crate version.
    version: &'static str,
    /// Base URL when live; null in self-test.
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
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

    let (mode, scorecard, receipt, url) = if args.self_test {
        eprintln!("[membench] mode=self-test — grading in-memory stub from built-in fixture");
        eprintln!(
            "[membench] fixture: {} items, {} retrieval queries, {} learning episodes, {} long-range items, {} conflict cases",
            fixture::ITEMS.len(),
            fixture::RETRIEVAL_QUERIES.len(),
            fixture::LEARNING_EPISODES.len(),
            fixture::LONG_RANGE_ITEMS.len(),
            fixture::CONFLICT_CASES.len(),
        );
        eprintln!("[membench] NOT run: live HTTP, server-signed receipt (self-test is unsigned by design)");
        let stub = StubResponder::from_fixture();
        let card = score::run_all(&stub).await?;
        ("self-test", card, None, None)
    } else {
        eprintln!("[membench] mode=live — driving responder at {base_url}");
        eprintln!("[membench] running: /v1/recall, /v1/memory_contradictions, receipt fetch");
        eprintln!("[membench] note: test-time-learning axis grades the server's CURRENT value (no client write primitive in the public surface)");
        let http = HttpResponder::new(&base_url)?;
        let card = score::run_all(&http).await?;
        let receipt = match http.signed_receipt().await {
            Ok(Some(r)) => {
                eprintln!("[membench] embedded signed receipt from responder");
                Some(r)
            }
            Ok(None) => {
                eprintln!("[membench] WARN: responder returned no signed receipt");
                None
            }
            Err(e) => {
                eprintln!("[membench] WARN: receipt fetch failed: {e}");
                None
            }
        };
        ("live", card, receipt, Some(base_url.clone()))
    };

    let out = Output {
        mode,
        version: env!("CARGO_PKG_VERSION"),
        url,
        scorecard,
        receipt,
    };

    let json = serde_json::to_string_pretty(&out)?;

    // In self-test mode, persist to var/benchmarks/ for CI artifact capture.
    if args.self_test {
        std::fs::create_dir_all(&args.out_dir)?;
        let path = args.out_dir.join("membench-self-test.json");
        std::fs::write(&path, &json)?;
        eprintln!("[membench] wrote scorecard to {}", path.display());
    }

    println!("{json}");

    // Exit non-zero if any axis failed to evaluate any item (would mean a
    // broken fixture / responder), so CI catches an empty scorecard.
    let c = &out.scorecard;
    let all_nonempty = c.retrieval_accuracy.items > 0
        && c.test_time_learning.items > 0
        && c.long_range_understanding.items > 0
        && c.conflict_resolution.items > 0;
    if !all_nonempty {
        eprintln!("[membench] ERROR: at least one axis evaluated 0 items");
        std::process::exit(1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
            async fn signed_receipt(&self) -> anyhow::Result<Option<serde_json::Value>> {
                Ok(None)
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
