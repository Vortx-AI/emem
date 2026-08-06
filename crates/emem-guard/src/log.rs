//! Sign the verdict, log it, and only then answer.
//!
//! This is the product, and it is the half that is missing everywhere. Every
//! checkpoint contract we have read signs requests on the way IN and takes
//! verdicts back as unsigned JSON into a mutable audit database. Nobody can
//! later prove what was allowed, denied, or why. A compliance API that reports
//! its own history is testimony; an append-only log anyone can check is
//! evidence, and the difference only shows up on the day it matters.
//!
//! # Why the order is not negotiable
//!
//! Assemble, sign, append, THEN return. The alternative, returning first and
//! logging asynchronously, is faster and produces a verdict that was acted on
//! but never recorded whenever the process dies in between. A gap in an
//! append-only log is indistinguishable from a deletion, so a single crash
//! would permanently weaken every claim the log makes. Paying the append
//! latency inline is the cost of the log meaning anything.
//!
//! The one exception is the archival pattern the platform documents: a server
//! that always allows may answer before persisting, because it is not making
//! a decision anyone needs to prove. That is not this.
//!
//! # What is signed
//!
//! The preimage binds every input that could change the verdict: which
//! checkpoint asked, the request id, the decision, the code, the token, the
//! count of citations examined, whether the node was enforcing, and what the
//! rules concluded regardless. Anything omitted is something an operator could
//! later alter without breaking the signature, so the rule is that a field
//! either enters the preimage or does not exist.
//!
//! `mode` and `evaluated` are why the domain moved to v2: a shadow report is a
//! claim about how often a rule WOULD have fired, and a claim like that is
//! only worth anything if the operator making it could not have edited the
//! answer afterwards.
//!
//! `config_digest`, `module` and `evidence_digest` are why it is now v3. A
//! verdict has to be reproducible, and it is not reproducible unless it names
//! the exact detector set that produced it. The version lives IN the domain
//! string, so an older verifier handed a newer record does not silently accept
//! it: the signature simply does not verify.
//!
//! The preimage is pre-1.0 and will move again if something load-bearing is
//! found to be unbound. It freezes at the first conformance-green release, and
//! after that a change is a new version served alongside the old one.

use serde::{Deserialize, Serialize};

use crate::checkpoint::Outcome;
use crate::policy::{Decision, DenyCode, Fix, Mode};

/// A verdict as it is written to the log.
///
/// Serialized field order is the preimage order, and both are fixed. A
/// verifier reconstructs this from the log alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerdictRecord {
    /// Which checkpoint class asked. Part of the preimage because the same
    /// transcript through different checkpoints is a different event, and an
    /// auditor needs to know which surface was governed.
    pub checkpoint: String,
    /// The checkpoint's own request id, which is also the idempotency key.
    pub request_id: String,
    /// allow / deny.
    ///
    /// Owned rather than `&'static str`: a record read back from the log is
    /// data from disk, and a borrowed lifetime cannot survive that. The audit
    /// path reconstructs records from a file, which is the whole point.
    pub outcome: String,
    /// The deny code, absent on allow.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<DenyCode>,
    /// The offending token, absent on allow.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// The remedy offered, absent on allow.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<Fix>,
    /// How many citations were examined. In the preimage because it is the
    /// number the shadow report aggregates, and a number worth reporting is a
    /// number worth being unable to change afterwards.
    pub checked: usize,
    /// When we decided, unix seconds.
    pub decided_at_unix_s: i64,
    /// Whether this verdict was acted on or only observed.
    ///
    /// In the preimage because the difference between "we blocked this" and
    /// "we would have blocked this" is the entire content of a shadow report.
    /// An operator who could relabel one as the other after the fact could
    /// claim enforcement they never ran, or disown a block they did.
    #[serde(default)]
    pub mode: Mode,
    /// What the rules concluded, whatever `outcome` says.
    ///
    /// Equal to `outcome` under [`Mode::Enforce`]. Under [`Mode::Shadow`] this
    /// is `deny` on an entry whose `outcome` is `allow`, and that pair is the
    /// measurement: how often the rule would have fired, on this org's own
    /// traffic, signed at the moment it was observed rather than counted later
    /// from a mutable table.
    #[serde(default)]
    pub evaluated: String,
    /// Digest over the loaded module set, in order.
    ///
    /// What makes a verdict REPRODUCIBLE rather than merely genuine. Without
    /// it, an operator could change which detectors are loaded and the same
    /// signed record would describe two different evaluations, which is the
    /// exact substitution an audit trail exists to prevent.
    #[serde(default)]
    pub config_digest: String,
    /// Which module denied, when one did. `id@version`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
    /// A digest of what that module matched. **Never the content.** A log that
    /// recorded what a secret scanner found would be a catalogue of secrets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_digest: Option<String>,
}

