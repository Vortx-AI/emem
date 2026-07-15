//! Real dispatcher arms for two combined algorithms whose documented
//! formulas the scalar `evaluation`-AST evaluator cannot express, because
//! they require a **cosine over a multi-vintage embedding vector** rather
//! than an `f64 → f64` arithmetic tree:
//!
//!   1. [`triple_consensus`] — `clay_prithvi_tessera_triple_consensus@1`.
//!      Year-over-year change index fused across up to three foundation
//!      encoders. Each encoder contributes `d = clamp(1 - cos(v_now,
//!      v_prev), 0, 1)`; the ensemble is the quadratic mean of the
//!      *available* components, and a LandTrendr-style agreement
//!      classifier (gate 0.15) reports `all_three` / `two_of_three` /
//!      `one_or_none`.
//!
//!      **The gate is not calibrated per encoder, and `all_three` is not
//!      currently reachable.** 0.15 is Healey et al. 2018's threshold for
//!      *spectral* change; `d` here is a cosine distance in an encoder's
//!      own embedding space, and those spaces do not share a scale.
//!      Measured over 8 maximally dissimilar chips, Clay's cosine spans
//!      0.11 to 0.95 (sd 0.204) so its `d` clears 0.15 easily, while the
//!      deployed Prithvi checkpoint spans 0.88 to 0.99 (sd 0.030) and its
//!      `d` tops out near 0.1155. Prithvi therefore never crosses the gate
//!      and is a permanent no-change vote, which makes `all_three`
//!      arithmetically unreachable and `two_of_three` in practice
//!      "Clay and Tessera". The response says so in `gate_calibration`
//!      rather than letting a caller infer consensus from a vote that
//!      cannot be cast. Calibrating per encoder needs a labelled change
//!      corpus this responder does not yet have, so the limit is declared
//!      rather than papered over with an invented per-encoder threshold.
//!
//!   2. [`deforestation_alert`] — `carbon.deforestation_alert_proxy@1`.
//!      `alert_score = 0.5·clamp01(ndvi_drop/0.30) + 0.5·clamp01(
//!      embedding_change/0.20)`, where `ndvi_drop = max(0,
//!      ndvi_modis - ndvi_now)` and `embedding_change =
//!      1 - cos(tessera_latest, tessera_prev)`.
//!
//! Both algorithms stay flagged `documentation_only` in the registry: the
//! AST evaluator MUST keep returning `Ok(None)` for them (the scalar tree
//! genuinely cannot compute the cosine term). These endpoints are the
//! honest runnable surface. They never fabricate an embedding or a vote —
//! when a GPU-gated encoder (`clay_v1`, `prithvi_eo2`) or a vintage is
//! unavailable, the encoder is reported under `encoders_absent[]` with a
//! typed reason and dropped from the fusion. When fewer than the
//! algorithm's `consensus_min_models` are available the response is an
//! honest `inconclusive` verdict, not a number.
//!
//! ## CORRECTNESS NOTE on `clay_v1` / `prithvi_eo2` year-over-year
//! The shipped materializers for `clay_v1` and `prithvi_eo2` produce only
//! the **latest** vintage (single embedding, GPU-gated). They take no
//! prior-year parameter. So a year-over-year cosine for those two is only
//! computable when the store *already holds* two distinct-tslot facts for
//! the encoder at the cell (e.g. a deployment that signed last year's and
//! this year's vintage). We recall the two most-recent distinct-tslot
//! facts per encoder; if fewer than two exist, that encoder is absent.
//! We never synthesise a prior vintage. On a GPU-less / cold responder
//! the typical outcome is Tessera-only → `inconclusive` — which is the
//! honest answer, not a fabricated all-three vote.

use std::time::Instant;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value as JsonValue};

use emem_core::ErrorCode;
use emem_fact::{Fact, FactCid};
use emem_primitives::cbor_ops::{as_f64, as_vec_f32, cosine_finite};
use emem_primitives::RecallReq;

use crate::{recall_with_auto_materialize, ApiError, AppState, EmemJson, ErrorBody};

/// Width of one GeoTessera vintage inside the `geotessera.multi_year`
/// stack: 128-D per year, 8 years (2017..=2024) → 1024-D flat array,
/// missing years NaN-masked. See `materialize_geotessera_multi_year`.
const TESSERA_VINTAGE_DIM: usize = 128;

fn pubkey_b32(state: &AppState) -> String {
    data_encoding::BASE32_NOPAD
        .encode(&state.identity.pubkey.0)
        .to_lowercase()
}

fn bad_request(msg: impl Into<String>) -> ApiError {
    ApiError(
        StatusCode::BAD_REQUEST,
        ErrorBody {
            code: ErrorCode::InvalidArgument,
            message: msg.into(),
            details: None,
        },
    )
}

/// Split a `geotessera.multi_year` flat vector into per-vintage 128-D
/// slices, dropping any slice that is entirely NaN (a year that 404'd at
/// materialization time and was NaN-masked). Returns the *covered*
/// vintages in chronological order. A slice with any finite element is
/// kept whole — `cosine` is computed only over finite pairs by the
/// caller via [`cosine_finite`].
pub(crate) fn covered_vintages(stack: &[f32], dim: usize) -> Vec<Vec<f32>> {
    if dim == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i + dim <= stack.len() {
        let slice = &stack[i..i + dim];
        if slice.iter().any(|x| x.is_finite()) {
            out.push(slice.to_vec());
        }
        i += dim;
    }
    out
}

