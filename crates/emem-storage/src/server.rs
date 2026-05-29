//! Server orchestrator — bundles `Storage` with the responder ed25519
//! signing key and the active manifest CIDs so primitives can build
//! signed receipts without each carrying its own context.
//!
//! Lives in `emem-storage` because it is the natural home for "the live
//! state of an emem responder process": cache + log + fetch + identity.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use blake3::Hasher;
use ed25519_dalek::{Signer, SigningKey};
use rand::rngs::OsRng;
use rand::RngCore;

use emem_core::{AttesterKey, KeyEpoch, Signature};
use emem_fact::{AsOfReceipt, Cost, EdgeCid, FactCid, Receipt, RegistryCid, SchemaCid};

use crate::{AsOfBound, Storage};

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

    /// Build a signed [`Receipt`] for a primitive response. Signature
    /// covers the canonical `request_id || served_at || primitive ||
    /// cells || fact_cids` byte sequence so any client can offline-verify
    /// with the responder's epoch-pubkey.
    pub fn sign_receipt(
        &self,
        primitive: &'static str,
        cells: Vec<String>,
        fact_cids: Vec<FactCid>,
        was_cached: bool,
        started: Instant,
        intent: Option<String>,
    ) -> Receipt {
        let request_id = ulid::Ulid::new().to_string();
        let served_at = iso8601_now();
        let elapsed_ms = started.elapsed().as_millis().min(u32::MAX as u128) as u32;

        let source_versions = self.manifest_versions_snapshot();
        let manifest_hex = Self::manifest_versions_blake3_hex(&source_versions);

        let mut h = Hasher::new();
        h.update(request_id.as_bytes());
        h.update(b"|");
        h.update(served_at.as_bytes());
        h.update(b"|");
        if let Some(ref mh) = manifest_hex {
            h.update(mh.as_bytes());
            h.update(b"|");
        }
        h.update(primitive.as_bytes());
        h.update(b"|");
        for c in &cells {
            h.update(c.as_bytes());
            h.update(b",");
        }
        h.update(b"|");
        for c in &fact_cids {
            h.update(c.as_str().as_bytes());
            h.update(b",");
        }
        let msg = h.finalize();

        let dalek_sig = self.identity.signing.sign(msg.as_bytes());
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
                latency_p50_ms: elapsed_ms,
                latency_p99_ms: elapsed_ms,
                source_freshness_s: 0,
                was_cached,
            },
            as_of: None,
            scope: None,
            edge_cids: Vec::new(),
        }
    }

    /// Scope-aware sibling of [`Server::sign_receipt`]. When `scope` is
    /// `None` or every field is `None`, the signed bytes are byte-identical
    /// to `sign_receipt` and the receipt body omits the scope — keeps
    /// every pre-v0.0.8 receipt verifiable under unchanged rules.
    ///
    /// When at least one scope field is `Some`, the preimage becomes
    ///
    /// ```text
    /// <request_id>|<served_at>|<scope_blake3_hex>|<primitive>|<cells>,|<fact_cids>,
    /// ```
    ///
    /// (the `<scope_blake3_hex>|` segment is inserted between
    /// `served_at` and `primitive`) and the receipt body carries the
    /// `Scope` struct so an offline verifier rebuilds the same digest
    /// and re-checks the signature.
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
        // Empty / absent scope must produce the legacy preimage so
        // existing offline verifiers continue to round-trip byte-for-byte.
        let scope_present = scope.as_ref().is_some_and(|s| !s.is_empty());
        if !scope_present {
            return self.sign_receipt(primitive, cells, fact_cids, was_cached, started, intent);
        }
        let scope_inner = scope.expect("checked just above");

        let request_id = ulid::Ulid::new().to_string();
        let served_at = iso8601_now();
        let elapsed_ms = started.elapsed().as_millis().min(u32::MAX as u128) as u32;

        let scope_hex = scope_inner.blake3_hex();
        let source_versions = self.manifest_versions_snapshot();
        let manifest_hex = Self::manifest_versions_blake3_hex(&source_versions);

        let mut h = Hasher::new();
        h.update(request_id.as_bytes());
        h.update(b"|");
        h.update(served_at.as_bytes());
        h.update(b"|");
        h.update(scope_hex.as_bytes());
        h.update(b"|");
        if let Some(ref mh) = manifest_hex {
            h.update(mh.as_bytes());
            h.update(b"|");
        }
        h.update(primitive.as_bytes());
        h.update(b"|");
        for c in &cells {
            h.update(c.as_bytes());
            h.update(b",");
        }
        h.update(b"|");
        for c in &fact_cids {
            h.update(c.as_str().as_bytes());
            h.update(b",");
        }
        let msg = h.finalize();

        let dalek_sig = self.identity.signing.sign(msg.as_bytes());
        let mut sig_bytes = [0u8; 64];
        sig_bytes.copy_from_slice(&dalek_sig.to_bytes());

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
                latency_p50_ms: elapsed_ms,
                latency_p99_ms: elapsed_ms,
                source_freshness_s: 0,
                was_cached,
            },
            as_of: None,
            scope: Some(scope_inner),
            edge_cids: Vec::new(),
        }
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

    /// Composed signer that honours both `scope` (v0.0.8) and `bound`
    /// (bi-temporal valid/transaction time). The preimage rule extends
    /// the v0.0.8 scope rule with an `as_of_blake3_hex|` segment when
    /// the bound is non-unbounded:
    ///
    /// ```text
    /// <request_id>|<served_at>|[scope_blake3_hex|][as_of_blake3_hex|]<primitive>|<cells>,|<fact_cids>,
    /// ```
    ///
    /// Both optional segments are independent: the scope segment is
    /// emitted when `scope` is non-empty; the `as_of` segment is
    /// emitted when `bound` is non-unbounded. When both are absent the
    /// preimage is byte-identical to the legacy `sign_receipt` rule and
    /// every pre-v0.0.8 receipt continues to verify.
    ///
    /// Until this commit (audit finding F2), the `as_of` field was
    /// attached to the receipt body AFTER signing, so a malicious
    /// responder could claim it honoured a bound it ignored and the
    /// verifier would pass. The bound now enters the signed bytes.
    ///
    /// Existing call sites that only care about one axis call
    /// [`Server::sign_receipt_with_as_of`] (no scope) or
    /// [`Server::sign_receipt_with_scope`] (no bound); this method is
    /// the convergence point for any primitive that takes both.
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
        // Honour the legacy preimage when neither axis applies — keeps
        // every pre-v0.0.8 receipt verifiable byte-for-byte.
        let scope_present = scope.as_ref().is_some_and(|s| !s.is_empty());
        let bound_present = !bound.is_unbounded();
        if !scope_present && !bound_present {
            return self.sign_receipt(primitive, cells, fact_cids, was_cached, started, intent);
        }
        // Scope-only path stays byte-identical to v0.0.8 sign_receipt_with_scope.
        if scope_present && !bound_present {
            return self.sign_receipt_with_scope(
                primitive, cells, fact_cids, was_cached, started, intent, scope,
            );
        }

        let request_id = ulid::Ulid::new().to_string();
        let served_at = iso8601_now();
        let elapsed_ms = started.elapsed().as_millis().min(u32::MAX as u128) as u32;

        // Build the AsOfReceipt block first so its canonical-CBOR
        // digest enters the preimage exactly as a verifier reconstructs
        // it from the same struct in the receipt body.
        let as_of = AsOfReceipt {
            valid_time: bound.valid_time,
            transaction_time: bound.transaction_time.clone(),
        };
        let as_of_hex = as_of.blake3_hex();
        let scope_hex_opt = scope
            .as_ref()
            .filter(|s| !s.is_empty())
            .map(|s| s.blake3_hex());
        let source_versions = self.manifest_versions_snapshot();
        let manifest_hex = Self::manifest_versions_blake3_hex(&source_versions);

        let mut h = Hasher::new();
        h.update(request_id.as_bytes());
        h.update(b"|");
        h.update(served_at.as_bytes());
        h.update(b"|");
        if let Some(ref sh) = scope_hex_opt {
            h.update(sh.as_bytes());
            h.update(b"|");
        }
        h.update(as_of_hex.as_bytes());
        h.update(b"|");
        if let Some(ref mh) = manifest_hex {
            h.update(mh.as_bytes());
            h.update(b"|");
        }
        h.update(primitive.as_bytes());
        h.update(b"|");
        for c in &cells {
            h.update(c.as_bytes());
            h.update(b",");
        }
        h.update(b"|");
        for c in &fact_cids {
            h.update(c.as_str().as_bytes());
            h.update(b",");
        }
        let msg = h.finalize();

        let dalek_sig = self.identity.signing.sign(msg.as_bytes());
        let mut sig_bytes = [0u8; 64];
        sig_bytes.copy_from_slice(&dalek_sig.to_bytes());

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
                latency_p50_ms: elapsed_ms,
                latency_p99_ms: elapsed_ms,
                source_freshness_s: 0,
                was_cached,
            },
            as_of: Some(as_of),
            scope: scope.filter(|s| !s.is_empty()),
            edge_cids: Vec::new(),
        }
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

    /// Edge-aware signer. The preimage extends the v0.0.8 scope + as_of
    /// rule with an `edges_blake3_hex|` segment placed AFTER `as_of` and
    /// BEFORE `manifest`:
    ///
    /// ```text
    /// <request_id>|<served_at>|[scope|][as_of|][edges|][manifest|]<primitive>|<cells>,|<fact_cids>,
    /// ```
    ///
    /// CRITICAL back-compat: when `edges.is_empty()` this early-returns to
    /// [`Server::sign_receipt_full`], so the signed bytes are byte-
    /// identical to today for every existing call site and every pre-
    /// v0.0.9 receipt continues to verify.
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
        // No edges → byte-identical to the established sign_receipt_full
        // path (which itself collapses to sign_receipt / *_with_scope when
        // those axes are also absent).
        if edges.is_empty() {
            return self.sign_receipt_full(
                primitive, cells, fact_cids, was_cached, started, intent, scope, bound,
            );
        }

        let request_id = ulid::Ulid::new().to_string();
        let served_at = iso8601_now();
        let elapsed_ms = started.elapsed().as_millis().min(u32::MAX as u128) as u32;

        let bound_present = !bound.is_unbounded();

        let scope_hex_opt = scope
            .as_ref()
            .filter(|s| !s.is_empty())
            .map(|s| s.blake3_hex());
        let as_of = if bound_present {
            Some(AsOfReceipt {
                valid_time: bound.valid_time,
                transaction_time: bound.transaction_time.clone(),
            })
        } else {
            None
        };
        let as_of_hex_opt = as_of.as_ref().map(|a| a.blake3_hex());
        let edges_hex = Self::edges_blake3_hex(edges).expect("edges non-empty checked above");
        let source_versions = self.manifest_versions_snapshot();
        let manifest_hex = Self::manifest_versions_blake3_hex(&source_versions);

        let mut h = Hasher::new();
        h.update(request_id.as_bytes());
        h.update(b"|");
        h.update(served_at.as_bytes());
        h.update(b"|");
        if let Some(ref sh) = scope_hex_opt {
            h.update(sh.as_bytes());
            h.update(b"|");
        }
        if let Some(ref ah) = as_of_hex_opt {
            h.update(ah.as_bytes());
            h.update(b"|");
        }
        // edges segment: AFTER as_of, BEFORE manifest.
        h.update(edges_hex.as_bytes());
        h.update(b"|");
        if let Some(ref mh) = manifest_hex {
            h.update(mh.as_bytes());
            h.update(b"|");
        }
        h.update(primitive.as_bytes());
        h.update(b"|");
        for c in &cells {
            h.update(c.as_bytes());
            h.update(b",");
        }
        h.update(b"|");
        for c in &fact_cids {
            h.update(c.as_str().as_bytes());
            h.update(b",");
        }
        let msg = h.finalize();

        let dalek_sig = self.identity.signing.sign(msg.as_bytes());
        let mut sig_bytes = [0u8; 64];
        sig_bytes.copy_from_slice(&dalek_sig.to_bytes());

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
                latency_p50_ms: elapsed_ms,
                latency_p99_ms: elapsed_ms,
                source_freshness_s: 0,
                was_cached,
            },
            as_of,
            scope: scope.filter(|s| !s.is_empty()),
            edge_cids: edges.to_vec(),
        }
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

    /// CRITICAL back-compat: sign_receipt_with_edges(edges=[]) is byte-
    /// identical to the legacy sign_receipt_full path. We assert the
    /// preimage segment layout has no edges segment and the signature
    /// verifies under the legacy (no-edges) preimage.
    #[test]
    fn legacy_receipt_still_verifies() {
        let srv = test_server();
        let started = Instant::now();
        let bound = AsOfBound::default();
        let r = srv.sign_receipt_with_edges(
            "emem.recall",
            vec!["damO.zb000.xUti.zde78".into()],
            vec![FactCid::new("fc-1")],
            true,
            started,
            None,
            None,
            &bound,
            &[],
        );
        assert!(r.edge_cids.is_empty(), "no edges → empty edge_cids");

        // Reconstruct the LEGACY preimage (no scope, no as_of, no edges) —
        // request_id | served_at | [manifest|] primitive | cells, | fact_cids,
        let manifest_hex_opt = if r.source_versions.is_empty() {
            None
        } else {
            let mut buf = Vec::new();
            let _ = ciborium::into_writer(&r.source_versions, &mut buf);
            Some(data_encoding::HEXLOWER.encode(blake3::hash(&buf).as_bytes()))
        };
        let mut h = Hasher::new();
        h.update(r.request_id.as_bytes());
        h.update(b"|");
        h.update(r.served_at.as_bytes());
        h.update(b"|");
        if let Some(ref mh) = manifest_hex_opt {
            h.update(mh.as_bytes());
            h.update(b"|");
        }
        h.update(r.primitive.as_bytes());
        h.update(b"|");
        for c in &r.cells {
            h.update(c.as_bytes());
            h.update(b",");
        }
        h.update(b"|");
        for c in &r.fact_cids {
            h.update(c.as_str().as_bytes());
            h.update(b",");
        }
        let msg = h.finalize();
        let pk = ed25519_dalek::VerifyingKey::from_bytes(&r.responder.0).unwrap();
        let sig = ed25519_dalek::Signature::from_bytes(&r.signature.0);
        pk.verify_strict(msg.as_bytes(), &sig)
            .expect("no-edges receipt must verify under the legacy preimage");
    }

    /// With edges present, the receipt verifies ONLY when the preimage
    /// includes the new edges segment AFTER as_of, BEFORE manifest.
    #[test]
    fn edges_receipt_verifies_with_edges_segment() {
        let srv = test_server();
        let started = Instant::now();
        let bound = AsOfBound::default();
        let edges = vec![EdgeCid::new("ecid-1"), EdgeCid::new("ecid-2")];
        let r = srv.sign_receipt_with_edges(
            "emem.recall",
            vec!["cellX".into()],
            vec![FactCid::new("fc-1")],
            true,
            started,
            None,
            None,
            &bound,
            &edges,
        );
        assert_eq!(r.edge_cids.len(), 2);

        let edges_hex = Server::edges_blake3_hex(&r.edge_cids).unwrap();
        let manifest_hex_opt = if r.source_versions.is_empty() {
            None
        } else {
            let mut buf = Vec::new();
            let _ = ciborium::into_writer(&r.source_versions, &mut buf);
            Some(data_encoding::HEXLOWER.encode(blake3::hash(&buf).as_bytes()))
        };
        let mut h = Hasher::new();
        h.update(r.request_id.as_bytes());
        h.update(b"|");
        h.update(r.served_at.as_bytes());
        h.update(b"|");
        // no scope, no as_of → edges segment next.
        h.update(edges_hex.as_bytes());
        h.update(b"|");
        if let Some(ref mh) = manifest_hex_opt {
            h.update(mh.as_bytes());
            h.update(b"|");
        }
        h.update(r.primitive.as_bytes());
        h.update(b"|");
        for c in &r.cells {
            h.update(c.as_bytes());
            h.update(b",");
        }
        h.update(b"|");
        for c in &r.fact_cids {
            h.update(c.as_str().as_bytes());
            h.update(b",");
        }
        let msg = h.finalize();
        let pk = ed25519_dalek::VerifyingKey::from_bytes(&r.responder.0).unwrap();
        let sig = ed25519_dalek::Signature::from_bytes(&r.signature.0);
        pk.verify_strict(msg.as_bytes(), &sig)
            .expect("edges receipt must verify with the edges segment");
    }
}
