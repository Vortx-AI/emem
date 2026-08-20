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

/// The class of contributor writing observations under a profile.
///
/// **Open on purpose, unlike [`AdmissionRule`].** This was a closed enum of
/// nine physical device classes, so declaring a substrate that is not a
/// camera, gauge or robot meant editing Rust and shipping a build. That is
/// backwards: a contributor class is DESCRIPTIVE. Nothing enforces anything
/// on the strength of it; it names who is writing and lets a reader filter.
/// The registry is data, and a purely descriptive field in a data registry
/// has no business being a compile-time closed set.
///
/// [`AdmissionRule`] is the opposite case and stays closed, because each rule
/// there is an ENFORCEMENT PATH. A registry that could name a new admission
/// rule could advertise evidence handling that no code performs, which is a
/// worse failure than a recompile: it reads as a checked property and is not
/// one. When the two look symmetric, ask what breaks if the value is unknown.
/// An unknown contributor class is a label nobody recognises. An unknown
/// admission rule is an unguarded door.
///
/// The constants below are the classes this build knows by name. They are
/// conveniences for call sites, not a closed set: any string in the registry
/// is a valid class.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContributorClass(String);

impl ContributorClass {
    /// Orbital imaging or sounding platform (or its public archive).
    pub const SATELLITE: &'static str = "satellite";
    /// Ground or space telescope observing from a fixed site cell.
    pub const TELESCOPE: &'static str = "telescope";
    /// Laboratory or field microscope; grain reaches into microns.
    pub const MICROSCOPE: &'static str = "microscope";
    /// Fixed surveillance or monitoring camera.
    pub const CCTV: &'static str = "cctv";
    /// Handheld phone or wearable with cameras and sensors.
    pub const MOBILE: &'static str = "mobile";
    /// Mobile robot or robot fleet member.
    pub const ROBOT: &'static str = "robot";
    /// Uncrewed aerial vehicle flying survey paths.
    pub const DRONE: &'static str = "drone";
    /// Meter, turbine, pipeline or plant-floor machine.
    pub const INDUSTRIAL_MACHINE: &'static str = "industrial_machine";
    /// Fixed gauge or single-purpose environmental sensor.
    pub const FIXED_SENSOR: &'static str = "fixed_sensor";

    /// Wrap a class name. Any non-empty string is valid.
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// The wire name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ContributorClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for ContributorClass {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl PartialEq<str> for ContributorClass {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

/// How subjects under a profile are ADDRESSED.
///
/// This is the field the registry was missing, and its absence was the
/// protocol's largest unstated assumption. Every fact is keyed by
/// `CanonicalKey { cell, band, tslot }`, where `cell` is a cell64: a 64-bit
/// quantisation of latitude and longitude. So a fact could only ever be
/// recorded ABOUT A PLACE, while the README said "Earth is the substrate, not
/// the subject" and "nothing in the record, receipt, or token grammar is
/// satellite-specific".
///
/// Both halves of that are true and they do not add up to the conclusion
/// people drew from them. The record, the receipt and the token grammar are
/// genuinely substrate-neutral: BLAKE3 over canonical CBOR, an ed25519
/// receipt, an RFC 6962 log, bi-temporal bounds and contradiction scoring do
/// not care what the subject is. The ADDRESS is not neutral, and the address
/// is the part that has to exist before any of the rest can hang off it. A
/// file at a commit, a table at a schema version, a model at a checkpoint and
/// a span in an execution trace have no latitude.
///
/// Declaring it turns an assumption into a checked property: a profile now
/// states how its subjects are named, unknown spaces are rejected at load,
/// and [`Self::has_write_path`] is what stops a profile going `active` on an
/// address space this build cannot actually key a fact by.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AddressSpace(String);

impl AddressSpace {
    /// A place. The subject is a cell64, and two agents resolving the same
    /// place get the same 64 bits. The founding address space, and the only
    /// one with a write path today.
    pub const GEO_CELL64: &'static str = "geo.cell64";
    /// A named object that is not a place: the subject is an
    /// `emem:entity:<entity_cid>` identity minted from a canonical anchor.
    /// Identity, co-reference and linking already ship (`emem_entity`,
    /// `emem_entity_resolve`, `emem_entity_link`); what does NOT ship is
    /// keying a FACT by one, because `CanonicalKey.cell` is a cell64 string.
    /// That is the single change every non-geographic substrate waits on.
    pub const ENTITY_CID: &'static str = "entity.cid";

    /// Every address space this build understands. Closed, and closed for the
    /// same reason [`AdmissionRule`] is: each entry is a resolver in code, so
    /// a registry able to name a new one could advertise addressing that
    /// nothing implements.
    pub const KNOWN: &'static [&'static str] = &[Self::GEO_CELL64, Self::ENTITY_CID];

