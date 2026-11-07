---
name: emem-recall-polygon
description: Recalls signed Earth-observation facts at sampled cells inside an area in one call. The area is a named region, a bounding box or a GeoJSON boundary. Use when the user asks about an extent rather than a point. Examples are "average NDVI inside this watershed" and "precipitation across the Western Ghats". Returns facts and an Ed25519 receipt for each cell. Also returns the sampling that the call did, so the agent can state the coverage honestly.
---

# emem-recall-polygon

| | |
|---|---|
| Use it when | The user asks about an area, not one point. |
| You need | A place name, a bounding box with named corners, or a GeoJSON polygon. One or more band names. No key. |
| You get | Facts for each sampled cell, one Ed25519 receipt for each cell, and the sample size and coverage. |
| Calls | `emem_recall_polygon` (MCP, by name through `tools/call`). Or `POST /v1/recall_polygon`. |

> **Network use.** The commands in this skill send JSON to `https://emem.dev` and receive JSON from it. `https://emem.dev` is the service that the MCP server of the plugin connects to. The commands do not execute what they download. This skill ships no scripts. The commands read no environment variable, key or credential. They read only local files that an earlier step saved from `https://emem.dev`.

One call samples cells inside an area and recalls the bands at each
cell. It does the work of `locate`, a cell sample and `recall_many`
together. The rollup (mean, minimum, maximum) is your job.

## When to use this skill

- "What is the average NDVI inside this watershed?"
- "Show me the precipitation across the area bounded by ..."
- The user pastes a bounding box or a GeoJSON polygon.

Use a different path in these cases:

| You have or want | Do this |
|---|---|
| One point | Use [`emem-locate-and-recall`](../emem-locate-and-recall/SKILL.md). |
| The full-resolution pixel grid over an area | Use [`emem-field-tokens`](../emem-field-tokens/SKILL.md). |

## Step 1: give the area

Give the area in one of three fields:

| Field | Form | Notes |
|---|---|---|
| `place` | Free text | The geocoder resolves it. Check `polygon_bbox.source` in the result (step 3). |
| `polygon_bbox` | An object with `min_lat`, `max_lat`, `min_lng`, `max_lng` | The server refuses an array with a 400. Array orders differ between conventions. |
| `polygon_geojson` | A GeoJSON Polygon, MultiPolygon or Feature | `geojson` and `geometry_geojson` are aliases. |

A polygon alone sets the area. Its bounds become the box, and its
boundary masks the sample. When you also send a bbox, the polygon only
masks the sample.

## Step 2: recall the area

Call `emem_recall_polygon` with this body:

```json
{"polygon_bbox": {"min_lat": 12.95, "max_lat": 13.05, "min_lng": 77.55, "max_lng": 77.65},
 "bands": ["copdem30m.elevation_mean"],
 "max_cells": 16, "budget_ms": 20000}
```

`emem_recall_polygon` is not in the short tool list that the MCP server
of the plugin shows. The server runs it by name through `tools/call`.
If your host calls only listed tools, use the REST route.

An MCP result larger than about 24 KB comes back slimmed.
`_emem_truncation` names what the server removed. For the full
response, send the same body to the REST route:

```sh
curl -s -X POST https://emem.dev/v1/recall_polygon -H 'content-type: application/json' \
  -d '{"polygon_bbox":{"min_lat":12.95,"max_lat":13.05,"min_lng":77.55,"max_lng":77.65},
       "bands":["copdem30m.elevation_mean"],"max_cells":16,"budget_ms":20000}' \
  -o poly.json
jq '{cells_sampled, facts_returned, area_km2, converged, coverage_fraction,
     is_exhaustive, pending: (.pending | length), source: .polygon_bbox.source}' poly.json
```

On 6 and 7 November 2026 this call returned 200 with 16 cells sampled,
16 facts, `area_km2` 120.05, `coverage_fraction` 0.0000133,
`converged: true` and no pending entries.

The two limits:

| Field | Default | Range |
|---|---|---|
| `max_cells` | 64 | 1 to 1024. A value out of range gives a 400, not a silent clamp. |
| `budget_ms` | Set by the server | A soft time budget for fetches. See step 4. |

## Step 3: read the coverage

| Field | Meaning |
|---|---|
| `cells_sampled` | The number of cells that the call read. |
| `native_grid_cells_in_polygon` | The number of 10 m cells in the area. |
| `coverage_fraction` | `cells_sampled` divided by `native_grid_cells_in_polygon`. |
| `is_exhaustive` | `true` only when the sample covers every cell in the area. |
| `converged` | `false` when the time budget ran out before all fetches finished. |
| `pending` | One typed entry for each fetch that did not finish. |
| `polygon_bbox.source` | Where the extent came from. See the next table. |

