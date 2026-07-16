//! Server orchestrator — bundles `Storage` with the responder ed25519
//! signing key and the active manifest CIDs so primitives can build
//! signed receipts without each carrying its own context.
//!
//! Lives in `emem-storage` because it is the natural home for "the live
//! state of an emem responder process": cache + log + fetch + identity.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use ed25519_dalek::{Signer, SigningKey};
use rand::rngs::OsRng;
use rand::RngCore;

use emem_core::{AttesterKey, KeyEpoch, Signature};
use emem_fact::{AsOfReceipt, Cost, EdgeCid, FactCid, Receipt, RegistryCid, SchemaCid};

use crate::{AsOfBound, Storage};

// ── Per-primitive receipt latency ────────────────────────────────────
//
// Every signed receipt carries `cost.latency_p50_ms` / `p99_ms`, which
// the spec documents as "observed latency for this primitive class". To
// make that true — rather than echoing one request's elapsed time twice
// — the responder keeps a disjoint-bucket latency histogram per primitive
// `&'static str` and reads back real percentiles on each sign. The bucket
// ladder matches the API-layer request histogram (`LATENCY_BUCKETS_MS` in
// emem-api-rest) so a receipt and `/metrics` read on the same scale. This
// is process-wide and advisory: it never enters a fact CID or the signed
// receipt preimage.

/// Twelve log-spaced upper bounds (ms); slot 12 is the `> 5000 ms`
/// overflow. Each request increments exactly one slot.
const RECEIPT_LATENCY_BUCKETS_MS: [u32; 12] =
    [1, 2, 5, 10, 20, 50, 100, 200, 500, 1000, 2000, 5000];

static PRIMITIVE_LATENCY: OnceLock<Mutex<HashMap<&'static str, [u64; 13]>>> = OnceLock::new();

