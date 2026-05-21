"""CrewAI + emem MCP example — EUDR Article 2(4) verdict at Manaus.

Connects a CrewAI Agent to the emem MCP server over Streamable HTTP,
auto-discovers all tools, and runs a single-task crew that asks:
"Is the point at Manaus (Brazil) deforested under EUDR Article 2(4)?"

The agent dispatches emem_eudr_dds, inspects the per-cell verdict
block, and reports the verdict plus signed fact_cids and the responder
pubkey so the answer is independently verifiable.

Install:
    pip install 'crewai[tools]' 'mcp[cli]'

Usage:
    export OPENAI_API_KEY="sk-..."
    python emem_mcp_geospatial_agent.py

emem itself requires no API key (reads are anonymous, ed25519-signed).
The OpenAI key is only for the LLM driving the crew.
"""

from __future__ import annotations

import os

from crewai import Agent, Crew, Process, Task
from crewai_tools import MCPServerAdapter

EMEM_MCP_URL = os.getenv("EMEM_MCP_URL", "https://emem.dev/mcp")

QUESTION = (
    "Using the emem MCP tools, decide whether the point at latitude "
    "-3.10 longitude -60.02 (Manaus, Brazil) is deforested under EUDR "
    "Article 2(4). Call emem_eudr_dds with a single-point plot of "
    "cocoa (HS 1801), commodity quantity 1000 kg, country BR. Report "
    "the top-level verdict, every per-cell verdict, the fact_cids "
    "supporting the answer, and the responder pubkey. Do not invent "
    "fact_cids; only quote what the tool returns."
)


def main() -> None:
    # MCPServerAdapter is a context manager. Tools are populated on
    # __enter__ and bound to the agent before the task fires.
    server_params = {"url": EMEM_MCP_URL, "transport": "streamable-http"}
    with MCPServerAdapter(server_params) as tools:
        analyst = Agent(
            role="Geospatial Evidence Analyst",
            goal=(
                "Decide EUDR Article 2(4) verdicts on plots and report "
                "every supporting fact_cid for independent verification."
            ),
            backstory=(
                "You are an EU Deforestation Regulation due-diligence "
                "officer. Every claim you make must be backed by a "
                "fact_cid the operator can verify offline at /verify."
            ),
            tools=list(tools),
            verbose=True,
        )

        task = Task(
            description=QUESTION,
            expected_output=(
                "A markdown block with: 'Verdict', 'Per-cell verdicts', "
                "'Fact CIDs', and 'Responder pubkey' headings."
            ),
            agent=analyst,
        )

        crew = Crew(
            agents=[analyst],
            tasks=[task],
            process=Process.sequential,
            verbose=True,
        )

        result = crew.kickoff()
        print(result)


if __name__ == "__main__":
    main()
