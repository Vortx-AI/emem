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
            args: scalar_args(&[
                ("cell", cell.clone()),
                ("band", band.clone()),
                ("tslot_a", window[0].to_string()),
                ("tslot_b", window[1].to_string()),
            ]),
        }],
        Intent::FindLike { key, .. } => vec![ToolCall {
            primitive: "emem_find_similar".into(),
            args: scalar_args(&[("key", key.clone())]),
        }],
        Intent::Confirm { cell, .. } => vec![ToolCall {
            primitive: "emem_verify".into(),
            args: scalar_args(&[("cell", cell.clone())]),
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
