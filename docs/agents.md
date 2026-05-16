# emem agent guide

## What this is

**Working memory of Earth, for AI agents.** Every patch of ground gets a
64-bit address (`cell64`, about 9.55 m on a side at the equator); every
measurement there is stored as a fact keyed by `(cell, band, tslot)`;
every read returns a content-addressed receipt the caller can verify
offline. The address space is the planet, the state is persistent, the
bytes are reproducible across any replica that mirrors them.

emem sits one layer beneath whatever memory your agent runtime ships
internally. Per-session associative memory, per-tenant scratchpads, and
vector-indexed document stores all answer different questions; emem
answers *what is at this place*, once, signed, byte-identical for every
caller that ever asks again. A `cell64` is to an emem-grounded reasoning
chain what a token is to an LLM: a stable, hierarchical, machine-readable
handle the rest of the pipeline can quote, share, and verify. Three core
moves: (a) locate a place by name to a `cell64`, (b) recall signed facts
at that cell, (c) find places similar to it by foundation embedding.
Every response carries an Ed25519 receipt any agent can verify offline.

The endpoints map cleanly onto the vocabulary other agent-memory libraries
use; if you arrive from mem0, Letta, LangGraph, or a custom retrieval
stack, the mapping below is the rosetta-stone:

| Memory operation                | emem primitive       | Endpoint                          |
|---------------------------------|----------------------|-----------------------------------|
| retrieve by address             | `recall`             | `POST /v1/recall`                 |
| retrieve by similarity          | `find_similar`       | `POST /v1/find_similar`           |
| retrieve over a region          | `recall_polygon`     | `POST /v1/recall_polygon`         |
| retrieve over time              | `trajectory`         | `POST /v1/trajectory`             |
| address-by-name (place → key)   | `locate`             | `POST /v1/locate`                 |
| search-by-pattern               | `hunt`               | `POST /v1/hunt`                   |
| write (signed attestation)      | `attest`             | `POST /v1/attest`                 |
| compare states                  | `diff`, `compare`    | `POST /v1/diff`, `POST /v1/compare` |
| summarize / place-anchored Q&A  | `ask`                | `POST /v1/ask`                    |
| verify a receipt                | `verify_receipt`     | `POST /v1/verify_receipt`         |
| reflect / record task outcome   | `reviews`            | `POST /v1/reviews`                |
| compose a memory token          | `memory_token`       | `POST /v1/memory_token`           |

The hosted responder is at `https://emem.dev`; local self-host runs on
port 5051. The live surface ships 72 OpenAPI-documented paths under
`/v1/*`, 50 MCP tools, 159 algorithms in the content-addressed registry,
41 bands in the manifest, 43 source schemes, and 12 fetch connectors.
Version 0.0.6, MSRV Rust 1.88. No API keys; the MCP surface is read-only
because writes need an Ed25519 secret no LLM host can manage safely.

Four discovery URLs for agent onboarding:

| URL | Purpose |
|---|---|
| `GET /openapi.json` | Full OpenAPI 3.1, every endpoint and schema |
| `GET /mcp` | MCP 2025-03-26 Streamable-HTTP transport |
| `GET /v1/agent_card` | Discover-first card: band taxonomy + tool list |
| `GET /llms.txt` | Page-scoped manifest for LLM crawlers |

A fifth surface, `https://emem.dev/humans`, is an interactive console
where every visible cell carries `data-emem-*` attributes and every
`/v1/*` call prints in a copy-as-curl / copy-as-python / copy-as-MCP
log. See "Watching humans use the API" below.

![the agent loop — discover, locate, recall, reason, verify](/docs/diagrams/04-agent-loop.svg)
*The five-step loop. After first contact, agents skip whatever they have already cached and call directly into `recall` / `find_similar` with a known cell.*

   ## Quick reference

| Resource | Live count |
|---|---|
| REST paths (OpenAPI) | 71 documented, 68 under `/v1/*` |
| MCP tools | 49 |
| Algorithms (composition recipes) | 155 |
| Band-cube slots | 35 |
| Materializer-wired band names | 118 |
| Source schemes | 43 |
| Data connectors | 12 + 6 utility modules |
| Topics (declared / live) | 26 / 11 |
| Version | 0.0.6 |

---

## Connect

All hosted-MCP clients point at `https://emem.dev/mcp` (Streamable-HTTP,
MCP 2025-03-26). For self-host on port 5051, replace the URL. Reads
require no keys.

| Client | Config |
|---|---|
| Claude Code | `.mcp.json`: `{ "mcpServers": { "emem": { "type": "http", "url": "https://emem.dev/mcp" } } }` |
| Cursor 0.42+ | `.cursor/mcp.json`: `{ "mcpServers": { "emem": { "url": "https://emem.dev/mcp" } } }` |
| Claude Desktop | Same JSON in `claude_desktop_config.json` (macOS / Linux / Windows paths) |
| Cline (VS Code) | Same JSON via MCP Settings; add `"autoApprove": [...]` for read-only tools |
| Gemini CLI | `gemini extensions install https://emem.dev/gemini-extension.json` |
| OpenAI custom GPT | GPT builder → Actions → Import from URL → `https://emem.dev/openapi.json` |
| Plain HTTP | `POST /v1/...` directly; the MCP layer is a convenience wrapper |

For runtimes without native Streamable-HTTP MCP, use the `mcp-remote`
stdio bridge: `"command": "npx", "args": ["-y", "mcp-remote", "https://emem.dev/mcp"]`.

LangChain and LlamaIndex tool examples:

```python
import os, requests
from langchain_core.tools import tool

EMEM = os.environ.get("EMEM_URL", "https://emem.dev")

@tool
def emem_recall(cell: str, bands: list[str] | None = None) -> dict:
    """Recall facts at an emem cell64. Returns signed receipt + facts."""
    body = {"cell": cell}
    if bands: body["bands"] = bands
    return requests.post(f"{EMEM}/v1/recall", json=body, timeout=30).json()
```

Full tool sets: `examples/langchain.py` (nine tools),
`examples/llamaindex.py` (six tools).

---

## The 60-second tour

Locate, recall, find similar, verify. Every response carries a
`receipt`: Ed25519 signature over a canonical preimage that cites every
contributing `fact_cid`.

   ### Locate

```bash
curl -s -X POST https://emem.dev/v1/locate \
  -H 'content-type: application/json' \
  -d '{"q":"Bengaluru"}'
```

```json
{
  "cell64": "defi.zb493.xoso.zcb6a",
  "centre": {"lat_deg": 12.971641, "lng_deg": 77.594609},
  "polygon_bbox": {"min_lat": 12.83, "max_lat": 13.14,
                   "min_lng": 77.46, "max_lng": 77.78,
                   "source": "nominatim_boundingbox"},
  "neighborhood_cells": ["...9 cells around centre..."],
  "polygon_sample_cells": ["...64 cells inside the bbox..."],
  "data_at_this_cell": {
    "live_bands_by_topic": {
      "vegetation_condition": ["indices.ndvi", "indices.evi", "modis.ndvi_mean"],
      "weather_now": ["weather.temperature_2m", "weather.cloud_cover"],
      "elevation_land_only": ["copdem30m.elevation_mean"]
    }
  },
  "via": "embedded"
}
```

`via` reports which geocoder layer answered (`embedded`, `geonames`,
`cache`, `photon`, `nominatim`, or `overture_divisions`).

   ### Recall

```bash
curl -s -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb493.xoso.zcb6a","bands":["copdem30m.elevation_mean"]}'
```

```json
{
  "facts": [{
    "band": "copdem30m.elevation_mean", "cell": "defi.zb493.xoso.zcb6a",
    "tslot": 0, "value": 910.0, "unit": "m", "confidence": 0.95,
    "derivation": {"fn_key": "open_meteo_copdem90m@1", "args": [12.971641, 77.594609]},
    "sources": [{"scheme": "open_meteo",
                 "id": "https://api.open-meteo.com/v1/elevation?latitude=12.971641&longitude=77.594609",
                 "captured_at": "2026-05-03T17:45:32Z"}],
    "signed_at": "2026-05-03T17:45:32Z",
    "signer_pubkey_b32": "777er3yihgifqmv5hmc2wwmyszgddzderzhsx6rex4yoakwomvka"
  }],
  "bands_already_attested_at_cell": ["...keys actually present at this cell..."],
  "receipt": {
    "request_id": "01KR39HY37333FD3C9PBV0F67B",
    "primitive": "emem.recall", "served_at": "2026-05-08T07:59:08Z",
    "cells": ["defi.zb493.xoso.zcb6a"],
    "fact_cids": ["cxjiu7l54ujzrpnekp24n4534yojpue4mprddbvevnqtti3lh5bq"],
    "responder_pubkey_b32": "777er3yihgifqmv5hmc2wwmyszgddzderzhsx6rex4yoakwomvka",
    "responder_key_epoch": 0,
    "schema_cid": "d24rgwlq47a5ism5vkkbiuav3wi2voewqqgy4x4ttnhdnzziyfkq",
    "registry_cid": "g5bv5bin2xlegkwmhk7bimis3l7642t5lvzfs4isenb32faxi35q",
    "signature": [254, 85, 234, "..."]
  }
}
```

Key fields:

- `request_id` is a ULID, sortable, useful as a correlation key.
- `signature` is 64 bytes Ed25519 over a BLAKE3 digest of a canonical
  preimage (see "Verify a receipt offline").
- `fact_cid` is content-addressed:
  `base32_nopad(blake3(canonical_cbor(fact))[..16])`. Identical fact at
  any responder produces the same CID.
- `bands_already_attested_at_cell` is the no-silent-fallback escape
  hatch: if your band returned empty but the cell carries data under a
  different name, this list shows what is there.

   ### Find similar

```bash
curl -s -X POST https://emem.dev/v1/find_similar \
  -H 'content-type: application/json' \
  -d '{"key":"defi.zb493.xoso.zcb6a","k":3}'
```

```json
{
  "neighbors": [
    {"cell": "defi.zb5cf.nura.zd83c", "score": 0.6537, "lat": 40.7229, "lng": -73.9987,
     "place_label_cached": "New York City, USA", "similarity_method": "cosine"},
    {"cell": "defi.zb563.noxo.xAvu", "score": 0.6426, "lat": 31.2304, "lng": 121.4737,
     "place_label_cached": "Shanghai, China", "similarity_method": "cosine"}
  ],
  "requested_k": 3, "returned_k": 3,
  "receipt": {"primitive": "emem.find_similar", "fact_cids": ["..."]}
}
```

Default scoring is cosine over the 128-D Tessera foundation embedding
(`geotessera`). Three modes exist:

- `cosine` (default): exact float dot product, full recall.
- `hamming`: sign-bit popcount against the binary sibling
  (`geotessera.bin128`); ~1000× faster, ~65% recall@10.
