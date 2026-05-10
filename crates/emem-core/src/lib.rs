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

pub mod algorithms;
pub mod bands;
pub mod bbox;
pub mod cell;
pub mod error;
pub mod functions;
pub mod key;
pub mod manifest;
pub mod polygon;
pub mod privacy;
pub mod schema;
pub mod sources;
pub mod taxonomy;
pub mod topics;
pub mod tslot;

pub use algorithms::{Algorithm, AlgorithmInput, AlgorithmKind, AlgorithmRegistry};
pub use bands::{Band, BandFamily, BandRegistry};
pub use bbox::{Bbox, BboxError};
pub use cell::{BaseCell, Cell, Mode, Resolution, DEFAULT_RESOLUTION, MAX_RESOLUTION};
pub use error::{Error, ErrorCode};
pub use functions::{FnKind, Function, FunctionRegistry, SourceRequirement};
pub use key::{AttesterKey, KeyEpoch, Signature};
pub use manifest::{manifest_cid, Manifest, ManifestError};
pub use polygon::Polygon;
pub use privacy::PrivacyClass;
pub use schema::{SchemaFragment, SchemaRegistry};
pub use sources::{ConnectorKind, Provider, SourceRegistry, SourceScheme};
pub use taxonomy::{Lcv1, LcvFamily};
pub use topics::{Topic, TopicRegistry, TopicRoutingPolicy};
pub use tslot::{Tempo, Tslot};