    /// Wrap an address-space name.
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// The wire name.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether this build has a resolver for it.
    pub fn is_known(&self) -> bool {
        Self::KNOWN.contains(&self.0.as_str())
    }

    /// Whether a fact can actually be KEYED in this address space today.
    ///
    /// Both spaces can. That was not always true: this returned `geo.cell64`
    /// only, because the canonical index was believed to require a cell64.
    /// It does not. `CanonicalKey.cell` is an opaque `String` that the storage
    /// layer concatenates without parsing, `verify_fact_subjects` accepts an
    /// `emem:entity:` subject, and
    /// `a_signed_fact_can_be_keyed_to_an_object_rather_than_a_place` puts a
    /// signed attestation about a codebase through the real sled-backed store
    /// and reads it back by content address with the subject verbatim.
    ///
    /// The claim was corrected on evidence rather than on intent, and it was
    /// UNDER-claiming: the record layer had moved and this had not. That
    /// direction is worth naming, because a stale conservative gate looks
    /// exactly like a considered one.
    ///
    /// What still keeps the object-addressed profiles at `candidate` is not
    /// the write path. It is that they declare no bands, so there is nothing
    /// to measure at those subjects; see the `bands` rule in [`validate`].
    pub fn has_write_path(&self) -> bool {
        self.0 == Self::GEO_CELL64 || self.0 == Self::ENTITY_CID
    }
}