- `hamming_then_rerank`: Hamming triage over an EWMA-adaptive
  oversampling factor, then cosine re-rank. Matches cosine precision at
  ~16× less work.

When the binary sibling band is absent, the responder auto-derives it
inline from cosine via a TurboQuant rotation (seed
`emem.binary_embedding.turboquant.v1`).

   ### Verify

```bash
curl -s -X POST https://emem.dev/v1/verify \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb493.xoso.zcb6a","claim":{"band":"copdem30m.elevation_mean","op":"gt","value":500}}'
```

```json
{
  "verdict": true,
  "evidence": ["cxjiu7l54ujzrpnekp24n4534yojpue4mprddbvevnqtti3lh5bq"],
  "receipt": {"primitive": "emem.verify"}
}
```

`evidence` is the list of fact CIDs that produced the verdict. Quote any
one in your reply; that is what makes the answer cite-able. Pull the
underlying fact with `POST /v1/fetch {"cid":"..."}`.

In-browser receipt verification is available at `https://emem.dev/verify`
and `https://emem.dev/verify/<fact_cid>`. The page imports
`@noble/curves` and `@noble/hashes` from a pinned esm.sh URL,
reconstructs the canonical preimage, and runs Ed25519 in-page. If the
CDN imports time out (2.5 s), the page falls back to
`POST /v1/verify_receipt` and labels itself accordingly so the trust
mode never silently downgrades.

---

## Reference

   ### Asking about the corpus vs asking about a place

