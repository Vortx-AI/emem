# emem agent guide

## What this is

A spatial-memory protocol for AI agents. Three things it does: (a) locate
a place by name → cell64, (b) recall signed facts at that cell, (c) find
places similar to a given cell. Every response carries an Ed25519 receipt
that any agent can verify offline. The hosted responder is at
`https://emem.dev`; local self-host runs on port 5051. 73 REST endpoints,
36 MCP tools; the MCP surface is entirely read-only. Writes (attestation,
challenge) are REST-only because they require an Ed25519 secret no LLM
host can manage safely.

Three discovery URLs: `GET /openapi.json` (full machine surface),
`GET /.well-known/emem.json` (manifest CIDs + responder pubkey),
`GET /v1/agent_card` (discover-first card with band taxonomy + tool list).

There's also a fourth surface designed for *learning* the protocol by
observation: `https://emem.dev/humans` is a single-page interactive
console where every visible cell carries `data-emem-*` attributes and
every `/v1/*` call the page makes prints in a live log pane with
copy-as-curl / copy-as-python / copy-as-MCP pivots. JSON twin at
`/humans.json` (`schema=emem.humans.v1`); page-scoped manifest at
`/humans/llms.txt`. See "Watching humans use the API" below.

---

## Connect

Pick the client you use and paste the block. All hosted-MCP clients point
at `https://emem.dev/mcp` (Streamable-HTTP per MCP 2025-03-26). For
self-host, replace the URL.

### Claude Code

Save as `.mcp.json` at the project root:

```json
{ "mcpServers": { "emem": { "type": "http", "url": "https://emem.dev/mcp" } } }
```

For runtimes without native Streamable-HTTP MCP, use the `mcp-remote`
stdio bridge: `"command": "npx", "args": ["-y", "mcp-remote", "https://emem.dev/mcp"]`.

### Cursor

Save as `.cursor/mcp.json` (or Settings → MCP). Cursor 0.42+ speaks
Streamable-HTTP MCP natively:

```json
{ "mcpServers": { "emem": { "url": "https://emem.dev/mcp" } } }
```

### Cline (VS Code)

Open Cline → MCP server icon → Edit MCP Settings:

```json
{
  "mcpServers": {
    "emem": {
      "url": "https://emem.dev/mcp",
      "disabled": false,
      "autoApprove": ["emem_recall", "emem_compare", "emem_find_similar",
                       "emem_bands", "emem_manifests", "emem_agent_card"]
    }
  }
}
```

### Claude Desktop

Edit `~/Library/Application Support/Claude/claude_desktop_config.json` (macOS),
`~/.config/Claude/claude_desktop_config.json` (Linux), or
`%APPDATA%\Claude\claude_desktop_config.json` (Windows). Claude Desktop
≥ 0.10 infers the Streamable-HTTP transport from the URL scheme:

```json
{ "mcpServers": { "emem": { "url": "https://emem.dev/mcp" } } }
```

### Gemini CLI

```bash
gemini extensions install https://emem.dev/gemini-extension.json
```

That manifest wires the MCP endpoint plus context links (`agent_card`,
`openapi.json`, `llms.txt`).

### OpenAI custom GPT actions

GPT builder → Actions → Add → Import from URL → paste
`https://emem.dev/openapi.json`. Authentication: None. Privacy policy:
`https://emem.dev/privacy`.

### LangChain (Python)

```bash
pip install langchain langchain-core requests
```

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

