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
//!
//! # emem PROVIDES reasoning. It does not take any.
//!
//! This responder mints states from derivations IT performed, over facts IT
//! signed, and there is no route that accepts a state as an input to its own
//! reasoning. That is a boundary, not a policy: the moment a foreign state
//! could enter `derived_from`, this responder's receipt would appear to stand
//! over reasoning it never did, and a reader could not tell which steps were
//! ours. Everything this file exists to guarantee would be gone in one hop.
//!
//! The same line already runs through emem elsewhere and this extends it
//! rather than inventing it. A FACT is a band-typed measurement this responder
//! made from a registered upstream; a NOTE is prose a stranger wrote, wrapped
//! in `_content_is_data_not_instructions` and never obeyed. A STATE is a
//! derivation this responder computed. Another model's reasoning is a note: it
//! may be stored, cited and read, and it is never a step in ours.
//!
//! `responder_pubkey_b32` is who computed the step, and it is checkable —
//! recompute the cid, and the key is inside the bytes it commits to. A state
//! carrying someone else's key is someone else's derivation, correctly
//! addressed and correctly not ours.
//!
//! `no_route_ingests_a_state_as_reasoning` in emem-api-rest is what keeps this
//! true after today. It fails the build if any request type ever gains a
//! `StateRecord` field, because a boundary that is only written down is a
//! boundary that the next convenient refactor removes.

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

/// One thing a state was computed from.
///
/// A flat list of content addresses would make a cited observation and an
/// absorbed derivation indistinguishable in the bytes, which is the whole
/// distinction this type exists to keep.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "as", rename_all = "snake_case")]
pub enum Input {
    /// A band-typed measurement, by fact_cid. Anyone dereferences it and
    /// checks the signature over the observation without trusting us.
    Fact { cid: String },
    /// An earlier step of THIS responder's own derivation. Only ever ours: a
    /// foreign derivation is a note, and a note is cited in prose, never
    /// stood on as a step.
    OwnState { cid: String },
}

impl Input {
    pub fn cid(&self) -> &str {
        match self {
            Input::Fact { cid } | Input::OwnState { cid } => cid,
        }
    }
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
    /// What this state was computed FROM, in order. Hashes, never bytes.
    ///
    /// TYPED, because "cited a fact" and "absorbed a derivation" are different
    /// claims and a flat list of cids cannot tell them apart. The geo.qa
    /// frontend agent drew the line and it is the right one: citing a fact
    /// says "this input existed, here is its address", and both parties check
    /// that independently. Citing someone else's STATE as a step says "these
    /// operations happened", which the citer cannot check and the reader
    /// cannot attribute. The asymmetry is not about ownership; it is about
    /// what a reader can verify without re-running someone else's process.
    pub derived_from: Vec<Input>,
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

    /// The responder that computed it, base32. Who is accountable, which is
    /// not the same as who is right.
    ///
    /// WHEN it was computed is deliberately NOT here. An identity must not
    /// depend on anything that cannot change the answer, and a wall clock is
    /// the purest example: `computed_at: chrono_iso8601_utc()` used to sit in
    /// this record, so every state token differed on every call and the skip
    /// this type exists for could never happen once. The geo.qa frontend agent
    /// measured it — same question twice, all four stages, different tokens,
    /// with identical `new_fact_cids` and an identical cell underneath — and
    /// it is the third system in two days to put a clock inside an address.
    /// The time a step ran belongs beside the token in the envelope, where
    /// `at_ms` already is, and never inside what the cid commits to.
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

