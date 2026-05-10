//! Hot-tier cache backed by sled.
//!
//! Two trees:
//!
//! - `emem.canonical_index` — `cell\0band\0tslot_be8` → `fact_cid_string_bytes`
//! - `emem.facts`           — `fact_cid_string_bytes` → canonical CBOR of the fact
//!
//! Fact CIDs are derived deterministically: `base32_nopad_lc(blake3(canonical_cbor(fact)))`.
//! Two implementations encoding the same fact converge on the same CID, so cache
//! lookups are content-addressed end to end.

use async_trait::async_trait;
use blake3::Hasher;
use data_encoding::BASE32_NOPAD;

use crate::{Cache, CacheError, CanonicalKey, Tier};
use emem_fact::{Fact, FactCid};

const TREE_INDEX: &str = "emem.canonical_index";
const TREE_FACTS: &str = "emem.facts";

const SEP: u8 = 0u8;

/// Hot tier on top of sled.
pub struct SledHotCache {
    db: sled::Db,
    idx: sled::Tree,
    facts: sled::Tree,
}

impl SledHotCache {
    /// Open or create a sled DB at the given path.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, CacheError> {
        let db = sled::open(path)?;
        let idx = db.open_tree(TREE_INDEX)?;
        let facts = db.open_tree(TREE_FACTS)?;
        Ok(Self { db, idx, facts })
    }

    /// Open an in-memory (temporary) cache. Useful for tests and the
    /// dev server's first-boot bootstrap.
    pub fn open_temporary() -> Result<Self, CacheError> {
        let db = sled::Config::new().temporary(true).open()?;
        let idx = db.open_tree(TREE_INDEX)?;
        let facts = db.open_tree(TREE_FACTS)?;
        Ok(Self { db, idx, facts })
    }

    /// Iterate every (canonical_key, fact_cid) in the index. Used by
    /// primitives like find_similar that need a corpus-wide scan.
    pub fn iter_index(
        &self,
    ) -> impl Iterator<Item = Result<(CanonicalKey, FactCid), CacheError>> + '_ {
        self.idx.iter().map(|kv| {
            let (k, v) = kv?;
            let key = decode_key(&k).map_err(CacheError::Cbor)?;
            let cid_s = std::str::from_utf8(&v)
                .map_err(|e| CacheError::Cbor(e.to_string()))?
                .to_string();
            Ok((key, FactCid::new(cid_s)))
        })
    }

    /// Prefix-scan the index by cell64 (and optional tslot equality filter).
    /// Caps iteration at `EMEM_SCAN_CELL_LIMIT` rows (default 10_000) so a
    /// pathologically dense cell can't tie up a request thread. The cap is
    /// well above any expected legitimate density (a single cell holds one
    /// fact per (band, tslot)); hitting it indicates either an attack or a
    /// schema mistake, both of which we want logged.
    pub fn scan_cell(
        &self,
        cell: &str,
        tslot: Option<u64>,
    ) -> Result<Vec<(CanonicalKey, FactCid)>, CacheError> {
        let limit: usize = std::env::var("EMEM_SCAN_CELL_LIMIT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(10_000);
        let mut prefix = Vec::with_capacity(cell.len() + 1);
        prefix.extend_from_slice(cell.as_bytes());
        prefix.push(SEP);
        let mut out = Vec::new();
        let mut seen = 0usize;
        for kv in self.idx.scan_prefix(&prefix) {
            seen += 1;
            if out.len() >= limit {
                tracing::warn!(
                    target: "emem::storage",
                    scan_cell = %cell,
                    scan_limit = limit,
                    scan_seen = seen,
                    "scan_cell_limit_hit",
                );
                break;
            }
            let (k, v) = kv?;
            let key = decode_key(&k).map_err(CacheError::Cbor)?;
            if let Some(t) = tslot {
                if key.tslot != t {
                    continue;
                }
            }
            let cid_s = std::str::from_utf8(&v)
                .map_err(|e| CacheError::Cbor(e.to_string()))?
                .to_string();
            out.push((key, FactCid::new(cid_s)));
        }
        Ok(out)
    }

    /// Approximate item count across the index tree.
    pub fn len(&self) -> usize {
        self.idx.len()
    }
    /// Whether the index has zero entries.
    pub fn is_empty(&self) -> bool {
        self.idx.is_empty()
    }
    /// Total bytes across both trees on disk (sled estimate).
    /// Borrow the underlying sled DB so callers (e.g., the attester
    /// reputation tracker) can open additional named trees alongside the
    /// canonical index + facts trees without re-opening the file.
    pub fn db(&self) -> &sled::Db {
        &self.db
    }

    pub fn size_on_disk(&self) -> Result<u64, CacheError> {
        Ok(self.db.size_on_disk()?)
    }
}

