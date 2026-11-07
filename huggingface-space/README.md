---
title: emem, shared verifiable memory (MCP server)
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
  - mcp-server
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

# emem node on Hugging Face Spaces

This folder holds the files for the emem Space on Hugging Face. The Space runs
`emem-server`, the same server program as [emem.dev](https://emem.dev), as a
second public node. Its address is `https://vortx-ai-emem.hf.space`.

Use it when you want to try emem on a node other than emem.dev. Use it also
when you want an example of how to run your own node from the published image.
Most agents should use `https://emem.dev/mcp`. That node runs the newest release.

Try it. This call needs no key:

```bash
curl -s https://vortx-ai-emem.hf.space/health
```

The reply has `"ok":true` and the node version. Read more in
[docs/agents.md](https://github.com/Vortx-AI/emem/blob/main/docs/agents.md) (how to use emem from an agent) and
[docs/operators/operating.md](https://github.com/Vortx-AI/emem/blob/main/docs/operators/operating.md) (how to run a node).

## How the Space differs from emem.dev

The two nodes run the same program, but they are separate nodes. On
7 November 2026 the live values were these:

| | Space | emem.dev |
|---|---|---|
| Version | 2.4.2 | 2.4.4 |
| Key that signs facts (`responder_pubkey_b32`) | `2hi4tnjizc2zl5e4h6hdwkmxphh2fbax3iu6jpmjypf2y4dsgidq` | `777er3yihgifqmv5hmc2wwmyszgddzderzhsx6rex4yoakwomvka` |
| MCP tools (`band_taxonomy.tools.count` in `/v1/agent_card`) | 115 | 116 |
| Composition algorithms | 168 | 172 |
| Wired measurements (`materializer_wired`) | 127 | 115 |

The Space runs on the Hugging Face `cpu-basic` hardware tier.

Each node signs facts with its own key. To check a fact from the Space, use
the key from the Space's own `/.well-known/emem.json`, at
`federation.node.responder_pubkey_b32`. The key of emem.dev does not verify it.

The Space keeps its own small corpus. On 7 November 2026, `/health` reported
107 cells and 25 bands. A cell that emem.dev already holds can be empty on the
Space. The Space then fetches the value from the open source when you ask.

The `Dockerfile` pins the image by digest, not by tag. The Space version
changes only when someone changes that digest and pushes the Space. The
comment in the `Dockerfile` gives the command that shows the newest digest.

## Read a signed fact from the Space

Find the cell for a place:

```bash
curl -s https://vortx-ai-emem.hf.space/v1/locate \
  -H 'content-type: application/json' \
  -d '{"q":"Mt Fuji"}'
```

On 7 November 2026 the reply had `"cell64":"defi.zb592.nemu.zEvO"`. Read the
elevation there:

```bash
curl -s https://vortx-ai-emem.hf.space/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb592.nemu.zEvO","bands":["elevation"]}'
```

The reply had one fact, `copdem30m.elevation_mean`, with the value
3766.73486328125 m. The `receipt` holds the fact id, the Ed25519 signature and
the Space key. The first call on a cell that the Space does not hold can take
longer, because the Space fetches the source data first.

## Connect an MCP client

The MCP endpoint is `https://vortx-ai-emem.hf.space/mcp` (Streamable HTTP).
Use the client configs in [examples/](https://github.com/Vortx-AI/emem/tree/main/examples) and change the URL from
`https://emem.dev/mcp` to the Space URL.

Claude Desktop does not connect to a remote URL from
`claude_desktop_config.json`. Use one of these two methods:

- In Claude Desktop, open Customize > Connectors, click + Add, choose Add
  custom connector, and enter the Space MCP URL.
- In `claude_desktop_config.json`, run the `mcp-remote` bridge. This needs
  Node.js.

```json
{
  "mcpServers": {
    "emem-hf": {
      "command": "npx",
      "args": ["-y", "mcp-remote", "https://vortx-ai-emem.hf.space/mcp"]
    }
  }
}
```

We did not run either Claude Desktop method against the Space. A plain MCP
`tools/list` call to the Space returned 18 tools on 7 November 2026. That is
the short starter list. The tool `emem_tools` finds the others.

## Endpoints

All of these paths returned HTTP 200 from the Space on 7 November 2026.
`/openapi.json` lists every path.

| Path | What it does |
|---|---|
| `GET /health` | Shows that the node is up, with its version, key and corpus size |
| `GET /v1/agent_card` | Describes the node for agents, with live counts |
| `GET /openapi.json` | Gives the OpenAPI 3.1 description of the REST surface |
| `GET /.well-known/emem.json` | Gives the node key and the manifest digests for offline checks |
| `POST /mcp` | Takes MCP JSON-RPC 2.0 requests |
| `POST /v1/locate` | Changes a place name into a `cell64` |
| `POST /v1/recall` | Reads signed facts at a cell |
| `POST /v1/find_similar` | Finds cells with a similar stored embedding |
| `GET /v1/algorithms` | Lists the composition algorithms |
| `GET /v1/cells/:cell/scene.png` | Gives a Sentinel-2 true-colour picture of the cell |
| `GET /v1/coverage_map.svg` | Draws a world map of the cells the node holds |

## Verify a receipt

A response from a read primitive, for example `recall`, carries a signed
receipt. The receipt holds the fact ids, the node key and an Ed25519
signature. You can check it offline with the Space key. The
[emem-verify-receipt skill](https://github.com/Vortx-AI/emem/tree/main/plugins/emem/skills/emem-verify-receipt) has a
small Python verifier. `/v1/locate` does not return a receipt.

## Source and licence

The source is at [github.com/Vortx-AI/emem](https://github.com/Vortx-AI/emem),
under Apache-2.0. The official MCP Registry entry `io.github.Vortx-AI/emem`
points to `https://emem.dev/mcp`, not to the Space. On 7 November 2026, the
[emem.dev reference page](https://emem.dev/reference#client-setup) also lists
emem in ChatGPT, the Claude directory and the GitHub MCP Registry. That page has
the install guide for each host.
