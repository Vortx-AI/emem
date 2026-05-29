//! `edges_recall(subj, pred?, as_of_tslot?, limit?)` — read temporal
//! knowledge-graph edges originating at a fact, bi-temporally filtered.
//!
//! Edges are signed, content-addressed relations `subj --pred--> obj`
//! valid over `[valid_from, valid_to)` (Zep / Graphiti edge model). They
//! are written additively inside an [`emem_fact::Attestation`] (the edge
//! leaves are folded into the merkle root so the signature commits to
//! them) and indexed in the `emem.edge_spo` sled tree for ascending
//! valid-time range scans.
//!
//! This primitive scans that index for a subject (optionally narrowed to
//! one predicate, or `""` for all predicates), applies the supersession +
//! `as_of` rules in [`emem_storage::Storage::recall_edges`], and signs the
//! result. The receipt cites the subject + every object fact CID, and the
//! new edge segment of the preimage commits to the returned edge CIDs so
//! the response rebinds offline against the responder pubkey.

use std::time::Instant;

use serde::{Deserialize, Serialize};

use emem_fact::{EdgeFact, FactCid, Receipt};
use emem_storage::{AsOfBound, Server, StorageError};

/// Default cap on returned edges. Override per-call with `limit`.
const DEFAULT_LIMIT: usize = 100;
/// Hard ceiling on `limit` regardless of caller value.
const MAX_LIMIT: usize = 1_000;

/// Request: the subject fact, optional predicate + valid-time bound.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EdgesRecallReq {
    /// Subject fact CID. Edges originating here are returned.
    pub subj: String,
    /// Predicate filter. Empty string `""` (the default) scans every
    /// predicate for the subject.
    #[serde(default)]
    pub pred: String,
    /// Bi-temporal valid-time bound. When set, only edges with
    /// `valid_from <= as_of_tslot` and `valid_to` either `None` or
    /// `>= as_of_tslot` are returned (closed intervals); supersession
    /// keeps the newest edge per object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub as_of_tslot: Option<u64>,
    /// Maximum edges to return. Defaults to 100, capped at 1000.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

/// Response: matching edges, the distinct object CIDs, an agent hint, and
/// the signed receipt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgesRecallResp {
    /// Edges in ascending `valid_from` order (ties broken by object CID).
    pub edges: Vec<EdgeFact>,
    /// Distinct object fact CIDs cited, in first-seen order.
    pub objs: Vec<String>,
    /// One-paragraph agent-facing summary of what was scanned.
    pub agent_hint: String,
    /// Signed receipt — `fact_cids` = subject + objects; `edge_cids`
    /// commit the returned edges into the signature preimage.
    pub receipt: Receipt,
}

/// Recall edges for a subject. See module docs for semantics.
pub async fn edges_recall(
    req: &EdgesRecallReq,
    srv: &Server,
) -> Result<EdgesRecallResp, StorageError> {
    let started = Instant::now();
    let limit = req.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let subj = FactCid::new(&req.subj);

    let edges = srv
        .storage
        .recall_edges(&subj, &req.pred, req.as_of_tslot, limit)
        .await?;

    // Distinct objects, first-seen order.
    let mut objs: Vec<String> = Vec::new();
    for e in &edges {
        let o = e.obj.as_str().to_string();
        if !objs.iter().any(|x| x == &o) {
            objs.push(o);
        }
    }

    // Receipt citations: the subject fact + every object fact. Cells are
    // not derivable from a fact CID alone (the CID is a content address,
    // not a cell64), so `cells` is empty — the load-bearing binding is
    // `fact_cids` + `edge_cids`.
    let mut fact_cids: Vec<FactCid> = Vec::with_capacity(objs.len() + 1);
    fact_cids.push(subj.clone());
    for o in &objs {
        fact_cids.push(FactCid::new(o));
    }
    let edge_cids: Vec<emem_fact::EdgeCid> = edges.iter().map(|e| e.cid()).collect();

    let agent_hint = build_agent_hint(&edges, &req.subj, &req.pred, req.as_of_tslot);

    let bound = AsOfBound::default();
    let receipt = srv.sign_receipt_with_edges(
        "emem.edges_recall",
        Vec::new(),
        fact_cids,
        true,
        started,
        None,
        None,
        &bound,
        &edge_cids,
    );

    Ok(EdgesRecallResp {
        edges,
        objs,
        agent_hint,
        receipt,
    })
}

