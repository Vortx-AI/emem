"""AutoGen + emem MCP example.

Connects a Microsoft AutoGen assistant to the emem MCP server over
Streamable HTTP and asks a place-based geospatial verification question.

Install:
    pip install autogen-agentchat autogen-ext[openai,mcp]

Usage:
    export OPENAI_API_KEY="sk-..."
    python emem_mcp_geospatial_agent.py

The agent will check whether Helsinki Airport, Finland (60.3172, 24.9633)
appears to be low-lying or flood-prone, citing signed receipts.
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
                "Use emem tools for place-based evidence. "
                "When emem returns signed facts or receipts, cite them in the answer."
            ),
            reflect_on_tool_use=True,
            model_client_stream=True,
        )

        await Console(
            agent.run_stream(
                task=(
                    "Using emem, check whether Helsinki Airport, Finland "
                    "(60.3172, 24.9633) appears to be low-lying or flood-prone. "
                    "Use verifiable evidence and cite signed facts or receipts "
                    "when available."
                )
            )
        )

    await model_client.close()


if __name__ == "__main__":
    asyncio.run(main())
