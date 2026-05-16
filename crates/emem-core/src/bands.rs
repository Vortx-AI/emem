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
    /// Cross-domain identity / addressing (e.g. cell64, tslot framing).
    Foundation,
    /// Visible / NIR / SWIR optical reflectance (Sentinel-2, MODIS, Landsat).
    Optical,
    /// Synthetic-aperture radar (Sentinel-1 RTC, ALOS).
    Radar,
    /// Elevation, slope, aspect, and other DEM-derived surfaces.
    Terrain,
    /// Atmospheric / climatic state (Open-Meteo, MET Norway).
    Climate,
    /// Soil composition, moisture, and properties.
    Soil,
    /// Vegetation indices and phenology (NDVI, LAI, FPAR).
    Vegetation,
    /// Land-cover classification (ESA WorldCover, JRC, Hansen).
    Landcover,
    /// Surface and ground water occurrence (JRC GSW, hydrography).
    Water,
    /// Human-built environment (population, OSM, Overture).
    Human,
    /// Learned visual embeddings (Tessera, Clay, Prithvi).
    Vision,
    /// Network / topology relations (graphs, adjacency).
    Topology,
    /// Compression / encoding-only metadata.
    Encoding,
    /// Reserved for future families; not used by current ontology.
    Reserved,
}