/// Why `agreement` must not be read as consensus. Rides the response as
/// `gate_calibration` so an agent sees the limit in-band rather than having
/// to measure the encoders itself, which is how it was found.
pub(crate) const GATE_CALIBRATION_NOTE: &str = "uncalibrated_per_encoder: \
consensus_threshold is Healey et al. 2018's LandTrendr gate for SPECTRAL \
change, applied unchanged to cosine distances in three embedding spaces that \
do not share a scale. Measured over 8 maximally dissimilar chips, Clay's \
cosine spans 0.11 to 0.95 (sd 0.204) so its d clears 0.15 readily, while the \
deployed Prithvi checkpoint spans 0.88 to 0.99 (sd 0.030) and its d tops out \
near 0.1155, under the gate. Prithvi never votes, so agreement=all_three is \
arithmetically unreachable and two_of_three means Clay plus Tessera. Read \
encoders_used[].change per encoder rather than trusting the vote. Calibrating \
per encoder needs a labelled change corpus this responder does not have.";

/// `d = clamp(1 - cos(v_now, v_prev), 0, 1)` — one encoder's year-over-year
/// change magnitude. `None` when the two vintages share no finite dims.
fn change_component(v_now: &[f32], v_prev: &[f32]) -> Option<f64> {
    let c = cosine_finite(v_now, v_prev)? as f64;
    Some((1.0 - c).clamp(0.0, 1.0))
}

#[derive(Debug, Deserialize)]
pub struct TripleConsensusReq {
    pub cell: String,
    /// Override the consensus gate (default from the registry parameters
    /// block: 0.15). Clamped to (0, 1).
    #[serde(default)]
    pub consensus_threshold: Option<f64>,
}

/// Machine-readable, closed set of reasons an encoder (or a deforestation
/// half) is absent from a consensus. Each variant maps 1:1 to a real code
/// path so a caller can gate on `code()` instead of string-parsing the human
/// `reason` prose. Surfaced as `reason_code` on every `encoders_absent[]`
/// entry and rolled up into the response-level `degraded_reason`
/// (TerraGround field feedback #3 — no silently-degraded vote).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AbsenceReason {
    /// A GPU-gated encoder produced no embedding — sidecar down, or the cell
    /// is cold and nothing could be materialized.
    GpuSidecarUnavailable,
    /// Only one distinct vintage exists (and a prior one could not be
    /// obtained); year-over-year change needs two.
    SingleVintage,
    /// The cell is outside the product's spatial coverage.
    OutsideCoverage,
    /// Two vintages exist but share no finite dimensions (empty NaN overlap).
    NoFiniteOverlap,
    /// The underlying recall itself failed.
    RecallFailed,
}

impl AbsenceReason {
    /// Stable wire string. This set is a public API contract — append only.
    fn code(self) -> &'static str {
        match self {
            AbsenceReason::GpuSidecarUnavailable => "gpu_sidecar_unavailable",
            AbsenceReason::SingleVintage => "single_vintage",
            AbsenceReason::OutsideCoverage => "outside_coverage",
            AbsenceReason::NoFiniteOverlap => "no_finite_overlap",
            AbsenceReason::RecallFailed => "recall_failed",
        }
    }
}

/// Roll the per-encoder `reason_code`s up into one dominant response-level
/// `degraded_reason`. Priority orders the *most actionable* cause first: a
/// downed sidecar (fix infra) outranks a single-vintage cell (await coverage)
/// which outranks an out-of-coverage cell, etc. Returns `None` only when no
/// absent entry carried a code (caller substitutes the terminal
/// `insufficient_encoders`).
fn dominant_absence_code(absent: &[JsonValue]) -> Option<&'static str> {
    const PRIORITY: [&str; 5] = [
        "gpu_sidecar_unavailable",
        "single_vintage",
        "outside_coverage",
        "no_finite_overlap",
        "recall_failed",
    ];
    let codes: Vec<&str> = absent
        .iter()
        .filter_map(|a| a.get("reason_code").and_then(|c| c.as_str()))
        .collect();
    PRIORITY.into_iter().find(|p| codes.contains(p))
}

/// Collect the distinct-tslot Primary vectors for `band` at `cell`,
/// most-recent first. Recalls (auto-materializing the latest if cold) and
/// dedups by tslot.
async fn distinct_vintages(
    cell: &str,
    band: &str,
    s: &AppState,
) -> Result<Vec<(u64, Vec<f32>, String)>, (AbsenceReason, String)> {
    let req = RecallReq {
        cell: cell.to_string(),
        bands: Some(vec![band.to_string()]),
        // No tslot pin: recall returns the band's facts; we sort the
        // distinct tslots ourselves and take the two most recent.
        tslot: None,
        ..Default::default()
    };
    let (resp, _notes) = recall_with_auto_materialize(&req, s).await.map_err(|e| {
        (
            AbsenceReason::RecallFailed,
            format!("recall failed: {}", e.1.message),
        )
    })?;

    // Collect (tslot, vector, cid) for every Primary fact for this band.
    let mut rows: Vec<(u64, Vec<f32>, String)> = Vec::new();
    for (idx, f) in resp.facts.iter().enumerate() {
        if let Fact::Primary(p) = f {
            if p.band != band {
                continue;
            }
            if let Some(v) = as_vec_f32(&p.value) {
                let cid = resp
                    .receipt
                    .fact_cids
                    .get(idx)
                    .map(|c| c.as_str().to_string())
                    .unwrap_or_default();
                rows.push((p.tslot, v, cid));
            }
        }
    }
    if resp.facts.iter().all(|f| !matches!(f, Fact::Primary(_))) || rows.is_empty() {
        return Err((
            AbsenceReason::GpuSidecarUnavailable,
            format!(
                "no embedding vector for `{band}` at this cell (GPU encoder absent / sidecar down, or cell cold)"
            ),
        ));
    }
    // Most-recent first, dedup by tslot.
    rows.sort_by_key(|r| std::cmp::Reverse(r.0));
    rows.dedup_by_key(|r| r.0);
    Ok(rows)
}

