//! The write-path trace gate — where "emem rejects output without
//! verified execution" becomes enforcement rather than doctrine.
//!
//! Three sled trees:
//!
//! - `emem.device_enrollment`: attester pubkey (base32) to substrate
//!   profile ID. Enrolling a key opts it into, and locks it to, the
//!   trace-gated write path. Keys never enrolled are untouched: the
//!   founding archive writers and every existing attester keep the
//!   plain `put_attestation` semantics, so the gate is migration-safe
//!   by construction.
//! - `emem.os_traces`: trace_cid to the canonical CBOR of the admitted
//!   [`OsTrace`] record, so `emem:trace:` tokens resolve to
//!   byte-identical evidence later.
//! - `emem.fact_trace`: fact_cid to trace_cid, the audit edge from any
//!   admitted device fact back to the execution that produced it.
//!
//! The gate's decision logic is [`TraceGate::check`]: enrolled key,
//! matching device identity and profile, a verdict-clean
//! [`verify_os_trace`] run, and every primary fact's payload digest
//! bound among the trace's emitted outputs. Failures surface as
//! [`StorageError::AttestationInvalid`] with the full reason list, the
//! same wire class as a bad merkle root, because that is what they
//! are: an attestation whose evidence does not hold.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sled::Tree;

use emem_core::device_platforms;
use emem_core::key::AttesterKey;
use emem_core::substrates::{AdmissionRule, SubstrateProfile};
use emem_fact::{Attestation, Fact, FactCid};
use emem_trace::enroll::Verdict as EnrollVerdict;
use emem_trace::{
    payload_digest_of_value, verify_os_trace, verify_platform_attestation, OsTrace,
    PlatformAttestation, Verdict,
};

use crate::StorageError;

const ENROLLMENT_TREE: &str = "emem.device_enrollment";
const TRACES_TREE: &str = "emem.os_traces";
const FACT_TRACE_TREE: &str = "emem.fact_trace";
const EVIDENCE_TREE: &str = "emem.enrollment_evidence";

/// What backs an enrolment. Stored as canonical CBOR under the device key.
///
/// Back-compat: entries written before this type existed are the bare
/// substrate profile ID as raw UTF-8. [`TraceGate::enrollment_of`] decodes
/// CBOR first and falls back to that legacy shape, so an old enrolment
/// keeps resolving with `platform_id: None`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnrollmentRecord {
    /// Substrate profile the key writes under.
    pub profile_id: String,
    /// The device platform whose attestation admitted the key, when the
    /// enrolment was attested. `None` for an operator-asserted enrolment
    /// (the migration-safe legacy path: the operator vouches, no hardware
    /// root of trust was presented).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_id: Option<String>,
    /// The trust anchor (Endorser) that signed the platform attestation,
    /// on an attested enrolment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endorsed_by: Option<String>,
}

impl EnrollmentRecord {
    /// `"platform_attested"` when a whitelisted anchor endorsed the key,
    /// else `"operator_asserted"`. The assurance level a reader can weigh.
    pub fn assurance(&self) -> &'static str {
        if self.platform_id.is_some() {
            "platform_attested"
        } else {
            "operator_asserted"
        }
    }
}

/// Outcome of a successful gate check for an enrolled device.
#[derive(Debug, Clone)]
pub struct AdmittedTrace {
    /// Content ID of the verified trace record.
    pub trace_cid: String,
    /// The `emem:trace:` token for it.
    pub token: String,
    /// Profile the device is enrolled under.
    pub profile_id: String,
}

/// Sled-backed enrollment registry + trace store. Cheap to clone.
#[derive(Clone)]
pub struct TraceGate {
    enrollment: Arc<Tree>,
    traces: Arc<Tree>,
    fact_trace: Arc<Tree>,
    evidence: Arc<Tree>,
}

impl TraceGate {
    /// Open (or create) the gate's trees on a sled db.
    pub fn open(db: &sled::Db) -> sled::Result<Self> {
        Ok(Self {
            enrollment: Arc::new(db.open_tree(ENROLLMENT_TREE)?),
            traces: Arc::new(db.open_tree(TRACES_TREE)?),
            fact_trace: Arc::new(db.open_tree(FACT_TRACE_TREE)?),
            evidence: Arc::new(db.open_tree(EVIDENCE_TREE)?),
        })
    }

    /// Validate that a profile exists and admits device output by trace.
    fn trace_admitted_profile(profile_id: &str) -> Result<(), StorageError> {
        let registry = &*emem_core::substrates::DEFAULT;
        let profile = registry.lookup(profile_id).ok_or_else(|| {
            StorageError::AttestationInvalid(format!(
                "os_trace gate: unknown substrate profile {profile_id}"
            ))
        })?;
        if profile.admission != AdmissionRule::OsTraceRequired {
            return Err(StorageError::AttestationInvalid(format!(
                "os_trace gate: profile {profile_id} is not trace-admitted"
            )));
        }
        Ok(())
    }