/// Compute the deterministic FactCid for a fact: base32-nopad-lowercase of
/// blake3(canonical_cbor(fact)). Always 52 chars (256 bits).
pub fn fact_cid_of(fact: &Fact) -> Result<FactCid, CacheError> {
    let cbor = fact_to_cbor(fact)?;
    let mut h = Hasher::new();
    h.update(&cbor);
    let hash = h.finalize();
    let s = BASE32_NOPAD.encode(hash.as_bytes()).to_lowercase();
    Ok(FactCid::new(s))
}

fn fact_to_cbor(fact: &Fact) -> Result<Vec<u8>, CacheError> {
    let mut buf = Vec::new();
    ciborium::ser::into_writer(fact, &mut buf).map_err(|e| CacheError::Cbor(e.to_string()))?;
    Ok(buf)
}

fn cbor_to_fact(bytes: &[u8]) -> Result<Fact, CacheError> {
    ciborium::de::from_reader(bytes).map_err(|e| CacheError::Cbor(e.to_string()))
}

fn encode_key(k: &CanonicalKey) -> Vec<u8> {
    let mut buf = Vec::with_capacity(k.cell.len() + k.band.len() + 10);
    buf.extend_from_slice(k.cell.as_bytes());
    buf.push(SEP);
    buf.extend_from_slice(k.band.as_bytes());
    buf.push(SEP);
    buf.extend_from_slice(&k.tslot.to_be_bytes());
    buf
}

fn decode_key(b: &[u8]) -> Result<CanonicalKey, String> {
    let mut parts = b.splitn(3, |c| *c == SEP);
    let cell = parts.next().ok_or("missing cell")?;
    let band = parts.next().ok_or("missing band")?;
    let rest = parts.next().ok_or("missing tslot")?;
    if rest.len() != 8 {
        return Err(format!("tslot must be 8 BE bytes, got {}", rest.len()));
    }
    let mut t = [0u8; 8];
    t.copy_from_slice(rest);
    Ok(CanonicalKey {
        cell: std::str::from_utf8(cell)
            .map_err(|e| e.to_string())?
            .to_string(),
        band: std::str::from_utf8(band)
            .map_err(|e| e.to_string())?
            .to_string(),
        tslot: u64::from_be_bytes(t),
    })
}

/// The canonical key derived from a fact's storage tuple. Returns None for
/// derivative facts (which are keyed by parent CIDs, not cell/band/tslot).
fn fact_canonical_key(fact: &Fact) -> Option<CanonicalKey> {
    match fact {
        Fact::Primary(p) => Some(CanonicalKey {
            cell: p.cell.clone(),
            band: p.band.clone(),
            tslot: p.tslot,
        }),
        Fact::Absence(n) => Some(CanonicalKey {
            cell: n.cell.clone(),
            band: n.band.clone(),
            tslot: n.tslot,
        }),
        Fact::Derivative(_) => None,
    }
}

#[async_trait]
impl Cache for SledHotCache {
    async fn lookup_many(&self, keys: &[CanonicalKey]) -> Result<Vec<Option<FactCid>>, CacheError> {
        let mut out = Vec::with_capacity(keys.len());
        for k in keys {
            let kb = encode_key(k);
            match self.idx.get(&kb)? {
                Some(v) => {
                    let s = std::str::from_utf8(&v)
                        .map_err(|e| CacheError::Cbor(e.to_string()))?
                        .to_string();
                    out.push(Some(FactCid::new(s)));
                }
                None => out.push(None),
            }
        }
        Ok(out)
    }

