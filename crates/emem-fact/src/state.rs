//! `emem:state:` — a signed, content-addressed node in a derivation graph.
//!
//! # What this is for
//!
//! A responder computes a great deal on the way to an answer and returns only
//! the answer. `/v1/ask` resolves a place, routes the question to topics,
//! recalls or materialises facts, evaluates dozens of algorithms and cites a
//! hundred fact_cids — an ordered derivation, used to pick an output and then
//! discarded. A consumer can cite the answer and cannot cite, skip or reuse any
//! step that produced it.
//!
//! A `StateRecord` gives one derivation step an address. Ask the same question
//! twice and the steps that did not change have the same cid, so a consumer
//! that already holds one skips the bytes entirely. Measured on the live
//! responder, two identical asks seconds apart: 22.9 KB of 72.2 KB was
//! byte-identical, and 106 of 107 fact_cids matched in the same order.
//!
//! # Why it is a tree, not a copy
//!
//! A state commits to the CIDs of its inputs, never to their bytes. That is
//! git's answer and it dissolves the value-or-reference question: the
//! reference IS the value, because an input cannot be substituted without
//! changing this state's cid, and the bytes travel once. Embedding inputs
//! would defeat the only purpose — two states derived from one input would
//! each carry that input again.
//!
//! Whether an input still RESOLVES is a separate property from whether this
//! state is intact, and they are published separately for that reason. An
//! unfetchable input means you cannot get something, not that you cannot trust
//! something.
//!
//! # Why the family needed a new node
//!
//! `FactCid` is 32 bytes over a body, because a fact is a leaf and there is
//! nothing underneath it. A bundle is a 16-byte anchor over a NAME, because
//! being wrong about membership costs a wrong set. A derivation is neither: it
//! has children, and being wrong about it costs a consumer skipping bytes it
//! then believes it holds. So it takes the tree form at full width — 32 bytes,
//! over child hashes.
//!
//! # The property that makes it worth having
//!
//! The cid is `base32_nopad_lc(blake3(canonical_cbor))`, the same recipe
//! `Fact` and `EdgeFact` already use, over the state's own bytes. A consumer
//! recomputes it offline without asking the responder anything. A cid only the
//! server could compute would be an ETag with better branding: a model that
//! skipped on the strength of it would have verified nothing, only trusted a
//! server about what it would have sent.

use serde::{Deserialize, Serialize};

use crate::cid::StateCid;

/// How much a consumer may rely on a state, and it is not a confidence score.
///
/// The same vocabulary the band registry uses for tamper-provenance, because a
/// derivation is subject to exactly the question a measurement is: can someone
/// else arrive at this, or must they take our word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StateClass {
    /// A pure function of the inputs named in `derived_from`, under the
    /// function named in `fn_key`. Anyone holding the inputs re-derives this
    /// exactly. A withheld verdict is this: "the interval spans half the
    /// range" is a decision, it is reusable, and it is reproducible.
    DeterministicIndex,
    /// Produced by a model. The cid commits to THIS OUTPUT HAVING BEEN
    /// PRODUCED, never to it being re-derivable, and the two must not share a
    /// class: a consumer that cannot tell them apart will re-run one and
    /// believe a mismatch is tampering.
    ModelOutput,
    /// A reading this responder made from a registered upstream. Rare here:
    /// an observation is a Fact, and a state that merely carries one should
    /// cite it in `derived_from` instead.
    DirectSensor,
}

/// One addressed step in how an answer was reached.
///
/// Field order is the wire order: `ciborium` emits map keys in declaration
/// order, so this declaration IS the canonicalisation a third party
/// reimplements.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StateRecord {
    /// Schema tag, first, so a decoder knows what it is holding before it
    /// reads anything it has to interpret.
    pub schema: String,
    /// What sort of step this is: `place_resolved`, `topics_routed`,
    /// `facts_recalled`, `algorithms_scored`, `verdict_withheld`. A small
    /// vocabulary on purpose — a kind a consumer cannot recognise is a state
    /// it cannot reuse, so growing this is a decision and not a convenience.
    pub kind: String,
    /// The cids this state was computed FROM, in order: other state cids,
    /// fact cids, or any content address. Hashes, never bytes.
    pub derived_from: Vec<String>,
    /// The function that produced it, where one exists. Required in practice
    /// for `DeterministicIndex`: without it "re-derivable" names no procedure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fn_key: Option<String>,
    /// The step's own result.
    pub payload: ciborium::Value,
    /// What relying on this state gets you.
    pub class: StateClass,
    /// What this state does NOT establish, named rather than left to be
    /// inferred from what sits next to it. A field and not prose: a consumer
    /// composing from this record never reads a sentence.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub does_not_cover: Vec<String>,
    /// RFC 3339, when this responder computed it.
    pub computed_at: String,
    /// The responder that computed it, base32. Who is accountable, which is
    /// not the same as who is right.
    pub responder_pubkey_b32: String,
}