`POST /v1/ask` is for place-anchored questions ("is South Bombay
flood-prone", "how hot are nights in Karachi"). It geocodes the
`place` field to a `cell64` and routes the question to one of the
27 band-topics in `/v1/topics`. If you instead want to know about
the **corpus** — where the responder already has signed facts, how
dense coverage is, which bands are wired — skip `/v1/ask` and call
the introspection endpoints directly:

| Question shape | Call this instead | Returns |
|---|---|---|
| "where do you have signed facts" | `GET /v1/coverage_map.svg` | 1440 × 720 plate-carrée SVG of attested cells |
| "how many cells / facts overall" | `GET /v1/coverage_matrix` | per-band live status + freshness + signer pubkey |
| "what's the corpus density over <region>" | `GET /v1/coverage_matrix` + filter client-side, or `POST /v1/recall_polygon` with a bbox | per-cell densities you can aggregate |
| "which bands are wired here" | `GET /v1/materializers` | per-band auto-fetch registry |
| "what does this responder know about" | `GET /v1/discover` | typed bootstrap that names every catalog |

These are corpus meta-questions, not band recalls — `/v1/ask` has no
dedicated topic for them by design (a "where do you have data" query
isn't a question about any cell, and routing it through the topic
embedder will produce noise like `vegetation_condition` + `scene_classification`
on whatever the geocoder guesses). The dedicated introspection
endpoints answer in one round-trip with signed receipts where applicable.

   ### REST endpoints by category

The full machine surface is at `/openapi.json`. The tables below cover
the high-traffic groups; numbers reflect the live OpenAPI document.

   #### Discovery (8)

| Method | Path | Purpose |
|---|---|---|
| GET | `/health` | Liveness, version, corpus stats, manifest CIDs |
| GET | `/openapi.json` | OpenAPI 3.1 |
| GET | `/openapi.action.json` | OpenAI custom-GPT-action variant |
| GET | `/.well-known/emem.json` | Manifest CIDs, responder pubkey |
| GET | `/.well-known/agent.json` | Agent manifest |
| GET | `/.well-known/agent-card.json` | A2A agent-card v1 |
| GET | `/.well-known/mcp.json` | MCP server descriptor |
| GET, POST | `/mcp` | MCP JSON-RPC 2.0 transport |

   #### Introspection (15)

| Path | Purpose |
|---|---|
| `/v1/bands` | Active band ontology, offsets, dims, tempo |
| `/v1/topics` | Topic-grouped registry of bands and algorithms |
| `/v1/algorithms` | 155 composition recipes (paginated) |
| `/v1/algorithms/:key` | One recipe, formula + inputs + citation |
| `/v1/functions` | Derivation function registry |
| `/v1/sources` | Upstream connectors with license metadata |
| `/v1/materializers` | Per-band auto-fetch registry |
| `/v1/data_availability` | Per-band tempo + history bounds |
| `/v1/coverage_matrix` | Per-band live status, freshness, signer pubkey |
| `/v1/manifests` | All manifest CIDs together |
| `/v1/grid_info` | cell64 grid encoding, ground resolution |
| `/v1/errors` | Stable error code catalog |
| `/v1/tools` | MCP tool descriptors over plain HTTP |
| `/v1/schema` | CDDL/JSON schema bundle |
| `/v1/capabilities` | Sidecar extensions snapshot |

   #### Read primitives

| Method | Path | Body shape |
|---|---|---|
| POST | `/v1/recall` | `{cell, bands?, tslot?}` |
| POST | `/v1/recall_many` | `{cells:[...], bands?}` (max 256) |
| POST | `/v1/recall_polygon` | `{place?, polygon_bbox?, bands?, max_cells?, include?:["ftw_fields"]}` |
| POST | `/v1/field_boundaries` | `{place?, polygon_bbox?, zoom?}` |
| GET | `/v1/cells/:cell64` | One-shot recall, all bands |
| POST | `/v1/query_region` | `{geometry, bands?, agg?, max_cells?}` |
| POST | `/v1/compare` | `{a, b, family?}` |
| POST | `/v1/compare_bands` | `{cell, a, b, tslot_a?, tslot_b?, predicate?}` |
| POST | `/v1/find_similar` | `{key, k?, band?, mode?}` |
| POST | `/v1/trajectory` | `{cell, band, window:[a,b]}` |
| POST | `/v1/diff` | `{cell, band, tslot_a, tslot_b}` |

`query_region` `max_cells` is bbox-area-derived (target ~1 cell per
(10 km)², clamped to `[64, 1024]`).

   #### Geocoding and dispatch

| Method | Path | Body shape |
|---|---|---|
| GET, POST | `/v1/locate` | `{place\|q\|name, lat?, lng?}` |
| POST | `/v1/ask` | `{q, place?, cell?, lat?, lng?}` |
| POST | `/v1/intent` | typed `Intent`, see `emem-intent` |

   #### Physics

| Method | Path | Body shape |
|---|---|---|
| POST | `/v1/heat_solve` | `{cell, hours_ahead?, diffusivity_m2_per_s?}` |
| POST | `/v1/wave_solve` | `{coastal_cell, offshore_height_m, period_s, n_offshore_cells?}` |
| POST | `/v1/jepa_predict` | `{cell, band?, lookback_months?, forecast_horizon_months?}` |
| POST | `/v1/jepa_predict_v2` | `{cell}` |

`heat_solve` and `wave_solve` are explicit-FD solvers (CFL-stable).
`jepa_predict` is a closed-form AR(2) NDVI predictor with fixed
coefficients. `jepa_predict_v2` is **untrained today**; a metadata-only
`is_trained()` check short-circuits to a last-attested-vintage identity
baseline. The receipt carries `via: short_circuit_untrained` and
`untrained_baseline`. Read `model.honesty_warnings` from the response
before relying on the output.

   #### Verification and attestation

| Method | Path | Purpose |
|---|---|---|
| POST | `/v1/verify` | Verify a structured claim, returns verdict + evidence |
| POST | `/v1/verify_receipt` | Recompute preimage, verify Ed25519 |
| POST | `/v1/fetch` | Resolve a fact by CID |
| POST | `/v1/attest` | Submit a signed Attestation (JSON), write |
| POST | `/v1/attest_cbor` | Submit a signed Attestation (canonical CBOR), write |

`/v1/attest*` require an Ed25519 secret and are not exposed via MCP.

   #### Convenience lat/lng

GET and POST variants for `/v1/{at, elevation, ndvi, air, lst, soil,
water, forest, weather}`. Each takes `{lat, lng}` or `{place, lat, lng}`
and returns one or a few facts plus a receipt. Same data flows through
`/v1/recall` for the underlying cell.

   #### Imagery

| Method | Path | Purpose |
|---|---|---|
| GET | `/v1/cells/:cell64/info` | lat/lng/bbox |
| GET | `/v1/cells/:cell64/geojson` | Cell polygon as GeoJSON |
| GET | `/v1/cells/:cell64/recall_geojson` | FeatureCollection of recalled bands |
| GET | `/v1/cells/:cell64/scene.png` | Sentinel-2 L2A 256×256 RGB |
| GET | `/v1/cells/:cell64/scene.rgb` | Raw RGB bytes |
| GET | `/v1/coverage_map.svg` | 1440×720 corpus density map |

   #### Backfill

| Method | Path | Body shape |
|---|---|---|
| POST | `/v1/backfill` | `{cell, band, start_unix?, end_unix?, max_facts?}` |

Iterates the per-tslot upstream materializer over a window. Bands
without historical fetch return `status: "present_only"`; check
`/v1/data_availability` before picking a window.

   ### MCP tools (49)

All MCP tools are read-only (`readOnlyHint: true`). Inputs are JSON; MCP
tools omit top-level `anyOf`/`oneOf` (Claude.ai's MCP frontend accepts
only `{type, properties, required}`). Wire schemas live in
`crates/emem-mcp/src/lib.rs`.

Read primitives (14):

| Tool | Purpose | REST equivalent |
|---|---|---|
| `emem_locate` | Place name → cell64 + band inventory | POST `/v1/locate` |
| `emem_ask` | Single-shot Q&A with packaged receipts | POST `/v1/ask` |
| `emem_recall` | Facts at a cell (auto-materializes on miss) | POST `/v1/recall` |
| `emem_recall_many` | Batch recall, up to 256 cells | POST `/v1/recall_many` |
| `emem_recall_polygon` | Recall across a place's polygon | POST `/v1/recall_polygon` |
| `emem_field_boundaries` | Per-field agri polygons (Fields of The World) | POST `/v1/field_boundaries` |
| `emem_query_region` | Aggregate over a region | POST `/v1/query_region` |
| `emem_compare` | Two-cell cosine + per-band deltas | POST `/v1/compare` |
| `emem_compare_bands` | Two-band comparison at one cell | POST `/v1/compare_bands` |
| `emem_find_similar` | k-NN by embedding | POST `/v1/find_similar` |
| `emem_trajectory` | Time series for (cell, band) | POST `/v1/trajectory` |
| `emem_diff` | Signed delta between two tslots | POST `/v1/diff` |
| `emem_fetch` | Resolve a fact by CID | POST `/v1/fetch` |
| `emem_backfill` | Materialize history in a window | POST `/v1/backfill` |

Domain shortcuts (9, one-shot locate→recall→aggregate):

| Tool | Topic |
|---|---|
| `emem_at` | Mixed bands at a place |
| `emem_ndvi` | Vegetation condition |
| `emem_air` | CAMS air quality (PM2.5, NO2, O3) |
| `emem_lst` | MODIS land-surface temperature |
| `emem_soil` | SoilGrids carbon, pH, texture |
| `emem_water` | Surface water recurrence + indices |
| `emem_forest` | Hansen GFC + canopy bands |
| `emem_weather` | Now-only met.no nowcast |
| `emem_elevation` | Cop-DEM 30 m elevation |

Physics (4): `emem_heat_solve`, `emem_wave_solve`, `emem_jepa_predict`,
`emem_jepa_predict_v2`.

Verify (2): `emem_verify`, `emem_verify_receipt`.

Introspection (20): `emem_bands`, `emem_functions`, `emem_sources`,
`emem_schema`, `emem_errors`, `emem_manifests`, `emem_capabilities`,
`emem_grid_info`, `emem_coverage_matrix`, `emem_coverage_map`,
`emem_materializers`, `emem_data_availability`, `emem_fleet`,
`emem_temporal_route`, `emem_algorithms`, `emem_explain_algorithm`,
`emem_topics`, `emem_cell_scene_rgb`, `emem_cell_geojson`, `emem_intent`.

The MCP catalog omits two L2 write surfaces. `emem_attest` and
`emem_challenge` need an Ed25519 secret the attester controls. LLM
hosts cannot manage keys safely, so writes go through plain REST from a
process you control: `POST /v1/attest` (JSON) or `POST /v1/attest_cbor`
(canonical CBOR). Key generation, signing, and rotation are documented
in `docs/ATTESTING.md`; the schema is in `/openapi.json`.

---

## Algorithms: triple-encoder consensus

The 155-entry algorithm registry includes the standard agronomic and
hydrological indices (NDVI, NBR, NDWI, walkability, heat index, RUSLE).
The differentiator is the **triple-encoder consensus pattern**: when
three independent foundation encoders flag the same cell, the answer
ships with `agreement: all_three`, `two_of_three`, or `one_or_none`.

`independent_receptive_field_agreement` is the mathematical claim. Clay,
Prithvi, and Tessera see different inputs (10-band S2 256×256 vs. 6-band
HLS 224×224 vs. annual learned embedding) and were trained on different
corpora. Joint agreement at a cell is unlikely under noise.

Each tuned threshold carries a `learned_from` citation; the
`parameters` block on every `AlgorithmSpec` is typed and accessor-driven
(`Algorithm::param_f64("consensus_threshold")`).

| Algorithm | Recipe | Gate | Notes |
|---|---|---|---|
| `clay_prithvi_tessera_triple_consensus@1` | Year-on-year change vector from all three encoders | 0.15 | Base recipe; tunable via `parameters.consensus_threshold` |
| `deforestation_triple@1` | Triple consensus + Hansen GFC mask uplift | 0.20 | Verdict `hansen_confirmed` when GFC agrees |
| `wetland_change_triple@1` | JRC GSW recurrence delta substitutes the Tessera leg | 0.10 | For monsoon / coastal wetland flux |
| `urban_expansion_triple@1` | Overture buildings delta + S2 B11 SWIR corroboration | 0.20 | Co-registered building footprint truth |
| `disaster_anomaly_triple@1` | Spatial only (no temporal leg) | 2-σ neighbour z-score | Adaptive gate, no fixed threshold |
| `climate_archetype_triple@1` | 12-class Köppen-Geiger classifier with type-locality centroids | n/a | Seeded from `climate_archetype_centroids_v1.json` (Beck et al. 2018) |
| `coastal_erosion_triple@1` | Same as base + bathymetry clamp `[-5 m, +5 m]` | 0.12 | Restricts evaluation to active-coastline cells |

Every gate threshold traces back through `learned_from` to a referee
paper or operational test. Re-executable: pass the algorithm key to
`POST /v1/algorithms/:key` for the formula body, then call `/v1/recall`
for the input bands and replay the math locally.

   ### Foundation embeddings, sidecar-resident

Four GPU encoders co-reside in a 20 GB VRAM budget at the live
responder.

| Band | Encoder | Output | Input shape | Latency |
|---|---|---|---|---|
| `clay_v1` | Clay v1.5 | 1024-D CLS | Sentinel-2 L2A 10-band 256×256 | ~12 ms warm |
| `prithvi_eo2` | IBM-NASA Prithvi-EO-2.0-300M-TL | 1024-D CLS | HLS V2 6-band 224×224 | ~13 ms warm |
| `galileo` | Galileo (variant via `EMEM_GALILEO_VARIANT`, default `base`) | variant-dependent | Sentinel-2 only wired | warm |
| (none) | JEPA v2 dynamics | UNTRAINED | n/a | short-circuits |

The Clay encoder ships its DINOv2 teacher
(`vit_large_patch14_reg4_dinov2.lvd142m`) pre-staged at boot so
`HF_HUB_OFFLINE=1` holds. Galileo's multimodal scaffold (S1, ERA5, TC,
VIIRS, SRTM, DW, WC, LandScan, location) is present but **only the S2
modality is wired** today; the rest are zero-masked. Do not claim full
multimodal coverage.

