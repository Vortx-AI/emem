# emem memory standard, v1

This page says what a good emem memory is, as checks a machine can run. It
then audits every kind of memory emem serves against those checks, ranks the
gaps by what they cost an agent, and records what changed.

Status: draft for the owner, 14 November 2026. Code base: commit `0d613cda`.
Every `file:line` below is at that commit. Live evidence comes from 9 read
requests to emem.dev on 14 November 2026 between 03:27Z and 03:36Z, at the
cell `defi.zb6f5.fUvE.xAtA` (Santa Claus Village, 66.544 N, 25.848 E).

## The criteria

A memory is one record an agent can cite: a fact, a derivation, a raster, a
bundle, an entity, a note or a chain. Each criterion has a rule, a check and,
where it has one, a target with a number.

| id | criterion | rule | check | target |
|---|---|---|---|---|
| M1 | Content address | The id is a full-length hash over a framed, canonical encoding of the whole record. The token resolves to the same bytes on every node that serves it. | Fetch the bytes, re-hash them, compare with the id. | 256-bit hash (52 base32 characters). No truncation, no hash over a part of the record. |
| M2 | Signed and in the log | The record's id is inside a signed preimage, and the record is appended to the RFC 6962 log, so inclusion is provable by id. | `GET /v1/log/inclusion?fact_cid=` returns `matched`. The receipt preimage, not an unsigned field, carries the id. | Every write. A write whose log append fails says so. |
| M3 | Estimand | The record states what was measured: the support (footprint, with its size), the statistic, the unit, and the source scene or pixel. | Each item is a field a program can read, not prose. | Support and statistic present for every primary fact. |
| M4 | Method | The record names a rule or function key that resolves to a rule text or code id. Its inputs are cited by content id. Its arguments are named. | Resolve the key. Re-run it on the cited inputs and get the value. | Every key resolves. No positional argument without a name. |
| M5 | Time | The record separates the time the value describes (valid time) from the time the source captured or released it and from `signed_at`. It says how long the value may be taken to hold. | Three distinct fields, plus a hold rule. | A release-dated layer never presents its release date as the time it describes. |
| M6 | Provenance and uncertainty | The record carries a provenance class (never absent) and an uncertainty in the value's own unit where the source publishes one. Where the evidence does not decide, it abstains. | A class on every record. An interval or sigma where the upstream has one. | No record without a class. |
| M7 | Corrections linked | A correction is a new record that names what it replaces. The old record stays resolvable. Nothing is overwritten under the same id. | Follow `replaced_by` or an erratum id from the old record to the new one. | No silent overwrite. Every known defect class has a served erratum. |
| M8 | Cheap to cite | One record has one short token. Many records fold into one token. | Measure token length. | 90 characters or fewer for one record. One token for up to 256 records. |
| M9 | Fast to read | A stored record reads back quickly. A cold read answers, or returns a retryable note, inside the cap. | Time the read. | Read by id: 100 ms or less. Cold read: an answer or a retryable note within the 14 s cap. |
| M10 | Nothing private | A responder-signed, logged record holds no caller identifier or free text unless the caller asked to publish it. Free text is served as data, never as instructions. | Read what the log stores. Look for the data-not-instructions marker on every free-text read. | No caller identifier in a signed public record by default. |

What changed from the starting list. M1 now asks for the whole record,
framed, at full length, because two kinds hash a part of the record or
truncate to 128 bits. M2 now asks for the id inside the signed preimage,
because two kinds carry their id only in an unsigned field. M5 now separates
valid time from capture time, because for release-dated layers
`captured_at` is the release. M6 now forbids an absent class, because 30
bands had none. M8 and M9 have numbers. M10 now covers free text that a
responder signs on a caller's behalf.

## How a fact meets M3 and M5 now

On branch `memstd-v2`, a fact on a single-cell recall and on a token
resolve carries an `estimand` block (schema `emem.estimand.v1`), outside
the signed bytes. It is declared for eight source families: Sentinel-2,
ERA5, ERA5-Land, Copernicus DEM GLO-30, JRC GFC2020, Hansen GFC, ESA CCI
biomass and SoilGrids. A fact from any other source has no block, which
means "not declared".

