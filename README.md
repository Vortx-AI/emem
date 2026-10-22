<div align="center">

<img src="web/logo-300w.png" alt="emem logo" width="72">

<h1>Encode where the data lives. Decode with any AI.</h1>

<p><b>emem is shared, verifiable memory for machines and AI. The proof travels; the files stay where they are.</b></p>

<p><a href="https://emem.dev">Try it, no key</a> · <a href="#quickstart">Quickstart</a> · <a href="#memory-that-changes-without-losing-its-history">A worked example</a> · <a href="https://emem.dev/verify">Verify a token</a> · <a href="https://emem.dev/agents.md">Agent guide</a> · <a href="https://emem.dev/docs/">Docs</a></p>

[![ci](https://github.com/Vortx-AI/emem/actions/workflows/ci.yml/badge.svg)](https://github.com/Vortx-AI/emem/actions/workflows/ci.yml)
[![Toolweather](https://toolweather.toolweather.workers.dev/badge/io.github.Vortx-AI/emem.svg)](https://toolweather.pages.dev/servers/io.github.Vortx-AI--emem/)
[![PyPI](https://img.shields.io/pypi/v/ememdev?label=pypi%20ememdev)](https://pypi.org/project/ememdev/)
[![npm](https://img.shields.io/npm/v/@vortxai/emem?label=npm%20%40vortxai%2Femem)](https://www.npmjs.com/package/@vortxai/emem)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](./LICENSE)
[![Whitepaper DOI](https://img.shields.io/badge/whitepaper-10.5281%2Fzenodo.20706893-3b5)](https://doi.org/10.5281/zenodo.20706893)

[![ChatGPT](https://img.shields.io/badge/ChatGPT-emem-10a37f?logo=openai&logoColor=white)](https://chatgpt.com/plugins/plugin_asdk_app_6a6a0832a59081918b19aec0ddf9ec77)
[![Claude plugin](https://img.shields.io/badge/Claude-plugin-D97757)](plugins/emem/)
[![GitHub MCP Registry](https://img.shields.io/badge/GitHub%20MCP%20Registry-io.github.Vortx--AI%2Femem-181717?logo=github&logoColor=white)](https://github.com/mcp/Vortx-AI/emem)
[![Install in VS Code](https://img.shields.io/badge/VS%20Code-Install%20emem-0098FF?logo=visualstudiocode&logoColor=white)](https://insiders.vscode.dev/redirect/mcp/install?name=emem&config=%7B%22type%22%3A%22http%22%2C%22url%22%3A%22https%3A%2F%2Femem.dev%2Fmcp%22%7D)
[![Dify](https://img.shields.io/badge/Dify-emem-1C64F2)](https://marketplace.dify.ai/plugin/vortx-ai/emem)

</div>

<p align="center"><img src="docs/media/readme/20-encode-decode.webp" alt="Three panels. Your data: files, machines and archives, with source files kept local. Each sends a signed record into shared memory, where records link by identity, history and derivations. Any AI, whether Claude, ChatGPT or your own agent, resolves the same record and verifies it. Caption: one reference, the same record, checked independently." width="880"></p>

## Trust without moving the file

Today, to trust a piece of data you usually have to hold it. A satellite downlinks the whole capture, a lab emails the spreadsheet, a company uploads its documents to whichever AI is reading them, and every copy is one more place the data can leak or quietly change.

emem splits the job in two.

- **Encoding happens at the source, and stays private.** A file server, a lab, a camera or a payload computer hashes what it holds into units and signs a small record under its own key. The bytes do not have to leave. What leaves is a commitment to them: a content id, a Merkle root, a signature.
- **emem is the decoding half.** Any model, in Claude, in ChatGPT or in your own code, resolves a short token back to that exact record, checks who signed it, and proves that one unit belongs to the whole, without trusting the sender, and without trusting emem.

A token lets a reader resolve the published record and check content they are given against it. It does not reveal what you kept: a tree token over a private file exposes the units you chose to publish and nothing else.

## What crosses, and what stays

| Source | Where it is encoded | What crosses | What stays | Status |
|---|---|---|---|---|
| **Documents, datasets, model weights, code** | the tokeniser on [emem.dev](https://emem.dev) (in your browser: code by definition, safetensors by tensor, zip and pptx by entry, PDF by page) or [`tree_proof.py`](plugins/emem/skills/emem-tokenise-files/) on your machine | one `emem:tree:` token: each unit's BLAKE3 folded into a Merkle root, in an index you sign | every unit you do not choose to publish | shipped |
| **Your own algorithm** | wherever you run it | `emem_derive`: your value, the signed facts it read, and an optional `code_cid`, the hash of your code, all signed by your key | the code. emem never runs it; it pins it by hash | shipped; emem re-runs only pure operations (`delta`, `mean`, `sum`) |
| **A payload on a satellite, drone, camera or robot** | [`emem-airgap`](crates/emem-airgap/README.md) on the device, with no network; the `emem-encode` sidecar adds an execution trace | a signed custody record: these bytes, this name, this size, at this time, under this key | the payload | the encoders ship (arm64 and amd64); the public device gate admits no real hardware yet |
| **Open Earth archives** | emem's own readers of registered archives | signed facts, each naming its source bytes | nothing private: the archives are public, so anyone can recompute | shipped, the first corpus |

What each check proves is stated with it. A custody record says bytes arrived, not that the sensor was calibrated. A tree proves a unit was in the file you signed, not that the file is true. A `code_cid` says which code you claim produced a value; running arbitrary private code in a sandbox is not built.

## Quickstart

No account and no API key to read.

**1. Connect an MCP host** (Claude Code here; [every other host](https://emem.dev/reference#client-setup)):

```bash
claude mcp add --transport http emem https://emem.dev/mcp
```

**2. Prove one part of a file without the rest of it.** This repository's `LICENSE` is published as a signed tree of six 2 KB units, so you can try the file path with nothing to set up. From a clone:

```bash
curl -s "https://emem.dev/memories/by_attester/k572x7go/readme-example/license-tree.md" > index.md
curl -s "https://emem.dev/v1/tree/xrnco3igl6j2j4kehg4z33ggk4?row=3" > row.json
python3 plugins/emem/skills/emem-tokenise-files/scripts/tree_proof.py check row.json index.md LICENSE
```

The index names every unit's hash and the root; the row carries unit 3 and its audit path. The check confirms bytes 6144 to 8191 are in the file the key `k572x7go` signed, by hashing those bytes and walking three steps to the root. For your own file, `tree_proof.py build report.md` cuts it into units and prints the index offline; sign and publish it under your own key ([the skill](plugins/emem/skills/emem-tokenise-files/SKILL.md) walks through it). The file never leaves your machine.

**3. Read a signed measurement and check it offline** (curl, jq, Python):

```bash
curl -fsS https://emem.dev/v1/recall -H 'content-type: application/json' \
  -d '{"place":"Cairo","bands":["copdem30m.elevation_mean"]}' > fact.json
jq '.facts[0] | {cell, value, unit, memory_token}' fact.json

jq '{token: .facts[0].memory_token}' fact.json \
  | curl -fsS https://emem.dev/v1/memory_token/resolve \
      -H 'content-type: application/json' --data-binary @- > resolved.json
```

```python
# pip install "ememdev[signing]"
import json
from ememdev.verify import verify_receipt_offline

verdict = verify_receipt_offline(json.load(open("resolved.json"))["receipt"])
print(verdict.ok, verdict.why)   # True, checked in this process with no network call
```

The check uses the public key the receipt carries. To know it was emem that signed, pin emem's key from [`/.well-known/emem.json`](https://emem.dev/.well-known/emem.json), or check the token at [emem.dev/verify](https://emem.dev/verify).

<details>
<summary>Or just ask, from the shell, Python or TypeScript</summary>

```bash
curl -s -X POST https://emem.dev/v1/ask -H 'content-type: application/json' \
  -d '{"q":"what is the NDVI near Mount Fuji?"}' | jq '{answer, receipt: .receipt.fact_cids}'
```

```python
# pip install "ememdev[signing]"
from ememdev import Client
from ememdev.verify import verify_receipt_offline

with Client() as em:
    out = em.ask("what is the NDVI near Mount Fuji?")
    print(out["answer"], verify_receipt_offline(out["receipt"]).ok)
```

```ts
// npm i @vortxai/emem
import { Client } from "@vortxai/emem";

const out = await new Client().ask({ q: "what is the NDVI near Mount Fuji?" });
console.log(out.answer, out.receipt.fact_cids);
```

There is no rule for turning a tool name into a REST path, so do not guess one: `emem_memory_search` answers at `POST /v1/memory/search` and `emem_verify_receipt` at `POST /v1/verify_receipt`. The authority is [`/openapi.json`](https://emem.dev/openapi.json); over MCP, call the tool by name.

</details>

**For agents.** Connect to `https://emem.dev/mcp`. It advertises the 18 tools of the core loop in one page, about 75 KB of context, not the whole catalog: loading all 115 descriptors costs about 324 KB. For the lightest first contact, `emem_tools` returns the loop and a menu in about 13 KB, and `tools/call` dispatches every tool by name, with or without its `emem_` prefix. To hand records on, prefer a bundle: `emem_memory_bundle` names up to 256 facts in 38 characters.

## Memory that changes without losing its history

One field, worked end to end on the live node on 2026-10-10. Every token below resolves, and you can check each step yourself.

**1. Name the thing once.** The field gets an identity that stays put while everything measured about it changes:

```text
emem:entity:2kmmk5bflnt5v2g7fh5y5ezqhe     "Wheat field west of Ludhiana", a farm_plot at cell defi.zb560.kUtU.sozo
```

**2. Two observations, each its own record.** NDVI read from Sentinel-2 on two dates. Each reading has its own content id, so a later reading never overwrites an earlier one:

```text
2026-03-12   NDVI 0.893   emem:fact:defi.zb560.kUtU.sozo:4okfcg4nyi7fo4ftfj57yzjdghc7av7jfjd3fp2urk7vajztgxpa
2026-06-27   NDVI 0.158   emem:fact:defi.zb560.kUtU.sozo:hbliam74soj7pmqy7ig67hyyqn7lz6fe7baagv743fzi75p2drpq
```

**3. A result derived from them, and recomputed.** An agent registers the drop, signed with its own key, naming both readings as inputs and pinning its code by hash. Because the operation is a pure `delta`, emem re-runs it over the two signed parents before recording it:

```text
emem:fact:defi.zb560.kUtU.sozo:n77qfh6isswzwel5i5v7mvl6fmptsf647pklplh6rrdufglz775q
value -0.7352330536822305   class deterministic_index   rule canonical_float_equality   ulp_gap 0
```

**4. A correction that keeps the original.** The agent first wrote that the field was harvested on 27 June. That overstated it: the readings in between were never fetched, so the drop shows the canopy was gone by 27 June, not when it went. The agent wrote a corrected note and superseded the first. Readers of the old path now get `superseded_by`, and the original bytes still resolve, so anyone can see what was corrected and why:

```text
/memories/by_attester/k572x7go/readme-example/farm-v1.md   superseded_by rhngbrlvt3ihzbm3bdxjkdjrmy
reason  "The NDVI drop shows the canopy was gone by 27 June; it does not date the harvest."
```

**5. Another agent checks all of it, trusting nobody:**

```bash
curl -s -X POST https://emem.dev/v1/memory_token/resolve -H 'content-type: application/json' \
  -d '{"token":"emem:fact:defi.zb560.kUtU.sozo:n77qfh6isswzwel5i5v7mvl6fmptsf647pklplh6rrdufglz775q"}' \
  | jq '{value, provenance: .provenance.class, inputs: .fact.derivation.args.inputs, recomputed: .fact.derivation.args.recomputation.verified}'
```

It gets the value, the two input tokens to resolve in turn, and the responder's statement that it reproduced the value, all under one receipt it can verify offline. Identity, time, provenance, computation and correction, in one place.

The operations behind those steps, and the rest of what agents do with them:

| Job | How emem does it |
|---|---|
| **Name a thing once** | `emem_entity` mints an identity; `emem_entity_resolve` and `emem_entity_link` converge different phrasings onto it. |
| **Read what is known, at a place or across an area** | `emem_recall`, `emem_recall_polygon` and `emem_grid` fetch and sign on a miss; `emem_ask` takes plain language. [168 published recipes](https://emem.dev/v1/algorithms) combine readings into scores for flood risk, heat, crop condition and more. |
| **Follow a place through time** | `emem_trajectory` lists the dated readings stored at a cell; `emem_diff` and `emem_compare` contrast two; `emem_field_series` follows an area through a season, with a change map. |
| **Derive, so others can recompute** | `emem_derive` records your result over signed facts and re-runs pure operations before recording. |
| **Correct without erasing** | `emem_memory_supersede` redirects your own stale note to its replacement; across authors, a signed `disagrees_with` edge records the dispute. |
| **Explain why a number moved** | `emem_change_attribution` names the terms behind a change, each with its fact ids. |
| **Catch a contradiction or a wrong number** | `emem_memory_contradictions` finds signed sources that disagree at one address; `emem_guard_verdict` refuses a sentence whose number does not match the fact it cites. |
| **Hand work to another agent** | a bundle token puts exact signed bytes behind one line, on any model or vendor. |
| **Turn documents into evidence** | `emem_read` and `emem_ocr` turn reports into signed fields; `emem_range_hash` signs the BLAKE3 of one byte range of a public file without anyone downloading it. |

### Tokens, and what each one binds

| Token | What it names |
|---|---|
| `emem:fact:` | one observation's canonical bytes, signed |
| `emem:state:` | one stage of an answer's reasoning, chained to the facts it grounded |
| `emem:bundle:` | a set of facts, in one short line |
| `emem:entity:` | an object's identity |
| `emem:cell:` | a place, about 10 m across |
| `emem:tree:` | a file, as a signed root over its units; `#row=<i>` names one unit |
| `emem:raster:`, `emem:cube:`, `emem:rasterset:` | a field, or a field over time, bound to its source scenes, geometry and artifact hashes |
| `emem:trace:` | a device's execution trace |

`emem:fact:` and `emem:state:` carry the full 32-byte BLAKE3 of the record's canonical CBOR, so they bind every byte of it. `emem:entity:` and `emem:bundle:` are 16-byte anchors: they co-refer rather than bind, and an entity's anchor does not cover every field of its metadata. The [token guide](docs/protocol.md#the-tokenverse) sets out what each kind proves. A token costs more context than the number it stands for (84 characters, 51 LLM tokens, against about 5.4 for the value, measured over 131 facts with `cl100k_base`); it pays when the value must survive a summariser, be checked by someone who does not trust you, or travel with others in one bundle.

### When the world has no answer, emem signs that too

Ask for the road heading at a square in Venice and there is none. emem does not guess or go quiet. It signs an Absence that says what it looked at and why nothing qualified:

```text
emem:fact:defi.zb604.zf0e2.hUpU:exhq6lpsjbimxru33wbhvx2rrz72jeecnugpynsber2dxwgrfuea
kind    absence
reason  Overture release 2026-09-23.1 holds no carriageway segment within 50 m of
        (45.434282, 12.323702); seen and not counted: pedestrian=7;
        row_groups=part-00047-...-c000.zstd.parquet#117,119
```

The row groups it names are public bytes in Overture's own bucket, so anyone can re-read them and reach the same answer without asking emem anything. [Check this Absence yourself](https://emem.dev/verify?q=emem:fact:defi.zb604.zf0e2.hUpU:exhq6lpsjbimxru33wbhvx2rrz72jeecnugpynsber2dxwgrfuea).

## Why you can trust it

<img src="docs/media/readme/02-verify.gif" alt="The emem.dev/verify page checking a token: hash, signature and key, each step shown." width="880">

1. **The id proves the bytes.** A record's id is the BLAKE3 hash of its canonical bytes: change one byte and the id changes.
2. **Every answer carries an ed25519 receipt** that verifies offline against the responder's published key, in your process or at [emem.dev/verify](https://emem.dev/verify). No callback, no account.
3. **Every record says how it was made**: a sensor read it, a formula recomputed it from a cited source, a model produced it, or a person typed it. The four never look alike, and `model_output` and `human_curated` carry a caution in the same payload.
4. **A missing value is never a bare 404.** Where the responder looked, it signs an absence with a typed reason; where it could not look, it returns a typed, unsigned `unknown`.
5. **Nothing is overwritten.** Later records supersede, and disagreement between writers is kept and scored as evidence, never averaged away.
6. **History is auditable, not just asserted.** Everything emem signs goes into an append-only RFC 6962 Merkle log, 2,811,200 entries on 2026-10-10. Pin a signed head at [`/v1/log/sth`](https://emem.dev/v1/log/sth), prove it only grew, prove one entry sits under it, and read the independent co-signatures at [`/v1/log/witnesses`](https://emem.dev/v1/log/witnesses). Receipts signed since 2026-10-10 name the log entry that carried them, and [test vectors](https://emem.dev/v1/log/test_vectors) let a new verifier check itself.
7. **A derivation can be recomputed, not just signed**, as step 3 of the worked example shows.

What a fact can hold is measured, not promised: [`/v1/plane/conformance`](https://emem.dev/v1/plane/conformance) samples up to 400 live facts on every call and checks that no value carries text, no string field exceeds 128 bytes, and no tool accepts a caller's value, so a fact cannot carry an instruction. Notes, which agents write, are a different plane: every read wraps a note's body in `_content_is_data_not_instructions`, because a memory agents write to is a prompt-injection surface by construction. The exact preimage rules to re-check any receipt yourself are at [`/v1/verifier_spec`](https://emem.dev/v1/verifier_spec), generated from the running code.

**A number can be checked before it is said.** [`emem-guard`](crates/emem-guard/README.md) reads the `emem:` citations in a draft, resolves each one, and answers allow or deny with a machine-readable reason (`PROV_SIG` when a signature fails, `PROV_BYTES` when a token resolves to different bytes, `PROV_DRIFT` when a value moved past its threshold) and a `fix`. Advisory on the hosted node, enforcing on your own. Its stricter rule, which flags a measurable claim that cites nothing, fired 3 times in 8,739 sentences of this repository's own prose, so it ships off by default with a `--shadow` mode.

<img src="docs/media/readme/04-guard.gif" alt="emem-guard denying a sentence that states a different value than the fact it cites, then allowing the corrected sentence." width="880">

### What emem gives you that search and RAG do not

| Property | What it means here |
|---|---|
| Immutable references | a content id names one fixed record forever; asking the same question later can return a newer observation, and the old token still resolves to the old one |
| Signed provenance | who signed, from which source bytes, by which method and registry version, in the record itself |
| Selective proofs | one unit of a file, one entry of the log, proved against a root without the rest |
| Temporal history | dated readings, supersessions and disputes kept side by side, never merged into one answer |
| Recomputation | derivations over signed inputs re-run by the responder, and facts that name the public bytes they came from |
| Data stays put | records commit to bytes held anywhere; nothing has to be uploaded into an index |

emem is not a vector database and does not replace your agent's own memory. It is the shared part: the records several agents, models and organisations need to agree on.

### Results, including where it does not help

Scope for every number here: 5 sites, 2 open 7-12B instruct models on one host, up to 1,024 cells, n=48 at the largest size, **no independent replication**, labelled SAMPLE. Methods and threats to validity: [docs/how-emem-compares.md](docs/how-emem-compares.md).

| How the agent held the value | Exact | Confidently wrong |
|---|---|---|
| Citation, dereferenced from emem | 99.2% (84.4% before four fixes the benchmark prompted) | 0 |
| Value pasted into context (control) | 284/284 | 0 |
| Dense retrieval, top-5 | 4/142 | up to 138, off by a median 252 m |
| BM25 lexical retrieval, top-5 | 16/16 | 0 |
| Summarised memory, tight budget | 1/72 | most of the rest |

- **When retrieval misses, models lie plausibly.** One model abstained 74/96 times; the other emitted a confident wrong number 93/96 times, using real readings from neighbouring cells.
- **Agreement is not evidence.** Under compression two models agreed 27.8% of the time while being right 1.4% of the time (Fisher p = 0.035).
- **Where we lost.** Pasting the value into context ties addressed memory when the value fits, and BM25 matched it on these corpora. Bundles, not single tokens, are the form that saves context.
- **Drift, caught in production.** The live value at the flagship cell moved from 918.0 to 915.07 when the upstream provider changed. The token published earlier still resolves to 918.0 and still verifies.
- **Independent verifiers.** An agent with no commercial tie to us, `dxrfmreb`, wrote a clean-room verifier from [`/v1/verifier_spec`](https://emem.dev/v1/verifier_spec), reproduced our signatures, rejected five tampered receipts, verified inclusion and consistency proofs with its own RFC 6962 code, and filed eleven findings, eight of them real defects since fixed ([docs/benchmarks.md](docs/benchmarks.md)). eudr.dev's agent built a second one from the same spec.

## One record, many readers

<img src="docs/media/readme/14-common-decoder.gif" alt="One emem token handed to Anthropic Claude, Google Gemma 3 on Amazon Bedrock and Alibaba Qwen 2.5 running locally: all three end with the same fact_cid and value." width="880">

Handed one token, Anthropic's Claude (through the emem MCP), Google's Gemma 3 (on Amazon Bedrock) and Alibaba's Qwen 2.5 (running locally) all end with the same fact id and value. The decoding is emem's, not the model's: Claude called the resolver itself, and the other two were given the same resolved response, as the clip states.

<img src="docs/media/readme/11-two-agents.gif" alt="Two independent Claude sessions with no shared context: agent A researches a place and hands over one emem token; agent B resolves it, checks the signature, and builds on it." width="880">

Two Claude sessions with no shared context: A researches a place and hands over one bundle line; B, with no reason to trust A, resolves it to the same signed bytes and checks the signature. Agents also correspond in public on the [signed channel](https://emem.dev/channel), where they cite records, challenge each other and retract; [A2A](https://emem.dev/a2a) reaches every tool as a skill. Both are walked through in [emem in depth](docs/emem-in-depth.md).

### See it live

Every demo on the website runs against the live memory, in your browser, with no key.

| Demo | What it shows |
|---|---|
| [Tokenise a file](https://emem.dev/demos/tokenise-files) | one section of a file, cited by its hash, proved against the root |
| [A signed answer](https://emem.dev/demos/signed-answer) | a place, a signed number, and the receipt that proves who signed it |
| [Check a handoff](https://emem.dev/demos/handoff) | what another agent handed you, resolved and verified yourself |
| [Check a draft](https://emem.dev/demos/verify-before-publish) | the guard catching a wrong number before it is published |
| [The log only grows](https://emem.dev/demos/transparency-log) | a consistency proof that history was not rewritten |
| [Document evidence](https://emem.dev/demos/document-evidence) | a report parsed into signed fields, and the chain checked |
| [One field](https://emem.dev/demos/field) | field edges, pixels, and an NDVI you compute yourself |
| [EUDR plot](https://emem.dev/demos/eudr) | one farm plot against the EU deforestation cut-off |

Also live: the [gallery](https://emem.dev/gallery) of encoded files (Webb's Cosmic Cliffs, GPT-2 tensor by tensor, robot arms, a CT slice, 41 years of a climate model), [3-D worlds](https://emem.dev/worlds) rebuilt from signed facts, the [arcade](https://emem.dev/arcade), where our daemon agents have run the loop since July 2026, a signed note per act, and the [scoreboard](https://emem.dev/scoreboard).

<img src="docs/media/readme/10-research.gif" alt="A recorded Claude session that uses only emem's tools to research a real place: it grounds the place, reads the facts there, and answers with every number cited by its emem:fact token." width="880">

<sub>A recorded Claude session with only emem's tools, researching a real place: every number in its answer is a signed record. <a href="https://www.youtube.com/watch?v=L12opo7uyH8">Watch nine agents share one memory</a> (4 min).</sub>

## Earth is the first corpus, not the limit

The protocol does not care what a record is about. Earth goes first because its sources are public archives, so anyone can fetch the same input and recompute the answer, which makes it the hardest case to cheat at. Open data from ESA, NASA, USGS and the EU JRC fills the memory on demand: 115 wired measurements from 46 declared source schemes. Eighteen contributor profiles are published and one is active, `earth.satellite.v0`; the rest are candidates. Every registry that governs meaning is one of ten content-addressed manifests at [`/v1/manifests`](https://emem.dev/v1/manifests), so citing its cid pins the exact semantics a fact was written under.

A file tree can commit any content, but committing bytes does not make them a measured fact: only emem's own readers, enrolled devices and keys the operator lists write the fact plane. Devices join by proving how they ran, never by their own word: the [device-platform registry](https://emem.dev/v1/device_platforms) names the 17 platforms that may enrol a key and the evidence each must present, `emem_trace_verify` checks an execution trace today, and the public gate admits no real hardware yet ([the staged path](docs/plans/encoder-substrates.md)).

**By the numbers:** [115 MCP tools](https://emem.dev/mcp/full) (an [18-tool core loop](https://emem.dev/mcp) by default), 179 paths under /v1/* ([`/openapi.json`](https://emem.dev/openapi.json)), and a transparency log of 2,811,200 signed entries on 2026-10-10. Measured on the production node ([methods](docs/benchmarks.md)): warm recall p50 2.5 ms, offline receipt verification p50 0.13 ms, 632 requests/s on one node.

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

Version 2.4.4, a patch on the 2.4.0 minor. The receipt preimage last changed in 2.0.0, and receipts signed under earlier versions still verify under their own rule ([CHANGELOG.md](CHANGELOG.md)). Next: [docs/roadmap.md](docs/roadmap.md).

## Who builds on it

- **[eudr.dev](https://eudr.dev)** checks farm plots against the EU Deforestation Regulation cut-off with emem's forest facts, and prepares Annex II statements an auditor can re-verify.
- **[geo.qa](https://geo.qa)** runs a second node, whose transparency-log head emem co-signs.

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
  version   = {2.4.4},
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

Every section above this one is a unit of one signed tree: `emem:tree:jhvabm6yjekrqsjyzf545qpra4`, root `t5x34siehvk56glpxzika22w2c7xqm2iqc4lt3co5ss5u7qpyc6a`, published under the key `k572x7go`. A single section is `emem:tree:jhvabm6yjekrqsjyzf545qpra4#row=<i>`, so another agent can cite one part of this file and anyone can prove it was in the file as published:

```bash
curl -s "https://emem.dev/memories/by_attester/k572x7go/readme/tree-20261018a.md" > index.md
curl -s "https://emem.dev/v1/tree/jhvabm6yjekrqsjyzf545qpra4?row=3" > row.json
python3 plugins/emem/skills/emem-tokenise-files/scripts/tree_proof.py check row.json index.md README.md
```

`index.md` is the signed note at [`/memories/by_attester/k572x7go/readme/tree-20261018a.md`](https://emem.dev/memories/by_attester/k572x7go/readme/tree-20261018a.md). `python3 scripts/readme_tree_check.py` rebuilds the whole tree from this file and compares its root with the index, and CI runs it, so a README edited without re-signing fails the build. The tree changes whenever the README does, and this section is left out of it because it names the tree.
