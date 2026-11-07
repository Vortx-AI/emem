# emem data sources

This page tells you where the node gets each value, and how to add a new source.
The facts here come from the code at version 2.4.4 and from emem.dev on 7 November 2026.

## How a miss becomes a fact

The node holds no data when it starts. A recall for a cell and band that it does not hold is a miss.
On a miss, the node reads the value from an open archive, signs it, and stores it. The next
caller gets the stored fact. This is lazy materialization. `EMEM_AUTO_MATERIALIZE` controls it,
and it is on by default.

All default sources are keyless. The node needs no account and no API key to read them.

![The path of one value in six steps: an open pixel is read with an HTTP Range request, a stateless connector makes a PrimaryFact or Derivative, canonical CBOR encodes it, BLAKE3 gives its content id, and the responder signs the fact. The same bytes give the same content id on every machine.](/docs/diagrams/02-data-flow.svg)
*The path of one value from an upstream tile to a fact you can cite. The same upstream and the same registries give the same CID.*

```text
POST /v1/recall {cell, band, tslot}
       |
       v
look up (cell, band, tslot) in facts.redb
       |-- hit  --> return the fact and a receipt
       |-- miss
       v
EMEM_AUTO_MATERIALIZE is not 0 or false?
       |
       v
try_materialize_bands: one match arm per band
       |
       v
read the upstream, one of:
   STAC search, then a COG range read (Sentinel-2, Sentinel-1)
   COG range read (Copernicus DEM, ESA WorldCover, Hansen, JRC, CHIRPS, WorldPop)
   JSON REST (Open-Meteo, NASA POWER, MET Norway, ORNL DAAC, ISRIC, GMRT)
   Parquet row groups on S3 (Overture)
   NCSS CSV (TerraClimate)
   ZIP central directory, then a cached TIFF (Koppen-Geiger)
   Overpass QL (protected areas)
       |
       v
parse, compute, then a Primary fact or a signed Absence
       |
       v
sign as the responder and store in facts.redb
       |
       v
return the fact and a receipt
```

Two timeouts apply. The values are in `crates/emem-api-rest/src/lib.rs`.

| Variable | Default | Range | Covers |
|---|---|---|---|
| `EMEM_MATERIALIZER_TIMEOUT_SECS` | `14` | 2 to 240 s | One upstream read |
| `EMEM_TIMEOUT_SECS` | `40` | 1 to 600 s | The whole request, with all its upstream reads |

[operating.md](../operators/operating.md) lists every other variable.

## What the node can read today

The node publishes its band registrations at `GET /v1/materializers`. The list has pages, 20 entries
by default and 100 at most. On 7 November 2026, emem.dev listed 113 registrations on 2 pages.

```sh
curl -s 'https://emem.dev/v1/materializers?page_size=100&summary=true' \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["pagination"]["total"])'
```

Each entry gives the band, the upstream wire path, the derivation function key, and the history
window. Use this list, not this page, to learn what a running node serves.