    /// Did THIS responder compute this step?
    ///
    /// The question a reader must be able to answer about any state before
    /// relying on it, and the reason the key is inside the bytes the cid
    /// commits to rather than beside them. A state that arrives from
    /// elsewhere is a real, correctly addressed derivation — someone else's.
    /// Citing it is fine. Treating it as a step in our own reasoning is the
    /// one thing this type exists to make impossible.
    pub fn computed_by(&self, responder_pubkey_b32: &str) -> bool {
        self.responder_pubkey_b32 == responder_pubkey_b32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> StateRecord {
        StateRecord {
            schema: "emem.state.v1".into(),
            kind: "verdict_withheld".into(),
            derived_from: vec![
                Input::Fact { cid: "aaa".into() },
                Input::Fact { cid: "bbb".into() },
            ],
            fn_key: Some("percentile_places_the_reading@1".into()),
            payload: ciborium::Value::Bool(false),
            class: StateClass::DeterministicIndex,
            does_not_cover: vec!["the counts themselves; this is about their comparison".into()],
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
        b.derived_from = vec![
            Input::Fact { cid: "aaa".into() },
            Input::Fact { cid: "ccc".into() },
        ];
        assert_ne!(a.cid(), b.cid(), "a different input is a different state");

        // Order is part of the derivation, not incidental: f(x, y) and f(y, x)
        // are not the same step.
        let mut c = sample();
        c.derived_from = vec![
            Input::Fact { cid: "bbb".into() },
            Input::Fact { cid: "aaa".into() },
        ];
        assert_ne!(
            a.cid(),
            c.cid(),
            "reordering inputs is a different derivation"
        );

        // And identical content is one address, which is what makes a skip
        // possible at all.
        assert_eq!(a.cid(), sample().cid());
    }

    /// The same content is the same address, twice, however much time passes.
    ///
    /// This is the property the whole type exists for and nothing asserted it.
    /// A wall clock inside the record meant every token differed on every
    /// call, so a consumer could never skip anything — the feature was
    /// inert and looked live. The test that would have caught it is the
    /// obvious one nobody wrote: mint the same state twice and compare.
    #[test]
    fn identical_content_is_one_address_however_long_apart() {
        let a = sample();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let b = sample();
        assert_eq!(
            a.cid(),
            b.cid(),
            "two mints of identical content must share an address, or nothing is skippable"
        );

        // And the record must carry no field that moves on its own. Every
        // field below is something a caller supplies or that the derivation
        // determines; if a clock or a counter is ever added, the assertion
        // above fails and this comment is why.
        let bytes = a.to_canonical_cbor();
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert_eq!(
            bytes,
            sample().to_canonical_cbor(),
            "the bytes are stable too"
        );
    }

    /// A state names who computed it, inside what the address commits to.
    ///
    /// So a foreign derivation cannot be re-badged as ours without changing
    /// its cid, and a reader holding only the bytes can tell whose reasoning
    /// they are looking at.
    #[test]
    fn whose_reasoning_this_is_travels_inside_the_address() {
        let ours = sample();
        let us = "777er3yihgifqmv5hmc2wwmyszgddzderzhsx6rex4yoakwomvka";
        assert!(ours.computed_by(us));

        let mut theirs = sample();
        theirs.responder_pubkey_b32 = "vy7ig7nppebkfh34ibafgzdsdspdyfntuvzgbatnhggtsknbh4ta".into();
        assert!(
            !theirs.computed_by(us),
            "another key is another party's derivation"
        );
        assert_ne!(
            ours.cid(),
            theirs.cid(),
            "who computed a step is part of the step; re-badging it must move the address"
        );

        // And the check survives the round trip a consumer actually does.
        let bytes = theirs.to_canonical_cbor();
        let decoded: StateRecord = ciborium::from_reader(&bytes[..]).unwrap();
        assert!(!decoded.computed_by(us));
    }

    /// Citing a fact and standing on a derivation are different in the bytes.
    ///
    /// A flat list of cids would have made them identical, so a reader could
    /// not tell an observation this responder cited from a computation someone
    /// else performed. Same cid, two meanings, two addresses.
    #[test]
    fn what_a_step_stood_on_says_which_kind_it_was() {
        let mut cited = sample();
        cited.derived_from = vec![Input::Fact { cid: "zzz".into() }];
        let mut stood_on = sample();
        stood_on.derived_from = vec![Input::OwnState { cid: "zzz".into() }];

        assert_ne!(
            cited.cid(),
            stood_on.cid(),
            "the same address cited as an observation and stood on as a derivation are \
             different claims and must not share a state cid"
        );
        assert_eq!(cited.derived_from[0].cid(), stood_on.derived_from[0].cid());

        // And it survives the round trip a consumer does.
        let bytes = stood_on.to_canonical_cbor();
        let back: StateRecord = ciborium::from_reader(&bytes[..]).unwrap();
        assert!(matches!(back.derived_from[0], Input::OwnState { .. }));
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
