//! Real dispatcher arms for three documented EO algorithms whose
//! published formulas the scalar `evaluation`-AST evaluator cannot
//! express, because each needs either a **multi-year time series** or
//! **two distinct scenes** rather than a single `f64 → f64` arithmetic
//! tree over one snapshot:
//!
//!   1. [`spi`] — `spi_meteorological_drought@1`.
//!      Standardized Precipitation Index (McKee, Doesken & Kleist 1993).
//!      Accumulate precipitation over the SPI window, fit a two-parameter
//!      gamma to the cell's historical same-window accumulations (Thom
//!      1958 MLE per Guttman 1999), apply the mixed CDF
//!      `H(x) = q + (1-q)·G(x|α,β)` (q = P(zero accumulation)), then map
//!      to the standard normal via the inverse-normal so the output is a
//!      z-score (negative = drought). Needs a multi-year SERIES — the AST
//!      sees a single sample and cannot fit the distribution.
//!
//!   2. [`burn_severity`] — `burn_severity_from_dnbr@1`.
//!      USGS / Key & Benson (FIREMON) ΔNBR. `dnbr = NBR_pre − NBR_post`,
//!      classified into the standard severity ladder. Needs TWO scenes
//!      (pre/post fire) — the AST has no notion of pairing two tslots.
//!
//!   3. [`rice_ch4`] — `rice_methane_awd_modeled@1`.
//!      IPCC 2019 Refinement Vol 4 Ch 5 Eq 5.1 seasonal rice CH4:
//!      `CH4 = EFc · SFw · SFp · SFo · days · T_mod`. The water regime
//!      (SFw) is classified from an NDWI series (continuous-flood vs AWD
//!      vs mid-season drainage). Needs the NDWI SERIES + a declared
//!      cultivation-period length — not expressible as a scalar AST.
//!
//! All three stay flagged `documentation_only` in the registry: the AST
//! evaluator MUST keep returning `Ok(None)` for them. These endpoints are
//! the honest runnable surface. They NEVER fabricate a number when the
//! inputs are not materializable — too-short precipitation history, a
//! single fire scene, or a too-sparse NDWI series each yield a signed
//! `inconclusive` / `Absence`-style response carrying the observed count,
//! not a bogus value.

use std::time::Instant;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value as JsonValue};

use emem_core::ErrorCode;
use emem_fact::FactCid;
use emem_primitives::cbor_ops::as_f64;
use emem_primitives::trajectory::{trajectory, TrajectoryReq};

use crate::{ApiError, AppState, ErrorBody};

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

// ════════════════════════════════════════════════════════════════════
//  Numerical primitives (pure, unit-tested)
// ════════════════════════════════════════════════════════════════════

/// Natural log of the gamma function — Lanczos approximation (g = 7,
/// n = 9). Accurate to ~15 significant figures for x > 0, which is all
/// the gamma-fit + incomplete-gamma code below needs.
fn ln_gamma(x: f64) -> f64 {
    // Lanczos coefficients (g=7, n=9).
    const G: f64 = 7.0;
    const C: [f64; 9] = [
        0.999_999_999_999_809_9,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_8,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];
    if x < 0.5 {
        // Reflection formula.
        std::f64::consts::PI.ln() - (std::f64::consts::PI * x).sin().ln() - ln_gamma(1.0 - x)
    } else {
        let x = x - 1.0;
        let mut a = C[0];
        let t = x + G + 0.5;
        for (i, &c) in C.iter().enumerate().skip(1) {
            a += c / (x + i as f64);
        }
        0.5 * (2.0 * std::f64::consts::PI).ln() + (x + 0.5) * t.ln() - t + a.ln()
    }
}

/// Regularized lower incomplete gamma P(a, x) = γ(a,x)/Γ(a).
/// Series expansion for x < a+1, continued fraction otherwise
/// (Numerical Recipes §6.2). Returns a value in [0, 1].
fn gamma_p(a: f64, x: f64) -> f64 {
    if x <= 0.0 || a <= 0.0 {
        return 0.0;
    }
    if x < a + 1.0 {
        // Series representation.
        let mut ap = a;
        let mut sum = 1.0 / a;
        let mut del = sum;
        for _ in 0..200 {
            ap += 1.0;
            del *= x / ap;
            sum += del;
            if del.abs() < sum.abs() * 1e-15 {
                break;
            }
        }
        (sum.ln() - x + a * x.ln() - ln_gamma(a)).exp()
    } else {
        // Continued fraction for the complement Q, then P = 1 - Q.
        let tiny = 1e-300;
        let mut b = x + 1.0 - a;
        let mut c = 1.0 / tiny;
        let mut d = 1.0 / b;
        let mut h = d;
        for i in 1..200 {
            let an = -(i as f64) * (i as f64 - a);
            b += 2.0;
            d = an * d + b;
            if d.abs() < tiny {
                d = tiny;
            }
            c = b + an / c;
            if c.abs() < tiny {
                c = tiny;
            }
            d = 1.0 / d;
            let del = d * c;
            h *= del;
            if (del - 1.0).abs() < 1e-15 {
                break;
            }
        }
        let q = (-x + a * x.ln() - ln_gamma(a)).exp() * h;
        1.0 - q
    }
}

