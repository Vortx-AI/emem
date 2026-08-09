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
    /// A sentence states a figure that disagrees with the fact it cites.
    ///
    /// The gap every other code leaves open. `PROV_SIG` is a bad signature,
    /// `PROV_BYTES` is a token resolving to something other than claimed, and
    /// `CLAIM_UNGROUNDED` is a claim with no citation at all. None of them can
    /// say "the citation is real, resolves, verifies, is about the right cell,
    /// and the number written beside it is not the number inside it". That is
    /// the shape a fabricated figure takes once someone has learned to cite.
    ProvValue,
    /// A loaded policy module denied.
    ///
    /// One code for every module rather than one per module, because the
    /// reason line is a fixed grammar with no field for a module id and
    /// growing it would break every agent already parsing it. Which module
    /// fired reaches the caller through the native route's `module` block and
    /// the log record, where the schema is ours to extend.
    PolicyModule,
}

impl DenyCode {
    /// Every code, so a contract can be generated from the enum rather than
    /// from a list somebody remembers to extend. A `match` below would not
    /// help: the compiler cannot force a `vec!` to be complete, and that is
    /// exactly how PROV_VALUE shipped unpublished for one commit.
    pub const ALL: &'static [DenyCode] = &[
        DenyCode::ProvSig,
        DenyCode::ProvBytes,
        DenyCode::ProvDrift,
        DenyCode::GeoZone,
        DenyCode::ClaimUngrounded,
        DenyCode::ProvValue,
        DenyCode::PolicyModule,
    ];

    /// The wire form. Stable.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProvSig => "PROV_SIG",
            Self::ProvBytes => "PROV_BYTES",
            Self::ProvDrift => "PROV_DRIFT",
            Self::GeoZone => "GEO_ZONE",
            Self::ClaimUngrounded => "CLAIM_UNGROUNDED",
            Self::ProvValue => "PROV_VALUE",
            Self::PolicyModule => "POLICY_MODULE",
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
    /// Remove the offending content and retry.
    ///
    /// The remedy a detection module offers: the request itself is fine, one
    /// span of it is not. Distinct from `remove_reference`, which is about a
    /// citation that cannot be made to verify.
    RedactAndRetry,
    /// Ground the claim: resolve the observation through emem and cite the
    /// token it returns.
    ///
    /// The only fix that asks for something to be ADDED rather than changed or
    /// dropped, and the one the whole gate exists to produce. An agent that
    /// reads this and acts starts carrying citations, which is worth more than
    /// the single request it was denied.
    CiteObservation,
    /// Restate the figure as the cited fact reports it, or cite the fact that
    /// says what you wrote.
    ///
    /// Never "drop the citation": the citation is the sound half. The prose is
    /// what disagrees with it.
    CorrectValue,
}

impl Fix {
    /// Every remedy, for the same reason as [`DenyCode::ALL`].
    pub const ALL: &'static [Fix] = &[
        Fix::RefreshToken,
        Fix::RemoveReference,
        Fix::ContactAdmin,
        Fix::RedactAndRetry,
        Fix::CiteObservation,
        Fix::CorrectValue,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::RefreshToken => "refresh_token",
            Self::RemoveReference => "remove_reference",
            Self::ContactAdmin => "contact_admin",
            Self::RedactAndRetry => "redact_and_retry",
            Self::CiteObservation => "cite_observation",
            Self::CorrectValue => "correct_value",
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
    /// Which module denied, when one did.
    ///
    /// Not in the reason line: that grammar is fixed and has no field for it.
    /// It reaches the caller through the native route and the log record,
    /// where the schema is ours to extend.
    pub module: Option<ModuleAttribution>,
}

