# emem-fetch

`emem-fetch` reads open Earth-observation data from its publishers. It
holds one module per upstream dataset, a pure-Rust reader for COG
(cloud-optimised GeoTIFF) files, and the place-name tables that
`POST /v1/locate` searches.

You need this crate when you add a data source, fix how emem reads one,
or change how a place name becomes a point. To only use emem, you do not
need it: `POST /v1/recall` runs these readers for you on a cache miss.

| | |
|---|---|
| Version | 2.4.4 (workspace version in the root `Cargo.toml`) |
| Used by | `emem-api-rest`, `emem-cli`, `emem-primitives`, `emem-storage` |
| Tests | 254 unit tests inline in `src/` (208 `#[test]`, 46 `#[tokio::test]`), and 7 network tests in `tests/live_cog_fetch.rs` |
| Embedded data | 6 GeoNames files in `data/`, 27,961,590 bytes gzipped, compiled into the binary |

## Try it

Every signed fact names the file that a module in this crate read. This
fact is the elevation at Mount Fuji. The `copernicus_dem` module made the
URL, and the `cog` module read one pixel from it:

```sh
curl -s https://emem.dev/v1/facts/qejpz7cgvk347ordd6vrdtmwzprqlzcnkmotsjton2x4y2jjbplq \
  | jq -c '{band, value, url: .sources[0].url, fn: .derivation.fn_key}'
# {"band":"copdem30m.elevation_mean","value":3539.0422,
#  "url":"https://copernicus-dem-30m.s3.amazonaws.com/Copernicus_DSM_COG_10_N35_00_E138_00_DEM/Copernicus_DSM_COG_10_N35_00_E138_00_DEM.tif",
#  "fn":"copernicus_dem_30m_aws_pixel@1"}
```

The responder also serves its sources manifest. The manifest lists each
source scheme, its providers and their URL templates:

```sh
curl -s https://emem.dev/v1/sources | jq '.sources | length'
# 46
```

Both commands ran on 7 November 2026. We wrapped the output of the first
command to fit the page.

To run the tests from a checkout (needs Rust 1.91 or later):

```sh
cargo test -p emem-fetch --lib                 # no network
cargo test -p emem-fetch --test live_cog_fetch # reads real public files
```

We did not run `cargo test` for this page, because the host had no Rust
toolchain. The counts come from the test attributes in the source. 24
lines in `src/` mark a test `#[ignore]`, so a plain run skips those tests.
The network tests return early when `EMEM_NO_NETWORK=1`. They also return
early on a transport error, so a run without network still passes.

## How the responder reads upstream data

The crate has two paths. Only the second one runs on a cache miss today.

1. **The dispatcher.** `Dispatcher` picks a connector by
   `ConnectorKind`, expands the URL template from the sources manifest
   (`crates/emem-core/data/sources-v0.json`), and tries each provider in
   order. `emem-storage` builds a dispatcher with
   `register_default_https`. No code outside this crate's tests calls
   `Dispatcher::fetch`. On a miss, `emem-storage` returns
   `MaterializeMiss` (wire code `CidNotFound`) and does not fetch.
2. **Direct calls.** The inline materializers in
   `crates/emem-api-rest/src/lib.rs` (47 functions named
   `materialize_*`) call the dataset modules and `cog::open_profile`,
   `cog::sample_pixel` and `cog::sample_window` directly. They compute
   the value, sign it, and store it with `put_attestation`.

`register_default_https` registers 3 of the 7 connector kinds in the
manifest: `https_geotiff`, `https_cog_vsicurl` and `gcs_cog`. A `gcs_cog`
URL `gs://bucket/path` becomes `https://storage.googleapis.com/bucket/path`.
The `IpldConnector` always returns an error. An operator must register a
block store for it.

## What each module does

### Upstream datasets

| Module | Reads |
|---|---|
| `chirps` | CHIRPS daily precipitation |
| `copernicus_dem` | Copernicus DEM GLO-30 tile URLs on AWS Open Data |
| `dmsp_ols` | DMSP-OLS Stable Lights V4 night lights (NOAA NCEI) |
| `esa_cci_biomass` | ESA CCI Biomass v7.0 |
| `esa_worldcover` | ESA WorldCover 2021 v200 tile URLs |
| `firms` | NASA FIRMS active fires, from the bulk CSV that needs no key |
| `ftw` | Fields of The World field boundaries, from one PMTiles archive |
| `gmrt` | GMRT bathymetry and topography at one point |
| `hansen_gfc` | Hansen Global Forest Change |
| `jrc_gfc2020` | JRC Global Forest Cover 2020, the version under `LATEST/` |
| `jrc_gsw` | JRC Global Surface Water v1.4 tile URLs |
| `jrc_tmf` | JRC Tropical Moist Forest v2025 |
| `koppen` | Koppen-Geiger climate zones (Beck et al.) |
| `opera_dist` | NASA OPERA DIST-ALERT-HLS disturbance alerts. Needs an Earthdata token |
| `overture` | Overture Maps buildings, places and divisions, from public S3 parquet |
| `radd_alerts` | RADD deforestation alerts. Returns `NotImplemented`, see below |
| `terraclimate` | TerraClimate monthly climate and water balance |
| `wdpa` | Protected areas, from OSM Overpass `boundary=protected_area` |
| `worldpop` | WorldPop population, from the WorldPop statistics API |
| `wri_gdm_drivers` | WRI and Google DeepMind drivers of forest loss |

