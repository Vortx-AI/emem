"""emem + LlamaIndex via MCP -- geospatial verification agent.

Connects to the live emem MCP server over Streamable HTTP, auto-discovers
all tools, and runs a LlamaIndex FunctionAgent that answers a geospatial
verification question.

Install:
    pip install llama-index-tools-mcp llama-index-llms-openai llama-index-core

Usage:
    export OPENAI_API_KEY="sk-..."
    python emem_mcp_geospatial_agent.py

The agent will check whether Helsinki Airport, Finland (60.3172, 24.9633)
appears to be low-lying or flood-prone, citing signed receipts.
"""

import asyncio

from llama_index.tools.mcp import BasicMCPClient, McpToolSpec
from llama_index.core.agent.function_calling import FunctionAgent
from llama_index.llms.openai import OpenAI

EMEM_MCP_URL = "https://emem.dev/mcp"

QUESTION = (
    "Using emem, check whether Helsinki Airport, Finland (60.3172, 24.9633) "
    "appears to be low-lying or flood-prone. Use verifiable evidence and "
    "cite signed facts or receipts when available."
)


async def main():
    mcp_client = BasicMCPClient(EMEM_MCP_URL)
    mcp_tool_spec = McpToolSpec(mcp_client)
    tools = await mcp_tool_spec.to_tool_list_async()
    print(f"Loaded {len(tools)} emem tools via MCP\n")

    llm = OpenAI(model="gpt-4.1-mini")
    agent = FunctionAgent(tools=tools, llm=llm)

    response = await agent.run(QUESTION)
    print(response)


if __name__ == "__main__":
    asyncio.run(main())
