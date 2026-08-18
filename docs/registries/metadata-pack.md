# The registry metadata pack

One canonical block of facts for every directory submission, so the
copy is written once and stays consistent. Counts below were verified
against the live responder on 2026-07-16 (`python3
scripts/sync_counts.py`); re-verify before any new submission, the
numbers move.

## Where emem already is

- **The official MCP Registry**: published and current.
  `io.github.Vortx-AI/emem`, current at the latest released version. Publishing
  is automated: `.github/workflows/mcp-publish.yml` pushes `server.json`
  on every `v*` tag over GitHub OIDC, no human in the loop. Keep
  `server.json`'s version in step with the workspace version before
  tagging.
- **The server card**: `https://emem.dev/.well-known/mcp.json`, served
  live and generated from the same tool registry as `tools/list`, so it
  cannot drift from the surface it describes.

## Canonical facts (2026-07-16)

- Name: emem. Publisher: Vortx AI. Namespace: `io.github.Vortx-AI/emem`.
- Endpoint: `https://emem.dev/mcp`, MCP Streamable HTTP (2025-03-26).
  `/mcp` advertises the 14-tool core loop; `/mcp/full` lists all 102;
  `tools/call` dispatches every tool by name at either endpoint.
- 108 MCP tools (16 core, 92 extended), 20 static resources + 9 URI
  templates, 114 REST paths under `/v1/*`.
- Auth posture: reads are open, no key, no account; writes are
  authenticated by an ed25519 signature on the request body.
- The claim to lead with: every read returns a signed receipt that
  verifies offline, and a Memory Token resolves to byte-identical
  signed bytes for anyone. Satellites are the first substrate, not the
  pitch.
- Repo: https://github.com/Vortx-AI/emem (Apache-2.0). Homepage:
  https://emem.dev. Container: `ghcr.io/vortx-ai/emem:latest`.

## The one-liner (use verbatim, adjust only per directory format)

> [emem](https://emem.dev) ([repo](https://github.com/Vortx-AI/emem)):
> shared, verifiable memory of the physical world for AI agents. Every
> place resolves to one signed, content-addressed record; every read
> returns an ed25519 receipt that verifies offline. 108 MCP tools, 129
> wired Earth-observation measurements (Sentinel-1/2, MODIS, ERA5,
> JRC GSW, Hansen GFC, SoilGrids, Open-Meteo), no API key to read.
> Streamable HTTP at `https://emem.dev/mcp`.

## Directories still to submit (each needs a human account or PR)

| Directory | How | Status |
|---|---|---|
| punkpeye/awesome-mcp-servers | PR against their README (base the line on the one-liner above; the old patch in this directory carries April counts, do not reuse it verbatim) | open |
| mcp.so | submit form / GitHub issue on chatmcp/mcp-directory | open |
| Smithery | account + server registration at smithery.ai | open |
| PulseMCP | submission form at pulsemcp.com | open |
| glama.ai | auto-indexes GitHub + the official registry; verify the entry exists and claim it | verify |

When a submission lands, record the listing URL here so the next
update knows where to go.
