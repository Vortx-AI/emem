# Limits and roadmap

One page for the edges: what emem does not do yet, the staged work to
federation, and the open research list. Everything here says what
already ships and what is still open, so nothing is a promise dressed
up as a feature. The short version lives in the
[README](https://github.com/Vortx-AI/emem#readme); this is the whole
picture.

## Honest limits

emem is at version 2.1.0, a minor that adds `emem-guard` and breaks nothing. The receipt preimage last moved in 2.0.0: the 1.x line promised the wire format, the receipt preimage and the cell64 address space would not break under a 1.x, and 2.0.0 changed the preimage, so it was a major for exactly that reason. Receipts signed under 1.x verify unchanged under the 1.x rule, and a verifier selects the rule from the receipt's own `preimage_version`. The cell64 address space is untouched and stays settled. What it does not do yet, so you can plan around it:

- **Single host.** No federation, no global routing, no SOC 2 yet. One responder, one signing key. Durability today is the hosted node plus any node you run; content addressing means any node that holds the bytes can re-serve and re-verify them, so run your own if the facts matter to you.
- **Thousands of places, not billions.** The memory grows every day it is used, but it is early. Check the live count before you assume coverage.
- **Two layers, two scopes, one write path.** The geospatial fact corpus grounds facts about physical places, not arbitrary text, and is not a general-purpose citation store for any document. The agent-memory layer under `/memories/*` is different on purpose: free-form, signed, BGE-searchable notes an agent wants another party or a later run to resolve and verify. What neither layer is, on the hosted node, is private; the next bullet says exactly how.
- **Place ids are compact, not yet token-optimal.** A `cell64` measures 12 to 13 BPE tokens today. The id format is built for a tokenizer-optimized alphabet that would cut that further; the shipped alphabet does not achieve it yet.
- **The learned predictor is an honest baseline.** `jepa_predict_v2` carries a dynamics head trained on wholly synthetic sequences; on real NDVI pairs it does not beat persistence (skill -0.064), so the endpoint serves the persistence baseline and says so with an `untrained_baseline` warning. Treat it as a research surface, not a forecast; the measurements live in the whitepaper's honest limits (section 14.5).
- **Some foundation-model fingerprints are sidecar-gated on the hosted node** today. A cold place returns a signed absence for those, so `triple_consensus` runs partial when cold. Tessera fetches on demand.
- **Upstream rate limits.** Some sources are rate-limited or slow to fetch (one land-surface-temperature source takes about 30 seconds per place).
- **No sub-meter imagery** in the default build, and no notebook UI. Drive it from a notebook against REST or MCP.
- **Agent-memory writes are attested; reads are not yet tenant-scoped.** As of 2026-07-14 the global `/memories/` namespace is closed to unauthenticated writes: every write needs a valid ed25519 `attester` block, so no anonymous caller can plant a fact that surfaces in another agent's recall. Confidentiality is coarser than that, and the honest map is: the flat namespace and `/memories/by_attester/<pubkey>/` are both world-readable (only writes are attester-scoped), and the `vault` seals under a key derived from the RESPONDER's secret, so on the hosted node the operator can open a vault entry and the writing agent cannot. There is no client-controlled private memory on hosted emem today. Private agent state belongs in a local store or on a self-hosted responder, where you hold the key, until owner-scoped reads ship (see below). Do not put secrets in any hosted namespace.
- **The corpus is thin and skewed.** Facts materialize lazily, one place at a time, from upstream sources, so coverage today is deep in a few bands and empty in the tail. Check the live count and the specific band before you assume a place is warm; a cold place can time out on first read and return a typed `skipped` note, not a value. Filling this fast is the supply-side work below.

## Where it is going

emem is a protocol, not a single service. The end state is a federation of independent responders that resolve the same ids byte-for-byte, cross-cite each other, and record where they disagree. One account of the world is a claim; several independent ones that verify against each other are a record. **The multi-host federation routing does not ship yet in the 1.x line.** What ships today is the machinery it stands on: content addressing, signed receipts, an append-only attestation log with per-fact merkle proofs, a multi-writer attest endpoint, typed temporal links, cross-source disagreement scoring, and an offline refinement loop.

The staged work from here, building on those pieces:

1. **Receipts that name their log leaf.** The public transparency log itself shipped 2026-07-10 (signed tree heads, consistency and inclusion proofs, entry enumeration, and witness co-signing, all under `/v1/log/*`). What remains is chaining each receipt to its leaf, so tying one fact to the log is a single check instead of the receipt's batch proof plus enumeration, and gossiping heads between responders so a split view cannot survive.
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
  verdict is unreachable). Nothing splits a delta among the terms.
  Ships as of 2026-07-16, the LEDGER: `POST /v1/change_attribution`
  (tool `emem_change_attribution`, registry key `change_attribution@1`)
  reports per-term evidence for one cell, the observed Tessera
  year-over-year change, the label-free index pairs with their raw
  deltas, the sources each visit was observed through, the encoder
  pinning proof, and the scene class per visit, every item carrying its
  fact cids under a signed receipt, with `split` explicitly null and an
  in-band note saying why. The ledger also persists: each run stores
  itself as a derivative fact and returns its own `emem:fact:` token,
  so an attribution is cited and dereferenced like any reading. Open:
  the estimator and the calibrated cross-encoder and cross-sensor
  stability model the numeric split needs.
