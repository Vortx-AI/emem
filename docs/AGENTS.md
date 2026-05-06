# AGENTS — using emem from inside an agent loop

> Cite-able, content-addressed, ed25519-signed memory of every place on Earth.
> Three calls is the steady state: `locate` → `recall` → cite `receipt.fact_cids[0]`.
> Bootstrap with one call: `GET /v1/discover`.

This guide is written under seven rules: minimal English, structured schemas,
typed operational semantics, spatial symbolic algebra, machine-readable
guarantees, executable examples, ontology-first. Prose appears only where
the meaning is irreducible.

---

## 1. Algebra

    Cell      = b1024(lat[21] · lng[22])         pitch ≈ 9.55 m at equator
    Tslot     = u64                              wall-clock or vintage anchor
    Band      = string                           name in the bands manifest
    Value     = f32 | f32[] | enum | absence
    Fact      :: Cell × Band × Tslot → {value, unit, provenance, fact_cid}
    fact_cid  = base32-nopad-lowercase(blake3(canonical_cbor(fact)), 32B)
    Receipt   :: Request → {primitive, cells, fact_cids, responder, served_at, sig}
    sig       = ed25519(blake3(request_id ‖ served_at ‖ primitive ‖ cells ‖ fact_cids))

Identical canonical facts → identical CIDs across responders. CIDs survive
copy-paste between conversations and between agents. There is no "session";
emem is global, append-only memory of place.

---

## 2. Ontology

| Object       | Definition                                                   | Wire form                          | Manifest              |
|--------------|--------------------------------------------------------------|------------------------------------|-----------------------|
| `Cell`       | spatial address; ~10 m square at equator                     | `cell64` (4 base-1024 bigrams)     | `topics_cid` not used |
| `Band`       | named scalar/vector slot in the 1792-D voxel layout          | `s2.B04`, `indices.ndvi`, …        | `bands_cid`           |
| `Tslot`      | u64 time slot                                                | `t.<base32>`                       | `schema_cid`          |
| `Fact`       | one signed `(cell, band, tslot, value, …)` record            | canonical CBOR                     | `schema_cid`          |
| `Receipt`    | signed envelope for a request                                | canonical CBOR                     | `schema_cid`          |
| `Algorithm`  | composition recipe over multiple Facts                       | name@version                       | `algorithms_cid`      |
| `Topic`      | natural-language query → relevant bands+algorithms           | string                             | `topics_cid`          |

Pin the relevant manifest CIDs in your receipt to make the answer fully
reproducible.

---

## 3. Primitives (typed)

Each primitive is `Type :: Input → Output × Receipt`. Schemas referenced by
JSON Pointer into `/openapi.json`. Every example below uses a current-grid
cell64 (legacy pre-2026 cell strings will return `cell decode` errors).

### 3.1 ask  ::  Question × (Place|Cell) → Fact[] × Algorithm[] × Receipt
- URL: `POST /v1/ask`  ·  MCP: `emem_ask`  ·  Schema: `/openapi.json#/paths/~1v1~1ask/post`
- Guarantees: `signed`, `content_addressed`, `topic_routed`, `idempotent`
- One call replaces locate → topic-route → recall → algorithm fan-out.
- Example:
    ```json
    {"q":"air quality in new delhi today","place":"New Delhi"}
    ```
- Response carries `facts[]`, `algorithms_for_question[]`, `algorithm_outcomes[]`,
  `band_observations[]`, `temporal_composition[]`, `topic_routing.routing.method`.
- Cite: `$.receipt.fact_cids[0]` (13-char prefix).

### 3.2 locate  ::  (Place|LatLng) → Cell × Bbox × Neighbours
- URL: `POST /v1/locate`  ·  MCP: `emem_locate`
- Guarantees: `deterministic`, `via_field_traceable`
- Geocoder ladder: embedded gazetteer → cache → Photon → Nominatim. The
  `via` field traces which path resolved the query.
- Example:
    ```json
    {"q":"Mount Fuji"}
    ```
- Response: `{"cell64":"defi.zb592.nemu.zEvE","lat_input":35.3606,"lng_input":138.7274,"via":"photon", …}`

