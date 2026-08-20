//! Device-platform whitelist registry — loaded from the
//! **content-addressed device-platforms manifest**.
//!
//! The [`substrates`](crate::substrates) registry answers *what a device
//! must present* (its complete OS execution trace, under an admission
//! rule). This registry answers the question one layer beneath that:
//! **why should the record believe the key that signed the trace belongs
//! to a real device at all?**
//!
//! Today the trace verifier proves a trace is internally consistent and
//! signed by *some* ed25519 key. It cannot, on its own, tell a genuine
//! NVIDIA Orin from a laptop that generated a key and wrote
//! `platform: "jetson-orin-nx"` into a string field. The Earth substrate
//! does not need this: its trust rests on recomputability from public
//! archives. A device's reading is a one-time physical event that no
//! third party can recompute, so its analogue must be **attestation** —
//! a hardware root of trust vouches that this key runs on this measured
//! platform.
//!
//! This registry is the whitelist and the appraisal policy for that
//! attestation, following the IETF RATS architecture (RFC 9334):
//!
//! - A [`TrustAnchor`] is an **Endorsement**: a long-lived vendor key that
//!   vouches for a platform *class* ("this CA signs genuine Orins").
//!   Whitelisting a platform means pinning its anchor.
//! - `reference_values_ref` points at the **Reference Values**: the
//!   known-good boot measurements that vouch for a specific *device*
//!   ("a clean Orin measures to these digests"). Matching them is roadmap.
//! - [`RootOfTrust`] / [`EvidenceFormat`] / [`DeviceKeyKind`] name the
//!   standards a platform's evidence is expressed in (TCG DICE, IEEE
//!   802.1AR DevID, TPM 2.0 quote, Arm PSA / EAT), so the enrolment
//!   verifier knows how to appraise it.
//!
//! Honesty, as in the substrates manifest: every embedded platform is
//! [`ProfileStatus::Candidate`] and every anchor is `provisional`. We do
//! not yet hold published vendor anchors, and device ingest is not open.
//! The shape is pinned so device makers and the enrolment gate can build
//! against it; a provisional anchor whitelists nothing until replaced.

use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

use crate::manifest::{Manifest, ManifestError, MANIFEST_DEVICE_PLATFORMS};
use crate::substrates::{ContributorClass, ProfileStatus};

const DEVICE_PLATFORMS_V0_JSON: &str = include_str!("../data/device-platforms-v0.json");

/// The class of hardware root of trust a platform provides — the
/// standard the protocol appraises its evidence against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RootOfTrust {
    /// TCG DICE with an X.509 certificate chain (layered identity from an
    /// immutable hardware secret; the NVIDIA Orin class).
    TcgDiceX509,
    /// TCG DICE with CBOR/CWT certificates (constrained-device profile).
    TcgDiceCbor,
    /// TPM 2.0 measured boot: a PCR quote signed by an attestation key.
    Tpm2Quote,
    /// Arm PSA Certified initial attestation (an EAT profile).
    ArmPsa,
    /// Bare IEEE 802.1AR DevID certificate with no measured boot — a
    /// birth certificate proves the part, but not the running firmware.
    /// Lowest assurance in the registry.
    X509Devid,
    /// No hardware root of trust at all: an ordinary machine somebody runs.
    ///
    /// Named rather than omitted, because the category is enormous and leaving
    /// it out forces every cloud host and bare VM to claim a mechanism it does
    /// not have. A platform declaring this can never be admitted by a vendor
    /// endorsement, because no vendor is vouching; the only route in is an
    /// operator saying "this is my machine and I stand behind it", which is
    /// exactly what [`OperatorEndorsement`] records.
    ///
    /// It is the lowest assurance the registry can express, below
    /// [`RootOfTrust::X509Devid`], and that ordering is the point: a reader
    /// comparing two devices should be able to see which one's hardware
    /// vouched for it and which one's owner did.
    SoftwareOnly,
    /// Caliptra: an open-source silicon-level DICE root of trust (OCP).
    Caliptra,
    /// Intel TDX confidential-VM attestation (a TD quote appraised via the
    /// Intel DCAP PCK certificate chain).
    IntelTdx,
    /// AMD SEV-SNP attestation (a report signed by the VCEK, whose
    /// certificate chains to AMD's ARK/ASK).
    AmdSevSnp,
    /// Apple Secure Enclave attestation (DeviceCheck / app attest keys
    /// rooted in Apple's attestation CA).
    AppleSecureEnclave,
    /// Android hardware-backed Keystore / StrongBox key attestation (an
    /// X.509 chain to the Google hardware attestation root).
    AndroidKeystore,
}

