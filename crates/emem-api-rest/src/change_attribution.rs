//! `change_attribution@1` — the attribution LEDGER, not the split.
//!
//! When the readout at a pinned reference moves between two visits, the
//! move decomposes as `Δz = Δ_env + Δ_sensor + Δ_geo + Δ_encoder + ε`
//! (whitepaper §1.1, §10.3). This endpoint is the first runnable surface
//! of that decomposition, and it is deliberately a **ledger**: per-term
//! evidence a third party can check, with NO numeric split of a delta
//! among the terms.
//!
//! What each term gets in v1:
//!
//!   * `observed`   — the raw thing that moved: the Tessera year-over-year
//!     embedding change (`d = clamp(1 - cos(latest, prev), 0, 1)`), same
//!     construction as `triple_consensus`, plus the label-free index
//!     deltas below. Facts, not attribution.
//!   * `env`        — evidence only: the two most-recent distinct-tslot
//!     values of the label-free indices (NDVI, NBR, NDWI) with their raw
//!     deltas and both fact cids. These are the estimators a future split
//!     would read; today they are reported, not weighed.
//!   * `sensor`     — recorded: the `sources[]` scheme and id of the
//!     now/prev fact per index, and whether they match. A different scene
//!     or scheme between visits is on the record; its contribution to the
//!     delta is not computed.
//!   * `geo`        — not estimated, and said plainly: no registration
//!     residual surface exists in this build.
//!   * `encoder`    — pinned by construction and checked: the embedding
//!     facts carry their `derivation.fn_key`, and the Tessera multi-year
//!     stack is a single signed fact, so both vintages passed through one
//!     recipe. The response states what pins the term.
//!   * `noise`      — evidence only: the S2 scene-classification class
//!     (`s2.scl`) at the two visits, so a cloud-class flip between
//!     vintages is visible.
//!
//! The numeric split needs a calibrated cross-encoder, cross-sensor
//! stability model that this responder does not have (the same honesty as
//! `triple_consensus`'s `gate_calibration`). Until it exists, `split` is
//! null and `ATTRIBUTION_NOTE` says why. Fabricating per-term magnitudes
//! from uncalibrated evidence would be exactly the failure mode the
//! decomposition exists to prevent.
//!
//! Persistence: the ledger is STORED as a `DerivativeFact` under the
//! responder's own key (band `change_attribution.ledger`, op
//! `attribution_ledger`, parents = every fact the ledger read), on the
//! same attest-and-put path `/v1/derive` uses. The stored fact's value
//! is the ledger itself in canonical CBOR, so its `fact_cid`
//! content-addresses the attribution and the returned
//! `emem:fact:<cell>:<cid>` token dereferences to the byte-identical
//! ledger later, like any other fact. A derivative is deliberately
//! outside the canonical `(cell, band, tslot)` index, so it is reached
//! by citation, not by recall; the receipt binds the input cids plus
//! the ledger's own cid. If the store write fails the response still
//! carries the computed ledger and says `persistence` failed honestly
//! rather than turning good evidence into a 500.
//!
//! The tslot window on the stored fact comes from the env-evidence
//! pairs only (all S2-derived, one tempo family); tslots from different
//! tempos are indices on different clocks and are never min/maxed
//! together. When no env pair exists the window collapses to the
//! Tessera fact's tslot on both ends.

use std::time::Instant;

use axum::extract::State;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value as JsonValue};

use emem_fact::{Derivation, DerivativeFact, Fact, FactCid};
use emem_primitives::cbor_ops::{as_f64, as_vec_f32, cosine_finite};
use emem_primitives::RecallReq;

use crate::triple_consensus::covered_vintages;
use crate::{recall_with_auto_materialize, ApiError, AppState, EmemJson};

/// Width of one GeoTessera vintage inside `geotessera.multi_year`.
const TESSERA_VINTAGE_DIM: usize = 128;

