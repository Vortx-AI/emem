---
name: emem-locate-and-recall
description: Resolves a place name to an emem cell64 address and recalls signed Earth-observation facts there. Bands include weather, vegetation indices, elevation, soil, land cover, surface water and forest change. Use when the user wants a measurable value at a named place, with an answer that others can verify offline. Examples are "what is the temperature in Bengaluru", "how high is Denali" and "what is the NDVI in the Sundarbans". Returns content-addressed facts, an emem:fact citation token for each fact, and an Ed25519 receipt. Reads need no key.
---

# emem-locate-and-recall

| | |
|---|---|
| Use it when | The user asks for a measurable value at one named place or one coordinate. |
| You need | A place name, or `lat` and `lng`, or a `cell64` you already have. No key. |
| You get | Signed facts, one `emem:fact:` token for each fact, and an Ed25519 receipt. |
| Calls | `emem_locate`, then `emem_recall` (MCP). Or `POST /v1/locate`, then `POST /v1/recall`. |

> **Network use.** The commands in this skill send JSON to `https://emem.dev` and receive JSON from it. Some steps also receive an image or a raster file. `https://emem.dev` is the service that the MCP server of the plugin connects to. The commands do not execute what they download. This skill ships no scripts. The commands read no environment variable, key or credential. They read only local files that an earlier step saved from `https://emem.dev`.

Two calls change a place name into signed facts. Each fact comes back
with its own `emem:fact:` token. Thus the number always travels with
its citation.

## When to use this skill

The user asks about a measurable geospatial value at a named place:

- "What is the current 2 m air temperature in Bengaluru?"
- "Show me the NDVI in the Sundarbans."
- "What is the elevation of Mount Kilimanjaro?"

Use a different path in these cases:

| You have or want | Do this |
|---|---|
| A `cell64` already (a string like `defi.zb493.zezo.zcb35`) | Skip step 1. |
| An area, not a point | Use [`emem-recall-polygon`](../emem-recall-polygon/SKILL.md). |
| A picture of the place | `GET https://emem.dev/v1/cells/<cell64>/scene.png` |

## Step 1: resolve the place

Call `emem_locate` with this body:

```json
{"q": "Bengaluru, India"}
```

Without MCP, send the same body to `POST https://emem.dev/v1/locate`:

```sh
curl -s -X POST https://emem.dev/v1/locate -H 'content-type: application/json' \
  -d '{"q":"Bengaluru, India"}' \
  | jq -c '{cell64, place_label, via, disambiguation_required}'
```

Read `cell64`, `place_label`, `via` and `disambiguation_required` in the
result. On 6 November 2026 this call returned `defi.zb493.zezo.zcb35`,
"Bengaluru (city), Karnataka, India", through the `photon` geocoder.

The tool also accepts `place` as a synonym of `q`. A body with
`{"lat":..,"lng":..}` does not use the geocoder.

Sometimes the name can mean more than one place. Then
`disambiguation_required` is true, or `selected.is_high_confidence` is
false. In that case, read `alternatives` and ask the user which place
they mean. Do not cite rank 0. For example, "Springfield" returns
Springfield, Missouri first, with `disambiguation_required: true` and
five alternatives.

## Step 2: recall the bands

Call `emem_recall` with the `cell64` from step 1:

```json
{"cell": "defi.zb493.zezo.zcb35", "bands": ["weather.temperature_2m", "indices.ndvi"]}
```

Without MCP, send the same body to `POST https://emem.dev/v1/recall`
and save the result as `recall.json`:

```sh
curl -s -X POST https://emem.dev/v1/recall -H 'content-type: application/json' \
  -d '{"cell":"defi.zb493.zezo.zcb35","bands":["weather.temperature_2m","indices.ndvi"]}' \
  -o recall.json
```

Over MCP, save the JSON text of the tool result as `recall.json`. Then
print the latest reading for each band, with the token to cite it:

```sh
jq -c '. as $r | .current_by_band | to_entries[] | .value as $c
       | $r.facts[] | select(.fact_cid == $c)
       | {band, value_verbatim, unit, observed_at, memory_token}' recall.json
```

Read the response by field name, not by position:

| Field | What it holds |
|---|---|
| `facts[]` | Every stored reading for the bands you asked for, in `fact_order` (`tslot_ascending`). Thus `facts[0]` is the oldest reading, not the current one. |
| `current_by_band` | The `fact_cid` of the latest reading for each band. Use it to pick the fact that you quote. |
| `value_verbatim` | The exact decimal string that the responder signed. Quote it. Do not re-type the JSON number. |
| `memory_token` | The citation `emem:fact:<cell64>:<fact_cid>`. Give it to the user or to the next agent. |
| `materialize_notes[]` | One note for each band you asked for: `materialized`, or `skipped` with a `reason_class`. |
| `receipt` | The responder's signature over the `fact_cids` in this response. Verify it with [`emem-verify-receipt`](../emem-verify-receipt/SKILL.md). |

A `timeout` note with `retryable: true` means that the fetch continues
in the background. The same call some seconds later usually answers
from the store.

