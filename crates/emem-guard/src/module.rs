//! The plugin interface: everyone else's detection, on our chassis.
//!
//! emem-guard is deliberately bad at content detection and will stay that way.
//! That race is thousands of patterns maintained by hundreds of engineers, and
//! winning it would grow nothing that matters here. What this crate has that
//! none of those engines do is the half after the verdict: a signed,
//! hash-chained, offline-checkable record of every decision.
//!
//! So the trade is explicit. Bring your own detection; we make its verdicts
//! provable. A DLP denial as evidence rather than a database row is a thing no
//! incumbent ships, and it costs them nothing to get: a module is a trait
//! implementation or an HTTP endpoint.
//!
//! # The two declarations that are load-bearing
//!
//! A module declares a [`LatencyClass`] and a [`DataClass`], and both change
//! where it is allowed to run.
//!
//! **Latency.** A verdict has a hard deadline measured in hundreds of
//! milliseconds. A module that takes a second is not slow, it is a transport
//! failure, and a transport failure is not a deny: it hands the decision to the
//! caller's failure policy and, sustained, trips the breaker that stops
//! enforcement for everything. So [`LatencyClass::Slow`] modules never run on
//! the enforcing path at all.
//!
//! And the declaration is **measured, not trusted**. A module that claims
//! [`LatencyClass::Fast`] and repeatedly exceeds [`FAST_BUDGET`] is demoted to
//! shadow by the registry and stays there, which is visible at `/modules`. A
//! promise about latency that nothing checks is how one slow regex takes an
//! organisation's enforcement offline.
//!
//! **Data.** Some deployments will not let transcript text leave a boundary. A
//! module that declares [`DataClass::DigestsOnly`] gets no text even when text
//! is available, so a relay can run it where a text-hungry module could not go.
//! The context carries a digest of the transcript either way, so a module can
//! still correlate without reading.
//!
//! # What reaches the log
//!
//! The module id, its version, and an **evidence digest**. Never the matched
//! content. A log that recorded what a secret scanner found would be a
//! catalogue of secrets, readable by everyone the log is readable by, which is
//! the entire point of the log.
//!
//! The registry's [`ModuleRegistry::config_digest`] enters the verdict
//! preimage, so a verdict names the exact module set that produced it. Without
//! that, an operator could change which modules are loaded and the same signed
//! record would describe two different evaluations.

use std::time::{Duration, Instant};

use crate::claim::Claim;
use crate::policy::{DenyCode, Fix, TokenStatus};
use crate::tokens::FoundToken;

/// The ceiling for a module that wants to run on the enforcing path.
///
/// The verdict self-budget is 800 ms and citation resolution may take half of
/// it, so the whole module chain has to fit in what is left beside signing and
/// the log append. 50 ms per module is enough for a regex sweep over a
/// megabyte and nowhere near enough for a model call, which is the distinction
/// the class is drawing.
pub const FAST_BUDGET: Duration = Duration::from_millis(50);

/// How many budget overruns before a fast module is demoted to shadow.
///
/// Not one: a single overrun is as likely to be a cold page or a noisy
/// neighbour as a mis-declared module, and demoting on it would make the
/// registry flap. Three consecutive-in-effect overruns is a pattern.
pub const FAST_STRIKES: u32 = 3;

/// Where a module is allowed to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LatencyClass {
    /// Bounded by [`FAST_BUDGET`]. Runs inline, and its verdict can block.
    Fast,
    /// Anything else. Runs in shadow only: evaluated, signed and logged, never
    /// acted on. A sidecar that calls a model belongs here.
    Slow,
}

/// What a module is allowed to see.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataClass {
    /// Needs the transcript text itself.
    NeedsText,
    /// Works from digests, tokens and claims. Can run where text may not go.
    DigestsOnly,
}

