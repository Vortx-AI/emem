---
name: emem-recall-polygon
description: Recall signed Earth-observation facts at every cell inside a user-supplied polygon. Use when the user asks about an extent rather than a point — "what's the average NDVI inside this watershed", "show me precipitation across the Western Ghats", "what's the elevation profile of this region". Accepts a polygon as [lng, lat] coordinate pairs and returns per-cell facts plus a summary. Each cell carries its own Ed25519 receipt.
allowed-tools: Bash(curl:*) Bash(jq:*) Read
---

# emem-recall-polygon

This skill calls `/v1/recall_polygon` to fetch facts inside an
arbitrary polygon. The responder samples every cell64 whose centre
falls inside the polygon (cap: 4096 cells per request), recalls the
requested band(s) at each, and returns a structured response with a
per-cell receipt for verification.

## When to invoke

The user defines a *region* rather than a *point*:

- "What's the average NDVI inside this watershed: [coords]?"
- "Show me precipitation across the area bounded by …"
- "Recall elevation across this admin boundary."
- The user pastes GeoJSON or a list of `[lng, lat]` pairs.

If the user just wants a single cell, use `emem-locate-and-recall`.
If they want a *similar-cells* list rather than a polygon, use
`emem-find-similar`.

## How to invoke

The polygon is a closed ring: `[[lng0, lat0], [lng1, lat1], …,
[lng0, lat0]]`. Coordinates are WGS-84 decimal degrees, longitude
first (GeoJSON convention).

```sh
curl -sf -X POST https://emem.dev/v1/recall_polygon \
  -H 'content-type: application/json' \
  -d '{
    "polygon": [
      [77.55, 12.95],
      [77.65, 12.95],
      [77.65, 13.05],
      [77.55, 13.05],
      [77.55, 12.95]
    ],
    "bands": ["indices.ndvi", "weather.precipitation_mm"]
  }' | jq '{
    cells_returned: (.by_cell | length),
    facts_total:    (.by_cell | to_entries | map(.value.facts | length) | add),
    area_km2:       .area_km2
  }'
```

### Picking the band

If the user asked about *precipitation*, use
`weather.precipitation_mm` (current 24 h) or `era5.precip` (1940→
present hourly). If they asked about *vegetation*, use
`indices.ndvi` (computed live from S2) or `modis.ndvi_mean` (16-day
MODIS composite, larger pixels but global). If they asked about
*elevation*, use `copdem30m.elevation_mean` (30 m, terrestrial only;
`gmrt.topobathy_mean` for ocean depths). Full band list at
`https://emem.dev/v1/bands`.

### Computing aggregates

The response is per-cell facts; the agent does the rollup:

```sh
curl -sf -X POST https://emem.dev/v1/recall_polygon \
  -H 'content-type: application/json' \
  -d '<polygon_payload>' \
  | jq '
    [.by_cell | to_entries[].value.facts[] | select(.band == "indices.ndvi") | .value]
    | { count: length, mean: (add / length), min: min, max: max }
  '
```

For more sophisticated geometry (point-in-polygon with holes,
intersection with admin boundaries), pull the full response and
process locally.

## Response shape

```jsonc
{
  "by_cell": {
    "defi.zb493.xoso.zcb6a": {
      "facts": [{ "band": "indices.ndvi", "value": 0.42, ... }],
      "receipt": { "request_id": "...", "fact_cids": [...], "signature": [...] },
      "bands_already_attested_at_cell": ["weather.temperature_2m", "indices.ndvi", ...]
    },
    "defi.zb493.xoso.zcb70": { /* ... */ }
  },
  "polygon_vertices": 5,
  "cells_sampled": 12,
  "facts_returned": 24,
  "area_km2": 1.2,
  "schema": "emem.recall_polygon.v1"
}
```

## Pitfalls

- **Polygon must be closed** — the first and last `[lng, lat]` pair
  must be identical. Open polygons return `400 invalid_polygon`.
- **Cap at 4096 cells.** Very large regions return `413 polygon_too_large`
  with a hint about subdividing. The `/humans` page's lasso uses
  this cap as the geometric guide.
- **Counter-clockwise orientation** is conventional for outer rings;
  most responders accept either, but the area calculation flips
  sign on clockwise input.
- **Per-cell receipts** — each cell's receipt verifies independently.
  There is no aggregate receipt; verifying any one cell verifies
  that cell only. To audit the bulk call, iterate `by_cell` and
  pass each `.receipt` to the `emem-verify-receipt` skill.

## Worked example

```
USER: What's the average NDVI inside this watershed:
      [[77.5,12.9],[77.7,12.9],[77.7,13.1],[77.5,13.1],[77.5,12.9]]?

CLAUDE invokes this skill:
  POST /v1/recall_polygon
    body: { polygon: [[77.5,12.9],...], bands: ["indices.ndvi"] }
    → cells_sampled: 12
    → facts_returned: 12
    → area_km2: ~5

  jq aggregation:
    → mean NDVI: 0.41

CLAUDE replies: "Across the 12 cells inside that polygon (~5 km²
in central Bengaluru), the average NDVI is 0.41 — typical for a
mixed urban core with pockets of vegetation. The lowest cell
returned 0.18 (heavily built up), the highest 0.62 (the
Cubbon Park area). All twelve facts are signed individually;
re-fetching any fact_cid returns the same bytes."
```
