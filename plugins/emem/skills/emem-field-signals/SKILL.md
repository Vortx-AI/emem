---
name: emem-field-signals
description: Answers agronomy questions about one farm field with signed evidence from emem. It gets the boundary polygons of the field. It checks for crop-residue burns over a date window (Sentinel-2 NBR and NDTI plus MODIS burned area). It sums actual evapotranspiration over a season (MODIS MOD16). It fetches a true-colour Sentinel-2 picture of the plot. Use when the user asks whether a field burned or how much water a crop used. Also use it for the look of a plot or its edges, when the user wants per-cell fact ids behind the answer.
---

# emem-field-signals

| | |
|---|---|
| Use it when | The user asks about one farm field: its edges, a residue burn, the crop water use, or the look of the plot. |
| You need | The field as a bounding box, a GeoJSON polygon or a list of cell64s. For burn and water, a date window of at most 400 days. No key. |
| You get | Field polygons with a licence, a burn verdict, an evapotranspiration total in mm, or a PNG. The burn, water and series answers also give the fact cids that they used and a signed `receipt`. |
| Calls | `POST /v1/field_boundaries`, `POST /v1/field_burn_scar`, `POST /v1/field_actual_et`, `GET /v1/scene.png`, `POST /v1/field_series`. |

> **Network use.** The commands in this skill send JSON to `https://emem.dev` and receive JSON from it. One step also receives an image. `https://emem.dev` is the service that the MCP server of the plugin connects to. The commands do not execute what they download. This skill ships no scripts. Its commands read no environment variable, key or credential. The only local files that they read are files that an earlier step saved from `https://emem.dev`. This skill writes out every request body.

## Pick the call

Each question is one call.

| Call | Answers | Signed receipt |
|---|---|---|
| `POST /v1/field_boundaries` (MCP `emem_field_boundaries`) | The field polygons inside a bbox or place, from Fields of The World (CC-BY-4.0). | No |
| `POST /v1/field_burn_scar` | Was residue burned between two dates: `burn`, `signal_only`, `no_burn` or `inconclusive`. | Yes |
| `POST /v1/field_actual_et` | Actual evapotranspiration over a window, in mm and m³/ha. | Yes |
| `GET /v1/scene.png?bbox=w,s,e,n` | A true-colour Sentinel-2 crop of the plot at native 10 m, up to 1024 px a side. | No. Response headers name the scene. |
| `POST /v1/field_series` (MCP `emem_field_series`) | An index over the field for each scene: `ndvi`, `ndwi`, `ndmi` (alias `lswi`), `evi`, or Sentinel-1 `vv`, `vh`, `vh_vv`. | Yes |

`field_boundaries` and `field_series` are tools of the MCP server of
the plugin. They are not in the short tool list that the server shows.
The server runs them by name through `tools/call`. If your host calls
only listed tools, or has no MCP, send the same JSON body to the REST
route in the table.

`field_burn_scar` and `field_actual_et` have no MCP tool. Send their
bodies to the REST routes. The two routes take the same body:

| Field | Meaning |
|---|---|
| `start`, `end` | Dates as YYYY-MM-DD. Both dates are in the window. The window is at most 400 days. |
| `geometry_geojson` | A GeoJSON Polygon or MultiPolygon, or `{"bbox":[w,s,e,n]}`. |
| `cells` | A list of cell64s. Use this in place of `geometry_geojson`. |
| `budget_ms` | The warm-up budget. The default is 25000 and the maximum is 300000. |
| `max_cells` | Burn scar only. The number of cells to sample. The default is 16, the range is 1 to 64. |

The warm-up fetches facts that the responder does not hold yet. When
the budget ends, the call answers from the stored facts. The warm-up
continues after the answer, so a retry can read more facts.

## Step 1: get the field edges

Send this body to `POST https://emem.dev/v1/field_boundaries`, or call
`emem_field_boundaries` with it:

```json
{"polygon_bbox": {"min_lat": 30.90, "max_lat": 30.903, "min_lng": 75.80, "max_lng": 75.804}, "clean": true}
```

Save the answer as `fields.json`. Then print a summary:

