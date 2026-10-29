# What the research found missing in emem

*Gap ledger, opened 29 October 2026. Companion to [the research program](program.md) and
[the canopy-loss paper](canopy-loss-2026-10.md).*

Every experiment the methods lab runs tests emem as well as a method: its sensors, its readings, its references and
its own records. This ledger keeps what the experiments showed emem cannot yet do, with the evidence, the cost to an
agent, and the state of the fix. A gap closes when the fix ships and an experiment has used it.

## Readings and sensors

| gap | evidence | cost to an agent | state |
|---|---|---|---|
| `field_series` keeps pixels Sen2Cor classes as snow (SCL 11) unless the caller sends a `mask_policy` | At Västerbotten every detector flagged much of the stable forest. The snow-mask experiment (`w54isezafoxudzsqffpcbazcbu`) found rejecting SCL 11 cuts every detector's false alarms in snowy forest by a fifth to a half, but finds no more loss: MCC moved for three of 15 detectors | A snowy canopy reads as loss; no rule tested has skill in snowy boreal forest at 400 m | Tested; the default stays. The open gap is a detector that works in snow: radar, or a snow-aware memory |
| Radar area responses lacked what optical ones carry: no `blocks` map, `change_map` or `anomaly` for VV, VH or VH−VV | Read from `field_series.rs`; radar per-cell series did exist and serve the fusion detector | An agent could read radar over an area as one series, not as a map of where it changed | Fixed on main (`bb0d2e4`), awaiting deploy: block medians in dB with pixel counts, a same-season change map and an anomaly on one orbit |
| No radar raster: `band_raster` serves Sentinel-2 bands and forest references, not Sentinel-1 | `band_raster.rs` has no Sentinel-1 path | No signed `emem:raster` of backscatter for a world model or a change picture | Open |
| No snow index among `field_series` indices (NDVI, NDWI, NDMI, EVI, NBR, NDTI) | NDSI needs B03 (10 m) and B11 (20 m) on one pixel grid, which the index table does not mix today | A snow screen must rely on Sen2Cor's class | Open |
| No Landsat, HLS or MODIS per-cell series; one served text even claimed algorithms on "S1/S2/Landsat" read 10 m inputs | `field_series` read Sentinel-2 L2A and Sentinel-1 RTC only, and no Landsat surface-reflectance reader existed | A cell's memory started in 2019 here, and fusion had two sensors, not three | Landsat fixed on main, awaiting deploy: `sensor: "landsat"` reads Landsat 8 and 9 Collection 2 Level-2 surface reflectance (Tier 1, 30 m, from 2013), signed as `landsat_index_series@1`; the 10 m claim corrected. HLS and MODIS remain open |
| Grids finer than 200 m do not fit one call: 500 fields and 12,000 field-scenes per request | 144 cells get 83 scenes, 180 get 66 (the response says so in `cautions`); a 4.8 km box at 200 m is 576 cells | The cell-size result (paper, section 5.10) says finer reading finds more of the loss; agents cannot read it at scale | Open. `band_cube` serves per-pixel stacks over time and may be the route to per-pixel detection |
| Three quantiles per cell and pass (p10, median, p90) | The green-fraction rule interpolates a pixel share from three points | Sub-pixel readings are coarse | Open: a share-above-a-line statistic, or more quantiles, in `fields` rows |

## References

| gap | evidence | cost to an agent | state |
|---|---|---|---|
| No reference for 2024 and 2025 except Hansen, and none finer than a year: JRC TMF ends in 2023, and RADD and OPERA DIST-ALERT are registered but cannot be read | RADD's raster is in a Requester-Pays bucket with no public mirror; OPERA DIST needs a NASA Earthdata token on the server and its fetch is staged as not implemented (`radd_alerts.rs`, `opera_dist.rs`) | The 2024-2025 results rest on one reference, and detection dates can only be checked to the year | Open: needs an operator action, a free Earthdata token for OPERA DIST or AWS Requester-Pays credentials for RADD |
| Hansen is the only reference the experiments scored against, and it agrees with JRC TMF at an MCC of only 0.39 to 0.48 on the same cells | Reference comparison on 4,391 tropical forest cells, 2021 to 2023 (paper, section 6.5) | No method can be verified above that level against either; claims of near-perfect accuracy are unverifiable | Open, and the largest: a probability-sample reference checked on very-high-resolution imagery (Olofsson et al. 2014), held in emem memory |

## Algorithms and the registry

