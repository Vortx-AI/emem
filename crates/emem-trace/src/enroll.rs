//! Enrolment evidence: the platform attestation that turns a self-minted
//! device key into an attested device identity, and its verifier.
//!
//! [`super::verify_os_trace`] proves a trace is internally consistent and
//! signed by *some* key. It cannot, on its own, tell a genuine device from
//! a laptop that minted a key. This module closes that gap at enrolment
//! time, following the IETF RATS architecture (RFC 9334):
//!
//! - The device is the **Attester**. Its platform's hardware root of trust
//!   produces **Evidence** — here a [`PlatformAttestation`], shaped after
//!   an EAT (RFC 9711): it carries the endorsed device key (the `ueid`),
//!   the hardware model and OEM, a freshness nonce, and the boot
//!   measurements, all signed by an **Endorser**.
//! - The Endorser is a trust anchor from the device-platforms manifest —
//!   a vendor key that vouches for a platform *class*. Its public key is
//!   committed to by an effective [`TrustAnchor`](emem_core::device_platforms::TrustAnchor)
//!   fingerprint on the platform.
//! - emem is the **Verifier + Relying Party**: [`verify_platform_attestation`]
//!   appraises the evidence against the platform's endorsements and admits
//!   the key only when a whitelisted anchor signed for it.
//!
//! v0 scope (deliberately small, no X.509 parser): the Endorser is an
//! ed25519 key whose blake3 the anchor fingerprint commits to, and the
//! evidence is one ed25519 signature over
//! [`emem_attest::platform_attestation_preimage_v1`]. Full X.509 / DICE
//! chain walking and reference-value matching of the boot measurements are
//! the v1 step. Because every anchor shipped today is provisional, the
//! verifier admits nothing yet: [`RejectReason::NoEffectiveAnchor`] fires
//! for every real platform, which is exactly the safe, no-new-admissions
//! property this stage is meant to have.

use data_encoding::BASE32_NOPAD;
use ed25519_dalek::{Signer, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

use emem_core::device_platforms::DevicePlatform;
use emem_core::key::{AttesterKey, Signature};

/// The EAT profile every v0 platform attestation pins.
pub const PLATFORM_ATTESTATION_V0: &str = "emem.platform_attestation.v0";

/// One platform attestation: an Endorser's signed statement that
/// `device_key` runs on a genuine, measured instance of `platform_id`.
/// EAT-shaped (RFC 9711): `device_key` is the `ueid`, `hwmodel`/`oemid`
/// are the corresponding EAT claims, `measurements` are the boot
/// measurements, `nonce` is the freshness challenge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformAttestation {
    /// MUST equal [`PLATFORM_ATTESTATION_V0`].
    pub eat_profile: String,
    /// Device-platform ID this evidence attests to (must match the
    /// platform the key is being enrolled under).
    pub platform_id: String,
    /// The endorsed device key — the key being enrolled. Must equal the
    /// enrollee.
    pub device_key: AttesterKey,
    /// EAT `hwmodel` claim (e.g. `"jetson-orin-nx"`).
    pub hwmodel: String,
    /// EAT `oemid` claim.
    pub oemid: String,
    /// Freshness nonce echoed from the enrolment challenge. Checked
    /// against a live challenge in a later stage; carried now so the
    /// signed preimage already binds it.
    pub nonce: String,
    /// Boot measurement digests (base32-nopad lowercase). Matched against
    /// the platform's reference values when those ship; carried and
    /// signed now.
    #[serde(default)]
    pub measurements: Vec<String>,
    /// The Endorser's ed25519 public key — the trust anchor that signs
    /// this evidence. A whitelisted platform anchor commits to its blake3.
    pub endorser_key: AttesterKey,
    /// The Endorser's ed25519 signature over
    /// [`emem_attest::platform_attestation_preimage_v1`].
    pub endorser_sig: Signature,
}

