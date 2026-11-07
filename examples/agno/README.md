# Agno agent with emem over MCP

This example connects an Agno agent to the emem MCP server. The agent answers
a question about a real place with signed facts and cites their `fact_cid`
values. Another agent can check those facts.

## What you need

- Python 3.10 or later. We tested with Python 3.12.
- An OpenAI API key for the model. emem reads need no key.

## What you get

The agent checks Helsinki Airport, Finland for elevation and surface water. It
prints an answer in Markdown with the `fact_cid` of each value it uses.

## Install

```bash
pip install 'agno[mcp]' openai
```

The `mcp` extra installs `mcp` and `fastmcp`. `MCPTools` needs both.

## Run

```bash
export OPENAI_API_KEY="sk-..."
python emem_mcp_geospatial_agent.py
```

To use a different emem node, set `EMEM_MCP_URL`. The default is
`https://emem.dev/mcp`.

## What the script does

1. It connects to `https://emem.dev/mcp` over Streamable HTTP with `MCPTools`.
2. It loads the 18 core emem tools that `/mcp` lists, for example
   `emem_locate`, `emem_recall` and `emem_verify_receipt`.
3. It makes an Agno `Agent` with the `gpt-4.1-mini` model.
4. It asks the agent to check Helsinki Airport, Finland for elevation and
   surface-water or flood signals. The agent must use only facts from emem.

## Test status

On 7 November 2026 we installed the packages above (agno 3.1.12, mcp 2.3.0,
fastmcp 4.1.6). The script loaded 18 tools from emem and stopped at the
OpenAI call, because we used a test key. We did not run the model part.

We also made the emem calls that the agent needs, on the same day:

| call | result on 7 November 2026 |
|---|---|
| `emem_locate` with `Helsinki Airport, Finland` | cell `defi.zb6ae.nihE.rAvo`, "Helsinki-Vantaan lentoasema (aerodrome)" |
| `emem_recall` on that cell | elevation 49.94 m, surface-water recurrence 0 %, WorldCover class 30 (grassland) |
| `emem_recall` with the place name | error `no_geocoder_match`: the name is ambiguous |

If the model sends the place name to `emem_recall`, emem refuses to guess.
The model must call `emem_locate` first and then recall by `cell`.

## Notes

- Any model that Agno supports can drive the agent. Change `OpenAIChat` to
  another Agno model class, for example the Anthropic or Gemini class.
- `/mcp` lists 18 core tools. For all tools, set
  `EMEM_MCP_URL=https://emem.dev/mcp/full`. On 7 November 2026 that list had
  116 tools in 8 pages, and `MCPTools` loaded all 116.
