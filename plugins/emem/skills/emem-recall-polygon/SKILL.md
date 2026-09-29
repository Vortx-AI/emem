---
name: emem-recall-polygon
description: Recalls signed Earth-observation facts at sampled cells inside an area (a named region, a bounding box, or a GeoJSON boundary) in one call. Use when the user asks about an extent rather than a point, such as "average NDVI inside this watershed", "precipitation across the Western Ghats", or "elevation across this admin boundary". Returns per-cell facts, each cell with its own Ed25519 receipt, plus the sampling it actually did so the agent can state coverage honestly.
---

# emem-recall-polygon

> **Network use.** The commands in this skill send and receive JSON (and, where a step says so, an image or a raster file) to `https://emem.dev` only, the service the plugin's MCP server connects to. Nothing they download is executed. The files under `scripts/` read local files and make no network calls.

`POST /v1/recall_polygon` samples cells inside an area, recalls the
requested bands at each, and returns them per cell with per-cell
receipts. It is `locate`, then a sample of the area's cells, then
`recall_many`, collapsed into one call.

## When to invoke

- "What's the average NDVI inside this watershed?"
- "Show me precipitation across the area bounded by ..."
- The user pastes a bbox or a GeoJSON polygon.

For one point use [`emem-locate-and-recall`](../emem-locate-and-recall/SKILL.md).
For the full-resolution pixel grid over an area use
[`emem-field-tokens`](../emem-field-tokens/SKILL.md).

## How to invoke

Pass exactly one of `place` (free text, geocoded) or `polygon_bbox` (an
object with named corners; an array is refused, because bbox array
orders disagree between conventions). Add `polygon_geojson` beside a
bbox to mask the sample to a real boundary; on its own it is not enough.

```sh
curl -sf -X POST https://emem.dev/v1/recall_polygon \
  -H 'content-type: application/json' \
  -d '{"polygon_bbox":{"min_lat":12.95,"max_lat":13.05,"min_lng":77.55,"max_lng":77.65},
       "bands":["copdem30m.elevation_mean"],
       "max_cells":16, "budget_ms":20000}' \
  > poly.json
jq '{cells_sampled, facts_returned, area_km2, converged, coverage_fraction,
     pending: (.pending | length)}' poly.json
```

- `max_cells` defaults to 64 and caps at 1024; out of range is a 400,
  not a silent clamp.
- `budget_ms` is a soft materialisation budget. When it expires the
  answer is a partial 200 with `converged: false` and a typed
  `pending[]` (each entry says why and what to do). Pending fetches are
  detached, not aborted, so the identical request retried returns
  strictly more. Pending entries are unsigned; they are not signed
  absences.
- Recorded on 2026-09-28: this exact call returned 200 with
  `cells_sampled: 16`, `facts_returned: 0`, `converged: false` and 16
  `pending` entries (`state: "materializing"`), and nine identical
  retries over 30 minutes stayed at zero while the responder's fetch
  path was saturated. Retry a few times with a pause; if `pending` does
  not shrink, tell the user the upstream fetch is stalled rather than
  looping, and never report the empty answer as "no data here".
- A request with many cold cells and several bands can still pass the
  responder's 40 s transport budget and return `compute_timeout`.
  Narrow it: fewer bands, fewer cells, or retry once the first pass has
  warmed the cache.

## Aggregating

The response is per cell; the rollup is yours. Skip absences (a
`null` value is a signed statement that the upstream had no data) and
guard the empty case:

```sh
jq '[.by_cell[].facts[] | select(.band == "copdem30m.elevation_mean" and .value != null) | .value]
    | if length == 0 then {count: 0}
      else {count: length, mean: (add / length), min: min, max: max} end' poly.json
```

## Response shape (abridged)

```jsonc
{
  "by_cell": {
    "<cell64>": { "facts": [ ... ], "receipt": { ... }, "fact_order": "tslot_ascending",
                  "bands_already_attested_at_cell": [ ... ] }
  },
  "cells": [ ... ], "cells_sampled": 16, "facts_returned": 16,
  "area_km2": 120.05, "coverage_fraction": 0.0000133, "is_exhaustive": false,
  "converged": true, "pending": [], "polygon_bbox": { ..., "source": "..." },
  "schema": "emem.recall_polygon.v1"
}
```

`polygon_bbox.source` says where the extent came from when you passed a
`place`: `centre_cell_bbox` means the geocoder found no polygon and fell
back to one cell, so say that rather than presenting a one-cell answer
as a regional one.

## Pitfalls

- **A sample is not the area.** `coverage_fraction` and `is_exhaustive`
  state how much of the area the sample covers. Sixteen cells over
  120 km² is a sample; report it as one.
- **No aggregate receipt.** Each cell's receipt verifies that cell only.
  To audit the call, pass each `by_cell[].receipt` to
  [`emem-verify-receipt`](../emem-verify-receipt/SKILL.md).
- **History, not just the latest.** Each cell's `facts[]` can carry
  several readings of one band, oldest first. For "current" take the
  last per band, or the one whose `tslot` you mean.

## Before you quote the summary

An aggregate has no receipt of its own. Say which cells and which
statistic, and carry the cells' `memory_token`s (or bundle them with
`POST /v1/memory_bundle {"fact_cids":[...]}`) rather than the mean alone.
See [`emem-referential-drift`](../emem-referential-drift/SKILL.md) and
[`emem-verify-before-publish`](../emem-verify-before-publish/SKILL.md).
