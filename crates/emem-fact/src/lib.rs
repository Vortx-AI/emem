//! emem-fact — fact, attestation, and receipt types.
//!
//! Spec §5–§7. Three fact variants (Primary / Derivative / Negative), each
//! content-addressed under canonical CBOR with the emem-CBOR profile.

#![forbid(unsafe_code)]

pub mod attest;
pub mod cbor;
pub mod cid;
pub mod fact;
pub mod receipt;

pub use attest::Attestation;
pub use cid::{FactCid, ReasonCid, RegistryCid, SchemaCid};
pub use fact::{
    Derivation, DerivativeFact, Fact, FactKind, NegativeFact, PrimaryFact, Source, Uncertainty,
};
pub use receipt::{Cost, Receipt};