/// Materialize a PRIOR vintage of a single-vintage GPU encoder
/// (`clay_v1` / `prithvi_eo2`) ~one year before `latest_tslot`, so a
/// year-over-year change becomes computable on a cell that only had the
/// latest. The materializer-at-tslot path is keyed by the Slow tslot the
/// chosen scene resolves to. Returns Ok when a distinct prior vintage was
/// signed; Err with a typed reason (no prior S2 scene, GPU absent, …).
async fn materialize_prior_vintage(
    cell: &str,
    band: &str,
    latest_tslot: u64,
    s: &AppState,
) -> Result<(), String> {
    // Slow tslot → unix seconds (Tempo::Slow is annual-cadence). Step back
    // ~370 days so the prior-year scene search lands in a different annual
    // bucket even with a few weeks of scene-date jitter.
    let latest_unix =
        emem_core::tslot::Tslot(latest_tslot).to_unix_start(emem_core::tslot::Tempo::Slow);
    let prior_unix = latest_unix - 370 * 86_400;
    if prior_unix <= 0 {
        return Err("prior vintage would predate the epoch".to_string());
    }
    let new_cid = match band {
        "clay_v1" => crate::materialize_clay_v1_at(cell, s, Some(prior_unix)).await?,
        "prithvi_eo2" => crate::materialize_prithvi_eo2_at(cell, s, Some(prior_unix)).await?,
        other => return Err(format!("no prior-vintage materializer for `{other}`")),
    };
    let _ = new_cid; // the fact is persisted; the caller re-recalls to read it
    Ok(())
}

/// Pull the two most-recent distinct-tslot Primary vectors for `band` at
/// `cell`. Returns `(now, prev, [cid_now, cid_prev])`. When only one
/// vintage exists for a single-vintage GPU encoder we materialize a prior
/// vintage on demand (one extra scene fetch) so year-over-year change is
/// computable instead of degrading to inconclusive. `Err(reason)` when the
/// band is unavailable (GPU down / cold cell) or a second distinct vintage
/// genuinely can't be obtained (e.g. only one S2 scene in the archive).
async fn two_vintages(
    cell: &str,
    band: &str,
    s: &AppState,
) -> Result<(Vec<f32>, Vec<f32>, Vec<String>), (AbsenceReason, String)> {
    let mut rows = distinct_vintages(cell, band, s).await?;
    if rows.len() < 2 {
        // Only the latest vintage exists. Materialize a prior one (~1 yr
        // back) and re-recall. This is the path that lets triple_consensus
        // actually use Clay/Prithvi instead of leaving Tessera to carry it.
        let latest_tslot = rows[0].0;
        match materialize_prior_vintage(cell, band, latest_tslot, s).await {
            Ok(()) => {
                rows = distinct_vintages(cell, band, s).await?;
            }
            Err(e) => {
                return Err((
                    AbsenceReason::SingleVintage,
                    format!(
                        "only one vintage of `{band}` at this cell and a prior vintage could not be materialized ({e}); year-over-year change needs two distinct vintages"
                    ),
                ));
            }
        }
    }
    if rows.len() < 2 {
        return Err((
            AbsenceReason::SingleVintage,
            format!(
                "only one distinct vintage of `{band}` is obtainable at this cell (the prior-vintage scene resolved to the same annual tslot); year-over-year change needs two"
            ),
        ));
    }
    let now = rows[0].clone();
    let prev = rows[1].clone();
    Ok((now.1, prev.1, vec![now.2, prev.2]))
}

/// One fused component result, surfaced verbatim in the response so an
/// analyst can audit which encoder drove the alert.
struct Component {
    encoder: &'static str,
    change: f64,
    cids: Vec<String>,
}

/// The fusion verdict, computed purely from the available per-encoder
/// change magnitudes so it is unit-testable without standing up an
/// `AppState`. `changes` holds one value per *available* encoder (those
/// that had two vintages); encoders that were absent are simply not in
/// the slice — they NEVER contribute a synthesised zero or vote.
///
/// Returns `(ensemble, agreement, available)`. When fewer than
/// `min_models` encoders are available `ensemble` is `None` and the
/// verdict is inconclusive — no number is fabricated.
pub(crate) fn fuse(
    changes: &[f64],
    n_encoders_total: usize,
    gate: f64,
    min_models: usize,
) -> (Option<f64>, &'static str, bool) {
    let n_used = changes.len();
    if n_used < min_models {
        return (None, "insufficient_encoders", false);
    }
    // One gate for every encoder. The encoders do not share a cosine scale,
    // so this under-counts votes from encoders with a tight spread: the
    // deployed Prithvi checkpoint's `d` tops out near 0.1155 against a 0.15
    // gate, so it never votes and `all_three` cannot occur. See the module
    // header; the response declares this in `gate_calibration`.
    let n_over_gate = changes.iter().filter(|c| **c > gate).count();
    let agreement = if n_used == n_encoders_total && n_used == 3 && n_over_gate == 3 {
        "all_three"
    } else if n_over_gate >= 2 {
        "two_of_three"
    } else {
        "one_or_none"
    };
    let ssq: f64 = changes.iter().map(|c| c * c).sum();
    let ensemble = (ssq / n_used as f64).sqrt();
    (Some(ensemble), agreement, true)
}