fn primitive_latency() -> &'static Mutex<HashMap<&'static str, [u64; 13]>> {
    PRIMITIVE_LATENCY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Record one observation for `primitive` and return `(p50, p99)` over
/// every observation for that primitive class so far. Percentiles are at
/// bucket-boundary resolution (the bucket upper bound) — coarse by
/// construction, honest, and cheap. Advisory metadata, so a poisoned
/// lock is recovered rather than panicking on the signing path.
fn observe_primitive_latency(primitive: &'static str, elapsed_ms: u32) -> (u32, u32) {
    let bucket = RECEIPT_LATENCY_BUCKETS_MS
        .iter()
        .position(|b| elapsed_ms <= *b)
        .unwrap_or(12);
    let mut map = match primitive_latency().lock() {
        Ok(m) => m,
        Err(poisoned) => poisoned.into_inner(),
    };
    let hist = map.entry(primitive).or_insert([0u64; 13]);
    hist[bucket] = hist[bucket].saturating_add(1);
    let total: u64 = hist.iter().sum();
    let percentile = |p: f64| -> u32 {
        if total == 0 {
            return 0;
        }
        let target = ((total as f64) * p).ceil() as u64;
        let mut cum = 0u64;
        for (i, c) in hist.iter().enumerate() {
            cum += *c;
            if cum >= target {
                return RECEIPT_LATENCY_BUCKETS_MS
                    .get(i)
                    .copied()
                    .unwrap_or_else(|| RECEIPT_LATENCY_BUCKETS_MS[11].saturating_mul(2));
            }
        }
        0
    };
    (percentile(0.50), percentile(0.99))
}

/// A live emem responder. Owned by the HTTP server and lent to each
/// primitive call.
pub struct Server {
    /// Storage facade (cache + fetch + log).
    pub storage: Arc<dyn Storage>,
    /// Responder identity — used to sign every receipt.
    pub identity: ResponderIdentity,
    /// Active manifest CIDs (registry + schema). Embedded into receipts.
    pub manifests: ManifestCids,
    /// Wall-clock unix seconds when this responder process came up.
    /// Surfaced via `/health` so agents know whether they are talking
    /// to a freshly-restarted instance (cache cold, materialize stats
    /// reset) or a long-running one.
    pub started_at_unix_s: i64,
}

/// The pubkey + signing key + epoch for the responder.
pub struct ResponderIdentity {
    /// ed25519 signing key.
    pub signing: SigningKey,
    /// Pubkey wire form.
    pub pubkey: AttesterKey,
    /// Key rotation epoch.
    pub epoch: KeyEpoch,
}

impl ResponderIdentity {
    /// Generate a fresh key.
    pub fn fresh() -> Self {
        let mut sec = [0u8; 32];
        OsRng.fill_bytes(&mut sec);
        Self::from_secret(sec, 0)
    }

    /// Build from raw 32-byte secret.
    pub fn from_secret(secret: [u8; 32], epoch: u32) -> Self {
        let signing = SigningKey::from_bytes(&secret);
        let vk = signing.verifying_key();
        let mut pk = [0u8; 32];
        pk.copy_from_slice(vk.as_bytes());
        Self {
            signing,
            pubkey: AttesterKey(pk),
            epoch: KeyEpoch(epoch),
        }
    }

    /// 64-byte signing key (secret || pub) — base32-rendered for export.
    pub fn export_secret_b32(&self) -> String {
        data_encoding::BASE32_NOPAD
            .encode(&self.signing.to_bytes())
            .to_lowercase()
    }
}

/// Manifest CIDs in force at this responder.
#[derive(Debug, Clone)]
pub struct ManifestCids {
    /// Function-registry CID.
    pub registry_cid: RegistryCid,
    /// Schema (CDDL bundle) CID.
    pub schema_cid: SchemaCid,
    /// Bands manifest CID.
    pub bands_cid: String,
    /// Sources manifest CID.
    pub sources_cid: String,
}

impl Server {
    /// Build a server with a fresh key.
    pub fn new(storage: Arc<dyn Storage>, manifests: ManifestCids) -> Self {
        Self {
            storage,
            identity: ResponderIdentity::fresh(),
            manifests,
            started_at_unix_s: now_unix_s(),
        }
    }
}

fn now_unix_s() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl Server {
    /// Borrow the per-attester reputation registry if storage tracks it.
    pub fn storage_attesters(&self) -> Option<&crate::AttesterRegistry> {
        self.storage.attesters()
    }

    /// Snapshot of the active manifest CIDs for the receipt's
    /// `source_versions` field. Honest provenance per the spec: the
    /// receipt names the exact registry / schema / bands / sources
    /// CIDs in force at the moment the responder signed. An offline
    /// auditor reading a receipt months later can pull those CIDs and
    /// know which registry version produced the verdict.
    ///
    /// Audit 2026-05-29 finding F3: previously the field was
    /// hard-coded to `BTreeMap::new()` so the manifest provenance was
    /// signed-receipt theatre. Now populated; the verifier doesn't
    /// validate the entries against the preimage today (the map is
    /// outside the v0.0.8 preimage rule), but downstream auditors can
    /// at minimum see what the responder claimed it was using.
    fn manifest_versions_snapshot(&self) -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        m.insert(
            "registry_cid".into(),
            self.manifests.registry_cid.as_str().to_string(),
        );
        m.insert(
            "schema_cid".into(),
            self.manifests.schema_cid.as_str().to_string(),
        );
        m.insert("bands_cid".into(), self.manifests.bands_cid.clone());
        m.insert("sources_cid".into(), self.manifests.sources_cid.clone());
        m
    }

    /// Lowercase-hex blake3 of the canonical-CBOR encoding of a manifest
    /// snapshot. `BTreeMap` is canonically ordered by key so the CBOR
    /// encoding is deterministic across responders that share the same
    /// manifest tree. Empty maps return `None` so the signer can skip
    /// the segment and keep pre-f357568 receipts (which carried an empty
    /// `source_versions` field) verifiable byte-for-byte.
    ///
    /// Audit 2026-05-29 follow-up to F3: the snapshot is now mixed into
    /// the receipt preimage as a new optional segment, closing the
    /// "body claims X, signature binds nothing" gap. The verifier
    /// rebuilds the same digest when the receipt body carries a
    /// non-empty `source_versions`.
    pub(crate) fn manifest_versions_blake3_hex(
        versions: &BTreeMap<String, String>,
    ) -> Option<String> {
        if versions.is_empty() {
            return None;
        }
        let mut buf = Vec::with_capacity(128);
        // BTreeMap<String, String> is canonically encodable as a CBOR
        // map; ciborium emits keys in iteration order (BTreeMap is
        // already sorted), giving determinism without ad-hoc sorting.
        let _ = ciborium::into_writer(versions, &mut buf);
        Some(data_encoding::HEXLOWER.encode(blake3::hash(&buf).as_bytes()))
    }

    /// Build a signed [`Receipt`] for a primitive response. Since the
    /// preimage-v1 cutover every new receipt is signed over
    /// [`emem_attest::receipt_preimage_v1`] — domain-separated, every
    /// segment tagged and length-prefixed — and carries
    /// `preimage_version: 1` so verifiers pick the right rule. Receipts
    /// signed under the legacy v0 concatenation rule (no
    /// `preimage_version` field) continue to verify via the v0 branch in
    /// `/v1/verify_receipt` and the `/verify` page.
    pub fn sign_receipt(
        &self,
        primitive: &'static str,
        cells: Vec<String>,
        fact_cids: Vec<FactCid>,
        was_cached: bool,
        started: Instant,
        intent: Option<String>,
    ) -> Receipt {
        self.sign_receipt_v1_inner(
            primitive,
            cells,
            fact_cids,
            was_cached,
            started,
            intent,
            None,
            None,
            &[],
            None,
        )
    }

    /// Field sibling of [`Server::sign_receipt`]: the (aoi_cid,
    /// derivation_cid) binding enters the v1 preimage as the tagged FIELD
    /// segment and rides the receipt body so an offline verifier rebuilds
    /// the same digest (docs/plans/field-tokens.md).
    pub fn sign_receipt_field(
        &self,
        primitive: &'static str,
        cells: Vec<String>,
        fact_cids: Vec<FactCid>,
        was_cached: bool,
        started: Instant,
        field: emem_fact::FieldBinding,
    ) -> Receipt {
        self.sign_receipt_v1_inner(
            primitive,
            cells,
            fact_cids,
            was_cached,
            started,
            None,
            None,
            None,
            &[],
            Some(field),
        )
    }

    /// The ONE signing path for every receipt variant. Optional axes
    /// (scope, bi-temporal bound, edges) enter the v1 preimage as tagged
    /// segments only when present, so absence/presence is unambiguous in
    /// the signed bytes — under the v0 rule a scope digest and an as_of
    /// digest were indistinguishable untagged hex at the same position.
    #[allow(clippy::too_many_arguments)]
    fn sign_receipt_v1_inner(
        &self,
        primitive: &'static str,
        cells: Vec<String>,
        fact_cids: Vec<FactCid>,
        was_cached: bool,
        started: Instant,
        intent: Option<String>,
        scope: Option<emem_fact::Scope>,
        bound: Option<&AsOfBound>,
        edges: &[EdgeCid],
        field: Option<emem_fact::FieldBinding>,
    ) -> Receipt {
        let request_id = ulid::Ulid::new().to_string();
        let served_at = iso8601_now();
        let elapsed_ms = started.elapsed().as_millis().min(u32::MAX as u128) as u32;
        // Real per-primitive-class percentiles, not this one sample twice.
        let (latency_p50_ms, latency_p99_ms) = observe_primitive_latency(primitive, elapsed_ms);

        let scope = scope.filter(|s| !s.is_empty());
        let scope_hex = scope.as_ref().map(|s| s.blake3_hex());
        let as_of = bound.filter(|b| !b.is_unbounded()).map(|b| AsOfReceipt {
            valid_time: b.valid_time,
            transaction_time: b.transaction_time.clone(),
        });
        let as_of_hex = as_of.as_ref().map(|a| a.blake3_hex());
        let edges_hex = Self::edges_blake3_hex(edges);
        let field_hex = field.as_ref().map(|f| f.blake3_hex());
        let source_versions = self.manifest_versions_snapshot();
        let manifest_hex = Self::manifest_versions_blake3_hex(&source_versions);

        let msg = emem_attest::receipt_preimage_v1(
            &request_id,
            &served_at,
            scope_hex.as_deref(),
            as_of_hex.as_deref(),
            edges_hex.as_deref(),
            manifest_hex.as_deref(),
            field_hex.as_deref(),
            primitive,
            cells.iter().map(|s| s.as_str()),
            fact_cids.iter().map(|c| c.as_str()),
        );

        let dalek_sig = self.identity.signing.sign(&msg);
        let mut sig_bytes = [0u8; 64];
        sig_bytes.copy_from_slice(&dalek_sig.to_bytes());

        // Surface a merkle inclusion proof for the first cited fact when
        // one was persisted at attestation time. A receipt with multiple
        // fact_cids carries one proof (the schema's `merkle_proof` is
        // `Option<MerkleProof>`); a verifier with the responder pubkey
        // can already re-derive every other CID from the signed receipt
        // payload, so a single inclusion anchor is sufficient. None when
        // the cited facts pre-date the proof tree (ephemeral runs,
        // older attestations) — the receipt's signature still binds the
        // CIDs end-to-end.
        let merkle_proof = fact_cids
            .first()
            .and_then(|c| self.storage.proof_for_cid(c));

        Receipt {
            request_id,
            served_at,
            primitive: primitive.into(),
            intent,
            cells,
            fact_cids,
            schema_cid: self.manifests.schema_cid.clone(),
            merkle_proof,
            responder: self.identity.pubkey,
            responder_key_epoch: self.identity.epoch,
            signature: Signature(sig_bytes),
            source_versions,
            registry_cid: self.manifests.registry_cid.clone(),
            cost: Cost {
                credits: 0,
                latency_p50_ms,
                latency_p99_ms,
                source_freshness_s: 0,
                was_cached,
            },
            as_of,
            field,
            scope,
            edge_cids: edges.to_vec(),
            preimage_version: emem_attest::PREIMAGE_V1,
        }
    }

    /// Scope-aware sibling of [`Server::sign_receipt`]. The scope (when
    /// non-empty) enters the v1 preimage as a tagged
    /// `blake3(canonical_cbor(Scope))` hex segment and the receipt body
    /// carries the `Scope` struct so an offline verifier rebuilds the
    /// same digest.
    #[allow(clippy::too_many_arguments)]
    pub fn sign_receipt_with_scope(
        &self,
        primitive: &'static str,
        cells: Vec<String>,
        fact_cids: Vec<FactCid>,
        was_cached: bool,
        started: Instant,
        intent: Option<String>,
        scope: Option<emem_fact::Scope>,
    ) -> Receipt {
        self.sign_receipt_v1_inner(
            primitive,
            cells,
            fact_cids,
            was_cached,
            started,
            intent,
            scope,
            None,
            &[],
            None,
        )
    }

    /// Bi-temporal sibling of [`Server::sign_receipt`]. When `bound`
    /// constrains either axis the returned receipt carries an `as_of`
    /// block recording the exact bound the responder honoured, so an
    /// offline verifier can replay the same read. Unbounded bounds
    /// produce a receipt byte-identical (in the `as_of`-omitted JSON
    /// projection) to the historical `sign_receipt`.
    #[allow(clippy::too_many_arguments)]
    pub fn sign_receipt_with_as_of(
        &self,
        primitive: &'static str,
        cells: Vec<String>,
        fact_cids: Vec<FactCid>,
        was_cached: bool,
        started: Instant,
        intent: Option<String>,
        bound: &AsOfBound,
    ) -> Receipt {
        self.sign_receipt_full(
            primitive, cells, fact_cids, was_cached, started, intent, None, bound,
        )
    }

    /// Composed signer that honours both `scope` and the bi-temporal
    /// `bound`. Each axis enters the v1 preimage as its own tagged
    /// segment only when present; the receipt body records the same
    /// structs so an offline verifier rebuilds identical digests.
    #[allow(clippy::too_many_arguments)]
    pub fn sign_receipt_full(
        &self,
        primitive: &'static str,
        cells: Vec<String>,
        fact_cids: Vec<FactCid>,
        was_cached: bool,
        started: Instant,
        intent: Option<String>,
        scope: Option<emem_fact::Scope>,
        bound: &AsOfBound,
    ) -> Receipt {
        self.sign_receipt_v1_inner(
            primitive,
            cells,
            fact_cids,
            was_cached,
            started,
            intent,
            scope,
            Some(bound),
            &[],
            None,
        )
    }

    /// Lowercase-hex blake3 of the canonical-CBOR encoding of the SORTED
    /// edge-CID string list. `None` when empty so the signer skips the
    /// segment entirely and every pre-v0.0.9 receipt verifies byte-for-
    /// byte. Sorting makes the digest order-independent — a verifier
    /// rebuilds it from `receipt.edge_cids` without caring how the
    /// responder ordered them.
    pub fn edges_blake3_hex(edges: &[EdgeCid]) -> Option<String> {
        if edges.is_empty() {
            return None;
        }
        let mut strs: Vec<String> = edges.iter().map(|c| c.as_str().to_string()).collect();
        strs.sort();
        let mut buf = Vec::with_capacity(64 * strs.len());
        let _ = ciborium::into_writer(&strs, &mut buf);
        Some(data_encoding::HEXLOWER.encode(blake3::hash(&buf).as_bytes()))
    }

    /// Edge-aware signer: cited edge CIDs enter the v1 preimage as a
    /// tagged `blake3(canonical_cbor(sorted_cid_strings))` hex segment
    /// when non-empty, and the receipt body lists `edge_cids` so a
    /// verifier rebuilds the digest order-independently.
    #[allow(clippy::too_many_arguments)]
    pub fn sign_receipt_with_edges(
        &self,
        primitive: &'static str,
        cells: Vec<String>,
        fact_cids: Vec<FactCid>,
        was_cached: bool,
        started: Instant,
        intent: Option<String>,
        scope: Option<emem_fact::Scope>,
        bound: &AsOfBound,
        edges: &[EdgeCid],
    ) -> Receipt {
        self.sign_receipt_v1_inner(
            primitive,
            cells,
            fact_cids,
            was_cached,
            started,
            intent,
            scope,
            Some(bound),
            edges,
            None,
        )
    }
}

