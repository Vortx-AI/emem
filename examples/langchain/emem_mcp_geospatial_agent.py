"""emem + LangChain via MCP — geospatial verification agent.

Connects to the live emem MCP server over Streamable HTTP, auto-discovers
all tools, and runs a LangGraph ReAct agent that answers a geospatial
verification question.

Install:
    pip install langchain-mcp-adapters langgraph langchain-openai

Usage:
    export OPENAI_API_KEY="sk-..."
    python emem_mcp_geospatial_agent.py

The agent will check whether Helsinki Airport, Finland (60.3172, 24.9633)
appears to be low-lying or flood-prone, citing signed receipts.
"""

import asyncio
import os

from langchain_mcp_adapters.client import MultiServerMCPClient
from langgraph.prebuilt import create_react_agent
from langchain_openai import ChatOpenAI

EMEM_MCP_URL = os.getenv("EMEM_MCP_URL", "https://emem.dev/mcp")

QUESTION = (
    "Using emem, check whether Helsinki Airport, Finland (60.3172, 24.9633) "
    "appears to be low-lying or flood-prone. Use verifiable evidence and "
    "cite signed facts or receipts when available."
)


async def main():
    llm = ChatOpenAI(model="gpt-4.1-mini")

    async with MultiServerMCPClient(
        {
            "emem": {
                "transport": "streamable_http",
                "url": EMEM_MCP_URL,
            }
        }
    ) as client:
        tools = client.get_tools()
        print(f"Loaded {len(tools)} emem tools via MCP\n")

        agent = create_react_agent(llm, tools)

        response = await agent.ainvoke({"messages": [("user", QUESTION)]})

        for msg in response["messages"]:
            if hasattr(msg, "content") and msg.content:
                print(f"[{msg.type}] {msg.content}\n")


if __name__ == "__main__":
    asyncio.run(main())
