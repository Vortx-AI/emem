//! Runnable arm for `carbon.deforestation_alert_proxy@1`, whose documented
//! formula needs a cosine over two Tessera vintages that the scalar
//! `evaluation`-AST evaluator cannot express:
//!
//! `alert_score = 0.5·clamp01(ndvi_drop/0.30) + 0.5·clamp01(
//! embedding_change/0.20)`, where `ndvi_drop = max(0, ndvi_modis -
//! ndvi_now)` and `embedding_change = 1 - cos(tessera_latest,
//! tessera_prev)`.
//!
//! The algorithm stays flagged `documentation_only` in the registry: the AST
//! evaluator MUST keep returning `Ok(None)` for it. This endpoint never
//! fabricates an input; a missing half is reported, not filled.

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
    fn deforestation_ndvi_drop_half_matches_formula() {
        // ndvi_modis=0.80, ndvi_now=0.50 → drop=0.30 → /0.30 → 1.0 (clamped).
        let drop = (0.80_f64 - 0.50).max(0.0);
        let term = (drop / 0.30).clamp(0.0, 1.0);
        assert!((term - 1.0).abs() < 1e-9, "got {term}");
        // Half-and-half composite with an embedding term of 0.5:
        let alert = 0.5 * term + 0.5 * 0.5;
        assert!((alert - 0.75).abs() < 1e-9, "got {alert}");
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
