# Physics primitives — live behaviour (2026-05-04)

Three primitives ship in the `0.0.x` line. All accept either a `cell` (CID) or a `place` string (geocoded via embedded → cache → photon → nominatim) and return a CBOR-canonical, ed25519-signed receipt.

| primitive            | endpoint            | algorithm key                  | input band                | output band   |
| -------------------- | ------------------- | ------------------------------ | ------------------------- | ------------- |
| 2-D heat diffusion   | `POST /v1/heat_solve` | `heat_equation_2d@1`          | `modis.lst_day_8day` (K)  | `K` at t+Δt   |
| 1-D shallow-water    | `POST /v1/wave_solve` | `wave_equation_1d@1`          | `gmrt.topobathy_mean` (m) | `m` at coast  |
| AR(2) seasonal NDVI  | `POST /v1/jepa_predict` | `jepa_temporal_predictor@1` | `indices.ndvi`            | `ndvi` at t+1 |

All three are dispatched by `topic_router.rs` whenever `/v1/ask` matches a topic in `{public_health, weather_now, flood_*, vegetation_condition, agriculture, snow}` AND the question wording contains a physics-style verb (`forecast`, `diffuse`, `propagate`, `predict`, `arrival`, `evolve`).

---

## 1. `/v1/heat_solve` — explicit FTCS 2-D Laplacian

**Tested:** Tokyo +12h, Phoenix +24h.

```bash
curl -X POST https://emem.dev/v1/heat_solve \
  -H 'content-type: application/json' \
  -d '{"place":"Phoenix, Arizona","hours_ahead":24}'
```

**Key response fields:**

| field | meaning | example (Phoenix +24h) |
|---|---|---|
| `cell` | resolved cell64 of the centre | `defi.zb57c.wIma.dore` |
| `neighborhood_cells[9]` | NW…SE 3×3 stencil | 9 cell64s, centre at idx 4 |
| `neighborhood_initial_k[9]` | T₀ for each stencil cell | `[307.7, 307.7, …]` (all uniform) |
| `initial_condition_k` | T₀ at centre | `307.7` (= 34.6 °C) |
| `forecast_k` | T₁ at centre | `307.7` |
| `delta_k` | T₁ − T₀ | `0.0` |
| `cfl_factor` / `cfl_bound` | α·Δt/Δx² vs the FTCS 0.25 cap | `0.000864 / 0.25` |
| `dt_seconds` / `n_steps` | integration window | `86400 / 1` |
| `diffusivity_m2_per_s` | α (urban surface, Oke 2017) | `1e-6` |
| `input_band` | source band | `modis.lst_day_8day` |
| `imputed_neighbor_indices` | which stencil cells came from imputation | `[]` |
| `algorithm_citation` | Crank & Nicolson 1947 / Oke 2017 §2.3 | — |

**Honest finding (Phoenix run).** All 9 stencil cells returned the same 307.7 K because the lazy materializer fills the perimeter from a single point-sample (Open-Meteo). With a uniform field the Laplacian is exactly zero, so `delta_k = 0` is correct — but the diffusion is invisible until the perimeter facts come from a true raster (e.g., MODIS LST tile crop) or from genuinely different upstream sources. This is surfaced in the response: an agent can read `neighborhood_initial_k` and detect uniformity itself.

**When fired.** `/v1/ask` routes to it for queries like "predict temperature evolution …", "how will the urban heat island propagate", "forecast next 24h".

---

## 2. `/v1/wave_solve` — explicit CTCS 1-D linear shallow water

**Tested:** Miami Beach (12 cells), Mumbai (20 cells, T=12s, H_offshore=2.5 m).

```bash
curl -X POST https://emem.dev/v1/wave_solve \
  -H 'content-type: application/json' \
  -d '{"place":"Mumbai, India","offshore_height_m":2.5,"period_s":12,"n_offshore_cells":20}'
```

**Key response fields:**

| field | meaning | example (Mumbai 20-cell) |
|---|---|---|
| `coastal_cell` | resolved coastal cell64 | `defi.zb4d9.cojE.zf4be` |
| `profile_cells_offshore_to_coast[N]` | cell IDs from open ocean to shore | 20 cell64s |
| `depth_profile_m[N]` | bathymetry at each profile cell (positive = below MSL) | all `0.0` |
| `phase_speed_profile_m_per_s[N]` | c = √(gh) per cell | all `0.313` (= √(9.81·0.01)) |
| `arrival_height_m` | wave height at the coastal cell | `0.108` |
| `arrival_time_s` | first crossing of `arrival_threshold_m`; `null` if never | `null` |
| `arrival_threshold_m` | detection floor | `0.125` (= 5% of H_offshore) |
| `cfl_factor` / `cfl_bound` | c·Δt/Δx vs 1.0 cap (run at 0.5 safety) | `0.5 / 1.0` |
| `dt_seconds` / `n_steps` | timestep & step count for one full period | `15.96 / 41` |
| `input_band` | source band | `gmrt.topobathy_mean` |
| `algorithm_citation` | Lighthill 1978 §3.1; Holthuijsen 2007 §5.3 | — |

