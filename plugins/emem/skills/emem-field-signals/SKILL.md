---
name: emem-field-signals
description: Answers agronomy questions about one farm field with signed evidence from emem. Gets the field's boundary polygons, checks for crop-residue burning over a date window (Sentinel-2 NBR and NDTI plus MODIS burned area), sums actual evapotranspiration over a season (MODIS MOD16), and fetches a true-colour Sentinel-2 picture of the plot. Use when the user asks whether a field was burned, how much water a crop used, what a plot looks like, or where the field edges are, and wants per-cell fact ids behind the answer.
---

# emem-field-signals

> **Network use.** The commands in this skill send and receive JSON (and, where a step says so, an image or a raster file) to `https://emem.dev` only, the service the plugin's MCP server connects to. Nothing they download is executed. This skill ships no scripts. Its commands read no environment variable, key or credential, and the only local files they read are ones an earlier step saved from `https://emem.dev`. Every request body is written in the command itself.

Field-scale questions, each one call, each answer carrying the fact cids
it was computed from and a signed receipt.

| Call | Answers |
|---|---|
| `POST /v1/field_boundaries` (MCP `emem_field_boundaries`) | the field polygons inside a bbox or place, from Fields of The World (CC-BY-4.0) |
| `POST /v1/field_burn_scar` | was residue burned between two dates: `burn`, `signal_only`, `no_burn` or `inconclusive` |
| `POST /v1/field_actual_et` | actual evapotranspiration over a window, in mm and m³/ha |
| `GET /v1/scene.png?bbox=w,s,e,n` | a true-colour Sentinel-2 crop of the plot at native 10 m, up to 1024 px a side |
| `POST /v1/field_series` (MCP `emem_field_series`) | an index over the field per scene: `ndvi`, `ndwi`, `ndmi` (alias `lswi`), `evi`, or Sentinel-1 `vv`, `vh`, `vh_vv` |

`field_series` takes `geometry` (or `bbox`), `index`, `start_date` and
`end_date`. Radar reads one relative orbit; pass `orbit` to choose it.
Radar rows have no `clear_fraction`: `mean` is the dB of the mean
linear power, with `se_db`, `pixel_enl`, `below_nesz_share` and
`bright_rejected`. Pass `fields: [{id, geometry}]` for up to 500 fields
in one window of at most 512 px a side. `inner_buffer_m` drops pixels
near the edge, and `min_support_px` (4 to 30) sets the fewest pixels a
scene needs. `polygon_cid` identifies each outline. On optical indices
a scene that both time neighbours refute gets `quality: "suspect"` and
a `scene_check`, and the trend leaves it out. A `fields` answer adds a
`suspect` column and `suspect_dates`.

`field_burn_scar` and `field_actual_et` take the same body: `start` and
`end` (YYYY-MM-DD, inclusive, at most 400 days), and the field as
`geometry_geojson` (a GeoJSON Polygon or MultiPolygon, or
`{"bbox":[w,s,e,n]}`) or as `cells` (cell64s). `budget_ms` (default
25000, max 300000) is a warm-up budget: after it the call answers from
what is stored, and the warm-up continues, so a retry reads more.

## Step 1: the field's edges

```sh
curl -sf -X POST https://emem.dev/v1/field_boundaries \
  -H 'content-type: application/json' \
  -d '{"polygon_bbox":{"min_lat":30.90,"max_lat":30.903,"min_lng":75.80,"max_lng":75.804},"clean":true}' \
  > fields.json
jq '{count, returned, license, attribution,
     fields: [.geojson.features[] | {area_m2: .properties.area_m2, centroid: .properties.cell64_centroid}]}' fields.json
```

`clean: true` resolves overlapping and duplicate polygons
(`synthesis.overlap_after_m2` should be 0). The product is read per map
tile, so polygons extend past your bbox and `total_area_m2` is over the
tiles read, not the bbox. Quote `license` and `attribution` with any
map. Recorded on 2026-09-28 near Ludhiana: 19 input polygons, 17 after
cleaning, 2 duplicates dropped, overlap 165,206 m² resolved to 0.

