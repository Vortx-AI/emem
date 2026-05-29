//! `verify(claim, cell, mode)` — spec §8 / MCP `emem.verify`.
//!
//! Fast mode: look up canonical fact CIDs at the claim's `(cell, band, tslot|window)`,
//! evaluate the op against the stored values, return verdict + evidence CIDs.
//!
//! Resolve mode: when no fact exists at the (cell, band, tslot) the responder
//! requests materialization through `Storage::materialize_many` and re-scans;
//! a true `MaterializeMiss` (no upstream connector for this band) is surfaced
//! to the caller rather than silently collapsed to `verdict=false`.

use std::time::Instant;

use serde::{Deserialize, Serialize};

use emem_cache::CanonicalKey;
use emem_claim::{Claim, Op};
use emem_fact::{Fact, FactCid, Receipt};
use emem_storage::{Server, StorageError};

use crate::cbor_ops::{eq, lt};

/// Verification mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Look up canonical fact_cid; agree/disagree+evidence; no inference.
    Fast,
    /// If the fact is missing, trigger materialization (lazy fetch + attest)
    /// then re-evaluate. Honest gap: when the claim window is open-ended and
    /// no specific tslot is named, materialization needs a tslot to target —
    /// in that case Resolve degrades to Fast over the existing index.
    Resolve,
}

/// verify request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyReq {
    /// Structured claim.
    pub claim: Claim,
    /// Cell (cell64). `cell64` is accepted as an alias.
    #[serde(alias = "cell64")]
    pub cell: String,
    /// Mode (default Fast).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<Mode>,
}

/// verify response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyResp {
    /// Verdict.
    pub verdict: bool,
    /// CIDs of facts cited as evidence.
    pub evidence: Vec<FactCid>,
    /// `true` when the claim's `agg=mean` path applied a cos(lat)
    /// equal-area weight per cell. False for non-mean aggs (order
    /// statistics aren't reweighted) or when no aggregation ran.
    #[serde(default)]
    pub cos_lat_weighting_applied: bool,
    /// Signed receipt.
    pub receipt: Receipt,
}

/// Run a verification.
pub async fn verify(req: &VerifyReq, srv: &Server) -> Result<VerifyResp, StorageError> {
    let started = Instant::now();
    let storage = srv.storage.as_ref();
    let mode = req.mode.unwrap_or(Mode::Fast);

    let scan_scoped =
        |pairs: Vec<(emem_cache::CanonicalKey, FactCid)>| -> Vec<(emem_cache::CanonicalKey, FactCid)> {
            pairs
                .into_iter()
                .filter(|(k, _)| {
                    if k.band != req.claim.band {
                        return false;
                    }
                    match (req.claim.tslot, req.claim.window) {
                        (Some(t), _) => k.tslot == t,
                        (None, Some([s, e])) => k.tslot >= s && k.tslot <= e,
                        (None, None) => true,
                    }
                })
                .collect()
        };

    let pairs = storage.scan_cell(&req.cell, None).await?;
    let mut scoped = scan_scoped(pairs);

    // Resolve mode: when the band has no fact at the targeted tslot, ask the
    // storage layer to materialize it (function registry → upstream fetch →
    // signed Primary fact). A `MaterializeMiss` here means no upstream
    // connector exists for this band — surfaced to the caller, NOT silenced
    // to verdict=false. Open-ended windows (no `tslot`, no single-point
    // `window`) can't pick a target tslot to materialize, so they fall back
    // to Fast over whatever is already in the index.
    if matches!(mode, Mode::Resolve) && scoped.is_empty() {
        let target_tslot: Option<u64> = match (req.claim.tslot, req.claim.window) {
            (Some(t), _) => Some(t),
            (None, Some([s, e])) if s == e => Some(s),
            _ => None,
        };
        if let Some(tslot) = target_tslot {
            let key = CanonicalKey {
                cell: req.cell.clone(),
                band: req.claim.band.clone(),
                tslot,
            };
            // materialize_many returns an error if nothing produces this band;
            // bubble it so the caller can register a connector. On success we
            // re-scan to pick up the freshly-attested fact.
            storage.materialize_many(std::slice::from_ref(&key)).await?;
            let pairs = storage.scan_cell(&req.cell, None).await?;
            scoped = scan_scoped(pairs);
        }
    }

    let cids: Vec<FactCid> = scoped.iter().map(|(_, c)| c.clone()).collect();
    let facts: Vec<Fact> = storage
        .get_facts_many(&cids)
        .await?
        .into_iter()
        .flatten()
        .collect();

    // Carry the cell64 alongside each primary value so the `mean`
    // aggregate path can apply a cos(lat) equal-area weight. Every
    // scoped fact came from `req.cell`, so they all share the same
    // cell — but threading it through evaluate() keeps the contract
    // honest if a future multi-cell verify ships.
    let mut value_pairs: Vec<(&str, &ciborium::Value)> = Vec::new();
    let mut absences = false;
    for f in &facts {
        match f {
            Fact::Primary(p) => value_pairs.push((req.cell.as_str(), &p.value)),
            Fact::Absence(_) => absences = true,
            Fact::Derivative(_) => {}
        }
    }

    let verdict = evaluate(&req.claim, &value_pairs, absences);

    let receipt = srv.sign_receipt(
        "emem.verify",
        vec![req.cell.clone()],
        cids.clone(),
        true,
        started,
        None,
    );
    let cos_lat_weighting_applied = req.claim.agg.as_deref() == Some("mean");
    Ok(VerifyResp {
        verdict,
        evidence: cids,
        cos_lat_weighting_applied,
        receipt,
    })
}