/// The label-free indices reported as environment evidence, in report
/// order. Each is a deterministic S2-derived recipe already in the
/// registry; the ledger reads them, it does not re-derive them.
const ENV_EVIDENCE_BANDS: [&str; 3] = ["indices.ndvi", "indices.nbr", "indices.ndwi"];

/// Why `split` is null. Rides every response, in-band, the same way
/// `triple_consensus` carries `gate_calibration`.
pub(crate) const ATTRIBUTION_NOTE: &str = "ledger_not_split: this response \
attributes by EVIDENCE, not by magnitude. Splitting a specific delta into \
environment, sensor, geometry, encoder, and noise terms needs a calibrated \
cross-encoder, cross-sensor stability model this responder does not have; \
inventing per-term magnitudes from uncalibrated evidence would fabricate \
exactly the confusion the decomposition exists to prevent. Read the per-term \
evidence and its fact cids; the numeric split is roadmap work.";

#[derive(Debug, Deserialize)]
pub struct ChangeAttributionReq {
    /// cell64 or place name.
    pub cell: String,
}

/// One band's two most-recent distinct-tslot scalar facts, with the
/// provenance the sensor ledger needs. `now` is the more recent row.
struct ScalarPair {
    band: &'static str,
    now: ScalarRow,
    prev: ScalarRow,
}

struct ScalarRow {
    tslot: u64,
    value: f64,
    cid: String,
    /// `scheme:id` per source entry, the sensor ledger's unit of record.
    sources: Vec<String>,
}

/// Why a band contributes no evidence pair. Mirrors the closed-set
/// discipline of `triple_consensus::AbsenceReason`; append only.
enum EvidenceAbsence {
    SingleVintage(usize),
    NoScalarFacts,
    RecallFailed(String),
}

impl EvidenceAbsence {
    fn code(&self) -> &'static str {
        match self {
            EvidenceAbsence::SingleVintage(_) => "single_vintage",
            EvidenceAbsence::NoScalarFacts => "outside_coverage",
            EvidenceAbsence::RecallFailed(_) => "recall_failed",
        }
    }
    fn reason(&self, band: &str) -> String {
        match self {
            EvidenceAbsence::SingleVintage(n) => format!(
                "only {n} distinct tslot(s) of `{band}` at this cell; a change ledger needs two"
            ),
            EvidenceAbsence::NoScalarFacts => {
                format!("no scalar fact for `{band}` at this cell")
            }
            EvidenceAbsence::RecallFailed(e) => format!("recall failed: {e}"),
        }
    }
}

/// Collect `(tslot, value, cid, sources)` rows for every Primary scalar
/// fact of `band` in a recall response, most-recent first, deduped by
/// tslot. Pure over the response so it is unit-testable.
fn scalar_rows(resp: &emem_primitives::RecallResp, band: &str) -> Vec<ScalarRow> {
    let mut rows: Vec<ScalarRow> = Vec::new();
    for (idx, f) in resp.facts.iter().enumerate() {
        if let Fact::Primary(p) = f {
            if p.band != band {
                continue;
            }
            if let Some(v) = as_f64(&p.value) {
                rows.push(ScalarRow {
                    tslot: p.tslot,
                    value: v,
                    cid: resp
                        .receipt
                        .fact_cids
                        .get(idx)
                        .map(|c| c.as_str().to_string())
                        .unwrap_or_default(),
                    sources: p
                        .sources
                        .iter()
                        .map(|s| format!("{}:{}", s.scheme, s.id))
                        .collect(),
                });
            }
        }
    }
    rows.sort_by_key(|r| std::cmp::Reverse(r.tslot));
    rows.dedup_by_key(|r| r.tslot);
    rows
}

