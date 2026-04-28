//! emem-claim — structural claim algebra. Spec §8.1.
//!
//! Claims compose from `(band, op, value, tslot|window, agg?)`. No
//! human-mnemonic predicates ("LandCoverIs"). The grammar is extensible by
//! adding ops; new ops MUST ship under semver and degrade gracefully.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

/// A structured predicate over a fact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claim {
    /// Band key (e.g. `"indices.ndvi"`).
    pub band: String,
    /// Comparison or membership op.
    pub op: Op,
    /// Right-hand value (band-typed; CBOR).
    pub value: ciborium::Value,
    /// Specific tslot (one of `tslot` | `window` MUST be set).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tslot: Option<u64>,
    /// Tslot range [start, end] inclusive.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window: Option<[u64; 2]>,
    /// Aggregation over window: "any" | "all" | "mean" | "min" | "max".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agg: Option<String>,
}

/// Comparison / membership operators. Extensible under semver.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Op {
    /// equal
    Eq,
    /// not equal
    Ne,
    /// less than
    Lt,
    /// less or equal
    Le,
    /// greater than
    Gt,
    /// greater or equal
    Ge,
    /// member-of (RHS is a set)
    In,
    /// non-member-of
    Ni,
    /// fact exists for (cell, band, tslot)
    Exists,
    /// confirmed absence
    Absent,
}

/// Claim evaluation errors.
#[derive(Debug, thiserror::Error)]
pub enum ClaimError {
    /// Required tslot/window missing.
    #[error("claim requires either tslot or window")]
    NoTime,
    /// Type mismatch between fact value and claim value.
    #[error("type mismatch evaluating claim band {0}")]
    TypeMismatch(String),
    /// Op not supported for this band's value type.
    #[error("op {0:?} not supported for band {1}")]
    UnsupportedOp(Op, String),
}

/// Evaluator API — given facts at a cell, evaluate a Claim.
pub trait Evaluator {
    /// Evaluate. Returns `Ok(true|false)` if the claim is decidable;
    /// `Err` if undecidable (missing facts, type mismatch).
    fn evaluate(&self, claim: &Claim, cell: &str) -> Result<bool, ClaimError>;
}