/// Inverse of the standard-normal CDF (probit). Acklam's rational
/// approximation, relative error < 1.15e-9 across the open interval.
fn inv_normal_cdf(p: f64) -> f64 {
    if p <= 0.0 {
        return f64::NEG_INFINITY;
    }
    if p >= 1.0 {
        return f64::INFINITY;
    }
    const A: [f64; 6] = [
        -3.969_683_028_665_376e1,
        2.209_460_984_245_205e2,
        -2.759_285_104_469_687e2,
        1.383_577_518_672_69e2,
        -3.066_479_806_614_716e1,
        2.506_628_277_459_239,
    ];
    const B: [f64; 5] = [
        -5.447_609_879_822_406e1,
        1.615_858_368_580_409e2,
        -1.556_989_798_598_866e2,
        6.680_131_188_771_972e1,
        -1.328_068_155_288_572e1,
    ];
    const C: [f64; 6] = [
        -7.784_894_002_430_293e-3,
        -3.223_964_580_411_365e-1,
        -2.400_758_277_161_838,
        -2.549_732_539_343_734,
        4.374_664_141_464_968,
        2.938_163_982_698_783,
    ];
    const D: [f64; 4] = [
        7.784_695_709_041_462e-3,
        3.224_671_290_700_398e-1,
        2.445_134_137_142_996,
        3.754_408_661_907_416,
    ];
    let p_low = 0.024_25;
    let p_high = 1.0 - p_low;
    if p < p_low {
        let q = (-2.0 * p.ln()).sqrt();
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    } else if p <= p_high {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
            / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    } else {
        let q = (-2.0 * (1.0 - p).ln()).sqrt();
        -(((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    }
}

/// Outcome of fitting the SPI gamma + mapping one accumulation to a
/// z-score. `n_positive` is the number of strictly-positive samples used
/// in the MLE (zeros are folded into the mixed CDF, not the fit).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SpiResult {
    pub spi: f64,
    pub alpha: f64,
    pub beta: f64,
    pub q_zero: f64,
}

/// Fit a two-parameter gamma to the strictly-positive entries of
/// `history` via the Thom (1958) maximum-likelihood estimator (the
/// estimator Guttman 1999 / WMO-1090 prescribe for SPI), then map the
/// target accumulation `x` through the mixed CDF and the inverse normal.
///
/// Returns `None` when there are too few positive samples to fit
/// (< `min_positive`) — the caller MUST surface an honest inconclusive,
/// never a fabricated SPI. `min_positive` is a guard against a
/// degenerate fit, separate from the WMO 30-yr-record recommendation
/// which the endpoint enforces on the total sample count.
pub(crate) fn fit_spi(history: &[f64], x: f64, min_positive: usize) -> Option<SpiResult> {
    let n_total = history.len();
    if n_total == 0 {
        return None;
    }
    let positives: Vec<f64> = history
        .iter()
        .copied()
        .filter(|v| v.is_finite() && *v > 0.0)
        .collect();
    let n_pos = positives.len();
    if n_pos < min_positive {
        return None;
    }
    let n_zero = history
        .iter()
        .filter(|v| v.is_finite() && **v <= 0.0)
        .count();
    let n_finite = n_pos + n_zero;
    if n_finite == 0 {
        return None;
    }
    let q_zero = n_zero as f64 / n_finite as f64;

    let mean = positives.iter().sum::<f64>() / n_pos as f64;
    if mean <= 0.0 {
        return None;
    }
    let ln_mean = mean.ln();
    let mean_ln = positives.iter().map(|v| v.ln()).sum::<f64>() / n_pos as f64;
    let a_thom = ln_mean - mean_ln; // = ln(mean) - mean(ln x) >= 0
    if !(a_thom.is_finite()) || a_thom <= 0.0 {
        // Degenerate (all positives identical) — cannot fit a shape.
        return None;
    }
    // Thom 1958 closed-form MLE for the gamma shape parameter.
    let alpha = (1.0 + (1.0 + 4.0 * a_thom / 3.0).sqrt()) / (4.0 * a_thom);
    let beta = mean / alpha;
    if !alpha.is_finite() || !beta.is_finite() || alpha <= 0.0 || beta <= 0.0 {
        return None;
    }

    // Mixed CDF: H(x) = q + (1-q)·G(x), with G the gamma CDF = P(α, x/β).
    let g = if x <= 0.0 {
        0.0
    } else {
        gamma_p(alpha, x / beta)
    };
    let mut h = q_zero + (1.0 - q_zero) * g;
    // Clamp away from the open-interval endpoints so the probit is finite;
    // ±3.09 ≈ p=0.001/0.999, beyond the McKee categorical ladder anyway.
    h = h.clamp(0.001_350, 0.998_650);
    let spi = inv_normal_cdf(h);
    Some(SpiResult {
        spi,
        alpha,
        beta,
        q_zero,
    })
}

/// McKee, Doesken & Kleist (1993) categorical ladder for an SPI value.
pub(crate) fn spi_class(spi: f64) -> &'static str {
    if spi <= -2.0 {
        "extreme_drought"
    } else if spi <= -1.5 {
        "severe_drought"
    } else if spi <= -1.0 {
        "moderate_drought"
    } else if spi <= -0.5 {
        "mild_drought"
    } else if spi <= 0.5 {
        "near_normal"
    } else if spi <= 1.0 {
        "mildly_wet"
    } else if spi <= 1.5 {
        "moderately_wet"
    } else if spi <= 2.0 {
        "very_wet"
    } else {
        "extremely_wet"
    }
}

/// USGS / Key & Benson (2006) ΔNBR severity ladder. Thresholds match the
/// registry entry `burn_severity_from_dnbr@1` (in NBR ratio units — the
/// commonly-published ×1000 ΔNBR multiplies these by 1000). Note the
/// standard caveat: absolute thresholds are region-tuned; treat near a
/// boundary as ambiguous.
pub(crate) fn dnbr_class(dnbr: f64) -> &'static str {
    if dnbr < 0.10 {
        "unburned"
    } else if dnbr < 0.27 {
        "low"
    } else if dnbr < 0.44 {
        "moderate_low"
    } else if dnbr < 0.66 {
        "moderate_high"
    } else {
        "high"
    }
}

