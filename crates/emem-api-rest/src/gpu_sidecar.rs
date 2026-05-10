//! Rust → Python GPU-sidecar HTTP/1 client over a Unix socket.
//!
//! Why this module exists: the in-process `ort` + CUDA path is broken
//! (libonnxruntime-1.22-cuda12 deadlocks at session create on this
//! host). All GPU inference therefore lives in
//! `python/jepa_v2_sidecar/server.py`, a FastAPI process that holds
//! the models in VRAM. This module is the Rust caller — sends
//! JSON over a Unix socket, parses the response.
//!
//! Protocol: HTTP/1.1 with `Connection: close` so we read the body
//! until EOF and don't have to manage chunked transfer encoding.
//! The sidecar is on localhost over UDS; transport is reliable; we
//! only need enough of HTTP/1 to round-trip a JSON payload.
//!
//! Configuration:
//!   - `EMEM_SIDECAR_SOCK` — Unix socket path. Default `/run/emem/jepa_sidecar.sock`.
//!   - `EMEM_SIDECAR_TIMEOUT_MS` — per-request timeout. Default 5_000 ms.
//!
//! Failure mode: when the sidecar is unreachable (socket missing,
//! connection refused, timeout), this module returns
//! `SidecarError::Unavailable`. Callers chain to a fallback (the
//! in-process CPU path for jepa_v2 dynamics) so the surface stays
//! up even when the GPU process is down.

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

const DEFAULT_TIMEOUT_MS: u64 = 5_000;

fn socket_path() -> PathBuf {
    PathBuf::from(
        std::env::var("EMEM_SIDECAR_SOCK").unwrap_or_else(|_| "/run/emem/jepa_sidecar.sock".into()),
    )
}

fn timeout() -> Duration {
    let ms = std::env::var("EMEM_SIDECAR_TIMEOUT_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_TIMEOUT_MS);
    Duration::from_millis(ms)
}

#[derive(Debug, thiserror::Error)]
pub enum SidecarError {
    /// Socket missing, refused, or otherwise unreachable. Callers
    /// chain to the in-process fallback on this variant.
    #[error("sidecar unavailable: {0}")]
    Unavailable(String),
    /// Request was sent and the sidecar responded with non-2xx.
    /// The sidecar is up; the request was rejected. Callers should
    /// surface this as 5xx, not silently fall back.
    #[error("sidecar status {status}: {body}")]
    Upstream { status: u16, body: String },
    /// Request timed out before the sidecar responded.
    #[error("sidecar timeout after {0:?}")]
    Timeout(Duration),
    /// Response decoding failed (bad framing, non-UTF-8 headers,
    /// malformed JSON body).
    #[error("sidecar protocol: {0}")]
    Protocol(String),
}

/// Request body for the dynamics_v2 endpoint. Mirrors the FastAPI
/// `DynamicsRequest` schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicsRequest {
    /// Three 128-D Tessera vintages, oldest first. Same shape contract
    /// as the in-process `jepa_v2::predict_next_vintage`.
    pub lags: Vec<Vec<f32>>,
}

/// Response body. Carries the prediction + the receipt-shape model
/// block the Rust caller forwards verbatim into the signed receipt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicsResponse {
    pub prediction: Vec<f32>,
    pub model: JsonValue,
    pub inference_us: u64,
    pub device: String,
}

/// Call the sidecar's `/predict/dynamics_v2` endpoint. Returns the
/// 128-D prediction + the model block.
pub async fn predict_dynamics_v2(req: &DynamicsRequest) -> Result<DynamicsResponse, SidecarError> {
    let body =
        serde_json::to_vec(req).map_err(|e| SidecarError::Protocol(format!("encode req: {e}")))?;
    let resp_bytes = post_json("/predict/dynamics_v2", &body).await?;
    serde_json::from_slice::<DynamicsResponse>(&resp_bytes)
        .map_err(|e| SidecarError::Protocol(format!("decode resp: {e}")))
}

