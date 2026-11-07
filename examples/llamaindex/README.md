# LlamaIndex + emem MCP Agent

A LlamaIndex FunctionAgent that connects to emem over MCP (Streamable HTTP),
auto-discovers all tools, and answers questions about real places with
signed facts from the shared memory, citing receipts any agent can re-verify.

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
4. Asks what the signed record says about the elevation of South Mumbai.
5. The agent answers with the `fact_cid` and says how to verify it.

## Notes

- No API key needed for emem (reads are anonymous).
- Any LlamaIndex-compatible LLM works -- swap `OpenAI` for `Anthropic`,
  `Gemini`, etc.
- The existing `examples/llamaindex.py` in the repo root is a REST-based wrapper.
  This example uses the MCP protocol directly via `llama-index-tools-mcp`.
