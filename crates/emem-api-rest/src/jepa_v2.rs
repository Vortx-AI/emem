//! `jepa_temporal_predictor@2` runtime — small learned dynamics head
//! over Tessera embeddings.
//!
//! v2 is the upgrade path off v1 (the closed-form AR(2) seasonal
//! predictor in `physics::jepa_predict`). It shares the same
//! agent-facing question — "predict the next vintage" — but answers
//! it in the 128-D Tessera latent space rather than NDVI scalar
//! space. The fact this surface signs is therefore a 128-D vector,
//! not a clamped float; downstream agents can compare the prediction
//! to other Tessera-attested cells via cosine, or dot-decode it
//! through any algorithm in `algorithms_for_topic.foundation_embedding`.
//!
//! Wire path:
//! 1. Recall the K most-recent Tessera vintages at the cell. K=3.
//! 2. Stack into `[1, 3, 128]` fp32 tensor.
//! 3. Run the ONNX dynamics head via ort (CPU EP — the model is ~200k
//!    params and inference is ~50µs even on CPU).
//! 4. Sign the resulting `[128]` vector as a Primary fact under the
//!    band `geotessera.predicted_<next_year>`. The receipt's
//!    `derivation` carries `model_cid` (blake2b of the ONNX bytes) +
//!    the metadata sidecar's training stats so agents can replay /
//!    audit which checkpoint produced any given prediction.
//!
//! Artifact location: `${EMEM_JEPA_V2_DIR}/dynamics_v2.onnx` plus a
//! sidecar `dynamics_v2.metadata.json`. Defaults to
//! `${EMEM_DATA}/jepa_v2/`.
//!
//! Honesty: when the artifact is missing, calls return a structured
//! error with the exact `python/jepa_v2/export_baseline.py` command
//! to seed it. When the loaded model's metadata flags
//! `training.trained == false`, every receipt's `honesty_warnings`
//! carries `untrained_baseline` so an LLM never reads the prediction
//! as a learned forecast.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::ApiError;

/// Number of past vintages the dynamics head conditions on. Locked at
/// load time — must match the .onnx artifact's input shape `[*, K, 128]`.
pub const INPUT_LAGS: usize = 3;

/// Tessera embedding dimension.
pub const TESSERA_DIM: usize = 128;

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
    /// the warning". When `train.py` ships a real model it must
    /// explicitly write `training.trained = true` into metadata.
    pub fn is_trained(&self) -> bool {
        self.training
            .get("trained")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }
}

/// Lazy-loaded session + parsed metadata. ort sessions are `Send` but
/// `run` takes `&mut self`, so we hold the session inside a `Mutex`.
/// One process-wide instance — first call pays load latency
/// (~tens of ms for an 8 KB ONNX), subsequent calls hit the hot
/// session.
struct LoadedModel {
    session: Arc<Mutex<ort::session::Session>>,
    metadata: ModelMetadata,
}

static MODEL: OnceLock<Result<Arc<LoadedModel>, String>> = OnceLock::new();

/// Metadata-only cache, populated independently of `MODEL` so a caller
/// that just wants to check `is_trained()` doesn't have to pay the ONNX
/// session commit (~tens of ms cold, plus the GPU sidecar's first-call
/// warmup of several seconds). Lets the v2 handler short-circuit on the
/// untrained-baseline sentinel without spinning up any inference tier.
static METADATA: OnceLock<Result<ModelMetadata, String>> = OnceLock::new();