/// Phase 3 — Prithvi-EO-2.0-300M-TL per-cell embedding request.
///
/// `chip` is a `[6, 224, 224]` reflectance window in HLS V2 band order
/// (Blue, Green, Red, Narrow NIR, SWIR1, SWIR2). The sidecar handles
/// per-band mean/std normalization. Optional `year` + `julian_day`
/// + `lng` + `lat` engage the model's temporal/location embeddings;
///   pass None to use the dropout-trained no-metadata path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrithviRequest {
    pub chip: Vec<Vec<Vec<f32>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub julian_day: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lng: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lat: Option<f64>,
}

/// Prithvi response — 1024-D CLS-token embedding from the encoder's
/// last block (post-norm). The `model` JSON object is the receipt-shape
/// block the Rust caller forwards verbatim into the signed receipt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrithviResponse {
    pub embedding: Vec<f32>,
    pub embedding_dim: usize,
    pub model: JsonValue,
    pub inference_us: u64,
    pub device: String,
}

/// Call the sidecar's `/predict/prithvi_eo2_embed` endpoint.
///
/// The first request after sidecar restart pays the model load cost
/// (~10 s — copying the 1.24 GB checkpoint to VRAM). Subsequent calls
/// are ~20 ms warm. Callers should treat this surface as best-effort
/// — when the sidecar is unavailable, recall on the `prithvi_eo2`
/// band returns existing attestations only (no in-process fallback;
/// CPU inference at ViT-L scale is not viable).
pub async fn predict_prithvi_eo2_embed(
    req: &PrithviRequest,
) -> Result<PrithviResponse, SidecarError> {
    let body =
        serde_json::to_vec(req).map_err(|e| SidecarError::Protocol(format!("encode req: {e}")))?;
    let resp_bytes = post_json("/predict/prithvi_eo2_embed", &body).await?;
    serde_json::from_slice::<PrithviResponse>(&resp_bytes)
        .map_err(|e| SidecarError::Protocol(format!("decode resp: {e}")))
}

/// Phase 4 — Galileo S2-only embedding request.
///
/// Variant (tiny | base | nano) is determined by the sidecar's
/// `EMEM_GALILEO_VARIANT` at startup; embedding dimension follows
/// (Tiny=192, Base=768). `s2_chip` is `[T=1, H=8, W=8, 10]`
/// reflectance in Galileo's S2_BANDS order: B2, B3, B4, B5, B6, B7,
/// B8, B8A, B11, B12. Native scale (0–10000); the sidecar normalizes
/// against Galileo's pretraining stats. `month` is 1..12 (defaults
/// July if absent), engages the model's seasonal positional encoding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GalileoRequest {
    pub s2_chip: Vec<Vec<Vec<Vec<f32>>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub month: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lng: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lat: Option<f64>,
}

/// Galileo response — average-pooled embedding from the encoder.
/// Dimension depends on variant (Tiny=192, Base=768); read it from
/// `embedding_dim` or `model.config.embedding_size`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GalileoResponse {
    pub embedding: Vec<f32>,
    pub embedding_dim: usize,
    pub model: JsonValue,
    pub inference_us: u64,
    pub device: String,
}

/// Call the sidecar's `/predict/galileo_embed` endpoint (variant-agnostic).
///
/// First request after sidecar restart pays the model load cost
/// (~4 s for Tiny / 22 MB; ~5 s for Base / 330 MB). Warm calls
/// ~14 ms (Tiny) / ~25 ms (Base). As with Prithvi, no in-process CPU
/// fallback at ViT scale.
pub async fn predict_galileo_embed(req: &GalileoRequest) -> Result<GalileoResponse, SidecarError> {
    let body =
        serde_json::to_vec(req).map_err(|e| SidecarError::Protocol(format!("encode req: {e}")))?;
    let resp_bytes = post_json("/predict/galileo_embed", &body).await?;
    serde_json::from_slice::<GalileoResponse>(&resp_bytes)
        .map_err(|e| SidecarError::Protocol(format!("decode resp: {e}")))
}

