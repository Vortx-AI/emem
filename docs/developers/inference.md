# emem inference

## What this layer does

Three explicit-method solvers run in-process behind `/v1/heat_solve`
(2D heat), `/v1/wave_solve` (1D shallow water) and `/v1/jepa_predict`
(closed-form NDVI AR(2)). No GPU and no model server is involved.

Earlier versions also ran Clay v1.5, Prithvi-EO-2.0, Galileo and a
JEPA-v2 dynamics head on a GPU sidecar. Those are retired; facts they
signed still recall and verify.

## Physics solvers (`crates/emem-api-rest/src/physics.rs`)

These run in-process. CFL stability is checked at request time and the receipt cites every
fact CID that fed the discretisation.

### `/v1/heat_solve`: 2D explicit FTCS

|                  |                                                                  |
|------------------|------------------------------------------------------------------|
| input            | `cell, hours_ahead` (cap 168), `diffusivity_m2_per_s` (default 1.0e-6) |
| stencil          | 3×3 MODIS `lst_day_8day` (NW, N, NE, W, centre, E, SW, S, SE)    |
| pitch            | 10 m grid                                                        |
| CFL              | safety 0.20 (max stable 0.25 for 2D heat)                        |
| diagnostics      | uniform-stencil detection, imputed-neighbour count               |

The 5-point Laplacian update at every step is

    u_new = u + α·Δt·(N + S + E + W − 4·centre)/Δx²

with Dirichlet boundaries. Horizon ≤ 168 h and step count ≤ 2 × 10⁶
iterations; both caps are defensive against an agent passing a tiny
α and a long horizon that would spend minutes in the handler.

When the 3×3 stencil's range is below 0.01 K (well under the MODIS
LST instrument noise floor of ~0.5 K) the response flags
`is_uniform=true` and distinguishes a real "no diffusion expected"
outcome from a stencil that collapsed because the upstream
materialiser populated all 9 cells from one coarser source pixel.

### `/v1/wave_solve`: 1D shallow water

|                            |                                                                           |
|----------------------------|---------------------------------------------------------------------------|
| input                      | `coastal_cell, swell_height_m, swell_period_s`                            |
| profile                    | walk seaward (cardinal-only), pick deepest GMRT cell, build ≥ 3-cell profile, reverse |
| land-locked rejection      | offshore depth ≥ 5 m AND ≥ 50 % of profile > 1 m                          |
| boundary, offshore         | `H_s · sin(2π·t/T)` (driven swell)                                        |
| boundary, coastal          | hard wall (u = 0)                                                         |
| wave speed                 | `c = √(g·h)`, floored at 0.01 m                                           |
| CFL                        | safety 0.5                                                                |

A profile that fails the land-locked check returns 422 with the
actual GMRT depths attached and a hint (`try_longer_profile` or
`try_different_cell`) so an agent can iterate. Returning a fabricated
"wave" for an inland cell is the kind of silent hallucination the
protocol refuses.

### `/v1/jepa_predict`: closed-form NDVI AR(2)

Coefficients: α=0.6, β=0.3, γ=0.1.

    pred = clamp(α · lag_12 + β · (last + trend) + γ · recent_mean, [-1.0, 1.0])

`trend` is the least-squares slope through the last `lookback_months`
NDVI samples. `lag_12` is the same-month value one year ago when the
lookback window includes it; otherwise the α term degrades to
`recent_mean`. Default lookback is 6 months; cap is 24.

This is NOT a learned model. The receipt does not carry `trained`
claims and the response does not pretend to a forecast quality the
math cannot support. It is published as a reproducible AR(2)
seasonal predictor with stated coefficients; agents can reimplement
it from the receipt alone. The route name is historical.
