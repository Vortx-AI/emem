//! The yes/no logic.
//!
//! A first-match-wins rule pipeline over a normalised transcript. Pure: no
//! IO, no clock, no network. Everything the rules need to know arrives as
//! [`Evidence`], gathered by the caller, so this module can be exhaustively
//! tested without a server and cannot accidentally reach upstream on the
//! verdict path.
//!
//! That purity is a latency guarantee, not a style preference. The verdict
//! path has a 5000 ms org default and must behave correctly at the 1000 ms
//! floor, against a budget that covers TLS, parse, evaluation, signing and
//! logging. A rule that geocoded a place name or materialised a band would
//! blow it, and worse, would make the verdict depend on an upstream that can
//! be slow, down, or wrong.
//!
//! # The failure direction that matters
//!
//! A missed detection lets a prompt through that another control may still
//! catch. A wrong denial blocks a person's work and, at any scale, trains an
//! organisation to turn the guard off. They are not symmetric, so every rule
//! here denies only on evidence it positively established, and every
//! uncertainty resolves to proceed.

use serde::{Deserialize, Serialize};

use crate::checkpoint::Outcome;
use crate::tokens::{FoundToken, TokenKind};

/// Why a request was blocked, as a stable machine-readable code.
///
/// These strings appear in `deny_reason`, which is the only channel that
/// reaches the agent that will retry. They are part of the wire contract:
/// an agent that learns to branch on `PROV_SIG` must keep working after our
/// next release, so codes are added but never renamed or repurposed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DenyCode {
    /// A cited token's signature did not verify.
    ProvSig,
    /// A cited token resolved to different bytes than claimed.
    ProvBytes,
    /// A cited reading has drifted beyond its band's threshold since the
    /// conclusion that rests on it was formed.
    ProvDrift,
    /// A resolved reference falls inside an org-restricted zone.
    GeoZone,
    /// An unverifiable quantitative claim about physical-world state.
    ClaimUngrounded,
}

impl DenyCode {
    /// The wire form. Stable.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProvSig => "PROV_SIG",
            Self::ProvBytes => "PROV_BYTES",
            Self::ProvDrift => "PROV_DRIFT",
            Self::GeoZone => "GEO_ZONE",
            Self::ClaimUngrounded => "CLAIM_UNGROUNDED",
        }
    }
}

/// What the caller should do about it.
///
/// The point of naming a fix rather than describing a problem: an agent that
/// reads `fix=refresh_token` can act without a human, and an agent that acts
/// starts carrying valid tokens. That is the adoption path the whole gate
/// exists to create.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Fix {
    /// Re-resolve the token and retry with the current value.
    RefreshToken,
    /// Drop the citation; it cannot be made to verify.
    RemoveReference,
    /// A human decision: the org restricted this, not the evidence.
    ContactAdmin,
    /// Ground the claim: resolve the observation through emem and cite the
    /// token it returns.
    ///
    /// The only fix that asks for something to be ADDED rather than changed or
    /// dropped, and the one the whole gate exists to produce. An agent that
    /// reads this and acts starts carrying citations, which is worth more than
    /// the single request it was denied.
    CiteObservation,
}

impl Fix {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RefreshToken => "refresh_token",
            Self::RemoveReference => "remove_reference",
            Self::ContactAdmin => "contact_admin",
            Self::CiteObservation => "cite_observation",
        }
    }
}

/// The outcome of evaluating one transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub outcome: Outcome,
    pub code: Option<DenyCode>,
    /// The offending token, when one token is responsible.
    pub token: Option<String>,
    pub fix: Option<Fix>,
    /// Our transparency-log leaf for this verdict.
    ///
    /// Carried in the reason so an org can join a denial in the Activity
    /// Feed back to a signed record they can verify independently. Their
    /// audit database is testimony; the leaf is evidence.
    pub leaf: Option<String>,
    /// How many citations were checked, for the shadow report.
    pub checked: usize,
    /// The claim that triggered a `CLAIM_UNGROUNDED` denial.
    ///
    /// Never reaches the fixed reason line, which is a published grammar with
    /// no field for it. It reaches the open route, whose schema nobody else
    /// owns, so an agent calling that route learns which sentence and which
    /// band rather than only that something was ungrounded.
    pub claim: Option<crate::claim::Claim>,
}

