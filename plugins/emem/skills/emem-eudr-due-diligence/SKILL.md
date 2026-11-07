---
name: emem-eudr-due-diligence
description: Produces a signed, Annex II shaped Due Diligence Statement under the EU Deforestation Regulation (EU 2023/1115) for one or more production plots with emem, and says what it proves and what it leaves to the operator. It checks each plot against the JRC GFC2020 forest baseline and the Hansen Global Forest Change loss year at the 2020-12-31 cut-off. It cross-checks JRC TMF and grades how well the inputs support the verdict. Use when someone asks if a cocoa, coffee, soy, palm, rubber, cattle or wood plot is deforestation-free. Also use it for per-cell evidence for an Article 12 record.
---

# emem-eudr-due-diligence

| | |
|---|---|
| Use it when | A supplier, importer or auditor asks if a plot of an EUDR commodity is deforestation-free, or needs per-cell evidence for an Article 12 record. |
| You need | For each plot: a GeoJSON geometry, the country of production (ISO 3166-1 alpha-3), the HS code and the quantity in kg. No key. |
| You get | A verdict for each plot and for the statement, a TRACES NT shaped envelope, the signed fact cids behind each cell, and an Ed25519 receipt. |
| Calls | `POST /v1/eudr_dds` (MCP `emem_eudr_dds`). To warm a cold plot first: `POST /v1/locate`, `POST /v1/recall`. |

> **Network use.** The commands in this skill send JSON to `https://emem.dev` and receive JSON from it. `https://emem.dev` is the service that the MCP server of the plugin connects to. The commands do not execute what they download. This skill ships no scripts. Its commands read no environment variable, key or credential. The only local files that they read are files that an earlier step saved from `https://emem.dev`.

The endpoint tests each plot against the forest definition of the
Regulation: 10 % canopy, 0.5 ha and 5 m. It returns a signed envelope
that an operator can keep as evidence. It covers the geolocation and
deforestation parts of Annex II.

It does not cover legality (Article 9(1)(b)) or forest degradation
(Article 2(7)). The response says so in `legality_disclaimer` and
`degradation_disclaimer`. Give both to the user.

## Step 1: warm a cold plot

A cold plot can take minutes. It can also fail with
`source_fetch_failed`, which names the slow tile. To prevent this, read
the forest bands on one cell of the plot first.

Get the cell64 of the plot point:

```sh
curl -s -X POST https://emem.dev/v1/locate -H 'content-type: application/json' \
  -d '{"lat":6.8276,"lng":-5.5471}' | jq -r .cell64
```

This returned `defi.zb44d.zaedf.bujI` on 7 November 2026. Read the
three forest bands at that cell:

```sh
curl -s -X POST https://emem.dev/v1/recall -H 'content-type: application/json' \
  -d '{"cell":"defi.zb44d.zaedf.bujI","bands":["jrc_gfc2020.forest_2020","forest_change.lossyear","forest_change.treecover2000"]}' \
  | jq -c '.facts[] | {band, value, unit}'
```

On 7 November 2026 it returned `jrc_gfc2020.forest_2020` 0,
`forest_change.lossyear` 0 and `forest_change.treecover2000` 20. The MCP
tools `emem_locate` and `emem_recall` take the same bodies.

For a Point, the statement samples one cell in a box that reaches
0.0001 degrees on each side of the point. That cell can be a neighbour of the cell from
`locate`. On 7 November 2026 the statement read `defi.zb44d.zae9f.bujI`.
The warm-up still helps, because it loads the source tiles, and
neighbour cells share them.

## Step 2: send the request

```sh
curl -s -m 300 -X POST https://emem.dev/v1/eudr_dds -H 'content-type: application/json' \
  -d '{"operator":{"name":"Example Cocoa Importer"},"plots":[{"plot_id":"CIV-001","geometry_geojson":{"type":"Point","coordinates":[-5.5471,6.8276]},"country_of_production":"CIV","commodity_hs":"180100","quantity_kg":1200}]}' \
  -o dds.json
```

Allow up to 300 s for a cold plot. After step 1, this call answered in
less than 1 s on 7 November 2026.

Over MCP, call `emem_eudr_dds` with the same body. The plugin's MCP
server does not list this tool, but `tools/call` runs it by name. If
your host calls only listed tools, use the REST route. A long call over
MCP can use a `task` param and poll `tasks/get`. The MCP server refuses
a statement larger than its wire budget (about 24 KB). It does not cut
it, because a partial statement looks complete. In that case, use the
REST route.

### Fields for each plot

| Field | Required | Value |
|---|---|---|
| `plot_id` | yes | Your own id for the plot. |
| `geometry_geojson` | yes | A Polygon for a plot of more than 4 ha, or for cattle. A Point for a plot of 4 ha or less (Article 2(28)). `{"bbox":[w,s,e,n]}` is also accepted. |
| `country_of_production` | yes | ISO 3166-1 alpha-3, for example `CIV`. |
| `commodity_hs` | yes | HS-6 or longer. The endpoint checks it against Annex I. |
| `quantity_kg` | yes | The quantity in kg. |
| `request_visual_evidence` | no | `true` adds a Sentinel-2 NDVI and Sentinel-1 backscatter timeline for each year, with scene links. It adds upstream reads that can take minutes. |

