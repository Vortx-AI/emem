//! The three fact variants.

use serde::{Deserialize, Serialize};

use emem_core::{AttesterKey, Signature};
use crate::cid::{FactCid, ReasonCid, SchemaCid};

/// Tagged enum over the three fact variants. Tag is the CBOR field `kind`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Fact {
    /// A directly-attested observation about a (cell, band, tslot).
    Primary(PrimaryFact),
    /// A fact derived deterministically from one or more parent facts.
    Derivative(DerivativeFact),
    /// A confirmed absence — distinct from `null` / `unknown`.
    Absence(NegativeFact),
}

/// String enum used for switching at the wire level.
pub mod kind {
    pub const PRIMARY: &str    = "primary";
    pub const DERIVATIVE: &str = "derivative";
    pub const ABSENCE: &str    = "absence";
}

/// The kind discriminator (matches `Fact` variants).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactKind { Primary, Derivative, Absence }

/// A primary observation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrimaryFact {
    /// cell64 string.
    pub cell: String,
    /// Band key (e.g. `"indices.ndvi"`).
    pub band: String,
    /// Time slot — see `emem_core::Tslot`.
    pub tslot: u64,
    /// Band-typed value (numeric, vector, enum).
    pub value: ciborium::Value,
    /// SI unit if applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// 0..1
    pub confidence: f32,
    /// Optional uncertainty distribution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uncertainty: Option<Uncertainty>,
    /// At least one source.
    pub sources: Vec<Source>,
    /// Recipe for re-execution.
    pub derivation: Derivation,
    /// Privacy class as serialized at attestation time (snake_case).
    pub privacy_class: String,
    /// CID of the CDDL fragment this conforms to.
    pub schema_cid: SchemaCid,
    /// ed25519 attester pubkey.
    pub signer: AttesterKey,
    /// ISO 8601 wall clock at signing time (NOT the data time — that's `tslot`).
    pub signed_at: String,
}

/// A derivative fact: deterministic function over parent fact CIDs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DerivativeFact {
    /// cell64 string.
    pub cell: String,
    /// Band key the derivative pertains to.
    pub band: String,
    /// Inclusive [start, end] tslot window.
    pub tslot_window: [u64; 2],
    /// Operator: "delta" | "mean" | "trend" | "rate" | "anomaly".
    pub op: String,
    /// CIDs of input facts.
    pub parents: Vec<FactCid>,
    /// Output value.
    pub value: ciborium::Value,
    /// 0..1
    pub confidence: f32,
    /// Function registry recipe.
    pub derivation: Derivation,
    /// CID of the CDDL fragment.
    pub schema_cid: SchemaCid,
    /// Attester pubkey.
    pub signer: AttesterKey,
    /// ISO 8601 wall clock.
    pub signed_at: String,
}

/// A negative fact — absence with an evidence pointer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NegativeFact {
    /// cell64 string.
    pub cell: String,
    /// Band key whose absence is asserted.
    pub band: String,
    /// Time slot.
    pub tslot: u64,
    /// CID of evidence that confirmed the absence (e.g. an S1 scene).
    pub reason_cid: ReasonCid,
    /// 0..1
    pub confidence: f32,
    /// At least one source.
    pub sources: Vec<Source>,
    /// CID of the CDDL fragment.
    pub schema_cid: SchemaCid,
    /// Attester pubkey.
    pub signer: AttesterKey,
    /// ISO 8601 wall clock.
    pub signed_at: String,
}

/// An upstream source artifact reference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    /// Provider scheme: `"sentinel2.l2a"`, `"geotessera.v1"`, `"copernicus.dem.30m"`, ...
    pub scheme: String,
    /// Provider-defined ID (tile, scene, etc.).
    pub id: String,
    /// IPLD CID if available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cid: Option<String>,
    /// SHA-256 of source bytes if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash: Option<[u8; 32]>,
    /// ISO 8601 capture time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub captured_at: Option<String>,
    /// Direct fetch URL for the upstream artifact (when known and stable).
    /// Lets agents download the raw COG/parquet/JSON themselves instead of
    /// emem proxying bytes — keeps the protocol structured-fact-only while
    /// still surfacing the multimodal handoff. Optional; producers omit it
    /// when no stable URL exists or licensing forbids redistribution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Derivation recipe — function registry key + args.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Derivation {
    /// Function registry key, e.g. `"nv.l2a@1"`.
    pub fn_key: String,
    /// Deterministic arguments.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<ciborium::Value>,
}

/// Uncertainty distribution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Uncertainty {
    /// Family: "gaussian" | "interval" | "categorical".
    pub family: String,
    /// Family-specific parameters (CBOR map).
    pub params: ciborium::Value,
}
