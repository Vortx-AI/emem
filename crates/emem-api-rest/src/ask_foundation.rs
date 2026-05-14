//! Foundation-embedding fan-out for `/v1/ask`.
//!
//! The standard `ask_inner` path is anchored on the topic router, which
//! produces a list of *scalar* bands (NDVI, weather, surface_water,
//! …) to recall. That's the right answer for most questions, but it
//! leaves the 1024-D Clay, 1024-D Prithvi, and 128-D Tessera foundation
//! embeddings untouched even when the user explicitly asks
//! "find places like X" or "what changed here in the last year".
//!
//! This module adds a small intent classifier (keyword pre-pass over
//! the question text) and a fan-out helper that, when the intent
//! matches, exercises one of the foundation algorithms registered in
//! `algorithms-v0.json`:
//!
//! - `Similarity` → `clay_archetype_match@1`-style `find_similar` over
//!   the Clay band, optionally followed by Prithvi & Tessera for the
//!   triple-consensus pattern (`clay_prithvi_tessera_triple_consensus@1`).
//! - `Change` → recall of `clay_v1` + `prithvi_eo2` + `geotessera.multi_year`
//!   so the agent (or in-process AST evaluator) can compute the year-on-year
//!   triple-consensus change index.
//!
//! Returns a structured envelope under `foundation_embeddings` in the
//! `/v1/ask` response. The receipt remains the one signed by the
//! standard recall path; encoder fact CIDs are merged into the same
//! `fact_cids` list so the response is one signed envelope.
//!
//! Thresholds (`ask_timeout_ms`, k, etc.) are read from the
//! `clay_prithvi_tessera_triple_consensus@1` algorithm's `parameters`
//! block — see [`Algorithm::param_f64`] — so an operator can re-tune
//! at registry-CID time rather than recompiling.
//!
//! Designed to be additive: when the classifier doesn't fire, this
//! module returns `None` and the existing `ask_inner` path is
//! byte-equivalent to before.

use serde_json::{json, Value as JsonValue};

use crate::{find_similar_with_auto_materialize, AppState};
use emem_primitives::find_similar::{FindSimilarMode, FindSimilarReq};

/// What kind of question the agent appears to be asking about a place.
/// Used to decide whether — and how — to fan out across the three
/// foundation encoders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskIntent {
    /// "Find places like X", "where else looks like this", "similar
    /// to". Triggers a k-NN fan-out across foundation embeddings.
    Similarity,
    /// "What changed here", "year over year", "since last year",
    /// "deforestation", "new construction". Triggers a recall of the
    /// triple-consensus inputs so the agent can compute the
    /// year-on-year change index.
    Change,
}

/// Lightweight keyword classifier. Returns the first matching intent.
/// We intentionally don't run a transformer-anchored classifier here —
/// the topic router already does that work for scalar bands; we only
/// need an *additive* check to know whether the foundation fan-out is
/// worth its cost.
pub fn classify_intent(q: &str) -> Option<AskIntent> {
    let lower = q.to_ascii_lowercase();
    // Similarity patterns — k-NN against foundation embeddings.
    const SIMILARITY: &[&str] = &[
        "find places like",
        "places like",
        "similar to",
        "looks like",
        "where else",
        "analog of",
        "analogue of",
        "look-alike",
        "lookalike",
        "comparable to",
        "find similar",
        "what's similar",
        "what is similar",
    ];
    // Change / temporal patterns — triple-consensus year-on-year.
    const CHANGE: &[&str] = &[
        "what changed",
        "what has changed",
        "year over year",
        "year-over-year",
        "since last year",
        "in the last year",
        "deforestation",
        "deforested",
        "forest loss",
        "new construction",
        "urban expansion",
        "wetland change",
        "coastline change",
        "coastal erosion",
        "shoreline retreat",
        "anomaly",
        "anomalous",
    ];
    if SIMILARITY.iter().any(|s| lower.contains(s)) {
        return Some(AskIntent::Similarity);
    }
    if CHANGE.iter().any(|s| lower.contains(s)) {
        return Some(AskIntent::Change);
    }
    None
}

/// Run the foundation-embedding fan-out for a matched intent. Returns
/// a JSON envelope to merge under `foundation_embeddings` in the
/// `/v1/ask` response, or `None` if the intent doesn't match.
///
/// Cost: bounded by `ask_timeout_ms` from the triple-consensus
/// algorithm's parameters block (defaults to 4 s). Each leg of the
/// fan-out runs concurrently via `tokio::join!`; if the budget is
/// exhausted the helper returns an envelope with
/// `degraded_reason: "foundation_embedding_unavailable"` so the
/// `ask_inner` path still ships a useful answer.
pub async fn foundation_fanout(q: &str, cell: &str, s: &AppState) -> Option<JsonValue> {
    let intent = classify_intent(q)?;
    let alg = emem_core::algorithms::DEFAULT.lookup("clay_prithvi_tessera_triple_consensus@1");
    let timeout_ms = alg
        .and_then(|a| a.param_f64("ask_timeout_ms"))
        .unwrap_or(4000.0) as u64;
    let k_neighbors = alg.and_then(|a| a.param_f64("k_neighbors")).unwrap_or(8.0) as usize;

    let work = async move {
        match intent {
            AskIntent::Similarity => {
                similarity_fanout(cell.to_string(), k_neighbors, s.clone()).await
            }
            AskIntent::Change => change_fanout(cell.to_string(), s.clone()).await,
        }
    };
    match tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), work).await {
        Ok(v) => Some(v),
        Err(_) => Some(json!({
            "intent":           intent_label(intent),
            "available":        false,
            "degraded_reason":  "foundation_embedding_timeout",
            "budget_ms":        timeout_ms,
        })),
    }
}

