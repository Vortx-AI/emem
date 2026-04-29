//! Source-connector registry — loaded from the **content-addressed sources
//! manifest**. Operators may publish their own sources manifest CID with
//! mirrors, auth, rate limits — schemes are stable across manifests, URLs
//! are not.

use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

use crate::manifest::{Manifest, ManifestError, MANIFEST_SOURCE_REG};

const SOURCES_V0_JSON: &str = include_str!("../data/sources-v0.json");

/// Provider connector kind. Determines which fetch backend handles the URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorKind {
    /// GCS Cloud-Optimized GeoTIFF (gs://).
    GcsCog,
    /// HTTPS Cloud-Optimized GeoTIFF via vsicurl-style range reads.
    HttpsCogVsicurl,
    /// Plain HTTPS GeoTIFF download.
    HttpsGeotiff,
    /// IPLD content-addressed bundle (no external fetch).
    IpldCid,
    /// Microsoft Planetary Computer STAC API (anonymous; signed item URLs).
    StacPc,
}

/// One provider for a source scheme.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provider {
    /// Provider ID (e.g. `"gcs.public"`, `"aws.opendata"`).
    pub id: String,
    /// Connector kind.
    pub kind: ConnectorKind,
    /// URL template with `{variable}` interpolation. Variables are
    /// resolved by `emem-fetch` per request (cell64, year, month, tile_id, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url_template: Option<String>,
    /// Static IPLD CID, when `kind = IpldCid`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cid: Option<String>,
    /// Auth scheme: `"anonymous"`, `"earthdata_login"`, `"oauth2"`, etc.
    #[serde(default = "default_auth")]
    pub auth: String,
    /// Soft rate limit hint, queries per second.
    #[serde(default = "default_rate_limit")]
    pub rate_limit_qps: u32,
    /// Licence string.
    pub license: String,
}

fn default_auth() -> String {
    "anonymous".into()
}
fn default_rate_limit() -> u32 {
    50
}

/// One source scheme entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceScheme {
    /// Scheme key (matches function-registry `SourceRequirement.scheme`).
    pub scheme: String,
    /// Ordered list of providers (operators may try in order, fail over).
    pub providers: Vec<Provider>,
    /// Source's natural tempo class.
    pub tempo: String,
    /// Native resolution in meters.
    pub native_resolution_m: u32,
}

/// Sources manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceRegistry {
    /// MUST equal `"emem-sources"`.
    pub manifest: String,
    /// Version, e.g. `"v0"`.
    pub version: String,
    /// Source scheme entries.
    pub sources: Vec<SourceScheme>,
    /// Editorial note.
    #[serde(default, rename = "_note", skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Manifest for SourceRegistry {
    const KIND: &'static str = MANIFEST_SOURCE_REG;

    fn validate(&self) -> Result<(), ManifestError> {
        if self.manifest != Self::KIND {
            return Err(ManifestError::WrongKind {
                expected: Self::KIND,
                actual: self.manifest.clone(),
            });
        }
        let mut seen: std::collections::HashSet<&str> = Default::default();
        for s in &self.sources {
            if !seen.insert(&s.scheme) {
                return Err(ManifestError::Invalid(format!(
                    "duplicate source scheme: {}",
                    s.scheme
                )));
            }
            if s.providers.is_empty() {
                return Err(ManifestError::Invalid(format!(
                    "source {} has no providers",
                    s.scheme
                )));
            }
        }
        Ok(())
    }
}

impl SourceRegistry {
    /// Embedded v0 default.
    pub fn parse_default() -> Result<Self, ManifestError> {
        Self::parse_json(SOURCES_V0_JSON.as_bytes())
    }

    /// Look up a source scheme.
    pub fn lookup(&self, scheme: &str) -> Option<&SourceScheme> {
        self.sources.iter().find(|s| s.scheme == scheme)
    }
}

/// Process-wide cached default registry.
pub static DEFAULT: LazyLock<SourceRegistry> = LazyLock::new(|| {
    SourceRegistry::parse_default().expect("embedded sources-v0.json is malformed")
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_loads() {
        let r = &*DEFAULT;
        assert!(r.lookup("sentinel2.l2a").is_some());
        assert!(r.lookup("geotessera.v1").is_some());
    }
}
