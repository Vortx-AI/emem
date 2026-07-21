# The memory model

The aim, stated plainly: not the best geospatial memory system, but the
first mathematically defined memory protocol for physical observations.
This page is the current state of that definition. Everything in the
first three sections is shipped and points at its mechanism; the last
section is the open formal work, kept honest and separate.

## The object

The unit of memory is one observation:

```
O = (a, b, t, v, u, p, s)
```

| Element | Meaning | Wire mechanism |
|---|---|---|
| `a` | address | cell64, a 64-bit canonical cell (~9.55 m at the equator); antimeridian and pole handling pinned in code |
| `b` | band | typed channel from the content-addressed bands manifest (`bands_cid`) |
| `t` | valid time | `tslot`, band-tempo-relative; paired with transaction time `signed_at` inside `p` |
| `v` | value | scalar or vector, with `unit`, canonical CBOR encoded |
| `u` | uncertainty | `confidence` plus optional `uncertainty` on every `PrimaryFact` (crates/emem-fact) |
| `p` | provenance | `sources[]` (scheme, id, content hash, capture time), `derivation` (registry `fn_key` plus args), the band's provenance class with in-band `caution`, the attester key, and `signed_at` |
| `s` | signature | ed25519 over blake3 of the canonical CBOR body |

Identity is content: `cid(O) = blake3(canonical_cbor(O))`, 52 base32
characters, so the same bytes have the same name on every conformant
responder. The memory itself is

```
M = (O*, E*)
```

an append-only set of observations plus typed temporal edges
`E = (subj, pred, obj, valid_from, valid_to)` with predicates
`disagrees_with`, `supersedes`, `relates_to`. A token
(`emem:fact:<a>:<cid>`, `emem:bundle:`, `emem:entity:`, `emem:cell:`) is
the name of an element or a set, resolvable to byte-identical bytes.

The fundamental key is `(a, b, t)`, not `a`: time is part of the object,
not metadata on it. Every read primitive accepts the bi-temporal pair
(`as_of_tslot` for what was true on the ground, `as_of_signed_at` for
what the memory knew), so `Recall(M, a, t)` is the shipped read, not a
roadmap item.

## Properties

Stated as claims with their mechanism. "Mechanism shipped" means the
property holds by construction and is checkable today; a machine-checked
proof is open work for all of them.

| Property | Statement | Mechanism |
|---|---|---|
| Immutability | no operation rewrites an observation; change is a new observation that `supersedes` on the transaction-time axis | content addressing; a changed byte is a different `cid`; supersede edges; bi-temporal reads replay the old state |
| Determinism | encoding is canonical, so equal values produce equal bytes produce equal names | canonical CBOR + blake3; golden-vector tests pin the Rust and JS encoders to each other |
| Reproducibility | resolving a name returns byte-identical bytes on any responder holding it; `direct_sensor` and `deterministic_index` observations are additionally recomputable from their cited raw source | content addressing; provenance classes; the `deterministic: true` recall filter |
| Verifiability | any answer re-verifies offline with only the responder pubkey | receipt preimage v1 (domain-separated, length-prefixed, RFC 6962 merkle rules); `/verify` in-browser; the Python samples in [agents](./agents.md) run against production and print VALID |
| Composability | signed sets and named objects are first-class | `emem:bundle:` (N observations, one signed envelope), `emem:entity:` (one object identity over many observations) |
| Honest absence | the absence of data is itself a signed, citable answer with a typed reason | Absence facts; never an empty 200 pretending to be knowledge |
| Attested trust class | how a value was produced is attested transitively, not self-declared per response | provenance class rides the content-addressed bands manifest whose `bands_cid` is folded into every receipt preimage |

What the signature does NOT claim: objective truth. It proves who
attested the observation and that the bytes never changed; `u` and the
provenance class carry the epistemic status.

## The algebra, as shipped today

The operations below exist on the wire. Naming them as an algebra is the
point of this section: memory is something applications compute over,
not just retrieve.

