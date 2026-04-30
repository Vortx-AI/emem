# Multimodal cross-sensor architecture

emem.dev is a **multi-sensor** Earth-memory protocol. Every algorithm
that claims a fine-grained delivery resolution must earn it by
anchoring on at least one S1 / S2 / Landsat input on its variance side;
coarse-physics products (SPI on POWER precip, GDD on weather t2m)
declare their honest native resolution and stay valid by symmetry.
This document describes the policy, the registry validator that
enforces it, and the cross-modality test methodology used to verify it
on the live emem.dev responder.

> **Resolution truth (read first).** Three different "resolutions"
> sit behind the multimodal block — distinguish them carefully.
>
> 1. **`data_resolution_m`** (the value's fidelity) — when the
>    multimodal block declares `delivery_resolution_m: 10` and the
>    fact's anchor band is `s2.*`, `indices.*`, or `sentinel1_raw`,
>    the materializer reads a single **real 10 m pixel** at the cell-
>    centre lat/lng (`crates/emem-api-rest/src/lib.rs:8575`,
>    `sample_pixel(...)`). The value is not interpolated, not
>    coarsened, not block-averaged. The 10 m claim is honest.
> 2. **`cell_dedupe_m`** (cache key) — emem keys its persistent fact
>    store by `cell64`, the active grid quantizes at ~305 m on the
>    latitude axis at the equator. Two queries 10 m apart land in the
>    same cell and return the **same cached 10 m-fidelity value** —
>    they do *not* fall back to a coarser aggregate. If an agent needs
>    a different 10 m sample inside the same cell, it must call
>    `/v1/recall_polygon` with a tighter polygon (or wait for the
>    H3 migration).
> 3. **Spec target grid** — aperture-7 hex DGGS at ~3.4 m edge length
>    (`docs/SPEC.md §3`). Not yet active.
>
> The `multimodal.delivery_resolution_m` field is interpretation (1):
> the sensor pitch the algorithm consumes. Authoritative numbers live
> at `/v1/grid_info`; the boring-API responses surface all three as
> `data_resolution_m`, `cell_dedupe_m`, and the live `cell64`. See
> `docs/CLIENTS.md §0a` for the empirical demonstration.

---

## 1. The principle

> Earth observation is a game of precision, resolution, and repetition.
> The protocol's commitment is to deliver answers at 10 m where physics
> allows. Sentinel-1 and Sentinel-2 are the only free, open, daily-ish
> sources that hit that grid; coarser sources (MODIS, ERA5, NASA POWER,
> SoilGrids, Hansen, ESA WorldCover) provide the climatology, prior, or
> regulatory baseline an algorithm reasons against, but they cannot
> carry the variance signal an agent's question depends on at plot
> scale.

Concretely:

- **Baseline** = the slow / climatological / prior term: per-cell
  multi-year DOY-windowed mean, depth-static raster, regulatory cut-off.
  May come from any tier (POWER, ERA5, SoilGrids, Hansen, GSW).
- **Variance** = the fast / event / current term: a current observation
  that produces the *anomaly* the algorithm reports on. Must come from
  S1, S2, or Landsat when the algorithm claims ≤10 m delivery.
- **Anchor band** = the single input that defines the algorithm's
  declared `delivery_resolution_m`. Its tier must lead the
  `priority_chain`.

## 2. Sensor tiers

The protocol orders sources by their grid pitch / revisit cadence:

| Tier | Members | Native pitch | Revisit |
|---|---|---|---|
| `S1` | `sentinel1_raw` | 10 m | 6–12 d |
| `S2` | `s2.B01..B12`, `indices.*`, `geotessera*` | 10 / 20 m | 5 d |
| `Landsat` | reserved (`landsat.*`) — not yet wired | 30 m | 16 d |
| `IoT` | reserved (`iot.*`) | per-station | per-station |
| `OtherSat` | `modis.*`, `cams.*`, `marine.*`, `viirs.*` | 250–11 000 m | 1 d–monthly |
| `Static` | `power.*`, `era5.*`, `weather.*`, `soilgrids.*`, `hansen.*`, `esa_worldcover.*`, `surface_water.*`, `copdem30m.*`, `gmrt.*`, `chirps.*`, `openet.*`, `dynamic_world.*`, `tropomi.*`, `overture.*` | varies | varies |

Tier classification is mechanical (string-prefix on the band key) and
exposed via `SourceTier::for_band()` in `crates/emem-core/src/algorithms.rs`.

## 3. The registry validator

The algorithms registry (`crates/emem-core/data/algorithms-v0.json`)
load-time validation enforces four rules on every entry that carries a
`multimodal` block:

| Rule | Constraint |
|---|---|
| **R1** | `anchor_band` MUST appear in the algorithm's `inputs[]`. |
| **R2** | `delivery_resolution_m ≤ 10` ⇒ at least one S1 / S2 / Landsat band MUST appear in `variance_sources`. |
| **R3** | `variance_sources` MUST NOT be Static-only (variance is, by definition, observational). |
| **R4** | `priority_chain[0]` MUST equal the anchor band's tier. No narrative drift between the declared anchor and the tier ordering. |

Algorithms whose variance flows through other algorithm composites set
`composite_inherit: true` and bypass R1–R4 (the validator stops at the
`<composite>` boundary; the responder walks the composing algorithms
at request time).

Failure mode: a malformed registry refuses to load — the responder
process won't start. This is deliberate. An algorithm over-claiming
10 m delivery without an S1/S2 input is a **correctness** error, not a
warning.

Test coverage:
- `multimodal_validator_rejects_overclaim` — 10 m claim with only
  SoilGrids variance → `Err`.
- `multimodal_validator_accepts_honest_coarse` — SPI on POWER precip
  declared at 55 km → `Ok`.

## 4. Fusion methods

```rust
pub enum FusionMethod {
    WeightedMean,         // flood_risk, water_consensus
    ConsensusVote,        // residue_burn_multisensor, eudr_compliance
    SequentialPriority,   // tier-demote fallback chains
    BayesianBlend,        // soc_estimate (SoilGrids prior + S2 residual)
    None,                 // single-source algorithms
}
```

When inputs span resolutions (e.g. S2 10 m + SoilGrids 250 m + cell64
~305 m), the fusion rule is: **emit at the variance-source's native
grid; treat coarser inputs as per-cell scalar priors**. The cell64
(or future H3) grid does the spatial aggregation; we do not artificially
upsample SoilGrids to 10 m or downsample S2 to 250 m. Each fact's
native resolution is recorded in the receipt; the algorithm declares
its delivery resolution honestly.

## 5. Algorithms with multimodal blocks (live as of 2026-04-30)

| Algorithm | Anchor | Variance sources | Delivery |
|---|---|---|---|
| `soil_moisture_sar@1` | `sentinel1_raw` (S1) | sentinel1_raw + indices.ndvi + weather.precipitation_mm | **10 m** |
| `soc_estimate_s2_dem@1` | `s2.B12` (S2) | s2.B11/B12 + indices.bsi/ndmi | **10 m** |
| `soc_change_credit@1` | `<composite>` | composite (S2-anchored) | **10 m** |
| `typed_stress_attribution@1` | `indices.ndvi` (S2) | indices.ndvi/ndmi/ndre + weather.t2m | **10 m** |
| `n_uptake_ndre@1` | `indices.ndre` (S2) | indices.ndre + indices.ndvi | **10 m** |
| `eudr_compliance@1` | `indices.ndvi` (S2) | indices.ndvi + sentinel1_raw | **10 m** |
| `residue_burn_multisensor@1` | `indices.nbr` (S2) | indices.nbr + indices.ndti | **10 m** |
| `rice_methane_awd_modeled@1` | `sentinel1_raw` (S1) | indices.ndwi + sentinel1_raw + indices.ndvi | **10 m** |
| `rusle_soil_erosion@1` | `indices.bsi` (S2) | indices.bsi + sentinel1_raw | **10 m** |
| `soil_salinity_index@1` | `s2.B11` (S2) | s2.B04/B11/B12 + indices.bsi | **10 m** |
| `sowing_date_detection@1` | `indices.ndvi` (S2) | indices.ndvi + sentinel1_raw | **10 m** |
| `harvest_date_detection@1` | `indices.ndvi` (S2) | indices.ndvi + indices.ndti + s2.B11 + sentinel1_raw | **10 m** |
| `crop_loss_event_assessment@1` | `indices.ndvi` (S2) | indices.{ndvi,nbr,ndwi} + sentinel1_raw | **10 m** |
| `land_degradation_trend@1` | `indices.evi` (S2) | indices.evi + indices.bsi + hansen.loss_year | **10 m** |
| `parametric_trigger@1` | `<composite>` | composite + weather + era5.precip | inherited |
| `gdd_phenology@1` | `weather.temperature_2m` (Static) | weather.temperature_2m | **25 km** (honest coarse) |
| `spi_meteorological_drought@1` | `weather.precipitation_mm` (Static) | weather.precipitation_mm | **5.5 km** (honest coarse) |

## 6. Cross-modality test methodology

The verification suite checks four orthogonal things on the live
emem.dev responder. All probes use the published REST and MCP surfaces
— there is no privileged path.

### 6.1 Sensor-tier sweep

Recall a representative band from each modality at a known agri cell
(Mansa, Punjab — `damO.zb000.wabo.zd9ad`) in parallel batches sized
≤8 to fit inside the gateway timeout.

```bash
python3 - <<'PY'
import json, subprocess, concurrent.futures
CELL = "damO.zb000.wabo.zd9ad"
TIERS = {
    "S1":       ["sentinel1_raw"],
    "S2-refl":  ["s2.B04","s2.B11","s2.B12","s2.B8A"],
    "S2-idx":   ["indices.ndvi","indices.ndre","indices.ndti","indices.bsi","indices.ndmi","indices.nbr","indices.evi"],
    "MODIS":    ["modis.lst_day_8day","modis.lai_8day","modis.gpp_8day","modis.et_8day"],
    "POWER":    ["power.t2m","power.t2m_max","power.precip","power.rh2m"],
    "ERA5":     ["era5.t2m","era5.precip","era5.rh2m"],
    "CAMS":     ["cams.pm25","cams.aod_550","cams.no2"],
    "Static":   ["copdem30m.elevation_mean","hansen.tree_cover_2000","hansen.loss_year","esa_worldcover.lc_2021","surface_water.recurrence"],
    "Tessera":  ["geotessera","geotessera.bin128"],
    "SoilGrid": ["soilgrids.soc_0_30cm","soilgrids.phh2o_0_30cm","soilgrids.clay_0_30cm","soilgrids.sand_0_30cm","soilgrids.bdod_0_30cm","soilgrids.nitrogen_0_30cm"],
    "Overture": ["overture.transportation.road_length_m","overture.places.count","overture.buildings.count"],
}
def recall(tier, bands):
    body = json.dumps({"cell": CELL, "bands": bands})
    r = subprocess.run(["curl","-sS","-X","POST","https://emem.dev/v1/recall",
        "-H","content-type: application/json","--max-time","60","-d",body],
        capture_output=True, text=True)
    return tier, json.loads(r.stdout)
with concurrent.futures.ThreadPoolExecutor(max_workers=8) as ex:
    for tier, resp in ex.map(lambda kv: recall(*kv), TIERS.items()):
        ok = sum(1 for f in resp.get("facts",[]) if "value" in f)
        print(f"{tier:<10} {ok}/{len(TIERS[tier])} primary")
PY
```

Pass criterion: every band returns a Primary fact OR a citable Absence
(signed `reason_cid`). A skipped band with `no_auto_materializer_registered`
is a fail.

### 6.2 Cross-modal coherence

Three sensors measuring the same physical quantity should agree:

| Quantity | Source A | Source B | Source C | Coherence test |
|---|---|---|---|---|
| Surface temperature | weather.temperature_2m (met.no nowcast) | era5.t2m (ECMWF) | modis.lst_day_8day (MOD11A2) | within ±5 K (sensor type, time-of-pass differ) |
| Precipitation | weather.precipitation_mm | era5.precip | power.precip | within ±2 mm/d on dry days, monsoon outliers expected |
| Land cover | esa_worldcover.lc_2021 | hansen.tree_cover_2000 | indices.ndvi current | crop class ≠ forest, NDVI bounds match |

Live test on Mansa, Punjab returned 28.5 °C / 31.1 °C / 29.3 °C across
MODIS / ERA5 / met.no — within tolerance.

### 6.3 Algorithm composition

For each algorithm with a `multimodal` block, fetch its declared
concrete-band inputs in one recall and verify all return Primary:

```bash
ALG=soc_estimate_s2_dem@1
INPUTS=$(curl -sS https://emem.dev/v1/algorithms | python3 -c "
import json,sys
d=json.load(sys.stdin)
a=next(a for a in d['algorithms'] if a['key']=='$ALG')
print(','.join(i['band'] for i in a['inputs'] if i.get('band') and not i['band'].startswith('<')))")
curl -sS -X POST https://emem.dev/v1/recall -H 'content-type: application/json' \
  -d "{\"cell\":\"damO.zb000.wabo.zd9ad\",\"bands\":[\"${INPUTS//,/\",\"}\"]}" | python3 -m json.tool
```

Pass criterion: 100 % primary on the agri reference cell. Live verified
on 2026-04-30 — 13/13 algorithms return all inputs as Primary.

### 6.4 Cross-cell graceful degradation

Repeat the eight-band probe (S1 / S2-idx / MODIS / POWER / SoilGrids /
DEM / WorldCover / CAMS) at three cells with different physical
properties:

| Cell | Expected behaviour |
|---|---|
| Mansa, Punjab (rural agri) | All Primary |
| Hyderabad city centre | SoilGrids Absence (urban-mask null), MODIS LAI Absence, indices.ndvi negative (built-up), WorldCover class 50, S1 may be missing if no recent IW |
| Mumbai coastal | DEM ≈ sea-level, WorldCover 50, SoilGrids Absence, S1 elevated (urban backscatter) |

Pass criterion: every Absence carries a citable signed `reason_cid`;
no transport errors disguised as zeros.

### 6.5 Topic routing

Twelve representative natural-language questions must each route to the
correct topic and surface the correct algorithm:

```bash
for Q in "soil organic carbon at this plot" "EUDR compliance for this plot" \
         "PMFBY claim assessment" "nitrogen uptake of the crop"; do
  curl -sS -X POST https://emem.dev/v1/ask -H 'content-type: application/json' \
    -d "{\"q\":\"$Q\",\"cell\":\"damO.zb000.wabo.zd9ad\"}" | python3 -c "
import json,sys
d=json.load(sys.stdin)
print(f\"{d['question'][:50]:<50} → {d['topic_routing']['matched_topics']}\")"
done
```

Pass criterion: 12/12 route correctly. Verified on 2026-04-30.

### 6.6 MCP surface

```bash
curl -sS -X POST https://emem.dev/mcp \
  -H 'content-type: application/json' \
  -H 'mcp-protocol-version: 2025-06-18' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
```

Pass criterion: all 28 tools listed with strict-mode-clean input
schemas (no top-level `anyOf`, no array `type`). `tools/call emem_recall`
must return cross-modal facts identical to the REST `/v1/recall`
endpoint.

## 7. Timeouts (post-doubling, 2026-04-30)

| Setting | Default | Tunable via | Rationale |
|---|---|---|---|
| Gateway request timeout | **60 s** | `EMEM_TIMEOUT_SECS` | doubled 30 → 60 — multimodal recalls trigger 8–12 parallel materializers |
| Per-materializer fetch timeout | **30 s** | `EMEM_MATERIALIZER_TIMEOUT_SECS` | doubled 15 → 30 — SoilGrids REST + ORNL DAAC + STAC PC p95 ~12 s on cold paths |
| Generic JSON-API client | **16 s** | (compile-time) | doubled 8 → 16 |
| STAC + COG client | **90 s** | (compile-time) | doubled 45 → 90 — multi-step COG path needs headroom for retries |
| Redeploy smoke-check window | **20 s** | (compile-time) | doubled 10 → 20 — cold ACME challenge can take 15–30 s |

## 8. References

- emem source-tier ordering: `crates/emem-core/src/algorithms.rs` (`SourceTier`)
- Validator rules R1–R4: `AlgorithmRegistry::validate` in same file
- Multimodal block schema: `pub struct Multimodal` in same file
- Topic routing dispatch: `TOPIC_BANDS` / `TOPIC_ALGORITHMS` in `crates/emem-api-rest/src/lib.rs`
- Live algorithms catalog: `https://emem.dev/v1/algorithms`
- Live source registry: `https://emem.dev/v1/sources`
- Wagner et al. 1999 — SAR change-detection paradigm for soil moisture
- Bauer-Marschallinger et al. 2018 — Sentinel-1 surface SM at 1 km
- Borrelli et al. 2017 — dynamic-C RUSLE2 with S2 BSI
- Bouvet et al. 2018 — SAR forest-clearing signature for EUDR
