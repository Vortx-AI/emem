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

/// Tamper-provenance class: how the intelligence in a band's values is
/// produced. This is the load-bearing distinction an agent needs to decide
/// how far to trust a fact: a value it can independently recompute from raw
/// satellite data (no human or model in the loop) versus one it can only
/// trust through a signature over a learned model or a human edit.
///
/// Unlike [`BandFamily`] (editorial), this is load-bearing: it is declared
/// per band in the content-addressed bands manifest, so it rides into every
/// receipt through the pinned `bands_cid` and a verifier can reproduce the
/// classification offline. See [`ProvenanceClass::is_deterministic`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceClass {
    /// A quantity FITTED from data, and re-runnable: reproducible from
    /// signed inputs plus a named estimator plus a seed.
    ///
    /// The gap between [`Self::DeterministicIndex`] and [`Self::ModelOutput`].
    /// An index is a closed formula over cited pixels and reproduces
    /// bit-for-bit; a model output came from trained weights and does not
    /// reproduce at all. A fitted threshold, a per-sensor background rate, an
    /// estimated operating point: none of those is a formula, and calling
    /// them model output throws away the one property worth publishing, which
    /// is that a third party holding the same signed inputs and the same
    /// estimator gets the same number.
    ///
    /// Argued for by dpwotikn on 2026-08-13, from a telescope pipeline whose
    /// background rate, one-sided-versus-two-sided choice and union-versus-
    /// single decision are each fitted and each exactly re-runnable:
    /// "registering the lot as model_output discards the one property worth
    /// publishing, which is re-runnability".
    ///
    /// A band in this class owes its estimator id, its input fact cids and its
    /// seed. Without those it is [`Self::ModelOutput`] wearing a better name,
    /// which is worse than the honest label.
    Estimator,
    /// Direct instrument reading, no algorithm and no model in the path
    /// (e.g. Copernicus DEM elevation, Sentinel-2 L2A reflectance,
    /// Sentinel-1 RTC backscatter, DMSP/VIIRS nightlights). Tamper-evident:
    /// the value is the sensor product itself.
    DirectSensor,
    /// A deterministic formula computed over raw satellite pixels or over
    /// fixed coordinates/time (e.g. NDVI/NDRE/NDMI from Sentinel-2 L2A).
    /// Recomputable from source: a party holding the cited source and
    /// applying the responder's documented formula, in the documented order
    /// and in f64, reproduces the value. The live members of this class use
    /// only IEEE-754 correctly-rounded operations, so that agreement is
    /// bit-exact. Two caveats travel with that. An algebraically equivalent
    /// rearrangement need not agree in the last bits: Sentinel-2 reflectance
    /// scaling (`DN * 1e-4`) is applied per band before the ratio, and
    /// folding it through the ratio instead shifts the result by a few ULP.
    /// And the cited source is pinned by URL, id and capture time, not by a
    /// content hash, so this is a claim about the formula and not a guarantee
    /// that the upstream bytes are unchanged. No learned weights, no human
    /// edit.
    DeterministicIndex,
    /// A direct device reading admitted through a **verified OS execution
    /// trace** (`emem.os_trace.v1`), from a device key **attested to a
    /// whitelisted platform** (`emem-device-platforms`). Tamper-evident
    /// through verified execution rather than recomputation: a third party
    /// cannot re-derive a one-time physical reading, but can verify that
    /// the trace's merkle root and device signature bind the emitted
    /// output, and that a whitelisted trust anchor endorsed the device.
    /// This is the device-substrate analogue of [`Self::DirectSensor`],
    /// which is reserved for values recomputable from a public archive.
    /// The founding Earth substrate stays `direct_sensor`; a robot,
    /// telescope, microscope, or operator satellite writes
    /// `attested_execution`.
    AttestedExecution,
    /// Output of a trained model or a published ML / statistical product
    /// (e.g. Tessera / Clay / Prithvi / Galileo embeddings, ESA WorldCover,
    /// SoilGrids, WorldPop, GHSL, Hansen, CHIRPS, forecast blends). A model,
    /// not a fixed formula, produced the value; trust rests on a signed
    /// model checkpoint, not on recomputation from source.
    ModelOutput,
    /// Human-curated vector or authored boundary data, or a deterministic
    /// derivation whose inputs are such data (e.g. OpenStreetMap, Overture,
    /// WDPA, GADM admin boundaries, WWF ecoregions, road/river topology).
    /// A human placed or drew the underlying geometry.
    HumanCurated,
    /// Not yet classified. Treated as the lowest trust and never claims
    /// tamper-evidence. Fail-safe default for a band that omits the field
    /// (e.g. a zero-padded reservation slot), so a forgotten tag can never
    /// over-claim that a value is reproducible from raw satellite data.
    Unclassified,
}