impl StateRecord {
    /// Canonical CBOR. Deterministic for a given record on any platform:
    /// declaration-order keys and `skip_serializing_if` keeps absent optionals
    /// out of the byte stream entirely.
    pub fn to_canonical_cbor(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(512);
        let _ = ciborium::into_writer(self, &mut buf);
        buf
    }

    /// 32-byte blake3 of the canonical CBOR.
    pub fn blake3_digest(&self) -> [u8; 32] {
        *blake3::hash(&self.to_canonical_cbor()).as_bytes()
    }

    /// Content address: `base32_nopad_lc(blake3(canonical_cbor))`.
    ///
    /// The full 32 bytes, not a truncated anchor. A collision in an anchor
    /// costs a merged name; a collision here costs a consumer skipping bytes
    /// it never saw and believing it holds them.
    pub fn cid(&self) -> StateCid {
        StateCid::new(
            data_encoding::BASE32_NOPAD
                .encode(&self.blake3_digest())
                .to_lowercase(),
        )
    }

    /// The citable handle: `emem:state:<cid>`.
    pub fn token(&self) -> String {
        format!("emem:state:{}", self.cid().0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> StateRecord {
        StateRecord {
            schema: "emem.state.v1".into(),
            kind: "verdict_withheld".into(),
            derived_from: vec!["aaa".into(), "bbb".into()],
            fn_key: Some("percentile_places_the_reading@1".into()),
            payload: ciborium::Value::Bool(false),
            class: StateClass::DeterministicIndex,
            does_not_cover: vec!["the counts themselves; this is about their comparison".into()],
            computed_at: "2026-09-10T20:00:00Z".into(),
            responder_pubkey_b32: "777er3yihgifqmv5hmc2wwmyszgddzderzhsx6rex4yoakwomvka".into(),
        }
    }

    /// A consumer recomputes the address from the bytes, with no responder in
    /// the loop. This is the whole difference between a shared state and an
    /// ETag.
    #[test]
    fn the_cid_is_recomputable_from_the_bytes_alone() {
        let s = sample();
        let bytes = s.to_canonical_cbor();

        // What a third party does: decode the bytes, re-encode canonically,
        // hash. No access to the record we started from.
        let decoded: StateRecord = ciborium::from_reader(&bytes[..]).expect("round-trips");
        assert_eq!(decoded, s, "canonical CBOR must round-trip losslessly");
        let recomputed = data_encoding::BASE32_NOPAD
            .encode(blake3::hash(&decoded.to_canonical_cbor()).as_bytes())
            .to_lowercase();
        assert_eq!(recomputed, s.cid().0);
        assert_eq!(s.token(), format!("emem:state:{recomputed}"));
        // 32 bytes at base32 is 52 characters. A shorter address here would
        // be an anchor, and an anchor cannot carry a skip.
        assert_eq!(recomputed.len(), 52, "the full digest, not a truncation");
    }

    /// The tree property: an input cannot be swapped without the address
    /// moving, and the input's bytes never travel.
    #[test]
    fn substituting_an_input_changes_the_address() {
        let a = sample();
        let mut b = sample();
        b.derived_from = vec!["aaa".into(), "ccc".into()];
        assert_ne!(a.cid(), b.cid(), "a different input is a different state");

        // Order is part of the derivation, not incidental: f(x, y) and f(y, x)
        // are not the same step.
        let mut c = sample();
        c.derived_from = vec!["bbb".into(), "aaa".into()];
        assert_ne!(
            a.cid(),
            c.cid(),
            "reordering inputs is a different derivation"
        );

        // And identical content is one address, which is what makes a skip
        // possible at all.
        assert_eq!(a.cid(), sample().cid());
    }

    /// A decision and a model's output must not be able to look alike.
    #[test]
    fn the_class_is_part_of_what_is_addressed() {
        let a = sample();
        let mut b = sample();
        b.class = StateClass::ModelOutput;
        assert_ne!(
            a.cid(),
            b.cid(),
            "re-derivable and merely-produced are different claims and must not \
             share an address"
        );
    }

    /// An absent optional leaves no trace in the bytes, so two records that
    /// differ only by an explicit `None` are one state.
    #[test]
    fn an_absent_field_is_absent_from_the_bytes() {
        let mut a = sample();
        a.fn_key = None;
        a.does_not_cover = vec![];
        let bytes = a.to_canonical_cbor();
        let decoded: StateRecord = ciborium::from_reader(&bytes[..]).expect("round-trips");
        assert_eq!(decoded.fn_key, None);
        assert!(decoded.does_not_cover.is_empty());
        assert_eq!(decoded.cid(), a.cid());
    }
}