/// ISO 8601 UTC timestamp like `2026-04-26T13:55:00Z`. Computed without a
/// chrono dependency using the Howard Hinnant civil-from-days algorithm.
pub fn iso8601_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    iso8601_from_unix(secs)
}

fn iso8601_from_unix(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let hh = sod / 3600;
    let mm = (sod % 3600) / 60;
    let ss = sod % 60;
    let (y, mo, d) = days_to_ymd(days);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, mo, d, hh, mm, ss)
}

fn days_to_ymd(days: i64) -> (i32, u32, u32) {
    // Howard Hinnant, "civil_from_days": https://howardhinnant.github.io/date_algorithms.html
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y_civil = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = (if m <= 2 { y_civil + 1 } else { y_civil }) as i32;
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso8601_formats_known_unix() {
        // 1970-01-01T00:00:00Z
        assert_eq!(iso8601_from_unix(0), "1970-01-01T00:00:00Z");
        // 2026-01-01T00:00:00Z (the emem epoch).
        assert_eq!(iso8601_from_unix(1_767_225_600), "2026-01-01T00:00:00Z");
        // One full year on must roll the year forward.
        assert_eq!(
            iso8601_from_unix(1_767_225_600 + 365 * 86_400),
            "2027-01-01T00:00:00Z"
        );
        // Hours/minutes/seconds round-trip.
        assert_eq!(
            iso8601_from_unix(1_767_225_600 + 13 * 3600 + 55 * 60 + 7),
            "2026-01-01T13:55:07Z"
        );
    }

    fn test_server() -> Server {
        use crate::MaterializingStorage;
        let bands = std::sync::Arc::new(emem_core::bands::DEFAULT.clone());
        let functions = std::sync::Arc::new(
            emem_core::FunctionRegistry::parse_default().expect("default functions"),
        );
        let sources = std::sync::Arc::new(
            emem_core::SourceRegistry::parse_default().expect("default sources"),
        );
        let storage = std::sync::Arc::new(
            MaterializingStorage::ephemeral(bands, functions, sources).expect("ephemeral"),
        );
        Server {
            storage,
            identity: ResponderIdentity::fresh(),
            manifests: ManifestCids {
                registry_cid: RegistryCid::new("test-registry"),
                schema_cid: SchemaCid::new("test-schema"),
                bands_cid: "test-bands".into(),
                sources_cid: "test-sources".into(),
            },
            started_at_unix_s: 0,
        }
    }

    /// `edges_blake3_hex([]) == None`; non-empty is order-independent.
    #[test]
    fn edges_blake3_hex_empty_is_none_and_sorted() {
        assert!(Server::edges_blake3_hex(&[]).is_none());
        let a = Server::edges_blake3_hex(&[EdgeCid::new("z"), EdgeCid::new("a")]);
        let b = Server::edges_blake3_hex(&[EdgeCid::new("a"), EdgeCid::new("z")]);
        assert_eq!(a, b, "sorted → order-independent digest");
        assert!(a.is_some());
    }

    /// Rebuild the v1 preimage from a receipt's body exactly as an
    /// offline verifier does, returning the bytes ed25519 should accept.
    fn rebuild_v1_preimage(r: &Receipt) -> [u8; 32] {
        let manifest_hex = if r.source_versions.is_empty() {
            None
        } else {
            let mut buf = Vec::new();
            let _ = ciborium::into_writer(&r.source_versions, &mut buf);
            Some(data_encoding::HEXLOWER.encode(blake3::hash(&buf).as_bytes()))
        };
        let scope_hex = r
            .scope
            .as_ref()
            .filter(|s| !s.is_empty())
            .map(|s| s.blake3_hex());
        let as_of_hex = r
            .as_of
            .as_ref()
            .filter(|a| !a.is_unbounded())
            .map(|a| a.blake3_hex());
        let edges_hex = Server::edges_blake3_hex(&r.edge_cids);
        let field_hex = r.field.as_ref().map(|f| f.blake3_hex());
        emem_attest::receipt_preimage_v1(
            &r.request_id,
            &r.served_at,
            scope_hex.as_deref(),
            as_of_hex.as_deref(),
            edges_hex.as_deref(),
            manifest_hex.as_deref(),
            field_hex.as_deref(),
            &r.primitive,
            r.cells.iter().map(|s| s.as_str()),
            r.fact_cids.iter().map(|c| c.as_str()),
        )
    }

    fn verify_receipt_v1(r: &Receipt) {
        assert_eq!(r.preimage_version, emem_attest::PREIMAGE_V1);
        let msg = rebuild_v1_preimage(r);
        let pk = ed25519_dalek::VerifyingKey::from_bytes(&r.responder.0).unwrap();
        let sig = ed25519_dalek::Signature::from_bytes(&r.signature.0);
        pk.verify_strict(&msg, &sig)
            .expect("receipt must verify under the rebuilt v1 preimage");
    }

    /// A plain recall receipt (no scope/as_of/edges) is signed under v1
    /// and verifies when the verifier rebuilds the tagged preimage from
    /// the receipt body.
    #[test]
    fn plain_receipt_verifies_v1() {
        let srv = test_server();
        let r = srv.sign_receipt_with_edges(
            "emem.recall",
            vec!["damO.zb000.xUti.zde78".into()],
            vec![FactCid::new("fc-1")],
            true,
            Instant::now(),
            None,
            None,
            &AsOfBound::default(),
            &[],
        );
        assert!(r.edge_cids.is_empty(), "no edges → empty edge_cids");
        verify_receipt_v1(&r);
    }

    /// Edges present → the edges segment enters the tagged v1 preimage
    /// and the verifier rebuilds it from `receipt.edge_cids`.
    #[test]
    fn edges_receipt_verifies_v1() {
        let srv = test_server();
        let edges = vec![EdgeCid::new("ecid-1"), EdgeCid::new("ecid-2")];
        let r = srv.sign_receipt_with_edges(
            "emem.recall",
            vec!["cellX".into()],
            vec![FactCid::new("fc-1")],
            true,
            Instant::now(),
            None,
            None,
            &AsOfBound::default(),
            &edges,
        );
        assert_eq!(r.edge_cids.len(), 2);
        verify_receipt_v1(&r);
    }

    /// A v1 receipt must NOT verify if a verifier mistakenly tries the
    /// optional segment in the wrong slot — proves the tagging defeats
    /// the v0 scope/as_of positional ambiguity. Here we forge a preimage
    /// that puts the manifest hash where a scope segment would sit and
    /// confirm it's rejected.
    #[test]
    fn v1_rejects_segment_confusion() {
        let srv = test_server();
        let r = srv.sign_receipt(
            "emem.recall",
            vec!["cellX".into()],
            vec![FactCid::new("fc-1")],
            true,
            Instant::now(),
            None,
        );
        // Genuine preimage verifies.
        verify_receipt_v1(&r);
        // A preimage that injects a bogus scope segment must not.
        let bogus = emem_attest::receipt_preimage_v1(
            &r.request_id,
            &r.served_at,
            Some("deadbeef"),
            None,
            None,
            None,
            None,
            &r.primitive,
            r.cells.iter().map(|s| s.as_str()),
            r.fact_cids.iter().map(|c| c.as_str()),
        );
        let pk = ed25519_dalek::VerifyingKey::from_bytes(&r.responder.0).unwrap();
        let sig = ed25519_dalek::Signature::from_bytes(&r.signature.0);
        assert!(
            pk.verify_strict(&bogus, &sig).is_err(),
            "forged scope segment must not verify against a no-scope receipt"
        );
    }
}
