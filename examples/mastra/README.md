# Mastra agent with emem over MCP

This example connects a Mastra agent (TypeScript) to the emem MCP server. The
agent looks for algal bloom hotspots over Lake Erie and cites the signed fact
behind each one.

## What you need

- Node.js with `npm` and `npx`.
- An OpenAI API key for the model. emem reads need no key.

## What you get

A list of hotspots. Each hotspot has its `cell64`, the primary band value,
the gate band value and the `fact_cid`. The list ends with the responder
public key from the receipt.

## Install

Run this in this directory:

```bash
npm install
```

`package.json` sets the versions: `@mastra/core` and `@mastra/mcp` `^0.10.0`,
`@ai-sdk/openai` `^1.0.0`, and `tsx`. Do not install the packages by name
without versions. On 6 November 2026 npm gave `@mastra/core` 1.82.0 and
`@mastra/mcp` 2.8.0 as the latest versions, and we did not test the script
with them.

## Run

```bash
export OPENAI_API_KEY="sk-..."
npx tsx emem_mcp_geospatial_agent.ts
```

`npm start` runs the same command. To use a different emem node, set
`EMEM_MCP_URL`. The default is `https://emem.dev/mcp/full`.

## What the script does

1. It connects to `https://emem.dev/mcp/full` over Streamable HTTP with the
   `MCPClient` of `@mastra/mcp`. It uses `/mcp/full` because `emem_hunt` is
   not one of the 18 core tools on `/mcp`.
2. It makes a Mastra `Agent` with those tools and the `gpt-4.1-mini` model.
3. It asks the agent to call `emem_hunt` with `event='algal_bloom'` and
   `region='Lake Erie'`.
4. The agent reports each hotspot with its `cell64`, band values and
   `fact_cid`.

## Test status

We did not run this script. Node.js is not on our test host.

We called `emem_hunt` with the same arguments over MCP on 6 November 2026. It
returned hotspots with the algorithm `algal_bloom_chlorophyll_ndci@1`. The
first hotspot was cell `defi.zb5de.rAsI.wEfA`, with gate band `indices.ndwi`
at 0.537. Each hotspot had a `fact_cid` and a `facts_url`.

A hunt ranks cells by the primary band of the algorithm only. It does not
compute the full formula. See [Hunt use cases](../hunt-use-cases.md) for how
a hunt samples and ranks cells.

## Notes

- `/mcp/full` sends its tools in pages. On 7 November 2026 it had 116 tools
  in 8 pages. `emem_hunt` was on the first page. We did not check if
  `MCPClient` reads all the pages.
- Any AI SDK provider that Mastra supports can drive the agent. Change the
  `openai(...)` import to `@ai-sdk/anthropic`, `@ai-sdk/google` or another
  provider.