| Operation | Shipped surface |
|---|---|
| `recall(M, a, t)` | `POST /v1/recall` with `as_of_tslot` / `as_of_signed_at`; provenance-filtered variant via `deterministic` / `provenance` |
| `diff(M, a, b, t1, t2)` | `POST /v1/diff` (same band, two times), `/v1/state_diff` (embedding delta), `/v1/temporal_route` (staleness-ranked) |
| `merge(O1..On)` | `POST /v1/memory_bundle`: one signed envelope, one citable name for the set |
| `verify(O)` | `POST /v1/verify_receipt`, or fully offline from the receipt bytes |
| `trace(O)` | in the object itself: `derivation.fn_key` resolves in the algorithm registry to a published formula and citation; `sources[]` carry upstream ids, content hashes, and capture times; edges extend the chain across observations |
| `competing(M, a)` | `POST /v1/memory_contradictions`: disagreement scored per band kind (normalised spread, 1 - cosine, mode share), kept as evidence, never averaged away |
| `explain(O)` | the receipt plus the registry formula are the signed explanation; the natural-language layer (`/v1/ask`) is an explicitly unsigned sidecar over signed reads |
| `evolve(M)` | attest; a later observation `supersedes` without erasing, and the refinement loop records `disagrees_with` edges from contradictions |
| `resolve(name)` | token dereference: `emem:fact:` / `emem:bundle:` / `emem:entity:` back to byte-identical bytes |
| `ensure(M, a, B)` | shipped at single-hop: `POST /v1/recall` with a band list states the goal; the responder reuses what exists (`was_cached`), fetches and signs what does not (`materialize_notes`), and the agent never orchestrates the fetch. Multi-hop targets are open work (see the compiler view) |
| `valid(M, a)` | `POST /v1/temporal_route`: per-band staleness Q(dt) from the band's physics decay kernel, split into `cite_now` (fresh enough to cite) and `fetch_for_intent` (needs re-materialisation): validity intervals and invalidation, on the wire |

Deliberately absent: a silent `forget()`. Removal without trace would
break every property above. What exists instead is supersession on the
transaction-time axis and privacy classes at write time.

Partial today: `counterfactual(M, before)` in the transaction-time
direction is exactly `as_of_signed_at`: the memory as it stood before an
observation arrived. Counterfactual REMOVAL (the memory with one
observation excised and downstream effects recomputed) is open work, and
harder than it looks because derivations would need re-execution.

## The compiler view

Read the model above from a different angle and a second system appears:
a content-addressed build graph over the physical world, one graph per
cell. The leaf level is shipped and serving traffic today:

- **Rule**: every derived observation names its rule, `derivation.fn_key`
  (`sentinel2_l2a_indices_ndvi@1`), a versioned entry in the
  content-addressed algorithm registry whose `algorithms_cid` is pinned
  into receipts, together with the full argument list.
- **Inputs**: `sources[]` carry the upstream ids, content hashes, and
  capture times.
- **Output**: the `fact_cid`, blake3 of the canonical bytes.
- **Demand-driven**: recall on a miss materialises, signs, and persists,
  which is a lazy build; the receipt's `was_cached` is cache semantics
  on the wire.
- **Hermeticity labels**: the provenance classes are exactly this.
  `deterministic_index` is a hermetic action (rebuild the value from the
  cited sources, no hidden input); `model_output` is a pinned
  non-hermetic action (reproducible given the signed checkpoint);
  `direct_sensor` and `human_curated` are sources, not actions.

Hermetic does not mean bit-identical for every action, and the split is
accumulation. An op with no accumulation rebuilds bit for bit: a
difference, a ratio, a tree of 2-ary selections. A reduction over many
inputs does not, because the summation order is an implementation detail
nobody signed. Measured over real signed NDVI parents, a `sum` recomputed
against a caller's own lands 0 to 2 ULP away depending on how many
parents there are, and not monotonically in the count, since accumulation
error cancels as readily as it compounds. So the recompute path compares
`mean` and `sum` over more than two parents inside a published 4-ULP
window and everything else exactly, and it returns the rule it used, the
tolerance, and the measured gap on every answer, so a caller who needs
bit-identity can require a gap of 0.

