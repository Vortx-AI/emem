//! Per-axis scoring.
//!
//! Every axis grades a [`Responder`] against the fixture ground truth and
//! returns an [`AxisScore`] with a score in `[0,1]` and the number of
//! items evaluated. Scores are *computed* — there are no baked-in numbers.

use serde::Serialize;

use crate::dataset::Dataset;
use crate::fixture;
use crate::live::{ConflictMethod, LiveDriver, RetrievalPath};
use crate::responder::{normalise, Responder};

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct AxisScore {
    /// Fraction correct, in `[0,1]`.
    pub score: f64,
    /// Number of fixture items evaluated for this axis.
    pub items: usize,
    /// Number scored correct.
    pub correct: usize,
}

impl AxisScore {
    fn new(correct: usize, items: usize) -> Self {
        let score = if items == 0 {
            0.0
        } else {
            correct as f64 / items as f64
        };
        Self {
            score,
            items,
            correct,
        }
    }
}

/// Retrieval accuracy: recall each query's cell+band, compare to expected.
pub async fn retrieval_accuracy<R: Responder>(r: &R) -> anyhow::Result<AxisScore> {
    let qs = fixture::RETRIEVAL_QUERIES;
    let mut correct = 0;
    for q in qs {
        let got = r.recall(q.cell, q.band).await?;
        if got.as_deref().map(normalise) == Some(normalise(q.expected)) {
            correct += 1;
        }
    }
    Ok(AxisScore::new(correct, qs.len()))
}

/// Test-time learning: after replaying each episode's session writes, the
/// recalled value must equal the LAST write (last-write-wins).
pub async fn test_time_learning<R: Responder>(r: &R) -> anyhow::Result<AxisScore> {
    let eps = fixture::LEARNING_EPISODES;
    let mut correct = 0;
    for e in eps {
        let Some(expected) = e.writes.last() else {
            continue;
        };
        let got = r.learn_then_recall(e.cell, e.band, e.writes).await?;
        if got.as_deref().map(normalise) == Some(normalise(expected)) {
            correct += 1;
        }
    }
    Ok(AxisScore::new(correct, eps.len()))
}

/// Long-range understanding: recall items planted at various positions in
/// a long transcript; each must still return the planted value.
pub async fn long_range_understanding<R: Responder>(r: &R) -> anyhow::Result<AxisScore> {
    // Evaluate needles front-to-back by transcript position so a
    // recency-biased system's failures show up on the earliest items.
    let mut items: Vec<&fixture::LongRangeItem> = fixture::LONG_RANGE_ITEMS.iter().collect();
    items.sort_by_key(|it| it.position);
    let mut correct = 0;
    let mut earliest_resolved: Option<usize> = None;
    for it in &items {
        let got = r.recall(it.cell, it.band).await?;
        if got.as_deref().map(normalise) == Some(normalise(it.expected)) {
            correct += 1;
            earliest_resolved.get_or_insert(it.position);
        }
    }
    eprintln!(
        "[membench] long-range: earliest correctly-recalled needle at position {}",
        earliest_resolved
            .map(|p| p.to_string())
            .unwrap_or_else(|| "<none>".to_string())
    );
    Ok(AxisScore::new(correct, items.len()))
}

/// Conflict resolution: for each case, the responder's contradiction
/// verdict must match `should_conflict` (both true-positive and
/// true-negative count).
pub async fn conflict_resolution<R: Responder>(r: &R) -> anyhow::Result<AxisScore> {
    let cases = fixture::CONFLICT_CASES;
    let mut correct = 0;
    for c in cases {
        let report = r
            .contradiction(c.cell, c.band, c.value_a, c.value_b)
            .await?;
        if report.conflict == c.should_conflict {
            correct += 1;
        }
    }
    Ok(AxisScore::new(correct, cases.len()))
}

/// Full scorecard across the four axes.
#[derive(Clone, Debug, Serialize)]
pub struct Scorecard {
    pub retrieval_accuracy: AxisScore,
    pub test_time_learning: AxisScore,
    pub long_range_understanding: AxisScore,
    pub conflict_resolution: AxisScore,
    /// LongMemEval-S-style single topline: item-weighted mean across all
    /// axes (every fixture item counts once, regardless of axis size).
    pub longmemeval_topline: f64,
}