The JEPA v2 dynamics head is untrained. Inference short-circuits via a
metadata-only `is_trained()` check before any ONNX or sidecar call and
returns the last attested vintage as an identity baseline. Receipt
carries `via: short_circuit_untrained` and `untrained_baseline`. Treat
the output as a no-op.

Tessera (`geotessera`, 128-D annual) is consumed as an upstream
foundation fact, not run in-process. Today 2024 is the reliably-served
vintage; historical backfill is partial. Source: `dl2.geotessera.org`.

---

## Foundation-embedding fan-out from `/v1/ask`

`/v1/ask` dispatches to `clay_v1`, `prithvi_eo2`, and `geotessera` in
parallel when the question matches either intent:

- **Similarity**: "find places like X", "similar to", "look-alike",
  "where else does this pattern appear".
- **Change**: "what changed", "year over year", "deforestation",
  "anomaly", "before-after".

The response carries a top-level `foundation_embeddings` envelope:

```json
{
  "foundation_embeddings": {
    "per_encoder": {
      "clay_v1": {"neighbors": [{"cell": "...", "score": 0.91}, "..."]},
      "prithvi_eo2": {"neighbors": ["..."]},
      "geotessera": {"neighbors": ["..."]}
    },
    "consensus": {
      "all_three": ["defi.zb493.xoso.zcb6a", "..."],
      "two_of_three": ["..."],
      "one_or_none": ["..."]
    },
    "budget_ms": 4000,
    "degraded_reason": null
  }
}
```

