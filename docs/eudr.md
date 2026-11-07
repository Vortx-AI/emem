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

The endpoint samples each plot at `max_cells_per_plot` cell64s inside the polygon.
If the operator does not set a count, the sampler uses the plot area: about 110 cells per
hectare. The limit is 51,200 cells per plot.
The verdict runs **`eudr_compliance@1`** on each cell, from two static COG baselines:

| Band | Role |
|---|---|
| `jrc_gfc2020.forest_2020`        | EUDR forest baseline at the 2020-12-31 cut-off (Bourgoin et al. 2026 ESSD); single-band uint8 `1`=forest |
| `forest_change.treecover2000`    | Hansen GFC v1.13 canopy fraction (%) at year 2000 |
| `forest_change.lossyear`         | Hansen GFC v1.13 first-loss year as a **calendar year** (`0`=no loss, else `2001..=2025`); loss strictly after the cut-off year is the failure signal |

The algorithm cards at `GET /v1/algorithms/eudr_compliance@1` and `eudr_dds@1` still name
Hansen v1.12 (loss years `2001..2024`) on 7 November 2026.
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
  The `s2_search_with_fallback` cloud ladder finds the clearest scene for each cell.
  The ladder tries 40, then 60, then 80 % cloud, with a window of ±30, then 60, then 90 days.
  Per-pixel SCL masking then removes the remaining cloud pixels.
- Sentinel-1 RTC VV backscatter at the same anchors, from `materialize_sentinel1_vv`.
  Radar sees through cloud, so this signal confirms the NDVI in monsoon regions where
  Sentinel-2 has gaps.
- Per-year `scene.png` URLs for up to 6 cells of the plot.
  Each URL asks for a full-year window, `?datetime=YYYY-01-01.../YYYY-12-31...`.
  The [`GET /v1/cells/{cell64}/scene.png`](https://emem.dev/openapi.json) handler then
  picks the latest clearest scene in that year.
  Show them as a year-by-year grid of 6 images in an audit packet.

### Verdict thresholds

Operators can change both thresholds with environment variables.
Use a lower value for audits with high stakes and a higher value for noisy regions.

| Signal | Default threshold | Env var | Source |
|---|---|---|---|
| NDVI drop vs 2020 baseline       | ≥ 0.15 | `EMEM_VISUAL_NDVI_DROP_THRESHOLD` | Pelletier et al. 2024 |
| S1 VV backscatter drop vs 2020   | ≥ 3 dB | `EMEM_VISUAL_S1_DROP_DB_THRESHOLD` | Reiche et al. 2018 |

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

### Receipts

Each annual NDVI and S1 backscatter value is a signed Primary fact under the responder's
identity.
Each entry in `visual_evidence.years[]` carries `ndvi_fact_cids[]` and `s1_fact_cids[]`.
An auditor can cite the exact upstream fact with these CIDs.
The auditor can check its signature at `/verify/<fact_cid>` and does not need to trust the
response envelope.

### Time budget

The visual block reads 8 cells of the plot by default (`EMEM_EUDR_VISUAL_CELLS`, 1 to 32).
It reads 2 bands for each year from 2020 through the current year.
In 2026 that is 7 years × 8 cells × 2 bands = 112 reads for each plot.

| Limit | Default | Env var | Range |
|---|---|---|---|
| Visual block, for each plot | 60 s | `EMEM_EUDR_VISUAL_BUDGET_SECS` | 5 to 150 s |
| Whole `/v1/eudr_dds` call | 280 s | `EMEM_EUDR_TIMEOUT_SECS` | 15 to 600 s |
| Visual reads at one time | 6 | `EMEM_EUDR_VISUAL_CONCURRENCY` | 1 to 64 |
| EUDR requests at one time | 3 | `EMEM_EUDR_MAX_INFLIGHT` | 1 to 64 |

If the visual budget runs out, the plot still gets its verdict.
The visual block then has `verdict: "incomplete"`.
The endpoint stores the reads that finished, so the next request for the same plot
completes the block.
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
The JRC TMF `jrc_tmf.degradation_year` band is available on request.
The endpoint does not read it, and it does not enter the verdict.
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

- `GET /v1/algorithms/eudr_compliance@1`: input bands, formula, accuracy notes
- `GET /v1/algorithms/eudr_dds@1`: algorithm card for the polygon step
- `GET /v1/schemas/eudr_dds.json`: JSON Schema (draft 2020-12); it does not list every response field
- `GET /v1/cells/{cell64}/scene.png`: true-colour Sentinel-2 RGB chip
- `GET /v1/cells/{cell64}/scene.rgb`: raw 256×256×3 byte stream that the client draws itself