```sh
jq -c '{count, returned, license, overlap_after_m2: .synthesis.overlap_after_m2,
  fields: [.geojson.features[] | {area_m2: .properties.area_m2, centroid: .properties.cell64_centroid}]}' fields.json
```

A result over the MCP wire budget (about 24 KB) comes back slimmed.
The `_emem_truncation` field names what the server left out. To get
every polygon, use the REST route.

`clean: true` resolves overlapping and duplicate polygons. Then
`synthesis.overlap_after_m2` must be 0. The responder reads the product
for each map tile, so polygons extend past your bbox. `total_area_m2`
is the area over the tiles read, not over the bbox. Quote `license` and
`attribution` with any map.

On 7 November 2026 this call gave 19 input polygons and 17 output
polygons. It dropped 2 duplicates and resolved an overlap of 165,206 m²
to 0. This answer has no `receipt`. Each polygon has a `polygon_cid`
that identifies its outline.

## Step 2: check for a residue burn

Send this body to `POST https://emem.dev/v1/field_burn_scar`:

```json
{"geometry_geojson": {"bbox": [75.80, 30.90, 75.804, 30.903]}, "start": "2025-10-01", "end": "2025-11-30", "max_cells": 16}
```

Save the answer as `burn.json`. Then print a summary:

```sh
jq -c '{verdict, burn_events, events, observations}' burn.json
```

To check one field and not the full box, put a polygon from Step 1 in
`geometry_geojson` in place of the `bbox`.

The call compares each pair of clear Sentinel-2 dates in each cell.
It uses three sensors:

| Sensor | Fires when |
|---|---|
| `indices.nbr` | The NBR drop (dNBR) is 0.2 or more. A drop alone can also be a harvest. |
| `indices.ndti` | The NDTI drop is 0.1 or more. This separates burning from tillage or mulch. |
| `modis.burned_area_monthly` | MCD64A1 marks a burn date in the pair. It is 500 m and monthly. |

Each pair with a dNBR of 0.1 or more gets a verdict in
`cell_signals[]`. The verdict depends on
the number of sensors that agree: 0 is `no_burn`, 1 is
`single_sensor_signal`, 2 is `likely_burn`, 3 is
`high_confidence_burn`. The call does not compare pairs more
than 30 days apart.

The verdict for the field:

| Verdict | Meaning | What to say |
|---|---|---|
| `burn` | On one post-burn date, `max(2, 10 %)` of the clear cells are `likely_burn` or better. `events[]` lists each event. | A burn, with the date and the fact cids. |
| `signal_only` | One sensor fired in a cell, but no event passed the test. | A signal for triage. It is not evidence. |
| `no_burn` | No sensor fired, with at least 2 clear dates for each cell (median). | No burn signature found in the window. |
| `inconclusive` | Fewer than 2 clear Sentinel-2 dates for each cell (median). | Not enough clear imagery yet. Never say "no burning". |

For `inconclusive`, make the window wider, or retry after the warm-up
ends. `observations.warmed` shows which bands finished the warm-up in
the budget. CAMS smoke bands are in `sensors_not_evaluated` with the
reason: they are 0.4° model fields.

On 7 November 2026 this call gave `verdict: "signal_only"` and
`burn_events: 0`. Each of the 16 cells had a median of 14 clear dates.
Two of ten cell signals were `single_sensor_signal`. The receipt named
451 fact cids. These counts grow while the warm-up adds facts: an
earlier call on the same day saw 6 clear dates and 195 fact cids. On
28 September 2026 the same call gave `inconclusive`, because the
responder had not fetched the Sentinel-2 facts yet.

## Step 3: get the crop water use

Send this body to `POST https://emem.dev/v1/field_actual_et`:

```json
{"geometry_geojson": {"bbox": [77.03, 32.57, 77.036, 32.573]}, "start": "2026-05-01", "end": "2026-06-30"}
```

Save the answer as `et.json`. Then print a summary:

```sh
jq -c '{verdict, et_mm, et_m3_per_ha, coverage, pixel_m,
  composites: [.composites[] | {composite_start, days_in_window, et_mm, fact_cid}]}' et.json
```

