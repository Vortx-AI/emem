# PR text for `modelcontextprotocol/servers`

The official MCP server catalog (`https://github.com/modelcontextprotocol/servers`)
is hand-curated. To get emem listed, open a PR against
`https://github.com/modelcontextprotocol/servers/blob/main/README.md`
adding the line below under **Earth & Geospatial** (create the section
if it does not yet exist — sort it alphabetically among the other
domain sections).

## Markdown to add (single line)

```markdown
- **[emem](https://emem.dev)** ([repo](https://github.com/Vortx-AI/emem)) — Cite-able, content-addressed, signed Earth-memory protocol. 36 MCP tools, 35 live materializable Earth-observation bands (Sentinel-1/2, MODIS, ERA5, CAMS, JRC GSW, Hansen GFC, ESA WorldCover, SoilGrids, Open-Meteo, MET Norway), no API key, ed25519 receipts. Streamable-HTTP at `https://emem.dev/mcp`. By **[Vortx-AI](https://github.com/Vortx-AI)**.
```

## PR title

```
docs: add emem (Earth memory protocol) under Earth & Geospatial
```

## PR body

```markdown
## Server

**Name:** emem — Earth memory for AI agents
**URL:** https://emem.dev
**Repo:** https://github.com/Vortx-AI/emem
**License:** Apache-2.0
**Operator:** Vortx AI Private Limited (India)
**Registry name:** `io.github.Vortx-AI/emem` (in the official
`registry.modelcontextprotocol.io`)
**MCP version:** 2025-03-26 (Streamable HTTP, JSON-RPC 2.0)
**Authentication:** none (anonymous reads)

## What it does

Cite-able, content-addressed, signed Earth-memory protocol. Every
answer about a place on Earth is a CBOR-canonical fact addressed by
its blake3 CID and signed with the responder's ed25519 key. Receipts
verify offline against the public key at `/.well-known/emem.json` —
no callback to the server needed.

36 MCP tools. 35 materializable Earth-observation bands wired today
(Sentinel-1/2, MODIS LST/NDVI/ET/GPP/LAI, NASA POWER, ERA5, CAMS air
quality, JRC Global Surface Water, Hansen Global Forest Change, ESA
WorldCover, SoilGrids 2.0, Copernicus DEM, GMRT bathymetry, Open-Meteo,
MET Norway). Live count at `https://emem.dev/v1/agent_card`
(`band_taxonomy.materializer_wired`).

## Why an LLM would pick this

- "What is the air quality / elevation / NDVI / weather / soil at place X?"
  → `emem_locate(q)` then `emem_recall(cell, bands)` returns a signed
  fact with `fact_cid` the agent quotes.
- "How has the Sundarbans / Amazon / [region] changed over time?"
  → `emem_diff(cell, t1, t2, band)` returns a signed Derivative.
- "Find places like [X]" → `emem_find_similar(key:cell, band:"geotessera")`
  returns k similar cells with cosine scores.
- Polygon questions like "average PM2.5 across Lagos" →
  `emem_recall_polygon(place, bands)` fans out to per-cell facts and
  aggregates with mean/median/min/max/std + an SVG overlay URL.

Every numeric answer ships with `responder_pubkey_b32`, `signature`,
`fact_cid`, `schema_cid`, `data_resolution_m`, and `source` — what an
LLM needs to give the user a confident, cite-able answer.

## Categories / keywords

`data`, `geospatial`, `earth-observation`, `satellite`, `carbon-mrv`,
`ed25519`, `no-api-key`

## Status

Live, version 0.0.6 (May 2026). CI green
(`https://github.com/Vortx-AI/emem/actions`). Container at
`ghcr.io/vortx-ai/emem:latest`. HuggingFace Space mirror at
`https://vortx-ai-emem.hf.space`.

## Links

- Live: https://emem.dev — `GET /health`, `POST /v1/recall`,
  `POST /mcp`
- MCP discover: `GET https://emem.dev/mcp`
- OpenAPI 3.1: https://emem.dev/openapi.json (full)
  / https://emem.dev/openapi.action.json (28-op subset for
  OpenAI Custom GPT Actions)
- Agent card (A2A): https://emem.dev/.well-known/agent-card.json
- llms.txt: https://emem.dev/llms.txt
- /llms-full.txt: https://emem.dev/llms-full.txt
- Repo + issues: https://github.com/Vortx-AI/emem
```

## Submission checklist (parallel marketplaces)

- [ ] **Official registry** (`registry.modelcontextprotocol.io`) —
  bump from v0.0.2 (current) to v0.0.6 via the `mcp-publisher` CLI
  and GitHub OAuth on the Vortx-AI org. See
  `https://github.com/modelcontextprotocol/registry/blob/main/docs/guides/publishing/publish-server.md`.
- [ ] **`modelcontextprotocol/servers`** PR — paste the line above
  into the README under Earth & Geospatial.
- [ ] **mcp.so** — open issue at
  `https://github.com/chatmcp/mcp-directory`, attach `server.json`
  + a screenshot of `https://emem.dev/v1/coverage_map.svg`.
- [ ] **Smithery (smithery.ai)** — manual web form at
  `https://smithery.ai/new`. Needs README + repo URL + MCP endpoint.
- [ ] **Glama (glama.ai/mcp/servers)** — auto-indexes from GitHub
  topics. Add `mcp` and `model-context-protocol` topics on the
  `Vortx-AI/emem` repo via GitHub UI; Glama picks up within ~24h.
- [ ] **mcphub.io** / **mcpmarket.com** — same form-driven path as
  Smithery.
