# emem.dev: verifiable memory for AI agents

emem is a memory substrate AI agents read, write, and cite. It is a vendor-neutral, citeable identity layer that stops referential drift: every place resolves to one canonical address (cell64), every observation to one signed fact (fact_cid), and every object to one citeable identity (emem:entity:<entity_cid>, minted by emem_entity), so different models reason from the same world object instead of divergent descriptions. Drift runs in both directions: the paraphrase that drifts from its referent, which the token pins, and the readout that drifts at a pinned reference, which the change-attribution ledger now reports as per-term evidence (the numeric split is still roadmap). Every read returns an ed25519-signed receipt. Every write is content-addressed. Every byte is reproducible on any peer that mirrors the data, verifiable in the browser at `/verify`.

emem is a shared memory for AI agents that connects facts and gets better over time. Every answer is signed, so anyone can check it without trusting the server.

The substrate has two layers riding the same trust surface:

- **Earth memory.** The address space is the planet. A `cell64` addresses a place the way a token addresses text in an LLM: every patch of ground gets a 64-bit identifier (~9.55 m at the equator). Every measurement at that cell is a fact keyed by `(cell, band, tslot)`. Each fact is signed: the responder fingerprints the answer with a blake3 hash (change one byte, the fingerprint changes) over bytes packed in a fixed order (canonical CBOR), then signs that fingerprint with an ed25519 key. 129 materializer-wired band names across 43 cube slots, 46 upstream sources (five declared-but-not-yet-wired), 168 named composition algorithms. Cold cells materialise on first ask (sign, persist, return), so every cell on Earth answers cite-ably from day one without a pre-seeded corpus.
- **Agent memory.** Above the spatial fact store sits a writable scratchpad the agent owns. Six Anthropic memory-tool verbs (`emem_memory_view`, `emem_memory_create`, `emem_memory_str_replace`, `emem_memory_insert`, `emem_memory_delete`, `emem_memory_rename`); each file carries a `kind` from the CoALA taxonomy (`episodic`, `semantic`, `procedural`, `resource`); writes are capability-bound to an ed25519 attester: a release responder refuses unattested writes by default, and paths under `/memories/by_attester/<pubkey>/...` reject any signer that isn't their owner under every policy. `memory_search` runs BGE-768 against a LanceDB IVF_PQ partition over file contents. `memory_bundle` composes N facts into one signed envelope (`emem:bundle:<bundle_cid>`). `memory_contradictions` walks a parallel multi-attester index and scores disagreement per band kind. `memory/sse` streams writes with server-side filter.

Every read primitive across both layers accepts a bi-temporal axis: `as_of_tslot` asks what the world looked like on a given date; `as_of_signed_at` asks what emem knew on a given date. Set both and both predicates hold simultaneously. The receipt carries an `as_of` block when the bound is non-empty so an auditor in 2027 takes a 2026 receipt to any peer and replays the exact same query.

## Where this is going

emem is built to be a protocol, not a single service. Because every fact is content-addressed and signed, any responder can serve it and any client can verify it offline, without trusting the source. Today that runs as one hosted responder plus self-hosted nodes. The design target is a federation of independent responders that resolve the same content ids byte-for-byte, cross-cite each other's attestations, and record where they disagree, so the shared memory gets more trustworthy the more agents read and write against it. None of the multi-host federation routing ships yet in the 1.x line. What ships today is the substrate that makes it possible: content addressing, signed receipts, typed temporal edges, multi-attester contradiction scoring, and a deterministic refinement loop.

This site renders the canonical docs straight from the repo at `docs/`. Every page here ships in the same signed server binary as the rest of `web/`; there is no separate docs host.

## Where to start

