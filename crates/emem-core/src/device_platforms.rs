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
            if p.trust_anchors.is_empty() {
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

impl DevicePlatformRegistry {
    /// Embedded v0 default.
    pub fn parse_default() -> Result<Self, ManifestError> {
        Self::parse_json(DEVICE_PLATFORMS_V0_JSON.as_bytes())
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
pub static DEFAULT: LazyLock<DevicePlatformRegistry> = LazyLock::new(|| {
    DevicePlatformRegistry::parse_default().expect("embedded device-platforms-v0.json is malformed")
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
}