/// Item-weighted topline so larger axes pull proportionally more weight —
/// matches LongMemEval-S's "fraction of all questions answered correctly".
pub fn topline(axes: &[&AxisScore]) -> f64 {
    let total_items: usize = axes.iter().map(|a| a.items).sum();
    let total_correct: usize = axes.iter().map(|a| a.correct).sum();
    if total_items == 0 {
        0.0
    } else {
        total_correct as f64 / total_items as f64
    }
}

/// Grade a free-text retrieval: the responder is correct when the content
/// it returned for a query CONTAINS the ground-truth answer (case- and
/// whitespace-normalised substring). This is the standard LongMemEval /
/// MemoryAgentBench answer-recall criterion for short factual answers; it
/// rewards "the right needle surfaced" without demanding the responder echo
/// the exact phrasing.
pub fn contains_answer(retrieved: Option<&str>, expected: &str) -> bool {
    let Some(got) = retrieved else { return false };
    let hay = normalise_text(got);
    let needle = normalise_text(expected);
    !needle.is_empty() && hay.contains(&needle)
}

/// Text normaliser for substring grading: lowercase, collapse runs of
/// whitespace to single spaces, trim. Reuses [`normalise`] for the numeric
/// canonicalisation of pure-number answers ("50000" vs "50000.0").
fn normalise_text(s: &str) -> String {
    let n = normalise(s);
    n.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Run the four axes against a LIVE responder over a loaded dataset,
/// computing every score from the responder's own retrieval output. The
/// corpus MUST already be loaded into the responder (see
/// [`LiveDriver::load_corpus`]).
pub async fn run_all_live(
    driver: &LiveDriver,
    ds: &Dataset,
    path: RetrievalPath,
) -> anyhow::Result<(Scorecard, Option<ConflictMethod>)> {
    // --- retrieval accuracy: query each item, grade retrieved content. ---
    let mut ra_correct = 0;
    let mut ra_items = 0;
    for it in &ds.items {
        ra_items += 1;
        let got = driver.retrieve(&it.query, path).await?;
        if contains_answer(got.as_deref(), &it.expected_answer) {
            ra_correct += 1;
        }
    }
    let ra = AxisScore::new(ra_correct, ra_items);

    // --- long-range understanding: re-query the items loaded EARLIEST
    // (front of the transcript). With more items loaded after them, a
    // recency-biased store would drop these first. We grade the first
    // third (at least 1) of the corpus by load order. ---
    let lr_n = (ds.items.len() / 3).max(1);
    let mut lr_correct = 0;
    for it in ds.items.iter().take(lr_n) {
        let got = driver.retrieve(&it.query, path).await?;
        if contains_answer(got.as_deref(), &it.expected_answer) {
            lr_correct += 1;
        }
    }
    eprintln!("[membench] long-range: graded the {lr_n} earliest-loaded items (front of transcript)");
    let lru = AxisScore::new(lr_correct, lr_n);

    // --- test-time learning: for items with an `update`, apply the update
    // (last-write-wins), then re-query and require the POST-update answer. ---
    let learn_items: Vec<_> = ds.items.iter().filter(|i| i.update.is_some()).collect();
    let mut ttl_correct = 0;
    let mut ttl_items = 0;
    for it in &learn_items {
        let Some(expected) = it.update_answer.as_deref() else {
            // An update without a ground-truth post-value can't be graded.
            continue;
        };
        ttl_items += 1;
        driver.apply_update(it).await?;
        let got = driver.retrieve(&it.query, path).await?;
        if contains_answer(got.as_deref(), expected) {
            ttl_correct += 1;
        }
    }
    let ttl = AxisScore::new(ttl_correct, ttl_items);

    // --- conflict resolution: for items with `conflicts_with`, load the
    // disagreeing companion and require the responder to SURFACE the
    // disagreement (true positive). ---
    let conflict_items: Vec<_> = ds
        .items
        .iter()
        .filter(|i| i.conflicts_with.is_some())
        .collect();
    let mut cr_correct = 0;
    let mut cr_items = 0;
    // Track the strongest conflict method observed: a single
    // contradiction-scan hit outranks the stored-distinct fallback.
    let mut conflict_method: Option<ConflictMethod> = None;
    for it in &conflict_items {
        cr_items += 1;
        let outcome = driver.conflict_verdict(it).await?;
        match (conflict_method, outcome.method) {
            (None, m) => conflict_method = Some(m),
            (Some(ConflictMethod::StoredDistinct), ConflictMethod::ContradictionScan) => {
                conflict_method = Some(ConflictMethod::ContradictionScan)
            }
            _ => {}
        }
        if outcome.surfaced {
            cr_correct += 1;
        }
    }
    let cr = AxisScore::new(cr_correct, cr_items);

    let topline = topline(&[&ra, &ttl, &lru, &cr]);
    Ok((
        Scorecard {
            retrieval_accuracy: ra,
            test_time_learning: ttl,
            long_range_understanding: lru,
            conflict_resolution: cr,
            longmemeval_topline: topline,
        },
        conflict_method,
    ))
}

/// Run every axis against a responder and assemble the scorecard.
pub async fn run_all<R: Responder>(r: &R) -> anyhow::Result<Scorecard> {
    let ra = retrieval_accuracy(r).await?;
    let ttl = test_time_learning(r).await?;
    let lru = long_range_understanding(r).await?;
    let cr = conflict_resolution(r).await?;
    let topline = topline(&[&ra, &ttl, &lru, &cr]);
    Ok(Scorecard {
        retrieval_accuracy: ra,
        test_time_learning: ttl,
        long_range_understanding: lru,
        conflict_resolution: cr,
        longmemeval_topline: topline,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_answer_exact_and_substring() {
        assert!(contains_answer(Some("The user lives in Lisbon"), "Lisbon"));
        assert!(contains_answer(Some("capital is PARIS today"), "paris"));
        // numeric canonicalisation
        assert!(contains_answer(Some("budget was 50000 dollars"), "50000"));
        assert!(!contains_answer(Some("the user lives in Porto"), "Lisbon"));
        assert!(!contains_answer(None, "Lisbon"));
        assert!(!contains_answer(Some("anything"), ""));
    }

    /// A "perfect" corpus where every retrieval returns content containing
    /// the expected answer scores 1.0 on the retrieval axis. We grade
    /// directly with `contains_answer` (no network) to prove the grader,
    /// the same function the live path calls.
    #[test]
    fn perfect_text_corpus_scores_one() {
        let pairs = [
            ("The capital of France is Paris", "Paris"),
            ("NDVI was 0.71 in June", "0.71"),
            ("The dog is named Biscuit", "Biscuit"),
            ("Budget is 50000 dollars", "50000"),
        ];
        let correct = pairs
            .iter()
            .filter(|(content, ans)| contains_answer(Some(content), ans))
            .count();
        let axis = AxisScore::new(correct, pairs.len());
        assert_eq!(axis.score, 1.0);
        assert_eq!(axis.items, 4);
    }

    /// A corpus where exactly half the retrievals miss scores ~0.5.
    #[test]
    fn half_wrong_text_corpus_scores_about_half() {
        let pairs = [
            ("The capital of France is Paris", "Paris"),   // hit
            ("NDVI was 0.71 in June", "0.71"),             // hit
            ("The user lives in Porto", "Lisbon"),         // miss
            ("Some unrelated note", "Biscuit"),            // miss
        ];
        let correct = pairs
            .iter()
            .filter(|(content, ans)| contains_answer(Some(content), ans))
            .count();
        let axis = AxisScore::new(correct, pairs.len());
        assert!((axis.score - 0.5).abs() < 1e-9, "got {}", axis.score);
        assert_eq!(axis.correct, 2);
    }

    #[test]
    fn topline_is_item_weighted() {
        let a = AxisScore::new(2, 4); // 0.5
        let b = AxisScore::new(1, 1); // 1.0
        // item-weighted: (2+1)/(4+1) = 0.6, NOT the axis mean 0.75.
        assert!((topline(&[&a, &b]) - 0.6).abs() < 1e-9);
    }
}