| field | meaning |
|---|---|
| `support.m` | Size of the source pixel or model grid cell along a meridian, in metres. For an index, the coarsest input. |
| `support.inputs` | Sentinel-2 only: each input band and its pitch. NDSI reads B03 at 10 m and B11 at 20 m, so its support is 20 m. |
| `support.deg`, `support.lng_deg` | The source grid in degrees, where the source is a degree grid. GLO-30 widens its longitude spacing north of 50 degrees. |
| `statistic` | `pixel_value`, `pixel_class`, `index_of_pixel_values`, `model_value`, `model_hour_sum` or `thickness_weighted_mean`. |
| `valid_time` | The period the value describes: an instant, an hour, a year, a period, or `edition` for a static product. |
| `captured_at_is` | What the source's `captured_at` means here: `sensing_time`, `model_hour`, `release_date` or `edition_date`. |

The pixel pitches come from the GeoTIFF headers of the source files, read
on 14 November 2026:

| source file | pixel scale | along a meridian |
|---|---|---|
| `JRC_GFC2020_V4_N70_E20.tif` | 1/12000 degree | 9.3 m |
| `Hansen_GFC-2025-v1.13_lossyear_70N_020E.tif` | 0.00025 degree | 27.8 m |
| `N70E020_ESACCI-BIOMASS-L4-AGB-MERGED-100m-2020-fv7.0.tif` | 3.2 arc-seconds | 99.0 m |
| `Copernicus_DSM_COG_10_N12_00_E077_00_DEM.tif` | 1 by 1 arc-second | 30.9 m |
| `Copernicus_DSM_COG_10_N66_00_E025_00_DEM.tif` | 1 arc-second north, 2 east | 30.9 m |

Every tile of one product has the same pixel scale, so these files are only
where the scales were read. A fact's `support.basis` names the product and
its pixel, and then the file that fact read, from the fact's own sources.

The ERA5 and ERA5-Land grids (0.25 and 0.1 degree) are the grids Open-Meteo
serves, as the materialiser states (`lib.rs:56500`). SoilGrids is 250 m, as
the bands registry states.

The same table now feeds `data_resolution_m` on aggregate responses
(`band_input_resolution_m`, `lib.rs:15370`). That function said 10 m for
every Sentinel-2 index and 11 km for every ERA5 band, while
`spatial_basis.per_band_resolution` said no mapping existed. Both now read
one table.

## The audit

Verdicts: **meets**, **partly**, **misses**. "Live" marks evidence read from
emem.dev on 14 November 2026. The rest is code.

### Primary facts (Sentinel-2 indices, ERA5, forest and EUDR layers, SoilGrids, DEM)

| criterion | verdict | evidence |
|---|---|---|
| M1 | meets | `fact_cid` is blake3 over the canonical CBOR of the whole fact (`docs/protocol.md` section 3.2). Live: `GET /v1/facts/4m3xiq...si2qq` returned 1,370 bytes. |
| M2 | meets | Live: `GET /v1/log/inclusion?fact_cid=4m3xiq...si2qq` matched leaf 3,331,396 of a 3,333,499-leaf signed tree head, with an 18-hash audit path. |
| M3 | partly | Source pixel: the coordinates, the reader stamp `cog-pixel-floor@2` and the asset URLs are signed. Unit: the signed `unit` is null on NDVI and NDSI (live). Support and statistic: none on the fact; `spatial_basis.per_band_resolution` said "not stated" (`lib.rs:83526`). Now declared for 8 families in `estimand` (response-level). |
| M4 | partly | Every fact names a `fn_key`, but 101 of 114 entries in the live `/v1/materializers` catalogue are `auto_generated` with no `derivation_fn_key`, and 8 of the other 13 are not among the 23 keys of the function registry (`crates/emem-core/data/functions-v0.json`). The catalogue names `sentinel2_l2a_ndvi@1` for NDVI; the live NDVI fact signs `sentinel2_l2a_indices_ndvi@1`. The NDSI fact signs 14 positional arguments with no names (live). |
| M5 | partly | `observed_at` and `signed_at` are separate. But `observed_at` is the source's `captured_at`, which for release-dated layers is the release (live): `jrc_gfc2020.forest_2020` said 2026-09-14 for a 2020 map, `esa_cci_biomass.agb_t_per_ha_2020` said 2025-03-01, the DEM said 2021-04-30. The descriptor token binds its date to `captured_at` as "valid time" (`lib.rs:36962`). No hold rule; `include:["freshness"]` gives an advisory decay score (`lib.rs:90178`). |
| M6 | partly | A class rides in `band_metadata.provenance`. Live: `era5.t2m` and `era5.snow_depth` had no block at all; 30 of 114 catalogue bands have no registry family (now fixed, see below). `uncertainty` is filled by 2 materialisers only, SoilGrids (interval, `lib.rs:61560`) and CCI biomass (sigma, live). `confidence` is a fixed number per band (0.95 for Sentinel-2, 0.7 for ERA5-Land snow depth). |
| M7 | partly | Two served errata name a fresh fact (`element84_erratum.rs:1`, `:150`). A JRC GFC2020 V3 fact has no link to its V4 successor; V3 URLs now answer 404 (`lib.rs:15663`). |
| M8 | meets | `emem:fact:<cell64>:<fact_cid>` is 83 characters. |
| M9 | partly | Live: by id 25 ms, token resolve 29 ms, inclusion proof 29 ms. A first recall of 9 bands took 14.0 s and returned 2; 7 bands hit the 14 s cap with `retryable: true`. The third call returned all 8 asked in 6.3 s. |
| M10 | meets | Every fact carries `privacy_class: "public"`; no caller text enters a fact. |

