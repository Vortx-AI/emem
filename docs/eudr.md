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

Each plot is sampled at `max_cells_per_plot` cell64s within the polygon (default 16,
hard cap 256). Per cell, the responder runs **`eudr_compliance@1`** with these inputs:

| Band | Role |
|---|---|
| `jrc_gfc2020.forest_2020`        | EUDR forest baseline at 2020-12-31 cut-off (Bourgoin et al. 2026 ESSD) |
| `forest_change.treecover2000`    | Hansen GFC v1.12 canopy fraction at year 2000 |
| `forest_change.lossyear`         | Hansen GFC v1.12 first-loss year (post-cutoff if ≥ 2021) |
| `jrc_tmf.deforestation_year`     | JRC TMF v2025 tropical-belt loss year (ORs with Hansen for consensus) |
| `wri_gdm.driver_class`           | Sims et al. 2025 driver attribution *(signed Absence today)* |
| `radd.alert_date`                | Reiche et al. 2021 SAR alert date *(signed Absence today)* |

All six are fetched in **parallel** per cell (commit `b302164` parallelised the prior
serial loop), so a 16-cell 4 ha plot completes a base verdict in roughly 5–20 s warm.

The plot-level verdict aggregates per-cell verdicts with the Article 2(4) 0.5 ha MMU
floor: if `failing_area_ha < 0.5`, the verdict is demoted from `fail` to `below_mmu` and
treated as compliant in DDS aggregation.

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
The `eudr_compliance@1` baseline (JRC GFC2020 + Hansen + JRC TMF) is the
Commission's expected (non-binding) baseline per Bourgoin et al. 2026 and gives the
strongest audit defensibility today. The `visual_evidence` block is supplementary
narrative evidence; it does not change the strict pass/fail verdict on its own.

## Related endpoints

- `GET /v1/algorithms/eudr_compliance@1`: input bands, formula, accuracy notes
- `GET /v1/algorithms/eudr_dds@1`: polygon-aggregator algorithm card
- `GET /v1/schemas/eudr_dds.json`: full JSON Schema (draft 2020-12)
- `GET /v1/cells/{cell64}/scene.png`: true-colour Sentinel-2 RGB chip
- `GET /v1/cells/{cell64}/scene.rgb`: raw 256×256×3 byte stream for client-side rendering