### Fields at the top level

| Field | Default | Value |
|---|---|---|
| `operator` | none | `{"name": ...}` for the envelope. |
| `cut_off_date` | `2020-12-31` | The deforestation cut-off. |
| `forest_baseline_override` | JRC with Hansen | `jrc_gfc2020_v4`, `hansen_only` or `both`. `jrc_gfc2020_v3` is still accepted. |
| `max_cells_per_plot` | from the polygon area | The sample size for each plot. |
| `per_cell_detail` | `rows` | `rows` puts the per-cell rows in the response. `anchor` leaves them out and keeps `per_cell_rows_cid`. |
| `per_cell_offset` | none | Pages the per-cell rows. See step 4. |

The request schema is `EudrDdsReq` in the OpenAPI document at
`GET https://emem.dev/openapi.json`. The response schema is at
`GET https://emem.dev/v1/schemas/eudr_dds.json`.

## Step 3: read the response

```sh
jq '{verdict: .due_diligence_statement.verdict,
     signable: .traces_nt_envelope.statementOfComplianceSignable,
     review_required: .traces_nt_envelope.statementOfComplianceReviewRequired,
     baseline: .forest_baseline, datasets: .forest_baseline_dataset,
     plots: [.per_plot_results[] | {plot_id, verdict, failing_area_ha,
             support: .verdict_support.level, limits: .verdict_support.limits,
             tmf: .tmf_cross_check | {agreement, review_required, post_cutoff_loss}}]}' dds.json
```

### Plot verdicts

| Verdict | Meaning |
|---|---|
| `pass` | No forest loss after the cut-off on forest that stood at the cut-off. |
| `fail` | Forest loss after the cut-off, over at least 0.5 ha. |
| `not_in_scope` | The land was cleared at or before the cut-off, so it was not forest at the cut-off. |
| `indeterminate` | The inputs do not decide the verdict. |
| `below_mmu` | The failing area is less than the 0.5 ha floor. It counts as compliant. |

`per_cell_verdicts` gives the signed fact cids behind each cell. Put
them in the Article 12 record.

### Other fields

| Field | What it tells you |
|---|---|
| `forest_baseline_dataset` | The name, version and source of each dataset that was read. Quote the versions from this field. |
| `forest_baseline` | The JRC version that the facts came from, for example `jrc_gfc2020_v4`. The fact derivations carry it too (`jrc_gfc2020_v4_pixel@1`). |
| `tmf_cross_check` | The JRC TMF deforestation year on each cell, compared with Hansen after the cut-off: `both`, `hansen_only`, `tmf_only`, `neither` and `agreement`. It does not change the verdict. A `tmf_only` loss sets `review_required: true`, because TMF often sees a clearance that started as degradation and that Hansen misses. |
| `verdict_support.level` | `strong`, `moderate` or `weak`. It shows how complete the inputs are. It is not a calibrated probability. Each shortfall is in `limits`. |
| `sampling_caveat` | Present for a Point plot. It says that one cell, about 9.5 m, stands for a plot of up to 4 ha. |
| `traces_nt_envelope` | The TRACES NT shaped statement. `ddsReferenceNumber` is an emem placeholder. TRACES NT issues the real number. |

These inputs set `verdict_support.level` (`verdict_support` in
`lib.rs`):

| Input | `strong` | `weak` |
|---|---|---|
| Cells with both the JRC baseline and the Hansen loss year | 95 % or more | less than 80 % |
| TMF agreement with Hansen, where TMF reads | 90 % or more | less than 70 % |
| Cells within 2 points of the 10 % canopy threshold | 10 % or less | more than 30 % |
| Sample spacing, for a plot of 0.5 ha or more | 71 m or less | more than 141 m |

A value between the two columns gives `moderate`. Three more cases give
`moderate`:

- The plot has one sample cell, for example a Point.
- The plot is less than 0.5 ha.
- TMF read no cell, and the plot is within 30 degrees of the equator.

### When the statement is held for review

The statement is held for review when one of these is true:

- A plot has `tmf_cross_check.review_required: true`.
- A plot has `verdict_support.level` `weak`.
- The JRC baseline was not available. `forest_baseline_computed` is then
  `hansen_only_jrc_unavailable`.

`statement_of_compliance_signable` at the top level and
`statementOfComplianceSignable` in the envelope use one rule, so they
always agree. Each is true only when the overall verdict is `pass` and
the statement is not held for review.

`statement_of_compliance_held_because` (in the envelope:
`statementOfComplianceHeldBecause`) lists each cause that applies. It is
empty when the statement is signable.

| Cause | Reason |
|---|---|
| `verdict_<label>` | The overall verdict is not `pass`. |
| `tmf_cross_check_review_required` | TMF saw a loss that Hansen did not see. |
| `verdict_support_weak` | One plot has `weak` support. |
| `jrc_baseline_unavailable_hansen_only` | The verdict used Hansen only. |

### Result on 7 November 2026