| gap | evidence | cost to an agent | state |
|---|---|---|---|
| None of the registry's 169 entries carries a measured accuracy | Registry audit (program, section 1) | An agent picks algorithms by name | Partly closed: the catalogue (`research/algorithms/index.md`) gives the two canopy rules' measured scores in every experiment. A registry field citing evaluation records is the operator's change |
| `spi_meteorological_drought@1` cannot run: `weather.precipitation_mm` holds the current hour | Registry audit | The drought index in the registry returns nothing | Partly closed: `/v1/rain_series` computes the standardized index from 30 years of ERA5; the registry entry still points at the band |
| 44 entries do not run on emem.dev: 36 read a retired encoder or band, 8 cannot run (2 name a route that is gone; 4 read a band no materializer serves: `overture.buildings.area_m2` and `.height_mean`, `esa_cci_biomass.agb_t_per_ha_2020`, `soilgrids.v2.soc`; 2 score crops against prototype vectors that no file on the responder holds) | Registry audit, then a gate over every input name. The audit's 41 counted `jepa_temporal_predictor@1` by its name, but it is closed-form and runs at `/v1/jepa_predict` (its entry named `/v1/jepa_predict_v2`, which answers 404; fixed). `crop_water_stress_index_proxy@1` read `weather.relative_humidity`, which recall answers as an unknown band; it now reads `weather.relative_humidity_2m` | Recipes that cannot run were listed beside ones that can | Partly closed: every entry carries `here.status` (`runs`, `stored_only`, `cannot_run`) on this deployment, and `?runnable=true` lists only those that run (128 of 172). 25 name a served recipe to use instead: 13 an existing recipe, 12 the new `memory_similarity@1`, `memory_shift@1` and `memory_novelty@1`, which compare fields by their own seasonal fit (`/v1/field_series` with `memory: true`: mean, drift, annual harmonics, residual scale, spread of whole years) where the retired encoders compared learned vectors. No accuracy has been measured for them. 19 still have no served counterpart |
| The registry's canopy rules are weak where they are offered | `deforestation_alert_ndvi_drop@1` flagged 43 to 74% of stable forest in six experiments; MCC 0.051 to 0.338 | An agent using it raises alarms on most of the forest it watches | Three replicated replacements are proposed; registering one is the operator's change |

## Bugs and weak maths in the server

A read-only audit of the server's statistical code on 29 October, prompted by what the experiments found. "Checked"
means the finding was traced in the code again before it was written here; the others are as the audit reported
them, with file and line, and are confirmed before they are fixed.

