---
name: emem-locate-and-recall
description: Resolves a place name to an emem cell64 address and recalls signed Earth-observation facts there (weather, vegetation indices, elevation, soil, land cover, surface water, forest change). Use when the user asks for a measurable value at a named place ("what is the temperature in Bengaluru", "how high is Denali", "what is the NDVI in the Sundarbans") and wants an answer another agent can verify offline. Returns content-addressed facts, a ready-made emem:fact citation token for each, and an Ed25519 receipt. Reads need no key.
---

# emem-locate-and-recall

> **Network use.** The commands in this skill send and receive JSON (and, where a step says so, an image or a raster file) to `https://emem.dev` only, the service the plugin's MCP server connects to. Nothing they download is executed. The files under `scripts/` read local files and make no network calls.

Two REST calls turn a place name into signed facts. Each fact comes back
with its own `emem:fact:` token, so the number never has to travel
without its citation.

## When to invoke

The user asks about a measurable geospatial value at a named place:

- "What's the current 2 m air temperature in Bengaluru?"
- "Show me the NDVI in the Sundarbans."
- "What's the elevation of Mount Kilimanjaro?"

If you already hold a `cell64` (a string like `defi.zb493.zezo.zcb35`),
skip step 1. For an extent rather than a point, use
[`emem-recall-polygon`](../emem-recall-polygon/SKILL.md). For a picture,
`GET https://emem.dev/v1/cells/<cell64>/scene.png`.

## Step 1: resolve the place

```sh
curl -sf -X POST https://emem.dev/v1/locate \
  -H 'content-type: application/json' \
  -d '{"q":"Bengaluru, India"}' \
  | jq '{cell64, place_label, via, disambiguation_required}'
```

`place` is accepted as a synonym of `q`, and `{"lat":..,"lng":..}` skips
the geocoder. When `disambiguation_required` is true, or
`selected.is_high_confidence` is false, read `alternatives` and ask the
user which one they meant rather than citing rank 0. "Springfield"
returns Missouri first and flags it; that is the case this exists for.

## Step 2: recall the bands

```sh
CELL=defi.zb493.zezo.zcb35
curl -sf -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d "{\"cell\":\"$CELL\",\"bands\":[\"weather.temperature_2m\",\"indices.ndvi\"]}" \
  > recall.json

# The latest reading per band, with the token to cite it by.
jq -r '.current_by_band | to_entries[] | .value' recall.json | while read cid; do
  jq -c --arg c "$cid" '.facts[] | select(.fact_cid == $c)
        | {band, value_verbatim, unit, observed_at, memory_token}' recall.json
done
```

Read the response this way, not by position:

- `facts[]` holds **every stored reading** for the requested bands, in
  `fact_order` (`tslot_ascending`), so `facts[0]` is the oldest, not the
  current one.
- `current_by_band` maps each band to the `fact_cid` of its latest
  reading. Use it to pick the fact you quote.
- `value_verbatim` is the exact decimal string that was signed. Quote it
  rather than re-typing the JSON number.
- `memory_token` on each fact is the `emem:fact:<cell64>:<fact_cid>`
  citation. Hand that to the user or the next agent.
- `materialize_notes[]` says what the responder did for each band you
  asked for: `materialized`, or `skipped` with a `reason_class`. A
  `timeout` note with `retryable: true` means the fetch continues in the
  background; the same call a few seconds later usually answers warm.
- `receipt` signs the `fact_cids` this response served. Verify it with
  [`emem-verify-receipt`](../emem-verify-receipt/SKILL.md).

Recorded on 2026-09-28 at `defi.zb493.zezo.zcb35` (Bengaluru), in 14 s:
eleven facts came back, nine of them stored `indices.ndvi` readings
from earlier dates and two temperature readings. `current_by_band`
picked `weather.temperature_2m` = `28.0` degC observed
`2026-09-28T10:00:00Z`, token
`emem:fact:defi.zb493.zezo.zcb35:nflpddk7zsncywguwjzk5koksseqfyx4jnngkuryrnd4aykqlpfq`.
The fresh NDVI fetch hit the 14 s materialiser cap and came back as a
`retryable` timeout note, so the NDVI answer was the latest stored one.
The receipt verified offline with `verify.py`.

## Picking bands

Ask the responder, not this page: the catalogue changes per deployment.

- `GET /v1/bands` lists the cube bands and the scalar keys under each
  (`.bands[].key`, `.bands[].scalar_keys`).
- `GET /v1/materializers?page_size=100&summary=true` lists the bands this
  responder can fetch on a miss (follow `pagination.next_url`).

Common scalars that auto-materialise on emem.dev today:

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

The foundation-model embedding bands are retired on emem.dev: facts they
signed before still read and verify, nothing new is computed, and asking
for one returns a note with reason `band_retired_at_this_responder`.
Prefer the deterministic indices, which anyone can recompute from the
same Sentinel scene.

## A value is a pixel sample, not a cell average

`spatial_basis` on every recall says it plainly: the cell (about 9.5 m)
is an address, and the value is the source pixel overlapping it. A
30 m DEM or a 463 m MODIS pixel covers many cells, so never difference
two adjacent cells and call it a gradient.

## Recovering from errors

- `cid_not_found` (404): nothing is stored for that band at that cell and
  no materialiser could fetch it. `GET /v1/data_availability` says which
  bands can be materialised.
- A malformed `cell64` is a 400 naming the bad symbol. Re-run step 1.
- `compute_timeout`: the request passed the responder's 40 s transport
  budget. Ask for fewer bands, retry once the background fetch lands, or
  send the MCP tool call with a `task` param and poll `tasks/get`.

## One-call alternative

`POST /v1/ask {"question":"what is the elevation of Bengaluru?"}` (MCP
`emem_ask`) classifies the question, runs locate and recall for you, and
returns the same signed facts. Use the two-step path above when you need
to control the bands or the place resolution.

## Before you quote the number

Carry the `memory_token`, not the bare number: whoever receives it
resolves the byte-identical signed fact rather than trusting your
summary. See [`emem-referential-drift`](../emem-referential-drift/SKILL.md).
Before the sentence goes out, run
[`emem-verify-before-publish`](../emem-verify-before-publish/SKILL.md).
