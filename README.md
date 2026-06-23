<div align="center">
  <img src="[https://vortx.ai/assets/vortx-logo-200.gif](https://geo.qa/vortxgola.gif)" width="128" alt="emem" />

  <h1>emem</h1>

  <p>Earth as memory, for real-world agents.</p>

  <p>
    <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue.svg" alt="License"></a>
    <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/rust-1.91-orange.svg" alt="Rust"></a>
    <a href="https://modelcontextprotocol.io/"><img src="https://img.shields.io/badge/MCP-Streamable%20HTTP-7af.svg" alt="MCP"></a>
    <a href="https://www.openapis.org/"><img src="https://img.shields.io/badge/OpenAPI-3.1-green.svg" alt="OpenAPI"></a>
    <a href="https://github.com/Vortx-AI/emem/pkgs/container/emem"><img src="https://img.shields.io/badge/ghcr.io-vortx--ai%2Femem-181717?logo=github" alt="Container"></a>
  </p>

  <p>
    <a href="https://eudr.dev"><img src="https://img.shields.io/badge/built%20with%20emem-eudr.dev%20%C2%B7%20EUDR%20compliance%20agent-2ea043.svg" alt="eudr.dev: EU Deforestation Regulation compliance agent built on emem"></a>
  </p>

  <p>
    <a href="https://emem.dev"><strong>Hosted</strong></a> ·
    <a href="https://emem.dev/agents.md">Docs</a> ·
    <a href="https://emem.dev/spec.md">Spec</a> ·
    <a href="https://emem.dev/openapi.json">OpenAPI</a> ·
    <a href="https://emem.dev/humans">Try it</a> ·
    <a href="https://emem.dev/verify">/verify</a> ·
    <a href="https://emem.dev/docs/gallery">Gallery</a> ·
    <a href="https://huggingface.co/spaces/vortx-ai/emem">HF Space</a>
  </p>
</div>

---

**Ask an AI agent what is on the ground at 19.07° N, 72.87° E and it will guess.** It has no fixed handle for that patch of Earth, and no way to prove whatever number it returns. emem is the handle. It is a shared memory of the planet that an agent can read, write, and cite, where every answer is signed so anyone can check it later without trusting the server that produced it.

The planet is cut into fixed cells about 9.55 m across, the way a page is cut into words. One measurement at one cell is a **fact**: an elevation, a rainfall total, this year's forest loss, a satellite embedding. Every fact is signed. When an agent asks about a place nobody has measured yet, the responder pulls the value from a real satellite source, signs it, and hands it back in the same response. Nothing is pre-seeded. Every cell on Earth answers from the first request.

emem is a protocol. A fact is named by the blake3 hash of its own bytes, so the name carries the data's fingerprint and means the same thing on every machine. The responder signs that name with an ed25519 key, the kind that secures SSH and HTTPS. Any responder can serve a fact and any client can verify it offline, with no account and no key to manage. Paste a fact id into a chat and a colleague pulls the same bytes from any node and checks the signature in their own browser at [/verify](https://emem.dev/verify). The hosted node is `https://emem.dev`. The same binary self-hosts with one `docker run`, and the same handlers answer both MCP and plain REST. Run enough nodes and you get a federation: independent responders that resolve the same ids byte for byte and write down where they disagree. The memory gets more trustworthy as more agents use it.

### How a fact gets made and proven

A **cell64** addresses a place the way a token addresses text in an LLM. Every patch of ground about 9.55 m wide gets a 64-bit id, and ids that look alike sit physically near each other. A **fact** is one measurement at that cell, keyed by `(cell, band, time)` and packed in a fixed byte order (canonical CBOR) so the same reading hashes the same way on every machine. That blake3 hash is the fact's **content id**. Change one byte and the id changes, so the id proves the bytes. The responder signs it. The signed envelope it returns is the **receipt**, and the receipt checks out offline against the responder's public key without any trust in the server.

When an agent asks for a band at a cell that has no signed fact yet, the responder fetches the underlying tile through one of its 46 upstream sources, signs the result under its own key, persists it, and returns it in the same response. A cold read takes about 180 ms. A warm read is under ten. Five of the 46 schemes are declared but not yet wired (`openet.30m.daily`, `dynamic_world.v1`, `tropomi.s5p.ch4`, `tropomi.s5p.no2`, `viirs.dnb.monthly`); they answer with a typed Absence. When a band genuinely has no value at a cell, because the place is outside coverage or the upstream is unreachable, the answer is still a signed absence with a reason you can read. An empty answer is a citable receipt. The catalog never promises more than it can sign.

## Try it (no install, no key)

