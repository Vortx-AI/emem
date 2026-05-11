# emem data sources

## How sources flow into facts

The `emem-fetch` crate exposes typed connectors over an anonymous open-data
corpus — every default provider is keyless. The `emem-api-rest` crate calls
into those connectors (and a parallel set of inline HTTPS-JSON materializers
that live in `lib.rs`) to materialize facts on demand. Lazy materialization
(`EMEM_AUTO_MATERIALIZE=1`, on by default in production) makes a `/v1/recall`
miss kick off an upstream fetch, sign the resulting value as the responder's
own Primary attestation, and persist it forever — so the next caller hits the
hot cache. This is the read → attest → cache loop that lets every cell on
Earth answer cite-ably from day one.

## Materialization flow

```text
GET /v1/recall {cell, band, tslot}
       │
       ▼
sled hot lookup (cell, band, tslot) ──► fact_cid?
       │            ┌─ hit  ──► get_facts → return fact + Receipt
       └─ miss ─────┤
                    ▼
             EMEM_AUTO_MATERIALIZE != 0?
                    │
                    ▼
         try_materialize_bands match arm for `band`
                    │
                    ▼
           connector dispatch
              ├─ STAC search (Element84 / MS PC)
              ├─ COG range read (Cop-DEM, ESA WC, Hansen, JRC GSW)
              ├─ HTTPS JSON REST (Open-Meteo, NASA POWER, met.no, ORNL DAAC, ISRIC, GMRT)
              ├─ Parquet S3 row-group prune (Overture)
              ├─ NCSS CSV (TerraClimate)
              ├─ TAR/ZIP central-dir read (DMSP-OLS, Köppen)
              └─ Overpass QL (WDPA)
                    │
                    ▼
           parse → compute → PrimaryFact OR signed Absence
                    │
                    ▼
           sign as responder + put_attestation (sled persist)
                    │
                    ▼
           return fact + Receipt (CID, signed_at, fn_key, source URL)
```

Per-upstream timeout: **30 s** (`EMEM_MATERIALIZER_TIMEOUT_SECS`, clamp 2..240).
Gateway timeout: **180 s** (`EMEM_TIMEOUT_SECS`, clamp 1..600). The latter
covers the whole `/v1/recall` round-trip including any temporal-recipe
fan-out (3+ windows × ~8 samples for `flood_risk@2` cold).

## Live connector inventory

The `emem-fetch` crate (16 modules, ~8.8 k LoC) handles the fetch half. Some
connectors live as inline materializers in `crates/emem-api-rest/src/lib.rs` —
they hit JSON REST endpoints directly without a dedicated fetch module. Both
are listed below.

