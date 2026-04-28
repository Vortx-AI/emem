//! Band ontology — loaded from the **content-addressed bands manifest**.
//!
//! Spec §4. The 1792D layout is NOT a Rust constant — it is data. The default
//! manifest is embedded via `include_str!` for bootstrap convenience, but any
//! deployment can swap in a different manifest CID and hot-swap the ontology
//! without recompiling.

use std::collections::HashMap;
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

use crate::manifest::{Manifest, ManifestError, MANIFEST_BAND_ONTOLOGY};
use crate::privacy::PrivacyClass;
use crate::tslot::Tempo;

/// Embedded default v0 manifest. Operators may override at runtime.
const BANDS_V0_JSON: &str = include_str!("../data/bands-v0.json");

/// Family classification for a band. Editorial; not load-bearing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BandFamily {
    Foundation, Optical, Radar, Terrain, Climate, Soil, Vegetation,
    Landcover, Water, Human, Vision, Topology, Encoding, Reserved,
}

/// A single band record loaded from the manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Band {
    /// Stable wire-format band key (e.g. `"geotessera"`, `"indices"`).
    pub key: String,
    /// Editorial family.
    pub family: BandFamily,
    /// Offset within the 1792D layout.
    pub offset: u16,
    /// Number of dimensions this band occupies.
    pub dims: u16,
    /// Tempo class.
    pub tempo: Tempo,
    /// Privacy class.
    pub privacy: PrivacyClass,
}

/// The full band-ontology manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BandRegistry {
    /// Manifest discriminator, MUST equal `"emem-bands"`.
    pub manifest: String,
    /// Manifest version, e.g. `"v0"`.
    pub version: String,
    /// Total dimensions; bands MUST sum to this.
    pub total_dims: u16,
    /// Permitted tempo class strings.
    pub tempo_classes: Vec<String>,
    /// Permitted privacy class strings.
    pub privacy_classes: Vec<String>,
    /// The bands themselves, in physical-layout order.
    pub bands: Vec<Band>,
    /// Optional editorial note.
    #[serde(default, rename = "_note", skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Manifest for BandRegistry {
    const KIND: &'static str = MANIFEST_BAND_ONTOLOGY;

    fn validate(&self) -> Result<(), ManifestError> {
        if self.manifest != Self::KIND {
            return Err(ManifestError::WrongKind {
                expected: Self::KIND,
                actual: self.manifest.clone(),
            });
        }
        let mut expected_offset: u16 = 0;
        let mut keys: HashMap<&str, ()> = HashMap::with_capacity(self.bands.len());
        for b in &self.bands {
            if b.offset != expected_offset {
                return Err(ManifestError::Invalid(format!(
                    "band '{}' offset {} != expected {}", b.key, b.offset, expected_offset)));
            }
            if keys.insert(&b.key, ()).is_some() {
                return Err(ManifestError::Invalid(format!("duplicate band key: {}", b.key)));
            }
            expected_offset = expected_offset
                .checked_add(b.dims)
                .ok_or_else(|| ManifestError::Invalid("dims overflow".into()))?;
        }
        if expected_offset != self.total_dims {
            return Err(ManifestError::Invalid(format!(
                "bands sum to {} dims, expected {}", expected_offset, self.total_dims)));
        }
        Ok(())
    }
}

impl BandRegistry {
    /// The embedded v0 default. Most callers should use [`default`].
    pub fn parse_default() -> Result<Self, ManifestError> {
        Self::parse_json(BANDS_V0_JSON.as_bytes())
    }

    /// Look up a band by key. O(n) but only called from hot paths via the
    /// lazy index in [`default`].
    pub fn lookup(&self, key: &str) -> Option<&Band> {
        self.bands.iter().find(|b| b.key == key)
    }

    /// Indexed view for O(1) key lookup. Returns a HashMap built once.
    pub fn key_index(&self) -> HashMap<&str, &Band> {
        self.bands.iter().map(|b| (b.key.as_str(), b)).collect()
    }
}

/// Process-wide cached default registry. Loaded once on first access.
pub static DEFAULT: LazyLock<BandRegistry> = LazyLock::new(|| {
    BandRegistry::parse_default().expect("embedded bands-v0.json is malformed")
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_loads_and_validates() {
        let r = &*DEFAULT;
        assert_eq!(r.total_dims, 1792);
        assert_eq!(r.bands.len(), 33);
    }

    #[test]
    fn key_lookup_finds_known_bands() {
        let r = &*DEFAULT;
        for k in &["geotessera", "overture", "_reserved_512", "sentinel2_raw", "indices",
                   "dem", "landcover", "koppen", "soilgrids", "reserved"] {
            assert!(r.lookup(k).is_some(), "missing band: {k}");
        }
    }

    #[test]
    fn matches_canonical_offsets() {
        let r = &*DEFAULT;
        let idx = r.key_index();
        assert_eq!(idx["geotessera"].offset,    0);
        assert_eq!(idx["overture"].offset,      128);
        assert_eq!(idx["overture"].dims,        64);
        assert_eq!(idx["_reserved_512"].offset, 192);
        assert_eq!(idx["_reserved_512"].dims,   512);
        assert_eq!(idx["sentinel2_raw"].offset, 704);
        assert_eq!(idx["sam3_visual"].offset,   894);
        assert_eq!(idx["qwen_visual"].offset,   1086);
        assert_eq!(idx["reserved"].offset,      1672);
    }
}
