//! The capability→intent registry: what an agent needs, in the agent's words.
//!
//! Every other discovery surface here answers a question the agent has not
//! asked yet. `/openapi.json` says which routes exist, the A2A card says which
//! skills exist, `/v1/tools` says what each tool does. All of them presuppose
//! that the reader already decided to use emem and now wants the door. None
//! answers the question actually being asked first, which is *whether to knock
//! at all*.
//!
//! Agents do not think "I need emem". They think:
//!
//!   "I need memory another agent can trust."
//!   "I need to verify a claim someone handed me."
//!   "I need state that survives context compaction."
//!
//! So this table is keyed on the need, phrased the way the agent phrases it,
//! and maps need → capability → protocol → endpoint → tool → how to check it
//! worked. The tool name is the LAST field, not the first, because it is the
//! least useful thing to someone who does not yet know what we are.
//!
//! # Why the unserved intents are in here
//!
//! The temptation with a registry like this is to list only what we do well.
//! That builds the same failure this whole protocol argues against: a claim
//! that stops matching, discovered by the caller rather than declared by us.
//! An agent that routes "keep this private" to emem and finds out afterwards
//! that every note is world-readable has been misled by our own index.
//!
//! So `Coverage::No` and `Coverage::Partial` rows are first-class, each owing
//! a `why` and an `instead`. A registry that cannot say no is advertising.
//! `intents_test::every_limit_states_why_and_what_instead` enforces it.
//!
//! Three of those rows were measured against the live responder on
//! 2026-08-17 rather than recalled: of every path `/openapi.json` documents,
//! not one is a session, checkpoint, owner-scope or server-side-signing route,
//! and no memory write verb has a REST twin at all. The count is deliberately
//! not quoted here; it moves every time a route lands, and a number that has
//! to be maintained in a comment is a number that will eventually be wrong.

use serde_json::{json, Value as JsonValue};

/// How well emem actually serves a need. Ordered worst-last on purpose: a
/// reader scanning the serialised registry meets what works before caveats.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Coverage {
    /// Shipped, live, and checkable by the `verify` field on the same row.
    Yes,
    /// Part of the need is met and part is not. The `why` says which half.
    Partial,
    /// Not served. Said out loud so an agent routes elsewhere immediately
    /// instead of discovering it after a write it cannot take back.
    No,
}

impl Coverage {
    fn as_str(self) -> &'static str {
        match self {
            Coverage::Yes => "served",
            Coverage::Partial => "partial",
            Coverage::No => "not_served",
        }
    }
}

pub struct Intent {
    /// The need in the agent's own words. This is the match key, so it is
    /// phrased as a first-person statement of the problem rather than as a
    /// feature name.
    pub need: &'static str,
    /// Other phrasings of the same need. A router matching on any of these
    /// should land on the same row.
    pub also: &'static [&'static str],
    /// The label a human-facing summary uses, chosen rather than derived.
    ///
    /// The decision layer in the agent guide first picked the shortest alias
    /// and produced "persistent Earth state" and a bare "provenance" as the
    /// headline for two rows, which is both unclear and the geo-first framing
    /// this table exists to correct. A display label is content; deriving it
    /// from string length picks whichever alias happens to be terse.
    pub short: &'static str,
    /// What emem calls the thing. Second, deliberately: our vocabulary is
    /// only useful once the need has been recognised.
    pub capability: &'static str,
    pub coverage: Coverage,
    /// Required for Partial and No. The specific reason, naming the missing
    /// mechanism, never a vague "not yet supported".
    pub why: &'static str,
    /// Required for Partial and No. Where the agent should go instead. May
    /// point back into emem when a different primitive does cover it.
    pub instead: &'static str,
    /// Empty for a No row: there is nothing to call.
    pub tool: &'static str,
    /// The REST twin, or "" where the verb is MCP-only. Not every capability
    /// has both doors, and saying so here is cheaper than a 404.
    pub rest: &'static str,
    /// A call an agent can make immediately, with real arguments.
    pub first_call: &'static str,
    /// How the agent confirms it worked, without trusting this document.
    /// Every served row owes one; that is what makes this a contract rather
    /// than a brochure.
    pub verify: &'static str,
}

