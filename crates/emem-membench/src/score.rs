//! Per-axis scoring.
//!
//! Every axis grades a [`Responder`] against the fixture ground truth and
//! returns an [`AxisScore`] with a score in `[0,1]` and the number of
//! items evaluated. Scores are *computed* — there are no baked-in numbers.

use serde::Serialize;

use crate::fixture;
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
