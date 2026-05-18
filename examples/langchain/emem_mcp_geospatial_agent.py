"""emem + LangChain via MCP -- ask a place question and cite receipts.

Connects to the live emem MCP server over Streamable HTTP, auto-discovers
all tools, and runs a LangGraph ReAct agent that resolves South Mumbai,
recalls its elevation, and answers with the signed fact CID/receipt.

Install:
    pip install langchain-mcp-adapters langgraph langchain-openai

Usage:
    export OPENAI_API_KEY="sk-..."
    python emem_mcp_geospatial_agent.py

The agent will resolve South Mumbai, recall its elevation from
copdem30m.elevation_mean, and return the answer with a signed fact CID.
"""

import asyncio
import os

from langchain_mcp_adapters.client import MultiServerMCPClient
from langgraph.prebuilt import create_react_agent
from langchain_openai import ChatOpenAI

EMEM_MCP_URL = os.getenv("EMEM_MCP_URL", "https://emem.dev/mcp")

QUESTION = (
    "Using emem, resolve South Mumbai, recall its elevation "
    "(copdem30m.elevation_mean), and answer with the signed fact CID/receipt. "
    "Show the elevation value and the receipt so the answer can be independently verified."
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
