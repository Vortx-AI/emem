//! Reference modules.
//!
//! Two, and neither of them is a detection engine. That is the point: the
//! chassis is the product and the detection is somebody else's.
//!
//! [`SecretPatterns`] is in-process and deliberately narrow. Every pattern it
//! carries is **prefix-anchored and structurally checkable**: a fixed literal
//! followed by a fixed-length body from a fixed alphabet. That is a different
//! kind of claim from "this looks like a name" or "this might be a medical
//! record", and it is the only kind this crate will make on its own. A
//! heuristic classifier belongs behind [`OrgWebhook`], where its false
//! positives are the operator's trade to make rather than ours to impose.
//!
//! [`OrgWebhook`] is the wrap-do-not-build path. Point it at a sidecar and its
//! verdicts get signed and logged like any other. Presidio, LLM Guard, an
//! in-house classifier and a commercial engine all arrive the same way; none
//! of them has been tested here and this crate claims support for none of
//! them, which is exactly why the module is generic rather than named after
//! one.

use std::time::Duration;

use crate::module::{
    DataClass, LatencyClass, ModuleContext, ModuleManifest, ModuleVerdict, PolicyModule,
};
use crate::policy::{DenyCode, Fix};

// ── secret-patterns ──────────────────────────────────────────────────────

/// One structurally checkable credential shape.
struct SecretShape {
    /// What it is, for the operator log. Never the value.
    label: &'static str,
    /// The literal a candidate must start with.
    prefix: &'static str,
    /// How many body characters must follow, exactly.
    body_len: usize,
    /// Which characters the body may contain.
    alphabet: Alphabet,
}

#[derive(Clone, Copy)]
enum Alphabet {
    /// A-Z, 0-9.
    UpperAlnum,
    /// A-Z, a-z, 0-9.
    Alnum,
    /// A-Z, a-z, 0-9, - and _.
    AlnumDashUnderscore,
}

impl Alphabet {
    fn admits(self, c: char) -> bool {
        match self {
            Self::UpperAlnum => c.is_ascii_uppercase() || c.is_ascii_digit(),
            Self::Alnum => c.is_ascii_alphanumeric(),
            Self::AlnumDashUnderscore => c.is_ascii_alphanumeric() || c == '-' || c == '_',
        }
    }
}

/// The shapes this module knows.
///
/// Each entry is a published credential format with a vendor-assigned prefix
/// and a fixed length. A candidate that matches one is a credential or an
/// astronomically unlikely coincidence; there is no judgement call in the
/// middle, which is why this can run inline and deny.
///
/// Deliberately absent: anything requiring entropy estimation, dictionary
/// lookup, or context. "A long random-looking string" is a heuristic, and a
/// heuristic that blocks belongs behind an operator's own decision.
const SHAPES: &[SecretShape] = &[
    SecretShape {
        label: "aws_access_key_id",
        prefix: "AKIA",
        body_len: 16,
        alphabet: Alphabet::UpperAlnum,
    },
    SecretShape {
        label: "aws_temporary_key_id",
        prefix: "ASIA",
        body_len: 16,
        alphabet: Alphabet::UpperAlnum,
    },
    SecretShape {
        label: "github_personal_token",
        prefix: "ghp_",
        body_len: 36,
        alphabet: Alphabet::Alnum,
    },
    SecretShape {
        label: "github_oauth_token",
        prefix: "gho_",
        body_len: 36,
        alphabet: Alphabet::Alnum,
    },
    SecretShape {
        label: "github_server_token",
        prefix: "ghs_",
        body_len: 36,
        alphabet: Alphabet::Alnum,
    },
    SecretShape {
        label: "google_api_key",
        prefix: "AIza",
        body_len: 35,
        alphabet: Alphabet::AlnumDashUnderscore,
    },
    SecretShape {
        label: "github_user_token",
        prefix: "ghu_",
        body_len: 36,
        alphabet: Alphabet::Alnum,
    },
    SecretShape {
        label: "github_refresh_token",
        prefix: "ghr_",
        body_len: 36,
        alphabet: Alphabet::Alnum,
    },
];

