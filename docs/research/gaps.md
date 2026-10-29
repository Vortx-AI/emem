# What the research found missing in emem

*Gap ledger, opened 29 October 2026. Companion to [the research program](program.md) and
[the canopy-loss paper](canopy-loss-2026-10.md).*

Every experiment the methods lab runs tests emem as well as a method: its sensors, its readings, its references and
its own records. This ledger keeps what the experiments showed emem cannot yet do, with the evidence, the cost to an
agent, and the state of the fix. A gap closes when the fix ships and an experiment has used it.

## Readings and sensors

| gap | evidence | cost to an agent | state |
|---|---|---|---|
| `field_series` keeps pixels Sen2Cor classes as snow (SCL 11) unless the caller sends a `mask_policy` | At Västerbotten every detector flagged much of the stable forest: the registry rule 80%, the seasonal NBR rule 28% (paper, section 5.9) | A snowy canopy reads as canopy loss to every rule that reads NDVI or NBR | Snow-mask experiment registered (`w54isezafoxudzsqffpcbazcbu`), running. If the mask helps, the default or a named preset is the operator's change, since it alters every caller's series |
| Radar area responses lack what optical ones carry: no `blocks` map, `change_map` or `anomaly` for VV, VH or VH−VV | Read from `field_series.rs`; radar per-cell series do exist and serve the fusion detector | An agent can read radar over an area as one series, not as a map of where it changed | In progress on a branch |
| No radar raster: `band_raster` serves Sentinel-2 bands and forest references, not Sentinel-1 | `band_raster.rs` has no Sentinel-1 path | No signed `emem:raster` of backscatter for a world model or a change picture | Open |
| No snow index among `field_series` indices (NDVI, NDWI, NDMI, EVI, NBR, NDTI) | NDSI needs B03 (10 m) and B11 (20 m) on one pixel grid, which the index table does not mix today | A snow screen must rely on Sen2Cor's class | Open |
| No Landsat, HLS or MODIS per-cell series | `field_series` reads Sentinel-2 L2A and Sentinel-1 RTC only | A cell's memory starts in 2017 at best, and fusion has two sensors, not three or four | Open |
| Grids finer than 200 m do not fit one call: 500 fields and 12,000 field-scenes per request | 144 cells get 83 scenes, 180 get 66 (the response says so in `cautions`); a 4.8 km box at 200 m is 576 cells | The cell-size result (paper, section 5.10) says finer reading finds more of the loss; agents cannot read it at scale | Open. `band_cube` serves per-pixel stacks over time and may be the route to per-pixel detection |
| Three quantiles per cell and pass (p10, median, p90) | The green-fraction rule interpolates a pixel share from three points | Sub-pixel readings are coarse | Open: a share-above-a-line statistic, or more quantiles, in `fields` rows |

## References

| gap | evidence | cost to an agent | state |
|---|---|---|---|
| Only annual loss references come as rasters (Hansen loss year, JRC TMF deforestation year) | RADD and OPERA DIST-ALERT are point bands; `band_raster` serves neither | Detection dates can only be checked to the year | Open |
| Hansen is the only reference the experiments used | JRC TMF is reachable as a raster and was not used | Every result so far depends on one product's labels | Open: register a canopy experiment scored against both |

## Algorithms and the registry

| gap | evidence | cost to an agent | state |
|---|---|---|---|
| None of the registry's 169 entries carries a measured accuracy | Registry audit (program, section 1) | An agent picks algorithms by name | Partly closed: the catalogue (`research/algorithms/index.md`) gives the two canopy rules' measured scores in every experiment. A registry field citing evaluation records is the operator's change |
| `spi_meteorological_drought@1` cannot run: `weather.precipitation_mm` holds the current hour | Registry audit | The drought index in the registry returns nothing | Partly closed: `/v1/rain_series` computes the standardized index from 30 years of ERA5; the registry entry still points at the band |
| 41 entries rest on retired encoders or bands | Registry audit | Recipes that cannot run are listed beside ones that can | Open |
| The registry's canopy rules are weak where they are offered | `deforestation_alert_ndvi_drop@1` flagged 43 to 74% of stable forest in six experiments; MCC 0.051 to 0.338 | An agent using it raises alarms on most of the forest it watches | Three replicated replacements are proposed; registering one is the operator's change |

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
