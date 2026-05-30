# JEPA v2 — training-data audit (2026-05-28)

This document reports what the emem corpus actually contains for the
candidate scalar bands the v2 dynamics predictor was supposed to
condition on, and explains why the training pipeline ended up
synthetic-dominant.

## Corpus audit method

A read-only Rust binary (`audit_corpus.rs` in this directory) opens
the production sled DB at `${EMEM_DATA}/cache.sled/`, walks the
`emem.canonical_index` tree, decodes the `(cell, band, tslot)` triples,
and resolves each canonical key to its fact CID + CBOR-decoded scalar
value. We then bucket per band and report:

- total facts
- distinct cells covered
- distinct tslots covered
- per-cell tslot-depth histogram (how many cells have N distinct
  tslots, for the K=6 lag-window requirement)

The audit ran against a copy of the live DB (`/tmp/emem_sled_snapshot/cache.sled`)
to avoid contending with the running `emem-server` on the sled file
lock.

## Results — corpus snapshot 2026-05-28

`idx len = 42,556` keys across `facts len = 73,438` distinct facts.

### All bands present, counts (excerpt)

```
band                                    facts   cells  tslots
cams.no2                                  188     187      40
cams.o3                                   119     118      26
cams.pm25                                 195     194      41
era5.precip                               133     133      24
era5.t2m                                  133     133      24
indices.bsi                              1684    1684       6
indices.ndvi                             6398    5783     148
indices.ndwi                              913     913      20
modis.lst_day_8day                        277     277       7
modis.lst_night_8day                       82      82       5
modis.ndvi_mean                           285     153      71
sentinel1_raw                            1737    1231      51
weather.precipitation_mm                  292     292      20
weather.temperature_2m                    421     421      26
```

### Temporal-depth-per-cell histograms (the load-bearing number)

For each candidate band, distribution of how many distinct tslots each
cell is attested at:

```
indices.ndvi                  cells=5783  max_depth=68
  histogram: 1t->5572c 2t->5c 3t->2c 4t->27c 5t->45c 6t->5c 7t->18c 68t->1c

modis.lst_day_8day            cells= 277  max_depth= 1
  histogram: 1t->277c

modis.lst_night_8day          cells=  82  max_depth= 1
  histogram: 1t->82c

cams.pm25                     cells= 194  max_depth= 2
  histogram: 1t->193c  2t->1c

cams.no2                      cells= 187  max_depth= 2
  histogram: 1t->186c  2t->1c

cams.o3                       cells= 118  max_depth= 2
  histogram: 1t->117c  2t->1c

weather.temperature_2m        cells= 421  max_depth= 1
  histogram: 1t->421c

weather.precipitation_mm      cells= 292  max_depth= 1
  histogram: 1t->292c
```

The story is unambiguous:

- **`indices.ndvi`** is the **only** scalar band with usable temporal
  depth. 96 cells have >= 4 monthly tslots, 18 have >= 7, one has 68.
- Every other candidate (MODIS LST day/night, CAMS PM2.5/NO2/O3,
  Open-Meteo weather) has essentially **1 tslot per cell** -- the
  materializer was run point-in-time for those cells, not as a
  historical sweep.
- Tessera vintages 2017-2024 each carry ~10 cells / 2 tslots --
  same upstream sparseness the previous v0 sentinel was retired for.

## Conclusion -- training strategy

A K=6-lag predictor needs at least 7 same-cell observations of each
band to construct a single training pair. Across the four target bands
the simultaneous condition reduces to:

`min(96, 0, 0, 0) = 0` real (cell, t-window) quadruples.

The corpus alone cannot train this predictor end-to-end. Two honest
options:

1. **Train on synthetic data** generated from a documented climatology
   model, validated against the real NDVI cells that do exist. Ship a
   real trained model whose metadata is transparent about
   synthetic_fraction.
2. **Backfill the corpus first** (multi-year fetches of MODIS LST,
   Open-Meteo, CAMS for the 96 deep cells). Adds ~minutes per cell *
   96 cells per band * 4 bands -> tens of minutes of upstream calls.
   Could be done later as a follow-on PR.

This PR ships option 1, with the metadata flagging `synthetic_fraction`
~ 1.0 on training and the real-NDVI cells used as a held-out *eval*
slice (not training) to surface real-data MSE alongside synthetic-val
MSE. When option 2 lands, re-run `train_dynamics_v2.py` against the
densified corpus and the same script will preferentially use real
quadruples whenever they exist.

## Synthetic generator -- physical motivation

The synthetic series in `train_dynamics_v2.generate_synthetic_pairs()`
encode the same priors a meteorologist would write for a cell:

| band                     | model                                                                                                       | citation                              |
| ------------------------ | ----------------------------------------------------------------------------------------------------------- | ------------------------------------- |
| `indices.ndvi`           | seasonal cosine, amplitude scaled by `tanh(|lat|/30)`, hemispheric phase flip, AR(1) drift on baseline      | Tucker (1979); standard NDVI phenology |
| `modis.lst_day_8day`     | seasonal cosine in Kelvin, +/-15 K at high latitudes, +/-3 K at equator                                     | MODIS MOD11A2 climatological priors   |
| `modis.lst_night_8day`   | daytime - 8 K offset                                                                                        | typical mid-latitude diurnal range    |
| `cams.pm25`              | winter-peaked in mid-latitudes, hemispheric phase flip, lognormal-sampled per-cell baseline                 | Mahowald et al. (2018) global AQ priors |

