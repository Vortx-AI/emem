# Connect & evolve

> **Status.** Typed temporal edges (`/v1/edges`, `emem_edges_recall`)
> and multi-attester contradiction scoring (`memory_contradictions`)
> shipped in v0.0.9 and answer on emem.dev. The deterministic refinement
> loop ships in the same code but is opt-in (`EMEM_REFINEMENT_ENABLED=1`),
> and emem.dev does not enable it, so no edge there was drawn by it.
> Everything here is signed and non-destructive by design. The runnable
> walkthrough, against a local responder, is
> [examples/connect-and-evolve.md](https://github.com/Vortx-AI/emem/tree/main/examples/connect-and-evolve.md).

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
- `pred`: the relationship, a free-form string (e.g. `disagrees_with`,
  `supersedes`, `replaced_by`, `co_located_with`).
- `valid_from`, `valid_to`: the window the relationship held. An open
  `valid_to` means "still true".

Read edges with `POST /v1/edges/recall`. It takes a valid-time bound,
`as_of_tslot`, which returns the latest edge per neighbour whose
`[valid_from, valid_to)` covers that tslot:

```bash
curl -s -X POST https://emem.dev/v1/edges/recall \
  -H 'content-type: application/json' \
  -d '{"subj":"l7hm437whbei33v37ntoxydt572ac7ldkopotvuu67idpdu5nxcq",
       "pred":"disagrees_with"}' \
  | jq '{edges: (.edges|length), hint: .agent_hint}'
```

`subj` is a fact CID, not a cell. The one above is a real fact (the JRC
2020 forest flag near Soubré) and it has no edges yet, so this returns
`edges: []` with an `agent_hint` saying so. That is the honest "no
relation known", not a lookup failure: `/v1/edges/recall` rejects an
empty or ambiguous request with a 400 rather than returning a silent
empty array.

To read the other direction, what points AT a fact (what disagrees with
it, supersedes it or relates to it), send `obj` instead of `subj`, or set
`direction: "in"`. Set exactly one of the two. The response carries
`direction`, `objs`, and for a reverse read `subjs`:

```bash
curl -s -X POST https://emem.dev/v1/edges/recall \
  -H 'content-type: application/json' \
  -d '{"obj":"l7hm437whbei33v37ntoxydt572ac7ldkopotvuu67idpdu5nxcq"}' \
  | jq '{direction, edges: (.edges|length), subjs}'
```

For this fact that prints `direction: "in"`, `edges: 0`, `subjs: []`:
nothing points at it yet.

The MCP tool is `emem_edges_recall`. The recall receipt is signed like
every other and commits the returned edge CIDs into its preimage, so it
verifies offline the same way.

Edges are written by `POST /v1/edges`: a signed attestation envelope
whose `edges[]` leaves fold into the Merkle root the signature covers.
Unlike facts, edges key no `(cell, band, tslot)` address, so a caller's
own key can write them. That is how agent B records that it disagrees
with agent A's fact. One gap to know: `emem_memory_view` does not show
inbound edges on a note, so a reader has to ask `edges/recall` with
`obj` to see a refutation.

### Bi-temporal supersession: newer shadows older, nothing is deleted

When a newer edge with a later `valid_from` arrives for the same
`(subj, pred, obj)`, it **shadows** the older one rather than replacing
it. A query with `as_of_tslot` in 2025 still sees the edge that was true
in 2025; one in 2027 sees the newer one. History is never overwritten,
so an audit can always replay the state as of any date.

### Attach a fact's edges in one call

Add `include: ["edges"]` to a recall and the matching edges ride back
with the fact, so an agent does not need a second round trip:

```bash
curl -s -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb441.zd21e.zd3ee","band":"indices.ndvi","include":["edges"]}' \
  | jq '{value: .facts[0].value, edges: .edges}'
```

The edges ride at the top level of the response, one `edges` array for
all returned facts, not inside each fact. Asked for and absent, it is
`edges: []`, which is what this cell returns today; not asked for, the
key is not there at all, and the response is byte-identical to a recall
from before edges existed. When edges are attached, their CIDs are
threaded into the receipt. When the refinement loop has flagged a
returned fact, the response also carries a top-level `contested` array
naming it; that note is advisory and sits outside the signed surface.

## The refinement loop: how the memory evolves

**What it does.** When the loop is on and attesters disagree, it records
the disagreement as a link and marks the contested fact for another
look, so conflicts are kept visible rather than resolved silently. Its
input is the contradiction scan, which by default counts only distinct
attesters. The fact plane admits few of those (the responder, enrolled
devices and an operator allowlist), so on a single-responder corpus the
loop has little to act on.

The loop is opt-in (`EMEM_REFINEMENT_ENABLED`, tuned by
`EMEM_REFINEMENT_INTERVAL_SECS`, `EMEM_REFINEMENT_MIN_SEVERITY`,
`EMEM_REFINEMENT_CELL_PREFIX` and `EMEM_REFINEMENT_MAX_PAIRS`) and
non-destructive:

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

## What this adds

A plain store remembers facts. This layer lets facts **connect** (typed
temporal edges between them) and lets the memory **evolve** (a loop
that turns multi-attester disagreement into recorded edges and
re-attestation flags). Both are append-only and signed, so a change in
what the memory holds never removes the record of what it held before.

## Where this is going

The edges and the refinement loop run inside one responder today. Because
every edge and every contradiction is itself a content-addressed, signed
fact, the same machinery extends to a federation of independent
responders: nodes that resolve the same content ids byte-for-byte could
cross-cite each other's attestations and record where they disagree
across hosts, so the disagreement graph spans the network rather than one
store. What federation ships today is mutual witnessing of transparency
log heads between nodes ([federation](./federation.md)); read federation,
where one node answers from another's facts, does not exist yet, so a
disagreement graph still lives on one responder.