/// The wire format the platform's attestation evidence arrives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceFormat {
    /// Entity Attestation Token as a signed CWT (RFC 9711).
    EatCwt,
    /// Entity Attestation Token as a signed JWT (RFC 9711).
    EatJwt,
    /// A raw X.509 certificate chain to the trust anchor.
    X509Chain,
    /// A TPM 2.0 quote structure with its signature and AK certificate.
    Tpm2Quote,
    /// DMTF SPDM measurement block.
    SpdmMeasurements,
}

/// What the enrolled `device_key` is, in device-identity terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKeyKind {
    /// A DICE alias key: derived per boot, its certificate attesting the
    /// hardware and the boot layers that produced it.
    DiceAlias,
    /// IEEE 802.1AR LDevID: a locally significant identity enrolled onto
    /// the device, chaining to the manufacturer IDevID / vendor CA.
    #[serde(rename = "ieee_802_1ar_ldevid")]
    Ieee8021arLdevid,
    /// IEEE 802.1AR IDevID: the manufacturer's burned-in birth identity.
    #[serde(rename = "ieee_802_1ar_idevid")]
    Ieee8021arIdevid,
    /// A TPM 2.0 attestation key certified by the TPM's endorsement key.
    Tpm2Ak,
    /// A raw ed25519 key that a specific OPERATOR has vouched for.
    ///
    /// The distinction from [`DeviceKeyKind::RawEd25519`] is not the key, it
    /// is who spoke. A bare key identifies nothing, which is why whitelisting
    /// one is refused: it would whitelist a key rather than a device. This
    /// says an operator pointed at one particular machine they hold and said
    /// so, one key at a time, and their endorsement is the evidence.
    ///
    /// It is weaker than every certificate-backed kind above and the registry
    /// keeps it that way: no hardware attests anything here, and any enrolment
    /// under it carries the operator's anchor id rather than a manufacturer's,
    /// so a reader can see exactly whose word it rests on.
    OperatorRegistered,
    /// A raw ed25519 key with no device-identity certificate — no
    /// attestation is possible; present only for completeness.
    RawEd25519,
}

/// An Endorsement: a vendor/deployment trust anchor that vouches for a
/// platform class. Pinning one is what "whitelisting a platform" means.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustAnchor {
    /// Stable anchor ID (e.g. `"nvidia.jetson.attestation-ca.v0"`).
    pub id: String,
    /// How the fingerprint is computed. The v0 enrolment verifier
    /// understands `"ed25519_pk_blake3"`: blake3 of the Endorser's raw
    /// 32-byte ed25519 public key. An anchor declaring any other scheme
    /// (e.g. a future `"spki_blake3"` over an X.509 SubjectPublicKeyInfo)
    /// is carried but not matched by the v0 rule, so it cannot admit a
    /// device under a fingerprint the verifier did not actually compute.
    pub kind: String,
    /// The anchor fingerprint, base32-nopad lowercase. A non-provisional
    /// anchor MUST decode to 32 bytes; a provisional placeholder need not.
    pub fingerprint: String,
    /// True while this is a placeholder standing in for an anchor we do
    /// not yet hold. A provisional anchor whitelists nothing: the
    /// enrolment verifier rejects evidence chained only to it.
    #[serde(default)]
    pub provisional: bool,
    /// Editorial note.
    #[serde(default, rename = "_note", skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl TrustAnchor {
    /// Whether this anchor is real enough to admit a device: not
    /// provisional and a well-formed 32-byte fingerprint.
    pub fn is_effective(&self) -> bool {
        !self.provisional && decodes_to_32_bytes(&self.fingerprint)
    }
}

/// A device family: the organizing layer above individual platforms,
/// analogous to how the Earth substrate draws from many sources. A single
/// platform (NVIDIA Orin) is one member of a family (edge-AI compute), the
/// way Sentinel-2 is one source in the Earth substrate. Families make the
/// registry legible as it grows from a handful of platforms to the full
/// device population the protocol means to integrate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceFamily {
    /// Stable family ID (e.g. `"edge_ai"`, `"trusted_host"`).
    pub id: String,
    /// Human-readable title.
    pub title: String,
    /// One-line description of what the family covers.
    pub description: String,
}

