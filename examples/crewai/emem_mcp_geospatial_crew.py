"""CrewAI + emem MCP example -- underwriting crew: flood/elevation/surface-water check.

Connects a CrewAI underwriting crew to the emem MCP server over Streamable
HTTP. The crew checks a site for elevation, flood/surface-water signals,
and built-up context, then writes a short risk note with receipt IDs.

Install:
    pip install crewai crewai-tools[mcp]

Usage:
    export OPENAI_API_KEY="sk-..."
    python emem_mcp_geospatial_crew.py

The crew will run a real-estate/insurance underwriting check on a site,
using emem for signed geospatial evidence.
"""

import os

from crewai import Agent, Crew, Process, Task
from crewai_tools import MCPServerAdapter

EMEM_MCP_URL = os.getenv("EMEM_MCP_URL", "https://emem.dev/mcp")


def main() -> None:
    server_params = {
        "url": EMEM_MCP_URL,
        "transport": "streamable-http",
    }

    with MCPServerAdapter(server_params) as emem_tools:
        agent = Agent(
            role="Real-estate underwriting analyst",
            goal=(
                "Check a site for elevation, flood/surface-water signals, and "
                "built-up context using emem. Write a short risk note with "
                "receipt IDs for each fact."
            ),
            backstory=(
                "You are an underwriting analyst at an insurance/real-estate firm. "
                "You use satellite-derived geospatial evidence from emem to assess "
                "physical risk at a site before a policy or deal closes."
            ),
            tools=emem_tools,
            verbose=True,
        )

        task = Task(
            description=(
                "Run an underwriting check on South Mumbai. "
                "Using emem, check elevation, surface-water/flood signals, "
                "and built-up context. Write a short risk note that includes "
                "the value and receipt ID for each fact."
            ),
            expected_output=(
                "A short underwriting risk note covering elevation, flood/surface-water, "
                "and built-up context, with emem receipt IDs for each fact cited."
            ),
            agent=agent,
        )

        crew = Crew(
            agents=[agent],
            tasks=[task],
            process=Process.sequential,
            verbose=True,
        )

        result = crew.kickoff()
        print(result)


if __name__ == "__main__":
    main()
