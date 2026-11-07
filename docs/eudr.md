# EUDR: Due Diligence Statements + Visual Evidence

`POST /v1/eudr_dds` makes a signed Due Diligence Statement in the Annex II shape of
[Regulation (EU) 2023/1115](https://eur-lex.europa.eu/eli/reg/2023/1115/oj).
It takes one or more plots that the operator supplies.
The endpoint covers the geolocation and deforestation parts of Annex II.

It does not cover the legality check of Article 9(1)(b): land tenure, FPIC and the laws
of the country of origin. Earth observation cannot measure these.
The response carries a `legality_disclaimer` for this reason.
The operator sends the legality part to a partner module before the statement goes to
TRACES NT.

The JSON Schema is at
[`/v1/schemas/eudr_dds.json`](https://emem.dev/v1/schemas/eudr_dds.json) (schema id
`emem.eudr_dds.v1`).
The schema does not list every field in the response.
On 7 November 2026 it did not list `loss_year_histogram`, `tmf_cross_check`,
`sampled_polygon_fraction`, `degradation_disclaimer` or `regulation_status_note`.
The schema does not set `additionalProperties`, so these extra fields pass validation.
The visual `verdict` enum is different: see [Verdict thresholds](#verdict-thresholds).

## Per-cell verdict

The endpoint samples each polygon plot at `max_cells_per_plot` cell64s inside the polygon.
A plot without a polygon (a GeoJSON Point or a `bbox`) of 4 ha or less uses 1 cell,
unless the commodity is cattle.
If the operator does not set a count, the sampler uses about 110 cells per hectare of the
largest plot in the request.
All other plots in that request get this same count.
The count is at least 55 and at most 51,200.
If no plot geometry gives an area, the count is 64.
The endpoint clamps a count that the operator sets to 1 through 51,200.
The verdict runs **`eudr_compliance@1`** on each cell, from two static COG baselines:

| Band | Role |
|---|---|
| `jrc_gfc2020.forest_2020`        | EUDR forest baseline at the 2020-12-31 cut-off (Bourgoin et al. 2026 ESSD); single-band uint8 `1`=forest |
| `forest_change.treecover2000`    | Hansen GFC v1.13 canopy fraction (%) at year 2000 |
| `forest_change.lossyear`         | Hansen GFC v1.13 first-loss year as a **calendar year** (`0`=no loss, else `2001..=2025`); loss strictly after the cut-off year is the failure signal |

The algorithm card at `GET /v1/algorithms/eudr_compliance@1` still names
Hansen v1.12 (loss years `2001..2024`) on 7 November 2026. The `eudr_dds@1` card names no Hansen version.
That text comes from `crates/emem-core/data/algorithms-v0.json`, which did not change when the connector moved to v1.13.
The connector in `crates/emem-fetch/src/hansen_gfc.rs` reads `GFC-2025-v1.13`.
The `eudr_dds@1` card also writes the cut-off test as `>= 21`, the raw upstream byte.
The responder decodes that byte to a calendar year, so the test it applies is `>= 2021`.
The `eudr_dds@1` card lists `radd.alert_date` and `wri_gdm.driver_class` as inputs.
This endpoint does not read them (see below).

The endpoint reads each baseline band with one `cog::sample_window` call over the polygon
bounding box.
It then finds each cell in that in-memory buffer.
Thus the upstream reads stay constant and the lookups grow with the cell count (O(1) and
O(N)).
With warm caches, a fully sampled polygon gets its base verdict in a few seconds.
This batched read is on by default. An operator can set `EMEM_EUDR_BATCH_PATH=0` to read each cell on its own.

The endpoint also reads the JRC TMF v1.2025 deforestation year on every cell.
It reads a COG copy of the JRC tiles with HTTP Range requests.
The plot result reports it as `tmf_cross_check`, which shows where TMF and Hansen agree on
loss after the cut-off.
TMF does not change the verdict. A loss that only TMF sees sets `review_required`.
This endpoint does not use WRI-Sims driver attribution or RADD SAR alerts.

### From cells to a plot verdict

The plot verdict combines the cell verdicts and applies the 0.5 ha minimum area of
Article 2(4).
If `failing_area_ha < 0.5`, the plot gets `below_mmu` instead of `fail`.
The DDS total counts a `below_mmu` plot as compliant.
A cell that lost its forest at or before the cut-off year was not forest at the cut-off.
Its verdict is `not_in_scope`, never `pass`.

### When the responder holds the statement for review

A `pass` does not always give a statement that you can sign.
Each plot carries a `verdict_support` block. It grades the inputs by a fixed rule, not by a probability.

| Input | `strong` needs | `weak` when |
|---|---|---|
| Cells with both the JRC baseline and the Hansen loss year | 95 % or more | below 80 % |
| TMF and Hansen agree on loss after the cut-off, where TMF reads | 90 % or more | below 70 % |
| Cells within 2 points of the 10 % canopy threshold | 10 % or less | above 30 % |
| Distance between samples, on a plot of 0.5 ha or more | 71 m or less | above 141 m |

A value between the two columns gives `moderate`.
The block lists each limit in `limits`, with `severity` `weakens` or `qualifies`.
Three more cases add a `qualifies` limit:

- a plot with one point sample
- a plot smaller than 0.5 ha
- a plot within 30 degrees of the equator where TMF read no cell.

A `qualifies` limit gives `moderate` at best.
Thus a plot with one sample, or a plot under 0.5 ha, never gets `strong`.
The responder counts a GeoJSON Point as 0.01 ha, so a Point plot gets both limits.

The sample spacing is the square root of the plot area divided by the cell count.
A `bbox` plot of 4 ha or less gets one sample, so its spacing is the square root of the box area.
Above 2 ha, that spacing is more than 141 m.
The level is then `weak`, and the responder holds the statement.
For a `bbox` plot of more than 2 ha, send the plot as a polygon.
A polygon plot gets about 110 cells per hectare.

The responder holds the statement back when one of these is true:

| `statement_of_compliance_held_because` value | Cause |
|---|---|
| `tmf_cross_check_review_required` | On a plot, TMF sees a loss after the cut-off that Hansen does not |
| `verdict_support_weak` | A plot has `verdict_support.level` `weak` |
| `jrc_baseline_unavailable_hansen_only` | The verdict ran on Hansen alone, because the JRC baseline gave no value |

The list names every cause that applies.
If the DDS verdict is not `pass`, the list starts with `verdict_<label>`, for example `verdict_fail`.
`statement_of_compliance_signable` is true only for a `pass` with an empty list of causes.
When the responder holds it, `statement_of_compliance` gives the reason in place of the Annex II point 5 text.
The code is `verdict_support`, `tmf_cross_check` and `statement_of_compliance` in
`crates/emem-api-rest/src/lib.rs`.

## Loss-year histogram

Each plot carries a `loss_year_histogram`.
It shows how the Hansen loss year spreads across the sampled cells of the plot.
It has these fields:

| Field | Meaning |
|---|---|
| `by_year` | Calendar year mapped to a cell count |
| `after_cutoff_cells` | Cells with loss after the cut-off year |
| `no_loss_cells` | Cells with no loss |
| `unknown_cells` | Cells with no loss-year value |
| `total_sampled_cells` | All sampled cells |

The endpoint signs the histogram as its own derived fact, `forest_change.lossyear_histogram`.
That fact has `op: "histogram"`, and its `parents` cite the `forest_change.lossyear` facts
of the cells.
The response receipt includes the CID of the histogram fact.
Thus the breakdown is a signed figure that anyone can make again.
The same cells give the same bytes, and the receipt binds the result.

The counts cover only the sampled cells (`basis: "sampled_cells"`).
To estimate the whole polygon, weight the counts by the plot's
`sampled_polygon_fraction`.
On very large plots the endpoint limits the `parents` list and sets `parents_capped`.
`total_sampled_cells` is always the full count.

## Visual evidence (opt-in)

Set `request_visual_evidence: true` on a plot to add a `visual_evidence` block to it.
The block shows in images whether the plot lost forest between the 2020-12-31 cut-off and
the current calendar year.

```json
{
  "plots": [{
    "plot_id": "CACAO-01",
    "geometry_geojson": {"type": "Polygon", "coordinates": [[ ... ]]},
    "country_of_production": "CIV",
    "commodity_hs": "180100",
    "quantity_kg": 500.0,
    "request_visual_evidence": true
  }],
  "activity_type": "IMPORT"
}
```

The block has the schema id `emem.visual_evidence.v2`. It contains these parts:

- Sentinel-2 `indices.ndvi` at a July-1 anchor for each year from 2020 through the current
  calendar year.
  The scene picker `s2_pick_clear_scene` chooses one scene for each cell and year.
  It tries three steps. With the default values, each step widens the scene cloud cap and the date window:

  | Step | Scene cloud cover below | Window around the anchor |
  |---|---|---|
  | 1 | 40 % | ±30 days |
  | 2 | 60 % | ±60 days |
  | 3 | 80 % | ±90 days |

  In each step it takes up to 4 scenes, newest first, and reads the SCL class at the pixel.
  It uses the newest scene whose pixel is not no-data, defective, cloud or thin cirrus
  (SCL classes 0, 1, 8, 9 and 10). Cloud shadow (class 3) does not reject a scene.
  If the SCL read fails, the scene cloud cap alone decides.
  Thus the picker does not choose the clearest scene. It chooses the newest usable scene.
  The block's `method` string says `annual_s2_l2a_least_cloudy`, but this paragraph gives the real rule.
  If scenes exist but no pixel is usable, the signed fact records an absence.
  If no step finds a scene, that cell and year get an error and no fact.
  The window never goes past the current date.
  Operators can change the base values with three variables.
  `EMEM_S2_MAX_CLOUD` has default 40 and range 0 to 100.
  `EMEM_S2_LOOKBACK_DAYS` has default 30 and range 1 to 365.
  `EMEM_S2_MAX_SCENES` has default 4 and range 1 to 12. Step 2 uses 1.5 times the base cloud value (at most 60 %)
  and 2 times the base window. Step 3 uses 2 times the base cloud value (at most 80 %)
  and 3 times the base window.
  Thus a base cloud value above 60 gives a step 2 cap that is lower than the step 1 cap.
  A value outside its range gives the default. The responder does not clamp it.
- Sentinel-1 RTC VV backscatter at the same anchors, from `materialize_sentinel1_vv`.
  Radar has no cloud filter. Its window widens from ±15 to ±30 to ±60 days.
  `EMEM_S1_LOOKBACK_DAYS` sets the base window (default 15, range 1 to 365).
  The second and third steps use 2 times and 4 times the base window.
  A value outside the range gives the default.
  It uses the newest scene in the first window that has a scene.
  The window never goes past the current date.
  The signed fact has band `sentinel1_raw` and holds VV in dB (`10 × log10` of the linear value).
  Its derivation arguments record the half-width in days of the window that found the scene.
  If the pixel value is zero, negative or not a number (water mask or no-data), that cell
  and year get an error and no fact.
  Radar sees through cloud, so this signal confirms the NDVI in monsoon regions where
  Sentinel-2 has gaps.
- Per-year `scene.png` URLs for up to 6 cells of the plot, in `scene_png_urls` and
  `scene_metadata`.
  Each URL asks for `max_cloud=20` and a window of ±20 days (`EMEM_VISUAL_SCENE_WINDOW_DAYS`,
  1 to 120; a value outside the range gives the default).
  The endpoint centres the window on the earliest NDVI observation date of that year.
  If the year has no NDVI date, it uses the July-1 anchor as the centre.
  The field `scene_coregistered_with_ndvi` tells you which centre the URL uses.
- The [`GET /v1/cells/{cell64}/scene.png`](https://emem.dev/openapi.json) handler returns
  the latest scene in the window with less than 20 % cloud.
  It does not pick the clearest scene.
  The `x-emem-scene-datetime` and `x-emem-scene-cloud-cover` headers name the scene it used.
  These URLs set `max_cloud` and `datetime`, so the handler tries one search only.
  If no scene passes, it returns 404 `no_clear_scene` and does not widen the search.
  Show the images as a year-by-year grid in an audit packet.

On 7 November 2026, a request at the Mount Fuji cell `defi.zb592.nUkO.zEzE` with the window
2025-06-11 to 2025-07-21 returned the scene of 2025-07-20, with 10.59 % cloud:

```sh
curl -s -o scene.png -D - \
  "https://emem.dev/v1/cells/defi.zb592.nUkO.zEzE/scene.png?datetime=2025-06-11T00%3A00%3A00Z/2025-07-21T00%3A00%3A00Z&max_cloud=20" \
  | grep -iE '^x-emem-scene-(datetime|cloud-cover)'
```

### Values in each year entry

The block reads the first cells of the plot sample, 8 by default (see [Time budget](#time-budget)).
Each entry in `years[]` gives statistics over those cells, not one value for the plot.

| Field | What it holds |
|---|---|
| `ndvi_median`, `ndvi_p10`, `ndvi_p90` | Median, 10th and 90th percentile of NDVI over the cells with a number |
| `ndvi_delta_vs_2020` | `ndvi_median` minus the 2020 `ndvi_median` |
| `s1_vv_db_median` | Median VV backscatter in dB over the cells with a number |
| `s1_vv_delta_db_vs_2020` | `s1_vv_db_median` minus the 2020 value |
| `n_cells_ndvi_ok`, `n_cells_s1_ok` | Cells that gave a number |
| `n_cells_ndvi_errors`, `n_cells_s1_errors` | Cells that gave an error or ran out of time, with no fact |
| `ndvi_fact_cids`, `s1_fact_cids` | Every fact the read stored, including absence facts |

A cell with a signed absence (no usable pixel) is in `ndvi_fact_cids`.
It is not in `n_cells_ndvi_ok` or in `n_cells_ndvi_errors`.
Thus the two counts can add up to less than the number of cells.
A median is `null` when no cell gave a number for that year.

### Dates to cite in a statement

The block has one entry for each year in `years`.
The field `anchor_unix` in each entry is the July-1 date that the request asked for.
It is not the date of a satellite pass. Do not cite it as an evidence date.

Cite these fields instead:

| Field | What it holds |
|---|---|
| `ndvi_observed_date_range` | `from` and `to`: the earliest and latest Sentinel-2 `captured_at` in the plot cells for that year |
| `s1_observed_date_range` | The same range for the Sentinel-1 facts |
| `x-emem-scene-datetime` header | The date of the scene in one `scene.png` image |

Each range is `null` when no cell has a fact with a number and a date for that year.
The block repeats this rule in its `date_correctness_note` field.
Its `anchor_policy` field names a ±30, 60 and 90 day ladder.
That ladder is the Sentinel-2 ladder only. Radar uses ±15, 30 and 60 days.

### Verdict thresholds

Operators can change both thresholds with environment variables.
Use a lower value for audits with high stakes and a higher value for noisy regions.
The responder clamps each value to its range.
It uses the default when it cannot parse the value.

| Signal | Default threshold | Env var | Range | Source |
|---|---|---|---|---|
| NDVI drop vs 2020 baseline       | ≥ 0.15 | `EMEM_VISUAL_NDVI_DROP_THRESHOLD` | 0.05 to 0.50 | Pelletier et al. 2024 |
| S1 VV backscatter drop vs 2020   | ≥ 3 dB | `EMEM_VISUAL_S1_DROP_DB_THRESHOLD` | 1 to 10 dB | Reiche et al. 2018 |

The two signals use different years:

- NDVI breaches when the drop from the 2020 median reaches the threshold in any year.
- S1 VV breaches only when the latest year drops from the 2020 median by the threshold.

The block reports the values it used in `thresholds`.
`metrics.ndvi_max_drop_vs_baseline` gives the largest NDVI drop.
`metrics.ndvi_worst_year_over_year_drop` gives the largest drop from one year to the next.
That second value is for information only; the verdict does not use it.

The block carries its own `verdict`:

| Visual verdict | When |
|---|---|
| `visual_deforestation_suspected` | One or both signals reach the threshold |
| `no_visual_deforestation` | Both signals stay below the threshold |
| `indeterminate_no_baseline` | The plot has no 2020 baseline data; the block does not pass it silently |
| `incomplete` | The visual time budget ran out before the timeline was complete |

The schema enum for this `verdict` lists only the first three values.
On 7 November 2026 it did not list `incomplete`, but the endpoint can return it.
If you validate responses against the schema, allow `incomplete` in your validator.

The block evaluates S1 VV only when the plot has a 2020 median and a latest-year reading.
It evaluates NDVI when the plot has a 2020 median and a drop in any year, or a latest-year reading.
`verdict_basis` names the signals that the verdict uses: `ndvi_and_s1_vv`, `ndvi_only`,
`s1_vv_only` or `neither`.
`signals.s1_vv.why_not` gives the reason when the block did not evaluate S1 VV.
Read `verdict_basis` before you quote `no_visual_deforestation`.
With `ndvi_only`, only NDVI cleared the plot, because S1 VV did not run.

### Receipts

The response has one Ed25519 `receipt` for the whole call.
It signs the per-cell fact CIDs of every plot and the CID of each loss-year histogram fact.
It also signs the cell64 of each sampled cell.
Check it offline at `/verify`, or with `POST /v1/verify_receipt`.

Large calls hit two caps.
The table gives the defaults; both env vars accept 256 to 51,200.

| Cap | Default | Env var | When the cap applies |
|---|---|---|---|
| Fact CIDs in the receipt | 2048 | `EMEM_EUDR_RECEIPT_MAX_FACTS` | The receipt keeps the first 2048 CIDs in sort order. The response adds `receipt_fact_cids_capped: true`, `receipt_fact_cids_total` and `receipt_fact_cids_cap`. |
| Cells in the receipt | 2048 | `EMEM_EUDR_RECEIPT_MAX_CELLS` | The receipt lists the centroid cell of each plot. The response adds `receipt_cells_truncated: true`, `receipt_cells_total` and `receipt_cells_cap`. |

The sort runs only above the cap. It makes two capped calls with the same
inputs keep the same fact CIDs. The receipts are still not byte-equal,
because `request_id` and `served_at` change on each call. A capped receipt does not sign every fact, but the endpoint stores every fact.
Each plot's `per_cell_rows_cid` addresses all of its facts.
Every fact CID in the response resolves at `/v1/facts/<cid>`.

The receipt does not sign the visual evidence facts.
Each annual NDVI and S1 backscatter value is a separate signed Primary fact under the
responder's identity.
Each entry in `visual_evidence.years[]` carries `ndvi_fact_cids[]` and `s1_fact_cids[]`.
An auditor can cite the exact upstream fact with these CIDs.
The auditor can check its signature at `/verify/<fact_cid>` and does not need to trust the
response envelope.

### Time budget

The visual block reads 8 cells of the plot by default (`EMEM_EUDR_VISUAL_CELLS`, 1 to 32).
The responder does not clamp this variable: a value outside 1 to 32 gives the default of 8.
The responder clamps the variables in the table below to their range.
It reads 2 bands for each year from 2020 through the current year.
In 2026 that is 7 years × 8 cells × 2 bands = 112 reads for each plot.

| Limit | Default | Env var | Range |
|---|---|---|---|
| Visual block, for each plot | 60 s | `EMEM_EUDR_VISUAL_BUDGET_SECS` | 5 to 150 s |
| Whole `/v1/eudr_dds` call | 280 s | `EMEM_EUDR_TIMEOUT_SECS` | 15 to 600 s |
| Visual reads at one time | 6 | `EMEM_EUDR_VISUAL_CONCURRENCY` | 1 to 64 |
| EUDR requests at one time | 3 | `EMEM_EUDR_MAX_INFLIGHT` | 1 to 64 |
| Wait for a free EUDR slot | 30 s | `EMEM_EUDR_QUEUE_SECS` | 0 to 120 s |

`/v1/eudr_dds`, `/v1/field_burn_scar` and `/v1/field_actual_et` share these slots.
When all slots are in use, up to 4 times that number of requests wait for a slot.
A request that gets no slot in the wait time gets HTTP 503 with `code: "overloaded"` and `Retry-After: 1`.
A request that arrives when the wait queue is full gets the same 503 at once.
A wait time of 0 sends the 503 at once when all slots are in use.

If the visual budget runs out, the plot still gets its verdict.
The visual block then has `verdict: "incomplete"`.
The endpoint stores the reads that finished, and a later request for the same plot reuses them.
If the upstream reads are slow, the block can need more than one more request.
The 280 s default stays below the client and edge cut of about 300 s.
Thus a call that cannot finish gets a clear 504 response before the connection closes.

## Compliance position

EUDR Article 2(4) defines forest by canopy, height and area, **not by data source**.
Bourgoin et al. 2026 name JRC GFC2020 with Hansen as the baseline that the Commission
expects. This baseline is not binding.
The `eudr_compliance@1` verdict uses this baseline because it gives the strongest audit
position today.
The `visual_evidence` block is extra evidence in support.
It does not change the strict pass or fail verdict.

The verdict measures **deforestation**.
Article 2(3) defines this as forest changed to non-forest after the cut-off; emem measures
it through canopy loss.
The verdict does not measure **forest degradation** (Article 2(7)).
Degradation is a change in structure that reduces the biomass or ecological capacity of a
forest.
One example is primary or naturally regenerating forest that becomes planted forest.
Another is selective loss that keeps the canopy above the 10 % threshold.

The standard `pass` statement of compliance makes a claim about both deforestation and
degradation.
For this reason, the response carries a `degradation_disclaimer`.
The operator must handle degradation with a separate layer.
You can recall the JRC TMF `jrc_tmf.degradation_year` band by name.
`/v1/eudr_dds` does not read it, and it does not enter the verdict.
The `legality_disclaimer` for Article 9(1)(b) works the same way.
Both disclaimers state the limits of the Earth-observation verdict.

### Application timeline

The deforestation cut-off is 31 December 2020 (Article 1(2)). It does not change.
Regulations (EU) 2024/3234 and 2025/2650 moved the start of the obligations.
For large operators they apply from 30 December 2026.
For micro and small enterprises they apply from 30 June 2027.
The `regulation_status_note` field in the response gives the current position.
The deferral does not change the cut-off that the verdict uses.

## Related endpoints

| Request | What it returns |
|---|---|
| `GET /v1/algorithms/eudr_compliance@1` | Card for the per-cell rule: `inputs`, `formula`, `accuracy_band`, `citation` |
| `GET /v1/algorithms/eudr_dds@1` | Card for the polygon step that wraps `eudr_compliance@1` |
| `GET /v1/schemas/eudr_dds.json` | JSON Schema (draft 2020-12) of the response. It does not list every response field. |
| `GET /v1/cells/{cell64}/scene.png` | True-colour Sentinel-2 chip, 256 x 256 pixels, PNG |
| `GET /v1/cells/{cell64}/scene.rgb` | The same chip as raw bytes, without PNG framing |
| `GET /v1/scene.png?bbox=w,s,e,n` | True-colour Sentinel-2 chip cropped to a plot box in WGS 84, PNG |

All six requests returned 200 on 7 November 2026.

### Read a scene chip

The three scene requests use one pipeline and send the same `x-emem-scene-*` headers.
Only `.rgb` sends `x-emem-scene-width` and `x-emem-scene-height`, because a PNG holds its own size.
If you send no `max_cloud`, `datetime` or `at`, the handler tries up to three searches:

| Step | Cloud cap | Window before now | `x-emem-scene-fallback` header |
|---|---|---|---|
| 1 | 20 % | 90 days | not sent |
| 2 | 20 % | 365 days | `widened_window` |
| 3 | 60 % | 365 days | `relaxed_cloud` |

If you send `max_cloud` or a window, the handler tries one search only.
That search uses a 20 % cap if you send no `max_cloud`.
It uses the last 90 days if you send no window.
The `at=<date>` parameter sets a window of 7 days that ends at that date.
If no search finds a scene, the handler returns 404 `no_clear_scene` with a `tried` list.
If a fetch from the upstream fails, it returns 502 `scene_upstream_failed`.

The bbox request reads at 10 m and crops the scene to your box.
Each side is at least 32 and at most 1024 pixels, so the box can be about 10 km wide.
A larger box gets 400 with the size it would need.
On 7 November 2026, `bbox=138.72,35.35,138.74,35.37` near Mount Fuji returned a PNG of 182 x 222 pixels.

The `.rgb` body is `width x height x 3` bytes: 8-bit R, G and B values, one pixel after the next.
Read the size from `x-emem-scene-width` and `x-emem-scene-height`.
`x-emem-scene-bbox-crs` gives the bounds as `min_x,min_y,max_x,max_y` in the CRS of `x-emem-scene-epsg`.
The bounds are not in WGS 84. Project your point to that CRS first, then find its pixel:
`col = (x - min_x) / pixel_x` and `row = (max_y - y) / pixel_y`.
`x-emem-scene-pixel-size` gives `pixel_x,pixel_y` in metres.

On 7 November 2026, this request at the Mount Fuji cell returned 196,608 bytes (256 x 256 x 3):

```sh
curl -s -o scene.rgb -D - \
  "https://emem.dev/v1/cells/defi.zb592.nUkO.zEzE/scene.rgb" \
  | grep -iE '^x-emem-scene-(width|height|datetime|cloud-cover|epsg|bbox-crs|pixel-size|fallback)'
```

It sent no fallback header, so step 1 found the scene of 2026-10-28, with 9.90 % cloud.
The EPSG was 32654 and the pixel size was 10 m.