- **The inputs are fields, not points.** Attribution over an area needs
  the native-resolution raster surface further down this page; the two
  items share one keystone.

### Open validation: the measurements the claims still rest on

Every other item on this page is *build X*. This one is *prove X*, and it
is listed separately because the two go stale differently: a feature that
does not exist is obvious, while a claim nobody has measured looks exactly
like a claim that has been. Each item below names what the current
evidence actually establishes, which is usually narrower than the sentence
it supports.

- **The grid, compared rather than asserted.** Partly closed, and it went
  against us. Token cost is now measured: a cell64 is 12.5 BPE tokens
  (cl100k_base, mean over 14 live cells; 12.1 under o200k_base) against
  8.5 for H3 at a comparable pitch and 7.5 to 8.0 for a geohash or a hex
  S2 id, so the address is the most expensive of the four in the unit a
  language model pays in. Earlier drafts of the whitepaper claimed the
  opposite as the justification for the design; that sentence is now
  corrected. Still open, and needed before the choice can be defended
  rather than merely disclosed: encode and decode cost against the same
  three, address stability under a resolution change, area distortion as
  a function of latitude quantified rather than described, and whether
  the addressing scheme measurably changes recall latency at all. The
  honest position today is that cell64 loses to H3 on both equal-area
  and token economy, and buys decode-free prefix locality in exchange.
- **Per-encoder calibration for the change gate.** `triple_consensus`
  votes each encoder's cosine delta against one threshold, 0.15, borrowed
  from Healey et al. 2018's LandTrendr gate for spectral change. The
  embedding spaces do not share a scale: over 8 maximally dissimilar
  chips Clay spans 0.11 to 0.95 (sd 0.204) and the deployed Prithvi
  checkpoint spans 0.88 to 0.99 (sd 0.030) with its delta topping out
  near 0.1155, so Prithvi never votes and `all_three` is arithmetically
  unreachable. The responder says so in-band on every response. What is
  missing is not a better number but the corpus that would produce one: a
  labelled set of known-change and known-no-change cells per encoder,
  against which a threshold could be fitted and reported with a
  false-positive rate. Until that exists the three-tier vote is a report
  of which encoders moved, and one worked example cannot make it more.
  The quadratic-mean ensemble is likewise a choice and not a derivation;
  with components on different scales it is dominated by Clay.
