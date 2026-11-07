# emem inference

## What this layer does

emem runs three small numeric solvers in the server process. They use no
GPU and no model server.

| route | method | input band |
|---|---|---|
| `POST /v1/heat_solve` | 2D explicit heat equation (FTCS) | `modis.lst_day_8day` |
| `POST /v1/wave_solve` | 1D explicit shallow-water wave equation | `gmrt.topobathy_mean` |
| `POST /v1/jepa_predict` | closed-form NDVI AR(2) seasonal predictor | `indices.ndvi` |

The MCP tools are `emem_heat_solve`, `emem_wave_solve` and
`emem_jepa_predict`. The code is in `crates/emem-api-rest/src/physics.rs`.

Earlier versions also ran Clay v1.5, Prithvi-EO-2.0, Galileo and a
JEPA-v2 dynamics head on a GPU sidecar. emem no longer runs these models. Facts
that they signed still recall and verify.

## Common behaviour

Each solver reads its input facts through recall, and fetches a fact
when it is missing. Each response has a receipt that cites the CID of
every input fact. The `cell` field accepts a cell64 or a place name. For
a place name, the solver calls `/v1/locate` first.

The grid pitch is the cell64 pitch at the equator, about 9.55 m
(`emem_codec::CELL_PITCH_M_EQUATOR`). The response field `cell_pitch_m`
shows the value. The solvers check the CFL stability condition before
they start.

## `/v1/heat_solve`: 2D explicit FTCS

| field or setting | value |
|---|---|
| `cell` (alias `place`) | required |
| `hours_ahead` | default 6, maximum 168 |
| `diffusivity_m2_per_s` | default `1.0e-6` |
| stencil | 3 x 3 cells of `modis.lst_day_8day`: the centre and its 8 neighbours |
| time step | `hours_ahead` cut into equal steps of at most `0.20 · Δx² / α` (stability limit 0.25) |
| step limit | 2 000 000 steps |
| diagnostics | `stencil_diagnostic`, `imputed_neighbor_indices` |

Each step applies the 5-point Laplacian update:

    u_new = u + α·Δt·(N + S + E + W - 4·centre)/Δx²

The 8 perimeter cells keep their values (Dirichlet). Only the centre
cell changes. The scheme is stable while `α·Δt/Δx²` is 0.25 or less.
The solver uses 0.20 as a safety margin, so its largest step is
`0.20 · Δx² / α`. With the default α this step is about 211 days, so a normal call takes one
step. A large α needs many steps. The step limit stops such a call from
holding the handler for minutes. The 168 h limit exists because an 8-day MODIS
composite is not a good start state for more than a week.

When a neighbour cell has no usable fact, the solver uses the centre
value in its place. `imputed_neighbor_indices` lists these cells.

When the range of the 9 stencil values is less than 0.01 K, the
response sets `stencil_diagnostic.is_uniform` to true. The MODIS LST noise floor is about
0.5 K. A uniform stencil can mean two things. The surface is really
uniform, or one coarse source pixel filled all 9 cells. The flag tells
the caller to check which.

The answer is in `forecast_k`, the centre temperature after
`hours_ahead`. `delta_k` is the change from `initial_condition_k`.
`neighborhood_initial_k` gives the 9 start values in the order of
`neighborhood_order`.

Example, run on emem.dev on 7 November 2026 (HTTP 200, `n_steps` 1,
`dt_seconds` 21600, which is the default 6 hours):

```sh
curl -s https://emem.dev/v1/heat_solve \
  -H 'content-type: application/json' \
  -d '{"cell":"Tokyo"}'
```

That call gave `initial_condition_k` 305.04, `forecast_k` 305.025 and
`delta_k` -0.015. The 8 neighbours all read 289.58 K. The `neighborhood_signed_at`
field shows why. The responder signed the centre fact on 28 May 2026,
and the neighbour facts on 7 November 2026. Thus the 15.46 K range in the stencil
is a difference in time, not in space.

The solver recalls each of the 9 cells with no `tslot`. A cell with no
fact gets a fetch first. The solver does not check that the 9 facts come from the
same 8-day composite. `neighborhood_signed_at` gives the time when the
responder signed each fact, not the composite date. A different signing
date suggests a different composite, but does not prove it. Compare those
dates before you use `delta_k`.

## `/v1/wave_solve`: 1D shallow water