Nine-tool set at [examples/langchain.py](https://emem.dev/examples/langchain.py).

### LlamaIndex

```bash
pip install llama-index-core requests
```

```python
import requests
from llama_index.core.tools import FunctionTool

def emem_recall(cell: str) -> dict:
    return requests.post("https://emem.dev/v1/recall",
                         json={"cell": cell}, timeout=30).json()

tools = [FunctionTool.from_defaults(fn=emem_recall, name="emem_recall",
    description="Recall signed facts at an emem cell64.")]
```

Six-tool set at [examples/llamaindex.py](https://emem.dev/examples/llamaindex.py).

### Plain HTTP

If you can speak HTTP, hit `https://emem.dev/v1/...` directly — the MCP
layer is a convenience wrapper over the same primitives. No keys are
required for reads.

---

## The 60-second tour

Locate a place, recall a fact, find similar cells, verify a claim. Every
response carries a `receipt` — an Ed25519-signed envelope citing every
`fact_cid` that contributed.

### 1. Locate

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
  "neighborhood_cells": [/* 9 cells around centre */],
  "polygon_sample_cells": [/* 64 cells inside the bbox */],
  "data_at_this_cell": {
    "live_bands_by_topic": {
      "vegetation_condition": ["indices.ndvi", "indices.evi", "modis.ndvi_mean"],
      "weather_now": ["weather.temperature_2m", "weather.cloud_cover"],
      "elevation_land_only": ["copdem30m.elevation_mean"]
    },
    "algorithms_for_topic": {
      "flood_risk_composite": ["flood_risk@2", "route_flood_exposure@1"],
      "urban_livability": ["walkability_score@1", "bikeability_score@1"]
    }
  },
  "via": "embedded"
}
```

`via` reports which geocoder path answered (embedded gazetteer, cache,
Photon, or Nominatim). See the geocoder note in Gotchas.

### 2. Recall

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
  "bands_already_attested_at_cell": [/* 43 actual keys at this cell */],
  "receipt": {
    "request_id": "01KR39HY37333FD3C9PBV0F67B",
    "primitive": "emem.recall", "served_at": "2026-05-08T07:59:08Z",
    "cells": ["defi.zb493.xoso.zcb6a"],
    "fact_cids": ["cxjiu7l54ujzrpnekp24n4534yojpue4mprddbvevnqtti3lh5bq"],
    "responder_pubkey_b32": "777er3yihgifqmv5hmc2wwmyszgddzderzhsx6rex4yoakwomvka",
    "responder_key_epoch": 0,
    "schema_cid": "d24rgwlq47a5ism5vkkbiuav3wi2voewqqgy4x4ttnhdnzziyfkq",
    "registry_cid": "yjvd6cxkhwykc5b7l43cimeoz2flm72p2qaaqlryic6combde27q",
    "signature": [254, 85, 234, /* ...64 bytes */]
  }
}
```

Things to notice:

- `request_id` is a ULID — sortable, useful as a correlation key.
- `signature` is 64 bytes Ed25519 over a known preimage (see "Verify a receipt offline").
- `fact_cid` is content-addressed: `base32_nopad(blake3(canonical_cbor(fact))[..16])`.
  Same fact at any responder produces byte-identical bytes and the same CID.
- `bands_already_attested_at_cell` is the no-silent-fallback escape hatch:
  if your requested band returned empty but the cell has data under a
  different name, this list tells you what's actually there.

### 3. Find similar

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
     "place_label_cached": "Shanghai, China", "similarity_method": "cosine"},
    {"cell": "defi.zb53b.lazI.himE", "score": 0.6374, "lat": 27.7083, "lng": 85.3206,
     "similarity_method": "cosine"}
  ],
  "requested_k": 3, "returned_k": 3,
  "receipt": {... primitive: "emem.find_similar", fact_cids: 3 entries ...}
}
```

Default scoring is cosine over the 128-D Tessera foundation embedding
(`geotessera`). For 1000× speedup at ~65% recall@10, set
`"mode":"hamming"` against the binary sibling band; for cosine-quality
results at ~16× less work, `"mode":"hamming_then_rerank"`.

### 4. Verify

```bash
curl -s -X POST https://emem.dev/v1/verify \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb493.xoso.zcb6a","claim":{"band":"copdem30m.elevation_mean","op":"gt","value":500}}'
```

```json
{
  "verdict": true,
  "evidence": ["cxjiu7l54ujzrpnekp24n4534yojpue4mprddbvevnqtti3lh5bq"],
  "receipt": {... primitive: "emem.verify" ...}
}
```

`evidence` is the list of fact CIDs that produced the verdict. Pull any
one with `POST /v1/fetch {"cid":"..."}` to see the underlying signed
fact.

---

## Reference

### REST endpoints by category

#### Discovery (8)

| Method | Path | Purpose |
|---|---|---|
| GET | `/health` | Liveness, version, corpus stats, manifest CIDs |
| GET | `/openapi.json` | OpenAPI 3.1 — full machine surface |
| GET | `/openapi.action.json` | OpenAI custom-GPT-action variant |
| GET | `/.well-known/emem.json` | Manifest CIDs, responder pubkey, operator info |
| GET | `/.well-known/agent.json` | Agent manifest |
| GET | `/.well-known/agent-card.json` | A2A agent-card v1 |
| GET | `/.well-known/mcp.json` | MCP server descriptor |
| GET, POST | `/mcp` | MCP JSON-RPC 2.0 transport |

#### Introspection (14)

| Path | Purpose |
|---|---|
| `/v1/bands` | Active band ontology — offsets, dims, tempo |
| `/v1/topics` | Topic-grouped registry of bands and algorithms |
| `/v1/algorithms` | 149 composition recipes (full catalog) |
| `/v1/algorithms/:key` | One recipe — formula, inputs, citation |
| `/v1/functions` | Derivation function registry |
| `/v1/sources` | Upstream connectors with license metadata |
| `/v1/materializers` | Per-band auto-fetch registry |
| `/v1/data_availability` | Per-band tempo + history bounds |
| `/v1/coverage_matrix` | Per-band live status, freshness, signer pubkey |
| `/v1/manifests` | All manifest CIDs together |
| `/v1/grid_info` | cell64 grid encoding (lat/lng bits, ground resolution) |
| `/v1/errors` | Stable error code catalog |
| `/v1/tools` | MCP tool descriptors over plain HTTP |
| `/v1/schema` | CDDL/JSON schema bundle |

#### Read primitives (11)

| Method | Path | Body shape |
|---|---|---|
| POST | `/v1/recall` | `{cell, bands?, tslot?}` |
| POST | `/v1/recall_many` | `{cells:[...], bands?}` (max 256) |
| POST | `/v1/recall_polygon` | `{place?, polygon_bbox?, bands?, max_cells?, include?:["ftw_fields"]}` |
| POST | `/v1/field_boundaries` | `{place?, polygon_bbox?, zoom?}` — per-field agri polygons from Fields of The World (CC-BY-4.0) |
| GET | `/v1/cells/:cell64` | One-shot recall, all bands |
| POST | `/v1/query_region` | `{geometry, bands?, agg?}` |
| POST | `/v1/compare` | `{a, b, family?}` |
| POST | `/v1/compare_bands` | `{cell, a, b, tslot_a?, tslot_b?, predicate?}` |
| POST | `/v1/find_similar` | `{key, k?, band?, mode?}` |
| POST | `/v1/trajectory` | `{cell, band, window:[a,b]}` |
| POST | `/v1/diff` | `{cell, band, tslot_a, tslot_b}` |

#### Geocoding & dispatch (4)

| Method | Path | Body shape |
|---|---|---|
| GET, POST | `/v1/locate` | `{place\|q\|name, lat?, lng?}` |
| POST | `/v1/ask` | `{q, place?, cell?, lat?, lng?}` |
| POST | `/v1/intent` | typed `Intent` — see `emem-intent` |

#### Physics (4)

| Method | Path | Body shape |
|---|---|---|
| POST | `/v1/heat_solve` | `{cell, hours_ahead?, diffusivity_m2_per_s?}` |
| POST | `/v1/wave_solve` | `{coastal_cell, offshore_height_m, period_s, n_offshore_cells?}` |
| POST | `/v1/jepa_predict` | `{cell, band?, lookback_months?, forecast_horizon_months?}` |
| POST | `/v1/jepa_predict_v2` | `{cell}` |

`heat_solve` and `wave_solve` are real explicit-FD solvers (CFL-stable).
`jepa_predict` is a closed-form AR(2) NDVI predictor with fixed
coefficients. `jepa_predict_v2` is currently `untrained_baseline` — the
ONNX artifact is the residual-zero-init sentinel that returns
last_input_vintage. Always read `model.honesty_warnings` from the
response — when it contains `untrained_baseline`, treat the prediction as
a no-op.

#### Verification & attestation (5)

| Method | Path | Purpose |
|---|---|---|
| POST | `/v1/verify` | Verify a structured claim, returns verdict + evidence |
| POST | `/v1/attest` | Submit a signed Attestation (JSON) — write |
| POST | `/v1/attest_cbor` | Submit a signed Attestation (canonical CBOR) — write |
| POST | `/v1/verify_receipt` | Recompute the preimage, verify Ed25519 |
| POST | `/v1/fetch` | Resolve a fact by CID |

`/v1/attest` and `/v1/attest_cbor` need an Ed25519 secret. They are not
exposed via MCP — see "What MCP intentionally does not expose" below.

#### Convenience lat/lng (18)

GET and POST variants for `/v1/{at, elevation, ndvi, air, lst, soil,
water, forest, weather}`. Take `{lat, lng}` (POST `{place, lat, lng}`),
return one or a few facts plus a receipt. Useful when you don't want to
think about cell64. The same data flows through `/v1/recall` for the
underlying cell.

#### Imagery (6)

| Method | Path | Purpose |
|---|---|---|
| GET | `/v1/cells/:cell64/info` | lat/lng/bbox |
| GET | `/v1/cells/:cell64/geojson` | Cell polygon as GeoJSON |
| GET | `/v1/cells/:cell64/recall_geojson` | GeoJSON FeatureCollection of recalled bands |
| GET | `/v1/cells/:cell64/scene.png` | Sentinel-2 L2A 256×256 RGB |
| GET | `/v1/cells/:cell64/scene.rgb` | Raw RGB bytes |
| GET | `/v1/coverage_map.svg` | 1440×720 corpus density map |

#### Backfill (1)

| Method | Path | Body shape |
|---|---|---|
| POST | `/v1/backfill` | `{cell, band, start_unix?, end_unix?, max_facts?}` |

Iterates the per-tslot upstream materializer over a window. Bands
without historical fetch (e.g. `weather.*` from the met.no nowcast)
return `status: "present_only"` for past tslots — check
`/v1/coverage_matrix.history_available_from` before calling.

### MCP tools (36)

All MCP tools are read-only (`readOnlyHint: true`). Inputs are JSON; MCP
tools deliberately omit top-level `anyOf`/`oneOf` (Claude.ai's MCP
frontend only accepts the `{type, properties, required}` subset).
Wire schemas live in `crates/emem-mcp/src/lib.rs`.

| Tool | Level | Purpose | REST equivalent |
|---|---|---|---|
| `emem_locate` | L0 | Place name → cell64 + band inventory | POST `/v1/locate` |
| `emem_ask` | L0 | Single-shot Q&A with packaged receipts | POST `/v1/ask` |
| `emem_recall` | L0 | Facts at a cell (auto-materializes on miss) | POST `/v1/recall` |
| `emem_recall_polygon` | L0 | Recall across a place's polygon (optionally with `include:["ftw_fields"]`) | POST `/v1/recall_polygon` |
| `emem_field_boundaries` | L0 | Per-field agricultural-boundary polygons (Fields of The World, CC-BY-4.0) | POST `/v1/field_boundaries` |
| `emem_query_region` | L0 | Aggregate over a region | POST `/v1/query_region` |
| `emem_compare` | L0 | Two-cell cosine + per-band deltas | POST `/v1/compare` |
| `emem_compare_bands` | L0 | Two-band comparison at one cell | POST `/v1/compare_bands` |
| `emem_find_similar` | L0 | k-NN by embedding | POST `/v1/find_similar` |
| `emem_trajectory` | L0 | Time series for (cell, band) | POST `/v1/trajectory` |
| `emem_diff` | L0 | Signed delta between two tslots | POST `/v1/diff` |
| `emem_fetch` | L0 | Resolve a fact by CID | POST `/v1/fetch` |
| `emem_backfill` | L0 | Materialize history in a window | POST `/v1/backfill` |
| `emem_heat_solve` | L0 | 2-D heat-equation forecast | POST `/v1/heat_solve` |
| `emem_wave_solve` | L0 | 1-D shallow-water swell propagation | POST `/v1/wave_solve` |
| `emem_jepa_predict` | L0 | NDVI scalar AR(2) forecast | POST `/v1/jepa_predict` |
| `emem_jepa_predict_v2` | L0 | 128-D embedding forecast (untrained baseline) | POST `/v1/jepa_predict_v2` |
| `emem_verify` | L1 | Verify a structured claim | POST `/v1/verify` |
| `emem_intent` | L0 | Intent → Plan → executed result | POST `/v1/intent` |
| `emem_bands` | L0 | Band ontology | GET `/v1/bands` |
| `emem_functions` | L0 | Derivation function registry | GET `/v1/functions` |
| `emem_sources` | L0 | Upstream connector registry | GET `/v1/sources` |
| `emem_schema` | L0 | CDDL/JSON schema bundle | GET `/v1/schema` |
| `emem_errors` | L0 | Error code catalog | GET `/v1/errors` |
| `emem_manifests` | L0 | All manifest CIDs | GET `/v1/manifests` |
| `emem_grid_info` | L0 | cell64 grid encoding | GET `/v1/grid_info` |
| `emem_coverage_matrix` | L0 | Per-band live status | GET `/v1/coverage_matrix` |
| `emem_materializers` | L0 | Auto-fetch registry | GET `/v1/materializers` |
| `emem_data_availability` | L0 | Per-band history bounds | GET `/v1/data_availability` |
| `emem_algorithms` | L0 | Composition recipe catalog | GET `/v1/algorithms` |
| `emem_explain_algorithm` | L0 | One algorithm's full body | GET `/v1/algorithms/:key` |
| `emem_topics` | L0 | Topic-grouped band + algorithm registry | GET `/v1/topics` |
| `emem_coverage_map` | L0 | SVG corpus density (EmbeddedResource) | GET `/v1/coverage_map.svg` |
| `emem_cell_scene_rgb` | L0 | Sentinel-2 RGB thumbnail (ImageContent PNG) | GET `/v1/cells/:cell64/scene.png` |
| `emem_cell_geojson` | L0 | Cell polygon (EmbeddedResource GeoJSON) | GET `/v1/cells/:cell64/geojson` |

L0 is the conformance baseline (read + introspect + plan). L1 adds
verification. L2 (writes) is REST-only.

---

## Recipes

Eight worked examples. Every curl runs today against
`https://emem.dev` (or `http://127.0.0.1:5051` for self-host).

