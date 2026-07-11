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
2. Auto-discovers all emem MCP tools (locate, recall, compare, verify, etc.).
3. Creates a ReAct agent that reasons over the tools.
4. Asks whether Helsinki Airport, Finland (60.3172, 24.9633) appears to be
   low-lying or flood-prone.
5. The agent recalls elevation, land cover, surface water, and summarises
   with signed `fact_cid`s from the receipts.

## Notes

- No API key needed for emem (reads are anonymous).
- Any LangChain-compatible LLM works — swap `ChatOpenAI` for `ChatAnthropic`,
  `ChatGoogle`, etc.
- The existing `examples/langchain.py` in the repo root is a REST-based wrapper.
  This example uses the MCP protocol directly via `langchain-mcp-adapters`.