### Place names for `/v1/locate`

| Module | Holds |
|---|---|
| `countries` | ISO 3166 countries (`countryInfo.txt.gz`) |
| `admin1`, `admin2`, `admin3` | GeoNames first, second and third level divisions |
| `geonames` | GeoNames `cities1000` populated places |
| `pois` | GeoNames landmarks such as mountains, lakes and airports |
| `wikidata` | Wikidata search, used to rank candidates by prominence |

### Shared helpers

| Module | Does |
|---|---|
| `cog` | Reads one pixel or one window from a COG with HTTP range requests |
| `stac` | Searches STAC catalogues for Sentinel scenes |
| `proj` | Converts WGS84 to UTM for scenes in a UTM grid |
| `template` | Expands `{name}` variables in a manifest URL template |
| `connectors` | The HTTPS, GCS and IPLD connectors |
| `tile_store` | Keeps whole local copies of four static forest and land-cover rasters |
| `parcels` | Removes overlaps and duplicates in a set of field polygons |
| `cache_window` | `CoalesceWindow`, one shared fetch per key. No caller uses it |
| `latency` | `StageTimer`, which logs the time of each cold-read stage |

`cog` reads little-endian TIFF and BigTIFF with Deflate (8), LZW (5) or
ZSTD (50000) compression. It returns an error for any other layout. It
does not guess.

## Settings

| Variable | Module | Effect | Default |
|---|---|---|---|
| `EMEM_COG_TILE_CACHE_CAP` | `cog` | Raw tiles kept in memory | 512 |
| `EMEM_COG_DECODED_CACHE_CAP` | `cog` | Decoded tiles kept in memory | 64 |
| `EMEM_COG_DECODE_CONCURRENCY` | `cog` | Tile decodes that run at the same time | the CPU count |
| `EMEM_OVERTURE_RELEASE` | `overture` | Pins one Overture release | the latest release |
| `EMEM_OVERTURE_PARALLEL` | `overture` | Parallel parquet file reads, from 1 to 256 | 32 |
| `EMEM_EARTHDATA_TOKEN`, `EMEM_EARTHDATA_TOKEN_FILE` | `opera_dist` | NASA Earthdata token, or a file that holds it | none |
| `EMEM_DATA` | `jrc_tmf`, `koppen`, `dmsp_ols` | Root of the local cache for downloaded files | see each module |
| `EMEM_TILE_STORE_GB` | `tile_store`, read by `emem-server` | Disk budget for the local raster copies under `<EMEM_DATA>/tile_store`. `0` turns the store off | 40 |
| `EMEM_WRI_GDM_CACHE` | `wri_gdm_drivers` | Local copy of the drivers raster | none |
| `EMEM_WIKIDATA_API` | `wikidata` | Wikidata API URL | `https://www.wikidata.org/w/api.php` |
| `EMEM_NO_NETWORK` | network tests | `1` skips the network tests | not set |

The two cache caps clear the whole cache when it is full. To see where a
cold read spends its time, set `RUST_LOG=emem::latency=debug`.

## Rules for changes

- Do not invent a value. When an upstream has no data or is not
  reachable, return an error. The materializer then signs an `Absence`
  with a reason. `opera_dist` without a token and `radd_alerts` show the
  pattern.
- Keep the default build free of keys. `firms` reads the bulk CSV
  because the FIRMS area API needs a `MAP_KEY`. `wdpa` uses OSM Overpass
  because the Protected Planet API refuses anonymous calls.
- A fact cites the URL it read. `tile_store` serves a local copy only when
  the copy has the publisher's full length, so the URL stays the source.
- `tile_store` is off until the server calls `tile_store::enable`.
  `emem-server` calls it at start. A library user or a test never
  downloads a whole raster.

## What does not ship

- `radd_alerts::fetch_alert` returns `RaddError::NotImplemented`. The
  RADD rasters are in a Requester-Pays S3 bucket, so an anonymous read
  gets HTTP 403. The module comment dates this to 16 May 2026.
- `opera_dist` signs an `Absence` with reason `not_enabled` when the
  operator sets no Earthdata token.
- The crate ships the dispatcher path, the IPLD connector and
  `CoalesceWindow`, but no responder code path uses them. See
  [How the responder reads upstream data](#how-the-responder-reads-upstream-data).
- The manifest lists 3 `gpu_sidecar` providers (Prithvi, Clay,
  Galileo). emem.dev runs no GPU sidecar today. Facts those models
  signed before still verify.

## Read more

- [docs/developers/architecture.md](../../docs/developers/architecture.md):
  where this crate sits, and the list of connectors.
- [docs/developers/data-sources.md](../../docs/developers/data-sources.md):
  the upstream sources and how to add one.
- [crates/emem-storage/README.md](../emem-storage/README.md): the cache
  and log that store what the materializers sign.
- [crates/emem-codec/README.md](../emem-codec/README.md): the `cell64`
  address that each fact carries.