### 1. What's at this place?

Goal: fetch elevation at a free-text place name.

```bash
CELL=$(curl -s -X POST https://emem.dev/v1/locate \
  -H 'content-type: application/json' \
  -d '{"q":"Bengaluru"}' | jq -r .cell64)

curl -s -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d "{\"cell\":\"$CELL\",\"bands\":[\"copdem30m.elevation_mean\"]}"
```

```json
{
  "facts": [{"value": 910.0, "unit": "m", "tslot": 0, "confidence": 0.95,
             "sources": [{"scheme": "open_meteo", ...}], ...}],
  "receipt": {... fact_cids: ["cxjiu7l54ujzrpnekp24n4534yojpue4mprddbvevnqtti3lh5bq"]}
}
```

What to notice:

- `confidence` is upstream-declared (0.95 for Cop-DEM 90 m).
- `derivation.fn_key` says exactly which function produced this fact.
- The first call to a never-seeded cell + band can take ~180 ms (cold
  upstream fetch); subsequent calls are ~10 ms (cached + signed).

### 2. Find places like this one

Goal: top-K cells whose foundation embedding is nearest a target.

```bash
curl -s -X POST https://emem.dev/v1/find_similar \
  -H 'content-type: application/json' \
  -d '{"key":"defi.zb493.xoso.zcb6a","k":10,"mode":"cosine"}'
```

