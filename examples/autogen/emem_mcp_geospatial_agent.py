"""AutoGen + emem MCP example -- multi-step verification: locate, recall, verify.

Connects a Microsoft AutoGen assistant to the emem MCP server over
Streamable HTTP and runs a multi-step verification chain for South Mumbai:
resolve the place, recall elevation, then verify the receipt/fact CID.

Install:
    pip install autogen-agentchat 'autogen-ext[openai,mcp]' 'mcp<2'

Usage:
    export OPENAI_API_KEY="sk-..."
    python emem_mcp_geospatial_agent.py

The agent will resolve South Mumbai, recall its elevation, then verify
the receipt/fact CID path, showing each step in the chain.
"""

import asyncio
import os

from autogen_agentchat.agents import AssistantAgent
from autogen_agentchat.ui import Console
from autogen_ext.models.openai import OpenAIChatCompletionClient
from autogen_ext.tools.mcp import McpWorkbench, StreamableHttpServerParams

EMEM_MCP_URL = os.getenv("EMEM_MCP_URL", "https://emem.dev/mcp")


async def main() -> None:
    model_client = OpenAIChatCompletionClient(model="gpt-4.1-mini")

    server_params = StreamableHttpServerParams(
        url=EMEM_MCP_URL,
    )

    async with McpWorkbench(server_params) as emem_workbench:
        agent = AssistantAgent(
            name="emem_geospatial_agent",
            model_client=model_client,
            workbench=emem_workbench,
            system_message=(
                "You are a geospatial verification agent. "
                "Follow these steps in order: "
                "1. Call emem_locate for South Mumbai and keep the cell64 it returns. "
                "2. Call emem_recall with that cell64 as `cell` and the band "
                "copdem30m.elevation_mean. Do not send the place name to emem_recall. "
                "3. Call emem_verify_receipt with the receipt from step 2. "
                "Show each step and the final verified answer."
            ),
            reflect_on_tool_use=True,
            model_client_stream=True,
        )

        await Console(
            agent.run_stream(
                task=(
                    "Using emem, locate South Mumbai, recall its elevation "
                    "at that cell, then verify the receipt. Show each step: "
                    "locate, recall, verify."
                )
            )
        )

    await model_client.close()


if __name__ == "__main__":
    asyncio.run(main())