/// IPCC 2019 Refinement Vol 4 Ch 5 Table 5.12 water-regime scaling
/// factor SFw, keyed by the regime detected from the NDWI series.
pub(crate) fn sfw_for_regime(regime: &str) -> f64 {
    match regime {
        "continuous_flood" => 1.00,
        "mid_season_drainage" => 0.71,
        "AWD" => 0.52,
        "upland_or_dry_seeded" => 0.00,
        _ => 0.55, // regular rainfed default (Table 5.12)
    }
}

/// Classify the cultivation-period water regime from an NDWI series,
/// per the registry formula: mean(NDWI) and the count of zero-crossings
/// (aeration cycles). Returns `None` when the series is too short to
/// distinguish a regime (fewer than `min_points`), so the caller can
/// degrade honestly instead of guessing `continuous_flood`.
pub(crate) fn classify_water_regime(ndwi: &[f64], min_points: usize) -> Option<&'static str> {
    let s: Vec<f64> = ndwi.iter().copied().filter(|v| v.is_finite()).collect();
    if s.len() < min_points {
        return None;
    }
    let mean = s.iter().sum::<f64>() / s.len() as f64;
    // Count transitions of sign relative to the 0.0 flooding threshold.
    let mut crossings = 0usize;
    for w in s.windows(2) {
        let a = w[0] > 0.0;
        let b = w[1] > 0.0;
        if a != b {
            crossings += 1;
        }
    }
    Some(if mean <= 0.0 && crossings == 0 {
        "upland_or_dry_seeded"
    } else if mean > 0.0 && crossings <= 1 {
        "continuous_flood"
    } else if crossings >= 3 {
        "AWD"
    } else {
        // crossings == 2 with positive mean → a single drainage episode.
        "mid_season_drainage"
    })
}

/// IPCC 2019 Eq 5.1 seasonal CH4 per hectare, plus the Yan et al. (2005)
/// Q10 ≈ 2.5 temperature modifier (`T_mod = exp(0.0917·(T−25))`).
///
/// `CH4 = EFc · SFw · SFp · SFo · days · T_mod`. Every factor is an
/// IPCC-table-documented parameter supplied (with documented defaults) by
/// the caller — nothing is invented here.
#[allow(clippy::too_many_arguments)]
pub(crate) fn rice_ch4_kg_ha_season(
    efc_kg_ch4_ha_day: f64,
    sfw: f64,
    sfp: f64,
    sfo: f64,
    cultivation_period_days: f64,
    t_paddy_c: Option<f64>,
) -> f64 {
    let t_mod = match t_paddy_c {
        Some(t) if t.is_finite() => (0.0917 * (t - 25.0)).exp(),
        _ => 1.0,
    };
    efc_kg_ch4_ha_day * sfw * sfp * sfo * cultivation_period_days * t_mod
}

// ════════════════════════════════════════════════════════════════════
//  1. SPI endpoint
// ════════════════════════════════════════════════════════════════════

const TSLOT_DAY: u64 = 86_400;

/// Number of strictly-positive samples below which the gamma fit is
/// refused outright (degenerate). The WMO 30-yr-record recommendation is
/// enforced separately as a soft confidence flag on the total count.
const SPI_MIN_POSITIVE: usize = 10;
/// WMO-1090 recommends ≥30 yr of record; with monthly SPI-3 that is many
/// samples, but we accept fewer with a lowered-confidence flag. Below
/// this hard floor we return inconclusive — 2 points can never fit a
/// gamma.
const SPI_MIN_SAMPLES: usize = 20;

#[derive(Debug, Deserialize)]
pub struct SpiReq {
    pub cell: String,
    /// SPI window length in days the precipitation is accumulated over
    /// (SPI-3 = 90 d default; SPI-1 = 30 d; SPI-12 = 360 d).
    #[serde(default)]
    pub window_days: Option<u64>,
    /// Optional explicit history of same-window precipitation
    /// accumulations (mm). When supplied the endpoint fits directly to
    /// these — lets an agent that already ran `/v1/backfill` feed the
    /// series without a second round-trip. When omitted the endpoint
    /// reads the stored `weather.precipitation_mm` trajectory at the
    /// cell and builds the accumulations itself.
    #[serde(default)]
    pub precip_history_mm: Option<Vec<f64>>,
    /// The current-window accumulation (mm) to standardize. Required when
    /// `precip_history_mm` is supplied; otherwise taken as the most-recent
    /// window from the stored series.
    #[serde(default)]
    pub current_accumulation_mm: Option<f64>,
}

