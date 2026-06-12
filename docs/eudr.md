# EUDR: Due Diligence Statements + Visual Evidence

`POST /v1/eudr_dds` produces a signed Annex II–shaped Due Diligence Statement under
[Regulation (EU) 2023/1115](https://eur-lex.europa.eu/eli/reg/2023/1115/oj) for one or more
operator-supplied plots. The endpoint covers the geolocation + deforestation parts of
Annex II; Article 9(1)(b) legality verification (land tenure, FPIC, country-of-origin
laws) is structurally out of Earth-observation scope and surfaces in the response as a
`legality_disclaimer` so the operator can route legality to a partner module before
submitting to TRACES NT.

The canonical JSON Schema lives at
[`/v1/schemas/eudr_dds.json`](https://emem.dev/v1/schemas/eudr_dds.json) (schema id
`emem.eudr_dds.v1`).

## Per-cell verdict

Each plot is sampled at `max_cells_per_plot` cell64s within the polygon. The sampler
auto-scales to full coverage from the plot's area (~110 cells/ha) unless the operator
pins a count; the ceiling is 51,200 cells. The verdict consensus runs
**`eudr_compliance@1`** per cell from two static-COG baselines:

| Band | Role |
|---|---|
| `jrc_gfc2020.forest_2020`        | EUDR forest baseline at the 2020-12-31 cut-off (Bourgoin et al. 2026 ESSD) — single-band uint8 `1`=forest |
| `forest_change.treecover2000`    | Hansen GFC v1.12 canopy fraction (%) at year 2000 |
| `forest_change.lossyear`         | Hansen GFC v1.12 first-loss year as a **calendar year** (`0`=no loss, else `2001..=2024`); loss strictly after the cut-off year is the failure signal |

Both baselines are read with one `cog::sample_window` per band over the polygon bounding
box and indexed per cell from the in-memory buffer — O(1) upstream reads, O(N) lookups —
so a fully-sampled polygon completes a base verdict in a couple of seconds warm. JRC TMF
v2025 deforestation, WRI-Sims driver attribution and RADD SAR alerts are not in the
hot-path consensus (TMF carries no upstream HTTP Range and costs ~78 s cold; WRI/RADD are
signed Absence today); each remains available as an explicit band request off the verdict
path.

The plot-level verdict aggregates per-cell verdicts with the Article 2(4) 0.5 ha MMU
floor: if `failing_area_ha < 0.5`, the verdict is demoted from `fail` to `below_mmu` and
treated as compliant in DDS aggregation. A cell cleared at or before the cut-off year is
`not_in_scope` (no longer forest at the cut-off), never `pass`.

## Loss-year histogram

Every plot carries a `loss_year_histogram`: the per-year distribution of Hansen loss-year
across the plot's sampled cells, with `by_year` (calendar year → cell count),
`after_cutoff_cells`, `no_loss_cells`, `unknown_cells`, and `total_sampled_cells`. It is
emitted as its own signed `forest_change.lossyear_histogram` derivative — `op: "histogram"`,
`parents` citing the per-cell `forest_change.lossyear` facts it aggregates — whose CID is
folded into the response receipt. The breakdown is therefore a verifiable, reproducible
figure rather than an unsigned summary: anyone re-aggregating the same cells signs the same
bytes, and the receipt binds the result.

Counts are over the cells actually sampled (`basis: "sampled_cells"`); weight by the plot's
`sampled_polygon_fraction` to extrapolate to the whole polygon. On very large plots the
`parents` list is bounded (with a `parents_capped` flag) so the derivative stays compact —
`total_sampled_cells` remains authoritative regardless.

## Visual evidence (opt-in)

Set `request_visual_evidence: true` on any plot to attach a per-plot `visual_evidence`
block that demonstrates *visually* whether the plot has experienced deforestation
between the 2020-12-31 cut-off and the current calendar year.

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

The block (schema id `emem.visual_evidence.v1`) is built from:

- **Sentinel-2 `indices.ndvi`** at a July-1 anchor for each year from 2020 through the
  current calendar year. The existing `s2_search_with_fallback` cloud ladder
  (40 → 60 → 80 % cloud, ±30 → 60 → 90 day window) finds the cleanest scene per cell;
  per-pixel SCL gating drops residual cloud pixels.
- **Sentinel-1 RTC VV-backscatter** at the same anchors via `materialize_sentinel1_vv`,
  providing a cloud-independent confirmation signal, critical in monsoon regions
  where S2 has cloud gaps.
- **Per-year `scene.png` URLs** for up to 6 representative cells of the plot, framed
  as full-year `?datetime=YYYY-01-01.../YYYY-12-31...` windows so the existing
  [`GET /v1/cells/{cell64}/scene.png`](https://emem.dev/openapi.json) handler picks
  the latest cleanest scene within each year. Render these as a 6-up year-by-year grid
  for an audit packet.

### Verdict thresholds

Both thresholds are env-tunable so operators can tighten for high-stakes audits or
loosen for noisy regions:

| Signal | Default threshold | Env var | Source |
|---|---|---|---|
| NDVI drop vs 2020 baseline       | ≥ 0.15 | `EMEM_VISUAL_NDVI_DROP_THRESHOLD` | Pelletier et al. 2024 |
| S1 VV backscatter drop vs 2020   | ≥ 3 dB | `EMEM_VISUAL_S1_DROP_DB_THRESHOLD` | Reiche et al. 2018 |

If either signal breaches its threshold the visual verdict is
`visual_deforestation_suspected`; if both stay within bounds the verdict is
`no_visual_deforestation`. A plot with no 2020 baseline data at all returns
`indeterminate_no_baseline` rather than silently passing: honest absence over
false confidence.

### Receipts

Every annual NDVI and S1 backscatter value is a signed Primary fact under the
responder's existing identity. Each year's entry in `visual_evidence.years[]` carries
`ndvi_fact_cids[]` and `s1_fact_cids[]` so an auditor can cite the exact upstream
attestation and verify the signature at `/verify/<fact_cid>` without trusting the
endpoint envelope.

### Latency

A single plot with visual_evidence enabled adds roughly 90 s of upstream fan-out
(7 years × N cells × 2 bands = ~112 materialisations for the default 8-cell sampling).
The EUDR budget auto-bumps to `60 s + 90 s × n_visual_plots` (capped at 600 s) when
any plot has the flag set, so multi-plot DDS submissions stay inside a single
synchronous round-trip. Operators can override via `EMEM_EUDR_TIMEOUT_SECS`.

## Compliance posture

EUDR Article 2(4) defines forest by canopy / height / area, **not by data source**.
The `eudr_compliance@1` baseline (JRC GFC2020 + Hansen) is the Commission's expected
(non-binding) baseline per Bourgoin et al. 2026 and gives the strongest audit
defensibility today. The `visual_evidence` block is supplementary narrative evidence; it
does not change the strict pass/fail verdict on its own.

The verdict measures **deforestation** — conversion of forest to non-forest after the
cut-off (Article 2(3)), via canopy loss. It does not measure **forest degradation**
(Article 2(7): structural changes that reduce a forest's biomass or ecological capacity,
e.g. primary or naturally-regenerating forest converted to planted/plantation forest, or
selective loss that keeps canopy above the 10 % threshold). Because the standard `pass`
statement-of-compliance wording asserts both limbs, the response carries a
`degradation_disclaimer` so the operator routes the degradation limb to a dedicated layer
(the JRC TMF `jrc_tmf.degradation_year` band is available on request). This sits alongside
the `legality_disclaimer` for Article 9(1)(b) — both make the boundary of the
Earth-observation verdict explicit rather than implied.

### Application timeline

The 31 December 2020 deforestation cut-off is fixed (Article 1(2)). The *obligations*
were deferred by Regulations (EU) 2024/3234 and 2025/2650: they apply from 30 December
2026 for large operators and 30 June 2027 for micro and small enterprises. The response's
`regulation_status_note` carries the current position; the cut-off the verdict applies is
unchanged by the deferral.

## Related endpoints

- `GET /v1/algorithms/eudr_compliance@1`: input bands, formula, accuracy notes
- `GET /v1/algorithms/eudr_dds@1`: polygon-aggregator algorithm card
- `GET /v1/schemas/eudr_dds.json`: full JSON Schema (draft 2020-12)
- `GET /v1/cells/{cell64}/scene.png`: true-colour Sentinel-2 RGB chip
- `GET /v1/cells/{cell64}/scene.rgb`: raw 256×256×3 byte stream for client-side rendering
