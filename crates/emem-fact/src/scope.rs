//! Scope — multi-tenant axis on every read and every signed receipt.
//!
//! v0.0.8 adds a four-tuple `{user_id, agent_id, run_id, org_id}` axis
//! orthogonal to the existing `(cell, band, tslot)` + `(attester
//! pubkey)` axes. A scope answers the four questions Mem0, Letta, and
//! Graphiti's `group_id` answer in their own way:
//!
//! - `user_id`  — the end-user the agent is acting on behalf of
//! - `agent_id` — the agent instance (one process / one persona)
//! - `run_id`   — a single conversation / task / workflow run
//! - `org_id`   — the deploying organisation (multi-tenant SaaS)
//!
//! All four are `Option<String>` so a single-tenant deployment can keep
//! sending `None` and get the legacy unscoped semantics. A scope with
//! every field `None` is byte-equivalent to "no scope at all" and the
//! receipt-signing path drops the scope segment from the preimage so
//! pre-v0.0.8 receipts continue to verify byte-for-byte.
//!
//! ## Receipt binding
//!
//! When at least one scope field is `Some`, the responder includes
//! `blake3(canonical_cbor(Scope))` (32 bytes, hex-encoded) inside the
//! receipt preimage immediately after `served_at`. The receipt body
//! carries the `Scope` struct so an offline verifier can reconstruct
//! the same digest and re-check the signature.
//!
//! When every field is `None`, the responder uses the legacy preimage
//! (no scope segment) and omits the scope from the receipt body. This
//! is the byte-identical pre-v0.0.8 path.
//!
//! ## Canonical CBOR
//!
//! Fields are serialised in declaration order
//! (`user_id, agent_id, run_id, org_id`) using `ciborium` with
//! `serde(skip_serializing_if = "Option::is_none")` so absent fields
//! produce no bytes — keeps the canonical form stable when later
//! versions add fields that older callers don't set.
//!
//! ## Sled indexing
//!
//! `emem.scope_index` (added in a follow-up commit) is keyed
//! `(scope_hash, cell, band) -> fact_cid_set` so a recall with a
//! scope filter range-scans a prefix in O(matches) without scanning
//! every fact at the cell.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

/// Four-axis multi-tenant scope. Carried on every write that wants to
/// be addressable by tenant later, and on every read that wants to
/// restrict its result set to a tenant.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Scope {
    /// End-user the agent is acting on behalf of. Free-form opaque
    /// string; the responder never tries to interpret it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    /// Agent instance (one process / one persona).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Single conversation / task / workflow run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// Deploying organisation (multi-tenant SaaS).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org_id: Option<String>,
}

impl Scope {
    /// `true` when every field is `None`. Such a scope contributes
    /// nothing to the preimage and is dropped from the receipt body —
    /// callers should pass `None` rather than constructing one of
    /// these, but it's safe if they do.
    pub fn is_empty(&self) -> bool {
        self.user_id.is_none()
            && self.agent_id.is_none()
            && self.run_id.is_none()
            && self.org_id.is_none()
    }

    /// Canonical CBOR encoding. Deterministic for a given `Scope`
    /// regardless of platform — uses `ciborium` which encodes maps in
    /// declaration order via serde's field-visit sequence.
    pub fn to_canonical_cbor(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(64);
        // ciborium::into_writer is infallible for Scope (Option<String>
        // fields only); unwrap_or_default keeps the helper total.
        let _ = ciborium::into_writer(self, &mut buf);
        buf
    }

    /// 32-byte blake3 of the canonical CBOR. The preimage segment that
    /// goes into the receipt signature.
    pub fn blake3_digest(&self) -> [u8; 32] {
        let bytes = self.to_canonical_cbor();
        *blake3::hash(&bytes).as_bytes()
    }

    /// Lowercase hex of [`Scope::blake3_digest`]. The wire form that
    /// gets concatenated into the preimage byte stream.
    pub fn blake3_hex(&self) -> String {
        data_encoding::HEXLOWER.encode(&self.blake3_digest())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_scope_is_empty() {
        assert!(Scope::default().is_empty());
    }

    #[test]
    fn any_field_makes_it_non_empty() {
        for s in [
            Scope {
                user_id: Some("u".into()),
                ..Default::default()
            },
            Scope {
                agent_id: Some("a".into()),
                ..Default::default()
            },
            Scope {
                run_id: Some("r".into()),
                ..Default::default()
            },
            Scope {
                org_id: Some("o".into()),
                ..Default::default()
            },
        ] {
            assert!(
                !s.is_empty(),
                "scope with one Some field must not be empty: {s:?}"
            );
        }
    }

    #[test]
    fn canonical_cbor_is_deterministic() {
        let s = Scope {
            user_id: Some("alice".into()),
            agent_id: Some("ops-agent-1".into()),
            run_id: None,
            org_id: Some("acme-co".into()),
        };
        let a = s.to_canonical_cbor();
        let b = s.to_canonical_cbor();
        assert_eq!(a, b, "encoding must be byte-identical across calls");
    }

    #[test]
    fn empty_scope_cbor_is_short() {
        // ciborium emits an empty map for a struct with no Some fields;
        // exact bytes are CBOR major-type 5 (map) with length 0.
        let empty = Scope::default().to_canonical_cbor();
        assert_eq!(
            empty,
            vec![0xa0],
            "empty Scope must encode as CBOR map(0) = 0xa0"
        );
    }

    #[test]
    fn digest_changes_when_any_field_changes() {
        let base = Scope {
            user_id: Some("alice".into()),
            ..Default::default()
        };
        let other = Scope {
            user_id: Some("bob".into()),
            ..Default::default()
        };
        assert_ne!(base.blake3_digest(), other.blake3_digest());
        assert_ne!(base.blake3_digest(), Scope::default().blake3_digest());
    }

    #[test]
    fn blake3_hex_is_64_chars() {
        let s = Scope {
            user_id: Some("u1".into()),
            ..Default::default()
        };
        let h = s.blake3_hex();
        assert_eq!(h.len(), 64);
        assert!(h
            .chars()
            .all(|c| c.is_ascii_hexdigit() && (c.is_ascii_digit() || c.is_ascii_lowercase())));
    }

    #[test]
    fn serde_round_trips_via_cbor() {
        let s = Scope {
            user_id: Some("u".into()),
            agent_id: Some("a".into()),
            run_id: Some("r".into()),
            org_id: Some("o".into()),
        };
        let bytes = s.to_canonical_cbor();
        let back: Scope = ciborium::de::from_reader(bytes.as_slice()).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn serde_round_trips_via_json() {
        let s = Scope {
            user_id: Some("u".into()),
            agent_id: None,
            run_id: Some("r".into()),
            org_id: None,
        };
        let j = serde_json::to_string(&s).unwrap();
        // Absent fields must not appear in the JSON form.
        assert!(!j.contains("agent_id"));
        assert!(!j.contains("org_id"));
        let back: Scope = serde_json::from_str(&j).unwrap();
        assert_eq!(s, back);
    }
}
