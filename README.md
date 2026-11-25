<div align="center">

<img src="web/logo-300w.png" alt="emem logo" width="64">

<h1>The memory of the physical world.</h1>

<p><b>Search it. Every answer is signed.</b></p>

<p>Where space meets AI. Ask about any place, field, river, forest or building: emem reads it from orbit and from open sensors, keeps every reading at one address, and signs it, so any AI can read the answer back and check it. When web search misses, space answers. One MCP URL, no key.</p>

[![Install in VS Code](https://img.shields.io/badge/VS%20Code-Install%20emem-0098FF?logo=visualstudiocode&logoColor=white)](https://insiders.vscode.dev/redirect/mcp/install?name=emem&config=%7B%22type%22%3A%22http%22%2C%22url%22%3A%22https%3A%2F%2Femem.dev%2Fmcp%22%7D)
[![Claude plugin](https://img.shields.io/badge/Claude-plugin-D97757)](plugins/emem/)
[![ChatGPT](https://img.shields.io/badge/ChatGPT-emem-10a37f?logo=openai&logoColor=white)](https://chatgpt.com/plugins/plugin_asdk_app_6a6a0832a59081918b19aec0ddf9ec77)
[![GitHub MCP Registry](https://img.shields.io/badge/GitHub%20MCP%20Registry-io.github.Vortx--AI%2Femem-181717?logo=github&logoColor=white)](https://github.com/mcp/Vortx-AI/emem)

[![ci](https://github.com/Vortx-AI/emem/actions/workflows/ci.yml/badge.svg)](https://github.com/Vortx-AI/emem/actions/workflows/ci.yml)
[![PyPI](https://img.shields.io/pypi/v/ememdev?label=pypi%20ememdev)](https://pypi.org/project/ememdev/)
[![npm](https://img.shields.io/npm/v/@vortxai/emem?label=npm%20%40vortxai%2Femem)](https://www.npmjs.com/package/@vortxai/emem)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](./LICENSE)

</div>

## The memory model

emem is a memory, not a search index and not a database. Every memory it holds is defined by five lines:

```text
address  = (cell, band, tslot)            where, what and when
fact_cid = BLAKE3(canonical CBOR(fact))   the memory is named by its own bytes
receipt  = Ed25519 over (cell, fact_cid)  who vouches for it, checkable offline
token    = emem:fact:<cell>:<fact_cid>    any AI reads back the same bytes
log      = append-only tree, witnessed    nothing changes quietly
```

One real memory, a point on the shore of the Gobind Sagar reservoir seen from orbit on 22 October 2026: address (`defi.zb565.yOqU.xApA`, `indices.mndwi`, `20748`), surface read as open water by Sentinel-2 L2A, token `emem:fact:defi.zb565.yOqU.xApA:deqxdntu2clvx2qasongy5bjhvjk7enrdkcyaso65xpg3s2imdza`. The pass of 10 June 2026 at the same address reads bare ground: two memories, one place. Paste a token at [emem.dev/verify](https://emem.dev/verify) and the check runs in your browser.

The same model holds two kinds of memory. **Earth memory** is addressed by place, band and time. **Agent memory** is signed notes that one agent writes in its own namespace and any other agent can read and check. [How emem works](https://emem.dev/how-it-works) walks one request end to end.

## Try it: one question, no key

```bash
curl -s -X POST https://emem.dev/v1/ask -H 'content-type: application/json' \
  -d '{"q":"What is the air quality in Delhi right now?"}' \
  | jq '{answer: (.answer | split("\n")[0]), limits: .direct_answer.limits, fact: .direct_answer.fact_cid, signed_facts: (.receipt.fact_cids | length)}'
```

What came back on 20 November 2026, in 2.3 s:

```json
{
  "answer": "PM2.5 at Delhi: PM2.5 69.80 ug/m^3, on a reading dated 2026-11-20 (today).",
  "limits": [
    "Delhi covers more than one cell. This is the reading at the one cell the name resolves to, not a value over the whole place.",
    "CAMS is a model analysis on a coarse grid, not a street-level monitor."
  ],
  "fact": "olew2lwppg4uazm4sgiiv56qv4p4owphfon3ijqb3bvkqb35lyua",
  "signed_facts": 49
}
```

The answer names its source and says what it cannot tell you. Every fact behind it is signed, and the fact id resolves, for you or any other agent, to the same bytes.

The same for one point on a reservoir shore, from the Sentinel-2 pass of 22 October 2026:

```bash
curl -s -X POST https://emem.dev/v1/recall -H 'content-type: application/json' \
  -d '{"lat":31.43288,"lng":76.46947,"bands":["indices.mndwi"],"tslot":20748}' \
  | jq '.facts[0] | {value, observed_at, scene: .derivation.args[2], surface: .surface_class.label, token: .memory_token}'
```

That reads scene `S2A_MSIL2A_20261022T053241_R105_T43RFQ` and a surface of open water. Ask for `"tslot":20614`, the pass of 10 June, and the same address reads bare ground: the reservoir was low.

## What it remembers

1. **The physical world.** Weather, air, water, land, crops, forests, terrain and buildings from 46 open sources, satellites first. Read when asked, kept at one address.
2. **What agents write.** Notes signed with an agent's own key. Another agent, from another vendor, reads the same bytes and checks who wrote them.
3. **What machines and documents record.** Device traces, lab reports, land records and files, signed where they are made. The bytes stay put; a short name travels and proves any part of them.

## What you do with it

- **Search it.** Ask in words; the value comes back with its source, its date and its limits.
- **Encode where the data is generated, decode with any AI.** The record is signed at its source, and Claude, ChatGPT, Gemini or your own model all read the same bytes.
- **Share it and verify it.** Hand another agent a token instead of a paraphrase. It resolves to byte-identical content, and anyone can check it offline.

## When web search misses, space answers

| | Web search | emem |
|---|---|---|
| Returns | pages that mention the place | a measured value at the place |
| Source | whoever wrote the page | the sensor or model reading, named |
| Freshness | when it was crawled | read on demand from the latest pass or model run |
| Trust | rank | a signature, an append-only log, and a check anyone can run |
| For an agent | text to paraphrase | a value and a token to cite |
| When nothing is known | more pages | a signed absence that says where it looked |

## Connect your agent

```bash
claude mcp add --transport http emem https://emem.dev/mcp     # Claude Code
```

Any MCP host takes the same URL, `https://emem.dev/mcp`: Claude Desktop, ChatGPT, Cursor, VS Code, Cline and Gemini CLI. The Claude Code plugin adds nineteen skills on top: `/plugin marketplace add Vortx-AI/emem`, then `/plugin install emem@emem`. In code: `pip install ememdev` or `npm i @vortxai/emem`.

**For agents.** Connect to `https://emem.dev/mcp`. It advertises the 18 tools of the core loop in one page, about 75 KB of context, not the whole catalog: loading all 120 descriptors costs about 363 KB. For the lightest first contact, `emem_tools` returns the loop and a menu in about 15 KB, and `tools/call` dispatches every tool by name, with or without its `emem_` prefix. To hand records on, prefer a bundle: `emem_memory_bundle` names up to 256 facts in 38 characters.

Then ask in plain words. Each prompt below runs one tool:

| Ask your agent | What emem reads | Tool |
|---|---|---|
| "What is the air quality here right now?" | CAMS analysis: PM2.5, NO2 and ozone | `emem_air` |
| "Will it rain here this week?" | met.no forecast: temperature, cloud, rain and wind (ERA5 past hours through `emem_recall`) | `emem_weather` |
| "Is there open water here, and how often?" | JRC surface-water history since 1984 and Sentinel-1 radar, which sees water through cloud | `emem_water` |
| "Is this cocoa plot deforestation-free under the EUDR?" | the EU forest map and forest loss after 31 December 2020, one signed check per plot | `emem_eudr_dds` |
| "Was there tree-cover loss here since 2020?" | Hansen tree cover and year of loss, ESA WorldCover land class | `emem_forest` |
| "How steep is this slope? What is the soil?" | Copernicus DEM slope and ruggedness, SoilGrids organic carbon and texture | `emem_terrain`, `emem_soil` |
| "How many buildings and roads are in this block?" | Overture buildings, places and road length | `emem_recall` |
| "Read this scanned lab report" | Tesseract OCR, with a receipt that binds the image, the text and the engine | `emem_ocr` |
| "Did this robot run what it says it ran?" | its signed execution trace, checked segment by segment | `emem_trace_verify` |
| "Check the numbers in my draft before I send it" | each cited token resolves, and its value is compared with your sentence | `emem_guard_verdict` |
| "Show the NDVI of this field through the last kharif season" | every clear Sentinel-2 pass in one call, each date with its pixel count (4.2 s for a 4.4 ha field, 13 dates) | `emem_field_series` |
| "Draw the field boundaries in this 1 km box" | Fields of The World outlines with net area and, if asked, the 10 m pixels each field holds | `emem_field_boundaries` |

117 wired measurements from open sources, read on demand. The full list: [emem.dev/tools](https://emem.dev/tools).

## Examples

Real-world tasks where a cited answer matters. Each one uses the same memory; none of them is the product.

| Task | What emem gives | Where |
|---|---|---|
| Water, rain and drought | surface water history, rainfall ranked against thirty years, drought indices, each with its date | `emem_water`, `/v1/rain_series`, `emem_spi` |
| Forests and land change | tree cover, year of loss, canopy change, land class | `emem_forest`, `emem_deforestation_alert` |
| EUDR due diligence | each supplier plot checked against the EU forest map and loss after 31 December 2020, one signed result per plot | [emem.dev/eudr](https://emem.dev/eudr) |
| Agents that cite before they answer | every number in a draft checked against the signed value it cites | `emem_guard_verdict` |
| Documents and devices | lab reports and land records read and signed line by line; device runs checked segment by segment | `emem_ocr`, `emem_trace_verify` |
| Crop and field intelligence | season curves, field boundaries with pixel support, harvest and burning, field by field | [emem.dev/agriculture](https://emem.dev/agriculture) |

## Why the answers hold up

- **Every number traces to its source:** the satellite scene, the band, the method and the registry version are in the record.
- **Nothing changes quietly:** records go into an append-only log ([RFC 6962](https://emem.dev/v1/log/sth)) that two independent operators co-sign. When we find a fault we correct it in the open with a signed erratum.
- **Anyone can check it without us:** each receipt verifies offline (`pip install "ememdev[signing]"`, then `verify_receipt_offline`), or paste a token at [emem.dev/verify](https://emem.dev/verify).
- **Gaps are stated, not filled:** where nothing qualifies, a signed absence says where emem looked and why. Cautions (cloud, a field too small for its pixels) travel with the value.

The long form, with what each record promises and how each promise is checked, is in [docs/settlement.md](docs/settlement.md).

## Choose your path

| You are | Start here |
|---|---|
| Trying it | the command above, or `https://emem.dev/mcp` in any MCP host. Reads need no key. |
| Building an agent or app | [agent guide](https://emem.dev/agents.md), [OpenAPI](https://emem.dev/openapi.json), [examples](examples/) for LangChain, LlamaIndex, CrewAI, AutoGen, Agno and Mastra |
| A compliance or agri team | [EUDR evidence, plot by plot](https://emem.dev/eudr), [crop intelligence](https://emem.dev/agriculture), or [open an issue](https://github.com/Vortx-AI/emem/issues) to talk to us |
| Running your own node | `docker run -p 127.0.0.1:5051:5051 ghcr.io/vortx-ai/emem:latest`, then [docs/self-host.md](docs/self-host.md); an air-gapped build is in [emem-airgap](crates/emem-airgap/README.md) |

## Numbers

120 MCP tools · 177 registered algorithms · 117 wired measurements from open sources, satellites and weather models to buildings and soil · a transparency log of 3.39 million entries co-signed by 2 independent operators · Apache-2.0, pure Rust, runs as one binary.

## Who builds on it

- **[eudr.dev](https://eudr.dev)** checks farm plots against the EU Deforestation Regulation cut-off with emem's forest facts, and prepares Annex II statements an auditor can re-verify.
- **[geo.qa](https://geo.qa)** runs a second node, whose transparency-log head emem co-signs.

## Research

- **ESA Φ-lab and BIFOLD, Berlin, October 2026.** *EMEM: A Content-Addressed, Verifiable Earth-Memory Protocol*, presented at Agentic AI for Earth Observation: [poster, demo and reproduction](https://vortx-ai.github.io/esa_poster/) ([source](https://github.com/Vortx-AI/esa_poster)).
- **Whitepaper.** The protocol, its proofs and its limits: [DOI 10.5281/zenodo.20706893](https://doi.org/10.5281/zenodo.20706893), with the current revision in [docs/whitepaper-v2.md](docs/whitepaper-v2.md).
- **ememdemo.** A site built on emem, with probes that test what it claims: [vortx-ai.github.io/ememdemo](https://vortx-ai.github.io/ememdemo/) ([source](https://github.com/Vortx-AI/ememdemo)).
- **Hugging Face Space.** The same image, running on a Space: [huggingface.co/spaces/vortx-ai/emem](https://huggingface.co/spaces/vortx-ai/emem).

## Use it where you already work

| | |
|---|---|
| **In a chat** | the [emem app in ChatGPT](https://chatgpt.com/plugins/plugin_asdk_app_6a6a0832a59081918b19aec0ddf9ec77) (mention `@emem`), and emem in the Claude directory |
| **In Claude Code** | the [plugin](plugins/emem/), the MCP server plus nineteen skills: `/plugin marketplace add Vortx-AI/emem`, then `/plugin install emem@emem` |
| **In an IDE or MCP host** | Claude Desktop, Cursor, Cline, VS Code and Copilot, Gemini CLI (`gemini extensions install https://github.com/Vortx-AI/emem`), all on one endpoint, `https://emem.dev/mcp` |
| **In a workflow tool** | the verified [Dify plugin](https://marketplace.dify.ai/plugin/vortx-ai/emem) |
| **In code** | the [Python](https://pypi.org/project/ememdev/) and [TypeScript](https://www.npmjs.com/package/@vortxai/emem) clients, REST ([OpenAPI](https://emem.dev/openapi.json)), and [examples](examples/) for LangChain, LlamaIndex, CrewAI, AutoGen, Agno and Mastra |
| **Agent to agent** | [A2A](https://emem.dev/a2a): read the [agent card](https://emem.dev/.well-known/agent-card.json), send a task, verify the signed result |

Per-host configs, and every directory emem is listed in, are on [the reference page](https://emem.dev/reference#client-setup). Writes are signed with an ed25519 key you generate locally; a refused write hands back the exact digest to sign.

## Run it yourself

The hosted node runs the binary in this repo, and a receipt minted on one verifies on the other:

```bash
docker run -p 127.0.0.1:5051:5051 ghcr.io/vortx-ai/emem:latest
```

Mount a volume for `EMEM_DATA`, which holds the node's signing key, before you hand out records you care about, and pin an image digest for anything long-lived ([docs/self-host.md](docs/self-host.md)).

For a machine with no route out, such as a payload computer in orbit, [`emem-airgap`](crates/emem-airgap/README.md) takes custody of files in one directory and writes signed records to another, and `emem-encode` captures the execution evidence it can. Each record says which of the two guarantees it carries. The image is `FROM scratch` with one static binary and no networking or database crate linked, so `--network none` agrees with the binary rather than merely being asked of it; [`quickstart.sh`](crates/emem-airgap/quickstart.sh) goes from nothing to a signed, verified record without a clone or a Rust toolchain.

Independent witnesses co-sign the log head, listed live at [`/v1/log/witnesses`](https://emem.dev/v1/log/witnesses), so a split view is detectable. Federation today is witnessing; fetching facts across nodes is separate, unbuilt work.

## What the checks do not prove

- **A signature says who, never that it is true.** A receipt proves what one responder signed. Source correctness and scientific validity need their own assessment.
- **A token points at bytes held somewhere.** A content id names one fixed record forever; being able to fetch it depends on someone keeping the bytes.
- **A place name resolves to one 10 m cell.** Neighbourhood questions need the area tools, and a first read of a new place can take tens of seconds while emem reads the archives.
- **Time series are sparse.** At one warm cell geo.qa measured 38 NDVI readings over three years, about 12.7 a year: enough for a direction, not for a full phenology curve.
- **One responder signs, not a network.** A receipt never claims consensus.
- **Notes are public and permanent.** Encoding a file never uploads it; publishing a note does. Any caller can read any agent's notes, a sealed `vault` entry is readable by the operator, deletion unpublishes rather than erases, and `memory_supersede` works only in your own namespace. Encrypt client-side for anything private ([PRIVACY.md](PRIVACY.md#agent-written-memory)).

Version 2.4.6, a patch on the 2.4.0 minor. The receipt preimage last changed in 2.0.0, and receipts signed under earlier versions still verify under their own rule ([CHANGELOG.md](CHANGELOG.md)). Next: [docs/roadmap.md](docs/roadmap.md).

## Learn more

| When you want to | Go |
|---|---|
| the long form: what a record promises, A2A and the channel, the agent card, security, every limit | [docs/emem-in-depth.md](docs/emem-in-depth.md) |
| why it matters: robot fleets, satellite tasking, agent swarms, compaction | [emem in depth, what breaks without it](docs/emem-in-depth.md#what-breaks-without-it) |
| see it work in ten minutes | [tutorial](docs/tutorials/first-verified-memory.md), [agent walkthroughs](examples/agent-walkthroughs.md) |
| follow one request end to end, with live consoles | [emem.dev/how-it-works](https://emem.dev/how-it-works) |
| wire your agent in | [agent guide](https://emem.dev/agents.md), [skills](https://emem.dev/skills.md), [connect a client](https://emem.dev/reference#client-setup) |
| put it on a machine | [machines that ask emem where they are](docs/robots.md), [emem-airgap](crates/emem-airgap/README.md), [federation](docs/federation.md) |
| check the trust model, formally | [whitepaper](https://emem.dev/whitepaper), [protocol](docs/protocol.md), [formal model](docs/model.md), [security](docs/security.md), [verifier spec](https://emem.dev/v1/verifier_spec) |
| read the full API | [/openapi.json](https://emem.dev/openapi.json), [/mcp/full](https://emem.dev/mcp/full), [all tools](https://emem.dev/tools), the [wire spec](https://emem.dev/spec.md) |
| use it for evals, and what is unmeasured | [benchmarks with methods](docs/benchmarks.md), [how emem compares](docs/how-emem-compares.md), [roadmap](docs/roadmap.md) |

## Citation

Two artefacts, cited separately: the **software** if you ran it, the **preprint** if you build on the protocol. GitHub's *Cite this repository* button reads [CITATION.cff](CITATION.cff), which carries both.

```bibtex
@software{emem_software,
  title     = {emem: shared, verifiable memory for AI agents},
  author    = {Kumari, Jaya and Singh, Avijeet},
  year      = {2026},
  version   = {2.4.6},
  url       = {https://github.com/Vortx-AI/emem},
  license   = {Apache-2.0},
  publisher = {Vortx AI Private Limited}
}

@misc{emem2026,
  title     = {emem: A research on Content-Addressed, Verifiable Earth-Memory
               Protocol for AI Agents over Foundation-Model Embeddings},
  author    = {Kumari, Jaya and Singh, Avijeet},
  year      = {2026},
  doi       = {10.5281/zenodo.20706893},
  publisher = {Zenodo}
}
```

The preprint is open (CC-BY-4.0) and not yet peer-reviewed.

## Contributing and license

Issues and pull requests welcome: [CONTRIBUTING.md](CONTRIBUTING.md), [SECURITY.md](SECURITY.md). Pure Rust, Apache-2.0 ([LICENSE](LICENSE), [NOTICE](NOTICE)). Default data sources are open, with no API keys and no lock-in. A shared memory is worth more the more agents read and write it; if yours use emem, a star helps other builders find it.

## Content address

Every section above this one is a unit of one signed tree: `emem:tree:gxk3b4vucffuy6l6o25l74uyny`, root `56hhtewsmahxud5u5lvqvulk3iz3tf3rj575kfepsy4diqtjmnyq`, published under the key `k572x7go`. A single section is `emem:tree:gxk3b4vucffuy6l6o25l74uyny#row=<i>`, so another agent can cite one part of this file and anyone can prove it was in the file as published:

```bash
curl -s "https://emem.dev/memories/by_attester/k572x7go/readme/2026-11-25-sizes.md" > index.md
curl -s "https://emem.dev/v1/tree/gxk3b4vucffuy6l6o25l74uyny?row=3" > row.json
python3 tools/verifiers/emem-tokenise-files/tree_proof.py check row.json index.md README.md
```

`index.md` is the signed note at [`/memories/by_attester/k572x7go/readme/2026-11-25-sizes.md`](https://emem.dev/memories/by_attester/k572x7go/readme/2026-11-25-sizes.md). `python3 scripts/readme_tree_check.py` rebuilds the whole tree from this file and compares its root with the index, and CI runs it, so a README edited without re-signing fails the build. The tree changes whenever the README does, and this section is left out of it because it names the tree.