/// A module's self-description.
///
/// Also the thing a third party publishes as a signed fact to ship a module
/// nobody here compiled. The signature binds the publisher; the `build_hash`
/// binds the declaration, so a manifest cannot be edited after signing to
/// claim a different latency class than the one it was reviewed under.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModuleManifest {
    /// Stable, lowercase, no spaces. Appears in the log and at `/modules`.
    pub id: String,
    pub version: String,
    /// `blake3` over the declaration below, hex.
    ///
    /// For an in-process module this proves the manifest is self-consistent
    /// and nothing more, which is worth saying plainly rather than implying a
    /// supply-chain guarantee it does not give. It becomes load-bearing when
    /// the manifest is a signed fact: then the hash is what the signature
    /// covers, and a manifest edited after review no longer verifies.
    pub build_hash: String,
    pub latency_class: LatencyClass,
    pub data_class: DataClass,
    /// The deny code this module's denials carry.
    ///
    /// Declared here rather than returned per verdict, so a module cannot
    /// answer with a code it never advertised. `/modules` publishes it, which
    /// means an agent can learn what a node might say before it says it.
    pub deny_code: DenyCode,
    /// The remedy offered with a denial. Same reasoning.
    pub fix: Fix,
    /// One line, for `/modules`. What it detects, in the author's words.
    pub description: String,
}

impl ModuleManifest {
    /// The hash of everything declared. `build_hash` must equal this.
    pub fn declared_hash(&self) -> String {
        let mut h = blake3::Hasher::new();
        let mut seg = |b: &[u8]| {
            h.update(&(b.len() as u32).to_le_bytes());
            h.update(b);
        };
        seg(b"emem.guard.module.v1");
        seg(self.id.as_bytes());
        seg(self.version.as_bytes());
        seg(format!("{:?}", self.latency_class).as_bytes());
        seg(format!("{:?}", self.data_class).as_bytes());
        seg(self.deny_code.as_str().as_bytes());
        seg(self.fix.as_str().as_bytes());
        h.finalize().to_hex().to_string()
    }

    /// Fill in `build_hash` from the rest. For module authors.
    pub fn sealed(mut self) -> Self {
        self.build_hash = self.declared_hash();
        self
    }
}

/// What a module is given.
///
/// Borrowed throughout: the caller already holds all of it, and a verdict path
/// with a sub-second budget should not be copying a transcript per module.
pub struct ModuleContext<'a> {
    /// The transcript. **Empty** for a [`DataClass::DigestsOnly`] module, and
    /// empty for every module when the deployment forbids text, so a module
    /// that reads this must handle emptiness rather than assume it.
    pub texts: &'a [&'a str],
    /// Citations found, with what resolving them produced.
    pub tokens: &'a [(FoundToken, TokenStatus)],
    /// Measurable physical-world claims found, when claim scanning ran.
    pub claims: &'a [Claim],
    /// `blake3` of the joined transcript, hex. Present even when `texts` is
    /// not, so a digests-only module can still correlate across calls.
    pub text_digest: &'a str,
}

/// What a module concluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModuleOutcome {
    /// Looked, found nothing.
    Allow,
    /// Found something it is willing to block on.
    Deny,
    /// Could not decide: timed out, lacked the data it needed, or is not
    /// applicable. **Never a block.** An abstain is the honest answer for a
    /// module whose sidecar is down, and treating it as a deny would convert
    /// somebody else's outage into a blocked request.
    Abstain,
}

/// A module's answer.
#[derive(Debug, Clone, PartialEq)]
pub struct ModuleVerdict {
    pub outcome: ModuleOutcome,
    /// Free text for the operator log. Never reaches the caller's reason line,
    /// which is a fixed grammar, and never contains matched content.
    pub reason: Option<String>,
    /// 0.0 to 1.0, the module's own confidence. Advisory: nothing in the
    /// pipeline branches on it, because a self-declared number from a
    /// third-party module is not a thing to gate enforcement on. It is
    /// recorded so an operator can compare modules against each other.
    pub confidence: f32,
    /// A digest of WHAT matched. Never the content.
    pub evidence_digest: Option<String>,
}

impl ModuleVerdict {
    /// Looked and found nothing.
    pub fn allow() -> Self {
        Self {
            outcome: ModuleOutcome::Allow,
            reason: None,
            confidence: 1.0,
            evidence_digest: None,
        }
    }

    /// Could not decide. Never blocks.
    pub fn abstain(why: impl Into<String>) -> Self {
        Self {
            outcome: ModuleOutcome::Abstain,
            reason: Some(why.into()),
            confidence: 0.0,
            evidence_digest: None,
        }
    }