- **Accuracy, measured separately from verifiability.** The grounding
  comparison scores whether an answer carries a resolvable `fact_cid`. A
  model with no tools cannot produce one by construction, so that result
  establishes that emem supplies checkable output and CANNOT establish
  that emem's values are closer to the truth than another geospatial
  source's. Those are different claims and only one of them is currently
  supported. What is needed is an arm that ignores citations entirely:
  emem's band values against independent reference data (published DEM
  validation sets, in-situ stations, an established provider queried
  directly), scored on absolute error, with the disagreements reported
  whichever way they fall. This is the single most load-bearing gap on
  this page, because "verifiable" is being read as "accurate" by
  everyone who has not read the method.
- **Contradiction scoring, quantitatively.** The primitive ships and now
  reports provider substitutions as well as multi-attester disagreement,
  but its severity score has never been evaluated. Needed: seeded
  disagreements of known magnitude across scalar, vector and categorical
  bands, and precision and recall of the score against them, so
  `severity: 0.62` means something to a reader who did not write it.
- **Memory search, quantitatively.** `emem_memory_search` is BGE over the
  agent-memory namespace and has no reported recall@k on any held-out
  set. Until it does, it is a feature with no number, and the honest
  description of it in any paper is "provided" rather than "effective".

### Next substrates: and the address space that gates all of them

Satellite Earth observation is the first substrate because its data is
open, global, and already keyed by location. The protocol requirement
for any other substrate is the same pair: a canonical address and a
signing key.

**The second half of that pair is done and the first half is not, and
until 2026-08-11 the registry did not say so.** The signing key
generalises completely: content addressing, the receipt, the append-only
log, bi-temporal bounds, contradiction scoring and the token grammar do
not care what the subject is. The ADDRESS does not generalise at all.
Every fact is keyed by `CanonicalKey { cell, band, tslot }` where `cell`
is a cell64, a 64-bit quantisation of latitude and longitude, so a fact
can only be recorded about a place. A file at a commit, a table at a
schema version, a model at a checkpoint and a span in an execution trace
have no latitude, and no amount of signing machinery gives them one.

That is why the README's "Earth is the substrate, not the subject" needs
reading carefully. It is true of the record, the receipt and the token
grammar. It is not true of the address, and the address is the part
everything else hangs from.

So every profile now declares an `address_space`, and the registry
refuses to load a profile that is `active` on one this build cannot key
a fact by. `geo.cell64` is the only address space with a write path
today. `entity.cid` names a subject that is not a place, using the
`emem:entity:` identity that already ships (mint, resolve and link are
core tools) but that a fact cannot yet be keyed by. Five profiles are
pinned on it as candidates, so builders can write against a fixed shape
while the honest status stays visible:

- **`space.deep.v1`**: a telescope SITE has a cell; the thing it looks
  at does not. The cleanest statement of the gap, on an instrument we
  already model.
- **`code.repository.v1`**: a file or symbol at a commit. Admitted by
  recomputability exactly as the Earth archives are, and the fit is
  closer than it looks: git is already content-addressed. What emem adds
  over git is the half git does not do, a signed reading ABOUT the code
  at that address, contradiction when two analysers disagree, and a
  bi-temporal answer to what we believed about this function in May.
- **`data.table.v1`**: a table or column at a schema version. The
  readings are the statistics teams already compute and cannot defend:
  null rate, cardinality, freshness lag, today passed around as
  screenshots. Recomputable only against an immutable snapshot.
- **`model.checkpoint.v1`**: a model at a checkpoint. Every encoder
  record already pins its checkpoint so a model swap cannot pose as
  change in the world; here the model is the subject and an eval score
  is the reading. Two labs quoting one benchmark are usually quoting
  two.
- **`exec.trace.v1`**: a span, as the subject rather than the evidence.
  Same verifier and same required layers as a device trace, inverted
  role.