Determinism buys the property a build system wants most: derived
artifacts are evictable without breaking citations, because
re-materialisation by the same implementation yields the same bytes,
hence the same cid, hence every token keeps resolving. Nothing permanent
is required beyond immutable observations and the rule registry.

What is missing before "build graph" is earned rather than aspirational,
in dependency order: multi-hop composition as signed objects (the worlds
bake already derives splats from facts with re-derivable bilinear
weights, but the splat layer lives in a sidecar, not in the fact model);
the one-traversal `lineage(O)` DAG from the open-work list below; a
target language (ask for a mesh at a cell and the graph compiles
gaussians, mesh, and navigation surfaces on demand); and portable,
sandboxed action definitions, since today the registry formulas document
Rust implementations rather than define hermetic rules a second
implementation could execute.

### From build graph to execution substrate

The deeper consequence, and the direction this is being built toward:
the agent no longer owns the state, the world owns the state. An agent's
progress is not "I completed step 17" in a session transcript; it is the
state of the cells it touched: which observations exist, at which
versions, derived from what, still valid until when. A different agent,
months later, under a different model, continues from the world.

The state the session inherits still lacks one operator: the derivative.
Two states of one cell say what changed; nothing yet says why it moved,
how much of the delta is the world and how much is the instrument, the
registration, or the encoder. That operator is specified as change
attribution on the roadmap, and it slots into this graph as one more
derived, signed object over parent facts.