/// Read + parse `dynamics_v2.metadata.json` once and cache it. Cheap —
/// just a small JSON file. Used by the v2 handler's short-circuit and
/// by any caller that wants the model's blake2b hash / training flag
/// without loading the ONNX runtime.
pub fn ensure_metadata() -> Result<ModelMetadata, String> {
    METADATA
        .get_or_init(|| {
            let meta_path = metadata_path();
            if !meta_path.exists() {
                return Err(format!(
                    "jepa_v2 metadata not found at {}; the .onnx and metadata \
                     sidecar ship together. Re-run the python export script.",
                    meta_path.display()
                ));
            }
            let text = std::fs::read_to_string(&meta_path)
                .map_err(|e| format!("read metadata: {e}"))?;
            serde_json::from_str::<ModelMetadata>(&text)
                .map_err(|e| format!("parse metadata: {e}"))
        })
        .clone()
}

/// True when the loaded model's metadata reports `training.trained ==
/// true`. Defaults to **false** when metadata is missing or malformed —
/// same fail-safe as `ModelMetadata::is_trained`. Used by the v2
/// handler to short-circuit ONNX/sidecar inference for the untrained
/// baseline (which by construction returns `last_input_vintage`).
pub fn is_trained() -> bool {
    ensure_metadata()
        .map(|m| m.is_trained())
        .unwrap_or(false)
}

/// Load the ONNX model + metadata. Idempotent — returns the cached
/// `Arc<LoadedModel>` on subsequent calls. The result is cached
/// regardless of success so a missing artifact always returns the
/// same error message; restarting the server is the only way to
/// re-attempt load (matches "no model offload" rule).
fn ensure_loaded() -> Result<Arc<LoadedModel>, String> {
    MODEL
        .get_or_init(|| {
            let onnx = onnx_path();
            if !onnx.exists() {
                return Err(format!(
                    "jepa_v2 artifact not found at {}; \
                     bootstrap with `python python/jepa_v2/export_baseline.py` \
                     for the untrained sentinel, or `python python/jepa_v2/train.py` \
                     after assembling Tessera vintages.",
                    onnx.display()
                ));
            }
            let meta_path = metadata_path();
            if !meta_path.exists() {
                return Err(format!(
                    "jepa_v2 metadata not found at {}; the .onnx and metadata \
                     sidecar ship together. Re-run the python export script.",
                    meta_path.display()
                ));
            }
            let meta_text =
                std::fs::read_to_string(&meta_path).map_err(|e| format!("read metadata: {e}"))?;
            let metadata: ModelMetadata =
                serde_json::from_str(&meta_text).map_err(|e| format!("parse metadata: {e}"))?;

            // Quick consistency check: the on-disk file size matches the
            // metadata. A mismatch means the .onnx and metadata fell out
            // of sync (e.g. someone re-ran train.py but didn't refresh
            // the sidecar). Fail fast rather than serve a fact whose
            // model_cid lies.
            let on_disk_bytes = std::fs::read(&onnx).map_err(|e| format!("read onnx: {e}"))?;
            if on_disk_bytes.len() as u64 != metadata.artifact.size_bytes {
                return Err(format!(
                    "jepa_v2 size mismatch: metadata says {} bytes but \
                     {} on disk is {} bytes. Re-run the export script.",
                    metadata.artifact.size_bytes,
                    onnx.display(),
                    on_disk_bytes.len()
                ));
            }

            // ort session, CPU EP — the model is small enough that GPU
            // is overkill and CPU keeps the path orthogonal to the
            // bigger V-JEPA 2 / Prithvi tiers (Phase 3+4) which need GPU.
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
                "jepa_v2 loaded"
            );

            Ok(Arc::new(LoadedModel {
                session: Arc::new(Mutex::new(session)),
                metadata,
            }))
        })
        .clone()
}