### Two recorded runs at Bengaluru

| Date | Time | Facts | Latest values from `current_by_band` |
|---|---|---|---|
| 28 September 2026 | 14 s | 11 (9 NDVI, 2 temperature) | `weather.temperature_2m` = `28.0` degC at 2026-09-28T10:00:00Z. The fresh NDVI fetch hit the 14 s materialiser limit. It came back as a `retryable` timeout note, so the NDVI answer was the latest stored reading. |
| 6 November 2026 | 0.6 s | 23 (11 NDVI, 12 temperature) | `weather.temperature_2m` = `19.9` degC at 2026-11-06T23:00:00Z. `indices.ndvi` = `0.8323313293253172` from the scene of 2026-10-29. Both notes said `materialized`. |

The second run returned this temperature token:
`emem:fact:defi.zb493.zezo.zcb35:wmiie6af6m7ovchgs6ke36gak7fcowtz5zuroxwrwynlkbrlitba`.
The number of facts grows each time the responder stores a new reading.

## How to choose bands

The band catalogue changes for each deployment. Ask the responder:

- `GET /v1/bands` lists the cube bands and the scalar keys of each band
  (`.bands[].key`, `.bands[].scalar_keys`).
- `GET /v1/materializers?page_size=100&summary=true` lists the bands
  that the responder can fetch on a miss. Follow
  `pagination.next_url` to get the next page. On 6 November 2026 the
  list on emem.dev had 113 entries on two pages.

These common scalars materialise automatically on emem.dev. All of them
were in the materializer list on 6 November 2026.

| Topic | Bands |
|---|---|
| Weather now | `weather.temperature_2m`, `weather.precipitation_mm`, `weather.relative_humidity_2m`, `weather.wind_speed_10m` |
| Climate history | `era5.t2m`, `era5.precip`, `power.t2m`, `terraclimate.precip_normal_mm` |
| Air | `cams.pm25`, `cams.no2`, `cams.o3`, `cams.aod_550` |
| Vegetation | `indices.ndvi`, `indices.evi`, `indices.ndmi`, `modis.ndvi_mean`, `modis.lai_8day` |
| Terrain | `copdem30m.elevation_mean`, `gmrt.topobathy_mean` (ocean depth) |
| Land cover | `esa_worldcover.lc_2021`, `koppen` |
| Surface water | `surface_water.occurrence`, `surface_water.recurrence`, `surface_water.seasonality`, `surface_water.transition_class` |
| Forest | `forest_change.treecover2000`, `forest_change.lossyear` (Hansen GFC), `jrc_gfc2020.forest_2020`, `jrc_tmf.deforestation_year` |
| Soil | `soilgrids.phh2o_0_30cm`, `soilgrids.soc_0_30cm`, `soilgrids.clay_0_30cm` |

emem.dev no longer computes the foundation-model embedding bands, for
example `clay_v1`. Facts that these bands signed before still read and
verify. A request for one of these bands returns a note with the reason
`band_retired_at_this_responder`. Use the deterministic indices
instead. Anyone can compute them again from the same Sentinel scene.

## A value is a pixel sample, not a cell average

Each recall response has a `spatial_basis` block that says this
clearly. The cell is an address, about 9.5 m across. The value is a
sample of the source pixel that overlaps the cell. One 30 m DEM pixel
or one 463 m MODIS pixel covers many cells. Thus, do not subtract the
values of two adjacent cells and call the result a gradient.

## What to do when a call fails

| Response | Cause | What to do |
|---|---|---|
| `cid_not_found` (404) | No stored fact for that band at that cell, and no materialiser could fetch one. | Read `GET /v1/data_availability` to see which bands the responder can materialise. |
| `invalid_cell` (400) | The `cell` has the shape of a `cell64` but has a symbol that is not in the alphabet. The responder does not geocode it. | Do step 1 again. |
| `band_not_in_registry` | The responder does not know the band name. | Find the correct name in `GET /v1/bands`. |
| `compute_timeout` | The request took longer than the transport limit of the responder, 40 s on emem.dev. | Ask for fewer bands. Or try again after the background fetch is complete. |
| `_emem_truncation` in an MCP result | The result was larger than the MCP wire limit, about 24 KB. The server made it smaller and lists what it removed. | Ask for fewer bands, or send the same body to the REST route. The REST route returns the full response. |

Recall does not accept the MCP `task` parameter. Only `emem_eudr_dds`
and `emem_hunt` run as tasks.

## One-call alternative

`POST /v1/ask {"question":"what is the elevation of Bengaluru?"}` (MCP
`emem_ask`) classifies the question and runs locate and recall for you.
It returns the same signed facts and a receipt. Use the two-step path
when you must control the bands or the place resolution.

## Before you quote the number

Give the `memory_token`, not only the number. The receiver then
resolves the byte-identical signed fact and does not have to trust your
summary. See [`emem-referential-drift`](../emem-referential-drift/SKILL.md).
Before you send the answer, run
[`emem-verify-before-publish`](../emem-verify-before-publish/SKILL.md).
