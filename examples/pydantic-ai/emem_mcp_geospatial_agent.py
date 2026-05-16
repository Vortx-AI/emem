"""Pydantic AI + emem MCP example.

Connects a Pydantic AI agent to the emem MCP server over Streamable HTTP
and asks a place-based geospatial verification question.

Install:
    pip install pydantic-ai

Usage:
    export OPENAI_API_KEY="sk-..."
    python emem_mcp_geospatial_agent.py

The agent will check whether Helsinki Airport, Finland (60.3172, 24.9633)
appears to be low-lying or flood-prone, citing signed receipts.
"""

import asyncio
import os

from pydantic_ai import Agent
from pydantic_ai.mcp import MCPServerStreamableHTTP

EMEM_MCP_URL = os.getenv("EMEM_MCP_URL", "https://emem.dev/mcp")


async def main() -> None:
    server = MCPServerStreamableHTTP(EMEM_MCP_URL)

    agent = Agent(
        "openai:gpt-4.1-mini",
        toolsets=[server],
        system_prompt=(
            "You are a geospatial verification agent. "
            "Use emem tools for place-based evidence. "
            "When emem returns signed facts or receipts, cite them in the answer."
        ),
    )

    async with server:
        result = await agent.run(
            "Using emem, check whether Helsinki Airport, Finland "
            "(60.3172, 24.9633) appears to be low-lying or flood-prone. "
            "Use verifiable evidence and cite signed facts or receipts "
            "when available."
        )

    print(result.output)


if __name__ == "__main__":
    asyncio.run(main())