    fn write_enrollment(
        &self,
        pubkey_b32: &str,
        record: &EnrollmentRecord,
    ) -> Result<(), StorageError> {
        let mut buf = Vec::new();
        ciborium::ser::into_writer(record, &mut buf)
            .map_err(|e| StorageError::Cbor(e.to_string()))?;
        self.enrollment
            .insert(pubkey_b32.as_bytes(), buf)
            .map_err(sled_err)?;
        self.enrollment.flush().map_err(sled_err)?;
        Ok(())
    }

    /// Enroll an attester key under a substrate profile **on the
    /// operator's assertion alone** — no hardware root of trust is
    /// presented. This is the migration-safe legacy path: the resulting
    /// enrolment records `assurance = operator_asserted`, and a reader can
    /// tell it apart from an attested one. The profile must exist and be
    /// `os_trace_required`; enrolling under the archive profile is refused
    /// because the archive path does not admit device output at all.
    ///
    /// Prefer [`TraceGate::enroll_attested`] wherever the device can
    /// present a platform attestation.
    pub fn enroll(&self, pubkey_b32: &str, profile_id: &str) -> Result<(), StorageError> {
        Self::trace_admitted_profile(profile_id)?;
        self.write_enrollment(
            pubkey_b32,
            &EnrollmentRecord {
                profile_id: profile_id.to_string(),
                platform_id: None,
                endorsed_by: None,
            },
        )
    }

    /// Enroll an attester key under a substrate profile **only if a
    /// whitelisted platform attestation vouches for it**. The evidence
    /// must (1) attest the platform being enrolled under, (2) endorse
    /// exactly this device key, (3) be signed by a trust anchor the
    /// device-platforms manifest whitelists for that platform, and the
    /// platform must serve the profile's contributor class. On admit the
    /// enrolment records the endorsing anchor and the evidence is stored
    /// for later audit.
    ///
    /// Because every anchor shipped today is provisional, this currently
    /// rejects every attestation with `no_effective_anchor` — the
    /// intended no-new-admissions state until a real vendor anchor is
    /// pinned. It changes nothing prod accepts; it opens the path.
    pub fn enroll_attested(
        &self,
        pubkey_b32: &str,
        profile_id: &str,
        platform_id: &str,
        attestation: &PlatformAttestation,
    ) -> Result<EnrollmentRecord, StorageError> {
        Self::trace_admitted_profile(profile_id)?;
        let profile = emem_core::substrates::DEFAULT
            .lookup(profile_id)
            .ok_or_else(|| {
                StorageError::AttestationInvalid(format!(
                    "os_trace gate: unknown substrate profile {profile_id}"
                ))
            })?;
        let platform = device_platforms::DEFAULT
            .lookup(platform_id)
            .ok_or_else(|| {
                StorageError::AttestationInvalid(format!(
                    "os_trace gate: unknown device platform {platform_id}; \
                 GET /v1/device_platforms lists the whitelist"
                ))
            })?;
        if !platform.serves(profile.contributor_class) {
            return Err(StorageError::AttestationInvalid(format!(
                "os_trace gate: platform {platform_id} does not serve the {:?} contributor \
                 class that profile {profile_id} requires",
                profile.contributor_class
            )));
        }
        let device_key = decode_key(pubkey_b32).ok_or_else(|| {
            StorageError::AttestationInvalid(
                "os_trace gate: enrollee key is not a 32-byte base32 device key".into(),
            )
        })?;
        let report = verify_platform_attestation(platform, attestation, &device_key);
        if report.verdict != EnrollVerdict::Admit {
            let reasons: Vec<String> = report.reasons.iter().map(|r| r.to_string()).collect();
            return Err(StorageError::AttestationInvalid(format!(
                "os_trace gate: platform attestation rejected: {}",
                reasons.join("; ")
            )));
        }
        let record = EnrollmentRecord {
            profile_id: profile_id.to_string(),
            platform_id: Some(platform_id.to_string()),
            endorsed_by: report.endorsed_by.clone(),
        };
        self.write_enrollment(pubkey_b32, &record)?;
        // Persist the evidence so an attested enrolment is auditable later,
        // under both the device key (evidence_of) and the attestation CID
        // (get_attestation, so an emem:attestation: token resolves).
        let mut buf = Vec::new();
        ciborium::ser::into_writer(attestation, &mut buf)
            .map_err(|e| StorageError::Cbor(e.to_string()))?;
        self.evidence
            .insert(pubkey_b32.as_bytes(), buf.clone())
            .map_err(sled_err)?;
        if let Some(cid) = attestation.attestation_cid() {
            self.evidence
                .insert(cid.as_bytes(), buf)
                .map_err(sled_err)?;
        }
        self.evidence.flush().map_err(sled_err)?;
        Ok(record)
    }