### 3.3 recall  ::  Cell × Band[] → Fact[] × Receipt
- URL: `POST /v1/recall`  ·  MCP: `emem_recall`
- Guarantees: `signed`, `content_addressed`, `auto_materialize`, `absence_is_signed`
- Empty `bands` returns every attested band at the cell. Auto-materializes
  on miss for any wired band (S2 indices, MODIS, weather, Cop-DEM, etc.).
- Example:
    ```json
    {"cell":"defi.zb592.nemu.zEvE","bands":["copdem30m.elevation_mean"]}
    ```
- Cite: `$.facts[0].fact_cid`.

### 3.4 recall_polygon  ::  (Place|Bbox) × Band[] → Fact[][] × Aggregate × Receipt
- URL: `POST /v1/recall_polygon`  ·  MCP: `emem_recall_polygon`
- Closes locate → polygon-sample → recall_many in one call. `n_cells` ≤ 64.
- Example:
    ```json
    {"place":"Lagos","bands":["cams.pm25"],"n_cells":16}
    ```

### 3.5 compare  ::  Cell × Cell × BandFamily → Score × Receipt
- URL: `POST /v1/compare`  ·  MCP: `emem_compare`
- Score is family-dependent: cosine for embedding families, Δ for scalar.
- Example:
    ```json
    {"a":"defi.zb592.nemu.zEvE","b":"defi.zb53e.rIzA.semU","family":"elevation"}
    ```

### 3.6 find_similar  ::  Cell × Embedding × k → (Cell × cos)[k] × Receipt
- URL: `POST /v1/find_similar`  ·  MCP: `emem_find_similar`
- `band` selects which embedding to k-NN over (`geotessera`, `prithvi_eo2`,
  `galileo_base_v1`). Embedding manifest pinned in the receipt.
- Example:
    ```json
    {"key":"defi.zb595.zeaf8.zd3a0","band":"geotessera","k":5}
    ```

### 3.7 diff  ::  Cell × Tslot × Tslot × Band[] → Δ × Receipt
- URL: `POST /v1/diff`  ·  MCP: `emem_diff`
- Returns a DerivativeFact carrying its own `fact_cid`.
- Example:
    ```json
    {"cell":"defi.zb592.nemu.zEvE","t1":1577836800,"t2":1704067200,"bands":["indices.ndvi"]}
    ```

### 3.8 trajectory  ::  Cell × Band × [t0,t1] → Fact[] × Receipt
- URL: `POST /v1/trajectory`  ·  MCP: `emem_trajectory`
- Returns only already-attested tslots (no auto-materialization). Use
  `backfill` to densify history.

### 3.9 backfill  ::  Cell × Band × [t0,t1] → Fact[] × Receipt
- URL: `POST /v1/backfill`
- Materializes per-tslot history for one cell × band; bounded by
  `history_available_from_unix`/`to_unix` from `/v1/coverage_matrix`.

### 3.10 verify_receipt  ::  Receipt → {valid: bool, signer: pubkey}
- URL: `POST /v1/verify_receipt`  ·  MCP: `emem_verify`
- Stateless, offline-verifiable. Recomputes `blake3(request_id ‖ served_at ‖
  primitive ‖ cells ‖ fact_cids)`, then `ed25519_verify(sig, hash, signer)`.

### 3.11 query_region · attest_cbor · heat_solve · wave_solve · jepa_predict_v2
- See `/v1/agent_card` (skill descriptors) and `/openapi.json` (input schemas).

---

## 4. Citation contract

Every numeric response carries the following sibling fields. Quote them
verbatim — no second round-trip required.

| Field                                | Required for cite | Notes                                                   |
|--------------------------------------|-------------------|---------------------------------------------------------|
| `value`                              | yes               | the number                                              |
| `unit`                               | yes               | physical unit                                           |
| `band_metadata.interpretation`       | when relevant     | editorial guidance — quote, don't reinvent              |
| `band_metadata.pitfalls`             | when intersects   | only mention when user's question intersects            |
| `band_metadata.dimension_description`| for vector bands  | the per-scalar slot label                               |
| `value_decoded`                      | for categorical   | human label (ESA WorldCover, JRC SW class, S2 SCL)      |
| `signer_pubkey_b32`                  | yes               | quote in cid64-short form                               |
| `receipt.fact_cids[i]`               | yes               | parallel-array CIDs; quote first 13 chars               |
| `receipt.responder_pubkey_b32`       | once per session  | not per fact                                            |