**The keystone is one change: letting a fact be keyed by an
`emem:entity:` identity instead of a cell64.** It touches the canonical
index, the receipt preimage and the storage key, so it is a major, not
something to slip into a minor. Nothing above it should be built until
it lands, and nothing above it is claimed to work in the meantime.

What ships today that a new substrate would use unchanged: the
multi-writer attest endpoint (`POST /v1/attest`, ed25519 envelope over a
merkle root, no transport auth to configure), per-attester namespaces,
the provenance classes with in-band cautions, contradiction scoring
between writers, and the token grammar.

The next upgrade here is not another substrate. It is a trust layer,
and its core now ships as code. The written profiles live in the
ninth content-addressed manifest (`emem-substrates`,
`crates/emem-core/data/substrates-v0.json`), and every device-borne
profile carries the same admission rule: the protocol respects the
device as an observer of the physical world and refuses its output
alone. A device must present its complete, unaltered OS execution
trace for the capture window (`emem.os_trace.v1`, the `emem-trace`
crate), with the layers its profile requires (syscalls, scheduler,
memory tracks, energy, thermal, signal, sensor bus, network, storage,
on-device inference), digest-chained, merkle-rooted, and signed by the
device key together with the emitted output digests. Only output bound
inside a verified trace is admitted; the verifier names every failure
it finds. The founding Earth substrate keeps its own rule,
recomputability from free public archives, and takes the standing role
of drift anchor: device claims are contradiction-scored against it,
tension is surfaced rather than dropped, and the full design plus the
wiring steps live in `docs/plans/encoder-substrates.md`.

Candidate substrates that DO observe a location, in the order the
machinery favours them, each pinned as a profile in the registry and all
of them on `geo.cell64`:

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

The profiles above are pinned in the substrates manifest as
`candidate` status: declared direction a device maker can build
against today, not open ingest. The registry, the trace record, the
verification engine, the storage-side write gate (device enrollment
plus `put_attestation_gated`, with `emem:trace:` tokens and the first
committed conformance vectors), the `GET /v1/substrates` and
`POST /v1/trace_verify` surfaces with their MCP tools, and a runnable
satellite-operator example
(`crates/emem-primitives/examples/satellite_downlink.rs`) all ship;
the `os_trace` field on the hosted attest surface, an authenticated
enrollment endpoint, and the drift-anchor wiring are the open work,
in that order. The test each substrate must pass is unchanged
and now has a second half: observations sign at the source, resolve
byte-identically anywhere, verify offline, and carry the execution
evidence their profile demands.

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
  namespace returns only to its owner. This is the gate for two stories,
  not one: per-tenant ingest, and private agent memory. Until it ships,
  private agent state belongs in a local store or on a self-hosted
  responder; the hosted commons is world-readable, and the vault is
  openable by the operator, not by the caller.
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
- **Scheduled densification for priority areas.** Ships as of
  2026-07-16, exactly the shape this bullet predicted: partial results
  settled first, and the warmer is a cell list on `emem_backfill` plus
  a schedule. The preparer form takes up to 64 cells across a band and
  window under the partial-results contract, and the warmer loops it
  every `EMEM_WARM_INTERVAL_SECS` over `$EMEM_DATA/warm_priority.json`
  (re-read each pass, so the list edits without a restart), one
  budgeted bite per interval, never pretending to be synchronous. Off
  by default; coverage grows where an operator declared it matters,
  and convergence is the store filling up. Open: declaring areas as
  bboxes rather than cell lists, once the sampling helper is shared.

### Client surfaces, ranked by leverage

