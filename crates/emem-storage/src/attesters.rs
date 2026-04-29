//! Per-attester reputation tracker — the contributor-of-intelligence layer.
//!
//! Every successful `put_attestation` increments a counter row for each of
//! the attestation's signers (the attester pubkey + per-fact signers). Every
//! `get_facts_many` increments a citation counter per fact's signer. The
//! totals are sled-persisted under a dedicated tree so the data survives
//! restart.
//!
//! This does not require a cryptocurrency or any external token. It is
//! purely additive: high-quality contributors accumulate verifiable
//! citation count which other tools (rate-limit relax, leaderboards,
//! external incentive layers) can read.
//!
//! Schema (CBOR map at `attesters/<pubkey_b32>`):
//!   {
//!     "pubkey_b32":          str
//!     "attestations":        u64    # total accepted batches signed by this key
//!     "facts":               u64    # total individual facts signed by this key
//!     "citations":           u64    # times any of their facts were retrieved
//!     "unique_cells":        u64    # count of distinct cell64s contributed
//!     "first_seen_unix_s":   u64
//!     "last_seen_unix_s":    u64
//!     "last_cited_unix_s":   u64
//!   }

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use ciborium::Value as CborValue;
use sled::Tree;

use emem_fact::Fact;

/// Per-attester reputation snapshot.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct AttesterStats {
    pub pubkey_b32: String,
    #[serde(default)]
    pub attestations: u64,
    #[serde(default)]
    pub facts: u64,
    #[serde(default)]
    pub citations: u64,
    #[serde(default)]
    pub unique_cells: u64,
    #[serde(default)]
    pub first_seen_unix_s: u64,
    #[serde(default)]
    pub last_seen_unix_s: u64,
    #[serde(default)]
    pub last_cited_unix_s: u64,
}

impl AttesterStats {
    /// Composite reputation score: citations dominate (they reflect
    /// downstream usefulness), facts and attestations contribute logarithmic
    /// floor weight so a brand-new contributor still ranks.
    pub fn score(&self) -> f64 {
        let cited = self.citations as f64;
        let facts = self.facts as f64;
        let atts = self.attestations as f64;
        cited * 1.0 + (1.0 + facts).ln() * 8.0 + (1.0 + atts).ln() * 4.0
    }
}

/// Sled-backed reputation tracker. Cheap to construct, safe to share via Arc.
#[derive(Clone)]
pub struct AttesterRegistry {
    tree: Arc<Tree>,
}

impl AttesterRegistry {
    /// Open the `attesters` tree on the given sled DB. Creates if missing.
    pub fn open(db: &sled::Db) -> sled::Result<Self> {
        let tree = db.open_tree("emem.attesters")?;
        Ok(Self {
            tree: Arc::new(tree),
        })
    }

    /// Update stats after a successful attestation. Counts:
    ///  • the attester pubkey once,
    ///  • each fact's per-signer pubkey once per fact.
    pub fn record_attestation(
        &self,
        attester_pubkey: &[u8; 32],
        facts: &[Fact],
    ) -> sled::Result<()> {
        let now = unix_secs();
        let attester_b32 = b32(attester_pubkey);

        // Count distinct cells contributed in this batch — useful for
        // unique_cells.
        let mut cells_for_attester: std::collections::BTreeSet<&str> =
            std::collections::BTreeSet::new();

        // Per-signer rollup; the attester key may differ from per-fact
        // signer (e.g. when an aggregator submits on behalf of others).
        let mut per_signer: std::collections::BTreeMap<
            String,
            (u64, std::collections::BTreeSet<String>),
        > = std::collections::BTreeMap::new();

        for f in facts {
            let (signer, cell) = match f {
                Fact::Primary(p) => (&p.signer.0, p.cell.clone()),
                Fact::Derivative(d) => (&d.signer.0, d.cell.clone()),
                Fact::Absence(a) => (&a.signer.0, a.cell.clone()),
            };
            let s_b32 = b32(signer);
            let entry = per_signer
                .entry(s_b32)
                .or_insert((0, std::collections::BTreeSet::new()));
            entry.0 += 1;
            entry.1.insert(cell.clone());
            if signer == attester_pubkey {
                let leaked: &str = Box::leak(cell.into_boxed_str());
                cells_for_attester.insert(leaked);
            }
        }

        // Attester batch increment (one attestation per call).
        self.update(&attester_b32, |s| {
            if s.first_seen_unix_s == 0 {
                s.first_seen_unix_s = now;
            }
            s.last_seen_unix_s = now;
            s.attestations += 1;
        })?;

        // Per-signer fact increments + unique cell tracking.
        for (signer_b32, (n_facts, cells)) in per_signer {
            self.update(&signer_b32, |s| {
                if s.first_seen_unix_s == 0 {
                    s.first_seen_unix_s = now;
                }
                s.last_seen_unix_s = now;
                s.facts += n_facts;
                // unique_cells is monotonic over the lifetime of the row
                // — we treat this as a lower bound rather than an exact
                // distinct-count to avoid storing the full cell set.
                s.unique_cells = s.unique_cells.saturating_add(cells.len() as u64);
            })?;
        }
        self.tree.flush()?;
        Ok(())
    }

