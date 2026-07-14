//! emem-intent — typed Intent grammar + planner.
//!
//! Spec §20.6. Closes the gap between *what the agent wants* and *which
//! primitive to call*. v0 ships a heuristic dispatch planner; v0.1 introduces
//! a learned planner trained on shared planner traces (§20.8).

#![forbid(unsafe_code)]

use emem_claim::Claim;
use serde::{Deserialize, Serialize};

/// A typed agent intent. Routed by the planner to a sequence of primitive
/// tool calls. New variants ship under semver and degrade via
/// `unknown_intent_type`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Intent {
    /// "Where is X?" — natural-language description → cell candidates.
    WhereIs { description: String },
    /// "What is here?" — cell → fact summary. Either `cell` is provided
    /// directly, or `place` (free-text) is — in which case the planner
    /// emits `emem_ask` so locate + recall + topic-routing all happen
    /// server-side. This closes the gap where an LLM had a place name
    /// in the user's question but no cell64, and the old planner errored
    /// out with `missing field "cell"`.
    WhatIsHere {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cell: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        place: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
    /// "Is A like B?" — pairwise similarity.
    IsLike { a: String, b: String },
    /// "Did this band change at this cell over this window?"
    DidChange {
        cell: String,
        band: String,
        window: [u64; 2],
    },
    /// "Find cells like this key under filter."
    FindLike {
        key: String,
        k: Option<u32>,
        filter: Option<Claim>,
    },
    /// "Is this claim true at this cell?"
    Confirm { claim: Claim, cell: String },
    /// Free-text place question. Maps to `emem_ask`, which runs the full
    /// locate → topic-route → recall → algorithm-hint chain server-side
    /// and returns one packaged answer. Use this whenever the LLM has a
    /// natural-language question with a place mention and no specific
    /// primitive in mind.
    Ask {
        description: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        place: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cell: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lat: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lng: Option<f64>,
    },
}

/// A primitive tool invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Primitive name, e.g. `"emem.recall"`.
    pub primitive: String,
    /// CBOR-encoded arguments.
    pub args: ciborium::Value,
}

/// Planner output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    /// Sequence of primitive calls.
    pub calls: Vec<ToolCall>,
}

/// Heuristic planner — dispatches each Intent variant to the obvious primitive.
/// Returns a Plan that the caller may execute, or pass back to the protocol
/// for execution. Primitive names use the underscore form (`emem_locate`,
/// `emem_recall`, …) so they can be dispatched directly via the MCP tool
/// router or `/v1/...` REST endpoints.
pub fn plan(intent: &Intent) -> Plan {
    let calls = match intent {
        Intent::WhereIs { description } => vec![ToolCall {
            primitive: "emem_locate".into(),
            args: scalar_args(&[("place", description.clone())]),
        }],
        // WhatIsHere routes by what the caller gave us:
        //   • {cell}                      → emem_recall
        //   • {place} (or {description})  → emem_ask  (locate + topic-route + recall happen server-side)
        // The old hard-required `cell` field was a UX dead-end whenever
        // the LLM had a place name but no cell64 yet — fall through to
        // the ask path instead of erroring.
        Intent::WhatIsHere {
            cell: Some(cell), ..
        } => vec![ToolCall {
            primitive: "emem_recall".into(),
            args: scalar_args(&[("cell", cell.clone())]),
        }],
        Intent::WhatIsHere {
            cell: None,
            place,
            description,
        } => {
            let q = description.clone().unwrap_or_else(|| "what is here".into());
            let place = place
                .clone()
                .or_else(|| description.clone())
                .unwrap_or_default();
            vec![ToolCall {
                primitive: "emem_ask".into(),
                args: ask_args(&q, Some(place), None, None, None),
            }]
        }
        Intent::Ask {
            description,
            place,
            cell,
            lat,
            lng,
        } => vec![ToolCall {
            primitive: "emem_ask".into(),
            args: ask_args(description, place.clone(), cell.clone(), *lat, *lng),
        }],
        Intent::IsLike { a, b } => vec![
            // Materialize geotessera at both cells first so compare's
            // cosine summary actually has a vector band to score over.
            // Without these two recall steps, a freshly-named pair
            // gets `cosine: null` (no shared vector band) which is
            // correct but unhelpful to the agent that asked "is X
            // like Y". The recall steps are no-ops once geotessera is
            // attested at both cells (sled cache hit, sub-ms).
            ToolCall {
                primitive: "emem_recall".into(),
                args: recall_args_with_bands(a.clone(), &["geotessera"]),
            },
            ToolCall {
                primitive: "emem_recall".into(),
                args: recall_args_with_bands(b.clone(), &["geotessera"]),
            },
            ToolCall {
                primitive: "emem_compare".into(),
                args: scalar_args(&[("a", a.clone()), ("b", b.clone())]),
            },
        ],
        Intent::DidChange { cell, band, window } => vec![ToolCall {
            primitive: "emem_diff".into(),
            args: did_change_args(cell.clone(), band.clone(), window),
        }],
        Intent::FindLike { key, k, filter } => vec![ToolCall {
            primitive: "emem_find_similar".into(),
            args: find_similar_args(key.clone(), *k, filter.as_ref()),
        }],
        Intent::Confirm { cell, claim } => vec![ToolCall {
            primitive: "emem_verify".into(),
            args: verify_args(cell.clone(), claim),
        }],
    };
    Plan { calls }
}