/// One named scalar slot inside a multi-dim band (e.g. `indices` carries
/// NDVI, NDRE, NDMI as three named slots). Editorial — present only on
/// bands where individual dimensions have distinct semantic meaning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BandDimension {
    /// 0-indexed offset inside the band (NOT inside the 1792D cube).
    pub index: u16,
    /// Short stable name for this slot (e.g. `"ndvi"`, `"elevation_mean"`).
    pub name: String,
    /// One-line description of what this scalar represents.
    pub description: String,
    /// Physical units (e.g. `"unitless"`, `"meters"`, `"dB"`, `"percent"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub units: Option<String>,
    /// Inclusive admissible range as `[min, max]`. Use `null` on either
    /// side for half-open. Editorial — meant for sanity-checking values
    /// that came back from `/v1/recall`, not for hard validation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_range: Option<serde_json::Value>,
    /// Optional plain-math formula (e.g. `"NDVI = (B08 − B04) / (B08 + B04)"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formula: Option<String>,
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
    /// One-paragraph editorial description of what the band carries and
    /// why an agent would pull it. Optional so the legacy short-form
    /// manifest stays valid; populated progressively per band.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Physical units when the band is a scalar (vector bands list units
    /// per `dimensions[]` instead).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub units: Option<String>,
    /// Inclusive admissible range for scalar bands (`[min, max]`, `null`
    /// for half-open).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_range: Option<serde_json::Value>,
    /// Editorial guide on how to read the value (e.g. NDVI thresholds for
    /// bare soil / vegetation / dense canopy).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interpretation: Option<String>,
    /// Common gotchas an agent should know before relying on the band
    /// (cloud contamination, snow seasonality, etc).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pitfalls: Option<String>,
    /// Citations / canonical doc URLs (newline-joined string for compact
    /// JSON readability).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub references: Option<String>,
    /// Per-slot breakdown for multi-dim bands. Optional — when present,
    /// `dimensions.len()` SHOULD equal `dims`. Lets `/v1/bands` answer
    /// "what is dimension 1 of `indices`?" without external docs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dimensions: Vec<BandDimension>,
    /// Materializer scalar names that read from this cube band, in
    /// `family.field` form (e.g. `["indices.ndvi", "indices.ndre",
    /// "indices.ndmi"]`). Lets an agent jump from `/v1/bands` straight to
    /// the dotted keys it can use in `/v1/recall` without consulting
    /// `/v1/materializers` separately.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scalar_keys: Vec<String>,
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
                    "band '{}' offset {} != expected {}",
                    b.key, b.offset, expected_offset
                )));
            }
            if keys.insert(&b.key, ()).is_some() {
                return Err(ManifestError::Invalid(format!(
                    "duplicate band key: {}",
                    b.key
                )));
            }
            expected_offset = expected_offset
                .checked_add(b.dims)
                .ok_or_else(|| ManifestError::Invalid("dims overflow".into()))?;
        }
        if expected_offset != self.total_dims {
            return Err(ManifestError::Invalid(format!(
                "bands sum to {} dims, expected {}",
                expected_offset, self.total_dims
            )));
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
pub static DEFAULT: LazyLock<BandRegistry> =
    LazyLock::new(|| BandRegistry::parse_default().expect("embedded bands-v0.json is malformed"));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_loads_and_validates() {
        let r = &*DEFAULT;
        assert_eq!(r.total_dims, 1792);
        // 33 cube bands + chirps.precip_daily_mm (offset 1672) = 34.
        // Adding new tail bands shifts `reserved` forward and decrements
        // its dims by the same total so Σ stays at 1792.
        // 2026-05-10: sam3_visual + qwen_visual (192+192=384 dims at
        // offset 894) reclaimed for prithvi_eo2 (384 dims at 894); count
        // dropped 35 → 34 because two placeholder bands collapse into one
        // live foundation band. terrain_derived offset stays 1278.
        // 2026-05-11: _reserved_512 (505 dims at 199) split into
        // clay_v1 (384 dims at 199) + _reserved_128 (121 dims at 583);
        // count rose 34 → 35 because one slot split into two. Subsequent
        // band offsets stay byte-stable.
        // 2026-05-16: reserved (119 dims at 1673) carved into four EUDR
        // bands (jrc_gfc2020, jrc_tmf, radd, wri_gdm; total 20 dims) plus
        // a slimmed reserved (99 dims at 1693). Count rose 35 → 39 because
        // one reserved slot split into five. Σ stays at 1792.
        // 2026-05-16 (later): reserved (99 dims at 1693) carved further to
        // add esa_cci_biomass (4 dims at 1693, AGB + AGB_SD at the 2022 +
        // 2020 epochs as addressable scalars) plus a slimmed reserved
        // (95 dims at 1697). Count rose 39 → 40. Σ stays at 1792.
        assert_eq!(r.bands.len(), 40);
    }

    #[test]
    fn key_lookup_finds_known_bands() {
        let r = &*DEFAULT;
        for k in &[
            "geotessera",
            "overture",
            "clay_v1",
            "_reserved_128",
            "sentinel2_raw",
            "indices",
            "dem",
            "landcover",
            "koppen",
            "soilgrids",
            "reserved",
        ] {
            assert!(r.lookup(k).is_some(), "missing band: {k}");
        }
    }

    #[test]
    fn matches_canonical_offsets() {
        let r = &*DEFAULT;
        let idx = r.key_index();
        assert_eq!(idx["geotessera"].offset, 0);
        assert_eq!(idx["overture"].offset, 128);
        assert_eq!(idx["overture"].dims, 64);
        assert_eq!(idx["air_quality"].offset, 192);
        assert_eq!(idx["air_quality"].dims, 7);
        // 2026-05-11: clay_v1 (384 dims at offset 199) carved out of
        // the historic _reserved_512 (505 dims) → _reserved_128
        // (121 dims at offset 583). Subsequent band offsets stay
        // byte-stable: clay_v1 + _reserved_128 = 384 + 121 = 505.
        assert_eq!(idx["clay_v1"].offset, 199);
        assert_eq!(idx["clay_v1"].dims, 384);
        assert_eq!(idx["_reserved_128"].offset, 583);
        assert_eq!(idx["_reserved_128"].dims, 121);
        assert_eq!(idx["sentinel2_raw"].offset, 704);
        // sam3_visual + qwen_visual placeholders (192+192=384 dims at 894)
        // were reclaimed for prithvi_eo2 (384 dims at 894). Subsequent
        // band offsets stay byte-stable because 192+192 = 384.
        assert_eq!(idx["prithvi_eo2"].offset, 894);
        assert_eq!(idx["prithvi_eo2"].dims, 384);
        // chirps.precip_daily_mm sits at the head of the new tail block;
        // reserved was 1672 → moved to 1673 (one slot taken).
        assert_eq!(idx["chirps.precip_daily_mm"].offset, 1672);
        assert_eq!(idx["chirps.precip_daily_mm"].dims, 1);
        // 2026-05-16: reserved (119 dims at 1673) split into four EUDR
        // bands (20 dims total) + slimmed reserved (99 dims at 1693).
        assert_eq!(idx["jrc_gfc2020"].offset, 1673);
        assert_eq!(idx["jrc_gfc2020"].dims, 4);
        assert_eq!(idx["jrc_tmf"].offset, 1677);
        assert_eq!(idx["jrc_tmf"].dims, 8);
        assert_eq!(idx["radd"].offset, 1685);
        assert_eq!(idx["radd"].dims, 4);
        assert_eq!(idx["wri_gdm"].offset, 1689);
        assert_eq!(idx["wri_gdm"].dims, 4);
        // 2026-05-16 (later): esa_cci_biomass carves 4 dims at 1693 from
        // the reserved tail; reserved drops from 99→95 dims at 1697.
        assert_eq!(idx["esa_cci_biomass"].offset, 1693);
        assert_eq!(idx["esa_cci_biomass"].dims, 4);
        assert_eq!(idx["reserved"].offset, 1697);
        assert_eq!(idx["reserved"].dims, 95);
    }
}