| Connector             | Where         | Source                    | Read path                                         | Bands populated                                                                 | License                          |
|-----------------------|---------------|---------------------------|---------------------------------------------------|---------------------------------------------------------------------------------|----------------------------------|
| `chirps.rs`           | emem-fetch    | UCSB CHC (data.chc.ucsb.edu) | HTTPS Range on daily COG (~6.5 MB f32, NoData=-9999) | `chirps.precip_daily_mm` (±50° lat, 0.05°, 1981→present, ~30 d final-quality lag) | Public domain (UCSB CHC)         |
| `cog.rs`              | emem-fetch    | (any COG source)          | HTTPS Range, Deflate(8)/LZW(5), predictor 1/2/3   | sampler used by every raster connector                                          | (per source)                     |
| `connectors.rs`       | emem-fetch    | dispatcher                | reqwest + Range, no gzip, 90 s pool-idle          | (HTTPS / GCS / IPLD plumbing)                                                   | n/a                              |
| `dmsp_ols.rs`         | emem-fetch    | NOAA NCEI V4              | TAR head + gzipped TIFF, cached locally          | `nightlights.dmsp_ols_avg_dn`                                                   | Public domain (NOAA NGDC)        |
| `firms.rs`            | emem-fetch    | NASA FIRMS bulk CSV       | HTTPS GET + 60-min `RwLock` cache                 | `firms.active_fires`                                                            | Public domain (NASA)             |
| `hansen_gfc.rs`       | emem-fetch    | earthenginepartners GCS   | COG range, LZW strips, 10° tiles                  | `forest_change.{lossyear, treecover2000, gain}`                                 | Public (Hansen et al.)           |
| `koppen.rs`           | emem-fetch    | Figshare ZIP              | central-dir partial + cached PackBits TIFF        | `climate.koppen_geiger_present_day`                                             | CC-BY-4.0                        |
| `overture.rs`         | emem-fetch    | Overture S3 (us-west-2)   | Parquet row-group prune + WKB centroid            | `overture.{buildings,places,transportation}.count`, `overture.transportation.road_length_m` | ODbL / CDLA-Permissive-2.0       |
| `terraclimate.rs`     | emem-fetch    | UI THREDDS NCSS + RDA     | CSV REST, packed-int unpacking                    | `terraclimate.{ppt,tmax,tmin,vap,pet,aet,…}_normal`                             | CC0 / public domain              |
| `worldpop.rs`         | emem-fetch    | WorldPop /v1/services/stats | JSON REST (sync, 2-4 s/cell)                    | `population.count`, `population.density_mean`                                   | CC-BY-4.0                        |
| `wdpa.rs`             | emem-fetch    | OSM Overpass              | QL `is_in` server-side point-in-poly              | `protected.is_protected_area`                                                   | ODbL (OSM)                       |
| `stac.rs`             | emem-fetch    | Element84 / MS PC         | POST `intersects: Point`, SAS-token cache         | (scene discovery for S2/S1/Cop-DEM/Landsat)                                     | Copernicus open / per-scene      |
| `proj.rs`             | emem-fetch    | (no network)              | hand-rolled WGS84↔UTM (Snyder 1987)               | feeds Sentinel pixel sampling                                                   | n/a                              |
| `template.rs`         | emem-fetch    | (no network)              | URL `{var}` interpolation                         | feeds dispatcher                                                                | n/a                              |
| `cache_window.rs`     | emem-fetch    | (no network)              | in-flight fetch coalescing via `tokio::Notify`    | suppresses duplicate concurrent fetches                                         | n/a                              |
| inline `gmrt`         | api-rest:10235| NOAA GMRT PointServer     | JSON REST                                         | `gmrt.topobathy_mean`                                                           | CC-BY-4.0 (MGDS)                 |
| inline `ornl_modis`   | api-rest:12313| ORNL DAAC TESViS          | JSON REST                                         | `modis.{ndvi_mean, lst_day_8day, lst_night_8day, et_8day, gpp_8day, lai_8day, burned_area_monthly}` | Public domain (NASA / ORNL DAAC) |
| inline `nasa_power`   | api-rest:11510| NASA POWER                | JSON REST                                         | `power.{t2m, t2m_min, t2m_max, precip, rh2m, allsky_sw, ws10m}`                  | Public domain (NASA POWER)       |
| inline `weather`      | api-rest:11351| MET Norway locationforecast| JSON REST (no key, ECMWF + EUMETSAT-fed)         | `weather.{temperature_2m, cloud_cover, precipitation_mm, wind_speed_10m}`        | NLOD 2.0                         |
| inline `cams`         | api-rest:11668| Open-Meteo air-quality    | JSON REST                                         | `cams.{pm25, pm10, no2, o3, so2, co, aod_550}`                                  | CC-BY-4.0                        |
| inline `era5`         | api-rest:11798| Open-Meteo era5 archive   | JSON REST                                         | `era5.*` history bands                                                          | CC-BY-4.0                        |
| inline `marine`       | api-rest:11917| Open-Meteo marine API     | JSON REST                                         | `marine.*` wave/SST scalars                                                     | CC-BY-4.0                        |
| inline `soilgrids`    | api-rest:14490| ISRIC SoilGrids 2.0       | JSON REST (one call per property)                 | `soilgrids.{soc, phh2o, clay, sand, bdod, nitrogen}_0_30cm`                      | CC-BY-4.0                        |
| inline `viirs.fire.nrt` | api-rest:14278| NASA FIRMS (delegates to firms.rs) | bulk CSV via `firms.rs`                  | `firms.active_fires`                                                            | Public domain (NASA)             |
| inline `s2`           | api-rest:12880| Element84 STAC + COG      | STAC search ≤40 % cloud, ≤30 d lookback, then `cog.rs` | `s2.B01..B12`, `s2.B8A`, `s2.scl`, `indices.{ndvi,ndwi,mndwi,evi,nbr,ndmi,savi,bsi,ndbi}` | Copernicus open data             |
| inline `s1`           | api-rest:13219| MS PC STAC + COG          | STAC + SAS asset, dB conversion `10·log10(VV)`    | `sentinel1_raw` (VV slot)                                                       | Copernicus open data             |
| inline `geotessera`   | api-rest:10609| dl2.geotessera.org        | HTTP range on .npy (~640 B/cell)                  | `geotessera`, `geotessera.{2017..2024}`, `geotessera.multi_year`, `geotessera.bin128` | Apache-2.0                       |
| inline `cop_dem`      | api-rest:10091| Open-Meteo elevation      | JSON REST (Cop-DEM 90 m wrap)                     | `copdem30m.elevation_mean` (Primary on land, Absence over water)                | Public domain (Open-Meteo)       |
| inline `jrc_gsw`      | api-rest:13323| JRC GSW v1.4 GCS COG      | Range read via `cog.rs`                           | `surface_water.recurrence`                                                      | JRC open                         |
| inline `esa_worldcover` | api-rest:13625| ESA WorldCover S3 COG    | Range read via `cog.rs`                           | `landcover.{tree,shrub,grass,crop,built,bare,snow_ice,water}`                   | CC-BY-4.0                        |
| inline `prithvi_eo2`  | api-rest:10712| Prithvi-EO-2.0-300M-TL    | Python sidecar (frozen pretrained encoder)        | `prithvi_eo2.*`                                                                 | Apache-2.0                       |
| inline `galileo`      | api-rest:10823| Galileo Tiny              | Python sidecar (S2 modality only wired)           | `galileo.*`                                                                     | (per upstream)                   |
| inline `temporal_diff`| api-rest:14944| (derives from parents)    | computes Δ from two `geotessera` parents          | `temporal_diff`                                                                 | n/a (derivative)                 |
| `ftw.rs`              | emem-fetch    | Fields of The World (source.coop) | PMTiles HTTP range reads + MVT decode             | `/v1/field_boundaries` + `include:["ftw_fields"]` on `/v1/recall_polygon` (~3.17 B field polygons, 10 m, 241 countries) | CC-BY-4.0                        |
| `geonames.rs`         | emem-fetch    | GeoNames cities-5000 (embedded)   | `include_bytes!` 5.5 MB gzip → 68 581 records     | layer 3 of `/v1/locate` cascade (no network)                                    | CC-BY-4.0                        |
| `overture.rs` divisions | emem-fetch  | Overture `divisions/division_area`| Parquet row-group prune + WKB polygon decode      | polygon resolver for `/v1/locate` (replaces Nominatim polygon round-trip)       | ODbL                             |