/// The registry. Served rows first, then partial, then unserved, because a
/// reader who stops early should stop having read the capabilities.
pub const INTENTS: &[Intent] = &[
    Intent {
        need: "I need memory another agent can read and trust without trusting me",
        also: &[
            "I need shared memory across agents",
            "persistent Earth state",
            "satellite evidence for a location",
            "what was observed here",
            "I need persistent memory between sessions",
            "I need a memory my whole fleet can read",
        ],
        short: "Memory two agents can both trust",
        capability: "signed, content-addressed shared memory",
        coverage: Coverage::Yes,
        why: "",
        instead: "",
        tool: "emem_recall",
        rest: "POST /v1/recall",
        first_call: "POST /v1/recall {\"place\":\"Bengaluru\",\"bands\":[\"copdem30m.elevation_mean\"]}",
        verify: "The response carries an ed25519 receipt. Check it with POST /v1/verify_receipt, or offline against the responder pubkey in /.well-known/emem.json. A read you cannot verify is hearsay.",
    },
    Intent {
        need: "I need to hand another agent a claim it can check instead of a sentence it has to believe",
        also: &[
            "I need a citeable handle for this fact",
            "I need my citation to survive leaving this conversation",
            "I need cross-agent provenance",
            "provenance",
            "pass evidence to another agent",
            "can another agent verify this",
        ],
        short: "A citeable handle for a fact",
        capability: "emem:fact: / emem:bundle: tokens over signed bytes",
        coverage: Coverage::Yes,
        why: "",
        instead: "",
        tool: "emem_memory_token",
        rest: "POST /v1/memory_token",
        first_call: "POST /v1/memory_token {\"fact_cid\":\"<cid from a recall>\"}",
        verify: "Hand the token to a different agent, or to a fresh context. POST /v1/memory_token/resolve returns the byte-identical body. Same bytes on both sides is the whole property.",
    },
    Intent {
        need: "I need two agents to mean the same object when they say the same word",
        also: &[
            "I need a canonical identity for this thing",
            "I need to stop referential drift",
            "is this the same object we discussed before",
            "remember this location",
            "cross-agent referential consistency",
        ],
        short: "One identity for a thing, across agents",
        capability: "entity identity (emem:entity:<entity_cid>)",
        coverage: Coverage::Yes,
        why: "",
        instead: "",
        tool: "emem_entity",
        rest: "POST /v1/entity",
        first_call: "POST /v1/entity {\"label\":\"Mount Fuji\",\"kind\":\"place\",\"place\":\"Mount Fuji\"}",
        verify: "Register the same object from a second phrasing via POST /v1/entity/resolve and confirm it converges on the identical entity_cid. Note the honest limit: an entity token is hashed from an anchor, not the whole record, so it is a shared reference rather than shared bytes.",
    },
    Intent {
        need: "I need to verify a claim someone handed me",
        also: &[
            "I need to check a citation is real",
            "somebody gave me a token, is it genuine",
            "I need to audit what another agent told me",
        ],
        short: "Check a claim another agent gave you",
        capability: "offline receipt verification against an append-only log",
        coverage: Coverage::Yes,
        why: "",
        instead: "",
        tool: "emem_verify_receipt",
        rest: "POST /v1/verify_receipt",
        first_call: "POST /v1/memory_token/resolve {\"token\":\"emem:fact:…\"} then POST /v1/verify_receipt with the receipt it returns",
        verify: "Verification is arithmetic on your side, so it does not require trusting this responder. Cross-check inclusion in the transparency log with POST /v1/log/inclusion.",
    },
    Intent {
        need: "I need to know whether my sources disagree before I answer",
        also: &[
            "do these sources agree",
            "I need to detect drift between attesters",
            "agents disagree about this place",
            "which of these two numbers is right",
        ],
        short: "Find where sources disagree",
        capability: "contradiction detection at one address",
        coverage: Coverage::Yes,
        why: "",
        instead: "",
        tool: "emem_memory_contradictions",
        rest: "POST /v1/memory_contradictions",
        first_call: "POST /v1/memory_contradictions {\"place\":\"Bengaluru\"}",
        verify: "Each side of a reported disagreement is a resolvable fact_cid. Resolve both and compare them yourself rather than taking the severity score on faith.",
    },
    Intent {
        need: "I need to check my own draft is grounded before I send it",
        also: &[
            "I need a gate that catches ungrounded claims",
            "I need to stop myself hallucinating a number",
            "does every citation in this still resolve",
        ],
        short: "Gate your own draft before sending",
        capability: "grounding gate over a draft",
        coverage: Coverage::Yes,
        why: "",
        instead: "",
        tool: "emem_guard_verdict",
        rest: "POST /v1/guard/verdict",
        first_call: "POST /v1/guard/verdict {\"text\":\"<your draft>\"}",
        verify: "Send a draft with a deliberately broken token and confirm it denies, then the corrected one and confirm it allows. A gate you have not tried to fool is not a gate. Read `action` and `clearance` as separate fields.",
    },
    Intent {
        need: "I need to hand work to another agent across a trust boundary",
        also: &[
            "I need a multi-agent handoff that survives",
            "the next agent has no context, how do I brief it",
            "I need to pass state between agents",
        ],
        short: "Hand work across a trust boundary",
        capability: "bundle tokens plus a read-side mailbox",
        coverage: Coverage::Yes,
        why: "",
        instead: "",
        tool: "emem_memory_bundle",
        rest: "POST /v1/memory_bundle",
        first_call: "POST /v1/memory_bundle {\"fact_cids\":[\"<cid>\",\"<cid>\"]}",
        verify: "One bundle token is 38 characters at any count up to 256. Resolve it from a context that never saw the originals; if the receiving agent can reconstruct the facts, the boundary held.",
    },
    Intent {
        need: "I need to know what is actually true at a place",
        also: &[
            "what is at <place>",
            "I need grounded environmental data",
            "did <band> change between t1 and t2",
        ],
        short: "Signed observations at a place",
        capability: "signed Earth-observation facts at a canonical address",
        coverage: Coverage::Yes,
        why: "",
        instead: "",
        tool: "emem_locate",
        rest: "POST /v1/locate",
        first_call: "POST /v1/locate {\"place\":\"Mount Fuji\"} then POST /v1/recall with the cell64 it returns",
        verify: "Ask a second agent to resolve the same place name and compare cell64 strings. Identical addressing is the property; the value is only as good as the band's provenance_class, which /v1/bands states per band.",
    },
    Intent {
        need: "I need to compare places, or find the ones like this one",
        also: &[
            "compare X and Y",
            "find places like X",
            "find similar places",
            "average <band> over this region",
            "find <event> in <region>",
            "where is <event> happening",
            "show me <event> hotspots in <region>",
        ],
        short: "Compare places, or find similar ones",
        capability: "comparison, similarity search and region aggregation over signed facts",
        coverage: Coverage::Yes,
        why: "",
        instead: "",
        tool: "emem_compare",
        rest: "POST /v1/compare",
        first_call: "POST /v1/compare {\"a\":\"Bengaluru\",\"b\":\"Chennai\"}",
        verify: "Every value in a comparison carries its own fact_cid. Resolve one and confirm it matches the number you were shown, rather than trusting the aggregate.",
    },
    Intent {
        need: "I need state that survives context compaction",
        also: &[
            "I need a checkpoint before my context window ends",
            "I need to remember this after I am compacted",
            "I need durable scratch memory for myself",
        ],
        short: "State that outlives your context window",
        capability: "durable notes plus a bundle token you carry forward",
        coverage: Coverage::Partial,
        why: "There is no private per-session checkpoint. /openapi.json routes no session, checkpoint or compaction endpoint, and a write lands in /memories/by_attester/<your pk8>/ which is world-readable. What works is the durable half: write the note, take a token, and dereference it after compaction. What does not work is privacy, and automatic capture, which you must do yourself before the window ends.",
        instead: "Use it as a handoff to your future self that anyone can also read. If the content must stay private, keep it in your own store, or self-host via /v1/guard/selfhost. Do not route secrets here on the strength of the word 'memory'.",
        tool: "emem_memory_create",
        rest: "",
        first_call: "emem_memory_create (MCP) to write the note, then emem_memory_bundle to get one token to carry across the boundary",
        verify: "Resolve the token in a fresh context with no prior state and confirm you get the note back. Then fetch the same path anonymously to see for yourself that it is public.",
    },
    Intent {
        need: "I need to write memory over plain HTTP, without an MCP client",
        also: &[
            "I need a REST write path",
            "my runtime does not speak MCP",
            "can I curl a write",
        ],
        short: "Write without an MCP client",
        capability: "memory write verbs",
        coverage: Coverage::Partial,
        why: "Reads are fully available over REST, writes are not: emem_memory_create, insert, str_replace, rename and delete are MCP-only and have no REST twin. /openapi.json lists no write path, so a client that only speaks HTTP can read everything and write nothing.",
        instead: "Attest facts over REST with POST /v1/attest, which is the signed write path that does have an HTTP door. For note-shaped memory, speak MCP at /mcp/full.",
        tool: "emem_memory_create",
        rest: "",
        first_call: "POST /v1/attest for facts; MCP /mcp/full for notes",
        verify: "Fetch /openapi.json and search for a write path. Finding none is the point; this row exists so you learn that here rather than from a 404.",
    },
    Intent {
        need: "I need memory that is private to me",
        also: &[
            "I need owner-scoped reads",
            "nobody else should see this",
            "I need per-tenant isolation",
        ],
        short: "Memory only you can read",
        capability: "",
        coverage: Coverage::No,
        why: "There are no owner-scoped reads. Every note and fact written here is readable by anyone, by design: the protocol's value is that a third party can check a claim without trusting the writer, and that property is incompatible with a private read. Writes are namespace-scoped to your key, which controls who may WRITE where, not who may read.",
        instead: "Self-host with /v1/guard/selfhost, or keep private state in your own store and publish only the claims you want checkable. Namespace scoping stops another agent writing as you; it does not hide anything.",
        tool: "",
        rest: "",
        first_call: "",
        verify: "Write a note, then fetch its path with no credentials at all. You will get it back. Confirm the limit rather than believing this row.",
    },
    Intent {
        need: "I need to participate without managing signing keys",
        also: &[
            "I do not want to hold a private key",
            "can the server sign for me",
            "I need zero-setup writes",
        ],
        short: "Take part without holding a key",
        capability: "",
        coverage: Coverage::No,
        why: "Writing requires an ed25519 key you hold and a per-verb preimage you sign; there is no server-side signing route, and there should not be. A signature the responder could produce on your behalf would attest nothing about you, which would make every downstream verification meaningless.",
        instead: "Read freely: every read path is open and needs no key, no token and no account. Generate a keypair only when you want to write. Bearer tokens exist on the OAuth surface but gate nothing.",
        tool: "",
        rest: "",
        first_call: "",
        verify: "Call any read endpoint with no headers and watch it answer. The asymmetry is deliberate: reading is free, asserting costs a key.",
    },
];

