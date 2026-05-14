# emem architecture

System-level mental model for `emem.dev` v0.0.6. Covers process
topology, the 14 workspace crates, the data / trust / fetch /
inference / agent planes, the auto-materialize loop, and the storage
layout on disk. Companion documents: `protocol.md` for byte-level
rules, `agents.md` for calling conventions, `operating.md` for
deploy, `whitepaper.md` for the math.

## The shape of the system

A single Rust binary `emem-server` listens on one port (default
`0.0.0.0:5051`) and serves both HTTP/REST (**169 routes**, **79 under
`/v1/*`**) and an MCP JSON-RPC endpoint at `POST /mcp`
(**49 tools**). An optional Python sidecar over a Unix domain socket
handles GPU inference for Clay v1.5, Prithvi-EO-2.0, Galileo, and
JEPA v2. Storage is a sled hot cache plus an append-only Merkle
log on local disk. Identity is a 32-byte ed25519 secret at
`<EMEM_DATA>/identity.secret.b32` (mode 0600); the matching pubkey is
published at `/.well-known/emem.json` so any client verifies receipts
offline.

Place resolution and admin-boundary lookup run entirely on open data
the binary carries or pulls keylessly: GeoNames cities-5000 ships
embedded (CC-BY-4.0, 68 581 populated places); Overture's
`divisions/division_area` theme supplies polygon geometry over
anonymous S3 (ODbL). Photon → Nominatim is the long-tail fallback.
The agricultural-field surface (`/v1/field_boundaries` plus
`include:["ftw_fields"]` on `/v1/recall_polygon`) reads Fields of The
World's global product (CC-BY-4.0, ~3.17 B field polygons) via
PMTiles range reads.

![emem architecture — one binary, two wire surfaces, one optional sidecar](/docs/diagrams/01-architecture.svg)
*The whole stack in one figure. The ASCII variant below is the same shape, terminal-friendly.*

## Process topology

```
+------------------------------------------------------------------+
|                         emem-server (Rust)                       |
|                                                                  |
|  +---------------+   +-----------------+   +------------------+  |
|  |  axum router  |-->|   primitives    |-->|     storage      |  |
|  |  /v1/*  /mcp  |   | recall, etc.    |   | cache+log+ident  |  |
|  +-------+-------+   +--------+--------+   +--------+---------+  |
|          |                    |                     |            |
|          |                    |                     v            |
|          |                    |            +-----------------+   |
|          |                    |            |   sled (hot)    |   |
|          |                    |            |   merkle.log    |   |
|          |                    |            |   fact_proofs   |   |
|          |                    |            +-----------------+   |
|          |                    v                                  |
|          |           +-----------------+                         |
|          |           |  fetch          |--> vsicurl / S3 / REST  |
|          |           |  Dispatcher     |    (S2, Cop-DEM, MODIS, |
|          |           +-----------------+     Tessera, GMRT, ...) |
|          v                                                       |
|   +---------------+                                              |
|   | gpu_sidecar   |---UDS---> Python FastAPI                     |
|   | (HTTP/1.1)    |          (Clay / Prithvi / Galileo / JEPA v2)|
|   +---------------+                                              |
+------------------------------------------------------------------+
```

The router constructs `AppState = Arc<Server>` (`emem-storage::server::Server`)
at boot. `Server` owns the `Storage` trait object, a
`ResponderIdentity` (ed25519), and `ManifestCids` (registry, schema,
bands, sources). Every primitive call gets `&Server` and returns a
signed `Receipt`.

## Crates (14)