The 4 s budget is read from
`clay_prithvi_tessera_triple_consensus@1.parameters.ask_timeout_ms`. On
timeout, `degraded_reason: "foundation_embedding_timeout"`; the rest of
the answer still ships with its receipt.

---

## Signed Absence

When a band has no data at a cell, the responder does not return `404`
or an empty array. It returns a signed **Absence** fact with a typed
reason:

| Reason | Meaning |
|---|---|
| `unavailable_capability` | Sidecar extension missing (GPU off, encoder offline) |
| `outside_coverage` | Cell falls outside the upstream product footprint |
| `gpu_unavailable` | Required GPU model is loaded but not serving |
| `archetype_seed_unavailable` | Climate/place archetype centroids not loaded |
| `no_auto_materializer_registered` | Band reserved in cube but no connector wired |
| `present_only` | Band is now-only (e.g. met.no nowcast); past tslots return absence |

Absence facts are content-addressed and citable in receipts. An agent
that quotes `evidence: ["absence_cid_..."]` in its answer is making a
verifiable claim that "no data exists for (cell, band, tslot)" rather
than guessing from a `404`.

---

## Auto-materialize on miss

An empty `/v1/recall` on a cell with a registered materializer triggers
an upstream fetch (Sentinel-2 STAC + COG range reads, Copernicus DEM,
JRC GSW, Hansen GFC, Overture, ...), signs the result under the
responder's identity, persists it, and returns it in the same response.

Latency: ~180 ms cold (one upstream round-trip), ~10 ms warm (sled
cache). Every cell on Earth answers; nothing needs to be pre-seeded.

20 materializers are registered today. When the band key is not even in
the registry, the response is explicit, not silent:

```json
{
  "facts": [],
  "materialize_notes": [{
    "band": "foo.bar", "status": "skipped",
    "reason": "no_auto_materializer_registered: no upstream connector wired for band=foo.bar; submit a signed Attestation via /v1/attest_cbor to seed it. Call GET /v1/bands to see all known band keys."
  }],
  "bands_already_attested_at_cell": ["...keys actually present..."]
}
```

That is the no-silent-fallbacks contract: empty result must distinguish
"wrong query" from "place is empty."

---

## Geocoder cascade

`/v1/locate` walks layers in order; `via` reports which one answered.

| Layer | Source | Notes |
|---|---|---|
| `wide_bbox` | Curated table for ambiguous large polities | Exits with full bbox + admin polygon |
| `embedded` | In-binary gazetteer | Major cities, country capitals |
| `geonames` | GeoNames cities-5000, 68,581 places, in-process | Unified gazetteer surface |
| `cache` | sled cache | Hits prior Photon/Nominatim lookups |
| `photon` | komoot Photon (primary live) | Fuzzy matching |
| `nominatim` | OSM Nominatim (secondary) | Fallback |
| `overture_divisions` | Overture `divisions/division_area` | Admin-boundary fix when Nominatim returns a POI (e.g. Karnal-district courthouse case) |

The unified gazetteer is `emem_fetch::geonames`; all hand-curated
in-binary tables have been retired in favour of GeoNames cities-5000.

---

## Recipes

The 60-second tour already shows the basic locate / recall / find_similar
/ verify shapes. The recipes below cover the cases an agent reaches for
next.

   ### 1. Verify with windowed predicate

```bash
curl -s -X POST https://emem.dev/v1/verify \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb493.xoso.zcb6a","claim":{"band":"modis.ndvi_mean","op":"gt","value":0.3,"window":[1700000000,1735689600],"agg":"mean"}}'
```

`op` is one of `eq | ne | lt | le | gt | ge | in | ni | exists | absent`.
`claim.window: [a, b]` plus `claim.agg: any|all|mean|min|max` evaluates
over a tslot window. `mode: "zk"` returns an explicit error (reserved).
Quote `evidence` CIDs in your reply.

   ### 2. Signed delta between two tslots

```bash
curl -s -X POST https://emem.dev/v1/diff \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb493.xoso.zcb6a","band":"indices.ndvi","tslot_a":0,"tslot_b":1}'
```

When both tslots exist, returns a signed
`DerivativeFact{op:"delta", parents:[a_cid, b_cid]}`. When only one side
is attested:

```json
{"code":"cid_not_found","message":"CidNotFound: no fact at tslot_b=1 for (defi.zb493.xoso.zcb6a,indices.ndvi)"}
```

Seed history with `/v1/backfill` first. Diff rejects `tslot_a == tslot_b`
(tautology guard).

   ### 3. Region aggregate