- **[Quickstart](./quickstart.md):** sixty seconds from cold to a signed answer, in five copy-pasteable forms (curl, Python, TypeScript, Go, MCP)
- **[Memory substrate](./memory.md):** full wire reference for typing, capability binding, bi-temporal reads, semantic search, contradiction detection, SSE events, sled storage
- **[Agents](./agents.md):** agent integrator's rosetta, every memory operation mapped to its emem primitive
- **[Whitepaper](./whitepaper-v2.md):** math, trust layer, receipt rules
- **[Protocol](./protocol.md):** wire format, `cell64`, `cid64`, `tslot`, receipts
- **[Registries](./registries.md):** bands (43 cube slots, 124 wired names), algorithms (162), functions, sources (46), topics (27), schema, lcv-1, alphabet
- **[Errors](./errors.md):** typed error codes including `memory_attestation_invalid`, `memory_namespace_violation`, `invalid_temporal_bound`, `invalid_signed_at_format`
- **[Developers / Operators](./developers/architecture.md):** build, run, deploy

## Conventions

- *Receipts* are ed25519-signed over a domain-separated, length-prefixed preimage, `blake3("emem.preimage.v1" ‖ "receipt" ‖ tagged(request_id, served_at, [scope], [as_of], [edges], [manifest], primitive, cells[], fact_cids[]))`, so no two distinct responses can share signed bytes. The receipt's `preimage_version` selects the rule; both it and `/v1/verify_receipt` are verifiable in-browser at `/verify`.
- *One memory, three kinds of users.* AI agents ground claims in tokens instead of paraphrases; robots get durable spatial memory (landmarks as `emem:entity:` identities, terrain and hazards as signed facts at drift-free addresses, shared across fleets and vendors); satellites, drones, and fixed sensors attest at the source. Earth observation is the first substrate, not the boundary. The formal object, properties, and memory algebra are written down in [the memory model](model.md).
- *A fact is a signed observation.* The signature proves who attested it and that the bytes are unchanged, never that the value is objectively true; confidence, uncertainty, and the band's provenance class (with an in-band `caution` on model and human classes) travel with it.
- *Bi-temporal* receipts carry an additional `as_of: { valid_time, transaction_time }` block when at least one bound was set; the block is absent for current-state reads so old receipts deserialise byte-identically.
- *Content-addressed everywhere*. A `fact_cid` is 52 base32 chars, the full 32-byte BLAKE3 of the fact's canonical CBOR; the same bytes hash to the same CID on every responder. Bundle and entity CIDs are shorter 26-char prefixes. `emem:fact:<cell64>:<fact_cid>` is the cite handle for one fact at one place; `emem:bundle:<bundle_cid>` for N facts at N places; `emem:entity:<entity_cid>` for a whole object (minted by `emem_entity`), the object you cite rather than only a fact. The pre-rename prefixes `memt:`, `memb:`, and `meme:` still resolve.
- *No silent fallbacks*. An empty result distinguishes wrong-query from empty-place. `find_similar` with a bi-temporal bound bypasses the LanceDB ANN fast-path (the index has no `signed_at` column) and reports `via: brute_force_fallback`.
- *No stubs*. Nothing here is aspirational. If a primitive is documented, it ships and the receipt verifies.

## Discoverability

The same content surfaces are reachable to agents via:

- `GET /openapi.json`: full REST surface (browseable at [/docs/api/](/docs/api/) via ReDoc)
- `POST /mcp` `tools/list` returns the 16 core tools by default, so a host's context carries about 39 KB of descriptors instead of 204 KB; `POST /mcp/full` advertises all 107 (16 core, 91 extended), and `{"tier":"core"|"extended"|"all"}` overrides either default. `tools/call` dispatches all 107 by name at both endpoints, so narrowing discovery removes no capability. `emem_tools` maps the surface: every tool declares one `dev.emem/shape` (the form of the answer: `scalar`, `raster`, `identity`, `token`, `proof`, and so on) and any number of overlapping `dev.emem/bundles` (the job: `tokenisation`, `verification`, `agent_to_agent`, `robotics`, …) in MCP-standard `_meta`, and both are filters
- `POST /mcp` `resources/list`: 18 resources + 8 URI templates (memory anchors include `memory://emem/cell/<cell64>`, `memory://emem/fact/<cid>`, `memory://emem/bundle/<token>`)
- `GET /llms.txt`, `GET /skills.md`
- `GET /agent.json`, `GET /ai-plugin.json`
- `GET /sitemap.xml`
- MCP directory listing at [/agents](/agents)