fn intent_label(i: AskIntent) -> &'static str {
    match i {
        AskIntent::Similarity => "similarity",
        AskIntent::Change => "change",
    }
}

/// Similarity fan-out: run `find_similar_with_auto_materialize` against
/// the geotessera band first (cheapest — already on disk for the
/// query cell in most deployments), then optionally also against
/// `clay_v1` and `prithvi_eo2` when their materializers can answer.
/// Returns the geotessera neighbours plus a per-encoder hit-count
/// summary so the agent can see whether the triple corroborated.
async fn similarity_fanout(cell: String, k: usize, s: AppState) -> JsonValue {
    let mut encoder_results: Vec<JsonValue> = Vec::new();
    let mut all_fact_cids: Vec<String> = Vec::new();
    for band in ["geotessera", "clay_v1", "prithvi_eo2"] {
        let req = FindSimilarReq {
            key: cell.clone(),
            k: Some(k as u32),
            filter: None,
            band: Some(band.to_string()),
            mode: FindSimilarMode::Cosine,
        };
        let band_used = band.to_string();
        match find_similar_with_auto_materialize(&req, &band_used, &s).await {
            Ok((resp, _notes)) => {
                let cells: Vec<String> = resp.neighbors.iter().map(|n| n.cell.clone()).collect();
                if let Ok(v) = serde_json::to_value(&resp.receipt) {
                    if let Some(fc) = v.get("fact_cids").and_then(|x| x.as_array()) {
                        for c in fc {
                            if let Some(cid) = c.as_str() {
                                all_fact_cids.push(cid.to_string());
                            }
                        }
                    }
                }
                encoder_results.push(json!({
                    "encoder": band,
                    "k_returned": resp.neighbors.len(),
                    "neighbors": cells,
                }));
            }
            Err(e) => {
                encoder_results.push(json!({
                    "encoder": band,
                    "available": false,
                    "reason": e.1.message,
                }));
            }
        }
    }
    // Cross-encoder consensus — cells that appear in ≥2 encoder
    // result lists are flagged as "all_three" / "two_of_three"
    // neighbours, matching the triple-consensus pattern.
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for r in &encoder_results {
        if let Some(arr) = r.get("neighbors").and_then(|x| x.as_array()) {
            for c in arr {
                if let Some(cell) = c.as_str() {
                    *counts.entry(cell.to_string()).or_default() += 1;
                }
            }
        }
    }
    let mut consensus: Vec<(String, usize)> = counts.into_iter().collect();
    consensus.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let consensus_top: Vec<JsonValue> = consensus
        .iter()
        .map(|(c, n)| {
            json!({
                "cell": c,
                "encoder_agreement": match *n {
                    3 => "all_three",
                    2 => "two_of_three",
                    _ => "one_or_none",
                },
            })
        })
        .collect();
    json!({
        "intent":              "similarity",
        "available":           true,
        "computation":         "similarity_consensus",
        "algorithm_key":       "clay_prithvi_tessera_triple_consensus@1",
        "encoder_results":     encoder_results,
        "consensus_neighbors": consensus_top,
        "encoder_fact_cids":   all_fact_cids,
    })
}

/// Change fan-out: ensure clay_v1, prithvi_eo2, geotessera.multi_year
/// are recallable at the query cell so the agent can compute the
/// year-on-year triple-consensus change index via dispatch_algorithms.
/// This helper does not evaluate the formula itself — the existing
/// algorithm-dispatch path in ask_inner picks up the algorithm key
/// once the recall has populated the bands. We return a summary of
/// which encoders are available + the threshold the consensus
/// algorithm will gate on.
async fn change_fanout(cell: String, s: AppState) -> JsonValue {
    let alg = emem_core::algorithms::DEFAULT.lookup("clay_prithvi_tessera_triple_consensus@1");
    let gate = alg
        .and_then(|a| a.param_f64("consensus_threshold"))
        .unwrap_or(0.15);
    let min_models = alg
        .and_then(|a| a.param_f64("consensus_min_models"))
        .unwrap_or(2.0) as u8;

    // For each foundation band, run an auto-materializing recall at
    // the cell. The auto-materializer is the same path /v1/recall
    // exercises — fetches+signs on miss, no special-casing needed.
    let bands = ["clay_v1", "prithvi_eo2", "geotessera.multi_year"];
    let mut available: Vec<&'static str> = Vec::new();
    for band in &bands {
        let req = crate::RecallReq {
            cell: cell.clone(),
            bands: Some(vec![(*band).to_string()]),
            tslot: None,
        };
        if let Ok((resp, _notes)) = crate::recall_with_auto_materialize(&req, &s).await {
            if !resp.facts.is_empty() {
                available.push(band);
            }
        }
    }
    json!({
        "intent":               "change",
        "available":            !available.is_empty(),
        "computation":          "triple_consensus_year_over_year",
        "algorithm_key":        "clay_prithvi_tessera_triple_consensus@1",
        "encoders_available":   available,
        "consensus_threshold":  gate,
        "consensus_min_models": min_models,
        "agent_hint":           "Apply the formula in `algorithms_for_question` for clay_prithvi_tessera_triple_consensus@1; the receipt cites the encoder fact CIDs alongside any topic-anchored scalar bands. For domain-specific variants see deforestation_triple@1 (forest), wetland_change_triple@1 (water), urban_expansion_triple@1 (urban), or coastal_erosion_triple@1 (coast).",
    })
}
