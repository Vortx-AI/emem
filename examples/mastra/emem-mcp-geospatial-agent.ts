/**
 * Mastra + emem MCP example
 *
 * This example connects a Mastra agent to the emem MCP server and asks
 * a place-based geospatial verification question.
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
    name: 'emem-geospatial-agent',
    instructions:
      'You are a geospatial verification agent. Use emem tools for place-based evidence. When emem returns signed facts or receipts, cite them in the answer.',
    model: openai('gpt-4o-mini'),
    tools,
  });

  const response = await agent.generate(
    'Using emem, check whether Helsinki Airport, Finland (60.3172, 24.9633) appears to be low-lying or flood-prone. Use verifiable evidence and cite signed facts or receipts when available.',
  );

  console.log(response.text);

  await mcpClient.disconnect();
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