```json
{
  "neighbors": [
    {"cell": "defi.zb5cf.nura.zd83c", "score": 0.6537,
     "lat": 40.7229, "lng": -73.9987, "place_label_cached": "New York City, USA"},
    {"cell": "defi.zb563.noxo.xAvu", "score": 0.6426,
     "lat": 31.2304, "lng": 121.4737, "place_label_cached": "Shanghai, China"},
    ...
  ],
  "returned_k": 10
}
```

What to notice:

- `score` is cosine in [-1, 1]; 1.0 = same vector.
- `place_label_cached` is a convenience label cached on attestation; it
  may be absent for fresh cells.
- For the binary 16-byte sibling embedding (`mode: "hamming"`) you must
  first materialize `geotessera.bin128` — the responder will tell you
  with a `cid_not_found` error and the exact recall command to fix it.

### 3. Has it changed?

Goal: signed delta between two tslots at one (cell, band).

```bash
curl -s -X POST https://emem.dev/v1/diff \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb493.xoso.zcb6a","band":"indices.ndvi","tslot_a":0,"tslot_b":1}'
```

If only one tslot is attested, the response is honest:

```json
{"code":"cid_not_found","message":"CidNotFound: no fact at tslot_b=1 for (defi.zb493.xoso.zcb6a,indices.ndvi)"}
```

To seed history first, call `/v1/backfill` over the window. When both
tslots exist, `/v1/diff` returns a `DerivativeFact` whose CID is itself
citable.

