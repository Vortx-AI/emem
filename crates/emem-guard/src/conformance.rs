//! The conformance suite: does a DEPLOYMENT behave, not does the code.
//!
//! Unit tests prove the handlers are right. They prove nothing about the
//! server an operator actually stood up, and every failure this suite looks
//! for lives between the two: a proxy that rejects a 9 MB body, a TLS
//! terminator that adds 400 ms, a reverse proxy that title-cases headers, a
//! deployment that answers 502 while its own tests are green.
//!
//! Every one of those produces the same outcome, which is the only outcome
//! that matters here: **a server that appears to enforce and does not.** A
//! transport failure is not a deny. It hands the decision to the caller's
//! failure policy and, under fail-open, lets the request through uninspected;
//! sustained failures trip a breaker that stops enforcement for everything.
//!
//! So this runs against a URL over the wire, exactly as a checkpoint would.
//! `emem-guard --conformance <url>` and a non-zero exit if any check fails.
//!
//! # What it does not do
//!
//! It does not test detection quality, and it never will. Whether the policy
//! engine reaches the right verdict is what the unit tests are for. This asks
//! one question per check: would a real checkpoint be able to rely on this
//! deployment.

use std::time::{Duration, Instant};

/// One thing a deployment must do.
pub struct Check {
    pub id: &'static str,
    /// What breaks in production if this fails. Not a restatement of the
    /// check: the consequence, so an operator reading a red line knows
    /// whether to page someone.
    pub matters: &'static str,
}

/// The result of running one check.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub id: &'static str,
    pub passed: bool,
    pub detail: String,
    pub took_ms: u128,
}

/// Everything the suite asserts, in the order it runs.
pub const CHECKS: &[Check] = &[
    Check {
        id: "reachable",
        matters: "nothing else can be true if the node does not answer",
    },
    Check {
        id: "health_is_honest_about_verification",
        matters: "an operator who thinks citations are being checked, and they are not, has a guard that logs and nothing more",
    },
    Check {
        id: "unknown_event_still_gets_a_verdict",
        matters: "a new event type from an upgraded checkpoint would otherwise become a transport failure on every request",
    },
    Check {
        id: "malformed_body_still_gets_a_verdict",
        matters: "a truncated or re-encoded body would otherwise trip the breaker instead of being ignored",
    },
    Check {
        id: "empty_body_still_gets_a_verdict",
        matters: "health checkers and probes send these, and each one would count against the breaker",
    },
    Check {
        id: "large_body_is_accepted",
        matters: "transcripts arrive untruncated up to 10 MB; nginx defaults to 1 MB and Express to 100 kB, so this fails in front of the server far more often than in it",
    },
    Check {
        id: "every_open_route_answers",
        matters: "a route advertised in the descriptor that does not answer is a documented lie",
    },
    Check {
        id: "descriptor_is_complete",
        matters: "a cold agent integrates from the descriptor alone; a missing field is an integration that cannot be written",
    },
    Check {
        id: "replay_is_idempotent",
        matters: "a retry after a connection failure reuses the request id, and a second evaluation could contradict the answer already acted on",
    },
    Check {
        id: "leaf_is_fetchable_and_verifies",
        matters: "every denial names a leaf; if it cannot be fetched and checked, the audit trail is an assertion",
    },
    Check {
        id: "answers_inside_the_floor",
        matters: "the tightest configurable deadline we have seen is 1000 ms for the whole exchange, and a late verdict is an unreachable server",
    },
    Check {
        id: "holds_up_under_concurrency",
        matters: "a deployment that meets the deadline once and misses it under load trips the breaker in production and never in staging",
    },
];

/// How many concurrent requests the load check drives.
///
/// Not a benchmark. The question is whether the deadline still holds when the
/// node is not idle, because a deployment measured only at rest is measured in
/// the one condition production never has.
pub const LOAD_CONCURRENCY: usize = 16;
pub const LOAD_REQUESTS: usize = 100;

/// The floor a verdict has to answer inside, end to end.
pub const DEADLINE: Duration = Duration::from_millis(1000);

