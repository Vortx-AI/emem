# LlamaIndex + emem MCP Agent

A LlamaIndex FunctionAgent that connects to emem over MCP (Streamable HTTP),
auto-discovers all tools, and answers geospatial verification questions
with signed receipts.

## Install

```bash
pip install llama-index-tools-mcp llama-index-llms-openai llama-index-core
```

## Run

```bash
export OPENAI_API_KEY="sk-..."
python emem_mcp_geospatial_agent.py
```

## What it does

1. Connects to `https://emem.dev/mcp` via Streamable HTTP.
2. Auto-discovers all emem MCP tools (locate, recall, compare, verify, etc.).
3. Creates a FunctionAgent that reasons over the tools.
4. Asks whether Helsinki Airport, Finland (60.3172, 24.9633) appears to be
   low-lying or flood-prone.
5. The agent recalls elevation, land cover, surface water, and summarises
   with signed `fact_cid`s from the receipts.

## Notes

- No API key needed for emem (reads are anonymous).
- Any LlamaIndex-compatible LLM works -- swap `OpenAI` for `Anthropic`,
  `Gemini`, etc.
- The existing `examples/llamaindex.py` in the repo root is a REST-based wrapper.
  This example uses the MCP protocol directly via `llama-index-tools-mcp`.