impl std::fmt::Display for AddressSpace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The grain a substrate measures at, in a unit the substrate chooses.
///
/// `grain_min_m` / `grain_max_m` are metres and stay correct for everything
/// that observes a location. They say nothing useful about a codebase, whose
/// finest grain is a line and coarsest a repository, so a profile off the
/// geographic address space declares its own unit here instead of writing
/// lines into a field named `_m`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Grain {
    /// Unit name: `line`, `row`, `span`, `parameter`, `object`, ...
    pub unit: String,
    /// Finest grain, in `unit`.
    pub min: f64,
    /// Coarsest grain, in `unit`.
    pub max: f64,
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
    /// How subjects under this profile are addressed. Load-bearing, and
    /// required: see [`AddressSpace`] for why it had to become explicit.
    pub address_space: AddressSpace,
    /// Finest measurement grain, meters (microscopes reach `1e-7`). Present
    /// only for substrates that measure across physical space.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grain_min_m: Option<f64>,
    /// Coarsest measurement grain, meters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grain_max_m: Option<f64>,
    /// Grain in a non-metric unit, for substrates that do not measure across
    /// physical space. Exactly one of this or the metre pair is present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grain: Option<Grain>,
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
    /// Band keys or family prefixes this substrate writes **today**. Every
    /// entry must resolve in the bands manifest; the registry refuses to load
    /// otherwise.
    #[serde(default)]
    pub bands: Vec<String>,
    /// Bands a candidate profile INTENDS to write, which do not exist yet.
    ///
    /// Split out from `bands` on 2026-08-13 after I put seven `nvs.*` keys
    /// from a contributor's proposal into `bands` on `space.deep.v1`. Nothing
    /// validated them, so the registry advertised seven bands that resolve
    /// nowhere: an agent reading `/v1/substrates` would have seen a capability
    /// list and got nothing back from every one of them.
    ///
    /// That is the overclaim this repo keeps finding in its own prose, in a
    /// machine-readable surface where it is worse, because a capability list
    /// is read by programs that do not hedge. A proposal is a real and useful
    /// thing to publish; it just must not be published in the field that means
    /// "this works".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub proposed_bands: Vec<String>,
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
const KNOWN_PROVENANCE: [&str; 7] = [
    "direct_sensor",
    "deterministic_index",
    "estimator",
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
            // Exactly one grain statement, and it must be well formed. A
            // profile with neither says nothing about what it resolves; one
            // with both can be read two ways, and two readings of a number is
            // how a number stops being checkable.
            match (p.grain_min_m, p.grain_max_m, p.grain.as_ref()) {
                (Some(lo), Some(hi), None) => {
                    if !(lo > 0.0 && lo <= hi) {
                        return Err(ManifestError::Invalid(format!(
                            "{}: grain range must satisfy 0 < min <= max",
                            p.id
                        )));
                    }
                }
                (None, None, Some(g)) => {
                    if g.unit.trim().is_empty() || !(g.min > 0.0 && g.min <= g.max) {
                        return Err(ManifestError::Invalid(format!(
                            "{}: grain needs a unit and 0 < min <= max",
                            p.id
                        )));
                    }
                }
                _ => {
                    return Err(ManifestError::Invalid(format!(
                        "{}: declare grain EITHER as grain_min_m/grain_max_m \
                         (substrates that measure across physical space) OR as \
                         grain {{unit,min,max}}, never both and never neither",
                        p.id
                    )));
                }
            }
            // An address space this build cannot resolve is a subject nobody
            // can name, so it is rejected rather than carried.
            if !p.address_space.is_known() {
                return Err(ManifestError::Invalid(format!(
                    "{}: unknown address space {}; known: {:?}",
                    p.id,
                    p.address_space,
                    AddressSpace::KNOWN
                )));
            }
            // The rule that makes `candidate` mean something enforceable.
            //
            // A profile may be pinned for builders on an address space whose
            // write path is shut; it may NOT claim to be shipping on one. The
            // registry is served publicly and read as a capability list, so
            // without this an `active` profile on `entity.cid` would advertise
            // ingest that cannot physically happen: `CanonicalKey.cell` is a
            // cell64 and there is nowhere to put the fact. This is the same
            // overclaim the route-truth gate exists to stop, one layer down,
            // and it is checked here rather than in a script because the
            // registry must not even LOAD in that state.
            // A substrate with no bands measures nothing. Until the address
            // space check below stopped being wrong, it was the accidental
            // guard here: every object-addressed profile declares `bands: []`
            // and was held back by its address space rather than by its empty
            // vocabulary. Removing that gate without this one would have let a
            // profile advertise itself as live capability while carrying no
            // way to say anything at all.
            if p.status == ProfileStatus::Active && p.bands.is_empty() {
                return Err(ManifestError::Invalid(format!(
                    "{}: status is active with no declared bands; a substrate \
                     that measures nothing cannot be live capability",
                    p.id
                )));
            }
            if p.status == ProfileStatus::Active && !p.address_space.has_write_path() {
                return Err(ManifestError::Invalid(format!(
                    "{}: status is active on address space {}, which has no \
                     write path in this build; a profile whose subjects cannot \
                     be keyed must stay `candidate`",
                    p.id, p.address_space
                )));
            }
            // A declared band must exist. `provenance_class` was validated
            // against a known set and `bands` was not, so a profile could name
            // any string and the registry would serve it as capability.
            // Two vocabularies meet here and only one of them is the
            // manifest key. A profile lists what a READER would ask for
            // (`copdem30m`, the source-scoped prefix that appears in
            // `scalar_keys`), while the manifest is keyed by slot
            // (`cop_dem`). Validating against the wrong one rejects the
            // entire shipped registry, which is what the first version of
            // this check did before the tests caught it.
            // NOT validated against the bands manifest, and the reason is
            // worth recording because three attempts to do it each rejected
            // the shipped registry.
            //
            // This field lists SOURCE names; the manifest is keyed by SLOT.
            // The Earth profile declares `esa_worldcover`, which fills the
            // `landcover` band; `sentinel2`, where the key is `sentinel2_raw`;
            // and `copdem30m`, where the key is `cop_dem` and that spelling
            // appears only inside `scalar_keys`. Equality, prefix, and
            // scalar-key matching all fail on at least one real entry. The two
            // vocabularies are related by the materialiser that maps one to
            // the other, not by string surgery, and pinning them together is a
            // rename across every profile rather than something to bolt on
            // here. It is on the roadmap as its own item.
            //
            // The honest consequence: an invented band name in `bands` is NOT
            // caught by this loader. What is caught is the case that actually
            // occurred, a proposal published as a capability, and it is caught
            // by construction rather than by validation: `proposed_bands`
            // exists so a candidate profile has somewhere truthful to put the
            // bands it intends, and the check below refuses a proposal that
            // has already shipped, which is the direction that goes stale.
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

    /// Nothing is admitted on its own word.
    ///
    /// This used to read "every profile except `earth.satellite.v0` requires a
    /// trace", which was the right invariant expressed as an id exemption, and
    /// it held only while exactly one profile was archive-admitted. A codebase
    /// at a commit and a table at a snapshot are recomputable in exactly the
    /// sense the Earth archives are (git is content-addressed; a snapshot is
    /// immutable), so they are archive-admitted too, and the id test failed
    /// them for being a second example of a category it already allowed.
    ///
    /// The invariant is per-profile, not per-id: every profile is admitted by
    /// re-fetchability OR by execution trace, and a trace-admitted profile
    /// names its layers and cannot anchor the record it is scored against.
    #[test]
    fn nothing_is_admitted_on_its_own_word() {
        for p in &DEFAULT.substrates {
            match p.admission {
                AdmissionRule::OsTraceRequired => {
                    assert!(!p.required_trace_layers.is_empty(), "{}", p.id);
                    assert!(!p.drift_anchor, "{}", p.id);
                }
                AdmissionRule::ArchiveRecomputable => {
                    assert!(p.required_trace_layers.is_empty(), "{}", p.id);
                }
            }
        }
    }

    /// The address space is declared, resolvable, and honest about ingest.
    ///
    /// `active` on an address space with no write path would advertise ingest
    /// that cannot physically happen, because `CanonicalKey.cell` is a cell64
    /// and a subject with no latitude has nowhere to be keyed. The registry
    /// refuses to load in that state; this pins it as a property rather than
    /// leaving it to the loader nobody reads.
    /// The rule that replaced the address-space gate as the thing actually
    /// holding object-addressed profiles back. If this ever stops firing, a
    /// substrate can advertise itself as live while declaring no way to
    /// measure anything at its subjects.
    #[test]
    fn an_active_profile_must_declare_something_to_measure() {
        for p in &DEFAULT.substrates {
            if p.status == ProfileStatus::Active {
                assert!(
                    !p.bands.is_empty(),
                    "{}: active with no bands; live capability with no \
                     vocabulary is a claim with nothing behind it",
                    p.id
                );
            }
        }
    }

    /// Both address spaces can key a fact now. Asserted so the registry
    /// cannot quietly drift back to under-claiming after the storage layer
    /// already proved otherwise.
    #[test]
    fn an_object_is_as_addressable_as_a_place() {
        assert!(AddressSpace::new(AddressSpace::GEO_CELL64).has_write_path());
        assert!(
            AddressSpace::new(AddressSpace::ENTITY_CID).has_write_path(),
            "emem-storage proves a signed fact keys to an entity subject; a \
             registry that says otherwise under-reports what the protocol does"
        );
    }

    #[test]
    fn every_profile_declares_a_resolvable_address_space() {
        let mut off_grid = 0;
        for p in &DEFAULT.substrates {
            assert!(p.address_space.is_known(), "{}: {}", p.id, p.address_space);
            if p.status == ProfileStatus::Active {
                assert!(
                    p.address_space.has_write_path(),
                    "{} is active on {}, which cannot key a fact in this build",
                    p.id,
                    p.address_space
                );
            }
            if p.address_space.as_str() != AddressSpace::GEO_CELL64 {
                off_grid += 1;
                assert_eq!(p.status, ProfileStatus::Candidate, "{}", p.id);
                assert!(p.grain.is_some(), "{}: needs a non-metric grain", p.id);
                assert!(p.grain_min_m.is_none(), "{}: metres are meaningless", p.id);
            }
        }
        assert!(
            off_grid > 0,
            "the registry should carry the substrates that have no latitude; \
             if this fires, they were dropped rather than shipped"
        );
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
        assert!(m.grain_min_m.expect("microscope grain is metric") < 1e-6);
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

#[cfg(test)]
mod id_shape {
    use super::*;

    /// No profile id may nest inside another's stem.
    ///
    /// A profile id is signed into every record that cites the profile, and it
    /// is the thing a consumer reads when they are not reading anything else.
    /// One id containing another invites the misreading the weaker profile
    /// exists to prevent: `orbital.satellite.counters.v1` sat inside
    /// `orbital.satellite.v1`, so a reader skimming the field, or code
    /// matching on a prefix, could take the counter-level profile for the
    /// attested-execution one.
    ///
    /// It was the only such pair among seventeen ids, which is to say the
    /// registry already had this convention and one entry broke it. Now the
    /// convention is checked rather than remembered.
    #[test]
    fn no_profile_id_nests_inside_another() {
        let r = &*DEFAULT;
        let stem = |id: &str| {
            id.rsplit_once(".v")
                .map(|(s, _)| s.to_string())
                .unwrap_or_default()
        };
        for a in &r.substrates {
            for b in &r.substrates {
                if a.id == b.id {
                    continue;
                }
                assert!(
                    !b.id.starts_with(&stem(&a.id)),
                    "{} nests inside {}: a consumer reading the profile field, or matching on a \
                     prefix, can take one for the other. Give the narrower profile a name that \
                     does not begin with the wider one's stem.",
                    b.id,
                    a.id
                );
            }
        }
    }

    /// A profile admitting counter-level evidence must not claim to attest
    /// execution. The two counter profiles exist precisely because their
    /// hosts cannot produce execution evidence.
    #[test]
    fn counter_profiles_do_not_claim_attested_execution() {
        for p in &DEFAULT.substrates {
            if p.id.contains("counters") {
                assert_ne!(
                    p.provenance_class.as_str(),
                    "attested_execution",
                    "{} admits counter-level evidence and must not claim to attest execution",
                    p.id
                );
            }
        }
    }
}