For `/v1/ask`, also surface `algorithm_outcomes[].{algorithm_key, value}` and
look up the human label at `/v1/algorithms[key].output.values[value-1]`.

---

## 5. Foundation embeddings (multimodal)

Three live foundation embeddings, signed individually:

| Band              | Source                            | Dims | Modality                           | Tempo |
|-------------------|-----------------------------------|------|------------------------------------|-------|
| `geotessera`      | Tessera v1 (Cambridge)            | 128  | S2-derived global                  | 2024-only upstream |
| `prithvi_eo2`     | Prithvi-EO-2.0-300M-TL (NASA/IBM) | 1024 | HLS V2 6-band ViT-L                | vintage-agnostic |
| `galileo_base_v1` | Galileo Base (NASA Harvest)       | 768  | S2 + (S1+DEM+climate masked-zero)  | vintage-agnostic |

All are L2-normalised and exposed under `find_similar`. The embedding
manifest is pinned in the receipt so the same neighbourhood is reproducible.

---

## 6. Multimodality

The fusion contract: when an algorithm claims ≤10 m delivery, it must be
anchored on a sensor capable of 10 m: S1 GRD (10 m) > S2 L2A (10/20/60 m) >
Landsat (30 m) > IoT scalar > OtherSat > Static. The validator blocks
delivery claims that resolve only to coarser sources.

See `/multimodal.md` for the priority chain + per-fact `data_resolution_m`
contract.

---

## 7. Catalogs (machine-readable)

| Endpoint                     | What                                                        |
|------------------------------|-------------------------------------------------------------|
| `/v1/bands`                  | band ontology, dims, tempo, materializer status             |
| `/v1/algorithms`             | composition recipes (flood_risk, walkability, …)            |
| `/v1/materializers`          | wire-stable list of auto-materializing bands                |
| `/v1/data_availability`      | per-band kind/tempo/history-window                          |
| `/v1/coverage_matrix`        | has_materializer, facts_count, last_attested                |
| `/v1/fleet`                  | sensor lineage (S1/S2/MODIS/Cop-DEM/…)                      |
| `/v1/grid_info`              | cell64 pitch + DGGS interop                                 |
| `/v1/temporal_route`         | PDE-based band scoring vs query time                        |

Don't memorise band lists; fetch the catalog when you need it.

---

## 8. Decision tree

    user mentions a place / lat-lng / cell64
       └─ POST /v1/locate                          → cell64
       └─ POST /v1/recall {cell, bands?}           → Facts + Receipt

    user names a wide region
       └─ POST /v1/recall_polygon {place, bands}   → per-cell Facts + Aggregate

    free-text question (any topic)
       └─ POST /v1/ask {q, place|cell}             → Facts + Algorithms + Receipt

    "how similar is X to Y"
       └─ POST /v1/compare {a, b, family?}

    "find places like X"
       └─ POST /v1/find_similar {key, band, k}

    "what changed between t1 and t2"
       └─ POST /v1/diff

    "show me history over a window"
       └─ POST /v1/backfill        (materializes per-tslot)
       └─ POST /v1/trajectory      (already-attested only)

    spatial yes/no with citable evidence
       └─ POST /v1/verify

    region average / median / p90
       └─ POST /v1/query_region

    underspecified spatial ask
       └─ POST /v1/intent

    short-horizon LST forecast
       └─ POST /v1/heat_solve      (∂u/∂t = α∇²u, 2-D explicit-FD)

    swell / surge propagation
       └─ POST /v1/wave_solve      (∂²u/∂t² = c²∂²u/∂x², 1-D shallow water)

    next-month NDVI
       └─ POST /v1/jepa_predict    (constrained AR(2), closed-form coefficients)

---

