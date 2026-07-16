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

use emem_core::ErrorCode;
use emem_fact::{EdgeFact, FactCid, Receipt};
use emem_storage::{AsOfBound, Server, StorageError};

/// Default cap on returned edges (env `EMEM_EDGES_DEFAULT_LIMIT`, default
/// 100). Override per-call with `limit`.
fn default_limit() -> usize {
    crate::memory_consolidation::env_usize("EMEM_EDGES_DEFAULT_LIMIT", 100, 1, 1_000_000)
}
/// Hard ceiling on `limit` regardless of caller value (env
/// `EMEM_EDGES_MAX_LIMIT`, default 1000).
fn max_limit() -> usize {
    crate::memory_consolidation::env_usize("EMEM_EDGES_MAX_LIMIT", 1_000, 1, 1_000_000)
}

/// Request: an anchor fact (subject for `direction="out"`, object for
/// `direction="in"`), optional predicate + valid-time bound.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EdgesRecallReq {
    /// Subject fact CID for the forward (`out`) direction — edges
    /// originating here are returned. Leave empty when reading the reverse
    /// (`in`) direction with `obj` set.
    #[serde(default)]
    pub subj: String,
    /// Object fact CID for the reverse (`in`) direction — edges
    /// TERMINATING here ("what points at this fact") are returned. Setting
    /// this implies `direction="in"` unless `direction` is given
    /// explicitly and consistently. (v0.0.9 reverse lookup.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub obj: Option<String>,
    /// Traversal direction. `"out"` (the default) walks `subj --pred--> ?`
    /// (what this fact points at); `"in"` walks `? --pred--> obj` (what
    /// points at this fact — disagrees-with / supersedes / relates-to).
    /// When omitted it is inferred: `obj` set → `"in"`, else `"out"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
    /// Predicate filter. Empty string `""` (the default) scans every
    /// predicate for the anchor fact.
    #[serde(default)]
    pub pred: String,
    /// Bi-temporal valid-time bound. When set, only edges with
    /// `valid_from <= as_of_tslot` and `valid_to` either `None` or
    /// `>= as_of_tslot` are returned (closed intervals); supersession
    /// keeps the newest edge per neighbour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub as_of_tslot: Option<u64>,
    /// Maximum edges to return. Defaults to 100, capped at 1000.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

/// Resolved traversal direction for an [`EdgesRecallReq`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    /// `subj --pred--> ?` — forward, what the anchor points at.
    Out,
    /// `? --pred--> obj` — reverse, what points at the anchor.
    In,
}

impl EdgesRecallReq {
    /// Resolve `(direction, anchor_cid)` from the request, rejecting the
    /// ambiguous / empty cases with an honest error rather than a silent
    /// empty result (the no-silent-fallbacks contract).
    fn resolve(&self) -> Result<(Direction, String), StorageError> {
        let subj = self.subj.trim();
        let obj = self.obj.as_deref().map(str::trim).unwrap_or("");
        let explicit: Option<Direction> = match self.direction.as_deref().map(str::trim) {
            None | Some("") => None,
            Some("out") => Some(Direction::Out),
            Some("in") => Some(Direction::In),
            Some(other) => {
                return Err(StorageError::Protocol {
                    code: ErrorCode::InvalidArgument,
                    message: format!(
                        "edges_recall: unknown direction '{other}'. Use \"out\" (subj→objs, the default) or \"in\" (obj→subjs, what points at this fact)."
                    ),
                });
            }
        };
        // Infer direction when not given: obj present (and subj absent) → in,
        // else out.
        let dir = match explicit {
            Some(d) => d,
            None if !obj.is_empty() && subj.is_empty() => Direction::In,
            None => Direction::Out,
        };
        match dir {
            Direction::Out => {
                if subj.is_empty() {
                    return Err(StorageError::Protocol {
                        code: ErrorCode::InvalidArgument,
                        message: "edges_recall: direction=\"out\" requires a non-empty `subj` (subject fact CID).".into(),
                    });
                }
                if !obj.is_empty() {
                    return Err(StorageError::Protocol {
                        code: ErrorCode::InvalidArgument,
                        message: "edges_recall: ambiguous request — direction=\"out\" with `obj` set. Set exactly one of `subj` (out) or `obj` (in).".into(),
                    });
                }
                Ok((Direction::Out, subj.to_string()))
            }
            Direction::In => {
                if obj.is_empty() {
                    return Err(StorageError::Protocol {
                        code: ErrorCode::InvalidArgument,
                        message: "edges_recall: direction=\"in\" requires a non-empty `obj` (object fact CID — the fact you want the inbound edges of).".into(),
                    });
                }
                if !subj.is_empty() {
                    return Err(StorageError::Protocol {
                        code: ErrorCode::InvalidArgument,
                        message: "edges_recall: ambiguous request — direction=\"in\" with `subj` set. Set exactly one of `subj` (out) or `obj` (in).".into(),
                    });
                }
                Ok((Direction::In, obj.to_string()))
            }
        }
    }
}