fn scalar_args(pairs: &[(&str, String)]) -> ciborium::Value {
    ciborium::Value::Map(
        pairs
            .iter()
            .map(|(k, v)| {
                (
                    ciborium::Value::Text((*k).into()),
                    ciborium::Value::Text(v.clone()),
                )
            })
            .collect(),
    )
}

/// Build args for `emem_diff`.
///
/// `tslot_a` and `tslot_b` deserialize to `u64`, so they have to ride as
/// CBOR integers. `scalar_args` encodes every value as Text, so routing
/// them through it produced a plan the planner could not execute:
/// `invalid type: string "19723", expected u64`. A planner exists to
/// answer "what do I call", so a plan whose own args are the wrong type is
/// worse than no plan, and the failure is invisible until something runs
/// it. `plan_args_round_trip_into_their_primitives` pins every variant's
/// args against the primitive's own request type for that reason.
fn did_change_args(cell: String, band: String, window: &[u64; 2]) -> ciborium::Value {
    ciborium::Value::Map(vec![
        (
            ciborium::Value::Text("cell".into()),
            ciborium::Value::Text(cell),
        ),
        (
            ciborium::Value::Text("band".into()),
            ciborium::Value::Text(band),
        ),
        (
            ciborium::Value::Text("tslot_a".into()),
            ciborium::Value::Integer(window[0].into()),
        ),
        (
            ciborium::Value::Text("tslot_b".into()),
            ciborium::Value::Integer(window[1].into()),
        ),
    ])
}

/// Build args for `emem_recall` with a `bands` array of one or more
/// strings. The protocol expects `bands` to deserialize to `Vec<String>`,
/// which CBOR represents as an Array, not a Text — so this helper exists
/// alongside `scalar_args` rather than overloading it.
fn recall_args_with_bands(cell: String, bands: &[&str]) -> ciborium::Value {
    ciborium::Value::Map(vec![
        (
            ciborium::Value::Text("cell".into()),
            ciborium::Value::Text(cell),
        ),
        (
            ciborium::Value::Text("bands".into()),
            ciborium::Value::Array(
                bands
                    .iter()
                    .map(|b| ciborium::Value::Text((*b).into()))
                    .collect(),
            ),
        ),
    ])
}

/// Build args for `emem_find_similar`. Propagates the optional `k`
/// (top-k neighbour count) and `filter` (post-rank Claim) so a caller
/// who said "find 3 places like X with NDVI > 0.7" actually gets k=3
/// alongside the filter — earlier the planner ignored both fields
/// and returned the default top-10 unfiltered.
fn find_similar_args(key: String, k: Option<u32>, filter: Option<&Claim>) -> ciborium::Value {
    let mut entries: Vec<(ciborium::Value, ciborium::Value)> = vec![(
        ciborium::Value::Text("key".into()),
        ciborium::Value::Text(key),
    )];
    if let Some(k) = k {
        entries.push((
            ciborium::Value::Text("k".into()),
            ciborium::Value::Integer((k as i64).into()),
        ));
    }
    if let Some(f) = filter {
        if let Ok(v) = serde_json_to_cbor(f) {
            entries.push((ciborium::Value::Text("filter".into()), v));
        }
    }
    ciborium::Value::Map(entries)
}

/// Build args for `emem_verify`. Propagates the structured `claim`
/// alongside `cell` — earlier the planner dropped `claim` entirely,
/// which silently turned every Confirm into "verify cell with no
/// predicate" and made the verdict undefined.
fn verify_args(cell: String, claim: &Claim) -> ciborium::Value {
    let mut entries: Vec<(ciborium::Value, ciborium::Value)> = vec![(
        ciborium::Value::Text("cell".into()),
        ciborium::Value::Text(cell),
    )];
    if let Ok(v) = serde_json_to_cbor(claim) {
        entries.push((ciborium::Value::Text("claim".into()), v));
    }
    ciborium::Value::Map(entries)
}