impl VerdictRecord {
    /// Build the record for a decision.
    ///
    /// `decision` is the decision as it will be RETURNED. The rule outcome is
    /// recovered from the code, which survives a shadow downgrade, so a
    /// shadowed deny records `outcome: allow` and `evaluated: deny` and both
    /// are signed.
    pub fn new(
        checkpoint: &str,
        request_id: &str,
        decision: &Decision,
        decided_at_unix_s: i64,
        mode: Mode,
        config_digest: &str,
    ) -> Self {
        let word = |o: Outcome| match o {
            Outcome::Proceed => "allow".to_string(),
            Outcome::Block => "deny".to_string(),
        };
        Self {
            checkpoint: checkpoint.to_string(),
            request_id: request_id.to_string(),
            outcome: word(decision.outcome),
            code: decision.code,
            token: decision.token.clone(),
            fix: decision.fix,
            checked: decision.checked,
            decided_at_unix_s,
            mode,
            evaluated: if decision.would_block() {
                "deny".to_string()
            } else {
                "allow".to_string()
            },
            config_digest: config_digest.to_string(),
            module: decision
                .module
                .as_ref()
                .map(|m| format!("{}@{}", m.id, m.version)),
            evidence_digest: decision
                .module
                .as_ref()
                .and_then(|m| m.evidence_digest.clone()),
        }
    }

    /// The bytes a signature covers.
    ///
    /// Domain-separated and length-prefixed, the same construction the
    /// receipt preimage uses, so a verifier that already implements one can
    /// implement the other without new primitives. Two records with different
    /// field boundaries cannot collide: `("ab","c")` and `("a","bc")` hash
    /// differently because each segment carries its own length.
    ///
    /// Optional fields are written as a tagged empty segment rather than
    /// skipped, so "no token" is a signed statement and not an absence an
    /// intermediary could introduce. That is the same lesson the receipt
    /// preimage learned when a stripped Merkle proof verified: absence has to
    /// be said, not implied.
    pub fn preimage(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(256);
        let mut seg = |tag: u8, bytes: &[u8]| {
            out.push(tag);
            out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            out.extend_from_slice(bytes);
        };
        // Domain, so a verdict preimage can never be confused with a receipt.
        //
        // v2 adds the mode and the evaluated outcome. The version is IN the
        // domain rather than beside it, so a v1 verifier cannot be handed a v2
        // record and told the extra segments were always there: the domain
        // string itself differs, so the signature simply does not verify.
        seg(0x00, b"emem.guard.verdict.v3");
        seg(0x01, self.checkpoint.as_bytes());
        seg(0x02, self.request_id.as_bytes());
        seg(0x03, self.outcome.as_bytes());
        seg(
            0x04,
            self.code.map(DenyCode::as_str).unwrap_or("").as_bytes(),
        );
        seg(0x05, self.token.as_deref().unwrap_or("").as_bytes());
        seg(0x06, self.fix.map(Fix::as_str).unwrap_or("").as_bytes());
        seg(0x07, &(self.checked as u64).to_le_bytes());
        seg(0x08, &self.decided_at_unix_s.to_le_bytes());
        seg(0x09, self.mode.as_str().as_bytes());
        seg(0x0a, self.evaluated.as_bytes());
        seg(0x0b, self.config_digest.as_bytes());
        seg(0x0c, self.module.as_deref().unwrap_or("").as_bytes());
        seg(
            0x0d,
            self.evidence_digest.as_deref().unwrap_or("").as_bytes(),
        );
        out
    }
}