### Absence facts

| criterion | verdict | evidence |
|---|---|---|
| M1, M2, M8 | meets | Same id, token and log as a primary fact. |
| M3 | meets | Live: the `jrc_tmf.annual_change` absence states its reason, "outside the 30 degree tropical belt", with a `reason_cid`. |
| M5 | partly | Live: its `observed_at` is the signing time; it says when emem looked, not a period. |
| M6 | meets | Class from the band; `confidence: 1.0` on a coverage gap. |
| M7 | partly | Nothing links an absence to a reading that later replaces it. |

### Derivations (`POST /v1/derive`, `/v1/derived`, `/v1/inferences`)

| criterion | verdict | evidence |
|---|---|---|
| M1 | meets | Full 52-character cid over the whole derivative fact (`crates/emem-cache/src/sled_hot.rs:1010`). |
| M2 | meets | Signed by the responder over the caller's signature (`lib.rs:43539`); logged through `put_attestation` (`crates/emem-storage/src/lib.rs:1129`). |
| M3 | partly | `band`, `op` and `tslot_window` are stored (`lib.rs:43522`); no unit; `band` is only checked for being non-empty (`lib.rs:42945`). |
| M4 | partly | Parents are cited by cid and checked before the write (`lib.rs:43033`). The responder recomputes only when a `code_cid` is present (`lib.rs:43164`), and never checks the `code_cid` against code. |
| M5 | partly | `tslot_window` is the caller's and is not compared with the parents' tslots (`lib.rs:43102`). |
| M6 | meets | The caller may declare only model_output, human_curated or estimator; `deterministic_index` is awarded only after a recompute (`lib.rs:43293`). |
| M7 | misses | No supersede link; `/v1/inferences` shows only the newest per cell, function and attester (`lib.rs:44293`), so a correction hides the old value without a link. |
| M8 | meets | 83 to 84 characters. |
| M10 | partly | `value` is arbitrary JSON and is logged. `/v1/inferences` marks it as data (`lib.rs:44311`); `/v1/memory_token/resolve` does not. |

### Field and raster tokens (`emem:raster:`, `emem:cube:`)

| criterion | verdict | evidence |
|---|---|---|
| M1 | partly | Grid bytes have a full blake3 `artifact_cid` (`crates/emem-storage/src/artifacts.rs:37`). The token's `derivation_cid` hashes the whole record including `signed_at` (`band_raster.rs:473`), so the same pixels minted twice give two tokens. |
| M2 | meets | Responder-signed, logged, and the field receipt binds `aoi_cid` and `derivation_cid` (`band_raster.rs:475`, `:497`). |
| M3 | partly | Grid size, EPSG, origin, step, scene id and cloud cover are recorded (`band_raster.rs:420`). A Sentinel-2 grid has no unit field; its values are harmonised DNs (`band_raster.rs:60`). |
| M4 | partly | Pixels re-hash through `GET /v1/artifacts/:cid` (`band_raster.rs:2088`). The advertised rebuild picks the nearest scene again rather than the pinned item (`band_raster.rs:2115`). |
| M5 | partly | Scene day in `tslot`; `captured_at` apart from `signed_at`; no hold rule. |
| M6 | misses | `confidence: 1.0` fixed (`band_raster.rs:466`); no class, no uncertainty. |
| M7 | misses | No supersede link; the Element84 erratum is not applied on raster resolve. |
| M8 | partly | Tokens are 120 to 135 characters. The artifact store holds 4 GiB and evicts (`band_raster.rs:115`), so the pixels behind an old token can go. |
| M10 | partly | The caller's bounding box is in a public, logged record (`band_raster.rs:422`). |

