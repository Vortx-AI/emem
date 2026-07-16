# Limits and roadmap

One page for the edges: what emem does not do yet, the staged work to
federation, and the open research list. Everything here says what
already ships and what is still open, so nothing is a promise dressed
up as a feature. The short version lives in the
[README](https://github.com/Vortx-AI/emem#readme); this is the whole
picture.

## Honest limits

emem is version 1.0.0, its first stable release: the wire format, the receipt preimage, and the cell64 address space are settled and will not break under a 1.x. What it does not do yet, so you can plan around it:

- **Single host.** No federation, no global routing, no SOC 2 yet. One responder, one signing key. Durability today is the hosted node plus any node you run; content addressing means any node that holds the bytes can re-serve and re-verify them, so run your own if the facts matter to you.
- **Thousands of places, not billions.** The memory grows every day it is used, but it is early. Check the live count before you assume coverage.
- **It grounds facts about physical places,** not arbitrary text. It is not a general-purpose citation store for any document.
- **Place ids are compact, not yet token-optimal.** A `cell64` measures 12 to 13 BPE tokens today. The id format is built for a tokenizer-optimized alphabet that would cut that further; the shipped alphabet does not achieve it yet.
- **The learned predictor is an honest baseline.** `jepa_predict_v2` carries a dynamics head trained on wholly synthetic sequences; on real NDVI pairs it does not beat persistence (skill -0.064), so the endpoint serves the persistence baseline and says so with an `untrained_baseline` warning. Treat it as a research surface, not a forecast; the measurements live in the whitepaper's honest limits (section 14.5).
- **Some foundation-model fingerprints are sidecar-gated on the hosted node** today. A cold place returns a signed absence for those, so `triple_consensus` runs partial when cold. Tessera fetches on demand.
- **Upstream rate limits.** Some sources are rate-limited or slow to fetch (one land-surface-temperature source takes about 30 seconds per place).
- **No sub-meter imagery** in the default build, and no notebook UI. Drive it from a notebook against REST or MCP.
- **Agent-memory writes are attested; reads are not yet tenant-scoped.** As of 2026-07-14 the global `/memories/` namespace is closed to unauthenticated writes: every write needs a valid ed25519 `attester` block, so no anonymous caller can plant a fact that surfaces in another agent's recall. Confidentiality is coarser than that: the flat namespace is a shared commons any caller can read, private per-agent state belongs in `/memories/by_attester/<pubkey>/` or the capability-gated `vault`, and full owner-scoped read isolation across the whole namespace is open work (see below). Do not put secrets in the flat namespace.
- **The corpus is thin and skewed.** Facts materialize lazily, one place at a time, from upstream sources, so coverage today is deep in a few bands and empty in the tail. Check the live count and the specific band before you assume a place is warm; a cold place can time out on first read and return a typed `skipped` note, not a value. Filling this fast is the supply-side work below.

## Where it is going

emem is a protocol, not a single service. The end state is a federation of independent responders that resolve the same ids byte-for-byte, cross-cite each other, and record where they disagree. One account of the world is a claim; several independent ones that verify against each other are a record. **The multi-host federation routing does not ship in 1.0.0.** What ships today is the machinery it stands on: content addressing, signed receipts, an append-only attestation log with per-fact merkle proofs, a multi-writer attest endpoint, typed temporal links, cross-source disagreement scoring, and an offline refinement loop.

The staged work from here, building on those pieces:

1. **One public transparency log.** Signed tree heads and consistency proofs over the existing append-only log, so no responder can tell two clients two different stories about the same history.
2. **Signed absence proofs.** Turn "no fact here" from a signed statement into a checkable non-membership proof.
3. **A public attester spec.** So partners and customers run their own signing nodes against the write path that already exists.
4. **Quorum reads across responders.** Content addressing makes agreement trivial to check: same canonical bytes, same id, k signatures. The design is written up in [`federation.md`](federation.md).

Federate later; the fact ids will not move. Near-term work is tracked in [issues](https://github.com/Vortx-AI/emem/issues).


## Open research

emem works end to end today. This is the honest list of what it does not do yet, kept in one place so a contributor or an agent can see where the edges are and pick one up. Two threads run through all of it. The first is the substrate: memory an agent can trust, carry between vendors, and verify without a callback. The second is the worlds at [`/worlds`](https://emem.dev/worlds), which are not the product but the proof, the most direct way to show a memory can be both generated and checkable at the same time. Each item says what already ships and what is still open, so nothing here is a promise dressed up as a feature.

The test we hold every item to: does it make emem's memories more trusted, portable, and verifiable? If yes, it belongs here. Recall makes an agent convenient; verifiable memory makes it accountable, and accountability is the part still missing between today's demos and agents a regulated business will let run on their own. The same test carries the longer thesis: generating a plausible answer is cheap and getting cheaper, and what stays scarce is a shared account of the physical world that is measured, signed, and checkable by someone who was not there. Every item below exists so an agent inherits that account instead of re-observing it.

### Change attribution: why did this place's readout move

- **A change-attribution primitive.** When the readout at a pinned
  reference moves between two visits, the move is not one event. It
  decomposes as `Δz = Δ_env + Δ_sensor + Δ_geo + Δ_encoder + ε`: the
  world changed, the instrument changed, the pixels moved, the model
  changed, noise. Only the first term is about the world, and a memory
  that cannot tell them apart either raises false alarms (a sensor swap
  read as a wildfire) or hides real events (a flood read as a
  registration error). What ships today pins most of that ledger by
  construction: an embedding fact carries its encoder checkpoint and its
  versioned recipe, so a model swap can never pose as change on the
  ground; bi-temporal recall keeps "the world changed" and "what the
  memory knew changed" as separate bounds; every change points at a
  specific immutable fact by cid; and the receipt makes whatever split
  is computed checkable by a third party. What does not exist is the
  split itself: `emem_diff` returns a raw delta (`nd.delta@1`),
  `state_diff` returns residual, L2, and cosine, and `triple_consensus`
  reports agreement across three encoders behind a gate it documents as
  uncalibrated (Prithvi's deltas top out near 0.1155, so the strongest
  verdict is unreachable). Nothing attributes. Open: the estimator, a
  calibrated cross-encoder and cross-sensor stability model, and the
  primitive itself as `change_attribution@1`: a `runtime_evaluable`
  recipe in the algorithm registry, an executor modelled on
  `triple_consensus`, a tool on the `burn_severity` pattern, and output
  as a derivative fact with parent cids under a signed receipt, so the
  attribution is as citeable and offline-checkable as the readings it
  explains.
- **The inputs are fields, not points.** Attribution over an area needs
  the native-resolution raster surface further down this page; the two
  items share one keystone.

### Next substrates: everything that observes a location

Satellite Earth observation is the first substrate because its data is
open, global, and already keyed by location. The protocol requirement
for any other substrate is the same pair: a canonical address and a
signing key. Location stays the first key throughout.

What ships today that a new substrate would use unchanged: the
multi-writer attest endpoint (`POST /v1/attest`, ed25519 envelope over a
merkle root, no transport auth to configure), per-attester namespaces,
the provenance classes with in-band cautions, contradiction scoring
between writers, and the token grammar. What is open per substrate is
the written profile: which bands, which canonical source scheme, which
provenance class each measurement carries.

Candidate substrates, in the order the machinery favours them:

- **CCTV and fixed sensors.** A camera or gauge is the simplest case: a
  fixed location, a stream of observations, one key. Its readings land
  as facts at its cell, timestamped and signed at the edge.
- **Drones.** A survey flight is a moving attester writing along a
  path; each observation lands at the cell it measured, and the flight
  becomes citable evidence rather than a file someone hosts.
- **Robot fleets.** Landmarks as `emem:entity:` identities, terrain,
  hazard, and traversability as signed facts. Two vendors' robots share
  one map and verify each other's contributions without a shared
  backend. A runnable two-vendor example ships today at
  `examples/fleet-memory/` (plain HTTP, runs on the robot); a ROS 2
  client package is open work.
- **Industrial machines.** A meter, a turbine, a pipeline sensor:
  location-pinned observations whose provenance class says exactly how
  the value was produced.
- **Government registries and open data programs.** Cadastral records,
  permits, environmental monitoring: `human_curated` by class, carrying
  that caution in-band, signed by the issuing authority's key.

None of these profiles ship yet; listing them here is direction, not
availability. The test each one must pass is the same as the first
substrate's: observations sign at the source, resolve byte-identically
anywhere, and verify offline.

### Tenancy: closing the memory layer before scaling ingest

This is the precondition for everything on the supply side. A shared
memory that any caller can write is a memory any caller can poison, and
that gate has to hold before bulk ingest or a per-tenant enterprise layer
is safe to build.

What ships today (2026-07-14): the global namespace is closed to
unauthenticated writes. Every `memory_create` / `str_replace` / `insert`
/ `delete` / `rename` requires a valid ed25519 `attester` block
(`RequireAll` is the release default; an operator can re-open for the
unattested Anthropic memory-tool contract with `EMEM_MEMORY_OPEN=1`).
Attested writes default to the caller's own
`/memories/by_attester/<pubkey8>/...` space, the shortcode is bound to the
signing key, and the `vault` kind seals bytes under a capability so its
reads are gated. This closes the memory-poisoning vector: an anonymous
caller can no longer plant a `kind=semantic` file that surfaces in another
agent's `memory_search`.

What is open, and gates the enterprise story:

- **Owner-scoped reads.** Reads, listing, and search currently carry no
  caller identity, so the flat namespace is a shared commons and even the
  per-attester namespace is world-readable. Real confidentiality needs a
  caller-identity channel threaded through the read path so a private
  namespace returns only to its owner. Until it ships, private data belongs
  in `vault` or `by_attester`, never the flat commons.
- **Per-tenant isolation.** The four-tuple `Scope {user_id, agent_id,
  run_id, org_id}` already scopes geospatial facts; extending the same
  scope to the memory-file layer is the multi-tenant primitive the private
  ingest layer below is built on.

### The supply side: filling the corpus

Separate demand from supply, because they have different answers. On the
demand side, meaning agents reading and citing emem, MCP plus REST plus
OpenAPI is the right surface and is good enough; the work there is
ergonomics, not more protocols. The supply side, meaning getting the
world's observations into signed facts, is the real bottleneck, and the
corpus numbers show it: coverage is deep in a few bands and empty in the
tail because everything materializes lazily, one place at a time, from
slow upstream sources. A cold place can burn the per-source timeout and
return a signed hole instead of a value.

- **A geospatial database tokeniser.** Point emem at a table that has a
  geometry, a PostGIS column or a lat/lng pair, and each row maps to a
  `cell64` and becomes a signed, citeable fact. Bulk ingest fills the
  corpus orders of magnitude faster than per-cell materialization and
  removes the cold-start timeout, because the data is already local and
  indexed. Three constraints keep it honest: it is geospatial only, a row
  without a geometry is not Earth memory and does not belong here; it runs
  as a private, per-tenant ingest under [geo.qa](https://geo.qa), not on
  the public commons, which is the tenancy work above at scale; and the
  signature attests ingestion, not truth. Tokenising a row signs "this
  responder ingested these bytes, from this source, at this time" and
  carries the `human_curated` or `model_output` provenance class with its
  in-band caution. It gives tamper-evidence, ingestion provenance, and
  cross-agent citability. It does not give ground truth, and the claim
  will not outrun the receipt. The primitive underneath is "tabular geo
  source in, signed facts out," so one engine serves Postgres first,
  Snowflake next, and a plain CSV or Parquet path (which covers Excel
  exports) as a mode rather than a separate product.
- **Scheduled densification for priority areas.** Coverage is deep in a
  few bands and empty in the tail because everything materializes
  lazily, one place at a time. Open: a background warmer that takes a
  declared priority area and fills it across a window before anyone
  asks. This is the supply-side face of the memory preparer under
  partial results further down, and it is gated on the same design:
  cells times tslots is strictly larger than the cold polygon that
  already does not fit, so a warmer that pretends to be synchronous is
  just a slower 504. Settle partial results first; the warmer is then a
  cell list on `emem_backfill` plus a schedule.

### Client surfaces, ranked by leverage

- **A first-class SDK.** In the repo today: typed clients for Python and
  TypeScript, plus a LangChain `BaseStore` adapter (`emem-langmem`) that
  signs its writes, each wrapping the REST surface so a signed receipt is
  the only new thing a caller learns. **Not yet installable.** Every
  `ememdev` release on PyPI (0.0.9, 0.1.0 and 1.0.0) is about 2.4 KB of
  metadata containing no Python modules: a root-anchored `/src/` pattern in
  the repo's `.gitignore` was re-anchored by the build backend to the SDK
  directory, so each wheel built clean and empty, and `pip install ememdev`
  then `import emem` raises `ModuleNotFoundError`. CI tested the source
  tree over `PYTHONPATH` and never the artefact, so it stayed green
  throughout. `@emem/client` failed more quietly: it has never been
  published, and it did not compile, carrying three type errors under
  `strict` that no CI job ran `tsc` to catch.

  The builds are fixed. Three publish workflows (`publish-pypi.yml`,
  `publish-pypi-langmem.yml`, `publish-npm.yml`) each build the artefact,
  look inside it, install it into a fresh environment and import it, and
  refuse to upload anything that ships no modules or will not import.
  `ememdev` is bumped to 1.0.1 because PyPI versions are immutable and the
  empty releases can only be superseded, never replaced. One more trap is
  external: the PyPI name `emem` belongs to an unrelated company's real
  package, so the install name stays `ememdev` and the import name is a
  decision to make deliberately before the first verified publish, either
  rename the module to match the install name or document the divergence
  loudly. A developer who guesses `pip install emem` installs someone
  else's library.

  Open, and each blocked on a human once. PyPI registers a Trusted
  Publisher per project, so `ememdev` and `emem-langmem` each need one
  registered before their first upload. npm cannot register a trusted
  publisher for a package that does not exist, so the first `@emem/client`
  publish needs an API token, and the `@emem` scope has to be owned before
  any of it works. After that first upload each ships from one
  `workflow_dispatch`. Then a warm-and-retry wrapper that hides the
  cold-start timeout on first read.
- **Framework adapters as thin wrappers over the SDK.** LangChain,
  LlamaIndex, CrewAI, the Claude Agent SDK, the OpenAI Agents SDK. Build
  one SDK and make each adapter a thin layer over it; hand-maintaining six
  integrations against raw MCP is a maintenance trap. Medium leverage, and
  only after the SDK ergonomics above land.
- **A native stdio MCP transport.** The server speaks Streamable HTTP at
  `/mcp`, which is what Docker-hosted directory listings (Glama) run, and
  the docs recommend the `mcp-remote` bridge for hosts that need stdio.
  A native stdio transport would make the one-click, no-bridge listings
  work directly. Open, and lower priority than the SDK because the bridge
  already covers it.
- **Robotics, as a single lighthouse.** Real and aligned with the fleet
  story, and later. It needs an edge encoder, offline sync, and a
  non-HTTP transport, because ROS 2 and intermittent-connectivity robots
  are the wrong shape for an HTTP MCP call. Do not start until the SDK,
  the signed-write path, and tenancy are solid; when it starts, one
  flagship design partner beats a generic ROS package. A runnable
  two-vendor HTTP example ships today at `examples/fleet-memory/`.

### Distribution: being findable and installable

Adoption is a distribution problem before it is a content problem, and
the two leaks sit at the top of the funnel: a developer cannot install
the SDK yet (above), and an agent browsing a registry does not find the
server. Nothing here ships a new capability; all of it decides whether
the existing ones are found.

- **Registry listings and a server card.** emem is absent from the MCP
  registries where servers get found: the official registry, mcp.so,
  Smithery, glama.ai, PulseMCP, and awesome-mcp-servers. Open: one
  metadata pack (name, Streamable HTTP transport, `https://emem.dev/mcp`,
  tool list, repo) prepared once and submitted everywhere; the seeds
  already exist under `docs/registries/`. With it, a
  `.well-known/mcp.json` server card generated from the same source of
  truth as the live tool surface, so the card cannot drift, stating the
  honest auth posture: reads open, writes ed25519-attested.
- **Framework integrations as published packages.** The LangChain,
  LlamaIndex, CrewAI, Agno, AutoGen, and Mastra examples exist as config
  files. The channel that compounds is being an installable, listed
  integration inside the frameworks developers already use.
  `emem-langmem`, the BaseStore adapter that signs its writes, publishes
  first, then the framework directories. Gated on the first verified SDK
  publish above.
- **A launch that lands after the leaks are fixed.** One top-of-funnel
  moment, led by the demonstrable claim (the same token resolves to the
  same bytes for anyone, and the receipt verifies offline), not by
  satellites. The assets are the one-call win at the top of the README,
  a sixty-second recording of the cite-then-verify loop across two
  processes, the worlds, and one completable tutorial that ends in a
  token the reader can paste anywhere and a `/verify` link. Deliberately
  gated on the SDK publish and the registry listings: a launch that
  drives installs to an empty wheel burns the one spike.
- **A public reliability signal.** Registries and evaluators weight
  liveness. Open: an uptime signal for the hosted node, freshness
  signals the README does not have to restate (latest release, CI
  state), and aggregate, non-identifying usage counts if and only if
  they are consistent with the privacy policy.
- **A doc-lint gate.** The prose convention and the claim discipline are
  enforced by hand today. Open: a CI check that fails on the banned
  characters and filler-word list, verifies internal links resolve, and
  parses every token example in the docs, with the convention written
  down in CONTRIBUTING.md so outside contributors inherit it rather than
  rediscover it in review.

### What agents building on emem actually hit

Reported by an agent that built the 4D Gaussian-splat worlds at
[/splats](https://emem.dev/splats) from emem's signed facts, plus an
independent outside measurement of the MCP endpoint. Both landed on the
same shape of problem: emem computes the right thing and then collapses it
one function before the caller sees it.

- **Serialised tool discovery.** Ships today: `/mcp` advertises the
  14-tool loop rather than the full catalog, `/mcp/full` serves all of it,
  and `emem_tools` maps the whole surface on demand, returning any single
  tool's schema and a runnable example. Connecting costs 38 KB of context
  instead of 190 KB, with the map at 19 KB and one tool at about 2 KB,
  fetched only when wanted. `tools/call` still dispatches every tool by
  name at either endpoint, so narrowing discovery removes no capability.
  This supersedes an earlier attempt that paged the catalog behind a
  non-standard `nextCursor`, which hosts ignored.
- **The write path explains itself.** Ships today: a refused memory write
  returns the exact 32-byte digest to sign for that verb, path and body,
  the base32 rules, how `body_hash` is defined per verb, how the namespace
  shortcode is derived, and a worked example. None of it is secret, and an
  agent can go from refusal to a signed write in one turn without reading
  the source. MCP tool errors now carry their typed `details` too, which
  they previously dropped, making every typed error in this codebase
  invisible to exactly the callers that could not go and look it up.
- **A native-resolution raster for an area of interest.** Open, and the
  single change most likely to keep a world-model pipeline inside emem. A
  world model is a field, not a set of points, and emem has no way to
  return one: `build_cell_scene_rgb` reads only B04/B03/B02 and returns a
  256x256 8-bit stretched PNG that is not invertible for science, and every
  other route returns per-cell scalars. A 2 km area of interest holds
  40,040 cells at 10 m against `recall_polygon`'s `max_cells` limit of
  1024, so per-cell recall cannot express the query at all. The reporting
  agent wrote its own STAC and COG reader against emem's own upstream and
  got a clean 200x200 at 10.00 m/px, which means emem was cut out of its
  own pipeline. The primitive already exists: `cog::sample_window` returns
  a native-resolution row-major `Vec<f64>`, and all five of its call sites
  destroy it (three crush it to 8-bit RGB, one discards it to warm a cache,
  one collapses it to a single scalar per cell). The plumbing is not the
  hard part. The open question is what the receipt attests when a response
  is bulk bytes over a bounding box with no cell and no `fact_cid`, since
  that is the first emem response that would not be a signed fact addressed
  by cell. Worth answering deliberately rather than stapling a signature
  onto a byte pipe. As of 2026-07-16 this is a committed item, not just a
  reported gap. The shape is `POST /v1/band_raster` plus field tokens:
  `emem:raster:<aoi_cid>:<band>:<tslot>` names one field and `emem:cube:`
  names a stack of them across a window, where `aoi_cid`
  content-addresses the geometry so the same area always yields the same
  token. The receipt question above stays as the named design gate; it
  gets a deliberate answer before any of this ships.
- **Partial results instead of a timeout.** Open. A cold NDVI polygon
  cannot finish inside the 40 s gateway: worst case is roughly 31
  sequential upstream round trips for a single cell, and `recall_polygon`
  runs 64 cells in waves. Bounding the S2 materializer and parallelising
  the cloud-mask probe (both in flight) shrink the window but do not change
  the shape. An agent handed 40 of 64 cells plus a list of what is still
  materializing can proceed; a 504 teaches it to stop calling emem. That is
  an API change and it deserves a design.

  The same decision blocks a memory preparer: one call that says "warm this
  area across this window" so an agent can build its memory before it needs
  to reason over it. Nothing is missing from the read path, since
  `emem_recall` already materializes on a miss, so this is throughput and
  latency rather than capability. Two tools each cover half of it.
  `emem_backfill` walks every tslot for one `(cell, band)` across a window;
  `emem_recall_many` fans out over a list of up to 256 cells at one time
  selection. Nothing composes them, so an area across a window is the
  caller's own loop today. It also cannot simply be added: cells times
  tslots is strictly larger than the cold polygon above that already does
  not fit, so a synchronous preparer is impossible by construction rather
  than by tuning, and building one before this question is settled would
  ship a route whose honest answer is usually a 504. Settle partial results
  first and the preparer is a cell list on `emem_backfill` rather than a new
  tool. That ordering is also why none of the fetch verbs (`emem_backfill`,
  `emem_fetch`, `emem_data_availability`, the coverage pair) need promoting
  into the core 14 before then: the core loop already reads and
  materializes, and a preparer that cannot finish is not worth the context
  it costs to advertise.

### Protocol consistency

- **One verifier spec, generated from code.** Ships today:
  `GET /v1/verifier_spec` (also `/.well-known/emem-verifier.json`) emits
  the receipt preimage segment table directly from the `emem-attest` tag
  constants, so the offline-verification spec cannot drift from the signer
  the way a hand-written doc does. The receipt preimage is the single
  canonical, single-sourced construction: the signer and the
  `/v1/verify_receipt` verifier both call one function. Every object the
  responder signs now uses that one rule: the three bespoke
  pipe-delimited preimages (`corpus_state_stats`, `operator_attestation`,
  `stream.tick`) were folded onto the tagged `preimage_v1` family, and
  the attestation, STH, and witness segment tags are named constants the
  spec serializes rather than re-typed literals.
- **The one construction left outside `preimage_v1`, and why.** The
  memory-write attester binding
  (`blake3("emem.memory_write|" || verb || "|" || path || "|" ||
  body_hash)`) is signed by the *caller*, not the responder, and its
  shape is pinned by the whitepaper and by every client that already
  signs against it. Migrating it would break those clients to buy
  consistency in a preimage that is already domain-separated and already
  carries the verb, so a signature cannot cross verbs or paths. It stays
  as it is, and `GET /v1/verifier_spec` lists it under
  `caller_signed_objects` with its per-verb body rule rather than
  omitting it. What it lacks against `preimage_v1` is explicit length
  prefixes. That costs nothing at its current shape: the verb is supplied
  by the responder from a closed set that contains no separator, and
  `body_hash` is a fixed-length 32-byte digest at the tail, which leaves
  the path as the only variable-length field and brackets it
  unambiguously. A path may itself contain a `|` without collision. The
  ambiguity length-prefixing exists to prevent needs two adjacent
  variable-length fields, and there are none. Revisit if the binding ever
  grows a second one.

### The paper: one canonical, submittable whitepaper

Two whitepapers overlap today. v1 carries the protocol depth and is
archived unedited as the DOI-cited record; v2 carries the framing and
the honesty discipline. A submittable paper is one document, and what
the prose still lacks is exactly the scaffolding a reviewer scans for.

- **One canonical paper.** Merge with v2's spine (referential drift in
  both directions, external identity) and v1's protocol depth (cell64,
  tslot, the band voxel, algorithms, primitives) folded into the design
  sections. The old files stay where they are, marked superseded, since
  v1 is the frozen citation target behind the DOI.
- **The scaffolding.** Formal definitions: the address map, the cid as
  blake3 over canonical CBOR, the tagged length-prefixed preimage
  stream, the bi-temporal read predicate, and the change decomposition
  already stated in the whitepaper's sections 1.1 and 10.3. A figure
  set: architecture, addressing, the preimage, the token lifecycle, the
  log, the decomposition. A reference list that covers agent memory, EO
  foundation models, content addressing, provenance standards, and
  verifiable data structures, every identifier checked to exist before
  it is cited. An evaluation section that lifts the measured numbers
  from [benchmarks.md](benchmarks.md) verbatim and keeps their
  sample-versus-full honesty split. The honest-limits list restructured
  once as an explicit adversary model: what a malicious sender, a
  malicious responder, and a network attacker each can and cannot do,
  with split-view equivocation and the first-contact baseline named.
  And a LaTeX source for the submission itself, since the repo renders
  HTML today and preprint servers compile LaTeX.

### The substrate: trusted, portable, verifiable memory

- **A drop-in memory API that returns a receipt.** Ships today: `memory_create` and `emem_memory_search` write and read a private per-agent memory, `emem_memory_token` mints the `emem:fact:` handle, and `emem_verify_receipt` checks any of it offline. Open: the same three-line `add` / `search` ergonomics the popular memory frameworks expose, so a signed receipt is the only new thing a caller has to learn, plus a public head-to-head on the recall benchmarks (LongMemEval, LoCoMo), reported alongside the offline verification those frameworks do not provide.
- **A memory passport.** Ships today: `emem_memory_bundle` collapses a set of facts into one signed `emem:bundle:` token that resolves and re-checks anywhere. Open: a written import and export profile so that bundle carries between memory stores from different vendors, and the bundle as the unit of agent-to-agent handoff end to end: the self-teaching `how_to_sign` refusal extended to bundle writes, a `resolve_many` that pulls a whole handoff in one verified call, and immutable cache headers on resolved tokens so a client-side token store is trivial to build.
- **Typed identity kinds for objects.** An entity's `kind` is a free-text string today, which is enough to mint an identity and not enough to say what changed. One address carries several identities at once, and they change on different clocks: the coordinate never moves, the land cover cycles over years, the land use moves once over decades, the administrative status changes by decree. "Did this place change" has no answer until you say which of those you mean. Open: a small closed set of identity kinds (physical, material, ecological, functional, administrative), so "same place, different object" and "same object, changed state" stop colliding, and so change attribution can say what kind of thing changed instead of raising one alarm bit.
- **Signed state for agent-to-agent work.** Ships today: emem answers the A2A task surface at `/a2a/tasks` and serves a signed card at [`/.well-known/agent-card.json`](https://emem.dev/.well-known/agent-card.json). Open: an attestation that rides an agent card so two agents from two companies verify each other's claimed memory offline before acting on it, closing the trust gap the A2A spec leaves to implementers.
- **Quantitative evaluation in the open.** Ships today: every receipt carries its own cost block (`latency_p50_ms`, `was_cached`), `emem-membench` scores retrieval accuracy against a running responder with no hardcoded numbers, and [benchmarks.md](benchmarks.md) publishes dated, single-node latency and throughput measurements with the method next to each number. Open: multi-node scaling, storage per fact, cache-hit ratio under realistic mixes, and head-to-head comparisons against spatial databases and geospatial data infrastructures on identical queries.
- **Substrate generality.** The signed fact, the receipt preimage, the provenance class, and the token grammar operate on any canonical address; nothing in them is satellite-specific. Ships today: Earth observation as the first substrate over cell64. Open: a written profile for a second sensing domain with its own canonical address space, plus the formal memory model started in [model.md](model.md): the observation tuple, the property table with mechanisms, and the shipped memory algebra, with machine-checked proofs, uncertainty propagation, semantic compression, and a conformance profile as the open work.
- **The cell build graph.** Ships today, at the leaf level: every derived observation names its versioned rule (`derivation.fn_key` in the content-addressed algorithm registry), its hashed inputs (`sources[]`), and its content-addressed output, and recall-on-miss is a demand-driven build with cache semantics in the receipt. Determinism means derived layers are evictable without breaking citations: rebuilding yields the same bytes and the same cid. Open: multi-hop derivation as signed objects (facts to gaussians to meshes to navigation surfaces; the worlds bake prototypes the first hop), the one-traversal lineage graph, and a target language, so a cell compiles like a Bazel or Nix package but verifiably across trust domains. The consequence for long-running agents: the world owns the state, not the session. What exists, what is still valid (`/v1/temporal_route`), and what replaced what are all one call away, recall with a band list is already a single-hop `ensure`, and a thousand agents coordinate through shared content-addressed artifacts instead of messages. The atom underneath it stays the existing observation, band is its type field, not the object, and the open generalisation is artifact-typed values (a mesh signed and cited exactly like an NDVI reading), with derivation discovery on top: rules stay declared and signed, which chain applies at a cell is learned, and the planner answers `prove(goal)` with an artifact plus its validity chain. The compiler and execution-substrate view is written down in [model.md](model.md).
- **Re-runnable derivations.** `/v1/derive` records a `code_cid` for a caller-registered derivation but never fetches or runs it, so a registered derivation is a signed assertion, and it is forced to the `model_output` class no matter how deterministic its code is. Open: a sandbox tier for pure functions that re-runs the `code_cid` against the parent tokens and, when the output reproduces byte-for-byte, earns the derivation `deterministic_index`. That makes a derived artifact independently recomputable from raw source bytes by a stranger, which is a property no unsigned map or proprietary reconstruction service offers.
- **Derivations you can find by standing at the place.** A registered derivation is reachable only by its token today; `recall` at the same cell does not surface it, so a co-located agent recomputes what a peer already derived instead of co-referring to it, which is referential drift by another route. Open: opt-in place-indexing of registered derivations, typed clearly as derivative and attributed to their attester, plus the reverse-lineage index ("what is downstream of this parent fact_cid") so that when an upstream source moves, staleness propagates to everything built on it. Knowing what to invalidate when an instrument or source changes is the sensor term of change attribution, operationalized.
- **An audit trail for regulated work.** Ships today: bi-temporal recall (`as_of_tslot` for what was on the ground, `as_of_signed_at` for what the memory knew) and a signed absence for what was never there. Open: the profile that turns those into a procurement-grade record of what an agent knew and when, aimed at the data-provenance gap that content-only provenance standards do not cover.

### The worlds: making the proof denser, live, and portable

- **The responder URL in baked provenance.** Done. A bake fetches from a fast local node but the sidecar now records the public responder the artifacts are served from, so the re-check recipe points somewhere a reader can actually reach. The signing key is unchanged, so every receipt still verifies. See `--public-responder` in `examples/3d-worlds/make_splats.py`, wired through `scripts/bake_worlds.sh`.
- **Provenance-preserving densification.** The exporter now does this. `python3 examples/3d-worlds/make_splats.py --densify F` subdivides each grid quad and writes `emem.splat_provenance.v2`, in which every splat is labelled `measured` (its own `fact_cid`) or `derived` (its up to four source cells, their `fact_cids`, and bilinear weights that sum to 1), so a derived continuous value is exactly re-derivable as `sum_i weight_i * source_i` and every source stays signature-checkable. Categorical bands (a loss year, a class code) are inherited from the nearest signed cell rather than averaged, and a node on an original cell stays that exact signed cell, so densifying never invents a value or drops a measured fact. `--check-derived` re-verifies a whole sidecar offline. The live `/worlds` viewer now densifies in lockstep with the exporter (the `splat-math.js` and Python paths are pinned to 1e-6 by a golden fixture), with a detail control and a pick panel that resolves any derived splat to its signed sources. Still open: carrying the same labelling into a standard splat container (next item).
- **A world that rolls forward.** Open. `emem_jepa_predict_v2` is built to predict a cell's next step from its attested history; today it serves the persistence baseline with a warning, per the honest limit at the top of this page, so this item is a design, not a capability. Applied across a whole baked world, a predictor with real skill becomes a sequence of scene frames, each one a signed forecast that says it is a forecast, carrying the model id and the lags it read. A generated frame nobody has to take on faith. Rolling forward also wants a subscription surface, a token that resolves to the current value and says when it changed, instead of a caller polling recall in a loop; open, filed from the agent channel.
- **Riding the splat standard.** Open. The worlds emit a bespoke 32-byte splat plus a PLY. As gaussian splatting consolidates on glTF and compressed transport formats, emem's provenance should ride inside the standard as a custom block, so any viewer renders the geometry and only emem-aware clients light up the click-to-verify layer.
- **Planet scale.** Open. The cell ids are already hierarchical, so a world can become a tile pyramid: coarse gaussians far out, finer tiles baked on demand as a camera or an agent drills in, cached the way recalls already are.
- **Generative where the memory is empty.** A first cut is live, the rest is the furthest-out item here. Where no fact exists, generate a plausible value from the embedding field and its neighbours, but stamp it with its own class of id, its model, its conditioning cells, and a confidence, so an agent can ask for measured cells only, or measured plus inferred, and always know which is which. The dense worlds at [`/splats`](https://emem.dev/splats) show the shape of it: every splat is labelled `measured`, `interpolated`, or `synthesized`, the measured trust root stays ed25519-signed, and the invented layers peel back off, so a viewer can drop to grounded-only in one step. Open is generalising that labelling beyond a splat scene to arbitrary bands, and giving each generated value the same signed envelope a measured fact gets. Grounded where grounded, generative where not, and labelled either way.

Near-term protocol work is tracked in [issues](https://github.com/Vortx-AI/emem/issues); the reasoning behind the sequencing is in the [whitepaper](https://doi.org/10.5281/zenodo.20706893).