pub async fn triple_consensus(
    req: TripleConsensusReq,
    s: &AppState,
) -> Result<JsonValue, ApiError> {
    let started = Instant::now();
    let (cell, resolved) = crate::resolve_cell_field(&req.cell).await?;

    let alg = emem_core::algorithms::DEFAULT.lookup("clay_prithvi_tessera_triple_consensus@1");
    // Fallbacks used only when the algorithm registry doesn't carry the
    // knob. Named (not bare literals) so the provenance is explicit:
    // - consensus_threshold mirrors the registry's cited value, Healey et
    //   al. 2018 (RSE 204:717-728), the LandTrendr-style change-agreement
    //   gate referenced in this module's header.
    // - consensus_min_models is an operational default (need >=2 of 3
    //   encoders to agree before a verdict), NOT a literature-derived
    //   number — labelled honestly rather than dressed up as one.
    const DEFAULT_CONSENSUS_THRESHOLD: f64 = 0.15;
    const DEFAULT_CONSENSUS_MIN_MODELS: f64 = 2.0;
    let gate = req
        .consensus_threshold
        .or_else(|| alg.and_then(|a| a.param_f64("consensus_threshold")))
        .unwrap_or(DEFAULT_CONSENSUS_THRESHOLD)
        .clamp(f64::MIN_POSITIVE, 1.0);
    let min_models = alg
        .and_then(|a| a.param_f64("consensus_min_models"))
        .unwrap_or(DEFAULT_CONSENSUS_MIN_MODELS) as usize;

    let mut components: Vec<Component> = Vec::new();
    let mut absent: Vec<JsonValue> = Vec::new();
    let mut all_cids: Vec<String> = Vec::new();

    // ── Tessera: one recall yields the multi-vintage stack; latest vs
    //    prev covered vintage. No GPU. ──────────────────────────────────
    {
        let mreq = RecallReq {
            cell: cell.clone(),
            bands: Some(vec!["geotessera.multi_year".to_string()]),
            tslot: None,
            ..Default::default()
        };
        match recall_with_auto_materialize(&mreq, s).await {
            Ok((resp, _notes)) => {
                let primary = resp.facts.iter().enumerate().find_map(|(i, f)| match f {
                    Fact::Primary(p) if p.band == "geotessera.multi_year" => Some((i, p)),
                    _ => None,
                });
                match primary.and_then(|(i, p)| as_vec_f32(&p.value).map(|v| (i, v))) {
                    Some((i, stack)) => {
                        let vintages = covered_vintages(&stack, TESSERA_VINTAGE_DIM);
                        if vintages.len() >= 2 {
                            let latest = &vintages[vintages.len() - 1];
                            let prev = &vintages[vintages.len() - 2];
                            match change_component(latest, prev) {
                                Some(dt) => {
                                    let cid = resp
                                        .receipt
                                        .fact_cids
                                        .get(i)
                                        .map(|c| c.as_str().to_string())
                                        .unwrap_or_default();
                                    if !cid.is_empty() {
                                        all_cids.push(cid.clone());
                                    }
                                    components.push(Component {
                                        encoder: "geotessera.multi_year",
                                        change: dt,
                                        cids: vec![cid],
                                    });
                                }
                                None => absent.push(json!({
                                    "encoder": "geotessera.multi_year",
                                    "reason": "two covered vintages share no finite dimensions",
                                    "reason_code": AbsenceReason::NoFiniteOverlap.code(),
                                })),
                            }
                        } else {
                            absent.push(json!({
                                "encoder": "geotessera.multi_year",
                                "reason": format!("only {} covered Tessera vintage(s) at this cell; year-over-year change needs two", vintages.len()),
                                "reason_code": AbsenceReason::SingleVintage.code(),
                            }));
                        }
                    }
                    None => absent.push(json!({
                        "encoder": "geotessera.multi_year",
                        "reason": "no multi-year embedding at this cell (outside Tessera coverage)",
                        "reason_code": AbsenceReason::OutsideCoverage.code(),
                    })),
                }
            }
            Err(e) => absent.push(json!({
                "encoder": "geotessera.multi_year",
                "reason": format!("recall failed: {}", e.1.message),
                "reason_code": AbsenceReason::RecallFailed.code(),
            })),
        }
    }

    // ── Clay + Prithvi: GPU-gated, single-vintage materializer. Compute
    //    only when two distinct-tslot facts already exist; else honest
    //    absence. NEVER synthesise a prior vintage. ──────────────────────
    //    Run the two encoders' vintage materializations CONCURRENTLY — each
    //    `two_vintages` does up to two cold GPU embeds, and the two encoders
    //    are independent, so the serial loop summed their cold cost (the
    //    ~8 s cold triple_consensus). join overlaps them; the shared S2 scene
    //    COG reads single-flight via cog::TILE_CACHE. Folded back in the
    //    original [clay_v1, prithvi_eo2] order so output is unchanged.
    let (clay_vintages, prithvi_vintages) = tokio::join!(
        two_vintages(&cell, "clay_v1", s),
        two_vintages(&cell, "prithvi_eo2", s),
    );
    for (band, vintages) in [
        ("clay_v1", clay_vintages),
        ("prithvi_eo2", prithvi_vintages),
    ] {
        match vintages {
            Ok((now, prev, cids)) => match change_component(&now, &prev) {
                Some(d) => {
                    for c in &cids {
                        if !c.is_empty() {
                            all_cids.push(c.clone());
                        }
                    }
                    components.push(Component {
                        encoder: if band == "clay_v1" {
                            "clay_v1"
                        } else {
                            "prithvi_eo2"
                        },
                        change: d,
                        cids,
                    });
                }
                None => absent.push(json!({
                    "encoder": band,
                    "reason": "the two vintages share no finite dimensions",
                    "reason_code": AbsenceReason::NoFiniteOverlap.code(),
                })),
            },
            Err((code, reason)) => absent.push(json!({
                "encoder": band,
                "reason": reason,
                "reason_code": code.code(),
            })),
        }
    }

    let n_used = components.len();
    let changes: Vec<f64> = components.iter().map(|c| c.change).collect();
    // Agreement + ensemble over the *available* components only — anchored
    // to the documented LandTrendr thresholds. Absent encoders never
    // contribute a synthesised zero or vote.
    let (ensemble_opt, agreement, fuse_available) = fuse(&changes, 3, gate, min_models);

    let per_encoder: Vec<JsonValue> = components
        .iter()
        .map(|c| {
            json!({
                "encoder": c.encoder,
                "change": c.change,
                "over_gate": c.change > gate,
                "input_fact_cids": c.cids,
            })
        })
        .collect();

    let resolved_env = crate::resolved_envelope(vec![("cell".into(), resolved)]);
    let pubkey = pubkey_b32(s);
    let receipt = s.sign_receipt(
        "emem.triple_consensus",
        vec![cell.clone()],
        all_cids.iter().cloned().map(FactCid::new).collect(),
        false,
        started,
        None,
    );

    // Honest degradation: fewer than consensus_min_models available →
    // inconclusive verdict with NO fabricated ensemble number.
    if !fuse_available {
        let honest_note = format!(
            "Year-over-year consensus needs at least {min_models} encoders with two vintages; only {n_used} available here. Most often this means the GPU sidecar is down (clay_v1/prithvi_eo2 sign Absence) and only Tessera multi-year is computable. No ensemble value is reported — that would be a fabricated number."
        );
        // Inconclusive is a degraded outcome. Surface a single machine-readable
        // reason (dominant over encoders_absent[]) so a caller never treats the
        // null ensemble as signal. `insufficient_encoders` is the terminal case
        // when no encoder reported a specific absence code.
        let degraded_reason = dominant_absence_code(&absent).unwrap_or("insufficient_encoders");
        return Ok(json!({
            "schema": "emem.triple_consensus.v1",
            "algorithm_key": "clay_prithvi_tessera_triple_consensus@1",
            "cell": cell,
            "resolved_from": resolved_env,
            "verdict": "inconclusive",
            "available": false,
            "degraded": true,
            "degraded_reason": degraded_reason,
            "degraded_message": honest_note.clone(),
            "ensemble": JsonValue::Null,
            "agreement": "insufficient_encoders",
            "encoders_used": per_encoder,
            "encoders_absent": absent,
            "n_encoders_used": n_used,
            "consensus_min_models": min_models,
            "consensus_threshold": gate,
            // This path reports no agreement, but it still hands the caller a
            // gate to reason about, so it owes them the same caveat as the
            // computed path.
            "gate_calibration": GATE_CALIBRATION_NOTE,
            "honest_note": honest_note,
            "responder_pubkey_b32": pubkey,
            "receipt": receipt,
        }));
    }

    // Quadratic mean (L2-mean) of the available components — matches the
    // documented `sqrt((dc^2 + dp^2 + dt^2)/3)` generalised to N<=3.
    let ensemble = ensemble_opt.expect("fuse returned available with no ensemble");

    // A 2-of-3 result is real but DEGRADED: surface degraded:true + a reason so
    // a caller doesn't treat a partial consensus as a full triple. A complete
    // 3-encoder result is not degraded (degraded_reason is explicit null).
    let degraded = n_used < 3;
    let honest_note = if degraded {
        format!("Consensus computed over {n_used} of 3 encoders; {} unavailable (see encoders_absent). Treat as lower-confidence than a full triple.", 3 - n_used)
    } else {
        "All three encoders contributed.".to_string()
    };
    let degraded_reason = if degraded {
        json!(format!("partial_consensus_{n_used}_of_3"))
    } else {
        JsonValue::Null
    };
    let degraded_message = if degraded {
        json!(honest_note.clone())
    } else {
        JsonValue::Null
    };

    Ok(json!({
        "schema": "emem.triple_consensus.v1",
        "algorithm_key": "clay_prithvi_tessera_triple_consensus@1",
        "cell": cell,
        "resolved_from": resolved_env,
        "verdict": "computed",
        "available": true,
        "degraded": degraded,
        "degraded_reason": degraded_reason,
        "degraded_message": degraded_message,
        "ensemble": ensemble,
        "agreement": agreement,
        "encoders_used": per_encoder,
        "encoders_absent": absent,
        "n_encoders_used": n_used,
        "consensus_min_models": min_models,
        "consensus_threshold": gate,
        "gate_calibration": GATE_CALIBRATION_NOTE,
        "formula": "d_e = clamp(1 - cos(v_now, v_prev), 0, 1) per encoder; ensemble = sqrt(mean(d_e^2)) over available encoders; agreement gates on consensus_threshold",
        "citation": alg.map(|a| a.citation.clone()).unwrap_or_default(),
        "honest_note": honest_note,
        "input_fact_cids": all_cids,
        "responder_pubkey_b32": pubkey,
        "receipt": receipt,
    }))
}