A sample is not the area. Sixteen cells over 120 km² is a sample.
Report it as a sample.

When you send a `place`, read `polygon_bbox.source`:

| `polygon_bbox.source` | Where the extent came from |
|---|---|
| `request_polygon_bbox` | The `polygon_bbox` that you sent. |
| `request_polygon_geojson` | The bounds of the `polygon_geojson` that you sent. |
| `nominatim_boundingbox` | The bounding box that the Nominatim geocoder returned for the place. |
| `wide_bbox_table` | A built-in table of large regions. |
| `overture_division_area` | An Overture administrative boundary. |
| `geocoder` | A geocoder that the server does not name. |
| `centre_cell_bbox` | No extent. The geocoder found only a point, so the call read one cell. |

For `centre_cell_bbox`, tell the user that the answer is for one cell.
Do not present a one-cell answer as a regional answer.

On 7 November 2026, `"place": "Cubbon Park, Bengaluru"` with
`max_cells` 4 returned `nominatim_boundingbox`, `area_km2` 1.64 and 4
facts.

## Step 4: handle a partial answer

When `budget_ms` runs out, the call returns a partial 200 with
`converged: false` and a `pending[]` list. Each entry says why it is
pending and what to do. The server does not stop the pending fetches.
Thus the same request, sent again, can return more facts. The responder
does not sign the entries in `pending`. They do not state that a cell
has no data.

On 28 September 2026, the call in step 2 returned 200 with 0 facts,
`converged: false` and 16 entries in `pending` (`state:
"materializing"`). Nine retries of the same call over 30 minutes also
returned 0 facts. The fetch path of the responder had no free capacity.

Retry a few times with a pause. If `pending` does not get shorter, tell
the user that the upstream fetch has stopped. Do not loop. Never report
an empty answer as "no data here".

A request with many cold cells and several bands can pass the 40 s
transport budget of the responder. It then returns `compute_timeout`.
Use fewer bands or fewer cells. Or retry after the first pass has
filled the cache.

## Step 5: make the rollup

The response gives facts for each cell. Skip absences: a `null` value
is a signed statement that the upstream had no data. Guard the empty
case:

```sh
jq '[.by_cell[].facts[] | select(.band == "copdem30m.elevation_mean" and .value != null) | .value]
    | if length == 0 then {count: 0}
      else {count: length, mean: (add / length), min: min, max: max} end' poly.json
```

On 6 and 7 November 2026 this filter gave 16 values, a mean of 906.7 m, a
minimum of 885.1 m and a maximum of 940.7 m.

Each cell `facts[]` can hold several readings of one band, oldest
first (`fact_order: "tslot_ascending"`). For the current value, take
the last reading of each band. Or take the reading with the `tslot`
that you mean.

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
  "merged_facts": [ ... ], "schema": "emem.recall_polygon.v1"
}
```

## Step 6: cite and verify the cells

The rollup has no receipt of its own. Each cell receipt verifies that
cell only. Say which cells and which statistic you used.

The facts in this response carry no `memory_token` field. Make the
`emem:fact:<cell64>:<fact_cid>` tokens from each cell receipt:

```sh
jq -r '.by_cell | to_entries[] | .key as $c | .value.receipt.fact_cids[]
       | "emem:fact:\($c):\(.)"' poly.json
```

To check one cell receipt on the server, send it as `{"receipt": ...}`:

```sh
jq -c '{receipt: .by_cell | to_entries[0].value.receipt}' poly.json \
  | curl -s -X POST https://emem.dev/v1/verify_receipt -H 'content-type: application/json' -d @- \
  | jq -c '{valid, signature_valid, merkle_proof_valid}'
```

On 6 and 7 November 2026 both commands ran. The check returned `true` for all
three fields. To check offline, use
[`emem-verify-receipt`](../emem-verify-receipt/SKILL.md).

To give many cells as one citation, send their `fact_cid` values to
`POST /v1/memory_bundle {"fact_cids":[...]}`. It returns one
`emem:bundle:` token. This call stores the bundle on the responder, so
this skill did not run it. See
[`emem-multi-agent-handoff`](../emem-multi-agent-handoff/SKILL.md).

Before you publish the rollup, see
[`emem-referential-drift`](../emem-referential-drift/SKILL.md) and
[`emem-verify-before-publish`](../emem-verify-before-publish/SKILL.md).