```bash
curl -s -X POST https://emem.dev/v1/query_region \
  -H 'content-type: application/json' \
  -d '{"geometry":"cells:defi.zb493.xoso.zcb6a,defi.zb493.xozI.zcb6a","bands":["copdem30m.elevation_mean"],"agg":"mean"}'
```

`geometry` accepts `cell64` or `cells:c1,c2,c3,...`. `agg` is one of
`mean | median | p90 | vector_centroid`. Vector centroid returns the
dimension-wise mean of vector bands. `max_cells` defaults to
bbox-area-derived `[64, 1024]`.

   ### 4. Materialize history

```bash
curl -s -X POST https://emem.dev/v1/backfill \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb493.xoso.zcb6a","band":"weather.temperature_2m","max_facts":2}'
```

Each step carries `status` (`cached | materialized | present_only |
error`). `present_only` is the no-silent-fallbacks discipline: the band
answers, but the upstream is now-only. For bands with real history
(`surface_water.recurrence`, `modis.ndvi_mean`), check
`/v1/data_availability` before picking a window.

   ### 5. Sentinel-2 thumbnail

```bash
curl -s "https://emem.dev/v1/cells/defi.zb493.xoso.zcb6a/scene.png?max_cloud=80" \
  --output bengaluru.png
```

```
HTTP/1.1 200 OK
content-type: image/png
x-emem-scene-item-id: S2B_43PGQ_20260502_0_L2A
x-emem-scene-datetime: 2026-05-02T05:25:16.655000Z
x-emem-scene-cloud-cover: 27.23
x-emem-scene-epsg: 32643
```

Pipeline is pure-Rust: STAC search → HTTP-Range COG read → 2-98
percentile stretch → PNG encode. No GDAL. 256×256 px ≈ 2.56 km × 2.56 km
at S2's 10 m native pitch. Raise `max_cloud` to 60-80 for tropical /
cloudy regions.

   ### 6. EUDR pre-screen

```bash
curl -s -X POST https://emem.dev/v1/intent \
  -H 'content-type: application/json' \
  -d '{"kind":"verify","claim":{"algorithm":"deforestation_triple@1","cell":"defi.zb493.xoso.zcb6a","window":["2020-12-31","2024-12-31"]}}'
```

Returns the triple-consensus verdict, the Hansen GFC mask uplift, and a
signed receipt citing every contributing fact CID. Drop the receipt
into an EUDR DDS submission.

---

## Verify a receipt offline

The preimage the responder hashes and Ed25519-signs is:

```
blake3(
    request_id || "|" || served_at  || "|" || primitive || "|" ||
    cell1 "," cell2 "," ... || "|" || cid1 "," cid2 "," ...
)
```

The trailing commas after the last cell and the last cid are intentional
(the signing loop appends `","` after every entry). The pubkey is loaded
from `responder_pubkey_b32` (base32-nopad) or the byte array; both encode
the same 32-byte Ed25519 verifying key.

Self-contained Python verification:

```python
"""Verify an emem receipt offline. Reproduces /v1/verify_receipt locally."""
import base64
import requests
import blake3
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
from cryptography.exceptions import InvalidSignature

EMEM = "https://emem.dev"

def b32_nopad_decode_lower(s: str) -> bytes:
    s = s.upper()
    pad = (-len(s)) % 8
    return base64.b32decode(s + "=" * pad)

resp = requests.post(f"{EMEM}/v1/recall",
                     json={"cell": "defi.zb493.xoso.zcb6a",
                           "bands": ["copdem30m.elevation_mean"]},
                     timeout=30).json()
r = resp["receipt"]

h = blake3.blake3()
h.update(r["request_id"].encode()); h.update(b"|")
h.update(r["served_at"].encode());  h.update(b"|")
h.update(r["primitive"].encode());  h.update(b"|")
for c in r["cells"]:
    h.update(c.encode()); h.update(b",")
h.update(b"|")
for c in r["fact_cids"]:
    h.update(c.encode()); h.update(b",")
preimage_digest = h.digest()

pk_bytes  = b32_nopad_decode_lower(r["responder_pubkey_b32"])
sig_bytes = bytes(r["signature"])
assert len(pk_bytes) == 32 and len(sig_bytes) == 64

pk = Ed25519PublicKey.from_public_bytes(pk_bytes)
try:
    pk.verify(sig_bytes, preimage_digest)
    print(f"VALID, signed by {r['responder_pubkey_b32']}")
except InvalidSignature:
    print("INVALID")

echo = requests.post(f"{EMEM}/v1/verify_receipt",
                     json={"receipt": r}, timeout=30).json()
assert echo["valid"] is True
assert echo["preimage_blake3_hex"] == preimage_digest.hex()
```

The responder pubkey is the only trust anchor; it is published at
`/.well-known/emem.json`. The in-browser `/verify/<fact_cid>` page runs
the same procedure with `@noble/curves` and `@noble/hashes`.

---

## Grids, time, and gotchas

   ### cell64 grid

The active codec is `cell64-geo-21x22`: 21 lat bits, 22 lng bits, packed
to a u64 and base-1024 bigram-encoded as four `.`-joined groups (e.g.
`defi.zb493.xoso.zcb6a`). `GET /v1/grid_info` reports the actual ground
resolution: `lat_axis_metres_at_equator: 9.54`,
`lng_axis_metres_at_equator: 9.55`, `lng_axis_metres_at_lat_60: 4.77`.
Latitude pitch is uniform; longitude pitch narrows with cos(lat) so
cells become taller than wide above ~lat 30°.

