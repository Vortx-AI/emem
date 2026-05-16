# Mastra + emem MCP Agent

A Mastra agent that connects to emem over MCP and answers geospatial
verification questions with signed receipts.

## Install

```bash
npm install @mastra/core @mastra/mcp @ai-sdk/openai dotenv
```

## Run

```bash
export OPENAI_API_KEY="sk-..."
npx tsx emem-mcp-geospatial-agent.ts
```

Optional:

```bash
export EMEM_MCP_URL="https://emem.dev/mcp"
```

## What it does

1. Connects to `https://emem.dev/mcp` using Mastra's `MCPClient`.
2. Auto-discovers all emem MCP tools (locate, recall, compare, verify, etc.).
3. Creates a Mastra `Agent` with a geospatial verification prompt.
4. Asks whether Helsinki Airport, Finland (60.3172, 24.9633) appears to be
   low-lying or flood-prone.
5. The agent recalls elevation, land cover, surface water, and summarises
   with signed `fact_cid`s from the receipts.

## Notes

- No API key needed for emem (reads are anonymous).
- Any AI SDK-compatible model works -- swap `openai('gpt-4o-mini')` for
  another provider.