On 7 November 2026 this call gave `verdict: "complete"`, `et_mm: 148.04`,
`et_m3_per_ha: 1480.38` and `coverage: 1.0`. It used eight 8-day
composites, each with its `fact_cid`. The last composite counts 5 of
its 8 days.

| Verdict | Meaning |
|---|---|
| `complete` | Composites cover 90 % or more of the window. |
| `partial` | Composites cover less than 90 %. The sum counts only the covered days. It does not extrapolate. |
| `no_valid_composite` | No composite. MOD16 gives this for urban, water and barren pixels, and for dates that it has not published yet. |

The value is MOD16A2 at one 463 m pixel over the field centre. A field
smaller than the pixel shares it with its neighbours. The value is the
water that the field evaporated and transpired. It is not the
irrigation applied.

## Step 4: look at the plot

No MCP tool returns this picture. Send a GET to this URL and save the
answer as `plot.png`:

```text
https://emem.dev/v1/scene.png?bbox=75.80,30.90,75.804,30.903&datetime=2025-10-01/2025-10-31&max_cloud=30
```

Read the response headers `x-emem-scene-item-id`,
`x-emem-scene-datetime` and `x-emem-scene-cloud-cover`.

The PNG is a Sentinel-2 pass over the box at native 10 m. The
`x-emem-scene-item-id`, `x-emem-scene-datetime` and
`x-emem-scene-cloud-cover` headers name the scene. On 7 November 2026
this call gave a 39 x 34 px PNG from scene
`S2C_MSIL2A_20251028T053951_R005_T43REQ_20251028T084920`, 1.96 % cloud.

| Parameter | Meaning |
|---|---|
| `datetime` | An ISO-8601 interval such as `2025-10-01/2025-10-31`. |
| `at` | One date. The search looks back 7 days from it. |
| `max_cloud` | The highest scene cloud cover in percent. The default is 20. |

Without `datetime`, `at` or `max_cloud`, the responder searches recent
scenes and widens the search step by step. On 7 November 2026 that
default search for this box failed with HTTP 502
`scene_upstream_failed` ("no valid red pixels"). If you get this
error, set `datetime` and `max_cloud`.

If every search misses, the answer is HTTP 404 `no_clear_scene` with
the searches tried. The picture is a view, not a signed artifact. For
pixels that a third party can hash again, use
[`emem-field-tokens`](../emem-field-tokens/SKILL.md).

## Step 5: get an index for each scene

Send this body to `POST https://emem.dev/v1/field_series`:

```json
{"bbox": [75.80, 30.90, 75.804, 30.903], "index": "ndvi", "start_date": "2025-10-01", "end_date": "2025-10-31"}
```

Save the answer as `series.json`. Then print one line for each scene:

```sh
jq -c '.rows[] | {date, mean, clear_fraction, quality, scene, scene_cloud_cover}' series.json
```

On 7 November 2026 this call gave 6 rows. The first row was
2025-10-08, mean NDVI 0.523, `clear_fraction` 1.0.

| Option | Meaning |
|---|---|
| `geometry` or `bbox` | The field. |
| `orbit` | Radar only. The relative orbit to read. Radar reads one orbit. |
| `fields: [{id, geometry}]` | Up to 500 fields in one window of at most 512 px a side. The answer adds a `suspect` column and `suspect_dates`. |
| `inner_buffer_m` | Drops pixels near the field edge. |
| `min_support_px` | The fewest pixels that a scene needs, 4 to 30. |

Radar rows have no `clear_fraction`. Their `mean` is the dB of the
mean linear power, with `se_db`, `pixel_enl`, `below_nesz_share` and
`bright_rejected`. On optical indices, a scene that both time
neighbours refute gets `quality: "suspect"` and a `scene_check`. The
trend does not use it.

## Verify and cite

The burn, water and series answers carry a `receipt` over the fact
cids that they used. Check it with
[`emem-verify-receipt`](../emem-verify-receipt/SKILL.md). Cite the
per-cell or per-composite `fact_cid` values, not the verdict alone.
`POST /v1/memory_bundle {"fact_cids":[...]}` collapses many cids into
one token, but it stores a record on the responder. For an EUDR
deforestation check on the same plot, see
[`emem-eudr-due-diligence`](../emem-eudr-due-diligence/SKILL.md).