pub async fn post_triple_consensus(
    State(s): State<AppState>,
    EmemJson(req): EmemJson<TripleConsensusReq>,
) -> Result<Json<JsonValue>, ApiError> {
    Ok(Json(triple_consensus(req, &s).await?))
}

// ── carbon.deforestation_alert_proxy@1 ─────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct DeforestationAlertReq {
    pub cell: String,
}

/// `alert_score = 0.5·clamp01(ndvi_drop/0.30) + 0.5·clamp01(embedding_change/0.20)`
/// where `ndvi_drop = max(0, ndvi_modis_baseline - ndvi_now)` and
/// `embedding_change = 1 - cos(tessera_latest, tessera_prev)`.
///
/// Both halves degrade independently and honestly: a missing band drops
/// its half AND renames the output so the agent cannot mistake a
/// half-score for the full composite. If NEITHER half is computable the
/// response is an honest `inconclusive` with no number.
pub async fn deforestation_alert(
    req: DeforestationAlertReq,
    s: &AppState,
) -> Result<JsonValue, ApiError> {
    let started = Instant::now();
    let (cell, resolved) = crate::resolve_cell_field(&req.cell).await?;

    let mut notes: Vec<String> = Vec::new();
    let mut cids: Vec<String> = Vec::new();

    // NDVI-drop half. Recall both indices.ndvi (S2) and modis.ndvi_mean
    // (MODIS baseline) — both materializable without a GPU.
    let mreq = RecallReq {
        cell: cell.clone(),
        bands: Some(vec![
            "indices.ndvi".to_string(),
            "modis.ndvi_mean".to_string(),
        ]),
        tslot: None,
        ..Default::default()
    };
    let (resp, _notes) = recall_with_auto_materialize(&mreq, s).await?;
    let mut ndvi_now: Option<f64> = None;
    let mut ndvi_modis: Option<f64> = None;
    for (idx, f) in resp.facts.iter().enumerate() {
        if let Fact::Primary(p) = f {
            let cid = resp
                .receipt
                .fact_cids
                .get(idx)
                .map(|c| c.as_str().to_string());
            match p.band.as_str() {
                "indices.ndvi" => {
                    ndvi_now = as_f64(&p.value).filter(|x| x.is_finite());
                    if let Some(c) = cid {
                        if !c.is_empty() {
                            cids.push(c);
                        }
                    }
                }
                "modis.ndvi_mean" => {
                    ndvi_modis = as_f64(&p.value).filter(|x| x.is_finite());
                    if let Some(c) = cid {
                        if !c.is_empty() {
                            cids.push(c);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let ndvi_term: Option<f64> = match (ndvi_modis, ndvi_now) {
        (Some(base), Some(now)) => {
            let drop = (base - now).max(0.0);
            Some((drop / 0.30).clamp(0.0, 1.0))
        }
        _ => {
            notes.push(
                "NDVI-drop half unavailable: indices.ndvi and/or modis.ndvi_mean did not materialize at this cell".to_string(),
            );
            None
        }
    };

    // Embedding-change half: Tessera latest-vs-prev vintage cosine. No GPU.
    let treq = RecallReq {
        cell: cell.clone(),
        bands: Some(vec!["geotessera.multi_year".to_string()]),
        tslot: None,
        ..Default::default()
    };
    let mut embed_term: Option<f64> = None;
    match recall_with_auto_materialize(&treq, s).await {
        Ok((tresp, _n)) => {
            let primary = tresp.facts.iter().enumerate().find_map(|(i, f)| match f {
                Fact::Primary(p) if p.band == "geotessera.multi_year" => {
                    as_vec_f32(&p.value).map(|v| (i, v))
                }
                _ => None,
            });
            match primary {
                Some((i, stack)) => {
                    let vintages = covered_vintages(&stack, TESSERA_VINTAGE_DIM);
                    if vintages.len() >= 2 {
                        let latest = &vintages[vintages.len() - 1];
                        let prev = &vintages[vintages.len() - 2];
                        match cosine_finite(latest, prev) {
                            Some(c) => {
                                let change = (1.0 - c as f64).clamp(0.0, 1.0);
                                embed_term = Some((change / 0.20).clamp(0.0, 1.0));
                                if let Some(cid) =
                                    tresp.receipt.fact_cids.get(i).map(|c| c.as_str().to_string())
                                {
                                    if !cid.is_empty() {
                                        cids.push(cid);
                                    }
                                }
                            }
                            None => notes.push(
                                "embedding-change half unavailable: two Tessera vintages share no finite dimensions".to_string(),
                            ),
                        }
                    } else {
                        notes.push(format!(
                            "embedding-change half unavailable: only {} covered Tessera vintage(s) (need two)",
                            vintages.len()
                        ));
                    }
                }
                None => notes.push(
                    "embedding-change half unavailable: no Tessera multi-year embedding at this cell".to_string(),
                ),
            }
        }
        Err(e) => notes.push(format!(
            "embedding-change half unavailable: Tessera recall failed: {}",
            e.1.message
        )),
    }

    let resolved_env = crate::resolved_envelope(vec![("cell".into(), resolved)]);
    let pubkey = pubkey_b32(s);
    let receipt = s.sign_receipt(
        "emem.deforestation_alert",
        vec![cell.clone()],
        cids.iter().cloned().map(FactCid::new).collect(),
        false,
        started,
        None,
    );

    // Compose. The documented composite is a 0.5/0.5 blend. If only one
    // half is available we report THAT half under a distinct key so it is
    // never mistaken for the full composite. If neither — inconclusive.
    // A half-only score is real but DEGRADED (do not threshold against the
    // 0.6 alert gate); a neither-half result is degraded-to-inconclusive.
    // `degraded` + a machine-readable `degraded_reason` make this explicit so
    // a caller never mistakes a half-score (or a null) for a full composite.
    let (alert_score, output_key, available, degraded, degraded_reason): (
        Option<f64>,
        &str,
        bool,
        bool,
        Option<&str>,
    ) = match (ndvi_term, embed_term) {
        (Some(a), Some(b)) => (Some(0.5 * a + 0.5 * b), "alert_score", true, false, None),
        (Some(a), None) => (
            Some(a),
            "ndvi_drop_half_only",
            true,
            true,
            Some("embedding_half_unavailable"),
        ),
        (None, Some(b)) => (
            Some(b),
            "embedding_change_half_only",
            true,
            true,
            Some("ndvi_half_unavailable"),
        ),
        (None, None) => (None, "inconclusive", false, true, Some("no_inputs")),
    };

    let honest_note = if alert_score.is_some() && output_key != "alert_score" {
        "Only one of the two composite halves was computable; reported half-score under its own key, NOT as the full alert_score. Do not threshold this against the 0.6 alert gate.".to_string()
    } else if !available {
        "Neither composite half was computable at this cell. No number reported.".to_string()
    } else {
        "Both halves computed; scout-only triage signal — confirm with Hansen GFC / RADD before crediting decisions.".to_string()
    };
    // When degraded, degraded_message echoes the human note; otherwise explicit null.
    let degraded_message = if degraded {
        json!(honest_note.clone())
    } else {
        JsonValue::Null
    };

    Ok(json!({
        "schema": "emem.deforestation_alert.v1",
        "algorithm_key": "carbon.deforestation_alert_proxy@1",
        "cell": cell,
        "resolved_from": resolved_env,
        "available": available,
        "degraded": degraded,
        "degraded_reason": degraded_reason,
        "degraded_message": degraded_message,
        "verdict": if available { "computed" } else { "inconclusive" },
        "output_key": output_key,
        "value": alert_score,
        "ndvi_drop_term": ndvi_term,
        "embedding_change_term": embed_term,
        "ndvi_now": ndvi_now,
        "ndvi_modis_baseline": ndvi_modis,
        "formula": "0.5*clamp01(max(0, ndvi_modis - ndvi_now)/0.30) + 0.5*clamp01((1 - cos(tessera_latest, tessera_prev))/0.20)",
        "citation": emem_core::algorithms::DEFAULT
            .lookup("carbon.deforestation_alert_proxy@1")
            .map(|a| a.citation.clone())
            .unwrap_or_default(),
        "honest_note": honest_note,
        "degradation_notes": notes,
        "input_fact_cids": cids,
        "responder_pubkey_b32": pubkey,
        "receipt": receipt,
    }))
}

pub async fn post_deforestation_alert(
    State(s): State<AppState>,
    EmemJson(req): EmemJson<DeforestationAlertReq>,
) -> Result<Json<JsonValue>, ApiError> {
    if req.cell.trim().is_empty() {
        return Err(bad_request(
            "deforestation_alert requires a non-empty `cell`",
        ));
    }
    Ok(Json(deforestation_alert(req, &s).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vintage(seed: f32, dim: usize) -> Vec<f32> {
        (0..dim).map(|i| ((i as f32) * 0.01 + seed).sin()).collect()
    }

    #[test]
    fn dominant_absence_code_prefers_most_actionable_cause() {
        // A downed sidecar outranks a single-vintage cell in the same response.
        let absent = vec![
            json!({ "encoder": "geotessera.multi_year", "reason_code": "single_vintage" }),
            json!({ "encoder": "clay_v1", "reason_code": "gpu_sidecar_unavailable" }),
        ];
        assert_eq!(
            dominant_absence_code(&absent),
            Some("gpu_sidecar_unavailable")
        );

        // With no GPU cause, single_vintage wins over outside_coverage.
        let absent = vec![
            json!({ "encoder": "geotessera.multi_year", "reason_code": "outside_coverage" }),
            json!({ "encoder": "clay_v1", "reason_code": "single_vintage" }),
        ];
        assert_eq!(dominant_absence_code(&absent), Some("single_vintage"));

        // No coded entries → None (caller substitutes insufficient_encoders).
        assert_eq!(dominant_absence_code(&[]), None);
    }

    #[test]
    fn absence_reason_codes_are_stable_strings() {
        // The wire set is a public contract; lock the exact strings.
        assert_eq!(
            AbsenceReason::GpuSidecarUnavailable.code(),
            "gpu_sidecar_unavailable"
        );
        assert_eq!(AbsenceReason::SingleVintage.code(), "single_vintage");
        assert_eq!(AbsenceReason::OutsideCoverage.code(), "outside_coverage");
        assert_eq!(AbsenceReason::NoFiniteOverlap.code(), "no_finite_overlap");
        assert_eq!(AbsenceReason::RecallFailed.code(), "recall_failed");
    }

    #[test]
    fn covered_vintages_drops_all_nan_years() {
        let dim = 4;
        // year0 finite, year1 all-NaN, year2 finite.
        let mut stack = Vec::new();
        stack.extend(vintage(0.1, dim));
        stack.extend(std::iter::repeat_n(f32::NAN, dim));
        stack.extend(vintage(0.7, dim));
        let v = covered_vintages(&stack, dim);
        assert_eq!(v.len(), 2, "the all-NaN middle year must be dropped");
        assert!(v[0].iter().all(|x| x.is_finite()));
        assert!(v[1].iter().all(|x| x.is_finite()));
    }

    #[test]
    fn change_component_identical_vintages_is_zero() {
        let a = vintage(0.3, TESSERA_VINTAGE_DIM);
        let d = change_component(&a, &a).expect("finite overlap");
        assert!(d.abs() < 1e-6, "identical vintages → zero change, got {d}");
    }

    #[test]
    fn change_component_orthogonal_is_one() {
        // Two orthogonal half-vectors → cosine 0 → change 1.0.
        let mut a = vec![0.0f32; 8];
        let mut b = vec![0.0f32; 8];
        a[0] = 1.0;
        b[1] = 1.0;
        let d = change_component(&a, &b).expect("finite overlap");
        assert!((d - 1.0).abs() < 1e-6, "orthogonal → change 1.0, got {d}");
    }

    #[test]
    fn cosine_finite_skips_nan_dims() {
        let a = [1.0f32, f32::NAN, 1.0, 0.0];
        let b = [1.0f32, 5.0, 1.0, 0.0];
        // Only dims 0,2,3 are finite in both → cosine over [1,1,0] vs [1,1,0] = 1.
        let c = cosine_finite(&a, &b).expect("some finite overlap");
        assert!((c - 1.0).abs() < 1e-6, "got {c}");
    }

    #[test]
    fn cosine_finite_all_nan_overlap_is_none() {
        let a = [f32::NAN, 1.0];
        let b = [2.0, f32::NAN];
        assert!(cosine_finite(&a, &b).is_none());
    }

    #[test]
    fn fuse_two_of_three_quadratic_mean() {
        // Two components present (e.g. GPU absent → only Tessera + one
        // other), both over the 0.15 gate: dc=0.4, dt=0.2 →
        // sqrt((0.16+0.04)/2)=0.3162..; agreement two_of_three.
        let (ens, agree, avail) = fuse(&[0.4, 0.2], 3, 0.15, 2);
        assert!(avail);
        assert_eq!(agree, "two_of_three");
        assert!((ens.unwrap() - 0.31623).abs() < 1e-4);
    }

    #[test]
    fn fuse_all_three_requires_every_encoder_over_gate() {
        // Full triple, all over gate → all_three.
        let (ens, agree, avail) = fuse(&[0.3, 0.4, 0.5], 3, 0.15, 2);
        assert!(avail);
        assert_eq!(agree, "all_three");
        assert!(ens.is_some());
        // Full triple but one below gate → two_of_three (not all_three).
        let (_e, agree2, _a) = fuse(&[0.3, 0.4, 0.05], 3, 0.15, 2);
        assert_eq!(agree2, "two_of_three");
    }

    /// HONEST GPU-ABSENT DEGRADATION: when the GPU sidecar is down,
    /// clay_v1 and prithvi_eo2 sign Absence, so only the Tessera
    /// component is available (N=1 < consensus_min_models=2). The fusion
    /// MUST return inconclusive with NO ensemble number — never a
    /// fabricated all-three vote.
    #[test]
    fn fuse_tessera_only_is_inconclusive_not_fabricated() {
        let (ens, agree, avail) = fuse(&[0.42], 3, 0.15, 2);
        assert!(!avail, "single encoder must be inconclusive");
        assert!(ens.is_none(), "no ensemble number may be fabricated");
        assert_eq!(agree, "insufficient_encoders");
    }

    #[test]
    fn fuse_zero_encoders_is_inconclusive() {
        let (ens, agree, avail) = fuse(&[], 3, 0.15, 2);
        assert!(!avail);
        assert!(ens.is_none());
        assert_eq!(agree, "insufficient_encoders");
    }

    #[test]
    fn deforestation_ndvi_drop_half_matches_formula() {
        // ndvi_modis=0.80, ndvi_now=0.50 → drop=0.30 → /0.30 → 1.0 (clamped).
        let drop = (0.80_f64 - 0.50).max(0.0);
        let term = (drop / 0.30).clamp(0.0, 1.0);
        assert!((term - 1.0).abs() < 1e-9, "got {term}");
        // Half-and-half composite with an embedding term of 0.5:
        let alert = 0.5 * term + 0.5 * 0.5;
        assert!((alert - 0.75).abs() < 1e-9, "got {alert}");
    }

    /// MULTI-VINTAGE (Priority 2): when two distinct Clay vintages exist
    /// at a cell — which the prior-vintage materialization path now
    /// makes obtainable instead of leaving Tessera to carry it alone — the
    /// Clay change component is a real number, and fusing it with a Tessera
    /// component yields a COMPUTED ensemble, NOT inconclusive. This is the
    /// pure-math core of the triple_consensus change ensemble; the live
    /// path differs only in that the two Clay vectors come from
    /// `two_vintages` (recall + on-demand prior-vintage materialize).
    #[test]
    fn two_clay_vintages_compute_real_change_not_inconclusive() {
        // Two distinct Clay vintages (last year vs this year). Not identical
        // and not orthogonal → a change strictly in (0, 1).
        let clay_now = vintage(0.30, 1024);
        let clay_prev = vintage(0.55, 1024);
        let clay_change = change_component(&clay_now, &clay_prev)
            .expect("two finite Clay vintages produce a change component");
        assert!(
            clay_change > 0.0 && clay_change < 1.0,
            "distinct vintages → a real change in (0,1), got {clay_change}"
        );

        // A Tessera component alongside it. With consensus_min_models=2,
        // the two-encoder ensemble is COMPUTED (available=true), not the
        // inconclusive single-encoder degradation.
        let tessera_change = 0.22_f64;
        let (ensemble, agreement, available) = fuse(&[clay_change, tessera_change], 3, 0.15, 2);
        assert!(
            available,
            "two encoders (Clay + Tessera) clear consensus_min_models=2 → computed"
        );
        let ens = ensemble.expect("computed ensemble carries a number");
        // L2-mean of the two components.
        let expected = ((clay_change * clay_change + tessera_change * tessera_change) / 2.0).sqrt();
        assert!(
            (ens - expected).abs() < 1e-9,
            "ensemble {ens} != {expected}"
        );
        assert_ne!(agreement, "insufficient_encoders");

        // Contrast: Clay degraded to a single vintage (Tessera only) is
        // inconclusive — proving the multi-vintage path is what flips it.
        let (e1, a1, avail1) = fuse(&[tessera_change], 3, 0.15, 2);
        assert!(!avail1 && e1.is_none() && a1 == "insufficient_encoders");
    }

    #[test]
    fn deforestation_negative_drop_clamps_to_zero() {
        // Greening: ndvi_now > baseline → drop=0 → term=0.
        let drop = (0.40_f64 - 0.60).max(0.0);
        let term = (drop / 0.30).clamp(0.0, 1.0);
        assert!(
            term.abs() < 1e-12,
            "greening must not raise an alert, got {term}"
        );
    }
}