## Step 2: was it burned?

```sh
curl -sf -X POST https://emem.dev/v1/field_burn_scar \
  -H 'content-type: application/json' \
  -d '{"geometry_geojson":{"bbox":[75.80,30.90,75.804,30.903]},
       "start":"2025-10-01","end":"2025-11-30","max_cells":16}' > burn.json
jq '{verdict, burn_events, events, observations, sensors_not_evaluated}' burn.json
```

To check one field rather than the box, put a polygon from Step 1 in
`geometry_geojson` in place of the `bbox`.

An event needs `max(2, 10 %)` of the cells that were clear on one
post-burn date to reach `likely_burn` or better, with both the NBR drop
(burn) and the NDTI drop (residue, which separates burning from tillage)
past threshold. `signal_only` means one sensor fired; that is triage,
not evidence. `inconclusive` means fewer than two clear Sentinel-2 dates
per cell: widen the window or retry once the warm-up has landed. CAMS
smoke bands are listed under `sensors_not_evaluated` with the reason.

Recorded on 2026-09-28 for a Ludhiana field, October to November 2025:
`verdict: "inconclusive"`, `clear_nbr_dates_per_cell_median: 0`,
`observations.warmed` showing Sentinel-2 NBR and NDTI not yet fetched
(the responder's Sentinel-2 path was saturated that hour). Report that
as "not enough clear imagery yet", never as "no burning".

## Step 3: how much water did it use?

```sh
curl -sf -X POST https://emem.dev/v1/field_actual_et \
  -H 'content-type: application/json' \
  -d '{"geometry_geojson":{"bbox":[77.03,32.57,77.036,32.573]},
       "start":"2026-05-01","end":"2026-06-30"}' \
  | jq '{verdict, et_mm, et_m3_per_ha, coverage, pixel_m,
         composites: [.composites[] | {composite_start, days_in_window, et_mm, fact_cid}]}'
```

Recorded on 2026-09-28: `verdict: "complete"`, `et_mm: 148.04`,
`et_m3_per_ha: 1480.38`, `coverage: 1.0`, eight 8-day composites (the
last one pro-rated to 5 days), each with its `fact_cid`; the receipt
verified offline.

This is MOD16A2 at one 463 m pixel over the field centre. A field
smaller than the pixel shares it with its neighbours, and the figure is
water evaporated and transpired, not irrigation applied. `partial`
sums only the days composites cover and does not extrapolate;
`no_valid_composite` is what MOD16 reports for urban, water and barren
pixels and for dates it has not published yet (a Ludhiana field for
June to September 2025 returned it on 2026-09-28).

## Step 4: look at it

```sh
curl -sf -o plot.png \
  'https://emem.dev/v1/scene.png?bbox=75.80,30.90,75.804,30.903'
```

The image is saved as `plot.png`: the newest clear Sentinel-2 pass over the
box, at native 10 m. `max_cloud` and `datetime` narrow the choice. To name
the scene, run `field_series` (above) over the same box: its rows give each
pass's date, scene id and cloud cover. Recorded on 2026-09-28: a 39 x 34 px PNG
from `S2C_MSIL2A_20260923T053641_R005_T43REQ_20260923T101809`, 14.74 % cloud.
The picture is a view, not a signed artifact; for pixels a third party
can re-hash, use [`emem-field-tokens`](../emem-field-tokens/SKILL.md).

## Verify and cite

Each JSON answer carries a `receipt` over the fact cids it used; check
it with [`emem-verify-receipt`](../emem-verify-receipt/SKILL.md), and
cite the per-cell or per-composite `fact_cid`s (or bundle them with
`POST /v1/memory_bundle {"fact_cids":[...]}`) rather than the verdict
alone. For an EUDR deforestation check on the same plot, see
[`emem-eudr-due-diligence`](../emem-eudr-due-diligence/SKILL.md).
