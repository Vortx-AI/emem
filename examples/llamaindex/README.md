# LlamaIndex agent with emem over MCP

This example connects a LlamaIndex `FunctionAgent` to the emem MCP server.
The agent reads a signed fact about a place and explains how to check it.

## What you need

- Python 3.10 or later. We tested with Python 3.12.
- An OpenAI API key for the model. emem reads need no key.

## What you get

The script prints the number of tools it loaded. Then it prints the answer
of the agent: the elevation of South Mumbai, its `fact_cid`, and how to
verify it.

## Install

```bash
pip install llama-index-tools-mcp llama-index-llms-openai llama-index-core
```

## Run

```bash
export OPENAI_API_KEY="sk-..."
python emem_mcp_geospatial_agent.py
```

To use a different emem node, set `EMEM_MCP_URL`. The default is
`https://emem.dev/mcp`.

## What the script does

1. It connects to `https://emem.dev/mcp` over Streamable HTTP with
   `BasicMCPClient`.
2. It loads the 18 core emem tools that `/mcp` lists with `McpToolSpec`.
3. It makes a `FunctionAgent` with the `gpt-4.1-mini` model.
4. It asks what the signed record says about the elevation of South Mumbai.
   It also asks for the `fact_cid` and how to verify it.

## Test status

On 7 November 2026 we installed the packages above
(llama-index-tools-mcp 0.6.0, llama-index-core 0.14.25, mcp 2.3.0). The script
printed `Loaded 18 emem tools via MCP` and stopped at the OpenAI call, because
we used a test key. We did not run the model part.

We also made the emem calls that the agent needs, on the same day:

| call | result on 7 November 2026 |
|---|---|
| `emem_locate` with `South Mumbai` | cell `defi.zb4d7.zb8ec.zf21e`, "South Mumbai (precinct of the city of Mumbai)" |
| `emem_recall` on that cell, band `copdem30m.elevation_mean` | 6.56 m |
| `emem_recall` with the place name | error `no_geocoder_match`: the name is ambiguous |

If the model sends the place name to `emem_recall`, emem refuses to guess.
The model must use the cell from `emem_locate`.

## Notes

- Any LlamaIndex LLM can drive the agent. Change `OpenAI` to the class for
  Anthropic, Gemini or another provider.
- `/mcp` lists 18 core tools. `https://emem.dev/mcp/full` lists all tools in
  pages. On 7 November 2026 it had 116 tools in 8 pages, and `McpToolSpec`
  loaded only the first page (13 tools). That page did not have
  `emem_recall`. Keep `/mcp` for this script.
- [`examples/llamaindex.py`](../llamaindex.py) wraps the REST API instead of
  MCP. This example uses MCP through `llama-index-tools-mcp`.
