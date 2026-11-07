# LangChain + emem MCP Agent

A LangGraph ReAct agent that connects to emem over MCP (Streamable HTTP),
auto-discovers all tools, and answers questions about real places with
signed facts from the shared memory, citing receipts any agent can re-verify.

## Install

```bash
pip install langchain-mcp-adapters langgraph langchain-openai
```

## Run

```bash
export OPENAI_API_KEY="sk-..."
python emem_mcp_geospatial_agent.py
```

## What it does

1. Connects to `https://emem.dev/mcp` via Streamable HTTP.
2. Discovers the 18 core emem MCP tools that `/mcp` lists (`emem_locate`,
   `emem_recall`, `emem_ask`, `emem_verify_receipt`, and the rest). Use
   `https://emem.dev/mcp/full` for all 116.
3. Creates a ReAct agent that reasons over the tools.
4. Asks for the elevation of South Mumbai (`copdem30m.elevation_mean`).
5. The agent resolves the place, recalls the signed fact, and answers with
   the value, the `fact_cid` and the receipt.

## Notes

- No API key needed for emem (reads are anonymous).
- Any LangChain-compatible LLM works — swap `ChatOpenAI` for `ChatAnthropic`,
  `ChatGoogle`, etc.
- The existing `examples/langchain.py` in the repo root is a REST-based wrapper.
  This example uses the MCP protocol directly via `langchain-mcp-adapters`.