impl ProvenanceClass {
    /// Whether a third party can recompute the value from the cited raw
    /// satellite source (or fixed coordinates/time), with no learned model
    /// and no human edit in the loop. "Recompute" means applying the
    /// responder's documented formula in the documented order and precision;
    /// it does not promise that an algebraically equivalent rearrangement
    /// agrees in the last bits, and it does not attest that the cited source
    /// still holds the bytes that were read. This is the core tamper-evidence
    /// property: `true` for [`Self::DirectSensor`] and
    /// [`Self::DeterministicIndex`], `false` for the rest.
    pub fn is_deterministic(self) -> bool {
        matches!(self, Self::DirectSensor | Self::DeterministicIndex)
    }

    /// What a verifier leans on to trust the value:
    /// `recomputable_from_source` (deterministic), `signed_model_checkpoint`
    /// (model output, where the responder hashes the checkpoint into the
    /// receipt), or `attester_only` (human-curated or unclassified: you
    /// trust the signer and the cited source).
    pub fn tamper_evidence(self) -> &'static str {
        match self {
            Self::DirectSensor | Self::DeterministicIndex => "recomputable_from_source",
            // Re-runnable rather than recomputable: same inputs and same
            // estimator give the same answer, but you need the estimator, not
            // just the formula and the source.
            Self::Estimator => "rerunnable_from_signed_inputs",
            Self::AttestedExecution => "verified_execution_trace",
            Self::ModelOutput => "signed_model_checkpoint",
            Self::HumanCurated | Self::Unclassified => "attester_only",
        }
    }

    /// Monotonic rank for the "promote tamper-proof first" ordering. Higher
    /// is more independently verifiable. The two recomputable classes rank
    /// highest (any party re-derives them from a cited public source);
    /// attested execution is tamper-evident but not recomputable (a
    /// one-time physical reading, trusted through its verified trace); a
    /// model checkpoint and human curation carry no execution evidence.
    pub fn trust_rank(self) -> u8 {
        match self {
            Self::DirectSensor => 5,
            Self::DeterministicIndex => 4,
            // Below a closed formula, above a device reading: it reproduces,
            // but only for someone who also holds the estimator and the seed.
            Self::Estimator => 3,
            Self::AttestedExecution => 3,
            Self::ModelOutput => 2,
            Self::HumanCurated => 1,
            Self::Unclassified => 0,
        }
    }

    /// Stable wire string (matches the `snake_case` serde form).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DirectSensor => "direct_sensor",
            Self::DeterministicIndex => "deterministic_index",
            Self::Estimator => "estimator",
            Self::AttestedExecution => "attested_execution",
            Self::ModelOutput => "model_output",
            Self::HumanCurated => "human_curated",
            Self::Unclassified => "unclassified",
        }
    }

    /// In-band caution for the classes an agent must not treat as ground
    /// truth. A signature proves the bytes were not altered after signing;
    /// it says nothing about systematic error upstream of the signature.
    /// `None` for the deterministic classes (their failure mode is the
    /// cited source itself, which any party can re-read). Surfaced verbatim
    /// on every provenance block so the warning travels with the value
    /// instead of living in documentation an agent may never read.
    pub fn caution(self) -> Option<&'static str> {
        match self {
            Self::DirectSensor | Self::DeterministicIndex => None,
            Self::Estimator => Some(
                "fitted quantity: re-runnable, not recomputable. The same signed \
                 inputs under the same named estimator and seed reproduce it \
                 exactly, so a third party can re-run rather than trust it, but \
                 the FIT itself carries the choices of whoever made it: the \
                 basis it was fitted on, and whether it was validated \
                 out-of-sample. Check the estimator's held-out score before \
                 treating the number as general.",
            ),
            Self::AttestedExecution => Some(
                "device reading: trusted through its verified OS execution trace \
                 and the device's platform attestation, not through recomputation. \
                 A genuine but miscalibrated or spoofed sensor still produces a \
                 signed trace; corroborate against the recomputable drift anchor \
                 where bands overlap.",
            ),
            Self::ModelOutput => Some(
                "model output: a learned representation, not a measurement. \
                 Subject to training bias, domain shift, and encoder revision; \
                 treat as a hypothesis and corroborate against a deterministic \
                 band before load-bearing use.",
            ),
            Self::HumanCurated => Some(
                "human-curated source: subject to editorial choices, mapping \
                 conventions, and coverage gaps that vary by region.",
            ),
            Self::Unclassified => Some(
                "unclassified band: production method not yet declared; \
                 lowest trust, never claims tamper-evidence.",
            ),
        }
    }
}