/// Run every check against `base`.
pub fn run(base: &str) -> Vec<Outcome> {
    let base = base.trim_end_matches('/').to_string();
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return vec![Outcome {
                id: "reachable",
                passed: false,
                detail: format!("could not build an http client: {e}"),
                took_ms: 0,
            }]
        }
    };
    let mut out = Vec::new();
    let mut one = |id: &'static str, f: &mut dyn FnMut() -> (bool, String)| {
        let t = Instant::now();
        let (passed, detail) = f();
        out.push(Outcome {
            id,
            passed,
            detail,
            took_ms: t.elapsed().as_millis(),
        });
    };

    // reachable
    let health = client.get(format!("{base}/health")).send();
    let health_json: Option<serde_json::Value> = match health {
        Ok(r) if r.status().is_success() => r.json().ok(),
        _ => None,
    };
    one("reachable", &mut || match &health_json {
        Some(_) => (true, "GET /health answered".into()),
        None => (
            false,
            format!("GET {base}/health did not answer 200 with json"),
        ),
    });
    if health_json.is_none() {
        // Everything downstream would report a second symptom of one cause.
        return out;
    }
    let h = health_json.unwrap();

    one("health_is_honest_about_verification", &mut || match h
        .get("verifies_citations")
        .and_then(|v| v.as_bool())
    {
        Some(true) => (true, "resolver attached; citations are verified".into()),
        Some(false) => (
            true,
            "no resolver: this node signs and logs but verifies no citation, and says so".into(),
        ),
        None => (
            false,
            "/health does not say whether citations are verified".into(),
        ),
    });

    let post = |path: &str, body: String| {
        client
            .post(format!("{base}{path}"))
            .header("content-type", "application/json")
            .body(body)
            .send()
    };

    let verdict_case = |path: &str, body: String, what: &str| -> (bool, String) {
        match post(path, body) {
            Ok(r) => {
                let status = r.status();
                let parsed = r.json::<serde_json::Value>().ok();
                match (status.is_success(), parsed) {
                    (true, Some(v)) if !v.is_null() => {
                        (true, format!("{what}: {status} with a verdict"))
                    }
                    (true, _) => (false, format!("{what}: {status} but no parseable verdict")),
                    (false, _) => (
                        false,
                        format!("{what}: {status}, which is a transport failure, not a deny"),
                    ),
                }
            }
            Err(e) => (false, format!("{what}: request failed: {e}")),
        }
    };

    one("unknown_event_still_gets_a_verdict", &mut || {
        verdict_case(
            "/verdict/anthropic-hook",
            r#"{"type":"an_event_type_from_a_future_release","request_id":"conf-unknown"}"#
                .to_string(),
            "unknown event type",
        )
    });
    one("malformed_body_still_gets_a_verdict", &mut || {
        verdict_case(
            "/verdict",
            "}{ not json at all".to_string(),
            "malformed body",
        )
    });
    one("empty_body_still_gets_a_verdict", &mut || {
        verdict_case("/verdict", String::new(), "empty body")
    });
    one("large_body_is_accepted", &mut || {
        // 9 MB, under the 10 MB ceiling and over every common proxy default.
        let filler = "x".repeat(9 * 1024 * 1024);
        verdict_case(
            "/verdict",
            format!(r#"{{"texts":["{filler}"]}}"#),
            "9 MB body",
        )
    });

    // Every route the node itself advertises.
    let descriptor: Option<serde_json::Value> = client
        .get(format!("{base}/.well-known/emem-guard.json"))
        .send()
        .ok()
        .and_then(|r| r.json().ok());

    one("descriptor_is_complete", &mut || {
        let Some(d) = &descriptor else {
            return (
                false,
                "GET /.well-known/emem-guard.json did not answer".into(),
            );
        };
        let mut missing: Vec<&str> = Vec::new();
        for k in [
            "checkpoints",
            "deny_codes",
            "fixes",
            "reason_grammar",
            "evidence",
            "signer_b32",
        ] {
            if d.get(k).is_none() {
                missing.push(k);
            }
        }
        if missing.is_empty() {
            let n = d["checkpoints"].as_array().map(|a| a.len()).unwrap_or(0);
            (
                true,
                format!("{n} checkpoints, codes and grammar published"),
            )
        } else {
            (
                false,
                format!("descriptor is missing: {}", missing.join(", ")),
            )
        }
    });

    one("every_open_route_answers", &mut || {
        let Some(d) = &descriptor else {
            return (false, "no descriptor to read routes from".into());
        };
        let Some(cps) = d["checkpoints"].as_array() else {
            return (false, "descriptor has no checkpoints array".into());
        };
        let mut bad = Vec::new();
        for cp in cps {
            let Some(route) = cp["route"].as_str() else {
                continue;
            };
            if !route.starts_with("/verdict") {
                continue;
            }
            if let Ok(r) = post(route, "{}".to_string()) {
                if !r.status().is_success() {
                    bad.push(format!("{route} -> {}", r.status()));
                }
            } else {
                bad.push(format!("{route} -> no response"));
            }
        }
        if bad.is_empty() {
            (true, format!("{} advertised routes all answer", cps.len()))
        } else {
            (false, format!("advertised but broken: {}", bad.join(", ")))
        }
    });

    one("replay_is_idempotent", &mut || {
        let body = r#"{"texts":["conformance replay probe"],"request_id":"conf-replay-1"}"#;
        let first = post("/verdict", body.to_string())
            .ok()
            .and_then(|r| r.json::<serde_json::Value>().ok());
        let second = post("/verdict", body.to_string())
            .ok()
            .and_then(|r| r.json::<serde_json::Value>().ok());
        match (first, second) {
            (Some(a), Some(b)) => {
                let (la, lb) = (a["leaf"].as_str(), b["leaf"].as_str());
                if la.is_some() && la == lb {
                    (true, format!("both answered {}", la.unwrap_or("-")))
                } else if la.is_none() {
                    (
                        false,
                        "no leaf on either answer, so a replay cannot be recognised".into(),
                    )
                } else {
                    (
                        false,
                        format!("re-evaluated: {la:?} then {lb:?}; a retry can contradict the answer already acted on"),
                    )
                }
            }
            _ => (false, "one of the two requests did not answer".into()),
        }
    });

    one("leaf_is_fetchable_and_verifies", &mut || {
        let v = match post(
            "/verdict",
            r#"{"texts":["conformance leaf probe"],"request_id":"conf-leaf-1"}"#.to_string(),
        )
        .ok()
        .and_then(|r| r.json::<serde_json::Value>().ok())
        {
            Some(v) => v,
            None => return (false, "no verdict to take a leaf from".into()),
        };
        let Some(leaf) = v["leaf"].as_str() else {
            return (
                false,
                "the verdict carries no leaf, so nothing about it can be proven later".into(),
            );
        };
        let entry: Option<serde_json::Value> = client
            .get(format!("{base}/log/entry/{leaf}"))
            .send()
            .ok()
            .and_then(|r| r.json().ok());
        let Some(e) = entry else {
            return (false, format!("GET /log/entry/{leaf} did not answer"));
        };
        let (Some(sig), Some(key)) = (e["signature_b32"].as_str(), e["signer_b32"].as_str()) else {
            return (false, "the entry carries no signature or no signer".into());
        };
        // Verified here, from the entry alone, with nothing taken from the
        // node that served it. That is the whole claim.
        let ok = (|| {
            let pk: [u8; 32] = data_encoding::BASE32_NOPAD
                .decode(key.to_uppercase().as_bytes())
                .ok()?
                .try_into()
                .ok()?;
            let s: [u8; 64] = data_encoding::BASE32_NOPAD
                .decode(sig.to_uppercase().as_bytes())
                .ok()?
                .try_into()
                .ok()?;
            let rec: crate::log::VerdictRecord =
                serde_json::from_value(e["record"].clone()).ok()?;
            use ed25519_dalek::Verifier as _;
            ed25519_dalek::VerifyingKey::from_bytes(&pk)
                .ok()?
                .verify(&rec.preimage(), &ed25519_dalek::Signature::from_bytes(&s))
                .ok()
        })()
        .is_some();
        if ok {
            (
                true,
                format!("{leaf} fetched and its signature verifies offline"),
            )
        } else {
            (
                false,
                format!("{leaf} fetched but its signature does not verify"),
            )
        }
    });

    one("answers_inside_the_floor", &mut || {
        let body = r#"{"texts":["conformance latency probe with emem:fact:a:b in it"]}"#;
        let mut worst = Duration::ZERO;
        for _ in 0..10 {
            let t = Instant::now();
            let _ = post("/verdict", body.to_string());
            worst = worst.max(t.elapsed());
        }
        if worst < DEADLINE {
            (
                true,
                format!("worst of 10 idle calls: {} ms", worst.as_millis()),
            )
        } else {
            (
                false,
                format!(
                    "worst of 10 idle calls was {} ms, past the {} ms floor",
                    worst.as_millis(),
                    DEADLINE.as_millis()
                ),
            )
        }
    });

    one("holds_up_under_concurrency", &mut || {
        let body = r#"{"texts":["conformance load probe"]}"#.to_string();
        let base = base.clone();
        let per = LOAD_REQUESTS / LOAD_CONCURRENCY;
        let mut handles = Vec::new();
        for _ in 0..LOAD_CONCURRENCY {
            let base = base.clone();
            let body = body.clone();
            handles.push(std::thread::spawn(move || {
                let c = reqwest::blocking::Client::builder()
                    .timeout(Duration::from_secs(30))
                    .build()
                    .ok()?;
                let mut worst = Duration::ZERO;
                let mut failures = 0usize;
                for _ in 0..per {
                    let t = Instant::now();
                    match c
                        .post(format!("{base}/verdict"))
                        .header("content-type", "application/json")
                        .body(body.clone())
                        .send()
                    {
                        Ok(r) if r.status().is_success() => {}
                        _ => failures += 1,
                    }
                    worst = worst.max(t.elapsed());
                }
                Some((worst, failures))
            }));
        }
        let mut worst = Duration::ZERO;
        let mut failures = 0usize;
        for h in handles {
            match h.join() {
                Ok(Some((w, f))) => {
                    worst = worst.max(w);
                    failures += f;
                }
                _ => failures += per,
            }
        }
        let n = per * LOAD_CONCURRENCY;
        if failures == 0 && worst < DEADLINE {
            (
                true,
                format!(
                    "{n} requests at {LOAD_CONCURRENCY} concurrent: worst {} ms, no failures",
                    worst.as_millis()
                ),
            )
        } else {
            (
                false,
                format!(
                    "{n} requests at {LOAD_CONCURRENCY} concurrent: worst {} ms, {failures} failed",
                    worst.as_millis()
                ),
            )
        }
    });

    out
}

