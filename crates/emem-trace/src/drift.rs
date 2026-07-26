//! Drift-anchor scoring: how a device claim is checked against the
//! founding Earth substrate.
//!
//! A verified trace proves a device really executed a capture; it does
//! not prove the sensed value describes the world. That second check is
//! the Earth substrate's job: it is independently recomputable from
//! free public archives, so it moves only when the world moves, and a
//! device claim can be scored against it. The pair is the whole trust
//! design: the trace closes the execution gap, the anchor closes the
//! world gap.
//!
//! This module is the pure scoring rule. Wiring it to a recall of the
//! anchor band at the claim's cell is ingest-side work.

use serde::{Deserialize, Serialize};

/// Verdict of one anchor check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriftVerdict {
    /// Within three standard deviations of the anchor: consistent.
    Consistent,
    /// Between three and nine standard deviations: flagged, kept, and
    /// surfaced through the contradiction index rather than dropped —
    /// a real change in the world looks exactly like this at first.
    Tension,
    /// Beyond nine standard deviations: contradicted by the anchor.
    Contradicted,
}

/// One scored comparison of a device claim against an anchor readout.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftAnchorCheck {
    /// Band both values speak about.
    pub band: String,
    /// The device's claimed value.
    pub device_value: f64,
    /// The anchor's recalled value.
    pub anchor_value: f64,
    /// The anchor's stated one-sigma uncertainty.
    pub anchor_uncertainty: f64,
    /// Score in `[0, 1]`; see [`drift_contradiction_score`].
    pub score: f64,
    /// Thresholded verdict.
    pub verdict: DriftVerdict,
}

impl DriftAnchorCheck {
    /// Score a device claim against an anchor readout.
    pub fn new(band: &str, device_value: f64, anchor_value: f64, anchor_uncertainty: f64) -> Self {
        let score = drift_contradiction_score(device_value, anchor_value, anchor_uncertainty);
        let verdict = if score < 0.5 {
            DriftVerdict::Consistent
        } else if score < 0.75 {
            DriftVerdict::Tension
        } else {
            DriftVerdict::Contradicted
        };
        DriftAnchorCheck {
            band: band.to_string(),
            device_value,
            anchor_value,
            anchor_uncertainty,
            score,
            verdict,
        }
    }
}

/// Contradiction score in `[0, 1]` between a device claim and an anchor
/// readout with one-sigma uncertainty `sigma`.
///
/// Let `z = |device - anchor| / (3 * sigma)`; the score is `z / (1 + z)`,
/// a saturating map with two pinned points: agreement at three sigma
/// scores exactly `0.5` (the consistent/tension boundary) and nine sigma
/// scores `0.75` (the tension/contradicted boundary). A non-positive or
/// non-finite `sigma` collapses the rule to exact match: `0.0` when the
/// values are equal, `1.0` otherwise, so a missing uncertainty can never
/// launder a disagreement into consistency.
pub fn drift_contradiction_score(device_value: f64, anchor_value: f64, sigma: f64) -> f64 {
    let delta = (device_value - anchor_value).abs();
    if !(sigma.is_finite() && sigma > 0.0) {
        return if delta == 0.0 { 0.0 } else { 1.0 };
    }
    let z = delta / (3.0 * sigma);
    z / (1.0 + z)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_points() {
        // Exact agreement.
        assert_eq!(drift_contradiction_score(10.0, 10.0, 1.0), 0.0);
        // Three sigma is the consistent/tension boundary.
        assert!((drift_contradiction_score(13.0, 10.0, 1.0) - 0.5).abs() < 1e-12);
        // Nine sigma is the tension/contradicted boundary.
        assert!((drift_contradiction_score(19.0, 10.0, 1.0) - 0.75).abs() < 1e-12);
    }

    #[test]
    fn missing_sigma_is_strict() {
        assert_eq!(drift_contradiction_score(5.0, 5.0, 0.0), 0.0);
        assert_eq!(drift_contradiction_score(5.0, 5.0001, 0.0), 1.0);
        assert_eq!(drift_contradiction_score(5.0, 6.0, f64::NAN), 1.0);
    }

    #[test]
    fn verdict_thresholds() {
        assert_eq!(
            DriftAnchorCheck::new("b", 10.0, 10.5, 1.0).verdict,
            DriftVerdict::Consistent
        );
        assert_eq!(
            DriftAnchorCheck::new("b", 10.0, 15.0, 1.0).verdict,
            DriftVerdict::Tension
        );
        assert_eq!(
            DriftAnchorCheck::new("b", 10.0, 40.0, 1.0).verdict,
            DriftVerdict::Contradicted
        );
    }
}