/// One whitelisted device platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevicePlatform {
    /// Platform ID (e.g. `"nvidia.jetson-orin"`).
    pub id: String,
    /// Human-readable title.
    pub title: String,
    /// The device family this platform belongs to (a declared
    /// [`DeviceFamily::id`]). Orin is one member of `edge_ai`, the way
    /// Sentinel-2 is one source in the Earth substrate.
    pub family: String,
    /// Vendor / trust-domain owner (e.g. `"nvidia"`, `"arm-psa"`).
    pub vendor: String,
    /// Lifecycle state. `Candidate` means the shape is pinned for device
    /// makers to build against; ingest under it is not open.
    pub status: ProfileStatus,
    /// Substrate contributor classes this platform can serve. An
    /// enrolment is refused if the target profile's contributor class is
    /// not in this list.
    pub contributor_classes: Vec<ContributorClass>,
    /// The hardware root of trust the platform provides.
    pub root_of_trust: RootOfTrust,
    /// The format its attestation evidence arrives in.
    pub evidence_format: EvidenceFormat,
    /// What the enrolled device key is.
    pub device_key_kind: DeviceKeyKind,
    /// Whether the platform performs measured boot (firmware is measured
    /// into the evidence, not merely a device certificate presented).
    #[serde(default)]
    pub measured_boot: bool,
    /// Endorsement anchors. At least one; an `Active` platform needs at
    /// least one effective (non-provisional) anchor.
    pub trust_anchors: Vec<TrustAnchor>,
    /// Capture encodings this platform may legitimately emit (segment
    /// `encoding` values). Cross-checked against the trace-encodings
    /// registry: every recognized encoding must be registered (asserted in
    /// tests below) and the write gate refuses a platform-enrolled trace
    /// whose segment names an encoding the platform does not emit.
    pub recognized_encodings: Vec<String>,
    /// Pointer to this platform's known-good reference-value set, when one
    /// is published; `None` until measured-boot appraisal ships.
    #[serde(default)]
    pub reference_values_ref: Option<String>,
    /// Editorial note.
    #[serde(default, rename = "_note", skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl DevicePlatform {
    /// Whether the platform can serve a given contributor class.
    pub fn serves(&self, class: &ContributorClass) -> bool {
        self.contributor_classes.contains(class)
    }

    /// Whether the platform recognises a capture encoding.
    pub fn recognizes_encoding(&self, encoding: &str) -> bool {
        self.recognized_encodings.iter().any(|e| e == encoding)
    }

    /// The effective (admittable) anchors: non-provisional, well-formed.
    pub fn effective_anchors(&self) -> impl Iterator<Item = &TrustAnchor> {
        self.trust_anchors.iter().filter(|a| a.is_effective())
    }
}

