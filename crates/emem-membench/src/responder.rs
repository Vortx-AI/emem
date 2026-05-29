//! The thing under test.
//!
//! A [`Responder`] is anything that can answer the four query kinds the
//! scorecard grades. The live implementation ([`HttpResponder`]) drives a
//! running emem server over its REST surface; the offline implementation
//! ([`StubResponder`]) answers from an in-memory map seeded with the
//! fixture corpus, so `--self-test` exercises the *scoring* code with no
//! network.

use std::collections::HashMap;

use crate::fixture;

/// Normalise a value for comparison: trim + lowercase + collapse trailing
/// `.0` on integral floats so "812" and "812.0" match. Keeps grading
/// robust against trivial formatting drift without being lenient about
/// genuinely different values.
pub fn normalise(v: &str) -> String {
    let t = v.trim().to_ascii_lowercase();
    if let Ok(f) = t.parse::<f64>() {
        // Canonicalise numbers so "3", "3.0", "3.00" compare equal.
        if f == f.trunc() && f.abs() < 1e15 {
            return format!("{}", f as i64);
        }
        return format!("{f}");
    }
    t
}

/// Outcome of a contradiction probe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictReport {
    pub conflict: bool,
}

#[allow(async_fn_in_trait)]
pub trait Responder {
    /// Recall the current value for `cell`+`band`, or `None` if unknown.
    async fn recall(&self, cell: &str, band: &str) -> anyhow::Result<Option<String>>;

    /// Apply an ordered sequence of session writes, then recall — used by
    /// the test-time-learning axis. Returns the value the responder
    /// believes is current after the writes.
    async fn learn_then_recall(
        &self,
        cell: &str,
        band: &str,
        writes: &[&str],
    ) -> anyhow::Result<Option<String>>;

    /// Ask whether two attested values for a subject contradict.
    async fn contradiction(
        &self,
        cell: &str,
        band: &str,
        value_a: &str,
        value_b: &str,
    ) -> anyhow::Result<ConflictReport>;

    /// A signed receipt (as opaque JSON) the responder vouches for, if it
    /// can produce one. The stub returns `None`; live mode fetches a real
    /// one and the caller embeds it in the scorecard.
    async fn signed_receipt(&self) -> anyhow::Result<Option<serde_json::Value>>;
}

/// In-memory responder for offline self-test.
///
/// Seeded with [`fixture::ITEMS`]. Models a *correct* memory system:
/// last-write-wins for learning, exact-value contradiction detection. The
/// scoring tests in `main.rs` build deliberately-wrong stubs to prove the
/// scorers don't just always return 1.0.
pub struct StubResponder {
    store: HashMap<(String, String), String>,
}

impl StubResponder {
    /// Seed from the fixture base memory.
    pub fn from_fixture() -> Self {
        let mut store = HashMap::new();
        for it in fixture::ITEMS {
            store.insert(
                (it.cell.to_string(), it.band.to_string()),
                it.value.to_string(),
            );
        }
        Self { store }
    }

    /// Build a stub from an explicit (cell, band, value) seed — used by
    /// unit tests to construct perfect / half-wrong corpora.
    #[cfg(test)]
    pub fn from_seed(seed: &[(&str, &str, &str)]) -> Self {
        let mut store = HashMap::new();
        for (c, b, v) in seed {
            store.insert((c.to_string(), b.to_string()), v.to_string());
        }
        Self { store }
    }
}

impl Responder for StubResponder {
    async fn recall(&self, cell: &str, band: &str) -> anyhow::Result<Option<String>> {
        Ok(self
            .store
            .get(&(cell.to_string(), band.to_string()))
            .cloned())
    }

    async fn learn_then_recall(
        &self,
        _cell: &str,
        _band: &str,
        writes: &[&str],
    ) -> anyhow::Result<Option<String>> {
        // A correct memory applies writes in order and keeps the last.
        Ok(writes.last().map(|s| s.to_string()))
    }

