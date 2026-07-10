//! Entity registry: first-class, content-addressed, signed references to
//! real-world objects. The substrate primitive that gives agents a shared
//! *identity* to co-refer to: an object (a bridge, a farm plot, a river, a
//! named place), not just a 10 m cell, and independent of any single band
//! reading. Two agents that resolve the same real object mint the SAME
//! `entity_cid`, so they reason about one canonical object instead of two
//! divergent textual descriptions. This is the left half of the anti-drift
//! story that `memt:`/`memb:` complete on the value side.
//!
//! Identity model:
//! - `entity_cid = base32_nopad_lc(blake3(identity-preimage)[..16])`.
//! - Convergence key: when a STABLE external id is known (Overture GERS, OSM,
//!   Wikidata) the identity is that id ALONE. Two agents that resolve the same
//!   real object to the same external id therefore mint the same `entity_cid`
//!   regardless of how they labelled or classified it, the strongest possible
//!   convergence.
//! - Without any external anchor the identity falls back to
//!   `(cell64, kind, normalized-label)` so distinct custom objects that share a
//!   10 m cell stay distinct, while re-minting the same one is idempotent.
//! - Geometry (point / bbox / polygon) is carried as METADATA, never in the
//!   identity preimage, so floats from a geocoder cannot perturb the id. This
//!   is also what lets an entity be finer or coarser than the cell grid.
//!
//! `meme:<entity_cid>` is the rebindable citation handle, resolvable like
//! `memt:`/`memb:` back to the signed entity body on any responder that holds
//! it. The mint is attested by a standard emem `Receipt` (primitive
//! `"emem.entity"`), so the text -> canonical-id resolution decision is itself
//! signed and citeable, closing the one drift boundary that a bare geocode
//! leaves open.
//!
//! Known limitation: the locate cache tier stores lat/lng/label/bbox but not a
//! POI's OSM id, so a POI resolved from a warm cache mints with a label-based
//! (weaker) identity rather than an OSM-anchored one; admin boundaries still
//! get GERS on a cache hit. The mint response flags this in its `convergence`
//! field, and `entity_resolve` with `near` still converges divergent phrasings
//! via the per-cell index. Enriching the locate cache with OSM ids so POI cache
//! hits carry the strong anchor is a tracked follow-up.

use serde::{Deserialize, Serialize};

/// Stable external identifiers for a real-world object, in descending order
/// of convergence strength. The first one present becomes the identity anchor.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalIds {
    /// Overture GERS division id (globally stable, citeable in receipts).
    /// Strongest convergence anchor; emitted by the locate cascade whenever an
    /// authoritative Overture division resolves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gers: Option<String>,
    /// OpenStreetMap object as `"<type>/<id>"`, e.g. `"way/717919508"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub osm: Option<String>,
    /// Wikidata QID, e.g. `"Q44440"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wikidata: Option<String>,
}

impl ExternalIds {
    /// True when no external anchor is present.
    pub fn is_empty(&self) -> bool {
        self.gers.is_none() && self.osm.is_none() && self.wikidata.is_none()
    }

    /// The single strongest external anchor, tagged with its scheme:
    /// `gers:<id>` > `osm:<type>/<id>` > `wikidata:<qid>`. `None` when the set
    /// carries no anchor. Empty/whitespace-only components are ignored so a
    /// blank field never masks a weaker-but-real one.
    pub fn best(&self) -> Option<String> {
        let non_empty = |o: &Option<String>| {
            o.as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        };
        if let Some(g) = non_empty(&self.gers) {
            return Some(format!("gers:{g}"));
        }
        if let Some(o) = non_empty(&self.osm) {
            return Some(format!("osm:{o}"));
        }
        if let Some(w) = non_empty(&self.wikidata) {
            return Some(format!("wikidata:{w}"));
        }
        None
    }
}

/// Geometry anchor. Lets an entity be finer or coarser than the 10 m cell.
/// Carried as metadata only; never part of the identity preimage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityGeometry {
    /// Representative point `[lng, lat]` (GeoJSON axis order).
    pub point: [f64; 2],
    /// Optional bounding box `[min_lng, min_lat, max_lng, max_lat]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bbox: Option<[f64; 4]>,
    /// Optional GeoJSON geometry (Polygon / MultiPolygon / LineString) verbatim
    /// from the locate cascade, when the object has a true boundary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geojson: Option<serde_json::Value>,
}

