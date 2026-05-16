# CrewAI + emem MCP Agent

A CrewAI agent that connects to emem over MCP (Streamable HTTP)
and answers geospatial verification questions with signed receipts.

## Install

```bash
pip install crewai crewai-tools[mcp]
```

## Run

```bash
export OPENAI_API_KEY="sk-..."
python emem_mcp_geospatial_crew.py
```

Optional:

```bash
export EMEM_MCP_URL="https://emem.dev/mcp"
```

## What it does

1. Connects to `https://emem.dev/mcp` via Streamable HTTP using
   `MCPServerAdapter`.
2. Auto-discovers all emem MCP tools (locate, recall, compare, verify, etc.).
3. Creates a CrewAI `Agent` with a geospatial verification role.
4. Asks whether Helsinki Airport, Finland (60.3172, 24.9633) appears to be
   low-lying or flood-prone.
5. The agent recalls elevation, land cover, surface water, and summarises
   with signed `fact_cid`s from the receipts.

## Notes

- No API key needed for emem (reads are anonymous).
- Any CrewAI-compatible LLM works via the CrewAI model configuration.
