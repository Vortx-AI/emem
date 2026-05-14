<div align="center">
  <img src="web/logo-mark.png" width="128" alt="emem" />

  <h1>emem</h1>

  <p><strong>Verifiable Earth observation for AI agents.</strong><br/>
  Three foundation encoders, one consensus, every answer cryptographically signed.</p>

  <p>
    <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue.svg" alt="License"></a>
    <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/rust-1.88-orange.svg" alt="Rust"></a>
    <a href="https://modelcontextprotocol.io/"><img src="https://img.shields.io/badge/MCP-Streamable%20HTTP-7af.svg" alt="MCP"></a>
    <a href="https://www.openapis.org/"><img src="https://img.shields.io/badge/OpenAPI-3.1-green.svg" alt="OpenAPI"></a>
    <a href="https://github.com/Vortx-AI/emem/pkgs/container/emem"><img src="https://img.shields.io/badge/ghcr.io-vortx--ai%2Femem-181717?logo=github" alt="Container"></a>
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

<p align="center">
  <a href="https://emem.dev/v1/coverage_map.svg">
    <img src="docs/gallery/coverage-map.svg" alt="emem global coverage — every dot is a cell with at least one signed fact" width="900"/>
  </a><br/>
  <em>Where emem has attested facts right now — 1440×720 plate-carrée. The same SVG renders live at <code>/v1/coverage_map.svg</code>.</em>
</p>

<p align="center">
  <a href="https://emem.dev/v1/places/scene_overlay.svg?place=Mumbai&band=copdem30m.elevation_mean&n_cells=64"><img src="docs/gallery/mumbai-elevation.svg" alt="Mumbai elevation" width="230"/></a>
  <a href="https://emem.dev/v1/places/scene_overlay.svg?place=Manhattan&band=copdem30m.elevation_mean&n_cells=64"><img src="docs/gallery/manhattan-elevation.svg" alt="Manhattan elevation" width="230"/></a>
  <a href="https://emem.dev/v1/places/scene_overlay.svg?place=Tokyo&band=copdem30m.elevation_mean&n_cells=64"><img src="docs/gallery/tokyo-elevation.svg" alt="Tokyo elevation" width="230"/></a>
  <br/>
  <em>Mumbai · Manhattan · Tokyo, painted by Copernicus DEM elevation. Each image is a live endpoint URL — click to see the latest signed render. <a href="https://emem.dev/docs/gallery">/docs/gallery</a> has the full set.</em>
</p>

---

LLMs hallucinate "what is at this place" because they have no stable handle on the ground. emem is that handle. Every fact lives at the tuple `(cell, band, tslot)`; the canonical CBOR of the fact hashes to a content ID; every read returns an ed25519 receipt that any client verifies offline. Open `https://emem.dev/verify/<fact_cid>` and the signature math runs in your browser — there is nothing to trust about the issuer.

The hosted instance is at `https://emem.dev`. REST and MCP read from the same router. No API keys.

## Why this exists

Three things competitors do not ship together:

- **Triple-encoder consensus.** Clay v1.5 (1024-D, 2.56 km receptive field), Prithvi-EO-2.0 (1024-D, 6.7 km), and Tessera (128-D per-pixel S1+S2) vote on year-on-year change. Their receptive-field aliasing is independent, so consensus across all three is signal where any single model is noise. Surfaced as `clay_prithvi_tessera_triple_consensus@1` plus six domain variants (`deforestation_triple@1`, `wetland_change_triple@1`, `urban_expansion_triple@1`, `disaster_anomaly_triple@1`, `climate_archetype_triple@1`, `coastal_erosion_triple@1`).

