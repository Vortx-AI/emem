---
name: emem-find-similar
description: Returns the top-K places most similar to a seed place by cosine over a stored 128-D surface-texture embedding (Sentinel-1 plus Sentinel-2, annual). Use when the user asks for analogues or look-alikes ("find places like Bangalore", "where else looks like the Sundarbans"). The embedding band is retired on emem.dev, so the index is frozen at what was already stored; the skill says how to tell "no analogue" from "this seed has no vector and cannot get one".
---

# emem-find-similar

| | |
|---|---|
| Use it when | The user asks for places that look like a seed place, for example "find cities like Bangalore". |
| You need | A place name or a coordinate for the seed. No key. |
| You get | Up to `k` neighbour cells, each with a cosine `score`, a place label and a coordinate. You also get a signed `receipt` and an `interpretation` block. If the seed has no stored vector, you get `cid_not_found`. |
| Calls | `emem_locate`, then `emem_find_similar`. Over REST: `POST /v1/locate`, then `POST /v1/find_similar`. |

> **Network use.** The commands in this skill send JSON to `https://emem.dev` and receive JSON from it. Where a step says so, they also receive an image or a raster file. `https://emem.dev` is the service that the MCP server of the plugin connects to. The commands do not execute what they download. This skill ships no scripts. Its commands read no environment variable, key or credential. The only local files that they read are files that an earlier step saved from `https://emem.dev`.

The skill runs a nearest-neighbour search over a stored 128-D
embedding of surface texture. The embedding comes from Sentinel-1 SAR
and Sentinel-2 optical data, one vector for each year. Two cells with
a cosine above about 0.85 usually show the same type of surface.

## Read this before you promise an answer

emem.dev retired the embedding band (`geotessera`). The
responder serves the vectors that it stored before. It computes no new
vectors.

| Seed | What happens |
|---|---|
| The seed cell has a stored vector | The search works. |
| The seed cell has no stored vector | `/v1/find_similar` returns `cid_not_found` (HTTP 404). `/v1/recall` on the band returns a note with the reason `band_retired_at_this_responder`. A retry does not change this. It is a deployment decision. |

Coverage is what the responder stored before the retirement. It is
dense near places that people asked about, and empty in other areas.

"No analogue found" and "this place has no vector" are different
answers. Report only the answer that you got.

## When to use a different tool

| Question | Use |
|---|---|
| "Find cities that look like Bangalore." | This skill. |
| "Where else has the forest signature of the Western Ghats?" | This skill. |
| "All places with NDVI above 0.7." | `POST /v1/query_region` or `POST /v1/compare_bands`. |
| "Where else has this climate?" | Compare `koppen` classes from recall. This embedding can match very different climates that have the same surface texture. |

## Step 1: get the seed cell

Call `emem_locate` with:

```json
{"q": "Bengaluru, India"}
```

Without MCP, send the same body to `POST https://emem.dev/v1/locate`:

```sh
curl -s -X POST https://emem.dev/v1/locate -H 'content-type: application/json' \
  -d '{"q":"Bengaluru, India"}' | jq -c '{cell64, via}'
```

Keep the `cell64`. On 7 November 2026 this call returned
`defi.zb493.zezo.zcb35`. For a coordinate, send
`{"lat": -44.123, "lng": -120.456}` in place of `q`.

## Step 2: run the search

Call `emem_find_similar`. Put the seed cell in `key`. The tool also
accepts `cell` as another name for `key`.

```json
{"key": "defi.zb493.zezo.zcb35", "k": 5}
```

Without MCP, send the same body to `POST https://emem.dev/v1/find_similar`.
Save the answer as `sim.json`, also when it is an error. Here a
refusal is an answer.

```sh
curl -s -X POST https://emem.dev/v1/find_similar -H 'content-type: application/json' \
  -d '{"key":"defi.zb493.zezo.zcb35","k":5}' -o sim.json
```

Over MCP, save the JSON text of the tool result as `sim.json`. A
refusal comes back as a tool error, and its message names the code,
for example `cid_not_found`.

