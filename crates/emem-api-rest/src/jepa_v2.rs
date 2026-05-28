//! `jepa_temporal_predictor@2.scalar` — multi-band scalar dynamics head.
//!
//! ## What changed in v0.0.1 (2026-05-28)
//!
//! The pre-2026-05-28 v2 surface was a residual MLP over K=3 Tessera
//! 128-D vintages → next-vintage 128-D vector. That contract was
//! broken in practice because the upstream `dl2.geotessera.org`
//! bucket only serves the 2024 vintage reliably (2017-2023 return
//! null), so all three "lags" collapsed to the same vector and the
//! identity short-circuit always fired. The .onnx was a zero-init
//! sentinel; the receipts surfaced `untrained_baseline` and
//! `upstream_geotessera_single_vintage` honesty warnings.
//!
//! This module now wraps a **real trained Transformer** over a
//! multi-band scalar state. The pivot is documented in
//! `python/jepa_v2_sidecar/training_data_report.md`.
//!
//! ## Wire shape (v0.0.1)
//!
//! Input  : `[INPUT_DIM = K_LAGS*N_BANDS + N_CONTEXT = 6*4 + 5 = 29]` f32
//!   layout: `lag0_band0, lag0_band1, lag0_band2, lag0_band3, lag1_*, ..., lag5_*, ctx0..ctx4`
//!   where ctx = `(lat_norm, lng_sin, lng_cos, month_sin, month_cos)`
//!   and each lag value is z-scored: `(v - band_mean) / band_std`.
//!
//! Output : `[2 * N_BANDS = 8]` f32 — first N_BANDS are normalised
//!   means, next N_BANDS are log-variances.
//!
//! ## Honesty
//!
//! When `dynamics_v2.metadata.json::training.trained == false`, the
//! handler short-circuits to identity and surfaces
//! `untrained_baseline`. When `training.trained == true` (current
//! state), the receipt drops the untrained warning but carries
//! `training_synthetic: <synthetic_fraction>` so verifiers see how
//! much real corpus history grounds the model. When a prediction's
//! confidence (= exp(-log_var/2)) drops below
//! `LOW_CONFIDENCE_THRESHOLD` for any band, the receipt adds a
//! `low_confidence_band:<key>` warning so agents downstream can fall
//! back to other signals.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::ApiError;

/// Number of past lags the dynamics head conditions on.
pub const K_LAGS: usize = 6;

/// Number of scalar bands per lag step.
pub const N_BANDS: usize = 4;

/// Context vector size: `(lat_norm, lng_sin, lng_cos, month_sin, month_cos)`.
pub const N_CONTEXT: usize = 5;

/// Total flat input dimension the ONNX expects.
pub const INPUT_DIM: usize = K_LAGS * N_BANDS + N_CONTEXT;

/// Output dimension: `mean[N_BANDS] || log_var[N_BANDS]`.
pub const OUTPUT_DIM: usize = 2 * N_BANDS;

/// Canonical band-key order the model was trained on.
pub const BAND_KEYS: [&str; N_BANDS] = [
    "indices.ndvi",
    "modis.lst_day_8day",
    "modis.lst_night_8day",
    "cams.pm25",
];

/// Confidence threshold below which the receipt adds a per-band
/// `low_confidence_band:<key>` honesty warning. The model's
/// confidence is `exp(-log_var / 2)`; values below ~0.3 indicate the
/// predicted std-dev is large relative to the band's normalised
/// scale, which downstream agents should treat as "don't fully trust".
pub const LOW_CONFIDENCE_THRESHOLD: f32 = 0.30;

/// Resolved on-disk paths for the artifact + sidecar.
fn artifact_dir() -> PathBuf {
    if let Ok(p) = std::env::var("EMEM_JEPA_V2_DIR") {
        return PathBuf::from(p);
    }
    let data = std::env::var("EMEM_DATA").unwrap_or_else(|_| "/home/ubuntu/emem/var/emem".into());
    PathBuf::from(data).join("jepa_v2")
}

fn onnx_path() -> PathBuf {
    artifact_dir().join("dynamics_v2.onnx")
}

fn metadata_path() -> PathBuf {
    artifact_dir().join("dynamics_v2.metadata.json")
}