    /// Remove an enrollment. The key returns to the ungated path.
    pub fn revoke(&self, pubkey_b32: &str) -> Result<(), StorageError> {
        self.enrollment
            .remove(pubkey_b32.as_bytes())
            .map_err(sled_err)?;
        self.evidence
            .remove(pubkey_b32.as_bytes())
            .map_err(sled_err)?;
        self.enrollment.flush().map_err(sled_err)?;
        self.evidence.flush().map_err(sled_err)?;
        Ok(())
    }

    /// The full enrolment record for a key, if enrolled. Decodes the CBOR
    /// record, falling back to the legacy raw-profile-ID shape.
    pub fn enrollment_of(&self, pubkey_b32: &str) -> Option<EnrollmentRecord> {
        let v = self.enrollment.get(pubkey_b32.as_bytes()).ok().flatten()?;
        if let Ok(rec) = ciborium::de::from_reader::<EnrollmentRecord, _>(v.as_ref()) {
            return Some(rec);
        }
        // Legacy: the value was the bare profile ID as raw UTF-8.
        String::from_utf8(v.to_vec())
            .ok()
            .map(|profile_id| EnrollmentRecord {
                profile_id,
                platform_id: None,
                endorsed_by: None,
            })
    }

    /// The stored platform attestation for an attested key, if any.
    pub fn evidence_of(&self, pubkey_b32: &str) -> Option<PlatformAttestation> {
        let v = self.evidence.get(pubkey_b32.as_bytes()).ok().flatten()?;
        ciborium::de::from_reader(v.as_ref()).ok()
    }

    /// Resolve a stored platform attestation by its content ID — what an
    /// `emem:attestation:` token names.
    pub fn get_attestation(&self, attestation_cid: &str) -> Option<PlatformAttestation> {
        let v = self
            .evidence
            .get(attestation_cid.as_bytes())
            .ok()
            .flatten()?;
        ciborium::de::from_reader(v.as_ref()).ok()
    }

    /// The profile an attester key is enrolled under, if any.
    pub fn profile_of(&self, pubkey_b32: &str) -> Option<String> {
        self.enrollment_of(pubkey_b32).map(|r| r.profile_id)
    }

    /// Gate an attestation. Returns `Ok(None)` when the attester is not
    /// enrolled (the gate does not apply); `Ok(Some(profile))` with the
    /// enrolled profile when the trace admits; an
    /// [`StorageError::AttestationInvalid`] otherwise.
    pub fn check(
        &self,
        att: &Attestation,
        trace: Option<&OsTrace>,
    ) -> Result<Option<SubstrateProfile>, StorageError> {
        let pubkey_b32 = render_key(&att.attester.0);
        let Some(enrollment) = self.enrollment_of(&pubkey_b32) else {
            return Ok(None);
        };
        let profile_id = enrollment.profile_id.clone();
        let registry = &*emem_core::substrates::DEFAULT;
        let profile = registry.lookup(&profile_id).ok_or_else(|| {
            StorageError::AttestationInvalid(format!(
                "os_trace gate: enrolled profile {profile_id} no longer in manifest"
            ))
        })?;
        let Some(trace) = trace else {
            return Err(StorageError::AttestationInvalid(format!(
                "os_trace gate: attester {pubkey_b32} is enrolled under {profile_id}; \
                 writes require the device's OS execution trace and none was presented"
            )));
        };
        // The trace must be the enrolled device speaking for itself.
        if trace.device.device_key != att.attester {
            return Err(StorageError::AttestationInvalid(
                "os_trace gate: trace device key does not match the attester key".into(),
            ));
        }
        let report = verify_os_trace(trace, profile, None);
        if report.verdict != Verdict::Admit {
            let reasons: Vec<String> = report.reasons.iter().map(|r| r.to_string()).collect();
            return Err(StorageError::AttestationInvalid(format!(
                "os_trace gate: trace rejected: {}",
                reasons.join("; ")
            )));
        }
        // Every segment's capture encoding must be a registered encoding
        // (the "trace of the trace": an unknown tracer is not admissible
        // evidence). For a platform-attested enrolment, the encoding must
        // additionally be one the enrolled platform is whitelisted to emit,
        // which binds device-platforms -> trace-encodings -> the trace.
        // Layer-consistency (an encoding producing only the layers it can
        // capture) is available in the registry but not yet enforced here,
        // pending encodings for the energy and thermal layers.
        let enc_registry = &*emem_core::trace_encodings::DEFAULT;
        let platform = enrollment
            .platform_id
            .as_deref()
            .and_then(|id| emem_core::device_platforms::DEFAULT.lookup(id));
        for seg in &trace.segments {
            if !enc_registry.recognizes(&seg.encoding) {
                return Err(StorageError::AttestationInvalid(format!(
                    "os_trace gate: segment {} names unregistered capture encoding {}; \
                     GET /v1/trace_encodings lists the vocabulary",
                    seg.seq, seg.encoding
                )));
            }
            if let Some(platform) = platform {
                if !platform.recognizes_encoding(&seg.encoding) {
                    return Err(StorageError::AttestationInvalid(format!(
                        "os_trace gate: platform {} does not emit encoding {}; segment {} is \
                         outside the platform's recognized encodings",
                        platform.id, seg.encoding, seg.seq
                    )));
                }
            }
        }
        // An enrolled device writes traced primary observations, and
        // nothing else. Derivative facts, absences, and edges are
        // claims about facts rather than emissions of a sensor; until
        // a traced-derivation rule exists they would be an untraced
        // side door, so the gate closes it outright.
        if !att.edges.is_empty() {
            return Err(StorageError::AttestationInvalid(
                "os_trace gate: enrolled device keys may not write edges \
                 (no traced-derivation rule yet)"
                    .into(),
            ));
        }
        for fact in &att.facts {
            let p = match fact {
                Fact::Primary(p) => p,
                Fact::Derivative(_) | Fact::Absence(_) => {
                    return Err(StorageError::AttestationInvalid(
                        "os_trace gate: enrolled device keys may write primary \
                         facts only (no traced-derivation rule yet)"
                            .into(),
                    ));
                }
            };
            let digest = payload_digest_of_value(&p.value).map_err(|e| {
                StorageError::AttestationInvalid(format!(
                    "os_trace gate: payload digest failed: {e}"
                ))
            })?;
            if !trace.outputs.iter().any(|o| o.payload_digest == digest) {
                return Err(StorageError::AttestationInvalid(format!(
                    "os_trace gate: fact at band {} carries payload digest {digest} \
                     that is not bound in the trace's emitted outputs",
                    p.band
                )));
            }
        }
        Ok(Some(profile.clone()))
    }

