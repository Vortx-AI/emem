//! Strongly-typed CID newtypes — keep the wire-string but distinguish purposes
//! at compile time.

use serde::{Deserialize, Serialize};

macro_rules! cid_newtype {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);
        impl $name {
            /// Wrap a string CID.
            pub fn new(s: impl Into<String>) -> Self { $name(s.into()) }
            /// Borrow inner.
            pub fn as_str(&self) -> &str { &self.0 }
        }
    };
}

cid_newtype!(FactCid,     "CID of a Fact (Primary / Derivative / Negative).");
cid_newtype!(RegistryCid, "CID of a function-registry version snapshot.");
cid_newtype!(SchemaCid,   "CID of a CDDL schema fragment.");
cid_newtype!(ReasonCid,   "CID of evidence pointing at why an absence is asserted.");
cid_newtype!(BatchCid,    "CID of a Merkle batch of facts.");
cid_newtype!(CoverageCid, "CID of a coverage manifest snapshot.");