/// Two most-recent distinct-tslot scalar facts for `band`, recalling
/// (and auto-materializing the latest) first. v1 does NOT backfill a
/// prior value when only one exists: an index's history either exists in
/// the store or the band is honest evidence-absent. Backfilling history
/// here would silently spend the same budget as `emem_backfill` per band.
async fn scalar_pair(
    cell: &str,
    band: &'static str,
    s: &AppState,
) -> Result<ScalarPair, EvidenceAbsence> {
    let req = RecallReq {
        cell: cell.to_string(),
        bands: Some(vec![band.to_string()]),
        tslot: None,
        ..Default::default()
    };
    let (resp, _notes) = recall_with_auto_materialize(&req, s)
        .await
        .map_err(|e| EvidenceAbsence::RecallFailed(e.1.message.clone()))?;
    let mut rows = scalar_rows(&resp, band);
    match rows.len() {
        0 => Err(EvidenceAbsence::NoScalarFacts),
        1 => Err(EvidenceAbsence::SingleVintage(1)),
        _ => {
            let prev = rows.remove(1);
            let now = rows.remove(0);
            Ok(ScalarPair { band, now, prev })
        }
    }
}

/// `d = clamp(1 - cos, 0, 1)` over finite dims, `None` on no overlap.
fn embedding_change(now: &[f32], prev: &[f32]) -> Option<f64> {
    let c = cosine_finite(now, prev)? as f64;
    Some((1.0 - c).clamp(0.0, 1.0))
}

/// JSON to CBOR for the stored ledger value. serde_json's default map is
/// a BTreeMap, so object keys arrive here sorted, which is exactly the
/// pre-sorted-keys discipline canonical CBOR requires for freeform maps
/// (whitepaper §5). Floats pass through; canonicalization of NaN and
/// negative zero happens in `to_canonical_cbor` at hash time.
pub(crate) fn json_to_cbor(j: &JsonValue) -> ciborium::Value {
    match j {
        JsonValue::Null => ciborium::Value::Null,
        JsonValue::Bool(b) => ciborium::Value::Bool(*b),
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                ciborium::Value::Integer(i.into())
            } else {
                ciborium::Value::Float(n.as_f64().unwrap_or(f64::NAN))
            }
        }
        JsonValue::String(s) => ciborium::Value::Text(s.clone()),
        JsonValue::Array(a) => ciborium::Value::Array(a.iter().map(json_to_cbor).collect()),
        JsonValue::Object(m) => ciborium::Value::Map(
            m.iter()
                .map(|(k, v)| (ciborium::Value::Text(k.clone()), json_to_cbor(v)))
                .collect(),
        ),
    }
}