What to notice:

- Diff rejects `tslot_a == tslot_b` (tautology guard).
- The output is a signed `DerivativeFact{op:"delta", parents:[a_cid, b_cid]}`
  — another agent can re-resolve the parents independently.

### 4. Confirm a claim

Goal: yes/no over a structured predicate, with citable evidence.

```bash
curl -s -X POST https://emem.dev/v1/verify \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb493.xoso.zcb6a","claim":{"band":"copdem30m.elevation_mean","op":"gt","value":500}}'
```

```json
{
  "verdict": true,
  "evidence": ["cxjiu7l54ujzrpnekp24n4534yojpue4mprddbvevnqtti3lh5bq"],
  "receipt": {"primitive": "emem.verify", ...}
}
```

What to notice:

- `op` is one of `eq | ne | lt | le | gt | ge | in | ni | exists | absent`.
- `evidence` lists the fact CIDs the verdict reads from. Quote them in
  your reply — that's what makes the answer cite-able.
- Optional `claim.window: [a, b]` plus `claim.agg: any|all|mean|min|max`
  evaluates over a tslot window.
- `mode: "zk"` returns an explicit error (reserved). `mode: "resolve"`
  currently degrades to fast.

### 5. What does this region average?

Goal: aggregate one band across a list of cells.

```bash
curl -s -X POST https://emem.dev/v1/query_region \
  -H 'content-type: application/json' \
  -d '{"geometry":"cells:defi.zb493.xoso.zcb6a,defi.zb493.xozI.zcb6a","bands":["copdem30m.elevation_mean"],"agg":"mean"}'
```

```json
{
  "aggregates": {"copdem30m.elevation_mean": 910.0},
  "facts": [...],
  "receipt": {...}
}
```

What to notice:

- `geometry` accepts `cell64` (one cell) or `cells:c1,c2,c3,...` (list).
  Bbox / GeoJSON returns an explicit error rather than silently
  pretending — there's no internal H3 indexer yet.
- `agg` is `mean | median | p90 | vector_centroid`. Vector centroid
  returns the dimension-wise mean of vector bands.

### 6. Materialize history

Goal: backfill a (cell, band) over a time window.

```bash
curl -s -X POST https://emem.dev/v1/backfill \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb493.xoso.zcb6a","band":"weather.temperature_2m","max_facts":2}'
```

```json
{
  "band": "weather.temperature_2m",
  "tempo": "ultrafast",
  "slot_seconds": 3600,
  "steps": [
    {"tslot": 0, "target_unix": 0, "fact_cid": "qi3jo4...", "status": "cached"},
    {"tslot": 1, "target_unix": 3600,
     "status": "present_only",
     "reason": "present_only: 'weather.temperature_2m' is a met.no nowcast — backfill only meaningful for the current tslot"}
  ],
  "materialized_count": 0,
  "skipped_count": 1,
  "notes": ["band 'weather.temperature_2m' is now-only at this responder"]
}
```

What to notice:

- Each step carries `status` (`cached | materialized | present_only |
  error`). `present_only` is the no-silent-fallbacks discipline: the
  band exists and answers, but the upstream is now-only and there's no
  point hammering history.
- For bands with real history (`surface_water.recurrence`,
  `modis.ndvi_mean`), check `/v1/data_availability` first to see the
  upstream's `history_available_from_unix` / `history_available_to_unix`
  before picking a window.

### 7. Get satellite imagery for a cell

Goal: Sentinel-2 L2A true-colour thumbnail for a cell.

```bash
curl -s "https://emem.dev/v1/cells/defi.zb493.xoso.zcb6a/scene.png?max_cloud=80" \
  --output bengaluru.png

curl -sI "https://emem.dev/v1/cells/defi.zb493.xoso.zcb6a/scene.png?max_cloud=80"
```

```
HTTP/1.1 200 OK
content-type: image/png
x-emem-scene-item-id: S2B_43PGQ_20260502_0_L2A
x-emem-scene-datetime: 2026-05-02T05:25:16.655000Z
x-emem-scene-cloud-cover: 27.23
x-emem-scene-epsg: 32643
```

What to notice:

- The pipeline is pure-Rust: STAC search → HTTP-Range COG read →
  2-98 percentile stretch → PNG encode. No GDAL.
- 256×256 px ≈ 2.56 km × 2.56 km at S2's 10 m native pitch.
- Headers expose the STAC item id, capture time, cloud cover, and EPSG
  so you can quote them alongside the image.
- Tropical / cloudy regions: raise `max_cloud` to 60-80 if you keep
  getting "no scene" errors.

### 8. Verify a receipt offline

Goal: confirm an Ed25519 signature without trusting the responder.

```bash
curl -s -X POST https://emem.dev/v1/verify_receipt \
  -H 'content-type: application/json' \
  -d '{"receipt": {paste-the-receipt-block-from-any-recall-response}}'
```

```json
{
  "valid": true,
  "signer_pubkey_b32": "777er3yihgifqmv5hmc2wwmyszgddzderzhsx6rex4yoakwomvka",
  "preimage_blake3_hex": "30f638020a2e00c0846359651d2c446e3d980dbfc8a8bcfaf8c9892a22e5a37b",
  "primitive": "emem.recall", "served_at": "2026-05-08T07:59:08Z",
  "fact_cids_count": 1
}
```

