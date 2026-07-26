//! The `emem.os_trace.v1` record: device identity, chained trace
//! segments, emitted outputs, and the device signature over all of it.
//!
//! Canonical encoding follows the emem-CBOR profile: serde-derived
//! structs serialize fields in declaration order through ciborium, so
//! two implementations produce byte-identical CBOR for the same record,
//! and every digest below is a function of those canonical bytes.

use data_encoding::BASE32_NOPAD;
use ed25519_dalek::Signer;
use serde::{Deserialize, Serialize};

use emem_core::key::{AttesterKey, KeyEpoch, Signature};
use emem_core::substrates::TraceLayerKind;

/// Schema identifier pinned in every trace record.
pub const OS_TRACE_SCHEMA_V1: &str = "emem.os_trace.v1";

/// Errors constructing a trace record.
#[derive(Debug, thiserror::Error)]
pub enum TraceBuildError {
    /// CBOR serialization failed (should not happen for well-formed input).
    #[error("canonical CBOR encoding failed: {0}")]
    Encode(String),
    /// A trace with no segments carries no evidence.
    #[error("a trace must contain at least one segment")]
    Empty,
    /// The capture window is empty or inverted.
    #[error("capture window must satisfy start < end")]
    BadWindow,
}

/// Who is tracing: one physical device, one key, one boot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceIdentity {
    /// The device's ed25519 public key — the same key type any attester
    /// uses, so a device is a first-class writer, not a special case.
    pub device_key: AttesterKey,
    /// Key rotation epoch.
    pub key_epoch: KeyEpoch,
    /// Substrate profile ID the device writes under
    /// (e.g. `"robot.fleet.v1"` in the substrates manifest).
    pub substrate_profile: String,
    /// Hardware platform string (e.g. `"jetson-orin-nx"`).
    pub platform: String,
    /// Operating system name and version.
    pub os: String,
    /// Kernel identity string.
    pub kernel: String,
    /// Per-boot identifier, fresh at every boot, so two traces from the
    /// same boot are linkable and a replayed trace from an old boot is
    /// not silently current.
    pub boot_id: String,
}

impl DeviceIdentity {
    /// blake3 of the canonical CBOR of this identity — the DEVICE
    /// segment of the signed preimage.
    pub fn digest(&self) -> Result<[u8; 32], TraceBuildError> {
        canonical_digest(self)
    }
}

/// One layer's capture over (part of) the window: not the raw log bytes
/// themselves, which stay on device or in cold storage, but their
/// digest, extent, and position in the chain. The record commits to the
/// bytes; the bytes are producible on audit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceSegment {
    /// Which trace layer this segment captures.
    pub layer: TraceLayerKind,
    /// Position in the chain, assigned contiguously from 0.
    pub seq: u64,
    /// Monotonic clock at segment start, nanoseconds.
    pub clock_start_ns: u64,
    /// Monotonic clock at segment end, nanoseconds.
    pub clock_end_ns: u64,
    /// Number of events the raw log holds.
    pub event_count: u64,
    /// blake3 of the raw captured log bytes, base32-nopad lowercase
    /// (the same rendering as every other emem digest).
    pub log_digest: String,
    /// Digest of the previous segment record; `None` only at `seq` 0.
    /// This is the chain that makes dropping or rewriting a middle
    /// segment detectable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev_digest: Option<String>,
    /// Capture encoding (e.g. `"linux.ftrace.v1"`, `"linux.ebpf.raw"`,
    /// `"ros2.bag.v2"`, `"zephyr.ctf.v1"`).
    pub encoding: String,
}

impl TraceSegment {
    /// blake3 of the canonical CBOR of this segment record (including
    /// `prev_digest`, which is what makes the digests a chain).
    pub fn digest(&self) -> Result<[u8; 32], TraceBuildError> {
        canonical_digest(self)
    }
}

/// One sensor payload the device emitted during the window and wants
/// admitted as a fact. The payload bytes travel in the write envelope;
/// here only their digest is bound into the signed trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmittedOutput {
    /// blake3 of the payload bytes, base32-nopad lowercase.
    pub payload_digest: String,
    /// Band the payload is destined for, when known at capture time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub band: Option<String>,
    /// Monotonic clock at emission, nanoseconds. Must fall inside the
    /// trace window.
    pub emitted_at_ns: u64,
    /// The layer whose log records the emission (usually
    /// [`TraceLayerKind::SensorBus`]).
    pub layer: TraceLayerKind,
}

/// The complete signed trace record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OsTrace {
    /// MUST equal [`OS_TRACE_SCHEMA_V1`].
    pub schema: String,
    /// The tracing device.
    pub device: DeviceIdentity,
    /// Capture window start, monotonic nanoseconds.
    pub window_start_ns: u64,
    /// Capture window end, monotonic nanoseconds.
    pub window_end_ns: u64,
    /// The chained segments, in chain order.
    pub segments: Vec<TraceSegment>,
    /// The outputs emitted inside the window.
    pub outputs: Vec<EmittedOutput>,
    /// v1 merkle root over the segment digests in chain order,
    /// base32-nopad lowercase.
    pub trace_root: String,
    /// ed25519 signature by `device.device_key` over
    /// [`emem_attest::os_trace_preimage_v1`].
    pub signature: Signature,
}

