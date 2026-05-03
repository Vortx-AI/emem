# Temporal qualification — the math behind band routing

> When an agent asks "what's the state at cell C right now?", which
> band do they ask for? The answer depends on physics — the
> phenomenon's natural cadence — not heuristics. This document is
> the math, the architectural inspiration, and the protocol surface.


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
PDE family. The kernel is the fundamental solution evaluated at the
temporal lag `Δt = |τ_query − t_obs|`:

| tempo | PDE | kernel | parameter |
| --- | --- | --- | --- |
| `static`     | identity | `Q = 1` | none |
| `slow`       | AR-1 / first-order Markov | `Q = max(0, 1 − Δt/T)` | T = annual slot duration |
| `medium`     | heat: `∂u/∂t = D∇²u` | `Q = exp(−(Δt/σ)²)` | σ = monthly slot |
| `fast`       | wave + seasonal: `∂²u/∂t² = c²∇²u + ω·sin(2πt)` | `Q = max(0, ½ + ½·cos(2π·Δt/T))` | T ≈ Sentinel-2 revisit (5 d × 5) |
| `ultra_fast` | advection: `∂u/∂t + v·∇u = 0` | `Q = max(0, 1 − Δt/H)` | H ≈ 6 hourly slots |

These mirror the Temporal Dynamics Module in
`/home/ubuntu/agri/training/tdm.py` exactly. The TDM's role inside
the agri training stack is to roll embeddings forward in time using
these PDE operators; the router's role inside emem is the *inverse*
— given a query time, decide which band's last attestation is still
load-bearing under the same physics.

### Why these specific PDEs

- **Heat / Gaussian**: optical-band composites (NDVI 16-day, surface
  water occurrence) are the spatial-temporal solution of a diffusion
  process — pixel values smear as new observations average in. The
  fundamental solution of the heat equation is exactly the Gaussian
  kernel; using it as the score function is dimensionally correct.

- **Wave / cosine**: any seasonally-driven band (raw S2 NDVI, daily
  SAR backscatter under vegetation) has a periodic reset every
  revisit period. The 1-D wave equation's traveling-wave solutions
  are sinusoids; truncating at one period and clipping below zero
  gives the half-cosine kernel.

- **AR-1 / linear**: yearly aggregates (AlphaEarth annual, land
  cover) have one usable observation per cycle. A linear decay
  matches what a Markov-1 process actually gives; piecewise constant
  with a year-boundary discontinuity is the same idea but uglier.

- **Advection / linear with horizon**: weather and traffic are
  transported (advection) much faster than they're created. Linear
  decay over a short horizon (~6 slots) reflects how quickly the
  signal goes out of date.

- **Identity**: DEM and Köppen don't change on human timescales.

The router doesn't *solve* these PDEs at request time. It uses the
fundamental solution as a closed-form quality score, which is
constant-time per band. The full PDE rollout is the agri-side
predictor's job; emem's job is to pick which band to ask for.

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
| foundation, embedding, latent, general | alphaearth, geotessera, foundation | 1.3× |

The intent affinity is **explicitly editorial** — the math score
(`score_math` in the response) is the protocol-level contract; the
intent multiplier is a usability layer. An agent that wants to
implement its own routing should drop the intent table and use
`score_math` directly.

---

## JEPA — predicting a missing observation (research direction)

The router answers "which band is freshest?". A separate question —
"what *would* the value be at this `(cell, time)` if no one has
attested it?" — is where JEPA-style architectures come in. Not shipped
in 0.0.x; this section is the design.

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
- Needs a stability story for the rollout: the wave equation drifts
  if you push too far; a JEPA predictor trained on AlphaEarth
  annual data has only ~9 data points per pixel; integration with
  the existing PDE kernels for hybrid prediction is open research.

When ready, the protocol surface is `POST /v1/predict` returning a
`Fact::Derivative` with the JEPA fn_key. The temporal router would
add a third ranked list `predict_at_query_time` for cells where no
attestation is in range.

---

## Connecting to the agri stack

The math here is *exactly* the math the agri Temporal Dynamics
Module uses (`/home/ubuntu/agri/training/tdm.py`):
- Static / slow / medium / fast / ultra-fast band classes.
- Identity, AR-1, heat, wave, advection PDE operators per class.
- Physics-informed prior on top of which JEPA Extended adds
  scenario conditioning (Phase 4, 15.4 M parameters).

The agri stack is the *predictor*. emem is the *router + persistor*.
They share the band manifest and the dynamics taxonomy; an
operator running both gets a closed loop:
1. emem's router tells the agent which band to ask for.
2. The agent calls `/v1/recall`; the responder materializes from the
   upstream OR (eventually) from a JEPA-style predictor.
3. The result is signed and persisted for the next call.

---

## Quick example

```bash
curl -s -X POST https://emem.dev/v1/temporal_route \
  -H "content-type: application/json" \
  -d '{"cell":"damO.zb000.xUti.zde7d","intent":"monitor crop yield this week","limit":4}' \
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
PDE? A composite question (`is this place flooded right now?`,
`is the wildfire risk acute today?`) needs more than one band, each
queried over a *specific* lookback. 0.0.3 adds an additive surface
for that.

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

## References

- [I-JEPA paper (Meta, CVPR 2023)](https://arxiv.org/abs/2301.08243)
- [ReJEPA — JEPA for remote sensing retrieval (2025)](https://arxiv.org/abs/2504.03169)
- [V-JEPA — video feature prediction](https://ai.meta.com/research/publications/revisiting-feature-prediction-for-learning-visual-representations-from-video/)
- [FoodLab agri TDM source](`/home/ubuntu/agri/training/tdm.py`) (sibling project, Phase 4 training)
- [Sentinel-2 revisit math](https://www.mdpi.com/2072-4292/9/9/902) — global revisit-interval analysis
- [State of AI Agent Memory 2026](https://mem0.ai/blog/state-of-ai-agent-memory-2026)