    /// Persist an admitted trace and its audit edges. Called after the
    /// attestation itself committed, so a stored trace always points at
    /// stored facts.
    pub fn persist(
        &self,
        trace: &OsTrace,
        fact_cids: &[FactCid],
        profile_id: &str,
    ) -> Result<AdmittedTrace, StorageError> {
        let trace_cid = trace
            .trace_cid()
            .map_err(|e| StorageError::Cbor(e.to_string()))?;
        let mut buf = Vec::new();
        ciborium::ser::into_writer(trace, &mut buf)
            .map_err(|e| StorageError::Cbor(e.to_string()))?;
        self.traces
            .insert(trace_cid.as_bytes(), buf)
            .map_err(sled_err)?;
        for cid in fact_cids {
            self.fact_trace
                .insert(cid.0.as_bytes(), trace_cid.as_bytes())
                .map_err(sled_err)?;
        }
        self.traces.flush().map_err(sled_err)?;
        self.fact_trace.flush().map_err(sled_err)?;
        Ok(AdmittedTrace {
            token: emem_trace::trace_token(&trace_cid),
            trace_cid,
            profile_id: profile_id.to_string(),
        })
    }

    /// Resolve a stored trace by CID to its byte-identical record.
    pub fn get_trace(&self, trace_cid: &str) -> Option<OsTrace> {
        let bytes = self.traces.get(trace_cid.as_bytes()).ok().flatten()?;
        ciborium::de::from_reader(bytes.as_ref()).ok()
    }

    /// The trace CID an admitted fact was gated under, if any.
    pub fn trace_for_fact(&self, cid: &FactCid) -> Option<String> {
        self.fact_trace
            .get(cid.0.as_bytes())
            .ok()
            .flatten()
            .and_then(|v| String::from_utf8(v.to_vec()).ok())
    }

    /// Count of enrolled device keys.
    pub fn enrolled_count(&self) -> u64 {
        self.enrollment.len() as u64
    }
}

/// Render a 32-byte attester key the way `AttesterRegistry` renders it:
/// base32-nopad lowercase, matching every other digest rendering.
fn render_key(key: &[u8; 32]) -> String {
    data_encoding::BASE32_NOPAD.encode(key).to_lowercase()
}

/// Inverse of [`render_key`]: a base32-nopad key string back to an
/// [`AttesterKey`], or `None` if it is not exactly 32 bytes.
fn decode_key(pubkey_b32: &str) -> Option<AttesterKey> {
    let bytes = data_encoding::BASE32_NOPAD
        .decode(pubkey_b32.to_uppercase().as_bytes())
        .ok()?;
    let arr: [u8; 32] = bytes.try_into().ok()?;
    Some(AttesterKey(arr))
}

fn sled_err(e: sled::Error) -> StorageError {
    StorageError::Io(std::io::Error::other(e))
}