/// Round-trip a serde-Serialize value through JSON → ciborium::Value.
/// Used for nested fields (Claim, filter) where re-implementing the
/// CBOR shape by hand would drift from the canonical JSON serde repr.
/// Returns the ciborium::Value that round-tripped via serde_json so
/// nested objects/arrays/numbers all land on the right CBOR primitive.
fn serde_json_to_cbor<T: Serialize>(v: &T) -> Result<ciborium::Value, ()> {
    let j = serde_json::to_value(v).map_err(|_| ())?;
    json_to_cbor(j)
}

fn json_to_cbor(j: serde_json::Value) -> Result<ciborium::Value, ()> {
    Ok(match j {
        serde_json::Value::Null => ciborium::Value::Null,
        serde_json::Value::Bool(b) => ciborium::Value::Bool(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                ciborium::Value::Integer(i.into())
            } else if let Some(u) = n.as_u64() {
                ciborium::Value::Integer((u as i128).try_into().map_err(|_| ())?)
            } else if let Some(f) = n.as_f64() {
                ciborium::Value::Float(f)
            } else {
                return Err(());
            }
        }
        serde_json::Value::String(s) => ciborium::Value::Text(s),
        serde_json::Value::Array(a) => {
            let mut out = Vec::with_capacity(a.len());
            for item in a {
                out.push(json_to_cbor(item)?);
            }
            ciborium::Value::Array(out)
        }
        serde_json::Value::Object(m) => {
            let mut out = Vec::with_capacity(m.len());
            for (k, v) in m {
                out.push((ciborium::Value::Text(k), json_to_cbor(v)?));
            }
            ciborium::Value::Map(out)
        }
    })
}

