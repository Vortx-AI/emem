# CrewAI agent with emem over MCP

This example connects a CrewAI agent to the emem MCP server. The agent asks
for an EU Deforestation Regulation (EUDR) verdict on one point and reports
the `fact_cid` values behind it.

## What you need

- Python 3.10 or later. We tested with Python 3.12.
- An OpenAI API key for the model. emem reads need no key.

## What you get

A Markdown block with four headings: Verdict, Per-cell verdicts, Fact CIDs,
and Responder pubkey. The point is latitude -3.10, longitude -60.02, in
Manaus, Brazil.

## Install

```bash
pip install 'crewai[tools]' 'crewai-tools[mcp]'
```

`MCPServerAdapter` needs the `mcp` extra of `crewai-tools`. Without it, the
adapter stops and asks on the terminal if it can install `mcp`.

## Run

```bash
export OPENAI_API_KEY="sk-..."
python emem_mcp_geospatial_agent.py
```

To use a different emem node, set `EMEM_MCP_URL`. The default is
`https://emem.dev/mcp/full`.

## What the script does

1. It connects to `https://emem.dev/mcp/full` over Streamable HTTP with
   `crewai_tools.MCPServerAdapter`. It uses `/mcp/full` because
   `emem_eudr_dds` is not one of the 18 core tools on `/mcp`.
2. It makes a crew with one agent, the "Geospatial Evidence Analyst".
3. The task asks the agent to call `emem_eudr_dds` with a one-point plot:
   cocoa (HS 1801), 1000 kg, country BRA.
4. The agent reports the verdict, each per-cell verdict, the `fact_cid`
   values and the responder public key.

## Test status

On 7 November 2026 we installed the packages above (crewai 1.15.36,
crewai-tools 1.15.36, mcp 1.30.0). The script loaded the emem tools and
stopped at the OpenAI call, because we used a test key. We did not run the
model part.

We also called `emem_eudr_dds` with the same plot on the same day:

| field | value on 7 November 2026 |
|---|---|
| `due_diligence_statement.verdict` | `pass` |
| `per_plot_results[0].verdict` | `not_in_scope` |
| the one cell | `defi.zb3dc.zba55.niye`, label `not_in_scope`, 4 `fact_cids` |
| `jrc_forest_2020` at the cell | 0 (not forest at the cut-off) |
| `responder_pubkey_b32` | `777er3yihgifqmv5hmc2wwmyszgddzderzhsx6rex4yoakwomvka` |

The point is in the city. It was not forest on 31 December 2020, so the
deforestation rule does not apply to it.

## Notes

- `/mcp/full` sends its tools in pages. On 7 November 2026 it had 116 tools in
  9 pages, and 8 pages earlier that day. The server cuts a page at about
  45,000 bytes, so the page count and the tools on each page change when the
  server changes. `MCPServerAdapter` loaded only the first page (12 tools with
  9 pages, 13 with 8). That page had `emem_eudr_dds` and `emem_hunt`. It did
  not have `emem_recall` or `emem_verify_receipt`.
- To add a verifier agent with `emem_verify_receipt`, give it a second
  `MCPServerAdapter` on `https://emem.dev/mcp`. That list has
  `emem_verify_receipt`.
- Any model that CrewAI supports can drive the agent. Add an `llm=` argument
  to `Agent`. The script does not set one, so CrewAI uses its default
  OpenAI model.
