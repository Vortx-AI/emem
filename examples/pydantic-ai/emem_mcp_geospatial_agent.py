"""Pydantic AI + emem MCP example -- structured typed answer with receipt fields.

Connects a Pydantic AI agent to the emem MCP server over Streamable HTTP
and asks for Lake Erie algal bloom hotspot data, returning a structured
typed answer.

Install:
    pip install pydantic-ai

Usage:
    export OPENAI_API_KEY="sk-..."
    python emem_mcp_geospatial_agent.py

The agent will hunt for algal bloom hotspots in Lake Erie and return a
structured answer with fields: place, event, top_cell, primary_band,
value, fact_cid, scene_url, caveats.
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
            "You are a geospatial evidence agent. "
            "Use emem tools to find algal bloom hotspots. "
            "Return a structured answer with these fields: "
            "place, event, top_cell, primary_band, value, fact_cid, scene_url, caveats. "
            "Use only facts that emem can support."
        ),
    )

    async with server:
        result = await agent.run(
            "Using emem, find algal bloom hotspots in Lake Erie. "
            "Return a structured answer with fields: place, event, top_cell, "
            "primary_band, value, fact_cid, scene_url, and caveats."
        )

    print(result.output)


if __name__ == "__main__":
    asyncio.run(main())