/// Pure computation core so the gamma fit + degradation are unit-testable
/// without an `AppState`. Returns the JSON body fields (minus receipt).
pub(crate) fn compute_spi_body(history: &[f64], current: f64, window_days: u64) -> JsonValue {
    let n = history.len();
    if n < SPI_MIN_SAMPLES {
        return json!({
            "verdict": "inconclusive",
            "available": false,
            "spi": JsonValue::Null,
            "n_samples": n,
            "honest_note": format!(
                "Only {n} historical window-accumulations available; SPI needs at least {SPI_MIN_SAMPLES} (WMO-1090 recommends ≥30 yr of record). No SPI reported — fitting a gamma to a handful of points would fabricate a z-score."
            ),
        });
    }
    match fit_spi(history, current, SPI_MIN_POSITIVE) {
        Some(r) => {
            let low_confidence = n < 30;
            json!({
                "verdict": "computed",
                "available": true,
                "spi": r.spi,
                "spi_class": spi_class(r.spi),
                "current_accumulation_mm": current,
                "gamma_alpha": r.alpha,
                "gamma_beta": r.beta,
                "q_zero": r.q_zero,
                "n_samples": n,
                "window_days": window_days,
                "spi_label": format!("SPI-{}", (window_days as f64 / 30.0).round() as i64),
                "confidence": if low_confidence { "reduced (record < 30 samples)" } else { "nominal" },
            })
        }
        None => json!({
            "verdict": "inconclusive",
            "available": false,
            "spi": JsonValue::Null,
            "n_samples": n,
            "honest_note": format!(
                "Gamma fit refused: fewer than {SPI_MIN_POSITIVE} strictly-positive accumulations among {n} samples, or a degenerate (constant) record. No SPI reported."
            ),
        }),
    }
}

pub async fn spi(req: SpiReq, s: &AppState) -> Result<JsonValue, ApiError> {
    let started = Instant::now();
    let (cell, resolved) = crate::resolve_cell_field(&req.cell).await?;
    let window_days = req.window_days.unwrap_or(90).max(1);

    let mut cids: Vec<String> = Vec::new();
    let mut source_note: &str;

    // Build (history, current). Prefer an explicit caller-supplied series.
    let (history, current): (Vec<f64>, Option<f64>) = if let Some(h) = req.precip_history_mm.clone()
    {
        source_note = "caller-supplied precip_history_mm";
        let cur = req.current_accumulation_mm.or_else(|| h.last().copied());
        (h, cur)
    } else {
        source_note = "stored weather.precipitation_mm trajectory";
        // Read the full stored trajectory and aggregate into
        // non-overlapping window sums. Each window-sum is one sample.
        let treq = TrajectoryReq {
            cell: cell.clone(),
            band: "weather.precipitation_mm".to_string(),
            window: [0, u64::MAX],
            as_of_tslot: None,
            as_of_signed_at: None,
            scope: None,
        };
        let tr = trajectory(&treq, s)
            .await
            .map_err(|e| bad_request(format!("trajectory read failed: {e}")))?;
        for p in &tr.series {
            if !p.fact_cid.is_empty() {
                cids.push(p.fact_cid.clone());
            }
        }
        let span = window_days.saturating_mul(TSLOT_DAY).max(1);
        // Bucket points into consecutive `span`-wide windows by tslot.
        let mut buckets: std::collections::BTreeMap<u64, f64> = Default::default();
        for p in &tr.series {
            if let Some(v) = as_f64(&p.value) {
                if v.is_finite() {
                    *buckets.entry(p.tslot / span).or_insert(0.0) += v.max(0.0);
                }
            }
        }
        let sums: Vec<f64> = buckets.values().copied().collect();
        let cur = sums.last().copied();
        (sums, cur)
    };

    let resolved_env = crate::resolved_envelope(vec![("cell".into(), resolved)]);
    let pubkey = pubkey_b32(s);
    let receipt = s.sign_receipt(
        "emem.spi",
        vec![cell.clone()],
        cids.iter().cloned().map(FactCid::new).collect(),
        false,
        started,
        None,
    );
    let citation = emem_core::algorithms::DEFAULT
        .lookup("spi_meteorological_drought@1")
        .map(|a| a.citation.clone())
        .unwrap_or_default();

    let current = match current {
        Some(c) if c.is_finite() => c,
        _ => {
            return Ok(json!({
                "schema": "emem.spi.v1",
                "algorithm_key": "spi_meteorological_drought@1",
                "cell": cell,
                "resolved_from": resolved_env,
                "verdict": "inconclusive",
                "available": false,
                "spi": JsonValue::Null,
                "honest_note": "No current-window accumulation available (empty stored trajectory and no current_accumulation_mm supplied). No SPI reported.",
                "source": source_note,
                "citation": citation,
                "responder_pubkey_b32": pubkey,
                "receipt": receipt,
            }));
        }
    };

    if history.is_empty() {
        source_note = "no precipitation history at this cell";
    }

    let mut body = compute_spi_body(&history, current, window_days);
    let obj = body
        .as_object_mut()
        .expect("compute_spi_body returns object");
    obj.insert("schema".into(), json!("emem.spi.v1"));
    obj.insert(
        "algorithm_key".into(),
        json!("spi_meteorological_drought@1"),
    );
    obj.insert("cell".into(), json!(cell));
    obj.insert("resolved_from".into(), json!(resolved_env));
    obj.insert("source".into(), json!(source_note));
    obj.insert(
        "formula".into(),
        json!("fit Γ(α,β) by Thom-1958 MLE to positive window-sums; H(x)=q+(1-q)·P(α,x/β); SPI=Φ⁻¹(H(x))"),
    );
    obj.insert("citation".into(), json!(citation));
    obj.insert("input_fact_cids".into(), json!(cids));
    obj.insert("responder_pubkey_b32".into(), json!(pubkey));
    obj.insert("receipt".into(), serde_json::to_value(&receipt).unwrap());
    Ok(body)
}

pub async fn post_spi(
    State(s): State<AppState>,
    Json(req): Json<SpiReq>,
) -> Result<Json<JsonValue>, ApiError> {
    if req.cell.trim().is_empty() {
        return Err(bad_request("spi requires a non-empty `cell`"));
    }
    Ok(Json(spi(req, &s).await?))
}