/// Serialise the registry. `trigger_phrases` on the agent card is generated
/// from this same table, so the phrasings an agent matches on cannot drift
/// away from the capabilities they claim to reach.
pub fn registry() -> JsonValue {
    let rows: Vec<JsonValue> = INTENTS
        .iter()
        .map(|i| {
            let mut row = json!({
                "need": i.need,
                "short": i.short,
                "also_phrased": i.also,
                "capability": i.capability,
                "coverage": i.coverage.as_str(),
            });
            let m = row.as_object_mut().expect("object");
            if i.coverage != Coverage::Yes {
                m.insert("why".into(), json!(i.why));
                m.insert("instead".into(), json!(i.instead));
            }
            if !i.tool.is_empty() {
                m.insert("tool".into(), json!(i.tool));
            }
            if !i.rest.is_empty() {
                m.insert("rest".into(), json!(i.rest));
            } else if i.coverage != Coverage::No {
                m.insert(
                    "rest".into(),
                    json!("none: this verb is MCP-only, see the REST-write row"),
                );
            }
            if !i.first_call.is_empty() {
                m.insert("first_call".into(), json!(i.first_call));
            }
            if !i.verify.is_empty() {
                m.insert("verify".into(), json!(i.verify));
            }
            row
        })
        .collect();

    let served = INTENTS
        .iter()
        .filter(|i| i.coverage == Coverage::Yes)
        .count();

    json!({
        "protocol": "emem.intents",
        "version": 1,
        "summary": "What agents need, in the words agents use, mapped to the capability, the endpoint and the way to check it worked. Read this before /openapi.json: that document says which routes exist, this one says which needs they answer.",
        "read_this_if": "You are deciding WHETHER to use emem. If you have already decided, /.well-known/emem.json has the endpoints and /v1/tools has the catalogue.",
        "how_to_match": "Match a caller's need against `need` and `also_phrased`. On a hit, read `coverage` FIRST: a `partial` or `not_served` row means the honest answer is a caveat or a redirect, not a call. Then `first_call`, then `verify`.",
        "counts": {
            "total": INTENTS.len(),
            "served": served,
            "partial": INTENTS.iter().filter(|i| i.coverage == Coverage::Partial).count(),
            "not_served": INTENTS.iter().filter(|i| i.coverage == Coverage::No).count(),
        },
        "why_the_gaps_are_listed": "An index that only lists strengths makes the caller discover the limits after committing. Three rows here say emem is the wrong tool, each naming the missing mechanism and where to go instead. They were measured against this responder, not recalled from a design document.",
        "intents": rows,
        "next": {
            "endpoints": "/.well-known/emem.json",
            "tool_catalogue": "/v1/tools",
            "openapi": "/openapi.json",
            "mcp": "/mcp (16-tool core loop), /mcp/full (the rest)",
        },
    })
}