The line numbers are at time-of-writing (2026-05-08); the `match` dispatcher
is at `try_materialize_bands` in `crates/emem-api-rest/src/lib.rs`. Grep
`fn materialize_` in that file for the live set.

## Connector deep dives

### cog.rs — the universal COG sampler

Pure-Rust point sampler (`crates/emem-fetch/src/cog.rs`, ~1 k LoC). Reads only
the IFD + the single tile that covers the requested pixel: a Sentinel-2 scene
is ~1 GB but a per-cell recall touches a few hundred KB.

Wire-level steps:

1. Range-read the first 64 KiB of the COG.
2. Parse TIFF header + IFD0 entries.
3. Pull `TileOffsets` / `TileByteCounts` from the buffer; refetch a wider
   header window if the IFD's external arrays sit past 64 KiB (bounded 8-retry
   loop). JRC GSW's 40 000 × 40 000 px tile has its IFD ~86 MB in; the loop
   handles it.
4. Compute world↔pixel transform from `ModelPixelScale` + `ModelTiepoint`.
5. Caller passes a world coord already in the COG's CRS (WGS84↔UTM is
   `proj.rs`'s job); we map to `(col, row)`, find the containing tile,
   range-read it, decompress, undo predictor, extract one pixel.

What's supported:

- Compression: Deflate(8), LZW(5).
- Predictor: 1 (no predictor), 2 (horizontal differencing), 3 (FP shuffle —
  the MS PC Sentinel-1 RTC f32 backscatter format).
- Bits per sample: 8, 16, 32.
- Endian: little-endian only (Sentinel-2 / -1 ship LE).
- Planar config: chunky only.

What's NOT supported:

- BigTIFF.
- JPEG2000 / WebP / CCITT compressions.
- Big-endian TIFF.
- Planar separation (each band in its own strip stack).

Strip-mode is synthesized as one-row tiles so Hansen GFC's 40 000-strip
TIFFs read through the same sampler as Sentinel-2's tile-mode COGs.

### overture.rs — parquet vector aggregation

Anonymous S3 (`overturemaps-us-west-2`, region `us-west-2`), monthly
snapshots. The pure-Rust read path:

- Per-release file list fetched once via S3 ListObjectsV2 XML and held in
  memory.
- Per-file parquet footer (1-2 KB) cached on first touch.
- Per-row-group bbox stats (`xmin/xmax/ymin/ymax`) used to prune row groups
  before any geometry decode — a 10 m cell typically reads one row group.
- WKB decoder hand-rolled: Point (places), LineString (transportation
  segments), Polygon centroid (buildings). No GEOS, no GDAL, no PyO3.

`EMEM_OVERTURE_RELEASE` pins a release tag; otherwise the connector
auto-discovers the latest with a 24 h TTL on the S3 ListObjectsV2 call
(`overture.rs:66`, `RELEASE_TTL`).

What's NOT supported: full Polygon / MultiPolygon decoding, divisions theme,
mixed-CRS data (Overture is WGS84 throughout).

### terraclimate.rs — NCSS CSV with failover

