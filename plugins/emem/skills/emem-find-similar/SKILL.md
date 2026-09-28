---
name: emem-find-similar
description: Returns the top-K places most similar to a seed place by cosine over a stored 128-D surface-texture embedding (Sentinel-1 plus Sentinel-2, annual). Use when the user asks for analogues or look-alikes ("find places like Bangalore", "where else looks like the Sundarbans"). The embedding band is retired on emem.dev, so the index is frozen at what was already stored; the skill says how to tell "no analogue" from "this seed has no vector and cannot get one".
---

# emem-find-similar

A nearest-neighbour search over a stored 128-D embedding of surface
texture (Sentinel-1 SAR plus Sentinel-2 optical, aggregated per year).
Two cells above about 0.85 cosine are usually the same physical
archetype.

## Read this before you promise an answer

The embedding band (`geotessera`) is **retired on emem.dev**. The
responder serves vectors it already stored and computes no new ones:

- A seed that has a stored vector works as before.
- A seed without one **cannot be given one**. `/v1/find_similar` answers
  `cid_not_found`, and `/v1/recall` on the band answers a note with
  reason `band_retired_at_this_responder`. That is a deployment decision,
  and retrying never changes it.
- Coverage is whatever was stored before the retirement: dense around
  places people asked about, empty elsewhere.

"No analogue found" and "this place has no vector" are different
answers. Only report the one that happened.

## When to invoke

- "Find cities globally that look like Bangalore."
- "Where else has the same forest signature as the Western Ghats?"

For predicate matching ("all places with NDVI above 0.7") this is the
wrong tool: use `POST /v1/query_region` or `POST /v1/compare_bands`.
For climate similarity, compare `koppen` classes from recall instead;
this embedding aliases very different climates that share a texture.

## Step 1: resolve the seed

```sh
SEED=$(curl -sf -X POST https://emem.dev/v1/locate \
  -H 'content-type: application/json' \
  -d '{"q":"Bengaluru, India"}' | jq -r '.cell64')
```

## Step 2: query, and branch on the answer

```sh
curl -s -X POST https://emem.dev/v1/find_similar \
  -H 'content-type: application/json' \
  -d "{\"key\":\"$SEED\",\"k\":12}" > sim.json
jq 'if .code then {code, message}
    else [.neighbors[] | {cell, score, place: .place_label_cached, lat, lng}] end' sim.json
```

Call it directly rather than probing with recall first. On 2026-09-28 a
Timbuktu seed returned no `geotessera` fact from `/v1/recall` (a
retired-band note) yet `/v1/find_similar` answered from its stored
vector, so recall is not a reliable coverage test. `cid_not_found` from
find_similar is.

Each neighbour carries `cell`, `score` (cosine), `lat`, `lng`,
`place_label_cached`, `band_used`, `similarity_method`, `scene_png_url`
and `deep_recall_url`. The response also carries a signed `receipt`
over the neighbours' fact cids and an `interpretation` block that says
what the similarity does and does not mean. Pass that on.

Recorded on 2026-09-28: seed `defi.zb493.zezo.zcb35` (Bengaluru) with
`k: 5` returned four neighbours, all in Bengaluru, top score 0.926. A
seed at 44.123 S, 120.456 W (open Pacific) returned `cid_not_found`.

## Options

- `band: "geotessera.2018"` (any year 2017 to 2024) searches that
  vintage; `geotessera.multi_year` searches the stacked 1024-D vector,
  which picks up places that changed in similar ways. Each vintage
  answers only where it was stored.
- `mode: "hamming"` runs the binary sign-bit index instead of cosine
  (`similarity_method: "hamming"` on each neighbour). Do not pass
  `band: "geotessera.bin128"`: on 2026-09-28 that returned an `internal`
  error.

## Pitfalls

- **Nearby is not similar.** The top neighbours of a city cell are often
  other cells of the same city. Ask for a larger `k` and filter by
  distance if the user wants places elsewhere.
- **Archetype, not society.** A "similar" city looks the same from
  space (density, vegetation, water) and may be nothing alike otherwise.
- **Thresholds are heuristic.** Above 0.85 same archetype, 0.7 to 0.85
  related, below 0.7 weak; the cutoff depends on the seed.

## Before you report the neighbours

Similarity is a ranking over a frozen index, not a measurement. Name
the neighbours with their cell64 and score, say the index is frozen,
and cite band facts (recall at each neighbour) for anything you claim
about them. See [`emem-referential-drift`](../emem-referential-drift/SKILL.md)
and [`emem-verify-before-publish`](../emem-verify-before-publish/SKILL.md).
