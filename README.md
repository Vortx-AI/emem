<div align="center">
  <img src="web/logo-mark.png" width="128" alt="emem" />

  <h1>emem</h1>

  <p><strong>Earth memory protocol for AI agents.</strong><br/>
  Signed, content-addressed, lazy-materialised memory of every place on Earth.</p>

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
    <a href="https://emem.dev/humans">Demo</a> ·
    <a href="https://huggingface.co/spaces/vortx-ai/emem">HF Space</a>
  </p>
</div>

---

LLMs hallucinate when asked "what is at this place" because they have no stable place to ground the answer. emem is that place. Every fact lives at the tuple `(cell, band, tslot)`. Its canonical CBOR hashes to a content ID. Every API call returns an ed25519 receipt that any client verifies offline against the responder's published public key.

When the cache is cold, the band is fetched from open data, signed, and persisted. The next caller pays nothing and gets the exact same bytes. REST and MCP read from the same router. Reads are open. The hosted instance is at `https://emem.dev`.

## Try it (no install, no key)

Geocode a place to a `cell64`, then recall a band at that cell.

```bash
curl -s -X POST https://emem.dev/v1/locate \
  -H 'content-type: application/json' \
  -d '{"q":"Bengaluru"}' | jq .cell64
# "defi.zb493.xoso.zcb6a"

curl -s -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb493.xoso.zcb6a","bands":["weather.temperature_2m"]}' \
  | jq '.facts[0]'
# { "band": "weather.temperature_2m", "value": 30.9, "unit": "degC", ... }
```

The receipt's `fact_cid` is a durable handle. Re-fetching it from any responder, in any year, returns the same bytes.

## Connect your AI assistant

The MCP endpoint is the same everywhere: `https://emem.dev/mcp`. Drop a config snippet into your client.

| Client                | Config                                                                      |
|-----------------------|-----------------------------------------------------------------------------|
| Claude Desktop        | [examples/claude-desktop.json](examples/claude-desktop.json)                |
| Claude Code           | [examples/claude-code.mcp.json](examples/claude-code.mcp.json)              |
| Cursor                | [examples/cursor.mcp.json](examples/cursor.mcp.json)                        |
| Cline (VS Code)       | [examples/cline.mcp.json](examples/cline.mcp.json)                          |
| Gemini CLI            | `gemini extensions install https://emem.dev/gemini-extension.json`          |
| ChatGPT (Custom GPT)  | [examples/openai-gpt-action.json](examples/openai-gpt-action.json)          |
| LangChain (Python)    | [examples/langchain.py](examples/langchain.py)                              |
| LlamaIndex (Python)   | [examples/llamaindex.py](examples/llamaindex.py)                            |

Or use the SDKs:

```bash
pip install emem            # sdks/emem-py: sync + async
npm install @emem/client    # sdks/emem-ts: zero runtime deps, native fetch
```

## What you can ask

36 MCP tools, 68 endpoints under `/v1/*`. The shape stays small.

- **Locate** a place by name or lat/lng to a `cell64`. A five-layer cascade resolves names without hitting a public geocoder for any of the ~68 000 populated places GeoNames carries.
- **Recall** any of 35 bands at any cell. Cold reads auto-fetch from open data.
- **Recall polygon** facts across every cell inside a place's boundary. The boundary itself comes from Overture's `divisions/division_area` theme, so the polygon-resolution path is keyless open data, not a public geocoder.
- **Field boundaries** — per-field agricultural polygons from Fields of The World (~3.17 B fields, 241 countries, 10 m, CC-BY-4.0). Pure-fetch shape or `?include=ftw_fields` on recall_polygon.
- **Compare** two cells, or two bands at one cell, with an optional signed verdict.
- **Find similar** places by foundation embedding (Tessera 128-D, plus a sign-bit Hamming fast path).
- **Trajectory** of a band over time at a cell.
- **Diff** a band between two timestamps.
- **Query a region** by polygon or bbox with mean / median / p90 / vector centroid.
- **Verify** a claim like "band ≤ X at cell" without trusting the responder.
- **Solve** physics: 2-D heat, 1-D wave, JEPA-v2 dynamics (CPU plus optional CUDA sidecar).
- **Ask** a free-text question and get a topic-routed multi-band answer with citable receipts.

107 named composition algorithms (`flood_risk@2`, `walkability_score@1`, `heat_index@2`, `carbon_sink_score@1`, `eudr_compliance@1`, ...) compose those primitives into named scores. Browse the live registry at `GET /v1/algorithms`. The full agent-targeted catalogue is at `GET /v1/agent_card`.

## Run it locally

```bash
# From source
cargo run --release --bin emem-server
# Or via container
docker run -p 5051:5051 ghcr.io/vortx-ai/emem:0.0.6
```

The server has no required env vars. `EMEM_BIND` overrides the listener (default `0.0.0.0:5051`). `EMEM_DATA` overrides the data directory (default `./var/emem`; use `:memory:` for ephemeral).