/// The same table in the shape a discovery crawler expects: capability first,
/// with the situations that should route to it.
///
/// A separate rendering, not a separate source. The measured problem with
/// this responder's discovery was never that it lacked surfaces, it was that
/// it had six of them totalling half a megabyte and no single answer to
/// "should I use this at all". Adding a seventh hand-written file would make
/// that worse; this one is `registry()` reshaped, so the two cannot disagree.
///
/// Unserved rows are carried through as `provider: null` with the reason
/// attached. A crawler that indexes only the positive rows still gets a
/// correct index; one that reads everything learns where not to send an agent,
/// which is the more useful fact and the one an advertisement would omit.
pub fn agent_intent_document() -> JsonValue {
    let caps: Vec<JsonValue> = INTENTS
        .iter()
        .map(|i| {
            let mut when: Vec<&str> = vec![i.need];
            when.extend_from_slice(i.also);
            let mut row = json!({
                "intent": intent_slug(i.need),
                "capability": i.capability,
                "when": when,
                "provider": if i.coverage == Coverage::No { JsonValue::Null }
                            else { json!("emem") },
                "coverage": i.coverage.as_str(),
            });
            let m = row.as_object_mut().expect("object");
            if i.coverage != Coverage::Yes {
                m.insert("why_not".into(), json!(i.why));
                m.insert("use_instead".into(), json!(i.instead));
            }
            if !i.tool.is_empty() {
                m.insert("tool".into(), json!(i.tool));
            }
            if !i.first_call.is_empty() {
                m.insert("first_call".into(), json!(i.first_call));
            }
            row
        })
        .collect();
    json!({
        "schema": "emem.agent_intent.v1",
        "provider": {
            "name": "emem",
            "what": "Shared, verifiable memory for AI agents. One place, one content-addressed address; one observation, one signed fact; one object, one citeable identity. Every read returns an ed25519 receipt any agent verifies offline, without trusting the responder.",
            "mcp": "https://emem.dev/mcp",
            "reads_need_no_key": true,
            "intents_url": "/v1/intents",
        },
        "note": "Generated from the same table as /v1/intents. Rows with a null provider are needs emem does NOT serve, kept here with the reason, because an index that lists only strengths makes a caller discover the limits after committing.",
        "capabilities": caps,
    })
}