`preimage_blake3_hex` is what the responder hashed and signed; you can
reconstruct it offline (see next section) and run
`ed25519_dalek::verify_strict` locally. `signer_pubkey_b32` should match
the responder pubkey at `/.well-known/emem.json` — if it doesn't, the
receipt isn't from the responder you think it is.

---

## What MCP intentionally does NOT expose

The catalog at `crates/emem-mcp/src/lib.rs` ships 34 read tools and
deliberately omits two L2 write surfaces — `emem_attest` (submit a
signed Attestation) and `emem_challenge` (challenge an existing one).
Both need an Ed25519 secret key the attester controls. Advertising them
as MCP tools caused every Claude.ai connector-onboarding tile to error
with "unknown tool" because no JSON-only dispatch arm could accept the
canonical CBOR Attestation envelope. LLM hosts cannot manage keys
safely, so writes go through plain REST from a process you control:
`POST /v1/attest` (JSON Attestation) or `POST /v1/attest_cbor`
(canonical CBOR). Key generation, signing, and rotation procedures are
in `docs/ATTESTING.md`; the schema is in `/openapi.json`.

---

## Grids, time, and gotchas

### cell64 grid

Active codec is `cell64-geo-21x22`: 21 lat bits, 22 lng bits, packed to
a u64 then base-1024 bigram-encoded as four `.`-joined groups (e.g.
`defi.zb493.xoso.zcb6a`). `GET /v1/grid_info` reports actual ground
resolution: `lat_axis_metres_at_equator: 9.54`,
`lng_axis_metres_at_equator: 9.55`, `lng_axis_metres_at_lat_60: 4.77`.
Latitude pitch is uniform; longitude pitch narrows with cos(lat) so
cells become taller than wide above ~lat 30°.

The spec target `aperture-7 hex DGGS` (~3.4 m edge) is reserved in the
manifest but **not active in this build** — today's cells are square at
the equator. Receipts pin the active manifest CID, so historical answers
do not drift when the migration lands.

### tslot

`tslot` is a `u64` slot offset, anchored at Unix epoch since release
0.0.3 (was an internal `emem-2026` anchor in 0.0.2 — facts attested
under the old anchor fail with `NotGeoCell` rather than silently
misplace). Tempo classes:

- `static` — one fact answers forever (`copdem30m.elevation_mean`,
  `surface_water.recurrence`, `koppen.major_class`)
- `annual` — `geotessera.YYYY`, `hansen.loss_year`, `esa_worldcover.lc_2021`
- `monthly` / `8day` — `modis.ndvi_mean`, `modis.lst_day_8day`
- `daily` — `era5.precip`, `era5.t2m`
- `now_only` / `ultrafast` — `weather.*` (met.no nowcast)

Check `/v1/data_availability` before picking a tslot or a backfill window.

### Materialization on miss

Recall against a wired band auto-fetches upstream, signs the result
under the responder's identity, persists, and returns it in the same
response (~180 ms cold, ~10 ms cached). When the band key isn't even
registered, the responder says so explicitly — no silent fallback:

```json
{
  "facts": [],
  "materialize_notes": [{
    "band": "foo.bar", "status": "skipped",
    "reason": "no_auto_materializer_registered: no upstream connector wired for band=foo.bar; submit a signed Attestation via /v1/attest_cbor to seed it. Call GET /v1/bands to see all known band keys."
  }],
  "bands_already_attested_at_cell": [/* 43 actual keys at this cell */]
}
```

That's the no-silent-fallbacks contract: empty result must distinguish
"wrong query" from "place is empty."

### Timeouts, geocoder, find_similar caveats

- Materializer per-upstream timeout: 30 s (`EMEM_MATERIALIZER_TIMEOUT_SECS`).
  Gateway timeout: 180 s (`EMEM_TIMEOUT_SECS`). Slow paths (Cop-DEM, S2
  STAC + COG range reads for an uncached scene) sometimes need the full
  window. Treat 5xx as transient and retry once; treat 4xx as permanent.
- `/v1/locate` walks four layers in order: embedded gazetteer → cache →
  Photon (primary live) → Nominatim (secondary). `via` reports which
  layer answered. Overpass was removed in 0.0.3.
- `find_similar`: self-cell is filtered, duplicate cells across tslots
  are deduped (max-score wins), NaN scores stripped. Type mismatch
  (asking with a non-vector band) returns Protocol error with a "use
  /v1/recall to inspect" hint, not an empty list. The `filter` field
  exists in the schema but is not yet wired.

### Receipts are immutable

`fact_cid = base32_nopad(blake3(canonical_cbor(fact))[..16])`. Same fact
at two responders → byte-identical CBOR → same CID. Re-encoding a fact
body changes the CID and the original signature stops verifying — always
fetch by CID through `/v1/fetch` or `GET /v1/facts/:cid`.

---

## Verifying a receipt offline (with code)

The preimage the responder hashes and Ed25519-signs is constructed at
`crates/emem-api-rest/src/lib.rs:7260-7276`:

```
blake3(
    request_id || "|" || served_at  || "|" || primitive || "|" ||
    cell1 "," cell2 "," ... || "|" || cid1 "," cid2 "," ...
)
```

The trailing commas after the last cell and the last cid are intentional
(the signing loop appends `","` after every entry). The pubkey is loaded
from `responder_pubkey_b32` (base32-nopad) or the `responder` byte array
— both encode the same 32-byte Ed25519 verifying key.

Self-contained Python verification (no emem dependency, just `blake3`
and `cryptography`):