### Memory bundles (`emem:bundle:`)

| criterion | verdict | evidence |
|---|---|---|
| M1 | misses | The cid is blake3 over unframed text, `purpose`, a newline, then one line per member of cell, band, tslot and cid joined by a bar, cut to 16 bytes (`crates/emem-primitives/src/memory_bundle.rs:159`). A `purpose` that ends in a newline and a member line hashes like a bundle with that member. The module doc says canonical CBOR (`memory_bundle.rs:7`). |
| M2 | misses | The receipt binds only cells and fact cids, not the bundle cid (`crates/emem-storage/src/server.rs:366`). The record lives in a sled tree and is never logged (`lib.rs:45338`). |
| M3 | partly | Each member names cell, band and tslot; the bundle states nothing of its own. |
| M5 | partly | `as_of` is accepted but neither hashed nor stored. |
| M6 | misses | No class at bundle level. |
| M7 | misses | Posting again overwrites the stored record (`lib.rs:45342`); resolve does not recompute the cid. |
| M8 | meets | 38 characters for up to 256 members (`lib.rs:45152`). |
| M10 | misses | `purpose` is free text, unbounded, returned without the data marker. |

### Raster bundles (`emem:rasterset:`)

| criterion | verdict | evidence |
|---|---|---|
| M1 | meets | Full blake3 over a canonical CBOR array; resolve recomputes it and refuses a mismatch with 409 (`band_raster.rs:1818`, `:2033`). |
| M2 | meets | A responder-signed derivative fact through `put_attestation` (`band_raster.rs:1902`). |
| M4 | meets | `raster_bundle@1`, members as parents, re-checked on resolve (`band_raster.rs:1907`, `:2045`). |
| M6, M7 | misses | `confidence: 1.0`; no supersede. |
| M10 | misses | An anonymous caller's `purpose` text is signed by the responder and logged. |

### Entities (`emem:entity:`)

| criterion | verdict | evidence |
|---|---|---|
| M1 | misses | blake3 cut to 16 bytes, 26 characters (`crates/emem-primitives/src/entity.rs:251`). With an external id, only that id is hashed. Geometry is in neither the cid nor a signature, and is altered on read (`lib.rs:45788`). |
| M2 | misses | The mint receipt carries `entity_cid` only in `intent` (`lib.rs:46146`), which the v2 preimage does not cover (`server.rs:366`); the signature covers the cell. Records live in sled and are never logged. |
| M4 | partly | `resolved_via` names the geocoder path; a geocoder lookup cannot be re-run to the same answer. |
| M5 | misses | Only `minted_at`. |
| M7 | partly | Aliases append and dispute; nothing is deleted. A re-mint returns the first record, so the first minter owns an external id even with a wrong label (`lib.rs:46110`). |
| M8 | meets | 38 characters. |
| M10 | partly | `/v1/entity/resolve` marks content as data (`lib.rs:46365`); `GET /v1/entity/:id` and the mint response do not. |

### Notes (`memory_*`)

| criterion | verdict | evidence |
|---|---|---|
| M1 | partly | `file_cid` is blake3 cut to 16 bytes, 26 characters (`lib.rs:46938`). A path citation is mutable. |
| M2 | partly | The caller's signature is stored and production refuses unsigned writes (`lib.rs:47100`). A failed log append is logged as a warning and the write still succeeds (`lib.rs:48940`). |
| M3, M4 | misses | Free text; no estimand, no method. |
| M5 | partly | `signed_at` only. |
| M6 | misses | No class or uncertainty field. |
| M7 | meets | History is append-only, delete leaves a tombstone, old versions resolve by cid, and supersede is signed and logged (`lib.rs:48894`, `:50578`). |
| M8 | meets | 26 characters. |
| M10 | partly | Notes are a world-readable commons by design. `GET /memories/*path` returns bare markdown without the data marker (`lib.rs:49257`), against the claim at `lib.rs:9372`. Vault entries seal under a key only the operator holds (`vault.rs:12`). |

### Persistence chain (`POST /v1/persist`, `persistence_snow@1`)

