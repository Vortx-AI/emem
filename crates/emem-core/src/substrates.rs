//! Substrate profile registry — loaded from the **content-addressed
//! substrates manifest**.
//!
//! A substrate is everything one class of machine contributes to the
//! memory: which bands it writes, at what measurement grain, and above
//! all under which admission rule its output enters the record. The
//! registry makes the protocol's trust stance explicit and machine
//! checkable:
//!
//! - The founding Earth substrate is admitted by **recomputability**: its
//!   sources are free, public archives anyone can re-fetch, so a value is
//!   trusted because a third party can reproduce it. It is the standing
//!   drift anchor every other substrate's claims are scored against.
//! - Every new contributor class (telescope, microscope, CCTV, mobile,
//!   robot, industrial machine) is admitted by **execution evidence**: the
//!   protocol respects the device as an observer of the physical world,
//!   but never accepts its output alone. The device must present its
//!   complete, unaltered OS execution trace, and only output bound inside
//!   a verified trace is admitted. The trace record itself is defined in
//!   the `emem-trace` crate; this registry pins which layers each profile
//!   must cover.

use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

use crate::manifest::{Manifest, ManifestError, MANIFEST_SUBSTRATE_REG};

const SUBSTRATES_V0_JSON: &str = include_str!("../data/substrates-v0.json");

/// The class of machine contributing observations under a profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContributorClass {
    /// Orbital imaging or sounding platform (or its public archive).
    Satellite,
    /// Ground or space telescope observing from a fixed site cell.
    Telescope,
    /// Laboratory or field microscope; grain reaches into microns.
    Microscope,
    /// Fixed surveillance or monitoring camera.
    Cctv,
    /// Handheld phone or wearable with cameras and sensors.
    Mobile,
    /// Mobile robot or robot fleet member.
    Robot,
    /// Uncrewed aerial vehicle flying survey paths.
    Drone,
    /// Meter, turbine, pipeline or plant-floor machine.
    IndustrialMachine,
    /// Fixed gauge or single-purpose environmental sensor.
    FixedSensor,
}

/// One layer of a device's OS execution trace. A substrate profile lists
/// the layers its devices MUST capture; the `emem-trace` verifier rejects
/// a trace that misses any of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceLayerKind {
    /// System-call stream: every kernel entry the workload made.
    Syscall,
    /// Scheduler activity: which tasks ran, when, on which core.
    Scheduler,
    /// Memory tracks: allocations, page faults, mapped regions.
    Memory,
    /// Energy draw over the capture window.
    Energy,
    /// Thermal readings from on-die and board sensors.
    Thermal,
    /// Raw signal path: RF, ADC, or bus-level signal captures.
    Signal,
    /// Sensor bus traffic: the bytes the instrument actually emitted.
    SensorBus,
    /// Network activity during the window.
    Network,
    /// Storage I/O during the window.
    Storage,
    /// On-device model execution: which weights ran over which input.
    Inference,
}

/// How a substrate's output is admitted into the memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdmissionRule {
    /// The device must present its complete, unaltered OS execution
    /// trace; output is admitted only when it is bound inside a verified
    /// trace. This is the rule for every device-borne substrate: the
    /// protocol respects the device but never takes its word.
    OsTraceRequired,
    /// The founding archive rule: sources are free, public archives, so
    /// admission rests on a third party's ability to re-fetch the cited
    /// source and recompute the value. No execution trace exists or is
    /// needed; the archive itself is the evidence.
    ArchiveRecomputable,
}

/// Lifecycle state of a profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileStatus {
    /// Shipping: writes under this profile are accepted today.
    Active,
    /// Declared direction: the profile is pinned so device makers can
    /// build against it, but the ingest path is not open yet.
    Candidate,
}