For TLS, systemd, ACME on `:443`, and the HuggingFace Space wrapper, see [docs/operating.md](docs/operating.md) and [https://emem.dev/agents.md](https://emem.dev/agents.md) (`§ Self-host`).

## How it works

A **fact** is a value at `(cell, band, tslot)` plus its provenance: source (`met.no`, `Copernicus DEM`, `Sentinel-2 L2A`, ...), derivation function (a key in `functions-v0.json`), and captured-at / signed-at timestamps. The canonical CBOR of the fact, hashed with BLAKE3, is its CID. Two responders running the same derivation against the same upstream produce the same CID byte-for-byte.

A **receipt** wraps a primitive call. It lists the cells, the fact CIDs, the responder pubkey, and an ed25519 signature over a stable preimage. Receipts verify without calling back to the issuer; the pubkey is published at `/.well-known/emem.json` and `/v1/agent_card`.

**Lazy materialisation** means a band is fetched the first time anyone asks. `POST /v1/recall` on a cold cell fans out to the connector for that band, decodes the upstream sample (vsicurl Range reads against open COGs, JSON forecasts, STAC searches), wraps the value as a fact, signs, persists, returns. The CID is computed before signing, so the materialised fact is identical to what any peer would have produced.

### Address algebra

| field   | bits         | wire form                        | example                    |
|---------|--------------|----------------------------------|----------------------------|
| `cell`  | 64           | four base-1024 bigrams, dot-sep  | `defi.zb493.xoso.zcb6a`    |
| `tslot` | 64           | base32-nopad-leb128, `t.` prefix | `t.aaaaagy`                |
| `cid`   | 32 B BLAKE3  | base32-nopad-lowercase, 26 chars | `qi3jo4sqcg…l2hgjtwm`      |
| `vec`   | 1792-D fp16  | 12-byte prefix in receipts       | (full vector via `recall`) |

The active grid is a ~9.55 m × ~9.54 m raster at the equator (lat 21 bits × lng 22 bits, asymmetric to match the 360° / 180° ratio). Above the equator, longitude pitch narrows with cos(lat). The spec target is the ~3.4 m H3 hexagonal DGGS; that migration is not yet active, and `GET /v1/grid_info` declares the current resolution honestly. The Hilbert-ordered base-1024 alphabet keeps neighbouring cells string-prefix-similar, so an LLM that emits `defi.zb493.…` is already at the right place.

### Conformance

Every receipt pins four content-addressed registries: `bands_cid`, `algorithms_cid`, `sources_cid`, `schema_cid`. They are the manifest CIDs of the 35-band 1792-D layout, the 107 named algorithm recipes, the source catalogue, and the wire schema. A peer that recomputes a fact under matching CIDs produces the same bytes. A peer with drifted registries returns a different `bands_cid` in `/health` and the divergence is visible before any data flows.

## Surface map

```
REST   139 routes  (68 under /v1/*, 7 under /.well-known/, plus /health,
                   /openapi.json, /openapi.action.json, /mcp, /metrics,
                   /llms.txt, /llms-full.txt, /humans, /agents.md and
                   the static landing/docs/branding tree)
MCP    36 tools    (11 read primitives incl. recall_polygon + field_boundaries,
                   4 physics solvers, 14 introspection, 2 imagery, 1 backfill,
                   1 verify, 1 fetch, 1 ask, 1 intent)
```

Discover at `GET /v1/discover`, `GET /v1/agent_card`, or `GET /openapi.json`. The MCP transport is `POST /mcp` (JSON-RPC 2.0).

For humans (and AI agents that want to watch what humans do), [https://emem.dev/humans](https://emem.dev/humans) is an interactive map of the corpus. Every attested cell is a star; every fact carries a clickable signed receipt. A console pane prints every `/v1/*` call the page makes, so an LLM scraping the rendered DOM learns the API by observation. Receipts verify in-browser via Ed25519 + BLAKE3, no server roundtrip.

## Repo layout

```
emem/
├── crates/                       # 14 workspace crates, MSRV 1.88, version 0.0.6
│   ├── emem-core/                # bands, algorithms, functions, sources, topics, schema
│   ├── emem-codec/               # cell64, cid64, vec64, hilbert, geo, alphabet
│   ├── emem-fact/                # canonical CBOR; fact, receipt, attestation
│   ├── emem-claim/               # claim predicates (Op enum)
│   ├── emem-cache/               # sled cache wrapper
│   ├── emem-fetch/               # 18 open-data connectors (cog, hansen, jrc, esa, overture, overture-divisions, ftw, geonames, firms, ...)
│   ├── emem-storage/             # sled hot cache + append-only merkle log
│   ├── emem-cubes/               # 1792-D voxel cube handle (offsets in bands-v0.json)
│   ├── emem-primitives/          # recall, find_similar, trajectory, compare, compare_bands, diff, verify, query_region
│   ├── emem-attest/              # merkle root over fact CIDs
│   ├── emem-intent/              # rule-based intent → plan planner
│   ├── emem-mcp/                 # MCP tool registry
│   ├── emem-api-rest/            # axum router, physics solvers
│   └── emem-cli/                 # binaries: emem-server, emem-livedemo, emem-realdemo, emem-demo, emem-ask-eval
├── sdks/
│   ├── emem-py/                  # Python client (httpx, sync + async)
│   └── emem-ts/                  # TypeScript client (zero runtime deps, native fetch)
├── python/                       # FastAPI sidecar (Prithvi-EO-2.0, Galileo, JEPA-v2 dynamics) over UDS
├── examples/                     # MCP configs (Claude / Cursor / Cline / Gemini / ChatGPT) + LangChain / LlamaIndex
├── ops/                          # systemd units, journald retention
├── scripts/                      # redeploy, install-topic-model, global_trial
└── web/                          # SSR HTML + llms.txt + agents.md
```

## Status

**Ships in 0.0.6**

- Read primitives: `recall`, `recall_many`, `recall_polygon`, `field_boundaries`, `find_similar`, `compare`, `compare_bands`, `trajectory`, `diff`, `query_region`, `verify`.
- Place resolution: five-layer cascade — wide-bbox table → embedded gazetteer → GeoNames cities-5000 (68 581 places, embedded) → sled cache → Photon → Nominatim. Polygon geometry comes from Overture's `divisions/division_area` theme; Nominatim handles only the long tail.
- Agricultural fields: Fields of The World global product (~3.17 B field polygons, 10 m, 241 countries, CC-BY-4.0) via PMTiles range reads on source.coop. Surfaced as the standalone `/v1/field_boundaries` primitive and as the `include: ["ftw_fields"]` supplement on `/v1/recall_polygon`.
- Physics solvers: 1-D wave, 2-D heat, JEPA-v2 dynamics (CPU; CUDA when `EMEM_SIDECAR_SOCK` points at a live UDS).
- Foundation embeddings: `geotessera` as 8 annual vintages 2017 to 2024 (each 128-D), plus `geotessera.bin128` (sign-bit) and `geotessera.multi_year` (1024-D, 8 × 128 stacked). Prithvi-EO-2.0-300M-TL and Galileo through the sidecar.
- Lazy materialisation: cold-cell recall fans out to the connector, signs, persists. Gated by `EMEM_AUTO_MATERIALIZE`.
- Receipts: ed25519 over a stable preimage; identity persisted at `<EMEM_DATA>/identity.secret.b32`. Verified offline by `verify_receipt`.
- Discovery: `/v1/discover`, `/v1/agent_card`, `/openapi.json`, `/.well-known/{emem,agent,mcp,ai-plugin}.json`.
- TLS termination: in-process rustls + Let's Encrypt ACME via TLS-ALPN-01. No Cloudflare, no reverse proxy.
- Python and TypeScript SDKs at version 0.0.6, covering every major `/v1/*` endpoint plus the boring lat/lng shortcuts.

**Deferred**

- zkML proofs. Receipts today are signed, not zero-knowledge.
- Trained JEPA-v2 dynamics head. Upstream Tessera now ships 8 vintages (2017 to 2024), but only the showcase cells have all 8 attested on the responder. Training the dynamics head needs the multi-year stack materialised across a wider candidate pool. Backfill is the unblocker. Until then, `/v1/jepa_predict_v2` returns the residual identity baseline with an `untrained_baseline` warning on the receipt.

## Resources

| | |
|--|--|
| Agent loop  | [https://emem.dev/agents.md](https://emem.dev/agents.md)                                           |
| Wire spec   | [https://emem.dev/spec.md](https://emem.dev/spec.md)                                               |
| llms.txt    | [https://emem.dev/llms.txt](https://emem.dev/llms.txt)                                             |
| OpenAPI 3.1 | [https://emem.dev/openapi.json](https://emem.dev/openapi.json)                                     |
| MCP         | `https://emem.dev/mcp`                                                                             |
| Container   | `ghcr.io/vortx-ai/emem:latest` (multi-arch, anonymously pullable)                                  |
| HF Space    | [huggingface.co/spaces/vortx-ai/emem](https://huggingface.co/spaces/vortx-ai/emem)                 |
| Issues / PRs| [github.com/Vortx-AI/emem/issues](https://github.com/Vortx-AI/emem/issues)                         |
| Security    | [SECURITY.md](SECURITY.md), `avijeet@vortx.ai`                                                     |

## License

Apache-2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).

All default-build data sources are open: Copernicus DEM, JRC GSW (CC-BY 4.0), Hansen GFC, ESA WorldCover (CC-BY 4.0), Overture Maps (places + buildings + transportation + `divisions/division_area` admin boundaries; ODbL / CDLA-Permissive), Fields of The World (~3.17 B agricultural-field polygons, CC-BY 4.0), GeoNames cities-5000 (embedded gazetteer, CC-BY 4.0), OSM (ODbL), met.no, Open-Meteo, Tessera. No API keys, no operator credentials, no SaaS lock-in.
