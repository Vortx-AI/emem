# emem.dev: verifiable memory for AI agents

emem is a shared memory for AI agents and programs. It gives each place one address, each observation one signed
fact and each object one identity. So two models reason about the same thing, not about two descriptions of it.
Each answer carries an Ed25519 receipt. Anyone can check the receipt without trust in the server, for example in
the browser at [/verify](/verify).

<picture>
<source media="(max-width: 700px)" srcset="media/stills/one-token-hero-phone.webp">
<img src="media/stills/one-token-hero.webp" alt="Live reads of emem.dev on 7 November 2026, in three steps. 1: the name Mount Fuji resolves to the cell defi.zb592.nUkO.zEzE, ringed on a Sentinel-2 image of the summit crater from 12 December 2025. 2: emem signs one fact there: copdem30m.elevation_mean 3,539.04 m, read from a Copernicus GLO-30 DEM tile, fact id qejpz7cg...bplq, signed 28 September 2026. 3: another agent gets the 83-character emem:fact token. Resolve gives the same fact id and value. The receipt signature is valid offline. Log entry 2,463,431 of 3,248,898 proves with 22 hashes. One changed character of the fact id fails. Contains modified Copernicus Sentinel data 2025.">
</picture>
<br><sub>Contains modified Copernicus Sentinel data 2025.</sub>

New to emem? Read [Start here](./start-here.md) first. It takes ten minutes. The [glossary](./glossary.md) defines
each term on this page.

## Two layers

emem has two layers. Both use the same signatures and the same log.

