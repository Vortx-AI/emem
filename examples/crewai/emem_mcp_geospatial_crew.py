"""CrewAI + emem MCP example.

Connects a CrewAI agent to the emem MCP server over Streamable HTTP
and asks a place-based geospatial verification question.

Install:
    pip install crewai crewai-tools[mcp]

Usage:
    export OPENAI_API_KEY="sk-..."
    python emem_mcp_geospatial_crew.py

The agent will check whether Helsinki Airport, Finland (60.3172, 24.9633)
appears to be low-lying or flood-prone, citing signed receipts.
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
            role="Geospatial verification agent",
            goal=(
                "Answer place-based questions using emem's signed geospatial facts "
                "and cite receipts when they are returned."
            ),
            backstory=(
                "You specialize in checking Earth-observation and geospatial evidence "
                "for real-world places."
            ),
            tools=emem_tools,
            verbose=True,
        )

        task = Task(
            description=(
                "Using emem, check whether Helsinki Airport, Finland "
                "(60.3172, 24.9633) appears to be low-lying or flood-prone. "
                "Use verifiable evidence and cite signed facts or receipts "
                "when available."
            ),
            expected_output=(
                "A concise geospatial verification answer with supporting emem evidence, "
                "including signed facts or receipt identifiers if available."
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