The mechanisms for that are the ones above, read as an execution
substrate. What already exists: every recall returns
`bands_already_attested_at_cell`, so "what has been computed here" is
one call. What is still valid: `temporal_route`'s per-band staleness
scores split `cite_now` from `fetch_for_intent`. What replaced what:
supersede edges. Why believe any of it: the receipt. And the goal verb
is shipped at single-hop: a recall with a band list is `ensure`, not
`get`; the agent states what must exist and the responder decides reuse
versus fetch. What is missing is stated in the gap list above:
multi-hop targets and a cost-based planner ("cheapest valid path to a
navigation surface at this cell"), which turn ensure from a band-level
verb into a graph-level one.

Coordination follows the same shape. A thousand agents working the same
region do not synchronise through messages; they coordinate through
shared, immutable, content-addressed artifacts and the dependency rules
between them, with contradiction scoring as the conflict rule when two
writers disagree. The objective underneath all of it: make repeated
reasoning unnecessary. If an artifact is deterministic, reproducible,
and already exists, no future agent should spend tokens re-deriving it
or arguing itself into trusting it; it resolves by name and re-checks by
hash.

One scope note, taken from review: "memory for long-horizon agents" is
one application of this substrate, not its definition. The architecture
is meant to stay valuable even if today's agents are replaced by a
different class of systems; durable, verifiable, reusable state about
the physical world is the invariant. The same invariant is what admits
the next substrates (fixed sensors, drones, robot fleets, machines,
government registries), each a written profile over the same attest,
recall, cite, verify path, with location as the first key.

### The atom, answered

Review asked the right whiteboard question: what is the smallest
immutable object, git's blob, IPFS's block? If the answer were "band",
emem would have stopped too early. It is not. The atom is the
observation `O` defined at the top of this page; the band is its
semantic TYPE field, not the object. The proposed "Earth Value"
(location, time, semantic meaning, algorithm, dependencies, cid,
confidence, validity, signature) maps element for element onto what
already ships:

| Proposed | Shipped element of `O` |
|---|---|
| location | `a` (cell64) |
| time | `t` plus `signed_at` (bi-temporal) |
| semantic meaning | `b` (band, from the content-addressed manifest) |
| algorithm | `p.derivation.fn_key@version` |
| dependencies | `p.sources[]` (ids, content hashes, capture times) |
| cid | `cid(O)` |
| confidence | `u` |
| validity | tempo class + Q(dt) staleness (`valid(M, a)`) |
| signature | `s` |

The git analogy holds the same way: blob, tree, commit correspond to
observation, edge, rule-plus-receipt. Worlds, meshes, and projects are
meant to emerge from those, not to become new stored types.

Where the review is right that we have stopped early: today `v` is
band-typed. A gaussian patch, a mesh, or a road graph is not yet an
observation; the worlds bake keeps its derived splats in a provenance
sidecar. Generalising `v` to artifact-typed values with canonical
encodings, so a mesh is signed, cited, and evicted exactly like an NDVI
reading, is the real content of the "multi-hop derivation as signed
objects" gap above, and it strengthens the existing fact model rather
than adding a second object system beside it.

On learned graphs versus written ones: build systems assume someone
wrote the graph, and for Earth nobody knows the graph. The position
here is deliberate: the RULES stay declared, versioned, and signed,
because a discovered rule cannot be re-verified, but WHICH declared
derivations exist, apply, and are worth running at a given cell for a
given goal is discovered, not hardcoded. The seed of that planner
already ships: `/v1/ask` routes a question to the applicable recipes in
`algorithms_for_question`, and one goal already has competing
derivations to choose between (deforestation via the NDVI-drop alert,
via SAR disturbance, or via the triple ensemble). Learn the graph;
never learn the rule.

The verb this converges on is `prove(goal)`, not `get` and not
`build(target)`: the answer to a goal is an artifact PLUS a
machine-checkable chain that every dependency is valid for this
location at this time. At single-hop that is shipped today, receipt
plus provenance class plus staleness on every ensure; making it hold
across a discovered multi-hop chain is the planner work in the gap
list, with its trust question answered per hop ("can I trust this
gaussian", not "does it exist").

Against the neighbours: Earth Engine is a computation graph inside one
org's trust domain with unsigned results; Bazel and Nix are
content-addressed builds inside one org's trust domain. The receipts are
what make this graph verifiable ACROSS trust domains: any robot can
re-check any org's compiled artifact offline. The memory is the product;
the build graph is the substrate thesis.

## Open formal work

The honest gap list, in priority order:

1. Machine-checked statements of the property table (immutability,
   determinism, reproducibility) over the canonical encodings, rather
   than by-construction arguments in prose.
2. Uncertainty propagation: `u` exists on every observation but most
   materializers write point confidence today; deriving the `u` of a
   computed observation from the `u` of its inputs through
   `derivation.fn_key` is defined nowhere yet.
3. Semantic compression: whether N observations of one place compress
   into a canonical memory without losing citability. This is not
   entropy coding; every compressed claim must still resolve to signed
   sources. Related shipped primitive: bundles name sets, but do not
   summarise them.
4. Full provenance graphs as one queryable object: today the chain
   lives across `sources[]`, `derivation`, and edges; a single
   `lineage(O)` traversal that renders acquisition to answer as one
   graph is not yet a surface.
5. A written conformance profile, so a second implementation can claim
   compatibility by satisfying the algebra and property table rather
   than by matching this codebase.
6. Artifact-typed observations: generalise `v` beyond band-typed values
   so gaussians, meshes, and route graphs are first-class signed
   observations with canonical encodings, extending the existing fact
   model instead of adding a parallel object system. Prerequisite for
   multi-hop signed derivation.
7. Derivation discovery and the `prove(goal)` contract: learn, per cell
   and goal, which declared rules apply and which chain is cheapest,
   while every edge stays a declared, signed rule; the planner's output
   is the artifact plus the validity chain. Seed shipped:
   `algorithms_for_question` routing and competing derivations for one
   goal.

The research roadmap in the [README](https://github.com/Vortx-AI/emem#open-research)
tracks these; the sequencing argument is in the whitepaper.