/// The signed entity body. Content-addressed by `entity_cid`; the enclosing
/// API response pairs it with a `Receipt` and the `meme:` token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    /// Schema discriminator. Always `"emem.entity.v1"`.
    #[serde(default = "default_entity_schema")]
    pub schema: String,
    /// Content-addressed id: `base32_nopad_lc(blake3(identity-preimage)[..16])`.
    pub entity_cid: String,
    /// Object class, normalized (lowercase, whitespace-collapsed): `bridge`,
    /// `river`, `farm_plot`, `building`, `admin_division`, `place`, `custom`, …
    pub kind: String,
    /// Human label as supplied (display form). Identity uses the normalized
    /// form; the display form is preserved for readers.
    pub label: String,
    /// The cell64 the representative point falls in, the recall anchor. Any
    /// agent can `recall`/`ask` at this cell for signed facts about the object.
    pub cell64: String,
    /// Geometry anchor (may be finer or coarser than `cell64`).
    pub geometry: EntityGeometry,
    /// Stable external ids that drove (or can drive) convergence.
    #[serde(default, skip_serializing_if = "ExternalIds::is_empty")]
    pub external_ids: ExternalIds,
    /// Optional parent `entity_cid` (containment: a bridge within a locality).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// How the mint resolved the reference, the geocoder tier or path
    /// (`direct_latlng`, `cell64`, `overture_division_match`, `photon`,
    /// `nominatim`, `embedded`, …). Provenance of the text->id decision the
    /// mint receipt signs over.
    pub resolved_via: String,
}

fn default_entity_schema() -> String {
    "emem.entity.v1".to_string()
}

/// Sled tree: `entity_cid -> CBOR({entity, receipt, minted_at})`.
pub const ENTITIES_TREE: &str = "emem.entities";
/// Sled tree: `normalize_alias(key) -> JSON([entity_cid, ...])`. Keys are the
/// normalized label, each tagged external id, and `cell:<cell64>`, so resolve
/// can converge divergent phrasings and list objects at a place.
pub const ENTITY_ALIASES_TREE: &str = "emem.entity_aliases";

/// Normalize a label/kind for identity + alias indexing: trim, lowercase,
/// collapse internal whitespace runs to a single space. Deterministic and
/// locale-independent (ASCII-lowercase only, so it never depends on a locale
/// table that could differ between responders).
pub fn normalize_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_was_space = false;
    for ch in s.trim().chars() {
        if ch.is_whitespace() {
            if !last_was_space {
                out.push(' ');
                last_was_space = true;
            }
        } else {
            out.extend(ch.to_lowercase());
            last_was_space = false;
        }
    }
    out
}

/// Compute `entity_cid` over a deterministic identity preimage. When an
/// external anchor is present the identity is that anchor alone (kind/label are
/// metadata); otherwise it is `(cell64, normalized-kind, normalized-label)`.
///
/// Preimage layout (either branch):
/// ```text
/// "emem.entity.v1|ext|" best_external_id
/// "emem.entity.v1|loc|" cell64 "|" kind_norm "|" label_norm
/// ```
pub fn compute_entity_cid(kind: &str, label: &str, cell64: &str, ext: &ExternalIds) -> String {
    let mut h = blake3::Hasher::new();
    if let Some(eid) = ext.best() {
        h.update(b"emem.entity.v1|ext|");
        h.update(eid.as_bytes());
    } else {
        h.update(b"emem.entity.v1|loc|");
        h.update(cell64.as_bytes());
        h.update(b"|");
        h.update(normalize_text(kind).as_bytes());
        h.update(b"|");
        h.update(normalize_text(label).as_bytes());
    }
    let hash = h.finalize();
    data_encoding::BASE32_NOPAD
        .encode(&hash.as_bytes()[..16])
        .to_lowercase()
}

/// Compose the `meme:<entity_cid>` citation handle.
pub fn entity_token(entity_cid: &str) -> String {
    format!("meme:{entity_cid}")
}

/// Parse a `meme:<entity_cid>` handle, strict on shape. Mirrors
/// `parse_bundle_token` in [`crate::memory_bundle`].
pub fn parse_entity_token(token: &str) -> Result<String, String> {
    let token = token.trim();
    let cid = token.strip_prefix("meme:").ok_or_else(|| {
        "entity token must start with `meme:` discriminator; expected `meme:<entity_cid>`"
            .to_string()
    })?;
    if cid.is_empty() {
        return Err("entity token has empty entity_cid component".into());
    }
    if !(cid.len() >= 16 && cid.len() <= 96 && cid.bytes().all(|c| c.is_ascii_alphanumeric())) {
        return Err(format!(
            "entity token entity_cid segment `{cid}` is not a recognisable CID shape"
        ));
    }
    Ok(cid.to_string())
}

/// The alias keys an entity should be indexed under, so `resolve` can find it
/// from a fuzzy phrasing (normalized label), a stable id (`gers:`/`osm:`/
/// `wikidata:`), or a place (`cell:<cell64>`). All returned keys are already
/// normalized for storage.
pub fn alias_keys(entity: &Entity) -> Vec<String> {
    let mut keys = Vec::new();
    let label_key = normalize_text(&entity.label);
    if !label_key.is_empty() {
        keys.push(label_key);
    }
    if let Some(eid) = entity.external_ids.best() {
        keys.push(eid);
    }
    // Index every present external id (not only the strongest) so a resolve by
    // OSM id still hits when GERS was the identity anchor.
    if let Some(g) = &entity.external_ids.gers {
        let g = g.trim();
        if !g.is_empty() {
            keys.push(format!("gers:{g}"));
        }
    }
    if let Some(o) = &entity.external_ids.osm {
        let o = o.trim();
        if !o.is_empty() {
            keys.push(format!("osm:{o}"));
        }
    }
    if let Some(w) = &entity.external_ids.wikidata {
        let w = w.trim();
        if !w.is_empty() {
            keys.push(format!("wikidata:{w}"));
        }
    }
    keys.push(format!("cell:{}", entity.cell64));
    // Dedup preserving order.
    let mut seen = std::collections::HashSet::new();
    keys.retain(|k| seen.insert(k.clone()));
    keys
}