/// The maximum length of the generated reason, before the org's own suffix.
///
/// The platform truncates `deny_reason` at 500 characters and appends org
/// text after ours, so a long reason eats the administrator's message. 200
/// keeps the machine-readable line intact with room to spare.
pub const REASON_MAX: usize = 200;

impl Decision {
    /// Let it through.
    pub fn proceed() -> Self {
        Self {
            outcome: Outcome::Proceed,
            code: None,
            token: None,
            fix: None,
            leaf: None,
            checked: 0,
            claim: None,
        }
    }

    /// Block, naming the code, the token and the fix.
    pub fn block(code: DenyCode, token: Option<String>, fix: Fix) -> Self {
        Self {
            outcome: Outcome::Block,
            code: Some(code),
            token,
            fix: Some(fix),
            leaf: None,
            checked: 0,
            claim: None,
        }
    }

    /// Attach the ungrounded claim behind a `CLAIM_UNGROUNDED` denial.
    pub fn with_claim(mut self, claim: crate::claim::Claim) -> Self {
        self.claim = Some(claim);
        self
    }

    /// Attach the log leaf this verdict was written to.
    pub fn with_leaf(mut self, leaf: impl Into<String>) -> Self {
        self.leaf = Some(leaf.into());
        self
    }

    /// Record how many citations were examined.
    pub fn with_checked(mut self, n: usize) -> Self {
        self.checked = n;
        self
    }

    /// What the caller is told, under `mode`.
    ///
    /// The decision itself is not re-derived. In shadow the same evaluation
    /// runs, the same record is signed and logged, and only the outcome
    /// returned to the checkpoint changes. The code, the fix and the claim
    /// survive on the allow, which is what lets the open route answer "this
    /// would have been denied, and here is why" without blocking anyone.
    pub fn under(self, mode: Mode) -> Self {
        match (mode, self.outcome) {
            (Mode::Shadow, Outcome::Block) => Self {
                outcome: Outcome::Proceed,
                ..self
            },
            _ => self,
        }
    }

    /// Whether a rule fired, whatever the caller was told.
    pub fn would_block(&self) -> bool {
        self.outcome == Outcome::Block || self.code.is_some()
    }

    /// Whether a denial was recorded but not acted on.
    pub fn is_shadowed(&self) -> bool {
        self.outcome == Outcome::Proceed && self.code.is_some()
    }

    /// The machine-first reason line.
    ///
    /// Grammar, fixed:
    ///
    /// ```text
    /// EMEM-GUARD DENY <CODE> token=<token|-> fix=<fix> leaf=<leaf|->
    /// ```
    ///
    /// Machine-first because the agent is the reader who can act. A human
    /// sees this followed by their administrator's standing message, which
    /// is where the human-facing guidance belongs and where the org, not us,
    /// gets to write it.
    ///
    /// The token is elided rather than truncated when it would push the line
    /// past [`REASON_MAX`]: a half-token is worse than none, because an agent
    /// would retry against an identifier that does not exist.
    pub fn reason_line(&self) -> String {
        let Some(code) = self.code else {
            return String::new();
        };
        let fix = self.fix.map(Fix::as_str).unwrap_or("contact_admin");
        let leaf = self.leaf.as_deref().unwrap_or("-");
        let token = self.token.as_deref().unwrap_or("-");
        let full = format!(
            "EMEM-GUARD DENY {} token={token} fix={fix} leaf={leaf}",
            code.as_str()
        );
        if full.len() <= REASON_MAX {
            return full;
        }
        format!(
            "EMEM-GUARD DENY {} token=- fix={fix} leaf={leaf}",
            code.as_str()
        )
    }

    /// The `reference_id` for the platform's Activity Feed.
    ///
    /// The log leaf, which is what makes the join useful: an auditor reading
    /// a denial in their feed can take this id, pull the leaf, and verify the
    /// verdict offline without asking us anything.
    pub fn reference_id(&self) -> Option<&str> {
        self.leaf.as_deref()
    }
}

