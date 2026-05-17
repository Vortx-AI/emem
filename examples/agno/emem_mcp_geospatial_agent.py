"""emem + Agno via MCP -- geospatial verification agent.

Connects to the live emem MCP server over Streamable HTTP, auto-discovers
all tools, and runs an Agno Agent that answers a geospatial verification
question.

Install:
    pip install agno openai

Usage:
    export OPENAI_API_KEY="sk-..."
    python emem_mcp_geospatial_agent.py

The agent will check whether Helsinki Airport, Finland (60.3172, 24.9633)
appears to be low-lying or flood-prone, citing signed receipts.
"""

import asyncio
import os

from agno.agent import Agent
from agno.models.openai import OpenAIChat
from agno.tools.mcp import MCPTools

EMEM_MCP_URL = os.getenv("EMEM_MCP_URL", "https://emem.dev/mcp")

QUESTION = (
    "Using emem, check whether Helsinki Airport, Finland (60.3172, 24.9633) "
    "appears to be low-lying or flood-prone. Use verifiable evidence and "
    "cite signed facts or receipts when available."
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