/// A stable machine key for a need, derived from the need itself so it cannot
/// be assigned inconsistently by hand.
fn intent_slug(need: &str) -> String {
    let mut s = String::new();
    for w in need
        .to_lowercase()
        .split_whitespace()
        .filter(|w| {
            !matches!(
                *w,
                "i" | "need"
                    | "to"
                    | "a"
                    | "the"
                    | "that"
                    | "for"
                    | "of"
                    | "it"
                    | "can"
                    | "so"
                    | "at"
                    | "in"
                    | "my"
                    | "me"
                    | "another"
                    | "and"
                    | "or"
                    | "is"
                    | "are"
                    | "with"
                    | "without"
            )
        })
        .take(4)
    {
        let cleaned: String = w.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        if cleaned.is_empty() {
            continue;
        }
        if !s.is_empty() {
            s.push('_');
        }
        s.push_str(&cleaned);
    }
    s
}

/// The phrasings a router should treat as "consider emem". Generated from the
/// served rows only: an agent must never be steered here by a phrase whose
/// row says we do not serve it.
pub fn trigger_phrases() -> Vec<&'static str> {
    let mut out = Vec::new();
    for i in INTENTS.iter().filter(|i| i.coverage == Coverage::Yes) {
        out.push(i.need);
        out.extend_from_slice(i.also);
    }
    out
}