fn build_agent_hint(edges: &[EdgeFact], subj: &str, pred: &str, as_of: Option<u64>) -> String {
    let pred_label = if pred.is_empty() {
        "any predicate".to_string()
    } else {
        format!("predicate '{pred}'")
    };
    let asof_label = match as_of {
        Some(t) => format!(" as of valid-time {t}"),
        None => String::new(),
    };
    if edges.is_empty() {
        return format!(
            "No edges originate at subject '{subj}' under {pred_label}{asof_label}. This is the honest 'no relation known', not a lookup failure — write edges by POSTing a signed Attestation with an `edges` array to /v1/edges."
        );
    }
    format!(
        "Found {n} edge(s) from subject '{subj}' under {pred_label}{asof_label}. Each edge is content-addressed and the receipt's edge_cids commit them into the signature; verify offline via /v1/verify_receipt, and follow `obj` to recall the related fact.",
        n = edges.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    use async_trait::async_trait;

    use emem_cache::CanonicalKey;
    use emem_core::AttesterKey;
    use emem_fact::{Attestation, EdgeCid, Fact, RegistryCid, SchemaCid};
    use emem_storage::server::{ManifestCids, ResponderIdentity};
    use emem_storage::{Server, Storage, StorageError};

    /// Minimal in-memory `Storage` carrying just an edge map so the
    /// primitive can be exercised without `MaterializingStorage`.
    struct MockEdgeStorage {
        edges: std::sync::Mutex<Vec<EdgeFact>>,
    }

    impl MockEdgeStorage {
        fn new(edges: Vec<EdgeFact>) -> Self {
            Self {
                edges: std::sync::Mutex::new(edges),
            }
        }
    }

    #[async_trait]
    impl Storage for MockEdgeStorage {
        async fn lookup_canonical_many(
            &self,
            _keys: &[CanonicalKey],
        ) -> Result<Vec<Option<FactCid>>, StorageError> {
            Ok(Vec::new())
        }
        async fn get_facts_many(
            &self,
            _cids: &[FactCid],
        ) -> Result<Vec<Option<Fact>>, StorageError> {
            Ok(Vec::new())
        }
        async fn put_attestation(&self, _att: &Attestation) -> Result<Vec<FactCid>, StorageError> {
            unimplemented!("not used")
        }
        async fn materialize_many(
            &self,
            _keys: &[CanonicalKey],
        ) -> Result<Vec<FactCid>, StorageError> {
            unimplemented!("not used")
        }
        async fn scan_cell(
            &self,
            _cell: &str,
            _tslot: Option<u64>,
        ) -> Result<Vec<(CanonicalKey, FactCid)>, StorageError> {
            Ok(Vec::new())
        }
        async fn iter_index(
            &self,
            _limit: Option<usize>,
        ) -> Result<Vec<(CanonicalKey, FactCid)>, StorageError> {
            Ok(Vec::new())
        }
        async fn recall_edges(
            &self,
            subj: &FactCid,
            pred: &str,
            as_of: Option<u64>,
            limit: usize,
        ) -> Result<Vec<EdgeFact>, StorageError> {
            let mut out: Vec<EdgeFact> = self
                .edges
                .lock()
                .unwrap()
                .iter()
                .filter(|e| e.subj.as_str() == subj.as_str())
                .filter(|e| pred.is_empty() || e.pred == pred)
                .filter(|e| match as_of {
                    Some(t) => e.valid_from <= t && e.valid_to.map(|vt| vt >= t).unwrap_or(true),
                    None => true,
                })
                .cloned()
                .collect();
            out.sort_by_key(|a| a.valid_from);
            out.truncate(limit);
            Ok(out)
        }
        async fn has_edge(&self, cid: &EdgeCid) -> Result<bool, StorageError> {
            Ok(self
                .edges
                .lock()
                .unwrap()
                .iter()
                .any(|e| e.cid().as_str() == cid.as_str()))
        }
    }

    fn test_server(storage: Arc<MockEdgeStorage>) -> Server {
        Server {
            storage,
            identity: ResponderIdentity::fresh(),
            manifests: ManifestCids {
                registry_cid: RegistryCid::new("test-registry"),
                schema_cid: SchemaCid::new("test-schema"),
                bands_cid: "test-bands".into(),
                sources_cid: "test-sources".into(),
            },
            started_at_unix_s: 0,
        }
    }

    fn mk_edge(subj: &str, pred: &str, obj: &str, vf: u64, vt: Option<u64>) -> EdgeFact {
        EdgeFact {
            subj: FactCid::new(subj),
            pred: pred.into(),
            obj: FactCid::new(obj),
            valid_from: vf,
            valid_to: vt,
            confidence: 1.0,
            signer: AttesterKey([1u8; 32]),
            signed_at: "2026-05-29T00:00:00Z".into(),
            schema_cid: None,
            note: None,
        }
    }

    #[tokio::test]
    async fn edges_recall_cites_subject_and_objects() {
        let e = mk_edge("subj-a", "replaced_by", "obj-b", 10, None);
        let storage = Arc::new(MockEdgeStorage::new(vec![e.clone()]));
        let srv = test_server(storage);
        let resp = edges_recall(
            &EdgesRecallReq {
                subj: "subj-a".into(),
                ..Default::default()
            },
            &srv,
        )
        .await
        .expect("ok");
        assert_eq!(resp.edges.len(), 1);
        assert_eq!(resp.objs, vec!["obj-b".to_string()]);
        // fact_cids = subj + obj.
        let fc: Vec<&str> = resp.receipt.fact_cids.iter().map(|c| c.as_str()).collect();
        assert!(fc.contains(&"subj-a"));
        assert!(fc.contains(&"obj-b"));
        // edge_cids cite the returned edge.
        assert_eq!(resp.receipt.edge_cids.len(), 1);
        assert_eq!(resp.receipt.edge_cids[0], e.cid());
    }

    /// The signed receipt MUST verify offline, reconstructing the preimage
    /// INCLUDING the new edges segment (after as_of, before manifest).
    #[tokio::test]
    async fn edges_recall_receipt_verifies_offline() {
        let e = mk_edge("subj-a", "replaced_by", "obj-b", 10, None);
        let storage = Arc::new(MockEdgeStorage::new(vec![e]));
        let srv = test_server(storage);
        let resp = edges_recall(
            &EdgesRecallReq {
                subj: "subj-a".into(),
                ..Default::default()
            },
            &srv,
        )
        .await
        .expect("ok");
        let r = &resp.receipt;
        assert!(!r.edge_cids.is_empty(), "edge_cids must be present");

        use blake3::Hasher;
        let edges_hex = {
            let mut strs: Vec<String> =
                r.edge_cids.iter().map(|c| c.as_str().to_string()).collect();
            strs.sort();
            let mut buf = Vec::new();
            let _ = ciborium::into_writer(&strs, &mut buf);
            data_encoding::HEXLOWER.encode(blake3::hash(&buf).as_bytes())
        };
        let manifest_hex_opt = if r.source_versions.is_empty() {
            None
        } else {
            let mut buf = Vec::new();
            let _ = ciborium::into_writer(&r.source_versions, &mut buf);
            Some(data_encoding::HEXLOWER.encode(blake3::hash(&buf).as_bytes()))
        };
        let mut h = Hasher::new();
        h.update(r.request_id.as_bytes());
        h.update(b"|");
        h.update(r.served_at.as_bytes());
        h.update(b"|");
        // no scope, no as_of in this call → straight to edges segment.
        h.update(edges_hex.as_bytes());
        h.update(b"|");
        if let Some(ref mh) = manifest_hex_opt {
            h.update(mh.as_bytes());
            h.update(b"|");
        }
        h.update(r.primitive.as_bytes());
        h.update(b"|");
        for c in &r.cells {
            h.update(c.as_bytes());
            h.update(b",");
        }
        h.update(b"|");
        for c in &r.fact_cids {
            h.update(c.as_str().as_bytes());
            h.update(b",");
        }
        let msg = h.finalize();
        let pk = ed25519_dalek::VerifyingKey::from_bytes(&r.responder.0).expect("pubkey");
        let sig = ed25519_dalek::Signature::from_bytes(&r.signature.0);
        pk.verify_strict(msg.as_bytes(), &sig)
            .expect("edges_recall receipt must verify against responder pubkey");
    }

    #[tokio::test]
    async fn empty_subject_returns_honest_hint() {
        let storage = Arc::new(MockEdgeStorage::new(vec![]));
        let srv = test_server(storage);
        let resp = edges_recall(
            &EdgesRecallReq {
                subj: "nope".into(),
                ..Default::default()
            },
            &srv,
        )
        .await
        .expect("ok");
        assert!(resp.edges.is_empty());
        assert!(resp.receipt.edge_cids.is_empty());
        assert!(resp.agent_hint.contains("No edges"));
    }
}