/// Print a report and return the exit code.
pub fn report(outcomes: &[Outcome]) -> i32 {
    let by_id = |id: &str| CHECKS.iter().find(|c| c.id == id);
    let mut failed = 0;
    println!("emem-guard conformance\n");
    for o in outcomes {
        let mark = if o.passed { "pass" } else { "FAIL" };
        println!("  {mark}  {:<38} {:>6} ms  {}", o.id, o.took_ms, o.detail);
        if !o.passed {
            failed += 1;
            if let Some(c) = by_id(o.id) {
                // The consequence, not a restatement. An operator reading a
                // red line needs to know whether to page someone.
                println!("        why it matters: {}", c.matters);
            }
        }
    }
    let ran = outcomes.len();
    println!(
        "\n{} of {ran} checks passed{}",
        ran - failed,
        if failed == 0 {
            ". This deployment can be relied on by a checkpoint."
        } else {
            ". Do NOT point a checkpoint at this deployment yet."
        }
    );
    if ran < CHECKS.len() {
        println!(
            "{} checks did not run, because an earlier failure made them meaningless.",
            CHECKS.len() - ran
        );
    }
    i32::from(failed > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every check has to say what breaks, not what it does. A suite whose
    /// failures do not tell an operator the consequence gets ignored.
    #[test]
    fn every_check_explains_the_consequence_of_failing() {
        for c in CHECKS {
            assert!(!c.id.is_empty());
            assert!(
                c.matters.len() > 40,
                "{} does not explain what breaks: {:?}",
                c.id,
                c.matters
            );
        }
        let mut ids: Vec<_> = CHECKS.iter().map(|c| c.id).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "duplicate check id");
    }

    /// The suite runs against a URL, so an unreachable one has to fail fast
    /// and stop rather than report eleven symptoms of one cause.
    #[test]
    fn an_unreachable_node_fails_the_first_check_and_stops() {
        // Port 1 is reserved and refuses immediately.
        let out = run("http://127.0.0.1:1");
        assert_eq!(out.len(), 1, "should not have continued: {out:?}");
        assert_eq!(out[0].id, "reachable");
        assert!(!out[0].passed);
        assert_eq!(report(&out), 1, "a failing suite must exit non-zero");
    }

    /// A green suite exits zero, or it cannot gate anything.
    #[test]
    fn a_clean_report_exits_zero() {
        let clean = vec![Outcome {
            id: "reachable",
            passed: true,
            detail: "ok".into(),
            took_ms: 1,
        }];
        assert_eq!(report(&clean), 0);
    }
}