Code only; no chain was minted for this audit.

| criterion | verdict | evidence |
|---|---|---|
| M1, M2, M8 | meets | A responder-signed derivative fact, stored through `put_attestation`, cited as an 83-character `emem:fact:` token (`persist.rs:610`, `:626`, `:648`). |
| M3 | partly | Quantity, window and the series grid are fields; the model grid sizes are prose inside `source` strings (`persist.rs:427`, `:441`). |
| M4 | meets | The rule text and every parameter are in the record (`persist.rs:54`, `:584`). The observation and the depth fact are parents. The hourly series is an artifact by cid (`persist.rs:565`), in the same evictable store as rasters. |
| M5 | meets | Window from and to, the observation's time, the depth fact's hour and `signed_at`. |
| M6 | partly | `model_output` with a caution (`persist.rs:594`). `confidence` is a fixed 0.6 (`persist.rs:617`). It abstains with UNKNOWN in six named cases (`persist.rs:287`), which is the right shape for uncertainty without a number. |
| M7 | misses | A second chain for the same cell and date is a new fact with no link to the first. |
| M10 | meets | Public reanalysis and a public scene only. |

### EUDR per-cell facts and the DDS (`POST /v1/eudr_dds`)

| criterion | verdict | evidence |
|---|---|---|
| M1 | partly | Per-cell facts have full cids. The DDS has no content address: its `request_id` is a random nonce (`lib.rs:78530`), and no route reads a DDS back. |
| M2 | partly | Per-cell facts are logged. The DDS receipt signs request id, time, cells and fact cids, not the verdict, the operator or `per_cell_rows_cid` (`lib.rs:74548`). |
| M3 | partly | One pixel at the cell centre, unit `boolean_eudr_forest_2020` (`lib.rs:61800`); support now in `estimand`. |
| M4 | partly | Function keys are versioned (`jrc_gfc2020_v4_pixel@1`). No source pixel hash (`lib.rs:61807`); V3 facts cannot be re-run because V3 URLs answer 404. |
| M5 | partly | `captured_at` is the release date (`lib.rs:61808`); the 2020 valid year is now in `estimand`. |
| M6 | partly | Class from the band at read time; confidence fixed at 0.88 and 0.93. |
| M7 | partly | Version in the scheme and function key; no V3 to V4 link. |
| M10 | misses | The loss-year histogram fact signs the caller's `plot_id` at the plot's centroid cell, even with `geolocation_confidential` set (`lib.rs:78093`, `:78542`). It is logged, and `/v1/log/entries` serves the CBOR to anyone. |

## The gaps, ranked by cost to an agent

| rank | gap | what it costs an agent | fix | size |
|---|---|---|---|---|
| 1 | M3 and M5 on primary facts: no support, statistic or valid time; release dates shown as observation times | Wrong meaning with a valid signature: a 20 m index read as 10 m, two cells of one pixel differenced, a 2020 forest map dated 2026 | Response-level `estimand` for 8 families (done). Then extend the table family by family, and decide whether to sign it (owner). | Done: about 340 lines of code and 140 of tests. Each further family: 10 to 30 lines. |
| 2 | M1 and M2 on bundles and entities: 128-bit, unframed or partial hashes, ids outside the signed preimage, no log, silent overwrite | A cited `emem:bundle:` token can resolve to members the citer never saw; an entity's geometry is not bound by anything | A v2 cid over canonical CBOR at 256 bits; store each as a responder-signed derivative fact like `emem:rasterset:`, so it is logged and the receipt binds it; keep v1 resolvable | Medium: about 300 lines per kind, a new token version, docs. Owner. |
| 3 | M10 in EUDR: a caller's plot id and its location in a signed, permanent public record | A supplier's plot list leaks to anyone who reads the log, and the log cannot forget | Stop signing `plot_id` in new histogram facts, or sign a salted hash; say so in `docs/eudr.md` | Small in code; it changes signed bytes of new facts. Owner. |
| 4 | M6: 30 catalogue bands had no provenance block (ERA5, NASA POWER, marine, MODIS products, FIRMS, temporal differences) | A reanalysis or a model read as an observation | Response-level block with the manifest's own class, `unclassified`, and an `upstream` note (done). Families in the bands manifest (owner, changes `bands_cid`). | Done: about 40 lines with tests. |
| 5 | M4: methods do not resolve | No third party can re-run a value from its key: 101 of 114 catalogue entries name no function, 8 of the other 13 are not in the function registry, and Sentinel-2 facts sign 14 unnamed arguments | Fill the catalogue's `derivation_fn_key` from each materialiser; serve `GET /v1/functions/:key` with a named argument legend per key family | Medium: about 200 lines plus one legend per family. |
| 6 | M7: corrections are not linked outside two erratum classes | An agent holding a superseded value cannot learn it was superseded | A general `replaced_by` index keyed on cell, band and function family, served like the Element84 erratum; supersede for derivations and chains | Medium. |
| 7 | M6 uncertainty: 2 materialisers fill `uncertainty`; `confidence` is a constant per band | No error bar to carry into a decision | Fill `uncertainty` where the upstream publishes one (ERA5 ensemble spread, Hansen and GFC2020 accuracy tables) | Medium; changes new facts' bytes. Owner. |
| 8 | M2 on notes: a failed log append still succeeds | A note's inclusion cannot be proved, and the response may not say so | Return `logged: false` with the reason, or refuse the write | Small. |
| 9 | M9 cold reads: 7 of 9 bands hit the 14 s cap at a warm cell | Two or three round trips per question | Prewarm by place on locate; the retry notes already exist | Medium. |
| 10 | M1 and M2 on the DDS: no content address, verdict not signed | A filed statement cannot be cited or re-read by id | Store the DDS as a responder-signed derivative fact whose value is the verdict table | Medium. Owner. |