fn evaluate(claim: &Claim, value_pairs: &[(&str, &ciborium::Value)], absences: bool) -> bool {
    if matches!(claim.op, Op::Exists) {
        return !value_pairs.is_empty();
    }
    if matches!(claim.op, Op::Absent) {
        return absences;
    }
    if value_pairs.is_empty() {
        return false;
    }

    let agg = claim.agg.as_deref().unwrap_or("any");
    let per: Vec<bool> = value_pairs
        .iter()
        .map(|(_, v)| eval_one(claim, v))
        .collect();
    match agg {
        "any" => per.iter().any(|x| *x),
        "all" => per.iter().all(|x| *x),
        // Numeric aggregates compare a fold of values to claim.value.
        // `mean` is reweighted by cos(centroid_lat) so a multi-cell
        // verify across a lat/lng-uniform sample doesn't over-weight
        // polar cells — at lat 60° each cell covers half the area of
        // an equator cell, so the unweighted mean overstates polar
        // contributions. min/max are order statistics and don't change.
        "mean" | "min" | "max" => {
            if agg == "mean" {
                let weighted: Vec<(f64, f64)> = value_pairs
                    .iter()
                    .filter_map(|(cell, v)| {
                        crate::cbor_ops::as_f64(v)
                            .map(|x| (emem_codec::equal_area_weight_for(cell), x))
                    })
                    .collect();
                if weighted.is_empty() {
                    return false;
                }
                let sum_w: f64 = weighted.iter().map(|(w, _)| *w).sum();
                let folded = if sum_w > 0.0 {
                    weighted.iter().map(|(w, x)| w * x).sum::<f64>() / sum_w
                } else {
                    let n = weighted.len() as f64;
                    weighted.iter().map(|(_, x)| *x).sum::<f64>() / n
                };
                let folded_v = ciborium::Value::Float(folded);
                return eval_one(claim, &folded_v);
            }
            let nums: Vec<f64> = value_pairs
                .iter()
                .filter_map(|(_, v)| crate::cbor_ops::as_f64(v))
                .collect();
            if nums.is_empty() {
                return false;
            }
            let folded = match agg {
                "min" => nums.iter().cloned().fold(f64::INFINITY, f64::min),
                "max" => nums.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
                _ => unreachable!(),
            };
            let folded_v = ciborium::Value::Float(folded);
            eval_one(claim, &folded_v)
        }
        _ => per.iter().any(|x| *x),
    }
}

fn eval_one(claim: &Claim, fact_value: &ciborium::Value) -> bool {
    match claim.op {
        Op::Eq => eq(fact_value, &claim.value),
        Op::Ne => !eq(fact_value, &claim.value),
        Op::Lt => lt(fact_value, &claim.value).unwrap_or(false),
        Op::Le => lt(fact_value, &claim.value).unwrap_or(false) || eq(fact_value, &claim.value),
        Op::Gt => lt(&claim.value, fact_value).unwrap_or(false),
        Op::Ge => lt(&claim.value, fact_value).unwrap_or(false) || eq(fact_value, &claim.value),
        Op::In => match &claim.value {
            ciborium::Value::Array(set) => set.iter().any(|x| eq(fact_value, x)),
            _ => false,
        },
        Op::Ni => match &claim.value {
            ciborium::Value::Array(set) => !set.iter().any(|x| eq(fact_value, x)),
            _ => false,
        },
        Op::Exists | Op::Absent => false, // handled by the caller
    }
}