/// What a cited token turned out to be, once the caller resolved it.
///
/// The caller does the IO and hands results in. That is what keeps this
/// module pure and what stops a rule from reaching upstream mid-verdict.
///
/// Not `Eq`: `Drifted` carries an f64 magnitude, and float equality is the
/// wrong relation for a physical measurement. Rules match on the VARIANT, so
/// nothing here needs to compare two drifts for exact equality anyway.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenStatus {
    /// Resolved and the signature verified over the claimed bytes.
    Verified,
    /// Resolved, but the signature does not verify.
    SignatureFailed,
    /// Resolved to different bytes than the transcript asserts.
    ByteMismatch,
    /// Not present in this node's warm cache.
    ///
    /// NOT a denial. A token minted by another responder is unresolvable
    /// here and entirely legitimate; a self-hosted node holds a subset of the
    /// corpus by design; and the verdict path is forbidden from fetching, so
    /// "cold" and "false" are indistinguishable from in here. Denying on this
    /// would block honest agents for citing something we have not cached,
    /// which is the single most likely way this product would be uninstalled.
    Unresolved,
    /// Resolved and verified, but the world has moved further than the band
    /// tolerates since this reading.
    Drifted {
        /// How far, in the band's own units, for the reason line and the
        /// shadow report.
        magnitude: f64,
    },
}

/// Everything the rules are allowed to know.
///
/// Assembled by the caller from warm cache and local state only.
#[derive(Debug, Clone, Default)]
pub struct Evidence {
    /// Each citation found, paired with what resolving it produced.
    pub tokens: Vec<(FoundToken, TokenStatus)>,
    /// cell64 addresses referenced by cited tokens that fall inside an
    /// org-restricted zone.
    ///
    /// Populated only from the LOCAL entity registry by exact match. The
    /// verdict path never geocodes: a fuzzy resolution that lands on the
    /// wrong place produces a confident block of innocent work, and a wrong
    /// deny here is the worst failure this system has.
    pub restricted_cells: Vec<String>,
    /// Measurable, anchored, assertive claims found in the transcript.
    ///
    /// Gathered unconditionally so a shadow report can count them whether or
    /// not the rule is enforcing. The rule itself needs the pair of this and
    /// `tokens`: a claim is only ungrounded when the transcript cited nothing.
    pub claims: Vec<crate::claim::Claim>,
}

/// Which rules are on.
///
/// Defaults are shadow-safe: the two provenance rules, which only fire on
/// evidence we positively established, and nothing else. Geo restriction
/// needs org configuration to mean anything, and claim gating is off because
/// it denies on the ABSENCE of evidence, which is a different and much
/// stronger claim to make about someone's prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Deny when a cited token fails signature or byte checks.
    pub provenance: bool,
    /// Deny when a cited reading has drifted past its band threshold.
    pub freshness: bool,
    /// Deny when a cited reference falls in a restricted zone.
    pub geo_restriction: bool,
    /// Deny quantitative physical-world claims that carry no citation.
    ///
    /// Off. This is the only rule that denies on absence rather than on a
    /// failed check, so it will block legitimate conversation until an org
    /// has seen its own shadow numbers and chosen the trade deliberately.
    /// [`Mode::Shadow`] is how you see those numbers without blocking anyone.
    pub claim_gating: bool,
    /// Whether a denial is acted on or only recorded.
    pub mode: Mode,
}

/// Whether the guard enforces or observes.
///
/// The rule that makes claim gating shippable at all. An org turns the rule on
/// in [`Mode::Shadow`], runs its own traffic through it for as long as it
/// likes, reads the counts back out of the log with `emem-guard --report`, and
/// only then decides whether the trade is one it wants. Nothing about the
/// evaluation changes between the two: the same decision is reached and signed,
/// and shadow differs only in what is returned to the caller.
///
/// That symmetry is deliberate. A shadow report produced by a different code
/// path than the enforcing one measures the wrong thing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// A deny is returned to the checkpoint and blocks the request.
    #[default]
    Enforce,
    /// A deny is signed and logged, and an allow is returned.
    ///
    /// The log entry records what WOULD have happened, so the report is
    /// evidence rather than an estimate.
    Shadow,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Enforce => "enforce",
            Self::Shadow => "shadow",
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            provenance: true,
            freshness: true,
            geo_restriction: false,
            claim_gating: false,
            mode: Mode::Enforce,
        }
    }
}