/// Fail-safe default when a manifest band omits `provenance_class`: the
/// lowest-trust class, so an unclassified band never claims tamper-evidence.
fn default_provenance_class() -> ProvenanceClass {
    ProvenanceClass::Unclassified
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
    /// Tamper-provenance class: how this band's values are produced (direct
    /// sensor read, deterministic index over raw pixels, trained-model
    /// output, or human-curated). Load-bearing, unlike `family`: it rides
    /// into every receipt via the content-addressed `bands_cid`, so a
    /// verifier can filter tamper-evident facts (`direct_sensor`,
    /// `deterministic_index`) from model / human ones offline. Defaults to
    /// `unclassified` (lowest trust, never claims tamper-evidence).
    #[serde(default = "default_provenance_class")]
    pub provenance_class: ProvenanceClass,
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
    /// Units for individual `scalar_keys`, where the band-wide `units`
    /// string describes several of them at once.
    ///
    /// Ten cube bands pair one `units` string with several scalars, so 32
    /// scalar bands reported a unit that was not theirs:
    /// `copdem30m.elevation_mean` answered
    /// `"metres (elevation), degrees (slope), unitless (sin/cos)"`, and an
    /// agent reading that cannot tell what the number is in. The
    /// `dimensions[]` overlay already solves this where it is populated, but
    /// it requires a slot `index` this registry does not record for these
    /// bands, and inventing one would publish a worse claim than the one
    /// being fixed. This carries only what is actually known: the unit of a
    /// named scalar.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub scalar_units: std::collections::BTreeMap<String, String>,
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

    /// Tamper-provenance class for any band key an agent can cite,
    /// including dotted materializer scalars ("copdem30m.elevation_mean")
    /// that ride atop a cube band (resolved through [`cube_band_alias`]).
    /// Unknown keys fall back to [`ProvenanceClass::Unclassified`], the
    /// lowest-trust class, so an unrecognised band never claims
    /// tamper-evidence.
    pub fn provenance_class_for(&self, band_key: &str) -> ProvenanceClass {
        self.lookup(cube_band_alias(band_key))
            .or_else(|| self.lookup(band_key))
            .map(|b| b.provenance_class)
            .unwrap_or(ProvenanceClass::Unclassified)
    }
}