- **A first-class SDK.** In the repo today: typed clients for Python and
  TypeScript, plus a LangChain `BaseStore` adapter (`emem-langmem`) that
  signs its writes, each wrapping the REST surface so a signed receipt is
  the only new thing a caller learns. **The Python package is installable as of 2026-07-17** (`ememdev` on PyPI, a real wheel verified by clean-environment install and a live call), and the npm client shipped too as `@vortxai/emem` (scoped because npm refuses `ememdev` as too similar to `okemdev`), verified the same way; both registries are at 1.2.1. The history that made this bullet necessary: Every
  `ememdev` release on PyPI (0.0.9, 0.1.0 and 1.0.0) is about 2.4 KB of
  metadata containing no Python modules: a root-anchored `/src/` pattern in
  the repo's `.gitignore` was re-anchored by the build backend to the SDK
  directory, so each wheel built clean and empty, and `pip install ememdev`
  then `import emem` raises `ModuleNotFoundError`. CI tested the source
  tree over `PYTHONPATH` and never the artefact, so it stayed green
  throughout. `ememdev` failed more quietly: it has never been
  published, and it did not compile, carrying three type errors under
  `strict` that no CI job ran `tsc` to catch.

  The builds are fixed. Three publish workflows (`publish-pypi.yml`,
  `publish-pypi-langmem.yml`, `publish-npm.yml`) each build the artefact,
  look inside it, install it into a fresh environment and import it, and
  refuse to upload anything that ships no modules or will not import.
  `ememdev` is bumped past 1.0.0 because PyPI versions are immutable and
  the empty releases can only be superseded, never replaced. The SDK
  itself moved meanwhile: v1.1.0 adds `ememdev[signing]` and an `ememdev`
  CLI (whoami / sign / write) with a standard identity at
  `~/.emem/agent_ed25519.pem`, mirror-tested against the Rust preimage,
  so the signed write path is one command from the repo until the first
  verified publish lands. One more trap is
  external: the PyPI name `emem` belongs to an unrelated company's real
  package, so the install name stays `ememdev` and the import name is a
  decision to make deliberately before the first verified publish, either
  rename the module to match the install name or document the divergence
  loudly. A developer who guesses `pip install emem` installs someone
  else's library.

  Open, and each blocked on a human once. PyPI registers a Trusted
  Publisher per project, so `ememdev` and `emem-langmem` each need one
  registered before their first upload. npm cannot register a trusted
  publisher for a package that does not exist, so the first `ememdev`
  npm publish needs an API token. The npm package is `ememdev`, unscoped,
  the same name as the pip install: the `emem` name on npm is squatted by
  an unrelated package, and a scope would have needed an org nobody can
  claim. After that first upload each ships from one
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
  story, and now with its trust contract pinned: the `robot.fleet.v1`
  profile in the substrates manifest and the `emem.os_trace.v1` record
  in `emem-trace` define exactly what a robot must capture (syscalls,
  scheduler, memory, sensor bus, energy, thermal, on-device inference)
  for its writes to be admitted. What remains is the edge encoder that
  produces those windows on-robot, offline sync, and a non-HTTP
  transport, because ROS 2 and intermittent-connectivity robots are
  the wrong shape for an HTTP MCP call. Do not start the transport
  until the SDK, the signed-write path, and tenancy are solid; when it
  starts, one flagship design partner beats a generic ROS package. A
  runnable two-vendor HTTP example ships today at
  `examples/fleet-memory/`; upgrading it to trace-bound writes is step
  six in `docs/plans/encoder-substrates.md`.

### Distribution: being findable and installable

Adoption is a distribution problem before it is a content problem, and
the two leaks sit at the top of the funnel: a developer cannot install
the SDK yet (above), and an agent browsing a registry does not find the
server. Nothing here ships a new capability; all of it decides whether
the existing ones are found.