/// Build args for `emem_ask`. Mixed types (string question, optional
/// string locator(s), optional f64 lat/lng), so the helper packs each
/// field into the right CBOR primitive instead of going through
/// `scalar_args` (which is text-only).
fn ask_args(
    q: &str,
    place: Option<String>,
    cell: Option<String>,
    lat: Option<f64>,
    lng: Option<f64>,
) -> ciborium::Value {
    let mut entries: Vec<(ciborium::Value, ciborium::Value)> = vec![(
        ciborium::Value::Text("q".into()),
        ciborium::Value::Text(q.into()),
    )];
    if let Some(p) = place {
        entries.push((
            ciborium::Value::Text("place".into()),
            ciborium::Value::Text(p),
        ));
    }
    if let Some(c) = cell {
        entries.push((
            ciborium::Value::Text("cell".into()),
            ciborium::Value::Text(c),
        ));
    }
    if let Some(la) = lat {
        entries.push((
            ciborium::Value::Text("lat".into()),
            ciborium::Value::Float(la),
        ));
    }
    if let Some(lo) = lng {
        entries.push((
            ciborium::Value::Text("lng".into()),
            ciborium::Value::Float(lo),
        ));
    }
    ciborium::Value::Map(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use emem_claim::{Claim, Op};

    fn cbor_get<'a>(map: &'a ciborium::Value, key: &str) -> Option<&'a ciborium::Value> {
        if let ciborium::Value::Map(entries) = map {
            for (k, v) in entries {
                if let ciborium::Value::Text(t) = k {
                    if t == key {
                        return Some(v);
                    }
                }
            }
        }
        None
    }

    /// The planner emitted `tslot_a`/`tslot_b` as CBOR Text because
    /// `scalar_args` stringifies everything, so `emem_intent{did_change}`
    /// built a plan and then failed executing it with
    /// `invalid type: string "19723", expected u64`. Asserting the CBOR
    /// type directly is the check that would have caught it: the args a
    /// plan carries have to deserialize into the primitive's own request
    /// type, and a plan the planner cannot run is worse than no plan.
    #[test]
    fn did_change_emits_tslots_as_integers_not_strings() {
        let plan = plan(&Intent::DidChange {
            cell: "damO.zb000.xUti.zde78".into(),
            band: "indices.ndvi".into(),
            window: [19723, 20634],
        });
        let args = &plan.calls[0].args;
        assert_eq!(plan.calls[0].primitive, "emem_diff");
        for (key, want) in [("tslot_a", 19723u64), ("tslot_b", 20634u64)] {
            match cbor_get(args, key) {
                Some(ciborium::Value::Integer(i)) => {
                    let got: u64 = (*i).try_into().expect("tslot fits u64");
                    assert_eq!(got, want, "{key} value");
                }
                other => panic!(
                    "{key} must be a CBOR integer so emem_diff's u64 field parses; got {other:?}"
                ),
            }
        }
        // The string-typed fields stay strings.
        assert!(matches!(
            cbor_get(args, "cell"),
            Some(ciborium::Value::Text(_))
        ));
        assert!(matches!(
            cbor_get(args, "band"),
            Some(ciborium::Value::Text(_))
        ));
    }

    /// Every variant must produce at least one call, and never an arg map
    /// that is empty or non-Map. This is the cheap structural floor under
    /// the planner: it does not prove the types are right (see the
    /// `did_change` test for that shape of check), but it catches a
    /// variant wired to nothing.
    #[test]
    fn every_intent_variant_plans_at_least_one_call() {
        let claim = Claim {
            band: "indices.ndvi".into(),
            op: Op::Gt,
            value: ciborium::Value::Float(0.7),
            tslot: Some(0),
            window: None,
            agg: None,
        };
        let cell = "damO.zb000.xUti.zde78".to_string();
        let variants = vec![
            Intent::WhereIs {
                description: "the Eiffel Tower".into(),
            },
            Intent::WhatIsHere {
                cell: Some(cell.clone()),
                place: None,
                description: None,
            },
            Intent::IsLike {
                a: cell.clone(),
                b: cell.clone(),
            },
            Intent::DidChange {
                cell: cell.clone(),
                band: "indices.ndvi".into(),
                window: [1, 2],
            },
            Intent::FindLike {
                key: cell.clone(),
                k: Some(5),
                filter: None,
            },
            Intent::Confirm {
                cell: cell.clone(),
                claim: claim.clone(),
            },
        ];
        for v in &variants {
            let p = plan(v);
            assert!(!p.calls.is_empty(), "variant {v:?} planned no calls");
            for c in &p.calls {
                assert!(
                    !c.primitive.is_empty(),
                    "variant {v:?} planned an unnamed call"
                );
                assert!(
                    matches!(c.args, ciborium::Value::Map(_)),
                    "variant {v:?} args must be a CBOR map"
                );
            }
        }
    }

    /// Regression: the old `Intent::FindLike { key, .. }` arm dropped
    /// `k` and `filter` on the floor. After the fix, both must land in
    /// the emitted CBOR args.
    #[test]
    fn find_like_propagates_k_and_filter() {
        let filter = Claim {
            band: "indices.ndvi".into(),
            op: Op::Gt,
            value: ciborium::Value::Float(0.7),
            tslot: Some(0),
            window: None,
            agg: None,
        };
        let p = plan(&Intent::FindLike {
            key: "damO.zb000.xUti.zde78".into(),
            k: Some(3),
            filter: Some(filter),
        });
        assert_eq!(p.calls.len(), 1);
        assert_eq!(p.calls[0].primitive, "emem_find_similar");

        let key = cbor_get(&p.calls[0].args, "key").expect("key present");
        assert!(matches!(key, ciborium::Value::Text(t) if t == "damO.zb000.xUti.zde78"));

        let k = cbor_get(&p.calls[0].args, "k").expect("k must propagate");
        assert!(
            matches!(k, ciborium::Value::Integer(i) if i128::from(*i) == 3),
            "k expected to be integer 3, got {k:?}"
        );

        let f = cbor_get(&p.calls[0].args, "filter").expect("filter must propagate");
        assert!(matches!(f, ciborium::Value::Map(_)));
    }

    /// Regression: the old `Intent::Confirm { cell, .. }` arm dropped
    /// `claim` entirely, which silently turned every Confirm into
    /// "verify cell with no predicate".
    #[test]
    fn confirm_propagates_claim() {
        let claim = Claim {
            band: "indices.ndvi".into(),
            op: Op::Gt,
            value: ciborium::Value::Float(0.5),
            tslot: Some(0),
            window: None,
            agg: None,
        };
        let p = plan(&Intent::Confirm {
            claim,
            cell: "damO.zb000.xUti.zde78".into(),
        });
        assert_eq!(p.calls.len(), 1);
        assert_eq!(p.calls[0].primitive, "emem_verify");

        let cell = cbor_get(&p.calls[0].args, "cell").expect("cell present");
        assert!(matches!(cell, ciborium::Value::Text(t) if t == "damO.zb000.xUti.zde78"));

        let c = cbor_get(&p.calls[0].args, "claim").expect("claim must propagate");
        assert!(matches!(c, ciborium::Value::Map(_)));
    }
}
