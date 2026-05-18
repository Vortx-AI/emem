/**
 * Mastra + emem MCP example -- event hunt: algal bloom in Lake Erie
 *
 * This example connects a Mastra agent to the emem MCP server and hunts
 * for algal bloom hotspots in Lake Erie using emem's hunt/event workflow.
 *
 * Prerequisites:
 *   npm install @mastra/core @mastra/mcp @ai-sdk/openai dotenv
 *
 * Environment:
 *   export OPENAI_API_KEY="..."
 */

import 'dotenv/config';
import { Agent } from '@mastra/core/agent';
import { MCPClient } from '@mastra/mcp';
import { openai } from '@ai-sdk/openai';

const EMEM_MCP_URL = process.env.EMEM_MCP_URL ?? 'https://emem.dev/mcp';

async function main() {
  const mcpClient = new MCPClient({
    id: 'emem-mcp-client',
    servers: {
      emem: {
        url: new URL(EMEM_MCP_URL),
      },
    },
  });

  const tools = await mcpClient.getTools();

  const agent = new Agent({
    name: 'emem-event-hunter',
    instructions:
      'You are a geospatial event hunter. Use emem tools to find environmental events and hotspots. When emem returns signed facts, cell IDs, or scene URLs, include them in the answer.',
    model: openai('gpt-4o-mini'),
    tools,
  });

  const response = await agent.generate(
    'Using emem, find algal bloom hotspots in Lake Erie. Return the top cells, primary band, values, fact CIDs, and scene URLs if available.',
  );

  console.log(response.text);

  await mcpClient.disconnect();
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
