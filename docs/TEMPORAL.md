# Temporal qualification — the math behind band routing

> When an agent asks "what's the state at cell C right now?", which
> band do they ask for? The answer is a per-band **staleness score**,
> motivated by the analytical solutions of the heat / wave / advection
> PDEs in time but applied as a closed-form decay function — not a
> full PDE rollout. This document is the math, the architectural
> inspiration, and the protocol surface.
>
> **Honesty boundary**: the kernels below are decay-scoring
> heuristics. The math is correct (the Gaussian is the 1-D heat
> Green's function, the half-cosine is a one-period truncation of a
> sinusoidal traveling wave) but it is evaluated as a scalar score
> per band — there is no spatial mesh, no `∇²`, no characteristic
> solver, no diffusion-based forecasting in the shipped router.
> Real 2-D spatiotemporal solvers (`POST /v1/heat_solve`,
> `POST /v1/wave_solve`, `POST /v1/jepa_predict`) ship from
> 2026-05; see "Real PDE primitives (shipped 2026-05)" at the
> bottom of this doc.


---

## What `/v1/temporal_route` returns

`POST /v1/temporal_route` (also `GET` with query params) takes a cell,
a query time, and an optional intent string, then returns two ranked
lists:

1. **`cite_now`** — bands with the highest cite-able quality at the
   query time, given the existing attestations on this cell. Static
   phenomena (DEM, climate zone) dominate because their attestations
   never decay.
2. **`fetch_for_intent`** — bands whose family matches the intent
   keyword. The agent should `/v1/recall` these to trigger the
   responder's materializer.

Each candidate carries a quality score `Q ∈ [0, 1]`, the kernel
family used to compute it (`heat_gaussian`, `wave_seasonal`,
`linear_ar1`, `advection_linear`, or `identity`), and the derivation
formula so an agent (or a curious operator) can reproduce the score.

---

## The kernels

Each band's `tempo` class (declared in `bands-v0.json`) maps to one
PDE family for *inspiration*. The kernel is the analytical solution
of that PDE collapsed to time only, evaluated at the temporal lag
`Δt = |τ_query − t_obs|` and used as a per-band quality score. The
router does not solve the PDE; it just evaluates a closed-form
decay function:

| tempo | PDE inspiration | shipped kernel (decay score) | parameter |
| --- | --- | --- | --- |
| `static`     | identity | `Q = 1` | none |
| `slow`       | AR-1 / first-order Markov | `Q = max(0, 1 − Δt/T)` | T = annual slot duration |
| `medium`     | heat: `∂u/∂t = D∇²u` | `Q = exp(−(Δt/σ)²)` | σ = monthly slot |
| `fast`       | wave + seasonal: `∂²u/∂t² = c²∇²u + ω·sin(2πt)` | `Q = max(0, ½ + ½·cos(2π·Δt/T))` | T ≈ Sentinel-2 revisit (5 d × 5) |
| `ultra_fast` | advection: `∂u/∂t + v·∇u = 0` | `Q = max(0, 1 − Δt/H)` | H ≈ 6 hourly slots |

The "PDE inspiration" column names the equation the kernel is
*motivated by*; the "shipped kernel" column is what actually runs
in `quality_kernel` (`crates/emem-api-rest/src/lib.rs`). The agri-
side Temporal Dynamics Module
(`/home/ubuntu/agri/training/tdm.py`) uses the same five families
as actual PDE operators to roll embeddings forward in time. emem
0.0.x evaluates the closed-form analytical solutions of those PDEs
*in time only* and uses them as scalar staleness scores per band.
The taxonomy is shared; the runtime is not.

### Why these specific kernels

- **Heat / Gaussian** (`Q = exp(−(Δt/σ)²)`): the Gaussian *is* the
  fundamental solution (Green's function) of the 1-D heat equation
  `∂u/∂t = D∇²u` collapsed to time only. The math is correct in
  that narrow sense, and dimensionally it gives a well-behaved decay
  with a single parameter σ tied to the band's slot duration.
  **What it is not**: a 2-D spatiotemporal heat solver. There is no
  spatial mesh, no diffusion across neighbouring cells, no forecast.
  It is a scalar staleness score per band.

- **Wave / cosine** (`Q = max(0, ½ + ½·cos(2π·Δt/T))`): a half-cosine
  decay tuned to T ≈ Sentinel-2's 5-day revisit (≈25 days here, see
  the parameter column above). Useful for periodic phenomena because
  any seasonally-driven band resets every revisit; mathematically it
  is **not** a wave equation solver. There is no second-order time
  derivative, no spatial Laplacian, no characteristic-line solver,
  no propagation. The motivation is "the 1-D wave equation's
  traveling-wave solutions are sinusoids", but what ships is a
  cosine seasonality function clipped at one period — a periodic
  decay heuristic, nothing more.