/// Normalize an arbitrary resolve query into an alias-lookup key. A raw
/// external id (`osm:way/1`, `gers:...`) is preserved; anything else is treated
/// as a label and text-normalized.
pub fn alias_lookup_key(query: &str) -> String {
    let q = query.trim();
    if q.starts_with("gers:")
        || q.starts_with("osm:")
        || q.starts_with("wikidata:")
        || q.starts_with("cell:")
    {
        return q.to_string();
    }
    normalize_text(q)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geom() -> EntityGeometry {
        EntityGeometry {
            point: [-122.4756, 37.8075],
            bbox: None,
            geojson: None,
        }
    }

    #[test]
    fn external_id_is_identity_regardless_of_label_or_kind() {
        // Two agents resolve the same OSM object but label/classify it
        // differently. They MUST mint the same entity_cid.
        let ext = ExternalIds {
            gers: None,
            osm: Some("way/717919508".into()),
            wikidata: None,
        };
        let a = compute_entity_cid("bridge", "Golden Gate Bridge", "defi.zb5ae.hepE.kasI", &ext);
        let b = compute_entity_cid("highway", "golden gate", "defi.zb5ae.hepE.kasI", &ext);
        assert_eq!(a, b, "external id must dominate identity");
    }

    #[test]
    fn gers_outranks_osm() {
        let ext = ExternalIds {
            gers: Some("08abc".into()),
            osm: Some("way/1".into()),
            wikidata: Some("Q1".into()),
        };
        assert_eq!(ext.best(), Some("gers:08abc".into()));
    }

    #[test]
    fn no_external_id_falls_back_to_cell_kind_label() {
        let ext = ExternalIds::default();
        // Same cell+kind+label (case/space-insensitive) -> same cid (idempotent
        // re-mint). Different label -> different cid.
        let a = compute_entity_cid("bridge", "The  Old   Bridge", "defi.zb5ae.hepE.kasI", &ext);
        let b = compute_entity_cid("Bridge", "the old bridge", "defi.zb5ae.hepE.kasI", &ext);
        let c = compute_entity_cid("bridge", "a different bridge", "defi.zb5ae.hepE.kasI", &ext);
        assert_eq!(a, b, "normalized (cell,kind,label) must be idempotent");
        assert_ne!(a, c, "distinct labels at a cell stay distinct");
    }

    #[test]
    fn different_cell_diverges_without_external_id() {
        let ext = ExternalIds::default();
        let a = compute_entity_cid("place", "camp", "defi.zb5ae.hepE.kasI", &ext);
        let b = compute_entity_cid("place", "camp", "defi.zb5ae.hepE.kasO", &ext);
        assert_ne!(a, b);
    }

    #[test]
    fn token_round_trip() {
        let cid = "abcd1234efgh5678".to_string();
        let tok = entity_token(&cid);
        assert_eq!(tok, "meme:abcd1234efgh5678");
        assert_eq!(parse_entity_token(&tok).unwrap(), cid);
    }

    #[test]
    fn token_rejects_bad_shape() {
        assert!(parse_entity_token("memt:abc").is_err());
        assert!(parse_entity_token("meme:").is_err());
        assert!(parse_entity_token("meme:!!!").is_err());
        assert!(parse_entity_token("meme:short").is_err());
    }

    #[test]
    fn alias_keys_cover_label_extids_and_cell() {
        let ext = ExternalIds {
            gers: Some("08abc".into()),
            osm: Some("way/717919508".into()),
            wikidata: None,
        };
        let e = Entity {
            schema: default_entity_schema(),
            entity_cid: "x".into(),
            kind: "bridge".into(),
            label: "Golden Gate Bridge".into(),
            cell64: "defi.zb5ae.hepE.kasI".into(),
            geometry: geom(),
            external_ids: ext,
            parent: None,
            resolved_via: "overture_division_match".into(),
        };
        let keys = alias_keys(&e);
        assert!(keys.contains(&"golden gate bridge".to_string()));
        assert!(keys.contains(&"gers:08abc".to_string()));
        assert!(keys.contains(&"osm:way/717919508".to_string()));
        assert!(keys.contains(&"cell:defi.zb5ae.hepE.kasI".to_string()));
    }

    #[test]
    fn alias_lookup_key_preserves_scheme_but_normalizes_labels() {
        assert_eq!(alias_lookup_key("  OSM:way/1 "), "osm:way/1");
        assert_eq!(
            alias_lookup_key("The  Damaged  Bridge"),
            "the damaged bridge"
        );
    }
}
