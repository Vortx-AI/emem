//! Active schema bundle — loaded from the `emem-schema` content-addressed
//! manifest. Operators may publish their own bundle CID; the protocol is
//! agnostic to fragment count.

use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

use crate::manifest::{Manifest, ManifestError, MANIFEST_SCHEMA};

/// Embedded default v0 schema manifest.
const SCHEMA_V0_JSON: &str = include_str!("../data/schema-v0.json");

/// A single CDDL/JSON fragment description.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaFragment {
    /// Fragment name, e.g. `"PrimaryFact"`.
    pub name: String,
    /// Hash algorithm (always `"blake3-32"` in v0).
    pub cid_alg: String,
    /// Canonical encoding (always `"canonical-cbor"`).
    pub encoding: String,
}

/// The full schema manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaRegistry {
    /// Manifest discriminator, MUST equal `"emem-schema"`.
    pub manifest: String,
    /// Manifest version, e.g. `"v0"`.
    pub version: String,
    /// Listed fragments.
    pub fragments: Vec<SchemaFragment>,
    /// Hash algorithm name.
    pub hash: String,
    /// Signature algorithm name.
    pub signature: String,
    /// CID encoding ("base32-nopad-lowercase" in v0).
    pub cid_encoding: String,
    /// Optional editorial note.
    #[serde(default, rename = "_note", skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Manifest for SchemaRegistry {
    const KIND: &'static str = MANIFEST_SCHEMA;

    fn validate(&self) -> Result<(), ManifestError> {
        if self.manifest != Self::KIND {
            return Err(ManifestError::WrongKind {
                expected: Self::KIND,
                actual: self.manifest.clone(),
            });
        }
        if self.fragments.is_empty() {
            return Err(ManifestError::Invalid(
                "schema bundle must list at least one fragment".into(),
            ));
        }
        if self.hash != "blake3" {
            return Err(ManifestError::Invalid(format!(
                "v0 supports hash=blake3; got '{}'",
                self.hash
            )));
        }
        if self.signature != "ed25519" {
            return Err(ManifestError::Invalid(format!(
                "v0 supports signature=ed25519; got '{}'",
                self.signature
            )));
        }
        Ok(())
    }
}

/// Default v0 schema bundle, parsed once.
pub static DEFAULT: LazyLock<SchemaRegistry> = LazyLock::new(|| {
    SchemaRegistry::parse_json(SCHEMA_V0_JSON.as_bytes()).expect("default schema bundle parses")
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_parses_and_validates() {
        let r = &*DEFAULT;
        assert_eq!(r.manifest, MANIFEST_SCHEMA);
        assert!(!r.fragments.is_empty());
        assert!(r.fragments.iter().any(|f| f.name == "PrimaryFact"));
    }
}