/// Evaluate the pipeline. First match wins; falls through to proceed.
///
/// Rule order is severity order, not convenience: a token whose signature
/// fails is a stronger statement than one that merely drifted, and an agent
/// that gets one reason per denial should be told the most serious one.
pub fn evaluate(cfg: &Config, ev: &Evidence) -> Decision {
    let checked = ev.tokens.len();

    if cfg.provenance {
        // A broken signature first: the bytes are not what was signed, which
        // is the strongest thing we can establish and the least ambiguous to
        // act on.
        for (t, st) in &ev.tokens {
            if *st == TokenStatus::SignatureFailed {
                return Decision::block(
                    DenyCode::ProvSig,
                    Some(t.token.clone()),
                    Fix::RefreshToken,
                )
                .with_checked(checked);
            }
        }
        for (t, st) in &ev.tokens {
            if *st == TokenStatus::ByteMismatch {
                // A mismatch cannot be fixed by refreshing: the token
                // resolves fine, it just does not say what the transcript
                // says it says. Re-resolving returns the same bytes again.
                return Decision::block(
                    DenyCode::ProvBytes,
                    Some(t.token.clone()),
                    Fix::RemoveReference,
                )
                .with_checked(checked);
            }
        }
    }

    if cfg.freshness {
        for (t, st) in &ev.tokens {
            if matches!(st, TokenStatus::Drifted { .. }) {
                // Drift is only meaningful for a token that names an
                // observation. An entity token identifies an object, and an
                // object does not go stale the way a reading does.
                if t.kind.resolves_to_bytes() {
                    return Decision::block(
                        DenyCode::ProvDrift,
                        Some(t.token.clone()),
                        Fix::RefreshToken,
                    )
                    .with_checked(checked);
                }
            }
        }
    }

    if cfg.geo_restriction {
        if let Some(cell) = ev.restricted_cells.first() {
            // The org restricted this, not the evidence, so the fix is a
            // human one and the reason must not imply the citation is wrong.
            return Decision::block(DenyCode::GeoZone, Some(cell.clone()), Fix::ContactAdmin)
                .with_checked(checked);
        }
    }

    if cfg.claim_gating {
        // The transcript-level condition, and the reason this rule is safe
        // enough to exist. A transcript that cited ANYTHING is the work of an
        // agent that knows how to ground itself, and gating its individual
        // sentences would need attribution this engine cannot do: which claim
        // does which citation support. Rather than guess, the rule declines to
        // fire the moment a single token is present.
        //
        // The cost is an agent that cites once and asserts ten times. That is
        // a real gap and it is the conservative side of it.
        if ev.tokens.is_empty() {
            if let Some(c) = ev.claims.first() {
                return Decision::block(DenyCode::ClaimUngrounded, None, Fix::CiteObservation)
                    .with_claim(c.clone())
                    .with_checked(checked);
            }
        }
    }

    Decision::proceed().with_checked(checked)
}

