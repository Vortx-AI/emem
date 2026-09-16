---
name: emem-find-similar
description: Return the top-K most similar places on Earth by cosine over a stored 128-D surface-texture embedding. Use when the user asks for analogues, look-alikes or counterparts ("find cities like Bangalore", "where else looks like the Sundarbans"). IMPORTANT, changed 2026-09-15: the embedding band is RETIRED on emem.dev — the index is frozen, no new vectors are computed, and a cell without one cannot get one. The skill tells you how to check coverage before promising an answer.
allowed-tools: Bash(curl:*) Bash(jq:*) Read
---

# emem-find-similar

This skill runs a nearest-neighbour search over a stored 128-D
embedding of surface texture (Sentinel-1 SAR plus Sentinel-2 optical,
aggregated annually per cell). Two cells above about 0.85 cosine are
usually the same physical archetype.

## Read this before you promise an answer

**The embedding band is retired on emem.dev as of 2026-09-15.** The
responder no longer computes new vectors: it serves the ones already
stored and refuses to make more. The consequences are concrete:

- A cell that already carries a vector works exactly as before.
- A cell that does not **cannot be given one**. `/v1/find_similar`
  returns `cid_not_found`, and calling `/v1/recall` to materialise the
  band returns a note with reason `band_retired_at_this_responder`.
  That is a deployment decision, not an outage, and retrying never
  succeeds.
- So coverage is whatever was materialised before the retirement. It is
  dense around places that have been asked about and empty elsewhere.

Say that to the user when it happens. "No analogue found" and "this
place has no vector and cannot get one" are different answers, and only
the second is true here.

The direction of travel is deterministic indices computed from Sentinel-1
and Sentinel-2 directly rather than a learned encoder, because a
reproducible index can be recomputed by anyone and a frozen model's
weights cannot.

## When to invoke

The user asks for analogues:

- "Find cities globally that look like Bangalore."
- "What other places have an urban canopy similar to Singapore?"
- "Show me regions with the same forest signature as the Western Ghats."
- "Compare Mumbai and Lagos by their surface-texture embedding."

If the user wants exact-band matching (e.g., "all places with NDVI >
0.7"), this is the wrong skill — use `query_region` or
`compare_bands` instead. This skill is *vector cosine*, not
predicate filtering.

## How to invoke

### Step 1 — resolve the seed place to cell64

```sh
SEED_CELL=$(curl -sf -X POST https://emem.dev/v1/locate \
  -H 'content-type: application/json' \
  -d '{"q":"Bangalore, India"}' | jq -r '.cell64')
echo "seed cell: $SEED_CELL"
```

### Step 2 — check the seed HAS a vector (you can no longer create one)

`/v1/find_similar` returns `cid_not_found` when the seed cell carries
no embedding. The old version of this skill told you to materialise it
with `/v1/recall`; that no longer works and the attempt returns
`band_retired_at_this_responder`.

Check instead, and branch honestly:

```sh
curl -sf -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d "{\"cell\":\"$SEED_CELL\",\"bands\":[\"geotessera\"]}" \
  | jq '{has_vector: ((.facts // []) | length > 0),
         retired: ((.materialize_notes // [])
                   | map(select(.reason == "band_retired_at_this_responder"))
                   | length > 0)}'
```

`has_vector: true` — proceed. `retired: true` with `has_vector: false` —
stop and tell the user this place has no stored vector and the responder
cannot compute one, rather than reporting an empty result as if the
search had run.

### Step 3 — query top-K neighbours

```sh
curl -sf -X POST https://emem.dev/v1/find_similar \
  -H 'content-type: application/json' \
  -d "{\"key\":\"$SEED_CELL\",\"k\":12}" \
  | jq '.neighbors[] | {cell, score, place: .place_label_cached, lat, lng}'
```

The response includes:

- `neighbors[].cell` — cell64 of the neighbour
- `neighbors[].score` — cosine similarity in [0, 1]
- `neighbors[].lat`, `.lng` — centre coords
- `neighbors[].place_label_cached` — cached human label if known
- `neighbors[].band_used` — almost always `geotessera`
- `neighbors[].similarity_method` — `cosine` (default) or `hamming`
  (if you set `band: "geotessera.bin128"`)
- `neighbors[].deep_recall_url` — the `/v1/recall` payload that
  fetches the neighbour's full embedding for further drill-down

## Picking the right vintage

The same retirement applies per vintage: a vintage answers only where it
was already materialised, and a miss cannot be filled. Check before you
offer a year, do not promise one.

The default is the 2024 vintage. If the user asks "what looked like X in
2018?", you can change the band:

```sh
curl -sf -X POST https://emem.dev/v1/find_similar \
  -H 'content-type: application/json' \
  -d '{"key":"defi.zb493.xoso.zcb6a","k":12,"band":"geotessera.2018"}'
```

Available vintages: `geotessera.{2017..2024}` plus
`geotessera.multi_year` (1024-D = 8×128 stacked, fuses all years).
The multi-year vector picks up *trajectory* similarity — places that
changed in similar ways.

## Pitfalls

- **Coverage is frozen.** The index holds what was materialised before
  the band was retired. An empty result may mean "no analogue" or "this
  cell was never embedded"; `cid_not_found` means the second. Never
  report the second as the first.
- **Cosine reflects physical archetype, not socioeconomic
  features.** A "similar" city may look the same from space (urban
  density, vegetation, water proximity) but be socially very
  different. Be honest with the user about what the similarity
  measures.
- **Score thresholds are heuristic.** >0.85 is "same archetype",
  0.7–0.85 is "related", <0.7 is "weak". The actual cutoff depends
  on the seed cell's environment.
- **First call may be slow** if the seed needs materialisation; the
  second is cache-warm.

## Worked example

```
USER: Find cities globally that look most like Bangalore.

CLAUDE invokes this skill:
  POST /v1/locate {"q":"Bangalore"}    → defi.zb493.xoso.zcb6a
  POST /v1/recall {"cell":..., "bands":["geotessera"]}    → has_vector: true
  POST /v1/find_similar {"key":"defi.zb493.xoso.zcb6a","k":8}
    → neighbors:
       defi.zb5cf.nura.zd83c  0.91  New York City, USA
       defi.zb563.noxo.xAvu   0.89  Shanghai, China
       defi.zb541.…           0.87  Lagos, Nigeria
       …

CLAUDE replies: "By cosine over the stored 2024 embedding,
Bangalore's nearest neighbours globally are New York (0.91),
Shanghai (0.89), Lagos (0.87), and …. The similarity captures
shared physical archetype — dense urban core with patches of
green and surrounding peri-urban — not social or economic
features. Each result is content-addressed; click 'recall' on
any to fetch the underlying vector and verify it offline."
```

## Before you report the neighbours

Similarity is a ranking, not a measurement, and it carries no receipt of
its own. When you name a neighbour, cite the band facts that make the
claim checkable, and say the embedding is a frozen index rather than a
live computation. [`emem-referential-drift`](../emem-referential-drift/SKILL.md)
covers carrying values as tokens;
[`emem-verify-before-publish`](../emem-verify-before-publish/SKILL.md)
covers the draft.