impl PlatformAttestation {
    /// Content ID of the whole signed attestation:
    /// `base32(blake3(canonical_cbor(self)))`, the same rule facts and
    /// traces use. This is the CID an `emem:attestation:` token names.
    pub fn attestation_cid(&self) -> Option<String> {
        let mut buf = Vec::new();
        ciborium::ser::into_writer(self, &mut buf).ok()?;
        Some(
            data_encoding::BASE32_NOPAD
                .encode(blake3::hash(&buf).as_bytes())
                .to_lowercase(),
        )
    }

    /// The `emem:attestation:` token for this evidence, if it encodes.
    pub fn token(&self) -> Option<String> {
        Some(crate::token::attestation_token(&self.attestation_cid()?))
    }

    /// The 32 bytes the Endorser key signs.
    pub fn preimage(&self) -> [u8; 32] {
        emem_attest::platform_attestation_preimage_v1(
            &self.eat_profile,
            &self.platform_id,
            &self.device_key.0,
            &self.hwmodel,
            &self.oemid,
            &self.nonce,
            self.measurements.iter().map(|s| s.as_str()),
        )
    }

    /// Build and sign a v0 attestation with an Endorser's secret key —
    /// for anchors, device-maker tooling, and tests.
    #[allow(clippy::too_many_arguments)]
    pub fn build_and_sign_v0(
        platform_id: &str,
        device_key: AttesterKey,
        hwmodel: &str,
        oemid: &str,
        nonce: &str,
        measurements: Vec<String>,
        endorser: &ed25519_dalek::SigningKey,
    ) -> Self {
        let mut att = PlatformAttestation {
            eat_profile: PLATFORM_ATTESTATION_V0.to_string(),
            platform_id: platform_id.to_string(),
            device_key,
            hwmodel: hwmodel.to_string(),
            oemid: oemid.to_string(),
            nonce: nonce.to_string(),
            measurements,
            endorser_key: AttesterKey(endorser.verifying_key().to_bytes()),
            endorser_sig: Signature([0u8; 64]),
        };
        att.endorser_sig = Signature(endorser.sign(&att.preimage()).to_bytes());
        att
    }
}

/// Why a platform attestation failed. Every variant names the offending
/// part, in the same collect-all style as [`super::verify::RejectReason`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum RejectReason {
    /// `eat_profile` is not the one this verifier understands.
    #[error("unknown attestation profile: {profile}")]
    ProfileUnknown {
        /// The profile string the evidence carried.
        profile: String,
    },
    /// The evidence attests to a different platform than the one enrolled.
    #[error("attestation claims platform {claimed}, enrolling under {expected}")]
    PlatformMismatch {
        /// Platform the evidence claims.
        claimed: String,
        /// Platform the key is being enrolled under.
        expected: String,
    },
    /// The evidence endorses a different key than the one being enrolled.
    #[error("attestation endorses a different device key than the enrollee")]
    DeviceKeyMismatch,
    /// The platform has no effective (non-provisional, well-formed) anchor,
    /// so nothing can be admitted under it yet. This is the state of every
    /// shipped platform today.
    #[error("platform {platform} has no effective trust anchor; it admits nothing yet")]
    NoEffectiveAnchor {
        /// The platform in question.
        platform: String,
    },
    /// The Endorser key is not committed to by any effective anchor of the
    /// platform: whoever signed the evidence is not whitelisted.
    #[error("endorser is not a whitelisted anchor of platform {platform}")]
    EndorserNotWhitelisted {
        /// The platform in question.
        platform: String,
    },
    /// The Endorser's signature over the evidence does not verify.
    #[error("endorser signature invalid")]
    EndorserSignatureInvalid,
}

/// Admit or reject.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Every check passed; the device key may be enrolled under the platform.
    Admit,
    /// At least one check failed; the key must not be enrolled.
    Reject,
}

/// The full appraisal outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrollmentReport {
    /// Admit only when `reasons` is empty.
    pub verdict: Verdict,
    /// Every failed check.
    pub reasons: Vec<RejectReason>,
    /// The anchor ID that endorsed the key, on admit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endorsed_by: Option<String>,
}