| Crate | Role |
|-------|------|
| emem-api-rest | HTTP/MCP router + AppState + inline materializers + sidecar client + physics solvers |
| emem-fetch | 12 data connectors + 6 utility modules: `cache_window`, `chirps`, `cog`, `connectors`, `dmsp_ols`, `firms`, `ftw`, `geonames`, `hansen_gfc`, `koppen`, `overture`, `proj`, `stac`, `template`, `terraclimate`, `wdpa`, `worldpop` |
| emem-primitives | recall / find_similar / trajectory / compare / compare_bands / diff / verify / query_region + binary_embedding + refinement + cbor_ops |
| emem-core | bands, algorithms, functions, sources, topics, schema, taxonomy, manifest, privacy, tslot, cell, bbox |
| emem-cli | 7 binaries: `emem`, `emem-server`, `emem-demo`, `emem-livedemo`, `emem-realdemo`, `emem-ask-eval`, `emem-purge-fnkey` |
| emem-storage | `MaterializingStorage` (cache + fetch + log composite), `Server`, `AttesterRegistry`, `AttestationLog` |
| emem-mcp | MCP tool registry (49 tools) |
| emem-codec | cell64 / cid64 / tslot_text / vec64 / hilbert / geo / alphabet |
| emem-cache | sled cache wrapper (`SledHotCache`) |
| emem-intent | 7-variant `Intent` enum and rule-based planner |
| emem-fact | `Fact` / `Receipt` / `Attestation` CBOR types and signing primitives |
| emem-attest | `merkle_root`, `merkle_root_and_paths`, `verify_merkle_path` |
| emem-claim | `Claim` predicate (`Op` enum + value, no signature) |
| emem-cubes | AgriSynth `.npz` handle (Python authoritative) |

`emem-storage` is the keystone: `MaterializingStorage` glues the
cache trait to the `Dispatcher` and the `AttestationLog`, then
exposes the `Storage` trait that primitives program against.

## Invariants

- **CID rule.** `FactCid = base32_nopad_lower(blake3(canonical_cbor(fact))[..16])`.
  No code path produces a CID through any other rule.
- **Receipt rule.** Every primitive response carries a `Receipt` with
  an ed25519 signature over the canonical preimage. Empty `cells` /
  `fact_cids` lists still emit their trailing field separator so a
  verifier reproduces the bytes unambiguously.
- **Tempo rule.** A band cannot be served at a tempo finer than its
  declared cadence (`bands-v0.json`).
- **Sensor-tier rule.** An algorithm claiming delivery resolution
  ≤ 10 m must declare at least one S1 / S2 / Landsat input in
  `variance_sources`.
- **Verify-on-write.** `MaterializingStorage::put_attestation` rebuilds
  the merkle root from the supplied facts and re-checks the
  signature; rejected attestations never reach the cache or the log.
- **No silent fallback.** Empty results, missing capabilities, and
  timeouts surface as typed Absence / structured error, never as a
  zeroed numeric.

## The data plane

### Fact lifecycle

Every value the protocol cites is one of three `Fact` variants
(`emem-fact/src/fact.rs`): `PrimaryFact`, `DerivativeFact`,
`NegativeFact`. The CID rule is invariant: canonical CBOR, blake3-32,
base32-nopad-lowercase. Two encodings of the same Fact converge on
the same CID, so cache hits are content-addressed end to end.

```
Upstream source --> fetch connector --> materializer --> PrimaryFact
                                                            |
                                                            v
                                              ciborium canonical CBOR
                                                            |
                                                            v
                                                blake3 -> FactCid (26 chars)
                                                            |
                                                            v
                                          Attestation { facts, batch_root,
                                                       attester, signature,
                                                       registry_cid, schema_cid }
                                                            |
                                              +-------------+--------------+
                                              v             v              v
                                          sled hot       merkle.log    fact_proofs
                                          (index +       (segment      (per-cid
                                           facts trees)  + fsync)       merkle path)
```

`SledHotCache` (`emem-cache/src/sled_hot.rs`) holds two trees:

- `emem.canonical_index` — `cell\0band\0tslot_be8` -> `fact_cid_string_bytes`
- `emem.facts` — `fact_cid_string_bytes` -> canonical CBOR of the fact

`MaterializingStorage` (`emem-storage/src/lib.rs`) opens a third tree
`emem.fact_proofs` where it persists per-fact merkle inclusion proofs
at attestation-write time so receipts carry a verifier-ready path
back to the batch root.

### Recall path

