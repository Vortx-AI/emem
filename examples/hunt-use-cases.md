# Hunter mode — three worked examples

`/v1/hunt` answers open-world event-discovery questions. The classifier in `/v1/ask` routes free-text `find <event> in <region>` to the same path; structured callers hit the endpoint directly with the event keyword and a region. Twelve events ship today. Each maps to one registered detection algorithm; the responder samples cells from the region polygon, recalls the algorithm's primary scalar plus any configured gate band, and returns the top eight hotspots with signed receipts.

The three walks below are live against `https://emem.dev`. Every value, cell, and fact CID below was produced by an actual call. If you copy the curl, the numbers will move (which cells rank in the top eight shifts with the cache and the time of year), but the response shape and the receipts will hold.

## 1. Algal bloom on Lake Erie

```bash
curl -sS -X POST https://emem.dev/v1/hunt \
  -H 'content-type: application/json' \
  -d '{"event":"algal_bloom","region":"Lake Erie"}'
```

The algorithm is `algal_bloom_chlorophyll_ndci@1`. NDVI residual over a water-gated cell is the chlorophyll-a proxy; > 30 mg/m³ is bloom-level. The water gate keeps shoreline vegetation out of the ranking — without it, NDVI 0.8 grass-on-the-coast would outrank actual blooms.

What the agent receives:

```json
{
  "status": "hunter_mode",
  "event": "algal_bloom",
  "algorithm_key": "algal_bloom_chlorophyll_ndci@1",
  "algorithm_url": "https://emem.dev/v1/algorithms/algal_bloom_chlorophyll_ndci@1",
  "region_anchor": "Lake Erie",
  "ranking": {
    "primary_band": "indices.ndvi",
    "gate": { "band": "indices.ndwi", "op": ">", "threshold": 0.0 },
    "direction": "descending",
    "cells_with_facts": 11,
    "slow_band_cap": null
  },
  "hotspots": [
    {
      "cell64":   "defi.zb5dc.bObe.wOgI",
      "lat": 41.838, "lng": -81.163,
      "primary_band": "indices.ndvi", "value": -0.057,
      "gate_band": "indices.ndwi",    "gate_value": 0.141,
      "fact_cid": "tc54vlpl62a4d2kah5rwcmfvd3eo7xqkszmkpwxk4xrkpaeeshaa",
      "scene_url": "https://emem.dev/v1/cells/defi.zb5dc.bObe.wOgI/scene.png"
    }
  ],
  "embedding_rerank": { "applied": false, "reason": "too_few_tessera_vectors" },
  "materializer_status": [
    { "band": "indices.ndvi", "has_live_materializer": true,  "note": "..." },
    { "band": "indices.ndwi", "has_live_materializer": true,  "note": "..." }
  ]
}
```

How to interpret the top hotspot. NDVI = -0.057 over a water-classed cell (NDWI = 0.141 > 0) means there is no active chlorophyll signal at this cell today. The full algorithm rescales `max(0, ndvi)` into mg/m³ — at this NDVI the chlorophyll proxy floors at 0. That is the honest May 2026 answer: Erie's cyanobacteria window opens in late July. The same call in August will return positive NDVI values and the proxy will climb past the 30 mg/m³ bloom threshold.

How to cite. Quote the `fact_cid` directly so the user can paste it into [/verify](https://emem.dev/verify) and recompute the signature in their browser. Quote `algorithm_key` so the user can read the formula at `/v1/algorithms/<key>`. Quote `ranking.gate` so the user knows the cell was confirmed over water before being ranked.

## 2. Deforestation in Borneo

```bash
curl -sS -X POST https://emem.dev/v1/hunt \
  -H 'content-type: application/json' \
  -d '{"event":"deforestation","region":"Borneo"}'
```

The algorithm is `deforestation_alert_ndvi_drop@1`. A 60-day NDVI drop greater than 0.20 on a cell whose Hansen 2000 tree-cover baseline is at least 30 % qualifies. The hunter ranks by lowest NDVI; the algorithm's formal scoring layers the Hansen gate on top. Cells with negative NDVI are bare ground or active clearing where canopy used to sit.

What to look for in the response. The top eight hotspots cluster near established palm-oil concession edges in Sabah and West Kalimantan — well-known deforestation fronts. NDVI values run from −0.55 down to slightly negative; that range corresponds to bare-soil reflectance in the red and near-infrared.

How to cite. The receipt's `fact_cid` references the NDVI scalar at the hotspot cell. To prove the cell was *forested* in 2000 (and therefore the loss is genuine, not pre-existing bare ground), call `/v1/recall` with `bands: ["hansen.tree_cover_2000"]` at the same cell and quote that fact CID alongside. Two signed facts together carry the full algorithmic claim.

## 3. Crop stress in the Punjab

```bash
curl -sS -X POST https://emem.dev/v1/hunt \
  -H 'content-type: application/json' \
  -d '{"event":"crop_stress","region":"Punjab"}'
```

The algorithm is `crop_stress_score@1`. NDVI lower than expected in the growing season is the primary signal; the formal algorithm z-scores against a per-polygon rolling baseline. The hunter ranks by lowest NDVI — the cheapest available stress proxy without the baseline.

What to look for. Growing-season cells with NDVI below 0.4 (over what should be irrigated cropland) usually indicate water stress, salinity, or late sowing. Cross-check with the soil-moisture or salinity hunters at the same cell:

```bash
curl -sS -X POST https://emem.dev/v1/hunt \
  -H 'content-type: application/json' \
  -d '{"event":"soil_salinity","region":"Punjab"}'
```

If the same cell ranks in both the `crop_stress` and `soil_salinity` top-N, the stress is salinity-driven; if only in `crop_stress`, water management is the more likely cause.

How to cite. Surface both `fact_cid`s and the algorithm key for each. The composite claim ("this field is salinity-stressed") is two signed facts plus the algorithm reference — exactly the trust contract an extension officer or insurer needs to act on.

## Honest limits in hunter mode

The responder caps fan-out at 32 cells per region (8 for slow primary bands like MODIS LST). For larger sweeps, split the region into sub-polygons and call once per sub-polygon, or use `polygon_bbox` directly to anchor on coordinates you already have.

`oil_slick` has no algorithm in the registry yet. The responder returns `status: not_yet_implemented` with pointers at `flood_extent_sar_threshold@1` (SAR darkening — same physics) and `water_turbidity_red_band@1` (red-band sediment for impacted coastline). Do not fabricate slick detections from these proxies; surface them as candidate-generators only.

When the Tessera upstream is unreachable, the embedding-coherence rerank gracefully degrades to primary-scalar order. The response carries `embedding_rerank.applied: false` plus a `reason` field — `too_few_tessera_vectors` (no upstream), `skipped_for_slow_primary_band` (LST family), or `disabled_by_env`. Always check that field before claiming a coherence-ranked answer.

## See also

- Hunter case studies (live, with screenshots): [/docs/gallery](https://emem.dev/docs/gallery)
- Algorithm registry: [/v1/algorithms](https://emem.dev/v1/algorithms)
- Anti-triggers (what *not* to hunt for): [/v1/agent_card](https://emem.dev/v1/agent_card)
- Receipt verification: [/verify](https://emem.dev/verify)