    /// Found something. `evidence` is hashed here rather than stored, so a
    /// module author cannot leak matched content into the log by accident.
    pub fn deny(why: impl Into<String>, evidence: &[u8], confidence: f32) -> Self {
        Self {
            outcome: ModuleOutcome::Deny,
            reason: Some(why.into()),
            confidence: confidence.clamp(0.0, 1.0),
            evidence_digest: Some(blake3::hash(evidence).to_hex().to_string()),
        }
    }
}

/// One pluggable detector.
pub trait PolicyModule: Send + Sync {
    fn manifest(&self) -> &ModuleManifest;

    /// Look at the context and answer. Must return within [`FAST_BUDGET`] if
    /// the manifest declares [`LatencyClass::Fast`]; the registry measures
    /// this and demotes a module that does not.
    fn evaluate(&self, ctx: &ModuleContext<'_>) -> ModuleVerdict;
}

/// A manifest with a publisher's signature over it.
///
/// This is how a module ships that nobody here compiled. The `build_hash`
/// alone proves a manifest is self-consistent; a signature over it proves WHO
/// declared it, which is the part that matters when the module is a binary
/// somebody else built.
///
/// The operator still decides. A signature from a publisher they have not
/// listed is refused, so this is not a web of trust and does not become one:
/// it is a key an operator typed, and nothing loads without one.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SignedManifest {
    pub manifest: ModuleManifest,
    /// The publisher's ed25519 key, base32-nopad-lowercase.
    pub publisher_b32: String,
    /// ed25519 over [`Self::preimage`], base32-nopad-lowercase.
    pub signature_b32: String,
}

impl SignedManifest {
    /// The bytes a publisher signs.
    ///
    /// Domain-separated and over the DECLARED hash rather than over a
    /// re-serialisation of the struct, so a publisher and a verifier cannot
    /// disagree about JSON field order. The same construction every other
    /// signature on this surface uses.
    pub fn preimage(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(96);
        let d = b"emem.guard.module_manifest.v1";
        out.push(0x00);
        out.extend_from_slice(&(d.len() as u32).to_le_bytes());
        out.extend_from_slice(d);
        let h = self.manifest.declared_hash();
        out.push(0x01);
        out.extend_from_slice(&(h.len() as u32).to_le_bytes());
        out.extend_from_slice(h.as_bytes());
        out
    }

    /// Whether this manifest was signed by the key it names.
    ///
    /// Does NOT decide whether that key should be trusted. Separate on
    /// purpose: "genuinely signed by X" and "we accept modules from X" are
    /// different questions, and collapsing them is how a valid signature from
    /// a stranger becomes an installed module.
    pub fn signature_verifies(&self) -> bool {
        (|| {
            let pk: [u8; 32] = data_encoding::BASE32_NOPAD
                .decode(self.publisher_b32.to_uppercase().as_bytes())
                .ok()?
                .try_into()
                .ok()?;
            let sig: [u8; 64] = data_encoding::BASE32_NOPAD
                .decode(self.signature_b32.to_uppercase().as_bytes())
                .ok()?
                .try_into()
                .ok()?;
            use ed25519_dalek::Verifier as _;
            ed25519_dalek::VerifyingKey::from_bytes(&pk)
                .ok()?
                .verify(
                    &self.preimage(),
                    &ed25519_dalek::Signature::from_bytes(&sig),
                )
                .ok()
        })()
        .is_some()
    }
}

/// Why a module could not be loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleError {
    /// `build_hash` does not match the declaration it covers.
    TamperedManifest { id: String, expected: String },
    /// Two modules with the same id. Ambiguous in the log, so refused.
    DuplicateId(String),
    /// An id that cannot appear in a log line unambiguously.
    MalformedId(String),
    /// The publisher's signature does not verify over the manifest.
    BadSignature { id: String },
    /// Genuinely signed, by somebody this operator has not listed.
    UntrustedPublisher { id: String, publisher_b32: String },
    /// The signed manifest describes a different module than the one loaded.
    ManifestMismatch { declared: String, loaded: String },
}

