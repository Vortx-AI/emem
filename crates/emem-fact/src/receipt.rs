//! Receipt — proof of recall, with cost self-declaration. Spec §7.

use serde::{Deserialize, Serialize};

use crate::cid::{FactCid, RegistryCid, SchemaCid};
use crate::scope::Scope;
use emem_core::{AttesterKey, KeyEpoch, Signature};

/// Returned with every read response. Cryptographically rebindable evidence
/// that a particular set of facts was served.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    /// ULID.
    pub request_id: String,
    /// ISO 8601 serve time.
    pub served_at: String,
    /// "recall" | "verify" | "find_similar" | ...
    pub primitive: String,
    /// If served via emem.intent, the intent type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
    /// cell64 references in the response.
    pub cells: Vec<String>,
    /// Fact CIDs cited.
    pub fact_cids: Vec<FactCid>,
    /// CID of the response schema.
    pub schema_cid: SchemaCid,
    /// Inclusion proof to the current attestation root, if applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merkle_proof: Option<MerkleProof>,
    /// Responder pubkey.
    pub responder: AttesterKey,
    /// Responder key rotation epoch.
    pub responder_key_epoch: KeyEpoch,
    /// ed25519 signature.
    pub signature: Signature,
    /// Per-source version pins (e.g. {"geotessera.v1": "2024"}).
    pub source_versions: std::collections::BTreeMap<String, String>,
    /// CID of registry used to serve.
    pub registry_cid: RegistryCid,
    /// Cost / latency / freshness self-declaration.
    pub cost: Cost,
    /// Bi-temporal filter that produced this response, when the caller
    /// pinned either valid-time (`as_of_tslot`) or transaction-time
    /// (`as_of_signed_at`). Recorded in the receipt body so a verifier
    /// can replay the same query later by reissuing it with the same
    /// bound. Omitted from JSON when the caller did not constrain either
    /// axis — back-compat for pre-bi-temporal receipts that never
    /// carried this field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_of: Option<AsOfReceipt>,
    /// Multi-tenant scope (`{user_id, agent_id, run_id, org_id}`) the
    /// responder honoured when serving this response. Recorded so an
    /// offline verifier reconstructs the same `blake3(canonical_cbor(
    /// Scope))` segment used in the signature preimage. Omitted from
    /// JSON when the caller supplied no scope — back-compat for
    /// pre-v0.0.8 receipts that never carried this field; their
    /// preimage rule did not include a scope segment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<Scope>,
}

/// Replay-able bi-temporal filter recorded in a [`Receipt`] when the
/// caller pinned valid-time or transaction-time. The verifier reconstructs
/// the query by reissuing the same read with these two fields populated.
/// At least one of the two is `Some` when this struct is present in a
/// receipt; an "unbounded" caller skips emitting the struct entirely so
/// existing offline verifiers continue to round-trip pre-bi-temporal
/// receipts byte-for-byte.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct AsOfReceipt {
    /// Valid-time bound — only facts with `tslot <= valid_time` were
    /// considered. `None` means valid-time was unconstrained.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_time: Option<u64>,
    /// Transaction-time bound — only facts with `signed_at <=
    /// transaction_time` were considered. RFC 3339 string, identical to
    /// the request's `as_of_signed_at`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_time: Option<String>,
}

impl AsOfReceipt {
    /// `true` when both bounds are `None`. Such an `AsOfReceipt` has no
    /// information to bind, and the receipt-signing path drops the
    /// `as_of` segment from the preimage — keeps every pre-v0.0.8
    /// receipt verifiable byte-for-byte. Callers should pass `None`
    /// rather than constructing one of these, but it's safe if they do.
    pub fn is_unbounded(&self) -> bool {
        self.valid_time.is_none() && self.transaction_time.is_none()
    }

