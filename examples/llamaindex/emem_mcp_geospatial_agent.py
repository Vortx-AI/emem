"""emem + LlamaIndex via MCP -- retrieve signed evidence for a place.

Connects to the live emem MCP server over Streamable HTTP, auto-discovers
all tools, and runs a LlamaIndex FunctionAgent that retrieves signed
geospatial evidence about South Mumbai's elevation.

Install:
    pip install llama-index-tools-mcp llama-index-llms-openai llama-index-core

Usage:
    export OPENAI_API_KEY="sk-..."
    python emem_mcp_geospatial_agent.py

The agent will retrieve the signed record for South Mumbai's elevation
and explain how the fact CID can be independently verified.
"""

import asyncio
import os

from llama_index.tools.mcp import BasicMCPClient, McpToolSpec
from llama_index.core.agent.function_calling import FunctionAgent
from llama_index.llms.openai import OpenAI

EMEM_MCP_URL = os.getenv("EMEM_MCP_URL", "https://emem.dev/mcp")

QUESTION = (
    "Using emem, answer: what does the signed record say about South Mumbai's "
    "elevation? Return the fact CID and explain how it can be verified."
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