- **AR-1 / linear**: yearly aggregates (Tessera annual, Prithvi
  vintage-anchored, land cover) have one usable observation per
  cycle. A linear decay matches what a Markov-1 process actually
  gives; piecewise constant with a year-boundary discontinuity is
  the same idea but uglier.

- **Advection / linear with horizon**: weather and traffic are
  transported (advection) much faster than they're created. Linear
  decay over a short horizon (~6 slots) reflects how quickly the
  signal goes out of date. Again: a scalar decay, not a real
  advection solver — no velocity field, no `v·∇u` term evaluated on
  a grid.

- **Identity**: DEM and Köppen don't change on human timescales.

The router doesn't *solve* these PDEs at request time, and the
0.0.x reference build does not solve them anywhere else either.
Each kernel is a closed-form decay function, constant-time per
band. The full PDE rollout — actual diffusion, actual wave
propagation, actual advection — is queued for v0.1 (see the bottom
of this doc).

---

## How `intent` fits in

The intent string is a free-text phrase like `"monitor crop yield
this week"` or `"flood risk for the next storm"`. The router
applies a small multiplicative boost (≤ 1.5×) to bands whose family
matches one of these keyword groups:

| keywords | family match | boost |
| --- | --- | --- |
| flood, water, wet, river | surface_water, ocean_chl, water | 1.5× |
| forest, deforest, tree, logging | forest_change, mangrove, vegetation | 1.5× |
| crop, farm, agri, harvest, yield | indices, phenology, ndvi, vegetation | 1.5× |
| urban, city, population, human | nightlights, ghsl, population, human | 1.5× |
| climate, weather, temperature, rainfall | climate, terraclimate, koppen | 1.5× |
| terrain, elevation, mountain, depth, bathymetry | dem, cop_dem, topobathy, terrain | 1.5× |
| radar, all-weather, cloud, night | sentinel1 | 1.4× |
| foundation, embedding, latent, general | geotessera, prithvi_eo2, galileo_base_v1, alphaearth (reserved), foundation | 1.3× |

The intent affinity is **explicitly editorial** — the math score
(`score_math` in the response) is the protocol-level contract; the
intent multiplier is a usability layer. An agent that wants to
implement its own routing should drop the intent table and use
`score_math` directly.

---

## JEPA — predicting a missing observation

The router answers "which band is freshest?". A separate question —
"what *would* the value be at this `(cell, time)` if no one has
attested it?" — is where JEPA-style architectures come in. From
2026-05 emem ships a **constrained** version of this:
`POST /v1/jepa_predict` returns a one-month-ahead NDVI forecast
under a closed-form AR(2) seasonal predictor (α=0.6 year-over-year,
β=0.3 recent slope, γ=0.1 long-term mean reversion). The
coefficients are fixed from the agricultural-NDVI literature, not
learned — this is honestly NOT a learned MLP. The full learned
encoder/predictor pair (per the I-JEPA / V-JEPA / ReJEPA design
described below) is the planned `jepa_temporal_predictor@2`.

### What JEPA does

