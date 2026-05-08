# emem — Earth memory protocol for AI agents

Signed, content-addressed, lazy-materialised memory of every place on Earth.

[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.88-orange.svg)](https://www.rust-lang.org/)
[![MCP](https://img.shields.io/badge/MCP-Streamable%20HTTP-7af.svg)](https://modelcontextprotocol.io/)
[![OpenAPI](https://img.shields.io/badge/OpenAPI-3.1-green.svg)](https://www.openapis.org/)
[![Container](https://img.shields.io/badge/ghcr.io-vortx--ai%2Femem-181717?logo=github)](https://github.com/Vortx-AI/emem/pkgs/container/emem)

LLMs hallucinate spatial answers because they have no stable place to ground them.
emem is that place. Every fact is the tuple `(cell, band, tslot)`; the canonical
CBOR of that tuple hashes to a CID; every read carries an ed25519 receipt that any
client can verify offline. When the cache is cold, a band is materialised from open
data on demand, signed by the responder, and persisted — so the second caller pays
nothing and the answer is byte-for-byte the same.

The protocol is for agents. REST and MCP are the same wire. Reads need no auth.
Live at [https://emem.dev](https://emem.dev), source at
[github.com/Vortx-AI/emem](https://github.com/Vortx-AI/emem).

## First call in 60 seconds

Geocode a place to a `cell64`, then recall a band at that cell. Both calls go to
the hosted responder — no install, no key.

```bash
# 1. Resolve "Bengaluru" to a cell64 (the responder uses OSM Photon/Nominatim).
curl -s -X POST https://emem.dev/v1/locate \
  -H 'content-type: application/json' \
  -d '{"q":"Bengaluru"}' | jq '.cell64, .place_label'

# → "defi.zb493.xoso.zcb6a"
# → "Bengaluru, India"

# 2. Recall current 2 m air temperature at that cell.
curl -s -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb493.xoso.zcb6a","bands":["weather.temperature_2m"]}' \
  | jq '.facts[0] | {band, value, unit, signed_at}, .receipt.fact_cids[0]'

# → { "band": "weather.temperature_2m", "value": 30.9, "unit": "degC", ... }
# → "qi3jo4sqcgklqfcvrfl55ctz4l2hgjtwmvzxunsrkmifyfq4h25a"
```

The `fact_cid` is the durable handle. Re-querying it from any responder, in any
year, returns the same bytes. The receipt's `signature` field over
`blake3(request_id ‖ served_at ‖ primitive ‖ cells ‖ fact_cids)` proves which
responder served it.

## Run it locally

```bash
cargo run --release --bin emem-server
# binds 0.0.0.0:5051, persists to ./var/emem
```

The server has no required env vars. `EMEM_BIND` overrides the listener;
`EMEM_DATA` overrides the data directory (`:memory:` for ephemeral). For
TLS, systemd, the GHCR container, ACME on `:443`, and the HuggingFace
Space wrapper, see the operating guide at
[https://emem.dev/agents.md](https://emem.dev/agents.md) (`§ Self-host`).

## Address algebra

| field   | bits        | wire form                       | example                    |
|---------|-------------|----------------------------------|----------------------------|
| `cell`  | 64          | four base-1024 bigrams, dot-sep  | `defi.zb493.xoso.zcb6a`    |
| `tslot` | 64          | base32-nopad-leb128, `t.` prefix | `t.aaaaagy`                |
| `cid`   | 32 B blake3 | base32-nopad-lowercase, 26 chars | `qi3jo4sqcg…l2hgjtwm`     |
| `vec`   | 1792 D fp16 | 12-byte prefix in receipts       | (full vector via `recall`) |

The active grid is a square ~9.55 m × ~9.54 m raster at the equator (lat 21
bits × lng 22 bits — asymmetric to match the 360°/180° ratio). Above the
equator, longitude pitch narrows with cos(lat). The spec target is the
~3.4 m H3 hexagonal DGGS; that migration is not yet active and `GET
/v1/grid_info` declares the current resolution honestly. The Hilbert-ordered
base-1024 alphabet keeps neighbouring cells string-prefix-similar, so an LLM
that emits `defi.zb493.…` is already at the right place.

## Shape of the protocol

A **fact** is a value at `(cell, band, tslot)` with provenance: which source
(`met.no`, `Copernicus DEM`, `Sentinel-2 L2A`), which derivation function (a
key in `functions-v0.json`), captured-at and signed-at timestamps. The
canonical CBOR of the fact, hashed with blake3, is its CID — a 26-character
base32 string. Two responders that ran the same derivation against the same
upstream produce the same CID byte-for-byte.

A **receipt** wraps a primitive call: it lists the cells, the fact CIDs, the
responder pubkey, and an ed25519 signature over a stable preimage. Receipts
verify without calling back to the issuer. The pubkey is published at
`/.well-known/emem.json` and at `/v1/agent_card`.

**Lazy materialisation** means a band is fetched the first time anyone asks
for it. `POST /v1/recall` on a cold cell fans out to the connector for that
band, decodes the upstream sample (vsicurl Range reads against open COGs,
JSON forecasts, STAC searches), wraps the value as a fact, signs it,
persists it, and returns it. The CID is computed before signing, so the
materialised fact is identical to one a peer would have produced.

## Surface map

The wire surface is one router with two entry points. REST (axum, OpenAPI 3.1)
and MCP (JSON-RPC over Streamable HTTP, registered tool list) read from the
same handlers.

```
REST   79 routes  (67 under /v1/*, plus /health, /openapi.json,
                  /.well-known/{emem,agent,mcp,ai-plugin}.json,
                  /mcp, /metrics)
MCP    34 tools   (10 read primitives, 4 physics solvers, 14 introspection,
                  2 imagery, 1 backfill, 1 verify, 1 fetch, 1 ask, 1 intent)
```

Discover at `GET /v1/discover`, `GET /v1/agent_card`, or `GET /openapi.json`.
The MCP transport is `POST /mcp` (JSON-RPC 2.0). A walked tour of the agent
loop lives at [https://emem.dev/agents.md](https://emem.dev/agents.md).

For humans (and AI agents that want to watch what humans do):
[https://emem.dev/humans](https://emem.dev/humans) — an interactive
constellation of the corpus where every attested cell is a star, every
fact carries a clickable signed receipt, and a console pane prints every
`/v1/*` call the page makes so an LLM scraping the rendered DOM learns
the agent API by observation. Receipts verify in-browser via Ed25519 +
BLAKE3, no server roundtrip.

## Repo layout

```
emem/
├── Cargo.toml                  # workspace root, version 0.0.4, MSRV 1.88
├── crates/                     # 14 workspace crates
│   ├── emem-core/              # bands, algorithms, functions, sources,
│   │                           # topics, schema, manifest, taxonomy, tslot
│   ├── emem-codec/             # cell64, cid64, vec64, hilbert, geo, alphabet
│   ├── emem-fact/              # canonical CBOR, fact + receipt + attestation
│   ├── emem-claim/             # claim predicates (Op enum)
│   ├── emem-cache/             # sled cache wrapper
│   ├── emem-fetch/             # 15 open-data connectors (cog, hansen, jrc,
│   │                           # esa worldcover, overture, firms, …)
│   ├── emem-storage/           # sled hot cache + append-only merkle log
│   ├── emem-cubes/             # 1792-D voxel cube handle (offsets in
│   │                           # bands-v0.json, content-addressed)
│   ├── emem-primitives/        # recall, find_similar, trajectory,
│   │                           # compare, compare_bands, diff, verify,
│   │                           # query_region
│   ├── emem-attest/             # merkle root over fact CIDs
│   ├── emem-intent/             # rule-based intent → plan planner
│   ├── emem-mcp/                # MCP tool registry
│   ├── emem-api-rest/           # axum router (the bulk of the code lives
│   │                           # here: 23k-line lib.rs + physics solvers)
│   └── emem-cli/                # binaries: emem-server, emem-livedemo,
│                               # emem-realdemo, emem-demo, emem-ask-eval
├── python/                      # FastAPI sidecar (Prithvi-EO-2.0, Galileo,
│                               # JEPA-v2 dynamics) over UDS
├── examples/                    # MCP configs + LangChain/LlamaIndex
├── ops/                         # systemd units, journald retention
├── scripts/                     # redeploy, install-topic-model, global_trial
└── web/                         # SSR HTML + llms.txt + agents.md
```

## Conformance gates

Every receipt pins four content-addressed registries: `bands_cid`,
`algorithms_cid`, `sources_cid`, `schema_cid`. They are the manifest CIDs of
the 1792-D band layout (34 bands), the 107 named algorithm recipes
(`flood_risk@2`, `walkability_score@1`, `heat_index@2`, …), the source
catalogue (Copernicus DEM, JRC GSW, Hansen GFC, ESA WorldCover, OSM, met.no,
Open-Meteo, Tessera, …), and the wire schema. A peer that recomputes a fact
under matching CIDs produces the same bytes; a peer with drifted registries
returns a different `bands_cid` in `/health` and the divergence is visible
before any data flows.

## What works in 0.0.4

- Read primitives: `recall`, `recall_many`, `recall_polygon`, `find_similar`,
  `compare`, `compare_bands`, `trajectory`, `diff`, `query_region`, `verify`.
- Physics solvers: 1-D wave, 2-D heat, JEPA-v2 dynamics (CPU + Python sidecar
  with CUDA when `EMEM_SIDECAR_SOCK` points at a live UDS).
- Foundation embeddings: `geotessera` live as 8 annual vintages 2017–2024
  (each 128-D), plus `geotessera.bin128` (sign-bit) and
  `geotessera.multi_year` (1024-D = 8×128 stacked); Prithvi-EO-2.0-300M-TL
  and Galileo through the sidecar.
- Lazy materialisation: cold-cell recall fans out to the connector for the
  band, signs, persists. Gated by `EMEM_AUTO_MATERIALIZE`.
- Receipts: ed25519 over a stable preimage, identity persisted at
  `<EMEM_DATA>/identity.secret.b32`. Verified offline by `verify_receipt`.
- Discovery: `/v1/discover`, `/v1/agent_card`, `/openapi.json`,
  `/.well-known/{emem,agent,mcp,ai-plugin}.json`.
- TLS termination: in-process rustls + Let's Encrypt ACME via TLS-ALPN-01.
  No Cloudflare, no reverse proxy.

## Deferred — not yet shipped

- zkML proofs. Receipts today are signed, not zero-knowledge.
- Trained JEPA-v2 dynamics head. Upstream Tessera now ships 8 vintages
  (2017–2024), but only the showcase cells have all 8 attested on the
  responder. Training the dynamics head needs the multi-year stack
  materialised across a wider candidate pool — backfill is the
  unblocker. Until then, `/v1/jepa_predict_v2` returns the residual
  identity baseline with an `untrained_baseline` warning on the receipt.
- Polished Python and TypeScript SDKs. `sdks/emem-py` and `sdks/emem-ts`
  exist as empty placeholders. Use REST or MCP directly until they ship;
  `examples/langchain.py` and `examples/llamaindex.py` are the working
  reference clients.

## Pointers

- Agent loop: [https://emem.dev/agents.md](https://emem.dev/agents.md)
- Wire spec: [https://emem.dev/spec.md](https://emem.dev/spec.md)
- llms.txt: [https://emem.dev/llms.txt](https://emem.dev/llms.txt)
- OpenAPI 3.1: [https://emem.dev/openapi.json](https://emem.dev/openapi.json)
- MCP endpoint: `https://emem.dev/mcp`
- Container: `ghcr.io/vortx-ai/emem:latest` (multi-arch, anonymously pullable)
- HF Space: [huggingface.co/spaces/vortx-ai/emem](https://huggingface.co/spaces/vortx-ai/emem)
- Issues + PRs: [github.com/Vortx-AI/emem/issues](https://github.com/Vortx-AI/emem/issues)
- Security disclosure: [SECURITY.md](SECURITY.md) — `avijeet@vortx.ai`

## License

Apache-2.0 — see [LICENSE](LICENSE) and [NOTICE](NOTICE). All default-build
data sources are open: Copernicus DEM, JRC GSW (CC-BY 4.0), Hansen GFC, ESA
WorldCover (CC-BY 4.0), OSM (ODbL), met.no, Open-Meteo, Tessera. No API
keys, no operator credentials, no SaaS lock-in.
