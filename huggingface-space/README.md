---
title: emem — Earth memory protocol (MCP server)
emoji: 🌍
colorFrom: blue
colorTo: green
sdk: docker
app_port: 5051
pinned: true
license: apache-2.0
short_description: 'Verifiable memory for AI agents, checkable offline'
tags:
  - mcp
  - geospatial
  - earth-observation
  - ai-agents
  - rust
  - sentinel-2
  - sentinel-1
  - copernicus
  - openstreetmap
  - claude
  - openai
models: []
datasets: []
---

# emem: verifiable memory for AI agents

This Space hosts the **emem MCP server**: a shared, verifiable memory of the
physical world. Every place resolves to one signed, content-addressed fact an
agent can cite as an 84-character token, and anyone can re-check the ed25519
receipt offline. Connect from Claude Desktop, Cursor, Cline, or any
MCP-compatible agent.

## What this Space gives you

- A live MCP JSON-RPC 2.0 endpoint at `${SPACE_URL}/mcp`.
- A REST + OpenAPI 3.1 surface at `${SPACE_URL}/v1/...`.
- The same tool surface as [emem.dev](https://emem.dev): 94 MCP tools,
  124 wired Earth-observation measurements across 43 cube slots, and 162
  composition algorithms (live counts at `/v1/agent_card`).
- Multimodal MCP content blocks: true-colour Sentinel-2 RGB scenes,
  GeoJSON cell polygons, live SVG coverage maps.
- No keys for L0/L1 reads. Apache-2.0. Pure Rust.

## How to connect

### Claude Desktop

Add this to `~/Library/Application Support/Claude/claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "emem-hf": {
      "type": "http",
      "url": "https://YOUR-SPACE.hf.space/mcp"
    }
  }
}
```

(Replace `YOUR-SPACE` with the real hostname this Space is published on.)

### Cursor / Cline

See the paste-ready configs in the
[examples directory](https://github.com/Vortx-AI/emem/tree/main/examples)
of the upstream repo — swap the `url` field to point at this Space.

### curl

```bash
curl -s https://YOUR-SPACE.hf.space/health
curl -s https://YOUR-SPACE.hf.space/v1/agent_card
curl -s https://YOUR-SPACE.hf.space/v1/locate \
  -H 'content-type: application/json' \
  -d '{"q":"Mt Fuji"}'
```

## Endpoints

| path                              | purpose                                                     |
|-----------------------------------|-------------------------------------------------------------|
| `GET /health`                     | Liveness                                                    |
| `GET /v1/agent_card`              | Capability advertisement for AI agents                      |
| `GET /openapi.json`               | OpenAPI 3.1 spec                                            |
| `GET /.well-known/emem.json`      | Responder pubkey + manifests for offline receipt verification |
| `POST /mcp`                       | MCP JSON-RPC 2.0                                            |
| `POST /v1/recall`                 | Recall facts at a cell × bands                              |
| `POST /v1/locate`                 | Geocode → cell64                                            |
| `POST /v1/find_similar`           | Embedding-space neighbour search                            |
| `POST /v1/intent`                 | Free-text question → plan                                   |
| `POST /v1/algorithms`             | Browse the 162-recipe registry                              |
| `GET /v1/cells/:cell/scene.png`   | Sentinel-2 L2A 256×256 RGB thumbnail                        |
| `GET /v1/coverage_map.svg`        | Live world map of attested cells                            |

## Privacy & verification

Every read returns a signed receipt with the responder's ed25519 public key,
the request canonicalisation hash, and the fact CIDs — verifiable offline by
any client. The pubkey is at `/.well-known/emem.json`.

## Source code

[github.com/Vortx-AI/emem](https://github.com/Vortx-AI/emem) — Apache-2.0.