/// Appraise a platform attestation for `expected_device_key` against
/// `platform`'s endorsements. Collects every failure, in the same spirit
/// as [`super::verify_os_trace`]: an admit means a whitelisted anchor
/// signed fresh evidence binding exactly the enrollee key to a platform it
/// serves.
pub fn verify_platform_attestation(
    platform: &DevicePlatform,
    att: &PlatformAttestation,
    expected_device_key: &AttesterKey,
) -> EnrollmentReport {
    let mut reasons: Vec<RejectReason> = Vec::new();

    if att.eat_profile != PLATFORM_ATTESTATION_V0 {
        reasons.push(RejectReason::ProfileUnknown {
            profile: att.eat_profile.clone(),
        });
    }
    if att.platform_id != platform.id {
        reasons.push(RejectReason::PlatformMismatch {
            claimed: att.platform_id.clone(),
            expected: platform.id.clone(),
        });
    }
    if att.device_key != *expected_device_key {
        reasons.push(RejectReason::DeviceKeyMismatch);
    }

    // The Endorser must be committed to by an effective anchor. blake3 of
    // the endorser's ed25519 public key is the v0 anchor fingerprint.
    let endorser_fp = BASE32_NOPAD
        .encode(blake3::hash(&att.endorser_key.0).as_bytes())
        .to_lowercase();
    let matched = platform
        .effective_anchors()
        .find(|a| a.fingerprint == endorser_fp);
    let mut endorsed_by = None;
    match matched {
        Some(anchor) => endorsed_by = Some(anchor.id.clone()),
        None => {
            // Distinguish "nothing could ever match yet" from "a key
            // signed but is not the whitelisted one" — different fixes for
            // whoever is debugging an enrolment.
            if platform.effective_anchors().count() == 0 {
                reasons.push(RejectReason::NoEffectiveAnchor {
                    platform: platform.id.clone(),
                });
            } else {
                reasons.push(RejectReason::EndorserNotWhitelisted {
                    platform: platform.id.clone(),
                });
            }
        }
    }

    // The Endorser signature must verify over the evidence preimage.
    match VerifyingKey::from_bytes(&att.endorser_key.0) {
        Ok(vk) => {
            let sig = ed25519_dalek::Signature::from_bytes(&att.endorser_sig.0);
            if vk.verify(&att.preimage(), &sig).is_err() {
                reasons.push(RejectReason::EndorserSignatureInvalid);
            }
        }
        Err(_) => reasons.push(RejectReason::EndorserSignatureInvalid),
    }

    let verdict = if reasons.is_empty() {
        Verdict::Admit
    } else {
        Verdict::Reject
    };
    EnrollmentReport {
        verdict,
        reasons,
        endorsed_by: if verdict == Verdict::Admit {
            endorsed_by
        } else {
            None
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use emem_core::device_platforms::DEFAULT;

    fn endorser_and_platform_with_real_anchor() -> (SigningKey, DevicePlatform) {
        // Take Orin, replace its provisional anchor with a real one that
        // commits to a known endorser key.
        let endorser = SigningKey::from_bytes(&[3u8; 32]);
        let fp = BASE32_NOPAD
            .encode(blake3::hash(&endorser.verifying_key().to_bytes()).as_bytes())
            .to_lowercase();
        let mut p = DEFAULT.lookup("nvidia.jetson-orin").expect("orin").clone();
        p.trust_anchors[0].provisional = false;
        p.trust_anchors[0].fingerprint = fp;
        (endorser, p)
    }

    fn device_key(seed: u8) -> (SigningKey, AttesterKey) {
        let sk = SigningKey::from_bytes(&[seed; 32]);
        let ak = AttesterKey(sk.verifying_key().to_bytes());
        (sk, ak)
    }

    #[test]
    fn shipped_platforms_admit_nothing_because_anchors_are_provisional() {
        // The safe property of this stage: with the real (provisional)
        // registry, no attestation can be admitted for any platform.
        let endorser = SigningKey::from_bytes(&[3u8; 32]);
        let (_dsk, dk) = device_key(7);
        for platform in &DEFAULT.platforms {
            let att = PlatformAttestation::build_and_sign_v0(
                &platform.id,
                dk,
                "some-model",
                "some-oem",
                "nonce-abc",
                vec![],
                &endorser,
            );
            let report = verify_platform_attestation(platform, &att, &dk);
            assert_eq!(report.verdict, Verdict::Reject, "{}", platform.id);
            assert!(
                report
                    .reasons
                    .iter()
                    .any(|r| matches!(r, RejectReason::NoEffectiveAnchor { .. })),
                "{}: expected no_effective_anchor, got {:?}",
                platform.id,
                report.reasons
            );
        }
    }

    #[test]
    fn a_whitelisted_endorser_admits_the_bound_key() {
        let (endorser, platform) = endorser_and_platform_with_real_anchor();
        let (_dsk, dk) = device_key(9);
        let att = PlatformAttestation::build_and_sign_v0(
            &platform.id,
            dk,
            "jetson-orin-nx",
            "nvidia",
            "nonce-xyz",
            vec![],
            &endorser,
        );
        let report = verify_platform_attestation(&platform, &att, &dk);
        assert_eq!(report.verdict, Verdict::Admit, "{:?}", report.reasons);
        assert_eq!(
            report.endorsed_by.as_deref(),
            Some("nvidia.jetson.attestation-ca.v0")
        );
    }

    #[test]
    fn a_non_whitelisted_endorser_is_rejected() {
        let (_good, platform) = endorser_and_platform_with_real_anchor();
        let stranger = SigningKey::from_bytes(&[99u8; 32]);
        let (_dsk, dk) = device_key(9);
        let att = PlatformAttestation::build_and_sign_v0(
            &platform.id,
            dk,
            "jetson-orin-nx",
            "nvidia",
            "n",
            vec![],
            &stranger,
        );
        let report = verify_platform_attestation(&platform, &att, &dk);
        assert_eq!(report.verdict, Verdict::Reject);
        assert!(report
            .reasons
            .iter()
            .any(|r| matches!(r, RejectReason::EndorserNotWhitelisted { .. })));
    }

    #[test]
    fn evidence_for_a_different_key_is_rejected() {
        let (endorser, platform) = endorser_and_platform_with_real_anchor();
        let (_dsk, dk) = device_key(9);
        let (_osk, other) = device_key(10);
        let att = PlatformAttestation::build_and_sign_v0(
            &platform.id,
            dk,
            "jetson-orin-nx",
            "nvidia",
            "n",
            vec![],
            &endorser,
        );
        // Enrolling `other`, but the evidence endorses `dk`.
        let report = verify_platform_attestation(&platform, &att, &other);
        assert_eq!(report.verdict, Verdict::Reject);
        assert!(report
            .reasons
            .iter()
            .any(|r| matches!(r, RejectReason::DeviceKeyMismatch)));
    }

    #[test]
    fn a_tampered_signature_is_rejected() {
        let (endorser, platform) = endorser_and_platform_with_real_anchor();
        let (_dsk, dk) = device_key(9);
        let mut att = PlatformAttestation::build_and_sign_v0(
            &platform.id,
            dk,
            "jetson-orin-nx",
            "nvidia",
            "n",
            vec![],
            &endorser,
        );
        // Flip a claim after signing.
        att.hwmodel = "not-an-orin".into();
        let report = verify_platform_attestation(&platform, &att, &dk);
        assert_eq!(report.verdict, Verdict::Reject);
        assert!(report
            .reasons
            .iter()
            .any(|r| matches!(r, RejectReason::EndorserSignatureInvalid)));
    }

    #[test]
    fn platform_mismatch_is_named() {
        let (endorser, platform) = endorser_and_platform_with_real_anchor();
        let (_dsk, dk) = device_key(9);
        let att = PlatformAttestation::build_and_sign_v0(
            "tpm2.host", // wrong platform in the evidence
            dk,
            "m",
            "o",
            "n",
            vec![],
            &endorser,
        );
        let report = verify_platform_attestation(&platform, &att, &dk);
        assert_eq!(report.verdict, Verdict::Reject);
        assert!(report
            .reasons
            .iter()
            .any(|r| matches!(r, RejectReason::PlatformMismatch { .. })));
    }
}
