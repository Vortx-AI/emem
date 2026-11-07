# Semantic Kernel agent with emem over MCP

This example connects a Microsoft Semantic Kernel kernel to the emem MCP
server. The model finds a place, reads a signed fact there, and checks the
receipt of that fact.

## What you need

- Python 3.10 or later. We tested with Python 3.12.
- An OpenAI API key for the model. emem reads need no key.

## What you get

The model runs three steps for South Mumbai and prints each one: locate,
recall the elevation, and verify the receipt.

## Install

```bash
pip install semantic-kernel
```

## Run

Run these commands from the root of the repository.

```bash
export OPENAI_API_KEY="sk-..."
python examples/semantic-kernel/emem_mcp_geospatial_agent.py
```

To use a different emem node, set `EMEM_MCP_URL`. The default is
`https://emem.dev/mcp`.

## What the script does

1. It connects to `https://emem.dev/mcp` over Streamable HTTP with
   `MCPStreamableHttpPlugin`. The plugin loads the emem tools. You do not
   register them by hand.
2. It removes `emem_echo_verify` from the plugin. Semantic Kernel cannot load
   that tool, because one parameter has two JSON Schema types
   (`["string", "number"]`). The script does not need it.
3. It adds the plugin to a kernel with the `gpt-4o` model and automatic
   function calls.
4. It asks the model to locate South Mumbai, recall its elevation
   (`copdem30m.elevation_mean`), and verify the receipt.

## Test status

On 7 November 2026 we installed `semantic-kernel` 1.47.0 (with mcp 1.30.0).
The script loaded the emem tools and stopped at the OpenAI call, because we
used a test key. We did not run the model part. Without step 2, the script
stopped at `kernel.add_plugin` with a pydantic `ValidationError`.

We also made the emem calls that the model needs, on the same day:

| call | result on 7 November 2026 |
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

- Any Semantic Kernel chat service can drive the model. Change
  `OpenAIChatCompletion` to Azure OpenAI or to the service for another
  provider.
- `/mcp` lists 18 core tools. `https://emem.dev/mcp/full` lists all tools in
  pages. On 7 November 2026 it had 116 tools in 9 pages, and 8 pages earlier
  that day. The server cuts a page at about 45,000 bytes, so the page count
  and the tools on each page change when the server changes. The plugin loaded
  only the first page (12 tools with 9 pages, 13 with 8). That page did not
  have `emem_recall` or `emem_verify_receipt`. Keep `/mcp` for this script.