/// Response: matching edges, the neighbour CIDs, an agent hint, and the
/// signed receipt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgesRecallResp {
    /// Edges in ascending `valid_from` order (ties broken by object CID).
    pub edges: Vec<EdgeFact>,
    /// Direction this response answered: `"out"` (subj→objs) or `"in"`
    /// (obj→subjs). Added in v0.0.9; the forward (`out`) path keeps the
    /// historical `objs` field, so legacy readers are byte-unaffected.
    pub direction: String,
    /// Distinct OBJECT fact CIDs cited, in first-seen order. Always
    /// present for the forward (`out`) direction (back-compat); on the
    /// reverse (`in`) direction this is the distinct subjects' *objects*,
    /// i.e. the anchor object itself — so for `in` reads prefer `subjs`.
    pub objs: Vec<String>,
    /// Distinct SUBJECT fact CIDs cited ("what points at the anchor"), in
    /// first-seen order. Present (non-`None`) only for the reverse (`in`)
    /// direction; absent on the forward path so the pre-v0.0.9 `out`
    /// response shape is byte-identical. (v0.0.9 reverse lookup.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subjs: Option<Vec<String>>,
    /// One-paragraph agent-facing summary of what was scanned.
    pub agent_hint: String,
    /// Signed receipt — `fact_cids` = anchor + neighbours; `edge_cids`
    /// commit the returned edges into the signature preimage.
    pub receipt: Receipt,
}

/// Recall edges for an anchor fact. `direction="out"` (default) returns
/// edges the anchor SUBJECT points at; `direction="in"` returns edges that
/// point AT the anchor OBJECT. See module docs for semantics.
pub async fn edges_recall(
    req: &EdgesRecallReq,
    srv: &Server,
) -> Result<EdgesRecallResp, StorageError> {
    let started = Instant::now();
    let limit = req
        .limit
        .unwrap_or_else(default_limit)
        .clamp(1, max_limit());
    let (direction, anchor_cid) = req.resolve()?;
    let anchor = FactCid::new(&anchor_cid);

    let edges = match direction {
        Direction::Out => {
            srv.storage
                .recall_edges(&anchor, &req.pred, req.as_of_tslot, limit)
                .await?
        }
        Direction::In => {
            srv.storage
                .recall_edges_by_obj(&anchor, &req.pred, req.as_of_tslot, limit)
                .await?
        }
    };

    // Distinct objects (forward) and subjects (reverse), first-seen order.
    // `objs` is always populated (back-compat for the `out` path); `subjs`
    // is filled only on the reverse path so the `out` wire shape is
    // byte-identical to pre-v0.0.9.
    let mut objs: Vec<String> = Vec::new();
    for e in &edges {
        let o = e.obj.as_str().to_string();
        if !objs.iter().any(|x| x == &o) {
            objs.push(o);
        }
    }
    let subjs: Option<Vec<String>> = match direction {
        Direction::Out => None,
        Direction::In => {
            let mut v: Vec<String> = Vec::new();
            for e in &edges {
                let s = e.subj.as_str().to_string();
                if !v.iter().any(|x| x == &s) {
                    v.push(s);
                }
            }
            Some(v)
        }
    };

    // Receipt citations: the anchor fact + every distinct NEIGHBOUR fact.
    // Forward → neighbours are the objects; reverse → neighbours are the
    // subjects. Cells are not derivable from a fact CID alone (the CID is a
    // content address, not a cell64), so `cells` is empty — the
    // load-bearing binding is `fact_cids` + `edge_cids`.
    let neighbours: &[String] = match &subjs {
        Some(s) => s,
        None => &objs,
    };
    let mut fact_cids: Vec<FactCid> = Vec::with_capacity(neighbours.len() + 1);
    fact_cids.push(anchor.clone());
    for n in neighbours {
        fact_cids.push(FactCid::new(n));
    }
    let edge_cids: Vec<emem_fact::EdgeCid> = edges.iter().map(|e| e.cid()).collect();

    let agent_hint = build_agent_hint(&edges, direction, &anchor_cid, &req.pred, req.as_of_tslot);

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
        direction: match direction {
            Direction::Out => "out".into(),
            Direction::In => "in".into(),
        },
        objs,
        subjs,
        agent_hint,
        receipt,
    })
}