/// Which module produced a `POLICY_MODULE` denial, and what it found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleAttribution {
    pub id: String,
    pub version: String,
    /// A digest of what matched. Never the content.
    pub evidence_digest: Option<String>,
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
            module: None,
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
            module: None,
        }
    }

    /// Attach the ungrounded claim behind a `CLAIM_UNGROUNDED` denial.
    pub fn with_claim(mut self, claim: crate::claim::Claim) -> Self {
        self.claim = Some(claim);
        self
    }

    /// Attach the module that produced this denial.
    pub fn with_module(mut self, m: ModuleAttribution) -> Self {
        self.module = Some(m);
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
    /// `tokens`: a claim is ungrounded when nothing the transcript cited
    /// RESOLVED here. Citing something unresolvable is not grounding, however
    /// well-formed the token looks.
    pub claims: Vec<crate::claim::Claim>,
    /// What each loaded policy module concluded, in load order.
    ///
    /// Modules do IO, so they run in the caller alongside the resolver and
    /// arrive here already answered. That is what keeps this module pure and
    /// what stops a third party's plugin from reaching upstream mid-verdict.
    pub modules: Vec<(crate::module::ModuleManifest, crate::module::ModuleVerdict)>,
    /// The numeric reading behind each citation that resolved to one, keyed by
    /// the token as it appeared. Empty on a node with no corpus attached,
    /// which disables the value-agreement rule rather than failing it.
    pub values: Vec<(String, crate::resolve::FactValue)>,
    /// Sentences that state a number and cite exactly one observation.
    pub cited_numbers: Vec<crate::claim::CitedNumber>,
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

        // Then the number itself. A citation can resolve, verify and bind to
        // the right cell while the sentence beside it reports a different
        // figure entirely, and until 2026-08-09 nothing looked: two desks
        // running a lending simulation put "NDVI was 0.62" next to the genuine
        // token for a fact reading 0.138 and got an allow with balanced
        // fact_cids and a valid inclusion proof.
        //
        // This denies only on positively established disagreement, which is
        // the safe direction. It needs a resolved numeric value, so a node
        // with no corpus never reaches it; it needs exactly one citation in
        // the sentence, so attribution is never a guess; and agreement is
        // judged at the precision the writer chose, so reporting
        // 889.6439208984375 as `889.6 m` is correct rather than a lie.
        for cn in &ev.cited_numbers {
            let Some((_, fv)) = ev.values.iter().find(|(t, _)| *t == cn.token) else {
                continue;
            };
            if cn
                .numbers
                .iter()
                .any(|(n, d)| crate::claim::agrees(*n, *d, fv.value))
            {
                continue;
            }
            return Decision::block(
                DenyCode::ProvValue,
                Some(cn.token.clone()),
                Fix::CorrectValue,
            )
            .with_checked(checked);
        }
    }

    // Module verdicts enter the pipeline as rules, positioned by how much
    // they establish rather than by how alarming they sound. A broken
    // signature is a cryptographic fact and outranks everything. A module
    // finding is a positive match, so it outranks drift, which is a threshold
    // judgement about a measurement that did verify.
    //
    // An abstain is never a block: see ModuleOutcome::Abstain. A module whose
    // sidecar is down has not told us to deny.
    for (man, v) in &ev.modules {
        if v.outcome == crate::module::ModuleOutcome::Deny {
            return Decision::block(man.deny_code, None, man.fix)
                .with_module(ModuleAttribution {
                    id: man.id.clone(),
                    version: man.version.clone(),
                    evidence_digest: v.evidence_digest.clone(),
                })
                .with_checked(checked);
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
        // enough to exist. A transcript that GROUNDED something is the work of
        // an agent that knows how to ground itself, and gating its individual
        // sentences would need attribution this engine cannot do: which claim
        // does which citation support. Rather than guess, the rule declines to
        // fire the moment one citation actually resolved.
        //
        // The cost is an agent that cites once and asserts ten times. That is
        // a real gap and it is the conservative side of it.
        //
        // GROUNDED MEANS RESOLVED, NOT MERELY PRESENT. Until 2026-08-08 this
        // tested `ev.tokens.is_empty()`, so pasting any well-formed token
        // suppressed the denial the sentence had already earned: a real cid
        // with four characters changed turned a deny into an allow while
        // `receipt.fact_cids` stayed empty, and no provenance rule fired
        // either, because an unresolvable token is deliberately never a
        // denial. Defeating the gate was less work than satisfying it, which
        // is the worst property a control can have. An `Unresolved` token is
        // still never a DENIAL, for every reason given on that variant, but
        // it cannot count as EVIDENCE: this node has established nothing
        // about it, and silence is not a citation.
        let grounded = ev
            .tokens
            .iter()
            .any(|(_, st)| matches!(st, TokenStatus::Verified | TokenStatus::Drifted { .. }));
        if !grounded {
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
    /// enough to exist. An agent that GROUNDED anything is one that knows how
    /// to ground itself, and attributing individual sentences to individual
    /// citations is a problem this engine declines to guess at.
    ///
    /// Until 2026-08-08 this test was called
    /// `a_single_citation_anywhere_disarms_the_claim_gate` and asserted that
    /// an `Unresolved` token disarmed the rule. That was the defect, written
    /// down as an invariant: any well-formed token, including a real cid with
    /// four characters changed, turned this denial into an allow while
    /// `receipt.fact_cids` stayed empty. Defeating the gate was cheaper than
    /// satisfying it. Resolution is now the test.
    #[test]
    fn only_a_resolved_citation_disarms_the_claim_gate() {
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
        assert_eq!(
            d.claim.as_ref().unwrap().source_band,
            Some("copdem30m.elevation_mean")
        );

        // One citation that RESOLVED, and the rule declines.
        let grounded = Evidence {
            claims: vec![claim.clone()],
            tokens: vec![(tok("emem:fact:a:b", TokenKind::Fact), TokenStatus::Verified)],
            ..Default::default()
        };
        assert_eq!(evaluate(&cfg, &grounded).outcome, Outcome::Proceed);

        // A citation that did NOT resolve is not evidence, so the rule still
        // fires. This node has established nothing about that token, and a
        // string that merely looks like a citation must not buy an allow.
        //
        // Note what this does NOT change: an unresolved token is still never a
        // denial in its own right. No PROV_ code fires on it, because a token
        // minted by another responder is legitimate and indistinguishable from
        // a forgery from in here. The claim gate is a different statement, and
        // a true one: nothing you cited grounds this sentence HERE.
        let unresolved = Evidence {
            claims: vec![claim],
            tokens: vec![(
                tok("emem:entity:abc", TokenKind::Entity),
                TokenStatus::Unresolved,
            )],
            ..Default::default()
        };
        assert_eq!(evaluate(&cfg, &unresolved).outcome, Outcome::Block);
    }

    /// The gap the other four codes leave open.
    ///
    /// Reproduced twice by two independent desks during a lending simulation:
    /// a sentence asserting an NDVI of 0.62 carrying the GENUINE token for the
    /// fact that reads 0.138. Right cell, right band, right observation; it
    /// resolves, verifies and proves inclusion. Only the prose is false, and
    /// no code in the vocabulary could say so.
    #[test]
    fn a_figure_that_disagrees_with_the_fact_it_cites_is_denied() {
        let cfg = Config::default();
        assert!(cfg.provenance, "the rule lives under the provenance switch");
        let token =
            "emem:fact:defi.zb4e4.zcced.fUrI:3u37m4qbj67sc4lhdgh5yklp43kqik2evhvdtbqq2rjv7m7yg6oq";
        let fv = crate::resolve::FactValue {
            value: 0.138_136_153_337_739_65,
            unit: None,
            band: "indices.ndvi".to_string(),
        };
        let ev = |sentence: &str| Evidence {
            tokens: vec![(tok(token, TokenKind::Fact), TokenStatus::Verified)],
            values: vec![(token.to_string(), fv.clone())],
            cited_numbers: crate::claim::scan_cited_numbers([sentence]),
            ..Default::default()
        };

        let lying = format!("Peak-season NDVI at Dindori was 0.62 in 2025, per {token}.");
        let d = evaluate(&cfg, &ev(&lying));
        assert_eq!(d.code, Some(DenyCode::ProvValue), "0.62 is not 0.138");
        // The citation is the sound half. Telling the author to drop it would
        // remove the evidence and keep the false number.
        assert_eq!(d.fix, Some(Fix::CorrectValue));
        assert_eq!(d.token.as_deref(), Some(token));

        // Reporting the same fact at the precision a person would write is not
        // a lie, and calling it one would make the rule unusable.
        let honest = format!("Peak-season NDVI at Dindori was 0.14 in 2025, per {token}.");
        assert_eq!(evaluate(&cfg, &ev(&honest)).outcome, Outcome::Proceed);

        // Full width agrees with itself.
        let exact = format!("NDVI at Dindori was 0.13813615333773965, per {token}.");
        assert_eq!(evaluate(&cfg, &ev(&exact)).outcome, Outcome::Proceed);

        // A node with no corpus resolved no value, so it has established
        // nothing and must not deny.
        let bare = Evidence {
            tokens: vec![(tok(token, TokenKind::Fact), TokenStatus::Unresolved)],
            cited_numbers: crate::claim::scan_cited_numbers([lying.as_str()]),
            ..Default::default()
        };
        assert_eq!(evaluate(&cfg, &bare).outcome, Outcome::Proceed);
    }

    /// The ordinary way to cite is to finish the sentence first.
    ///
    /// "sits at 5000 m elevation. [emem:fact:...]" splits the figure away from
    /// the token, and a rule that only ever looked inside one sentence missed
    /// every memo written that way, including this repository's own example.
    #[test]
    fn a_citation_after_the_full_stop_still_answers_for_the_sentence() {
        let cfg = Config::default();
        let token =
            "emem:fact:defi.zb493.hufo.zccc6:cqyzrjbuej7fdgmbgjjkboqnhjlfz4u6zy3nezps4t2qntab5sqa";
        let fv = crate::resolve::FactValue {
            value: 889.643_920_898_437_5,
            unit: Some("m".to_string()),
            band: "copdem30m.elevation_mean".to_string(),
        };
        let ev = |s: &str| Evidence {
            tokens: vec![(tok(token, TokenKind::Fact), TokenStatus::Verified)],
            values: vec![(token.to_string(), fv.clone())],
            cited_numbers: crate::claim::scan_cited_numbers([s]),
            ..Default::default()
        };

        let lying = format!("The parcel at Jagraon sits at 5000 m elevation. [{token}]");
        assert_eq!(
            evaluate(&cfg, &ev(&lying)).code,
            Some(DenyCode::ProvValue),
            "the citation trails the sentence it supports"
        );

        let honest = format!("The parcel at Jagraon sits at 889.6 m elevation. [{token}]");
        assert_eq!(evaluate(&cfg, &ev(&honest)).outcome, Outcome::Proceed);

        // A citation adopts figures only from a sentence that cited nothing
        // itself. Two citations in a row leave the pairing ambiguous, so the
        // rule declines rather than guessing.
        let ambiguous = format!("Elevation is 5000 m [{token}]. And again [{token}]");
        let _ = evaluate(&cfg, &ev(&ambiguous));
    }

    /// Where you wrap a line must not change the verdict.
    ///
    /// A desk measured the same paragraph both ways and got two answers: with
    /// the figure and the token on one line it allowed, and with the wrap
    /// falling so that a DISTANCE was the last number before the citation it
    /// denied. Line breaks are typography. Treating them as meaning produces a
    /// deny nobody can act on, which costs more than a miss.
    #[test]
    fn wrapping_a_line_does_not_change_the_verdict() {
        let cfg = Config::default();
        let token =
            "emem:fact:defi.zb493.hufo.zccc6:2hsx3izrfsgpafhqb6neqnavximzpywr5uo7innarfftvj2yeeaa";
        let fv = crate::resolve::FactValue {
            value: -0.036_615_874_459_859,
            unit: None,
            band: "indices.ndbi".to_string(),
        };
        let ev = |s: &str| Evidence {
            tokens: vec![(tok(token, TokenKind::Fact), TokenStatus::Verified)],
            values: vec![(token.to_string(), fv.clone())],
            cited_numbers: crate::claim::scan_cited_numbers([s]),
            ..Default::default()
        };

        let flat = format!("Built-up index is -0.037 at 500 m south [{token}]");
        let wrapped = format!("Built-up index is -0.037\nat 500 m south [{token}]");
        assert_eq!(
            evaluate(&cfg, &ev(&flat)).outcome,
            Outcome::Proceed,
            "the figure agrees, so this must allow"
        );
        assert_eq!(
            evaluate(&cfg, &ev(&wrapped)).outcome,
            Outcome::Proceed,
            "and the same prose must not deny merely because it wrapped"
        );

        // Widening the candidates must not blunt the rule: a paragraph where
        // NO figure matches still denies, wrapped or not.
        let wrong = format!("Built-up index is 0.42 at 500 m south [{token}]");
        let wrong_wrapped = format!("Built-up index is 0.42\nat 500 m south [{token}]");
        assert_eq!(evaluate(&cfg, &ev(&wrong)).code, Some(DenyCode::ProvValue));
        assert_eq!(
            evaluate(&cfg, &ev(&wrong_wrapped)).code,
            Some(DenyCode::ProvValue)
        );
    }

    /// Two citations in one sentence and the rule declines, because deciding
    /// which number answers which citation is the same guess the claim gate
    /// refuses to make.
    #[test]
    fn two_citations_in_a_sentence_are_not_attributed() {
        let a =
            "emem:fact:defi.zb4e4.zcced.fUrI:3u37m4qbj67sc4lhdgh5yklp43kqik2evhvdtbqq2rjv7m7yg6oq";
        let b =
            "emem:fact:defi.zb4e4.zcced.fUrI:cj4ccplttk6oyp5sr7rn76igvqq7mno2b6z4kqxbztcjemsisdya";
        let sentence = format!("NDVI at Dindori was 0.62 and moisture was 0.31, per {a} and {b}.");
        assert!(
            crate::claim::scan_cited_numbers([sentence.as_str()]).is_empty(),
            "a two-citation sentence yields nothing to compare"
        );
    }

    /// The digits inside a cell64 are an address, not an assertion.
    #[test]
    fn an_address_is_not_a_number_the_prose_claimed() {
        let token =
            "emem:fact:defi.zb4e4.zcced.fUrI:3u37m4qbj67sc4lhdgh5yklp43kqik2evhvdtbqq2rjv7m7yg6oq";
        let found =
            crate::claim::scan_cited_numbers([
                format!("Elevation there is 640.89 m per {token}.").as_str()
            ]);
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].numbers,
            vec![(640.89, 2)],
            "only the figure in the prose, never the digits in the address"
        );
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
