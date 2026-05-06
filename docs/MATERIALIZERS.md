# Materializers — what fact-bands the responder will auto-fetch

> "Materialization" is the **read → attest → cache** loop. When an
> agent calls `/v1/recall` for a band on a cell with no fact yet, and
> the responder knows how to compute that band, it fetches the
> upstream value, signs a Primary or Absence fact under its identity,
> persists it, and returns it. The next call hits the hot cache.
>
> This document is the operator's view: which bands ship today, what
> their upstream is, and what the queue looks like.

The wire-stable list is at `GET /v1/materializers` (also indexed in
`sitemap.xml`). This file is the more discursive companion.

---

## Shipped (this branch)

### `modis.ndvi_mean` — vegetation greenness, 16-day MODIS composite

- **Upstream**: [ORNL DAAC TESViS REST](https://modis.ornl.gov/rst/api/v1/MOD13Q1/subset).
  No auth, free.
- **Coverage**: global terrestrial.
- **Resolution**: 250 m native; we sample the cell centroid.
- **Cadence**: 16-day composite from MOD13Q1 (Terra).
- **Cite-ability**: Primary fact, `derivation.fn_key = "modis_ornl_subset@1"`,
  `sources.scheme = "ornl_modis"`.
- **Latency**: ~8–15 s upstream cold; ~10 ms hot-cache hit.
- **Tempo class**: `medium` → temporal router uses
  `Q = exp(-(Δt/σ)²)` with σ = 30 days.
- **Quality filter**: skips fill_value `-3000`; clamps NDVI to
  `[-0.2, 1.0]` per MOD13 user guide. Walks back up to 90 days
  to find the latest valid composite (cloudy windows produce
  multiple invalid points before a usable one).

Wire the response with `POST /v1/recall {cell, bands: ["modis.ndvi_mean"]}`.

### `gmrt.topobathy_mean` — global topo + bathymetry, single dataset

- **Upstream**: [GMRT PointServer](https://www.gmrt.org/services/PointServer)
  (Lamont-Doherty Earth Observatory).
- **Coverage**: global. Positive over land, negative over water.
- **Source data**: GMRT v4.x — fuses Cop-DEM 30 m, GEBCO, multibeam
  swaths, and high-resolution sounder surveys into a single peer-
  reviewed topo-bathy raster.
- **Format**: HTTP REST returning a single number in plain text
  (`-10917.0`). One round-trip per cell.
- **Rate limit**: ~2 req/s suggested by GMRT. The responder
  serializes per-cell calls and never retries automatically; bulk
  fan-out across `polygon_sample_cells` would breach this.
- **Cite-ability**: Primary fact, signed by the responder.
  `derivation.fn_key = "gmrt_pointserver@1"`, sources scheme `"gmrt"`.
- **Confidence**: 0.9. Agents wanting a higher-trust signed value
  for a *land*-only DEM at sub-30 m can use `copdem30m.elevation_mean`
  alongside.

### `geotessera` — 128-D Tessera v1 foundation embedding (HTTP range, no GDAL)

- **Upstream**: [GeoTessera v1 public bucket](https://dl2.geotessera.org/v1/global_0.1_degree_representation/),
  Cambridge AAILab + Clay-style self-supervised global embedding.
- **Coverage**: global terrestrial; 0.1° tile grid; v1 vintage 2024.
- **Resolution**: 10 m native (per-tile UTM CRS); we sample one pixel
  per cell.
- **Cite-ability**: Primary fact, `derivation.fn_key = "geotessera_v1@1"`,
  source scheme `"geotessera"`. Value is a CBOR array of 128 float32s.
- **Fetch strategy**: HTTP range reads. The full tile is 91 MiB; we
  range-read the .npy header (~256 B) to learn the shape, then 128 B
  for the embedding pixel and 4 B for the matching scale, dequantize
  via `f32 = i8 * scale` (matches `geotessera.dequantize_embedding`
  in the upstream Python). **~640 B downloaded per cell**, no GDAL,
  no rasterio.
- **CRS caveat**: the tile is stored in per-tile UTM, but we map
  (lat, lng) → (row, col) by linear interpolation across the 0.1°
  EPSG:4326 extent. Sub-pixel error near the tile centre, ~1–2 px
  near the corners. The string `"linear_latlng_in_utm_tile"` is
  recorded in `derivation.args` so the recipe is reproducible — an
  attester wanting cite-able UTM-precise values can re-attest with
  a proper rasterio reproject.
- **Tempo class**: `slow` → AR-1 kernel `Q = max(0, 1 − Δt/T)` with
  T = annual.

### `copdem30m.elevation_mean` — land DEM with honest absence over water

- **Upstream**: [Open-Meteo Elevation API](https://api.open-meteo.com/v1/elevation),
  which serves the Copernicus DEM 90 m wrap.
- **Coverage**: land surface, |lat| < ~85°.
- **Cite-ability**:
  - **Primary fact** (signed value) when Cop-DEM returns a non-zero
    land elevation.
  - **`Fact::Absence`** (signed absence) when Cop-DEM returns 0 m
    (its no-data marker over water) or 5xx (no coverage above the
    polar circle). The `reason_cid` is `blake3(canonical_reason_text)`
    base32-nopad-lowercase truncated to 16 bytes — same algebra as
    FactCid, so an agent can cite a reason with the same scheme they
    cite a fact.
- **Why both bands?** GMRT is the answer for "give me a number for
  any point on Earth"; Cop-DEM is the answer for "give me a *signed
  absence* if this isn't land, so my downstream model treats it
  differently from a land elevation of 0".

### `weather.{temperature_2m,cloud_cover,precipitation_mm,wind_speed_10m}` — geostationary-fed 15-minute weather

- **Upstream**: [MET Norway Locationforecast 2.0/compact](https://api.met.no/weatherapi/locationforecast/2.0/compact).
  MET Norway's NWP blend is sat-fed via ECMWF runs that ingest the
  EUMETSAT geostationary fleet (Meteosat-9/11) plus GOES-16/17/18 and
  Himawari-9. JSON wire format; no API key, no per-IP rate limit;
  TOS asks only for an identifying User-Agent. See `/v1/fleet` for
  the per-platform lineage.
- **Coverage**: global; hourly point forecast updates.
- **Cite-ability**: Primary fact, signed by the responder.
  `derivation.fn_key = "met_no_locationforecast_compact@1"`,
  source scheme `"met_no"`.
- **Tempo class**: `ultra_fast` → advection kernel
  `Q = max(0, 1 − Δt/H)` with H ≈ 6 hourly slots — matches how
  weather is *transported* much faster than created.
- **Bands**:
  - `weather.temperature_2m` (degC, conf 0.85)
  - `weather.cloud_cover` (percent, conf 0.80) — pair with
    `modis.ndvi_mean` to gate vegetation reads when high-cover means
    the optical composite is stale.
  - `weather.precipitation_mm` (mm, conf 0.75)
  - `weather.wind_speed_10m` (m/s, conf 0.80)
- **Caveat**: weather bands are not in `bands-v0.json` (the cube
  layout). They are declared via `/v1/materializers` and `/v1/fleet`,
  recognized by the materializer dispatcher, and signed under the
  responder identity — but the temporal router's `cite_now` list
  iterates the cube, so weather bands won't appear there until the
  cube is bumped to a v1 manifest version that admits non-cube bands.

---

## Pre-declared in manifests, materializer not yet wired

These bands have entries in `crates/emem-core/data/sources-v0.json`
and `functions-v0.json` but no fetch path is wired in
`try_materialize_bands`. An agent calling `/v1/recall` for them today
gets `materialize_notes: [{status: "skipped", reason: "no_auto_materializer_registered"}]`.

| Band | Upstream | Format | Why next? |
| --- | --- | --- | --- |
| `indices.ndvi` | Sentinel-2 L2A on [Microsoft Planetary Computer](https://planetarycomputer.microsoft.com/) — `B04` + `B08` | COG via vsicurl | **Most-asked-for vegetation signal**; deterministic spectral formula `(B08-B04)/(B08+B04)`; no auth; CC-BY-4.0 |
| `sentinel2_raw` | Sentinel-2 L2A on Planetary Computer | COG | Same upstream as NDVI, returns the 10 raw bands instead of an index |
| `sentinel1_raw` | ASF / Planetary Computer | COG (GRD) | Radar; complements optical when cloudy |
| `landcover.esa_worldcover` | [ESA WorldCover v200](https://esa-worldcover.org/) | COG | Categorical (11 classes); annual; static once ingested |
| `forest_change.hansen_loss` | [Hansen GFC v1.x](https://glad.umd.edu/dataset/global-2010-tree-cover-30-m) | COG | One-shot pull; year-of-loss raster |
| `surface_water.jrc_recurrence` | [JRC GSW v1.4](https://global-surface-water.appspot.com/) | COG | Long temporal series compressed into one band |
| `soilgrids.organic_carbon` | [ISRIC SoilGrids v2](https://www.isric.org/explore/soilgrids) | COG | Static; quarter-annual update; CC-BY-4.0 |
| `nightlights.viirs_dnb` | [Earth Observation Group VIIRS DNB](https://eogdata.mines.edu/products/vnl/) | COG | Monthly; aggregate-only privacy class |
| `koppen.beckV2` | [Beck Köppen-Geiger v2 (2018)](https://doi.org/10.1038/sdata.2018.214) | COG | Static categorical climate zone |

### Foundation embeddings: live and reserved

The 0.0.x reference build ships three open-weight foundation embeddings as auto-materializing bands:

- **`geotessera`** — Tessera v1 (Cambridge), 128-D, vintage 2024 only. Upstream serves int8 + per-pixel f32-scale tiles via HTTPS Range. Decoded to f32 over the wire. ~640 B/cell delivery cost.
- **`prithvi_eo2`** — Prithvi-EO-2.0-300M-TL (NASA / IBM, Apache-2.0), 1024-D. The materializer fetches a 224×224 HLS V2 6-band chip (Blue, Green, Red, Narrow-NIR, SWIR1, SWIR2) at the cell from the Sentinel-2 L2A path, normalises per band, and runs the ViT-L locally on CUDA via the GPU sidecar (`python/jepa_v2_sidecar`). Cold recall ~2-4 s; warm forward ~19 ms.
- **`galileo_base_v1`** — Galileo Base (NASA Harvest, MIT), 768-D. The materializer fetches a 10-band 8×8 chip at 30 m equiv (24×24 block-pool for 10 m bands; 12×12 bilinear for 20 m), runs the encoder locally on CUDA. S1 + DEM + climate modalities are accepted zero-masked (S2-only mode in 0.0.x). Cold recall ~4 s.

#### `alphaearth.satellite_embedding_v1` — slot reserved, not wired

- **Upstream**: [Google AlphaEarth Foundations](https://deepmind.google/blog/alphaearth-foundations-helps-map-our-planet-in-unprecedented-detail/), 64-dim annual global embeddings since 2017. Available via Earth Engine `GOOGLE/SATELLITE_EMBEDDING/V1/ANNUAL` and the public GCS bucket `gs://alphaearth_foundations` as COGs.
- **Why not wired in 0.0.x?** Two reasons. First, DeepMind has not released open weights, so the embedding cannot run locally; the only delivery channel is the GEE-hosted layer. Second, `gs://alphaearth_foundations` is **Requester Pays** — the caller must pass a billing project ID with their GCS request, and the default responder serves zero-billing public data only.
- **Layout caveat**: the band entry in `bands-v0.json` reserves a 576-dim slot (9 yrs × 64) from a legacy AlphaEarth-v0 internal cube. If an operator ever wires the public V1 (64 dims), the materializer would write 64 floats into the leading slice and zero-pad the rest so byte offsets stay stable.

---

## Zarr support — for fast multi-temporal reads

Question that comes up: *"if our results need to be fast, should we
use Zarr?"* Short answer: yes for any band that's natively chunked
across a third dimension (time, depth, ensemble member), no extra
work for bands that are flat 2D rasters.

**Concrete plan** for adding a Zarr fetch path to a materializer:

1. Pick a band whose upstream is genuinely Zarr — e.g. ECMWF ERA5
   on Pangeo Forge, AlphaEarth annual stack as a single Zarr cube
   (vs. one COG per year), CMIP6 climate runs.
2. Add the [`zarrs`](https://github.com/LDeakin/zarrs) crate as a
   workspace dep. It does HTTP range reads against the Zarr v3
   spec without GDAL.
3. In the materializer: open the Zarr store, read the chunk that
   covers `(cell.lat, cell.lng, target_year)`, sample the value,
   sign as a Primary fact. The chunk-fetch is a single ranged
   `GET` against `https://<host>/<prefix>/<chunk-key>` — at the
   wire level identical to a vsicurl COG read.

**Performance notes from the cloud-native-geospatial community:**

- Bytes-on-disk are **near-identical** between a Zarr chunk and a
  COG tile when codecs match (zstd, blosc).
- Zarr wins when you want N timesteps × M bands at one (lat, lng)
  — one ranged read fetches a whole pencil through the cube.
- COG wins when you want N tiles × 1 band at one timestep — the
  COG IFD index is fewer hops.
- For emem's typical access pattern (one cell, one band, one or
  two times) **either format is roughly equal**. Zarr's structural
  win shows up in `find_similar` / `query_region` workloads where
  we read many cells at the same time slice.

This makes Zarr support a **per-band implementation detail**, not
a protocol-level decision. A band is "Zarr-backed" if its
materializer happens to use the `zarrs` crate; an agent calling
`/v1/recall` doesn't know or care.

---

## On the format question: COG, Zarr, vsicurl

For an emem materializer, a "cloud-native geospatial format" is anything
that supports HTTP range reads against a single durable URL. From a
fetch-side perspective:

- **COG** (Cloud-Optimized GeoTIFF) — single 2D raster, internal
  tiling + IFD index, widely-tooled (GDAL, rasterio, gdal-async).
  Right format for "snapshot at one time, give me a chunk".
- **Zarr** — generic chunked n-D array, widely adopted by Pangeo /
  ESA / Earth modelling. Right format for "stack of N timesteps × M
  bands × H × W". Bytes-per-chunk are nearly identical to COG when
  codecs match; the difference is the index layout and whether the
  array is single-IFD (COG) or hierarchical (Zarr).
- **vsicurl** — a GDAL VSI handler that does the HTTP range reads.
  Works against COG natively and against Zarr via a separate driver
  (or you bypass GDAL and use a Rust Zarr crate like
  [`zarrs`](https://github.com/LDeakin/zarrs)).

**Practical translation for emem materializers**: bands shipped as
COG (most Sentinel-2, Landsat, Cop-DEM, ESA WorldCover, Hansen GFC
data on AWS Open Data and Microsoft Planetary Computer) use
GDAL/vsicurl. Bands shipped as Zarr (CMIP6 climate runs, ICESat-2
ATL03 cloudfree, ECMWF reanalysis) use a Zarr reader. **Either way
it's HTTP range reads against a public URL** — no auth, no GDAL/Zarr
in the URL (it's an implementation detail of the materializer).

---

## Parity with the agri training stack

`/home/ubuntu/agri/integrate_10m.py` is the closest analogue project:
it builds 10 m × 10 m × 1792 D embedding cubes per farm by stacking
the same band families. Comparing the two ingestion paths makes the
shape of the remaining work explicit:

| Band family | agri (training) | emem (this branch) | parity? |
| --- | --- | --- | --- |
| GeoTessera 128 D | `GeoTessera().fetch_embedding()` (Python; downloads full 91 MiB tile, rasterio reproject) | `materialize_geotessera_embedding` (pure Rust; HTTP range, ~640 B/cell, linear-in-tile sample) | ✅ same upstream, different precision/cost trade |
| AlphaEarth annual | reads pre-exported per-farm `.tif` files via `rasterio.open` | declared with offset reservation; auto-materializer gated on `EMEM_ALPHAEARTH_BILLING_PROJECT` | ⚠ blocked on Requester Pays GCS auth |
| Sentinel-2 L2A | `rasterio` against pre-staged COG files | declared; PC COG via vsicurl pending | ⚠ needs GDAL or pure-Rust COG reader (e.g. [`tiff` crate + custom IFD parser]) |
| Sentinel-1 GRD | `rasterio` against pre-staged COG files | declared; ASF/PC COG via vsicurl pending | ⚠ same as S2 |
| MODIS 16-day NDVI | not used at agri's 10 m grain | `materialize_modis_ndvi` (REST point query) | ✅ emem covers a band agri doesn't |
| Cop-DEM | rasterio over public COG | `materialize_elevation_mean` (Open-Meteo wrap) | ✅ both paths land at the same number |
| Weather (GOES/Himawari/Meteosat-fed) | not in agri | `materialize_weather_current` (Open-Meteo Forecast `current`) | ✅ emem covers a band agri doesn't |

The remaining gap is **a pure-Rust COG reader** — `rasterio` (the
GDAL Python wrapper) is the agri stack's entry point for every
Planetary Computer band. emem deliberately avoids GDAL because (a)
it's a heavyweight C dep with hundreds of feature flags, (b) it
breaks the "no-auth, single-binary" promise, and (c) HTTP range
reads against a COG IFD are perfectly tractable in pure Rust:

1. Range-read first 64 KB → parse TIFF IFD list to get tile/strip
   offsets.
2. For each requested pixel, look up the tile that contains it,
   range-read just that tile.
3. Decompress (LZW/Deflate/Zstd via the `flate2`/`zstd` crates,
   already in the workspace via reqwest features).

The `tiff` crate at version 0.10 already provides IFD parsing; the
missing piece is a thin async wrapper that issues HTTP range reads
instead of file-system reads. This is the same pattern the GeoTessera
materializer uses (range-read .npy header → range-read pixel) and
fits cleanly under `try_materialize_bands`.

Once that wrapper exists, the four pending bands above become
~80-line each:

- `indices.ndvi` — fetch `B04` and `B08` Sentinel-2 COG pixels,
  compute `(B08-B04)/(B08+B04)`, sign as Primary or Absence (cloud).
- `sentinel2_raw` — same upstream, return the 10-band vector instead
  of the index.
- `sentinel1_raw` — ASF GRD COG; two-band (VV, VH) vector.
- `landcover.esa_worldcover` — single-band categorical COG; one
  pixel per cell.

## How to add a new materializer

1. Decide whether it needs a sub-second answer (REST point query like
   GMRT's `PointServer`) or a chunked raster sample (COG/Zarr range
   read). The first is one HTTP call; the second needs a parser.
2. Pick a function key — the convention is `<vendor>_<api>@<version>`
   (e.g. `open_meteo_copdem90m@1`, `gmrt_pointserver@1`,
   `planetary_computer_s2_l2a@1`). Register in `functions-v0.json`
   with the deterministic argument schema.
3. Pick a source scheme — register in `sources-v0.json` so receipts
   can name the upstream provider.
4. Implement `materialize_<band>` in `crates/emem-api-rest/src/lib.rs`
   following the pattern of `materialize_gmrt_topobathy` (Primary)
   or `materialize_elevation_mean` (Primary or Absence with
   content-addressed reason).
5. Wire into `try_materialize_bands` and add an entry to the
   `/v1/materializers` JSON.
6. Test against `scripts/global_trial.py`.

The scaffolding now exists for all of these — what's left for the
shipping queue above is the per-band HTTP/parse code.
