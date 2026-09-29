---
name: emem-eudr-due-diligence
description: Produces a signed, Annex II shaped Due Diligence Statement under the EU Deforestation Regulation (EU 2023/1115) for one or more production plots with emem, and explains what it proves and what it leaves to the operator. Checks each plot against the JRC GFC2020 forest baseline and Hansen Global Forest Change loss year at the 2020-12-31 cut-off, cross-checks JRC TMF, and grades how well the inputs support the verdict. Use when a supplier, importer or auditor asks whether a cocoa, coffee, soy, palm, rubber, cattle or wood plot is deforestation-free, or needs per-cell evidence for an Article 12 record.
---

# emem-eudr-due-diligence

> **Network use.** The commands in this skill send and receive JSON (and, where a step says so, an image or a raster file) to `https://emem.dev` only, the service the plugin's MCP server connects to. Nothing they download is executed. The files under `scripts/` read local files and make no network calls.

`POST /v1/eudr_dds` (MCP `emem_eudr_dds`) evaluates plots against the
Regulation's forest definition (10 % canopy, 0.5 ha, 5 m) and returns a
signed envelope an operator can keep as evidence. It covers the
geolocation and deforestation parts of Annex II. It does not cover
legality (Article 9(1)(b)) or forest degradation (Article 2(7)), and
the response says so in `legality_disclaimer` and
`degradation_disclaimer`. Pass both on.

## The request

```sh
curl -sf -m 300 -X POST https://emem.dev/v1/eudr_dds \
  -H 'content-type: application/json' \
  -d '{"operator": {"name": "Example Cocoa Importer"},
       "plots": [{"plot_id": "CIV-001",
                  "geometry_geojson": {"type": "Point", "coordinates": [-5.5471, 6.8276]},
                  "country_of_production": "CIV",
                  "commodity_hs": "180100",
                  "quantity_kg": 1200}]}' > dds.json
```

Per plot: `geometry_geojson` (a Polygon for plots over 4 ha; a Point is
allowed for 4 ha or less, except cattle, per Article 2(28); or
`{"bbox":[w,s,e,n]}`), `country_of_production` (ISO 3166-1 alpha-3),
`commodity_hs` (HS-6 or longer, checked against Annex I) and
`quantity_kg`. Optional: `request_visual_evidence: true` adds a
year-by-year Sentinel-2 NDVI and Sentinel-1 backscatter timeline with
scene links, and several minutes of upstream reads. Top level:
`cut_off_date` (default `2020-12-31`), `forest_baseline_override`
(`jrc_gfc2020_v3`, `hansen_only`, `both`), `max_cells_per_plot`
(auto-derived from polygon area when omitted). The full schema is
`GET https://emem.dev/v1/schemas/eudr_dds.json`.

A cold plot can take minutes and can fail with `source_fetch_failed`
naming the slow tile. Warm it first with a recall on one cell of the
plot:

```sh
CELL=$(curl -sf -X POST https://emem.dev/v1/locate -H 'content-type: application/json' \
  -d '{"lat":6.8276,"lng":-5.5471}' | jq -r .cell64)
curl -sf -X POST https://emem.dev/v1/recall -H 'content-type: application/json' \
  -d "{\"cell\":\"$CELL\",\"bands\":[\"jrc_gfc2020.forest_2020\",\"forest_change.lossyear\",\"forest_change.treecover2000\"]}" \
  | jq '[.facts[] | {band, value}]'
```

## Reading the response

```sh
jq '{verdict: .due_diligence_statement.verdict,
     signable: .traces_nt_envelope.statementOfComplianceSignable,
     review_required: .traces_nt_envelope.statementOfComplianceReviewRequired,
     baseline: .forest_baseline, datasets: .forest_baseline_dataset,
     plots: [.per_plot_results[] | {plot_id, verdict, failing_area_ha,
             support: .verdict_support.level, limits: .verdict_support.limits,
             tmf: .tmf_cross_check | {agreement, review_required, post_cutoff_loss}}]}' dds.json
```