Do not use recall to test for a vector first. On 28 September 2026, a
Timbuktu seed got no `geotessera` fact from `/v1/recall`, only a
retired-band note. But `/v1/find_similar` answered from its stored
vector. Thus recall is not a reliable test. `cid_not_found` from
find_similar is.

## Step 3: read the answer

Print the error, or the neighbours:

```sh
jq 'if .code then {code, message}
    else [.neighbors[] | {cell, score, place: .place_label_cached, lat, lng}] end' sim.json
```

| Field | What it holds |
|---|---|
| `neighbors[]` | The neighbour cells, best first. Each has `cell`, `score`, `lat`, `lng`, `place_label_cached`, `band_used`, `similarity_method`, `scene_png_url` and `deep_recall_url`. |
| `requested_k`, `returned_k` | The `k` you asked for, and the number of neighbours you got. `returned_k` can be smaller. |
| `receipt` | The responder's signature over the `fact_cids` of the neighbours. Verify it with [`emem-verify-receipt`](../emem-verify-receipt/SKILL.md). |
| `interpretation` | What the score measures (`embedding_axis`), what it does not mean (`does_not_mean`), and what to use for climate (`for_climate_proximity_use`). Give this block to the user with the neighbours. |
| `code`, `message` | Present only on a refusal. |

### Recorded runs

| Date | Seed | Result |
|---|---|---|
| 7 November 2026 | `defi.zb493.zezo.zcb35` (Bengaluru), `k: 5` | `returned_k` 4. All four neighbours are in Bengaluru. Top score 0.926 (`defi.zb493.wIxI.zcb34`). |
| 7 November 2026 | 44.123 S, 120.456 W (open Pacific), cell `defi.zb209.zfa8a.xitO` | HTTP 404, `cid_not_found`, "no vector found for key". |

## Options

| Option | What it does |
|---|---|
| `band: "geotessera.2018"` | Searches the vector of that year. The registry lists the years 2017 to 2024. A year answers only at cells where the responder stored a vector for that year. |
| `band: "geotessera.multi_year"` | Searches one stacked vector of 128 dimensions for each year, 1152-D for 2017 to 2025 (`lib.rs`, `materialize_geotessera_multi_year`). The responder masks years without data. It finds places that changed in similar ways. |
| `mode: "hamming"` | Uses the binary sign-bit index in place of cosine. Each neighbour then has `similarity_method: "hamming"`. |

A `hamming` score is `1 - 2 * distance / 128`, with the distance in
bits (`crates/emem-primitives/src/find_similar.rs`). It is not on the
cosine scale. Do not apply the 0.85 rule to it. On 7 November 2026 the
Bengaluru seed got a top hamming score of 0.625.

Do not pass `band: "geotessera.bin128"`. On 28 September 2026 it
returned an `internal` error.

The year and `hamming` options can be slow. On 7 November 2026, the
first call with `geotessera.2018` and the first call with `hamming`
both returned HTTP 504 `compute_timeout` after 40 s. The same calls
then answered in 5 s and 9 s. The `multi_year` call took 38 s. After
a `compute_timeout`, send the same call one more time. If it fails
again, run it as a task, as the error message tells you.

## Pitfalls

- Near is not similar. The top neighbours of a city cell are often
  other cells of the same city. Ask for a larger `k`. Then remove
  neighbours that are near the seed if the user wants other places.
- The match is about the surface. A "similar" city has the same
  density, vegetation and water as seen from space. It can be very
  different in other ways.
- The cosine thresholds are approximate. Above 0.85 means the same
  type of surface, 0.7 to 0.85 means related, and below 0.7 means weak.
  The correct cutoff changes with the seed.

## Before you report the neighbours

A similarity is a rank over a frozen index. It is not a measurement.
Give each neighbour with its cell64 and score. Tell the user that the
responder adds no new vectors. For each claim about a neighbour, recall that cell and cite
the band facts. See [`emem-referential-drift`](../emem-referential-drift/SKILL.md)
and [`emem-verify-before-publish`](../emem-verify-before-publish/SKILL.md).
