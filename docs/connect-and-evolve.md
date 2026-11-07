# Connect & evolve

This page shows how emem links facts to each other and how it records
disagreement. The short summary for an agent is in
[Connect facts and revise them](agents.md#connect-facts-and-revise-them).
A runnable walkthrough against a local responder is in
[examples/connect-and-evolve.md](../examples/connect-and-evolve.md).

## What ships and what does not

| Part | State on emem.dev |
|---|---|
| Typed edges: read with `POST /v1/edges/recall` or `emem_edges_recall` | Ships. All calls on this page ran on 7 November 2026. |
| Edges on a recall: `include: ["edges"]` on `POST /v1/recall` | Ships. |
| Write edges: `POST /v1/edges` | Ships. It needs a signed attestation. |
| Contradiction scan: `emem_memory_contradictions` | Ships. See [Find where sources disagree](agents.md#job-find-where-sources-disagree). |
| Refinement loop | In the code, but off by default (`EMEM_REFINEMENT_ENABLED`). emem.dev does not turn it on, so no edge there comes from the loop. |
| Edges across responders | Roadmap. See [Edges across responders](#edges-across-responders). |

No step on this page deletes or changes a fact. emem adds edges and
markers next to the facts.

## What an edge is

A fact tells what is at a place. An edge tells how one fact relates to
another fact. An edge is a signed record with five main fields:

```
EdgeFact(subj, pred, obj, valid_from, valid_to)
```

| Field | Meaning |
|---|---|
| `subj` | The content id (CID) of the fact that the edge starts at. |
| `pred` | The relation, a free string. Examples: `disagrees_with`, `supersedes`, `replaced_by`, `co_located_with`. |
| `obj` | The CID of the fact that the edge points to. |
| `valid_from` | The first tslot in which the relation is true. |
| `valid_to` | The last tslot in which the relation is true. No value means "still true". |

An edge also carries `confidence`, `signer`, `signed_at` and an optional
`note`.

## Read the edges of a fact

Send the fact CID as `subj`. The example fact is the JRC 2020 forest flag
(`jrc_gfc2020.forest_2020`, value 0) at cell `defi.zb441.zd21e.zd3ee`.
A recall of that cell and band returned this CID on 7 November 2026. The
responder signed the fact on 28 September 2026 from the JRC V4 tile.

```json
{"name": "emem_edges_recall",
 "arguments": {"subj": "ygcje35skz5dmiyzs5xt3oayakjpizemnpkvnpqvasnwa53qntza",
               "pred": "disagrees_with"}}
```

```bash
curl -s -X POST https://emem.dev/v1/edges/recall \
  -H 'content-type: application/json' \
  -d '{"subj":"ygcje35skz5dmiyzs5xt3oayakjpizemnpkvnpqvasnwa53qntza",
       "pred":"disagrees_with"}' \
  | jq '{direction, edges: (.edges|length), hint: .agent_hint}'
```

On 7 November 2026 this fact had no edges, with or without `pred`. The answer was
`direction: "out"`, `edges: 0` and an `agent_hint` that says "no relation
known". An empty list means that no edge exists. It is not a failure.

Leave out `pred`, or send `""`, to read edges with any relation. Add
`as_of_tslot` to read the edges that were true at that tslot.

The responder signs the recall receipt like all other receipts. Its preimage
includes the CIDs of the returned edges, so you can verify it offline.
See [Verify a receipt](agents.md#job-verify-a-receipt).

## Read the edges that point at a fact

Send the CID as `obj` to find the facts that point at it. Such a fact can
disagree with it, supersede it or relate to it in another way.

```json
{"name": "emem_edges_recall",
 "arguments": {"obj": "ygcje35skz5dmiyzs5xt3oayakjpizemnpkvnpqvasnwa53qntza"}}
```

```bash
curl -s -X POST https://emem.dev/v1/edges/recall \
  -H 'content-type: application/json' \
  -d '{"obj":"ygcje35skz5dmiyzs5xt3oayakjpizemnpkvnpqvasnwa53qntza"}' \
  | jq '{direction, edges: (.edges|length), subjs}'
```

On 7 November 2026 this printed `direction: "in"`, `edges: 0` and
`subjs: []`. No edge pointed at the fact.

Send `subj` or `obj`, not both. `direction` is `"out"` for `subj` and
`"in"` for `obj`. The route returns HTTP 400 `invalid_argument` for these
requests:

| Request | Error message (short form) |
|---|---|
| `{}` | `direction="out" requires a non-empty subj` |
| `subj` and `obj` together | `ambiguous request`. Set exactly one of `subj` (out) or `obj` (in). |
| `subj` with `direction: "in"` | `direction="in" requires a non-empty obj` |

`emem_memory_view` does not show the edges that point at a note. To find
a reply that disagrees with a note, send the note CID as `obj`.

## Get the edges with a recall

Add `include: ["edges"]` to a recall. The edges come back in the same
answer, so you do not need a second call.

```json
{"name": "emem_recall",
 "arguments": {"cell": "defi.zb441.zd21e.zd3ee", "band": "indices.ndvi",
               "include": ["edges"]}}
```

```bash
curl -s -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb441.zd21e.zd3ee","band":"indices.ndvi","include":["edges"]}' \
  | jq '{value: .facts[0].value, edges: .edges}'
```

On 7 November 2026 both calls returned a first value of
0.5138218516893375 and `edges: []`.

| You send | The answer |
|---|---|
| `include: ["edges"]`, and edges exist | One top-level `edges` array for all returned facts. The receipt signs the edge CIDs. |
| `include: ["edges"]`, and no edges exist | `edges: []`. |
| No `include` | No `edges` key. The answer and the receipt are the same as before edges existed. |

If the refinement loop marked a returned fact, the answer also has a
top-level `contested` array that names it. This array is advice only. It
is not in the receipt preimage, so a recall before and after the mark
signs the same receipt.

## Write an edge

This is a signed write. It needs your own Ed25519 key, and this page does
not run it.

Send a signed attestation envelope to `POST /v1/edges`. Its `edges[]`
array must not be empty. The edge leaves go into the Merkle root that the
signature covers. The answer gives `edge_cids` and `edge_count`.

An edge has no `(cell, band, tslot)` address. Thus a caller can sign an
edge with its own key. For example, a second agent can record that it
disagrees with a fact from the first agent. Do not use the steps for a
signed note here, because a note signs a different digest. The envelope
and its preimage are in
[protocol section 6](protocol.md#6-attestation-envelope). The worked example in
[examples/connect-and-evolve.md](../examples/connect-and-evolve.md) reads
edges back and describes this write.

## How a newer edge hides an older edge

Two edges can have the same `subj`, `pred` and `obj`. Then the edge with
the later `valid_from` hides the older edge. emem keeps both edges.

`as_of_tslot` selects the edges that were true at one tslot:

1. emem keeps the edges with `valid_from` less than or equal to the tslot.
2. emem removes the edges with a `valid_to` less than the tslot. An edge
   is still true at its `valid_to` tslot.
3. For each neighbour fact, emem returns the edge with the largest
   `valid_from`.

Without `as_of_tslot`, step 3 runs on all edges. Thus a read for a 2025
tslot shows the edge that was true in 2025, and a read for 2027 shows the
newer edge. An audit can read the state at any past tslot.

## The refinement loop

The refinement loop turns disagreement into edges. It is in the code, but
it is off by default, and emem.dev does not turn it on.

When the loop runs, it does these steps:

1. It runs the contradiction scan. The scan counts only different
   attesters. It leaves out one attester that changed its upstream source.
2. It skips each contradiction with a severity below the minimum.
3. For each remaining contradiction, it pairs the fact with the lowest
   confidence and the fact with the highest confidence.
4. It writes a signed `disagrees_with` edge between the two facts. `subj`
   is the smaller CID. `valid_from` is the tslot of the contradiction.
   The edge `confidence` is the severity of the contradiction, and the
   responder key signs the edge.
5. It marks the fact with the lower confidence as contested. A recall
   shows this mark in `contested`.

The loop does not change the original facts. It skips an edge that
already exists with the same `subj`, `obj` and `valid_from`, so a second
run writes no new edges.

| Variable | Default | Effect |
|---|---|---|
| `EMEM_REFINEMENT_ENABLED` | off | `1` turns the loop on. Any value other than empty, `0`, `false` or `no` also turns it on. |
| `EMEM_REFINEMENT_INTERVAL_SECS` | 86400 (one day) | Time between runs. Minimum 1. |
| `EMEM_REFINEMENT_MIN_SEVERITY` | 0.5 | Lower severities make no edge. Range 0 to 1. |
| `EMEM_REFINEMENT_CELL_PREFIX` | none | Limits the scan to cells with this cell64 prefix. |
| `EMEM_REFINEMENT_MAX_PAIRS` | 1 | Pairs written for each contradiction. Minimum 1. |

The fact plane admits few attesters: the responder, enrolled devices and
an operator allow list. Thus on a corpus with one responder, the loop has
few contradictions to act on.

## Edges across responders

Today the edges and the refinement loop work inside one responder. Each
edge and each contradiction has a signature and a content id. Thus a
federation of responders could, in a later version, cite edges from each other and
record disagreement across hosts. This does not ship.

What federation ships today is mutual witnessing of transparency log
heads between nodes. See [federation](./federation.md). Read federation,
where one node answers from the facts of another node, does not exist.
Thus a disagreement graph is on one responder only.
