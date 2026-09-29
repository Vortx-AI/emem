# What only emem does

Most memory stores let an agent save a note and search it later. emem
does that too. These three things are what a plain vector store does
not give you. They are three faces of one job, keeping a shared account
of a world that changes checkable by parties who do not trust each
other. Each is framed as a benefit first, then the surface that
delivers it.

## 1. It surfaces disagreement, with a severity score

**The benefit.** Three agents observed the same place. One disagreed.
emem does not silently pick a winner or average them away. It tells you
they disagree and how badly.

When several attesters sign facts at the same `(cell, band, tslot)`,
emem keeps all of them and scores the spread. Since 2026-09-14 only three
kinds of key may occupy an address: this responder, a device enrolled
through the OS-trace gate, and a key the operator lists. So on this node
the attesters that can disagree at one address are those, plus whatever
was signed before the plane closed. A scalar band is scored by
how far apart the numbers are; a vector band by mean cosine distance; a
categorical band by how lopsided the votes are. The agent gets a single
severity number it can threshold on: ignore a hairline disagreement,
escalate a real one.

```bash
curl -s -X POST https://emem.dev/v1/memory_contradictions \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb441.zd21e.zd3ee","band":"indices.ndvi"}' \
  | jq '.contradictions[] | {cell, band, severity, kind,
                             attesters: [.attestations[].attester_pubkey_b32]}'
```

`severity` and the attester list sit on each entry of `contradictions`,
not at the top of the response. The top level carries the scan
accounting instead: `corpus_scanned`, `scan_truncated`, `agent_hint` and
the signed `receipt`. An empty `contradictions` array with a non-zero
`corpus_scanned` is the honest "these attesters agree", not a lookup
failure, and it is what this call returns today: the live corpus holds
no `indices.ndvi` disagreement above the default threshold. Pass
`min_severity: 0.0` to see borderline ones.

On a corpus with one main writer, most real disagreement is one attester
answering the same address from two upstreams. The default scan does not
count that. Pass `include_same_attester_sources: true` and a key qualifies
when its facts differ in `derivation.fn_key` or in their `sources[].scheme`
set; each record then carries `disagreement_scope` and a `providers[]` list
naming the recipe behind each value. A re-signing from the same provider is
a refresh, not a disagreement, and is still not counted. On 2026-09-29 a
whole-corpus `indices.ndvi` scan with both options (37,759 keys) returned
none.

An agent that thinks a fact is wrong does not overwrite it. It signs a
`disagrees_with` edge between the two fact cids with its own key
(`POST /v1/edges`, a signed attestation envelope; edges take no address, so
any signed key may write one), and anyone can read it with
`emem_edges_recall`. The disputed fact is left as it was.

Why it matters: a guess from one model looks identical to a consensus of
three. The result is visible to the agent. (MCP: `memory_contradictions`.)

## 2. It answers "what did we believe on date X"

**The benefit.** You can ask the memory two different time questions and
get two different, correct answers, without keeping snapshots yourself.

Every read carries a bi-temporal axis: two independent time knobs.

- `as_of_tslot`: *what the world looked like* on that date. It returns
  the latest fact whose observation time is on or before the bound.
- `as_of_signed_at`: *what emem knew* on that date. It returns the
  latest fact whose signing time is on or before the bound.

Set both and both hold at once. So an auditor in 2027 can take a 2026
receipt, replay the exact query against any peer, and reproduce the same
answer the agent saw back then. The receipt carries an `as_of` block
when a bound is set, so old receipts still verify byte-for-byte.

Why it matters: compliance and incident reviews ask "what did you know,
and when". emem answers that as a first-class query, not as a forensic
reconstruction.

## 3. Writes can be locked to one signer

**The benefit.** An agent's own memory can be made tamper-evident: only
the agent that owns a path can write to it, and any byte change is
detectable.

A write to a path under `/memories/by_attester/<pubkey8>/...` (the first
eight characters of the key) must carry an ed25519 signature from that
exact key: a capability-bound write. A signer that is not the owner is
refused 403 `memory_namespace_violation`. The signature is over the
`emem.memory_write.v2` preimage, which binds the verb, the path, the body
hash and the file cid being replaced, so a signature copied off the public
log cannot be replayed once the file has moved on. Combined with content
addressing (the fingerprint changes if a byte changes), this means a
reader can confirm both *who* wrote a memory and that *nobody altered it
since*.

Why it matters: the same signing surface that proves a Sentinel-2
reading is real also proves an agent's notes are unmodified. The notes
are not private: anyone can read them, so write nothing you would not
publish. One trust surface, two layers of memory.

## In one line

emem keeps disagreement instead of hiding it and keeps history instead of
overwriting it. It also keeps proof of authorship, so you do not have to
trust the store. See [Connect & evolve](./connect-and-evolve.md) for how
these combine into a memory that links facts and improves over time.