```bash
# Geocode a place to a cell64.
curl -s -X POST https://emem.dev/v1/locate \
  -H 'content-type: application/json' \
  -d '{"q":"Bengaluru"}' | jq .cell64
# "defi.zb493.xuqA.zcb5f"   # (geocoder result, may drift)

# Recall a band at that cell (auto-fetched if cold).
curl -s -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb493.xuqA.zcb5f","bands":["weather.temperature_2m"]}' \
  | jq '.facts[0]'

# Ask a free-text question; the foundation-embedding fan-out fires
# automatically on "find places like" / "what changed" intents.
curl -s -X POST https://emem.dev/v1/ask \
  -H 'content-type: application/json' \
  -d '{"q":"find places like Yellowstone","place":"Yellowstone National Park"}' \
  | jq '.answer'

# Hunter mode: discover event hotspots over a named region. The same
# classifier reads "find <event> in <region>" from /v1/ask and routes
# here; structured callers can hit /v1/hunt directly.
curl -s -X POST https://emem.dev/v1/hunt \
  -H 'content-type: application/json' \
  -d '{"event":"algal_bloom","region":"Lake Erie"}' \
  | jq '.hotspots[0]'
```

The receipt's `fact_cid` is a durable handle. Re-fetching it from any responder, in any year, returns the same bytes.

## Verify an answer (four curls)

The pitch lives or dies on this flow. Every recall response carries a receipt with `fact_cids[]`, a `merkle_proof`, and an Ed25519 `signature` over a domain-separated, length-prefixed preimage: `blake3("emem.preimage.v1" ‖ "receipt" ‖ tagged(request_id, served_at, [scope], [as_of], [edges], [manifest], primitive, cells[], fact_cids[]))`. Tagging every field and prefixing its length means no two distinct responses can ever share signed bytes; the receipt's `preimage_version` selects the rule, and pre-v1 receipts still verify under the original one. The signer's public key is stable; the receipt verifies offline against any copy of the responder pubkey. The merkle tree uses RFC 6962 leaf/node domain separation and rejects duplicate leaves.

Here is a real one. Ask `https://emem.dev` for the elevation under Denver and it returns the city's nickname as a signed number, mile-high at 1609 m, which anyone can re-check without trusting the server:

```jsonc
// POST /v1/recall {"cell":"defi.zb5c4.guxe.nuxe","bands":["copdem30m.elevation_mean"]}
{
  "facts": [{ "cell": "defi.zb5c4.guxe.nuxe", "band": "copdem30m.elevation_mean",
              "value": 1609.0, "unit": "m", "source": "copernicus.dem.glo30" }],
  "receipt": {
    "primitive": "emem.recall",
    "fact_cids": ["72wdchiyurfrjxz7zat6kor7gjnvsn564fbrzjkmlhagoy4rrh4a"],
    "responder_pubkey_b32": "777er3yihgifqmv5hmc2wwmy…",
    "preimage_version": 1,
    "signature": "…ed25519 over the canonical preimage…"
  }
}
```