/// Tokens whose status the caller still needs to resolve.
///
/// Split out so the server can fetch statuses concurrently and bound the
/// whole set by the remaining budget rather than per token.
pub fn needs_resolution(found: &[FoundToken]) -> Vec<&FoundToken> {
    found
        .iter()
        // A cell token names an address and asserts no observation, so there
        // is nothing to verify about it and nothing that can fail.
        .filter(|t| t.kind != TokenKind::Cell)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens::TokenKind;

    fn tok(s: &str, kind: TokenKind) -> FoundToken {
        FoundToken {
            token: s.to_string(),
            kind,
        }
    }

    /// Nothing cited, nothing to object to. This is the overwhelming
    /// majority of traffic in a governed org and it must cost nothing.
    #[test]
    fn a_transcript_with_no_citations_proceeds() {
        let d = evaluate(&Config::default(), &Evidence::default());
        assert_eq!(d.outcome, Outcome::Proceed);
        assert_eq!(d.checked, 0);
        assert!(d.reason_line().is_empty());
    }

    /// Every status that is not a positively established failure proceeds.
    #[test]
    fn only_established_failures_block() {
        for st in [TokenStatus::Verified, TokenStatus::Unresolved] {
            let ev = Evidence {
                tokens: vec![(tok("emem:fact:a:b", TokenKind::Fact), st.clone())],
                ..Default::default()
            };
            let d = evaluate(&Config::default(), &ev);
            assert_eq!(d.outcome, Outcome::Proceed, "{st:?} must not block");
            assert_eq!(d.checked, 1, "it was still examined");
        }
    }

    /// A token we have not cached is indistinguishable from a token minted
    /// elsewhere. Blocking on it would deny honest agents for citing
    /// something this node has not seen.
    #[test]
    fn a_cold_token_never_blocks_however_many_there_are() {
        let ev = Evidence {
            tokens: (0..50)
                .map(|i| {
                    (
                        tok(&format!("emem:fact:c{i}:cid"), TokenKind::Fact),
                        TokenStatus::Unresolved,
                    )
                })
                .collect(),
            ..Default::default()
        };
        assert_eq!(evaluate(&Config::default(), &ev).outcome, Outcome::Proceed);
    }

    /// Severity order: the most serious finding is the one reported.
    #[test]
    fn the_most_serious_failure_is_the_one_reported() {
        let ev = Evidence {
            tokens: vec![
                (
                    tok("emem:fact:drift:x", TokenKind::Fact),
                    TokenStatus::Drifted { magnitude: 9.0 },
                ),
                (
                    tok("emem:fact:bytes:y", TokenKind::Fact),
                    TokenStatus::ByteMismatch,
                ),
                (
                    tok("emem:fact:sig:z", TokenKind::Fact),
                    TokenStatus::SignatureFailed,
                ),
            ],
            ..Default::default()
        };
        let d = evaluate(&Config::default(), &ev);
        assert_eq!(d.code, Some(DenyCode::ProvSig));
        assert_eq!(d.token.as_deref(), Some("emem:fact:sig:z"));
    }

    /// The fix must be actionable and correct: refreshing cannot repair a
    /// byte mismatch, because re-resolving returns the same bytes.
    #[test]
    fn each_failure_names_a_fix_that_would_actually_work() {
        let cases = [
            (
                TokenStatus::SignatureFailed,
                DenyCode::ProvSig,
                Fix::RefreshToken,
            ),
            (
                TokenStatus::ByteMismatch,
                DenyCode::ProvBytes,
                Fix::RemoveReference,
            ),
            (
                TokenStatus::Drifted { magnitude: 1.0 },
                DenyCode::ProvDrift,
                Fix::RefreshToken,
            ),
        ];
        for (st, code, fix) in cases {
            let ev = Evidence {
                tokens: vec![(tok("emem:fact:a:b", TokenKind::Fact), st.clone())],
                ..Default::default()
            };
            let d = evaluate(&Config::default(), &ev);
            assert_eq!(d.code, Some(code), "{st:?}");
            assert_eq!(d.fix, Some(fix), "{st:?}");
        }
    }

    /// An object identity does not go stale the way a reading does.
    #[test]
    fn drift_does_not_apply_to_an_entity_token() {
        let ev = Evidence {
            tokens: vec![(
                tok("emem:entity:abc", TokenKind::Entity),
                TokenStatus::Drifted { magnitude: 99.0 },
            )],
            ..Default::default()
        };
        assert_eq!(evaluate(&Config::default(), &ev).outcome, Outcome::Proceed);
    }

    /// Rules that need org configuration stay off until it exists, so a
    /// default install cannot block anyone on a policy nobody wrote.
    #[test]
    fn geo_and_claim_rules_are_off_by_default() {
        let cfg = Config::default();
        assert!(!cfg.geo_restriction);
        assert!(!cfg.claim_gating);
        let ev = Evidence {
            restricted_cells: vec!["defi.zb493.xuqA.zcb5f".into()],
            ..Default::default()
        };
        assert_eq!(evaluate(&cfg, &ev).outcome, Outcome::Proceed);
        let on = Config {
            geo_restriction: true,
            ..Default::default()
        };
        let d = evaluate(&on, &ev);
        assert_eq!(d.code, Some(DenyCode::GeoZone));
        // The org restricted it, so the remedy is a person, not a retry.
        assert_eq!(d.fix, Some(Fix::ContactAdmin));
    }

    /// The transcript-level condition, and the whole reason this rule is safe
    /// enough to exist. An agent that cited anything is one that knows how to
    /// ground itself, and attributing individual sentences to individual
    /// citations is a problem this engine declines to guess at.
    #[test]
    fn a_single_citation_anywhere_disarms_the_claim_gate() {
        let cfg = Config {
            claim_gating: true,
            ..Default::default()
        };
        let claim = crate::claim::scan_claims(["Elevation in Leh is 3500 m."])
            .pop()
            .expect("the fixture must be a claim");

        // Nothing cited: the rule fires.
        let bare = Evidence {
            claims: vec![claim.clone()],
            ..Default::default()
        };
        let d = evaluate(&cfg, &bare);
        assert_eq!(d.code, Some(DenyCode::ClaimUngrounded));
        // The remedy is to ADD a citation, which is the only fix in the set
        // that grows adoption rather than removing a reference.
        assert_eq!(d.fix, Some(Fix::CiteObservation));
        // And the denial carries what to go and cite.
        assert_eq!(d.claim.as_ref().unwrap().source_band, "dem");

        // One citation, even an unresolved one from another responder, and the
        // rule declines.
        let cited = Evidence {
            claims: vec![claim],
            tokens: vec![(
                tok("emem:entity:abc", TokenKind::Entity),
                TokenStatus::Unresolved,
            )],
            ..Default::default()
        };
        assert_eq!(evaluate(&cfg, &cited).outcome, Outcome::Proceed);
    }

    /// Claim gating is the lowest-severity rule: a failed check is a stronger
    /// statement than a missing one, and an agent gets one reason per denial.
    #[test]
    fn an_established_failure_outranks_a_missing_citation() {
        let cfg = Config {
            claim_gating: true,
            ..Default::default()
        };
        let ev = Evidence {
            claims: crate::claim::scan_claims(["Elevation in Leh is 3500 m."]),
            tokens: vec![(
                tok("emem:fact:a:b", TokenKind::Fact),
                TokenStatus::SignatureFailed,
            )],
            ..Default::default()
        };
        assert_eq!(evaluate(&cfg, &ev).code, Some(DenyCode::ProvSig));
    }

    /// Shadow changes what the caller is told and nothing else. The same
    /// evaluation runs, so a report measured in shadow describes the code that
    /// will run in enforcement.
    #[test]
    fn shadow_changes_the_answer_and_not_the_evaluation() {
        let ev = Evidence {
            tokens: vec![(
                tok("emem:fact:a:b", TokenKind::Fact),
                TokenStatus::SignatureFailed,
            )],
            ..Default::default()
        };
        let enforced = evaluate(&Config::default(), &ev);
        let shadowed = evaluate(
            &Config {
                mode: Mode::Shadow,
                ..Default::default()
            },
            &ev,
        )
        .under(Mode::Shadow);

        assert_eq!(enforced.outcome, Outcome::Block);
        assert_eq!(shadowed.outcome, Outcome::Proceed, "nobody is blocked");
        // Everything an auditor or an agent would want survives the downgrade.
        assert_eq!(shadowed.code, enforced.code);
        assert_eq!(shadowed.fix, enforced.fix);
        assert_eq!(shadowed.token, enforced.token);
        assert!(shadowed.would_block(), "the rule still fired");
        assert!(shadowed.is_shadowed());
        assert!(!enforced.is_shadowed());
    }

    /// The grammar is a wire contract: agents parse it to self-correct.
    #[test]
    fn the_reason_line_follows_the_fixed_grammar() {
        let d = Decision::block(
            DenyCode::ProvSig,
            Some("emem:fact:a:b".into()),
            Fix::RefreshToken,
        )
        .with_leaf("leaf_01HXPT");
        assert_eq!(
            d.reason_line(),
            "EMEM-GUARD DENY PROV_SIG token=emem:fact:a:b fix=refresh_token leaf=leaf_01HXPT"
        );
        // And the leaf doubles as the Activity Feed join id.
        assert_eq!(d.reference_id(), Some("leaf_01HXPT"));
    }

    /// A half-token would send an agent to retry an identifier that does not
    /// exist, so an over-long line drops it whole rather than truncating.
    #[test]
    fn an_overlong_reason_elides_the_token_rather_than_cutting_it() {
        let huge = format!("emem:fact:{}", "x".repeat(REASON_MAX * 2));
        let d = Decision::block(DenyCode::ProvSig, Some(huge), Fix::RefreshToken).with_leaf("l");
        let line = d.reason_line();
        assert!(line.len() <= REASON_MAX, "{} chars", line.len());
        assert!(line.contains("token=-"), "{line}");
        assert!(
            line.contains("fix=refresh_token"),
            "the remedy survives: {line}"
        );
        assert!(!line.contains("xxxx"), "no partial token: {line}");
    }

    /// A cell token asserts no observation, so there is nothing to resolve.
    #[test]
    fn a_cell_token_needs_no_resolution() {
        let found = vec![
            tok("emem:cell:defi.zb493.xuqA.zcb5f", TokenKind::Cell),
            tok("emem:fact:a:b", TokenKind::Fact),
        ];
        let need = needs_resolution(&found);
        assert_eq!(need.len(), 1);
        assert_eq!(need[0].kind, TokenKind::Fact);
    }
}
