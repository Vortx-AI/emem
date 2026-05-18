"""emem + Agno via MCP -- fast agent tool call: place to facts to answer.

Connects to the live emem MCP server over Streamable HTTP, auto-discovers
all tools, and runs an Agno Agent that checks Helsinki Airport for
elevation and surface-water/flood signals.

Install:
    pip install agno openai

Usage:
    export OPENAI_API_KEY="sk-..."
    python emem_mcp_geospatial_agent.py

The agent will check Helsinki Airport, Finland for elevation and
surface-water/flood signals, returning only facts that emem can support.
"""

import asyncio
import os

from agno.agent import Agent
from agno.models.openai import OpenAIChat
from agno.tools.mcp import MCPTools

EMEM_MCP_URL = os.getenv("EMEM_MCP_URL", "https://emem.dev/mcp")

QUESTION = (
    "Using emem, check Helsinki Airport, Finland for elevation and "
    "surface-water/flood signals. Return only facts that emem can support."
)


async def main():
    async with MCPTools(url=EMEM_MCP_URL, transport="streamable-http") as mcp_tools:
        agent = Agent(
            model=OpenAIChat(id="gpt-4.1-mini"),
            tools=[mcp_tools],
            markdown=True,
        )

        response = await agent.arun(QUESTION)
        print(response.content)


if __name__ == "__main__":
    asyncio.run(main())