/// Public entry point: given K=3 prior 128-D Tessera vectors (oldest
/// first), return the predicted next-vintage 128-D vector.
///
/// `lags` must be exactly `INPUT_LAGS * TESSERA_DIM` long, laid out
/// as `[lag0_dim0, lag0_dim1, …, lag0_dim127, lag1_dim0, …]`.
pub fn predict_next_vintage(lags: &[f32]) -> Result<(Vec<f32>, ModelMetadata), String> {
    if lags.len() != INPUT_LAGS * TESSERA_DIM {
        return Err(format!(
            "jepa_v2.predict_next_vintage: lags must be {} f32 values \
             (3 lags × 128 dim flat); got {}",
            INPUT_LAGS * TESSERA_DIM,
            lags.len()
        ));
    }
    let model = ensure_loaded()?;
    let input = ort::value::Tensor::from_array(([1usize, INPUT_LAGS, TESSERA_DIM], lags.to_vec()))
        .map_err(|e| format!("ort input tensor: {e}"))?;
    let mut guard = model
        .session
        .lock()
        .map_err(|e| format!("session mutex: {e}"))?;
    let outputs = guard
        .run(ort::inputs!["lags" => input])
        .map_err(|e| format!("ort run: {e}"))?;
    let (_name, output) = outputs.iter().next().ok_or("ort returned no outputs")?;
    let arr = output
        .try_extract_array::<f32>()
        .map_err(|e| format!("extract output: {e}"))?;
    // Output shape is [1, 128]. Take batch row 0.
    let batch0 = arr.index_axis(ort_ndarray::Axis(0), 0);
    let v: Vec<f32> = batch0.iter().copied().collect();
    if v.len() != TESSERA_DIM {
        return Err(format!(
            "jepa_v2 model produced {} dims, expected {}",
            v.len(),
            TESSERA_DIM
        ));
    }
    Ok((v, model.metadata.clone()))
}

/// Fold the loaded model's metadata into the receipt's derivation
/// args. Caller is the /v1/jepa_predict_v2 handler; this exists in
/// the runtime module so the receipt shape stays in sync with the
/// model artifact's contract.
pub fn receipt_block(metadata: &ModelMetadata) -> JsonValue {
    serde_json::json!({
        "model_id": metadata.model_id,
        "version": metadata.version,
        "blake2b_hex": metadata.artifact.blake2b_hex,
        "size_bytes": metadata.artifact.size_bytes,
        "trained": metadata.is_trained(),
        "training": metadata.training,
        "validation": metadata.validation,
        "trained_at_iso": metadata.trained_at_iso,
        "honesty_warnings": if metadata.is_trained() {
            serde_json::Value::Array(Vec::new())
        } else {
            serde_json::Value::Array(vec![
                serde_json::Value::String(
                    "untrained_baseline: this jepa_v2 model is the residual-zero-init \
                     sentinel that returns last_input_vintage by construction. \
                     Quality is the 'predict last vintage' baseline, NOT a learned \
                     forecast."
                        .into(),
                ),
                serde_json::Value::String(
                    "upstream_geotessera_single_vintage: as of 2026-05-06 the public \
                     dl2.geotessera.org bucket only serves the 2024 vintage reliably \
                     (2017–2023 return null). A learned next-vintage predictor is \
                     therefore not trainable from this responder's data — the \
                     thesis requires K≥3 historical vintages per cell. The endpoint \
                     stays online for protocol surface stability, but its prediction \
                     is degenerate (all K lags collapse to the same 2024 vector → \
                     output equals 2024 vector). For real foundation embeddings, use \
                     the Phase-3 prithvi_eo2 band (when shipped) which runs over our \
                     S2 L2A path and is vintage-agnostic by construction."
                        .into(),
                ),
            ])
        },
    })
}

