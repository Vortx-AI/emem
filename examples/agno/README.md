# Agno + emem MCP Agent

An Agno Agent that connects to emem over MCP (Streamable HTTP),
auto-discovers all tools, and answers geospatial verification questions
with signed receipts.

## Install

```bash
pip install agno openai
```

## Run

```bash
export OPENAI_API_KEY="sk-..."
python emem_mcp_geospatial_agent.py
```

## What it does

1. Connects to `https://emem.dev/mcp` via Streamable HTTP.
2. Auto-discovers all emem MCP tools (locate, recall, compare, verify, etc.).
3. Creates an Agno Agent with OpenAIChat.
4. Asks whether Helsinki Airport, Finland (60.3172, 24.9633) appears to be
   low-lying or flood-prone.
5. The agent recalls elevation, land cover, surface water, and summarises
   with signed `fact_cid`s from the receipts.

## Notes

- No API key needed for emem (reads are anonymous).
- Any Agno-compatible model works -- swap `OpenAIChat` for `Anthropic`,
  `Gemini`, etc.
