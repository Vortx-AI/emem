//! The verification engine: the single decision point that admits or
//! rejects a device's output against its substrate profile.
//!
//! The verifier collects **every** failure rather than stopping at the
//! first, because a device maker debugging an enrollment needs the full
//! list, and an auditor needs to see whether a rejection was one bad
//! clock or a systematically forged chain. The verdict is Admit only
//! when the reason list is empty.

use ed25519_dalek::VerifyingKey;
use serde::{Deserialize, Serialize};

use emem_core::substrates::{AdmissionRule, SubstrateProfile, TraceLayerKind};

use crate::schema::{decode_digest, render_digest, OsTrace, OS_TRACE_SCHEMA_V1};

/// Why a trace failed verification. Every variant carries enough to
/// point at the offending part of the record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum RejectReason {
    /// `schema` field is not `emem.os_trace.v1`.
    #[error("unknown trace schema: {schema}")]
    SchemaMismatch {
        /// The schema string the record carried.
        schema: String,
    },
    /// The trace names a different profile than the one verified against.
    #[error("trace claims profile {claimed}, verified against {expected}")]
    ProfileMismatch {
        /// Profile the device claimed.
        claimed: String,
        /// Profile the verifier was asked to enforce.
        expected: String,
    },
    /// The profile's admission rule is not trace-based; device output
    /// cannot enter through this path at all.
    #[error("profile {profile} is not admitted by OS trace")]
    AdmissionNotTraceBased {
        /// The profile in question.
        profile: String,
    },
    /// The capture window is empty or inverted.
    #[error("capture window start must precede end")]
    WindowEmpty,
    /// No segments: no evidence.
    #[error("trace has no segments")]
    EmptyTrace,
    /// A required trace layer is absent.
    #[error("required trace layer missing: {layer:?}")]
    MissingLayer {
        /// The absent layer.
        layer: TraceLayerKind,
    },
    /// Segment `seq` values are not contiguous from 0 in order.
    #[error("segment at position {position} has seq {seq}")]
    SeqOutOfOrder {
        /// Index in the segments vector.
        position: usize,
        /// The seq value found there.
        seq: u64,
    },
    /// A segment's `prev_digest` does not match the digest of the
    /// preceding segment: the chain was cut or rewritten.
    #[error("chain broken at seq {seq}")]
    ChainBroken {
        /// Sequence number where the chain fails.
        seq: u64,
    },
    /// A segment's clock runs backwards.
    #[error("segment {seq} clock is non-monotonic")]
    ClockNonMonotonic {
        /// Sequence number of the offending segment.
        seq: u64,
    },
    /// A segment claims time outside the trace window.
    #[error("segment {seq} lies outside the capture window")]
    ClockOutsideWindow {
        /// Sequence number of the offending segment.
        seq: u64,
    },
    /// Two segments hash identically — the duplicate-leaf equivocation
    /// v1 merkle verifiers must reject.
    #[error("duplicate segment digest at seq {seq}")]
    DuplicateSegmentDigest {
        /// Sequence number of the second occurrence.
        seq: u64,
    },
    /// The recorded `trace_root` does not match the recomputed root.
    #[error("trace_root does not match the segment chain")]
    RootMismatch,
    /// An output claims emission outside the window.
    #[error("output {index} emitted outside the capture window")]
    OutputOutsideWindow {
        /// Index into the outputs vector.
        index: usize,
    },
    /// The payload the write path is trying to admit is not bound in
    /// the trace: the device produced bytes its execution never emitted.
    #[error("claimed payload digest is not bound in the trace")]
    OutputUnbound {
        /// The digest the write path claimed.
        payload_digest: String,
    },
    /// `prev_trace_cid` is present but not a well-formed trace CID. The
    /// verifier cannot check the predecessor exists (it holds no store);
    /// the write gate enforces chain continuity. This only checks shape.
    #[error("prev_trace_cid is not a well-formed trace CID")]
    PrevTraceMalformed,
    /// The device signature over the preimage does not verify.
    #[error("device signature invalid")]
    SignatureInvalid,
    /// The record could not be canonically encoded for digesting.
    #[error("canonical encoding failed: {detail}")]
    EncodingFailed {
        /// Underlying encoder error.
        detail: String,
    },
}

/// Layer coverage summary: what the profile demanded, what the trace
/// presented, and the difference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Coverage {
    /// Layers the substrate profile requires.
    pub required: Vec<TraceLayerKind>,
    /// Layers present in the trace.
    pub present: Vec<TraceLayerKind>,
    /// Required layers that are absent.
    pub missing: Vec<TraceLayerKind>,
}

/// Admit or reject.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Every check passed; output bound in this trace may be written.
    Admit,
    /// At least one check failed; the output must not be written.
    Reject,
}

/// The full verification outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationReport {
    /// Admit only when `reasons` is empty.
    pub verdict: Verdict,
    /// Every failed check.
    pub reasons: Vec<RejectReason>,
    /// Layer coverage, reported on success and failure alike.
    pub coverage: Coverage,
    /// Content ID of the trace record, when it could be computed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_cid: Option<String>,
}