Paste that `fact_cid` into [/verify](https://emem.dev/verify) and the page re-derives the hash and checks the signature in your browser. The four curls below do the same from a shell:

```bash
# 1. Resolve a place to a cell64.
CELL=$(curl -s -X POST https://emem.dev/v1/locate \
  -H 'content-type: application/json' \
  -d '{"q":"Golden Gate Park, San Francisco"}' | jq -r .cell64)

# 2. Recall a band and capture the receipt envelope.
curl -s -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d "{\"cell\":\"$CELL\",\"bands\":[\"indices.ndvi\"]}" > /tmp/recall.json

jq '.receipt | {primitive, served_at, responder_pubkey_b32, fact_cids, merkle_proof: .merkle_proof.root}' \
  /tmp/recall.json

# 3. Ask the responder to verify its own signature (server-side check).
jq '{receipt: .receipt}' /tmp/recall.json > /tmp/receipt.json
curl -s -X POST https://emem.dev/v1/verify_receipt \
  -H 'content-type: application/json' --data @/tmp/receipt.json
# {"valid":true,"preimage_blake3_hex":"…","fact_cids_count":1,"signer_pubkey_b32":"…",…}

# 4. Reproduce: pull the same fact_cid from any responder, on any day.
# The cell, band, tslot, and derivation.fn_key are content-addressed, so
# the bytes you receive will hash to the same fact_cid.
jq '.facts[0].derivation' /tmp/recall.json
```

For a browser-only verify, open [`/verify/<fact_cid>`](https://emem.dev/verify); the page does the same Ed25519 check in WebCrypto + `@noble/ed25519` so you never have to trust the responder you got the receipt from. A guided walk lives at [`/demos/signed-answer`](https://emem.dev/demos/signed-answer).

## Architecture

<p align="center">
  <img src="docs/diagrams/architecture.png" alt="emem architecture: one binary at the centre, clients reaching it over MCP and REST through the same handlers, primitives ringing the core, 46 upstream sources feeding in from the left, the GPU sidecar from the right, every write dropping into an append-only signed log below, and four content-addressed manifests pinning what produced each answer" width="900"/>
  <br/>
  <em>One binary. The same handlers answer MCP and plain REST, reads need no auth, and every write lands in an append-only signed log. Four content-addressed manifests (<code>bands_cid</code>, <code>algorithms_cid</code>, <code>sources_cid</code>, <code>schema_cid</code>) pin exactly what produced each answer. The full deployment suite lives at <a href="https://emem.dev/docs/diagrams">/docs/diagrams</a>.</em>
</p>

## The memory layer

A cache hands back a tile. A memory remembers what it saw, links it to what it saw before, and says so when two sources disagree. emem gives an agent that second thing on top of the fact store, and the agent owns it.

<p align="center">
  <img src="docs/diagrams/memory-engram.png" alt="An engram of Earth, drawn as a neural-memory diagram in ink-on-paper Mithila style: each cell is a node, edges are synapses that relate, supersede, or disagree, and recall pathways converge on a central consolidation lotus" width="900"/>
  <br/>
  <em>An agent's memory of Earth, drawn as an engram. Each cell is a node and each edge a synapse that relates, supersedes, or disagrees. Recall draws signed facts inward to the lotus where the shared memory consolidates, and every node carries its own signature.</em>
</p>

Writes land in `/memories/` as content-addressed, signed files. `memory_create` makes one, `memory_str_replace` and `memory_insert` edit it, and `memory_search` runs a BGE-768 embedding query over the contents through a LanceDB IVF_PQ index, so an agent finds a note it wrote last week by meaning instead of by filename. Each file carries a `kind` from the CoALA taxonomy: `episodic` for what happened, `semantic` for what holds true, `procedural` for how to do a thing, `resource` for a pointer out. A write under `/memories/by_attester/<pubkey>/` is capability-bound, so a path owned by one key turns away every other signer. The signature that proves a Sentinel-2 reading is the same signature that proves the agent's own notes are untouched.

The memory connects facts and notices when they fight. `memory_bundle` folds N facts into one signed envelope, `memb:<bundle_cid>`, that resolves to identical bytes on any peer, so an agent hands over a single citation for a whole finding instead of a list of loose ids. `memory_contradictions` walks the cases where two attesters signed different values at the same `(cell, band, tslot)` and scores the gap by band kind: normalised spread for a scalar, mean cosine for a vector, mode-share for a category. A second node that read the same Sentinel scene on a cloudier day leaves a trace the agent can weigh instead of a silent overwrite.

Every read takes a bi-temporal bound. `as_of_tslot` asks what the world looked like at a past moment. `as_of_signed_at` asks what the system knew at a past moment. Set both and both hold. The receipt records the bound, so an auditor in 2027 takes a 2026 receipt to any peer and replays the exact same query.

### Streams

The memory is live, and you can watch it. `GET /v1/stream` is a Server-Sent Events heartbeat. Every few seconds the responder signs a snapshot of corpus state and pushes it, so a dashboard or an agent follows the shared memory growing without polling for it. The tick is signed like everything else, captured here straight off `https://emem.dev`:

```
event: state
data: {
  "type": "corpus.state",
  "served_at": "2026-06-12T16:17:30Z",
  "corpus": { "distinct_cells": 8147, "distinct_bands": 75, "facts_scanned": 32768 },
  "responder": { "pubkey_b32": "777er3yihgifqmv5hmc2wwmy…", "key_epoch": 0 },
  "signature": {
    "alg": "ed25519",
    "preimage": "emem.stream.tick|v0.1.0|epoch0|2026-06-12T16:17:30Z|registry:3pbqnyni…|cells:8147",
    "signature_b32": "xk2hiluwmfywwnfj…"
  }
}
```

`GET /v1/memory/sse?path_prefix=&kind=&attester=` is the narrow stream. It pushes one event the moment a memory write commits, filtered server-side, so a compliance subscriber sees a write to a watched path the instant its sled commit lands rather than on the next poll.

## Connect your AI assistant

The MCP endpoint is `https://emem.dev/mcp`. Drop a config snippet into your client.

| Client                | Config                                                              |
|-----------------------|---------------------------------------------------------------------|
| Claude Desktop        | [examples/claude-desktop.json](examples/claude-desktop.json)        |
| Claude Code           | [examples/claude-code.mcp.json](examples/claude-code.mcp.json)      |
| Cursor                | [examples/cursor.mcp.json](examples/cursor.mcp.json)                |
| Cline (VS Code)       | [examples/cline.mcp.json](examples/cline.mcp.json)                  |
| Gemini CLI            | `gemini extensions install https://emem.dev/gemini-extension.json`  |
| ChatGPT (Custom GPT)  | [examples/openai-gpt-action.json](examples/openai-gpt-action.json)  |
| LangChain (Python)    | [examples/langchain.py](examples/langchain.py)                      |
| LangChain MCP agent   | [examples/langchain/](examples/langchain/)                          |
| LlamaIndex (Python)   | [examples/llamaindex.py](examples/llamaindex.py)                    |
| LlamaIndex MCP agent  | [examples/llamaindex/](examples/llamaindex/)                        |
| Agno MCP agent        | [examples/agno/](examples/agno/)                                    |
| Pydantic AI MCP agent | [examples/pydantic-ai/](examples/pydantic-ai/)                      |
| AutoGen MCP agent     | [examples/autogen/](examples/autogen/)                              |
| CrewAI MCP agent      | [examples/crewai/](examples/crewai/)                                |
| Mastra MCP agent      | [examples/mastra/](examples/mastra/)                                |

Python (`ememdev`) and TypeScript (`@emem/client`) SDKs live under `sdks/` (PyPI / npm publication pending; install from the repo today).

## Primitives

81 MCP tools (10 core, 71 extended), 93 documented REST paths under `/v1/*`, surfaced through `/openapi.json`. Every tool carries a `when_to_use` string written for LLM tool-selection, and four MCP behavioural annotations (`readOnlyHint`, `destructiveHint`, `idempotentHint`, `openWorldHint`). A no-param `tools/list` returns all 81 tools (so every MCP client discovers the full surface); pass `{"tier":"core"}` for just the 10 essentials. Tools are callable via `tools/call` regardless of tier.

- **Locate:** name or lat/lng → `cell64`. Five-layer cascade: wide-bbox table → embedded gazetteer → GeoNames cities-5000 (68 581 places, in-process) → sled cache → Photon → Nominatim. Polygon geometry from Overture `divisions/division_area`. District-level queries reroute through Overture when Nominatim returns a POI courthouse.
- **Memory substrate** (state + tokens + bundles + memory files + search + contradictions + SSE): `POST /v1/state` returns a signed dense per-place embedding (`view=encoder` default 128-D, `view=cube` full 1792-D). `POST /v1/state_multi` fans across `geotessera` + `clay_v1` + `prithvi_eo2` + `galileo`. `POST /v1/state_diff` returns residual + L2 + cosine between two vintages. `POST /v1/memory_token` composes `memt:<cell64>:<fact_cid>`. `POST /v1/memory_bundle` composes a signed envelope `memb:<bundle_cid>` over N (cell, band, tslot) triples. Six MCP file-op verbs (`memory_view`, `memory_create`, `memory_str_replace`, `memory_insert`, `memory_delete`, `memory_rename`) conform to Anthropic's memory-tool spec; every write is ed25519-signed and content-addressed. Paths under `/memories/by_attester/<pubkey>/...` enforce capability binding (ed25519 signature over `blake3("emem.memory_write|" + verb + "|" + path + "|" + body_hash)`). Each file carries a `kind` from the CoALA taxonomy (`episodic` / `semantic` / `procedural` / `resource`). `POST /v1/memory/search` does BGE-768 semantic search over file contents via a LanceDB IVF_PQ partition. `POST /v1/memory_contradictions` walks a parallel multi-attester index and scores disagreement per band kind (scalar / vector / categorical). `GET /v1/memory/sse?path_prefix=&kind=&attester=` streams write events with server-side filter. Every read primitive accepts `as_of_tslot` + `as_of_signed_at` for bi-temporal queries (valid-time + transaction-time); the receipt carries an `as_of` block when set. See [`docs/memory.md`](docs/memory.md) for the full reference.
- **Recall / recall_many / recall_polygon:** 124 materializer-wired band names across 43 cube slots. Recall answers any wired band, auto-fetching on a cold miss and signing the result. Signed Absence on out-of-coverage.
- **Find similar:** k-NN over any vector band. Hamming fast path (sign-bit pop-count) auto-derives from the cosine band when the binary sibling is absent. Mode `hamming_then_rerank` triages with Hamming then re-orders by cosine; the over-sampling factor is EWMA-adaptive.
- **Compare / compare_bands / diff / trajectory:** pairwise and time-series.
- **Connect & evolve:** typed temporal edges (`emem_edges_recall` reads a fact's signed connections of type `disagrees_with`, `supersedes`, or `relates_to`, bounded by valid-time), multi-attester contradiction scoring (`memory_contradictions`, per band kind), and a deterministic refinement loop that re-derives a fact when a newer attestation or a `disagrees_with` edge lands. All three ship in 0.1.0.
- **Verify:** structured claim against attested facts; returns signed verdict + evidence CIDs.
- **Physics:** `/v1/heat_solve` (2-D explicit FTCS heat, MODIS LST stencil), `/v1/wave_solve` (1-D shallow-water along seaward bathymetry gradient), `/v1/jepa_predict` (closed-form NDVI AR(2) seasonal), `/v1/jepa_predict_v2` (Tessera embedding dynamics; short-circuits to last-vintage identity baseline while the trained head is pending, receipt carries `untrained_baseline`).
- **Ask:** free-text question with topic routing. The classifier covers three intent families: place-anchored topical questions (the topic router fan-out), foundation-embedding intents on `find places like` / `what changed` / `deforestation` / `anomaly` (cross-encoder consensus over Clay + Prithvi + Tessera), corpus-meta intents on `where do you have data` / `how fresh is your corpus` (redirect to coverage surfaces), and hunter-mode discovery on `find <event> in <region>` (routes to `/v1/hunt`).
- **Hunter:** `POST /v1/hunt` and MCP `emem_hunt` for open-world event discovery. Twelve event keywords (`algal_bloom`, `deforestation`, `flood_extent`, `wildfire`, `urban_heat_island`, `methane_plume`, `landslide`, `drought`, `soil_salinity`, `crop_stress`, `water_turbidity`, `oil_slick`) each map to a registered detection algorithm. The responder samples up to 32 cells from the named region (8 for slow primary bands such as MODIS LST), recalls the algorithm's primary scalar plus any configured gate band (e.g. NDWI > 0 for water-mask events), and returns the top 8 hotspots with cell64, lat/lng, recalled value, gate value, fact CID, and a Sentinel-2 scene URL. A Tessera embedding rerank fires when at least three candidate cells have a geotessera vector available, re-ordering by cosine similarity to the cluster centroid. `oil_slick` returns `status: not_yet_implemented` with pointers at `flood_extent_sar_threshold@1` and `water_turbidity_red_band@1` instead of fabricating detections.
- **EUDR Due Diligence Statement:** `POST /v1/eudr_dds` and MCP `emem_eudr_dds` produce a signed Annex II-shaped DDS under Regulation (EU) 2023/1115. The per-cell algorithm `eudr_compliance@1` implements Article 2(4) as written: >0.5 ha, >5 m height, >10 % canopy cover, excluding land predominantly under agricultural or urban use. The verdict is the consensus of two static baselines read with one windowed COG sample per band over the plot: JRC GFC2020 V3 (the Commission's expected non-binding baseline) for forest-at-cut-off, and Hansen GFC v1.12 loss-year for clearing strictly after the 31 December 2020 cut-off. A cell cleared on or before the cut-off is `not_in_scope`, never a pass. Plot aggregation applies the Article 2(4) 0.5 ha minimum-mapping-unit floor, and the Article 2(28) dispatch picks POINT (≤4 ha non-cattle) vs POLYGON (>4 ha or any cattle plot under HS 0102/0201/0202). Each plot also carries a `loss_year_histogram`, the per-year distribution of Hansen loss-year over its sampled cells, signed as its own `forest_change.lossyear_histogram` derivative whose id is folded into the receipt, so the loss-year breakdown is a verifiable figure rather than an unsigned summary. JRC TMF, Sims et al. 2025 driver attribution, and RADD Sentinel-1 alerts sit off the verdict hot path (their upstreams do not honour HTTP Range); each stays available as an explicit band request, and the responder will not fabricate a value for a connector it cannot read. Two disclaimers keep the scope honest: `legality_disclaimer` for Article 9(1)(b) (land tenure, FPIC, country-of-origin law, structurally out of EO scope), and `degradation_disclaimer` for Article 2(7) forest degradation (the verdict measures deforestation, not degradation). The JSON Schema at `/v1/schemas/eudr_dds.json` cites the exact EUR-Lex paragraph each field maps to; `regulation_status_note` tracks the application deferral (Regulations 2024/3234 and 2025/2650 → 30 December 2026 for large operators, 30 June 2027 for micro and small).
- **Domain shortcuts:** `emem_at`, `emem_ndvi`, `emem_air`, `emem_lst`, `emem_soil`, `emem_water`, `emem_forest`, `emem_weather`. Collapse locate → recall → polygon-aggregate into one call by place name.
- **Field boundaries:** Fields of The World (~3.17 B field polygons, 241 countries, 10 m, CC-BY-4.0) via PMTiles range reads on `source.coop`.
- **Visual surfaces:** `/v1/coverage_map.svg` (1440×720 plate-carrée of attested cells, log-scale density) and `/v1/places/scene_overlay.svg?place=…&band=…` (per-place value-painted bbox grid; band-aware ColorBrewer ramps, horizontal legend, km scale bar, signed source line). The MCP equivalents return the same SVG as an `EmbeddedResource` block. The full set, plus the 32-diagram protocol/industry suite, lives at [/docs/gallery](https://emem.dev/docs/gallery) and [/docs/diagrams](https://emem.dev/docs/diagrams).

<p align="center">
  <img src="docs/diagrams/eudr.png" alt="EUDR supply chain: geolocated plots checked against the forest baseline and Hansen loss-year over the 2020 cut-off, signed into one Due Diligence Statement whose 26-character handle clears customs at TRACES NT" width="900"/>
  <br/>
  <em>One use case, end to end. A geolocated plot is checked against the forest baseline and the Hansen loss-year over the 2020 cut-off, the verdict and its per-year loss histogram are signed into one Due Diligence Statement, and that 26-character handle is what clears customs. This is what <a href="https://eudr.dev">eudr.dev</a> runs on.</em>
</p>

## Algorithms

160 named composition recipes (`flood_risk@2`, `walkability_score@1`, `heat_index@2`, `carbon_sink_score@1`, `eudr_compliance@1`, `forest_carbon_loss_co2_flux@1`, `enteric_ch4_dairy_tier1_ipcc2019@1`, `n2o_synthetic_fertilizer_ef1_ipcc2019@1`, ...) live in a content-addressed registry. Each carries:

- `formula`: plain math the agent can read and apply.
- `inputs`: band keys with role + explanation.
- `when_to_use`: agent-targeted trigger guidance.
- `citation`: peer-reviewed source.
- `accuracy_band`: honest precision estimate, not marketing.
- `parameters`: typed tunable thresholds (gate, k, timeout, ...).
- `learned_from`: citation provenance for every tuned number. An auditor can trace any gate threshold back to a referee.

Algorithms with an `evaluation: Expr` AST are also re-executable in-process: the responder walks the AST against the snapshot recall and returns a signed composite scalar that any third party with matching `algorithms_cid` and input fact CIDs reproduces deterministically.

Browse at [`GET /v1/algorithms`](https://emem.dev/v1/algorithms) or per-key at [`GET /v1/algorithms/<key>`](https://emem.dev/v1/algorithms/clay_prithvi_tessera_triple_consensus@1).

## Discovery

Designed for agents to read, not for humans to remember:

```
GET /openapi.json                  OpenAPI 3.1 of every REST route
GET /v1/agent_card                 live capability snapshot + manifest CIDs
GET /v1/tools                      81 MCP tools (10 core, 71 extended) with when_to_use + annotations
GET /v1/algorithms?summary=true    160 algorithm keys + categories
GET /v1/topics                     27 topic-grouped bands + algorithms (router brain)
GET /v1/manifests                  bands_cid, algorithms_cid, sources_cid, schema_cid
GET /v1/schemas/eudr_dds.json      Annex II JSON Schema with EUR-Lex paragraph citations
GET /.well-known/{emem,agent,mcp,ai-plugin}.json
POST /v1/state                     signed dense state vector at any cell (view=encoder | view=cube)
POST /v1/state_multi               fan-out across geotessera + clay_v1 + prithvi_eo2 with typed missing[]
POST /v1/state_diff                vintage delta at one cell: residual vector + L2 + cosine
POST /v1/memory_token              compose memt:<cell64>:<fact_cid> citation handle
POST /v1/memory_token/resolve      single round-trip dereference back to signed fact body
GET /v1/stream                     Server-Sent Events corpus heartbeat, signed every 5-300 s
GET /v1/corpus_state_stats         signed snapshot of corpus liveness (one-shot equivalent of /v1/stream)
GET /v1/benchmark                  hand-verified eval items; pair with POST /v1/benchmark/grade
POST /v1/hunt                      structured event-discovery sweep (12 events × region)
POST /v1/eudr_dds                  EUDR Due Diligence Statement (Regulation EU 2023/1115)
POST /mcp                          JSON-RPC 2.0 (Streamable HTTP)
GET /llms.txt    /llms-full.txt    plaintext catalog for LLM ingestion
GET /humans      /humans.json      interactive try-it surface + machine twin
GET /verify      /verify/<fact_cid>   in-browser ed25519 receipt verifier
GET /docs/gallery                  live coverage map + hunter case studies + 32 diagrams
GET /docs/diagrams/                32 SVGs of protocol + industry deployments
```

The `operator_attestation` block in `/.well-known/emem.json` binds the running binary's BLAKE3 hash to its `git_commit` + `build_timestamp` and signs the triple under the responder's ed25519 key, so a verifier can confirm the live binary corresponds to the published source tree without trusting the operator.

Every receipt pins four content-addressed registry CIDs (`bands_cid`, `algorithms_cid`, `sources_cid`, `schema_cid`). A peer that recomputes a fact under matching CIDs produces the same bytes. A peer with drifted registries returns a different `bands_cid` on `/health` and the divergence is visible before any data flows.

## Run it locally

```bash
cargo run --release --bin emem-server
# Or via container.
docker run -p 5051:5051 ghcr.io/vortx-ai/emem:latest
```

No required env vars. `EMEM_BIND` overrides the listener (default `0.0.0.0:5051`). `EMEM_DATA` overrides the data directory (default `./var/emem`; pass `:memory:` for ephemeral). For TLS, systemd, ACME on `:443`, and the HuggingFace Space wrapper, see [docs/operators/operating.md](docs/operators/operating.md).

## Address algebra

| field   | bits         | wire form                        | example                    |
|---------|--------------|----------------------------------|----------------------------|
| `cell`  | 64           | four base-1024 bigrams, dot-sep  | `defi.zb493.xuqA.zcb5f`    |
| `tslot` | 64           | base32-nopad-leb128, `t.` prefix | `t.aaaaagy`                |
| `cid`   | 32 B BLAKE3  | base32-nopad-lowercase, 26 chars | `qi3jo4sqcg…l2hgjtwm`      |
| `vec`   | 1792-D fp16  | 12-byte prefix in receipts       | full vector via `recall`   |

The active grid is ~9.54 m × ~9.55 m at the equator (lat 21 bits × lng 22 bits, asymmetric to match the 360°/180° ratio). Above the equator, longitude pitch narrows with cos(lat). The Hilbert-ordered base-1024 alphabet keeps adjacent cells string-prefix-similar, so an LLM that emits `defi.zb493…` already lands in roughly the right place. `GET /v1/grid_info` declares the active resolution honestly; the spec target is a hierarchical migration toward H3-equivalent res-13 (~3.4 m).

## Repo layout

```
emem/
├── crates/                       # 16 workspace crates, MSRV 1.91, version 0.1.0
│   ├── emem-core/                # bands, algorithms, functions, sources, topics, schema
│   ├── emem-codec/               # cell64, cid64, vec64, hilbert, geo, alphabet
│   ├── emem-fact/                # canonical CBOR; fact, receipt, attestation
│   ├── emem-claim/               # claim predicates (Op enum)
│   ├── emem-cache/               # sled cache wrapper
│   ├── emem-fetch/               # 16 data connectors + 13 utility modules
│   ├── emem-storage/             # sled hot cache + append-only merkle log
│   ├── emem-cubes/               # 1792-D voxel cube handle
│   ├── emem-primitives/          # recall, find_similar, trajectory, compare, diff, verify, query_region
│   ├── emem-attest/              # merkle root over fact CIDs
│   ├── emem-intent/              # rule-based intent → plan planner
│   ├── emem-mcp/                 # MCP tool descriptor registry (81 tools, core + extended)
│   ├── emem-api-rest/            # axum router, physics solvers, foundation fan-out
│   ├── emem-cli/                 # binaries: emem-server, emem-livedemo, emem-realdemo, emem-demo, emem-ask-eval
│   ├── emem-membench/            # memory-substrate benchmark harness
│   └── emem-sleep-agent/         # offline refinement loop over contradictions + edges
├── sdks/
│   ├── emem-py/                  # Python client (httpx, sync + async)
│   └── emem-ts/                  # TypeScript client (zero runtime deps, native fetch)
├── python/                       # FastAPI sidecar over UDS: Prithvi-EO-2.0, Galileo, Clay v1.5, JEPA-v2
├── examples/                     # MCP configs + LangChain / LlamaIndex
├── ops/                          # systemd units, journald retention
└── web/                          # SSR HTML, humans, verify, llms.txt, agent.json
```

The 16 data connectors back **46 declared source schemes** and **124 live materializer registrations**. Five of the 46 schemes are declared-but-unwired (`openet.30m.daily`, `dynamic_world.v1`, `tropomi.s5p.ch4`, `tropomi.s5p.no2`, `viirs.dnb.monthly`); they return a typed Absence, not data. Most wired schemes route through `cog.rs`, the universal STAC + COG sampler, plus bespoke modules for `chirps` (rainfall), `dmsp_ols` (nightlights), `esa_cci_biomass` (above-ground biomass, CEDA), `firms` (active fire), `ftw` (Fields of The World), `geonames` (gazetteer), `gmrt` (topobathymetry, PointServer + GridServer), `hansen_gfc` (forest change), `jrc_gfc2020` (EUDR forest baseline, JEODPP single-COG), `jrc_tmf` (tropical moist forest, pull-and-cache), `koppen` (climate classification), `overture` (places / buildings / divisions), `radd_alerts` (Sentinel-1 disturbance), `terraclimate` (climate), `wdpa` (protected areas), `worldpop` (population), `wri_gdm_drivers` (Sims et al. 2025 driver attribution).

## Inference

The GPU sidecar (Python FastAPI over Unix domain socket) co-resides four encoders on a 20 GB VRAM budget:

- **Clay v1.5:** 1024-D CLS, S2 L2A 10 bands, ~12 ms warm. Teacher (DINOv2 `vit_large_patch14_reg4_dinov2.lvd142m`) pre-staged at boot so `HF_HUB_OFFLINE=1` holds.
- **Prithvi-EO-2.0-300M-TL:** 1024-D CLS, HLS V2 6-band, ~13 ms warm.
- **Galileo** (variant `base` in production; `tiny` / `nano` selectable via `EMEM_GALILEO_VARIANT`): S2-only modality wired (S1 / ERA5 / SRTM / VIIRS / Dynamic-World / WorldCover / LandScan / location zero-masked; the scaffold is multimodal but only S2 is connected today). The advertised capability is `galileo-<variant>` in `/v1/capabilities.extensions[]`.
- **JEPA v2 dynamics:** untrained baseline. Metadata-only `is_trained()` check short-circuits to last-vintage identity; receipt carries `untrained_baseline` and `via: "short_circuit_untrained"`. Training is upstream-bottlenecked on multi-vintage Tessera availability.

Sidecar crash does not cascade. The REST router degrades to scalar bands and signs the GPU-anchored algorithms as Absence with `gpu_unavailable`. See [docs/developers/inference.md](docs/developers/inference.md).

## Where this is going

emem is built to be a protocol, not a single service. Because every fact is content-addressed and signed, any responder can serve it and any client can verify it offline, without trusting the source. Today that runs as one hosted responder plus self-hosted nodes. The design target is a federation of independent responders that resolve the same content ids byte-for-byte, cross-cite each other's attestations, and record where they disagree, so the shared memory gets more trustworthy the more agents read and write against it. None of the multi-host federation routing ships in 0.1.0. What ships today is the substrate that makes it possible: content addressing, signed receipts, typed temporal edges, multi-attester contradiction scoring, and a deterministic refinement loop.

<p align="center">
  <img src="docs/diagrams/federation.png" alt="Federation: independent responders ringed around one shared content id, each signing under its own key, all resolving the same id byte-for-byte, cross-citing each other with one pair disagreeing, while clients verify offline" width="900"/>
  <br/>
  <em>The end state: many responders, one address space. A content id means the same bytes everywhere, every responder signs under its own key, and a client trusts the signature instead of the server. Where two responders disagree, the network records it.</em>
</p>

## Honest limits

- **No commercial sub-meter imagery.** Sentinel-2 (10 m), Landsat (30 m), HLS. For Planet Pelican (50 cm) or Maxar bring your own connector.
- **No edge / onboard inference.** Sidecar runs on a single host.
- **Single-host deployment.** No federation, no global routing, no SOC 2.
- **JEPA v2 is untrained today.** The endpoint exists and signs honestly; predictions equal the last attested vintage until the dynamics head is trained.
- **16 data connectors, 124 live materializer registrations.** Catalog-by-count is not the pitch; every wired band is auto-fetchable, signed, and content-addressed. Bands without a wired materializer are listed under `declared_but_no_materializer_at_this_responder`.
- **Foundation-encoder materializers are uneven.** `geotessera` (Tessera 128-D) has a wired materializer and auto-fetches on miss. `clay_v1` and `prithvi_eo2` are seed-only at this responder: the GPU sidecar runs both models, but the auto-materialise path that fans out to upstream tile archives is not wired today. Recall against either returns whatever has already been signed; the hunter-mode envelope discloses this per request under `materializer_status[]`.
- **Tessera is upstream-rate-limited.** `dl2.geotessera.org` reliably serves 2024 vintages today; historical backfill across all eight vintages (2017–2024) is partial. The Tessera-coherence rerank in hunter mode gracefully degrades to primary-scalar order when the upstream is unreachable, surfacing the reason under `embedding_rerank.reason`.
- **MODIS LST is rate-limited.** `modis.lst_day_8day` materialises through the NASA/ORNL REST API at roughly 30 s per cell. Hunter mode caps the per-region fan-out for the LST family to 8 cells (env override `EMEM_HUNTER_SLOW_BAND_CAP`) so urban-heat queries return inside the gateway timeout.
- **No interactive notebook UI.** For exploration there is `/humans` (try-it drawer, manifest grid, ontology SVG); for analytics, drive from a notebook against the REST or MCP endpoint.

## Resources

| | |
|--|--|
| Agent loop  | [https://emem.dev/agents.md](https://emem.dev/agents.md)                                           |
| Wire spec   | [https://emem.dev/spec.md](https://emem.dev/spec.md)                                               |
| llms.txt    | [https://emem.dev/llms.txt](https://emem.dev/llms.txt)                                             |
| OpenAPI 3.1 | [https://emem.dev/openapi.json](https://emem.dev/openapi.json)                                     |
| MCP         | `https://emem.dev/mcp`                                                                             |
| Verify      | [https://emem.dev/verify](https://emem.dev/verify)                                                 |
| Container   | `ghcr.io/vortx-ai/emem:latest` (multi-arch, anonymously pullable)                                  |
| HF Space    | [huggingface.co/spaces/vortx-ai/emem](https://huggingface.co/spaces/vortx-ai/emem)                 |
| MCP Directory | [docs/mcp-directory.md](docs/mcp-directory.md)                                                    |
| Issues / PRs| [github.com/Vortx-AI/emem/issues](https://github.com/Vortx-AI/emem/issues)                         |
| Security    | [SECURITY.md](SECURITY.md), `avijeet@vortx.ai`                                                     |

## License

Apache-2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).

Default-build data sources are open: Copernicus DEM, JRC GSW (CC-BY 4.0), Hansen GFC, ESA WorldCover (CC-BY 4.0), Overture Maps (places, buildings, transportation, `divisions/division_area`; ODbL / CDLA-Permissive), Fields of The World (CC-BY 4.0), GeoNames cities-5000 (CC-BY 4.0), OSM (ODbL), met.no, Open-Meteo, Tessera. No API keys, no operator credentials, no SaaS lock-in.