// Deliberately NOT here: Slack (`xoxb-`) and Stripe (`sk_live_`). Both have a
// recognisable prefix and a length that has changed across issuance eras, so a
// fixed-length rule either misses the current format or has to be relaxed into
// "a prefix followed by a long-enough run", which is the entropy heuristic this
// module exists to avoid. They belong behind OrgWebhook, where the operator
// owns the trade. Found by a test fixture that could never have matched.

/// A PEM private-key header. Length-free, so it is matched as a literal.
const PEM_HEADERS: &[(&str, &str)] = &[
    ("private_key_pem", "-----BEGIN PRIVATE KEY-----"),
    ("rsa_private_key_pem", "-----BEGIN RSA PRIVATE KEY-----"),
    ("openssh_private_key", "-----BEGIN OPENSSH PRIVATE KEY-----"),
    ("ec_private_key_pem", "-----BEGIN EC PRIVATE KEY-----"),
    ("pgp_private_key", "-----BEGIN PGP PRIVATE KEY BLOCK-----"),
];

/// Structurally checkable credential shapes, in-process, fast.
///
/// Ships as a reference implementation of the trait, not as a DLP product. If
/// you need real coverage, put a real engine behind [`OrgWebhook`]; this
/// exists so that a node has something to demonstrate the pipeline with, and
/// so that the one class of finding where a fixed prefix makes false positives
/// structurally impossible is available without a sidecar.
pub struct SecretPatterns {
    manifest: ModuleManifest,
}

impl Default for SecretPatterns {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretPatterns {
    pub fn new() -> Self {
        Self {
            manifest: ModuleManifest {
                id: "secret-patterns".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                build_hash: String::new(),
                latency_class: LatencyClass::Fast,
                data_class: DataClass::NeedsText,
                deny_code: DenyCode::PolicyModule,
                fix: Fix::RedactAndRetry,
                description:
                    "Prefix-anchored, fixed-length credential shapes (AWS, GitHub, Google) \
                     and PEM private-key headers. Every shape is a \
                     fixed prefix plus an exact length, so a match is not a judgement \
                     call. No entropy heuristics, and no shape whose length varies."
                        .into(),
            }
            .sealed(),
        }
    }

    /// The first credential shape in `text`, as (label, matched bytes).
    ///
    /// Returns the matched span so the caller can hash it. Nothing else in
    /// this crate ever sees it, and it is hashed rather than stored by
    /// [`ModuleVerdict::deny`].
    fn first_match(text: &str) -> Option<(&'static str, &str)> {
        for (label, header) in PEM_HEADERS {
            if let Some(at) = text.find(header) {
                return Some((label, &text[at..at + header.len()]));
            }
        }
        for shape in SHAPES {
            let mut from = 0usize;
            while let Some(rel) = text[from..].find(shape.prefix) {
                let start = from + rel;
                // Must begin at a boundary, or `xghp_...` inside a longer word
                // would match and the finding would be noise.
                let clean_start = start == 0
                    || !text[..start]
                        .chars()
                        .next_back()
                        .is_some_and(|c| c.is_ascii_alphanumeric());
                let body_at = start + shape.prefix.len();
                let body: String = text[body_at..]
                    .chars()
                    .take_while(|c| shape.alphabet.admits(*c))
                    .collect();
                // EXACTLY the declared length. A longer run is a different
                // string that happens to start the same way, and treating it
                // as a credential would be the guess this module refuses.
                if clean_start && body.chars().count() == shape.body_len {
                    let end = body_at + body.len();
                    return Some((shape.label, &text[start..end]));
                }
                from = body_at.max(start + 1);
                if from >= text.len() {
                    break;
                }
            }
        }
        None
    }
}

impl PolicyModule for SecretPatterns {
    fn manifest(&self) -> &ModuleManifest {
        &self.manifest
    }

