# The memory model

This page defines what emem stores and what you can do with it. A plain
summary comes first. The formal model follows it. The last section lists
the formal work that is still open.

The aim is a memory protocol for physical observations with a
mathematical definition. This page is the current state of that
definition. Each claim in the formal model points to the code or the
endpoint that does it. Open work stays in its own section.

## In short

- emem stores observations. One observation is one value for one place,
  one band and one time, signed by the responder.
- The id of an observation is the hash of its bytes. The same bytes have
  the same id on every responder.
- You never change an observation. A correction is a new observation
  with a `supersedes` edge to the old one. The old bytes stay readable.
- Time is part of the key. `as_of_tslot` asks what was true on the
  ground on a date. `as_of_signed_at` asks what emem knew on a date.
- A token is a short name that another agent resolves to the same
  record. `emem:fact:` names exact bytes. `emem:entity:` names an
  identity only.
- When two sources disagree at one place, emem keeps both and scores the
  disagreement. It does not average them.
- When emem has no data, it signs an absence with a typed reason. It
  never returns an empty answer as if it were knowledge.
- Every fact read carries a receipt. You check it offline with the
  public key of the responder. Some answers are not fact reads and have
  no receipt, for example a corpus audit from `/v1/ask` or a memory
  search hit.
- A signature proves who signed the bytes and that the bytes did not
  change. It does not prove that the value is true.

Notes are a separate plane. A note is a Markdown file that an agent
writes and signs in its own namespace. [Memory](./memory.md) describes
notes, namespaces and signed writes.

The common operations and their endpoints:

| You want to | Endpoint |
|---|---|
| read the facts at a place, at a date | `POST /v1/recall` with `as_of_tslot` or `as_of_signed_at` |
| compare one band at two times | `POST /v1/diff` |
| name a set of facts with one token | `POST /v1/memory_bundle` |
| find where sources disagree | `POST /v1/memory_contradictions` |
| check a receipt | `POST /v1/verify_receipt`, or offline |
| score how current each band is at a place, at a query time | `POST /v1/temporal_route` |

These parts of the model do not ship today:

- a `forget` operation, which is absent on purpose (see
  [Absent on purpose](#absent-on-purpose)),
- a counterfactual that removes one observation and computes its effects
  again,
- the numeric split of a change between the world and the instrument,
- derivations with more than one step as signed objects,
- machine-checked proofs of the properties.

## The formal model

### The observation

The unit of memory is one observation:

```
O = (a, b, t, v, u, p, s)
```

| Element | Meaning | Wire mechanism |
|---|---|---|
| `a` | address | cell64, a 64-bit canonical cell (about 9.55 m at the equator); the code fixes antimeridian and pole handling |
| `b` | band | typed channel from the content-addressed bands manifest (`bands_cid`) |
| `t` | valid time | `tslot`, relative to the tempo of the band; paired with the transaction time `signed_at` inside `p` |
| `v` | value | scalar or vector, with `unit`, encoded as canonical CBOR |
| `u` | uncertainty | `confidence` plus an optional `uncertainty` on every `PrimaryFact` (crates/emem-fact) |
| `p` | provenance | `sources[]` (scheme, id, content hash, capture time), `derivation` (registry `fn_key` plus args), the provenance class of the band with its in-band `caution`, the attester key, and `signed_at` |
| `s` | signature | Ed25519 over BLAKE3 of the canonical CBOR body |

### Identity and the memory

Identity is content: `cid(O) = blake3(canonical_cbor(O))`, written as 52
base32 characters. Thus the same bytes have the same name on every
conformant responder. The memory itself is

```
M = (O*, E*)
```

This is an append-only set of observations plus typed temporal edges
`E = (subj, pred, obj, valid_from, valid_to)`. The predicate is a free
label. The labels in use include `disagrees_with`, `supersedes`,
`replaced_by` and `co_located_with`.

### Tokens

A token is the name of an element or of a set. The token families do not
have equal strength:

| Token | Names | Strength |
|---|---|---|
| `emem:fact:<a>:<cid>` | one observation | the full 32-byte hash of the signed body, so it resolves to byte-identical bytes |
| `emem:bundle:<cid>` | a signed set of observations | 16 bytes of a hash over the purpose and the cell, band, tslot and fact cid of each citation. The member fact cids carry the full strength |
| `emem:entity:<cid>` | one object identity | 16 bytes of a hash over the identity anchor, not over the whole record; a shared reference, not shared bytes |
| `emem:cell:<a>` | a place | an address; it names no bytes |
| `emem:tree:<file_cid>#row=<i>` | one row of a tokenised file | an audit path checks it against the root in the note that the author signed |

The identity anchor of an entity is a stable external id, such as
Overture GERS, OSM or Wikidata. When there is none, the anchor is the
cell, the kind and the label.

The geometry of an entity is outside its cid and its mint signature.
Some geometry falls outside the cell of the entity. Where emem
serves such geometry, it withholds it (`bbox_withheld`,
`geojson_withheld`, `point_withheld`).

### Time

The key is `(a, b, t)`, not `a`. Time is part of the object, not
metadata on it. These read primitives accept the bi-temporal pair:
recall, query_region, trajectory, find_similar and memory_bundle.

- `as_of_tslot` gives what was true on the ground at that time.
- `as_of_signed_at` gives what the memory knew at that time.

Thus the read that ships today is `Recall(M, a, t)`: one place, and a
bound on one or both time axes. `diff` and `compare` take their own two
times and do not accept the pair. `memory_contradictions` does not
accept it either.

### Properties

Each property is a claim with the mechanism that makes it hold. The
mechanism ships, so you can check each property today. A proof that a
machine checks is open work for all of them.

| Property | Statement | Mechanism |
|---|---|---|
| Immutability | no operation rewrites an observation; a change is a new observation that `supersedes` the old one on the transaction-time axis | content addressing (a changed byte gives a different `cid`); supersede edges; bi-temporal reads replay the old state |
| Determinism | the encoding is canonical: equal values give equal bytes, and equal bytes give equal names | canonical CBOR and BLAKE3; a CI test checks the browser verifier against vectors from the Rust signer (`scripts/verify_core_test.cjs`) |
| Reproducibility | a name resolves to byte-identical bytes on any responder that holds it. You can also compute `direct_sensor` and `deterministic_index` observations again from their cited raw source | content addressing; provenance classes; the `deterministic: true` recall filter |
| Verifiability | any signed answer verifies offline with only the public key of the responder | the receipt preimage (see below). An RFC 6962 transparency log (`/v1/log/*`) that outside witnesses co-sign. `/verify` in the browser. The Python samples in [agents](./agents.md) run against production and print VALID |
| Composability | signed sets and named objects are first-class | `emem:bundle:` (N observations, one signed envelope); `emem:entity:` (one object identity over many observations) |
| Honest absence | an absence of data is itself a signed answer with a typed reason, and you can cite it | absence facts; never an empty 200 that looks like knowledge |
| Attested trust class | an attestation says how emem made a value. The class comes from the signed bands manifest, not from a claim in one response | the provenance class is in the content-addressed bands manifest, and every receipt preimage includes its `bands_cid` |

The receipt preimage is domain-separated, tagged and length-prefixed.
The current form is `preimage_version: 2`, in use since 2026-08-05. It
also binds the Merkle inclusion proof of the fact.

The signature does not claim that the value is true. It proves who attested
the observation and that the bytes did not change. The uncertainty `u`
and the provenance class carry the epistemic status.

### Operations

Each operation below exists on the wire. Together they are an algebra:
applications compute over the memory, and they also read from it.

| Operation | Endpoint or mechanism that ships |
|---|---|
| `recall(M, a, t)` | `POST /v1/recall` with `as_of_tslot` or `as_of_signed_at`; `deterministic` or `provenance` filters the result by provenance |
| `diff(M, a, b, t1, t2)` | `POST /v1/diff` (one band, two times); `/v1/state_diff` (embedding delta); `/v1/temporal_route` (ranked by staleness) |
| `merge(O1..On)` | `POST /v1/memory_bundle`: one signed envelope, one name to cite for the set |
| `verify(O)` | `POST /v1/verify_receipt`, or offline from the receipt bytes |
| `trace(O)` | in the object: `derivation.fn_key` resolves in the algorithm registry to a published formula and citation. `sources[]` carry upstream ids, content hashes and capture times. Edges continue the chain across observations |
| `competing(M, a)` | `POST /v1/memory_contradictions`: a disagreement score for each band kind (normalised spread, 1 - cosine, mode share); emem keeps the disagreement as evidence and never averages it away |
| `explain(O)` | the receipt and the registry formula are the signed explanation. `/v1/ask` returns prose over signed reads. Its receipt signs the fact cids that it read, not the prose |
| `evolve(M)` | attest; a later observation `supersedes` an older one and does not erase it; the refinement loop records `disagrees_with` edges from contradictions |
| `resolve(name)` | token dereference: `emem:fact:` gives byte-identical bytes, `emem:bundle:` gives its signed envelope, `emem:entity:` gives the object record |
| `ensure(M, a, B)` | ships for one step. A `POST /v1/recall` with a band list states the goal. The responder uses what exists and fetches and signs the rest. `materialize_notes` gives the status of each band it tried (`materialized`, `skipped` or `deferred`). The agent does not control the fetch |
| `valid(M, a)` | `POST /v1/temporal_route`: staleness Q(dt) for each band, from the physics decay kernel of the band. It splits bands into `cite_now` (fresh enough to cite) and `fetch_for_intent` (needs a new read) |
| `attribute(M, a, t1, t2)` | `POST /v1/change_attribution` (`change_attribution@1`): the evidence for why the readout of one cell moved between two visits, term by term. The request has only `cell`. The responder uses the two latest visits of each band, so the caller does not choose `t1` and `t2`. Each item has its fact cids, under a signed receipt. emem does not compute the numeric split of the delta among the terms, so `split` is null and says why |

`ensure` for targets that need more than one step is open work. See
[The memory as a build graph](#the-memory-as-a-build-graph).

### Absent on purpose

There is no silent `forget()`. A removal without a trace would break
every property above. These mechanisms exist instead:

- supersession on the transaction-time axis,
- privacy classes at write time,
- for notes, a delete by the author. It leaves a tombstone, and the
  bytes stay addressable by cid.

A counterfactual is partly there. `counterfactual(M, before)` on the
transaction-time axis is `as_of_signed_at`: the memory before an
observation arrived. A counterfactual REMOVAL is open work. That is the
memory with one observation cut out and its downstream effects computed
again. It is hard because each derivation would have to run again.

## The memory as a build graph

The same model is also a content-addressed build graph over the physical
world, one graph for each cell. The lowest level ships and serves
traffic today:

- Rule. Every derived observation names its rule, `derivation.fn_key`
  (for example `sentinel2_l2a_indices_ndvi@1`), with the full list of
  arguments. The rule is a versioned entry in the content-addressed
  algorithm registry. Receipts pin the `algorithms_cid` of that
  registry.
- Inputs. `sources[]` carry the upstream ids, content hashes and capture
  times.
- Output. The `fact_cid`, which is the BLAKE3 hash of the canonical
  bytes.
- Build on demand. A recall on a miss reads, signs and stores the value.
  This is a lazy build. `receipt.cost.was_cached` is true only when the
  cache served the whole read. It is outside the signed preimage.
- Hermeticity labels. The provenance classes are these labels.

| Provenance class | Build role |
|---|---|
| `deterministic_index` | a hermetic action: you can build the value again from the cited sources, with no hidden input |
| `model_output` | a pinned action that is not hermetic: you can reproduce it with the signed checkpoint |
| `direct_sensor` | a source, not an action |
| `human_curated` | a source, not an action |

### When a rebuild gives the same bits

Hermetic does not always mean bit-identical. The difference is
accumulation:

- An operation with no accumulation gives the same bits again. Examples
  are a difference (`delta`) and a classification, which is a tree of
  2-ary selections.
- A reduction over many inputs does not. The order of the sum is a
  detail that nobody signed.

We measured this over real signed NDVI parents. A `sum` that a caller
computes again lands 0 to 2 ULP from ours. The gap depends on the
number of parents, but it does not grow steadily with it. Errors of
rounding cancel as easily as they add up.

Thus the recompute path compares `mean` and `sum` over more than two
parents inside a published window of 4 ULP. It compares all other
operations exactly. Each answer returns the rule it used
(`rule`), the tolerance (`ulp_tolerance`) and the measured gap
(`ulp_gap`). A caller who needs the same bits can require a gap of 0.

Determinism gives a build system its most useful property. You can
delete a derived artifact and the citations still work. The same code
builds the same bytes again, so the cid is the same and every token
still resolves. Only the immutable observations and the rule registry
must be permanent.

### What a full build graph still needs

These items are missing, in order of dependency:

1. Composition over more than one step, as signed objects. The worlds
   bake already derives splats from facts with bilinear weights that you
   can derive again. But the splat layer is in a sidecar, not in the
   fact model.
2. The one-traversal `lineage(O)` graph from
   [Open formal work](#open-formal-work).
3. A target language. For example, you ask for a mesh at a cell. The
   graph then builds the gaussians, the mesh and the navigation surfaces
   on demand.
4. Portable action definitions that run in a sandbox. Today the registry
   formulas describe Rust code. They are not hermetic rules that a
   second codebase could run.

### State that outlives the agent

This is the direction of the work. The world owns the state, not the
agent. The progress of an agent is not "I completed step 17" in a
session transcript. It is the state of the cells that the agent touched:

- which observations exist, at which versions,
- what they come from,
- until when they stay valid.

Months later, a different agent with a different model continues from
that state.

The state that a session inherits still lacks part of one operator, the
derivative. Two states of one cell show what changed.
`change_attribution@1` ships the evidence for why it changed:

- the observed change (the Tessera embedding change and the NDVI, NBR
  and NDWI deltas),
- the sources that each visit used, and whether they match,
- the encoder pinning,
- the scene class (`s2.scl`) of each visit.

Each item has its fact cids. The ledger says that no registration error
estimate exists in this build. emem stores the ledger as a derived fact
under the responder key, with its own `emem:fact:` token. A recall does
not return it; you reach it through its token. Each call to
`POST /v1/change_attribution` stores one such fact.

The number does not ship: how much of the change comes from the world,
and how much from the instrument, the registration or the encoder. The
response field `split` is null. That split is on the
[roadmap](./roadmap.md#change-attribution-why-did-this-places-readout-move).
It will be one more derived, signed object over parent facts in this
graph.

The mechanisms above already give most of an execution substrate:

| Question | Mechanism that ships |
|---|---|
| What exists at this cell? | every recall returns `bands_already_attested_at_cell` |
| What is still valid? | the staleness scores of `temporal_route` split `cite_now` from `fetch_for_intent` |
| What replaced what? | supersede edges |
| Why believe it? | the receipt |
| How do I state a goal? | a recall with a band list is `ensure`: you state what must exist, and the responder decides to use a stored value or to fetch |

Two parts are missing. One is targets with more than one step. The other
is a planner that compares cost, for example "the least costly valid
path to a navigation surface at this cell". Together they make `ensure`
work on a graph, not only on one band.

Coordination has the same shape. The design aim is that many agents in
one region do not synchronise through messages. They share immutable,
content-addressed artifacts and the dependency rules between them. When
two writers disagree, the contradiction score is the conflict rule. The
aim is that no agent reasons the same thing twice. If an artifact is
deterministic and already exists, a later agent resolves it by name and
checks it by hash. It does not spend tokens to derive it again.

Memory for long-horizon agents is one application of this substrate. It
is not the definition of the substrate. If a different class of system
replaces the agents of today, durable and verifiable state about the
physical world keeps its value. Other sources use the same path (attest,
recall, cite, verify), with location as the first key. The registry
`crates/emem-core/data/substrates-v0.json` lists 18 profiles. Only
`earth.satellite.v0` is active. The other 17 have the status
`candidate`, for example `fixed.sensor.v1`, `aerial.drone.v1`,
`robot.fleet.v1` and `industrial.machine.v1`. A candidate profile
gives device makers a fixed target to build against. It does not open
ingest.

### The smallest object

A review asked: what is the smallest immutable object, as the blob is in
git and the block is in IPFS? The answer is not the band. The atom is
the observation `O` at the top of the formal model. The band is its
semantic TYPE field.

The review proposed an "Earth Value" with nine parts. Each part maps to
an element of `O` that ships:

| Proposed | Element of `O` that ships |
|---|---|
| location | `a` (cell64) |
| time | `t` plus `signed_at` (bi-temporal) |
| semantic meaning | `b` (band, from the content-addressed manifest) |
| algorithm | `p.derivation.fn_key@version` |
| dependencies | `p.sources[]` (ids, content hashes, capture times) |
| cid | `cid(O)` |
| confidence | `u` |
| validity | tempo class plus Q(dt) staleness (`valid(M, a)`) |
| signature | `s` |

The git analogy holds in the same way. Blob, tree and commit map to
observation, edge, and rule with receipt. Worlds, meshes and projects
are to come from these. They are not to become new stored types.

The review is correct on one point: today `v` is band-typed. emem signs
some whole fields as one object: a raster bundle, and the `emem:cube:`
band cube over time. A gaussian splat, a mesh or a road graph is not an
observation yet.

The worlds bake shows the gap. `scripts/bake_worlds.sh` runs
`examples/3d-worlds/make_splats.py`, which writes the splat files and a
sidecar file, `world.provenance.json`. The sidecar lists the signed facts
and receipts behind each splat, and the SHA-256 of each splat file. The
sidecar has no signature of its own.

To close the gap "derivation over more than one step as signed objects",
`v` must hold typed artifacts with canonical encodings. Then emem signs a
mesh, and agents cite and evict it in the same way as an NDVI reading.
This makes the fact model stronger. It does not add a second object
system next to it.

### Learned graphs and declared rules

A build system expects a person to write the graph. For the Earth, nobody
knows the graph. The position here is deliberate:

- The RULES stay declared, versioned and signed, because nobody can
  check a discovered rule again.
- A planner, not fixed code, is to pick WHICH declared derivations exist,
  apply and are worth a run at a cell for a goal.

Only the seed of that planner ships. `/v1/ask` sends a question to the
recipes that apply, in the `algorithms_for_question` list of its answer.
One goal already has competing derivations in the algorithms registry.
For deforestation, `deforestation_alert_ndvi_drop@1` reads an NDVI drop,
`sar_forest_disturbance@1` reads SAR, and `/v1/eudr_dds` reads the
Hansen and JRC TMF loss layers. The rule is: learn the graph, never learn
the rule.

The verb of this direction is `prove(goal)`. The answer to a goal is an
artifact plus a chain that a machine can check. The chain shows that
each dependency is valid for this location at this time.

For one step, this ships today. A `POST /v1/recall` with a band list
(`ensure`) returns a receipt. Add `"include": ["freshness", "provenance"]`
and each fact also carries its `provenance_class` and a `freshness` block
with the Q(dt) score. Both are outside the signed preimage. On 7 November
2026, a recall of `cams.pm25` at the Mount Fuji cell `defi.zb592.nUkO.zEzE`
gave class `model_output`, `q` 0.0 and `stale: true`.

Over a discovered chain of several steps, `prove` is planner work, in the
gap list. At each step it must ask a trust question ("can I trust this
gaussian"), not only "does it exist".

### Comparison with other systems

| System | Graph | Trust |
|---|---|---|
| Earth Engine | a computation graph | inside the trust domain of one organisation; it does not sign results |
| Bazel, Nix | content-addressed builds | inside the trust domain of one organisation |
| emem | content-addressed facts and rules | receipts let any robot check the artifact of any organisation offline |

The receipts make this graph verifiable across trust domains. The memory
is the product. The build graph is the thesis about the substrate.

## Open formal work

The gap list, in order of priority:

1. Machine-checked statements of the property table (immutability,
   determinism, reproducibility) over the canonical encodings. Today
   they are arguments in prose that follow from the construction.
2. Uncertainty propagation. `u` exists on every observation, but most
   materialisers write a point confidence today. No rule yet derives the
   `u` of a computed observation from the `u` of its inputs through
   `derivation.fn_key`.
3. Semantic compression. Can N observations of one place compress into
   a canonical memory and stay citable? This is not entropy coding.
   Every compressed claim must still resolve to signed sources. Two
   related primitives ship. Bundles name sets but do not summarise them.
   `emem:tree` checks one row of a tokenised file against a signed root
   and does not fetch the other rows.
4. Full provenance graphs as one object that you can query. Today the
   chain lives across `sources[]`, `derivation` and edges. No
   surface gives one `lineage(O)` traversal from acquisition to answer
   as one graph.
5. A written conformance profile. A second codebase could then claim
   compatibility when it satisfies the algebra and the property table.
   It would not need to match this codebase.
6. Artifact-typed observations. Let `v` hold more than band-typed
   values. Then gaussians, meshes and route graphs become first-class
   signed observations with canonical encodings. This extends the fact
   model. It does not add a parallel object system. It must come before
   signed derivation over more than one step.
7. Derivation discovery and the `prove(goal)` contract. For each cell
   and goal, learn which declared rules apply and which chain costs
   least. Every edge stays a declared, signed rule. The output of the
   planner is the artifact plus the validity chain. The seed ships:
   `algorithms_for_question` routing, and competing derivations for one
   goal.

[Limits and roadmap](./roadmap.md#open-research) tracks these items. The
[whitepaper](./whitepaper-v3.md) gives the argument for their order.
