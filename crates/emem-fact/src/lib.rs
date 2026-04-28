//! emem-fact — fact, attestation, and receipt types.
//!
//! Spec §5–§7. Three fact variants (Primary / Derivative / Negative), each
//! content-addressed under canonical CBOR with the emem-CBOR profile.

#![forbid(unsafe_code)]

pub mod fact;
pub mod attest;
pub mod receipt;
pub mod cbor;
pub mod cid;

pub use fact::{Fact, PrimaryFact, DerivativeFact, NegativeFact, Source, Derivation, Uncertainty, FactKind};
pub use attest::Attestation;
pub use receipt::{Receipt, Cost};
pub use cid::{FactCid, RegistryCid, SchemaCid, ReasonCid};