/// Phase 5 — Clay Foundation Model v1.5 per-cell embedding request.
///
/// `chip` is `[10, 256, 256]` reflectance in S2 L2A band order
/// (blue, green, red, rededge1, rededge2, rededge3, nir, nir08,
/// swir16, swir22), raw S2 L2A scaled (0..10000). The sidecar
/// normalizes via the model's per-band mean/std. `year`+`month`
/// engage the temporal encoder (sin/cos week-of-year + hour);
/// `lng`+`lat` engage the spatial encoder (sin/cos lat/lon).
/// Skipping any of those falls back to mid-year-noon and
/// origin-coords respectively — Clay was trained with conditioning
/// dropout so a no-metadata path exists, but quality is best with
/// real values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClayRequest {
    pub chip: Vec<Vec<Vec<f32>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub month: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub day: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lng: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lat: Option<f64>,
}

/// Clay v1.5 response — 1024-D CLS embedding from the encoder's
/// last block. The `model` JSON object is the receipt-shape block
/// the Rust caller forwards verbatim into the signed receipt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClayResponse {
    pub embedding: Vec<f32>,
    pub embedding_dim: usize,
    pub model: JsonValue,
    pub inference_us: u64,
    pub device: String,
}

/// Call the sidecar's `/predict/clay_embed` endpoint.
///
/// The first request after sidecar restart pays the model load cost
/// (~6-10 s — copying the 1.25 GB encoder weights to VRAM and
/// dropping the 304 MB DINOv2 teacher). Subsequent calls are
/// ~25-40 ms warm at fp16 / 70-120 ms at fp32. As with Prithvi and
/// Galileo, there is NO in-process CPU fallback: a CPU pass through
/// Clay's ViT-L/8 takes 3-8 s per chip and would change the
/// embedding's distribution (different kernel order accumulation).
/// Callers should treat `SidecarError::Unavailable` as "Clay band
/// is not available at this responder; sign Absence with
/// `gpu_unavailable` reason and let the agent route elsewhere."
pub async fn predict_clay_embed(req: &ClayRequest) -> Result<ClayResponse, SidecarError> {
    let body =
        serde_json::to_vec(req).map_err(|e| SidecarError::Protocol(format!("encode req: {e}")))?;
    let resp_bytes = post_json("/predict/clay_embed", &body).await?;
    serde_json::from_slice::<ClayResponse>(&resp_bytes)
        .map_err(|e| SidecarError::Protocol(format!("decode resp: {e}")))
}

/// Capability snapshot of the sidecar at last poll. Mirrors the
/// fields agents downstream actually query.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SidecarHealth {
    pub live: bool,
    pub ready: bool,
    pub cuda_available: bool,
    pub extensions: Vec<String>,
    pub models_loaded: Vec<String>,
}