- **Registry listings and a server card.** More ships here than the
  first draft of this item claimed. Ships today: the official MCP
  Registry listing (`io.github.Vortx-AI/emem`, published automatically
  on every `v*` tag by `.github/workflows/mcp-publish.yml` over OIDC,
  current at the workspace version), and the server card at
  `/.well-known/mcp.json`, generated from the same tool registry as
  `tools/list` so it cannot drift, stating the honest auth posture:
  reads open, writes ed25519-attested. Open: the third-party
  directories (mcp.so, Smithery, glama.ai, PulseMCP,
  awesome-mcp-servers), each needing a human account or PR. The
  canonical copy for those submissions is
  [registries/metadata-pack.md](registries/metadata-pack.md); the older
  files in that directory carry April counts and are history, not
  copy.
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
- **A doc-lint gate.** Ships as of 2026-07-16: `scripts/doc_lint.py`
  runs in CI and fails on em and en dashes (legacy files ride a visible
  burn-down list inside the script that can only shrink), on the
  banned-vocabulary and buzzword lists, and on relative doc links that
  do not resolve; the convention itself is written down in
  CONTRIBUTING.md. On its first run it caught eight broken source links
  and one integrations row pointing at an example directory that does
  not exist. Open: parsing every token example in the docs, and burning
  the legacy dash list down to empty.

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
  token. The receipt question above now has its deliberate answer written
  down in [plans/field-tokens.md](plans/field-tokens.md): the receipt
  attests a derivation, not an array; the artifact is content-addressed,
  the signed object is a small derivation record with pinned sources and
  sampled per-cell anchors bridging to the existing trust, and
  verification has a spot-check tier and a full recompute tier. The
  owner signed off on 2026-07-16 and the build began the same day:
  the FIELD preimage segment, the evictable artifact store, the
  canonical grid encoding, and the `POST /v1/band_raster` executor all
  ship (six raw S2 bands, 512 px per side, best-effort anchors, the
  derivation record persisted on the ledger envelope). The
  `emem:raster:` resolve surface (`POST /v1/raster/resolve`, every
  token claim bound to the signed record, mismatch a typed 409) and
  the `emem_band_raster` plus `emem_raster_resolve` tools ship the
  same day. As of 2026-07-18 the field-token pair is complete:
  `emem:cube:` ships too (`POST /v1/band_cube`, `POST /v1/cube/resolve`,
  the `emem_band_cube` plus `emem_cube_resolve` tools, algorithm
  `band_cube@1`). A cube is a signed manifest over N independently
  resolvable `emem:raster:` slices, one per date; `cube_cid` is the
  blake3 of the ordered member derivation cids, resolve recomputes it
  and refuses an altered membership, and lineage terminates in each
  slice's pinned scene. A world model is a field over an area across
  time, and both halves of the token that names one now exist and are
  live. As of 2026-07-19 the field-token family is complete across five
  shapes, all on the one `POST /v1/band_raster` surface plus the cube
  endpoint: a raw band grid (`band_raster@1`), a cloud-free masked median
  composite (`s2_median_composite@1`), static terrain elevation
  (`dem_raster@1`, a Copernicus GLO-30 field with no scene selection), a
  field over time (`band_cube@1`), and a foundation-encoder embedding
  (`embedding_raster@1`, a multi-channel grid where each cell carries the
  encoder's N-D vector, e.g. 128-D geotessera, and anchors to its own
  signed per-cell fact). The grid encoding gained a channel dimension for
  the embedding shape without moving a single existing artifact_cid: a
  scalar grid still encodes byte-identically. The anchors spot-check tier
  shipped 2026-07-19 as an opt-in on `/v1/raster/resolve` (`spot_check:
  true`): it decodes the artifact grid, reads each anchor's value back out
  at its pixel, and cross-checks it against the independently-signed
  per-cell fact the anchor cites, so a field pixel that resolves to a
  signed receipt whose value matches is the click-to-verify a regulator
  wants. And `raster_bundle@1` binds any N field tokens into one signed
  `emem:rasterset:` manifest a world or a report cites. Still open:
  raising the embedding fan-out cap past one tile, and fp16 artifacts.