The spec target `aperture-7 hex DGGS` (~3.4 m edge) is reserved in the
manifest but not active in this build. Receipts pin the active manifest
CID, so historical answers do not drift when the migration lands.

   ### tslot

`tslot` is a `u64` slot offset, anchored at Unix epoch since release
0.0.3 (was an internal `emem-2026` anchor in 0.0.2; facts attested under
the old anchor fail with `NotGeoCell` rather than misplace silently).
Tempo classes:

| Class | Examples |
|---|---|
| `static` | `copdem30m.elevation_mean`, `surface_water.recurrence`, `koppen.major_class` |
| `annual` | `geotessera.YYYY`, `hansen.loss_year`, `esa_worldcover.lc_2021` |
| `monthly` / `8day` | `modis.ndvi_mean`, `modis.lst_day_8day` |
| `daily` | `era5.precip`, `era5.t2m` |
| `now_only` / `ultrafast` | `weather.*` (met.no nowcast) |

Check `/v1/data_availability` before picking a tslot or backfill window.

   ### Timeouts

- Materializer per-upstream timeout: 30 s
  (`EMEM_MATERIALIZER_TIMEOUT_SECS`).
- Gateway timeout: 180 s (`EMEM_TIMEOUT_SECS`). Slow paths (Cop-DEM, S2
  STAC + COG range reads for an uncached scene) sometimes need the full
  window.
- Treat 5xx as transient and retry once; treat 4xx as permanent.

   ### find_similar caveats

- Self-cell is filtered. Duplicate cells across tslots are deduped
  (max-score wins). NaN scores stripped.
- Non-vector band returns Protocol error with a "use /v1/recall to
  inspect" hint, not an empty list.
- The `filter` field exists in the schema but is not yet wired.

   ### Receipts are immutable

`fact_cid = base32_nopad(blake3(canonical_cbor(fact))[..16])`. Same fact
at two responders produces byte-identical CBOR and the same CID.
Re-encoding the fact body changes the CID and the original signature
stops verifying. Always fetch by CID through `/v1/fetch` or
`GET /v1/facts/:cid`.

---

## Watching humans use the API: `/humans`

`https://emem.dev/humans` is an interactive surface designed primarily
for agents that learn the protocol by observation. The page is its own
API console.

   ### Surface

| View | Behaviour |
|---|---|
| Constellation field | Attested cells as stars; brightness `log10(1+facts_count)`, hue is dominant band family |
| Embedding projection | 2-D power-iteration PCA over 128-D Tessera vectors for the densest 80 cells; `p` key toggle |
| Lasso → recall_polygon | Drag or `L` key; picks the highest-count band and renders results inline |
| Force-graph | Verlet layout in canvas2D, top 200 cells, edges by lat/lng proximity |
| Registry view | Pure-SVG Poincaré disk over eight manifest registries |
| Log view | Rekor-style scroll of `/v1/coverage_matrix` rows with inline `verify` button |
| Try-it drawer (T) | Live REST playground; copy-as-curl / copy-as-py / copy-as-MCP pivots |
| Manifest grid, ontology graph, glossary, human/raw toggle | Full affordance map in the `/humans` source |

   ### Agent-readable

Every visible cell, fact, manifest CID, pubkey, and interactive control
carries `data-emem-*` attributes:

```html
<div class=fact data-emem-cell="defi.zb493.xoso.zcb6a"
                data-emem-band="weather.temperature_2m"
                data-emem-fact-cid="qi3jo4sqcg...l2hgjtwm"
                data-emem-tslot="1778237046"
                data-emem-verified="true">...</div>

<button data-emem-action=lasso-toggle>lasso<kbd>L</kbd></button>
```

A scraper walking `[data-emem-action]` learns the full affordance map.
Every `/v1/*` call the page makes prints in a structured console log;
hover any row for **curl**, **py**, **mcp**, **↻** pivots. The MCP
pivot emits a JSON-RPC 2.0 `tools/call` payload with the right tool
name from the `emem-mcp` registry.

   ### Sibling artifacts

- `/humans.json` (`schema=emem.humans.v1`): manifest CIDs, responder
  pubkey, top-10 bands, dense-20 cells.
- `/humans/llms.txt`: page-scoped manifest of endpoints invoked, data
  attributes, trust model.
- `/verify`, `/verify/<fact_cid>`: in-browser Ed25519 verifier with
  `POST /v1/verify_receipt` fallback on CDN timeout.

URL state encoding (saved-query convention):
`https://emem.dev/humans?cell=<cell>&proj=embed&mode=log&layout=noLeft,focus`.

---

## Where to read more

- `https://emem.dev/openapi.json`: every endpoint, every schema.
- `https://emem.dev/.well-known/emem.json`: manifest CIDs + responder
  pubkey.
- `https://emem.dev/v1/agent_card`, `/v1/quickstart`: discover-first
  card + onboarding flow.
- `docs/developers/architecture.md`, `docs/protocol.md`, `docs/whitepaper.md`,
  `docs/ATTESTING.md`: deeper protocol and math docs, write path.
- `https://emem.dev/humans`, `/humans.json`, `/humans/llms.txt`:
  interactive console + JSON twin.
- `crates/emem-mcp/src/lib.rs`: canonical MCP tool descriptors.
- `crates/emem-primitives/src/*`: the 12 primitive shapes.
- `examples/agent-walkthroughs.md`, `examples/langchain.py`,
  `examples/llamaindex.py`.

License: Apache-2.0. Contact: avijeet@vortx.ai.