[I-JEPA](https://arxiv.org/abs/2301.08243) (Meta AI, 2023) and its
descendants ([V-JEPA](https://ai.meta.com/research/publications/revisiting-feature-prediction-for-learning-visual-representations-from-video/),
[ReJEPA](https://arxiv.org/abs/2504.03169)) **predict embeddings of
masked regions, not pixels**. Given a context window and a set of
target locations, the model predicts what the encoder *would*
produce at those targets. ReJEPA in particular shows 40–60%
compute savings vs masked-autoencoder pixel reconstruction, and
strong remote-sensing retrieval results (FMoW-RGB +8.7% over
SS-CMIR, +17.2% over SatMAE).

### What it would look like in emem

For a query at `(cell C, time τ)` where no attestation exists:

1. Gather context tokens: signed Primary facts at neighboring cells
   `{C₁, …, Cₖ}` and at recent time slices `{τ₁, …, τₘ}` for the
   same band.
2. Encode each context fact's `(cell, band, value)` triple into the
   1792-D embedding space using the band-specific encoder declared
   in `functions-v0.json`.
3. Run the JEPA predictor over the context tokens to produce a
   target embedding for `(C, τ)`.
4. Decode that embedding back into a band-specific value (or just
   keep it as an embedding — agents working in latent space don't
   need decoding).

Critically, **the predicted value is not signed as a Primary fact**
— it's a derivative produced by a registered function. The
attestation would be `Fact::Derivative` with:
- `op`: `"jepa_temporal_predict"`
- `parents`: the context fact CIDs
- `derivation.fn_key`: `"emem_jepa@1"`

so any verifier can re-run the prediction and confirm the answer.

### Why we don't ship this yet

- Needs a trained predictor. The agri TDM exists but is wired into
  the training-stack-side, not the responder. Wiring requires
  exposing the predictor as a deterministic fn_key and integrating
  the model weights into the responder's storage.
- Needs a stability story for the rollout: a real wave-equation
  rollout drifts if you push too far; a JEPA-style predictor trained
  on Tessera annual data has only one data point per pixel (Tessera
  ships vintage 2024 only upstream as of v0.0.4); integration with
  the staleness kernels above for hybrid prediction is open research.

When ready, the protocol surface is `POST /v1/predict` returning a
`Fact::Derivative` with the JEPA fn_key. The temporal router would
add a third ranked list `predict_at_query_time` for cells where no
attestation is in range.

---

## Connecting to the agri stack

emem and the sibling agri stack share a *taxonomy*, not (yet) a
runtime. The shared pieces today:
- Static / slow / medium / fast / ultra-fast band classes.
- The same five kernel families per class (identity, AR-1, heat,
  wave, advection) — emem uses them as scalar staleness scores,
  agri uses them as actual PDE operators inside the Temporal
  Dynamics Module (`/home/ubuntu/agri/training/tdm.py`).

The agri stack is the (in-progress) *predictor*. emem is the
*router + persistor*. They share the band manifest and the dynamics
taxonomy; the closed loop is the v0.1 design, not the 0.0.x reality:

1. emem's router tells the agent which band to ask for.
2. The agent calls `/v1/recall`; in 0.0.x the responder materializes
   from the upstream open-data fetcher. A JEPA-style predictor as a
   second materializer source is queued for v0.1.
3. The result is signed and persisted for the next call.

---

## Quick example

```bash
curl -s -X POST https://emem.dev/v1/temporal_route \
  -H "content-type: application/json" \
  -d '{"cell":"defi.zb493.xoso.zcb6a","intent":"monitor crop yield this week","limit":4}' \
| jq '.cite_now[0:2], .fetch_for_intent[0:3]'
```

Expected: `cite_now` lists the static bands (DEM, terrain) the agent
can already cite; `fetch_for_intent` lists the vegetation bands
(`indices`, `phenology`, `temporal_diff`) the agent should request
to satisfy the crop-yield intent. Each entry carries the `kernel`,
`derivation`, and `last_obs_age_s` so the agent can audit the math.

---

## Algorithm-declared temporal recipes (0.0.3+)

The router above answers the *general* question — given a cell and a
query time, which bands are still cite-able under the appropriate
staleness kernel? A composite question (`is this place flooded right
now?`, `is the wildfire risk acute today?`) needs more than one band,
each queried over a *specific* lookback. 0.0.3 adds an additive
surface for that. The recipe is naive rolling-window aggregation
(sum / mean / median / max / min / latest / first); no PDE math
runs inside the windowing.

### `Algorithm.temporal_recipe`

Each algorithm in `crates/emem-core/data/algorithms-v0.json` may
declare a `temporal_recipe` block:

```json
"temporal_recipe": {
  "label": "antecedent-rainfall + recent-radar-water + optical-water",
  "windows": [
    {
      "band": "weather.precipitation_mm",
      "lookback_days": 7,
      "aggregator": "sum",
      "purpose": "antecedent_rainfall",
      "trigger_threshold": 50.0
    },
    {
      "band": "indices.ndwi",
      "lookback_days": 14,
      "aggregator": "max",
      "purpose": "recent_radar_water"
    },
    {
      "band": "water.recurrence",
      "lookback_days": 30,
      "aggregator": "max",
      "purpose": "optical_water_baseline"
    }
  ]
}
```

Live examples (0.0.3): `flood_risk@2`, `water_consensus@1`,
`wildfire_exposure_score@1`, `spi_meteorological_drought@1`. List
via `GET /v1/algorithms?has_temporal_recipe=true`.

### `temporal_composition[]` on `/v1/ask` and `/v1/intent`

When the matched topic routes to an algorithm carrying a
`temporal_recipe`, the responder runs each window and returns an
additive sibling array under `temporal_composition`:

```json
{
  "facts": [...],            // unchanged: the snapshot recall
  "temporal_composition": [   // NEW in 0.0.3, additive
    {
      "algorithm_key": "flood_risk@2",
      "label": "antecedent-rainfall + recent-radar-water + optical-water",
      "windows": [
        {
          "band": "weather.precipitation_mm",
          "lookback_days": 7,
          "aggregator": "sum",
          "samples": [
            {"tslot": 19873, "value": 12.4, "fact_cid": "..."},
            {"tslot": 19874, "value": 18.1, "fact_cid": "..."}
          ],
          "aggregate_value": 87.3,
          "trigger_threshold": 50.0,
          "trigger_fired":     true
        }
      ]
    }
  ]
}
```

`facts[]` keeps the snapshot. `temporal_composition[]` is empty
when no matched algorithm declares a recipe. Existing readers that
parse `facts[]` only continue to work unchanged. The recipe is also
mirrored inline on each `/v1/intent → composite_suggestions.applicable[]`
entry, so an agent planning the follow-up `/v1/ask` sees the
windows without a second `GET /v1/algorithms/<key>` round-trip.

### Open question (OQ-12)

The per-window fact CIDs are signed individually today. A
`composition_cid` (Merkle root over per-window CIDs + recipe CID +
aggregator output) is on the v0.0.4 backlog so a downstream
verifier can re-execute the recipe and audit the aggregator output
without re-fetching every input. See
`docs/MILESTONE_v0.0.4.md` §M4.

---

## Real PDE primitives (shipped 2026-05)

The 0.0.x router scores staleness with closed-form decay kernels
above. Three **real** explicit-finite-difference solvers now ship
alongside the router as first-class POST endpoints. Each evaluates
an actual PDE discretisation under a CFL stability check, signs the
result with the responder identity, and cites every input fact CID
in the receipt.

- **`heat_equation_2d@1`** — `POST /v1/heat_solve {cell, hours_ahead, diffusivity_m2_per_s}`.
  2-D explicit FTCS solver for `∂u/∂t = α∇²u` over a 3×3 cell
  stencil. Reads `modis.lst_day_8day` at the centre and 8 cell64
  neighbours, integrates forward under `α·Δt/Δx² ≤ 0.20` (the 2-D
  stability bound is 0.25; we run at 0.20 of it for round-off
  margin). Default α=1e-6 m²/s matches urban surfaces (Oke 2017
  §2.3 Table 2.4); horizon capped at 168 h. Returns a signed
  forecast Kelvin scalar plus the full 9-cell initial condition
  and CFL diagnostics.

- **`wave_equation_1d@1`** — `POST /v1/wave_solve {coastal_cell, offshore_height_m, period_s, n_offshore_cells}`.
  1-D explicit CTCS solver for `∂²u/∂t² = c²∂²u/∂x²` with
  `c² = g·h` from `gmrt.topobathy_mean` along the seaward
  bathymetric gradient. Walks N cells offshore from the coastal
  cell (default 8 → 80 m profile at the active 10 m grid),
  integrates under `c·Δt/Δx ≤ 0.5` (half the CFL bound), with a
  sinusoidal offshore boundary `H_s·sin(2π·t/T)` and a hard wall
  at the coast. Returns arrival height + arrival time + the
  depth and phase-speed profiles.

- **`jepa_temporal_predictor@1`** — `POST /v1/jepa_predict {cell, band, lookback_months}`.
  Constrained AR(2) seasonal predictor on `indices.ndvi`. Reads
  the past `lookback_months` of NDVI at the cell, fits the
  closed-form predictor `y_{t+1} = α·(lag-12 NDVI or recent_mean) + β·(last + slope) + γ·recent_mean`,
  clamps to `[-1, 1]`. **Coefficients (α=0.6, β=0.3, γ=0.1) are
  fixed from the agricultural-NDVI literature — NOT learned.** This
  is the honest constrained version of the JEPA pattern; v2 will
  train an actual encoder + predictor on the geotessera embedding
  pool. Returns the prediction plus the lookback values, the
  cited fact CIDs, and an explicit `lag_12_used` honesty flag.

Every primitive carries a responder-signed `Receipt` over the input
fact CIDs and is independently verifiable via `POST /v1/verify_receipt`.
The math is pure (`heat_step_2d`, `wave_step_1d`, `jepa_predict_ar2_seasonal`)
and unit-tested without storage.

---

## References

- [I-JEPA paper (Meta, CVPR 2023)](https://arxiv.org/abs/2301.08243)
- [ReJEPA — JEPA for remote sensing retrieval (2025)](https://arxiv.org/abs/2504.03169)
- [V-JEPA — video feature prediction](https://ai.meta.com/research/publications/revisiting-feature-prediction-for-learning-visual-representations-from-video/)
- [FoodLab agri TDM source](`/home/ubuntu/agri/training/tdm.py`) (sibling project, Phase 4 training)
- [Sentinel-2 revisit math](https://www.mdpi.com/2072-4292/9/9/902) — global revisit-interval analysis
- [State of AI Agent Memory 2026](https://mem0.ai/blog/state-of-ai-agent-memory-2026)