```
POST /v1/recall {cell, bands?, tslot?}
  → router: resolve_cell_field (place name → cell64)
  → recall: scan_cell(cell, tslot) or lookup_canonical_many(keys)
  → storage: sled prefix scan or point gets → fact_cids
  → storage: get_facts_many(fact_cids) → facts
  → recall: scan_cell(cell, None) → bands_already_attested_at_cell
  → sign_receipt("emem.recall", [cell], cids, ...)
  → router: blake3(sorted fact_cids) → ETag
  → 200 JSON + ETag + Cache-Control: private, max-age=60
```

The router honours `If-None-Match`: a repeat recall with a matching
ETag returns `304 Not Modified` with empty body. The receipt's
`served_at` differs per call, but the ETag derives from the
immutable `fact_cids`, so it stays bit-stable across calls.

### Auto-materialize on miss

`recall_with_auto_materialize` (`emem-api-rest/src/lib.rs:3742...`):
calls `recall` first; if the response is empty for some requested
band, `try_materialize_bands` looks up
`band_materializer_meta(band)`, dispatches to the appropriate
`materialize_<band>` arm (Open-Meteo elevation, ORNL MODIS, NASA
POWER, SoilGrids, FIRMS, CHIRPS, Sentinel-2 STAC pick + COG, ...),
builds a `PrimaryFact` (or `NegativeFact` over water), signs as
responder, persists through `cache.put_many` + `log.append` +
`persist_fact_proofs`, and then re-runs the recall — now a
cache-hit. The router returns the second recall's facts plus a
`materialize_notes[]` block describing what happened per band.

Three policies drive what materializes:

1. Bands are explicitly requested → every band missing from the
   cache-hit response becomes a candidate.
2. No bands requested AND the cell is empty → a small default set
   fires (`copdem30m.elevation_mean`, `weather.temperature_2m`) so a
   bare `recall` returns something citable.
3. No bands requested AND the cell already has facts → leave alone.

Gates:

- `EMEM_AUTO_MATERIALIZE` — `0`/`false` disables. Default on.
- `EMEM_MATERIALIZER_TIMEOUT_SECS` — default 30 s, clamped 2..=240.
- `EMEM_TIMEOUT_SECS` — gateway timeout, default 180 s, clamped 1..=600.

The signer of the materialised fact is the responder's own pubkey;
`derivation.fn_key` declares exactly how it was produced.

**20 live materializer registrations** answer recall today; **118
materializer-wired band names** flow through them (parametric
prefixes — `s2.*`, `overture.*`, `weather.*`, `era5.*`, `cams.*`,
`marine.*`, `power.*`, `terraclimate.*`, `modis.*`, `geotessera.*`,
`hansen.*`, `forest_change.*`, `soilgrids.*`, `firms.*`, `chirps.*`,
plus specific scalar bands). The dispatch arm lives in
`band_materializer_meta` at `emem-api-rest/src/lib.rs:16720`.

### find_similar (corpus scan)

`POST /v1/find_similar` parses the query (inline-encoded vector or
`load_cell_vec(key, band)`), then walks the full sled `iter_index`
applying the band filter and optional `Claim`-typed predicate. For
each candidate it loads the fact and scores cosine; the top-k is
deduped per cell and signed as `emem.find_similar`. The router
enriches each neighbour with lat/lng, the GeoNames nearest-place
label (cap 25 km), and deep-link URLs (`deep_recall_url`,
`scene_png_url`) before returning.

`Hamming` and `HammingThenRerank` modes branch before loading the
full vector — for binary-only modes the full f32 payload never
enters the scoring loop. When the binary sibling band is absent at a
cell but the cosine band is present, the path auto-derives bin128
inline via the TurboQuant rotation (seed
`emem.binary_embedding.turboquant.v1`) rather than returning
`CidNotFound`. Memoisation per cell keeps Claim filter evaluations
from re-scanning a cell once per tslot.