/// Map a dotted materializer band key ("esa_worldcover.lc_2021",
/// "copdem30m.elevation_mean") to the cube band it rides atop, so registry
/// lookups (provenance class, editorial metadata) resolve for scalar keys
/// that don't appear directly in the manifest. Keys with no alias pass
/// through unchanged. This is the inverse of the family-alias map
/// `/v1/coverage_matrix` derives from `bands-v0.json` `scalar_keys` — both
/// views must stay in sync when adding a new scalar to a family band.
pub fn cube_band_alias(band_key: &str) -> &str {
    match band_key {
        // Cop-DEM scalars → `cop_dem` slot.
        b if b.starts_with("copdem30m.") => "cop_dem",
        // GMRT bathymetry rides on the legacy `dem` slot today.
        "gmrt.topobathy_mean" => "dem",
        // ESA WorldCover scalar → `landcover` slot.
        "esa_worldcover.lc_2021" => "landcover",
        // Indices.* and s2.* → their respective family slots.
        b if b.starts_with("indices.") => "indices",
        b if b.starts_with("s2.") => "sentinel2_raw",
        b if b.starts_with("geotessera") => "geotessera",
        b if b.starts_with("overture.") => "overture",
        b if b.starts_with("weather.") => "climate",
        b if b.starts_with("surface_water.") => "surface_water",
        b if b.starts_with("hansen.") => "forest_change",
        // Canonical `forest_change.*` sub-bands resolve directly to the
        // `forest_change` cube slot — kept here next to the legacy
        // `hansen.*` arm so adding either form picks up the family
        // editorial fields (description / interpretation / …).
        b if b.starts_with("forest_change.") => "forest_change",
        // MODIS NDVI is an optical index → inherit `indices` editorial fields.
        // MODIS LST is land-surface temperature in Kelvin — it must NOT inherit
        // the `indices` (-1..1 unitless) metadata (that mislabelled LST 302 K as
        // a unitless index). It falls through to band_key, so the response shows
        // no inherited cube description rather than a wrong one; the fact itself
        // already carries the correct unit:"K".
        b if b.starts_with("modis.ndvi") => "indices",
        b if b.starts_with("soilgrids.") => "soilgrids",
        // CAMS scalars now have a dedicated `air_quality` band entry
        // (carved 7 dims off the front of `_reserved_512` in
        // bands-v0.json on 2026-05-04). Per-pollutant units +
        // value_range live in dimensions[]; the dimension lookup
        // in the API layer pulls the right one by name.
        b if b.starts_with("cams.") => "air_quality",
        // Everything else whose prefix IS the cube band key. The table above
        // exists for scalars whose prefix does NOT match their band
        // ("copdem30m." rides "cop_dem", "hansen." rides "forest_change"),
        // and every band whose scalars share its own name was simply left
        // out. That silently cost 41 scalar keys all of their registry
        // metadata: no units, no description, no pitfalls, and worst,
        // provenance_class_for fell through to `Unclassified`, the
        // lowest-trust class. jrc_gfc2020.forest_2020, the EUDR forest flag,
        // was one of them, and so was every RADD and OPERA alert.
        //
        // The caller looks the result up in the registry and falls back to
        // the full key if it misses, so returning a prefix that is not a band
        // is harmless.
        b => b.split_once('.').map(|(prefix, _)| prefix).unwrap_or(b),
    }
}

/// Process-wide cached default registry. Loaded once on first access.
pub static DEFAULT: LazyLock<BandRegistry> =
    LazyLock::new(|| BandRegistry::parse_default().expect("embedded bands-v0.json is malformed"));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provenance_class_resolves_through_cube_band_alias() {
        let r = &*DEFAULT;
        // Dotted materializer scalars resolve to their cube band's class.
        assert_eq!(
            r.provenance_class_for("copdem30m.elevation_mean"),
            ProvenanceClass::DirectSensor
        );
        assert_eq!(
            r.provenance_class_for("indices.ndvi"),
            ProvenanceClass::DeterministicIndex
        );
        // Direct cube keys resolve as themselves.
        assert_eq!(
            r.provenance_class_for("geotessera"),
            ProvenanceClass::ModelOutput
        );
        assert_eq!(
            r.provenance_class_for("overture.roads_km"),
            ProvenanceClass::HumanCurated
        );
        // Unknown keys never claim tamper-evidence.
        let unknown = r.provenance_class_for("no_such_band.at_all");
        assert_eq!(unknown, ProvenanceClass::Unclassified);
        assert!(!unknown.is_deterministic());
    }

    #[test]
    fn attested_execution_is_tamper_evident_but_not_recomputable() {
        let p = ProvenanceClass::AttestedExecution;
        // Trusted through the verified trace, not by re-derivation: it must
        // NOT claim the recomputable-from-source property, or a device
        // reading would masquerade as an archive value an agent can re-fetch.
        assert!(!p.is_deterministic());
        assert_eq!(p.tamper_evidence(), "verified_execution_trace");
        assert_eq!(p.as_str(), "attested_execution");
        assert!(p.caution().is_some());
        // Ranks below the two recomputable classes, above model / human.
        assert!(p.trust_rank() < ProvenanceClass::DeterministicIndex.trust_rank());
        assert!(p.trust_rank() > ProvenanceClass::ModelOutput.trust_rank());
        // The wire string round-trips through serde.
        let j = serde_json::to_string(&p).unwrap();
        assert_eq!(j, "\"attested_execution\"");
        assert_eq!(
            serde_json::from_str::<ProvenanceClass>(&j).unwrap(),
            ProvenanceClass::AttestedExecution
        );
    }

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
        // 2026-05-16 (later still): reserved (95 dims at 1697) carved
        // again to add gmrt (4 dims at 1697, topobathy mean/min/max/std
        // for the wave_solve / coastal_proximity / flood_risk@2
        // algorithms) plus a slimmed reserved (91 dims at 1701). Count
        // rose 40 → 41. Σ stays at 1792.
        // 2026-05-28: reserved (91 dims at 1701) carved further to add
        // galileo (48 dims at 1701, NASA Harvest Galileo Base/Tiny
        // foundation embedding via GPU sidecar) plus a slimmed reserved
        // (43 dims at 1749). Count rose 41 → 42. Σ stays at 1792.
        // 2026-06-01: reserved (43 dims at 1749) carved to add opera_dist
        // (2 dims at 1749, NASA OPERA DIST-ALERT NRT disturbance —
        // veg_dist_status + veg_dist_date, optional EDL-gated) plus a
        // slimmed reserved (41 dims at 1751). Count rose 42 → 43. Σ=1792.
        assert_eq!(r.bands.len(), 43);
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
        // 2026-05-16 (later still): gmrt carves 4 dims at 1697 for the
        // topo-bathymetric mean/min/max/std scalars; reserved drops
        // from 95 → 91 dims at 1701. Subsequent offsets stay byte-stable.
        assert_eq!(idx["gmrt"].offset, 1697);
        assert_eq!(idx["gmrt"].dims, 4);
        // 2026-05-28: galileo (NASA Harvest Galileo) carves 48 dims at
        // offset 1701 from the reserved tail to add the third live
        // GPU-sidecar foundation embedding (alongside clay_v1 and
        // prithvi_eo2). Reserved drops from 91 → 43 dims at 1749 with
        // total_dims = 1792 unchanged.
        assert_eq!(idx["galileo"].offset, 1701);
        assert_eq!(idx["galileo"].dims, 48);
        // 2026-06-01: opera_dist (NASA OPERA DIST-ALERT NRT) carves 2 dims
        // at offset 1749 from the reserved tail; reserved drops 43 → 41 at
        // offset 1751 with total_dims = 1792 unchanged.
        assert_eq!(idx["opera_dist"].offset, 1749);
        assert_eq!(idx["opera_dist"].dims, 2);
        assert_eq!(idx["reserved"].offset, 1751);
        assert_eq!(idx["reserved"].dims, 41);
    }
}