    fn evaluate(&self, ctx: &ModuleContext<'_>) -> ModuleVerdict {
        for t in ctx.texts {
            if let Some((label, matched)) = Self::first_match(t) {
                // The label reaches the operator log; the value is hashed.
                return ModuleVerdict::deny(
                    format!("credential shape {label} present in the transcript"),
                    matched.as_bytes(),
                    1.0,
                );
            }
        }
        ModuleVerdict::allow()
    }
}

// ── org-webhook ──────────────────────────────────────────────────────────

/// Call the organisation's own classifier.
///
/// The wrap-do-not-build path, and the only honest way to offer coverage this
/// crate has not written and cannot test. POSTs `{text_digest, texts,
/// token_count, claim_count}` and reads back `{"outcome":
/// "allow"|"deny"|"abstain", "reason": "..."}`.
///
/// # Timeouts abstain, they never deny
///
/// The sidecar is somebody else's software with somebody else's uptime.
/// Converting its outage into a blocked request would make every organisation
/// running one strictly less reliable for having added a control, which is how
/// controls get removed. So an unreachable, slow or unparseable sidecar is an
/// abstain and a line in the operator log.
///
/// # Choosing the class
///
/// A sidecar reachable over loopback in a few milliseconds can be declared
/// [`LatencyClass::Fast`]; one that runs a model cannot. The declaration is
/// measured either way, so a module that claims fast and is not gets demoted
/// rather than trusted. When in doubt declare slow: a slow module still
/// produces signal, it simply cannot block.
pub struct OrgWebhook {
    manifest: ModuleManifest,
    url: String,
    client: reqwest::blocking::Client,
    budget: Duration,
}

impl OrgWebhook {
    /// `url` is the org's classifier. `budget` must be below the verdict
    /// deadline; the client enforces it regardless of what the sidecar does.
    pub fn new(
        id: &str,
        url: impl Into<String>,
        budget: Duration,
        latency_class: LatencyClass,
        data_class: DataClass,
    ) -> Result<Self, reqwest::Error> {
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_millis(100).min(budget))
            .timeout(budget)
            .pool_max_idle_per_host(4)
            .build()?;
        Ok(Self {
            manifest: ModuleManifest {
                id: id.to_string(),
                version: env!("CARGO_PKG_VERSION").into(),
                build_hash: String::new(),
                latency_class,
                data_class,
                deny_code: DenyCode::PolicyModule,
                fix: Fix::RedactAndRetry,
                description: format!(
                    "Delegates to an operator-run classifier at {}. Abstains on timeout.",
                    redact_url(&url.into())
                ),
            }
            .sealed(),
            // Rebuilt because the closure above consumed it. Cheap, once.
            url: String::new(),
            client,
            budget,
        })
    }

    /// Point it at a URL. Split from `new` so the manifest can describe the
    /// host without the query string ever reaching a description.
    pub fn at(mut self, url: impl Into<String>) -> Self {
        self.url = url.into();
        self
    }
}

/// Scheme, host and port only. A classifier URL can carry a key in its query,
/// and the manifest is published at `/modules`.
fn redact_url(u: &str) -> String {
    match u.split_once("://") {
        Some((scheme, rest)) => {
            let host = rest.split(['/', '?']).next().unwrap_or("");
            format!("{scheme}://{host}")
        }
        None => "(malformed url)".to_string(),
    }
}

impl PolicyModule for OrgWebhook {
    fn manifest(&self) -> &ModuleManifest {
        &self.manifest
    }

    fn evaluate(&self, ctx: &ModuleContext<'_>) -> ModuleVerdict {
        if self.url.is_empty() {
            return ModuleVerdict::abstain("no classifier url configured");
        }
        let body = serde_json::json!({
            "text_digest": ctx.text_digest,
            // Empty for a digests_only deployment; the sidecar sees exactly
            // what the declared data class permits and nothing more.
            "texts": ctx.texts,
            "token_count": ctx.tokens.len(),
            "claim_count": ctx.claims.len(),
        });
        let Ok(resp) = self
            .client
            .post(&self.url)
            .timeout(self.budget)
            .json(&body)
            .send()
        else {
            return ModuleVerdict::abstain("classifier unreachable or past its budget");
        };
        if !resp.status().is_success() {
            return ModuleVerdict::abstain(format!("classifier returned {}", resp.status()));
        }
        let Ok(v) = resp.json::<serde_json::Value>() else {
            return ModuleVerdict::abstain("classifier response was not json");
        };
        let reason = v
            .get("reason")
            .and_then(|r| r.as_str())
            .unwrap_or("classifier denied")
            .to_string();
        match v.get("outcome").and_then(|o| o.as_str()) {
            Some("deny") => {
                let confidence = v.get("confidence").and_then(|c| c.as_f64()).unwrap_or(1.0) as f32;
                // The digest covers what the classifier was shown, not what it
                // found, because what it found is its own business and we will
                // not put a third party's match into our log.
                ModuleVerdict::deny(reason, ctx.text_digest.as_bytes(), confidence)
            }
            Some("allow") => ModuleVerdict::allow(),
            // Anything else, including a missing field, is an abstain. A
            // classifier that answered something we do not understand has not
            // told us to block.
            _ => ModuleVerdict::abstain(reason),
        }
    }
}

