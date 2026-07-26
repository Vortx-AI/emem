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

use sled::Tree;

use emem_core::substrates::{AdmissionRule, SubstrateProfile};
use emem_fact::{Attestation, Fact, FactCid};
use emem_trace::{payload_digest_of_value, verify_os_trace, OsTrace, Verdict};

use crate::StorageError;

const ENROLLMENT_TREE: &str = "emem.device_enrollment";
const TRACES_TREE: &str = "emem.os_traces";
const FACT_TRACE_TREE: &str = "emem.fact_trace";

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
}

impl TraceGate {
    /// Open (or create) the gate's trees on a sled db.
    pub fn open(db: &sled::Db) -> sled::Result<Self> {
        Ok(Self {
            enrollment: Arc::new(db.open_tree(ENROLLMENT_TREE)?),
            traces: Arc::new(db.open_tree(TRACES_TREE)?),
            fact_trace: Arc::new(db.open_tree(FACT_TRACE_TREE)?),
        })
    }

    /// Enroll an attester key under a substrate profile. The profile
    /// must exist in the substrates manifest and carry the
    /// `os_trace_required` admission rule; enrolling a key under the
    /// archive profile is refused because the archive path does not
    /// admit device output at all.
    pub fn enroll(&self, pubkey_b32: &str, profile_id: &str) -> Result<(), StorageError> {
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
        self.enrollment
            .insert(pubkey_b32.as_bytes(), profile_id.as_bytes())
            .map_err(sled_err)?;
        self.enrollment.flush().map_err(sled_err)?;
        Ok(())
    }

    /// Remove an enrollment. The key returns to the ungated path.
    pub fn revoke(&self, pubkey_b32: &str) -> Result<(), StorageError> {
        self.enrollment
            .remove(pubkey_b32.as_bytes())
            .map_err(sled_err)?;
        self.enrollment.flush().map_err(sled_err)?;
        Ok(())
    }

    /// The profile an attester key is enrolled under, if any.
    pub fn profile_of(&self, pubkey_b32: &str) -> Option<String> {
        self.enrollment
            .get(pubkey_b32.as_bytes())
            .ok()
            .flatten()
            .and_then(|v| String::from_utf8(v.to_vec()).ok())
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
        let Some(profile_id) = self.profile_of(&pubkey_b32) else {
            return Ok(None);
        };
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

fn sled_err(e: sled::Error) -> StorageError {
    StorageError::Io(std::io::Error::other(e))
}