/// One substrate profile entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubstrateProfile {
    /// Profile ID (e.g. `"earth.satellite.v0"`, `"robot.fleet.v1"`).
    pub id: String,
    /// Human-readable title.
    pub title: String,
    /// Machine class contributing under this profile.
    pub contributor_class: ContributorClass,
    /// Lifecycle state.
    pub status: ProfileStatus,
    /// Admission rule — the load-bearing field.
    pub admission: AdmissionRule,
    /// Finest measurement grain, meters (microscopes reach `1e-7`).
    pub grain_min_m: f64,
    /// Coarsest measurement grain, meters.
    pub grain_max_m: f64,
    /// Natural tempo class of the substrate's observations.
    pub tempo: String,
    /// Trace layers a device MUST capture. Empty when the admission rule
    /// is [`AdmissionRule::ArchiveRecomputable`].
    #[serde(default)]
    pub required_trace_layers: Vec<TraceLayerKind>,
    /// Evidence layers the substrate serves to readers (e.g. `"trace"`,
    /// `"signal"`, `"inference"`, `"weather"`). Editorial: names the
    /// causally linked views contradiction scoring runs across.
    #[serde(default)]
    pub evidence_layers: Vec<String>,
    /// Whether this substrate is a drift anchor: the independently
    /// recomputable record device claims are contradiction-scored
    /// against. The founding Earth substrate is the standing anchor.
    #[serde(default)]
    pub drift_anchor: bool,
    /// Default provenance class for bands written under this profile,
    /// as the snake_case wire name of a `bands` manifest provenance
    /// class (e.g. `"direct_sensor"`). Admission and provenance are
    /// orthogonal: this says how a value was produced, the admission
    /// rule says why the record believes the producer.
    pub provenance_class: String,
    /// Band keys or family prefixes this substrate writes.
    #[serde(default)]
    pub bands: Vec<String>,
    /// Lineage metadata keys the substrate's sources publish about how
    /// a product was made (for the Earth archives: the SAFE manifest's
    /// processing history, STAC properties like `s2:processing_baseline`
    /// and `s1:orbit_source`, and the Copernicus Data Space traceability
    /// register of per-product BLAKE3 checksums). This is **declared**
    /// lineage: the publisher's statement, checkable against the
    /// publisher's own register, not verified execution. An OS trace is
    /// what a device substrate presents instead; declared lineage is
    /// what an archive substrate has, and it feeds the sensor term of
    /// change attribution (a processing-baseline bump is a real cause
    /// of a value moving with the world unchanged).
    #[serde(default)]
    pub declared_lineage: Vec<String>,
    /// Editorial note.
    #[serde(default, rename = "_note", skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Substrates manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubstrateRegistry {
    /// MUST equal `"emem-substrates"`.
    pub manifest: String,
    /// Version, e.g. `"v0"`.
    pub version: String,
    /// Profile entries.
    pub substrates: Vec<SubstrateProfile>,
    /// Editorial note.
    #[serde(default, rename = "_note", skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Wire names of the provenance classes a profile may declare. Kept as a
/// string list (not the enum) so the registry validates without pulling
/// the band ontology into every loader.
const KNOWN_PROVENANCE: [&str; 6] = [
    "direct_sensor",
    "deterministic_index",
    "attested_execution",
    "model_output",
    "human_curated",
    "unclassified",
];

impl Manifest for SubstrateRegistry {
    const KIND: &'static str = MANIFEST_SUBSTRATE_REG;

