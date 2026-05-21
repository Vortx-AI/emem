# CrewAI + emem MCP Agent

A CrewAI Agent that connects to emem over MCP (Streamable HTTP),
auto-discovers all tools, and answers a geospatial verification question
with signed receipts.

## Install

```bash
pip install 'crewai[tools]' 'mcp[cli]'
```

## Run

```bash
export OPENAI_API_KEY="sk-..."
python emem_mcp_geospatial_agent.py
```

## What it does

1. Connects to `https://emem.dev/mcp` via Streamable HTTP using
   `crewai_tools.MCPServerAdapter`.
2. Auto-discovers all emem MCP tools (`emem_locate`, `emem_recall`,
   `emem_eudr_dds`, `emem_verify_receipt`, …).
3. Creates a single-Agent Crew with a `geospatial_analyst` role and a
   task that asks: is Manaus, Brazil deforested by EUDR Article 2(4)?
4. The agent dispatches `emem_eudr_dds`, inspects the per-cell verdict
   block, and reports the verdict + `fact_cid`s + responder pubkey.

## Notes

- No API key needed for emem (reads are anonymous, ed25519-signed).
- Swap the LLM by changing the `llm=` argument on `Agent` — any
  CrewAI-supported provider (Anthropic, Gemini, Groq, local
  Ollama, …) works.
- For multi-agent flows, add a `verifier` agent whose only tool is
  `emem_verify_receipt` and chain it after the analyst.