## What changed on this branch

1. Facts on a single-cell recall (and any surface that inlines band
   metadata) and on `/v1/memory_token/resolve` carry `estimand`
   (`crates/emem-api-rest/src/estimand.rs`). It is outside the signed bytes:
   the fact, its cid and the receipt preimage are unchanged.
2. `band_input_resolution_m` reads the same table, so `data_resolution_m`
   on aggregates now says 20 m for an index over a 20 m band, 27,830 m for
   ERA5 and 11,132 m for ERA5-Land.
3. A band with no family in the bands manifest gets a provenance block
   with class `unclassified`, which is what the recall filter already uses,
   and an `upstream` sentence naming the kind of process for ERA5, NASA
   POWER, marine, MODIS products and FIRMS.
4. `spatial_basis.per_band_resolution` points at `estimand.support`.

## Decisions for the owner

1. **Sign the estimand?** The block is response-level today. Putting
   `support`, `statistic` and `valid_time` into the bands registry changes
   `bands_cid`; putting them into new facts changes their cids. Both need
   `docs/protocol.md` and `web/humans.html` changed in step. The registry is
   the cheaper of the two: one wire change, and every fact inherits it.
2. **Bundle v2 and entity v2.** Full 256-bit ids over canonical CBOR, logged,
   bound by the receipt. Old tokens keep resolving. This adds token
   versions.
3. **Manifest families for the 30 unclassified bands.** The class would then
   be declared and pinned by `bands_cid`, and `provenance` filters would find
   them. Until then they stay `unclassified` with an `upstream` note.
4. **EUDR `plot_id`.** Stop signing it, or sign a salted hash, for new
   histogram facts. The log is append-only, so the ones already signed stay;
   decide whether to publish an erratum that says so.
5. **Descriptor tokens and release dates.** A descriptor token binds its date
   to `captured_at`, which for release-dated layers is the release. Binding
   it to `estimand.valid_time` instead would change what resolves and what
   is refused.
6. **Notes and the log.** Today the log is "evidence about the write, not a
   condition of it" (`lib.rs:48931`). Under M2 a write that is not logged
   should at least say so in its response.
7. **Fan-out size.** `estimand` is about 660 bytes on a Sentinel-2 fact, on
   top of about 4,400 bytes for the fact with its band metadata. It is added
   only where band metadata is inlined. Hoisting its per-band part on
   `recall_polygon`, as band metadata is, would keep fan-out replies small.

## What this audit did not check

- No write route was called, so the persistence chain, derive, bundle,
  entity and note findings are from code at `0d613cda`, not from live
  records.
- The findings on derivations, raster tokens, bundles, entities, notes and
  the DDS were gathered by two code-reading passes and spot-checked: the
  bundle preimage, the entity receipt preimage, the note log append and the
  EUDR `plot_id` were re-read by hand. The other line references were not.
- The GLO-30 longitude spacing beyond the two tiles read (12 N, 66 N) is
  taken from the Copernicus DEM product handbook.