| finding | where | an agent is misled when | state |
|---|---|---|---|
| A trace of rain in a dry season was classed extremely wet, and the tails counted fifteen overlapping totals a year as independent | `rain_series.rs` | 0.1 mm forecast against a sample of 445 zero totals in 450 read p = 0.988, "extremely wet"; a total beyond the record read as one in 900 rather than one in 31 | Checked. Fixed in `rain_series@2` (trace under 1 mm, no class when most totals are rainless, tails bounded by years of record); awaiting deploy |
| `emem_spi` buckets its history by epoch-aligned slots: the current bucket is partial and every season shares one gamma fit | `eo_runtime.rs` | ten days of rain ranked against full 90-day sums; a dry season reads as extreme drought against monsoon sums | Checked. Fixed on a branch, awaiting build and deploy: only complete windows count, never the partial current one, and under 30 of them the route answers inconclusive and points to `/v1/rain_series` |
| The anomaly and the same-season pairing take "an earlier year" to be an earlier calendar year | `field_series.rs` | a scene from 20 December is "last year's same season" for one of 15 January | Checked. Fixed on a branch, awaiting build and deploy: at least 335 days back and within 30 days of the day of year |
| Trend p-values come from a Kendall test that assumes independent, non-seasonal data, on autocorrelated seasonal medians, with no multiplicity control across fields | `field_series.rs` | a 16-month window from trough to peak reads as a significant trend; 500 fields give about 25 false trends at 5% | Fixed on a branch, awaiting build and deploy: Hamed-Rao variance correction, the p-value withheld when a partial year is over a tenth of the window, and Benjamini-Hochberg q-values across fields. A seasonal Kendall test is still open |
| The radar point-target cut assumes one speckle class, so on a field of two covers the brighter one is cut | `field_series.rs` | 55% water and 45% rice reads about -19 dB instead of -11 dB, and jumps as the water share crosses half | Open |
| The radar catalogue search stops at 400 items newest first and drops the completeness flag | `field_series.rs` | a long multi-orbit window silently loses its oldest years | Open (the radar parity change names the limit in its caution but does not raise it) |
| EUDR visual evidence: single-pixel NDVI anywhere from April to September and the largest drop over all years; single-pixel VV on any orbit against 3 dB | `lib.rs` | a seasonal shift alone can mark a plot suspected; speckle alone gives about 1.4 dB per difference | Open |
| The deforestation-alert "baseline" is the latest MODIS composite and "now" the latest Sentinel-2 pixel | `deforestation_alert.rs` | a difference across sensors, scales and dates reads as a change | Open |
| `jepa_predict` trims history before its seasonal lag lookup, so the seasonal term never applies by default | `physics.rs` | a forecast called seasonal is not | Open |
| The scene-dip check runs on water indices, where a one-pass flood is the signal | `field_series.rs` | a flood date is removed as "fog" | Open |
| `compare_same_doy` interpolates across any gap; `compare` mixes dates per band | `lib.rs`, `compare.rs` | a monsoon gap is interpolated through the crop peak; Tokyo 2026 is compared with Mumbai 2024 | Open |
| Heavy multi-year, multi-index reads time out or fail upstream often enough to cost an experiment its power: four of ten forests in the memory-fusion experiment | 502 and 504 on `field_series` and `band_raster` (paper, section 5.13) | An experiment, or an agent's large read, loses sites | Mostly the lab's: the server already runs any tool as a task (`/v1/a2a/tasks`) bounded by its ttl, and its 504 body says to use a task, but the lab neither used tasks nor read error bodies. Fixed in the lab (task reads, logged bodies, chunked windows). Open on the server: the REST 504 names the MCP task parameter but not `/v1/a2a/tasks` |
| The COG reader refused LZW blocks whose data was complete but whose stream ended without the end code | Hansen GFC v1.13 `treecover2000_00N_060W` row 30326 decodes all 40,000 bytes with no end code; libtiff accepts it, weezl's one-shot decode did not. Every window touching such a row failed, deterministically, as a 502 (Apuí, twice) | Forests are lost from experiments and agents get a 502 for a readable tile | Fixed in `cog.rs` (`decode_lzw` streams to the expected size, as libtiff does), awaiting deploy |
| `band_raster` reads one publisher tile per call and refuses a bbox that crosses a 10-degree tile line | A forest on the equator (Equateur, DR Congo) failed twice with 400 `bbox crosses an edge`; the server says to page the bbox | Any box near a multiple of 10 degrees of latitude or longitude needs several calls and a mosaic the caller writes | Fixed in the lab (pieces joined on the layer's lattice), and in `band_raster.rs`: a bbox across a tile line is read from each tile it touches, up to four, joined on the layer's own lattice without resampling and signed once, with every tile cited; awaiting deploy |
| A rate limit from Planetary Computer's catalogue fails the read outright: the server cools down (`stac cooling down 36s after a rate limit from this host`) and a task in that window fails | Humaitá was lost from the memory-fusion replication this way | A long experiment, or any agent's task, loses a site to a pause of seconds | Fixed in the lab (a failed task is resubmitted after a growing wait). Open on the server: a task could wait out its own cooldown instead of failing |
| Smaller: change-map blocks can rest on one or two pixels with no count; p10 to p90 described as "uncertainty bands"; scenes are spread by rank, not evenly in time as the record says; every derivative signs confidence 1.0; REST failures are not logged | various | each misstates a little | Open |

Checked since: Sentinel-1 RTC COGs on Planetary Computer are PixelIsArea (GTRasterTypeGeoKey 1), so the radar lattice is not offset; Landsat's are PixelIsPoint and the Landsat path corrects for it. Of the other products read through COG windows, Hansen GFC, JRC TMF, JRC GFC2020 and Sentinel-2 L2A are PixelIsArea; the Copernicus GLO-30 DEM is PixelIsPoint and its reader already treats the tiepoint as a pixel centre.

Found sound in the same audit: the gamma and probit implementations, the Theil-Sen estimator, the quantile rule, the
radar statistics (mean on linear power, the standard-error and look formulas, the 4.12 point-target threshold), the
Sentinel-2 processing-baseline offset handling, one CRS and one orbit per series, and the cache keys.

## The lab's own record

These were found and fixed in the lab, and are listed so the record shows them.

| gap | fix |
|---|---|
| Experiment records stored MCC intervals under `f1_minus_baseline_95`, with no score named | Records name `score`; the paper discloses the 29 October records |
| A rerun on the same sites could count as a replication | Experiments run once; replication needs sites no earlier row scored |
| Losses under the deciding score before a first win were excused | They count; such a method reads mixed |
| The desk's rules were never ledgered | Every non-registry method is ledgered and proposed |
| The plan template said the search picks by F1 when it picked by MCC | The template names the registered score |
| The lab never read the server's `cautions`, so a capped scene count went unnoticed | Result notes list every caution the server returned |
