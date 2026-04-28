//! emem-core — fundamental types of the emem protocol.
//!
//! This crate is intentionally small. It pins the wire-stable type identities
//! and the manifest-loading machinery that every other crate depends on.
//!
//! What lives here (the **protocol itself**):
//!   - cell algebra (bit layout, parent/child operators)
//!   - tslot type + tempo classes
//!   - manifest loader pattern (`Manifest` trait + CID derivation)
//!   - ed25519 key types
//!   - structured error catalog
//!
//! What does NOT live here (it's data, loaded from manifests):
//!   - the band ontology (in `data/bands-v0.json`, loaded by [`bands::DEFAULT`])
//!   - the function registry (in `data/functions-v0.json`)
//!   - the source-connector registry (in `data/sources-v0.json`)
//!   - the cell64 alphabet (in `crates/emem-codec/data/`)
//!   - the lcv-1 taxonomy (8 families × 8 leaves, structural IDs in core;
//!     mnemonic labels live in an operator-published label manifest)

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod cell;
pub mod bbox;
pub mod tslot;
pub mod manifest;
pub mod bands;
pub mod functions;
pub mod sources;
pub mod algorithms;
pub mod schema;
pub mod taxonomy;
pub mod privacy;
pub mod key;
pub mod error;

pub use cell::{Cell, Resolution, BaseCell, Mode, DEFAULT_RESOLUTION, MAX_RESOLUTION};
pub use bbox::{Bbox, BboxError};
pub use tslot::{Tslot, Tempo};
pub use manifest::{Manifest, ManifestError, manifest_cid};
pub use bands::{Band, BandFamily, BandRegistry};
pub use functions::{Function, FnKind, FunctionRegistry, SourceRequirement};
pub use sources::{SourceRegistry, SourceScheme, Provider, ConnectorKind};
pub use algorithms::{Algorithm, AlgorithmKind, AlgorithmRegistry, AlgorithmInput};
pub use schema::{SchemaFragment, SchemaRegistry};
pub use taxonomy::{Lcv1, LcvFamily};
pub use privacy::PrivacyClass;
pub use key::{AttesterKey, KeyEpoch, Signature};
pub use error::{Error, ErrorCode};
