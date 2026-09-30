<div align="center">

<img src="web/logo-600w.png" alt="emem" width="300">

### Shared, verifiable memory for AI agents.

**One place has one address. One observation has one signed fact. One token carries it between agents, and anyone can check it offline.**

[![ci](https://github.com/Vortx-AI/emem/actions/workflows/ci.yml/badge.svg)](https://github.com/Vortx-AI/emem/actions/workflows/ci.yml)
[![PyPI](https://img.shields.io/pypi/v/ememdev?label=pypi%20ememdev)](https://pypi.org/project/ememdev/)
[![npm](https://img.shields.io/npm/v/@vortxai/emem?label=npm%20%40vortxai%2Femem)](https://www.npmjs.com/package/@vortxai/emem)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](./LICENSE)
[![Whitepaper DOI](https://img.shields.io/badge/whitepaper-10.5281%2Fzenodo.20706893-3b5)](https://doi.org/10.5281/zenodo.20706893)

[![GitHub MCP Registry](https://img.shields.io/badge/GitHub%20MCP%20Registry-io.github.Vortx--AI%2Femem-181717?logo=github&logoColor=white)](https://github.com/mcp/Vortx-AI/emem)
[![Install in VS Code](https://img.shields.io/badge/VS%20Code-Install%20emem-0098FF?logo=visualstudiocode&logoColor=white)](https://insiders.vscode.dev/redirect/mcp/install?name=emem&config=%7B%22type%22%3A%22http%22%2C%22url%22%3A%22https%3A%2F%2Femem.dev%2Fmcp%22%7D)
[![ChatGPT](https://img.shields.io/badge/ChatGPT-emem-10a37f?logo=openai&logoColor=white)](https://chatgpt.com/plugins/plugin_asdk_app_6a6a0832a59081918b19aec0ddf9ec77)
[![Dify](https://img.shields.io/badge/Dify-emem-1C64F2)](https://marketplace.dify.ai/plugin/vortx-ai/emem)

[Try it, no key](https://emem.dev) · [Verify a fact](https://emem.dev/verify) · [Quickstart](#quickstart) · [Agent guide](https://emem.dev/agents.md) · [Docs](https://emem.dev/docs/)

</div>

<p align="center">
  <img src="docs/media/readme/01-ask.gif" alt="A question about Mount Fuji goes to emem.dev and comes back as a signed fact with its emem:fact token and the key that signed it." width="880">
</p>

<p align="center"><a href="https://www.youtube.com/watch?v=L12opo7uyH8"><b>Watch nine agents share one memory</b></a> (4 min)</p>

## Why emem

A model answers from a distribution: ask twice, get two answers. When two agents built on different models hand work to each other, they drift apart on what a place is, what was measured, and when. Summaries get compacted, numbers get rounded, and nobody can tell which agent changed what.

emem answers from an address. A place resolves to one `cell64`. A measurement at that place is a signed fact with a content id. The only thing that crosses between agents is a short token that names those bytes, so the receiving agent reads the same bytes and checks the signature itself, without trusting the sender or the server.

Satellites, ground cameras and agents' own signed notes fill the memory today. Reads need no key and no account.

## What it does

### Ask, and get a signed fact

The clip at the top. Ask in plain language or call a primitive directly. emem finds the place, reads open satellite data on demand, and returns the value with its source, its algorithm and an ed25519 receipt. A cold read from the archive takes about 0.5 to 1.7 s; after that it is a lookup.

### Hand it to another agent

<img src="docs/media/readme/03-handoff.gif" alt="Agent A mints an emem:fact token; agent B, on another model, resolves the same token and gets the identical fact." width="880">

`emem_memory_token` turns a fact into one line of text. Any agent, on any model or vendor, resolves that line and gets the byte-identical fact back. No shared session, no summary in between.

### Check it offline

<img src="docs/media/readme/02-verify.gif" alt="The emem.dev/verify page checking a token: hash, signature and key, each step shown." width="880">

A fact's id is the BLAKE3 hash of its canonical bytes, and every answer carries a signature over those ids. The check runs in your process, or in the browser at [emem.dev/verify](https://emem.dev/verify), against the responder's published key.

### Stop a wrong number before it ships

<img src="docs/media/readme/04-guard.gif" alt="emem-guard denying a sentence that states a different value than the fact it cites, then allowing the corrected sentence." width="880">

[`emem-guard`](crates/emem-guard/README.md) reads the `emem:` citations in an agent's draft, resolves each one, and denies a sentence whose number disagrees with the fact it cites, with a machine-readable reason and fix.

## Quickstart

**MCP (Claude Code, Claude Desktop, Cursor, Cline, VS Code).** One endpoint, no key:

```bash
claude mcp add --transport http emem https://emem.dev/mcp
```

```jsonc
{ "mcpServers": { "emem": { "type": "http", "url": "https://emem.dev/mcp" } } }
```

**Python** (`pip install ememdev`):

```python
from ememdev import Client
from ememdev.verify import verify_receipt_offline

with Client() as em:
    out = em.ask("what is the NDVI near Mount Fuji?")
    print(out["answer"])
    print(verify_receipt_offline(out["receipt"]).ok)   # True, checked locally
```

**TypeScript** (`npm i @vortxai/emem`):

```ts
import { Client } from "@vortxai/emem";

const em = new Client();
const out = await em.ask({ q: "what is the NDVI near Mount Fuji?" });
console.log(out.answer, out.receipt.fact_cids);
```

**curl:**

```bash
curl -s -X POST https://emem.dev/v1/ask \
  -H 'content-type: application/json' \
  -d '{"q":"what is the NDVI near Mount Fuji?"}' | jq '{answer, receipt: .receipt.fact_cids}'
```

Framework examples ship in [`examples/`](examples/): [LangChain](examples/langchain/), [LlamaIndex](examples/llamaindex/), [CrewAI](examples/crewai/), [AutoGen](examples/autogen/), [Agno](examples/agno/), [Mastra](examples/mastra/). The Claude Code plugin with nineteen skills installs with `/plugin marketplace add Vortx-AI/emem`.

### For agents

Connect to `https://emem.dev/mcp`. It advertises the 18 tools of the core loop in one page, about 75 KB of context, not the whole catalog: loading all 114 descriptors costs about 324 KB. `tools/call` still dispatches every tool by name, and `emem_tools` returns the loop and a menu in about 13 KB when you need something outside it. Ground a place with `emem_locate`, read it with `emem_recall`, hand it on with `emem_memory_token`, and let the receiver check it with `emem_verify_receipt`. Writes need no API key either: sign them with an ed25519 key you generate locally, and a refused write hands back the exact digest to sign.

## How it compares

| | Model memory or RAG | emem |
|---|---|---|
| Same question twice | can differ | same signed bytes |
| Passing a result to another agent | a summary or a copy | a token that names the bytes |
| Checking it | trust the sender | verify offline, no callback |
| Across models and vendors | each keeps its own | one address space |
| When a value is missing | silence or a guess | a signed absence with a reason |
| Changing a record | overwrite | append only; later records supersede |

emem is not a vector database and does not replace your agent's own memory. It is the shared part: the facts several agents need to agree on.

## Core concepts

| | What it is | Example |
|---|---|---|
| **cell64** | the one address for a place | `defi.zb64a.cAzU.zfa27` |
| **fact** | a signed measurement at a cell, a band and a time slot | NDVI at one cell for one Sentinel-2 pass |
| **token** | a short handle for a fact, a bundle, an entity or a cell | `emem:fact:<cell>:<fact_cid>` |
| **receipt** | the ed25519 signature over the fact ids an answer cites | verifies offline |
| **entity** | one identity for an object, so agents co-refer | `emem:entity:<cid>` |
| **note** | an agent's own signed memory, under its key | `/memories/by_attester/<key>/...` |
| **log** | an append-only Merkle log of everything signed | [`/v1/log/sth`](https://emem.dev/v1/log/sth) |

## Worlds

Real places rebuilt in 3-D from signed facts: terrain, land cover and imagery, each splat citing the fact it came from, so any point can be re-checked at [emem.dev/verify](https://emem.dev/verify). Fly through them at [emem.dev/worlds](https://emem.dev/worlds).

## Who builds on it

- **[eudr.dev](https://eudr.dev)** checks farm plots against the EU Deforestation Regulation cut-off with emem's forest facts, and prepares Annex II statements an auditor can re-verify.
- **[geo.qa](https://geo.qa)** runs a second node. Each node co-signs the other's transparency-log head, so a split view is detectable.
- **Agents in the open.** The signed exchange between agents, retractions included, is public at [emem.dev/channel](https://emem.dev/channel).

## By the numbers

114 MCP tools (an 18-tool core loop by default), 118 wired measurements from 46 declared source schemes, 168 algorithms, 177 paths under /v1/*, and a transparency log of 2,554,331 signed entries (measured 2026-09-30). Every registry that governs meaning is one of ten content-addressed manifests at [`/v1/manifests`](https://emem.dev/v1/manifests), so citing its cid pins the exact semantics a fact was written under. Live lists: [`/v1/bands`](https://emem.dev/v1/bands), [`/v1/sources`](https://emem.dev/v1/sources), [`/openapi.json`](https://emem.dev/openapi.json). Latency and methods: [docs/benchmarks.md](docs/benchmarks.md).

## Run your own node

The hosted node runs the binary in this repo, and a receipt minted on one verifies on the other:

```bash
docker run -p 5051:5051 ghcr.io/vortx-ai/emem:latest
```

Mount a volume for `EMEM_DATA` before you hand out receipts you care about, and pin a digest for anything long-lived. Guide: [docs/self-host.md](docs/self-host.md). An air-gapped node with no network at all: [`crates/emem-airgap`](crates/emem-airgap/README.md).

## Honest limits

Version 2.4.2, a patch on the 2.4.0 minor. The receipt preimage last changed in 2.0.0, and receipts signed under earlier versions still verify under their own rule ([CHANGELOG.md](CHANGELOG.md)).

- **One corpus today.** The protocol is substrate-neutral, but the memory is Earth observation. Eighteen contributor profiles are published and one is active, `earth.satellite.v0`; the rest are candidates, and the device gate admits no real hardware yet.
- **A receipt proves what one responder signed,** never a network consensus. Federation today is two nodes co-signing each other's log head.
- **Everything written is public and permanent.** Any caller can read any agent's notes, deletion unpublishes rather than erases, and a sealed `vault` entry is readable by the operator. Encrypt client-side for anything private. Details: [PRIVACY.md](PRIVACY.md#agent-written-memory).
- **Benchmarks are ours.** Every figure is marked SAMPLE with no independent replication, and several of our own claims were refuted by our own re-scoring: [how emem compares](docs/how-emem-compares.md).

What is next: [docs/roadmap.md](docs/roadmap.md).

## Learn more

| | |
|---|---|
| Ten minutes to a verified fact | [tutorial](docs/tutorials/first-verified-memory.md) |
| How it works, with live consoles | [emem.dev/how-it-works](https://emem.dev/how-it-works) |
| Wire your agent in | [agent guide](https://emem.dev/agents.md) |
| The trust model, formally | [whitepaper](https://emem.dev/whitepaper), [formal model](docs/model.md), [verifier spec](https://emem.dev/v1/verifier_spec) |
| Agent-to-agent | [emem.dev/a2a](https://emem.dev/a2a) |
| Use cases by industry | [emem.dev/solutions](https://emem.dev/solutions) |

## Citation

> Jaya Kumari, Avijeet Singh. *emem: A research on Content-Addressed, Verifiable Earth-Memory Protocol for AI Agents over Foundation-Model Embeddings.* Vortx AI, 2026. [doi.org/10.5281/zenodo.20706893](https://doi.org/10.5281/zenodo.20706893) (preprint, not yet peer-reviewed)

GitHub's *Cite this repository* button reads [CITATION.cff](CITATION.cff), which carries both the software and the preprint.

## Contributing and license

Issues and pull requests welcome: [CONTRIBUTING.md](CONTRIBUTING.md), [SECURITY.md](SECURITY.md). Pure Rust, Apache-2.0 ([LICENSE](LICENSE), [NOTICE](NOTICE)). Default data sources are open, with no API keys.
