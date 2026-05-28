//! Memory bundles — N (cell, band, tslot) triples bundled into one
//! signed envelope. The substrate primitive that lets an agent cite
//! "the state of these places at these vintages" with a single
//! `memb:<bundle_cid>` token instead of N memory tokens.
//!
//! Sign-and-persist model:
//! - `Bundle.bundle_cid` is `base32_nopad_lc(blake3(canonical_cbor(bundle_body)))`
//!   where `bundle_body` is the deterministic concatenation of
//!   `(cell, band, tslot, fact_cid)` quadruples plus the optional
//!   `purpose` string. Re-bundling the same triples on the same fact
//!   set yields a byte-identical `bundle_cid`.
//! - The envelope's `receipt` is a standard emem Receipt: signed by
//!   the responder identity over the canonical preimage
//!   `request_id|served_at|primitive|cells|fact_cids`. Primitive is
//!   `"emem.memory_bundle"`. Cells are the bundle's cells; fact_cids
//!   are every cited fact CID (deduplicated, sort-stable on order of
//!   first appearance).
//! - The bundle stays verifiable by the existing `/v1/verify_receipt`
//!   path: recompute the preimage from `(request_id, served_at,
//!   primitive, cells, fact_cids)` and check ed25519 against the
//!   embedded responder pubkey.

use serde::{Deserialize, Serialize};

use emem_fact::Receipt;

/// One (cell, band, tslot?) request triple. The resolved tslot the
/// responder served is echoed back as `resolved_tslot` next to the
/// `fact_cid` in the cited list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleTriple {
    /// cell64. May be a place name on input; the responder resolves
    /// it to cell64 before bundling.
    pub cell: String,
    /// Band key.
    pub band: String,
    /// Optional tslot pin. When omitted, the responder uses the band's
    /// natural latest tslot at that cell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tslot: Option<u64>,
}

/// One cited fact in the bundle. Carries the resolved cell64, band,
/// resolved tslot, and the fact CID the recall path produced.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleCitation {
    /// Resolved cell64 (input may have been a place name).
    pub cell: String,
    /// Band key.
    pub band: String,
    /// Resolved tslot (the tslot the cited fact was actually attested at).
    pub resolved_tslot: u64,
    /// Content-addressed fact CID. None on a recall miss — the bundle
    /// still carries the citation slot so the agent sees the gap.
    pub fact_cid: Option<String>,
    /// Optional miss-reason when `fact_cid` is None.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub miss_reason: Option<String>,
    /// Composed `memt:<cell>:<fact_cid>` handle for this citation
    /// (only when `fact_cid` is Some).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_token: Option<String>,
}

/// Request body for `POST /v1/memory_bundle`.
#[derive(Debug, Clone, Deserialize)]
pub struct BundleReq {
    /// One or more triples to bundle. At least one required.
    pub triples: Vec<BundleTriple>,
    /// Optional human-readable purpose string. Persisted verbatim
    /// inside the envelope and included in the bundle_cid preimage
    /// so the same triples + different purposes produce distinct CIDs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
}

/// Response body for `POST /v1/memory_bundle` (and the resolve path).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleResp {
    /// `memb:<bundle_cid>` rebindable handle. The full token an agent
    /// drops into another LLM context to cite this set of facts.
    pub bundle_token: String,
    /// `base32_nopad_lc(blake3(canonical_cbor(bundle_body)))`. Self-
    /// certifying — two responders that bundle the same triples and
    /// the same `purpose` over byte-identical facts produce the same
    /// `bundle_cid`.
    pub bundle_cid: String,
    /// Schema discriminator. Always `"emem.memory_bundle.v1"`.
    #[serde(default = "default_bundle_schema")]
    pub schema: String,
    /// One citation per triple in the same order as the request.
    pub citations: Vec<BundleCitation>,
    /// Convenience array of every fact_cid in the bundle, dedup-stable
    /// on first appearance.
    pub fact_cids: Vec<String>,
    /// Convenience array of every cell in the bundle, dedup-stable on
    /// first appearance.
    pub cells: Vec<String>,
    /// The optional purpose string the agent supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    /// ISO 8601 UTC instant the bundle was signed.
    pub signed_at: String,
    /// Responder ed25519 pubkey (base32-nopad-lowercase, 32-byte).
    pub responder_pubkey_b32: String,
    /// Signed Receipt — the offline-verify path is to recompute the
    /// preimage over `(request_id, served_at, primitive, cells,
    /// fact_cids)` and check ed25519 against `responder_pubkey_b32`.
    /// Primitive is `"emem.memory_bundle"`.
    pub receipt: Receipt,
}

