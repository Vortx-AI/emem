# What only emem does

Most memory stores let an agent save a note and search it later. emem
does that too. These three things are what a plain vector store does
not give you. Each is framed as a benefit first, then the surface that
delivers it.

## 1. It surfaces disagreement, with a severity score

**The benefit.** Three agents observed the same place. One disagreed.
emem does not silently pick a winner or average them away — it tells you
they disagree and how badly.

When several attesters sign facts at the same `(cell, band, tslot)`,
emem keeps all of them and scores the spread. A scalar band is scored by
how far apart the numbers are; a vector band by mean cosine distance; a
categorical band by how lopsided the votes are. The agent gets a single
severity number it can threshold on: ignore a hairline disagreement,
escalate a real one.

```bash
curl -s -X POST https://emem.dev/v1/memory_contradictions \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.q7k2x.m4ph.aa913","band":"indices.ndvi"}' \
  | jq '{severity, attesters: [.observations[].attester]}'
```

Why it matters: a guess from one model looks identical to a consensus of
three. The result is visible to the agent. (MCP: `memory_contradictions`.)

## 2. It answers "what did we believe on date X"

**The benefit.** You can ask the memory two different time questions and
get two different, correct answers — without keeping snapshots yourself.

Every read carries a bi-temporal axis: two independent time knobs.

- `as_of_tslot` — *what the world looked like* on that date. It returns
  the latest fact whose observation time is on or before the bound.
- `as_of_signed_at` — *what emem knew* on that date. It returns the
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

A write to a path under `/memories/by_attester/<pubkey>/...` must carry
an ed25519 signature from that exact key — a capability-bound write. A
signer that is not the owner is rejected. Combined with content
addressing (the fingerprint changes if a byte changes), this means a
reader can confirm both *who* wrote a memory and that *nobody altered it
since*.

Why it matters: the same signing surface that proves a Sentinel-2
reading is real also proves an agent's private notes are unmodified. One
trust surface, two layers of memory.

## In one line

emem keeps disagreement instead of hiding it and keeps history instead of
overwriting it. It also keeps proof of authorship, so you do not have to
trust the store. See [Connect & evolve](./connect-and-evolve.md) for how
these combine into a memory that links facts and improves over time.
