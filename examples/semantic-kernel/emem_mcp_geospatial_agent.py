"""Semantic Kernel + emem MCP example — multi-step verification: locate, recall, verify.

Connects a Microsoft Semantic Kernel kernel to the emem MCP server over
Streamable HTTP and runs a multi-step verification chain for South Mumbai:
resolve the place, recall elevation, then verify the receipt/fact CID.

Install:
    pip install semantic-kernel

Usage:
    export OPENAI_API_KEY="sk-..."
    python emem_mcp_geospatial_agent.py

The agent will resolve South Mumbai, recall its elevation, then verify
the receipt/fact CID path, showing each step in the chain.
"""

import asyncio
import os

from semantic_kernel import Kernel
from semantic_kernel.connectors.ai.open_ai import OpenAIChatCompletion
from semantic_kernel.connectors.ai.open_ai import OpenAIChatPromptExecutionSettings
from semantic_kernel.connectors.ai.function_choice_behavior import FunctionChoiceBehavior
from semantic_kernel.connectors.mcp import MCPStreamableHttpPlugin

EMEM_MCP_URL = os.getenv("EMEM_MCP_URL", "https://emem.dev/mcp")


async def main() -> None:
    kernel = Kernel()
    kernel.add_service(
        OpenAIChatCompletion(
            ai_model_id="gpt-4o",
            service_id="emem_agent",
        )
    )

    async with MCPStreamableHttpPlugin(
        name="emem",
        description="Shared, verifiable Earth memory — signed facts for any place on Earth",
        url=EMEM_MCP_URL,
    ) as emem_plugin:
        kernel.add_plugin(emem_plugin)

        execution_settings = OpenAIChatPromptExecutionSettings(
            service_id="emem_agent",
            function_choice_behavior=FunctionChoiceBehavior.Auto(),
        )

        response = await kernel.invoke_prompt(
            prompt=(
                "You are a geospatial verification agent. "
                "Follow these steps in order: "
                "1. Resolve South Mumbai to coordinates using emem locate. "
                "2. Recall the elevation (copdem30m.elevation_mean) for that location. "
                "3. Verify the receipt/fact CID returned by emem. "
                "Show each step and the final verified answer."
            ),
            settings=execution_settings,
        )

        print(response)


if __name__ == "__main__":
    asyncio.run(main())
