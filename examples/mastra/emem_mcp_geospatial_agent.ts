// Mastra + emem MCP example: algal-bloom hotspots over Lake Erie.
//
// Wires a Mastra Agent to the emem MCP server (Streamable HTTP) and
// gives it the tools that MCPClient gets. /mcp/full sends its tools in
// pages, and MCPClient reads only the first page (12 tools on
// 7 November 2026, with emem_hunt). Then it asks the agent to hunt for active
// algal-bloom hotspots. MCPClient puts the server name before each tool
// name, so the agent calls `emem_emem_hunt`. It reports each hotspot
// with its cell64, NDVI and NDWI values, and fact_cid.
//
// Install:
//   npm install   (in this directory; package.json pins the versions)
//
// Run:
//   export OPENAI_API_KEY="sk-..."
//   npx tsx emem_mcp_geospatial_agent.ts
//
// emem itself requires no API key. Reads are anonymous and
// ed25519-signed. The OpenAI key is only for the model driving the
// agent.

import { Agent } from "@mastra/core/agent";
import { MCPClient } from "@mastra/mcp";
import { openai } from "@ai-sdk/openai";

// /mcp/full, not /mcp: emem_hunt is an extended-tier tool, and /mcp
// lists only the 18 core tools.
const EMEM_MCP_URL = process.env.EMEM_MCP_URL ?? "https://emem.dev/mcp/full";

const QUESTION =
  "Using the emem MCP tools, find the most likely active algal bloom " +
  "hotspots over Lake Erie. Call the tool emem_emem_hunt with event='algal_bloom' " +
  "and region='Lake Erie'. For each hotspot, report the cell64, the " +
  "primary band value, the gate band value, and the fact_cid. Do not " +
  "invent fact_cids; only quote what the tool returns. End with the " +
  "responder pubkey from the receipt.";

async function main(): Promise<void> {
  const mcp = new MCPClient({
    servers: {
      emem: {
        url: new URL(EMEM_MCP_URL),
      },
    },
  });

  const tools = await mcp.getTools();

  const agent = new Agent({
    name: "geospatial-evidence-agent",
    instructions:
      "You are an evidence-first geospatial agent. Every value you " +
      "report must come from a tool result, and you cite the " +
      "fact_cid so a reader can independently verify at /verify.",
    model: openai("gpt-4.1-mini"),
    tools,
  });

  const result = await agent.generate(QUESTION);
  console.log(result.text);

  await mcp.disconnect();
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
