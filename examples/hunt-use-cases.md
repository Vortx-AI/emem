# Hunter mode: three worked examples

`POST /v1/hunt` finds candidate places for an event inside a region. It samples cells in the region, reads one input band per cell, and ranks the cells. It returns the top eight cells as hotspots, with one signed receipt that covers their fact CIDs.

`POST /v1/ask` sends a question such as `find algal bloom in Lake Erie` to the same code. Use `/v1/hunt` when you already know the event key and the region.

## What you need

- `curl` and `jq`. No key: hunts are reads.
- An event key. There are twelve. Eleven have a registered algorithm: `algal_bloom`, `deforestation`, `flood_extent`, `wildfire`, `urban_heat_island`, `methane_plume`, `landslide`, `drought`, `soil_salinity`, `crop_stress`, `water_turbidity`. The twelfth, `oil_slick`, has no algorithm (see [Limits](#limits)). The hunt also accepts some aliases, for example `fire` or `wildfire_burn_severity` for `wildfire`. For all three, the response gives `event: "wildfire_burn_severity"`. An unknown key returns `invalid_argument` with the list of the twelve keys.
- A region name, or an explicit `polygon_bbox` (`{min_lat, max_lat, min_lng, max_lng}`).

## What you get

- Up to eight hotspot cells, each with a value, a `fact_cid`, a `facts_url` and a `scene_url`. The `facts_url` link does not work with GET (see [How a hunt ranks cells](#how-a-hunt-ranks-cells)).
- The ranking rule in `ranking`: the band, the sort direction, the optional gate, and how many cells the hunt sampled.
- The algorithm key. `GET /v1/algorithms/<key>` gives its full formula.
- A signed `receipt` whose `fact_cids` list holds every hotspot's fact CID.

## How a hunt ranks cells

A hunt does not run the full algorithm. It ranks cells by the algorithm's primary input band only. When the event has a gate, the hunt first drops cells that fail the gate. The response says this in `caveats`. To get the real score, read the formula at `/v1/algorithms/<key>` and apply it to the recalled bands.

The number of sampled cells grows with the region area. A hunt asks for at least 12 cells, and the default maximum is 64. The sampler puts the same number of cells on each side of a square grid, so a small region can give fewer, for example 9. The operator can change the maximum with `EMEM_HUNT_MAX_CELLS` (12 to 128). Events whose primary band is MODIS land surface temperature sample 8 cells (`slow_band_cap`).

Each hotspot value is the first fact that recall returns for the primary band at that cell. Recall returns facts in ascending time order (`fact_order: tslot_ascending`), so this can be an older scene. Before you quote the value as current, recall the primary band at the hotspot cell and read `observed_at` on each fact:

```bash
curl -sS -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb5de.rAsI.wEfA","bands":["indices.ndvi"]}' \
  | jq -r '.facts[] | "\(.observed_at) \(.value)"'
# 7 facts on 7 November 2026, from 2026-09-14 (-0.216) to 2026-10-25 (-0.564)
```

Do not use the hotspot's `facts_url` as it is. It is a GET link, and `/v1/recall` accepts only POST. On 7 November 2026 it returned `405` with the code `method_not_allowed`.

All values below come from calls to `https://emem.dev` on 7 November 2026. When you run the commands, the hotspots and values can be different. The response shape stays the same.

## 1. Algal bloom on Lake Erie

```bash
curl -sS -X POST https://emem.dev/v1/hunt \
  -H 'content-type: application/json' \
  -d '{"event":"algal_bloom","region":"Lake Erie"}' | jq .
```

The algorithm is `algal_bloom_chlorophyll_ndci@1`. The hunt ranks cells by `indices.ndvi`, highest first. The gate keeps only cells with `indices.ndwi > 0`, which are water. Without the gate, green shoreline vegetation would rank above the lake.

The response, shortened to the first hotspot:

```json
{
  "schema": "emem.hunt.v1",
  "status": "hunter_mode",
  "event": "algal_bloom",
  "algorithm_key": "algal_bloom_chlorophyll_ndci@1",
  "algorithm_url": "https://emem.dev/v1/algorithms/algal_bloom_chlorophyll_ndci@1",
  "region_anchor": "Lake Erie",
  "ranking": {
    "primary_band": "indices.ndvi",
    "gate": { "band": "indices.ndwi", "op": ">", "threshold": 0.0, "reason": "..." },
    "direction": "descending",
    "cells_sampled": 9,
    "cells_with_facts": 9,
    "slow_band_cap": null,
    "interpretation": "..."
  },
  "hotspots": [
    {
      "cell64": "defi.zb5de.rAsI.wEfA",
      "lat": 42.04997637270753,
      "lng": -81.19967012397531,
      "primary_band": "indices.ndvi",
      "value": -0.21568627450980396,
      "gate_band": "indices.ndwi",
      "gate_value": 0.5366795366795367,
      "fact_cid": "utzsidsqpw3dbyphz4oidafddlkdu32ty5dphgbkhaeyjfnd442a",
      "facts_url": "https://emem.dev/v1/recall?cell=defi.zb5de.rAsI.wEfA",
      "scene_url": "https://emem.dev/v1/cells/defi.zb5de.rAsI.wEfA/scene.png"
    }
  ],
  "embedding_rerank": { "applied": false, "reason": "too_few_tessera_vectors", "explanation": "..." },
  "place_resolved": { "label": "Lake Erie (one of the Great Lakes in North America)", "region_resolution": "point_fallback_bbox", "...": "..." },
  "receipt": { "fact_cids": ["utzsidsq...", "..."], "signature": "...", "...": "..." }
}
```

The response also has `input_bands`, `materializer_status`, `caveats`, `recipe`, `agent_hint` and `question`.

How to read this result:

- `region_resolution: "point_fallback_bbox"` means the geocoder gave a point, not a lake outline. The point was 42.25 N, 81.00 W. The hunt then sampled a square around it, 0.60 degrees wide. Each side is 0.30 degrees from the point (`EMEM_HUNT_FALLBACK_HALF_DEG`). That is why it sampled only 9 cells, all near 42.0 to 42.5 N, 80.8 to 81.2 W.
- All eight hotspot NDVI values were between -0.333 and -0.216. The formula is `chl_a_mg_m3 = clip(70 * max(0, ndvi) - 5, 0, 200)`, so each cell scores 0 mg/m³. There was no chlorophyll signal in these facts.
- The top value came from a scene observed on 14 September 2026. The same cell had newer NDVI facts up to 25 October 2026 (see [How a hunt ranks cells](#how-a-hunt-ranks-cells)).

How to cite it: quote the hotspot's `fact_cid` and the `algorithm_key`. Quote `ranking.gate` to show that the cell passed the water test. Anyone can check the receipt at [/verify](https://emem.dev/verify) or with `POST /v1/verify_receipt`. This command runs the hunt again and sends its receipt to the check:

```bash
curl -sS -X POST https://emem.dev/v1/hunt \
  -H 'content-type: application/json' \
  -d '{"event":"algal_bloom","region":"Lake Erie"}' \
  | jq -c '{receipt}' \
  | curl -sS -X POST https://emem.dev/v1/verify_receipt \
      -H 'content-type: application/json' -d @- \
  | jq -c '{valid, signature_valid, primitive, fact_cids_count}'
# {"valid":true,"signature_valid":true,"primitive":"hunt","fact_cids_count":8}
# (7 November 2026)
```

## 2. Deforestation in Borneo

```bash
curl -sS -X POST https://emem.dev/v1/hunt \
  -H 'content-type: application/json' \
  -d '{"event":"deforestation","region":"Borneo"}' \
  | jq '{sampled: .ranking.cells_sampled, rerank: .embedding_rerank.applied, hotspots: [.hotspots[] | "\(.cell64) \(.value)"]}'
# 7 November 2026: sampled 64, rerank true, 8 hotspots,
# the second "defi.zb3d8.sulo.ze9cf -0.702048417132216"
```

The algorithm is `deforestation_alert_ndvi_drop@1`. Its formula from `/v1/algorithms`:

```text
if baseline_tc < 30: return null (not a forest cell); drop = ndvi_60d - ndvi_now; alert = drop > 0.20 AND ndvi_now < 0.5
```

The hunt ranks cells by `indices.ndvi`, lowest first, with no gate. It does not apply the Hansen 2000 tree-cover test or the 60-day drop. On 7 November 2026 it sampled 64 cells. The eight hotspot NDVI values were between -0.702 and -0.238. This hunt also applied the embedding rerank (`embedding_rerank.applied: true`, `candidates_used: 16`, `method: "cluster_centroid_cosine"`). The rerank takes the 16 cells with the lowest NDVI. It sorts them by how near each cell's stored embedding is to the mean of the stored embeddings in that group. Cells with no stored embedding go last. Thus the hotspots are not in NDVI order. The first hotspot had NDVI -0.275, and the second had -0.702.

A low NDVI alone does not prove forest loss. Check the baseline at each hotspot before you call it deforestation:

```bash
curl -sS -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb3d8.sulo.ze9cf","bands":["hansen.tree_cover_2000"]}' \
  | jq '.facts[0] | {band, value, fact_cid}'
# {"band": "hansen.tree_cover_2000", "value": 0,
#  "fact_cid": "g2pp3pf5xautw2qugfkmzbdpwqxitrx5ktxudg5hdluby7aseprq"}
```

This cell was the second hotspot (NDVI -0.702). Its tree cover in 2000 was 0 %, below the 30 % baseline. The full algorithm returns null for this cell, so it is not a deforestation alert.

How to cite it: quote the NDVI `fact_cid` from the hotspot and the `hansen.tree_cover_2000` `fact_cid` from the recall. The claim needs both facts and the algorithm key.

## 3. Crop stress in Punjab, India

Give the country in the region name. On 7 November 2026, `"Punjab"` alone resolved to Punjab, Pakistan.

```bash
curl -sS -X POST https://emem.dev/v1/hunt \
  -H 'content-type: application/json' \
  -d '{"event":"crop_stress","region":"Punjab, India"}' \
  | jq '{region: .place_resolved.label, sampled: .ranking.cells_sampled, hotspots: [.hotspots[] | "\(.cell64) \(.value)"]}'
# 7 November 2026: region "Punjab (state), India", sampled 39, 8 hotspots
```

The region resolved to `Punjab (state), India`, with a bounding box from Nominatim (`region_resolution: "polygon_bbox_from_locate"`). The hunt sampled 39 cells.

The algorithm is `crop_stress_score@1`. Its formula is a weighted sum of NDVI, NDMI, air temperature and precipitation:

```text
0.40*(1-clamp01(indices.ndvi)) + 0.35*(1-clamp01((indices.ndmi+1)/2)) + 0.15*sigmoid((weather.temperature_2m-30)/5) + 0.10*sigmoid((1.0-weather.precipitation_mm)*4)
```

The hunt ranks by `indices.ndvi` only, lowest first. The eight hotspot values were between 0.032 and 0.166. A low NDVI can mean crop stress, but it can also mean bare soil after harvest. Recall the band at the hotspot cell, as in [How a hunt ranks cells](#how-a-hunt-ranks-cells). Then compare its `observed_at` with the local crop calendar.

To look for a salinity signal at the same places, run the salinity hunt over the same region:

```bash
curl -sS -X POST https://emem.dev/v1/hunt \
  -H 'content-type: application/json' \
  -d '{"event":"soil_salinity","region":"Punjab, India"}' \
  | jq '{band: .ranking.primary_band, sampled: .ranking.cells_sampled, hotspots: [.hotspots[] | .cell64]}'
# 7 November 2026: band "s2.B04", sampled 39, 8 hotspot cells
```

This hunt ranks by red reflectance `s2.B04`, highest first. The full `soil_salinity_index@1` formula also uses B11, B12 and soil organic carbon. On 7 November 2026, three cells were in the top eight of both hunts: `defi.zb558.wUte.jaco`, `defi.zb55a.zfced.kifO` and `defi.zb55a.zfced.mEma`.

A cell in both lists is a candidate for a salinity check. It is not proof of salinity. To grade it, recall the bands that the `soil_salinity_index@1` formula needs at that cell and apply the formula.

How to cite it: quote each hotspot `fact_cid` with its algorithm key. Keep the two hunts separate in the citation.

## Limits

- The ranking uses one input band, not the full formula. Hotspots are candidates.
- The hunt samples a uniform grid. Small features can fall between samples. For denser sampling, use `POST /v1/recall_polygon` (the `recipe.fan_out` field gives the call). In that call, `cells_per_sqkm` sets the density, for example `4.0` for about 500 m between cells. `drill_on_water: true` does a coarse scan, then samples more cells near cells with surface water.
- A hunt over a large region still samples 64 cells or fewer by default. For more detail, split the region and pass each part as a `polygon_bbox`. On 7 November 2026 this bbox around Ludhiana sampled 9 cells and returned 8 hotspots:

  ```bash
  curl -sS -X POST https://emem.dev/v1/hunt \
    -H 'content-type: application/json' \
    -d '{"event":"crop_stress","polygon_bbox":{"min_lat":30.56,"max_lat":31.018,"min_lng":75.356,"max_lng":76.323}}' \
    | jq '{sampled: .ranking.cells_sampled, hotspots: (.hotspots | length)}'
  # {"sampled": 9, "hotspots": 8} (7 November 2026)
  ```
- `oil_slick` has no algorithm. The hunt returns `status: "not_yet_implemented"` and two `closest_algorithms`: `flood_extent_sar_threshold@1` (Sentinel-1 backscatter) and `water_turbidity_red_band@1`. These are not slick detectors. Use their results only as candidates.
- The embedding rerank needs Tessera vectors for at least 3 candidate cells. emem.dev no longer fetches new Tessera vectors, so the rerank uses only stored vectors. When the rerank does not run, `embedding_rerank.applied` is `false` and `reason` is one of `too_few_tessera_vectors`, `skipped_for_slow_primary_band` or `disabled_by_env`. Read this field before you say that the hunt ranked hotspots by coherence.

## See also

- Hunter case studies with screenshots: [/docs/gallery](https://emem.dev/docs/gallery)
- Algorithm registry: [/v1/algorithms](https://emem.dev/v1/algorithms)
- Agent card, with the phrases that should not start a hunt (`anti_trigger_phrases`): [/v1/agent_card](https://emem.dev/v1/agent_card)
- Receipt verification: [/verify](https://emem.dev/verify)