// ════════════════════════════════════════════════════════════════════
//  2. Burn severity (ΔNBR) endpoint
// ════════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize)]
pub struct BurnSeverityReq {
    pub cell: String,
    /// Pre-fire NBR. When omitted the endpoint reads the stored
    /// `indices.nbr` trajectory and uses the two most-recent scenes
    /// (older = pre, newer = post) — the agent should instead pin the
    /// scenes bracketing the fire date for a correct result.
    #[serde(default)]
    pub nbr_pre: Option<f64>,
    /// Post-fire NBR.
    #[serde(default)]
    pub nbr_post: Option<f64>,
}

/// Pure ΔNBR + class computation, unit-testable.
pub(crate) fn compute_dnbr(nbr_pre: f64, nbr_post: f64) -> JsonValue {
    let dnbr = nbr_pre - nbr_post;
    json!({
        "verdict": "computed",
        "available": true,
        "nbr_pre": nbr_pre,
        "nbr_post": nbr_post,
        "dnbr": dnbr,
        "dnbr_scaled_x1000": dnbr * 1000.0,
        "severity_class": dnbr_class(dnbr),
    })
}

pub async fn burn_severity(req: BurnSeverityReq, s: &AppState) -> Result<JsonValue, ApiError> {
    let started = Instant::now();
    let (cell, resolved) = crate::resolve_cell_field(&req.cell).await?;

    let mut cids: Vec<String> = Vec::new();
    let mut source_note = "caller-supplied nbr_pre/nbr_post";

    let (pre, post): (Option<f64>, Option<f64>) = match (req.nbr_pre, req.nbr_post) {
        (Some(a), Some(b)) => (Some(a), Some(b)),
        _ => {
            source_note = "stored indices.nbr trajectory (two most-recent scenes)";
            let treq = TrajectoryReq {
                cell: cell.clone(),
                band: "indices.nbr".to_string(),
                window: [0, u64::MAX],
                as_of_tslot: None,
                as_of_signed_at: None,
                scope: None,
            };
            let tr = trajectory(&treq, s)
                .await
                .map_err(|e| bad_request(format!("trajectory read failed: {e}")))?;
            let mut pts: Vec<(u64, f64, String)> = tr
                .series
                .iter()
                .filter_map(|p| {
                    as_f64(&p.value)
                        .filter(|v| v.is_finite())
                        .map(|v| (p.tslot, v, p.fact_cid.clone()))
                })
                .collect();
            pts.sort_by_key(|r| r.0);
            if pts.len() >= 2 {
                let pre = pts[pts.len() - 2].clone(); // older
                let post = pts[pts.len() - 1].clone(); // newer
                for c in [&pre.2, &post.2] {
                    if !c.is_empty() {
                        cids.push(c.clone());
                    }
                }
                (Some(pre.1), Some(post.1))
            } else {
                (pts.first().map(|p| p.1), None)
            }
        }
    };

    let resolved_env = crate::resolved_envelope(vec![("cell".into(), resolved)]);
    let pubkey = pubkey_b32(s);
    let receipt = s.sign_receipt(
        "emem.burn_severity",
        vec![cell.clone()],
        cids.iter().cloned().map(FactCid::new).collect(),
        false,
        started,
        None,
    );
    let citation = emem_core::algorithms::DEFAULT
        .lookup("burn_severity_from_dnbr@1")
        .map(|a| a.citation.clone())
        .unwrap_or_default();

    // Honest degradation: only one scene materializable → Absence, never
    // a fabricated ΔNBR from a single observation.
    let (pre, post) = match (pre, post) {
        (Some(a), Some(b)) => (a, b),
        _ => {
            return Ok(json!({
                "schema": "emem.burn_severity.v1",
                "algorithm_key": "burn_severity_from_dnbr@1",
                "cell": cell,
                "resolved_from": resolved_env,
                "verdict": "inconclusive",
                "available": false,
                "dnbr": JsonValue::Null,
                "honest_note": "Only one (or zero) NBR scene is available at this cell; ΔNBR requires two scenes bracketing the fire event. No severity reported — a single-scene ΔNBR would be fabricated. Supply nbr_pre and nbr_post, or pin the pre/post tslots before recalling indices.nbr.",
                "source": source_note,
                "citation": citation,
                "responder_pubkey_b32": pubkey,
                "receipt": receipt,
            }));
        }
    };

    let mut body = compute_dnbr(pre, post);
    let obj = body.as_object_mut().unwrap();
    obj.insert("schema".into(), json!("emem.burn_severity.v1"));
    obj.insert("algorithm_key".into(), json!("burn_severity_from_dnbr@1"));
    obj.insert("cell".into(), json!(cell));
    obj.insert("resolved_from".into(), json!(resolved_env));
    obj.insert("source".into(), json!(source_note));
    obj.insert(
        "formula".into(),
        json!("dnbr = NBR_pre − NBR_post; class via USGS/Key&Benson thresholds (0.10/0.27/0.44/0.66 NBR units)"),
    );
    obj.insert(
        "honest_note".into(),
        json!("Absolute ΔNBR class boundaries are region-tuned (Key & Benson 2006); treat a value within ~0.05 of a boundary as ambiguous and confirm against a field-calibrated CBI where stakes are high."),
    );
    obj.insert("citation".into(), json!(citation));
    obj.insert("input_fact_cids".into(), json!(cids));
    obj.insert("responder_pubkey_b32".into(), json!(pubkey));
    obj.insert("receipt".into(), serde_json::to_value(&receipt).unwrap());
    Ok(body)
}

