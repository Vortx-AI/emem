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
use emem_fact::{Cost, FactCid, Receipt, RegistryCid, SchemaCid};

use crate::Storage;

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

        let mut h = Hasher::new();
        h.update(request_id.as_bytes());
        h.update(b"|");
        h.update(served_at.as_bytes());
        h.update(b"|");
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
            source_versions: BTreeMap::new(),
            registry_cid: self.manifests.registry_cid.clone(),
            cost: Cost {
                credits: 0,
                latency_p50_ms: elapsed_ms,
                latency_p99_ms: elapsed_ms,
                source_freshness_s: 0,
                was_cached,
            },
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
}