impl core::fmt::Display for ModuleError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TamperedManifest { id, expected } => write!(
                f,
                "module {id}: build_hash does not cover its own declaration (expected {expected})"
            ),
            Self::DuplicateId(id) => write!(f, "module {id} is already loaded"),
            Self::MalformedId(id) => write!(
                f,
                "module id {id:?} must be lowercase a-z, 0-9, - or _, and non-empty"
            ),
            Self::BadSignature { id } => {
                write!(f, "module {id}: the publisher signature does not verify")
            }
            Self::UntrustedPublisher { id, publisher_b32 } => write!(
                f,
                "module {id} is signed by {publisher_b32}, which is not in this node's \
                 trusted-publisher list. Add it with --trust-publisher if you mean to."
            ),
            Self::ManifestMismatch { declared, loaded } => write!(
                f,
                "the signed manifest declares {declared} but the loaded module is {loaded}"
            ),
        }
    }
}

/// Health of one loaded module, measured rather than declared.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ModuleHealth {
    pub calls: u64,
    pub overruns: u32,
    /// True once [`FAST_STRIKES`] overruns have happened. A demoted module is
    /// still evaluated in shadow, so it keeps producing signal; it simply
    /// stops being able to block.
    pub demoted: bool,
    pub last_ms: u32,
    pub max_ms: u32,
}

/// The loaded module set.
///
/// Ordered: modules run in load order, and the order is part of
/// [`Self::config_digest`], because "the same modules in a different order"
/// is a different pipeline under first-match-wins.
#[derive(Default)]
pub struct ModuleRegistry {
    modules: Vec<Box<dyn PolicyModule>>,
    health: std::sync::Mutex<std::collections::HashMap<String, ModuleHealth>>,
}