/// The device-platform manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevicePlatformRegistry {
    /// MUST equal `"emem-device-platforms"`.
    pub manifest: String,
    /// Version, e.g. `"v0"`.
    pub version: String,
    /// Device families — the organizing layer above platforms.
    #[serde(default)]
    pub families: Vec<DeviceFamily>,
    /// Platform entries.
    pub platforms: Vec<DevicePlatform>,
    /// Editorial note.
    #[serde(default, rename = "_note", skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Manifest for DevicePlatformRegistry {
    const KIND: &'static str = MANIFEST_DEVICE_PLATFORMS;

    fn validate(&self) -> Result<(), ManifestError> {
        if self.manifest != Self::KIND {
            return Err(ManifestError::WrongKind {
                expected: Self::KIND,
                actual: self.manifest.clone(),
            });
        }
        let mut fam_seen: std::collections::HashSet<&str> = Default::default();
        for f in &self.families {
            if !fam_seen.insert(&f.id) {
                return Err(ManifestError::Invalid(format!(
                    "duplicate device family: {}",
                    f.id
                )));
            }
        }
        let mut seen: std::collections::HashSet<&str> = Default::default();
        for p in &self.platforms {
            if !seen.insert(&p.id) {
                return Err(ManifestError::Invalid(format!(
                    "duplicate device platform: {}",
                    p.id
                )));
            }
            if !fam_seen.contains(p.family.as_str()) {
                return Err(ManifestError::Invalid(format!(
                    "{}: unknown device family {}",
                    p.id, p.family
                )));
            }
            if p.contributor_classes.is_empty() {
                return Err(ManifestError::Invalid(format!(
                    "{}: a platform must serve at least one contributor class",
                    p.id
                )));
            }
            // A platform with no hardware root of trust structurally has no
            // vendor anchor, because no vendor is vouching. That is different
            // from a vendor platform that forgot one, which is what this guard
            // was written to catch, so the exemption is narrow: only
            // software_only, and only while it stays candidate. It can still
            // admit a device, but exclusively through an operator endorsement
            // added at load time, which carries the operator's id rather than
            // a manufacturer's.
            let vendorless = p.root_of_trust == RootOfTrust::SoftwareOnly;
            if vendorless && p.status == ProfileStatus::Active {
                return Err(ManifestError::Invalid(format!(
                    "{}: a software_only platform cannot be active. Nothing vendor-side can \
                     ever vouch for it, so it is admitted only by an operator endorsement \
                     supplied at load time.",
                    p.id
                )));
            }
            if p.trust_anchors.is_empty() && !vendorless {
                return Err(ManifestError::Invalid(format!(
                    "{}: a whitelist entry with no trust anchor whitelists nothing",
                    p.id
                )));
            }
            if p.recognized_encodings.is_empty() {
                return Err(ManifestError::Invalid(format!(
                    "{}: a platform that recognises no encoding can emit nothing",
                    p.id
                )));
            }
            // A key kind with no attestation cannot back a whitelist.
            if p.device_key_kind == DeviceKeyKind::RawEd25519 {
                return Err(ManifestError::Invalid(format!(
                    "{}: raw_ed25519 carries no device identity; it cannot be whitelisted",
                    p.id
                )));
            }
            // A non-provisional anchor must be a well-formed 32-byte
            // fingerprint. Provisional placeholders are exempt.
            for a in &p.trust_anchors {
                if !a.provisional && !decodes_to_32_bytes(&a.fingerprint) {
                    return Err(ManifestError::Invalid(format!(
                        "{}: anchor {} is not provisional but its fingerprint is not a 32-byte digest",
                        p.id, a.id
                    )));
                }
            }
            // Honesty enforcement: you cannot ACTIVATE a platform whose
            // only anchors are placeholders. Active demands real trust.
            if p.status == ProfileStatus::Active && p.effective_anchors().count() == 0 {
                return Err(ManifestError::Invalid(format!(
                    "{}: active platform needs at least one effective (non-provisional) trust anchor",
                    p.id
                )));
            }
        }
        Ok(())
    }
}

/// An endorsement the OPERATOR of a node makes, rather than the vendor.
///
/// Why this exists, and why it is not a way around the provisional guard.
///
/// Whitelisting a platform means pinning an Endorsement: a key that vouches
/// for the class. For NVIDIA Jetson that would be NVIDIA's device-attestation
/// CA, and we do not hold its published fingerprint, so the shipped anchor is
/// a placeholder and admits nothing. That guard is right and stays.
///
/// But a vendor is not the only party who can honestly vouch for a device. On
/// a self-hosted node where the operator installed the hardware, holds it, and
/// keeps every byte local, the operator IS the trust root, and saying so out
/// loud is more honest than either pretending NVIDIA vouched or refusing to
/// admit a device the operator can physically point at.
///
/// So an operator endorsement adds a REAL anchor: non-provisional, a genuine
/// 32-byte fingerprint, matched by exactly the same v0 rule as any vendor
/// anchor. Nothing is bypassed. What differs is only who is named: an
/// enrolment admitted this way carries `endorsed_by: "operator.local.v0"`
/// rather than a vendor anchor id, so a reader can always tell which kind of
/// claim they are looking at. The platform stays `candidate`, because status
/// records whether the VENDOR has vouched and NVIDIA still has not.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatorEndorsement {
    /// The platform this operator vouches for, e.g. `"nvidia.jetson-orin"`.
    pub platform_id: String,
    /// blake3 of the operator's raw 32-byte ed25519 endorser public key,
    /// base32-nopad lowercase: the same fingerprint scheme a vendor anchor
    /// uses, because the verifier computes it the same way.
    pub fingerprint: String,
    /// Optional operator-chosen label, recorded so a multi-site deployment can
    /// tell its own anchors apart. Defaults to `operator.local.v0`.
    #[serde(default = "default_operator_anchor_id")]
    pub anchor_id: String,
    /// Editorial note, e.g. where the endorser key lives and who holds it.
    #[serde(default, rename = "_note", skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