```python
"""Verify an emem receipt offline. Reproduces /v1/verify_receipt locally."""
import base64
import requests
import blake3
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
from cryptography.exceptions import InvalidSignature

EMEM = "https://emem.dev"

def b32_nopad_decode_lower(s: str) -> bytes:
    """RFC 4648 base32, no padding, lowercase. Pad back to a multiple of 8."""
    s = s.upper()
    pad = (-len(s)) % 8
    return base64.b32decode(s + "=" * pad)

# 1. Pull a fresh signed recall response.
resp = requests.post(f"{EMEM}/v1/recall",
                     json={"cell": "defi.zb493.xoso.zcb6a",
                           "bands": ["copdem30m.elevation_mean"]},
                     timeout=30).json()
r = resp["receipt"]

# 2. Reconstruct the preimage exactly as the responder did.
h = blake3.blake3()
h.update(r["request_id"].encode())
h.update(b"|")
h.update(r["served_at"].encode())
h.update(b"|")
h.update(r["primitive"].encode())
h.update(b"|")
for c in r["cells"]:
    h.update(c.encode())
    h.update(b",")
h.update(b"|")
for c in r["fact_cids"]:
    h.update(c.encode())
    h.update(b",")
preimage_digest = h.digest()  # 32 bytes — what was signed

# 3. Decode the responder's pubkey + the receipt signature.
pk_bytes = b32_nopad_decode_lower(r["responder_pubkey_b32"])
assert len(pk_bytes) == 32
sig_bytes = bytes(r["signature"])
assert len(sig_bytes) == 64

# 4. Verify Ed25519 over the BLAKE3 digest.
pk = Ed25519PublicKey.from_public_bytes(pk_bytes)
try:
    pk.verify(sig_bytes, preimage_digest)
    print(f"VALID — signed by {r['responder_pubkey_b32']}")
    print(f"preimage_blake3 = {preimage_digest.hex()}")
except InvalidSignature:
    print("INVALID")

# 5. Sanity-check against the responder's verify endpoint.
echo = requests.post(f"{EMEM}/v1/verify_receipt",
                     json={"receipt": r}, timeout=30).json()
assert echo["valid"] is True
assert echo["preimage_blake3_hex"] == preimage_digest.hex()
print("Local digest matches /v1/verify_receipt ✓")
```

A real receipt (captured 2026-05-08T08:02:48Z from the running
responder):

```json
{
  "request_id": "01KR39RM2EXBDTGGDF13Z16PH4",
  "served_at": "2026-05-08T08:02:48Z",
  "primitive": "emem.recall",
  "cells": ["defi.zb493.xoso.zcb6a"],
  "fact_cids": ["cxjiu7l54ujzrpnekp24n4534yojpue4mprddbvevnqtti3lh5bq"],
  "responder_pubkey_b32": "777er3yihgifqmv5hmc2wwmyszgddzderzhsx6rex4yoakwomvka",
  "signature": [
    79, 120,  96, 128, 126,  55, 232,  60,   7,  72,  85,  57,  10, 173,  49, 128,
   139, 222, 158,   5,  68,  61, 121,  80, 209, 111,  90,  41, 200, 142,  99, 154,
    31, 203,  11, 197, 251,   8,  63, 229,   0,  64,  70,  79, 138, 193, 193,  47,
    27, 253, 191, 165, 244,  46, 181, 168, 251,  11,  42,  79,  29,  46,  64,   6
  ]
}
```

Run the script with this `r` value: it prints `VALID` and matches the
remote `preimage_blake3_hex`. The same procedure works against any
emem responder — the responder pubkey is the only trust anchor, and
that pubkey is published at `/.well-known/emem.json`.

---

## Watching humans use the API: `/humans`

`https://emem.dev/humans` is an interactive surface designed primarily
for agents that want to learn the protocol by observation, with humans
as a secondary audience. The page is its own API console.

### What's on the page

- **Constellation field** — every attested cell in `/v1/coverage` is
  rendered as a star. Brightness is `log10(1 + facts_count)`, hue is
  the cell's dominant band family (derived from `/v1/coverage_matrix`,
  refined per-cell after a `/v1/recall`). Drag-pan, wheel-zoom, hover
  for a tooltip with `cell64` + lat/lng + family.
- **Embedding projection** — once `/v1/recall_many` returns the 128-D
  Tessera vectors for the densest 80 cells, a 2-D power-iteration PCA
  is computed in JS and the constellation reprojects (eased animation,
  ~700 ms) from sinusoidal lat/lng to embedding-space coordinates.
  Cells without a `geotessera` attestation stay at their geographic
  position so the picture is never partial. Toggle: `p` key or the
  chip in the dock. Banner is honest about the count
  ("N of M cells carry a 128-D Tessera vector").
- **Lasso → `/v1/recall_polygon`** — drag (or hold shift, or press
  `L`, or tap the dock chip) to draw a polygon. The page picks the
  band with the highest `facts_count` from the matrix, posts the
  polygon, and renders the results inline. Touch path mirrors the
  mouse path on mobile.
- **Force-graph view** — Verlet force layout in canvas2D (~150 LoC,
  no library), top 200 cells, edges by lat/lng proximity. Decays at
  equilibrium; persists across mode switches.
- **Registry view** — pure-SVG Poincaré-disk over the 8 manifest
  registries (`bands`, `algorithms`, `sources`, `schema`, `topics`,
  plus `lcv`, `alphabet`, `registry` placeholders). Top-N children
  hang off each registry by `facts_count`.