Plus Gaussian observation noise calibrated to plausible inter-month
variance. Each generated series is a 12-month window with a random
start month, sampled at K=6 + 1 steps for one training pair.

The synthetic generator is intentionally **simple** -- it captures
seasonal cycle + latitudinal phase + per-cell baseline diversity, but
nothing else. The model should learn to read the cyclic component
from the 6-month lag-window and project one step forward; if it
beats the identity baseline on validation, that is *exactly* what
"learned non-trivial dynamics" means.

## Training outcome (final epoch, val set N=2,000)

| metric                                 | model         | identity baseline | lift            |
| -------------------------------------- | ------------- | ----------------- | --------------- |
| Gaussian NLL                           | **-1.7647**   | 0.0405            | very large      |
| NDVI MSE (physical)                    | **0.00238**   | 0.00638           | **62.7 %**      |
| training time (CPU, single core)       | **149 s**     | -                 | -               |
| ONNX size                              | **171 KB**    | was 8 KB sentinel | -               |
| param count                            | **28,328**    | was 0 trainable   | -               |
| ONNX/PyTorch parity                    | **9.5e-7**    | -                 | -               |

Real-NDVI held-out slice (34 sliding windows across 96 cells, lat=0
fallback because the `cell64 -> lat/lng` helper is not exposed to
Python): **model MSE = 0.02109, baseline MSE = 0.01986**. The model
is **slightly worse** than the identity baseline on the real-NDVI
cells. This is consistent with the synthetic generator not covering
all real-NDVI failure modes (sparse months, snow, sensor gaps). The
signed receipt always carries both numbers so verifiers can see the
gap.

## What to do next to improve

1. **Densify the corpus** -- run `/v1/backfill` for MODIS LST, Open-Meteo
   weather, CAMS air-quality on the same NDVI-deep cells.
2. **Expose `cell64 -> lat/lng` from Python** so the real-NDVI eval can
   use real coordinates instead of (0, 0). *(Done 2026-05-30 -- ported
   to `latlng_from_cell64()`; 0 cells now fall back to Null Island.)*
3. **Add a hold-one-cell-out cross-validation pass** once the real
   corpus has enough multi-tslot quadruples. *(Done 2026-05-30 -- the
   `--ndvi-only` path runs grouped cell-wise K-fold; see below.)*
4. **Replace the AR(1) synthetic with an MJO/ENSO-aware climatology**
   to cover tropical NDVI variability. Out of scope.

## NDVI-only real retrain (2026-05-30)

The refreshed audit dump (corpus snapshot 2026-05-30) is materially
deeper than the 2026-05-28 snapshot above:

```
indices.ndvi  facts=12,316  cells=7,045  tslots=173
  depth (distinct tslots/cell): 1t->5884c 2t->127c 3t->13c 4t->133c
                                5t->651c 6t->33c 7t->18c 8t->1c 9t->1c 68t->1c
```

Cells with >= 4 distinct tslots: **838** (vs ~96 before). The dump
wrote 10,373 scalar NDVI rows (1,943 skipped as non-scalar embedding
facts).

**Critical caveat about cadence.** The deep cells are *annual* peak-
season composites, NOT monthly: the median inter-observation gap is
**365 days** and the values cluster in July (N. hemisphere). Example
cell tslots: 2020-07-26, 2021-06-21, 2022-07-31, 2023-07-21,
2024-07-05, 2025-07-10, 2026-05-11. Exactly one cell (`defi.zb552...`)
has true sub-seasonal (~5-day) cadence. So the only honest real-data
model here is a **one-step-ahead ANNUAL** NDVI predictor.

`train_dynamics_v2.py --ndvi-only` trains a single-band (`n_bands=1`)
version of the same Transformer with a configurable lag K, builds real
lag->target windows directly from each cell's sorted (tslot, ndvi)
series, runs **grouped cell-wise 5-fold CV** (no cell leaks
train->eval), and computes `skill = 1 - MSE_model / MSE_persistence`
ONLY on held-out real windows.

| config                      | n_windows | model MSE | persist MSE | real skill |
| --------------------------- | --------- | --------- | ----------- | ---------- |
| K=3, real-only              | 1,671     | 0.01707   | 0.03763     | **+0.546** |
| K=3, +4k synthetic aug      | 1,671     | 0.01934   | 0.03763     | +0.486     |
| K=4, +4k synthetic aug      | 833       | 0.03235   | 0.05343     | +0.394     |

All five folds are positive in every config; the candidate shipped to
`/tmp/jepa_ndvi_retrain/` is the **K=3 real-only** run (+0.546).

**Why this beats persistence (honesty).** Annual peak-NDVI is
mean-reverting: mean within-cell variance (0.0115) is far below the
persistence error (0.0376). A trivial "predict the prior-year mean"
baseline already scores +0.31 skill vs predict-last-value. The model
captures that mean-reversion plus a little cell-specific trend, hence
+0.55. Persistence is simply a *weak* baseline on noisy annual data --
the win is real but not magic. Exact-duplicate series across cells are
negligible (58 cells / 29 pairs of 838), so the CV skill is not a
spatial-duplication leak.

The candidate is `candidate_only: true` and is NOT deployed: the live
sidecar reconstructs a hard-coded 4-band `DynamicsModel`, so loading a
1-band checkpoint requires a loader change first (see the deploy notes
in the retrain report). The shipped 4-band model continues to fail-safe
to persistence.