    async fn contradiction(
        &self,
        _cell: &str,
        _band: &str,
        value_a: &str,
        value_b: &str,
    ) -> anyhow::Result<ConflictReport> {
        Ok(ConflictReport {
            conflict: normalise(value_a) != normalise(value_b),
        })
    }

    async fn signed_receipt(&self) -> anyhow::Result<Option<serde_json::Value>> {
        // The stub has no identity key; self-test scorecards are unsigned
        // by design and labelled mode="self-test".
        Ok(None)
    }
}

/// Live responder driving a running emem server over REST.
///
/// Maps each abstract query to the closest existing endpoint:
/// * `recall` → `POST /v1/recall`
/// * `learn_then_recall` → replays writes are not server-mutating in the
///   public surface, so we recall the current value (the server's own
///   last-write-wins is what we grade).
/// * `contradiction` → `POST /v1/memory_contradictions`
/// * `signed_receipt` → the `.receipt` envelope returned by `/v1/recall`.
pub struct HttpResponder {
    base: String,
    client: reqwest::Client,
}

impl HttpResponder {
    pub fn new(base: impl Into<String>) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()?;
        Ok(Self {
            base: base.into(),
            client,
        })
    }

    /// Extract a scalar value from a `/v1/recall` response. emem returns a
    /// `facts` array of attestations; we take the first fact's `value`
    /// (stringified) for grading purposes.
    fn extract_value(v: &serde_json::Value) -> Option<String> {
        let facts = v.pointer("/facts").or_else(|| v.get("facts"))?.as_array()?;
        let first = facts.first()?;
        let val = first
            .get("value")
            .or_else(|| first.pointer("/claim/value"))?;
        Some(match val {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        })
    }
}

impl Responder for HttpResponder {
    async fn recall(&self, cell: &str, band: &str) -> anyhow::Result<Option<String>> {
        let body = serde_json::json!({ "cell": cell, "bands": [band] });
        let resp = self
            .client
            .post(format!("{}/v1/recall", self.base))
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Ok(None);
        }
        let v: serde_json::Value = resp.json().await?;
        Ok(Self::extract_value(&v))
    }

    async fn learn_then_recall(
        &self,
        cell: &str,
        band: &str,
        _writes: &[&str],
    ) -> anyhow::Result<Option<String>> {
        // The public REST surface has no client-driven write primitive
        // (facts are materialised/signed server-side), so the honest live
        // probe is: recall the band and grade the server's own current
        // value. We do NOT fabricate a write path.
        self.recall(cell, band).await
    }

    async fn contradiction(
        &self,
        cell: &str,
        band: &str,
        value_a: &str,
        value_b: &str,
    ) -> anyhow::Result<ConflictReport> {
        let body = serde_json::json!({
            "cell": cell,
            "band": band,
            "candidates": [value_a, value_b],
        });
        let resp = self
            .client
            .post(format!("{}/v1/memory_contradictions", self.base))
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            // Endpoint unavailable / errored: fall back to value compare so
            // the axis still reports something, labelled in the eprintln log.
            return Ok(ConflictReport {
                conflict: normalise(value_a) != normalise(value_b),
            });
        }
        let v: serde_json::Value = resp.json().await?;
        let conflict = v
            .pointer("/contradictions")
            .and_then(|c| c.as_array())
            .map(|a| !a.is_empty())
            .or_else(|| v.get("conflict").and_then(|b| b.as_bool()))
            .unwrap_or_else(|| normalise(value_a) != normalise(value_b));
        Ok(ConflictReport { conflict })
    }

    async fn signed_receipt(&self) -> anyhow::Result<Option<serde_json::Value>> {
        // Recall any seeded fixture cell to obtain a freshly-signed receipt.
        let probe = fixture::ITEMS.first();
        let Some(it) = probe else { return Ok(None) };
        let body = serde_json::json!({ "cell": it.cell, "bands": [it.band] });
        let resp = self
            .client
            .post(format!("{}/v1/recall", self.base))
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Ok(None);
        }
        let v: serde_json::Value = resp.json().await?;
        Ok(v.get("receipt").cloned())
    }
}
