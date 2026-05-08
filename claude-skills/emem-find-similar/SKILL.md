---
name: emem-find-similar
description: Given a place name or cell64, return the top-K most similar places on Earth by cosine similarity over the 128-D Tessera foundation embedding. Use when the user asks for analogues, look-alikes, or counterparts ("find cities like Bangalore", "where else looks like the Sundarbans", "show me places with a similar urban canopy to Singapore"). Returns cell64s with scores, lat/lng, and cached place labels.
allowed-tools: Bash(curl:*) Bash(jq:*) Read
---

# emem-find-similar

This skill runs a nearest-neighbour search over the Tessera embedding
field on emem.dev. Tessera is a 128-D learned multimodal vector that
fuses Sentinel-2 optical, Sentinel-1 radar, and seasonality into one
position-stable representation per cell per year. Two cells with cosine
similarity >0.85 are usually the same physical archetype.

## When to invoke

The user asks for analogues:

- "Find cities globally that look like Bangalore."
- "What other places have an urban canopy similar to Singapore?"
- "Show me regions with the same forest signature as the Western Ghats."
- "Compare Mumbai and Lagos by their Tessera embedding."

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

### Step 2 — ensure the seed has a Tessera vector attested

`/v1/find_similar` returns `404 cid_not_found` when the seed cell
has no `geotessera` band attested on this responder. Materialise
it first (idempotent if already present):

```sh
curl -sf -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d "{\"cell\":\"$SEED_CELL\",\"bands\":[\"geotessera\"]}" > /dev/null
```

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

`geotessera` defaults to the 2024 vintage. If the user asks "what
looked like X in 2018?", you can change the band:

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

- **Cosine over Tessera reflects physical archetype, not socioeconomic
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
  POST /v1/recall {"cell":..., "bands":["geotessera"]}    → ok
  POST /v1/find_similar {"key":"defi.zb493.xoso.zcb6a","k":8}
    → neighbors:
       defi.zb5cf.nura.zd83c  0.91  New York City, USA
       defi.zb563.noxo.xAvu   0.89  Shanghai, China
       defi.zb541.…           0.87  Lagos, Nigeria
       …

CLAUDE replies: "By cosine over the Tessera 2024 embedding,
Bangalore's nearest neighbours globally are New York (0.91),
Shanghai (0.89), Lagos (0.87), and …. The similarity captures
shared physical archetype — dense urban core with patches of
green and surrounding peri-urban — not social or economic
features. Each result is content-addressed; click 'recall' on
any to fetch the underlying vector and verify it offline."
```