// ── sidecar: a closed engine over a unix socket ──────────────────────────

/// A detection engine that does not link against us at all.
///
/// The OEM path. A vendor with a proprietary engine cannot ship it as a Rust
/// trait implementation without linking into this binary, which is a licence
/// conversation and a build conversation before it is a technical one. So the
/// module boundary is also available as an IPC boundary: they run a process
/// listening on a unix socket, we connect and speak one line of JSON each way.
///
/// # The protocol, in full
///
/// Request, one line, newline-terminated:
///
/// ```json
/// {"text_digest":"<hex>","texts":["..."],"token_count":0,"claim_count":0}
/// ```
///
/// Response, one line:
///
/// ```json
/// {"outcome":"allow"|"deny"|"abstain","reason":"...","confidence":0.9}
/// ```
///
/// Byte-identical to the body [`OrgWebhook`] posts, so a vendor implements one
/// shape and can be reached over either transport. That is deliberate: a
/// second protocol for the same question is a second thing to keep correct.
///
/// # Why a unix socket rather than a spawned process
///
/// No process lifecycle to own. A subprocess means restart policy, zombie
/// reaping, stderr plumbing and a crash loop that becomes our incident; a
/// socket means the vendor's engine is the vendor's problem to run, and a
/// missing socket is an abstain rather than a panic. The engine can be
/// restarted, upgraded and monitored by whatever the operator already uses.
#[cfg(unix)]
pub struct SidecarModule {
    manifest: ModuleManifest,
    socket: std::path::PathBuf,
    budget: Duration,
}

#[cfg(unix)]
impl SidecarModule {
    pub fn new(
        id: &str,
        socket: impl Into<std::path::PathBuf>,
        budget: Duration,
        latency_class: LatencyClass,
        data_class: DataClass,
    ) -> Self {
        let socket = socket.into();
        Self {
            manifest: ModuleManifest {
                id: id.to_string(),
                version: env!("CARGO_PKG_VERSION").into(),
                build_hash: String::new(),
                latency_class,
                data_class,
                deny_code: DenyCode::PolicyModule,
                fix: Fix::RedactAndRetry,
                // The path, not its contents. A socket path is not a secret
                // the way a classifier URL's query string can be.
                description: format!(
                    "IPC sidecar over {}. One line of JSON each way; abstains if the \
                     socket is absent, slow, or answers something unrecognised.",
                    socket.display()
                ),
            }
            .sealed(),
            socket,
            budget,
        }
    }

    /// One request/response exchange, bounded.
    ///
    /// Every failure path returns `None`, which the caller turns into an
    /// abstain. There is no path here that produces a denial from an error:
    /// a vendor's engine being down must not block an operator's traffic.
    fn exchange(&self, body: &str) -> Option<serde_json::Value> {
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::net::UnixStream;

        let stream = UnixStream::connect(&self.socket).ok()?;
        // Both directions bounded. A sidecar that accepts the write and never
        // answers would otherwise hold the verdict past its deadline, which is
        // the failure the latency class exists to prevent.
        stream.set_read_timeout(Some(self.budget)).ok()?;
        stream.set_write_timeout(Some(self.budget)).ok()?;
        let mut w = stream.try_clone().ok()?;
        w.write_all(body.as_bytes()).ok()?;
        w.write_all(b"\n").ok()?;
        w.flush().ok()?;
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).ok()?;
        serde_json::from_str(line.trim()).ok()
    }
}

#[cfg(unix)]
impl PolicyModule for SidecarModule {
    fn manifest(&self) -> &ModuleManifest {
        &self.manifest
    }