    fn validate(&self) -> Result<(), ManifestError> {
        if self.manifest != Self::KIND {
            return Err(ManifestError::WrongKind {
                expected: Self::KIND,
                actual: self.manifest.clone(),
            });
        }
        let mut seen: std::collections::HashSet<&str> = Default::default();
        let mut anchors = 0usize;
        for p in &self.substrates {
            if !seen.insert(&p.id) {
                return Err(ManifestError::Invalid(format!(
                    "duplicate substrate profile: {}",
                    p.id
                )));
            }
            if !(p.grain_min_m > 0.0 && p.grain_min_m <= p.grain_max_m) {
                return Err(ManifestError::Invalid(format!(
                    "{}: grain range must satisfy 0 < min <= max",
                    p.id
                )));
            }
            if !KNOWN_PROVENANCE.contains(&p.provenance_class.as_str()) {
                return Err(ManifestError::Invalid(format!(
                    "{}: unknown provenance class {}",
                    p.id, p.provenance_class
                )));
            }
            match p.admission {
                AdmissionRule::OsTraceRequired => {
                    if p.required_trace_layers.is_empty() {
                        return Err(ManifestError::Invalid(format!(
                            "{}: os_trace_required demands at least one trace layer",
                            p.id
                        )));
                    }
                    // The anchor role belongs to independently
                    // recomputable archives; a device cannot anchor
                    // the record it is being scored against.
                    if p.drift_anchor {
                        return Err(ManifestError::Invalid(format!(
                            "{}: a trace-admitted substrate cannot be a drift anchor",
                            p.id
                        )));
                    }
                }
                AdmissionRule::ArchiveRecomputable => {
                    if !p.required_trace_layers.is_empty() {
                        return Err(ManifestError::Invalid(format!(
                            "{}: archive_recomputable must not require trace layers",
                            p.id
                        )));
                    }
                }
            }
            if p.drift_anchor && p.status == ProfileStatus::Active {
                anchors += 1;
            }
        }
        if anchors == 0 {
            return Err(ManifestError::Invalid(
                "registry needs at least one active drift-anchor substrate".into(),
            ));
        }
        Ok(())
    }
}

impl SubstrateRegistry {
    /// Embedded v0 default.
    pub fn parse_default() -> Result<Self, ManifestError> {
        Self::parse_json(SUBSTRATES_V0_JSON.as_bytes())
    }

    /// Look up a profile by ID.
    pub fn lookup(&self, id: &str) -> Option<&SubstrateProfile> {
        self.substrates.iter().find(|p| p.id == id)
    }

    /// The active drift-anchor profiles (the founding Earth substrate).
    pub fn drift_anchors(&self) -> impl Iterator<Item = &SubstrateProfile> {
        self.substrates
            .iter()
            .filter(|p| p.drift_anchor && p.status == ProfileStatus::Active)
    }
}

/// Process-wide cached default registry.
pub static DEFAULT: LazyLock<SubstrateRegistry> = LazyLock::new(|| {
    SubstrateRegistry::parse_default().expect("embedded substrates-v0.json is malformed")
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_loads_and_anchor_is_earth() {
        let r = &*DEFAULT;
        let earth = r.lookup("earth.satellite.v0").expect("earth profile");
        assert_eq!(earth.admission, AdmissionRule::ArchiveRecomputable);
        assert!(earth.drift_anchor);
        assert_eq!(r.drift_anchors().count(), 1);
    }

    #[test]
    fn every_device_profile_requires_a_trace() {
        for p in &DEFAULT.substrates {
            if p.id == "earth.satellite.v0" {
                continue;
            }
            assert_eq!(p.admission, AdmissionRule::OsTraceRequired, "{}", p.id);
            assert!(!p.required_trace_layers.is_empty(), "{}", p.id);
            assert!(!p.drift_anchor, "{}", p.id);
        }
    }

    #[test]
    fn earth_declares_its_archive_lineage() {
        let earth = DEFAULT.lookup("earth.satellite.v0").expect("earth");
        // The archives publish declared lineage, not OS traces; the
        // profile says so with real, live-checked metadata keys.
        assert!(earth
            .declared_lineage
            .iter()
            .any(|k| k == "s2:processing_baseline"));
        assert!(earth
            .declared_lineage
            .iter()
            .any(|k| k == "cdse.traceability.blake3"));
    }

    #[test]
    fn micron_grain_is_representable() {
        let m = DEFAULT.lookup("lab.microscope.v1").expect("microscope");
        assert!(m.grain_min_m < 1e-6);
    }

    #[test]
    fn trace_admitted_anchor_is_rejected() {
        let mut r = DEFAULT.clone();
        for p in &mut r.substrates {
            if p.id == "robot.fleet.v1" {
                p.drift_anchor = true;
            }
        }
        assert!(r.validate().is_err());
    }

    #[test]
    fn manifest_cid_is_stable_across_reparse() {
        let a = crate::manifest::manifest_cid(&*DEFAULT).expect("cid");
        let b = crate::manifest::manifest_cid(&SubstrateRegistry::parse_default().expect("parse"))
            .expect("cid");
        assert_eq!(a, b);
    }
}
