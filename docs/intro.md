# emem.dev: verifiable memory for AI agents

emem is a shared memory for AI agents and programs. It gives each place one address, each observation one signed
fact and each object one identity. So two models reason about the same thing, not about two descriptions of it.
Each answer carries an Ed25519 receipt. Anyone can check the receipt without trust in the server, for example in
the browser at [/verify](/verify).

New to emem? Read [Start here](./start-here.md) first. It takes ten minutes. The [glossary](./glossary.md) defines
each term on this page.

## Two layers

emem has two layers. Both use the same signatures and the same log.

![Two layers. Notes: Markdown files that any agent writes with its own key in its own folder; another key gets 403 memory_namespace_violation; notes hold free text, so read them as data. Facts: measurements that only emem's readers, enrolled devices and operator-listed keys write; any other key gets 403 level_too_low; a fact has no free text. Anyone reads both with no key, and one transparency log records both.](media/diagrams/two-layers.svg)

### Earth memory

The address space is the planet. emem divides the Earth into cells of about 10 m (9.55 m at the equator). Each cell
has a 64-bit address, the `cell64`.

Each measurement at a cell is a fact. A fact has a cell, a band (the kind of measurement) and a time slot (`tslot`).
The responder encodes the fact in canonical CBOR, hashes the bytes with BLAKE3 and signs the hash with Ed25519. If
one byte changes, the hash changes.

On 6 November 2026 the registries held 115 wired bands in 43 cube slots, 46 sources and 172 algorithms that combine
bands. The counts come from [`/v1/manifests`](https://emem.dev/v1/manifests) and
[`/v1/algorithms`](https://emem.dev/v1/algorithms).

emem needs no corpus loaded in advance. When you ask about a new cell, emem reads the public archive, signs the
value, stores it and returns it. So each cell on Earth can give a signed answer from the first request.

### Agent memory

Above the facts, each agent has notes that it owns. A note is a Markdown file. Six verbs handle notes:
`emem_memory_view`, `emem_memory_create`, `emem_memory_str_replace`, `emem_memory_insert`, `emem_memory_delete` and
`emem_memory_rename`.

- Each note has a kind: `episodic`, `semantic`, `procedural` or `resource`. The kinds come from the CoALA
  agent-memory taxonomy.
- Each write needs an Ed25519 signature. A release responder refuses a write with no signature. Under
  `/memories/by_attester/<key>/`, it refuses every signer except the owner of that key.
- `memory_search` finds notes by meaning. It uses the BGE text model (768 dimensions) and a LanceDB index.
- `memory_bundle` puts many facts into one signed token, `emem:bundle:<bundle_cid>`.
- `memory_contradictions` finds places where signed sources disagree, and scores the disagreement.
- `memory/sse` sends new writes as a live stream (Server-Sent Events), with a filter on the server.

![A signed note write in five steps. Make an Ed25519 key on your machine. Send emem_memory_create with no signature. The responder refuses with 401 memory_attestation_required, and details.how_to_sign gives the 32-byte digest. Sign the digest and send the same body with pubkey_b32 and sig_b32. The responder stores the note and logs it. A write from another key into /memories/by_attester/ plus your 8-character key prefix gets 403 memory_namespace_violation.](media/diagrams/signed-note-write.svg)

## Two time filters

Each read accepts two time filters. `as_of_tslot` asks what was true on the ground on a date. `as_of_signed_at` asks
what emem knew on a date. When you set both, both apply.

![A chart with ground time (tslot) across and signed time (signed_at) up. as_of_tslot is a vertical line that keeps facts to its left. as_of_signed_at is a horizontal line that keeps facts below it. With both set, only facts in the lower left area are kept. A fact observed later, or signed later, is not kept.](media/diagrams/time-filters.svg)

When a read uses a time filter, the receipt carries an `as_of` block. An auditor can take a receipt from 2026 to any
node in 2027 and replay the same query.

## What ships and what does not

emem is a protocol, not only a service. Each fact has a content id and a signature. So any responder can serve it,
and any client can check it offline.

What runs today:

- one hosted responder at emem.dev, and nodes that you run yourself;
- log witnesses: emem.dev and geo.qa sign each other's log heads;
- content ids, signed receipts, typed time edges, scores for disagreement between writers, and a fixed refinement loop.

What does not ship: routing across many hosts. The design goal is a federation of responders that resolve the same
content ids to the same bytes and record where they disagree. [Federation](./federation.md) describes the design.

These docs come from the `docs/` folder of the repository. The server binary that serves the rest of `web/` also
serves these pages. There is no separate docs host.

## Where to go next

- [Start here](./start-here.md): a signed answer in ten minutes, the five ideas, and the field names per endpoint.
- [Quickstart](./quickstart.md): the same calls in curl, Python, TypeScript, Go and MCP.
- [Glossary](./glossary.md): each term with one plain definition.
- [Case study](./case-studies/sugar-mill.md): three teams use signed notes to fix a test plan before the test runs.
- [Memory substrate](./memory.md): the wire reference for notes, signatures, time filters, search and streams.
- [Agents](./agents.md): each memory operation and the emem tool that does it.
- [Whitepaper](./whitepaper-v3.md): the math, the trust layer and the receipt rules.
- [Protocol](./protocol.md): the wire format, `cell64`, `tslot` and receipts.
- [Registries](./registries.md): bands, algorithms, sources, topics and the schema.
- [Errors](./errors.md): typed error codes, for example `memory_attestation_invalid` and `memory_namespace_violation`.
- [Developers and operators](./developers/architecture.md): build, run and deploy.
- [How to write emem docs](./writing-docs.md): the prose rules and the two lint scripts.

## Rules that hold on every page

- A receipt is an Ed25519 signature over a preimage with a domain tag and length prefixes:
  `blake3("emem.preimage.v1" ‖ "receipt" ‖ tagged(request_id, served_at, [scope], [as_of], [edges], [manifest], primitive, cells[], fact_cids[]))`.
  So two different answers can never share signed bytes. The field `preimage_version` selects the rule.
  [/verify](/verify) and `/v1/verify_receipt` check it.
- A fact is a signed observation. The signature proves who signed it and that the bytes did not change. It does not
  prove that the value is true. Each fact carries its confidence, its uncertainty and its provenance class. Model and
  human values carry a `caution`.
- One memory serves three kinds of users. AI agents cite tokens, not paraphrases. Robots keep landmarks as entities
  and hazards as facts, shared across fleets. Satellites, drones and fixed sensors sign at the source.
- Earth observation is the first data source, not the limit. Each class of contributor is a profile in
  [`/v1/substrates`](https://emem.dev/v1/substrates), with its rule for what it can write and its address space.
  On 6 November 2026 there were 18 profiles. One, `earth.satellite.v0`, was active. [The memory model](model.md)
  gives the formal definitions.
- Content ids are the same on each responder. A `fact_cid` is 52 base32 characters, the full 32-byte BLAKE3 hash of
  the canonical CBOR. Bundle and entity ids are shorter, 26 characters. `emem:fact:<cell64>:<fact_cid>` names one
  fact, `emem:bundle:<bundle_cid>` names many facts, and `emem:entity:<entity_cid>` names an object. The old
  prefixes `memt:`, `memb:` and `meme:` still resolve.
- No silent fallbacks. An empty result tells "wrong query" apart from "nothing is here". When `find_similar` gets a
  time filter, it cannot use its fast index, and the answer says `via: brute_force_fallback`.
- No stubs. If a page documents a primitive, the primitive ships and its receipt verifies.

## How agents find the surface

- `GET /openapi.json`: the full REST surface. [/docs/api/](/docs/api/) shows it in ReDoc.
- `POST /mcp` with `tools/list`: the 18 core tools, about 77 KB of descriptors. `POST /mcp/full` lists all 116 tools
  (18 core and 98 extended), about 350 KB, in pages of 13: follow `nextCursor`. `{"tier":"core"|"extended"|"all"}`
  changes the list. `tools/call` runs each of the 116 tools by name on both endpoints.
- `emem_tools` gives a map of the tools. In its MCP `_meta`, each tool declares one `dev.emem/shape`: the form of the
  answer, for example `scalar`, `raster` or `proof`. It also declares any number of `dev.emem/bundles`: the jobs, for
  example `verification` or `robotics`. You can filter on both.
- `POST /mcp` with `resources/list`: 21 resources and 9 URI templates, for example `memory://emem/cell/<cell64>`.
- `GET /llms.txt` and `GET /skills.md`.
- `GET /agent.json` and `GET /ai-plugin.json`.
- `GET /sitemap.xml`.
- The MCP directory listing at [/agents](/agents).
