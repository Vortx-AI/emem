# emem.dev: verifiable memory for AI agents

emem is a shared memory for AI agents and programs. It gives each place one address, each observation one signed
fact and each object one identity. So two models reason about the same thing, not about two descriptions of it.
Each answer carries an Ed25519 receipt. Anyone can check the receipt without trust in the server, for example in
the browser at [/verify](/verify).

New to emem? Read [Start here](./start-here.md) first. It takes ten minutes. The [glossary](./glossary.md) defines
each term on this page.

## Two layers

emem has two layers. Both use the same signatures and the same log.

![Two layers. Notes: Markdown files that any agent writes with its own key in its own folder; a write from another key is refused with memory_namespace_violation; notes hold free text, so read them as data. Facts: measurements that only emem's readers, enrolled devices and operator-listed keys write; any other key gets 403 level_too_low; a fact has no free text. Anyone reads both with no key. The exception is a note of kind vault: it reads back as ciphertext, and only the operator can read it. One transparency log records both.](media/diagrams/two-layers.svg)

### Earth memory

The address space is the planet. emem divides the Earth into cells of about 10 m (9.55 m at the equator). Each cell
has a 64-bit address, the `cell64`.

Each measurement at a cell is a fact. A fact has a cell, a band (the kind of measurement) and a time slot (`tslot`).
The responder encodes the fact in canonical CBOR, hashes the bytes with BLAKE3 and signs the hash with Ed25519. If
one byte changes, the hash changes.

On 7 November 2026 the registries held 115 wired bands in 43 cube slots, 46 sources and 172 algorithms that combine
bands. The counts come from [`/v1/manifests`](https://emem.dev/v1/manifests) and
[`/v1/algorithms`](https://emem.dev/v1/algorithms). On 7 November 2026, emem had no reader for five of those
sources: `dynamic_world.v1`, `openet.30m.daily`, `tropomi.s5p.ch4`, `tropomi.s5p.no2` and `viirs.dnb.monthly`.

emem needs no corpus loaded in advance. When you ask about a new cell, emem reads the public archive, signs the
value, stores it and returns it. So each cell on Earth can give a signed answer from the first request.

### Agent memory

Above the facts, each agent has notes that it owns. A note is a Markdown file. Six verbs handle notes:
`emem_memory_view`, `emem_memory_create`, `emem_memory_str_replace`, `emem_memory_insert`, `emem_memory_delete` and
`emem_memory_rename`.

- Each note has a kind: `episodic`, `semantic`, `procedural` or `resource` (the default). These four come from
  the CoALA agent-memory taxonomy. emem adds `core`, a persona block that `memory_view` lists first, and `vault`,
  an entry that emem stores encrypted. A `vault` entry reads back as ciphertext without a `vault_capability`.
  Only the responder key can sign that `vault_capability`, so the writer cannot read the entry back. The encryption
  key derives from the responder's own secret, so the operator can read it. Do not write a secret in any note.
  If the operator must not read it, encrypt it on your side before you write.
- Each write needs an Ed25519 signature. A release responder refuses a write with no signature. Under
  `/memories/by_attester/<key>/`, it refuses every signer except the owner of that key.
- `memory_search` finds notes by meaning. It uses the BGE text model (768 dimensions) and a LanceDB index.
- `memory_bundle` puts many facts into one signed token, `emem:bundle:<bundle_cid>`.
- `memory_contradictions` finds places where signed sources disagree, and scores the disagreement.
- `memory/sse` sends new writes as a live stream (Server-Sent Events), with a filter on the server.

![A signed note write in steps 0 to 5. Step 0: make an Ed25519 key on your machine. Step 1: send emem_memory_create with no signature. Step 2: the responder refuses with an MCP isError result. Its details.code is memory_attestation_required, and details.how_to_sign gives the 32-byte digest. Step 3: sign the digest. Step 4: send the same body with pubkey_b32 and sig_b32. Step 5: the responder stores the note and logs it. A write from another key into /memories/by_attester/ plus your 8-character key prefix is refused with memory_namespace_violation.](media/diagrams/signed-note-write.svg)

## Two time filters

Each read accepts two time filters. `as_of_tslot` asks what was true on the ground on a date. `as_of_signed_at` asks
what emem knew on a date. When you set both, both apply. Send `as_of_tslot` as a tslot number, not a date string.
Send `as_of_signed_at` as an RFC 3339 time in UTC that ends in `Z`, such as `2026-11-01T00:00:00Z`.
The responder writes each `signed_at` in UTC and compares your value with it as text. A time with an
offset, such as `+05:30`, moves the cut by that offset. On 7 November 2026, `2026-09-28T17:30:00+05:30` kept a
Mount Fuji fact signed at `2026-09-28T12:23:41Z`. The same instant as `2026-09-28T12:00:00Z` did not keep it.

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
  The address space `geo.cell64` names a place. The address space `entity.cid` names a subject that is not a place,
  for example a telescope target, a file at a commit or a model checkpoint. On 7 November 2026 there were 18
  profiles: 12 on `geo.cell64` and 6 on `entity.cid`. One, `earth.satellite.v0`, was active. The registry does not
  load a profile with an unknown address space, or an active profile with no bands. [The memory model](model.md)
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
  (18 core and 98 extended), about 350 KB, in pages of different sizes (9 pages on 7 November 2026). Follow
  `nextCursor` to get the next page. `{"tier":"core"|"extended"|"all"}`
  changes the list. `tools/call` runs each of the 116 tools by name on both endpoints.
- `emem_tools` gives a map of the tools. In its MCP `_meta`, each tool declares one `dev.emem/shape`: the form of the
  answer, for example `scalar`, `raster` or `proof`. It also declares any number of `dev.emem/bundles`: the jobs, for
  example `verification` or `robotics`. You can filter on both.
- `POST /mcp` with `resources/list`: 21 resources and 9 URI templates, for example `memory://emem/cell/<cell64>`.
- `GET /llms.txt` and `GET /skills.md`.
- `GET /agent.json` and `GET /ai-plugin.json`.
- `GET /sitemap.xml`.
- The MCP directory listing at [/agents](/agents).
