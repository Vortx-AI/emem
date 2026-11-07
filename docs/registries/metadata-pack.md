# The registry metadata pack

Use this page when you submit emem to an MCP directory or update a listing.
It holds one set of facts, so every listing says the same thing.

The counts below come from the live responder on 7 November 2026.
They change with each release. Before a new submission, check them again:

```sh
python3 scripts/sync_counts.py --check
```

## Directories that list emem

We checked the five listings below on 7 November 2026.

| Directory | Entry | How it stays current |
|---|---|---|
| Official MCP Registry | `io.github.Vortx-AI/emem`, latest version 2.4.4 | `.github/workflows/mcp-publish.yml` publishes `server.json` on every `v*` tag. It logs in with GitHub OIDC. |
| Server card | `https://emem.dev/.well-known/mcp.json`, 116 tools | The responder builds the card from the same tool list as `tools/list` (`emem_mcp::TOOLS`). |
| Smithery | `vortxai/emem`, an external listing of `https://emem.dev/mcp` | `.github/workflows/publish-smithery.yml` republishes on each GitHub release. It skips the step when the repository has no `SMITHERY_API_KEY` secret. |
| Glama | `https://glama.ai/mcp/servers/Vortx-AI/emem` | Glama indexes GitHub and the official registry. |
| punkpeye/awesome-mcp-servers | One line, `Vortx-AI/emem`, with the Glama score badge | Manual. Edit the line with a pull request to that repository. |

On 7 November 2026, the Smithery registry entry listed then-16 tools. The live core has 18.
It shows the tool list that Smithery last scanned.

Before you tag a release, set the `version` in `server.json` to the workspace version in `Cargo.toml`.
The publish workflow copies the version from `server.json` without a change.

## Canonical facts

Use these facts in every listing.
The live values are from 7 November 2026.

| Fact | Value | Where to check |
|---|---|---|
| Name | emem | `server.json` `name` |
| Publisher | Vortx AI | `server.json` `x-emem.operator` |
| Registry namespace | `io.github.Vortx-AI/emem` | `server.json` |
| Version | 2.4.4 | `Cargo.toml`, MCP `initialize` `serverInfo.version` |
| Endpoint | `https://emem.dev/mcp`, MCP Streamable HTTP, protocol `2025-11-25` | MCP `initialize` |
| Tools on `/mcp` | 18 core tools on one page, with no `nextCursor` | `tools/list` on `/mcp` |
| Tools on `/mcp/full` | All 116 tools (18 core, 98 extended) over 9 pages | `tools/list` on `/mcp/full`, follow `nextCursor` |
| Tool calls | `tools/call` runs any of the 116 tools by name, on either endpoint | `tools/call` on `/mcp` |
| Resources | 21 static resources and 9 URI templates | `resources/list`, `resources/templates/list` |
| REST paths | 183 paths under `/v1/`, 194 paths in total | `GET /openapi.json` |
| Wired measurements | 115 band names that a materializer can fetch on a miss | `GET /v1/agent_card`, `band_taxonomy.materializer_wired` |
| Reads | Open. No key, no account. | Any read route |
| Writes | Signed with the writer's Ed25519 key. Some write surfaces also need a trust tier. | `GET /v1/enlist` |
| Repository | `https://github.com/Vortx-AI/emem`, Apache-2.0 | `LICENSE` |
| Homepage | `https://emem.dev` | `server.json` `websiteUrl` |
| Container | `ghcr.io/vortx-ai/emem:latest` (head of `main`) | `.github/workflows/publish.yml` |

Lead with this claim: every read returns a signed receipt that verifies offline.
A memory token resolves to the same signed bytes for every reader.
Earth observation is the first data source, not the main message.

## The one-line description

Use this text as it is.
Change only the format that a directory needs, such as the link syntax.

> [emem](https://emem.dev) ([repo](https://github.com/Vortx-AI/emem)):
> shared, verifiable memory of the physical world for AI agents. Every
> place resolves to one signed, content-addressed record. Every read
> returns an Ed25519 receipt that verifies offline. 116 MCP tools, 115
> wired measurements (Sentinel-1/2, MODIS, ERA5, JRC GSW, Hansen GFC,
> SoilGrids, Open-Meteo), no API key to read.
> Streamable HTTP at `https://emem.dev/mcp`.

The file `awesome-mcp-servers-emem.patch` in this directory carries counts from April 2026.
Do not use it again as it is.

## Directories not confirmed

On 7 November 2026, these directories had no emem page at the URLs that we tried.
A listing can exist at a different URL. Search the directory before you submit.

| Directory | How to submit | Status on 7 November 2026 |
|---|---|---|
| mcp.so | The submit form, or an issue on `chatmcp/mcp-directory` | Not confirmed |
| PulseMCP | The submission form at pulsemcp.com | Not confirmed |

When a directory accepts a submission, add the listing URL to the first table.
The next person who updates the listing can then find it.
