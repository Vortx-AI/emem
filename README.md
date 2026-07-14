<div align="center">

<img src="web/vortxgola.gif" alt="emem globe, a slowly rotating view of Earth" width="120" />

# emem

**The verifiable memory protocol for the physical world, built for AI agents to cite.**

*Every real-world observation becomes a shared, verifiable Memory Token that persists across long-horizon AI tasks.*

[![License: Apache-2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](./LICENSE)
[![Rust 1.91](https://img.shields.io/badge/Rust-1.91-orange.svg)](https://www.rust-lang.org)
[![MCP: Streamable HTTP](https://img.shields.io/badge/MCP-Streamable%20HTTP-black)](https://emem.dev/mcp)
[![OpenAPI 3.1](https://img.shields.io/badge/OpenAPI-3.1-green)](https://emem.dev/openapi.json)
[![Whitepaper: Zenodo](https://img.shields.io/badge/whitepaper-Zenodo%20DOI-3b5?logo=zenodo&logoColor=white)](https://doi.org/10.5281/zenodo.20706893)
[![Container: ghcr.io](https://img.shields.io/badge/ghcr.io-vortx--ai%2Femem-2496ed?logo=docker&logoColor=white)](https://github.com/Vortx-AI/emem/pkgs/container/emem)

[Walk the memory in 3-D](https://emem.dev/worlds) · [Try it, no key](https://emem.dev) · [Verify a fact](https://emem.dev/verify) · [Agent guide](https://emem.dev/agents.md)

</div>

---

## What is emem

A shared memory of the physical world. Location is the first key: every place on Earth has a stable 64-bit address, and every observation recorded there, an elevation, a temperature, a forest-loss year, is one signed, immutable record at that address. Any agent can read it, any keyholder can add to it, and anyone can check any of it offline. No account to read.

## The Memory Token

```
emem:fact:defi.zb493.xuqA.zcb5f:yqbolgeoycqkvj3zkxukb4bjw4odhpwvfzqo3fbgwf4spk45zala
```

One line: the address of a place plus the fingerprint of one signed observation there. It is 84 characters, about 50 BPE tokens; the full signed record it stands in for is about 1,600. An agent keeps the line and drops the payload. Any agent, any model, any month later resolves the line back to the exact same bytes and re-checks the signature without trusting whoever sent it.

In practice your agent runs four verbs: locate a place, recall its signed facts, reason over them, and cite the tokens in its output. Verification is the receiver's single call.

## When the token earns its keep

**A long task survives its own context window.** The harness compacts, the session ends, the model gets swapped. A paraphrase drifts; the token does not. After compaction it re-hydrates to the exact signed value, signature still checking. Record it once, cite it forever.

**Two agents stop re-deriving each other's work.** Agent A verifies a flood line and leaves a token in the report. Agent B, at another company, on another model, resolves it and proves it is genuine in one call. No shared database, no shared credentials, no "trust me".

**A fleet shares one map it can prove.** Robots and autonomous systems keep landmarks as `emem:entity:` identities and terrain or hazard readings as signed facts at addresses that never drift, shareable across vendors over the same MCP and REST surface agents use, verifiable without trusting the peer that wrote them. Runnable proof: [examples/fleet-memory/](examples/fleet-memory/), two vendors, one landmark, a 206-character handoff, verified offline.

**Technical long-horizon tasks**, the failure modes every agent and robot developer already knows:

| Your problem | What survives |
|---|---|
| Context compaction quietly turns your agent's verified details into paraphrase | the 84-char token outlives every summarization pass and re-hydrates to the exact signed bytes |
| A crash or restart lands mid-task and the transcript is gone | notes hold tokens, not payloads; the restarted agent resumes by resolving, not redoing |
| The model gets swapped or upgraded halfway through the project | the address derives from the bytes, not from who asked; the successor resolves the same tokens identically |
| Subagents fan out and the join step drowns in payload copies | workers receive and return tokens; the join resolves and verifies, contexts stay small |
| "Did I, or anyone, already compute this?" asked on every loop | recall is ensure, not get: what exists is reused (`was_cached`), what is missing is fetched and signed once |
| "Is what I knew last week still valid?" with no cheap way to answer | `/v1/temporal_route` scores per-band staleness: cite it or refetch it, no full re-read |
| A robot reboots, or a unit from another vendor joins the fleet | landmarks are `emem:entity:` identities at drift-free addresses; relocalize by resolving, merge maps by verifying |

**Long-horizon business tasks**, the same survival at business length:

| Task | What the memory does |
|---|---|
| An area watch that runs for months (ISR, disaster response, conservation) | signed change evidence accrues per cell; every alert cites the facts it fired on, and a relieving agent resumes from tokens |
| Autonomous inspection rounds over pipelines, rail, or solar fields | findings land as signed facts at fixed addresses, quarter after quarter; a contractor handover keeps every prior finding citable |
| A growing season managed end to end, planting to harvest | NDVI, soil, and rainfall land as signed facts; "stress began in week 23" resolves to the exact readings next spring |
| A forest baseline that must hold across years of annual reports | canopy and loss-year facts stay citable and re-checkable by any auditor, without trusting the author's laptop |
| Operations that commit real resources on a forecast | the forecast acted on is pinned at decision time (`as_of_signed_at`); "what did we know when we launched" has an answer months later |
| Lending, collateral, and insurance decisions tied to a date | a parcel's state on the origination, policy, or claim date stays citable for years; point-in-time recall replays it without hindsight |

<p align="center">
  <img src="docs/diagrams/png/36-memory-outlives-the-context-window.png" width="820" alt="Memory outlives the context window: as the conversation is compacted turn after turn, payloads fall out of context, the one-line emem:fact token survives, and after compaction it re-hydrates from the shared memory to the exact signed bytes." />
</p>

> **If you are an agent reading this**, four moves cover most sessions. Start at [`https://emem.dev/llms.txt`](https://emem.dev/llms.txt). Connecting to `/mcp` loads the 14 core tools rather than all 89, so call `emem_tools` when you need a capability you cannot see in your list, or connect to `/mcp/full` to have every tool registered up front. Recall with `emem_recall` or ask in plain language with `emem_ask`; every answer comes back signed. Before your context gets compacted or your turn ends, put the `emem:fact:` token for anything you verified into your notes or your final answer; you, your successor, or a different agent entirely can resolve and re-check it later.

## See the memory

The memory is not an abstraction; you can walk through it. Each 3-D world at [emem.dev/worlds](https://emem.dev/worlds) draws one gaussian per cell of signed facts: height, tilt, thickness, and colour are each a measurement. Click any splat to read its values, copy its Memory Token, or re-check its signature at [`/verify`](https://emem.dev/verify). The dense worlds at [emem.dev/splats](https://emem.dev/splats) push the same signed substrate to photoreal, with every splat labelled `measured`, `interpolated`, or `synthesized`, so the invented detail peels off and the signed trust root stays.

<p align="center">
  <img src="docs/media/world-interlaken.gif" width="800" alt="A rotating 3-D world of Interlaken built from signed facts: elevation, Sentinel-2 NDVI, and JRC water recurrence fused per cell, every splat carrying its own fact_cid." />
</p>

Prefer a console? [emem.dev](https://emem.dev) has a live recall on the homepage, and [emem.dev/humans](https://emem.dev/humans) is the whole corpus as an explorable constellation.

## Use it in two minutes

Reading needs no key, no account, no signup.

**MCP** (Claude Code, Claude Desktop, Cursor, Cline; drop into `.mcp.json`):

```jsonc
{ "mcpServers": { "emem": { "type": "http", "url": "https://emem.dev/mcp" } } }
```

**REST** (any language):

```bash
CELL=$(curl -s -X POST https://emem.dev/v1/locate \
  -H 'content-type: application/json' -d '{"q":"Bengaluru"}' | jq -r .cell64)
curl -s -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d "{\"cell\":\"$CELL\",\"bands\":[\"weather.temperature_2m\"]}" | jq '.facts[0].value'
```

**Python**: `pip install ememdev` ([PyPI](https://pypi.org/project/ememdev/)).

<details>
<summary>Copy-paste configs for 12 clients, packaged Claude skills, TypeScript SDK</summary>

| Client | Setup |
| --- | --- |
| Claude Desktop | `examples/claude-desktop.json` |
| Claude Code | `examples/claude-code.mcp.json` |
| Cursor | `examples/cursor.mcp.json` |
| Cline (VS Code) | `examples/cline.mcp.json` |
| Gemini CLI | `gemini extensions install https://emem.dev/gemini-extension.json` |
| ChatGPT (Custom GPT) | `examples/openai-gpt-action.json` |
| LangChain / LlamaIndex / Agno / AutoGen / CrewAI / Mastra | `examples/<name>/` |
| Any MCP client over the standard bridge | `{ "command": "npx", "args": ["-y", "mcp-remote", "https://emem.dev/mcp"] }` |

Packaged Claude skills live under `claude-skills/`; `llms-install.md` is a plain-text install guide an agent can follow by itself. TypeScript SDK: `sdks/emem-ts/` (not on npm yet).
</details>

## Build with it

| Operation | What it means for your agent | Tools |
|---|---|---|
| **Recall** | read memory for a place; a miss fetches, signs, and stores for everyone | `emem_recall`, `emem_locate`, `emem_recall_polygon` |
| **Cite** | one token per fact, or one `emem:bundle:` token for a set | `emem_memory_token`, `emem_memory_bundle` |
| **Verify** | trust a fact without trusting the sender, offline | `emem_verify_receipt`, [`/verify`](https://emem.dev/verify) |
| **Weigh** | every fact says how it was produced; model and human classes carry an in-band `caution`; `deterministic: true` keeps only facts recomputable from raw source | inside every recall |
| **Time travel** | `as_of_tslot` for what was on the ground, `as_of_signed_at` for what the memory knew | flags on every read |
| **Self-check** | disagreement between writers is kept and scored, never averaged away | `emem_memory_contradictions` |

Or skip the menu: `emem_ask` takes a plain-language question and returns a signed answer. Each agent also gets a private signed memory with the six standard file verbs, and any keyholder writes shared facts through `POST /v1/attest`. The full handbook is [emem.dev/agents.md](https://emem.dev/agents.md).

## Why you can trust it

1. A record's id is the blake3 hash of its canonical bytes: change one byte, the id changes, so the id proves the bytes.
2. Every answer carries an ed25519 receipt that verifies offline against the responder's published key. No callback, no account.
3. Every record names its source, its versioned algorithm, and its provenance class, so you know whether a value is recomputable from raw data or trusted through a model or a person.
4. A missing value is a signed absence with a typed reason, never a bare 404.
5. Nothing is overwritten. Later records supersede; disagreement between writers is kept and scored as evidence.
6. An append-only transparency log (RFC 6962) with witness co-signing records every attestation batch, so a responder cannot tell two clients two different histories. Pin a signed tree head from [`/v1/log/sth`](https://emem.dev/v1/log/sth), then prove the log only grew.

The signature proves who attested a record and that the bytes never changed, not that the value is objectively true; confidence, uncertainty, and provenance travel with it. Deeper: [how it works](https://emem.dev/how-it-works) with live consoles, [the formal model](docs/model.md), [the wire spec](https://emem.dev/spec.md).

## Substrates: today and next

**Today: satellite Earth observation.** Open data from ESA, NASA, USGS, and the EU JRC fills the memory on demand: 46 declared sources and 124 wired measurements (live catalogs at [`/v1/sources`](https://emem.dev/v1/sources) and [`/v1/bands`](https://emem.dev/v1/bands)), from elevation and NDVI to weather, forest change, and four open foundation-model embeddings.

**Next: everything else that observes a location.** Nothing in the record, receipt, or token grammar is satellite-specific; any observer with a location and a signing key can join the same attest, recall, cite, verify path. The multi-writer endpoint (`POST /v1/attest`) ships today; written substrate profiles for CCTV and fixed sensors, drones, robot fleets, industrial machines, government registries, and open data programs are roadmap work, tracked with the rest in [docs/roadmap.md](docs/roadmap.md). Location stays the first key for all of them.

## Run your own node

The hosted node runs the exact binary in this repo, and both name the planet the same way, so a receipt minted on one verifies on the other:

```bash
docker run -p 5051:5051 ghcr.io/vortx-ai/emem:latest   # or: cargo run --release --bin emem-server
```

One note worth reading twice: the signing key is your node's identity. Mount a volume for `EMEM_DATA` (or set `EMEM_SECRET_B32`) before you hand out receipts you care about. Full guide: [docs/self-host.md](docs/self-host.md).

Measured on the production node (methods in [docs/benchmarks.md](docs/benchmarks.md)): warm recall p50 2.5 ms, offline verification p50 0.13 ms, 632 requests/s on one node, cold materialize 0.5 to 1.6 s depending on the upstream.

## Honest limits

Version 1.0.0: the wire format, receipt preimage, and address space are settled and will not break under a 1.x. Today it is a single-host deployment (no federation yet), the memory holds thousands of places rather than billions, and it grounds facts about physical places, not arbitrary text. The complete edge list, the staged path to federation, and the open research live in [docs/roadmap.md](docs/roadmap.md).

## Learn more

| What | Where |
|---|---|
| How it works, with live consoles | https://emem.dev/how-it-works |
| The formal memory model and algebra | [docs/model.md](docs/model.md) |
| Agent integration handbook | https://emem.dev/agents.md |
| Limits, roadmap, open research | [docs/roadmap.md](docs/roadmap.md) |
| Benchmarks, with methods | [docs/benchmarks.md](docs/benchmarks.md) |
| Industry use cases | https://emem.dev/solutions |
| Wire spec · OpenAPI (106 paths) · MCP (89 tools) | https://emem.dev/spec.md · [/openapi.json](https://emem.dev/openapi.json) · [/mcp](https://emem.dev/mcp) |
| Live proof in a regulated workflow | https://eudr.dev |
| Companion open model | [TerraGround-Gemma](https://huggingface.co/avijeetsingh1608/TerraGround-Gemma-4-12B-LoRA) |

## Research and citation

> **emem: A research on Content-Addressed, Verifiable Earth-Memory Protocol for AI Agents over Foundation-Model Embeddings.**
> Jaya Kumari, Avijeet Singh. Vortx AI, 2026. Open preprint (Zenodo, CC-BY-4.0; not yet peer-reviewed).
> [doi.org/10.5281/zenodo.20706893](https://doi.org/10.5281/zenodo.20706893)

```bibtex
@misc{emem2026,
  title  = {emem: A research on Content-Addressed, Verifiable Earth-Memory
            Protocol for AI Agents over Foundation-Model Embeddings},
  author = {Kumari, Jaya and Singh, Avijeet},
  year   = {2026},
  doi    = {10.5281/zenodo.20706893},
  publisher = {Zenodo}
}
```

## Contributing and license

Issues and pull requests welcome: [CONTRIBUTING.md](CONTRIBUTING.md), [SECURITY.md](SECURITY.md). Pure Rust, Apache-2.0 ([LICENSE](LICENSE), [NOTICE](NOTICE)); default-build data sources are open, with no API keys and no lock-in. Built by [vortx.ai](https://vortx.ai). A shared memory is worth more the more agents read and write it; if yours use emem, a star helps other builders find it.