- **Log view** — Sigstore-Rekor-style scroll of
  `/v1/coverage_matrix` rows sorted by `last_attested_unix_s`. Each
  row has an inline `verify` button.

### How agents read it

Every visible cell, fact, manifest CID, pubkey, and interactive control
carries `data-emem-*` attributes. A scraping LLM extracts everything
from the rendered DOM:

```html
<div class=fact data-emem-cell="defi.zb493.xoso.zcb6a"
                data-emem-band="weather.temperature_2m"
                data-emem-fact-cid="qi3jo4sqcg…l2hgjtwm"
                data-emem-tslot="1778237046"
                data-emem-verified="true">…</div>

<button class=chip data-emem-action=open-cmdk>search<kbd>Ctrl-K</kbd></button>
<button id=chipLasso data-emem-action=lasso-toggle>lasso<kbd>L</kbd></button>
<button id=focusBtn  data-emem-action=toggle-focus>⤢ focus<kbd>F</kbd></button>
```

The full `data-emem-action` map is on every interactive element in the
header chips, the bottom dock (modes / projection / zoom / focus), the
collapse handles (`collapse-left/right/console`), and the legend toggle.
A scraper that walks `[data-emem-action]` learns the affordance map a
human sees with no extra instrumentation.

### Console pane — the page is the API console

Every `/v1/*` call the page makes prints below as a structured log row:

```
12:34:56  POST  /v1/recall_many  {"cells":[…80…],"bands":["geotessera"]}  → 200  · 51 ms
12:34:56  GET   /v1/cells/defi.zb493.xoso.zcb6a/info                       → 200  · 14 ms
12:35:02  UI    mode                                                       → registry
12:35:08  UI    lasso                                                      → on
12:35:09  POST  /v1/recall_polygon  {"polygon":[…],"bands":["weather.temperature_2m"]}  → 200  · 230 ms
```

Hover any row and four pivots appear: **curl**, **py**, **mcp**, **↻**.
The MCP pivot emits a JSON-RPC 2.0 `tools/call` payload with the right
tool name from the `emem-mcp` registry; an agent watching the page
learns both the REST and MCP surfaces from the same trail. UI state
toggles (mode, lasso, projection, focus, rail collapse) echo to the
same log so the trail is complete.

### Sibling artifacts

- `/humans.json` — JSON twin (`schema=emem.humans.v1`): manifest CIDs,
  responder pubkey, top-10 bands by `facts_count`, dense-20 cells with
  lat/lng, totals. Agents that prefer JSON over scraping DOM read this
  directly. Baked at release; the page's live runtime fetches the same
  shape from `/v1/coverage_matrix` and `/v1/coverage`.
- `/humans/llms.txt` — page-scoped llms.txt convention. Lists the
  endpoints the page invokes, the data attributes, the trust model,
  and the console pane behaviour.
- `/humans-og.svg` — 1200×630 OpenGraph card so a pasted link previews
  the constellation, not the generic landing image.

### Offline verification, in the browser

The page imports `@noble/curves@1.6.0/ed25519` and
`@noble/hashes@1.5.0/blake3` from a pinned `esm.sh` URL (CSP allows the
host in `script-src` and `connect-src`). Receipts verify locally:

1. Build the canonical preimage —
   `request_id | served_at | primitive | cell0,cell1,…,cellN, | cid0,cid1,…,cidN,`
   — byte-for-byte matching `crates/emem-storage/src/server.rs:132-148`.
2. `digest = blake3(preimage)`.
3. `ed25519.verify(receipt.signature, digest, responder_pubkey)`.

Pubkey decodes from `responder_pubkey_b32` via in-page base32-nopad. On
success, the row's pill flips to `verified offline` and the parent
`<div class=fact>` gets `data-emem-verified="true"`. If the noble libs
fail to load (CSP block, CDN slowdown), the page races a 2.5 s timeout
and silently routes to `POST /v1/verify_receipt` instead — the result
is still trustworthy, but the page labels itself accordingly so the
trust mode is never silently downgraded.

### URL state encoding

Every interesting state lives in the URL via `replaceState`:

```
https://emem.dev/humans?cell=defi.zb493.xoso.zcb6a&proj=embed&mode=log&layout=noLeft,focus
```

A pasted link opens with the cell selected, the projection toggled,
the log view active, and the left rail collapsed in focus mode. This
is the same convention Honeycomb's BubbleUp and Sigstore's Rekor UI
use — the URL is a saved query.

---

## Where to read more

- `https://emem.dev/openapi.json` — every endpoint, every schema
- `https://emem.dev/.well-known/emem.json` — manifest CIDs + responder pubkey
- `https://emem.dev/v1/agent_card`, `/v1/quickstart` — discover-first card + onboarding flow
- `https://emem.dev/spec.md`, `/whitepaper.md`, `/attesting.md` — protocol, math, write path
- `https://emem.dev/humans`, `/humans.json`, `/humans/llms.txt` — interactive console + JSON twin
- `crates/emem-mcp/src/lib.rs` — canonical MCP tool descriptors
- `crates/emem-primitives/src/*` — the 11 primitive shapes
- `examples/agent-walkthroughs.md`, `examples/langchain.py`, `examples/llamaindex.py`

License: Apache-2.0. Contact: avijeet@vortx.ai.