| field or setting | value |
|---|---|
| `coastal_cell` (aliases `cell`, `place`) | required |
| `offshore_height_m` | required, offshore wave height H_s, maximum 30 |
| `period_s` | required, 2 to 30 |
| `n_offshore_cells` | default 8, 1 to 64 |
| profile | walk seaward in the deepest of the 4 cardinal directions, then reverse |
| offshore boundary | `H_s · sin(2π·t/T)` (driven swell) |
| coastal boundary | hard wall (u = 0) |
| wave speed | `c = √(g·h)`, with h at least 0.01 m |
| CFL | 0.5 |
| run length | wave travel time along the profile plus one `period_s`, at least 3 steps |
| step limit | 200 000 steps |

The solver rejects a profile as land-locked unless both of these are
true:

- The offshore boundary cell is more than 5 m deep.
- At least 50 % of the profile cells are more than 1 m deep.

A land-locked profile returns HTTP 422. The body has `code: invalid_argument`,
a `message` and a `details` object. `details` has `error_kind: LandLockedProfile`, the measured
GMRT depths in `depth_profile_m` (offshore end first), the limits in
`thresholds` and a `next_steps` list. The list has
`try_longer_profile` and `try_different_cell`. The solver does not
return a wave for an inland cell.

The profile is short: 64 cells is about 610 m. The walk also stops
at the first cell that is on land again, and gives that cell a depth
of 0 m. Thus a profile can have fewer cells than `n_offshore_cells`.
Near shallow coasts the walk often does not reach 5 m of water.

The profile must have at least 3 cells. A shorter profile returns HTTP 422
with `code: source_fetch_failed` and the message "returned only N
cells; need at least 3 for the wave solver". A cell on the walk with no
usable GMRT value gives the same code, with the message "no usable
`gmrt.topobathy_mean` primary fact at cell ...". Read the message to know
which case you have.

On success, `arrival_height_m` is the largest wave height at the
coast. `arrival_time_s` is the time it first reaches
`arrival_threshold_m`, which is 5 % of `offshore_height_m`. It is null
when the wave does not reach the threshold. On 7 November 2026, every coastal
cell tried on emem.dev returned 422. Cells near Waikiki, Bondi Beach and
Nazare (from `/v1/locate`), with `n_offshore_cells: 64`, all gave the
land-locked rejection. At Bondi and Nazare the offshore end cell had 0 m depth.

This request runs, and shows the 422 rejection with its details. On
7 November 2026 its profile had 44 cells, not 64. The offshore end cell had
0 m depth, and 32 % of the cells were more than 1 m deep:

```sh
curl -s https://emem.dev/v1/wave_solve \
  -H 'content-type: application/json' \
  -d '{"coastal_cell":"defi.zb4f2.bOpA.zf12a","offshore_height_m":2,"period_s":10,"n_offshore_cells":64}'
```

## `/v1/jepa_predict`: closed-form NDVI AR(2)

| field or setting | value |
|---|---|
| `cell` (alias `place`) | required |
| `band` | `indices.ndvi`, the only band |
| `lookback_months` | default 6, 1 to 24. It counts samples, not months. |
| `forecast_horizon_months` | 1, the only value |
| coefficients | α = 0.6, β = 0.3, γ = 0.1 |

The prediction is:

    pred = clamp(α · lag_12 + β · (last + trend) + γ · recent_mean, [-1.0, 1.0])

The solver keeps the last `lookback_months` samples of the history.
`trend` is the least-squares slope through these samples. `recent_mean`
is their mean. `lag_12` is the sample nearest to 12 tslots before the
latest one, within one tslot (`physics.rs`). For `indices.ndvi` a tslot
is one day, so this is about 12 days, not 12 months. The solver looks for
it only among the kept samples. When it finds none, the α term uses
`recent_mean`. The response field `lag_12_used` shows which case
applied.

This is not a learned model. The coefficients come from the
agricultural NDVI literature, and no training changes them. The response states them in
`predictor_coefficients` and `honesty_note`. You can compute the same
value again from the `history_values` in the response. The route name
is historical.

The answer is in `forecast_value`. `history_values` and
`history_tslots` give the samples it used. `forecast_tslot` is the
latest sample tslot plus 1 (`physics.rs`). For `indices.ndvi` that is
one day later, not one month later, so do not read it as the date of
the forecast.

Example, run on emem.dev on 7 November 2026 (HTTP 200,
`lookback_months_used` 6, 6 samples, `lag_12_used` false,
`forecast_value` 0.018):

```sh
curl -s https://emem.dev/v1/jepa_predict \
  -H 'content-type: application/json' \
  -d '{"cell":"Mount Fuji"}'
```

A larger lookback keeps more samples, so the solver has more samples in
which to find `lag_12`. It uses `lag_12` only when a sample is within one
tslot of that point. At Mount Fuji on
7 November 2026, `"lookback_months": 13` still gave `lag_12_used` false.
