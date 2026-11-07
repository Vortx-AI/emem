# AutoGen agent with emem over MCP

This example connects a Microsoft AutoGen `AssistantAgent` to the emem MCP
server. The agent finds a place, reads a signed fact there, and checks the
receipt of that fact.

## What you need

- Python 3.10 or later. We tested with Python 3.12.
- An OpenAI API key for the model. emem reads need no key.

## What you get

The agent runs three steps for South Mumbai and prints each one: locate,
recall the elevation, and verify the receipt. The output streams to the
console.

## Install

```bash
pip install autogen-agentchat 'autogen-ext[openai,mcp]' 'mcp<2'
```

Keep `mcp<2`. On 6 November 2026, `autogen-ext` 0.7.5 did not import with
`mcp` 2.3.0 (`cannot import name 'RequestContext'`).

## Run

```bash
export OPENAI_API_KEY="sk-..."
python emem_mcp_geospatial_agent.py
```

To use a different emem node, set `EMEM_MCP_URL`. The default is
`https://emem.dev/mcp`.

## What the script does

1. It connects to `https://emem.dev/mcp` over Streamable HTTP with
   `McpWorkbench` and `StreamableHttpServerParams`.
2. It loads the 18 core emem tools that `/mcp` lists.
3. It makes an `AssistantAgent` with the `gpt-4.1-mini` model and a system
   message that lists the three steps.
4. It asks the agent to locate South Mumbai, recall its elevation
   (`copdem30m.elevation_mean`), and verify the receipt.

## Test status

On 6 November 2026 we installed the packages above (autogen-agentchat 0.7.5,
autogen-ext 0.7.5, mcp 1.30.0). The script loaded the emem tools and stopped
at the OpenAI call, because we used a test key. We did not run the model part.

We also made the emem calls that the agent needs, on the same day:

| call | result on 6 November 2026 |
|---|---|
| `emem_locate` with `South Mumbai` | cell `defi.zb4d7.zb8ec.zf21e`, "South Mumbai (precinct of the city of Mumbai)" |
| `emem_recall` on that cell, band `copdem30m.elevation_mean` | 6.56 m |
| `POST /v1/verify_receipt` with that receipt | `signature_valid: true`, `merkle_proof_valid: true` |
| `emem_recall` with the place name | error `no_geocoder_match`: the name is ambiguous |

If the model sends the place name to `emem_recall`, emem refuses to guess.
The model must use the cell from `emem_locate`.

`emem_verify_receipt` is the responder checking its own signature. It is not
an independent check. For an independent check, verify the receipt offline.
See [Verify the receipt](../../docs/quickstart.md#verify-the-receipt).

## Notes

- Any AutoGen model client can drive the agent. Change
  `OpenAIChatCompletionClient` to the client for another provider.
- `/mcp` lists 18 core tools. `https://emem.dev/mcp/full` lists all tools in
  pages. On 7 November 2026 it had 116 tools in 9 pages, and `McpWorkbench`
  loaded only the first page (12 tools). That page did not have
  `emem_recall` or `emem_verify_receipt`. Keep `/mcp` for this script.