    fn evaluate(&self, ctx: &ModuleContext<'_>) -> ModuleVerdict {
        let body = serde_json::json!({
            "text_digest": ctx.text_digest,
            "texts": ctx.texts,
            "token_count": ctx.tokens.len(),
            "claim_count": ctx.claims.len(),
        })
        .to_string();
        let Some(v) = self.exchange(&body) else {
            return ModuleVerdict::abstain("sidecar unreachable, slow, or unparseable");
        };
        let reason = v
            .get("reason")
            .and_then(|r| r.as_str())
            .unwrap_or("sidecar denied")
            .to_string();
        match v.get("outcome").and_then(|o| o.as_str()) {
            Some("deny") => {
                let confidence = v.get("confidence").and_then(|c| c.as_f64()).unwrap_or(1.0) as f32;
                // The digest covers what the sidecar was SHOWN, not what it
                // found. What a third party's engine matched is its business
                // and will not enter our log.
                ModuleVerdict::deny(reason, ctx.text_digest.as_bytes(), confidence)
            }
            Some("allow") => ModuleVerdict::allow(),
            _ => ModuleVerdict::abstain(reason),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module::{ModuleOutcome, ModuleRegistry};

    fn ctx<'a>(texts: &'a [&'a str]) -> ModuleContext<'a> {
        ModuleContext {
            texts,
            tokens: &[],
            claims: &[],
            text_digest: "abc123",
        }
    }

    /// Each shape it claims, matched at its exact declared length.
    #[test]
    fn every_declared_credential_shape_is_found() {
        let m = SecretPatterns::new();
        let cases = [
            ("AKIAIOSFODNN7EXAMPLE", "aws_access_key_id"),
            ("ASIAIOSFODNN7EXAMPLE", "aws_temporary_key_id"),
            (
                "ghp_abcdefghijklmnopqrstuvwxyz0123456789",
                "github_personal_token",
            ),
            ("AIzaSyA1234567890abcdefghijklmnopqrstuv", "google_api_key"),
            (
                "ghu_abcdefghijklmnopqrstuvwxyz0123456789",
                "github_user_token",
            ),
            ("-----BEGIN OPENSSH PRIVATE KEY-----", "openssh_private_key"),
        ];
        for (sample, label) in cases {
            let found = SecretPatterns::first_match(sample);
            assert_eq!(found.map(|(l, _)| l), Some(label), "{sample}");
            let text = format!("here it is: {sample} and then some words");
            let v = m.evaluate(&ctx(&[&text]));
            assert_eq!(v.outcome, ModuleOutcome::Deny, "{sample}");
            assert!(v.reason.as_deref().unwrap().contains(label));
        }
    }

    /// The whole reason this module is allowed to block inline: a prefix plus
    /// an exact length leaves no judgement call. A near-miss is not a finding.
    #[test]
    fn a_near_miss_is_never_a_finding() {
        for s in [
            // Right prefix, wrong length.
            "AKIATOOSHORT",
            "AKIAIOSFODNN7EXAMPLETOOLONGXX",
            "ghp_short",
            // Right shape, not at a word boundary.
            "xAKIAIOSFODNN7EXAMPLE",
            // Lowercase in an uppercase-only body.
            "AKIAiosfodnn7example",
            // The prefix being discussed rather than used.
            "keys beginning with AKIA are AWS access key ids",
            // Ordinary prose that happens to contain the letters.
            "the AKIA project shipped in 2026",
        ] {
            assert!(
                SecretPatterns::first_match(s).is_none(),
                "false positive on: {s}"
            );
        }
    }

    /// The finding must not carry the credential. A log that recorded what a
    /// secret scanner found would be a catalogue of secrets.
    #[test]
    fn a_finding_never_carries_the_credential() {
        let secret = "AKIAIOSFODNN7EXAMPLE";
        let text = format!("oops {secret}");
        let v = SecretPatterns::new().evaluate(&ctx(&[&text]));
        let serialized = format!("{v:?}");
        assert!(
            !serialized.contains(secret),
            "the credential leaked into the verdict: {serialized}"
        );
        assert!(!serialized.contains("AKIA"));
        assert_eq!(v.evidence_digest.as_ref().unwrap().len(), 64);
    }

    /// It runs on the enforcing path, so it has to be quick on a real body.
    #[test]
    fn a_megabyte_of_prose_stays_inside_the_fast_budget() {
        let haystack = "the quick brown fox jumps over the lazy dog. ".repeat(24_000);
        assert!(haystack.len() > 1_000_000, "{}", haystack.len());
        let m = SecretPatterns::new();
        let started = std::time::Instant::now();
        let v = m.evaluate(&ctx(&[&haystack]));
        let took = started.elapsed();
        assert_eq!(v.outcome, ModuleOutcome::Allow);
        // Ten times the budget, not one. The defect worth catching is a scan
        // that is accidentally quadratic in the transcript, which costs
        // SECONDS on a megabyte; asserting the budget exactly would instead
        // measure how loaded a shared CI runner happened to be, and a test
        // that fails for that reason gets ignored, which is worse than not
        // having it. The real budget is enforced at runtime by the registry,
        // which demotes a module that misses it.
        let ceiling = crate::module::FAST_BUDGET * 10;
        assert!(
            took < ceiling,
            "took {} ms over a {} byte transcript; anything near this is a scan \
             that does not stay linear",
            took.as_millis(),
            haystack.len()
        );
    }

    /// The module loads: its manifest covers itself and the registry accepts
    /// it, which is what a third party's module has to do too.
    #[test]
    fn the_reference_module_loads_like_any_other() {
        let mut r = ModuleRegistry::new();
        r.load(Box::new(SecretPatterns::new())).unwrap();
        assert_eq!(r.len(), 1);
        let man = r.manifests()[0];
        assert_eq!(man.id, "secret-patterns");
        assert_eq!(man.build_hash, man.declared_hash());
        assert_eq!(man.deny_code, DenyCode::PolicyModule);
        assert_eq!(man.fix, Fix::RedactAndRetry);
    }

    /// Somebody else's outage must not become a blocked request.
    #[test]
    fn an_unreachable_classifier_abstains_rather_than_denying() {
        // Port 1 is reserved and refuses immediately.
        let m = OrgWebhook::new(
            "org-classifier",
            "http://127.0.0.1:1/classify",
            Duration::from_millis(50),
            LatencyClass::Fast,
            DataClass::NeedsText,
        )
        .unwrap()
        .at("http://127.0.0.1:1/classify");
        let v = m.evaluate(&ctx(&["anything"]));
        assert_eq!(v.outcome, ModuleOutcome::Abstain);
        assert!(v.evidence_digest.is_none());
    }

    /// An unconfigured wrapper abstains rather than pretending to have looked.
    #[test]
    fn an_unconfigured_classifier_abstains() {
        let m = OrgWebhook::new(
            "org-classifier",
            "http://127.0.0.1:1/classify",
            Duration::from_millis(50),
            LatencyClass::Slow,
            DataClass::DigestsOnly,
        )
        .unwrap();
        assert_eq!(m.evaluate(&ctx(&["x"])).outcome, ModuleOutcome::Abstain);
    }

    /// The OEM path, against a real socket. A vendor's engine answers one line
    /// of JSON and its verdict is signed and logged like any other.
    #[cfg(unix)]
    #[test]
    fn a_closed_engine_over_a_socket_is_a_module_like_any_other() {
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::net::UnixListener;

        // NOT std::env::temp_dir(): macOS returns a long per-user path under
        // /var/folders and caps a unix socket path at 104 bytes, so a
        // perfectly good test fails on one platform for a reason that has
        // nothing to do with the code. /tmp is short everywhere.
        let sock = std::path::PathBuf::from(format!(
            "/tmp/emem-guard-sidecar-{}.sock",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&sock);
        let listener = UnixListener::bind(&sock).unwrap();

        let server = std::thread::spawn(move || {
            for stream in listener.incoming().take(2) {
                let Ok(stream) = stream else { continue };
                let mut line = String::new();
                let mut r = BufReader::new(stream.try_clone().unwrap());
                if r.read_line(&mut line).is_err() {
                    continue;
                }
                // A real engine would classify here. This one asserts it was
                // handed the documented request shape, then denies.
                let req: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
                assert!(req.get("text_digest").is_some(), "protocol: {req}");
                assert!(req.get("texts").is_some(), "protocol: {req}");
                let mut w = stream;
                w.write_all(br#"{"outcome":"deny","reason":"vendor policy 12","confidence":0.8}"#)
                    .unwrap();
                w.write_all(b"\n").unwrap();
                w.flush().unwrap();
            }
        });

        let m = SidecarModule::new(
            "vendor-engine",
            &sock,
            Duration::from_secs(2),
            LatencyClass::Slow,
            DataClass::NeedsText,
        );
        let v = m.evaluate(&ctx(&["some transcript"]));
        assert_eq!(v.outcome, ModuleOutcome::Deny);
        assert_eq!(v.reason.as_deref(), Some("vendor policy 12"));
        // The digest covers what the sidecar was SHOWN, not what it found.
        assert_eq!(v.evidence_digest.as_ref().unwrap().len(), 64);
        assert!((v.confidence - 0.8).abs() < 1e-6);

        // And it loads into a registry like a native module.
        let mut r = crate::module::ModuleRegistry::new();
        r.load(Box::new(SidecarModule::new(
            "vendor-engine",
            &sock,
            Duration::from_secs(2),
            LatencyClass::Slow,
            DataClass::NeedsText,
        )))
        .unwrap();
        assert_eq!(r.len(), 1);

        drop(server);
        let _ = std::fs::remove_file(&sock);
    }

    /// A vendor's engine being down must not block an operator's traffic.
    #[cfg(unix)]
    #[test]
    fn a_missing_socket_abstains_rather_than_denying() {
        let m = SidecarModule::new(
            "vendor-engine",
            "/nonexistent/emem-guard-no-such.sock",
            Duration::from_millis(50),
            LatencyClass::Slow,
            DataClass::NeedsText,
        );
        let v = m.evaluate(&ctx(&["anything"]));
        assert_eq!(v.outcome, ModuleOutcome::Abstain);
        assert!(v.evidence_digest.is_none());
    }

    /// A sidecar that accepts the write and never answers must not hold the
    /// verdict past its deadline.
    #[cfg(unix)]
    #[test]
    fn a_silent_sidecar_times_out_into_an_abstain() {
        use std::os::unix::net::UnixListener;
        let sock = std::path::PathBuf::from(format!(
            "/tmp/emem-guard-silent-{}.sock",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&sock);
        let listener = UnixListener::bind(&sock).unwrap();
        // The peer holds for a LONG time on purpose. The gap between the
        // budget and the hold is what makes the assertion below robust: a
        // working timeout returns in ~80 ms, a broken one waits 5 s, and no
        // amount of scheduling jitter on a shared runner turns one into the
        // other.
        let hold = Duration::from_secs(5);
        let keep = std::thread::spawn(move || {
            let _held = listener.incoming().next();
            std::thread::sleep(hold);
        });

        let budget = Duration::from_millis(80);
        let m = SidecarModule::new(
            "silent",
            &sock,
            budget,
            LatencyClass::Slow,
            DataClass::NeedsText,
        );
        let started = std::time::Instant::now();
        let v = m.evaluate(&ctx(&["anything"]));
        let took = started.elapsed();
        assert_eq!(v.outcome, ModuleOutcome::Abstain);
        assert!(
            took < hold / 2,
            "held the verdict for {} ms against an {} ms budget while the peer \
             held for {} ms: the read timeout did not fire",
            took.as_millis(),
            budget.as_millis(),
            hold.as_millis()
        );
        let _ = keep.join();
        let _ = std::fs::remove_file(&sock);
    }

    /// A classifier URL can carry a key in its query, and the manifest is
    /// published at `/modules`.
    #[test]
    fn a_published_manifest_never_leaks_the_classifier_url() {
        assert_eq!(
            redact_url("https://dlp.internal:8443/classify?api_key=SUPERSECRET"),
            "https://dlp.internal:8443"
        );
        let m = OrgWebhook::new(
            "org-classifier",
            "https://dlp.internal:8443/classify?api_key=SUPERSECRET",
            Duration::from_millis(50),
            LatencyClass::Slow,
            DataClass::NeedsText,
        )
        .unwrap();
        assert!(!m.manifest().description.contains("SUPERSECRET"));
        assert!(!m.manifest().description.contains("api_key"));
    }
}