fn build_agent_hint(
    edges: &[EdgeFact],
    direction: Direction,
    anchor: &str,
    pred: &str,
    as_of: Option<u64>,
) -> String {
    let pred_label = if pred.is_empty() {
        "any predicate".to_string()
    } else {
        format!("predicate '{pred}'")
    };
    let asof_label = match as_of {
        Some(t) => format!(" as of valid-time {t}"),
        None => String::new(),
    };
    match direction {
        Direction::Out => {
            if edges.is_empty() {
                return format!(
                    "No edges originate at subject '{anchor}' under {pred_label}{asof_label}. This is the honest 'no relation known', not a lookup failure — write edges by POSTing a signed Attestation with an `edges` array to /v1/edges."
                );
            }
            format!(
                "Found {n} edge(s) from subject '{anchor}' under {pred_label}{asof_label}. Each edge is content-addressed and the receipt's edge_cids commit them into the signature; verify offline via /v1/verify_receipt, and follow `obj` to recall the related fact.",
                n = edges.len()
            )
        }
        Direction::In => {
            if edges.is_empty() {
                return format!(
                    "Nothing points at object '{anchor}' under {pred_label}{asof_label}. This is the honest 'no fact references this one', not a lookup failure — inbound edges (disagrees_with / supersedes / relates_to) are written by POSTing a signed Attestation with an `edges` array to /v1/edges."
                );
            }
            format!(
                "Found {n} edge(s) pointing AT object '{anchor}' under {pred_label}{asof_label} (reverse lookup — what disagrees-with / supersedes / relates-to this fact). Each edge's `subj` is the fact that references the anchor; follow it to recall that fact, and verify the receipt's edge_cids offline via /v1/verify_receipt.",
                n = edges.len()
            )
        }
    }
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
        async fn recall_edges_by_obj(
            &self,
            obj: &FactCid,
            pred: &str,
            as_of: Option<u64>,
            limit: usize,
        ) -> Result<Vec<EdgeFact>, StorageError> {
            let mut out: Vec<EdgeFact> = self
                .edges
                .lock()
                .unwrap()
                .iter()
                .filter(|e| e.obj.as_str() == obj.as_str())
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

        // Rebuild the v1 preimage exactly as an offline verifier does.
        assert_eq!(r.preimage_version, emem_attest::PREIMAGE_V1);
        let edges_hex = emem_storage::Server::edges_blake3_hex(&r.edge_cids);
        let manifest_hex_opt = if r.source_versions.is_empty() {
            None
        } else {
            let mut buf = Vec::new();
            let _ = ciborium::into_writer(&r.source_versions, &mut buf);
            Some(data_encoding::HEXLOWER.encode(blake3::hash(&buf).as_bytes()))
        };
        let msg = emem_attest::receipt_preimage_v1(
            &r.request_id,
            &r.served_at,
            None,
            None,
            edges_hex.as_deref(),
            manifest_hex_opt.as_deref(),
            None,
            &r.primitive,
            r.cells.iter().map(|s| s.as_str()),
            r.fact_cids.iter().map(|c| c.as_str()),
        );
        let pk = ed25519_dalek::VerifyingKey::from_bytes(&r.responder.0).expect("pubkey");
        let sig = ed25519_dalek::Signature::from_bytes(&r.signature.0);
        pk.verify_strict(&msg, &sig)
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

    /// Reverse lookup through the primitive: `obj` set (no `subj`) infers
    /// `direction="in"`, dispatches to `recall_edges_by_obj`, and the
    /// response names the subjects pointing at the anchor.
    #[tokio::test]
    async fn reverse_lookup_via_primitive() {
        let e = mk_edge("subj-a", "relates_to", "obj-b", 10, None);
        let storage = Arc::new(MockEdgeStorage::new(vec![e.clone()]));
        let srv = test_server(storage);
        let resp = edges_recall(
            &EdgesRecallReq {
                obj: Some("obj-b".into()),
                ..Default::default()
            },
            &srv,
        )
        .await
        .expect("ok");
        assert_eq!(resp.direction, "in");
        assert_eq!(resp.edges.len(), 1);
        assert_eq!(resp.subjs.as_deref(), Some(&["subj-a".to_string()][..]));
        // Receipt cites anchor (obj-b) + neighbour subject (subj-a).
        let fc: Vec<&str> = resp.receipt.fact_cids.iter().map(|c| c.as_str()).collect();
        assert!(fc.contains(&"obj-b"));
        assert!(fc.contains(&"subj-a"));
        assert_eq!(resp.receipt.edge_cids[0], e.cid());
        assert!(resp.agent_hint.contains("pointing AT"));
    }

    /// Forward path is byte-unchanged: `subjs` is absent on the wire.
    #[tokio::test]
    async fn forward_response_omits_subjs() {
        let e = mk_edge("subj-a", "rel", "obj-b", 1, None);
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
        assert_eq!(resp.direction, "out");
        assert!(resp.subjs.is_none());
        let v = serde_json::to_value(&resp).unwrap();
        assert!(
            v.get("subjs").is_none(),
            "forward response must not carry a `subjs` key"
        );
    }

    /// Ambiguity + emptiness are honest errors, never a silent empty.
    #[tokio::test]
    async fn ambiguous_and_empty_requests_error() {
        let storage = Arc::new(MockEdgeStorage::new(vec![]));
        let srv = test_server(storage);

        // Neither subj nor obj.
        let err = edges_recall(&EdgesRecallReq::default(), &srv)
            .await
            .expect_err("empty must error");
        assert!(matches!(err, StorageError::Protocol { .. }));

        // Both subj and obj.
        let both = edges_recall(
            &EdgesRecallReq {
                subj: "subj-a".into(),
                obj: Some("obj-b".into()),
                ..Default::default()
            },
            &srv,
        )
        .await
        .expect_err("ambiguous must error");
        assert!(matches!(both, StorageError::Protocol { .. }));

        // direction=in but no obj.
        let dir = edges_recall(
            &EdgesRecallReq {
                direction: Some("in".into()),
                ..Default::default()
            },
            &srv,
        )
        .await
        .expect_err("in without obj must error");
        assert!(matches!(dir, StorageError::Protocol { .. }));

        // unknown direction.
        let unk = edges_recall(
            &EdgesRecallReq {
                subj: "subj-a".into(),
                direction: Some("sideways".into()),
                ..Default::default()
            },
            &srv,
        )
        .await
        .expect_err("unknown direction must error");
        assert!(matches!(unk, StorageError::Protocol { .. }));
    }
}