`HammingThenRerank` adapts its oversampling factor from an EWMA over
observed `|hamming_top_k ∩ cosine_top_k| / k`, backed by lock-free
`AtomicU64` storage (decay α = 0.05, warm-up ~50 calls). Cold-start
falls back to the historical 4× multiplier so the first 50 calls
match pre-EWMA behaviour byte-for-byte.

When a query cell has no vector under the requested band,
`find_similar` returns `ErrorCode::CidNotFound` with a hint naming
the bands that *are* attested at the cell.

### Verify-on-write

`MaterializingStorage::put_attestation` re-CBOR-encodes every fact,
hashes each to a 32-byte leaf, sorts bytewise, folds via
`merkle_root`, compares to `att.batch_root`, then hashes
`(batch_root || registry_cid_str || schema_cid_str)` and
`verify_strict`s the signature with `att.attester`'s ed25519 key.
Failure raises `StorageError::AttestationInvalid("merkle root
mismatch")` or `..."bad signature"`. On success, the storage writes
`cache.put_many` (both sled trees, async flush), `log.append`
(CBOR + 32-byte hash + fsync), `persist_fact_proofs` (best-effort),
and `attesters.record_attestation` (best-effort reputation). The
last two never fail a write; cache and log writes are atomically
gated by the verifier.

## The trust plane

Per-process responder identity (`emem-cli/src/bin/emem-server.rs::load_or_create_identity`):

- Highest priority: `EMEM_SECRET_B32` env (32-byte ed25519 secret,
  base32-nopad lowercase).
- Else: `<EMEM_DATA>/identity.secret.b32` if it exists.
- Else: generate fresh, write to that path, chmod 0600.
- `EMEM_DATA=:memory:` skips persistence (responder pubkey changes
  every restart).

The matching pubkey is surfaced at `/.well-known/emem.json` so any
client verifies a receipt offline without contacting the responder
again.

### Receipt signing

`Server::sign_receipt` (`emem-storage/src/server.rs:119-189`)
constructs the canonical preimage byte-by-byte:

```
blake3(
    request_id_bytes
  | "|"
  | served_at_bytes          (ISO 8601 UTC, no fractional)
  | "|"
  | primitive_bytes          (e.g. "emem.recall")
  | "|"
  | for each cell c: c_bytes | ","
  | "|"
  | for each fact_cid f: f_bytes | ","
)
```

The 32-byte digest is signed with the responder's
`ed25519_dalek::SigningKey`. Verification is
`vk.verify_strict(preimage, sig)`.

`Cost.was_cached` is `true` for recall (everything served already
lived in sled at sign time), `false` for find_similar (fresh scan),
and tracked per-primitive elsewhere.

### In-browser verification