#[cfg(test)]
mod intents_test {
    use super::*;

    /// The rule that makes this a contract. A row that admits a limit without
    /// naming the missing mechanism, or without saying where to go instead,
    /// is worse than no row: it tells an agent "no" and strands it.
    #[test]
    fn every_limit_states_why_and_what_instead() {
        for i in INTENTS.iter().filter(|i| i.coverage != Coverage::Yes) {
            assert!(
                i.why.len() > 40,
                "{}: a partial/unserved row owes a specific reason",
                i.need
            );
            assert!(
                i.instead.len() > 20,
                "{}: a partial/unserved row owes somewhere else to go",
                i.need
            );
        }
    }

    /// A served row that cannot be checked is an advertisement. The whole
    /// protocol argues that a claim should be verifiable by the reader, and
    /// the index of those claims does not get an exemption.
    #[test]
    fn every_served_intent_is_callable_and_checkable() {
        for i in INTENTS.iter().filter(|i| i.coverage == Coverage::Yes) {
            assert!(!i.tool.is_empty(), "{}: served but no tool", i.need);
            assert!(
                !i.first_call.is_empty(),
                "{}: served but nothing to call",
                i.need
            );
            assert!(
                i.verify.len() > 40,
                "{}: served but no way to check it worked",
                i.need
            );
        }
    }

    /// Never route an agent to us on a phrase we do not serve. The generated
    /// trigger list is the enforcement: it is built from served rows only, so
    /// this test fails the moment someone hand-adds a phrase back.
    #[test]
    fn triggers_never_advertise_an_unserved_need() {
        let triggers = trigger_phrases();
        for i in INTENTS.iter().filter(|i| i.coverage != Coverage::Yes) {
            assert!(
                !triggers.contains(&i.need),
                "{}: an unserved need is being advertised as a trigger",
                i.need
            );
            for alt in i.also {
                assert!(
                    !triggers.contains(alt),
                    "{alt:?}: phrasing of an unserved need is advertised as a trigger"
                );
            }
        }
        assert!(triggers.len() > 20, "the trigger list collapsed");
    }

    /// The need is the match key, so it has to be phrased as a need. A row
    /// keyed on our vocabulary ("entity resolution") only matches an agent
    /// that already knows our vocabulary, which is precisely the agent this
    /// registry is not for.
    #[test]
    fn needs_are_phrased_as_needs_not_as_feature_names() {
        for i in INTENTS {
            let n = i.need.to_lowercase();
            assert!(
                n.starts_with("i need") || n.starts_with("i do not") || n.starts_with("what"),
                "{}: phrase the row as the caller's need, not as our feature",
                i.need
            );
            assert!(
                !n.contains("emem_"),
                "{}: the need must not name our tool; the tool is the answer, not the question",
                i.need
            );
        }
    }

    /// Every row owes a display label, and it has to be short enough to scan.
    /// The decision layer is generated from these; a missing or sprawling one
    /// makes the guide's first screen a wall of prose, which is the failure
    /// that screen exists to fix.
    #[test]
    fn every_intent_has_a_scannable_label() {
        for i in INTENTS {
            assert!(
                !i.short.is_empty(),
                "{}: no display label; the decision layer would fall back to \
                 the matching sentence",
                i.need
            );
            assert!(
                i.short.len() <= 44,
                "{}: label {:?} is {} chars; a table of these stops being \
                 scannable past about 44",
                i.need,
                i.short,
                i.short.len()
            );
            assert!(
                !i.short.starts_with("I need"),
                "{}: the label is the summary form, not the matching form",
                i.need
            );
        }
    }

    #[test]
    fn registry_serialises_with_the_gaps_visible() {
        let r = registry();
        let rows = r["intents"].as_array().expect("array");
        assert_eq!(rows.len(), INTENTS.len());
        let unserved: Vec<_> = rows.iter().filter(|x| x["coverage"] != "served").collect();
        assert!(
            unserved.len() >= 3,
            "the measured limits went missing from the serialised registry"
        );
        for u in unserved {
            assert!(u["why"].is_string() && u["instead"].is_string());
        }
        assert!(r["counts"]["served"].as_u64().unwrap() >= 8);
    }
}