/// Verify one trace against one substrate profile, optionally checking
/// that a specific claimed payload digest is bound inside it.
///
/// `claimed_payload_digest` is what the write path passes when a device
/// submits a fact: the fact's payload digest must appear among the
/// trace's emitted outputs, or the fact is rejected even though the
/// trace itself is sound.
pub fn verify_os_trace(
    trace: &OsTrace,
    profile: &SubstrateProfile,
    claimed_payload_digest: Option<&str>,
) -> VerificationReport {
    let mut reasons: Vec<RejectReason> = Vec::new();

    if trace.schema != OS_TRACE_SCHEMA_V1 {
        reasons.push(RejectReason::SchemaMismatch {
            schema: trace.schema.clone(),
        });
    }
    if trace.device.substrate_profile != profile.id {
        reasons.push(RejectReason::ProfileMismatch {
            claimed: trace.device.substrate_profile.clone(),
            expected: profile.id.clone(),
        });
    }
    if profile.admission != AdmissionRule::OsTraceRequired {
        reasons.push(RejectReason::AdmissionNotTraceBased {
            profile: profile.id.clone(),
        });
    }
    if trace.window_start_ns >= trace.window_end_ns {
        reasons.push(RejectReason::WindowEmpty);
    }
    if trace.segments.is_empty() {
        reasons.push(RejectReason::EmptyTrace);
    }

    // Coverage: every required layer must be present.
    let mut present: Vec<TraceLayerKind> = trace.segments.iter().map(|s| s.layer).collect();
    present.sort();
    present.dedup();
    let missing: Vec<TraceLayerKind> = profile
        .required_trace_layers
        .iter()
        .copied()
        .filter(|l| !present.contains(l))
        .collect();
    for layer in &missing {
        reasons.push(RejectReason::MissingLayer { layer: *layer });
    }
    let coverage = Coverage {
        required: profile.required_trace_layers.clone(),
        present,
        missing,
    };

    // Chain, clocks, and root.
    let mut digests: Vec<[u8; 32]> = Vec::with_capacity(trace.segments.len());
    // A set beside the vector, because the vector is needed in chain order for
    // the merkle root and the duplicate check is not a linear scan.
    //
    // It was. `digests.contains(&d)` inside the segment loop made verification
    // quadratic, and this runs on an unauthenticated write path with a 16 MB
    // body limit: 50,000 segments in 11.9 MB cost 1.09 s of CPU, against 2 ms
    // for 1,000. Roughly 50 KB of request per second of CPU is an
    // amplification worth closing, on a machine that has wedged before.
    let mut seen_digests: std::collections::HashSet<[u8; 32]> =
        std::collections::HashSet::with_capacity(trace.segments.len());
    let mut prev_rendered: Option<String> = None;
    let mut chain_intact = true;
    for (i, seg) in trace.segments.iter().enumerate() {
        if seg.seq != i as u64 {
            reasons.push(RejectReason::SeqOutOfOrder {
                position: i,
                seq: seg.seq,
            });
        }
        if seg.prev_digest != prev_rendered {
            reasons.push(RejectReason::ChainBroken { seq: seg.seq });
            chain_intact = false;
        }
        if seg.clock_start_ns > seg.clock_end_ns {
            reasons.push(RejectReason::ClockNonMonotonic { seq: seg.seq });
        }
        if seg.clock_start_ns < trace.window_start_ns || seg.clock_end_ns > trace.window_end_ns {
            reasons.push(RejectReason::ClockOutsideWindow { seq: seg.seq });
        }
        match seg.digest() {
            Ok(d) => {
                if !seen_digests.insert(d) {
                    reasons.push(RejectReason::DuplicateSegmentDigest { seq: seg.seq });
                }
                prev_rendered = Some(render_digest(&d));
                digests.push(d);
            }
            Err(e) => {
                reasons.push(RejectReason::EncodingFailed {
                    detail: e.to_string(),
                });
                chain_intact = false;
            }
        }
    }
    if chain_intact && !digests.is_empty() {
        let recomputed = emem_attest::merkle_root_v1(&digests);
        match decode_digest(&trace.trace_root) {
            Some(recorded) if recorded == recomputed => {}
            _ => reasons.push(RejectReason::RootMismatch),
        }
    }

    // Outputs.
    for (i, out) in trace.outputs.iter().enumerate() {
        if out.emitted_at_ns < trace.window_start_ns || out.emitted_at_ns > trace.window_end_ns {
            reasons.push(RejectReason::OutputOutsideWindow { index: i });
        }
    }
    if let Some(claimed) = claimed_payload_digest {
        if !trace.outputs.iter().any(|o| o.payload_digest == claimed) {
            reasons.push(RejectReason::OutputUnbound {
                payload_digest: claimed.to_string(),
            });
        }
    }

    // Stream link: shape-only. A present prev_trace_cid must be a valid
    // 52-char base32 digest; continuity (does this predecessor exist, is it
    // this device's latest) is the write gate's job.
    if let Some(prev) = &trace.prev_trace_cid {
        if decode_digest(prev).is_none() {
            reasons.push(RejectReason::PrevTraceMalformed);
        }
    }

    // Device signature over the preimage.
    match trace.preimage() {
        Ok(preimage) => match VerifyingKey::from_bytes(&trace.device.device_key.0) {
            Ok(vk) => {
                let sig = ed25519_dalek::Signature::from_bytes(&trace.signature.0);
                // verify_strict: reject malleable signatures — a malleated
                // sig would verify yet change trace_cid (blake3 over the
                // CBOR including the signature), minting a distinct token.
                if vk.verify_strict(&preimage, &sig).is_err() {
                    reasons.push(RejectReason::SignatureInvalid);
                }
            }
            Err(_) => reasons.push(RejectReason::SignatureInvalid),
        },
        Err(e) => reasons.push(RejectReason::EncodingFailed {
            detail: e.to_string(),
        }),
    }

    let verdict = if reasons.is_empty() {
        Verdict::Admit
    } else {
        Verdict::Reject
    };
    VerificationReport {
        verdict,
        reasons,
        coverage,
        trace_cid: trace.trace_cid().ok(),
    }
}