`/verify` and `/verify/<fact_cid>` serve `web/verify.html`. The page
loads noble-curves@1 and noble-hashes@1 via esm.sh (sub-resource
CSP'd), reconstructs the canonical preimage from the receipt fields,
and runs ed25519 verification entirely in the browser. When noble
bundles are blocked, the page falls back to
`POST /v1/verify_receipt` and labels itself "server-assisted".
`/verify?receipt=<base64>` accepts a pasted receipt; idle `/verify`
is a landing page.

### Merkle inclusion proofs

`persist_fact_proofs` writes a per-fact
`MerkleProof { leaf_index, path, root }` keyed by FactCid into the
`emem.fact_proofs` sled tree. `Server::sign_receipt` looks up the
proof for the first cited fact and embeds it as `receipt.merkle_proof`.
A verifier with the responder pubkey re-derives every other CID from
the signed receipt payload, so one inclusion anchor is sufficient.

`emem-attest::merkle_root_and_paths` handles the leaf hashing: each
leaf is self-hashed `blake3(leaf || leaf)` (domain separation against
second-preimage attacks), then pairwise `blake3(left || right)`
upward. Empty input yields `[0u8; 32]`.

### Append-only Merkle log

`AttestationLog` (`emem-storage/src/merkle_log.rs`) writes to
`<EMEM_DATA>/log/merkle.log.{0,1,...}`:

- Record format: `[u32_LE len][CBOR bytes][32-byte blake3(CBOR)]`
- Segment cap: 1 GiB; new segment opens automatically
- Per-segment trailer: `segment_hash = blake3(all_records)`
- `append()` calls `fsync_all()` before returning
- `verify()` re-hashes every sealed segment and reports mismatches

### Reputation tracking

`AttesterRegistry` (`emem-storage/src/attesters.rs`) opens a sled tree
`emem.attesters`. Per-pubkey stats:
`{attestations, facts, citations, unique_cells, first_seen,
last_seen, last_cited}`. Atomic CAS update per attest call. Score
formula: `citations·1.0 + ln(1+facts)·8.0 + ln(1+atts)·4.0`.
Best-effort — tracker errors never fail a read or a write.

## The fetch plane

`emem-fetch::Dispatcher` is stateless. `register_default_https`
registers connectors against `ConnectorKind`s: `HttpsGeotiff` and
`HttpsCogVsicurl` share an `HttpsConnector` (anonymous reqwest,
`Accept-Encoding: identity` so `Range` offsets stay aligned with
the original GeoTIFF); `GcsCog` rewrites `gs://` to
`https://storage.googleapis.com`; `IpldCid` is a stub (operators
register their own blockstore). `HttpsConnector::fetch_range` is
the hot path: 90 s pool-idle, pool max-idle 8 per host; 429
surfaces as `FetchError::RateLimited` with `Retry-After`.

Two surface populations produce live facts.

**Dedicated `emem-fetch` modules — 12 data connectors:**

| Module | Read path | Bands |
|--------|-----------|-------|
| chirps | UCSB CHIRPS daily gzipped TIFF | `chirps.precip_daily_mm` |
| dmsp_ols | NOAA NCEI V4 nightlights tar+gz | `nightlights.dmsp_ols_avg_dn` |
| firms | NASA FIRMS bulk CSV, 60-min cache | `fire_detections.firms_modis_viirs_nrt` |
| ftw | source.coop PMTiles + MVT decode | `ftw.field_polygons.v1` |
| geonames | cities-5000 gazetteer, embedded | utility |
| hansen_gfc | earthenginepartners-hansen GCS, v1.12 | `forest_change.{lossyear, treecover2000, gain}` |
| koppen | Beck 2018 figshare PackBits TIFF | `climate.koppen_geiger_present_day` |
| overture | Overture S3, parquet row-group prune | `{buildings,places,transportation}.overture_count` |
| terraclimate | UI Climatology Lab THREDDS NCSS | `climate.terraclimate_*_normal` |
| worldpop | WorldPop `/v1/services/stats` REST | `population.worldpop_2020` |
| wdpa | OSM Overpass `boundary=protected_area` | `protected_areas.wdpa_via_osm` |
| stac | Element84 + MPC search | input to Sentinel materializers |

**Utility modules — 6:** `cache_window` (in-flight fetch coalescing
via `tokio::Notify`), `cog` (universal pure-Rust COG range sampler —
Deflate, LZW, Predictor 1/2/3, 8/16/32-bit LE), `connectors` (the
Dispatcher itself), `proj` (WGS84↔UTM), `template` (URL templating),
and the crate `lib`.

**Inline materializers in `emem-api-rest/src/lib.rs`** produce live
facts but live in the router crate (architectural debt):
`materialize_gmrt_topobathy`, `materialize_ornl_modis_band` (7 MODIS
bands), `materialize_power_band` (7 NASA POWER bands), the four
Open-Meteo arms (`weather_current`, `cams_band`, `era5_band`,
`marine_band`, ~25 bands total), `materialize_soilgrids_band` (6
depths), `materialize_firms_active_fires`,
`materialize_chirps_daily_precip`, plus Sentinel-1/-2, GeoTessera,
Prithvi / Galileo / Clay encoders, JRC GSW, Overture, ESA
WorldCover, Köppen, WorldPop, WDPA.

Of the **43 declared source schemes** in `sources-v0.json`, five
remain declared-but-unwired: `dynamic_world.v1`,
`openet.30m.daily`, `tropomi.s5p.{ch4, no2}`, `viirs.dnb.monthly`.
A recall on those bands returns a typed `MaterializeMiss` Absence.

## The inference plane

`crates/emem-api-rest/src/gpu_sidecar.rs` is a hand-rolled HTTP/1.1
client over a UDS resolved from `EMEM_SIDECAR_SOCK` (systemd unit:
`%t/emem/jepa_sidecar.sock`; Rust default `/run/emem/jepa_sidecar.sock`).
Timeout via `EMEM_SIDECAR_TIMEOUT_MS` (default 5000). On
`SidecarError::Unavailable` the caller falls back to in-process CPU
(where wired); on a non-503 from the sidecar it must refuse — no
silent downgrade.

Four GPU-pinned encoders co-resident on a 20 GB VRAM budget
(`EMEM_SIDECAR_VRAM_BUDGET_GB=20`): Clay v1.5 (1024-D, ~18 ms warm,
production), Prithvi-EO-2.0-300M-TL (1024-D, ~20 ms warm,
production), Galileo (variant selectable via `EMEM_GALILEO_VARIANT`, default `base`
in production; ~14 ms warm, S2-only modality wired), JEPA v2 dynamics (128-D, untrained baseline that
short-circuits ONNX/sidecar inference when `is_trained() == false`
and returns `last_input_vintage` directly). Per-model `*_BUDGET_GB`
constants sum to `TOTAL_BUDGET_GB`; the cap is enforced via one
`torch.cuda.set_per_process_memory_fraction` at registry init.
CUDA OOM surfaces as 503 to Rust. See `docs/developers/inference.md` for input
shapes, chip fetchers, and the trained-checkpoint loader contract.

Physics solvers in `crates/emem-api-rest/src/physics.rs` are
in-process Rust, no sidecar dependency: `/v1/heat_solve` (FTCS 2D,
3×3 MODIS `lst_day_8day` stencil, CFL safety 0.20),
`/v1/wave_solve` (CTCS 1D shallow water along a seaward profile,
land-locked rejection with profile + suggestion),
`/v1/jepa_predict` (closed-form NDVI AR(2) with fixed coefficients),
`/v1/jepa_predict_v2` (sidecar Tessera dynamics or short-circuit for
the untrained baseline).

`model.via` in the receipt records provenance: `python_sidecar` for
sidecar calls, `in_process_cpu` for CPU fallback, `short_circuit`
for the JEPA v2 untrained sentinel.

## The agent surface

REST and MCP serve the same primitives. The MCP tool list is a
strict read-only subset of REST; writes (`attest`, `backfill`,
reviews POST) go through REST only. `POST /mcp` is JSON-RPC 2.0,
backed by `crates/emem-mcp/src/lib.rs` (49 tools). Three
well-known endpoints publish capabilities: `/.well-known/mcp.json`
(MCP transport advertisement), `/.well-known/agent-card.json`
(recommended tool order), and `/.well-known/emem.json` (responder
pubkey).

Discovery chain on first contact: `/v1/agent_card` (recommended
tool order) → `/v1/manifests` (CIDs for bands, algorithms, sources,
schema, registry, topics) → `/v1/grid_info` (cell pitch ~10 m) →
`/v1/data_availability` (which bands have history) →
`POST /v1/locate {q:"<place>"}` (cascade: wide_bbox → embedded
gazetteer → GeoNames → cache → Photon → Nominatim; polygon from
Overture `divisions/division_area`) → `POST /v1/recall`,
`/v1/find_similar`, `/v1/verify`, `/v1/diff`.

The 169 REST endpoints split across 13 categories (full list at
`/v1/tools`):

- Health / discovery (8): `/health`, `/openapi*.json`,
  `/.well-known/{emem,agent,mcp}.json`, `GET /mcp`, `/verify`,
  `/verify/:cid`.
- Introspection (14): `/v1/{bands, topics, algorithms,
  algorithms/:key, functions, sources, materializers,
  data_availability, coverage_matrix, manifests, grid_info, errors,
  tools, schema}`.
- Read primitives (10): `POST /v1/{recall, recall_many,
  recall_polygon, query_region, compare, compare_bands,
  find_similar, trajectory, diff}`, `GET /v1/cells/:cell64`.
- Write primitives (3): `POST /v1/{attest, attest_cbor, backfill}`.
- Verify (2): `POST /v1/{verify, verify_receipt}`.
- Physics solvers (4): `POST /v1/{heat_solve, wave_solve,
  jepa_predict, jepa_predict_v2}`.
- Boring-API alias zoo (18): GET+POST `/v1/{elevation, ndvi, air,
  lst, soil, water, forest, weather, at}`.

`/v1/recall_many` accepts up to 256 cells per request; each cell
carries its own signed receipt — verifying any one cell only
verifies that cell. `/v1/ask` carries a `foundation_embeddings`
envelope when the question matches Similarity or Change intent;
fan-out runs concurrently across `clay_v1` + `prithvi_eo2` +
`geotessera` under budget `ask_timeout_ms` (default 4000, read from
the `clay_prithvi_tessera_triple_consensus@1` parameters block). On
timeout the envelope carries
`degraded_reason: "foundation_embedding_timeout"`.

## Storage layout on disk

`<EMEM_DATA>/` (default `./var/emem`):

```
<EMEM_DATA>/
  identity.secret.b32      0600, ed25519 secret base32-nopad lowercase
  cache.sled/              hot tier — index, facts, attesters, fact_proofs trees
  log/merkle.log.{0,1,...} append-only segments, 1 GiB cap each
  geocoder.sled/           locate cache (separate sled DB)
  hf_cache/                HuggingFace snapshots; HF_HUB_OFFLINE=1 ready
  models/                  BAAI/bge-base-en-v1.5 ONNX for topic router
  jepa_v2/                 dynamics_v2.onnx (~8 KB) + metadata.json
  acme.cache/              Let's Encrypt cert + account key (EMEM_TLS_DOMAINS)
```

`EMEM_DATA=:memory:` skips on-disk entirely: ephemeral sled DB,
fresh ed25519 key on every boot, no log persistence. Receipts from
such a process verify only until restart.

## Failure modes

| trigger | response |
|---------|----------|
| sled lock contention on `cache.sled/` | server holds exclusive lock; tools like `emem-purge-fnkey` require server stopped first |
| sidecar OOM during cold-start | 503 to Rust client; JEPA v2 short-circuits, Prithvi / Clay / Galileo propagate 503 |
| materializer timeout (`EMEM_MATERIALIZER_TIMEOUT_SECS`, default 30 s) | recall returns the original facts plus `materialize_notes[]` entry `{band, status:"skipped", reason}`; no zero-value fallback |
| attestation rejected | `StorageError::AttestationInvalid` with message (root mismatch or bad signature); cache and log untouched |
| upstream 502/503 | `FetchError::Transport`; materializer either propagates or signs a `NegativeFact` with `ReasonCid` (e.g. Cop-DEM over water) |
| `EMEM_SCAN_CELL_LIMIT` hit (default 10 000 rows per prefix walk) | logged `target=emem::storage scan_cell_limit_hit`; legitimate cells hold one fact per (band, tslot), so the cap signals schema mistake or attack |
| topic router warmup > 90 s | server starts anyway; `/v1/ask` falls back to keyword backend until the OnceLock initializer returns |

## Pointers

- `docs/protocol.md` — wire bytes (CBOR field order, CID rules,
  receipt preimage, attestation envelope).
- `docs/agents.md` — calling conventions for LLM agents (REST +
  MCP), the locate → recall → verify chain, in-browser receipt
  verification.
- `docs/operators/operating.md` — deploy paths (plain HTTP behind a reverse
  proxy, native TLS via Let's Encrypt TLS-ALPN-01), systemd units,
  env knobs.
- `docs/whitepaper.md` — math + design rationale + triple-consensus
  algorithm derivation.
- `docs/developers/inference.md` — sidecar protocol, per-encoder chip fetchers,
  trained-checkpoint loader contract, VRAM partitioning.