**Honest finding.** Both Miami Beach and Mumbai gave `depth_profile_m = [0,0,…0]`. The solver picks the offshore profile by walking N cells in a fixed direction from the input cell — when that walk lands on continental crust (very shallow nearshore tile, or wrong heading off a city centroid) the depth floor (0.01 m) kicks in to keep CFL finite. The phase-speed pancake of 0.313 m/s and the suspiciously-clean `arrival_height_m = 0.108` are the visible symptom. The honest fix surfaces in the `next.longer_profile` hint: try `n_offshore_cells: 40` or pass a `coastal_cell` that already sits in real water. A v0.0.4 task is to greedily search for the nearest deep-water heading instead of hard-coded direction stepping.

**When fired.** "wave run-up", "tsunami arrival", "storm-surge propagation", "swell at the coast" → flood_water_event_window or weather_now.

---

## 3. `/v1/jepa_predict` — closed-form AR(2) seasonal NDVI

**Tested:** Ludhiana (Punjab), Iowa City.

```bash
curl -X POST https://emem.dev/v1/jepa_predict \
  -H 'content-type: application/json' \
  -d '{"place":"Iowa City, Iowa","band":"indices.ndvi","lookback_months":12}'
```

**Key response fields:**

| field | meaning | example (Iowa City) |
|---|---|---|
| `cell` | resolved cell64 | `defi.zb5da.boto.zba5f` |
| `band` | NDVI band id | `indices.ndvi` |
| `lookback_months_requested` / `lookback_months_used` | requested vs what we had facts for | `12 / 1` |
| `history_tslots[N]` / `history_values[N]` | the actual NDVI series fed in | `[0]` / `[0.278]` |
| `lag_12_used` / `lag_12_value` | did the seasonal lag-12 path fire? | `true / 0.278` |
| `lag_12_fallback_to_recent_mean` | did lag-12 fall back to the recent mean? | `false` |
| `forecast_value` | y_{t+1}, clamped to [-1, 1] | `0.278` |
| `forecast_horizon_months` | always 1 in v1 | `1` |
| `predictor_form` | y_{t+1} = α·lag12 + β·(last+slope) + γ·mean | — |
| `predictor_coefficients` | α=0.6, β=0.3, γ=0.1 | — |
| `honesty_note` | "v1 closed-form, NOT a learned MLP" | — |
| `materialize_notes[]` | which facts were lazily fetched this call | 1 entry, status `materialized` |
| `next.extend_lookback` | hint to call `/v1/backfill` for more history | — |

**Honest finding.** Both Punjab and Iowa returned `lookback_months_used = 1` because only one NDVI fact existed at the moment of query — the AR(2) therefore collapses to `forecast = last_value` (lag-12 = last, slope = 0, mean = last). The agent can detect this from `history_values.len() == 1` and either retry after a `/v1/backfill` call (which the response itself suggests), or downgrade confidence. The `honesty_note` field is permanent and explicit: v1 ships **closed-form coefficients calibrated from the agricultural-NDVI literature, not a learned encoder**. v2 will load a real geotessera-trained predictor.

**When fired.** "forecast NDVI / vegetation / crop health for next month", agriculture or vegetation_condition topics with a forward-looking verb.

---

## Cross-cutting properties

All three primitives:

1. **Sign every byte** — the receipt's `signature` covers the whole response payload (including all numerical fields). Verify offline with `POST /v1/verify_receipt`.
2. **Cite their inputs by CID** — `input_fact_cids[]` is dereferenceable via `GET /v1/facts/{cid}`.
3. **Surface their CFL safety margin** — heat at 0.20 of the 0.25 bound; wave at 0.50 of the 1.0 bound; both clamped before any timestep runs.
4. **Honest about provenance** — `resolved_from.{kind, label, lat, lng, via}` shows whether the cell came from `embedded`, `cache`, `photon`, or `nominatim`.
5. **Honest about completeness** — `imputed_neighbor_indices` (heat), `depth_profile_m` (wave), `lookback_months_used` (jepa), `materialize_notes[]` (all three) all let the caller decide whether the answer is trustworthy for their purpose.

## Known limitations (open in 0.0.x)

- **Heat:** lazy materializer fills the 3×3 stencil from a single point-sample, so the Laplacian collapses to zero on first call. Fix: crop a real MODIS LST tile so the perimeter has spatial variation.
- **Wave:** offshore profile is generated by linear cell stepping; if the heading walks inland the depth profile zeros out. Fix: pre-compute a "nearest open-ocean heading" per coastal cell.
- **JEPA:** closed-form coefficients only; one-shot fact materialisation gives `n_history = 1` and degenerates to identity. Fix: backfill 12 months on first call, or ship a learned encoder as `jepa_temporal_predictor@2`.

All three failure modes are **visible in the response payload** — no silent fallbacks. That is the protocol contract.