## 9. Executable examples (current-grid cells)

    # elevation — Mount Fuji (Cop-DEM 30 m, signed Absence over water)
    curl -sX POST https://emem.dev/v1/recall -H 'content-type: application/json' \
      -d '{"cell":"defi.zb592.nemu.zEvE","bands":["copdem30m.elevation_mean"]}'

    # NDVI — Lagos (Sentinel-2 L2A 10 m, ≤40% cloud, ≤30 d lookback)
    curl -sX POST https://emem.dev/v1/recall -H 'content-type: application/json' \
      -d '{"cell":"defi.zb44a.kisu.xIna","bands":["indices.ndvi"]}'

    # weather — Tokyo current 2-m air temperature (MET Norway, no key)
    curl -sX POST https://emem.dev/v1/recall -H 'content-type: application/json' \
      -d '{"cell":"defi.zb595.zeaf8.zd3a0","bands":["weather.temperature_2m"]}'

    # foundation embedding — Tokyo, 128-D Tessera
    curl -sX POST https://emem.dev/v1/recall -H 'content-type: application/json' \
      -d '{"cell":"defi.zb595.zeaf8.zd3a0","bands":["geotessera"]}'

    # k-NN — find 5 places spectrally like Tokyo (Tessera vintage 2024)
    curl -sX POST https://emem.dev/v1/find_similar -H 'content-type: application/json' \
      -d '{"key":"defi.zb595.zeaf8.zd3a0","band":"geotessera","k":5}'

    # region question — Lagos PM2.5 over a polygon
    curl -sX POST https://emem.dev/v1/recall_polygon -H 'content-type: application/json' \
      -d '{"place":"Lagos","bands":["cams.pm25"],"n_cells":16}'

    # MCP tools/list
    curl -sX POST https://emem.dev/mcp -H 'content-type: application/json' \
      -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'

---

## 10. Errors

`/v1/errors` returns the canonical error catalog (codes + when each fires).
Read it once, key your retry / surface logic off the codes — never off prose.

Common codes: `cell_decode_failed`, `no_geocoder_match`, `band_not_materialized`,
`history_out_of_window`, `auto_materialize_disabled`, `source_fetch_failed`.

A 200 with empty `facts[]` and `materialize_notes[]` is the honest signal
that a band's connector isn't wired or upstream returned no data — not an
error. The response also carries `bands_already_attested_at_cell` so the
agent can pivot to an answerable band.

---

## 11. Trust model

- Hash: `blake3` over canonical CBOR. CID = base32-nopad-lowercase, 32 B.
- Sig: `ed25519` over `blake3(request_id ‖ served_at ‖ primitive ‖ cells ‖ fact_cids)`.
- Responder pubkey at `/health`, `/.well-known/emem.json`, and on every
  receipt as `responder_pubkey_b32`.
- Verify any receipt offline:
    1. parse the `receipt`
    2. recompute `hash = blake3(request_id ‖ served_at ‖ primitive ‖ cells ‖ fact_cids)`
    3. verify `sig` against `signer` pubkey using `ed25519`
    4. (optional) recompute `fact_cid = base32_nopad_lower(blake3(canonical_cbor(fact)))`

Or call `POST /v1/verify_receipt`.

---

## 12. Discovery

| Surface                              | Purpose                                       |
|--------------------------------------|-----------------------------------------------|
| `GET /v1/discover`                   | typed bootstrap (ontology, primitives, CIDs)  |
| `/openapi.json`                      | full OpenAPI 3.1 spec                         |
| `/.well-known/agent-card.json`       | A2A v0.2 agent card                           |
| `/.well-known/mcp.json`              | MCP server descriptor                         |
| `/.well-known/emem.json`             | manifest CIDs + responder pubkey              |
| `/llms.txt`                          | crawler-friendly minimal entry                |
| `/v1/tools`                          | curated MCP / REST tool catalog               |
| `/v1/quickstart`                     | six-step playbook                             |
| `/mcp`                               | MCP Streamable HTTP                           |

---

## 13. Operator and legal

Operated by Vortx AI Private Limited (India). Contact: avijeet@vortx.ai
Privacy: `/privacy` · Terms: `/terms` · Support: `/support`
Source: `github.com/Vortx-AI/emem` · License: Apache-2.0
