"""emem + LlamaIndex via MCP -- geospatial verification agent.

Connects to the live emem MCP server over Streamable HTTP, auto-discovers
all tools, and runs a LlamaIndex FunctionAgent that answers a geospatial
verification question.

Install:
    pip install llama-index-tools-mcp llama-index-llms-openai llama-index-core

Usage:
    export OPENAI_API_KEY="sk-..."
    python emem_mcp_geospatial_agent.py

The agent will locate the coordinates (23.351921, 85.309145), recall
geospatial facts, and summarise what it finds -- citing signed receipts.
"""

import asyncio

from llama_index.tools.mcp import BasicMCPClient, McpToolSpec
from llama_index.core.agent.function_calling import FunctionAgent
from llama_index.llms.openai import OpenAI

EMEM_MCP_URL = "https://emem.dev/mcp"

QUESTION = (
    "I have coordinates 23.351921, 85.309145. "
    "Locate this place, recall its geospatial facts (elevation, "
    "land cover, vegetation, surface water), and summarise what "
    "you find. Cite the signed receipt fact_cids in your answer."
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