pub async fn change_attribution(
    req: ChangeAttributionReq,
    s: &AppState,
) -> Result<JsonValue, ApiError> {
    let started = Instant::now();
    let (cell, resolved) = crate::resolve_cell_field(&req.cell).await?;

    let mut all_cids: Vec<String> = Vec::new();
    let mut tessera_tslot: Option<u64> = None;
    // Window bounds from the env-evidence pairs only: one tempo family,
    // so the min/max is meaningful. See the module header.
    let mut env_tslots: Vec<u64> = Vec::new();

    // ── observed: Tessera year-over-year embedding change, plus the
    //    encoder-term pinning evidence from the same fact. ─────────────
    let mut observed_embedding = json!(null);
    let mut encoder_term = json!({
        "changed": null,
        "evidence": "no embedding fact was readable at this cell",
        "pinned_by": null,
    });
    {
        let mreq = RecallReq {
            cell: cell.clone(),
            bands: Some(vec!["geotessera.multi_year".to_string()]),
            tslot: None,
            ..Default::default()
        };
        if let Ok((resp, _notes)) = recall_with_auto_materialize(&mreq, s).await {
            let primary = resp.facts.iter().enumerate().find_map(|(i, f)| match f {
                Fact::Primary(p) if p.band == "geotessera.multi_year" => Some((i, p)),
                _ => None,
            });
            if let Some((i, p)) = primary {
                tessera_tslot = Some(p.tslot);
                let cid = resp
                    .receipt
                    .fact_cids
                    .get(i)
                    .map(|c| c.as_str().to_string())
                    .unwrap_or_default();
                if !cid.is_empty() {
                    all_cids.push(cid.clone());
                }
                // Both vintages live inside ONE signed fact, so both
                // passed through one recipe: the encoder term is pinned
                // for this comparison by construction, and the fact's
                // fn_key names the recipe a verifier replays.
                encoder_term = json!({
                    "changed": false,
                    "evidence": "both vintages are slices of one signed multi-year fact, so one recipe produced both; an encoder change cannot hide inside this comparison",
                    "pinned_by": { "fn_key": p.derivation.fn_key, "fact_cid": cid },
                });
                if let Some(stack) = as_vec_f32(&p.value) {
                    let vintages = covered_vintages(&stack, TESSERA_VINTAGE_DIM);
                    if vintages.len() >= 2 {
                        let d = embedding_change(
                            &vintages[vintages.len() - 1],
                            &vintages[vintages.len() - 2],
                        );
                        observed_embedding = json!({
                            "encoder": "geotessera.multi_year",
                            "change": d,
                            "covered_vintages": vintages.len(),
                            "fact_cid": all_cids.last(),
                        });
                    } else {
                        observed_embedding = json!({
                            "encoder": "geotessera.multi_year",
                            "change": null,
                            "covered_vintages": vintages.len(),
                            "reason_code": "single_vintage",
                        });
                    }
                }
            }
        }
    }

    // ── env evidence: the label-free index pairs, concurrently. ───────
    let (ndvi, nbr, ndwi) = tokio::join!(
        scalar_pair(&cell, ENV_EVIDENCE_BANDS[0], s),
        scalar_pair(&cell, ENV_EVIDENCE_BANDS[1], s),
        scalar_pair(&cell, ENV_EVIDENCE_BANDS[2], s),
    );
    let mut env_evidence: Vec<JsonValue> = Vec::new();
    let mut evidence_absent: Vec<JsonValue> = Vec::new();
    let mut sensor_records: Vec<JsonValue> = Vec::new();
    for (band, outcome) in ENV_EVIDENCE_BANDS.into_iter().zip([ndvi, nbr, ndwi]) {
        match outcome {
            Ok(pair) => {
                for c in [&pair.now.cid, &pair.prev.cid] {
                    if !c.is_empty() {
                        all_cids.push(c.clone());
                    }
                }
                env_tslots.push(pair.now.tslot);
                env_tslots.push(pair.prev.tslot);
                env_evidence.push(json!({
                    "band": pair.band,
                    "value_now": pair.now.value,
                    "value_prev": pair.prev.value,
                    "delta": pair.now.value - pair.prev.value,
                    "tslot_now": pair.now.tslot,
                    "tslot_prev": pair.prev.tslot,
                    "fact_cids": [pair.now.cid, pair.prev.cid],
                }));
                // The sensor term's record for this band: what each visit
                // was observed through, and whether that changed.
                let same = pair.now.sources == pair.prev.sources;
                sensor_records.push(json!({
                    "band": pair.band,
                    "sources_now": pair.now.sources,
                    "sources_prev": pair.prev.sources,
                    "same_observation_path": same,
                }));
            }
            Err(a) => evidence_absent.push(json!({
                "band": band,
                "reason": a.reason(band),
                "reason_code": a.code(),
            })),
        }
    }

    // ── noise evidence: the S2 scene-classification class per visit. ──
    let noise_term = match scalar_pair(&cell, "s2.scl", s).await {
        Ok(pair) => {
            for c in [&pair.now.cid, &pair.prev.cid] {
                if !c.is_empty() {
                    all_cids.push(c.clone());
                }
            }
            json!({
                "band": "s2.scl",
                "class_now": pair.now.value,
                "class_prev": pair.prev.value,
                "class_changed": (pair.now.value - pair.prev.value).abs() > f64::EPSILON,
                "fact_cids": [pair.now.cid, pair.prev.cid],
                "note": "S2 scene-classification class per visit; a class flip (e.g. clear to cloud/shadow) is noise evidence for every optical delta above",
            })
        }
        Err(a) => json!({
            "band": "s2.scl",
            "reason": a.reason("s2.scl"),
            "reason_code": a.code(),
        }),
    };

    let n_env = env_evidence.len();
    let resolved_env = crate::resolved_envelope(vec![("cell".into(), resolved)]);

    // The ledger body is assembled once and used twice: stored verbatim
    // as the derivative fact's value, and inlined in the response. The
    // stored bytes and the response body are therefore the same claim.
    let ledger_body = json!({
        "schema": "emem.change_attribution.v1",
        "decomposition": "Δz = Δ_env + Δ_sensor + Δ_geo + Δ_encoder + ε",
        "observed": {
            "embedding": observed_embedding,
        },
        "terms": {
            "env": {
                "estimated": false,
                "evidence": env_evidence,
                "evidence_absent": evidence_absent,
                "note": "label-free index pairs with raw deltas; the estimators a future split would read, reported rather than weighed",
            },
            "sensor": {
                "estimated": false,
                "records": sensor_records,
                "note": "what each visit was observed through, per band; a changed observation path is on the record, its contribution to the delta is not computed",
            },
            "geo": {
                "estimated": false,
                "note": "no registration-residual surface exists in this build; the term is declared not estimated rather than silently folded into env",
            },
            "encoder": encoder_term,
            "noise": noise_term,
        },
        "split": JsonValue::Null,
        "attribution_note": ATTRIBUTION_NOTE,
    });

    // ── persist the ledger as a derivative fact, /v1/derive's path. ───
    let window = ledger_window(&env_tslots, tessera_tslot);
    let signed_at = emem_storage::server::iso8601_now();
    let ledger_fact = DerivativeFact {
        cell: cell.clone(),
        band: "change_attribution.ledger".to_string(),
        tslot_window: window,
        op: "attribution_ledger".to_string(),
        parents: all_cids.iter().cloned().map(FactCid::new).collect(),
        value: json_to_cbor(&ledger_body),
        confidence: 1.0,
        derivation: Derivation {
            fn_key: "change_attribution@1".to_string(),
            args: None,
        },
        schema_cid: s.manifests.schema_cid.clone(),
        signer: s.identity.pubkey,
        signed_at: signed_at.clone(),
    };
    let (persistence, ledger_fact_json, ledger_cid) =
        match emem_fact::Attestation::build_and_sign_v1(
            vec![Fact::Derivative(ledger_fact)],
            vec![],
            s.manifests.registry_cid.clone(),
            s.manifests.schema_cid.clone(),
            &s.identity.signing,
            s.identity.epoch,
            signed_at,
            None,
        ) {
            Ok(att) => match s.storage.put_attestation(&att).await {
                Ok(cids) if !cids.is_empty() => {
                    let cid = cids[0].as_str().to_string();
                    let token = format!("emem:fact:{cell}:{cid}");
                    (
                        json!("stored_derivative_fact"),
                        json!({ "fact_cid": cid, "token": token, "band": "change_attribution.ledger", "note": "reached by citation, not by recall: a derivative is outside the canonical (cell, band, tslot) index by design" }),
                        Some(cid),
                    )
                }
                Ok(_) => (
                    json!("persist_failed: store returned no cid"),
                    json!(null),
                    None,
                ),
                Err(e) => (json!(format!("persist_failed: {e}")), json!(null), None),
            },
            Err(e) => (
                json!(format!("persist_failed: attestation build: {e}")),
                json!(null),
                None,
            ),
        };

    // The receipt binds the inputs read plus, when stored, the ledger's
    // own cid, so one signature covers the evidence and the attribution.
    let mut receipt_cids: Vec<FactCid> = all_cids.iter().cloned().map(FactCid::new).collect();
    if let Some(c) = &ledger_cid {
        receipt_cids.push(FactCid::new(c.clone()));
    }
    let receipt = s.sign_receipt(
        "emem.change_attribution",
        vec![cell.clone()],
        receipt_cids,
        false,
        started,
        None,
    );

    let mut out = ledger_body;
    out["algorithm_key"] = json!("change_attribution@1");
    out["cell"] = json!(cell);
    out["resolved_from"] = json!(resolved_env);
    out["n_env_evidence_bands"] = json!(n_env);
    out["persistence"] = persistence;
    out["ledger_fact"] = ledger_fact_json;
    out["input_fact_cids"] = json!(all_cids);
    out["receipt"] = serde_json::to_value(&receipt).unwrap_or(JsonValue::Null);
    Ok(out)
}