/// Compute `bundle_cid` over a deterministic preimage. The same
/// triples + same `purpose` produce the same CID regardless of
/// responder, machine, or wall-clock.
///
/// Preimage layout:
/// ```text
/// "emem.memory_bundle.v1|" purpose? "\n"
///   for each citation in order:
///     cell "|" band "|" tslot "|" fact_cid_or_empty "\n"
/// ```
pub fn compute_bundle_cid(citations: &[BundleCitation], purpose: Option<&str>) -> String {
    let mut h = blake3::Hasher::new();
    h.update(b"emem.memory_bundle.v1|");
    if let Some(p) = purpose {
        h.update(p.as_bytes());
    }
    h.update(b"\n");
    for c in citations {
        h.update(c.cell.as_bytes());
        h.update(b"|");
        h.update(c.band.as_bytes());
        h.update(b"|");
        h.update(c.resolved_tslot.to_string().as_bytes());
        h.update(b"|");
        if let Some(cid) = &c.fact_cid {
            h.update(cid.as_bytes());
        }
        h.update(b"\n");
    }
    let hash = h.finalize();
    data_encoding::BASE32_NOPAD
        .encode(&hash.as_bytes()[..16])
        .to_lowercase()
}

/// Convert a list of [`FactCid`] to deduped string vec preserving
/// first-seen order.
pub fn dedupe_first(cids: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for s in cids {
        if seen.insert(s.clone()) {
            out.push(s);
        }
    }
    out
}

/// `memb:<bundle_cid>` parsing — strict on shape, mirrors the
/// `parse_memory_token` helper in emem-api-rest.
pub fn parse_bundle_token(token: &str) -> Result<String, String> {
    let token = token.trim();
    if !token.starts_with("memb:") {
        return Err(
            "memory bundle token must start with `memb:` discriminator; expected `memb:<bundle_cid>`"
                .into(),
        );
    }
    let cid = &token[5..];
    if cid.is_empty() {
        return Err("memory bundle token has empty bundle_cid component".into());
    }
    if !(cid.len() >= 16 && cid.len() <= 96 && cid.bytes().all(|c| c.is_ascii_alphanumeric())) {
        return Err(format!(
            "memory bundle bundle_cid segment `{cid}` is not a recognisable CID shape"
        ));
    }
    Ok(cid.to_string())
}

/// Sled tree name for `bundle_cid → CBOR(BundleResp)`. Opened on
/// demand from the API layer.
pub const BUNDLES_TREE: &str = "emem.memory_bundles";

fn default_bundle_schema() -> String {
    "emem.memory_bundle.v1".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cid_is_deterministic_over_order() {
        let a = vec![BundleCitation {
            cell: "defi.zb4d9.pefa.zf619".into(),
            band: "indices.ndvi".into(),
            resolved_tslot: 0,
            fact_cid: Some("abc123".into()),
            miss_reason: None,
            memory_token: None,
        }];
        let b = a.clone();
        assert_eq!(compute_bundle_cid(&a, None), compute_bundle_cid(&b, None));
    }

    #[test]
    fn cid_differs_with_purpose() {
        let c = vec![BundleCitation {
            cell: "defi.zb4d9.pefa.zf619".into(),
            band: "indices.ndvi".into(),
            resolved_tslot: 0,
            fact_cid: Some("abc123".into()),
            miss_reason: None,
            memory_token: None,
        }];
        assert_ne!(
            compute_bundle_cid(&c, None),
            compute_bundle_cid(&c, Some("audit-2026"))
        );
    }

    #[test]
    fn token_round_trip() {
        let cid = "abcd1234efgh5678".to_string();
        let token = format!("memb:{cid}");
        assert_eq!(parse_bundle_token(&token).unwrap(), cid);
    }

    #[test]
    fn token_rejects_bad_prefix() {
        assert!(parse_bundle_token("memt:abc").is_err());
        assert!(parse_bundle_token("memb:").is_err());
        assert!(parse_bundle_token("memb:!!!").is_err());
    }
}