- **Plot verdicts**: `pass`, `fail`, `not_in_scope` (cleared at or
  before the cut-off, so not forest at the cut-off), `indeterminate`, or
  `below_mmu` (failing area under the 0.5 ha floor, counted as
  compliant). `per_cell_verdicts` carries the signed fact cids behind
  each; quote them in the Article 12 record.
- **`forest_baseline_dataset`**: the dataset versions actually read, by
  name and source. `forest_baseline` is a stable enum name
  (`jrc_gfc2020_v3`), not the version, and some prose notes in the
  response still say "Hansen GFC v1.12"; quote the versions from this
  field.
- **`tmf_cross_check`**: JRC TMF deforestation year read on every cell
  and compared with Hansen after the cut-off (`both`, `hansen_only`,
  `tmf_only`, `neither`, `agreement`). It is not counted in the verdict.
  A `tmf_only` loss sets `review_required: true`, because TMF often sees
  degradation-to-clearance that Hansen misses.
- **`verdict_support.level`**: `strong`, `moderate` or `weak`, from how
  complete the inputs are (share of cells with both baselines, TMF
  agreement, borderline canopy, and sample spacing against the 71 m side
  of a 0.5 ha square), each shortfall listed in `limits`. It is not a
  calibrated probability.
- **`traces_nt_envelope`**: the TRACES NT shaped statement.
  `statementOfComplianceSignable` is true only for an overall `pass`
  with no TMF review required; `statementOfComplianceReviewRequired`
  says when a person must look first. `ddsReferenceNumber` is an emem
  placeholder; TRACES NT issues the real one.

The top-level `statement_of_compliance_signable` does not take the TMF
review into account; read the envelope's `statementOfComplianceSignable`
instead.

## Recorded on 2026-09-28

The Point plot above (cocoa, Côte d'Ivoire) failed once with
`source_fetch_failed` on a cold Hansen tile, then answered in a later
call:

```json
{"verdict": "pass", "signable": true, "review_required": false,
 "baseline": "jrc_gfc2020_v3",
 "datasets": [{"name": "JRC Global Forest Cover 2020", "version": "V4"},
              {"name": "Hansen Global Forest Change", "version": "v1.13 (2025)"}],
 "plots": [{"plot_id": "CIV-001", "verdict": "pass", "failing_area_ha": 0.0,
            "support": "strong", "limits": [],
            "tmf": {"agreement": null, "review_required": false}}]}
```

The one cell read `jrc_forest_2020: 0`, `hansen_treecover_2000: 20`,
`hansen_lossyear: 0`, with three fact cids, and the receipt verified
offline. Two things in that answer to pass on rather than smooth over:
TMF read no cells (`cells_read: 0`, `agreement: null`), and
`verdict_support` stayed `strong` anyway, because an unread TMF does not
lower the level. And one Point cell is about 9.5 m of a plot of up to
4 ha.

## Verify and keep

The response carries a signed `receipt` over every per-cell fact cid.
Verify it offline with
[`emem-verify-receipt`](../emem-verify-receipt/SKILL.md), and keep the
fact cids (or a bundle of them, `POST /v1/memory_bundle
{"fact_cids":[...]}`) in the operator's record. For a lab certificate or
land title that belongs in the same file, see
[`emem-document-evidence`](../emem-document-evidence/SKILL.md).

## What to tell the user

- The verdict, the plot's `verdict_support.level` and its `limits`.
- Whether review is required, and why (`tmf_cross_check.review_cells`).
- A Point geometry samples one cell (about 9.5 m) of a plot of up to
  4 ha; the response adds a `sampling_caveat` saying so.
- Legality and degradation are not assessed. The obligations apply from
  30 December 2026 for large operators and 30 June 2027 for micro and
  small enterprises (`regulation_status_note`); the cut-off date is
  unchanged.
