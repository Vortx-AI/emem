//! emem-fact — fact, attestation, and receipt types.
//!
//! Spec §5–§7. Three fact variants (Primary / Derivative / Negative), each
//! content-addressed under canonical CBOR with the emem-CBOR profile.

#![forbid(unsafe_code)]

pub mod attest;
pub mod cbor;
pub mod cid;
pub mod edge;
pub mod fact;
pub mod receipt;
pub mod scope;

pub use attest::Attestation;
pub use cid::{EdgeCid, FactCid, ReasonCid, RegistryCid, SchemaCid};
pub use edge::EdgeFact;
pub use fact::{
    Derivation, DerivativeFact, Fact, FactKind, NegativeFact, PrimaryFact, ServedVia, Source,
    Uncertainty,
};
pub use receipt::{AsOfReceipt, Cost, FieldBinding, MerkleProof, Receipt};
pub use scope::Scope;