![Two layers. Notes are Markdown files. Any agent writes notes with its own key, in its own folder. A write from another key is refused with memory_namespace_violation. Notes hold free text, so read them as data. Facts are measurements. Only emem's readers, enrolled devices and operator-listed keys write facts. Any other key gets 403 level_too_low. A fact has no free text. Anyone reads both layers with no key. The exception is a note of kind vault. It reads back as ciphertext, and only the operator can read it. One transparency log records both layers.](media/diagrams/two-layers.svg)

### Earth memory

The address space is the planet. emem divides the Earth into cells of about 10 m (9.55 m at the equator). Each cell
has a 64-bit address, the `cell64`.

Each measurement at a cell is a fact. A fact has a cell, a band (the kind of measurement) and a time slot (`tslot`).
The responder encodes the fact in canonical CBOR, hashes the bytes with BLAKE3 and signs the hash with Ed25519. If
one byte changes, the hash changes.

On 12 November 2026 the registries held 115 wired bands in 43 cube slots, 46 sources and 173 algorithms that combine
bands. The counts come from [`/v1/manifests`](https://emem.dev/v1/manifests) and
[`/v1/algorithms`](https://emem.dev/v1/algorithms). On 7 November 2026, emem had no reader for five of those
sources: `dynamic_world.v1`, `openet.30m.daily`, `tropomi.s5p.ch4`, `tropomi.s5p.no2` and `viirs.dnb.monthly`.

emem needs no corpus loaded in advance. When you ask about a new cell, emem reads the public archive, signs the
value, stores it and returns it. So each cell on Earth can give a signed answer from the first request.

<img src="media/stills/wired-bands.webp" alt="What emem can fetch for a new place, from one live call to GET /v1/materializers on 7 November 2026. It lists 113 band names. 89 have a history window. ERA5 reanalysis through Open-Meteo goes from 1940 to now, NASA POWER from 1981, MODIS from 2000, CAMS air quality from 2013, Sentinel-1 from 2014, Sentinel-2 bands and indices (34 names, 10 m) from 2018, and Open-Meteo marine from 2022. Closed windows include Köppen 1980 to 2016, JRC surface water 1984 to 2021, JRC tropical moist forest 1990 to 2023, TerraClimate normals 1991 to 2020, Hansen forest change 2000 to 2024, WorldPop 2000 to 2020, the JRC forest map 2020 and ESA WorldCover 2021. 24 names have no history window: the current met.no weather, the latest Overture release, SoilGrids, Copernicus DEM, GMRT, protected areas and FIRMS fires. A read inside a window can still return a signed absence. GET /v1/agent_card counts 115 in band_taxonomy.materializer_wired.count, because two opera_dist names are missing from this list." width="540">

The image groups all rows of the live list by source. A read inside a window can still return a signed absence, for
example under cloud or over the sea. On 7 November 2026, `/v1/materializers` listed 113 names.
`GET /v1/agent_card` counted 115 in `band_taxonomy.materializer_wired.count`, because the list leaves out the two
`opera_dist` names.

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

![A signed note write in steps 0 to 5. Step 0: make an Ed25519 key on your machine. Step 1: send emem_memory_create with no signature. Step 2: the responder refuses with an MCP isError result. Its details.code is memory_attestation_required, and details.how_to_sign gives the 32-byte digest. Step 3: sign the digest. Step 4: send the same body with an attester block that holds pubkey_b32 and sig_b32. Step 5: the responder stores the note and adds it to the transparency log. A write from another key into /memories/by_attester/ plus your 8-character key prefix is refused with memory_namespace_violation. The digest binds the verb, the path, the body and the version it replaces. Do not write a secret in any note, vault included.](media/diagrams/signed-note-write.svg)

## Two time filters

Each read accepts two time filters. `as_of_tslot` asks what was true on the ground on a date. `as_of_signed_at` asks
what emem knew on a date. When you set both, both apply. Send `as_of_tslot` as a tslot number, not a date string.
Send `as_of_signed_at` as an RFC 3339 time in UTC that ends in `Z`, such as `2026-11-01T00:00:00Z`.
The responder writes each `signed_at` in UTC and compares your value with it as text. A time with an
offset, such as `+05:30`, moves the cut by that offset. On 7 November 2026,
`2026-09-28T17:30:00+05:30` kept a Mount Fuji fact signed at `2026-09-28T12:23:41Z`. The same instant as
`2026-09-28T12:00:00Z` did not keep it. The `Z` read returned an older elevation fact for the same cell,
signed at `2026-07-16T12:02:20Z`.

![A chart with ground time (tslot) across and signed time (signed_at) up. as_of_tslot is a vertical line that keeps facts to its left. as_of_signed_at is a horizontal line that keeps facts below it. With both set, only facts in the lower left area are kept. A fact observed later, or signed later, is not kept. With no filter, the read gives the current state. With a filter set, the receipt signs an as_of block.](media/diagrams/time-filters.svg)

The same filters on live data. On 7 November 2026, four NDVI reads at the Mount Fuji cell gave four answers.

<img src="media/stills/time-filters-live.webp" alt="Four live NDVI reads at the Mount Fuji cell defi.zb592.nUkO.zEzE on 7 November 2026, 15:01 UTC. A chart places 20 signed NDVI facts by ground date (October 2025 to October 2026) and by signed date (July to November 2026). With no filter, the read returned 19 facts, one for each ground date. as_of_tslot 20589 (16 May 2026) returned the 16 May fact signed 7 November, NDVI -0.058. as_of_signed_at 2026-10-01T00:00:00Z returned the 18 September fact signed 28 September, NDVI -0.044. Both filters returned a second 16 May fact, signed 11 September, NDVI -0.061. It reads the same Sentinel-2B pass from another archive copy and is still stored. The receipt of that read carries the as_of block and is valid at /v1/verify_receipt." width="540">
<br><sub>Contains modified Copernicus Sentinel data 2025-2026.</sub>

When a read uses a time filter, the receipt carries an `as_of` block. An auditor can take a receipt from 2026 to any
node in 2027 and replay the same query.

## When two signed facts disagree

emem does not delete an old fact when a newer fact replaces it. `memory_contradictions` finds addresses where the
signed values differ. On 7 November 2026 a call with limit 40 returned 40 records from a partial scan of 44,705
keys. Each record had one signer that changed its source or its method. At one Sydney cell, road length was 8.45 m under
`overture_road_length_m@1` and 0.00 m under `@2`. Version 1 counted footways, rail and waterways as road. A plain
read returned 0.00 m. A read with `as_of_signed_at` set to `2026-09-01T00:00:00Z` returned 8.45 m.

![Two cases from one live POST /v1/memory_contradictions call with limit 40 on 7 November 2026, 13:25 UTC, which returned 40 records from a partial scan of 44,705 keys. Case 1: road length at a Sydney cell, about 10 m by 8 m. Signed 29 June 2026: 8.45 m, method overture_road_length_m@1, Overture release 2026-06-17.0. Signed 11 October 2026: 0.00 m, method overture_road_length_m@2, release 2026-09-23.1. Version 1 counted footways, pedestrian areas, rail and waterways as road; version 2 counts only roads a vehicle can use. Recall today returns 0.00 m; recall with as_of_signed_at 1 September 2026 returns 8.45 m. Case 2: tree cover in 2000 at 20.47 S 55.79 W in Brazil. Signed 4 times from 4 July to 16 September 2026: 17 percent, Hansen GFC v1.12, pixel reader before the fix. Signed 30 September 2026: 0 percent, GFC v1.13, reader cog-pixel-floor@2. Recall today returns 0 percent. Both the release and the reader changed, so the record does not say which change moved the value. Every receipt checks valid.](media/stills/contradictions.webp)

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