/// The stored fact's tslot window: min/max over the env-evidence pair
/// tslots (one tempo family). With no env pair, both ends collapse to
/// the Tessera fact's tslot; with neither, to zero, which is honest
/// (nothing time-bound was read).
fn ledger_window(env_tslots: &[u64], tessera_tslot: Option<u64>) -> [u64; 2] {
    if let (Some(min), Some(max)) = (env_tslots.iter().min(), env_tslots.iter().max()) {
        [*min, *max]
    } else if let Some(t) = tessera_tslot {
        [t, t]
    } else {
        [0, 0]
    }
}

pub async fn post_change_attribution(
    State(s): State<AppState>,
    EmemJson(req): EmemJson<ChangeAttributionReq>,
) -> Result<Json<JsonValue>, ApiError> {
    Ok(Json(change_attribution(req, &s).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedding_change_clamps_and_needs_overlap() {
        let a = vec![1.0f32, 0.0, 0.0];
        let b = vec![1.0f32, 0.0, 0.0];
        assert_eq!(embedding_change(&a, &b), Some(0.0));
        let c = vec![-1.0f32, 0.0, 0.0];
        // cos = -1 → 1 - cos = 2, clamped to 1.
        assert_eq!(embedding_change(&a, &c), Some(1.0));
        let nan = vec![f32::NAN, f32::NAN, f32::NAN];
        assert_eq!(embedding_change(&a, &nan), None);
    }

    #[test]
    fn ledger_window_uses_env_pairs_then_tessera_then_zero() {
        assert_eq!(
            ledger_window(&[20500, 20470, 20510, 20471], None),
            [20470, 20510]
        );
        assert_eq!(ledger_window(&[], Some(55)), [55, 55]);
        assert_eq!(ledger_window(&[], None), [0, 0]);
    }

    #[test]
    fn json_to_cbor_keeps_sorted_keys_and_types() {
        let j = serde_json::json!({"b": 1, "a": [1.5, null, "x"], "c": {"z": true, "y": 2}});
        let c = json_to_cbor(&j);
        let ciborium::Value::Map(m) = c else {
            panic!("not a map")
        };
        let keys: Vec<&str> = m
            .iter()
            .map(|(k, _)| match k {
                ciborium::Value::Text(t) => t.as_str(),
                _ => panic!("non-text key"),
            })
            .collect();
        // serde_json's BTreeMap ordering is what canonical CBOR requires
        // of freeform maps; if this ever fails, preserve_order got
        // enabled somewhere and the stored ledger cids would drift.
        assert_eq!(keys, vec!["a", "b", "c"]);
    }

    #[test]
    fn absence_codes_are_the_documented_closed_set() {
        assert_eq!(EvidenceAbsence::SingleVintage(1).code(), "single_vintage");
        assert_eq!(EvidenceAbsence::NoScalarFacts.code(), "outside_coverage");
        assert_eq!(
            EvidenceAbsence::RecallFailed(String::new()).code(),
            "recall_failed"
        );
    }
}