    /// Canonical CBOR encoding. Deterministic for a given `AsOfReceipt`
    /// regardless of platform — `ciborium` encodes maps in declaration
    /// order via serde's field-visit sequence, and
    /// `skip_serializing_if = "Option::is_none"` keeps absent fields
    /// out of the byte stream. The byte stream is what the signer
    /// hashes into the preimage and what an offline verifier rebuilds.
    ///
    /// The `transaction_time` field, when present, is normalised through
    /// [`normalise_rfc3339_utc`] before CBOR-encoding so that the three
    /// equivalent spellings `2026-05-29T00:00:00Z`,
    /// `2026-05-29T00:00:00.000Z`, and `2026-05-29T00:00:00+00:00` all
    /// hash to the same digest. Closes audit 2026-05-29 F-receipt-2.
    pub fn to_canonical_cbor(&self) -> Vec<u8> {
        let normalised = AsOfReceipt {
            valid_time: self.valid_time,
            transaction_time: self
                .transaction_time
                .as_deref()
                .map(|s| normalise_rfc3339_utc(s).unwrap_or_else(|| s.to_string())),
        };
        let mut buf = Vec::with_capacity(32);
        // ciborium::into_writer is infallible for AsOfReceipt
        // (Option<u64> + Option<String> fields only); ignore the result
        // to keep the helper total.
        let _ = ciborium::into_writer(&normalised, &mut buf);
        buf
    }

    /// 32-byte blake3 of [`AsOfReceipt::to_canonical_cbor`]. The
    /// preimage segment a responder hashes into the signed bytes when
    /// the receipt carries a non-unbounded `as_of`.
    pub fn blake3_digest(&self) -> [u8; 32] {
        let bytes = self.to_canonical_cbor();
        *blake3::hash(&bytes).as_bytes()
    }

    /// Lowercase hex of [`AsOfReceipt::blake3_digest`]. The wire form
    /// that gets concatenated into the preimage byte stream between
    /// `[scope_blake3_hex|]` (if any) and `primitive`.
    pub fn blake3_hex(&self) -> String {
        data_encoding::HEXLOWER.encode(&self.blake3_digest())
    }
}

/// Empirical cost+latency+freshness self-declared in every receipt.
/// See spec §20.4.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cost {
    /// Protocol credits charged for this call.
    pub credits: u64,
    /// Observed p50 latency for this primitive class, ms.
    pub latency_p50_ms: u32,
    /// Observed p99 latency, ms.
    pub latency_p99_ms: u32,
    /// Age of the stalest source in the response, seconds.
    pub source_freshness_s: u32,
    /// Whether the response was served from cache.
    pub was_cached: bool,
}

/// Merkle inclusion proof for a fact within an attestation batch root.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MerkleProof {
    /// Leaf index in the canonical-sorted batch.
    pub leaf_index: u32,
    /// Sibling hashes from leaf to root.
    pub path: Vec<[u8; 32]>,
    /// The expected batch root.
    pub root: [u8; 32],
}

/// Normalise an RFC 3339 timestamp into the canonical form
/// `YYYY-MM-DDTHH:MM:SSZ` so the AsOfReceipt preimage is byte-stable
/// across equivalent spellings (`Z`, `+00:00`, fractional seconds).
/// Returns `None` when the input is not RFC 3339 — the caller falls
/// back to the raw string in that case so non-UTC offsets aren't
/// silently dropped. Only UTC (`Z` / `+00:00` / `-00:00`) inputs are
/// canonicalised; offset-bearing inputs are passed through unchanged
/// so a verifier still rebuilds the same digest the signer wrote.
pub fn normalise_rfc3339_utc(s: &str) -> Option<String> {
    // Minimum: YYYY-MM-DDTHH:MM:SS (19 chars) + a trailing zone marker.
    if s.len() < 20 {
        return None;
    }
    let bytes = s.as_bytes();
    // Quick shape check on the date/time skeleton.
    if bytes[4] != b'-'
        || bytes[7] != b'-'
        || (bytes[10] != b'T' && bytes[10] != b' ')
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }
    for &i in &[0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18] {
        if !bytes[i].is_ascii_digit() {
            return None;
        }
    }
    let date_time = &s[..19];
    // Tail starts at byte 19. Accept optional `.NNN...` fractional
    // seconds, then either `Z`, `+00:00`, `-00:00`, or a non-UTC
    // offset. Only UTC variants normalise; others fall through.
    let mut rest = &s[19..];
    if rest.starts_with('.') {
        // Skip the dot + N digits.
        let frac_end = rest[1..]
            .find(|c: char| !c.is_ascii_digit())
            .map(|i| i + 1)
            .unwrap_or(rest.len());
        if frac_end <= 1 {
            return None;
        }
        rest = &rest[frac_end..];
    }
    let is_utc = matches!(rest, "Z" | "+00:00" | "-00:00");
    if !is_utc {
        return None;
    }
    Some(format!("{}Z", date_time.replace(' ', "T")))
}