impl ModuleRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Load one module, refusing a manifest that does not cover itself.
    pub fn load(&mut self, m: Box<dyn PolicyModule>) -> Result<(), ModuleError> {
        let man = m.manifest().clone();
        if man.id.is_empty()
            || !man
                .id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
        {
            return Err(ModuleError::MalformedId(man.id));
        }
        let expected = man.declared_hash();
        if man.build_hash != expected {
            return Err(ModuleError::TamperedManifest {
                id: man.id,
                expected,
            });
        }
        if self.modules.iter().any(|x| x.manifest().id == man.id) {
            return Err(ModuleError::DuplicateId(man.id));
        }
        self.modules.push(m);
        Ok(())
    }

    /// Load a module whose manifest a publisher signed.
    ///
    /// Three things must hold and each fails differently, because the operator
    /// response differs: an unverifiable signature is a corrupt or forged
    /// manifest, an untrusted publisher is a decision the operator has not
    /// made, and a mismatch means the signed document describes something else
    /// entirely.
    pub fn load_signed(
        &mut self,
        m: Box<dyn PolicyModule>,
        signed: &SignedManifest,
        trusted: &std::collections::HashSet<String>,
    ) -> Result<(), ModuleError> {
        if !signed.signature_verifies() {
            return Err(ModuleError::BadSignature {
                id: signed.manifest.id.clone(),
            });
        }
        if !trusted.contains(&signed.publisher_b32) {
            return Err(ModuleError::UntrustedPublisher {
                id: signed.manifest.id.clone(),
                publisher_b32: signed.publisher_b32.clone(),
            });
        }
        // The signature covers the DECLARED hash, so comparing hashes here
        // catches a signed manifest paired with a different binary: the exact
        // substitution a signature is supposed to prevent.
        let loaded = m.manifest();
        if loaded.declared_hash() != signed.manifest.declared_hash() {
            return Err(ModuleError::ManifestMismatch {
                declared: signed.manifest.id.clone(),
                loaded: loaded.id.clone(),
            });
        }
        self.load(m)
    }

    pub fn is_empty(&self) -> bool {
        self.modules.is_empty()
    }

    pub fn len(&self) -> usize {
        self.modules.len()
    }

    pub fn manifests(&self) -> Vec<&ModuleManifest> {
        self.modules.iter().map(|m| m.manifest()).collect()
    }

    pub fn health(&self, id: &str) -> ModuleHealth {
        self.health
            .lock()
            .ok()
            .and_then(|h| h.get(id).cloned())
            .unwrap_or_default()
    }

    /// A digest over the loaded set, in order.
    ///
    /// Enters the verdict preimage. Two nodes that agree on this ran the same
    /// pipeline; two verdicts that disagree on it were produced by different
    /// software, whatever else the record says. The empty registry has its own
    /// digest rather than an empty string, so "no modules" is a signed
    /// statement and not an absence somebody could introduce.
    pub fn config_digest(&self) -> String {
        let mut h = blake3::Hasher::new();
        h.update(b"emem.guard.modules.v1");
        h.update(&(self.modules.len() as u32).to_le_bytes());
        for m in &self.modules {
            let man = m.manifest();
            let b = man.build_hash.as_bytes();
            h.update(&(b.len() as u32).to_le_bytes());
            h.update(b);
        }
        h.finalize().to_hex().to_string()
    }

    /// Run every module eligible for this path.
    ///
    /// `enforcing` selects the fast, non-demoted set; shadow runs everything.
    /// A module whose data class forbids text gets an empty slice regardless
    /// of what the caller holds.
    pub fn run(
        &self,
        ctx: &ModuleContext<'_>,
        enforcing: bool,
    ) -> Vec<(ModuleManifest, ModuleVerdict)> {
        let empty: [&str; 0] = [];
        let mut out = Vec::new();
        for m in &self.modules {
            let man = m.manifest();
            let demoted = self.health(&man.id).demoted;
            let eligible = match man.latency_class {
                // A demoted module keeps producing signal in shadow; it just
                // stops being able to block.
                LatencyClass::Fast => !enforcing || !demoted,
                LatencyClass::Slow => !enforcing,
            };
            if !eligible {
                continue;
            }
            let scoped = ModuleContext {
                texts: match man.data_class {
                    DataClass::NeedsText => ctx.texts,
                    // Not a filter the module can opt out of: it is handed a
                    // different slice, so a module that declared it does not
                    // need text cannot read text.
                    DataClass::DigestsOnly => &empty,
                },
                tokens: ctx.tokens,
                claims: ctx.claims,
                text_digest: ctx.text_digest,
            };
            let started = Instant::now();
            let v = m.evaluate(&scoped);
            let elapsed = started.elapsed();
            self.record(man, elapsed);
            out.push((man.clone(), v));
        }
        out
    }

    /// Measure, and demote a fast module that keeps missing its budget.
    fn record(&self, man: &ModuleManifest, elapsed: Duration) {
        let Ok(mut book) = self.health.lock() else {
            return;
        };
        let h = book.entry(man.id.clone()).or_default();
        h.calls += 1;
        let ms = elapsed.as_millis().min(u32::MAX as u128) as u32;
        h.last_ms = ms;
        h.max_ms = h.max_ms.max(ms);
        if man.latency_class == LatencyClass::Fast && elapsed > FAST_BUDGET {
            h.overruns += 1;
            if h.overruns >= FAST_STRIKES && !h.demoted {
                h.demoted = true;
                // Loud, because a module silently leaving the enforcing path
                // is enforcement quietly getting weaker.
                eprintln!(
                    "emem-guard: module {} declared fast but exceeded {} ms {} times; \
                     demoted to shadow and no longer able to block",
                    man.id,
                    FAST_BUDGET.as_millis(),
                    h.overruns
                );
            }
        }
    }
}

