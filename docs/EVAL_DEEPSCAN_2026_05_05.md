# emem MCP — Deepscan Tool Evaluation (2026-05-05)

Deepscan of 32 tools against responder
`777er3yihgifqmv5hmc2wwmyszgddzderzhsx6rex4yoakwomvka` ·
29 tools live-tested across 4 real-world cells (Mt Fuji, Tokyo,
Tofino BC, Yosemite NP).

Scoring dimensions: correctness · discoverability · reliability ·
payload_quality · composability — each 0–5, total out of 25.

## Headline numbers

| metric | value |
| --- | --- |
| tools surfaced | 32 |
| tools live-tested | 29 |
| avg total score (tested) | 22.4 / 25 |
| bands wired | 95+ (35 cube + 60+ subkeys) |
| algorithms registered | 107 (16 solo · 78 combined · 13 embedding) |
| P0 issues | 3 |

## Per-tool scorecard

| tool | phase | score | verdict |
| --- | --- | --- | --- |
| `emem_manifests` | discovery | 25 | tiny payload of 7 CIDs; perfect citation anchor |
| `emem_data_availability` | discovery | 25 | 95+ entries with backfill flag + history bounds |
| `emem_query_region` | regional | 25 | multi-cell aggregation with full receipt |
| `emem_compare` | comparison | 25 | cosine + per-band deltas + only_a/only_b |
| `emem_verify` | verification | 25 | verdict + evidence CID. Clean. |
| `emem_fetch` | citation | 25 | CID → full signed body |
| `emem_cell_geojson` | viz | 25 | drop-in GeoJSON with neighbours |
| `emem_grid_info` | discovery | 24 | crisp; honest_warnings array surfaces resolution caveats |
| `emem_bands` | discovery | 24 | excellent ontology with interpretation/pitfalls/references |
| `emem_errors` | discovery | 24 | 27 codes with recover field; codes not always emitted live |
| `emem_locate` | location | 24 | topic-grouped data_at_this_cell is excellent |
| `emem_recall` | core | 24 | auto-materializes, signs, caches; `bands_available` field is misleading |
| `emem_trajectory` | temporal | 24 | honest about not auto-materializing |
| `emem_backfill` | temporal | 24 | materializes per-tslot signed facts |
| `emem_functions` | discovery | 23 | compact derivation-fn registry |
| `emem_heat_solve` | pde_solver | 23 | PDE math correct; 10m stencil inside 1km MODIS pixel |
| `emem_jepa_predict` | pde_solver | 23 | outstanding intellectual honesty about NOT-learned coefficients |
| `emem_sources` | discovery | 22 | 41 connectors; some schemes lack band wiring |
| `emem_diff` | comparison | 22 | allows `tslot_a == tslot_b` — should warn |
| `emem_coverage_matrix` | discovery | 21 | cube-slot vs subkey schism confuses agents |
| `emem_recall_polygon` | regional | 21 | `band_metadata` duplicated per-fact |
| `emem_compare_bands` | comparison | 21 | `tslot=0` default fails on medium-tempo bands |
| `emem_wave_solve` | pde_solver | 20 | accepts land-locked cells silently |
| `emem_materializers` | discovery | 19 | 62KB exceeds MCP token cap |
| `emem_algorithms` | discovery | 19 | 190KB massively exceeds cap |
| `emem_find_similar` | similarity | 18 | duplicate cells in top-k; empty `fact_cids` |
| `emem_ask` | single_shot | 18 | routing/composition excellent; 84KB exceeds cap |
| `emem_intent` | planner | 18 | drops caller params silently |
| `emem_cell_scene_rgb` | viz | — | skipped — returns binary PNG block |
| `emem_coverage_map` | viz | — | skipped — returns SVG block |
| `emem_schema` | discovery | — | skipped — rarely needed at chat time |

## Wiring inventory

| what | count | note |
| --- | --- | --- |
| bands declared in cube manifest | 35 | 1792 dims total |
| cube + subkey + free bands declared | 81 | per coverage_matrix |
| bands with ≥1 fact attested | 38 | 3,023 facts cached globally |
| bands with materializer (coverage_matrix) | 55 | vs 95+ in data_availability — registries disagree |
| source connectors registered | 41 | S2, S1, MODIS, ERA5, NASA POWER, GMRT, Tessera, Overture, JRC GSW, MET Norway, +more |
| source schemes without band wiring | 9 | VIIRS DNB, GHSL, WorldPop, Hansen GFC, TROPOMI, OpenET, CHIRPS, Dynamic World, Beck.Köppen |

## Capability gaps the user CAN'T currently get

| question a user might ask | why it doesn't work today |
| --- | --- |
| "How dense is the population here?" | `population` band has no materializer (WorldPop / GHSL not wired) |
| "Has this been deforested since 2010?" | `forest_change` no materializer (Hansen GFC schemes are wired in sources, just no fn maps to them) |
| "What's the soil pH?" | `soilgrids` cube slot unwired — subkey `soilgrids.phh2o_0_30cm` IS wired but agent has to know the subkey |
| "Show me nightlights trend" | `nightlights` no materializer (VIIRS DNB not wired) |
| "Is this in a protected area?" | `protected` no materializer (WDPA not wired) |
| "Active fire here right now?" | no band declared for VIIRS fire / FIRMS |
| "Methane / NO₂ plume from space?" | no band declared for TROPOMI CH4/NO2 |
| "Compare 2023 vs 2024 NDVI" | possible but agent must chain backfill → trajectory → diff — no `temporal_diff` materializer |
| "Predict 6-month NDVI" | `jepa_predict` caps at 1-month horizon (v1) |

## Recommended actions, prioritized

### P0
- **Fix payload-cap overflows** — paginate `emem_algorithms` (190KB) and `emem_materializers` (62KB); add `verbose=false` default to `emem_ask` (84KB).
- **Surface family-alias subkeys in `emem_coverage_matrix`** — when `air_quality` reports no materializer but `cams.*` subkeys are wired, agents falsely conclude no air-quality data exists.
- **Dedupe `emem_find_similar` + populate `fact_cids` in receipt** — same cell appears 2-3× in top-k, citation receipt is empty.

### P1
- Auto-pick latest `tslot` in `emem_compare_bands` when omitted — first call fails on medium-tempo bands due to default `tslot=0`.
- Reject land-locked walks in `emem_wave_solve` — Tofino village centre produced an all-zero depth profile and non-physical phase speeds.
- Warn on tiny stencil in `emem_heat_solve` — 10m grid inside 1km MODIS pixel always yields zero gradient.
- Wire missing materializers — order: population, forest_change, nightlights, protected, koppen, terraclimate.
- Plumb `emem_errors` catalog through every tool's error path — live errors ignore documented codes (e.g. `cid_not_found`).

### P2
- Param propagation in `emem_intent` — `k=3` was silently dropped; either propagate all fields or document which fields each intent type accepts.
- Rename `bands_available` in `emem_recall` response to `bands_already_attested_at_cell` — the current name suggests global wiring, not per-cell cache.
- Reject same-tslot diff in `emem_diff` — currently allows `tslot_a == tslot_b` and returns `delta=0`.
- Opt-in `band_metadata` in `emem_recall_polygon` — duplicates per-fact today, adds ~8KB to a 16-fact response.

### P3
- New endpoints — `emem_topics` (expose the topic registry), `emem_explain_algorithm` (per-key drill-down), native `temporal_diff` materializer, FIRMS active-fire connector, JEPA v2 (learned predictor).