    /// Increment the citation counter for each of the given fact signers.
    /// Called from read paths after a fact is served to a client.
    pub fn record_citations(&self, facts: &[Fact]) -> sled::Result<()> {
        let now = unix_secs();
        let mut per_signer: std::collections::BTreeMap<String, u64> =
            std::collections::BTreeMap::new();
        for f in facts {
            let signer = match f {
                Fact::Primary(p) => &p.signer.0,
                Fact::Derivative(d) => &d.signer.0,
                Fact::Absence(a) => &a.signer.0,
            };
            *per_signer.entry(b32(signer)).or_insert(0) += 1;
        }
        for (signer_b32, n) in per_signer {
            self.update(&signer_b32, |s| {
                s.citations += n;
                s.last_cited_unix_s = now;
            })?;
        }
        Ok(())
    }

    /// Look up one attester's stats. Returns `None` if unknown.
    pub fn get(&self, pubkey_b32: &str) -> sled::Result<Option<AttesterStats>> {
        Ok(self.tree.get(pubkey_b32)?.and_then(|ivec| decode(&ivec)))
    }

    /// Top-N attesters by composite score. `limit` ≤ 1000 enforced.
    pub fn top(&self, limit: usize) -> sled::Result<Vec<AttesterStats>> {
        let limit = limit.min(1000);
        let mut all: Vec<AttesterStats> = Vec::new();
        for kv in self.tree.iter() {
            let (_, ivec) = kv?;
            if let Some(s) = decode(&ivec) {
                all.push(s);
            }
        }
        all.sort_by(|a, b| {
            b.score()
                .partial_cmp(&a.score())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        all.truncate(limit);
        Ok(all)
    }

    /// Total number of unique attesters tracked.
    pub fn count(&self) -> sled::Result<u64> {
        Ok(self.tree.len() as u64)
    }

    fn update<F: FnMut(&mut AttesterStats)>(&self, pubkey_b32: &str, mut f: F) -> sled::Result<()> {
        loop {
            let cur = self.tree.get(pubkey_b32)?;
            let mut stats =
                cur.as_ref()
                    .and_then(|iv| decode(iv))
                    .unwrap_or_else(|| AttesterStats {
                        pubkey_b32: pubkey_b32.to_string(),
                        ..Default::default()
                    });
            f(&mut stats);
            let new = encode(&stats);
            // Atomic compare-and-set so concurrent updates don't lose
            // counter increments.
            match self
                .tree
                .compare_and_swap(pubkey_b32, cur.as_deref(), Some(new))?
            {
                Ok(_) => return Ok(()),
                Err(_) => continue, // retry on contention
            }
        }
    }
}

fn b32(b: &[u8; 32]) -> String {
    data_encoding::BASE32_NOPAD.encode(b).to_lowercase()
}

fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn encode(s: &AttesterStats) -> Vec<u8> {
    let cbor: CborValue = serde_cbor_value(s);
    let mut out = Vec::with_capacity(160);
    let _ = ciborium::ser::into_writer(&cbor, &mut out);
    out
}

fn decode(b: &[u8]) -> Option<AttesterStats> {
    ciborium::de::from_reader::<CborValue, _>(b)
        .ok()
        .and_then(|v| from_cbor_value(&v))
}

fn serde_cbor_value(s: &AttesterStats) -> CborValue {
    CborValue::Map(vec![
        (
            CborValue::Text("pubkey_b32".into()),
            CborValue::Text(s.pubkey_b32.clone()),
        ),
        (
            CborValue::Text("attestations".into()),
            CborValue::Integer(s.attestations.into()),
        ),
        (
            CborValue::Text("facts".into()),
            CborValue::Integer(s.facts.into()),
        ),
        (
            CborValue::Text("citations".into()),
            CborValue::Integer(s.citations.into()),
        ),
        (
            CborValue::Text("unique_cells".into()),
            CborValue::Integer(s.unique_cells.into()),
        ),
        (
            CborValue::Text("first_seen_unix_s".into()),
            CborValue::Integer(s.first_seen_unix_s.into()),
        ),
        (
            CborValue::Text("last_seen_unix_s".into()),
            CborValue::Integer(s.last_seen_unix_s.into()),
        ),
        (
            CborValue::Text("last_cited_unix_s".into()),
            CborValue::Integer(s.last_cited_unix_s.into()),
        ),
    ])
}

fn from_cbor_value(v: &CborValue) -> Option<AttesterStats> {
    let m = match v {
        CborValue::Map(m) => m,
        _ => return None,
    };
    let mut s = AttesterStats::default();
    for (k, v) in m {
        let key = match k {
            CborValue::Text(t) => t.as_str(),
            _ => continue,
        };
        match (key, v) {
            ("pubkey_b32", CborValue::Text(t)) => s.pubkey_b32 = t.clone(),
            ("attestations", CborValue::Integer(i)) => {
                s.attestations = (<i128>::from(*i)).max(0) as u64
            }
            ("facts", CborValue::Integer(i)) => s.facts = (<i128>::from(*i)).max(0) as u64,
            ("citations", CborValue::Integer(i)) => s.citations = (<i128>::from(*i)).max(0) as u64,
            ("unique_cells", CborValue::Integer(i)) => {
                s.unique_cells = (<i128>::from(*i)).max(0) as u64
            }
            ("first_seen_unix_s", CborValue::Integer(i)) => {
                s.first_seen_unix_s = (<i128>::from(*i)).max(0) as u64
            }
            ("last_seen_unix_s", CborValue::Integer(i)) => {
                s.last_seen_unix_s = (<i128>::from(*i)).max(0) as u64
            }
            ("last_cited_unix_s", CborValue::Integer(i)) => {
                s.last_cited_unix_s = (<i128>::from(*i)).max(0) as u64
            }
            _ => {}
        }
    }
    Some(s)
}