The Point plot in step 2 (cocoa, Côte d'Ivoire) returned this at
01:06 UTC. A second call at 03:31 UTC returned the same values:

```json
{"verdict": "pass", "signable": true, "review_required": false,
 "baseline": "jrc_gfc2020_v4",
 "datasets": [{"name": "JRC Global Forest Cover 2020", "version": "V4"},
              {"name": "Hansen Global Forest Change", "version": "v1.13 (2025)"}],
 "plots": [{"plot_id": "CIV-001", "verdict": "pass", "failing_area_ha": 0.0,
            "support": "moderate",
            "limits": [{"severity": "qualifies", "why": "one point sample: a clearance elsewhere on the plot is not seen"},
                       {"severity": "qualifies", "why": "the plot is 0.01 ha, under the 0.5 ha minimum mapping unit (Art. 2(4)), so no clearance of that size fits in it and none can fall between samples"}],
            "tmf": {"agreement": 1.0, "review_required": false,
                    "post_cutoff_loss": {"both": 0, "hansen_only": 0, "neither": 1, "tmf_only": 0}}}]}
```

The one cell read `jrc_forest_2020` 0, `hansen_treecover_2000` 20,
`hansen_lossyear` 0 and `jrc_tmf_deforestation_year` 0, with four fact
cids. The receipt signs five fact cids and verified. The `datasets`
lines above leave out the `source` URLs.

On 28 September 2026 the same plot first failed with
`source_fetch_failed` on a cold Hansen tile. A later call then answered
with support `strong`, while TMF read no cell. The code now lowers the
level to `moderate` for one sample cell and for an unread TMF in the
tropics.

## Step 4: verify and keep the evidence

The response has a signed `receipt` over the per-cell fact cids. On a
large plot the receipt signs only the first cids, sorted by cid. The
response then shows `receipt_fact_cids_capped` and
`receipt_fact_cids_total`.

Verify the receipt on the server:

```sh
jq -c '{receipt}' dds.json | curl -s -X POST https://emem.dev/v1/verify_receipt \
  -H 'content-type: application/json' -d @- | jq -c '{valid, signature_valid, merkle_proof_valid}'
```

On 7 November 2026 all three were `true`. To verify offline, without
trust in the responder, use
[`emem-verify-receipt`](../emem-verify-receipt/SKILL.md).

The responder stores the facts of every sampled cell, also the cells
that the response does not show. `facts_computed` and `facts_persisted`
give the counts, for each plot and in total. Each fact cid in the
response resolves at `GET https://emem.dev/v1/facts/<cid>`, and so does
each cid behind `per_cell_rows_cid`.

### Per-cell rows

- `per_cell_detail: "anchor"` leaves out `per_cell_verdicts` and keeps
  `per_cell_rows_cid`. Ask again with `rows` to get the rows.
- Send `per_cell_offset` to page the rows in canonical order, one inline
  cap at a time. Follow `per_cell_next_offset` until it is `null`.
- `per_cell_rows_cid` is a BLAKE3 hex over one line for each sampled
  cell: `<cell>|<verdict>|<fact_cids joined by ,>` and a newline. Use
  the number in `verdict` (for example `1`), not the text in `label`.
  The joined pages give the same hash. On 7 November 2026 the hash of
  the one row in step 3 matched `per_cell_rows_cid`. The receipt does not sign this hash,
  but it signs the fact cids in it.
- `sampled_polygon_fraction` is the coverage (sampled area over polygon
  area). Do not use the number of rows as coverage.
- Without `per_cell_offset`, the rows over the cap are a sample: failing
  cells first, then the first of the other cells.

Keep the fact cids in the operator's record. To give them as one token,
`POST /v1/memory_bundle {"fact_cids":[...]}` returns an `emem:bundle:`
token. That call stores a record on the responder. For a lab
certificate or a land title in the same file, see
[`emem-document-evidence`](../emem-document-evidence/SKILL.md).

## Failures

| Response | Cause | What to do |
|---|---|---|
| `source_fetch_failed` | An upstream tile was slow or cold. The error names it. | Do step 1, then send the request again. |
| `review_required: true` | TMF saw a loss that Hansen did not see. | Read `tmf_cross_check.review_cells`. A person must look at those cells. |
| `verdict_support.level` `weak` | An input is incomplete. | Read `limits`. Send a Polygon or a larger `max_cells_per_plot`. |
| `annex_i_warnings` not empty | The HS code is not in Annex I. The verdict is still computed, but EUDR does not cover that commodity. | Check `commodity_hs`. |
| Plot `verdict` `indeterminate` with no cells | The geometry did not parse. | Check `geometry_geojson`. |

## What to tell the user

- The verdict, the `verdict_support.level` of each plot, and its
  `limits`.
- If review is required, say why. Use `tmf_cross_check.review_cells`.
- For a Point plot: one cell, about 9.5 m, stands for a plot of up to
  4 ha. The `sampling_caveat` field says so.
- The statement does not assess legality or degradation.
- The obligations apply from 30 December 2026 for large operators. They
  apply from 30 June 2027 for micro and small enterprises
  (`regulation_status_note`). The cut-off date does not change.
