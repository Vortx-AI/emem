# LangChain agent with emem over MCP

This example connects a LangChain agent to the emem MCP server. The agent
finds a place, reads a signed fact there, and gives the `fact_cid` and
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
pip install langchain langchain-mcp-adapters langchain-openai
```

## Run

Run these commands from the root of the repository.

```bash
export OPENAI_API_KEY="sk-..."
python examples/langchain/emem_mcp_geospatial_agent.py
```

To use a different emem node, set `EMEM_MCP_URL`. The default is
`https://emem.dev/mcp`.

## What the script does

1. It makes a `MultiServerMCPClient` for `https://emem.dev/mcp` over
   Streamable HTTP.
2. It loads the 18 core emem tools that `/mcp` lists, for example
   `emem_locate`, `emem_recall`, `emem_ask` and `emem_verify_receipt`.
3. It makes an agent with `create_agent` from `langchain.agents` and the
   `gpt-4.1-mini` model. LangGraph 1.0 deprecated the older
   `create_react_agent`. Its warning says that LangGraph 2.0 will remove
   it.
4. It asks the agent to locate South Mumbai, recall its elevation
   (`copdem30m.elevation_mean`), and answer with the `fact_cid` and receipt.

## Test status

On 7 November 2026 we installed the packages above in a new virtual
environment (langchain 1.4.9, langchain-mcp-adapters 0.3.2, langgraph
1.2.20, mcp 1.30.0). The script loaded 18 emem tools and stopped at the
OpenAI call, because we used a test key. We did not run the model part.

We also made the emem calls that the agent needs, on the same day:

| call | result on 7 November 2026 |
|---|---|
| `emem_locate` with `South Mumbai` | `cell64` is `defi.zb4d7.zb8ec.zf21e`; `place_label` is "South Mumbai (precinct of the city of Mumbai)" |
| `emem_recall` on that cell, band `copdem30m.elevation_mean` | `facts[0].value` is 6.56 m; `facts[0].fact_cid` and `receipt` come with it |
| `emem_recall` with the place name | tool error `no_geocoder_match`: the name is ambiguous. Over REST it is HTTP 400 with `code` `no_geocoder_match` |

If the model sends the place name to `emem_recall`, emem refuses to guess.
The model must use the cell from `emem_locate`.

## Notes

- Any LangChain chat model can drive the agent. Change `ChatOpenAI` to
  `ChatAnthropic`, `ChatGoogleGenerativeAI` or another chat model class.
- `/mcp` lists 18 core tools. For all tools, set
  `EMEM_MCP_URL=https://emem.dev/mcp/full`. On 7 November 2026 that list had
  116 tools in 9 pages, and the adapter loaded all 116.
- [`examples/langchain.py`](../langchain.py) wraps the REST API instead of
  MCP. This example uses MCP through `langchain-mcp-adapters`.