/// Map the inference module's Result into a `JEPA-shaped` `ApiError`.
/// Bridges to the rest of the REST surface; kept thin.
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

    /// `is_trained` defaults to FALSE when the `training.trained` field
    /// is absent — fail-safe so malformed metadata can't silently strip
    /// the `untrained_baseline` honesty warning from the receipt. Real
    /// train.py output MUST explicitly write `training.trained = true`.
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

    /// And the trained=true case: must set the field explicitly.
    #[test]
    fn metadata_is_trained_true_when_field_explicitly_true() {
        let m: ModelMetadata = serde_json::from_value(json!({
            "model_id": "jepa_temporal_predictor@2",
            "version": "0.0.1",
            "training": {"trained": true, "epochs": 200},
            "validation": {"cosine_similarity": 0.81},
            "artifact": {"filename": "dynamics_v2.onnx", "size_bytes": 100, "blake2b_hex": "00"},
        }))
        .expect("parse");
        assert!(m.is_trained());
    }

    #[test]
    fn metadata_is_trained_false_when_explicitly_false() {
        let m: ModelMetadata = serde_json::from_value(json!({
            "model_id": "jepa_temporal_predictor@2",
            "version": "0.0.0-untrained-baseline",
            "training": {"trained": false},
            "validation": {},
            "artifact": {"filename": "dynamics_v2.onnx", "size_bytes": 100, "blake2b_hex": "00"},
        }))
        .expect("parse");
        assert!(!m.is_trained());
    }

    /// Receipt block carries the model_cid + honesty_warnings.
    /// `untrained_baseline` MUST appear when training.trained is false
    /// — the receipt is the only place an LLM sees the warning, so the
    /// field shape is part of the wire contract.
    #[test]
    fn receipt_block_emits_untrained_warning() {
        let m: ModelMetadata = serde_json::from_value(json!({
            "model_id": "jepa_temporal_predictor@2",
            "version": "0.0.0-untrained-baseline",
            "training": {"trained": false},
            "validation": {},
            "artifact": {"filename": "dynamics_v2.onnx", "size_bytes": 8070, "blake2b_hex": "abc"},
        }))
        .expect("parse");
        let block = receipt_block(&m);
        let warnings = block
            .get("honesty_warnings")
            .and_then(|v| v.as_array())
            .expect("honesty_warnings is an array");
        // Two warnings now: untrained_baseline + upstream_geotessera_single_vintage.
        // The single-vintage caveat is permanent until upstream Tessera ships
        // multi-year data (see project_jepa_audit memory note).
        assert_eq!(warnings.len(), 2);
        assert!(warnings.iter().any(|w| w
            .as_str()
            .unwrap_or("")
            .contains("upstream_geotessera_single_vintage")));
        let w = warnings[0].as_str().unwrap();
        assert!(
            w.contains("untrained_baseline"),
            "warning must namespace as untrained_baseline; got {w}"
        );
        assert_eq!(block["trained"].as_bool(), Some(false));
        assert_eq!(block["blake2b_hex"].as_str(), Some("abc"));
    }

    #[test]
    fn receipt_block_no_warnings_when_trained() {
        let m: ModelMetadata = serde_json::from_value(json!({
            "model_id": "jepa_temporal_predictor@2",
            "version": "0.0.1",
            // `trained: true` MUST be explicit per the fail-safe default.
            "training": {"trained": true, "epochs": 200, "n_train_pairs": 1500},
            "validation": {"cosine_similarity": 0.81, "cosine_lift_vs_baseline": 0.05},
            "artifact": {"filename": "dynamics_v2.onnx", "size_bytes": 800000, "blake2b_hex": "deadbeef"},
        })).expect("parse");
        let block = receipt_block(&m);
        let warnings = block
            .get("honesty_warnings")
            .and_then(|v| v.as_array())
            .expect("honesty_warnings array present even when empty");
        assert!(warnings.is_empty());
        assert_eq!(block["trained"].as_bool(), Some(true));
    }

    /// `predict_next_vintage` rejects mis-shaped input fast — agents
    /// shouldn't be able to stumble past the shape contract and get
    /// arbitrary ort errors.
    #[test]
    fn predict_rejects_wrong_input_length() {
        // 3 lags × 128 dims = 384 floats.
        let too_short = vec![0.0_f32; 100];
        let err = predict_next_vintage(&too_short).expect_err("must reject short input");
        assert!(
            err.contains("384"),
            "error should cite expected size; got {err}"
        );
    }
}