#[cfg(test)]
mod as_of_receipt_tests {
    use super::*;

    #[test]
    fn unbounded_default_is_unbounded() {
        assert!(AsOfReceipt::default().is_unbounded());
    }

    #[test]
    fn any_bound_makes_it_bound() {
        let v = AsOfReceipt {
            valid_time: Some(1_700_000_000),
            transaction_time: None,
        };
        assert!(!v.is_unbounded());
        let t = AsOfReceipt {
            valid_time: None,
            transaction_time: Some("2026-05-29T00:00:00Z".into()),
        };
        assert!(!t.is_unbounded());
    }

    #[test]
    fn canonical_cbor_is_deterministic() {
        let a = AsOfReceipt {
            valid_time: Some(1_700_000_000),
            transaction_time: Some("2026-05-29T00:00:00Z".into()),
        };
        assert_eq!(a.to_canonical_cbor(), a.to_canonical_cbor());
    }

    #[test]
    fn digest_changes_when_valid_time_flips_one_bit() {
        let a = AsOfReceipt {
            valid_time: Some(1_700_000_000),
            transaction_time: None,
        };
        let b = AsOfReceipt {
            valid_time: Some(1_700_000_001),
            transaction_time: None,
        };
        assert_ne!(a.blake3_digest(), b.blake3_digest());
    }

    #[test]
    fn blake3_hex_is_64_lowercase_chars() {
        let a = AsOfReceipt {
            valid_time: Some(42),
            transaction_time: None,
        };
        let h = a.blake3_hex();
        assert_eq!(h.len(), 64);
        assert!(h
            .chars()
            .all(|c| c.is_ascii_hexdigit() && (c.is_ascii_digit() || c.is_ascii_lowercase())));
    }

    #[test]
    fn transaction_time_equivalent_spellings_hash_same() {
        // Z, +00:00, and fractional-seconds-with-Z must all hash to the
        // same digest after normalisation. Pre-fix: each produced a
        // different digest, so receipts wouldn't cross-verify.
        let a = AsOfReceipt {
            valid_time: None,
            transaction_time: Some("2026-05-29T00:00:00Z".into()),
        };
        let b = AsOfReceipt {
            valid_time: None,
            transaction_time: Some("2026-05-29T00:00:00+00:00".into()),
        };
        let c = AsOfReceipt {
            valid_time: None,
            transaction_time: Some("2026-05-29T00:00:00.000Z".into()),
        };
        assert_eq!(a.blake3_digest(), b.blake3_digest());
        assert_eq!(a.blake3_digest(), c.blake3_digest());
    }

    #[test]
    fn normalise_rfc3339_utc_handles_common_variants() {
        assert_eq!(
            normalise_rfc3339_utc("2026-05-29T00:00:00Z"),
            Some("2026-05-29T00:00:00Z".into())
        );
        assert_eq!(
            normalise_rfc3339_utc("2026-05-29T00:00:00+00:00"),
            Some("2026-05-29T00:00:00Z".into())
        );
        assert_eq!(
            normalise_rfc3339_utc("2026-05-29T00:00:00.123Z"),
            Some("2026-05-29T00:00:00Z".into())
        );
        // Non-UTC offset falls through to None so the raw input is kept.
        assert_eq!(normalise_rfc3339_utc("2026-05-29T00:00:00+05:30"), None);
        // Garbage falls through.
        assert_eq!(normalise_rfc3339_utc("not a date"), None);
    }

    #[test]
    fn transaction_time_only_differs_from_valid_time_only() {
        let v = AsOfReceipt {
            valid_time: Some(1_700_000_000),
            transaction_time: None,
        };
        let t = AsOfReceipt {
            valid_time: None,
            transaction_time: Some("2026-05-29T00:00:00Z".into()),
        };
        assert_ne!(v.blake3_digest(), t.blake3_digest());
    }
}
