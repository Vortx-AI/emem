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
const SESSION_TREE: &str = "emem.trace_session";

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
    /// Whether this device consents to being listed publicly.
    ///
    /// Defaults to false, and the default is the whole design. Enrolling is
    /// how a device joins the protocol; being listed is a separate, later,
    /// deliberate act. A roster that included every device that ever enrolled
    /// would publish the shape of somebody's fleet as a side effect of them
    /// using the software, which nobody agreed to.
    ///
    /// `serde(default)` also makes this migration-safe in the strict sense:
    /// every enrolment written before this field existed decodes as private,
    /// which is the answer they would have given if asked.
    #[serde(default)]
    pub publish: bool,
}

/// One device on the public roster, and only what it consented to show.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PublishedDevice {
    /// The device's ed25519 public key, base32-nopad lowercase.
    pub device_key: String,
    /// Substrate profile it writes under.
    pub profile_id: String,
    /// Device platform, when the enrolment was attested.
    pub platform_id: Option<String>,
    /// The anchor that endorsed it. An operator's anchor id here rather than a
    /// manufacturer's is the reader's cue about whose word this rests on.
    pub endorsed_by: Option<String>,
    /// `platform_attested` or `operator_asserted`.
    pub assurance: String,
    /// How many traces this device has written.
    pub traces: u64,
    /// End of its most recent capture window, monotonic nanoseconds on the
    /// device. Not a wall clock, and not comparable between devices.
    pub last_seen: u64,
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
    session: Arc<Tree>,
}

impl TraceGate {
    /// Open (or create) the gate's trees on a sled db.
    pub fn open(db: &sled::Db) -> sled::Result<Self> {
        Ok(Self {
            enrollment: Arc::new(db.open_tree(ENROLLMENT_TREE)?),
            traces: Arc::new(db.open_tree(TRACES_TREE)?),
            fact_trace: Arc::new(db.open_tree(FACT_TRACE_TREE)?),
            evidence: Arc::new(db.open_tree(EVIDENCE_TREE)?),
            session: Arc::new(db.open_tree(SESSION_TREE)?),
        })
    }

