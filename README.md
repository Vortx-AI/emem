<div align="center">

<img src="web/vortxgola.gif" alt="emem globe, a slowly rotating view of Earth" width="120" />

# emem

**Shared, verifiable memory for AI agents.**

*Give your agents one memory of the real world they can read, write, and cite. Every fact is signed. Anyone can check it offline, with no account and no trust in the server that served it.*

[![License: Apache-2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](./LICENSE)
[![Rust 1.91](https://img.shields.io/badge/Rust-1.91-orange.svg)](https://www.rust-lang.org)
[![MCP: Streamable HTTP](https://img.shields.io/badge/MCP-Streamable%20HTTP-black)](https://emem.dev/mcp)
[![OpenAPI 3.1](https://img.shields.io/badge/OpenAPI-3.1-green)](https://emem.dev/openapi.json)
[![Whitepaper: Zenodo](https://img.shields.io/badge/whitepaper-Zenodo%20DOI-3b5?logo=zenodo&logoColor=white)](https://doi.org/10.5281/zenodo.20706893)
[![Model: TerraGround-Gemma](https://img.shields.io/badge/%F0%9F%A4%97%20model-TerraGround--Gemma-ffce3a)](https://huggingface.co/avijeetsingh1608/TerraGround-Gemma-4-12B-LoRA)
[![Container: ghcr.io](https://img.shields.io/badge/ghcr.io-vortx--ai%2Femem-2496ed?logo=docker&logoColor=white)](https://github.com/Vortx-AI/emem/pkgs/container/emem)

[Hosted node](https://emem.dev) · [Try it](https://emem.dev/humans) · [Verify a fact](https://emem.dev/verify) · [Whitepaper](https://doi.org/10.5281/zenodo.20706893) · [Companion model](https://huggingface.co/avijeetsingh1608/TerraGround-Gemma-4-12B-LoRA) · [Live proof: eudr.dev](https://eudr.dev) · [Agent guide](https://emem.dev/agents.md) · [Spec](https://emem.dev/spec.md) · [OpenAPI](https://emem.dev/openapi.json)

</div>

---

An AI agent forgets. Between one call and the next, it loses what it learned. Run two agents and they answer the same question twice, from scratch, and you have no way to check either one. When the answer touches the real world, a coordinate, a field, a warehouse, a flood line, the agent guesses or re-derives it every single time.

emem is the memory it is missing. One shared store. Append-only. Every fact signed. When any agent records a fact, that fact exists for every other agent, addressed by its content, and anyone can prove it is genuine without trusting whoever wrote it.

It is a working protocol with a hosted node, a written spec, an open [whitepaper](https://doi.org/10.5281/zenodo.20706893), and a companion open model. Point an MCP client at `https://emem.dev/mcp`. No key needed to read.

> **If you are an agent reading this:** start at [`https://emem.dev/llms.txt`](https://emem.dev/llms.txt). Call `emem_ask` with a plain question, or `emem_recall` with a place. Every answer comes back signed. To hand a fact to another agent, make a memory token with `emem_memory_token`. The other agent checks it on its own.

## Why this exists

Three problems show up the moment you run agents in production.

**They forget.** State lives inside one session. Close it and the knowledge is gone. The next run starts cold.

**They can't share what they know.** An agent at one company learns something true. An agent at another company needs it a month later. There is no common place to leave it, and no way for the second agent to trust it if there were.

**They make things up.** A confident answer about a real place, with no source you can open and check. This is the failure that ends pilots.

Session memory, vector stores, and knowledge graphs help with the first problem for text. None of them give you a shared record of the physical world that any agent can read, any agent can add to, and any agent can verify. That is what emem is.

## The 30-second demo

One agent records a fact about a real place. It gets back a signed memory. It hands a short token to a second agent. The second agent confirms the fact is real, without ever trusting the first agent.

**Agent A records a fact.** `POST /v1/recall` (MCP tool `emem_recall`):

```bash
curl -s -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb493.xuqA.zcb5f","bands":["copdem30m.elevation_mean"]}'
```

```json
{
  "facts": [{
    "band": "copdem30m.elevation_mean", "cell": "defi.zb493.xuqA.zcb5f",
    "tslot": 0, "value": 918.0, "unit": "m", "confidence": 0.95,
    "signed_at": "2026-05-03T17:45:32Z",
    "signer_pubkey_b32": "777er3yihgifqmv5hmc2wwmyszgddzderzhsx6rex4yoakwomvka"
  }],
  "receipt": {
    "request_id": "01KR39HY37333FD3C9PBV0F67B",
    "primitive": "emem.recall", "served_at": "2026-05-08T07:59:08Z",
    "cells": ["defi.zb493.xuqA.zcb5f"],
    "fact_cids": ["yqbolgeoycqkvj3zkxukb4bjw4odhpwvfzqo3fbgwf4spk45zala"],
    "responder_pubkey_b32": "777er3yihgifqmv5hmc2wwmyszgddzderzhsx6rex4yoakwomvka",
    "signature": [254, 85, 234, "..."]
  }
}
```

The place was empty a moment ago. This one call fetched the value from a named source, signed it, stored it, and returned it. It is now in memory for every agent.

**Agent A makes a token.** `emem_memory_token`:

```
memt:defi.zb493.xuqA.zcb5f:yqbolgeoycqkvj3zkxukb4bjw4odhpwvfzqo3fbgwf4spk45zala
```

That string is a portable citation. Agent A drops it into a message, a log, or a report.

**Agent B checks it.** `emem_verify_receipt`, or in a browser at [`/verify`](https://emem.dev/verify):

```json
{ "valid": true, "signer_pubkey_b32": "777er3yihgifqmv5hmc2wwmyszgddzderzhsx6rex4yoakwomvka" }
```

Agent B did not have to trust Agent A. It recomputed the content ID and checked the signature against the responder's public key. That is the entire trust model. No account, no callback, no shared password.

## Try it yourself (no install, no key)

Three requests, straight against the live node. You need `curl` and `jq`.

```bash
# 1. Turn a place into a stable id for one small square of ground.
CELL=$(curl -s -X POST https://emem.dev/v1/locate \
  -H 'content-type: application/json' \
  -d '{"q":"Bengaluru"}' | jq -r .cell64)
echo "$CELL"          # -> defi.zb493.xuqA.zcb5f
```

```bash
# 2. Read one measurement at that place. First ask fetches and signs it; repeats are instant.
curl -s -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d "{\"cell\":\"$CELL\",\"bands\":[\"weather.temperature_2m\"]}" \
  | jq '.facts[0].value'   # -> 27.3
```

```bash
# 3. Ask in plain language. It routes to the right data and signs the answer.
curl -s -X POST https://emem.dev/v1/ask \
  -H 'content-type: application/json' \
  -d '{"q":"find places like Yellowstone","place":"Yellowstone National Park"}' \
  | jq -r '.answer'
```

Explore the live console at [emem.dev](https://emem.dev), or spin the planet in the [geo.qa](https://geo.qa) demo.

## What an agent can do with it

Six operations. Each maps to real tools.

| Operation | What it means for an agent | Tools |
|---|---|---|
| **Recall** | read memory for a place | `emem_recall`, `emem_recall_many`, `emem_recall_polygon`, `emem_locate` |
| **Materialize** | write memory on a miss, automatically | happens inside `emem_recall`: fetch, sign, store, return |
| **Verify** | trust a fact without trusting the sender | `emem_verify`, `emem_verify_receipt`, `emem_memory_token`, `emem_memory_bundle` |
| **Compare** | find places that resemble each other | `emem_state`, `emem_state_multi`, `emem_find_similar`, `emem_triple_consensus` |
| **Predict** | run a forward model, with honesty flags | `emem_jepa_predict`, `emem_jepa_predict_v2` |
| **Self-check** | find where memory disagrees with itself | `emem_memory_contradictions`, `emem_edges_recall`, `emem_diff` |

You do not have to pick. Call `emem_ask` with a plain-language question and it routes to the right operation, then returns a signed answer.

```mermaid
flowchart LR
    A[AI agent] -->|1. recall a place| M{fact in memory?}
    M -->|hit, under 10ms| R[signed fact + receipt]
    M -->|miss| F[materialize: fetch, sign, store]
    F --> R
    R -->|2. compare, predict, self-check| W[model operations]
    R -->|3. memory token memt:| B[another agent, another company]
    B -->|4. verify on its own| V[valid: true]
    R -.enriches.-> C[(shared memory)]
    C -.next agent hits cache.-> M
```

## Memory that survives the handoff

This is the part built for long-running work across many agents and many companies.

A fact recorded today is still there next month, addressed by its content, not by a session that expired. An agent that has never met the agent that wrote it can read it and confirm it is genuine. The confirmation is a local calculation, not a call back to the writer, so it keeps working even if the original agent, service, or company is long gone.

That changes what a multi-party workflow can assume. Company A's agent leaves a signed observation. Company B's agent picks it up in a later step and proves it is authentic before acting on it. A regulator, an auditor, or a third agent months later recomputes the same content ID and gets the same answer. Nobody has to trust a shared database, because there is no shared database to trust. There is a shared memory that each party can check for itself.

Each agent also gets a private memory of its own. emem implements the six commands of the standard agent memory tool: `view`, `create`, `str_replace`, `insert`, `delete`, and `rename`. Writes are locked to one key. A path under `/memories/by_attester/<pubkey>/` only accepts writes signed by that key, so a reader can confirm both who wrote a note and that nobody changed it since. The same signature that proves a satellite reading proves the agent's private memory.

## The more it is used, the better it gets

emem is one shared memory, not a cache per agent. It grows because of what happens on a miss.

When an agent asks about a place that has no fact yet, the responder fetches it, signs it, stores it, and returns it in the same call. The next agent to ask the same question gets an answer in about 10 milliseconds instead of waiting about 180 for a cold fetch. Every question any agent asks makes the memory richer for every agent that comes after. Today the store holds signed facts across roughly 6,400 places and dozens of kinds of measurement. The live count is always at [`GET /v1/corpus_state_stats`](https://emem.dev/v1/corpus_state_stats).

Because every fact is addressed by its content, the same observation recorded by two independent responders lines up automatically. Two sources, one address, and you can see whether they agree.

## How a fact is made and proven

The first request for a place is a **cold** read. Every request after is **warm**.

| Request | What happens on the wire | Latency |
|------|--------------------------|--------:|
| First (cold) | fetch upstream value, sign, save, return | about 180 ms |
| Repeat (warm) | serve the stored, signed fact | under 10 ms |

Cold time depends on the source. About 180 ms is typical, but slow upstreams can take seconds (see [Honest limits](#honest-limits)).

When a value genuinely does not exist, whether it is out of coverage, upstream unreachable, or a source that is not wired here, you still get a signed **absence** carrying a reason a machine can read. Not a `404`, not an empty body. An empty answer is a citable receipt. The memory never promises more than it can sign.

<p align="center">
  <img src="docs/diagrams/png/03-anatomy-of-a-request.png" width="820" alt="Anatomy of a recall: a request resolves to a place, checks the signed store, fetches upstream on a cold miss, signs the result, and returns a fact plus a receipt." />
</p>

Every answer is signed, and you verify it yourself, offline, with no account and no key to manage. The chain, in order:

1. A **fact** is one measurement keyed by place, band, and time.
2. emem packs it in a fixed byte order, so the same reading serializes to the same bytes on every machine.
3. It hashes those bytes with **BLAKE3**. That hash is the `fact_cid`. The fact's address is its own fingerprint, which is what content-addressed means. Change one byte and the id changes, so the id proves the bytes.
4. The responder signs the result with an **ed25519** key, the same kind of key people use to log into servers over SSH. The signed envelope is the **receipt**, and it checks out against the responder's public key alone. No call back to the server.

<p align="center">
  <img src="docs/diagrams/png/10-trust-plane.png" width="820" alt="The trust plane: the exact fields a responder signs (domain tag, request id, served-at time, primitive, cells, fact_cids), hashed with BLAKE3 and signed with ed25519 into a receipt, verifiable offline against the responder public key from /.well-known/emem.json." />
</p>

Paste any `fact_cid` into [emem.dev/verify](https://emem.dev/verify), or open `https://emem.dev/verify/<fact_cid>` directly. Your browser pulls the bytes, re-derives the hash, and checks the signature locally. Nothing leaves the page.

<details>
<summary>Verify a receipt in 4 curl commands</summary>

```bash
# 1. Get a signed receipt for a real fact.
curl -s -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb5c4.guxe.nuxe","bands":["copdem30m.elevation_mean"]}' > receipt.json

# 2. Read out the fact's content id.
jq -r '.receipt.fact_cids[0]' receipt.json

# 3. Re-derive the hash and check the ed25519 signature.
curl -s -X POST https://emem.dev/v1/verify_receipt \
  -H 'content-type: application/json' --data @receipt.json | jq .

# 4. Pin who signed it, and which source binary they were running.
curl -s https://emem.dev/.well-known/emem.json \
  | jq '{responder_pubkey_b32, operator_attestation}'
```
</details>

<details>
<summary>The exact bytes that get signed</summary>

The signed input is domain-separated and length-prefixed: `blake3("emem.preimage.v1" || "receipt" || tagged(request_id, served_at, primitive, cells[], fact_cids[], ...))`, so no two different responses can ever share signed bytes. The `fact_cids` sit in an RFC 6962 merkle tree (duplicate leaves rejected) alongside a proof. A `preimage_version` field selects the rule, so older receipts still verify.

The `operator_attestation` in `/.well-known/emem.json` binds the running binary's BLAKE3 hash to its git commit and build time, signed, so you can confirm the live node runs the source it claims.
</details>

One honest limit on the trust model. The signature covers the facts and the places served. It does not cover the original question text or the choice of which place to read. It proves the responder signed these facts at these places. Whether those facts actually answer the question is the calling agent's job. Check `selected.is_high_confidence` from `emem_locate` before you trust a place-based answer.

## The memory links facts and improves

A plain vector store, the kind agents use for memory, saves a note and searches it later. emem does that too, and three things a plain store does not.

- **It flags disagreement, with a score.** When several sources sign different values at the same place, band, and time, `emem_memory_contradictions` keeps them all and scores the spread. Your agent gets one number to threshold on: ignore a hairline gap, escalate a real one. Two sources reporting 12% versus 31% forest loss at one place surface as a scored gap the agent can branch on, instead of an average that quietly splits the difference.
- **Facts carry typed links.** `emem_edges_recall` reads a fact's signed connections (`relates_to`, `supersedes`, `disagrees_with`), bounded by time. A newer, better reading does not silently overwrite the old one. It supersedes it, and the trail stays.
- **It re-derives a fact when better evidence arrives.** When a newer reading or a `disagrees_with` link lands, emem re-derives that one fact. Nothing rewrites silently, so the shared memory sharpens as more agents read and write against it.

## An open model that reads the memory

You do not need it to use emem. **[TerraGround-Gemma](https://huggingface.co/avijeetsingh1608/TerraGround-Gemma-4-12B-LoRA)** is an optional open model that turns the memory into plain answers. It is a small fine-tuning layer on Google's [Gemma 4 12B](https://huggingface.co/google/gemma-4-12B-it) that reads a place's signed record and answers grounded questions about it, and plans the tool calls an agent needs. It is trained to say "not enough data" when the evidence does not support an answer, the same honesty as a signed absence.

It was trained on 4,164 examples across 1,286 varied places, and it builds on the Tessera foundation model ([arXiv:2506.20380](https://arxiv.org/abs/2506.20380)). The adapter is under Gemma terms; the surrounding code is Apache-2.0.

<p align="center">
  <img src="docs/diagrams/png/31-encoders-in-orbit-decoders-on-ground.png" width="820" alt="Encoders in orbit, decoders on the ground: sensors feed four foundation models whose signed fingerprints emem stores; a companion model decodes them into grounded answers. Different models read the same place differently, so disagreement is informative." />
</p>

## Connect your assistant

Reading needs no key. Point your client at `https://emem.dev/mcp` and it gets all 81 tools.

```jsonc
// .mcp.json at your project root. Works in Claude Code, Claude Desktop, Cursor, Cline.
// Here "http" is MCP's transport type, not the URL scheme. Copy it exactly.
{ "mcpServers": { "emem": { "type": "http", "url": "https://emem.dev/mcp" } } }
```

**Any MCP client over the standard bridge:**
```jsonc
{ "command": "npx", "args": ["-y", "mcp-remote", "https://emem.dev/mcp"] }
```

<details>
<summary>Copy-paste configs for 12 clients (all under <code>examples/</code>)</summary>

| Client | Setup |
| --- | --- |
| Claude Desktop | `examples/claude-desktop.json` |
| Claude Code | `examples/claude-code.mcp.json` |
| Cursor | `examples/cursor.mcp.json` |
| Cline (VS Code) | `examples/cline.mcp.json` |
| Gemini CLI | `gemini extensions install https://emem.dev/gemini-extension.json` |
| ChatGPT (Custom GPT) | `examples/openai-gpt-action.json` |
| LangChain | `examples/langchain/` |
| LlamaIndex | `examples/llamaindex/` |
| Agno | `examples/agno/` |
| AutoGen | `examples/autogen/` |
| CrewAI | `examples/crewai/` |
| Mastra | `examples/mastra/` |

Native SDKs: Python `ememdev` (`sdks/emem-py/`) and TypeScript `@emem/client` (`sdks/emem-ts/`). PyPI and npm publication is pending; install from the repo today, for example `pip install ./sdks/emem-py`.
</details>

## Surfaces and primitives

One binary, one core. The same handlers answer MCP tool calls and plain REST, so an agent and a `curl` script see identical facts and identical receipts. Reads need no auth. Every write lands in an append-only, signed log.

| Surface | Endpoint | What you get |
| --- | --- | --- |
| **MCP** | `https://emem.dev/mcp` | JSON-RPC 2.0 over Streamable HTTP. 81 tools: 10 core plus 71 extended. |
| **REST** | `/v1/*` | 93 documented paths, described by [`/openapi.json`](https://emem.dev/openapi.json) (OpenAPI 3.1). |

Every MCP tool ships a `when_to_use` line and four hint flags (read-only, destructive, idempotent, open-world), so a planner picks the right tool without guessing. `tools/list` returns all 81; pass `{"tier":"core"}` for the 10 you need most.

<details>
<summary><b>160 named algorithm recipes</b>: a formula an agent can read, with its source and honest accuracy</summary>

Recipes like `flood_risk@2`, `heat_index@2`, `carbon_sink_score@1`, and `eudr_compliance@1` live in a content-addressed registry. Each one carries a `formula` the agent can read and apply, the `inputs` it needs, a `when_to_use` note, a `citation` to the primary source, an honest `accuracy_band`, and `learned_from` provenance for every tuned number. Recipes with an evaluation tree run in-process against the recalled facts and return a signed composite value that anyone with the same inputs reproduces. Browse [`/v1/algorithms`](https://emem.dev/v1/algorithms).
</details>

<details>
<summary><b>Machine discovery</b>: emem publishes its own map so an agent can read the memory before it acts</summary>

| Surface | Serves |
| --- | --- |
| `/v1/agent_card` | agent-readable capability card |
| `/v1/tools` | all 81 tools with `when_to_use` and hints |
| `/v1/algorithms` | 160 named algorithm recipes |
| `/v1/topics` | topic-grouped measurements |
| `/v1/manifests` | the four ids that pin measurements, algorithms, sources, schema |
| `/.well-known/{emem,agent,mcp,ai-plugin}.json` | discovery plus `operator_attestation` |
| `/agents.md` · `/spec.md` · `/openapi.json` | agent loop, wire spec, REST schema |
| `/llms.txt` · [`/llms-full.txt`](https://emem.dev/llms-full.txt) | plaintext catalog for LLMs |
| `/v1/stream` | signed live heartbeat of memory state |
</details>

## See it in the wild

Three surfaces, three jobs. Pick by what you are trying to do.

| Surface | Reach for it when | What it is |
|---|---|---|
| **[geo.qa](https://geo.qa)** | you are new and want to see it | a globe you spin and explore, no docs |
| **[emem.dev](https://emem.dev)** | you are building against a live node | the hosted responder, plus a try-it drawer and machine-discovery surfaces |
| **[eudr.dev](https://eudr.dev)** | you want proof it holds in a regulated workflow | a compliance agent whose every citation is a signed fact |

**eudr.dev is the flagship proof.** It is an independent compliance agent built on emem for the EU Deforestation Regulation. Every paragraph it quotes resolves to a signed `fact_cid` served from the emem responder: the forest baseline, the clearing history for a plot, the coverage check. Its output is a signed, Annex II-shaped Due Diligence Statement, and that statement is what clears customs. A customs officer, an auditor, or a rival lab can pull the exact bytes behind any claim from any node and re-check the signature offline. This is the whole idea, working in a setting where being wrong has a cost.

<p align="center"><img src="docs/diagrams/eudr.png" alt="EUDR end to end on emem: a plot geometry resolves to a forest baseline and clearing history, packaged as a signed Due Diligence Statement handle that clears customs." /></p>

<details>
<summary>Where a shared, signed memory earns its keep</summary>

The mechanism is the same every time: a stable handle for a real place, a fingerprint for the exact bytes, source disagreement kept and scored, and time-bound recall so you can ask both "what was on the ground" and "what did we know" on a given date.

- **Defense and intelligence**: many agents read and write one address space and cross-check each other, and where they disagree is recorded, not hidden.
- **Disaster response**: flood extent, burn severity, and landslide signals arrive signed and time-stamped, so field teams and models work off one verifiable picture.
- **Carbon accounting**: every measurement is auditable evidence with full provenance, from forest baseline to loss year.
- **Agriculture**: field boundaries, vegetation trends, soil, and crop-stress scans per place.
- **Insurance**: a signed receipt for the state of a location on a date settles what a policy covered.
- **Supply-chain compliance**: the eudr.dev workflow generalized, proving where a commodity came from with citations a regulator can verify.

<table>
  <tr>
    <td><img src="docs/diagrams/15-defense-geoint.svg" alt="Defense and intelligence: many agents sharing one signed address space and recording where they disagree" /></td>
    <td><img src="docs/diagrams/16-disaster-response.svg" alt="Disaster response: signed, time-stamped flood, burn, and landslide facts for field teams" /></td>
  </tr>
  <tr>
    <td><img src="docs/diagrams/27-precision-agriculture.svg" alt="Agriculture: signed field boundaries, vegetation trends, and crop-stress signals per place" /></td>
    <td><img src="docs/diagrams/29-eudr-supply-chain.svg" alt="Supply-chain compliance: commodity provenance backed by signed, verifiable facts" /></td>
  </tr>
</table>

Full set: [32 protocol and industry diagrams](https://emem.dev/docs/diagrams).
</details>

## Where the facts come from

This is the layer that fills memory on a miss. It sits below everything above, on purpose. An agent almost never touches it directly.

The raw material is the public record of the physical world: open satellite and earth-observation data from agencies like ESA, NASA, USGS, and the EU's Joint Research Centre, plus open reference datasets for land cover, water, terrain, weather, and rainfall. **46 declared sources** feed the memory, including Sentinel-2, Landsat/HLS, Copernicus DEM, JRC Global Surface Water, Hansen forest change, ESA WorldCover, Overture Maps, Fields of The World, CHIRPS rainfall, and more. **124 measurements are wired and fetch on demand**; a handful that are declared but not wired here return a typed "not available" rather than pretending.

Four open foundation models (Tessera, Clay v1.5, Prithvi-EO-2, Galileo) turn raw readings into the numeric fingerprints used in the Compare operation. emem runs them for you and signs what they produce, so there is no inference stack to stand up. `emem_state` and `emem_state_multi` return the signed fingerprints; `emem_triple_consensus` runs three of them together and reports where they agree and where they disagree, so a disagreement is a signal rather than an average that hides it.

<p align="center">
  <img src="docs/diagrams/png/06-memory-vs-stac.png" width="820" alt="Memory versus catalog: the classic search, window, reproject, mosaic, cache pipeline versus emem's locate-then-recall over one signed address space." />
</p>

emem's own job is the last step: sign the result, store it, and give it a stable address so it can be recalled and checked forever. The default build reads only open sources. No API keys, no operator credentials.

Individual measurements carry plain names like `indices.ndvi` (a greenness measure), `copdem30m.elevation_mean` (ground height), and `weather.temperature_2m` (air temperature). An agent rarely names them. It asks a question or names a place, and the router picks the right measurements. Every value is a point sample read from the named source in that source's own coordinate system; emem never resamples data onto a new grid or invents resolution the source does not have.

<details>
<summary>The four handles that address everything</summary>

All four are deterministic: the same input produces the same string on any node.

| Handle | What it names | Wire form |
| --- | --- | --- |
| `cell64` | one ground cell, about 9.55 m at the equator; ordered so string-near ids are physically near | `defi.zb493.xuqA.zcb5f` |
| `tslot` | a 64-bit time slot | `t.`-prefixed base32 |
| `cid` | fingerprint of a fact's bytes (32-byte BLAKE3); change one byte, the id changes | `72wdchiyurfrjxz7zat6kor7gjnvsn564fbrzjkmlhagoy4rrh4a` |
| `vec` | a place's full state vector | 12-byte prefix in receipts; full vector via `recall` |

The workspace is 16 Rust crates. `emem-server` lives in `crates/emem-cli/src/bin/emem-server.rs`; addressing and signing in `emem-core`, `emem-codec`, `emem-fact`, `emem-attest`; the two API faces in `emem-mcp` and `emem-api-rest`. Clients under `sdks/`, drop-in configs under `examples/`, wire spec and cross-language test vectors under `spec/`, diagrams under `docs/diagrams/`.
</details>

## Run your own node

The hosted node at [emem.dev](https://emem.dev) runs the exact binary in this repo. Self-hosting is not a fork or a cut-down build. You run the same thing, and it names the planet the same way.

```bash
# multi-arch image, anonymously pullable, the fastest way to a local node
docker run -p 5051:5051 ghcr.io/vortx-ai/emem:latest
```

```bash
# or build from source (needs Rust 1.91+; the first release build takes a few minutes)
cargo run --release --bin emem-server
```

No required env vars. It boots empty and fetches on the first request, same as the hosted node. Point a client at `http://localhost:5051/mcp` and reads work with no auth.

| Env var | Default | Effect |
| --- | --- | --- |
| `EMEM_BIND` | `0.0.0.0:5051` | listener address and port |
| `EMEM_DATA` | `./var/emem` | data dir; set to `:memory:` for a node that persists nothing |

What makes your node and emem.dev the same memory is not a network link. It is the addressing math. Both derive the same id for a place and the same id for a reading, so a receipt minted on one verifies against the other, offline. Paste a `fact_cid` from emem.dev into your local node and you pull the identical bytes.

<p align="center">
  <img src="docs/diagrams/architecture.png" width="760" alt="emem one-binary architecture: MCP and REST handlers over a single core, fronting the upstream sources, writing to an append-only signed log, pinning four content-addressed manifests per answer." />
</p>

## Honest limits

emem is version 0.1.0. What it does not do yet, so you can plan around it:

- **Single host.** No federation, no global routing, no SOC 2 yet. One responder, one signing key.
- **Thousands of places, not billions.** The memory grows every day it is used, but it is early. Check the live count before you assume coverage.
- **It grounds facts about physical places,** not arbitrary text. It is not a general-purpose citation store for any document.
- **The learned predictor is an honest baseline.** `jepa_predict_v2` returns the last known reading until its dynamics head is fully trained, and says so.
- **Some foundation-model fingerprints are sidecar-gated on the hosted node** today. A cold place returns a signed absence for those, so `triple_consensus` runs partial when cold. Tessera fetches on demand.
- **Upstream rate limits.** Some sources are rate-limited or slow to fetch (one land-surface-temperature source takes about 30 seconds per place).
- **No sub-meter imagery** in the default build, and no notebook UI. Drive it from a notebook against REST or MCP.

## Where it is going

emem is a protocol, not a single service. The end state is a federation of independent responders that resolve the same ids byte-for-byte, cross-cite each other, and record where they disagree. **None of the multi-host federation routing ships in 0.1.0.** What ships today is the machinery it stands on: content addressing, signed receipts, typed temporal links, cross-source disagreement scoring, and an offline refinement loop. Federate later; the fact ids will not move. Near-term work is tracked in [issues](https://github.com/Vortx-AI/emem/issues).

<p align="center"><img src="docs/diagrams/federation.png" width="720" alt="Roadmap: many independent emem responders sharing one address space, resolving the same content ids byte-for-byte and recording where they disagree." /></p>

## Research and citation

The protocol is written up in an open whitepaper:

> **emem: A Content-Addressed, Verifiable Earth-Memory Protocol for AI Agents over Foundation-Model Embeddings.**
> Jaya Kumari, Avijeet Singh. Vortx AI, 2026. Open preprint (Zenodo, CC-BY-4.0; not yet peer-reviewed).
> [doi.org/10.5281/zenodo.20706893](https://doi.org/10.5281/zenodo.20706893)

```bibtex
@misc{emem2026,
  title  = {emem: A Content-Addressed, Verifiable Earth-Memory Protocol
            for AI Agents over Foundation-Model Embeddings},
  author = {Kumari, Jaya and Singh, Avijeet},
  year   = {2026},
  doi    = {10.5281/zenodo.20706893},
  publisher = {Zenodo}
}
```

## Resources

| What | Where |
|---|---|
| Whitepaper (Zenodo, CC-BY-4.0) | https://doi.org/10.5281/zenodo.20706893 |
| Companion model (Hugging Face) | https://huggingface.co/avijeetsingh1608/TerraGround-Gemma-4-12B-LoRA |
| Hugging Face Space | https://huggingface.co/spaces/vortx-ai/emem |
| Agent loop · Wire spec | https://emem.dev/agents.md · https://emem.dev/spec.md |
| LLM catalog (plaintext) | https://emem.dev/llms.txt · https://emem.dev/llms-full.txt |
| OpenAPI 3.1 (93 REST paths) | https://emem.dev/openapi.json |
| MCP endpoint (81 tools) | https://emem.dev/mcp |
| In-browser receipt verifier | https://emem.dev/verify |
| Container (multi-arch) | `ghcr.io/vortx-ai/emem:latest` |
| Issues · Security | https://github.com/Vortx-AI/emem/issues · avijeet@vortx.ai |

## Contributing

Issues and pull requests welcome. See [CONTRIBUTING.md](CONTRIBUTING.md), [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md), and [SECURITY.md](SECURITY.md). Pure Rust.

## License

Apache-2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE). Default-build data sources are open (Copernicus DEM, JRC Global Surface Water, Hansen forest change, ESA WorldCover, Overture, Fields of The World, and more), with no API keys and no lock-in. Built by [vortx.ai](https://vortx.ai). Contact avijeet@vortx.ai.
