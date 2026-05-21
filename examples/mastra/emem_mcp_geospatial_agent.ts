// Mastra + emem MCP example — algal-bloom hotspots over Lake Erie.
//
// Wires a Mastra Agent to the emem MCP server (Streamable HTTP), lets
// the agent auto-discover all tools, and asks it to hunt for active
// algal-bloom hotspots. The agent dispatches `emem_hunt` and reports
// each hotspot with its cell64, NDVI/NDWI values, and fact_cid.
//
// Install:
//   npm install @mastra/core @mastra/mcp @ai-sdk/openai
//
// Run:
//   export OPENAI_API_KEY="sk-..."
//   npx tsx emem_mcp_geospatial_agent.ts
//
// emem itself requires no API key — reads are anonymous and
// ed25519-signed. The OpenAI key is only for the model driving the
// agent.

import { Agent } from "@mastra/core/agent";
import { MCPClient } from "@mastra/mcp";
import { openai } from "@ai-sdk/openai";

const EMEM_MCP_URL = process.env.EMEM_MCP_URL ?? "https://emem.dev/mcp";

const QUESTION =
  "Using the emem MCP tools, find the most likely active algal bloom " +
  "hotspots over Lake Erie. Call emem_hunt with event='algal_bloom' " +
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