fn default_operator_anchor_id() -> String {
    "operator.local.v0".to_string()
}

impl OperatorEndorsement {
    /// The anchor this endorsement contributes.
    ///
    /// `provisional: false` is not a shortcut: the fingerprint is checked to
    /// be a real 32-byte digest by the same `validate()` every other anchor
    /// passes, and an endorsement whose fingerprint is malformed is refused
    /// at overlay time rather than admitting anything.
    pub fn anchor(&self) -> TrustAnchor {
        TrustAnchor {
            id: self.anchor_id.clone(),
            kind: "ed25519_pk_blake3".to_string(),
            fingerprint: self.fingerprint.clone(),
            provisional: false,
            note: Some(self.note.clone().unwrap_or_else(|| {
                "Operator endorsement: this node's owner vouches for this device. \
                     Not a vendor attestation."
                    .to_string()
            })),
        }
    }
}

impl DevicePlatformRegistry {
    /// Embedded v0 default.
    pub fn parse_default() -> Result<Self, ManifestError> {
        Self::parse_json(DEVICE_PLATFORMS_V0_JSON.as_bytes())
    }

    /// Return a copy of this registry with operator endorsements overlaid.
    ///
    /// Errors rather than silently dropping: an endorsement naming a platform
    /// that does not exist, or carrying a fingerprint that is not a 32-byte
    /// digest, is a misconfiguration the operator needs told about. A node
    /// that quietly ignored it would look enrolled and admit nothing.
    pub fn with_operator_endorsements(
        &self,
        endorsements: &[OperatorEndorsement],
    ) -> Result<Self, ManifestError> {
        let mut out = self.clone();
        for e in endorsements {
            if !decodes_to_32_bytes(&e.fingerprint) {
                return Err(ManifestError::Invalid(format!(
                    "operator endorsement for {}: fingerprint is not a 32-byte digest. \
                     It must be blake3 of the endorser's raw ed25519 public key, \
                     base32-nopad lowercase.",
                    e.platform_id
                )));
            }
            let Some(p) = out.platforms.iter_mut().find(|p| p.id == e.platform_id) else {
                return Err(ManifestError::Invalid(format!(
                    "operator endorsement names platform {}, which is not in the registry. \
                     Read GET /v1/device_platforms for the ids that exist.",
                    e.platform_id
                )));
            };
            // Replacing rather than appending a duplicate id keeps re-running
            // an install idempotent instead of growing the anchor list.
            p.trust_anchors.retain(|a| a.id != e.anchor_id);
            p.trust_anchors.push(e.anchor());
        }
        out.validate()?;
        Ok(out)
    }

    /// Look up a platform by ID.
    pub fn lookup(&self, id: &str) -> Option<&DevicePlatform> {
        self.platforms.iter().find(|p| p.id == id)
    }

    /// Platforms that can serve a given contributor class.
    pub fn platforms_for_class<'a>(
        &'a self,
        class: &'a ContributorClass,
    ) -> impl Iterator<Item = &'a DevicePlatform> {
        self.platforms.iter().filter(move |p| p.serves(class))
    }

    /// Look up a family by ID.
    pub fn family(&self, id: &str) -> Option<&DeviceFamily> {
        self.families.iter().find(|f| f.id == id)
    }

    /// Platforms in a given family.
    pub fn platforms_in_family<'a>(
        &'a self,
        family_id: &'a str,
    ) -> impl Iterator<Item = &'a DevicePlatform> {
        self.platforms.iter().filter(move |p| p.family == family_id)
    }
}

/// Whether a base32-nopad lowercase string decodes to exactly 32 bytes.
fn decodes_to_32_bytes(s: &str) -> bool {
    data_encoding::BASE32_NOPAD
        .decode(s.to_uppercase().as_bytes())
        .map(|b| b.len() == 32)
        .unwrap_or(false)
}

/// Process-wide cached default registry.
/// Where an operator declares the devices they personally vouch for.
///
/// A path, read once at first use. Absent, the registry is exactly the
/// embedded one and admits nothing, which is the state every node ships in.
pub const OPERATOR_ENDORSEMENTS_ENV: &str = "EMEM_OPERATOR_ENDORSEMENTS";