    /// The CID of a device's most recently admitted trace for a given boot
    /// — the head of that boot's stream — or `None` if the device has
    /// written nothing under this boot yet. Keyed by `(device_key, boot_id)`
    /// so a reboot (a fresh `boot_id` in the signed trace) starts a new
    /// stream rather than wedging on a head the rebooted device no longer
    /// remembers.
    pub fn stream_head(&self, device_key_b32: &str, boot_id: &str) -> Option<String> {
        self.session
            .get(session_key(device_key_b32, boot_id).as_bytes())
            .ok()
            .flatten()
            .and_then(|v| String::from_utf8(v.to_vec()).ok())
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
                // Enrolling is joining; being listed is a separate act.
                publish: false,
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
        if !platform.serves(&profile.contributor_class) {
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
            publish: false,
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
                publish: false,
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
        // Stream continuity: consecutive traces from a device chain via
        // prev_trace_cid. A device with a stored stream head must present a
        // trace whose prev_trace_cid equals it; the first trace must carry
        // no prev. A dropped, duplicated, or reordered window is detected
        // here at ingest rather than left for a reader to notice.
        match (
            self.stream_head(&pubkey_b32, &trace.device.boot_id),
            trace.prev_trace_cid.as_deref(),
        ) {
            (None, None) => {}
            (Some(head), Some(prev)) if prev == head => {}
            (Some(head), prev) => {
                return Err(StorageError::AttestationInvalid(format!(
                    "os_trace gate: stream continuity broken — this device's stream head is \
                     {head}, but the trace's prev_trace_cid is {prev:?}"
                )));
            }
            (None, Some(prev)) => {
                return Err(StorageError::AttestationInvalid(format!(
                    "os_trace gate: this device has no prior trace, but this one names \
                     prev_trace_cid {prev}"
                )));
            }
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
        // evidence) AND must be able to produce the layer the segment
        // claims (an encoding that only sees the syscall stream cannot
        // vouch for a thermal reading). For a platform-attested enrolment,
        // the encoding must additionally be one the enrolled platform is
        // whitelisted to emit, binding device-platforms -> trace-encodings
        // -> the trace.
        let enc_registry = &*emem_core::trace_encodings::DEFAULT;
        // A recorded platform that has since left the manifest is an error,
        // not a silent downgrade of the encoding check (mirrors the
        // profile-missing case above).
        let platform = match enrollment.platform_id.as_deref() {
            Some(id) => Some(emem_core::device_platforms::DEFAULT.lookup(id).ok_or_else(|| {
                StorageError::AttestationInvalid(format!(
                    "os_trace gate: enrolled platform {id} no longer in the device-platforms manifest"
                ))
            })?),
            None => None,
        };
        for seg in &trace.segments {
            let Some(enc) = enc_registry.lookup(&seg.encoding) else {
                return Err(StorageError::AttestationInvalid(format!(
                    "os_trace gate: segment {} names unregistered capture encoding {}; \
                     GET /v1/trace_encodings lists the vocabulary",
                    seg.seq, seg.encoding
                )));
            };
            if !enc.can_capture(seg.layer) {
                return Err(StorageError::AttestationInvalid(format!(
                    "os_trace gate: encoding {} cannot capture the {:?} layer that segment {} \
                     claims; a tracer cannot vouch for a layer it does not produce",
                    seg.encoding, seg.layer, seg.seq
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
        // Advance this boot's stream head so the next window must chain to
        // this trace.
        self.session
            .insert(
                session_key(
                    &render_key(&trace.device.device_key.0),
                    &trace.device.boot_id,
                )
                .as_bytes(),
                trace_cid.as_bytes(),
            )
            .map_err(sled_err)?;
        self.traces.flush().map_err(sled_err)?;
        self.fact_trace.flush().map_err(sled_err)?;
        self.session.flush().map_err(sled_err)?;
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
    /// Devices that have consented to being listed, with what is safe to show.
    ///
    /// Only `publish: true` enrolments are returned, and that filter is here
    /// rather than in the route on purpose: a privacy rule enforced at the
    /// edge is one refactor away from being forgotten. Anything reading this
    /// method gets the consenting set and cannot accidentally get the rest.
    ///
    /// What comes back is deliberately thin. A device that agreed to appear on
    /// a status page did not thereby agree to publish its traffic: the count of
    /// traces it has written and when it last wrote one is enough to show a
    /// fleet is alive, and the traces themselves stay where they were.
    pub fn published_devices(&self) -> Vec<PublishedDevice> {
        let mut out = Vec::new();
        for kv in self.enrollment.iter().flatten() {
            let Ok(pubkey) = String::from_utf8(kv.0.to_vec()) else {
                continue;
            };
            let Some(rec) = self.enrollment_of(&pubkey) else {
                continue;
            };
            if !rec.publish {
                continue;
            }
            let (traces, last_seen) = self.trace_activity(&pubkey);
            let assurance = rec.assurance().to_string();
            out.push(PublishedDevice {
                device_key: pubkey,
                profile_id: rec.profile_id,
                platform_id: rec.platform_id,
                endorsed_by: rec.endorsed_by,
                assurance,
                traces,
                last_seen,
            });
        }
        out.sort_by_key(|d| std::cmp::Reverse(d.last_seen));
        out
    }

    /// How many traces a device has written, and the most recent window end.
    ///
    /// Scans rather than keeping a counter, because a counter that drifts from
    /// the traces it counts is worse than a scan that is merely slow, and this
    /// is read by a status page rather than a hot path.
    fn trace_activity(&self, device_key: &str) -> (u64, u64) {
        let mut count = 0u64;
        let mut latest = 0u64;
        for kv in self.traces.iter().flatten() {
            let Ok(trace) = ciborium::from_reader::<emem_trace::OsTrace, _>(&kv.1[..]) else {
                continue;
            };
            let key = data_encoding::BASE32_NOPAD
                .encode(&trace.device.device_key.0)
                .to_lowercase();
            if key != device_key {
                continue;
            }
            count += 1;
            latest = latest.max(trace.window_end_ns);
        }
        (count, latest)
    }

    /// Record a device's consent to be listed, or withdraw it.
    ///
    /// Separate from enrolment because consent is separate from joining, and
    /// reversible because consent that cannot be withdrawn is not consent.
    pub fn set_publish(&self, pubkey_b32: &str, publish: bool) -> Result<bool, StorageError> {
        let Some(mut rec) = self.enrollment_of(pubkey_b32) else {
            return Ok(false);
        };
        rec.publish = publish;
        // Reuse the one writer, so consent goes through exactly the same
        // encode and store path as every other change to an enrolment.
        self.write_enrollment(pubkey_b32, &rec)?;
        Ok(true)
    }

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

/// The session tree key for a device's stream under one boot. A NUL joins
/// the two so distinct `(key, boot)` pairs cannot collide.
fn session_key(device_key_b32: &str, boot_id: &str) -> String {
    format!("{device_key_b32}\0{boot_id}")
}

fn sled_err(e: sled::Error) -> StorageError {
    StorageError::Io(std::io::Error::other(e))
}

#[cfg(test)]
mod enrollment_record_tests {
    use super::*;

    fn temp_gate() -> TraceGate {
        let db = sled::Config::new()
            .temporary(true)
            .open()
            .expect("temp sled");
        TraceGate::open(&db).expect("gate")
    }

    #[test]
    fn legacy_bare_profile_id_enrollment_still_resolves() {
        // A pre-EnrollmentRecord entry was the bare profile ID as raw
        // UTF-8. enrollment_of() must fall back to that shape (a CBOR text
        // string is not a map, so the struct decode fails first).
        let gate = temp_gate();
        gate.enrollment
            .insert(b"legacykey".as_slice(), b"robot.fleet.v1".as_slice())
            .expect("insert legacy");
        let rec = gate.enrollment_of("legacykey").expect("legacy resolves");
        assert_eq!(rec.profile_id, "robot.fleet.v1");
        assert_eq!(rec.platform_id, None);
        assert_eq!(rec.assurance(), "operator_asserted");
    }

    #[test]
    fn new_cbor_enrollment_round_trips_and_is_operator_asserted() {
        let gate = temp_gate();
        gate.enroll("newkey", "robot.fleet.v1").expect("enroll");
        let rec = gate.enrollment_of("newkey").expect("resolves");
        assert_eq!(rec.profile_id, "robot.fleet.v1");
        assert_eq!(rec.platform_id, None);
        assert_eq!(rec.assurance(), "operator_asserted");
    }

    #[test]
    fn revoke_clears_the_enrollment() {
        let gate = temp_gate();
        gate.enroll("k", "robot.fleet.v1").expect("enroll");
        assert!(gate.enrollment_of("k").is_some());
        gate.revoke("k").expect("revoke");
        assert!(gate.enrollment_of("k").is_none());
    }

    /// Enrolling must never publish. This is the privacy default, and it is
    /// the kind of default that is only real if a test says so.
    #[test]
    fn enrolling_does_not_put_a_device_on_the_roster() {
        let gate = temp_gate();
        gate.enroll("aaaa1111", "robot.fleet.v1").unwrap();
        assert!(
            gate.published_devices().is_empty(),
            "a device that merely joined must not appear on a public roster"
        );
        assert_eq!(gate.enrolled_count(), 1, "but it IS enrolled");
    }

    /// Consent is a separate, deliberate act, and it is reversible.
    #[test]
    fn consent_can_be_given_and_withdrawn() {
        let gate = temp_gate();
        gate.enroll("bbbb2222", "robot.fleet.v1").unwrap();

        assert!(gate.set_publish("bbbb2222", true).unwrap());
        let listed = gate.published_devices();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].device_key, "bbbb2222");
        assert_eq!(listed[0].assurance, "operator_asserted");

        // Withdrawing must take it off again: consent that cannot be
        // withdrawn is not consent.
        assert!(gate.set_publish("bbbb2222", false).unwrap());
        assert!(gate.published_devices().is_empty());
    }

    /// Consent for a device that never enrolled is not silently invented.
    #[test]
    fn consent_for_an_unknown_device_is_refused() {
        let gate = temp_gate();
        assert!(
            !gate.set_publish("never-enrolled", true).unwrap(),
            "there is no enrolment to attach consent to"
        );
        assert!(gate.published_devices().is_empty());
    }

    /// An enrolment written before the field existed reads as private, which
    /// is the answer its operator would have given had they been asked.
    #[test]
    fn a_legacy_enrolment_defaults_to_private() {
        let older = EnrollmentRecord {
            profile_id: "robot.fleet.v1".into(),
            platform_id: None,
            endorsed_by: None,
            publish: false,
        };
        let mut buf = Vec::new();
        ciborium::into_writer(&older, &mut buf).unwrap();
        // Decode through the same path the gate uses.
        let back: EnrollmentRecord = ciborium::from_reader(&buf[..]).unwrap();
        assert!(!back.publish);
    }
}