`crates/emem-core/data/sources-v0.json` declares 46 source schemes. Some have no materializer. See
[Declared schemes that no code reads](#declared-schemes-that-no-code-reads).

## Connector modules in emem-fetch

The `emem-fetch` crate has 36 modules and the crate root, `lib.rs`. Most modules read one upstream.
The others hold shared tools and embedded gazetteers.

### Modules that read an upstream

| Module | Upstream | How it reads | Bands or use |
|---|---|---|---|
| `chirps.rs` | UCSB CHC CHIRPS 2.0 | Range read on a daily or monthly COG | `chirps.precip_daily_mm` |
| `copernicus_dem.rs` | Copernicus DEM GLO-30 on AWS Open Data | URL helpers for the COG sampler | `copdem30m.elevation_mean` |
| `dmsp_ols.rs` | NOAA NCEI DMSP-OLS V4 | TAR header, then a gzipped TIFF, cached | `nightlights.*`, retired in the code |
| `esa_cci_biomass.rs` | ESA CCI Biomass v7.0 on CEDA | GeoTIFF range read, 10 degree tiles | `esa_cci_biomass` |
| `esa_worldcover.rs` | ESA WorldCover 2021 v200 on S3 | URL helpers for the COG sampler | `esa_worldcover.lc_2021` |
| `firms.rs` | NASA FIRMS bulk CSV | HTTPS GET, cached for 60 minutes | `firms.active_fires` |
| `ftw.rs` | Fields of The World on Source Cooperative | PMTiles range reads, MVT decode | `/v1/field_boundaries` |
| `gmrt.rs` | GMRT PointServer | JSON REST | `gmrt.topobathy_mean` |
| `hansen_gfc.rs` | Hansen GFC v1.13 on GCS | Range read on strip TIFFs, 10 degree tiles | `forest_change.*`, `hansen.*` |
| `jrc_gfc2020.rs` | JRC Global Forest Cover 2020 | COG range read at the version `LATEST/` lists | `jrc_gfc2020.forest_2020` |
| `jrc_gsw.rs` | JRC Global Surface Water v1.4 | URL helpers for the COG sampler | `surface_water.*` |
| `jrc_tmf.rs` | JRC Tropical Moist Forest v2025 | Tile download and cache | `jrc_tmf.*` |
| `koppen.rs` | Beck et al. 2018 on Figshare | ZIP central directory, then a cached TIFF | `koppen` |
| `overture.rs` | Overture Maps on S3, `us-west-2` | Parquet row-group pruning, WKB decode | `overture.*`, place polygons for `/v1/locate` |
| `terraclimate.rs` | TerraClimate on THREDDS NCSS | CSV REST, two mirrors | `terraclimate.*_normal*` |
| `wdpa.rs` | OSM Overpass, `boundary=protected_area` | Overpass QL `is_in` | `protected`, `protected.is_protected_area` |
| `wikidata.rs` | Wikidata | HTTPS JSON | Prominence evidence for the geocoder |
| `worldpop.rs` | WorldPop | Per-country COG, then the stats API as a fallback | `population` |
| `wri_gdm_drivers.rs` | WRI and Google DeepMind drivers of forest loss, on Zenodo | Pixel read from a local copy at `EMEM_WRI_GDM_CACHE` | `wri_gdm.driver_class` |

### Modules with no open read path

Two modules exist, but the node cannot read their data. For a cell inside the coverage, the
recall gets an error and the node signs nothing. Only RADD signs an Absence, and only for a cell
outside its belt (latitude more than 30 degrees from the equator, `radd_alerts.rs`). OPERA never
signs an Absence: with or without a token, the recall gets an error (`lib.rs`,
`materialize_opera_dist_band`).

| Module | Upstream | Why there is no read |
|---|---|---|
| `radd_alerts.rs` | RADD forest alerts on the GFW data API | The raster tiles are on Requester-Pays S3. An anonymous GET gets 403. |
| `opera_dist.rs` | NASA OPERA DIST-ALERT-HLS V1 | It needs a NASA Earthdata token. Without `EMEM_EARTHDATA_TOKEN`, the reason is `not_enabled`. With a token, `fetch_alert` returns `NotImplemented`. |

### Shared tools and embedded data

| Module | Does |
|---|---|
| `cog.rs` | Reads one pixel or one window from a COG. Every raster connector uses it. |
| `stac.rs` | STAC POST search on Element84 and Microsoft Planetary Computer |
| `proj.rs` | WGS84 to UTM, so the COG sampler gets coordinates in the scene CRS |
| `connectors.rs` | HTTPS, GCS and IPLD fetch plumbing per `ConnectorKind` |
| `template.rs` | Fills `{name}` variables in source URL templates |
| `cache_window.rs` | Joins concurrent fetches of the same source into one |
| `tile_store.rs` | Keeps whole local copies of the static forest rasters |
| `latency.rs` | Times each stage of a cold read |
| `parcels.rs` | Removes overlaps and duplicates from a set of field polygons |
| `countries.rs`, `admin1.rs`, `admin2.rs`, `admin3.rs`, `geonames.rs`, `pois.rs` | Embedded GeoNames gazetteers for `/v1/locate` |
| `lib.rs` | The crate root and the connector framework |

## Materializers in emem-api-rest

Some upstreams have no module in `emem-fetch`. The materializer in `crates/emem-api-rest/src/lib.rs`
calls the JSON API itself. To find them all, search that file for `async fn materialize_`. Add
`materialize_algorithm_band`, which is not `async`. Other `fn materialize_` names, such as
`materialize_budget_per_min`, `materialize_share` and `materialize_inflight`, are helpers, not
materializers.

| Function | Upstream | Bands |
|---|---|---|
| `materialize_weather_current` | MET Norway locationforecast 2.0 | `weather.*` |
| `materialize_power_band` | NASA POWER | `power.*` |
| `materialize_cams_band` | Open-Meteo air quality (CAMS) | `cams.*` |
| `materialize_era5_band` | Open-Meteo archive (ERA5) | `era5.*` |
| `materialize_marine_band` | Open-Meteo marine | `marine.*` |
| `materialize_modis_ndvi`, `materialize_ornl_modis_band` | ORNL DAAC MODIS subsets | `modis.*` |
| `materialize_soilgrids_band` | ISRIC SoilGrids 2.0 REST | `soilgrids.*_0_30cm` |
| `materialize_sentinel2_band` | Planetary Computer, then Element84 STAC, then `cog.rs` | `s2.*`, `indices.*` |
| `materialize_sentinel1_vv` | Planetary Computer Sentinel-1 RTC | `sentinel1_raw` |
| `materialize_temporal_diff` | Two stored parent facts | `temporal_diff:<band>:<window>` |
| `materialize_algorithm_band` | The input bands of an algorithm, read with a recall | `<algorithm>@<version>`, such as `flood_risk@2` |

`materialize_algorithm_band` runs the `evaluation` expression of the algorithm in
`algorithms-v0.json`. On 7 November 2026, 74 of the 172 algorithms had one. When an input is
missing, the node signs an Absence that names the missing bands.

The other materializers in that file call the `emem-fetch` modules in the first table.

## Retired bands

A retired band is still in the band manifest. Facts that the node signed before keep their
recall and their verification. The node does not make new ones. A recall gets a note with
reason `band_retired_at_this_responder`.

| Band | Retired by | Why |
|---|---|---|
| `clay_v1`, `prithvi_eo2`, `galileo` | Code (`RETIRED_ENCODER_BANDS`) | The project removed the code that made them. |
| `nightlights` | Code (`RETIRED_UPSTREAM_GONE_BANDS`) | NOAA moved DMSP-OLS V4 behind a login in 2026. |
| `geotessera` | `EMEM_RETIRED_BANDS` in `deploy/systemd/emem-server.service` | The upstream answers 410 Gone. |

A self-hosted node retires more bands with `EMEM_RETIRED_BANDS`, a comma-separated list.
When you retire a family, you retire its children: `geotessera` also retires `geotessera.2024`.

## Declared schemes that no code reads

These 12 schemes are in `sources-v0.json`, but no connector or materializer in `crates/` reads them
(code on 7 November 2026). The first five reasons date from May 2026.

| Scheme | Why no code reads it |
|---|---|
| `openet.30m.daily` | The S3 bucket in the template answers `NoSuchBucket`. |
| `dynamic_world.v1` | It needs Google Earth Engine OAuth, so it breaks the keyless rule. |
| `tropomi.s5p.ch4`, `tropomi.s5p.no2` | They are NetCDF, not COG. The node has no NetCDF decoder. |
| `viirs.dnb.monthly` | Every upstream path needs a NASA Earthdata login. |
| `sentinel1.grd.iw`, `sentinel1.grd.iw.vh` | The node reads VV from `sentinel_s1_rtc_mpc` only. |
| `ghsl.builtup.r2023a`, `ghsl.population.r2023a` | No GHSL connector exists. A recall of `ghsl` gets `reason_class` `unknown_band`. |
| `model.prithvi_eo2_300m_tl`, `model.clay_v1_5`, `model.galileo_v1` | They needed the GPU sidecar, which no longer runs. See [Retired bands](#retired-bands). |

## Notes on each connector

### cog.rs: the COG sampler

`cog.rs` reads the TIFF header, the IFD and the one tile that holds the pixel. A per-cell read
of a Sentinel-2 scene moves a few hundred KB, not the whole scene.

1. Read the first 64 KiB of the file.
2. Parse the TIFF header and IFD0. If the tile offset arrays are past 64 KiB, read them separately.
3. Get the world-to-pixel transform from `ModelPixelScale` and `ModelTiepoint`.
4. Find the tile that holds the pixel, read it, decompress it, and undo the predictor.

| Supported | Not supported |
|---|---|
| Standard TIFF and BigTIFF | Big-endian TIFF |
| Deflate (8) and LZW (5) | JPEG2000 and other codecs |
| Predictor 1, 2, and 3 (3 for 32-bit float only) | Planar layouts |
| Strip TIFFs, read as one-row tiles | |

An unsupported file gives an error. The sampler does not guess.

### stac.rs: scene search

Sentinel-2 uses Planetary Computer first and Element84 second. `EMEM_S2_CATALOGUES` changes the
order or names one catalogue. The node goes to the next catalogue only when one fails. An empty
answer from a healthy catalogue means there is no scene.

The two catalogues give different DNs for the same scene. Element84 removed ESA's
`BOA_ADD_OFFSET`, and Planetary Computer did not. Reflectance is `(DN + offset) * 1e-4`. The
offset is -1000 for Planetary Computer scenes from processing baseline 04.00. For Element84 it is
usually 0. Baseline 04.00 items acquired from 2022-01-25 to 2022-02-26 still have -1000. The code does not
know the offset for items of 2022-02-27 and 2022-02-28, so the node refuses them
(`element84_dn_offset` in `stac.rs`). The node also refuses a scene that states no baseline when it
needs one. Each fact's `derivation.args` ends with the catalogue
and the offset.

A single cell searches with `intersects: Point`, not `bbox`. A bbox search can match the next
tile in an MGRS overlap zone. A read over many cells searches once for their bbox. It then keeps,
for each cell, the items whose footprint holds the point.

Planetary Computer asset URLs need a free anonymous SAS token. The connector caches one token for
each collection until 5 minutes before its `msft:expiry`. The cache time is at least 1 minute and
at most 55 minutes. When `msft:expiry` is missing, the cache time is 30 minutes (`stac.rs`).

### overture.rs: Parquet on S3

The bucket is `overturemaps-us-west-2`. The connector lists the release files once and caches each
Parquet footer. It uses the `bbox` column statistics to skip row groups before it decodes any
geometry. The WKB decoder is in Rust, with no GEOS and no GDAL.

`EMEM_OVERTURE_RELEASE` pins a release. Without it, the connector finds the newest release and
checks again every 24 hours (`RELEASE_TTL`).

The same client reads the `divisions/division_area` theme for `/v1/locate` place polygons
(`division_polygon_with_subtype`). It keeps the polygons that contain the point. The first rule
that has a candidate picks the polygon. "Smallest" and "largest" compare bbox areas.

1. The caller asks for a subtype, such as `country`, and a polygon of that subtype has the exact
   name. The largest of these wins.
2. A polygon has the exact name. The highest admin level wins, then the largest.
3. The caller asks for a subtype, and a polygon has that subtype. The smallest wins.
4. A polygon name contains the name, or the name contains it. The smallest wins.
5. No name matches. The smallest polygon wins.

When no polygon contains the point, the connector returns no polygon.

### terraclimate.rs: NCSS CSV with failover

TerraClimate is NetCDF-4 with compressed chunks, so a range read is not practical. The
connector asks the THREDDS NetCDF Subset Service for CSV instead. NCSS returns the packed
integers, so the connector applies `scale_factor`, `add_offset` and `_FillValue` itself.

`NCSS_BASES` lists two mirrors. The connector tries them in order on a transport error. The
source URL in the fact names the mirror that answered. `NORMAL_WINDOW` is `(1991, 2020)`, so the
bands are 30-year normals, not values for one year.

### worldpop.rs: per-country COG, then the stats API

The global WorldPop GeoTIFF ignores `Range` requests and sends the whole 870 MB file. The
per-country 1 km files accept ranges, so the connector reads those with `cog.rs` first. On a
transient COG error it falls back to the synchronous stats API, with a 1 km by 1 km area.

The year is 2020 (`WORLDPOP_DEFAULT_YEAR`). A cell over the ocean, or with zero population,
gets a signed Absence, not a zero.

### hansen_gfc.rs: strip TIFFs in 10 degree tiles

The source is the `earthenginepartners-hansen` bucket, version v1.13 (GFC-2025). It covers loss
from 2001 to 2025. `lossyear` is 0 for no loss and 1 to 25 for the years 2001 to 2025.

A tile name gives its top-left corner. The point at latitude -3.0, longitude -60.5 is in tile
`00N_070W`. A tile is 40 000 by 40 000 pixels at 0.00025 degrees, stored as one strip per row.

`lossyear = 0` on land is a real Primary fact. A point outside the dataset gets a signed Absence.

### koppen.rs: one file from a ZIP

The source is a 71 MB ZIP on Figshare with twelve GeoTIFFs. The connector reads the ZIP central
directory and then only the 1 km present-day TIFF. It inflates that TIFF and caches it under
`<EMEM_DATA>/cache/koppen/`. Pixel value 0 is outside the land mask and gives a signed Absence.

### wdpa.rs: protected areas from OSM

The Protected Planet API answers 401 to anonymous callers, so the connector does not use it. It
asks the public Overpass API for `boundary=protected_area` with `is_in()`. OSM `protect_class`
maps to the IUCN category.

A cell outside every mapped protected area gets a signed negative fact. Each cell costs at most
one Overpass call, because the node stores the answer.

### wri_gdm_drivers.rs: a local copy, not a range read

Zenodo sends the whole 295 MB GeoTIFF for any `Range` request, so a per-cell read is not
practical. The connector reads band 1 of a local copy. Set `EMEM_WRI_GDM_CACHE` to the absolute
path of that file. Without it, the node signs nothing and the recall gets an error that names
the variable. The node does not sign an Absence, because the cell itself is not empty.

Pixel value 0 means no Hansen loss event. The node signs an Absence with reason
`wri_gdm_no_hansen_loss`. On 7 November 2026, emem.dev gave a Primary fact with class 6 near
Novo Progresso, Brazil, at cell `defi.zb3af.zedd6.gEdE` (-7.0375, -55.4186):

```sh
curl -s https://emem.dev/v1/recall -H 'content-type: application/json' \
  -d '{"cell":"defi.zb3af.zedd6.gEdE","bands":["wri_gdm.driver_class"]}' \
  | jq '.facts[0].value'
```

### ftw.rs: field boundaries

Fields of The World is one global PMTiles archive on Source Cooperative, under CC-BY-4.0. The
connector reads only the tiles that cover the requested bbox. It lowers the zoom when a query
needs more than 16 tiles. The BLAKE3 hash of the decompressed tile bytes is the source CID.

Use it through `POST /v1/field_boundaries`, through `include: ["ftw_fields"]` on
`POST /v1/recall_polygon`, or through the MCP tool `emem_field_boundaries`.

### geonames.rs and the locate gazetteers

`/v1/locate` checks embedded tables before it calls any network geocoder. `geonames.rs` holds
GeoNames cities-1000, with every populated place of 1 000 people or more. On 7 November 2026 the
file had 169 051 rows. It is `crates/emem-fetch/data/cities1000.txt.gz`, about 10.5 MB.

The lookup folds diacritics and indexes alternate names. When several places share a name, the
place with the most people wins. The `via` field of the response names the layer that answered.

## Settings for data sources

| Variable | Default | Does |
|---|---|---|
| `EMEM_AUTO_MATERIALIZE` | on | `0` or `false` stops reads from the archives on a miss. |
| `EMEM_MATERIALIZER_TIMEOUT_SECS` | `14` | The timeout of one upstream read, 2 to 240 s. |
| `EMEM_MATERIALIZER_RETRIES` | `2` | Tries for one ORNL MODIS read, 1 to 5. |
| `EMEM_TIMEOUT_SECS` | `40` | The timeout of one request, 1 to 600 s. |
| `EMEM_S2_CATALOGUES` | `mpc,element84` | The Sentinel-2 catalogues, in order. |
| `EMEM_OVERTURE_RELEASE` | newest | Pins one Overture release. |
| `EMEM_RETIRED_BANDS` | empty | More bands that this node does not serve. |
| `EMEM_WRI_GDM_CACHE` | unset | The path of the local WRI drivers COG. Without it, a `wri_gdm.driver_class` recall gets an error. |

## Add a connector

A connector has two parts: a fetch module in `emem-fetch` and a materializer in `emem-api-rest`.
The WDPA connector is a good model: `crates/emem-fetch/src/wdpa.rs` and `materialize_protected`
in `crates/emem-api-rest/src/lib.rs`. Line numbers in `lib.rs` change often, so search for the names.

1. Add `crates/emem-fetch/src/<name>.rs` and a `pub mod <name>;` line in
   `crates/emem-fetch/src/lib.rs`. The top doc comment gives the upstream, the licence, the auth,
   the rate limits, and empty versus error.
2. Write an async fetch function that returns the value or a typed error. `fetch_wdpa_status`
   returns `Result<Option<WdpaMatch>, WdpaError>`.
   - Return `Ok(None)` for a real empty result. The materializer then signs an Absence.
   - Return `Err` for an upstream failure. The node stores nothing, and the next recall tries again.
   - Never invent a default value.
   - If the upstream URL changes for each read, return it in the sample, as
     `NightlightSample.upstream_url` does in `dmsp_ols.rs`.
3. Add `async fn materialize_<name>` in `lib.rs`. Set `sources[0].url` and `derivation.fn_key`
   on the fact. Sign through `sign_and_persist`, so the receipt shape stays the same.
4. Add a match arm for the band key in `materialize_bands_once`. `try_materialize_bands` calls
   it on a recall miss.
5. Add the band key to `all_materializable_bands`. If the key is not in this list,
   `band_is_known` is false. Then `/v1/materializers`, `/v1/bands` and `/v1/data_availability`
   do not show the band.
6. Add an arm to `band_materializer_meta` with the tempo, the history window and the resolution.
   Add a row to the `materializer_bands` table in `coverage_matrix` for `/v1/coverage_matrix`.
7. If the source has a history, add an arm to `materialize_band_at`. Backfill and the temporal
   windows call it for a past date.
8. Choose the band key. A dotted key, such as `copdem30m.elevation_mean`, needs no change to
   `bands-v0.json`. To give it the text of a cube slot, add an arm to `cube_band_alias` in
   `crates/emem-core/src/bands.rs`. Only a new cube slot needs a change to `bands-v0.json`.
   `BandRegistry::validate` needs contiguous offsets that sum to 1792. Use a reserved slot or
   the tail. Never move an existing offset.
9. Optional: declare the scheme in `crates/emem-core/data/sources-v0.json` and the function in
   `crates/emem-core/data/functions-v0.json` (`kind: primary`, `deterministic: true`). The node
   publishes these files at `/v1/sources` and `/v1/functions`, but no read or write path checks a
   fact against them. On 7 November 2026, `lib.rs` had 40 `fn_key` values outside its tests.
   Only 6 were in `functions-v0.json`, and `sources-v0.json` had no WDPA scheme.
10. Test offline with `cargo test -p emem-fetch <name>`. Give the parser bytes that the test
    builds or records, as the `dmsp_ols.rs` tests build a small TIFF. The network tests are in
    `crates/emem-fetch/tests/live_cog_fetch.rs` and run with
    `cargo test --workspace --test live_cog_fetch`. There is no `live` cargo feature.

Steps 8 and 9 can change registry files. A change there moves the manifest CIDs. Read
[architecture.md](architecture.md) before you change a registry.
