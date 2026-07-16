# Connect & evolve

> **Status: ships in v0.0.9.** Typed temporal edges
> (`emem_edges_recall`), multi-attester contradiction scoring
> (`memory_contradictions`), and the deterministic refinement loop
> (`EMEM_REFINEMENT_ENABLED`) are live on the running responder.
> Everything here is signed and non-destructive by design.

A pile of facts is not yet a memory. A memory connects things and gets
better as it learns. This layer adds two abilities: facts **link** to
each other through typed, time-bounded edges, and the corpus **evolves**
by turning disagreement into recorded links and re-checks, without ever
deleting what it had before.

## Typed temporal edges

A fact answers "what is at this place". An **edge** answers "how does
this relate to that". An edge is itself a signed fact:

```
EdgeFact(subj, pred, obj, valid_from, valid_to)
```

- `subj`, `obj`: the two things being related, each named by its
  content id.
- `pred`: the relationship, a plain string (e.g. `observed_at`,
  `disagrees_with`, `supersedes`).
- `valid_from`, `valid_to`: the window the relationship held. An open
  `valid_to` means "still true".

Read edges with a query that takes the same bi-temporal `as_of` rule as
band facts:

```bash
curl -s -X POST https://emem.dev/v1/edges/recall \
  -H 'content-type: application/json' \
  -d '{"subj":"qi3jo4sqcg…l2hgjtwm","pred":"disagrees_with"}' \
  | jq '.edges[] | {pred, obj, valid_from, valid_to}'
```

The MCP tool is `emem_edges_recall`. Edges are signed under the same
receipt envelope as band facts: no new key material, and they verify
offline the same way.

### Bi-temporal supersession: newer shadows older, nothing is deleted

When a newer edge with a later `valid_from` arrives for the same
`(subj, pred, obj)`, it **shadows** the older one rather than replacing
it. A query at `as_of:2025` still sees the edge that was true in 2025; a
query at `as_of:2027` sees the newer one. History is never overwritten,
so an audit can always replay the state as of any date.

### Attach a fact's edges in one call

Add `include: ["edges"]` to a recall and the matching edges ride back
with the fact, so an agent does not need a second round trip:

```bash
curl -s -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.q7k2x.m4ph.aa913","band":"indices.ndvi","include":["edges"]}' \
  | jq '{value: .facts[0].value, edges: .facts[0].edges}'
```

## The refinement loop: how the memory evolves

**The benefit.** When attesters disagree, emem records the disagreement
as a link and marks the contested fact for another look. The corpus
tightens over time instead of accumulating silent conflicts.

The loop is opt-in (`EMEM_REFINEMENT_ENABLED`) and non-destructive:

1. The contradiction scorer (see
   [What only emem does](./only-emem.md)) finds attesters that disagree
   at the same `(cell, band, tslot)`.
2. The loop writes a `disagrees_with` **edge** between the conflicting
   facts, with a `valid_from` of now. The original facts are untouched.
3. It flags the contested fact for **re-attestation**, a signal that
   this value is worth observing again.

Because every step is an edge or a flag (never a deletion), the whole
history stays verifiable: you can see that two attesters disagreed, when
the disagreement was recorded, and whether a later observation resolved
it. Nothing is silently reconciled, and every edge in the chain is
signed.

## Why this is the differentiator

A plain store remembers facts. This layer makes the memory **connect**
(typed temporal edges between facts) and **evolve** (a feedback loop
that converts multi-attester disagreement into recorded edges and
re-attestation flags). Both are append-only and signed, so a memory that
improves over time never costs you the ability to audit what it used to
believe.

## Where this is going

The edges and the refinement loop run inside one responder today. Because
every edge and every contradiction is itself a content-addressed, signed
fact, the same machinery extends to a federation of independent
responders: nodes that resolve the same content ids byte-for-byte could
cross-cite each other's attestations and record where they disagree
across hosts, so the disagreement graph spans the network rather than one
store. The multi-host routing that would make that automatic does not
ship yet in the 1.x line. What ships today is the substrate it builds on: signed
typed edges, multi-attester contradiction scoring, and the deterministic
refinement loop, all on a single responder.