/// Where a verdict is written before it is returned.
///
/// A trait so the core stays free of IO and the server can supply the real
/// log, while conformance tests supply one that counts appends and can be
/// made to fail on demand. There is no default implementation on purpose:
/// a guard with no log is a guard with no product, and silently accepting
/// one would let that ship.
pub trait VerdictLog {
    /// Append a signed record and return its leaf id.
    ///
    /// The leaf id travels back to the caller in `deny_reason` and as the
    /// `reference_id`, which is what lets an org join a denial in their
    /// Activity Feed to a record they can verify without asking us.
    fn append(&self, record: &VerdictRecord, signature: &[u8]) -> Result<String, LogError>;
}

/// Why a verdict could not be logged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogError {
    /// The log is unreachable or refused the write.
    Unavailable(String),
}

impl core::fmt::Display for LogError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unavailable(w) => write!(f, "verdict log unavailable: {w}"),
        }
    }
}

/// What to do when the log cannot accept a verdict.
///
/// Not a detail. If the log is down, a deny that cannot be logged is a
/// decision nobody can later audit, which is the one thing this product
/// exists to prevent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogFailurePolicy {
    /// Return the decision anyway, with no leaf id.
    ///
    /// The default, and the honest one. We are one control among several, and
    /// a guard that starts blocking work because its own storage is unhealthy
    /// converts our outage into the customer's outage. The verdict is still
    /// correct; only its provability is missing, and the response says so by
    /// carrying `leaf=-`.
    #[default]
    ProceedUnlogged,
    /// Downgrade any deny to an allow when it cannot be logged.
    ///
    /// For orgs whose rule is that an unprovable block is worse than a missed
    /// one. Never the default: it silently weakens enforcement exactly when
    /// the system is already degraded.
    DowngradeDeny,
}

