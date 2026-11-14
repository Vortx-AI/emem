# EUDR: Due Diligence Statements and visual evidence

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
The schema enum for the visual `verdict` also lacks one value (see
[Verdict thresholds](#verdict-thresholds)).

## How a plot is read

The request field `method` chooses how the endpoint reads a plot.

| `method` | What it reads | What it signs |
|---|---|---|
| `raster` (default) | Every 10 m JRC GFC2020 pixel the polygon or `bbox` covers, with the part of each that it covers | One fact for the plot |
| `sampled` | Cells sampled inside the plot, about 110 for each hectare of its bounding box | One fact for each cell and band, and one loss-year histogram |

A GeoJSON Point plot is read at its one pixel by either method: one JRC pixel and one Hansen pixel.
Its `method_note` says so. Send the plot as a polygon to read all of it.
The sampled method stays for one release.
Each plot says which method ran in `method`.
When the raster method cannot read a plot, the endpoint samples it and `method_note` says why:
the JRC GFC2020 tile under the plot cannot be opened or read, or the polygon covers no pixel.

The sampled method is thin on a large plot.
It caps a plot at 51,200 cells, so above about 465 ha of bounding box its cells are more than 10 m apart.
On a long, narrow plot it also samples one axis more thinly than the other.
The raster method has no cap: it reads every 10 m pixel of a plot of any size.

Both methods apply one per-cell rule, `eudr_verdict_for` in `crates/emem-api-rest/src/lib.rs`.
The forest rule in that function is in [The forest rule](#the-forest-rule).

### The raster method

The raster method reads four sources, each as a window over the plot's bounding box:

| Source | Band | Grid |
|---|---|---|
| JRC GFC2020, version in `forest_baseline_dataset` | `jrc_gfc2020.forest_2020` (`1` = forest) | 1/12,000 degree, about 9.3 m |
| Hansen GFC v1.13 | `forest_change.treecover2000` (% canopy in 2000) | 0.00025 degree, about 28 m |
| Hansen GFC v1.13 | `forest_change.lossyear` (calendar year, `0` = no loss) | 0.00025 degree, about 28 m |
| JRC TMF v1.2025 | `jrc_tmf.deforestation_year` (`0` = none) | 0.00025 degree, about 28 m |

Each source is read at full resolution: the first image of the TIFF, never an overview.
The fact gives each window's pixel size in metres as `native_m`.
The four reads of a plot run at the same time.
A plot across a 10° tile line reads each tile it touches.
TMF is read from its range-served copy on source.coop first.
If that read fails inside the tropical belt, the endpoint reads the JRC tile from the JRC download service, within the materializer timeout.

A request is read by consignment, not plot by plot.
Plots that fit in one chunk (2,048 JRC pixels a side, about 19 km) are grouped while the group's bounding box also fits in one chunk.
Each group reads each source once, and each plot counts its pixels in its own part of those windows.
Thus 50 farms of one cooperative cost 4 reads, not 200.
Each plot still gets its own windows, cut from the group's, and its own fact.
A larger plot is read and counted chunk by chunk, so the memory one plot holds stays bounded.
Each chunk counts the pixels whose centre it owns (`owns` in the fact), so no pixel counts twice.
A window wider or taller than 1,024 pixels is read in blocks of 1,024 by 1,024.
The responder keeps recent windows in memory, so a repeat of a plot reads nothing upstream.

The grid is the JRC GFC2020 grid.
Each pixel counts with the fraction of it that lies inside the polygon (the outer ring less the holes).
The fraction is computed exactly in degree space, so an edge pixel counts with the part the farm covers.
A pixel covered by 1e-9 or less is left out.
A pixel's area is the fraction times its geodesic area on the WGS84 ellipsoid, from the latitude band of its row.
The fact states this as `area_method: "coverage_fraction_geodesic"`.
Thus `failing_area_ha` is a sum of covered areas, not a projection, and `sampled_polygon_fraction` is 1.
A 0.5 ha decision does not turn on where a pixel centre falls.

Each 10 m pixel takes the Hansen and TMF values of the 30 m pixel that contains its centre.
No value is resampled or interpolated.
Each pixel then gets a verdict from the per-cell rule, as a sampled cell does.
On the raster method, loss after the cut-off is a Hansen loss year or a JRC TMF deforestation year after the cut-off year.
TMF votes. RADD is not read.
`loss_seen_by` splits the failing and the review area by the source that saw the loss: Hansen only, TMF only, or both.
Hansen and TMF map loss on a 30 m grid.
A loss area is the 10 m pixels, by covered fraction, that fall in a 30 m loss pixel.
So `failing_area_ha` and `review_area_ha` carry the 30 m boundary of the loss maps: `loss_boundary_m` is 30.

The raster method signs one derivative fact for each plot:

| Field | Value |
|---|---|
| `fn_key` | `eudr_plot_raster@1` |
| `op` | `eudr_plot_raster` |
| `band` | `eudr.plot_raster` |
| `cell` | the cell at the centre of the plot's bounding box |
| `parents` | none: the fact names its source windows instead |

Its value holds these parts:

- `forest_rule`, `cutoff_year`, `area_method`, `plot_commitment` and `geometry_commitment`
- `chunks`: for each chunk, `owns` and, for each band, the `artifact_cid` of the window, its grid, `native_m`, the files read and the dataset scheme
- `pixels` and `area_ha`: the plot, and each verdict class (`pass`, `fail`, `review`, `not_in_scope`, `indeterminate`), with the forest at the cut-off
- `loss_seen_by` and `failing_by_baseline`: the failing and review areas by source, and the failing area by the baseline that called it forest
- `hansen_loss` and `tmf_loss`: pixels and area by loss year, and after the cut-off
- `tmf_cross_check`: the pixels where TMF and Hansen agree or disagree on loss after the cut-off
- `verdict`: the label, the code, the failing and review areas, and whether the 0.5 ha floor applied

A pixel count is the number of pixels the polygon touches; an area adds their covered parts.

A repeat that gives the same value cites the fact signed before, and signs nothing new (`plot_fact_reused`).
The value includes `plot_commitment`, so send the same `plot_salt` to get a repeat that reuses its fact.
Without it, each request draws a new salt and signs a new fact.

Each window is stored in the artifact store and served at `GET /v1/artifacts/<artifact_cid>`.
Its id is the BLAKE3 hash of its bytes, in lowercase base32 without padding.
The bytes are the emem grid encoding: a 64-byte header, then one little-endian `f32` for each pixel, row by row from the north.
A pixel that no tile holds is NaN.
The artifact store can evict a window.
The fact keeps the files and the grid, so the same window can be read again from the publishers.

The plot fact holds the polygon's commitment, not the polygon.
Its windows show the plot's bounding box, as the sampled facts showed each sampled cell.

On the raster method, the plot result keeps the field names of the sampled method, and a cell is a 10 m pixel.
`failing_cells`, `total_cells` and the counts in `loss_year_histogram` and `tmf_cross_check` are pixels.
The method adds these fields:

| Field | Meaning |
|---|---|
| `area_method` | `coverage_fraction_geodesic` |
| `failing_area_ha_by_baseline` | Failing area split as `failing_cells_by_baseline` splits the count |
| `review_cells`, `review_area_ha`, `review_reason` | Loss after the cut-off on land the JRC map calls non-forest (see below) |
| `hansen_tree_cover_on_jrc_non_forest` | Pixels and area the JRC map calls non-forest where Hansen sees tree cover and no loss |
| `forest_at_cutoff_area_ha` | Area that the rule called forest at the cut-off |
| `loss_after_cutoff_area_ha` | Area with loss after the cut-off, by Hansen and by TMF |
| `loss_seen_by`, `loss_boundary_m`, `loss_boundary_note` | The loss areas by source, and the 30 m boundary they carry |
| `example_pixels` | Up to 32 failing or review pixels, with their cell64 and readings |
| `raster` | Each chunk's `owns`, and the id, URL, size, `native_m` and files of each source window |
| `plot_fact_cid`, `plot_fact_token`, `plot_fact_reused` | The plot fact, its `emem:fact:` token, and whether an earlier identical one was cited |

`per_cell_verdicts` is empty, `per_cell_detail` is `raster` and `per_cell_rows_cid` is null, because no per-cell fact exists.
`verdict_support` grades pixels as it grades cells.
The sample spacing is the pixel side, about 10 m, so the spacing limit does not apply.

To check a plot fact without trusting emem, run the verifier with the plot's GeoJSON:

```sh
python3 tools/verifiers/emem-eudr-due-diligence/verify_plot.py \
  "emem:fact:<cell>:<plot_fact_cid>" --geometry plot.geojson
```

The script checks that the fact's CBOR hashes to its id, and that the polygon hashes to `geometry_commitment`.
It checks that each window hashes to its `artifact_cid`.
It then counts the pixels again, chunk by chunk with the same rule and the same coverage fractions, and compares every count, area and the verdict.
With `--plot-id` and `--salt`, it also checks `plot_commitment`.
It uses the `blake3` package when it is installed, and plain Python otherwise.

### The sampled method

The sampled method reads each plot at one or more cell64s.
These plots use 1 cell, unless the commodity is cattle:

- a GeoJSON Point (the endpoint gives it an area of 0.01 ha)
- a `bbox` of 4 ha or less, when the request sets `method` to `sampled`

Every other plot in the request uses one shared count, `max_cells_per_plot`.
A polygon always uses this count, also when its area is 4 ha or less.
If the operator does not set it, the sampler uses about 110 cells per hectare of the
largest plot in the request.
The sampler takes the area of each plot's bounding box, not the area inside the polygon.
Thus a long or irregular polygon gets more cells than its own area needs.
The derived count is at least 55 and at most 51,200.
If no plot geometry gives an area, the count is 64.
The endpoint clamps a count that the operator sets to 1 through 51,200.
The raster method ignores `max_cells_per_plot`.
The sampled method runs **`eudr_compliance@1`** on each cell.
It reads three bands from two static COG baselines, JRC GFC2020 and Hansen GFC:

| Band | Role |
|---|---|
| `jrc_gfc2020.forest_2020`        | EUDR forest baseline at the 2020-12-31 cut-off (Bourgoin et al. 2026 ESSD); single-band uint8 `1`=forest |
| `forest_change.treecover2000`    | Hansen GFC v1.13 canopy fraction (%) at year 2000 |
| `forest_change.lossyear`         | Hansen GFC v1.13 first-loss year as a **calendar year** (`0`=no loss, else `2001..=2025`); loss strictly after the cut-off year is the failure signal |

The algorithm card at `GET /v1/algorithms/eudr_compliance@1` still names
Hansen v1.12 (loss years `2001..2024`) on 7 November 2026. The `eudr_dds@1` card names no Hansen version.
That text comes from `crates/emem-core/data/algorithms-v0.json`, which did not change when the connector moved to v1.13.
The cards also still give the `jrc_or_hansen` forest rule, which stopped being the default on 14 November 2026.
The connector in `crates/emem-fetch/src/hansen_gfc.rs` reads `GFC-2025-v1.13`.
The `eudr_dds@1` card also writes the cut-off test as `>= 21`, the raw upstream byte.
The responder decodes that byte to a calendar year, so the test it applies is `>= 2021`.
The `eudr_dds@1` card lists `radd.alert_date` and `wri_gdm.driver_class` as inputs.
This endpoint does not read them (see below).

The endpoint reads each baseline band with one `cog::sample_window` call over the polygon
bounding box.
It then finds each cell in that in-memory buffer.
Thus each band costs one upstream read, whatever the cell count.
Only the in-memory lookups grow with the cell count.
A comment in `eudr_batch_path_enabled` records the gain for a 4 ha plot.
It needs 4 HTTP reads instead of about 1,760, and took about 3 s instead of 180 s.
This batched read is on by default. An operator can set `EMEM_EUDR_BATCH_PATH=0` to read each cell on its own.

The batched path also reads the JRC TMF v1.2025 deforestation year on every cell.
It reads a COG copy of the JRC tiles with HTTP Range requests.
The per-cell path does not read TMF. It reads only the three bands in the table above.
The plot result reports the TMF reading as `tmf_cross_check`, which shows where TMF and Hansen agree on
loss after the cut-off.
`agreement` counts every cell TMF read, including the cells where neither product sees loss.
`loss_agreement` counts only the `loss_cells`, the cells where TMF or Hansen sees loss after the cut-off.
On a 2.9 ha cocoa plot where Hansen alone saw loss on 57 of 324 cells, `agreement` was 0.82 and
`loss_agreement` was 0. On a plot that fails or is `review`, `verdict_support` uses `loss_agreement`; on any other
verdict it uses `agreement`, as before.
On the sampled method TMF does not change the verdict; on the raster method it votes (`in_verdict: true`).
A cell can show loss after the cut-off in TMF but not in Hansen.
One such cell sets `tmf_cross_check.review_required` to `true` and lists the cell in `review_cells`.
The responder then holds the statement until a person reviews the plot.
On the raster method, `review_cells` names up to 64 such pixels, and `tmf_only_area_ha` gives their area.
This endpoint does not use WRI-Sims driver attribution or RADD SAR alerts.

### The forest rule

The request field `forest_rule` says which map decides that a cell was forest at the cut-off.
The answer repeats it in `forest_rule`, and `forest_rule_means` gives the rule in words.
The default is `jrc_only`, from 14 November 2026.
`jrc_or_hansen`, the earlier rule, stays for one release.

The reasons for `jrc_only` come from the regulation and the Commission's own texts:

- Article 2(4) to 2(6) and Recital 37 of Regulation (EU) 2023/1115 exclude agricultural plantations from "forest".
  Cocoa, coffee, oil palm, rubber and agroforestry are plantations in this sense.
- Article 2(3) defines deforestation as the conversion of forest to agricultural use.
- The regulation names no map (Article 18(2)(d); Commission FAQ v5, April 2026, section 5.18).
- Hansen tree cover counts all vegetation over 5 m, tree crops included. It is not a forest map in the sense of Article 2(4).
- The JRC GFC2020 map follows the EUDR forest definition. The FAQ (sections 9.10 and 9.10.1) calls it non-mandatory, non-exclusive and non-legally binding, and not a definitive source of information for compliance.
- The same FAQ section says that a plot outside mapped forest cannot be assumed compliant, and that mapped forest may indicate a risk to investigate.
  Article 10(2)(m) asks the operator to assess any information that points to a risk.
- Guidance C/2026/3896, section 5: the actual properties of a forest prevail over a map.
- The JRC GFC2020 V3 map missed 17.3 % of forest in Africa. The accuracy of V4 is not yet published.

Under `jrc_only`, each cell gets its verdict by this table:

| JRC GFC2020 | Hansen | Loss after the cut-off | Cell verdict | Refinement |
|---|---|---|---|---|
| forest | any | no | `pass` | |
| forest | any | yes | `fail` | |
| non-forest | any | yes | `review` | `loss_on_jrc_non_forest` |
| non-forest | tree cover 10 % or more, no loss up to the cut-off year | no | `pass` | `hansen_tree_cover_jrc_non_forest` |
| non-forest | other | no | `not_in_scope` | |
| no reading | tree cover 10 % or more, no loss up to the cut-off year | no | `pass` | `jrc_unavailable_hansen_only` |
| no reading | tree cover 10 % or more, no loss up to the cut-off year | yes | `fail` | `jrc_unavailable_hansen_only` |
| no reading | other Hansen reading | | `not_in_scope` | |
| no reading | no reading | | `indeterminate` | `no_baseline_input` |

Hansen tree cover never makes a cell forest where the JRC map has a reading.
Where JRC has no reading, Hansen decides, and the cell is marked `jrc_unavailable_hansen_only` as lower confidence.
Loss after the cut-off is a Hansen loss year after the cut-off year, or, on the raster method, a JRC TMF deforestation year after it.
The sampled method passes no TMF value to the rule, and neither method reads RADD.

`review` is not a fail and not a pass.
The map is not binding, so loss on land it calls non-forest may be loss of forest that it missed.
The plot result gives the area in `review_area_ha` and the reason in `review_reason`:
"loss after the cut-off on land the JRC map calls non-forest: confirm the land was already in agricultural use before 2021 (imagery before 2021, farm or cadastral records)".
A pass on JRC non-forest where Hansen sees tree cover is a pass.
The plot result reports that area in `hansen_tree_cover_on_jrc_non_forest`, because the two maps disagree there.

Under `jrc_or_hansen`, a cell is forest at the cut-off when JRC GFC2020 calls it forest.
It is also forest when Hansen `treecover2000` is 10 % or more and the Hansen loss year is `0` or after the cut-off year.
A missing Hansen loss year does not make a cell forest.
Forest with loss after the cut-off is `fail`, and other land is `not_in_scope`.
This rule has no `review`.
`failing_cells_by_baseline` says which of the two maps made each failing cell forest.
Tree crops such as cocoa, oil palm and rubber read as tree cover in Hansen, and Article 2(4) excludes them from forest.
So when a plot fails and every failing cell is forest by Hansen alone where JRC calls it non-forest,
`verdict_support` is `weak`, and
its limit says to check the land use before relying on the fail. When half or more are, the limit qualifies the fail.

Under both rules, take a cell that JRC calls forest, with a Hansen loss at or before the cut-off year.
That cell stays forest, and its verdict is `pass` when no later loss shows.
The Hansen loss year does not overrule JRC.
A cell with no JRC reading and a Hansen loss at or before the cut-off year was not forest at the cut-off, so its verdict is `not_in_scope`.

### From cells to a plot verdict

The plot verdict uses the first rule that applies:

1. Every cell is `not_in_scope`: the plot is `not_in_scope`.
2. More than half of the cells are `indeterminate`: the plot is `indeterminate`.
3. A cell is `fail`: the plot is `fail`, and the 0.5 ha minimum area of Article 2(4) applies.
4. A cell is `review`: the plot is `review`, and the 0.5 ha minimum area applies the same way.
5. Otherwise the plot is `pass`.

A plot with no cell is `indeterminate`.
The 0.5 ha rule works on areas:

- A failing area of 0.5 ha or more gives `fail`.
- Below that, a review area that reaches 0.5 ha together with the failing area gives `review`.
  The two areas count together because they can be one clearing that the map split.
- Any smaller loss gives `below_mmu`.

On the raster method the areas are sums of pixel areas.
On the sampled method, `failing_area_ha` is the failing fraction of the sampled cells times the area of the plot's bounding box.
`review_area_ha` is projected the same way.
For a polygon, this area can be more than the area of the plot.
For example, 10 failing cells out of 100 give 0.1 ha when the bounding box is 1 ha (`below_mmu`).
The same count gives 1 ha when the bounding box is 10 ha (`fail`).

A `not_in_scope` plot has `deforestation_free_non_forest: true` when no cell carries a `refinement_applied` value.
A natural-cause loss, `sims_natural_cause`, is one such value.
This endpoint does not read WRI-Sims, so no `not_in_scope` cell carries a refinement value.
Thus, in the code on 7 November 2026, every `not_in_scope` plot has `deforestation_free_non_forest: true`.

The DDS verdict combines the plot verdicts by the first rule that applies:

1. A plot is `fail`: the DDS is `fail`.
2. A plot is `review`: the DDS is `review`.
3. Every plot is `pass`, `below_mmu`, or `not_in_scope` with `deforestation_free_non_forest: true`: the DDS is `pass`.
4. Every plot is `not_in_scope`: the DDS is `not_in_scope`.
5. Otherwise the DDS is `indeterminate`.

Thus the DDS counts a `below_mmu` plot as compliant.
The code is `eudr_verdict_for`, `aggregate_plot_verdict_counts` and `mmu_by_area` in
`crates/emem-api-rest/src/lib.rs`.

A `pass` means that the maps read show no indication of deforestation after the cut-off.
It is not a finding of compliance: the operator's due diligence under Articles 9 to 11 decides that.
The answer says so in `verdict_meaning` and in each plot's `verdict_support.pass_means`.
`verdict_support.baseline_map` names the JRC GFC2020 version read and states that the map is not binding.

### When the responder holds the statement for review

A `pass` does not always give a statement that you can sign.
Each plot carries a `verdict_support` block. It grades the inputs by a fixed rule, not by a probability.

| Input | `strong` needs | `weak` when |
|---|---|---|
| Cells with both the JRC baseline and the Hansen loss year | 95 % or more | below 80 % |
| Cells where TMF and Hansen agree on loss after the cut-off (both see it, or neither does), of the cells TMF reads | 90 % or more | below 70 % |
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
For a polygon, this plot area is the area inside the polygon, not the area of its bounding box.
A `bbox` plot of 4 ha or less gets one sample, so its spacing is the square root of the box area.
A cattle plot (HS code that starts with 0102, 0201 or 0202) is the exception: it gets the polygon sample count.
Above 2 ha, that spacing is more than 141 m.
The level is then `weak`, and the responder holds the statement.
This applies to the sampled method only: the raster method reads every pixel of a `bbox` plot.
A polygon plot gets about 110 cells per hectare of its bounding box.
Its spacing is then about 10 m or less.
Above about 465 ha of box, the 51,200 cap applies and the spacing grows.

The responder holds the statement back when one of these is true:

| `statement_of_compliance_held_because` value | Cause |
|---|---|
| `tmf_cross_check_review_required` | On a plot, TMF sees a loss after the cut-off that Hansen does not |
| `verdict_support_weak` | A plot has `verdict_support.level` `weak` |
| `jrc_baseline_unavailable_hansen_only` | No cell on any plot of the request had a JRC value, and at least one cell had a Hansen value |

The list names every cause that applies.
The first two causes come from one plot. The third cause comes from all the cells of the request.
When TMF reads no cell on a plot, the TMF agreement row and the TMF cause do not apply to that plot.
If the DDS verdict is not `pass`, the list starts with `verdict_<label>`, for example `verdict_fail`.
A `review` verdict adds `review_loss_on_jrc_non_forest` after `verdict_review`.
The operator must assess and mitigate that risk under Articles 10 and 11 before filing.
`statement_of_compliance_signable` is true only for a `pass` with an empty list of causes.
When the responder holds it, `statement_of_compliance` gives the reason in place of the Annex II point 5 text.
The code is `verdict_support`, `tmf_cross_check` and `statement_of_compliance` in
`crates/emem-api-rest/src/lib.rs`.

## Filing in the EUDR Information System

The EU's EUDR Information System (TRACES) takes a statement through its V4 service,
`EUDRDueDiligenceStatementServiceV4`, over SOAP with WS-Security. It does not take JSON.
The response field `traces_nt_v4` names the V4 elements, with their values, for the
operator's software to serialise in the schema's order. We checked it against the
production V4 schema on 14 November 2026. The older `traces_nt_envelope` keeps the V1
shape for callers that read it, and the system does not accept that shape.

`traces_nt_v4` has these parts:

- `statement` holds the elements: `internalReferenceNumber`, `activityType`, `operatorRole`,
  `geoLocationConfidential` and `commodities`. Each commodity has `descriptors`
  (`descriptionOfGoods`, `goodsMeasure.netWeight` in kg), `hsHeading` (the caller's HS
  code, not cut to 4 digits) and `producers`. A producer has `country` in ISO 3166
  alpha-2 (`CI`, not `CIV`) and `geometryGeojson`, the production place as base64
  GeoJSON with `ProducerCountry` in alpha-2.
- An element with no value is left out, because an empty element fails the schema.
- `fileable` is `true` only when the verdict can be signed and nothing is missing.
  `not_fileable_because` lists every reason, such as `verdict_fail`,
  `verdict_review` with `review_loss_on_jrc_non_forest`, `missing_operatorRole`, `missing_activityType`, `missing_descriptionOfGoods_<hs>` or
  `geolocation_below_6_decimals`.
- `operator_must_supply` says what only the operator can give: the role, the activity,
  and the commercial description with the trade name. `drafts` carries a draft
  description from the HS heading text for the operator to confirm. A draft never goes
  into the element.

Send `operator_role`, `activity_type` and each plot's `description_of_goods` in the
request to fill them. The operator's name, address and EORI come from the account that
submits, not from the payload. The system assigns the reference and verification numbers.
Submitting the statement is the Annex II item 5 confirmation, so a statement whose
verdict is not `pass` must not be filed.
`statement_of_compliance` keeps the Annex II point 5 text word for word when it can be signed.
What a `pass` means is in `verdict_meaning`, beside it.

A new micro or small operator in a low-risk country may file a simplified declaration under Article 4a(5).
That declaration gives a postal address in place of geolocation, so it needs no polygon.
This endpoint does not make that declaration.

## Plot commitment

No signed fact holds the caller's `plot_id`.
A signed fact is public, also when the request sets `geolocation_confidential`.
In its place, the plot fact (raster) and the loss-year histogram (sampled) hold `plot_commitment`:

```text
plot_commitment = hex(blake3("emem.eudr.plot_commitment.v1" || 0x00 || salt || plot_id))
```

The salt is 16 bytes.
Send it as `plot_salt` (32 hex characters) on a plot to get the same commitment on every request.
Without it, each request draws a new random salt.
The answer returns the salt in that plot's `plot_salt`, beside `plot_commitment`.
The responder does not store or log the salt.
To prove that a plot fact belongs to a plot, give the verifier the `plot_id` and the salt.
It computes the commitment again and compares it with the one in the fact.
Do not reuse one salt for many plots: a person who learns it can then test guessed plot ids against each of them.

The plot fact also holds `geometry_commitment`, the BLAKE3 hash of the polygon as sent:
the tag `emem.eudr.geometry.v1` and a zero byte, then for each ring its vertex count (`u32`, little-endian),
then each vertex as two little-endian `f64` values, longitude then latitude.
A `bbox` plot is the ring `(min_lng, min_lat)`, `(max_lng, min_lat)`, `(max_lng, max_lat)`, `(min_lng, max_lat)`, `(min_lng, min_lat)`.
This commitment has no salt, so anyone who holds the polygon can check it.

## Loss-year histogram

Each plot that has at least one sampled cell carries a `loss_year_histogram`.
It shows how the Hansen loss year spreads across the sampled cells of the plot.
On the raster method, the counts are the 10 m pixels of the plot and `basis` is `raster_pixels`.
Each year then also carries `area_ha`, the block adds `after_cutoff_area_ha` and the same tally for TMF under `tmf`,
and `signed_fact_cid` is the plot fact, which holds the tally.
The rest of this section describes the sampled method.
It has these fields:

| Field | Meaning |
|---|---|
| `by_year` | List of `{"year", "cells"}` objects, one for each loss year above 0, in ascending order |
| `after_cutoff_cells` | Cells with a loss year after `after_cutoff_year` |
| `after_cutoff_year` | The year of `cut_off_date` in the request (2020 for the regulation date) |
| `no_loss_cells` | Cells with loss year 0 (no loss) |
| `unknown_cells` | Cells with no loss-year value, or a negative value |
| `total_sampled_cells` | All sampled cells |
| `cell_area_ha` | Area of one cell in hectares, rounded to 6 decimals (0.009129 on 7 November 2026) |
| `after_cutoff_area_ha_sampled` | `after_cutoff_cells` times the cell area, rounded to 4 decimals (sampled area, not polygon area) |
| `basis` | `sampled_cells` (`raster_pixels` on the raster method) |
| `note` | Fixed text that repeats the rules in this section |
| `signed_fact_cid` | CID of the signed histogram fact, or null if signing failed |
| `parents_capped` | True if the plot had more than 1024 distinct loss-year facts |

The endpoint signs the histogram as its own derived fact, `forest_change.lossyear_histogram`.
That fact has `op: "histogram"` and `fn_key: "lossyear_histogram@1"`.
Its `cell` is the plot centroid cell.
Its `tslot_window` always spans the loss years 2001 to 2024 (tslots 31 and 54).
Its `derivation.args` hold `plot_commitment`, `cutoff_year` and `n_sampled_cells`.
Until 14 November 2026 they held the caller's `plot_id` (see [Plot commitment](#plot-commitment)).
Its `parents` cite the `forest_change.lossyear` facts of the cells, at most 1024, in CID order.
The endpoint stores the fact and its parents, so each CID resolves at `/v1/facts/<cid>`.
The response receipt includes the CID of the histogram fact, unless the receipt cap removes it
(see [Receipts](#receipts)).
A signing error does not change the verdict. It only leaves `signed_fact_cid` null.

The signed value has the year pairs, the four cell counts and `cutoff_year`.
The same cells give the same value, so anyone can count the parents again and compare.
Only cells with a loss-year fact become parents. A cell without that fact counts in `unknown_cells`.
Thus, when `unknown_cells` is 0 and `parents_capped` is false, the parents cover every sampled cell.
The fact also has `signed_at`, the time of signing.
Thus each call signs a new fact with a new CID, even when the value does not change.

The counts cover only the sampled cells.
To estimate the whole polygon, weight the counts by the plot's `sampled_polygon_fraction`.
`total_sampled_cells` is always the full count, also when `parents_capped` is true.
The code is `compute_lossyear_tally` and `EUDR_LOSSYEAR_HISTOGRAM_PARENTS_CAP` in
`crates/emem-api-rest/src/lib.rs`.

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
  The block's `method` string is `annual_s2_l2a_least_cloudy + annual_s1_rtc_vv_cloud_independent`.
  The words "least cloudy" in that string are not correct. The rule in this paragraph is the real rule.
  The block's `anchor_policy` text always names the ±30/60/90 day ladder, also when an operator changes the base window.
  If scenes exist but no pixel is usable, the signed fact records an absence.
  If no step finds a scene, that cell and year get an error and no fact.
  The window never goes past the current date.
  Each step searches Microsoft Planetary Computer first.
  It asks Element84 Earth Search only when that search fails, not when it finds no scene.
  `EMEM_S2_CATALOGUES` changes the order or names one catalogue.
  If every catalogue fails in a step, the picker stops with an error and skips the later steps.
  The picker keeps a choice for 10 minutes for each pixel and anchor day. It does not keep an error.
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
  Radar searches only the Microsoft Planetary Computer `sentinel-1-rtc` collection.
  It has no second catalogue, and `EMEM_S2_CATALOGUES` does not apply to it.
  If a search fails, the ladder stops with an error and skips the later steps.
  A rate limit from a catalogue (HTTP 429, or a 403 that says "rate limit") pauses that catalogue for this process.
  The pause lasts for the `Retry-After` value (1 to 900 s), or 60 s when the reply has none.
  During the pause, a Sentinel-1 search fails at once with `stac cooling down`.
  A Sentinel-2 search goes to the next catalogue in `EMEM_S2_CATALOGUES`.
  The signed fact has band `sentinel1_raw` and holds VV in dB (`10 × log10` of the linear value).
  Its `tslot` is the day of the scene, not the July-1 anchor.
  Its derivation is `sentinel1_rtc_vv_db@1`. The arguments end with the half-width in days
  of the window that found the scene.
  The source id is the asset URL without the short-lived access token.
  If the pixel value is zero, negative or not a number (water mask or no-data), that cell
  and year get an error and no fact.
  Radar sees through cloud, so this signal confirms the NDVI in monsoon regions where
  Sentinel-2 has gaps.
- Per-year `scene.png` URLs for up to 6 cells of the plot, in `scene_png_urls` and
  `scene_metadata`. These are the first 6 of the cells that the block reads (see [Time budget](#time-budget)).
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
  If a fetch from the catalogue fails, it returns 502 `scene_upstream_failed`.
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
The `thresholds.rationale` text does not change. It always names the defaults.
If an operator changed a threshold, read the numbers, not that text.
`metrics.ndvi_max_drop_vs_baseline` gives the largest NDVI drop.
`metrics.ndvi_worst_year_over_year_drop` gives the largest drop from one year to the next.
That second value is for information only; the verdict does not use it.

The block carries its own `verdict`:

| Visual verdict | When |
|---|---|
| `visual_deforestation_suspected` | One or both signals reach the threshold |
| `no_visual_deforestation` | No signal reaches the threshold, and the plot has at least one 2020 median |
| `indeterminate_no_baseline` | The plot has no 2020 median for NDVI and none for S1 VV; the block does not pass it silently |
| `incomplete` | The visual time budget ran out before the timeline was complete |

The schema enum for this `verdict` lists only the first three values.
On 7 November 2026 it did not list `incomplete`, but the endpoint can return it.
If you validate responses against the schema, allow `incomplete` in your validator.
An `incomplete` block has only `schema`, `verdict`, `complete: false`, `budget_secs` and `reason`.
It has no `years`, `thresholds` or `verdict_basis`.

The block evaluates S1 VV only when the plot has a 2020 median and a latest-year reading.
It evaluates NDVI when the plot has a 2020 median and also one of these:

- a drop from the 2020 median in any year
- a latest-year NDVI median.

`verdict_basis` names the signals that the verdict uses: `ndvi_and_s1_vv`, `ndvi_only`,
`s1_vv_only` or `neither`.
`signals.s1_vv.why_not` gives the reason when the block did not evaluate S1 VV.
Read `verdict_basis` before you quote `no_visual_deforestation`.
With `ndvi_only`, only NDVI cleared the plot, because S1 VV did not run.
With `neither`, no signal ran, but the verdict can still be `no_visual_deforestation`.
This occurs when a 2020 median exists but the latest year has no reading and no year shows a drop.

### Receipts

The response has one Ed25519 `receipt` for the whole call.
On the raster method it signs each plot fact and the cell at the centre of each plot.
The answer then stays small: a plot adds one fact CID and one cell to the receipt.
On the sampled method it signs the per-cell fact CIDs of every plot and the CID of each loss-year histogram fact.
It also signs the cell64 of each sampled cell.
Check it offline at `/verify`, or with `POST /v1/verify_receipt`.

Large calls hit two caps.
The table gives the defaults; both env vars accept 256 to 51,200.

| Cap | Default | Env var | When the cap applies |
|---|---|---|---|
| Fact CIDs in the receipt | 2048 | `EMEM_EUDR_RECEIPT_MAX_FACTS` | The receipt keeps the first 2048 CIDs in sort order. The response adds `receipt_fact_cids_capped: true`, `receipt_fact_cids_total` and `receipt_fact_cids_cap`. |
| Cells in the receipt | 2048 | `EMEM_EUDR_RECEIPT_MAX_CELLS` | The receipt lists the centroid cell of each plot. The response adds `receipt_cells_truncated: true`, `receipt_cells_total` and `receipt_cells_cap`. |

The sort runs only above the cap.
Thus two capped calls with the same inputs keep the same fact CIDs.
The exceptions are the loss-year histogram fact and the plot fact, which the responder signs again on each call.
The two receipts are still not byte-equal, because `request_id` and `served_at` change on each call.
A capped receipt does not sign every fact, but the endpoint stores every fact.
Each plot's `per_cell_rows_cid` addresses all of its facts.
Every fact CID in the response resolves at `/v1/facts/<cid>`.

The receipt does not sign the visual evidence facts.
Each per-cell NDVI and S1 backscatter reading for a year is a separate signed Primary fact
under the responder's identity.
The yearly medians in `years[]` are not facts; the block computes them from these readings.
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
| EUDR requests at one time | 6 | `EMEM_EUDR_MAX_INFLIGHT` | 1 to 64 |
| Wait for a free EUDR slot | 60 s | `EMEM_EUDR_QUEUE_SECS` | 0 to 120 s |

The limit of 6 visual reads applies to the whole responder, not to one plot.
All plots in all running requests share it.
`/v1/eudr_dds`, `/v1/field_burn_scar` and `/v1/field_actual_et` share the EUDR request slots.
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
The 504 body has `code: "source_fetch_failed"` and gives the plot count and the budget in seconds.

A `/v1/eudr_dds` response over HTTP has a `Server-Timing` header with the wall time of each step in ms:
`prepare` (the plots checked), `raster` (the raster method's reads and counts for all its plots),
`plots` (the sampled method's band reads and verdicts, and each plot's answer), `assemble` (the response and
the facts it names), `store` (the facts signed and written), `receipt` and `total`. The time spent waiting for a
slot is not in `total`. The MCP tool and the A2A task give the same answer without the header.

The two field routes do not have the 280 s budget.
Their only limit is the route timeout: `EMEM_EUDR_TIMEOUT_SECS` plus 10 s, which is 610 s when no operator sets the variable.
That is above the edge cut of about 300 s, so the edge can close a slow field call before the responder sends a 504.
A route timeout gives a 504 with `code: "compute_timeout"`.
The deploy unit `deploy/systemd/emem-server.service` does not set `EMEM_EUDR_TIMEOUT_SECS`, so emem.dev uses these defaults.

## Compliance position

EUDR Article 2(4) defines forest by canopy, height and area, **not by data source**.
Bourgoin et al. 2026 name JRC GFC2020 as the baseline that the Commission expects.
This baseline is not binding, and an operator can use a different baseline that it can defend.
By default (`forest_rule: jrc_only`) the verdict takes forest at the cut-off from JRC GFC2020 alone.
Hansen tree cover counts only where JRC has no reading, and it includes tree crops, which Article 2(4) excludes.
The verdict takes the loss year from Hansen.
Loss on land the JRC map calls non-forest is `review`, because the map is not binding and can miss forest.
[The forest rule](#the-forest-rule) gives the sources for this choice.
A `pass` means that the maps show no indication of deforestation, not that the plot complies.
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
The verdict uses this date by default.
For a what-if audit, send a different `cut_off_date`; `cut_off_year_applied` gives the year that ran.
The verdict uses only the year: the first four characters of `cut_off_date`.
If those are not four digits, the request fails with 400 `invalid_argument`.
Loss in a year after that year is a failure.
A cell with Hansen loss in or before that year, and no JRC GFC2020 forest, is `not_in_scope`.
The forest baseline is the JRC GFC2020 map for every cut-off year.
Thus a JRC forest cell counts as forest at a 2018 cut-off too.
Regulations (EU) 2024/3234 and 2025/2650 moved the start of the obligations.
For large operators they apply from 30 December 2026.
For micro and small enterprises they apply from 30 June 2027.
The `regulation_status_note` field in the response states these dates.
The text does not change between calls.
It gives the position on 1 June 2026, so check EUR-Lex for later changes.
The deferral does not change the cut-off that the verdict uses.

## Related endpoints

| Request | What it returns |
|---|---|
| `GET /v1/algorithms/eudr_compliance@1` | Card for the per-cell rule: `inputs`, `formula`, `accuracy_band`, `citation` |
| `GET /v1/algorithms/eudr_dds@1` | Card for the polygon step that wraps `eudr_compliance@1`. Its `runtime_path` is `/v1/eudr_dds`. |
| `GET /v1/schemas/eudr_dds.json` | JSON Schema (draft 2020-12) of the response. On 7 November 2026 it listed 15 top-level properties and required 5. The response body has 35 or more top-level fields. The top of this page names some that the schema omits. |
| `GET /v1/cells/{cell64}/scene.png` | True-colour Sentinel-2 chip, 256 x 256 pixels, PNG |
| `GET /v1/cells/{cell64}/scene.rgb` | The same chip as raw bytes, without PNG framing |
| `GET /v1/scene.png?bbox=w,s,e,n` | True-colour Sentinel-2 chip cropped to a plot box in WGS 84, PNG |

All six requests returned 200 on 7 November 2026.

### Read a scene chip

The three scene requests use one pipeline.
Each sends `x-emem-scene-` headers for the item id, datetime, cloud cover, EPSG, bounds, pixel size, sun and view.
The sun and view headers are empty when the catalogue gives no angles.
The two PNG requests also send `x-emem-scene-stretch-r`, `-g` and `-b`.
Each gives the 2nd and 98th percentile of that channel, so you can make the same picture again.
The stretch is per scene, so brightness at two dates is not comparable.
Only `.rgb` sends `x-emem-scene-format`, `-width`, `-height` and `-channels`, because a PNG holds its own size.
If you send no `max_cloud`, `datetime` or `at`, the handler tries up to three searches:

| Step | Cloud cap | Window before now | `x-emem-scene-fallback` header |
|---|---|---|---|
| 1 | 20 % | 90 days | not sent |
| 2 | 20 % | 365 days | `widened_window` |
| 3 | 60 % | 365 days | `relaxed_cloud` |

If you send `max_cloud` or a window, the handler tries one search only.
A `max_cloud` that is not a number does not count. The handler ignores it and runs the three steps.
That search uses a 20 % cap if you send no `max_cloud`.
It uses the last 90 days if you send no window.
The `datetime=<start>/<end>` parameter sets the window directly.
The `at=<date>` parameter sets a window of 7 days that ends at that date.
The operator can change the 7 days with `EMEM_SCENE_AT_LOOKBACK_DAYS`.
If you send both, `datetime` wins. An `at` that is not an ISO 8601 date gets 400.
If no search finds a scene, the handler returns 404 `no_clear_scene` with a `tried` list.
Any other failure stops the ladder and returns 502 `scene_upstream_failed`.
That includes a failed fetch, and a scene with no valid pixels in the window, for example under cloud or over water.
The two PNG requests share a quota of 60,000 per client IP per day (`EMEM_SCENE_DAILY_QUOTA`).
Past it they return 429 with `retry-after: 86400`.

The bbox request reads at 10 m and crops the scene to your box.
The box must have `w` < `e` and `s` < `n`, with latitudes from -85 to 85. Otherwise the request gets 400.
The crop centres on the cell that holds the centre of the box, so it can move by about 5 m.
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