impl OsTrace {
    /// Compute the v1 merkle root over segment digests in chain order.
    /// Chain order (not sorted order) is canonical here: the chain is
    /// the evidence, and `prev_digest` already forbids reordering.
    pub fn compute_trace_root(segments: &[TraceSegment]) -> Result<[u8; 32], TraceBuildError> {
        let leaves: Vec<[u8; 32]> = segments
            .iter()
            .map(|s| s.digest())
            .collect::<Result<_, _>>()?;
        Ok(emem_attest::merkle_root_v1(&leaves))
    }

    /// The 32 bytes the device key signs.
    pub fn preimage(&self) -> Result<[u8; 32], TraceBuildError> {
        let device_digest = self.device.digest()?;
        let root = decode_digest(&self.trace_root)
            .ok_or_else(|| TraceBuildError::Encode("trace_root is not a digest".into()))?;
        Ok(emem_attest::os_trace_preimage_v1(
            &self.schema,
            &device_digest,
            &self.device.substrate_profile,
            self.window_start_ns,
            self.window_end_ns,
            &root,
            self.outputs.iter().map(|o| o.payload_digest.as_str()),
        ))
    }

    /// Content ID of the whole signed record:
    /// `base32(blake3(canonical_cbor(trace)))`, the same rule facts use.
    pub fn trace_cid(&self) -> Result<String, TraceBuildError> {
        Ok(render_digest(&canonical_digest(self)?))
    }

    /// Build and sign a v1 trace. Segments are taken in capture order;
    /// this assigns `seq` and links `prev_digest`, computes the root,
    /// and signs the preimage with the device's secret key. The caller
    /// provides segments with `seq: 0` and `prev_digest: None`; both
    /// are overwritten.
    pub fn build_and_sign_v1(
        device: DeviceIdentity,
        window_start_ns: u64,
        window_end_ns: u64,
        mut segments: Vec<TraceSegment>,
        outputs: Vec<EmittedOutput>,
        signing: &ed25519_dalek::SigningKey,
    ) -> Result<Self, TraceBuildError> {
        if segments.is_empty() {
            return Err(TraceBuildError::Empty);
        }
        if window_start_ns >= window_end_ns {
            return Err(TraceBuildError::BadWindow);
        }
        let mut prev: Option<String> = None;
        for (i, seg) in segments.iter_mut().enumerate() {
            seg.seq = i as u64;
            seg.prev_digest = prev.take();
            prev = Some(render_digest(&seg.digest()?));
        }
        let root = Self::compute_trace_root(&segments)?;
        let mut trace = OsTrace {
            schema: OS_TRACE_SCHEMA_V1.to_string(),
            device,
            window_start_ns,
            window_end_ns,
            segments,
            outputs,
            trace_root: render_digest(&root),
            signature: Signature([0u8; 64]),
        };
        let preimage = trace.preimage()?;
        trace.signature = Signature(signing.sign(&preimage).to_bytes());
        Ok(trace)
    }
}

/// The payload-digest rule the write gate uses to bind a fact to a
/// trace: `base32(blake3(canonical_cbor(value)))`, over the fact's
/// already-float-canonicalized CBOR value. The device computes this
/// over the same bytes it is about to attest, lists it as an
/// [`EmittedOutput::payload_digest`], and the gate recomputes it from
/// the submitted fact. One rule, both sides.
pub fn payload_digest_of_value(value: &ciborium::Value) -> Result<String, TraceBuildError> {
    Ok(render_digest(&canonical_digest(value)?))
}

/// blake3 of the canonical CBOR of any serializable record.
fn canonical_digest<T: Serialize>(v: &T) -> Result<[u8; 32], TraceBuildError> {
    let mut buf = Vec::new();
    ciborium::ser::into_writer(v, &mut buf).map_err(|e| TraceBuildError::Encode(e.to_string()))?;
    Ok(*blake3::hash(&buf).as_bytes())
}

/// Render a 32-byte digest the way every emem CID renders: base32
/// no-pad, lowercase.
pub(crate) fn render_digest(d: &[u8; 32]) -> String {
    BASE32_NOPAD.encode(d).to_lowercase()
}

/// Parse a rendered digest back to 32 bytes; `None` if malformed.
pub(crate) fn decode_digest(s: &str) -> Option<[u8; 32]> {
    let bytes = BASE32_NOPAD.decode(s.to_uppercase().as_bytes()).ok()?;
    bytes.try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_render_round_trips() {
        let d = *blake3::hash(b"segment bytes").as_bytes();
        assert_eq!(decode_digest(&render_digest(&d)), Some(d));
    }
}