- **Partial results instead of a timeout.** Open. A cold NDVI polygon
  cannot finish inside the 40 s gateway: worst case is roughly 31
  sequential upstream round trips for a single cell, and `recall_polygon`
  runs 64 cells in waves. Bounding the S2 materializer and parallelising
  the cloud-mask probe (both in flight) shrink the window but do not change
  the shape. An agent handed 40 of 64 cells plus a list of what is still
  materializing can proceed; a 504 teaches it to stop calling emem. That is
  an API change and it deserves a design; the design is now written, in
  [plans/partial-results.md](plans/partial-results.md): partiality as a
  first-class 200 with a typed `pending[]` and monotone convergence by
  identical-request retry, no job queue, because materialization
  already persists and the store is the state. The owner signed off
  the same day and step 1 ships: `recall_polygon` takes `budget_ms` and
  answers partially with a typed `pending[]` and monotone
  identical-request retry. All four steps ship: `recall_many` carries the same
  contract, `emem_backfill` grew the preparer form (a `cells` list,
  the same band and window, the same contract), and the densification
  warmer loops the preparer on a schedule, off by default until the
  operator declares a priority list.

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

- **One canonical paper.** Done, at [whitepaper.md](whitepaper.md)
  (2026-07-16): v2's spine (referential drift in both directions,
  external identity) with v1's protocol depth folded into the design
  sections, and the old files marked superseded in place, since v1 is
  the frozen citation target behind the DOI.
- **The scaffolding, mostly shipped with it.** Shipped: a definitions
  and notation section (the address map, the cid rule, the tagged
  preimage stream, the bi-temporal read predicate, the change
  decomposition with reference stability stated formally); six figures
  with captions; a reference list of 26 entries, every arXiv identifier
  verified against the arXiv API before inclusion; an evaluation
  section lifted verbatim from [benchmarks.md](benchmarks.md) keeping
  its sample-versus-full honesty split; and the honest-limits list
  restated as an adversary model (malicious sender, malicious
  responder, network attacker, malicious writer, with split-view
  equivocation and the first-contact baseline named). Still open: a
  transparency-log figure, and the LaTeX source for the submission
  itself, since the repo renders HTML today and preprint servers
  compile LaTeX.

### The substrate: trusted, portable, verifiable memory