pub async fn post_burn_severity(
    State(s): State<AppState>,
    Json(req): Json<BurnSeverityReq>,
) -> Result<Json<JsonValue>, ApiError> {
    if req.cell.trim().is_empty() {
        return Err(bad_request("burn_severity requires a non-empty `cell`"));
    }
    Ok(Json(burn_severity(req, &s).await?))
}

// ════════════════════════════════════════════════════════════════════
//  3. Rice CH4 (IPCC 2019) endpoint
// ════════════════════════════════════════════════════════════════════

/// Minimum NDWI samples needed to attempt a regime classification.
const RICE_MIN_NDWI: usize = 4;

#[derive(Debug, Deserialize)]
pub struct RiceCh4Req {
    pub cell: String,
    /// Cultivation-period length in days (typically 110–150). REQUIRED —
    /// IPCC Eq 5.1 integrates the daily EF over the cultivation period and
    /// there is no defensible default that fits all rice systems.
    pub cultivation_period_days: f64,
    /// Regional baseline EFc (kg CH4 / ha / day) from IPCC 2019 Table
    /// 5.11. REQUIRED — the regional pick (Asia.S 0.85, Asia.SE 1.22,
    /// Europe 1.56, …) drives the absolute magnitude and silently using
    /// the global 1.19 default would bias inventories by ~30 %. The
    /// caller must pick the row for the cell's IPCC region.
    pub efc_kg_ch4_ha_day: f64,
    /// Optional explicit NDWI series across the cultivation period. When
    /// omitted the endpoint reads the stored `indices.ndwi` trajectory.
    #[serde(default)]
    pub ndwi_series: Option<Vec<f64>>,
    /// Optional pre-season water-regime scaling factor SFp (Table 5.13).
    /// Default 0.68 = non-flooded pre-season > 180 d.
    #[serde(default)]
    pub sfp: Option<f64>,
    /// Optional organic-amendment scaling factor SFo (Table 5.14).
    /// Default 1.00 = no amendment.
    #[serde(default)]
    pub sfo: Option<f64>,
    /// Optional mean paddy-water temperature (°C) for the Yan 2005 Q10
    /// modifier. Omit to disable the temperature correction (T_mod = 1).
    #[serde(default)]
    pub t_paddy_c: Option<f64>,
}

pub async fn rice_ch4(req: RiceCh4Req, s: &AppState) -> Result<JsonValue, ApiError> {
    let started = Instant::now();
    let (cell, resolved) = crate::resolve_cell_field(&req.cell).await?;

    if !(req.cultivation_period_days.is_finite() && req.cultivation_period_days > 0.0) {
        return Err(bad_request(
            "rice_ch4 requires a positive cultivation_period_days",
        ));
    }
    if !(req.efc_kg_ch4_ha_day.is_finite() && req.efc_kg_ch4_ha_day > 0.0) {
        return Err(bad_request(
            "rice_ch4 requires a positive efc_kg_ch4_ha_day (IPCC 2019 Table 5.11 regional pick)",
        ));
    }

    let mut cids: Vec<String> = Vec::new();
    let mut source_note = "caller-supplied ndwi_series";

    let ndwi: Vec<f64> = if let Some(v) = req.ndwi_series.clone() {
        v
    } else {
        source_note = "stored indices.ndwi trajectory";
        let treq = TrajectoryReq {
            cell: cell.clone(),
            band: "indices.ndwi".to_string(),
            window: [0, u64::MAX],
            as_of_tslot: None,
            as_of_signed_at: None,
            scope: None,
        };
        let tr = trajectory(&treq, s)
            .await
            .map_err(|e| bad_request(format!("trajectory read failed: {e}")))?;
        let mut out = Vec::new();
        for p in &tr.series {
            if let Some(v) = as_f64(&p.value) {
                if v.is_finite() {
                    out.push(v);
                    if !p.fact_cid.is_empty() {
                        cids.push(p.fact_cid.clone());
                    }
                }
            }
        }
        out
    };

    let resolved_env = crate::resolved_envelope(vec![("cell".into(), resolved)]);
    let pubkey = pubkey_b32(s);
    let receipt = s.sign_receipt(
        "emem.rice_ch4",
        vec![cell.clone()],
        cids.iter().cloned().map(FactCid::new).collect(),
        false,
        started,
        None,
    );
    let citation = emem_core::algorithms::DEFAULT
        .lookup("rice_methane_awd_modeled@1")
        .map(|a| a.citation.clone())
        .unwrap_or_default();

    // Honest degradation: NDWI series too sparse to classify the water
    // regime → inconclusive, never a guessed continuous-flood SFw.
    let regime = match classify_water_regime(&ndwi, RICE_MIN_NDWI) {
        Some(r) => r,
        None => {
            return Ok(json!({
                "schema": "emem.rice_ch4.v1",
                "algorithm_key": "rice_methane_awd_modeled@1",
                "cell": cell,
                "resolved_from": resolved_env,
                "verdict": "inconclusive",
                "available": false,
                "ch4_kg_ha_season": JsonValue::Null,
                "n_ndwi_samples": ndwi.len(),
                "honest_note": format!(
                    "Only {} NDWI samples across the cultivation period; need at least {RICE_MIN_NDWI} to classify the water regime (continuous-flood vs AWD vs mid-season drainage). No CH4 reported — assuming a regime would fabricate the dominant SFw term.",
                    ndwi.len()
                ),
                "source": source_note,
                "citation": citation,
                "responder_pubkey_b32": pubkey,
                "receipt": receipt,
            }));
        }
    };

    let sfw = sfw_for_regime(regime);
    let sfp = req.sfp.unwrap_or(0.68);
    let sfo = req.sfo.unwrap_or(1.00);
    let ch4 = rice_ch4_kg_ha_season(
        req.efc_kg_ch4_ha_day,
        sfw,
        sfp,
        sfo,
        req.cultivation_period_days,
        req.t_paddy_c,
    );
    // AWD-vs-continuous-flood delta is what Verra VM0042 monetises:
    let ch4_continuous = rice_ch4_kg_ha_season(
        req.efc_kg_ch4_ha_day,
        sfw_for_regime("continuous_flood"),
        sfp,
        sfo,
        req.cultivation_period_days,
        req.t_paddy_c,
    );

    Ok(json!({
        "schema": "emem.rice_ch4.v1",
        "algorithm_key": "rice_methane_awd_modeled@1",
        "cell": cell,
        "resolved_from": resolved_env,
        "verdict": "computed",
        "available": true,
        "model": "IPCC_2019_Tier2",
        "water_regime": regime,
        "ch4_kg_ha_season": ch4,
        "ch4_co2e_kg_ha_season": ch4 * 27.9,
        "awd_vs_continuous_delta_kg_ha_season": (ch4_continuous - ch4).max(0.0),
        "factors": {
            "efc_kg_ch4_ha_day": req.efc_kg_ch4_ha_day,
            "sfw": sfw,
            "sfp": sfp,
            "sfo": sfo,
            "cultivation_period_days": req.cultivation_period_days,
            "t_mod": req.t_paddy_c.map(|t| (0.0917 * (t - 25.0)).exp()),
        },
        "n_ndwi_samples": ndwi.len(),
        "formula": "CH4 = EFc · SFw · SFp · SFo · days · exp(0.0917·(T−25)); SFw from regime; IPCC 2019 Eq 5.1",
        "honest_note": "Tier-2 INVENTORY estimate (model:IPCC_2019_Tier2), NOT a flux measurement. Verra VM0042 credit issuance requires ≥1 eddy-covariance flux tower per agro-ecological zone. EFc must be the caller's IPCC-region pick from Table 5.11 — no region default is applied.",
        "citation": citation,
        "input_fact_cids": cids,
        "responder_pubkey_b32": pubkey,
        "receipt": receipt,
    }))
}