- **Signed receipts you verify yourself.** Every read returns ed25519 over `(request_id | served_at | primitive | cells | fact_cids)`. The browser-side verifier at [/verify](https://emem.dev/verify) reconstructs the preimage and runs the signature check with [`@noble/curves`](https://github.com/paulmillr/noble-curves) — no callback to the issuer. The responder's public key is at `/.well-known/emem.json`.

- **Signed Absence.** When a band has no data at a cell, the responder returns a signed Absence fact with a typed reason (`unavailable_capability`, `outside_coverage`, `archetype_seed_unavailable`, ...) — not a 404, not an empty array. "We don't have this here" is itself a citable receipt.

The protocol layers a fourth differentiator on top: **auto-materialize on miss**. An empty `/v1/recall` on a cell with a registered materializer triggers an upstream fetch (Sentinel-2 STAC + COG range reads, Copernicus DEM, JRC GSW, Hansen GFC, Overture, ...), signs the result under the responder's identity, persists it, returns in the same response. ~180 ms cold, ~10 ms warm. Every cell on Earth answers without pre-seeding.

## Try it (no install, no key)

```bash
# Geocode a place to a cell64.
curl -s -X POST https://emem.dev/v1/locate \
  -H 'content-type: application/json' \
  -d '{"q":"Bengaluru"}' | jq .cell64
# "defi.zb493.xoso.zcb6a"

# Recall a band at that cell — auto-fetched if cold.
curl -s -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb493.xoso.zcb6a","bands":["weather.temperature_2m"]}' \
  | jq '.facts[0]'

# Ask a free-text question; the foundation-embedding fan-out fires
# automatically on "find places like" / "what changed" intents.
curl -s -X POST https://emem.dev/v1/ask \
  -H 'content-type: application/json' \
  -d '{"q":"find places like Yellowstone","place":"Yellowstone National Park"}' \
  | jq '.foundation_embeddings'
```

The receipt's `fact_cid` is a durable handle. Re-fetching it from any responder, in any year, returns the same bytes.

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
| LlamaIndex (Python)   | [examples/llamaindex.py](examples/llamaindex.py)                    |

Python and TypeScript SDKs live under `sdks/` (publication to PyPI / NPM pending; install from the repo today).

## Primitives

49 MCP tools, 71 documented REST paths (68 under `/v1/*`, surfaced through `/openapi.json`). Every tool carries a `when_to_use` string written for LLM tool-selection, and four MCP behavioural annotations (`readOnlyHint`, `destructiveHint`, `idempotentHint`, `openWorldHint`).

- **Locate** — name or lat/lng → `cell64`. Five-layer cascade: wide-bbox table → embedded gazetteer → GeoNames cities-5000 (68 581 places, in-process) → sled cache → Photon → Nominatim. Polygon geometry from Overture `divisions/division_area`. District-level queries reroute through Overture when Nominatim returns a POI courthouse.
- **Recall / recall_many / recall_polygon** — 118 materializer-wired band names across 35 cube slots. Auto-fetch on miss; signed Absence on out-of-coverage.
- **Find similar** — k-NN over any vector band. Hamming fast path (sign-bit pop-count) auto-derives from the cosine band when the binary sibling is absent. Mode `hamming_then_rerank` triages with Hamming then re-orders by cosine; the over-sampling factor is EWMA-adaptive.
- **Compare / compare_bands / diff / trajectory** — pairwise and time-series.
- **Verify** — structured claim against attested facts; returns signed verdict + evidence CIDs.
- **Physics** — `/v1/heat_solve` (2-D explicit FTCS heat, MODIS LST stencil), `/v1/wave_solve` (1-D shallow-water along seaward bathymetry gradient), `/v1/jepa_predict` (closed-form NDVI AR(2) seasonal), `/v1/jepa_predict_v2` (Tessera embedding dynamics; short-circuits to last-vintage identity baseline while the trained head is pending — receipt carries `untrained_baseline`).
- **Ask** — free-text question with topic routing. Intents matching "find places like" / "what changed" / "deforestation" / "anomaly" fan out across the three foundation encoders concurrently; the response carries `foundation_embeddings` with per-encoder neighbour lists and cross-encoder consensus voting.
- **Domain shortcuts** — `emem_at`, `emem_ndvi`, `emem_air`, `emem_lst`, `emem_soil`, `emem_water`, `emem_forest`, `emem_weather`. Collapse locate → recall → polygon-aggregate into one call by place name.
- **Field boundaries** — Fields of The World (~3.17 B field polygons, 241 countries, 10 m, CC-BY-4.0) via PMTiles range reads on `source.coop`.

## Algorithms

155 named composition recipes (`flood_risk@2`, `walkability_score@1`, `heat_index@2`, `carbon_sink_score@1`, `eudr_compliance@1`, ...) live in a content-addressed registry. Each carries:

- `formula` — plain math the agent can read and apply.
- `inputs` — band keys with role + explanation.
- `when_to_use` — agent-targeted trigger guidance.
- `citation` — peer-reviewed source.
- `accuracy_band` — honest precision estimate, not marketing.
- `parameters` — typed tunable thresholds (gate, k, timeout, ...).
- `learned_from` — citation provenance for every tuned number. An auditor can trace any gate threshold back to a referee.

Algorithms with an `evaluation: Expr` AST are also re-executable in-process: the responder walks the AST against the snapshot recall and returns a signed composite scalar that any third party with matching `algorithms_cid` and input fact CIDs reproduces deterministically.

Browse at [`GET /v1/algorithms`](https://emem.dev/v1/algorithms) or per-key at [`GET /v1/algorithms/<key>`](https://emem.dev/v1/algorithms/clay_prithvi_tessera_triple_consensus@1).

## Discovery

Designed for agents to read, not for humans to remember:

```
GET /openapi.json                  — OpenAPI 3.1 of every REST route
GET /v1/agent_card                 — live capability snapshot + manifest CIDs
GET /v1/tools                      — 49 MCP tools with when_to_use + annotations
GET /v1/algorithms?summary=true    — 155 algorithm keys + categories
GET /v1/manifests                  — bands_cid, algorithms_cid, sources_cid, schema_cid
GET /.well-known/{emem,agent,mcp,ai-plugin}.json
POST /mcp                          — JSON-RPC 2.0 (Streamable HTTP)
GET /llms.txt    /llms-full.txt    — plaintext catalog for LLM ingestion
GET /humans      /humans.json      — interactive try-it surface + machine twin
GET /verify  /verify/<fact_cid>    — in-browser ed25519 receipt verifier
```

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
| `cell`  | 64           | four base-1024 bigrams, dot-sep  | `defi.zb493.xoso.zcb6a`    |
| `tslot` | 64           | base32-nopad-leb128, `t.` prefix | `t.aaaaagy`                |
| `cid`   | 32 B BLAKE3  | base32-nopad-lowercase, 26 chars | `qi3jo4sqcg…l2hgjtwm`      |
| `vec`   | 1792-D fp16  | 12-byte prefix in receipts       | full vector via `recall`   |

The active grid is ~9.54 m × ~9.55 m at the equator (lat 21 bits × lng 22 bits, asymmetric to match the 360°/180° ratio). Above the equator, longitude pitch narrows with cos(lat). The Hilbert-ordered base-1024 alphabet keeps adjacent cells string-prefix-similar, so an LLM that emits `defi.zb493…` already lands in roughly the right place. `GET /v1/grid_info` declares the active resolution honestly; the spec target is a hierarchical migration toward H3-equivalent res-13 (~3.4 m).

## Repo layout

```
emem/
├── crates/                       # 14 workspace crates, MSRV 1.88, version 0.0.6
│   ├── emem-core/                # bands, algorithms, functions, sources, topics, schema
│   ├── emem-codec/               # cell64, cid64, vec64, hilbert, geo, alphabet
│   ├── emem-fact/                # canonical CBOR; fact, receipt, attestation
│   ├── emem-claim/               # claim predicates (Op enum)
│   ├── emem-cache/               # sled cache wrapper
│   ├── emem-fetch/               # 12 data connectors + 6 utility modules
│   ├── emem-storage/             # sled hot cache + append-only merkle log
│   ├── emem-cubes/               # 1792-D voxel cube handle
│   ├── emem-primitives/          # recall, find_similar, trajectory, compare, diff, verify, query_region
│   ├── emem-attest/              # merkle root over fact CIDs
│   ├── emem-intent/              # rule-based intent → plan planner
│   ├── emem-mcp/                 # 49-tool MCP descriptor registry
│   ├── emem-api-rest/            # axum router, physics solvers, foundation fan-out
│   └── emem-cli/                 # binaries: emem-server, emem-livedemo, emem-realdemo, emem-demo, emem-ask-eval
├── sdks/
│   ├── emem-py/                  # Python client (httpx, sync + async)
│   └── emem-ts/                  # TypeScript client (zero runtime deps, native fetch)
├── python/                       # FastAPI sidecar over UDS: Prithvi-EO-2.0, Galileo, Clay v1.5, JEPA-v2
├── examples/                     # MCP configs + LangChain / LlamaIndex
├── ops/                          # systemd units, journald retention
└── web/                          # SSR HTML, humans, verify, llms.txt, agent.json
```

The 12 data connectors back **43 declared source schemes** and **20 live materializer registrations** — most schemes route through `cog.rs`, the universal STAC + COG sampler, plus bespoke modules for `chirps`, `dmsp_ols`, `firms`, `ftw`, `geonames`, `hansen_gfc`, `koppen`, `overture`, `terraclimate`, `wdpa`, `worldpop`.

## Inference

The GPU sidecar (Python FastAPI over Unix domain socket) co-resides four encoders on a 20 GB VRAM budget:

- **Clay v1.5** — 1024-D CLS, S2 L2A 10 bands, ~12 ms warm. Teacher (DINOv2 `vit_large_patch14_reg4_dinov2.lvd142m`) pre-staged at boot so `HF_HUB_OFFLINE=1` holds.
- **Prithvi-EO-2.0-300M-TL** — 1024-D CLS, HLS V2 6-band, ~13 ms warm.
- **Galileo** (variant `base` in production; `tiny` / `nano` selectable via `EMEM_GALILEO_VARIANT`) — S2-only modality wired (S1 / ERA5 / SRTM / VIIRS / Dynamic-World / WorldCover / LandScan / location zero-masked; the scaffold is multimodal but only S2 is connected today). The advertised capability is `galileo-<variant>` in `/v1/capabilities.extensions[]`.
- **JEPA v2 dynamics** — untrained baseline. Metadata-only `is_trained()` check short-circuits to last-vintage identity; receipt carries `untrained_baseline` and `via: "short_circuit_untrained"`. Training is upstream-bottlenecked on multi-vintage Tessera availability.

Sidecar crash does not cascade — the REST router degrades to scalar bands and signs the GPU-anchored algorithms as Absence with `gpu_unavailable`. See [docs/developers/inference.md](docs/developers/inference.md).

## Honest limits

- **No commercial sub-meter imagery.** Sentinel-2 (10 m), Landsat (30 m), HLS. For Planet Pelican (50 cm) or Maxar bring your own connector.
- **No edge / onboard inference.** Sidecar runs on a single host.
- **Single-host deployment.** No federation, no global routing, no SOC 2.
- **JEPA v2 is untrained today.** The endpoint exists and signs honestly; predictions equal the last attested vintage until the dynamics head is trained.
- **12 data connectors, 20 live materializer registrations.** Catalog-by-count is not the pitch — every wired band is auto-fetchable, signed, and content-addressed. Bands without a wired materializer are listed under `declared_but_no_materializer_at_this_responder`.
- **Tessera is upstream-rate-limited.** `dl2.geotessera.org` reliably serves 2024 vintages today; historical backfill across all eight vintages (2017–2024) is partial.
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
| Issues / PRs| [github.com/Vortx-AI/emem/issues](https://github.com/Vortx-AI/emem/issues)                         |
| Security    | [SECURITY.md](SECURITY.md), `avijeet@vortx.ai`                                                     |

## License

Apache-2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).

Default-build data sources are open: Copernicus DEM, JRC GSW (CC-BY 4.0), Hansen GFC, ESA WorldCover (CC-BY 4.0), Overture Maps (places, buildings, transportation, `divisions/division_area`; ODbL / CDLA-Permissive), Fields of The World (CC-BY 4.0), GeoNames cities-5000 (CC-BY 4.0), OSM (ODbL), met.no, Open-Meteo, Tessera. No API keys, no operator credentials, no SaaS lock-in.