/// The device-platform registry this node uses.
///
/// The embedded manifest, plus any endorsements the operator of THIS node has
/// declared. That overlay is the only route by which anything is ever
/// admitted: every anchor shipped in the manifest is provisional, so a stock
/// node whitelists nothing, and it stays that way until an operator says
/// otherwise about a machine they hold.
///
/// Two consequences worth stating rather than discovering. A node with
/// endorsements has a different `manifest_cid` from the published one, which
/// is correct: its registry genuinely differs, and pretending otherwise would
/// hide the difference that matters. And a malformed endorsements file is
/// logged and IGNORED rather than fatal, because a node that refuses to boot
/// over a roster file is a node that took its own availability hostage to a
/// convenience.
pub static DEFAULT: LazyLock<DevicePlatformRegistry> = LazyLock::new(|| {
    let base = DevicePlatformRegistry::parse_default()
        .expect("embedded device-platforms-v0.json is malformed");
    let Ok(path) = std::env::var(OPERATOR_ENDORSEMENTS_ENV) else {
        return base;
    };
    let raw = match std::fs::read(&path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!(
                "emem: {OPERATOR_ENDORSEMENTS_ENV}={path} could not be read ({e}); \
                       continuing with no operator endorsements, so this node admits no device"
            );
            return base;
        }
    };
    let endorsements: Vec<OperatorEndorsement> = match serde_json::from_slice(&raw) {
        Ok(e) => e,
        Err(e) => {
            eprintln!(
                "emem: {path} is not a list of operator endorsements ({e}); \
                       continuing with none, so this node admits no device"
            );
            return base;
        }
    };
    match base.with_operator_endorsements(&endorsements) {
        Ok(overlaid) => {
            eprintln!(
                "emem: {} operator endorsement(s) loaded from {path}; \
                 devices admitted under them carry the operator's anchor id, not a vendor's",
                endorsements.len()
            );
            overlaid
        }
        Err(e) => {
            eprintln!(
                "emem: operator endorsements in {path} were refused ({e}); \
                       continuing with none"
            );
            base
        }
    }
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_loads_and_has_orin() {
        let r = &*DEFAULT;
        let orin = r.lookup("nvidia.jetson-orin").expect("orin platform");
        assert_eq!(orin.vendor, "nvidia");
        assert_eq!(orin.root_of_trust, RootOfTrust::TcgDiceX509);
        assert_eq!(orin.device_key_kind, DeviceKeyKind::DiceAlias);
        assert!(orin.measured_boot);
        assert!(orin.serves(&ContributorClass::new(ContributorClass::ROBOT)));
        assert!(orin.recognizes_encoding("nvidia.nsys.v1"));
    }

    #[test]
    fn every_platform_is_candidate_and_anchors_are_provisional() {
        // The honest-state invariant: nothing is live yet, so the whole
        // registry admits no real device. A future PR that flips a status
        // to active without pinning a real anchor fails validate().
        for p in &DEFAULT.platforms {
            assert_eq!(
                p.status,
                ProfileStatus::Candidate,
                "{}: expected candidate until a real anchor is pinned",
                p.id
            );
            assert_eq!(
                p.effective_anchors().count(),
                0,
                "{}: no anchor should be effective while provisional",
                p.id
            );
        }
    }

    #[test]
    fn active_platform_without_real_anchor_is_rejected() {
        let mut r = DEFAULT.clone();
        r.platforms[0].status = ProfileStatus::Active;
        assert!(
            r.validate().is_err(),
            "activating a platform whose only anchor is provisional must fail"
        );
    }

    #[test]
    fn real_anchor_lets_a_platform_activate() {
        let mut r = DEFAULT.clone();
        // A well-formed 32-byte fingerprint (blake3 of some bytes).
        let real = data_encoding::BASE32_NOPAD
            .encode(blake3::hash(b"a real vendor anchor").as_bytes())
            .to_lowercase();
        r.platforms[0].trust_anchors[0].provisional = false;
        r.platforms[0].trust_anchors[0].fingerprint = real;
        r.platforms[0].status = ProfileStatus::Active;
        assert!(r.validate().is_ok());
        assert_eq!(r.platforms[0].effective_anchors().count(), 1);
    }

    #[test]
    fn non_provisional_anchor_needs_a_valid_fingerprint() {
        let mut r = DEFAULT.clone();
        r.platforms[0].trust_anchors[0].provisional = false; // still the placeholder string
        assert!(
            r.validate().is_err(),
            "a non-provisional anchor with a malformed fingerprint must fail"
        );
    }

    #[test]
    fn duplicate_platform_id_is_rejected() {
        let mut r = DEFAULT.clone();
        let dup = r.platforms[0].clone();
        r.platforms.push(dup);
        assert!(r.validate().is_err());
    }

    #[test]
    fn raw_ed25519_cannot_be_whitelisted() {
        let mut r = DEFAULT.clone();
        r.platforms[0].device_key_kind = DeviceKeyKind::RawEd25519;
        assert!(r.validate().is_err());
    }

    #[test]
    fn manifest_cid_is_stable_across_reparse() {
        let a = crate::manifest::manifest_cid(&*DEFAULT).expect("cid");
        let b =
            crate::manifest::manifest_cid(&DevicePlatformRegistry::parse_default().expect("parse"))
                .expect("cid");
        assert_eq!(a, b);
    }

    #[test]
    fn platforms_for_class_finds_orin_for_robot() {
        let n = DEFAULT
            .platforms_for_class(&ContributorClass::new(ContributorClass::ROBOT))
            .count();
        assert!(n >= 1);
    }

    #[test]
    fn orin_is_one_member_of_the_edge_ai_family() {
        // The whole point of the family layer: Orin is one of several, not
        // the substrate itself.
        let orin = DEFAULT.lookup("nvidia.jetson-orin").expect("orin");
        assert_eq!(orin.family, "edge_ai");
        assert!(DEFAULT.family("edge_ai").is_some());
        let edge = DEFAULT.platforms_in_family("edge_ai").count();
        assert!(
            edge >= 2,
            "edge_ai should have more than just Orin, got {edge}"
        );
    }

    #[test]
    fn every_platform_names_a_declared_family() {
        // validate() enforces this, but assert it holds for the shipped
        // registry so a stray family typo is caught in unit tests too.
        for p in &DEFAULT.platforms {
            assert!(
                DEFAULT.family(&p.family).is_some(),
                "{} names undeclared family {}",
                p.id,
                p.family
            );
        }
        assert!(
            DEFAULT.families.len() >= 5,
            "expected several device families"
        );
    }

    #[test]
    fn platform_in_an_undeclared_family_is_rejected() {
        let mut r = DEFAULT.clone();
        r.platforms[0].family = "no_such_family".into();
        assert!(r.validate().is_err());
    }

    #[test]
    fn every_recognized_encoding_is_registered() {
        // The cross-manifest invariant: a platform cannot advertise a
        // capture encoding the trace-encodings registry does not define,
        // or the write gate would reference a vocabulary that isn't there.
        let enc = &*crate::trace_encodings::DEFAULT;
        for p in &DEFAULT.platforms {
            for e in &p.recognized_encodings {
                assert!(
                    enc.recognizes(e),
                    "{} advertises unregistered encoding {e}",
                    p.id
                );
            }
        }
    }

    /// The vendorless tier exists, and cannot pretend to be more than it is.
    ///
    /// A machine with no TPM and no DICE is the commonest thing that will ever
    /// run this software. Leaving the category out of the registry would force
    /// every such host to borrow a mechanism it does not have, which is the
    /// one failure a provenance registry must not enable.
    #[test]
    fn a_vendorless_platform_is_admittable_only_by_its_operator() {
        let base = DevicePlatformRegistry::parse_default().expect("registry");
        let host = base
            .lookup("generic.linux-host")
            .expect("the vendorless tier is registered");

        assert_eq!(host.root_of_trust, RootOfTrust::SoftwareOnly);
        assert!(!host.measured_boot, "there is no measured boot to claim");
        assert!(
            host.trust_anchors.is_empty(),
            "no vendor is vouching, so there is no vendor anchor to carry"
        );
        assert_eq!(
            host.effective_anchors().count(),
            0,
            "and therefore it admits nothing on its own"
        );
        assert_eq!(host.status, ProfileStatus::Candidate);

        // The only route in is an operator saying so, and the record shows it.
        let fp = data_encoding::BASE32_NOPAD
            .encode(blake3::hash(b"this operator's endorser key").as_bytes())
            .to_lowercase();
        let overlaid = base
            .with_operator_endorsements(&[OperatorEndorsement {
                platform_id: "generic.linux-host".to_string(),
                fingerprint: fp,
                anchor_id: default_operator_anchor_id(),
                note: None,
            }])
            .expect("an operator may vouch for their own machine");
        let host = overlaid.lookup("generic.linux-host").unwrap();
        let effective: Vec<_> = host.effective_anchors().collect();
        assert_eq!(effective.len(), 1);
        assert_eq!(
            effective[0].id, "operator.local.v0",
            "the anchor names the operator, never a manufacturer"
        );
    }

    /// An operator endorsement makes an otherwise-inert platform admittable,
    /// and says whose word it is standing on.
    #[test]
    fn operator_endorsement_creates_an_effective_anchor() {
        let base = DevicePlatformRegistry::parse_default().expect("default registry");
        let orin = base
            .lookup("nvidia.jetson-orin")
            .expect("orin is registered");
        // Shipped state: NVIDIA's anchor is a placeholder, so nothing is
        // admittable. This is the guard the operator path must not weaken.
        assert_eq!(
            orin.effective_anchors().count(),
            0,
            "the shipped Orin anchor must admit nothing"
        );

        let fp = data_encoding::BASE32_NOPAD
            .encode(blake3::hash(b"an operator endorser key").as_bytes())
            .to_lowercase();
        let e = OperatorEndorsement {
            platform_id: "nvidia.jetson-orin".to_string(),
            fingerprint: fp.clone(),
            anchor_id: default_operator_anchor_id(),
            note: None,
        };
        let overlaid = base
            .with_operator_endorsements(std::slice::from_ref(&e))
            .expect("overlay validates");
        let orin = overlaid.lookup("nvidia.jetson-orin").expect("still there");
        let effective: Vec<_> = orin.effective_anchors().collect();
        assert_eq!(effective.len(), 1, "operator anchor is now effective");
        assert_eq!(effective[0].id, "operator.local.v0");
        assert_eq!(effective[0].kind, "ed25519_pk_blake3");
        assert_eq!(effective[0].fingerprint, fp);

        // The vendor's placeholder is untouched, and the platform has NOT
        // been promoted: status still records that NVIDIA has not vouched.
        assert!(
            orin.trust_anchors.iter().any(|a| a.provisional),
            "the vendor placeholder must survive the overlay"
        );
        assert_eq!(
            orin.status,
            base.lookup("nvidia.jetson-orin").unwrap().status
        );

        // The base registry is unchanged: overlaying returns a copy.
        assert_eq!(
            base.lookup("nvidia.jetson-orin")
                .unwrap()
                .effective_anchors()
                .count(),
            0
        );
    }

    /// Re-running an install must not grow the anchor list.
    #[test]
    fn operator_endorsement_is_idempotent() {
        let base = DevicePlatformRegistry::parse_default().unwrap();
        let fp = data_encoding::BASE32_NOPAD
            .encode(blake3::hash(b"k").as_bytes())
            .to_lowercase();
        let e = OperatorEndorsement {
            platform_id: "nvidia.jetson-orin".to_string(),
            fingerprint: fp,
            anchor_id: default_operator_anchor_id(),
            note: None,
        };
        let once = base
            .with_operator_endorsements(std::slice::from_ref(&e))
            .unwrap();
        let twice = once
            .with_operator_endorsements(std::slice::from_ref(&e))
            .unwrap();
        let n = twice
            .lookup("nvidia.jetson-orin")
            .unwrap()
            .trust_anchors
            .iter()
            .filter(|a| a.id == "operator.local.v0")
            .count();
        assert_eq!(n, 1, "re-endorsing replaces rather than appends");
    }

    /// A misconfigured endorsement is reported, never silently ignored.
    #[test]
    fn operator_endorsement_refuses_bad_input() {
        let base = DevicePlatformRegistry::parse_default().unwrap();
        let good = data_encoding::BASE32_NOPAD
            .encode(blake3::hash(b"k").as_bytes())
            .to_lowercase();

        let short = OperatorEndorsement {
            platform_id: "nvidia.jetson-orin".to_string(),
            fingerprint: "abc".to_string(),
            anchor_id: default_operator_anchor_id(),
            note: None,
        };
        assert!(
            base.with_operator_endorsements(&[short]).is_err(),
            "a fingerprint that is not 32 bytes must be refused"
        );

        let unknown = OperatorEndorsement {
            platform_id: "nvidia.no-such-board".to_string(),
            fingerprint: good,
            anchor_id: default_operator_anchor_id(),
            note: None,
        };
        assert!(
            base.with_operator_endorsements(&[unknown]).is_err(),
            "an endorsement for an unregistered platform must be refused"
        );
    }
}