pub async fn post_rice_ch4(
    State(s): State<AppState>,
    Json(req): Json<RiceCh4Req>,
) -> Result<Json<JsonValue>, ApiError> {
    if req.cell.trim().is_empty() {
        return Err(bad_request("rice_ch4 requires a non-empty `cell`"));
    }
    Ok(Json(rice_ch4(req, &s).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── numerical primitives ────────────────────────────────────────
    #[test]
    fn ln_gamma_known_values() {
        // Γ(5) = 24 → ln = ln 24.
        assert!((ln_gamma(5.0) - 24.0_f64.ln()).abs() < 1e-9);
        // Γ(0.5) = sqrt(pi).
        assert!((ln_gamma(0.5) - std::f64::consts::PI.sqrt().ln()).abs() < 1e-9);
    }

    #[test]
    fn gamma_p_matches_exponential_cdf() {
        // For a=1, P(1,x) = 1 - e^{-x} (exponential CDF).
        for &x in &[0.5_f64, 1.0, 2.0, 5.0] {
            let expected = 1.0 - (-x).exp();
            assert!((gamma_p(1.0, x) - expected).abs() < 1e-9, "x={x}");
        }
    }

    #[test]
    fn inv_normal_cdf_known_quantiles() {
        // Φ⁻¹(0.5)=0, Φ⁻¹(0.975)≈1.95996, Φ⁻¹(0.025)≈-1.95996.
        assert!(inv_normal_cdf(0.5).abs() < 1e-6);
        assert!((inv_normal_cdf(0.975) - 1.959_964).abs() < 1e-4);
        assert!((inv_normal_cdf(0.025) + 1.959_964).abs() < 1e-4);
    }

    // ── 1. SPI ───────────────────────────────────────────────────────
    /// A symmetric-ish history fit to a gamma: an accumulation at the
    /// fitted MEDIAN maps to SPI ≈ 0; one far below maps strongly
    /// negative (drought); one far above maps strongly positive.
    #[test]
    fn spi_sign_and_magnitude() {
        // Synthetic gamma-like positive history (mean ≈ 100 mm).
        let history: Vec<f64> = (1..=60).map(|i| 40.0 + (i as f64) * 2.0).collect();
        let mean = history.iter().sum::<f64>() / history.len() as f64;
        // Accumulation at the mean → SPI near 0 (within ±0.5 for a skewed dist).
        let r_mid = fit_spi(&history, mean, SPI_MIN_POSITIVE).expect("fit");
        assert!(
            r_mid.spi.abs() < 0.6,
            "mid SPI should be ~0, got {}",
            r_mid.spi
        );
        // A very dry window (10 mm) → strongly negative SPI (drought).
        let r_dry = fit_spi(&history, 10.0, SPI_MIN_POSITIVE).expect("fit");
        assert!(
            r_dry.spi < -1.0,
            "dry SPI should be < -1, got {}",
            r_dry.spi
        );
        assert!(
            spi_class(r_dry.spi).contains("drought"),
            "class {}",
            spi_class(r_dry.spi)
        );
        // A very wet window (220 mm) → strongly positive SPI.
        let r_wet = fit_spi(&history, 220.0, SPI_MIN_POSITIVE).expect("fit");
        assert!(r_wet.spi > 1.0, "wet SPI should be > 1, got {}", r_wet.spi);
    }

    /// HONEST DEGRADATION: a 2-point history can NEVER fit a gamma.
    /// compute_spi_body must return inconclusive with the count, no SPI.
    #[test]
    fn spi_insufficient_history_is_inconclusive() {
        let body = compute_spi_body(&[80.0, 120.0], 50.0, 90);
        assert_eq!(body["verdict"], "inconclusive");
        assert_eq!(body["available"], false);
        assert!(
            body["spi"].is_null(),
            "no SPI may be fabricated from 2 points"
        );
        assert_eq!(body["n_samples"], 2);
    }

    #[test]
    fn spi_degenerate_constant_history_refused() {
        // All-identical positives → a_thom = 0 → no shape → None.
        let history = vec![100.0_f64; 40];
        assert!(fit_spi(&history, 100.0, SPI_MIN_POSITIVE).is_none());
    }

    // ── 2. ΔNBR burn severity ────────────────────────────────────────
    #[test]
    fn dnbr_known_value_and_class() {
        // NBR_pre 0.55, NBR_post 0.05 → ΔNBR 0.50 → moderate_high.
        let body = compute_dnbr(0.55, 0.05);
        assert!((body["dnbr"].as_f64().unwrap() - 0.50).abs() < 1e-9);
        assert_eq!(body["dnbr_scaled_x1000"].as_f64().unwrap(), 500.0);
        assert_eq!(body["severity_class"], "moderate_high");
    }

    /// Assert every USGS/Key&Benson class-boundary edge. The thresholds
    /// are half-open `[lo, hi)`: a value exactly AT a boundary lands in
    /// the higher class.
    #[test]
    fn dnbr_class_boundary_edges() {
        assert_eq!(dnbr_class(0.099), "unburned");
        assert_eq!(dnbr_class(0.10), "low"); // boundary → higher class
        assert_eq!(dnbr_class(0.269), "low");
        assert_eq!(dnbr_class(0.27), "moderate_low");
        assert_eq!(dnbr_class(0.44), "moderate_high");
        assert_eq!(dnbr_class(0.659), "moderate_high");
        assert_eq!(dnbr_class(0.66), "high");
        assert_eq!(dnbr_class(1.20), "high");
        // Negative ΔNBR (post greener than pre = regrowth) → unburned.
        assert_eq!(dnbr_class(-0.30), "unburned");
    }

    // ── 3. Rice CH4 (IPCC 2019) ──────────────────────────────────────
    #[test]
    fn rice_regime_classification() {
        // Continuously positive, no crossings → continuous_flood.
        assert_eq!(
            classify_water_regime(&[0.2, 0.25, 0.18, 0.3, 0.22], RICE_MIN_NDWI),
            Some("continuous_flood")
        );
        // Many oscillations across 0 → AWD.
        assert_eq!(
            classify_water_regime(&[0.2, -0.1, 0.3, -0.2, 0.25, -0.05], RICE_MIN_NDWI),
            Some("AWD")
        );
        // Single drainage (two crossings), positive mean → mid_season_drainage.
        assert_eq!(
            classify_water_regime(&[0.3, 0.2, -0.1, 0.25, 0.3], RICE_MIN_NDWI),
            Some("mid_season_drainage")
        );
        // All dry, no crossings → upland.
        assert_eq!(
            classify_water_regime(&[-0.2, -0.3, -0.1, -0.25], RICE_MIN_NDWI),
            Some("upland_or_dry_seeded")
        );
    }

    /// IPCC Eq 5.1 with the documented Asia.S baseline. Continuous-flood
    /// at EFc=0.85, SFp=0.68, SFo=1.0, 120 d, no temp correction:
    /// 0.85·1.00·0.68·1.00·120 = 69.36 kg CH4/ha. AWD (SFw 0.52) is 0.52×
    /// of that = 36.0672.
    #[test]
    fn rice_ch4_ipcc_known_value() {
        let cont = rice_ch4_kg_ha_season(
            0.85,
            sfw_for_regime("continuous_flood"),
            0.68,
            1.0,
            120.0,
            None,
        );
        assert!((cont - 69.36).abs() < 1e-6, "got {cont}");
        let awd = rice_ch4_kg_ha_season(0.85, sfw_for_regime("AWD"), 0.68, 1.0, 120.0, None);
        assert!((awd - 36.0672).abs() < 1e-6, "got {awd}");
        // The monetisable delta is positive (AWD emits less).
        assert!(cont - awd > 30.0);
    }

    #[test]
    fn rice_q10_temperature_modifier() {
        // At 25 °C, T_mod = 1 → same as None.
        let at25 = rice_ch4_kg_ha_season(1.0, 1.0, 1.0, 1.0, 100.0, Some(25.0));
        assert!((at25 - 100.0).abs() < 1e-9);
        // Warmer paddy → more CH4 (Q10 ≈ 2.5).
        let at35 = rice_ch4_kg_ha_season(1.0, 1.0, 1.0, 1.0, 100.0, Some(35.0));
        assert!(at35 > 100.0, "warmer paddy must emit more, got {at35}");
        assert!((at35 / at25 - (0.0917_f64 * 10.0).exp()).abs() < 1e-6);
    }

    /// HONEST DEGRADATION: a 3-sample NDWI series is below the
    /// classification floor → None, so the endpoint reports inconclusive.
    #[test]
    fn rice_sparse_ndwi_is_unclassifiable() {
        assert!(classify_water_regime(&[0.1, 0.2, 0.15], RICE_MIN_NDWI).is_none());
    }
}
