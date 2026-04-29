//! Function registry — loaded from the **content-addressed functions manifest**.
//!
//! Spec §16. Functions describe how to derive a band value from canonical
//! upstream sources. They are content-addressed so every fact pins
//! `Derivation.fn_key` to a registry CID it was attested under.

use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

use crate::manifest::{Manifest, ManifestError, MANIFEST_FUNCTION_REG};

const FUNCTIONS_V0_JSON: &str = include_str!("../data/functions-v0.json");

/// Discriminator for what a function produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FnKind {
    /// Produces a PrimaryFact directly from upstream sources.
    Primary,
    /// Produces a DerivativeFact from parent fact CIDs.
    Derivative,
    /// Produces a NegativeFact (confirmed absence).
    Negative,
}

/// A required upstream source for a Primary or Negative function.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceRequirement {
    /// Source scheme key (resolved against the sources manifest).
    pub scheme: String,
    /// Specific channels/bands this function reads (e.g. ["B04","B08"]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channels: Option<Vec<String>>,
    /// Source's natural tempo class.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tempo: Option<String>,
}

/// A function-registry entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Function {
    /// Function key including version, e.g. `"nv.l2a@1"`.
    pub key: String,
    /// Function kind.
    pub kind: FnKind,
    /// Output band key (must exist in the band registry).
    pub out_band: String,
    /// Index within the band's dims if the band has > 1 dim and this fn fills only one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_index: Option<u16>,
    /// Output unit description (free-form; informational).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_unit: Option<String>,
    /// Required upstream sources (Primary / Negative kinds).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<SourceRequirement>,
    /// For Derivative kind: required number of parent fact CIDs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parents_required: Option<u32>,
    /// For Derivative kind: minimum parent count for "any-N" ops like trend.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parents_min: Option<u32>,
    /// For Derivative kind: operator name (delta|mean|trend|rate|anomaly).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op: Option<String>,
    /// Human-readable formula or algorithm description.
    pub formula: String,
    /// Determinism guarantee. MUST be true for canonical-channel functions.
    pub deterministic: bool,
    /// For Negative kind: template for the reason CID's source pointer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_template: Option<String>,
}

/// The functions manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionRegistry {
    /// MUST equal `"emem-functions"`.
    pub manifest: String,
    /// Version, e.g. `"v0"`.
    pub version: String,
    /// Function entries.
    pub functions: Vec<Function>,
    /// Editorial note.
    #[serde(default, rename = "_note", skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Manifest for FunctionRegistry {
    const KIND: &'static str = MANIFEST_FUNCTION_REG;

    fn validate(&self) -> Result<(), ManifestError> {
        if self.manifest != Self::KIND {
            return Err(ManifestError::WrongKind {
                expected: Self::KIND,
                actual: self.manifest.clone(),
            });
        }
        let mut seen: std::collections::HashSet<&str> = Default::default();
        for f in &self.functions {
            if !seen.insert(&f.key) {
                return Err(ManifestError::Invalid(format!(
                    "duplicate function key: {}",
                    f.key
                )));
            }
            if !f.deterministic {
                return Err(ManifestError::Invalid(format!(
                    "non-deterministic function {} not allowed in canonical channel",
                    f.key
                )));
            }
            match f.kind {
                FnKind::Primary | FnKind::Negative => {
                    if f.sources.is_empty() {
                        return Err(ManifestError::Invalid(format!(
                            "{} kind requires at least one source",
                            f.key
                        )));
                    }
                }
                FnKind::Derivative => {
                    if f.parents_required.is_none() && f.parents_min.is_none() {
                        return Err(ManifestError::Invalid(format!(
                            "derivative {} requires parents_required or parents_min",
                            f.key
                        )));
                    }
                }
            }
        }
        Ok(())
    }
}

impl FunctionRegistry {
    /// Embedded v0 default.
    pub fn parse_default() -> Result<Self, ManifestError> {
        Self::parse_json(FUNCTIONS_V0_JSON.as_bytes())
    }

    /// Look up a function by key (e.g. `"nv.l2a@1"`).
    pub fn lookup(&self, key: &str) -> Option<&Function> {
        self.functions.iter().find(|f| f.key == key)
    }
}

/// Process-wide cached default registry.
pub static DEFAULT: LazyLock<FunctionRegistry> = LazyLock::new(|| {
    FunctionRegistry::parse_default().expect("embedded functions-v0.json is malformed")
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_loads_and_validates() {
        let r = &*DEFAULT;
        assert!(r.functions.len() >= 17);
        assert!(r.lookup("nv.l2a@1").is_some());
        assert!(r.lookup("gt.slice@1").is_some());
        assert!(r.lookup("abs.s1.water@1").is_some());
    }
}