University of Idaho Climatology Lab's TerraClimate is published as NetCDF-4
with chunked (gzip + shuffle) variables. NetCDF chunk indices sit deep in the
file and the chunks are deeply filtered, so a clean COG-style range read is
not possible. The connector instead uses the **NetCDF Subset Service (NCSS)**
that THREDDS exposes — a CSV REST surface that returns one row per timestep:

```text
http://thredds.northwestknowledge.net:8080/thredds/ncss/grid/
  agg_terraclimate_<var>_1950_CurrentYear_GLOBE.nc
  ?var=<var>&latitude=<lat>&longitude=<lng>
  &time_start=1991-01-01T00:00:00Z&time_end=2020-12-01T00:00:00Z
  &accept=csv
```

NCSS returns the **packed integer** stored in the NetCDF; the connector
applies the `scale_factor` / `add_offset` / `_FillValue` linear unpacking on
the client side (`PackedScale::unpack_real` at `terraclimate.rs:157`).

The 2026-05-08 hardening added failover. `NCSS_BASES`
(`terraclimate.rs:101-104`) is a two-element list — UI primary, NCAR RDA
secondary — and the connector tries them in order on Transport errors. The
receipt's `Source.url` records which mirror actually answered, so a verifier
can replay against either.

`NORMAL_WINDOW = (1991, 2020)` (`terraclimate.rs:80`) is the WMO-standard
30-year normal currently in force. Bumping to 2001-2030 in 2031 requires
editing this one tuple.

What's NOT supported: yearly anomalies (the connector returns the 1991-2020
mean only); raw monthly time series for arbitrary years (NCSS supports it,
but no materializer is wired); upstream NetCDF range reads.

### worldpop.rs — REST not COG

WorldPop publishes its global mosaic as a single ~870 MB GeoTIFF at
`data.worldpop.org/.../ppp_2020_1km_Aggregated.tif`. The hosting Apache
instance advertises `Accept-Ranges: bytes` but does **not** actually honour
`Range:` requests — every request returns `HTTP/1.1 200 OK` with the full
`Content-Length: 869715253` body instead of `206 Partial Content`. Verified
live with `curl -v` against multiple sub-ranges. That makes `cog.rs`
unusable here.

The connector falls back to WorldPop's **synchronous `/v1/services/stats`
JSON endpoint** (`runasync=false`). For each per-cell recall it sends a
~1 km × 1 km AOI and gets back `{"data":{"total_population":<n>}}` in 2-4 s.
The integration covers ~100 100-m WorldPop pixels around the cell centre,
giving "people inside the 1 km² window centred on this cell" which equals
"density at this cell, persons · km⁻²".

Honest empties: `total_population == 0` over ocean / unpopulated terrain
surfaces as `WorldPopError::EmptyAoi` so the materializer can sign Absence
rather than persist a synthetic zero. The 2020 vintage is hardcoded today.

What's NOT supported: any year other than 2020; the constrained product (the
synchronous endpoint serves the unconstrained dataset); polygon AOIs (only
the cell-centred 1 km² window is wired).

### dmsp_ols.rs — TAR + cached TIFF, frozen 1992-2013

NOAA NCEI archive at `www.ngdc.noaa.gov/eog/data/web_data/v4composites/`.
Each `F<sat><year>.v4.tar` is a USTAR archive whose first entry is the
216-byte TFW (world file) and second entry is a gzipped average-visible-band
radiance TIFF (~100 MB compressed → ~692 MB uncompressed: 43 200 × 16 800 px,
1 byte per pixel uint8).

Wire path:

1. Range-read first 2 KiB of the tar, parse both header blocks, capture TFW
   payload (gives pixel scale + tiepoint without parsing GeoTIFF tags) and
   the second entry's `comp_size` field.
2. Range-read the gzip payload `[1536, 1536 + comp_size - 1]`. We never
   download more than the avg_vis layer.
3. `flate2::read::GzDecoder::read_to_end` to inflate ~692 MB into memory.
4. Atomic-rename the inflated TIFF to
   `<EMEM_DATA>/cache/dmsp_ols_v4/F<sat><year>.avg_vis.tif`. Subsequent
   recalls bypass the network.
5. Per-cell sampling parses the cached TIFF (LZW + Predictor 2 + strip
   layout) via `cog::sample_pixel`.

Honest defaults:

- `avg_vis = 0` is the documented "no-light / background" sentinel — over
  open ocean, dark sky, or genuinely unlit terrain. That IS a meaningful
  Primary fact; downstream change-detection treats 0 as a real value. This
  is **different** from `population` semantics where 0 surfaces as Absence.
- Coverage `-65° S` to `75° N` (16 800 rows × 0.00833° per row); cells
  outside that window return Absence not zero.
- Default year 2013 (sensor F18, the most recent v4 publication).

### hansen_gfc.rs — strip TIFF, 10° tiles, three sub-bands

