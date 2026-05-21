# Mastra + emem MCP Agent

A Mastra Agent (TypeScript) that connects to emem over MCP (Streamable
HTTP), auto-discovers all tools, and answers a geospatial verification
question with signed receipts.

## Install

```bash
npm install @mastra/core @mastra/mcp @ai-sdk/openai
```

A `package.json` is included; `npm install` from this directory pulls
the same versions.

## Run

```bash
export OPENAI_API_KEY="sk-..."
npx tsx emem_mcp_geospatial_agent.ts
```

## What it does

1. Connects to `https://emem.dev/mcp` via Streamable HTTP using
   `@mastra/mcp` `MCPClient`.
2. Auto-discovers all emem MCP tools (`emem_locate`, `emem_recall`,
   `emem_hunt`, `emem_verify_receipt`, …).
3. Creates a Mastra `Agent` bound to those tools and an OpenAI model.
4. Asks: "Where is the active algal bloom over Lake Erie this week,
   and which cells signed the answer?"
5. The agent dispatches `emem_hunt` with `event=algal_bloom`,
   `region=Lake Erie`, then reports each hotspot with its `cell64`,
   gated band values, and `fact_cid`.

## Notes

- emem reads are anonymous and ed25519-signed; no emem API key.
- Swap the model by editing the `openai(...)` import to
  `@ai-sdk/anthropic`, `@ai-sdk/google`, etc. — any AI SDK provider
  Mastra supports works.
- For Mastra workflows, wire this agent as a step and chain it with a
  `verify` step that calls `emem_verify_receipt` on the reported CIDs.