    async fn get_many(&self, cids: &[FactCid]) -> Result<Vec<Option<Fact>>, CacheError> {
        let mut out = Vec::with_capacity(cids.len());
        for cid in cids {
            match self.facts.get(cid.as_str().as_bytes())? {
                Some(b) => out.push(Some(cbor_to_fact(&b)?)),
                None => out.push(None),
            }
        }
        Ok(out)
    }

    async fn put_many(&self, facts: &[Fact]) -> Result<Vec<FactCid>, CacheError> {
        let mut out = Vec::with_capacity(facts.len());
        for f in facts {
            let cbor = fact_to_cbor(f)?;
            let mut h = Hasher::new();
            h.update(&cbor);
            let hash = h.finalize();
            let cid_s = BASE32_NOPAD.encode(hash.as_bytes()).to_lowercase();
            let cid = FactCid::new(cid_s);
            self.facts.insert(cid.as_str().as_bytes(), cbor)?;
            if let Some(k) = fact_canonical_key(f) {
                self.idx.insert(encode_key(&k), cid.as_str().as_bytes())?;
            }
            out.push(cid);
        }
        self.idx
            .flush_async()
            .await
            .map_err(|e| CacheError::Cbor(e.to_string()))?;
        self.facts
            .flush_async()
            .await
            .map_err(|e| CacheError::Cbor(e.to_string()))?;
        Ok(out)
    }

    async fn tier_of(&self, cid: &FactCid) -> Result<Option<Tier>, CacheError> {
        Ok(if self.facts.contains_key(cid.as_str().as_bytes())? {
            Some(Tier::Hot)
        } else {
            None
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use emem_core::AttesterKey;
    use emem_fact::{Derivation, PrimaryFact, SchemaCid, Source};

    fn sample_fact(cell: &str, band: &str, tslot: u64) -> Fact {
        Fact::Primary(PrimaryFact {
            cell: cell.into(),
            band: band.into(),
            tslot,
            value: ciborium::Value::Float(0.42),
            unit: None,
            confidence: 1.0,
            uncertainty: None,
            sources: vec![Source {
                scheme: "test".into(),
                id: "t1".into(),
                cid: None,
                hash: None,
                captured_at: None,
                url: None,
            }],
            derivation: Derivation {
                fn_key: "test@1".into(),
                args: None,
            },
            privacy_class: "public".into(),
            schema_cid: SchemaCid::new("test"),
            signer: AttesterKey([0u8; 32]),
            signed_at: "2026-01-01T00:00:00Z".into(),
            served_via: None,
        })
    }

    #[tokio::test]
    async fn put_then_lookup_roundtrips() {
        let c = SledHotCache::open_temporary().unwrap();
        let f = sample_fact("ento.bria.calo.tris", "indices.ndvi", 7);
        let cids = c.put_many(std::slice::from_ref(&f)).await.unwrap();
        assert_eq!(cids.len(), 1);

        let key = CanonicalKey {
            cell: "ento.bria.calo.tris".into(),
            band: "indices.ndvi".into(),
            tslot: 7,
        };
        let hits = c.lookup_many(&[key]).await.unwrap();
        assert_eq!(hits[0], Some(cids[0].clone()));

        let facts = c.get_many(&cids).await.unwrap();
        assert!(facts[0].is_some());
    }

    #[tokio::test]
    async fn scan_cell_filters_by_tslot() {
        let c = SledHotCache::open_temporary().unwrap();
        c.put_many(&[
            sample_fact("ento.bria.calo.tris", "indices.ndvi", 7),
            sample_fact("ento.bria.calo.tris", "indices.evi", 7),
            sample_fact("ento.bria.calo.tris", "indices.ndvi", 8),
        ])
        .await
        .unwrap();

        let only_t7 = c.scan_cell("ento.bria.calo.tris", Some(7)).unwrap();
        assert_eq!(only_t7.len(), 2);
        let all = c.scan_cell("ento.bria.calo.tris", None).unwrap();
        assert_eq!(all.len(), 3);
    }
}