Source: `earthenginepartners-hansen` GCS bucket, version `v1.12` (the
2000-2024 annual update released 2025-05). Three `forest_change.*` sub-bands:

- `forest_change.lossyear`     (uint8, `0` = no loss, `1..=24` = years 2001..=2024)
- `forest_change.treecover2000` (uint8, 0..=100 % canopy cover at 30 m)
- `forest_change.gain`          (uint8 0/1, frozen 2000-2012 mask)

Tiles are 10° × 10° at 0.00025° (~30 m at the equator), 40 000 × 40 000 px
each. Naming convention is the **top-left corner** — `Hansen_GFC-2024-v1.12_<layer>_<lat>_<lon>.tif`
where lat is `00N`, `10S`, …, and lon is `000E`, `010W`, …. A point at
lat=-3.0, lng=-60.5 lives in tile `00N_070W` (top edge at 0°N, west edge at
70°W).

The TIFFs are stripped (one strip per row, 40 000 strips per file), LZW +
Predictor 1, single-band uint8. `cog.rs` synthesizes one-row tiles from the
strip tags, so the same sampler reads them.

Honest defaults: `lossyear = 0` is a meaningful Primary fact ("on-land pixel
with no canopy loss observed 2001-2024"); upstream tile not found
(Antarctica below 60°S; dataset bounded ±60° to ~80°N) returns
`HansenGfcError::TileNotFound`, materializer signs Absence.

### koppen.rs — ZIP central-dir partial + cached PackBits TIFF

Source: Beck et al. 2018 (Sci Data 5, 180214), Figshare ndownloader file
12407516. Published artefact is a ~71 MB ZIP containing twelve GeoTIFFs at
three resolutions. The 1-km present-day raster
(`Beck_KG_V1_present_0p0083.tif`, 43 200 × 21 600 px, uint8, PackBits) is the
canonical product for "what climate zone is this?".

Wire path:

1. Range-read the last ~64 KiB to locate the End-of-Central-Directory
   record, walk the central directory to the present-day TIFF entry,
   capture local-header offset + compressed/uncompressed sizes.
2. Range-read the local file header (~80 B) to skip past name + extra fields.
3. Range-read the deflate stream (~5.7 MB) and inflate to ~22 MB. The result
   is a small self-contained TIFF.
4. Atomic-rename to `<EMEM_DATA>/cache/koppen/Beck_KG_V1_present_0p0083.tif`.
5. Per-cell sampling PackBits-decodes the single 4 320 × 2 160 tile that
   contains the requested pixel and reads one byte. The integer 1..=30 maps
   to a Köppen-Geiger class string via the `KOPPEN_CLASSES` const at
   `koppen.rs:59`.

Honest defaults: pixel value 0 means "outside the land mask" (open ocean,
polar interior); fetcher returns `KoppenError::NoData` so the materializer
signs Absence rather than picking a default class.

### stac.rs — POST search with Point intersects

Two anonymous catalogs:

- **Element84** (`earth-search.aws.element84.com/v1/search`) — Sentinel-2
  L2A, Sentinel-1 GRD, Cop-DEM, Landsat, NAIP. Asset URLs are public AWS
  Open Data S3 paths.
- **MS Planetary Computer** (`planetarycomputer.microsoft.com/api/stac/v1/search`)
  — same S2/S1 surfaces plus `sentinel-1-rtc` (the only free RTC-format S1
  catalog with proper UTM-projected COG tiles). Asset URLs are Azure Blob
  URLs that need a free anonymous SAS token; the connector caches the token
  for 50 min.

The connector uses **`intersects: Point`**, not `bbox`. A bbox query can
match neighbouring tiles in MGRS overlap zones — a Point is unambiguous.

What's NOT supported: server-side reprojection, signed-URL chains beyond MS
PC's SAS token, any catalog requiring an account.

### wdpa.rs — OSM Overpass workaround

The Protected Planet REST API (`api.protectedplanet.net/v3`) is the canonical
WDPA query surface but every endpoint, including read-only
`protected_areas?per_page=1`, returns `401 Unauthorized` even for anonymous
callers. The token is gated behind a Protected Planet account request, which
violates the no-key-gated-default-build rule.

The open substitute is **OSM `boundary=protected_area`** via the public
Overpass API. OSM crowdsources the WDPA designations; every PA carries a
`protect_class` integer that maps 1:1 to IUCN category codes (1 = Ia/Ib,
2 = II, …, 6 = VI). Overpass `is_in()` does point-in-polygon server-side and
returns the matching `area` records' tags as a small JSON document — perfect
for per-cell lazy materialise.

Verified live: a query centred on (44.4, -110.6) returns Yellowstone with
`protect_class=2`, `protected_area=national_park`, `wikidata=Q351`,
matching WDPA ID 374 883 (NPS official). A query centred on (30.0, -150.0)
in the North Pacific gyre returns zero matches.

Per-cell64 cache mitigates the 1 req/sec Overpass rate limit: every cell pays
at most one Overpass call, then hits the signed Primary or Absence forever.

`Ok(Some(WdpaMatch))` carries `wdpa_id + name + iucn_category + designation +
country_iso3 + marine`. `Ok(None)` is the meaningful confirmed-no answer (the
cell does not fall inside any OSM-mapped PA) — the materializer signs this
as a NegativeFact, not an unsigned silence.

What's NOT supported: the WDPA-specific fields outside what OSM tags surface
(GIS-area-from-shapefile, year of last assessment, IUCN management category
detail). Those would require Protected Planet auth.

### ftw.rs — agricultural field boundaries from Fields of The World

Fields of The World publishes a global product of ~3.17 billion agricultural-
field polygons (241 countries, 10 m resolution, CC-BY-4.0) as a single PMTiles
archive on Source Cooperative S3 (`data.source.coop`). emem reads it lazily:
the bundle is ~2.14 TB but each bbox query touches only the covering Web-
Mercator tiles.

Wire-level path:

1. Process-wide `AsyncPmTilesReader` against `global.pmtiles` with an
   in-memory `HashMapCache` for PMTiles directory entries.
2. Bbox → covering `(z, x, y)` range via the canonical OSM slippy-map
   formula. Auto-shrinks zoom (z=14 default, capped by archive `max_zoom`)
   when the polygon would exceed a 16-tile-per-query cap — each step down
   is a 4× tile reduction.
3. Each tile blob is decompressed by the PMTiles reader (gzip) and
   decoded by `mvt-reader` to vector features. Tile-local coordinates
   (0..extent) are projected to WGS84 with the inverse-Mercator formula.
4. The blake3 hash of concatenated decompressed tile bytes is the
   provenance source CID — a verifier replays the same tile fetch and
   reproduces it byte-for-byte.

Surface:

- `POST /v1/field_boundaries` — pure-fetch shape, returns a GeoJSON
  FeatureCollection + provenance (source CID, provider URL, license,
  attribution). Place name or polygon_bbox.
- `POST /v1/recall_polygon` with `include: ["ftw_fields"]` — supplements
  the per-cell fan-out response with the per-field polygons in one
  envelope.
- MCP tool: `emem_field_boundaries`.

License: CC-BY-4.0 on the global product. The benchmark CC-BY-NC
countries (Latvia, Portugal) are not consumed; only the uniformly
CC-BY-4.0 global product is read.

### geonames.rs — embedded populated-places gazetteer

The `/v1/locate` cascade resolves place names through five layers
(`wide_bbox_lookup` → `embedded_gazetteer_lookup` → **geonames** →
sled cache → Photon → Nominatim). The geonames layer is GeoNames'
cities-5000 cut: every populated place on Earth with population
≥ 5 000 (68 581 records as of 2026-05-11), embedded as a 5.5 MB gzip
in `crates/emem-fetch/data/cities5000.txt.gz` via `include_bytes!`
and decompressed + indexed once on first lookup into a static
HashMap keyed by ASCII-folded normalized name.

Lookup behaviour:

- ASCII diacritic fold ("São Paulo" ↔ "Sao Paulo" → same key).
- Every alternate name in column 3 is a lookup key too — `"Bombay"`
  hits Mumbai's record.
- Population-ranked disambiguation: when "Springfield" hits 17
  cities, the most-populous wins. The full candidate list is
  available via `lookup_candidates()` for ambiguity-aware callers.

A geonames hit emits `via: "geonames"` in the locate response.
Polygon geometry for the resolved place comes from
`overture.rs::division_polygon_near` (see below); no external
geocoder is touched for either coord or boundary.

For non-city named features (national parks, lakes, transboundary
basins, archipelagos), geonames is intentionally not the answer —
the cascade falls through to Photon / Nominatim.

License: CC-BY-4.0. Attribution string surfaces in every receipt
that hit this layer.

### overture.rs `divisions/division_area` — admin polygon resolver

Overture's divisions theme covers countries, regions, counties,
localities, boroughs, neighborhoods, and microhoods worldwide
(ODbL). The resolver
(`OvertureClient::division_polygon_near(lat, lng, name_hint)`):

1. Lists `theme=divisions/type=division_area` parquet shards once
   per release (already cached in `OvertureClient`).
2. For a query anchored at `(lat, lng)` with a name hint, builds a
   half-degree search bbox and prunes row groups via the parquet
   `bbox` column statistics.
3. Decodes each row's WKB Polygon / MultiPolygon, runs a point-in-
   polygon ray-cast against the anchor, and keeps every containing
   row.
4. Picks the best match by **exact-name match + admin-level rank**:
   normalized `names.primary == normalized hint` first, with the
   highest `subtype` rank (country > region > county > localadmin >
   locality > borough > dependency > macrohood > neighborhood >
   microhood) winning. So `"Manhattan"` lands on the borough
   (`locality` subtype), not on a neighborhood inside it. When no
   name match exists, the smallest containing polygon wins —
   best-effort behaviour for queries whose GeoNames primary string
   doesn't appear in Overture's vocabulary.
5. Memoizes the result in-process keyed by `(round(lat·100),
   round(lng·100), normalized_name_hint)`. First call cold ~5 s
   (footer prefetch over ~250 shards), warm ~12 ms. Cached `None`
   results too, so places Overture doesn't carry don't repeat the
   full scan on every retry.

The cascade uses this from three branches of `locate_inner`
(`embedded`, `geonames`, `cache`) — Overture is the preferred
polygon source; Nominatim is the long-tail fallback only when
Overture has no covering match.

Returned `DivisionMatch` carries the GERS id (citable source CID),
the primary name, subtype, the GeoJSON geometry, and the bbox tuple
in `(min_lat, max_lat, min_lng, max_lng)` order. Surfaced in
`/v1/locate` and `/v1/recall_polygon` as
`polygon_bbox.source: "overture_division_area"`.

## Genuinely unwired schemes (5)

These appear in `sources-v0.json` with templates but have no materializer in
api-rest, so a `/v1/recall` for their bands returns Absence today.

| Scheme                   | Effort      | Gain                                                | Status   |
|--------------------------|-------------|-----------------------------------------------------|----------|
| `openet.30m.daily`       | ~1 week     | CONUS 30 m daily ET (six-model ensemble)            | Deferred |
| `dynamic_world.v1`       | ~2 weeks    | 9-class probabilistic land cover at S2 cadence (~2-5 d revisit) | Deferred |
| `tropomi.s5p.ch4`        | ~1.5 weeks  | Methane column ~5.5 × 7 km (basin/cluster scale)    | Deferred |
| `tropomi.s5p.no2`        | ~1.5 weeks  | NO₂ tropospheric column ~5.5 × 3.5 km (traffic + power-plant proxy) | Deferred |
| `viirs.dnb.monthly`      | ~1 week     | 2012-present nightlights (extends DMSP 2013 freeze) | Deferred (auth-gated upstream — see below) |

CHIRPS daily precipitation is now wired (UCSB CHC anonymous COG range
read, ±50° lat, 0.05° pitch, 1981→present with ~30-day final-quality
lag). The connector lives at `crates/emem-fetch/src/chirps.rs` and
materialises the `chirps.precip_daily_mm` band via fn_key
`chirps.precip@1`. Two schemes — `sentinel1.grd.iw.vh` and
`sentinel1.grd.iw` (separate from `sentinel_s1_rtc_mpc`) — remain
declared without an api-rest materializer; both are one-channel
additions on top of the live VV path.

`viirs.dnb.monthly` is in the registry but its upstream
(`eogdata.mines.edu/...vcmcfg/...avg_rade9h.tif`) requires Earthdata Login
since 2024 — that's why DMSP-OLS V4 (1992-2013) is the wired nightlight
source today, even though VIIRS gives 2012-present coverage. A re-wire
requires either a no-auth mirror or a policy change on the API key rule.

## WorldPop / TerraClimate / DMSP gotchas

- **WorldPop**: Apache server lies about `Accept-Ranges: bytes`. Always 200 OK,
  never 206. COG path is unusable; REST `/v1/services/stats` is the only open
  per-cell path. 2-4 s/cell synchronous → throughput-bound on hot fan-out.
  Pre-baking the global 1 km² raster to a real range-served COG in S3 is the
  proper fix — deferred (~6-8 hr but requires infra decisions).

- **TerraClimate**: NetCDF chunk index too deep for cheap range reads, so
  NCSS CSV is the wire. NCSS returns packed integers, not unpacked floats —
  client-side `scale_factor` / `add_offset` is mandatory. Two mirrors
  (UI primary, NCAR RDA secondary) since 2026-05-08 — receipt records which
  mirror answered. Normal window is hardcoded `(1991, 2020)` and bumps when
  WMO redefines.

- **DMSP-OLS (the nightlights gotcha)**: The protocol's no-API-keys constraint
  rules out VIIRS DNB v22 (every path on `eogdata.mines.edu/.../v22/...`
  302-redirects to a Keycloak/OAuth login flow). NASA Black Marble VNP46A4 on
  LAADS DAAC redirects to Earthdata Login the same way. NOAA NCEI's
  `www.ngdc.noaa.gov/eog/data/web_data/v4composites/` is the only globally
  mirrored, anonymous, Range-readable annual nightlight time series — 200 OK,
  `accept-ranges: bytes`, 206 on Range, no `Set-Cookie`/redirect. **Coverage
  is 1992-2013 only**. The dataset is genuinely complete; NOAA officially
  handed live VIIRS DNB to EOG/CSM in 2019 and froze the v4 archive at the
  2013 boundary.

  `/v1/data_availability` surfaces this honestly via the materializer's
  `history_to_unix` metadata. A query for `tslot` post-2013 returns Absence
  with reason `outside_window` rather than fake-extending the time series.
  Comparing DMSP-OLS DN to VIIRS DNB radiance requires the Li & Zhou 2017
  intercalibration regression — DN is not the same physical unit as
  nW · cm⁻² · sr⁻¹, and naive year-over-year deltas across the 2013/14
  boundary are spurious.

## Lazy materialization config

| env var                          | default     | meaning                                                         |
|----------------------------------|-------------|-----------------------------------------------------------------|
| `EMEM_AUTO_MATERIALIZE`          | `1`         | enable on-miss fetch (set `"0"` or `"false"` to disable; on by default in production responder, off in isolated test environments) |
| `EMEM_MATERIALIZER_TIMEOUT_SECS` | `30`        | per-upstream timeout (clamped 2..240)                            |
| `EMEM_TIMEOUT_SECS`              | `180`       | gateway timeout (clamped 1..600)                                 |
| `EMEM_OVERTURE_RELEASE`          | (auto)      | pin an Overture release tag; otherwise auto-discover with 24 h TTL |
| `EMEM_SCAN_CELL_LIMIT`           | `10_000`    | per-cell index scan cap (`emem-cache::sled_hot::scan_cell`)      |
| `EMEM_COVERAGE_MATRIX_LIMIT`     | `50_000`    | row cap on `/v1/coverage_matrix` aggregation                    |
| `EMEM_TOPIC_BACKEND`             | `ort`       | `ort` (default) \| `model2vec` \| `keyword`                      |
| `EMEM_TOPIC_MODEL_DIR`           | `<EMEM_DATA>/models/bge-base-en-v1.5/` | where the ort backend reads tokenizer.json + model.onnx from |
| `EMEM_TOPIC_THRESHOLD`           | `0.35`      | cosine threshold for topic routing                               |
| `EMEM_TOPIC_USE_GPU`             | unset       | set to `1` to attempt CUDAExecutionProvider in `ort`             |

## Adding a new connector

1. **New module** under `crates/emem-fetch/src/<name>.rs`. Top doc-comment must
   spell out the wire path, the licence/auth posture, the empty-vs-error
   contract, and any rate limits. Read `dmsp_ols.rs` and `wdpa.rs` for the
   house style.
2. **Implement a `fetch_*` async fn** that returns `(value, url)` or a
   structured `Err`. Never invent default values; let an empty result
   surface as `Ok(None)` so the materializer can sign a NegativeFact, and
   let an upstream failure surface as `Err` so the caller can retry.
3. **Add an entry to `crates/emem-core/data/sources-v0.json`**: scheme key,
   ordered providers (failover order matters), tempo, native_resolution_m,
   licence string per provider. Recompile — the new sources CID is taken
   automatically.
4. **Add a band entry to `crates/emem-core/data/bands-v0.json`** if the
   connector populates a new band. The validator at `bands.rs:163-180`
   enforces contiguous offsets and the 1792-D total — pick an offset that
   fills a `_reserved_*` slot or extends the tail; never shift an existing
   offset.
5. **Wire the materializer in `crates/emem-api-rest/src/lib.rs`** (or as a
   dedicated module that lib.rs delegates to). Add a `match` arm in
   `try_materialize_bands` keyed on the band's scalar_keys. The materializer
   signs through the shared `sign_and_persist(s, fact, signed_at)` helper so
   the responder identity + Receipt shape stay uniform.
6. **Add a `band_materializer_meta` entry** with `history_from_unix`,
   `history_to_unix`, `kind`, `tempo`, `native_resolution_m`. This drives
   `/v1/data_availability` and `/v1/materializers` so agents discover what
   you wired without grepping the source.
7. **Register a function in `crates/emem-core/data/functions-v0.json`** with
   `kind: primary`, the source scheme you just added, the formula string,
   and `deterministic: true`. The materializer's signed Primary fact must
   pin this `fn_key` in its `derivation` so a verifier can walk back.
8. **Test**: `cargo test -p emem-fetch <name>` for the unit cases (the test
   suite uses recorded bytes — see `dmsp_ols.rs::tests` for the pattern).
   Live test with `--features live` to hit the real upstream once.

That's the whole contract. The eight-step path keeps the registry, the fetch
crate, and the api-rest dispatcher in lockstep; skipping any one of them
leaves a band that exists in the manifest but cannot be materialized, or a
fact attested under a `fn_key` no manifest knows about.