/// Sign, log, and stamp the decision with its leaf.
///
/// `sign` is injected rather than imported so this module holds no key
/// material and the server can hand it whatever the node's identity is.
///
/// Returns the decision to send. On a log failure the behaviour follows
/// `policy`, and in both cases the caller still answers with a well-formed
/// verdict: refusing to answer would be a webhook failure, which blocks
/// nothing and counts toward the circuit breaker.
#[allow(clippy::too_many_arguments)]
pub fn seal(
    checkpoint: &str,
    request_id: &str,
    decision: Decision,
    decided_at_unix_s: i64,
    mode: Mode,
    config_digest: &str,
    sign: impl Fn(&[u8]) -> Vec<u8>,
    log: &dyn VerdictLog,
    policy: LogFailurePolicy,
) -> (Decision, Option<LogError>) {
    let record = VerdictRecord::new(
        checkpoint,
        request_id,
        &decision,
        decided_at_unix_s,
        mode,
        config_digest,
    );
    let signature = sign(&record.preimage());
    match log.append(&record, &signature) {
        Ok(leaf) => (decision.with_leaf(leaf), None),
        Err(e) => {
            let d = match policy {
                LogFailurePolicy::ProceedUnlogged => decision,
                LogFailurePolicy::DowngradeDeny if decision.outcome == Outcome::Block => {
                    Decision::proceed().with_checked(decision.checked)
                }
                LogFailurePolicy::DowngradeDeny => decision,
            };
            (d, Some(e))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct Recording {
        appended: RefCell<Vec<(VerdictRecord, Vec<u8>)>>,
        fail: bool,
    }
    impl Recording {
        fn ok() -> Self {
            Self {
                appended: RefCell::new(Vec::new()),
                fail: false,
            }
        }
        fn broken() -> Self {
            Self {
                appended: RefCell::new(Vec::new()),
                fail: true,
            }
        }
    }
    impl VerdictLog for Recording {
        fn append(&self, r: &VerdictRecord, sig: &[u8]) -> Result<String, LogError> {
            if self.fail {
                return Err(LogError::Unavailable("disk".into()));
            }
            self.appended.borrow_mut().push((r.clone(), sig.to_vec()));
            Ok(format!("leaf_{}", self.appended.borrow().len()))
        }
    }

    fn deny() -> Decision {
        Decision::block(
            DenyCode::ProvSig,
            Some("emem:fact:a:b".into()),
            Fix::RefreshToken,
        )
        .with_checked(2)
    }

    /// The record must be written BEFORE the leaf can appear on the decision,
    /// which is what makes "signed and logged" true of every verdict that was
    /// acted on.
    #[test]
    fn a_verdict_is_logged_before_it_carries_a_leaf() {
        let log = Recording::ok();
        let (d, err) = seal(
            "anthropic.inference_hook.v1",
            "req_1",
            deny(),
            1_700_000_000,
            Mode::Enforce,
            "cfg-test",
            |p| p.to_vec(),
            &log,
            LogFailurePolicy::default(),
        );
        assert!(err.is_none());
        assert_eq!(log.appended.borrow().len(), 1, "exactly one append");
        assert_eq!(d.leaf.as_deref(), Some("leaf_1"));
        // And the leaf reaches both channels an auditor can join on.
        assert!(d.reason_line().contains("leaf=leaf_1"));
        assert_eq!(d.reference_id(), Some("leaf_1"));
    }

    /// Every field that could change the verdict is bound. A preimage that
    /// ignored one would let an operator alter it afterwards without
    /// breaking the signature, which is the whole failure this design exists
    /// to prevent.
    #[test]
    fn every_field_that_could_change_the_verdict_is_signed() {
        let base = VerdictRecord::new(
            "cp",
            "req_1",
            &deny(),
            1_700_000_000,
            Mode::Enforce,
            "cfg-test",
        );
        let p = base.preimage();

        let mut m = base.clone();
        m.checkpoint = "other".into();
        assert_ne!(m.preimage(), p, "checkpoint");

        let mut m = base.clone();
        m.request_id = "req_2".into();
        assert_ne!(m.preimage(), p, "request_id");

        let mut m = base.clone();
        m.outcome = "allow".to_string();
        assert_ne!(m.preimage(), p, "outcome");

        let mut m = base.clone();
        m.code = Some(DenyCode::ProvBytes);
        assert_ne!(m.preimage(), p, "code");

        let mut m = base.clone();
        m.token = Some("emem:fact:c:d".into());
        assert_ne!(m.preimage(), p, "token");

        let mut m = base.clone();
        m.fix = Some(Fix::RemoveReference);
        assert_ne!(m.preimage(), p, "fix");

        let mut m = base.clone();
        m.checked = 3;
        assert_ne!(m.preimage(), p, "checked");

        let mut m = base.clone();
        m.decided_at_unix_s += 1;
        assert_ne!(m.preimage(), p, "decided_at");

        let mut m = base.clone();
        m.mode = Mode::Shadow;
        assert_ne!(m.preimage(), p, "mode");

        let mut m = base.clone();
        m.evaluated = "allow".to_string();
        assert_ne!(m.preimage(), p, "evaluated");
    }

    /// Shadow logs the decision it reached and returns the one it was allowed
    /// to act on. Both are signed, and the pair is the measurement an org uses
    /// to decide whether to enforce.
    #[test]
    fn a_shadowed_deny_is_recorded_as_a_deny_that_was_not_acted_on() {
        let log = Recording::ok();
        let returned = deny().under(Mode::Shadow);
        assert_eq!(
            returned.outcome,
            Outcome::Proceed,
            "the caller is not blocked"
        );
        let (_d, err) = seal(
            "cp",
            "req_shadow",
            returned,
            0,
            Mode::Shadow,
            "cfg-test",
            |p| p.to_vec(),
            &log,
            LogFailurePolicy::default(),
        );
        assert!(err.is_none());
        let (rec, _) = log.appended.borrow()[0].clone();
        assert_eq!(rec.outcome, "allow", "what the checkpoint was told");
        assert_eq!(rec.evaluated, "deny", "what the rules concluded");
        assert_eq!(rec.mode, Mode::Shadow);
        assert_eq!(
            rec.code,
            Some(DenyCode::ProvSig),
            "the reason survives, or the report cannot say which rule fired"
        );
    }

    /// Enforcing is the default, and there the two agree. A report that could
    /// not tell an enforced deny from an observed one would be worthless.
    #[test]
    fn under_enforcement_the_two_outcomes_agree() {
        assert_eq!(Mode::default(), Mode::Enforce);
        let r = VerdictRecord::new("cp", "r", &deny(), 0, Mode::Enforce, "cfg-test");
        assert_eq!((r.outcome.as_str(), r.evaluated.as_str()), ("deny", "deny"));
        let a = VerdictRecord::new(
            "cp",
            "r",
            &Decision::proceed(),
            0,
            Mode::Enforce,
            "cfg-test",
        );
        assert_eq!(
            (a.outcome.as_str(), a.evaluated.as_str()),
            ("allow", "allow")
        );
    }

    /// Length prefixes, so adjacent fields cannot be re-partitioned into a
    /// different record with the same bytes.
    #[test]
    fn field_boundaries_cannot_be_shifted() {
        let a = VerdictRecord::new(
            "ab",
            "c",
            &Decision::proceed(),
            0,
            Mode::Enforce,
            "cfg-test",
        );
        let b = VerdictRecord::new(
            "a",
            "bc",
            &Decision::proceed(),
            0,
            Mode::Enforce,
            "cfg-test",
        );
        assert_ne!(a.preimage(), b.preimage());
    }

    /// An absent token is a signed statement of absence, not a gap. The
    /// receipt preimage learned this when a stripped Merkle proof verified.
    #[test]
    fn absence_is_signed_rather_than_implied() {
        let allow = VerdictRecord::new(
            "cp",
            "req_1",
            &Decision::proceed(),
            0,
            Mode::Enforce,
            "cfg-test",
        );
        let p = allow.preimage();
        // The tag for `token` is present even though the value is empty.
        assert!(p.contains(&0x05u8));
        let mut with = allow.clone();
        with.token = Some("x".into());
        assert_ne!(
            with.preimage(),
            p,
            "adding a token must change the preimage"
        );
    }

    /// Our storage being unhealthy must not become the customer's outage.
    #[test]
    fn a_log_failure_does_not_by_default_change_the_answer() {
        let log = Recording::broken();
        let (d, err) = seal(
            "cp",
            "req_1",
            deny(),
            0,
            Mode::Enforce,
            "cfg-test",
            |p| p.to_vec(),
            &log,
            LogFailurePolicy::default(),
        );
        assert_eq!(err, Some(LogError::Unavailable("disk".into())));
        assert_eq!(d.outcome, Outcome::Block, "the verdict was still correct");
        assert!(d.leaf.is_none());
        // And the response says so rather than implying a leaf exists.
        assert!(d.reason_line().contains("leaf=-"), "{}", d.reason_line());
    }

    /// The opt-in posture for orgs that would rather miss a block than issue
    /// an unprovable one.
    #[test]
    fn downgrade_is_available_but_never_the_default() {
        assert_eq!(
            LogFailurePolicy::default(),
            LogFailurePolicy::ProceedUnlogged
        );
        let log = Recording::broken();
        let (d, err) = seal(
            "cp",
            "req_1",
            deny(),
            0,
            Mode::Enforce,
            "cfg-test",
            |p| p.to_vec(),
            &log,
            LogFailurePolicy::DowngradeDeny,
        );
        assert!(err.is_some());
        assert_eq!(d.outcome, Outcome::Proceed);
        assert_eq!(
            d.checked, 2,
            "the count survives so the shadow report is still honest"
        );
    }
}
