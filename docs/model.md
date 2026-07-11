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

Deliberately absent: a silent `forget()`. Removal without trace would
break every property above. What exists instead is supersession on the
transaction-time axis and privacy classes at write time.

Partial today: `counterfactual(M, before)` in the transaction-time
direction is exactly `as_of_signed_at`: the memory as it stood before an
observation arrived. Counterfactual REMOVAL (the memory with one
observation excised and downstream effects recomputed) is open work, and
harder than it looks because derivations would need re-execution.

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

The research roadmap in the [README](https://github.com/Vortx-AI/emem#open-research)
tracks these; the sequencing argument is in the whitepaper.