/// Raw deserialised contents of `dynamics_v2.metadata.json`. We hand
/// the parsed JSON straight back through the receipt so the wire form
/// is the single source of truth.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModelMetadata {
    pub model_id: String,
    pub version: String,
    #[serde(default)]
    pub training: serde_json::Value,
    #[serde(default)]
    pub validation: serde_json::Value,
    #[serde(default)]
    pub normalisation: serde_json::Value,
    #[serde(default)]
    pub architecture: serde_json::Value,
    #[serde(default)]
    pub wire: serde_json::Value,
    pub artifact: ArtifactInfo,
    #[serde(default)]
    pub trained_at_iso: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ArtifactInfo {
    pub filename: String,
    pub size_bytes: u64,
    pub blake2b_hex: String,
}

impl ModelMetadata {
    /// True when the metadata sidecar reports `training.trained == true`.
    ///
    /// Default is **false** (fail-safe). A malformed metadata file that
    /// omits the `trained` field MUST NOT silently flip the receipt's
    /// "trained" flag to true — that would mean a verifier reading the
    /// receipt sees no `untrained_baseline` honesty warning while the
    /// served prediction comes from random weights. The honest default
    /// is "I don't know what trained this, treat as untrained, surface
    /// the warning". When `train_dynamics_v2.py` ships a real model it
    /// must explicitly write `training.trained = true` into metadata.
    pub fn is_trained(&self) -> bool {
        self.training
            .get("trained")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    /// Synthetic-fraction reported by the trainer. Returns 1.0 when
    /// absent so receipts are conservatively honest (all-synthetic).
    pub fn synthetic_fraction(&self) -> f64 {
        self.training
            .get("synthetic_fraction_train")
            .and_then(|v| v.as_f64())
            .unwrap_or(1.0)
    }

    /// Per-band (mean, std) used to normalise lags + denormalise
    /// predictions. Returns hard-coded defaults if metadata is silent
    /// — those defaults must match the training-time
    /// `BAND_NORM` constants in `train_dynamics_v2.py`.
    pub fn band_norm(&self, band: &str) -> (f32, f32) {
        let fallback = match band {
            "indices.ndvi" => (0.30_f32, 0.30_f32),
            "modis.lst_day_8day" => (290.0, 15.0),
            "modis.lst_night_8day" => (280.0, 12.0),
            "cams.pm25" => (15.0, 20.0),
            _ => (0.0, 1.0),
        };
        if let Some(entry) = self
            .normalisation
            .get("band_norm")
            .and_then(|v| v.get(band))
            .and_then(|v| v.as_array())
        {
            if entry.len() == 2 {
                if let (Some(m), Some(s)) = (entry[0].as_f64(), entry[1].as_f64()) {
                    return (m as f32, s as f32);
                }
            }
        }
        fallback
    }

    /// Per-band (min, max) physical clamp applied after denormalisation.
    pub fn physical_range(&self, band: &str) -> (f32, f32) {
        let fallback = match band {
            "indices.ndvi" => (-1.0_f32, 1.0_f32),
            "modis.lst_day_8day" => (220.0, 340.0),
            "modis.lst_night_8day" => (210.0, 320.0),
            "cams.pm25" => (0.0, 500.0),
            _ => (f32::NEG_INFINITY, f32::INFINITY),
        };
        if let Some(entry) = self
            .normalisation
            .get("physical_ranges")
            .and_then(|v| v.get(band))
            .and_then(|v| v.as_array())
        {
            if entry.len() == 2 {
                if let (Some(lo), Some(hi)) = (entry[0].as_f64(), entry[1].as_f64()) {
                    return (lo as f32, hi as f32);
                }
            }
        }
        fallback
    }
}

/// Lazy-loaded session + parsed metadata.
struct LoadedModel {
    session: Arc<Mutex<ort::session::Session>>,
    metadata: ModelMetadata,
}

static MODEL: OnceLock<Result<Arc<LoadedModel>, String>> = OnceLock::new();
static METADATA: OnceLock<Result<ModelMetadata, String>> = OnceLock::new();

/// Read + parse `dynamics_v2.metadata.json` once and cache it.
pub fn ensure_metadata() -> Result<ModelMetadata, String> {
    METADATA
        .get_or_init(|| {
            let meta_path = metadata_path();
            if !meta_path.exists() {
                return Err(format!(
                    "jepa_v2 metadata not found at {}; the .onnx and metadata \
                     sidecar ship together. Re-run python/jepa_v2_sidecar/train_dynamics_v2.py.",
                    meta_path.display()
                ));
            }
            let text =
                std::fs::read_to_string(&meta_path).map_err(|e| format!("read metadata: {e}"))?;
            serde_json::from_str::<ModelMetadata>(&text).map_err(|e| format!("parse metadata: {e}"))
        })
        .clone()
}

/// True when the loaded model's metadata reports `training.trained == true`.
pub fn is_trained() -> bool {
    ensure_metadata().map(|m| m.is_trained()).unwrap_or(false)
}

fn ensure_loaded() -> Result<Arc<LoadedModel>, String> {
    MODEL
        .get_or_init(|| {
            let onnx = onnx_path();
            if !onnx.exists() {
                return Err(format!(
                    "jepa_v2 artifact not found at {}; \
                     bootstrap with `python python/jepa_v2_sidecar/train_dynamics_v2.py` \
                     which writes the trained ONNX + metadata sidecar.",
                    onnx.display()
                ));
            }
            let meta_path = metadata_path();
            if !meta_path.exists() {
                return Err(format!(
                    "jepa_v2 metadata not found at {}; the .onnx and metadata \
                     sidecar ship together. Re-run the python training script.",
                    meta_path.display()
                ));
            }
            let meta_text =
                std::fs::read_to_string(&meta_path).map_err(|e| format!("read metadata: {e}"))?;
            let metadata: ModelMetadata =
                serde_json::from_str(&meta_text).map_err(|e| format!("parse metadata: {e}"))?;

            let on_disk_bytes = std::fs::read(&onnx).map_err(|e| format!("read onnx: {e}"))?;
            if on_disk_bytes.len() as u64 != metadata.artifact.size_bytes {
                return Err(format!(
                    "jepa_v2 size mismatch: metadata says {} bytes but \
                     {} on disk is {} bytes. Re-run the training script.",
                    metadata.artifact.size_bytes,
                    onnx.display(),
                    on_disk_bytes.len()
                ));
            }

            let _ = ort::init().commit();
            let session = ort::session::Session::builder()
                .map_err(|e| format!("ort session builder: {e}"))?
                .with_optimization_level(ort::session::builder::GraphOptimizationLevel::Level3)
                .map_err(|e| format!("ort opt level: {e}"))?
                .with_intra_threads(2)
                .map_err(|e| format!("ort intra threads: {e}"))?
                .commit_from_file(&onnx)
                .map_err(|e| format!("ort commit_from_file: {e}"))?;

            tracing::info!(
                target: "emem::jepa_v2",
                model_id = %metadata.model_id,
                version = %metadata.version,
                size_bytes = metadata.artifact.size_bytes,
                blake2b = %metadata.artifact.blake2b_hex,
                trained = metadata.is_trained(),
                synthetic_fraction = metadata.synthetic_fraction(),
                "jepa_v2 loaded"
            );

            Ok(Arc::new(LoadedModel {
                session: Arc::new(Mutex::new(session)),
                metadata,
            }))
        })
        .clone()
}

/// A single inference call's output. `predictions` is in physical units
/// (denormalised + clamped). `confidence` is `exp(-log_var/2)` per band,
/// roughly the inverse predictive std-dev. `low_confidence_bands` lists
/// the band keys whose confidence dropped below
/// `LOW_CONFIDENCE_THRESHOLD`; the handler folds these into receipt
/// honesty warnings.
#[derive(Debug, Clone, Serialize)]
pub struct DynamicsOutput {
    pub predictions: Vec<f32>,
    pub confidence: Vec<f32>,
    pub low_confidence_bands: Vec<String>,
    pub band_keys: Vec<String>,
}

/// Public entry point: given K lags of N_BANDS scalars + spatial-
/// temporal context, predict next-step scalars + confidence.
///
/// `lags_normalised` must be exactly `K_LAGS * N_BANDS` f32 values
/// (already z-scored with `metadata.band_norm`). `context` must be
/// exactly `N_CONTEXT` f32 values in the wire order. Output is in
/// physical units (denormalised + clamped to `metadata.physical_range`).
pub fn predict_next_step(
    lags_normalised: &[f32],
    context: &[f32],
) -> Result<(DynamicsOutput, ModelMetadata), String> {
    if lags_normalised.len() != K_LAGS * N_BANDS {
        return Err(format!(
            "jepa_v2.predict_next_step: lags_normalised must be {} f32 values \
             (K_LAGS={}*N_BANDS={}); got {}",
            K_LAGS * N_BANDS,
            K_LAGS,
            N_BANDS,
            lags_normalised.len()
        ));
    }
    if context.len() != N_CONTEXT {
        return Err(format!(
            "jepa_v2.predict_next_step: context must be {} f32 values; got {}",
            N_CONTEXT,
            context.len()
        ));
    }
    let model = ensure_loaded()?;

    let mut flat: Vec<f32> = Vec::with_capacity(INPUT_DIM);
    flat.extend_from_slice(lags_normalised);
    flat.extend_from_slice(context);
    let input = ort::value::Tensor::from_array(([1usize, INPUT_DIM], flat))
        .map_err(|e| format!("ort input tensor: {e}"))?;
    let mut guard = model
        .session
        .lock()
        .map_err(|e| format!("session mutex: {e}"))?;
    let outputs = guard
        .run(ort::inputs!["input" => input])
        .map_err(|e| format!("ort run: {e}"))?;
    let (_name, output) = outputs.iter().next().ok_or("ort returned no outputs")?;
    let arr = output
        .try_extract_array::<f32>()
        .map_err(|e| format!("extract output: {e}"))?;
    let batch0 = arr.index_axis(ort_ndarray::Axis(0), 0);
    let raw: Vec<f32> = batch0.iter().copied().collect();
    if raw.len() != OUTPUT_DIM {
        return Err(format!(
            "jepa_v2 model produced {} dims, expected {}",
            raw.len(),
            OUTPUT_DIM
        ));
    }
    let meta = model.metadata.clone();
    let pred_norm = &raw[..N_BANDS];
    let log_var = &raw[N_BANDS..];

    let mut predictions = Vec::with_capacity(N_BANDS);
    let mut confidence = Vec::with_capacity(N_BANDS);
    let mut low_conf = Vec::new();
    for i in 0..N_BANDS {
        let band = BAND_KEYS[i];
        let (mean, std) = meta.band_norm(band);
        let (lo, hi) = meta.physical_range(band);
        let phys = (pred_norm[i] * std + mean).clamp(lo, hi);
        predictions.push(phys);
        // Clamp log_var into the same range the training loss clamped it.
        let lv = log_var[i].clamp(-6.0, 4.0);
        let conf = (-lv / 2.0).exp();
        confidence.push(conf);
        if conf < LOW_CONFIDENCE_THRESHOLD {
            low_conf.push(band.to_string());
        }
    }
    Ok((
        DynamicsOutput {
            predictions,
            confidence,
            low_confidence_bands: low_conf,
            band_keys: BAND_KEYS.iter().map(|s| s.to_string()).collect(),
        },
        meta,
    ))
}

/// Fold the loaded model's metadata into the receipt's derivation
/// args. Caller is the `/v1/jepa_predict_v2` handler.
///
/// `out` is the inference result so honesty warnings can include the
/// model's per-band low-confidence list. When `out` is None (e.g. the
/// caller short-circuited before running inference), no
/// confidence-derived warnings are added.
pub fn receipt_block(metadata: &ModelMetadata, out: Option<&DynamicsOutput>) -> JsonValue {
    let mut warnings: Vec<JsonValue> = Vec::new();
    if !metadata.is_trained() {
        warnings.push(JsonValue::String(
            "untrained_baseline: this jepa_v2 model is the residual-zero-init \
             sentinel that returns last_input_vintage by construction. \
             Quality is the 'predict last vintage' baseline, NOT a learned \
             forecast."
                .into(),
        ));
    }
    let synth = metadata.synthetic_fraction();
    if metadata.is_trained() && synth > 0.5 {
        warnings.push(JsonValue::String(format!(
            "training_synthetic_fraction={synth:.2}: the trained model was \
             fit predominantly on synthesized seasonality data because the \
             corpus on this responder has too little multi-tslot history \
             to train end-to-end. Real-corpus eval is held-out; see \
             training_data_report.md and metadata.training.real_ndvi_mse_model \
             for the honest real-cell number."
        )));
    }
    if let Some(o) = out {
        for band in &o.low_confidence_bands {
            warnings.push(JsonValue::String(format!(
                "low_confidence_band:{band}: model predictive std-dev for \
                 this band exceeded the LOW_CONFIDENCE_THRESHOLD ({thresh:.2}); \
                 downstream agents should fall back to other signals or \
                 collect more lags before acting.",
                thresh = LOW_CONFIDENCE_THRESHOLD
            )));
        }
    }
    serde_json::json!({
        "model_id": metadata.model_id,
        "version": metadata.version,
        "blake2b_hex": metadata.artifact.blake2b_hex,
        "size_bytes": metadata.artifact.size_bytes,
        "trained": metadata.is_trained(),
        "synthetic_fraction_train": synth,
        "training": metadata.training,
        "validation": metadata.validation,
        "architecture": metadata.architecture,
        "normalisation": metadata.normalisation,
        "wire": metadata.wire,
        "trained_at_iso": metadata.trained_at_iso,
        "honesty_warnings": JsonValue::Array(warnings),
    })
}

/// Map the inference module's Result into a JEPA-shaped `ApiError`.
pub fn into_api_error(err: String) -> ApiError {
    use crate::ErrorBody;
    use axum::http::StatusCode;
    use emem_core::error::ErrorCode;
    let lower = err.to_ascii_lowercase();
    let (status, code) = if lower.contains("not found") || lower.contains("not exist") {
        (StatusCode::SERVICE_UNAVAILABLE, ErrorCode::CacheError)
    } else if lower.contains("size mismatch") || lower.contains("parse metadata") {
        (StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::CacheError)
    } else {
        (StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::Internal)
    };
    ApiError(
        status,
        ErrorBody {
            code,
            message: err,
            details: None,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn untrained_meta() -> ModelMetadata {
        serde_json::from_value(json!({
            "model_id": "jepa_temporal_predictor@2",
            "version": "0.0.0-untrained-baseline",
            "training": {"trained": false},
            "validation": {},
            "artifact": {"filename": "dynamics_v2.onnx", "size_bytes": 8070, "blake2b_hex": "abc"},
        }))
        .expect("parse")
    }

    fn trained_meta() -> ModelMetadata {
        serde_json::from_value(json!({
            "model_id": "jepa_temporal_predictor@2.scalar",
            "version": "0.0.1-trained-multi-band-scalar",
            "training": {
                "trained": true,
                "synthetic_fraction_train": 1.0,
                "training_data_size": 12000,
                "val_loss": -1.76,
            },
            "validation": {},
            "normalisation": {
                "band_norm": {
                    "indices.ndvi": [0.30, 0.30],
                    "modis.lst_day_8day": [290.0, 15.0],
                    "modis.lst_night_8day": [280.0, 12.0],
                    "cams.pm25": [15.0, 20.0]
                },
                "physical_ranges": {
                    "indices.ndvi": [-1.0, 1.0],
                    "modis.lst_day_8day": [220.0, 340.0],
                    "modis.lst_night_8day": [210.0, 320.0],
                    "cams.pm25": [0.0, 500.0]
                }
            },
            "artifact": {
                "filename": "dynamics_v2.onnx", "size_bytes": 171443, "blake2b_hex": "deadbeef"
            },
        }))
        .expect("parse")
    }

    /// `is_trained` defaults to FALSE when the `training.trained` field
    /// is absent — fail-safe so malformed metadata can't silently strip
    /// the `untrained_baseline` honesty warning from the receipt.
    #[test]
    fn metadata_is_trained_default_false_when_field_absent() {
        let m: ModelMetadata = serde_json::from_value(json!({
            "model_id": "jepa_temporal_predictor@2",
            "version": "0.0.1",
            "training": {"epochs": 200},
            "validation": {"cosine_similarity": 0.81},
            "artifact": {"filename": "dynamics_v2.onnx", "size_bytes": 100, "blake2b_hex": "00"},
        }))
        .expect("parse");
        assert!(
            !m.is_trained(),
            "absent `training.trained` field must default to FALSE so receipts \
             carry the untrained_baseline honesty warning by default"
        );
    }

    #[test]
    fn metadata_is_trained_true_when_field_explicitly_true() {
        let m = trained_meta();
        assert!(m.is_trained());
    }

    #[test]
    fn metadata_is_trained_false_when_explicitly_false() {
        let m = untrained_meta();
        assert!(!m.is_trained());
    }

    /// Receipt block for an UNTRAINED model carries the
    /// `untrained_baseline` warning so an LLM never reads the
    /// prediction as a learned forecast. The
    /// `upstream_geotessera_single_vintage` warning from the previous
    /// v0 surface is dropped because the new shape doesn't depend on
    /// Tessera at all — that caveat no longer applies.
    #[test]
    fn receipt_block_emits_untrained_warning() {
        let m = untrained_meta();
        let block = receipt_block(&m, None);
        let warnings = block
            .get("honesty_warnings")
            .and_then(|v| v.as_array())
            .expect("honesty_warnings is an array");
        assert!(warnings
            .iter()
            .any(|w| w.as_str().unwrap_or("").contains("untrained_baseline")));
        // The new shape is Tessera-independent so the geotessera caveat
        // MUST NOT appear here.
        assert!(!warnings.iter().any(|w| w
            .as_str()
            .unwrap_or("")
            .contains("upstream_geotessera_single_vintage")));
        assert_eq!(block["trained"].as_bool(), Some(false));
        assert_eq!(block["blake2b_hex"].as_str(), Some("abc"));
    }

    /// Trained model with high synthetic_fraction emits a
    /// `training_synthetic_fraction=...` warning so verifiers see how
    /// little real corpus history anchors the model.
    #[test]
    fn receipt_block_trained_with_synthetic_emits_caveat() {
        let m = trained_meta();
        let block = receipt_block(&m, None);
        let warnings = block
            .get("honesty_warnings")
            .and_then(|v| v.as_array())
            .expect("honesty_warnings");
        // No untrained warning (since trained=true) but synthetic-fraction warning present.
        assert!(!warnings
            .iter()
            .any(|w| w.as_str().unwrap_or("").contains("untrained_baseline")));
        assert!(warnings.iter().any(|w| w
            .as_str()
            .unwrap_or("")
            .contains("training_synthetic_fraction")));
        assert_eq!(block["trained"].as_bool(), Some(true));
        assert_eq!(block["blake2b_hex"].as_str(), Some("deadbeef"));
    }

    /// Trained model with a low-confidence band emits a per-band warning.
    #[test]
    fn receipt_block_emits_low_confidence_warning() {
        let m = trained_meta();
        let out = DynamicsOutput {
            predictions: vec![0.5, 290.0, 280.0, 15.0],
            confidence: vec![0.10, 0.99, 0.99, 0.99],
            low_confidence_bands: vec!["indices.ndvi".to_string()],
            band_keys: BAND_KEYS.iter().map(|s| s.to_string()).collect(),
        };
        let block = receipt_block(&m, Some(&out));
        let warnings = block
            .get("honesty_warnings")
            .and_then(|v| v.as_array())
            .expect("honesty_warnings");
        assert!(warnings.iter().any(|w| w
            .as_str()
            .unwrap_or("")
            .contains("low_confidence_band:indices.ndvi")));
    }

    /// `predict_next_step` rejects mis-shaped input fast.
    #[test]
    fn predict_rejects_wrong_input_length() {
        let too_short = vec![0.0_f32; 5];
        let ctx = vec![0.0_f32; N_CONTEXT];
        let err = predict_next_step(&too_short, &ctx).expect_err("must reject short input");
        assert!(
            err.contains(&(K_LAGS * N_BANDS).to_string()),
            "error should cite expected lag size; got {err}"
        );
    }

    #[test]
    fn predict_rejects_wrong_context_length() {
        let lags = vec![0.0_f32; K_LAGS * N_BANDS];
        let ctx_short = vec![0.0_f32; 3];
        let err = predict_next_step(&lags, &ctx_short).expect_err("must reject short ctx");
        assert!(
            err.contains(&N_CONTEXT.to_string()),
            "error should cite expected ctx size; got {err}"
        );
    }

    /// Integration check: when the on-disk metadata reports
    /// `training.trained == true` (the post-2026-05-28 trained model),
    /// the receipt block MUST NOT carry the legacy
    /// `untrained_baseline` honesty warning AND MUST NOT carry the
    /// legacy `upstream_geotessera_single_vintage` warning. Both were
    /// load-bearing in the v0 sentinel surface; if either reappears
    /// after the pivot it means metadata corruption or a stale receipt
    /// path. Skips silently when the artifact isn't present (clean
    /// checkout).
    #[test]
    fn trained_receipt_drops_legacy_warnings() {
        let meta = match ensure_metadata() {
            Ok(m) => m,
            Err(_) => return,
        };
        if !meta.is_trained() {
            return; // sentinel still on disk — covered by other tests
        }
        let block = receipt_block(&meta, None);
        let warnings = block
            .get("honesty_warnings")
            .and_then(|v| v.as_array())
            .expect("honesty_warnings array present");
        for w in warnings.iter().filter_map(|w| w.as_str()) {
            assert!(
                !w.contains("untrained_baseline"),
                "trained model must NOT emit untrained_baseline; got {w}"
            );
            assert!(
                !w.contains("upstream_geotessera_single_vintage"),
                "the post-pivot model is Tessera-independent; old warning leaked: {w}"
            );
        }
        // It MAY (and currently must) carry the synthetic-fraction caveat.
        assert_eq!(block["trained"].as_bool(), Some(true));
        assert!(
            block.get("blake2b_hex").and_then(|v| v.as_str()).is_some(),
            "receipt must carry the model's artifact blake2b"
        );
    }

    /// End-to-end inference against the on-disk artifact. Only runs
    /// when the real ONNX is in $EMEM_DATA/jepa_v2/. Skips silently
    /// when the artifact is absent so the test suite still passes on
    /// clean checkouts. Asserts:
    /// - inference runs without error
    /// - output is in the documented physical ranges
    /// - the predicted next-step is NOT bit-identical to the last lag
    ///   (proving it's a real learned model, not identity).
    #[test]
    fn predict_against_real_onnx_when_present() {
        let onnx = onnx_path();
        if !onnx.exists() {
            return;
        }
        let meta = match ensure_metadata() {
            Ok(m) => m,
            Err(_) => return,
        };
        if !meta.is_trained() {
            // Untrained sentinel skips this assertion.
            return;
        }
        // Build a non-degenerate input: 6 lags of 4 distinct scalars.
        let mut lags_phys: Vec<Vec<f32>> = Vec::with_capacity(K_LAGS);
        for i in 0..K_LAGS {
            lags_phys.push(vec![
                0.4 + 0.05 * i as f32,  // NDVI
                285.0 + 2.0 * i as f32, // LST day
                275.0 + 1.5 * i as f32, // LST night
                12.0 + 0.5 * i as f32,  // PM25
            ]);
        }
        // Normalise.
        let mut lags_norm = Vec::with_capacity(K_LAGS * N_BANDS);
        for lag in &lags_phys {
            for (j, band) in BAND_KEYS.iter().enumerate() {
                let (mean, std) = meta.band_norm(band);
                lags_norm.push((lag[j] - mean) / std);
            }
        }
        let ctx: Vec<f32> = vec![0.5, 0.0, 1.0, 0.0, 1.0];
        let (out, _) = predict_next_step(&lags_norm, &ctx).expect("inference");

        // Range check.
        let last_lag = lags_phys.last().unwrap();
        for (i, band) in BAND_KEYS.iter().enumerate() {
            let (lo, hi) = meta.physical_range(band);
            let p = out.predictions[i];
            assert!(
                p >= lo && p <= hi,
                "{band}: prediction {p} outside [{lo}, {hi}]"
            );
            assert!(
                out.confidence[i] > 0.0,
                "{band}: confidence must be > 0; got {}",
                out.confidence[i]
            );
            // Identity check: predicted scalar should not equal the
            // last lag bit-for-bit. Allow a 1e-7 tolerance for FP
            // determinism but reject the bit-identical case.
            let diff = (p - last_lag[i]).abs();
            assert!(
                diff > 1e-7,
                "{band}: prediction {p} == last lag {} bit-for-bit; \
                 model behaving as identity, not a real predictor",
                last_lag[i]
            );
        }
    }
}