impl std::fmt::Debug for ModuleRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModuleRegistry")
            .field(
                "ids",
                &self.manifests().iter().map(|m| &m.id).collect::<Vec<_>>(),
            )
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed {
        man: ModuleManifest,
        answer: ModuleVerdict,
        sleep: Duration,
        /// Shared with the test rather than owned, because the registry takes
        /// the module by value and the crate forbids unsafe.
        saw_text: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    impl Fixed {
        fn new(id: &str, latency: LatencyClass, data: DataClass) -> Self {
            Self {
                man: ModuleManifest {
                    id: id.into(),
                    version: "0.1.0".into(),
                    build_hash: String::new(),
                    latency_class: latency,
                    data_class: data,
                    deny_code: DenyCode::PolicyModule,
                    fix: Fix::RedactAndRetry,
                    description: "test".into(),
                }
                .sealed(),
                answer: ModuleVerdict::allow(),
                sleep: Duration::ZERO,
                saw_text: Default::default(),
            }
        }
        fn denying(mut self) -> Self {
            self.answer = ModuleVerdict::deny("found", b"secret-value", 0.9);
            self
        }
        fn slow_by(mut self, d: Duration) -> Self {
            self.sleep = d;
            self
        }
    }

    impl PolicyModule for Fixed {
        fn manifest(&self) -> &ModuleManifest {
            &self.man
        }
        fn evaluate(&self, ctx: &ModuleContext<'_>) -> ModuleVerdict {
            if !ctx.texts.is_empty() {
                self.saw_text
                    .store(true, std::sync::atomic::Ordering::SeqCst);
            }
            if !self.sleep.is_zero() {
                std::thread::sleep(self.sleep);
            }
            self.answer.clone()
        }
    }

    fn ctx<'a>(texts: &'a [&'a str], digest: &'a str) -> ModuleContext<'a> {
        ModuleContext {
            texts,
            tokens: &[],
            claims: &[],
            text_digest: digest,
        }
    }

    /// The acceptance criterion from the spec, and the reason the class exists
    /// at all: a slow module must be PROVABLY absent from the enforcing path,
    /// not merely discouraged from it.
    #[test]
    fn a_slow_module_never_runs_on_the_enforcing_path() {
        let mut r = ModuleRegistry::new();
        r.load(Box::new(
            Fixed::new("slowly", LatencyClass::Slow, DataClass::NeedsText).denying(),
        ))
        .unwrap();
        let texts = ["anything"];

        assert!(
            r.run(&ctx(&texts, "d"), true).is_empty(),
            "a slow module ran while enforcing"
        );
        // And it is not disabled, only excluded from blocking: shadow still
        // gets its signal, which is the whole point of loading it.
        let shadow = r.run(&ctx(&texts, "d"), false);
        assert_eq!(shadow.len(), 1);
        assert_eq!(shadow[0].1.outcome, ModuleOutcome::Deny);
    }

    /// A module that declared it does not need text must not be able to read
    /// text. Handing it a filtered slice is the enforcement; asking it nicely
    /// is not.
    #[test]
    fn a_digests_only_module_cannot_read_the_transcript() {
        let blind = Fixed::new("blind", LatencyClass::Fast, DataClass::DigestsOnly);
        let seen = blind.saw_text.clone();
        let reader = Fixed::new("reader", LatencyClass::Fast, DataClass::NeedsText);
        let read = reader.saw_text.clone();

        let mut r = ModuleRegistry::new();
        r.load(Box::new(blind)).unwrap();
        r.load(Box::new(reader)).unwrap();
        let texts = ["a secret transcript"];
        r.run(&ctx(&texts, "digest-abc"), true);

        assert!(
            !seen.load(std::sync::atomic::Ordering::SeqCst),
            "a digests_only module was handed transcript text"
        );
        // The control: the same text WAS available, so the absence above is
        // the filter working rather than an empty transcript.
        assert!(
            read.load(std::sync::atomic::Ordering::SeqCst),
            "the needs_text module should have seen it"
        );
    }

    /// The latency class is measured, not trusted. One slow module must not be
    /// able to take an organisation's enforcement offline.
    #[test]
    fn a_fast_module_that_lies_about_its_budget_is_demoted() {
        let mut r = ModuleRegistry::new();
        r.load(Box::new(
            Fixed::new("liar", LatencyClass::Fast, DataClass::DigestsOnly)
                .denying()
                .slow_by(FAST_BUDGET + Duration::from_millis(20)),
        ))
        .unwrap();
        let texts: [&str; 0] = [];

        for _ in 0..FAST_STRIKES {
            assert_eq!(
                r.run(&ctx(&texts, "d"), true).len(),
                1,
                "runs until demoted"
            );
        }
        assert!(r.health("liar").demoted);
        assert!(
            r.run(&ctx(&texts, "d"), true).is_empty(),
            "a demoted module must no longer be able to block"
        );
        // But it keeps producing signal where it cannot do harm.
        assert_eq!(r.run(&ctx(&texts, "d"), false).len(), 1);
        assert!(r.health("liar").max_ms >= FAST_BUDGET.as_millis() as u32);
    }

    /// A well-behaved fast module is never demoted, or the strike rule would
    /// erode enforcement on its own.
    #[test]
    fn a_module_that_keeps_its_budget_is_left_alone() {
        let mut r = ModuleRegistry::new();
        r.load(Box::new(Fixed::new(
            "quick",
            LatencyClass::Fast,
            DataClass::DigestsOnly,
        )))
        .unwrap();
        let texts: [&str; 0] = [];
        for _ in 0..20 {
            r.run(&ctx(&texts, "d"), true);
        }
        let h = r.health("quick");
        assert_eq!((h.overruns, h.demoted), (0, false));
        assert_eq!(h.calls, 20);
    }

    /// The acceptance criterion: a tampered manifest is refused AT LOAD, not
    /// noticed later. A module that claimed fast while being reviewed as slow
    /// is the attack.
    #[test]
    fn a_manifest_that_does_not_cover_itself_is_refused_at_load() {
        let mut m = Fixed::new("edited", LatencyClass::Slow, DataClass::NeedsText);
        // Sealed as Slow, then edited to claim Fast, exactly as an operator
        // sneaking a model call onto the enforcing path would.
        m.man.latency_class = LatencyClass::Fast;
        let mut r = ModuleRegistry::new();
        let err = r.load(Box::new(m)).unwrap_err();
        assert!(matches!(err, ModuleError::TamperedManifest { .. }), "{err}");
        assert_eq!(r.len(), 0, "nothing was loaded");
    }

    /// Two modules with one id would be ambiguous in the log, which is the one
    /// place the id has to mean something.
    #[test]
    fn ids_are_unique_and_well_formed() {
        let mut r = ModuleRegistry::new();
        r.load(Box::new(Fixed::new(
            "dup",
            LatencyClass::Fast,
            DataClass::DigestsOnly,
        )))
        .unwrap();
        assert_eq!(
            r.load(Box::new(Fixed::new(
                "dup",
                LatencyClass::Fast,
                DataClass::DigestsOnly
            )))
            .unwrap_err(),
            ModuleError::DuplicateId("dup".into())
        );
        let bad = Fixed::new("Has Spaces", LatencyClass::Fast, DataClass::DigestsOnly);
        assert!(matches!(
            r.load(Box::new(bad)).unwrap_err(),
            ModuleError::MalformedId(_)
        ));
    }

    /// The config digest is what makes a verdict reproducible: it has to move
    /// when the pipeline moves, including when only the ORDER moves, because
    /// first-match-wins makes order semantic.
    #[test]
    fn the_config_digest_moves_when_the_pipeline_does() {
        let mk = |id: &str| {
            Box::new(Fixed::new(id, LatencyClass::Fast, DataClass::DigestsOnly))
                as Box<dyn PolicyModule>
        };
        let empty = ModuleRegistry::new().config_digest();
        assert!(!empty.is_empty(), "no modules is a signed statement too");

        let mut a = ModuleRegistry::new();
        a.load(mk("one")).unwrap();
        a.load(mk("two")).unwrap();

        let mut b = ModuleRegistry::new();
        b.load(mk("two")).unwrap();
        b.load(mk("one")).unwrap();

        let mut c = ModuleRegistry::new();
        c.load(mk("one")).unwrap();

        assert_ne!(a.config_digest(), empty);
        assert_ne!(a.config_digest(), c.config_digest(), "a module was added");
        assert_ne!(
            a.config_digest(),
            b.config_digest(),
            "order is semantic under first-match-wins, so it must be bound"
        );
        // And it is stable for the same set in the same order.
        let mut a2 = ModuleRegistry::new();
        a2.load(mk("one")).unwrap();
        a2.load(mk("two")).unwrap();
        assert_eq!(a.config_digest(), a2.config_digest());
    }

    /// A publisher signs the DECLARED hash, so the same manifest signed once
    /// cannot be re-pointed at a different module afterwards.
    #[test]
    fn a_signed_manifest_loads_only_from_a_publisher_the_operator_listed() {
        use ed25519_dalek::Signer as _;
        let key = ed25519_dalek::SigningKey::from_bytes(&[3u8; 32]);
        let pubk = data_encoding::BASE32_NOPAD
            .encode(key.verifying_key().as_bytes())
            .to_lowercase();

        let m = Fixed::new("vendor-engine", LatencyClass::Slow, DataClass::NeedsText);
        let mut signed = SignedManifest {
            manifest: m.man.clone(),
            publisher_b32: pubk.clone(),
            signature_b32: String::new(),
        };
        signed.signature_b32 = data_encoding::BASE32_NOPAD
            .encode(&key.sign(&signed.preimage()).to_bytes())
            .to_lowercase();
        assert!(signed.signature_verifies());

        // Genuinely signed, publisher not listed: refused.
        let mut r = ModuleRegistry::new();
        let empty = std::collections::HashSet::new();
        let err = r
            .load_signed(
                Box::new(Fixed::new(
                    "vendor-engine",
                    LatencyClass::Slow,
                    DataClass::NeedsText,
                )),
                &signed,
                &empty,
            )
            .unwrap_err();
        assert!(
            matches!(err, ModuleError::UntrustedPublisher { .. }),
            "{err}"
        );
        assert_eq!(r.len(), 0);

        // Listed: loads.
        let trusted: std::collections::HashSet<String> = [pubk].into_iter().collect();
        r.load_signed(Box::new(m), &signed, &trusted).unwrap();
        assert_eq!(r.len(), 1);
    }

    /// The substitution a signature exists to prevent: a manifest signed for
    /// one module, paired with a different binary.
    #[test]
    fn a_signed_manifest_cannot_be_paired_with_a_different_module() {
        use ed25519_dalek::Signer as _;
        let key = ed25519_dalek::SigningKey::from_bytes(&[4u8; 32]);
        let pubk = data_encoding::BASE32_NOPAD
            .encode(key.verifying_key().as_bytes())
            .to_lowercase();
        // Signed as a SLOW module, which is what a reviewer would have
        // approved.
        let reviewed = Fixed::new("engine", LatencyClass::Slow, DataClass::DigestsOnly);
        let mut signed = SignedManifest {
            manifest: reviewed.man.clone(),
            publisher_b32: pubk.clone(),
            signature_b32: String::new(),
        };
        signed.signature_b32 = data_encoding::BASE32_NOPAD
            .encode(&key.sign(&signed.preimage()).to_bytes())
            .to_lowercase();

        let trusted: std::collections::HashSet<String> = [pubk].into_iter().collect();
        let mut r = ModuleRegistry::new();
        // ...but the binary handed over declares FAST and wants text.
        let swapped = Fixed::new("engine", LatencyClass::Fast, DataClass::NeedsText);
        let err = r
            .load_signed(Box::new(swapped), &signed, &trusted)
            .unwrap_err();
        assert!(matches!(err, ModuleError::ManifestMismatch { .. }), "{err}");
        assert_eq!(r.len(), 0, "nothing loaded");
    }

    /// A forged or corrupt signature is a different failure from an untrusted
    /// one, because the operator response differs.
    #[test]
    fn a_signature_that_does_not_verify_is_named_as_such() {
        let m = Fixed::new("forged", LatencyClass::Slow, DataClass::DigestsOnly);
        let signed = SignedManifest {
            manifest: m.man.clone(),
            publisher_b32: "a".repeat(52),
            signature_b32: "b".repeat(103),
        };
        assert!(!signed.signature_verifies());
        let mut r = ModuleRegistry::new();
        let trusted: std::collections::HashSet<String> = ["a".repeat(52)].into_iter().collect();
        let err = r.load_signed(Box::new(m), &signed, &trusted).unwrap_err();
        assert!(matches!(err, ModuleError::BadSignature { .. }), "{err}");
    }

    /// Matched content must never reach the log. A denial carries a digest of
    /// what it found, and the constructor hashes it so an author cannot leak
    /// it by accident.
    #[test]
    fn a_denial_carries_a_digest_and_never_the_content() {
        let v = ModuleVerdict::deny("aws key", b"AKIAIOSFODNN7EXAMPLE", 0.99);
        let d = v.evidence_digest.unwrap();
        assert_eq!(d.len(), 64, "hex blake3");
        assert!(!d.contains("AKIA"));
        // The reason is the operator's, and it is the author's job to keep the
        // secret out of it; nothing else in this crate ever sees the content.
        assert_eq!(v.reason.as_deref(), Some("aws key"));
        assert!(v.confidence <= 1.0);
    }

    /// An abstain is not a block. A module whose sidecar is down must not turn
    /// somebody else's outage into a denied request.
    #[test]
    fn an_abstain_is_never_a_block() {
        let v = ModuleVerdict::abstain("sidecar unreachable");
        assert_eq!(v.outcome, ModuleOutcome::Abstain);
        assert_eq!(v.confidence, 0.0);
        assert!(v.evidence_digest.is_none());
    }
}
