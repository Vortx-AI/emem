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

Start from the root of the repository:

```bash
cd examples/mastra
npm install
```

`package.json` sets the versions: `@mastra/core` and `@mastra/mcp` `^0.10.0`,
`@ai-sdk/openai` `^1.0.0`, and `tsx`. Do not install the packages by name
without versions. On 7 November 2026 these ranges resolved on npm to
`@mastra/core` 0.10.15, `@mastra/mcp` 0.10.12 and `@ai-sdk/openai` 1.3.24.
The latest versions were `@mastra/core` 1.82.0 and `@mastra/mcp` 2.8.0. We
did not test the script with them.

## Run

Run these commands in `examples/mastra`.

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

We ran the script on 7 November 2026 with Node.js 24.19.0 and npm 11.17.0.
`npm install` resolved the versions above. npm warned that the install
scripts of `esbuild` and `protobufjs` did not run. The script ran without
them. We used a dummy OpenAI key, so the run stopped at the OpenAI error
`Incorrect API key provided`. Before that error, `MCPClient` connected to
emem and loaded the tools. `tsc --strict --noEmit` gave no errors.

We did not see the agent answer, because that needs a real OpenAI key.

We called `emem_hunt` with the same arguments over MCP on 7 November 2026. It
returned 8 hotspots with the algorithm `algal_bloom_chlorophyll_ndci@1`. The
first hotspot was cell `defi.zb5de.rAsI.wEfA`, with primary band
`indices.ndvi` at -0.216 and gate band `indices.ndwi` at 0.537.
Each hotspot had a `fact_cid` and a `facts_url`. The `facts_url` is a GET
on `/v1/recall`, which accepts only POST, so it returns 405. To read the
facts, send `POST /v1/recall` with the `cell64`.

A hunt ranks cells by the primary band of the algorithm only. It does not
compute the full formula. See [Hunt use cases](../hunt-use-cases.md) for how
a hunt samples and ranks cells.

## Notes

- `/mcp/full` sends its tools in pages. On 7 November 2026 it had 116 tools in
  9 pages, and 8 pages earlier that day. The server cuts a page at about
  45,000 bytes, so the page count and the tools on each page change when the
  server changes. The first page had 12 tools with 9 pages, and 13 with 8.
  `MCPClient` in `@mastra/mcp` 0.10.12 calls `tools/list` once and does not
  send the next cursor. So the agent gets only the first page. On 7 November
  2026, `mcp.getTools()` returned 12 tools, and `emem_hunt` was one of them. A
  tool on a later page does not get to the agent.
- `emem_verify_receipt` was not on the first page on 7 November 2026. So
  this agent cannot check a receipt itself. To add a verify step, give the
  agent a second `MCPClient` on `https://emem.dev/mcp`, which lists the 18
  core tools. You can also check the receipt offline, see
  [Verify the receipt](../../docs/quickstart.md#verify-the-receipt).
- `MCPClient` puts the server name before each tool name. With the server
  name `emem`, the agent sees `emem_emem_hunt`, not `emem_hunt`.
- Any AI SDK provider that Mastra supports can drive the agent. Change the
  `openai(...)` import to `@ai-sdk/anthropic`, `@ai-sdk/google` or another
  provider.
