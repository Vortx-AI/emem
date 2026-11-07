# LangChain agent with emem over MCP

This example connects a LangGraph ReAct agent to the emem MCP server. The
agent finds a place, reads a signed fact there, and gives the `fact_cid` and
receipt so that another agent can check the answer.

## What you need

- Python 3.10 or later. We tested with Python 3.12.
- An OpenAI API key for the model. emem reads need no key.

## What you get

The script prints the number of tools it loaded. Then it prints each message
of the agent run. The last message gives the elevation of South Mumbai with
its `fact_cid` and receipt.

## Install

```bash
pip install langchain-mcp-adapters langgraph langchain-openai
```

## Run

```bash
export OPENAI_API_KEY="sk-..."
python emem_mcp_geospatial_agent.py
```

To use a different emem node, set `EMEM_MCP_URL`. The default is
`https://emem.dev/mcp`.

## What the script does

1. It makes a `MultiServerMCPClient` for `https://emem.dev/mcp` over
   Streamable HTTP.
2. It loads the 18 core emem tools that `/mcp` lists, for example
   `emem_locate`, `emem_recall`, `emem_ask` and `emem_verify_receipt`.
3. It makes a ReAct agent with the `gpt-4.1-mini` model.
4. It asks the agent to locate South Mumbai, recall its elevation
   (`copdem30m.elevation_mean`), and answer with the `fact_cid` and receipt.

## Test status

On 7 November 2026 we installed the packages above
(langchain-mcp-adapters 0.3.2, langgraph 1.2.20, mcp 1.30.0). The script
loaded the emem tools and stopped at the OpenAI call, because we used a test
key. We did not run the model part.

We also made the emem calls that the agent needs, on the same day:

| call | result on 7 November 2026 |
|---|---|
| `emem_locate` with `South Mumbai` | cell `defi.zb4d7.zb8ec.zf21e`, "South Mumbai (precinct of the city of Mumbai)" |
| `emem_recall` on that cell, band `copdem30m.elevation_mean` | 6.56 m |
| `emem_recall` with the place name | error `no_geocoder_match`: the name is ambiguous |

If the model sends the place name to `emem_recall`, emem refuses to guess.
The model must use the cell from `emem_locate`.

## Notes

- Any LangChain chat model can drive the agent. Change `ChatOpenAI` to
  `ChatAnthropic`, `ChatGoogleGenerativeAI` or another chat model class.
- `/mcp` lists 18 core tools. For all tools, set
  `EMEM_MCP_URL=https://emem.dev/mcp/full`. On 7 November 2026 that list had
  116 tools in 9 pages (8 pages earlier that day), and the adapter loaded all
  116.
- [`examples/langchain.py`](../langchain.py) wraps the REST API instead of
  MCP. This example uses MCP through `langchain-mcp-adapters`.
