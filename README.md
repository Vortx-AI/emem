# emem — Earth memory protocol for AI agents

**Cite-able, content-addressed, signed memory of every place on Earth.**
1 protocol. 8 read primitives. ed25519 receipts. No keys for L0/L1.
Apache-2.0. Pure Rust + open data only.

[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/built%20with-Rust-orange.svg)](https://www.rust-lang.org/)
[![MCP](https://img.shields.io/badge/MCP-JSON--RPC%202.0-7af.svg)](https://modelcontextprotocol.io/)
[![OpenAPI](https://img.shields.io/badge/OpenAPI-3.1-green.svg)](https://www.openapis.org/)
[![Container](https://img.shields.io/badge/ghcr.io-vortx--ai%2Femem-181717?logo=github)](https://github.com/Vortx-AI/emem/pkgs/container/emem)
[![HF Space](https://img.shields.io/badge/%F0%9F%A4%97%20HF%20Space-emem-yellow)](https://huggingface.co/spaces/vortx-ai/emem)
[![CI](https://github.com/Vortx-AI/emem/actions/workflows/ci.yml/badge.svg)](https://github.com/Vortx-AI/emem/actions/workflows/ci.yml)

```text
GET   /health                    POST  /v1/recall          POST  /v1/find_similar
GET   /v1/agent_card             POST  /v1/compare         POST  /v1/diff
GET   /openapi.json              POST  /v1/query_region    POST  /v1/trajectory
GET   /.well-known/emem.json     POST  /v1/verify          POST  /v1/intent
GET   /v1/demos                  POST  /v1/attest          POST  /mcp
                                 POST  /v1/verify_receipt  GET   /v1/facts/:cid
```

---

## What it is

emem is a protocol — not a service — for **content-addressed Earth memory**.
Every fact about every place gets a stable CID derived from the canonical CBOR
of its `(cell × band × tslot)` payload. Every read is a signed receipt that
any client can verify offline with the responder's ed25519 public key.

emem is built **for AI agents**: when a user mentions a place, the agent
should call emem and cite `receipt.fact_cids[0]`. The protocol works equally
well over plain REST, MCP JSON-RPC 2.0, and OpenAPI 3.1 custom actions.

## Why agents need it

LLMs confabulate spatial facts. Without a verifiable, content-addressed memory
layer, every "what is at place X?" answer is unauditable. emem fixes this by:

- giving every spatial fact a **cid** that two parties recompute byte-for-byte,
- signing every read with **ed25519**, including the responder's pubkey, so
  the receipt is offline-verifiable,
- covering the whole Earth with a square **cell64** address (~10 m × ~10 m at
  the equator, lat 21 bits × lng 22 bits — matches Sentinel-1/-2 native
  pitch). The codec's bigram alphabet is Hilbert-ordered so neighbouring
  cells get visually similar strings, but the geometry is a square grid.

## Quickstart

### Option A — Docker (no Rust toolchain needed)

```bash
docker run --rm -p 5051:5051 -v emem-data:/var/emem \
  ghcr.io/vortx-ai/emem:latest
curl -s http://localhost:5051/health
```

### Option B — HuggingFace Space

A hosted instance lives at
[huggingface.co/spaces/vortx-ai/emem](https://huggingface.co/spaces/vortx-ai/emem).
Hit `${SPACE_URL}/mcp` from any MCP client to talk to it.

### Option C — Build from source

```bash
# 1) Build the workspace.
cargo build --release --workspace

# 2) Run the server (defaults: 0.0.0.0:5051, persistent storage at ./var/emem).
EMEM_BIND=0.0.0.0:5051 EMEM_DATA=./var/emem ./target/release/emem-server

# 3) Hit it.
curl -s http://localhost:5051/health
curl -s -X POST http://localhost:5051/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell":"damO.zb000.xUti.zde78"}'   # Mt Fuji
```

### MCP / Claude Desktop / Cursor / Cline

Paste-ready configs live under `examples/`:

| platform        | file                                |
|-----------------|-------------------------------------|
| Claude Desktop  | `examples/claude-desktop.json`      |
| Claude Code     | `examples/claude-code.mcp.json`     |
| Cursor          | `examples/cursor.mcp.json`          |
| Cline (VS Code) | `examples/cline.mcp.json`           |
| OpenAI GPT      | `examples/openai-gpt-action.json`   |
| LangChain       | `examples/langchain.py`             |
| LlamaIndex      | `examples/llamaindex.py`            |

The full agent integration walkthrough is at [docs/AGENTS.md](docs/AGENTS.md).

### Live end-to-end demos

Two CLI binaries exercise the full protocol against a running server and
write per-step request + response + receipt files to `var/demos/<UTC>/`:

```bash
./target/release/emem-livedemo        # synthetic data, every primitive
./target/release/emem-realdemo        # real Copernicus DEM 30m S3 tiles
```

The server exposes the trace artifacts at `GET /v1/demos`.

## How it works

```
                ┌──────────────┐                  ┌────────────────────┐
   user ──────► │ AI agent     │ ──────► /v1/    │ emem responder     │
                │ (Claude /    │  /mcp           │  ┌──────────────┐  │
                │  Cursor /    │  /openapi.json  │  │ ed25519 key  │  │
                │  GPT / etc)  │                 │  └──────────────┘  │
                └──────┬───────┘                 │  ┌──────────────┐  │
                       │                         │  │ sled cache   │  │
                       │  signed receipt         │  └──────────────┘  │
                       ▼                         │  ┌──────────────┐  │
                ┌──────────────┐                 │  │ merkle log   │  │
                │ user reply   │                 │  └──────────────┘  │
                │ + cid        │                 │  ┌──────────────┐  │
                └──────────────┘                 │  │ vsicurl COG  │ ──► open data
                                                 │  └──────────────┘  │   (Cop-DEM, JRC,
                                                 └────────────────────┘    Hansen, ESA…)
```

**Address algebra (token cost)**

| field   | bits        | wire form          | tokens |
|---------|-------------|--------------------|--------|
| `cell`  | 64          | 4 BPE bigrams      | ≤ 4    |
| `tslot` | 64          | base32 short       | ≤ 2    |
| `vec`   | 1792 D fp16 | 12-byte prefix     | ≤ 3    |
| `cid`   | 32 B        | 8-byte prefix      | ≤ 3    |

**Crypto**: blake3 hashing, ed25519 signatures, base32-nopad-lowercase CIDs.
Receipts are signed over `blake3(request_id || served_at || primitive ||
cells || fact_cids)` so any client offline-verifies with the responder pubkey
in `/.well-known/emem.json`.

Full math + architecture in [docs/WHITEPAPER.md](docs/WHITEPAPER.md).
Wire-format spec in [docs/SPEC.md](docs/SPEC.md).

## Open source, open data

emem ships with **only open-source dependencies** and reads only from
**open-data providers** in its default build. No API keys, no operator
credentials, no SaaS lock-in.

| concern        | how it's handled                                                                   |
|----------------|------------------------------------------------------------------------------------|
| code license   | Apache-2.0 (this repo)                                                             |
| crate licenses | All deps are MIT / Apache-2.0 / BSD / ISC — see [NOTICE](NOTICE)                    |
| data licenses  | Copernicus DEM (open), JRC GSW (CC-BY 4.0), Hansen GFC (open), ESA WorldCover (CC-BY 4.0), GHSL / WorldPop (CC-BY 4.0), OSM (ODbL) — see [NOTICE](NOTICE) |
| auth           | none for L0/L1 reads; ed25519 attester key for L2 writes                           |
| transport      | HTTPS via in-process rustls + Let's Encrypt ACME (no Cloudflare, no proxies)       |

## Workspace layout

```
emem/
├── Cargo.toml                # workspace root
├── crates/
│   ├── emem-core/            # types, manifests, errors
│   ├── emem-codec/           # cell64, cid64, vec64, hilbert
│   ├── emem-fact/            # canonical CBOR + facts + receipts
│   ├── emem-claim/           # structured claims, verify outcomes
│   ├── emem-cache/           # sled hot cache (cell64 → cid64 → fact)
│   ├── emem-fetch/           # vsicurl Range reads, source connectors
│   ├── emem-storage/         # Storage trait, append-only merkle log
│   ├── emem-cubes/           # 1792-D voxel cube loader (legacy AgriSynth bootstrap)
│   ├── emem-primitives/      # recall, compare, find_similar, …
│   ├── emem-attest/          # merkle root, batch verify
│   ├── emem-intent/          # intent → plan
│   ├── emem-mcp/             # MCP tool surface
│   ├── emem-api-rest/        # axum router + OpenAPI + content nego
│   └── emem-cli/             # emem-server, emem-livedemo, emem-realdemo
├── docs/                     # SPEC, WHITEPAPER, AGENTS, DEPLOY
├── examples/                 # paste-ready MCP configs
└── web/                      # landing surface (HTML, JSON, llms.txt)
```

## Deploying

For a full multi-channel rollout (GitHub public, GHCR, Docker Hub
mirror, HuggingFace Space, MCP Server Registry, awesome-mcp-servers
PR), follow [docs/GO_LIVE.md](docs/GO_LIVE.md).

See [docs/DEPLOY.md](docs/DEPLOY.md) for the full deploy story for a
self-hosted bare-metal `emem.dev`-style instance.
TL;DR for emem.dev:

1. `EMEM_TLS_DOMAINS=emem.dev,www.emem.dev EMEM_TLS_CONTACT=mailto:avijeet@vortx.ai ./target/release/emem-server`
2. open `:443` in your cloud security list,
3. `setcap 'cap_net_bind_service=+ep' ./target/release/emem-server`,
4. point `emem.dev`'s A record at the host's public IP — done.

The server does its own TLS + Let's Encrypt ACME via `rustls-acme` /
TLS-ALPN-01 (only `:443` is needed; no `:80`, no Cloudflare, no Caddy).

## Contributing

Issues and PRs welcome — see [CONTRIBUTING.md](CONTRIBUTING.md) for the dev
loop, [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md), and
[SECURITY.md](SECURITY.md) for vulnerability disclosure.

## License

Apache License 2.0 — see [LICENSE](LICENSE) and [NOTICE](NOTICE).