- **A drop-in memory API that returns a receipt.** Ships today: `memory_create` and `emem_memory_search` write and read a private per-agent memory, `emem_memory_token` mints the `emem:fact:` handle, and `emem_verify_receipt` checks any of it offline. Open: the same three-line `add` / `search` ergonomics the popular memory frameworks expose, so a signed receipt is the only new thing a caller has to learn, plus a public head-to-head on the recall benchmarks (LongMemEval, LoCoMo), reported alongside the offline verification those frameworks do not provide.
- **A memory passport.** Ships today: `emem_memory_bundle` collapses a set of facts into one signed `emem:bundle:` token that resolves and re-checks anywhere. Ships as of 2026-07-16: `POST /v1/memory_token/resolve_many` dereferences up to 256 tokens in one call, partial by design with typed per-item errors, one batch receipt over the resolved union plus a receipt per item; and token dereferences carry immutable cache headers (a content-addressed body never changes and a receipt never expires), so a client-side token store is trivial. Open: a written import and export profile so that bundle carries between memory stores from different vendors, and the self-teaching `how_to_sign` refusal extended to bundle writes. The session-state worked example ships as of 2026-07-16: [examples/agent-handoff/](https://github.com/Vortx-AI/emem/tree/main/examples/agent-handoff) parks a checkpoint as one signed memory file on a throwaway local responder and a second identity resumes from it, checking the content id and the receipt before reading a word; every command in it is the script itself.
- **Typed identity kinds for objects.** An entity's `kind` is a free-text string today, which is enough to mint an identity and not enough to say what changed. One address carries several identities at once, and they change on different clocks: the coordinate never moves, the land cover cycles over years, the land use moves once over decades, the administrative status changes by decree. "Did this place change" has no answer until you say which of those you mean. Open: a small closed set of identity kinds (physical, material, ecological, functional, administrative), so "same place, different object" and "same object, changed state" stop colliding, and so change attribution can say what kind of thing changed instead of raising one alarm bit.
- **Signed state for agent-to-agent work.** Ships today: emem answers the A2A task surface at `/a2a/tasks` and serves a signed card at [`/.well-known/agent-card.json`](https://emem.dev/.well-known/agent-card.json). Open: an attestation that rides an agent card so two agents from two companies verify each other's claimed memory offline before acting on it, closing the trust gap the A2A spec leaves to implementers.
- **Quantitative evaluation in the open.** Ships today: every receipt carries its own cost block (`latency_p50_ms`, `was_cached`), `emem-scorecard` scores retrieval accuracy against a running responder with no hardcoded numbers, and [benchmarks.md](benchmarks.md) publishes dated, single-node latency and throughput measurements with the method next to each number. Open: multi-node scaling, storage per fact, cache-hit ratio under realistic mixes, and head-to-head comparisons against spatial databases and geospatial data infrastructures on identical queries.
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

## A2A interoperability

Raised by consumer agents as "the gap is interoperability, not functions", with
a recommendation to build a bridge rather than replace the native layer. That
recommendation is accepted. What follows separates what already ships from what
does not, because a roadmap that restates shipped features as future work is how
a team ends up rebuilding something it has.

### Ships today

- **A spec-shaped A2A AgentCard** at `/.well-known/agent-card.json`:
  `protocolVersion 1.2.0`, `url`, `preferredTransport`, `defaultInputModes` /
  `defaultOutputModes`, `securitySchemes`, `additionalInterfaces`, and **102
  skills** each with an id, name, description and tags.
- **Declared capabilities**, including the honest negatives:
  `streaming: true`, `stateTransitionHistory: true`, `pushNotifications: false`.
- **Server-sent events** at `/v1/memory/sse`, so the channel is not
  polling-only.
- **A typed error taxonomy** (`emem.error.v1`): every error carries a stable
  `code`, a `message` naming the missing thing and the accepted alternatives,
  and a `details` block. An independent sweep of 70 tools found 20 of 20
  refusals self-repairing and zero hollow successes.
- **JSON-RPC 2.0** on `/mcp`.
- **Peer discovery** at `/v1/agents`, scanned from the store rather than
  configured.
- **Skill query** at `/v1/a2a/skills?q=&tag=&category=`, so a peer can ask
  "do you do X" without fetching all 102 skills.

### Roadmap, in the order we would build it

1. **Task lifecycle.** A2A tasks carry an id and move
   `submitted -> working -> input-required -> completed -> failed`. emem has
   async task tools (`emem_eudr_dds`, `emem_hunt`) and MCP `taskSupport`, but no
   A2A-shaped state machine and no `failed` state a peer can branch on. This is
   the largest real gap and the one that blocks long-running collaboration.
2. **Push notifications.** Webhook callbacks on task transition. Declared
   `false` today rather than implied.
3. **Multi-modal parts.** `TextPart` / `FilePart` / `DataPart`. emem is
   text-and-JSON; there is no file abstraction in the agent layer.
4. **Registry and capability federation.** `/v1/agents` lists peers on ONE
   responder. A directory spanning responders, with search by capability, is
   not built.
5. **Gossip between responders.** Requested alongside the above. Today every
   claim is verified against the responder that served it; there is no
   peer-to-peer propagation of heads or facts, which is also why the
   transparency log cannot yet resist a split view.

### The naming, which is a fair criticism

emem's `a2a` block advertises a collaboration convention (a ten-rule standard, a
curriculum, a pinned-key contacts registry) that is **not** the Agent2Agent
protocol, while the AgentCard beside it is spec-shaped. Two different things
under one abbreviation is a referential collision, which is a poor look for a
protocol whose stated purpose is preventing exactly that. The convention will be
named separately from the wire protocol.