#[cfg(test)]
mod provenance_vocabulary_tests {
    use super::*;

    /// Adding a provenance class must not move `bands_cid`.
    ///
    /// I told a contributor that adding one "moves a content-addressed
    /// manifest that every signature commits to", and therefore that the
    /// blocker was versioning. Measured, that is wrong: the cid is
    /// blake3(canonical_cbor(the registry DATA)), an unused variant appears
    /// nowhere in the serialization, and the cid before and after adding
    /// `Estimator` is byte-identical. The real cost was four exhaustive match
    /// arms in this file.
    ///
    /// That was the third time in one session I stated a cost from the shape
    /// of the design instead of measuring it, so this pins the property rather
    /// than the belief. If a future class DOES move the cid, that is a real
    /// wire change and this test is where it gets noticed, before the claim
    /// gets made again.
    #[test]
    fn the_bands_manifest_cid_is_unchanged_by_the_class_vocabulary() {
        assert_eq!(
            crate::manifest_cid(&*DEFAULT).expect("bands manifest serializes"),
            "mesoyti3qcs22pftcq27ljwcf3ifktnkqid4euqxyskhea5rpgra",
            "bands_cid moved. Every receipt commits to this value, so a change \
             here breaks verification for facts already signed. If a band's \
             provenance_class genuinely changed, that is a wire change and needs \
             a version, not a new constant here."
        );
    }

    /// `estimator` is declarable and no shipped band claims it yet.
    ///
    /// The class exists because a fitted-but-re-runnable quantity is neither a
    /// closed formula nor a trained model. Nothing in the Earth corpus is one,
    /// so adopting it now would be relabelling facts to fit a new word. It is
    /// vocabulary a contributor can declare, not a reclassification.
    #[test]
    fn estimator_is_available_and_unclaimed() {
        assert_eq!(ProvenanceClass::Estimator.as_str(), "estimator");
        assert!(
            ProvenanceClass::Estimator.caution().is_some(),
            "a fitted class owes a caution"
        );
        assert_eq!(
            ProvenanceClass::Estimator.tamper_evidence(),
            "rerunnable_from_signed_inputs"
        );
        assert!(
            DEFAULT
                .bands
                .iter()
                .all(|b| b.provenance_class != ProvenanceClass::Estimator),
            "no shipped band should have been relabelled into the new class"
        );
    }
}