/// Query the sidecar's `/health` endpoint and parse the consensus
/// shape (live / ready / extensions). Returns `Err(Unavailable)`
/// when the sidecar isn't reachable so callers can treat that as a
/// negative capability advertisement (no extensions, no GPU).
pub async fn health() -> Result<SidecarHealth, SidecarError> {
    let req = "GET /health HTTP/1.1\r\n\
               host: localhost\r\n\
               accept: application/json\r\n\
               connection: close\r\n\r\n";
    let resp_bytes = round_trip(req.as_bytes(), &[]).await?;
    let v: JsonValue = serde_json::from_slice(&resp_bytes)
        .map_err(|e| SidecarError::Protocol(format!("decode /health: {e}")))?;
    let cuda_available = v
        .get("cuda")
        .and_then(|c| c.get("available"))
        .and_then(|b| b.as_bool())
        .unwrap_or(false);
    let extensions = v
        .get("extensions")
        .and_then(|e| e.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let models_loaded = v
        .get("models_loaded")
        .and_then(|m| m.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(SidecarHealth {
        live: v.get("live").and_then(|b| b.as_bool()).unwrap_or(true),
        ready: v.get("ready").and_then(|b| b.as_bool()).unwrap_or(true),
        cuda_available,
        extensions,
        models_loaded,
    })
}

/// True when the sidecar advertises `gpu` in its `extensions[]`.
/// Used as the gate for algorithms that declare
/// `inference.tier = gpu` so the dispatcher can filter them at
/// planning time and skip honest Absence at materialize time when
/// the GPU is missing.
pub async fn is_gpu_available() -> bool {
    matches!(
        health().await,
        Ok(h) if h.cuda_available && h.extensions.iter().any(|e| e == "gpu")
    )
}

// ── HTTP/1 over Unix socket ──────────────────────────────────────────────

async fn post_json(path: &str, body: &[u8]) -> Result<Vec<u8>, SidecarError> {
    let req = format!(
        "POST {path} HTTP/1.1\r\n\
         host: localhost\r\n\
         content-type: application/json\r\n\
         content-length: {len}\r\n\
         connection: close\r\n\
         \r\n",
        len = body.len()
    );
    round_trip(req.as_bytes(), body).await
}

async fn round_trip(headers: &[u8], body: &[u8]) -> Result<Vec<u8>, SidecarError> {
    let sock = socket_path();
    let to = timeout();
    let result = tokio::time::timeout(to, async {
        let mut stream = UnixStream::connect(&sock)
            .await
            .map_err(|e| SidecarError::Unavailable(format!("connect {sock:?}: {e}")))?;
        stream
            .write_all(headers)
            .await
            .map_err(|e| SidecarError::Protocol(format!("write headers: {e}")))?;
        if !body.is_empty() {
            stream
                .write_all(body)
                .await
                .map_err(|e| SidecarError::Protocol(format!("write body: {e}")))?;
        }
        // Connection: close → read until EOF.
        let mut raw = Vec::with_capacity(8192);
        stream
            .read_to_end(&mut raw)
            .await
            .map_err(|e| SidecarError::Protocol(format!("read response: {e}")))?;
        Ok::<_, SidecarError>(raw)
    })
    .await;
    let raw = match result {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => return Err(e),
        Err(_) => return Err(SidecarError::Timeout(to)),
    };
    parse_http_response(&raw)
}

/// Minimal HTTP/1 response parser. We only need:
///   - the status code (status-line first space-separated token at index 1)
///   - the body (everything after the first `\r\n\r\n`)
///
/// The sidecar speaks `Connection: close` so chunked encoding is not
/// in play; we never see `Transfer-Encoding: chunked`.
fn parse_http_response(raw: &[u8]) -> Result<Vec<u8>, SidecarError> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| SidecarError::Protocol("no header/body delimiter".into()))?;
    let head = &raw[..split];
    let body = &raw[split + 4..];

    // Status line: "HTTP/1.1 NNN reason\r\n..."
    let status_line = head
        .split(|b| *b == b'\r' || *b == b'\n')
        .next()
        .ok_or_else(|| SidecarError::Protocol("empty headers".into()))?;
    let status_str =
        std::str::from_utf8(status_line).map_err(|e| SidecarError::Protocol(e.to_string()))?;
    let mut parts = status_str.split_whitespace();
    let _http_version = parts.next();
    let status: u16 = parts
        .next()
        .ok_or_else(|| SidecarError::Protocol("no status code".into()))?
        .parse()
        .map_err(|_| SidecarError::Protocol("status not a number".into()))?;

    if !(200..300).contains(&status) {
        let body_str = String::from_utf8_lossy(body).into_owned();
        return Err(SidecarError::Upstream {
            status,
            body: body_str,
        });
    }
    Ok(body.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tiny HTTP/1.1 fixture exercises the parser without spinning a
    /// real sidecar. Status, header/body delimiter, body verbatim.
    #[test]
    fn parse_response_ok_extracts_body() {
        let raw = b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\r\n{\"hello\":1}";
        let body = parse_http_response(raw).unwrap();
        assert_eq!(body, b"{\"hello\":1}");
    }

    #[test]
    fn parse_response_non_2xx_surfaces_status_and_body() {
        let raw = b"HTTP/1.1 503 Service Unavailable\r\n\r\nmodel not loaded yet";
        let err = parse_http_response(raw).unwrap_err();
        match err {
            SidecarError::Upstream { status, body } => {
                assert_eq!(status, 503);
                assert!(body.contains("model not loaded"));
            }
            other => panic!("expected Upstream(503), got {other:?}"),
        }
    }

    #[test]
    fn parse_response_no_delimiter_is_protocol_error() {
        let raw = b"garbage with no headers";
        let err = parse_http_response(raw).unwrap_err();
        assert!(matches!(err, SidecarError::Protocol(_)));
    }

    /// Live round-trip against a running sidecar. Requires:
    ///   `EMEM_SIDECAR_SOCK` set to a reachable socket
    /// Run explicitly: `cargo test -p emem-api-rest gpu_sidecar -- --ignored`
    #[tokio::test]
    #[ignore]
    async fn live_predict_dynamics_v2_returns_128d() {
        let req = DynamicsRequest {
            lags: vec![vec![0.0_f32; 128]; 3],
        };
        let resp = predict_dynamics_v2(&req).await.expect("sidecar reachable");
        assert_eq!(resp.prediction.len(), 128, "v2 output is 128-D");
        // Sentinel returns last_input_vintage by construction; with
        // all-zeros input that is all-zeros output.
        assert!(
            resp.prediction.iter().all(|x| x.abs() < 1e-6),
            "zero-init sentinel must return zeros for zero input; got non-zero"
        );
        // Receipt-shape model block carries the via tag.
        assert_eq!(
            resp.model.get("via").and_then(|v| v.as_str()),
            Some("python_sidecar")
        );
        assert!(resp.device.starts_with("cuda") || resp.device == "cpu");
    }

    /// Live Prithvi-EO-2.0 round-trip. Requires the sidecar at
    /// `EMEM_SIDECAR_SOCK` AND the local snapshot at
    /// `EMEM_PRITHVI_SNAPSHOT` (defaults to the path under
    /// `EMEM_DATA/hf_cache/...`). Run explicitly with `--ignored`.
    /// First call after sidecar restart costs ~10 s (loading the
    /// 1.24 GB checkpoint into VRAM); warm calls are ~20 ms.
    #[tokio::test]
    #[ignore]
    async fn live_predict_prithvi_eo2_returns_1024d() {
        // Synthetic chip: 6 bands × 224×224 of mid-range reflectance
        // values. The sidecar normalizes with the model's mean/std.
        let chip: Vec<Vec<Vec<f32>>> = (0..6)
            .map(|_| (0..224).map(|_| vec![3500.0_f32; 224]).collect())
            .collect();
        let req = PrithviRequest {
            chip,
            year: Some(2024),
            julian_day: Some(200),
            lng: Some(-73.98),
            lat: Some(40.76),
        };
        let resp = predict_prithvi_eo2_embed(&req)
            .await
            .expect("sidecar reachable + Prithvi snapshot present");
        assert_eq!(resp.embedding_dim, 1024, "Prithvi-EO-2.0 ViT-L is 1024-D");
        assert_eq!(resp.embedding.len(), 1024);
        let l2 = resp.embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            l2 > 0.1 && l2 < 1000.0,
            "embedding L2 should be sane; got {l2}"
        );
        assert_eq!(
            resp.model.get("via").and_then(|v| v.as_str()),
            Some("python_sidecar")
        );
        assert_eq!(
            resp.model.get("model_id").and_then(|v| v.as_str()),
            Some("prithvi_eo_v2_300m_tl")
        );
    }

    /// Connecting to a non-existent socket surfaces Unavailable
    /// (the variant that triggers in-process fallback).
    #[tokio::test]
    async fn missing_socket_returns_unavailable() {
        std::env::set_var("EMEM_SIDECAR_SOCK", "/tmp/this-socket-does-not-exist.sock");
        let err = post_json("/predict/dynamics_v2", b"{}").await.unwrap_err();
        match err {
            SidecarError::Unavailable(msg) => {
                assert!(
                    msg.contains("connect"),
                    "Unavailable message should mention connect; got {msg}"
                );
            }
            other => panic!("expected Unavailable, got {other:?}"),
        }
        std::env::remove_var("EMEM_SIDECAR_SOCK");
    }
}
