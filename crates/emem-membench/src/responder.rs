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
}
