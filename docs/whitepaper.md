# emem: an external identity layer for verifiable agent memory

**Canonical whitepaper / 2026-07-16**

*This is the one canonical paper. It carries whitepaper v2's framing and
honesty discipline as the spine and folds in whitepaper v1's protocol
depth; both parents remain in the repository, superseded but unedited.
[v1](/whitepaper/v1) (version 0.1.0, 2026-06-14) is the version cited by
[DOI 10.5281/zenodo.20706893](https://doi.org/10.5281/zenodo.20706893);
that record resolves to Zenodo's own frozen copy and is unaffected by
this document. [Section 18](#18-changes-from-v1) lists, claim by claim,
what v1 asserted that this responder does not do.*

---

## Abstract

emem is an external identity layer for AI agents: a shared, verifiable
memory whose purpose is to stop referential drift. A model that carries
its own prose description of a thing carries a description that decays,
and two models carrying two descriptions of one thing cannot tell whether
they are discussing the same thing. emem moves identity out of the model.
Every place resolves to one canonical address (`cell64`, §4). Every
observation there becomes one immutable record whose identifier is the
blake3 hash of its own canonical CBOR bytes (`fact_cid`, §5). A caller
hands another party a token; the token dereferences to those exact bytes,
and an ed25519 receipt over a domain-separated preimage lets the receiver
check them without trusting the sender, the network, or this responder
(§6).

That guarantee is precise, and its edges matter more than its centre.
It holds for `emem:fact:` tokens, where the identifier is a digest of the
stored bytes and the receipt binds `(cell, fact_cid)` into the signed
preimage. It does **not** hold uniformly across the token family:
`emem:entity:` converges two agents on a shared *name*, not on
byte-identical *bytes*, because an entity's identifier is computed from a
deliberately narrow identity preimage rather than from its whole body
(§2). §3 gives the grammar and states, per token type, exactly what
dereferencing proves. §16 collects every such limit in one place. Where a
mechanism proves less than its name suggests, this document says so
rather than let the name do the work.

The protocol is one trust surface over two layers. *Earth memory* is an
open-data corpus addressed by `(cell, band, tslot)` whose unit is a
signed fact. *Agent memory* is a content-addressed file store under
`/memories/*` with six file verbs conforming to the Anthropic memory-tool
specification, four memory kinds drawn from the CoALA agent-memory
ontology, and an ed25519 capability binding on paths under
`/memories/by_attester/<pubkey_short>/`. Both return signed receipts, and
one `/verify` page checks either without trusting the issuer.

Every signature this responder issues follows one rule: ed25519 over
blake3 of a domain-separated, tagged, length-prefixed segment stream
(§6). Two constructions sit outside it, and §6.4 names both. Every read
primitive accepts two optional bounds, `as_of_tslot` (observation time)
and `as_of_signed_at` (transaction time); both enter the signed preimage,
so a verifier in year *t+k* replays the exact query a system answered in
year *t*. Every band declares how its values are produced, in one of five
tamper-provenance classes (§7). An append-only log records attestation
batches under the RFC 6962 tree construction (§8). Callers may register
derivations over parent facts under their own key (§9).

The framing this document commits to follows from the guarantee rather
than preceding it. Generating a plausible account of the world is cheap
and getting cheaper; what stays scarce is a shared account that is
measured, signed, and checkable by someone who was not there. emem is
built to be that account: a substrate agents inherit rather than
re-derive, measured at canonical addresses, signed fact by fact,
bi-temporal, and checkable offline.

---

## 1. Referential drift

An agent that verifies something and then describes it has already begun
losing it. The description is a lossy re-encoding, and every subsequent
hop re-encodes again: a summary, a compaction pass, a handoff to another
model, a paraphrase in a report. Nothing in the pipeline marks the moment
the meaning changed, because at every step the text still reads fluently.
This is referential drift: the words survive, the referent does not.

Three failure modes follow, and all three are routine rather than exotic.

**Within one agent, across time.** A long task outlives its own context
window. The harness compacts, the session ends, the model is swapped
mid-project. A verified measurement becomes "roughly 3,700 m", then "high
elevation", then an assumption nobody can trace. The agent cannot tell
which of its beliefs are still grounded, because a paraphrase and a
measurement have the same shape in a transcript.

**Between two agents.** Agent A reports "the flood line north of the
river". Agent B reads it and reasons about a different line, at a
different time, from a different source, and neither can detect the
mismatch: both prose descriptions are internally coherent. Absent a
shared referent, the only way B can check A is to redo A's work, which
defeats the point of the handoff.

**Between an agent and an auditor, later.** "What did we know when we
committed the loan?" has no answer if the memory is prose. The forecast
that justified the decision has been superseded by a better forecast, and
the record of what was believed at decision time is gone.

The common cause is that identity lives inside the model. Each model
holds its own description of a thing, and descriptions do not compose:
there is no operation that takes two paraphrases and decides whether they
denote one object. The problem is not that models are careless. It is
that a natural-language description is not an identifier, and using one as
an identifier is a category error that only surfaces later, when the cost
of the mistake has compounded.

The fix is not better prose discipline. It is to give the thing an
address that lives outside every model, that derives from bytes rather
than from who is speaking, and that any party can dereference and check
alone. That is what the rest of this document specifies: an address for
places (§4), a content identifier for observations (§5), a citeable
handle for both (§3), and a signature over the bytes that makes the
handle checkable by someone who trusts nobody (§6).

The claim is narrow, and worth stating precisely because the surrounding
literature is full of larger ones. emem does not stop a model from
paraphrasing, and does not make a recorded value true. It gives the model
something better to carry than a paraphrase: an 83- or 84-character line
that re-hydrates to the exact signed bytes, for anyone, for as long as
the bytes exist. What is guaranteed is co-reference and integrity, never
truth. Confidence, uncertainty, and provenance travel alongside the value
(§7) precisely because the signature speaks to none of them.

The paper's contributions, stated once:

- **A failure model for agent memory.** Referential drift in two
  directions: the paraphrase that decays in language, and the readout
  that moves at a pinned reference, with a stated decomposition for the
  second (§1.1, §1.2).
- **An addressing discipline.** `(cell, band, tslot)` plus
  content-derived identifiers, so a place, an observation, and an
  object each have exactly one name (§2, §4, §5).
- **A receipt a stranger can check.** One domain-separated,
  length-prefixed signing rule across every signed object, with the
  verifier specification generated from the signer's own constants
  (§6).
- **A token grammar with per-type semantics.** What dereferencing an
  `emem:fact:`, `emem:entity:`, and `emem:cell:` token each proves, and
  what it does not (§3).
- **Two memory layers on one trust surface.** An Earth-memory corpus
  and an agent file memory, returning the same receipts (§10, §11).
- **An honest evaluation.** Measured latencies and throughput with the
  method stated per number, typed failure modes, and the limits
  restated as an adversary model (§14, §15, §16).

![emem architecture: one Rust binary, two wire surfaces, one optional sidecar](/docs/diagrams/01-architecture.svg)
*Figure 1: one binary, signed end to end. The same handlers answer MCP
and REST, reads need no auth, and every write is logged.*

### 1.1 Drift has two directions

Everything above describes drift in language: the reference decays while
the world stands still, and the token pins it. The world runs the same
failure in the other direction. The reference stands still, the readout
at it moves, and not every move is the world. A crop grows, but also a
satellite is swapped, a scene registers five metres off, an encoder is
upgraded, a cloud passes. Between two visits to one pinned address, the
observed change decomposes as

```text
Δz = Δ_env + Δ_sensor + Δ_geo + Δ_encoder + ε
```

the world changed, the instrument changed, the pixels moved, the model
changed, noise. Only the first term is about the world, and a memory that
cannot tell the terms apart either raises false alarms or hides real
events. The protocol already pins or separates every term but the first:
an embedding fact carries its encoder checkpoint and versioned recipe, so
Δ_encoder is checkable rather than hidden (§10.1); bi-temporality
separates a change in the world from a change in what the memory knew
(§4.3); every change points at a specific immutable fact (§5); and the
receipt makes whatever split is computed checkable by a third party (§6).
An attribution ledger ships (§10.3): per-term evidence under a signed
receipt. Splitting a delta numerically among the terms is still open
work; nothing in this build computes the split.

![the observed change at a pinned reference decomposes into environment, sensor, geometry, encoder, and noise terms; only the first is the world](/docs/diagrams/37-change-decomposition.svg)
*Figure 2: the decomposition. Only Δ_env is about the world; the
substrate pins or separates the other four by construction, and
splitting a specific delta is the open primitive of §10.3.*

### 1.2 Definitions and notation

The rest of the paper uses a small amount of notation, fixed here. All
of it formalizes what ships; none of it introduces a new mechanism.

**Addresses.** The address map takes a WGS-84 point to a 64-bit cell,
`A(lat, lng) = cell64`, quantizing latitude into 2^21 buckets and
longitude into 2^22 (§4.1). Time quantizes per band:
`tslot(t, b) = floor(unix(t) / slot_seconds(tempo(b)))`, so one instant
buckets differently for a weather band and a forest-loss band (§4.2). A
stored observation is addressed by the triple `(cell, band, tslot)`.

**Identifiers.** For a record `x` with canonical CBOR encoding
`cbor(x)` (§5), the content identifier is
`cid(x) = base32_nopad_lower(blake3(cbor(x)))`: 32 bytes, 52
characters. `entity_cid` and `bundle_cid` truncate the digest to its
first 16 bytes (26 characters) before encoding; `cid64`, the display
prefix used in logs, truncates to 8 (13 characters) and is a prefix of
the full identifier, never a separate hash. Manifest identifiers
(`bands_cid`, `registry_cid`, `schema_cid`, `sources_cid`) encode the
full digest of each manifest's canonical CBOR.

**Signatures.** Every responder signature is
`sig = ed25519_sign(sk, blake3(P))`, where the preimage `P` is a
domain-separated, tagged, length-prefixed segment stream:

```text
  P = "emem.preimage.v1\0" || len(d) || d || Σ_i ( tag_i || len(s_i) || s_i )
```

with `d` the domain string (for a receipt, `"receipt"`) and each
segment `s_i` one named field (§6.2). The tags are compiled constants,
and `GET /v1/verifier_spec` serializes those same constants, so the
normative description of the preimage cannot drift from the signer
(§6.6).

**Bi-temporal reads.** A read carries two optional bounds and returns
exactly the facts satisfying

```text
  tslot(f) <= as_of_tslot   ∧   signed_at(f) <= as_of_signed_at
```

each conjunct applying only when its bound is set (§4.3). Both bounds
enter the signed preimage, so the bound pair is part of what the
receipt proves, and a verifier replays the exact query years later.

**Readout change.** For one cell and band, let `z_t` be the stored
readout at tslot `t`, a scalar or an embedding. The observed change
between two visits decomposes as
`Δz = Δ_env + Δ_sensor + Δ_geo + Δ_encoder + ε` (§1.1), each term a
partial difference: the change that remains when only that factor
varies and the others are held fixed. For a memory readout `M(r, t)` at
reference `r` and an identity kind `k` with an equivalence relation
`≡_k` over readouts, reference stability is
`S_k = Pr[ M(r, t1) ≡_k M(r, t2) ]` and world-side referential drift is
`D_k = 1 - S_k`. Uncorrected, `D_k` measures the instruments; corrected
for the four nuisance terms, it measures the world. Because the
equivalence is a choice, `D` is a vector with one component per
identity kind, not a scalar; the identity kinds themselves are open
work (§10.3, §19).

---

## 2. External identity

Identity in emem is layered, and the layers differ in strength. Stating
them as one thing would be the same error this document exists to
correct, so they are separated here and kept separate throughout.

### 2.1 The address layer: places

A place is named by its `cell64` (§4). The address is a pure function of
latitude and longitude: it derives from the geometry, not from who asked
or when. Two agents that resolve the same point converge on the same
address by construction, with no coordination and no registry. This is
the strongest layer because there is nothing to agree about.

### 2.2 The content layer: observations

An observation is named by its `fact_cid`, the blake3 digest of its own
canonical CBOR encoding (§5). The bytes that are hashed are the bytes
that are stored, so the identifier is a checkable claim about the
content: recompute the digest, compare, and a substituted body is
detected. This layer is strong for the same reason: identity is derived
from the object, so agreement is arithmetic rather than negotiation.

### 2.3 The object layer: entities

Places and observations are not the only things agents talk about. A
bridge, a field, a landmark, a facility is an *object*, and objects are
where co-reference is hardest: "the collapsed span at the ford" and "the
Old Bridge" may denote one thing, and no amount of hashing resolves that,
because the phrasings share no bytes.

`POST /v1/entity` mints an `emem:entity:<entity_cid>`. The identifier is
computed by `compute_entity_cid`
(`crates/emem-primitives/src/entity.rs:176`) over one of two preimages:

```text
  with a stable external anchor:
      blake3("emem.entity.v1|ext|" || best_external_id)

  without one:
      blake3("emem.entity.v1|loc|" || cell64 || "|" ||
             normalize_text(kind) || "|" || normalize_text(label))
```

`best()` (`entity.rs:68`) prefers `gers:<id>`, then `osm:<type>/<id>`,
then `wikidata:<qid>`. The digest is truncated to its first 16 bytes and
base32-encoded, so an `entity_cid` is 26 characters, not the 52 of a
`fact_cid`.

The external-anchor branch is the one that works. Two agents that mint
from the same GERS or OSM id converge on the same `entity_cid` however
they phrase the label: the repository's own test asserts that
`("bridge", "Golden Gate Bridge")` and `("highway", "golden gate")` with
the same `osm:way/717919508` produce equal identifiers
(`entity.rs:291`). Geometry is deliberately excluded from the preimage so
that a geocoder's floats cannot perturb the identity (`entity.rs:20`).

The fallback branch is weaker and the responder says so in the response
itself. Without an external id, identity is label-based, and convergence
requires two agents to agree on a normalized label *and* a cell. The API
returns a `convergence` field marked `weak` in exactly that case
(`crates/emem-api-rest/src/lib.rs:22325`).

### 2.4 What the entity layer does not do

Three limits, all load-bearing, none of which the layer's name suggests.

**The mint is not signed over its content.** The receipt accompanying a
mint is signed over the segment set in §6.2: `request_id`, `served_at`,
optional axes, `primitive`, `cells`, `fact_cids`. The `entity_cid` rides
the receipt body as `intent`, and `intent` is not a preimage segment
(`crates/emem-attest/src/lib.rs:217` lists every tag; there is no
`INTENT`). So the receipt proves this responder served an `emem.entity`
request touching that cell at that time. It does not bind the identifier,
the label, the kind, or the external ids. The same applies to
`POST /v1/entity/alias`.

**An `emem:entity:` token does not dereference to byte-identical bytes.**
Because the identifier covers a narrow identity preimage rather than the
whole record, two nodes can hold different bodies under one `entity_cid`:
the stored `label` is the caller's display form, not the normalized form
that was hashed, and in the external-anchor branch `cell64` is not in the
preimage at all. There is no entity analogue of the cell-binding check
that protects facts (§3.3). What two agents holding one `emem:entity:`
token share is a canonical *name*. That is genuinely useful, and it is
less than the fact layer provides.

**There is no merge, and links do not conflict.** Nothing reconciles two
`entity_cid`s later discovered to denote one object. An alias key maps to
a growing list of identifiers: if one agent links "the north dam" to
entity X and another links the same phrase to entity Y, both survive, and
`entity_resolve` returns both as candidates with no contradiction signal
and no precedence.

### 2.5 Resolution is registration, not similarity

`POST /v1/entity/resolve` is an exact lookup on a normalized key
(`lib.rs:22369`). Normalization is trim, lowercase, and collapse internal
whitespace runs (`entity.rs:150`). There is no embedding, no edit
distance, no stemming, no substring match, and no ranking: candidates
come back in insertion order. A phrasing resolves only if that exact
normalized phrasing was previously registered as an alias, by a mint or
by `POST /v1/entity/alias`.

This is worth stating bluntly because the mechanism's value is real but
different from what "resolve" implies. Convergence here is
*registration-based*. Agents converge because someone recorded the
equivalence, not because the system inferred it. The `near` parameter
does not add inference either: it appends every entity indexed at one
exact cell, unranked and unfiltered by the query text.

---

## 3. The token grammar

A token is the citeable handle an agent carries instead of a payload. It
is one line of text, safe in a report, a commit message, a note, or an
audit log.

```text
  emem:fact:defi.zb5ff.pOyA.zaf69:mstu4hkloebwpupenqyubj6qacnrejhiwj4pcszsfltooqettr5q
```

The family is typed by its **second segment**, and the types are not
equivalent in what dereferencing proves. There is one type per shape of
memory: a point, a set, an object, a bare place, and, for a world model,
a field and a field over time.

![two agents, one memory: agent A hands agent B one line of text and agent B verifies it without trusting agent A](/docs/diagrams/35-two-agents-one-memory.svg)
*Figure 3: the token lifecycle. Recall returns a signed fact and its
receipt; the token is the one-line citation; the receiver resolves it to
the identical bytes and verifies the receipt offline.*

| Token | Shape | Dereferences to |
|---|---|---|
| `emem:fact:<cell64>:<fact_cid>` | 4 segments | the byte-identical signed fact (§3.2) |
| `emem:bundle:<bundle_cid>` | 3 segments | a signed set of citations |
| `emem:entity:<entity_cid>` | 3 segments | the body this node holds under that name (§2.4) |
| `emem:cell:<cell64>` | 3 segments | **nothing: no parser exists** (§3.4) |
| `emem:raster:<aoi_cid>:<band>:<tslot>:<derivation_cid>` | 5 segments | one native-resolution field over an area, as a signed derivation (§3.6) |
| `emem:cube:<aoi_cid>:<band>:<tslot_lo>..<tslot_hi>:<derivation_cid>` | 5 segments | a field over time: a signed manifest over raster slices (§3.6) |

Three legacy prefixes still resolve as full equivalents, not as degraded
forms: `memt:` for `emem:fact:`, `memb:` for `emem:bundle:`, `meme:` for
`emem:entity:`. Each is a single fallback on the prefix strip
(`lib.rs:20574`, `memory_bundle.rs:190`, `entity.rs:208`); the remainder
of the parse is identical. A resolve echoes the canonical spelling back,
so a legacy token is migrated by using it.

### 3.1 Why the second segment, and not a length

`parse_memory_token` (`lib.rs:20570`) strips the prefix and splits the
remainder exactly once, on the first colon. `cell64` is colon-free by
construction, so the split is unambiguous however long the digest is, and
the `fact_cid` is whatever follows. A shape check rejects the rest: the
cell must satisfy `is_cell64_shape`, and the digest must be
ASCII-alphanumeric within a length bound.

Length is not the discriminator, and cannot be: a `fact_cid` is 52
characters (the full 32-byte blake3 digest, base32-nopad-lowercase),
while an `entity_cid` and a `bundle_cid` are 26 (their preimages are
truncated to 16 bytes). Three identifier widths in one family is a wart,
and it is documented here rather than smoothed over.

### 3.2 What an `emem:fact:` token proves

This is the token the anti-drift claim rests on, and it is the one that
carries it.

The identifier is the digest of the stored bytes. `put_many`
(`crates/emem-cache/src/sled_hot.rs:446`) encodes the fact to canonical
CBOR, hashes *those* bytes, and inserts *those same bytes* under the
resulting key. Resolution reads them back and returns them inline. The
receipt over a resolve is signed with `fact_cids` populated
(`lib.rs:20766`), so unlike the entity paths, the identifier genuinely
enters the signed preimage.

The chain a receiver can check alone is therefore: the CID names the
bytes; the cell-binding rule refuses a mislabelled anchor (§3.3); the
receipt binds `(cell, fact_cid)` to the responder's key in a
domain-separated, length-prefixed preimage (§6); and every step verifies
offline.

One honest qualification. The responder does not recompute the digest on
read and compare it before serving; integrity of the read path rests on
the store's keying being honest. The *client* can and should recompute
it, which is the entire point of content addressing. The correct sentence
is "a client can detect a substituted body by recomputing the CID", not
"the responder guarantees the body matches the CID".

### 3.3 The cell-binding rule

A token names both a place and a digest, which creates a way to lie that
content addressing alone does not catch: cite a real Mumbai fact under a
Fuji cell. The resolver refuses it (`lib.rs:20728`). It reads the cell out
of the signed fact body, compares it against the cell in the token, and
returns **409 Conflict** on any mismatch rather than resolving with a
warning. The `cell` in a successful response is taken from the signed
body, never from the token.

This is the only cross-field integrity check in the token family. Bundles
and entities have no analogue, and §16 records that.

### 3.4 `emem:cell:` does not dereference

`cell_token` (`lib.rs:20561`) composes the string, and nothing anywhere
parses it. It appears in composed output and in reference documentation;
it has no resolution path. It is listed here for completeness and because
a previous version of this document's supporting material claimed
otherwise. A place is addressed directly by its `cell64`; the `emem:cell:`
wrapper adds nothing that resolves.

### 3.5 `bundle_cid` is not a CBOR digest

`compute_bundle_cid` (`memory_bundle.rs:139`) hashes a pipe- and
newline-delimited **text** preimage of the citations, then truncates to 16
bytes:

```text
  blake3("emem.memory_bundle.v1|" || purpose || "\n" ||
         (cell "|" band "|" tslot "|" fact_cid "\n")*)[..16]
```

It is not `blake3(canonical_cbor(body))`, which the module's own
docstring two lines above the function claims it is. The code is
authoritative and the docstring is wrong; both are noted in §18 because a
verifier that believes the docstring computes the wrong identifier.

### 3.6 Field tokens: what the receipt attests when the answer is an array

The scalar and identity tokens above split on the **first** colon (§3.1),
because everything after `<cell64>:` is one opaque digest. The two field
tokens do not: `emem:raster:` and `emem:cube:` carry four colon-separated
claims, and `parse_raster_token` / `parse_cube_token` (`band_raster.rs`)
split on every colon and bind each claim to the signed record before
anything dereferences. A world model is a field over an area across time,
and a scalar fact cannot name one; these two can.

A field token names an **array**, so its receipt attests a *derivation*,
not a value: this responder computed the artifact whose blake3 is
`artifact_cid`, from a pinned Sentinel-2 scene, via the versioned recipe
`band_raster@1`, over a content-addressed area (`aoi_cid`, the blake3 of
the request geometry) and time slot. The receipt's FIELD preimage segment
binds `(aoi_cid, derivation_cid)` through `emem_attest::field_binding_v1`,
reported by `/v1/verify_receipt` as `field_bound`. The pixels are a
canonical little-endian `f32` grid with a fixed 64-byte header
(`emem-codec/grid.rs`); a NaN folds to one quiet NaN and `-0.0` to `0.0`
before hashing, or two encoders that agree on every value still disagree
on the digest.

The artifact is evictable by design. It lives in a content-addressed blob
store served immutable at `GET /v1/artifacts/{cid}`; the small derivation
record persists like any fact and pins the scene, recipe, and geometry, so
eviction turns a dereference into a recompute, never a broken citation.

A cube adds no pixels. `emem:cube:` is a signed manifest over N
independently-resolvable raster slices, one per date, and `cube_cid` is
the blake3 of the ordered member `derivation_cid`s: the same slices in the
same order always name the same cube, and a resolve recomputes it and
refuses an altered membership with a typed 409. Its lineage terminates in
each member's pinned scene, so a stranger walks cube to slices to scenes
and re-derives every value from raw satellite bytes.

Verification has the two tiers §3.2's fact token has, at field size.
Spot-check (milliseconds, no upstream): re-hash the artifact against
`artifact_cid`, then read the sampled `anchors[]` back out of the grid
and, where an anchor carries a `fact_cid`, compare it against the ordinary
per-cell fact. Recompute (the full audit): fetch the pinned scene, rerun
the recipe, compare digests. A field token carries no new trust class: the
artifact inherits the band's provenance and the record says so.

---

## 4. Addressing: cell64 and tslot

### 4.1 cell64

A `cell64` is a 64-bit packed identifier for a square latitude/longitude
bucket on WGS-84 (`crates/emem-codec/src/geo.rs`).

```text
  bit:  63    60 59      52 51      44 43            22 21            0
        +-------+-----------+-----------+----------------+----------------+
        | mode  | resolution|   base    |     lat_q      |     lng_q      |
        | 0001  |    21     |   0xab    |    21 bits     |    22 bits     |
        +-------+-----------+-----------+----------------+----------------+
```

`mode = 1` marks a geo cell, `base = 0xab` is the geo aperture marker,
and `resolution = 21` distinguishes the active grid from the pre-0.0.3
305 m grid. A legacy cell64 fails decoding with `NotGeoCell` rather than
silently misplacing a fact by hundreds of metres.

![cell + band + tslot to canonical CBOR to blake3 to 52-character base32 CID](/docs/diagrams/09-address-algebra.svg)
*Figure 4: the address algebra. Three integers become one handle the
rest of the protocol cites.*

The string form is four base-65,536 bigrams joined by `.`, drawn from a
65,536-entry alphabet (`crates/emem-codec/src/alphabet.rs:48`); four
16-bit digits are exactly the 64 bits of the address. Bigram widths vary,
so the rendered cell is 20 or 21 characters and an `emem:fact:` token is
correspondingly 83 or 84. The alphabet is constructed so adjacent
codepoints map to adjacent cells, and the encoding is token-economical:
about four BPE tokens under cl100k/o200k.

The pitch is ~9.54 m on the latitude axis and ~9.55 m on the longitude
axis at the equator, chosen to match the native pitch of Sentinel-1 and
Sentinel-2. Latitude pitch is uniform; longitude pitch narrows with
cos(lat), so a cell at 60° is about 9.54 m by 4.77 m. Cells are square at
the equator and taller than wide elsewhere. The responder declares this
itself, with its own warnings, at `GET /v1/grid_info`; an equal-area hex
DGGS is the spec target and is not what this build serves.

### 4.2 tslot

A `tslot` is a `u64` bucket of the Unix timeline at a band's tempo
cadence (`crates/emem-core/src/tslot.rs`).

```text
  tslot = floor(unix_seconds / tempo.slot_seconds())
```

The anchor is the Unix epoch. Pre-1970 timestamps clamp to `tslot(0)`.
The cadence is a property of the band, not of the query: a weather band
and a forest-loss band bucket the same instant differently, because
asking for "the same time" means different things to a fast and a slow
observable.

The tempo classes are declared once per band in `bands-v0.json`:

```text
  Tempo       slot_seconds   typical bands
  ----------  -------------  --------------------------------------
  Static      0              copdem30m, gmrt, koppen
  Slow        31_536_000     geotessera vintages, soilgrids
  Medium      2_592_000      ndvi_monthly, modis composites
  Fast        86_400         s2 raw, s1 raw, modis lst_day_8day
  UltraFast   3_600          weather, air_quality
```

A band cannot be served at a tempo finer than its declared cadence, and
the slot is recoverable: a receipt that cites `(cell, band, tslot)`
lets a verifier recover the wall-clock window by multiplying the tslot
by the band's declared `slot_seconds`.

### 4.3 Bi-temporality

Every read primitive accepts two independent optional bounds:
`as_of_tslot` bounds *observation* time (what was on the ground) and
`as_of_signed_at` bounds *transaction* time (what the memory knew). Set
either, both, or neither.

The distinction is what makes "what did we know when we decided"
answerable. A forecast superseded next week is still the forecast that
was acted on, and `as_of_signed_at` replays it without hindsight. Both
bounds enter the signed preimage as a tagged segment when present (§6.2),
so the replay is checkable rather than asserted.

---

## 5. Content addressing

A fact's identifier is the blake3 digest of its own canonical CBOR
encoding, base32-encoded without padding and lowercased: 32 bytes, 52
characters. The property that matters is not the hash function but the
discipline around it, stated in the encoder's own header
(`crates/emem-fact/src/cbor.rs:4`): *two implementations must produce
byte-identical CBOR for the same fact*. Without that, a content address
is a per-implementation opinion.

Two rules deliver it, and both are narrower than "canonical CBOR"
implies.

**Struct field order is declaration order, not sorted.** `to_canonical_cbor`
serializes through ciborium and serde, which emit fields in declaration
order. It does *not* apply RFC 8949 §4.2.1 key sorting to struct maps. A
generic canonical-CBOR encoder that sorts keys computes a different
digest and will not interoperate. For freeform maps, callers must supply
pre-sorted keys; the encoder will not sort on their behalf.

**Floats are canonicalized before hashing.** Every NaN payload folds to
one quiet NaN and `-0.0` becomes `0.0` (`cbor.rs:43`). Without this, two
encoders that agree on the value disagree on the bytes, and the address
splits.

These are the kind of details a specification usually omits and an
implementer then discovers through a mismatched digest. They are stated
here because a content address is a promise about bytes, and a promise
about bytes has to descend to bytes.

The encoding carries four tags, and the identifier family has three
widths, stated once (§1.2 gives the rule). Tags: 65000 for a packed
cell, 65001 for a tslot, 65002 for a vec64 identifier, and IPLD tag 42
for external CIDs. Widths: a `fact_cid` is the full 32-byte digest (52
characters), `entity_cid` and `bundle_cid` are 16-byte prefixes (26
characters), and `cid64` is an 8-byte display prefix (13 characters)
for logs, never a key. The 16-byte truncation is a deliberate trade: a
birthday collision needs on the order of 2^64 objects, and the
canonical responder holds about 10^5 today. base32 without padding,
lowercased, is URL-safe, case-stable, and collision-free inside path
segments, which is where these identifiers spend their lives.

---

## 6. Trust: receipts and attestations

Every answer carries an ed25519 receipt that verifies offline against the
responder's published key. No callback, no account, no API key. The
receiver checks arithmetic, not reputation.

![receipt preimage, ed25519 signature, merkle path, offline verify](/docs/diagrams/10-trust-plane.svg)
*Figure 5: the trust plane. What a responder signs, and how anyone
re-checks it without trusting the server; the `/verify` page recomputes
these steps in the browser.*

### 6.1 One rule

Every object this responder signs uses a single construction: ed25519
over blake3 of a domain-separated, tagged, length-prefixed segment
stream.

```text
  stream    = "emem.preimage.v1\x00" || u32LE(len(domain)) || domain
              || segment*
  segment   = tag:u8 || u32LE(len) || bytes
  list      = tag:u8 || u32LE(count) || (u32LE(len) || bytes)*
  signature = ed25519(blake3(stream))
```

Optional segments are omitted entirely rather than written empty, so
presence and absence are unambiguous in the signed bytes. The signature
covers the 32-byte digest, not the raw stream.

Six object classes ride this rule today: the receipt, the attestation,
the transparency-log STH, a witness co-signature, the operator
attestation, corpus state stats, and stream ticks. Each declares its
domain string and its tag table at `GET /v1/verifier_spec`.

### 6.2 The receipt preimage

```text
  domain "receipt"
    0x01 request_id      0x06 manifest      (optional)
    0x02 served_at       0x07 primitive
    0x03 scope    (opt)  0x08 cells         (list)
    0x04 as_of    (opt)  0x09 fact_cids     (list)
    0x05 edges    (opt)
```

Two consequences are worth naming. The `as_of` bound **is** signed, so a
bi-temporal replay is checkable rather than asserted. And `intent` is
**not** in the table: it rides the receipt body but never enters the
signed bytes, which is why the entity mint and alias paths prove less
than their names suggest (§2.4).

### 6.3 Why v1 exists, and why v0 still verifies

The v0 rule concatenated variable-length fields with no length prefixes
and inserted optional fields untagged at a fixed position. Both are
exploitable ambiguities rather than aesthetic faults:

- Without length prefixes, `"abc" || "def"` and `"abcd" || "ef"` hash
  identically. Field boundaries were unsigned.
- An untagged optional 64-char hex segment is indistinguishable from any
  other. A scope digest and an `as_of` digest occupied the same position
  and the signature could not tell them apart.

The v0 Merkle construction had a third: leaves and interior nodes were
hashed with the same function, and odd layers duplicated their last
element, so `root([A,B,C]) == root([A,B,C,C])`. This is the CVE-2012-2459
pattern. v1 prefixes leaves with `0x00` and interior nodes with `0x01`,
and verifiers of v1 attestations must additionally reject duplicate
leaves; canonical order is sorted, so duplicates are adjacent.

Every new receipt is signed under v1 and carries `preimage_version: 1`.
Receipts signed before the cutover carry no such field and continue to
verify byte-for-byte under the v0 branch, which is retained for
verification and never for emission. Deleting v0 would silently
invalidate history, which is the one thing an immutable record may not
do.

### 6.4 The two exceptions, named rather than hidden

**Legacy v0 receipts.** Verify-only, never emitted, as above.

**`memory_write`.** The one construction that is not preimage_v1, because
it runs in the opposite direction: the *caller* signs and this responder
verifies.

```text
  attester_preimage = blake3("emem.memory_write|" || verb || "|"
                             || path || "|" || body_hash)
```

`verb` is one of `create | str_replace | insert | delete | rename`. The
pipe form is deliberately frozen rather than migrated: its shape is
pinned by every client that already signs against it, and re-cutting it
would invalidate signatures already issued. It is safe despite the
separator because the variable field is bracketed: `verb` comes from a
closed, separator-free set, and `body_hash` is a fixed-length 32-byte
digest tail.

`body_hash` is **per-verb**, and this is where a reader of v1 goes wrong:

| verb | `body_hash` |
|---|---|
| `create` | blake3 of the `file_text` UTF-8 bytes, exactly as transmitted |
| `str_replace` | blake3 of the whole file's bytes **after** the edit |
| `insert` | blake3 of the whole file's bytes **after** the insert |
| `delete` | `blake3("")` |
| `rename` | `path` is the **new** path; `body_hash` is blake3 of the **old** path |

The responder never hashes the request body. v1 of this document said it
did (§18), and a client that implemented v1's rule could not produce a
signature this responder accepts.

`rename` earns its own line. Hashing the source path is what makes one
signature bind both ends of the move. Without it, the signature
authorises "move something to `new_path`" while naming no source, and a
captured signature could be replayed to drag a different file to the same
destination.

### 6.5 Capability, not account

A write to `/memories/by_attester/<pubkey8>/...` is accepted only from
the key whose shortcode matches, where `pubkey8` is the first 8
characters of the base32-nopad lowercased pubkey. A mismatch returns
**403 `memory_namespace_violation`**. There is no registration step: any
ed25519 keypair works, generated locally, and the namespace is claimed by
whoever first writes to it and then held by that key. Bare `/memories/...`
paths accept writes with no attester block, for Anthropic memory-tool
back-compatibility.

Verification is `ed25519_dalek::verify_strict`, which rejects malleable
signatures.

A refusal is designed to be actionable in one turn: the 401 carries the
exact 32-byte digest to sign for that exact request, the base32 encoding
rules, the per-verb body-hash rule, and a runnable example. An agent gets
from refusal to signed write without going to look for documentation.

### 6.6 The spec cannot drift from the signer

`GET /v1/verifier_spec` is emitted by the responder from the same
compiled constants the signer uses: the tag tables are serialized from
`receipt_tag::*` and the domain strings from the modules that sign, so
the published spec cannot disagree with the wire. A number re-typed into
prose can rot; a symbol serialized from the signer cannot.

This document is prose, and prose rots. That is not a hypothetical: it is
what happened to v1 (§18), and it is why the machine-readable spec, not
this text, is normative where the two ever disagree.

---

## 7. Tamper-provenance

A signature proves who attested a value and that the bytes have not
changed. It says nothing about how the value came to exist. A satellite
radiance, an index recomputed from that radiance, a neural encoder's
embedding, and a human cartographer's judgement are all equally signable
and are not equally checkable. Collapsing them into one trust surface
would be the interesting lie: everything looks verified, so nothing is.

Every band therefore declares a **tamper-provenance class**
(`crates/emem-core/src/bands.rs:64`). There are five.

| class | `tamper_evidence` | deterministic | rank |
|---|---|---|---|
| `direct_sensor` | `recomputable_from_source` | yes | 4 |
| `deterministic_index` | `recomputable_from_source` | yes | 3 |
| `model_output` | `signed_model_checkpoint` | no | 2 |
| `human_curated` | `attester_only` | no | 1 |
| `unclassified` | `attester_only` | no | 0 |

`unclassified` is not a placeholder. It is the serde default, so a band
that fails to declare a class fails *closed*, at the lowest rank,
claiming no tamper-evidence. An unknown band key resolves to it rather
than to an optimistic guess.

The 43 bands of the active ontology distribute as 23 `model_output`, 7
`deterministic_index`, 6 `direct_sensor`, 5 `human_curated`, and 2
reserved slots left `unclassified`. Sentinel-1/2 raw, the DEMs,
nightlights, and GMRT are `direct_sensor`; spectral indices, terrain
derivatives, and phenology are `deterministic_index`, of which only
spectral indices are wired to a materializer and the remaining six are
declared slots with no connector; the foundation encoders, land cover,
surface water, and climate are `model_output`; Overture, protected areas,
ecoregions, and admin boundaries are `human_curated`.

### 7.1 The reader's controls

`deterministic: true` on a read is sugar that folds to a class list: it
keeps `direct_sensor` and `deterministic_index` and drops the rest. An
explicit `provenance` list intersects with it; an empty intersection is a
typed 400 rather than a silent empty result. The filter runs before facts
load and before the receipt is signed, so the signed preimage covers
exactly what was returned.

Three classes carry an in-band `caution` string, emitted from a single
definition site: `model_output` warns that a learned representation is a
hypothesis subject to training bias, domain shift, and encoder revision;
`human_curated` warns about editorial choices and coverage gaps that vary
by region; `unclassified` warns that the production method is undeclared.
`direct_sensor` and `deterministic_index` carry none.

### 7.2 What the class attests, and what it does not

`bands_cid` is the blake3 of the canonical CBOR of the band registry. It
enters `source_versions`, which is hashed into `manifest_hex` and mixed
into the receipt preimage as tag `0x06` (§6.2). So a verifier can
reproduce *which classification table was in force* when an answer was
signed, offline.

Six limits, each of which the mechanism's name invites a reader to miss.

**The class is an editorial declaration, not a measurement.** It is a
hand-maintained field in a JSON table, per band. Nothing in the codebase
re-derives a value to check its label. `deterministic: true` means "this
band is *declared* recomputable", never "this value *was* recomputed".

**`bands_cid` attests the table, not the fact.** It proves which table
was in force. It does not prove the classification is correct, nor that
the responder applied it faithfully to the value it returned.

**The manifest segment is optional.** A receipt with no manifest binding
still verifies; `verify_receipt` reports `manifest_bound: false` rather
than failing. Receipts predating the binding are unbound by design, so
their provenance is not covered.

**The class is per-band, so a fact's own lineage is invisible to it.** An
`indices.ndvi` computed over a cloud-contaminated pixel is
`deterministic_index` all the same. The class says the *formula* is
fixed. It says nothing about whether the *input* was sound.

**The cited source is not content-pinned.** `Source.hash` and
`Source.cid` are `None` on every fact the responder materializes
(`crates/emem-fact/src/fact.rs:207-212`; the only writer of `hash` in the
tree is a CLI demo). A fact pins its source by URL, scheme, id and
capture time, not by a digest of the bytes read. If an upstream object is
reprocessed in place, the receipt still verifies and the recomputation no
longer agrees. `recomputable_from_source` is a claim about the formula,
not about the immutability of the source.

**"Recomputable" is not "bit-identical from the published formula."** The
published form of NDVI is `(B08 - B04) / (B08 + B04)`. The responder
scales each band to reflectance (`DN * 1e-4`) before taking the ratio.
The two are algebraically equal and differ in the last bits: on a
measured Cambridge cell, `0.03959249826348693` against
`0.03959249826348692`, a difference of 2 ULP. Bit-exact agreement
requires the scaling convention, not just the formula. The convention now
travels in the band manifest's `formula` field, so a third party can
reproduce the bits from published information alone.

One further caveat is operational rather than architectural: the caution
rides `band_metadata`, which single-cell `/v1/recall` emits per fact but
which fan-out endpoints such as `recall_polygon` hoist to the response
root at their default verbosity. A caller walking `facts[]` alone on a
fan-out endpoint will not see it.

---

## 8. The transparency log

Immutability inside one responder is a promise about a data structure.
Immutability a client can *check* is a promise about a log. emem keeps an
append-only Merkle tree over attestation batches and serves proofs
against it.

### 8.1 What it is, precisely

It uses the **RFC 6962 tree construction**, with one substitution that
must be stated up front: the hash is **BLAKE3-256, not SHA-256**. RFC
6962 §2.1 mandates SHA-256. This log is therefore not RFC 6962
conformant, and no Certificate Transparency client, auditor, or monitor
will interoperate with it. What is borrowed is the construction: leaves
are prefixed `0x00`, interior nodes `0x01`, and a lone node is *promoted*
rather than paired with itself, so `root([A,B,C]) != root([A,B,C,C])`. A
test pins exactly that.

This makes the transparency tree structurally distinct from the **batch
root** carried inside a receipt, which uses the older self-pairing
construction and is malleable in the way §6.3 describes. The two trees
are not the same tree, and §8.4 explains why that matters more than it
sounds.

A leaf is `blake3(0x00 || blake3(attestation_cbor))`: the hash of a whole
attestation *batch*, appended from exactly one call site, after
verification. Individual facts are not leaves.

Five endpoints: `GET /v1/log/sth`, `GET /v1/log/inclusion`,
`GET /v1/log/consistency`, `POST /v1/log/witness`, and
`GET /v1/log/witnesses`. Inclusion and consistency proofs are both real,
both pure functions, and both round-trip tested for every leaf at every
size from 1 to 33; the inclusion verifier rejects an over-long path, and
the consistency verifier rejects a forked history.

The STH is signed by the responder's own key over a tagged preimage of
`(tree_size, root, signed_at, responder_pubkey)`. The `log_id` **is** the
responder's pubkey: the log has no identity independent of the server
serving it. It is recomputed and re-signed on every request, so there is
no issuance cadence and no maximum-merge-delay to reason about.

### 8.2 What it proves

Exactly one thing: **to a client that pinned an earlier `(tree_size,
root)` and comes back, the log can prove the current tree is an
append-only extension of what that client already saw.** That is
tamper-evidence against rewriting, for one client, over time.

Absent a pin, the endpoints prove nothing. A first-contact client has no
baseline, and a signed tree head it has never seen before is just an
assertion by the party it is trying to check.

### 8.3 What it does not prove

**It does not prevent split-view equivocation, and cannot today.** The
claim that a responder cannot tell two clients two different histories
does not hold, for four independently sufficient reasons. Every artefact
is served by the accused party: the STH, the proofs, and the witness list
all arrive over the same connection from the same server, and an
equivocating responder serves each victim a self-consistent view of all
three. There is no gossip protocol; detecting a split view is
definitionally impossible without an out-of-band channel between the two
victims, and none exists. Witness independence is unenforceable (§8.5).
And the client must pin and return, which is client-side discipline, not
a protocol guarantee.

**It does not prove the log is complete.** A responder that never appends
logs nothing. The log cannot prove absence, non-omission, or that
anything in particular was ever offered to it.

**It does not prove content is true.** It proves a byte sequence was
appended in an order.

### 8.4 The receipt does not chain to the log

This is the gap most worth stating plainly, because the two mechanisms
read as one system and are not.

A client **cannot currently prove that a fact it was served is in the
transparency log.** The receipt preimage has no log binding: there is no
`sth`, `root`, `tree_size`, or `leaf_index` segment in the tag table
(§6.2). The Merkle proof a receipt does carry is over the batch root, a
different tree. And to query `/v1/log/inclusion?entry_hash=`, a client
would need `blake3(attestation_cbor)` for a batch it never receives and
cannot request, because no endpoint reads an attestation back. There is
no `fact_cid -> leaf_index` index.

In practice the log is auditable as a standalone append-only structure,
disconnected from the facts anyone is actually served. Closing that gap
is tracked as the next increment on this substrate, and until it lands,
the honest description is the one in this paragraph rather than the one
the word "transparency" suggests.

### 8.5 Witnesses are scaffolding, not a network

The mechanism is real: a witness signs `(tree_size, root,
witness_pubkey)` over a tagged preimage, deliberately excluding
`signed_at` so the claim is purely "at size N I saw root R". The
responder verifies with `verify_strict` and refuses to record a root it
cannot reproduce, returning **409 `witness_root_mismatch`**. Submission
is idempotent per `(size, witness)`.

The deployment is not a network. As of this document's date the
production responder carries exactly **one** witness, whose co-signature
is for tree size 476,131 and dated 2026-07-10, tens of thousands of
entries behind the live tree; `GET /v1/log/witnesses` reports the current
state, and that endpoint rather than this sentence is the number to
quote. There is **no witness allowlist, registry, or trust anchor
anywhere in the codebase**, and no witness client or daemon:
`POST /v1/log/witness` accepts any well-formed ed25519 key, so an
operator could mint any number of nominally independent witnesses with a
keygen loop. Nothing tells a client that the one witness key is
independent of the responder key.

A witness co-signature therefore proves only that some key signed a
`(size, root)` pair. It does not prove that key is independent, honest,
ran any check, or is not the operator. The 409 guard protects the
responder from vouching for foreign roots; it does nothing for the
client.

Treat witness co-signing as a demonstrated mechanism awaiting an
operating network. It is listed here as scaffolding because a reader who
took the endpoint's existence for an independence guarantee would be
worse off than one who knew nothing about it.

---

## 9. Caller-registered derivations

The primitives to §8 describe reading a shared memory. `POST /v1/derive`
is the other direction: a caller registers a value *they* computed over
parent facts, under their own key.

```text
  sig       = ed25519(blake3("emem.memory_write|derive|/v1/derive|"
                             || body_hash))
  body_hash = blake3(cbor(DeriveBody))
```

The binding reuses the §6.4 attester scheme verbatim, with `verb =
"derive"`. One scheme, one verifier. The verb is what separates the
domains: no memory-file verb is spelled `derive`, so a signature minted
for a file write can never be replayed as a derivation, or the reverse,
and tests pin both directions. `/v1/derive` sits outside the
`by_attester` prefix, so the namespace rule is a no-op and the signature
binds content alone.

Four rules make a registered derivation worth citing.

**Parents must resolve.** Every input token is parsed, fetched, and
cell-bound (§3.3) before anything is written. Unvalidated lineage is
worse than no lineage: it would let a caller mint a token whose ancestry
appears to terminate in signed measurements when it terminates in
nothing.

**The value is canonicalized before hashing**, so the digest the caller
signs and the bytes that get stored describe the same number. Doing it
afterwards would sign one value and store another.

**Callers may declare only `model_output` or `human_curated`.**
`direct_sensor` and `deterministic_index` assert tamper-evidence this
responder cannot check for a value it did not compute; accepting them
would be the responder vouching for a stranger's arithmetic.

**The responder signs the storage, not the truth.** The receipt attests
that this attester submitted this derivation over these parents at this
time and that this responder stored it. It does not attest that the value
is correct. The caller's binding is recorded inside the fact body, hence
inside the `fact_cid`, hence covered by the responder's signature, so a
third party holding only the fact can rebuild the signed body and
re-check the *caller's* signature without the original request.

Derivatives carry no canonical `(cell, band, tslot)` key, so they never
enter the canonical, multi-attester, or scope indexes, and no default
read returns them. This is structural rather than a filter: every keyed
read walks one of those indexes. Retrieval requires naming the thing,
either by its token or through `POST /v1/derived`, which requires an
attester pubkey. A stranger's derivation cannot pollute the shared memory
at a cell, and the author's own token still resolves for anyone holding
it.

### 9.1 Recomputation: earning `deterministic_index`

The four rules above hold a derivation at `model_output`: attributed, not
checked. There is one narrow path to more. When the caller pins a
`code_cid` and the `op` is a pure scalar function the responder can
reproduce with no code execution (`delta` = `inputs[1] - inputs[0]`,
`mean`, `sum`), the responder re-runs that op over the cited parent facts
and compares the two scalars as canonical f64. If they agree, the
derivation is recorded as `deterministic_index`: recomputed, not merely
attributed. On a mismatch, or an op the responder cannot reproduce, it
stays `model_output` with an explicit reason. A derivation with no
`code_cid` is untouched. The caller still declares `model_output`; the
recomputation is a separate responder-signed record in the fact's `args`,
next to the caller's untouched declared class, so the offline authorship
check of §6.4 still verifies against the bytes the caller signed.

What "agree" means is not the same for every op. An op that does not
accumulate compares exactly: `delta` is one subtraction, a classification
is a tree of 2-ary selections, and a two-parent reduction has nothing to
accumulate, so the recomputed and claimed values are equal bit for bit or
the check fails. Rule `canonical_float_equality`, tolerance 0. A
reduction over many parents is a different object. Measured on this
responder over real signed NDVI parents, `sum`:

```text
  parents N   ULP gap   strict equality would
  ---------   -------   ---------------------
          5         0   verify
         16         1   fail
         32         2   fail
         64         2   fail
        128         0   verify
```

The gaps are one or two representable steps, about 1e-16 relative,
against 1e-6 as the tightest threshold any decision in this study
evaluates. And the failure is not monotonic in N, because accumulation
error cancels as readily as it compounds: under strict equality, whether
a given sum verifies is unpredictable from the caller's side, which is
worse than a stated bound because it looks like a guarantee. So `mean`
and `sum` over more than two parents compare under a published 4-ULP
window, rule `reduction_ulp_window`. The window is never silent. Every
response carries `rule` (which comparison ran), `ulp_tolerance` (the
bound), and `ulp_gap` (the measured distance), on success and on failure
alike, so a caller who needs bit-identity demands gap 0 and can see it.

The boundary is what was signed. A leaf fact's `value_verbatim` is the
signed preimage, so its bytes are cryptographically load-bearing and are
compared byte for byte, with no tolerance, ever. Nobody signed the sum.
The responder derives it, and no accumulation order was ever specified
for a caller to match, so bit-equality there would be luck about
summation order rather than fidelity to a signature. That is the limit on
what deterministic recomputation can promise across a language boundary:
exact reproduction is available for ops with no accumulation, and
reductions verify to a published bound with the measured gap returned.
This is tier 1. A sandboxed re-execution of arbitrary `code_cid` (tier 2)
is designed and not built.

---

## 10. Substrate: Earth observation

Nothing in §2 through §9 is satellite-specific. The record, the receipt,
and the token grammar know about addresses, bytes, and keys. Earth
observation is the substrate this responder ships with, chosen because it
is the hardest honest test available: open, adversarially large, and
already full of the drift the protocol exists to stop.

Open data from ESA, NASA, USGS, and the EU JRC fills the memory on
demand: **46 declared source schemes** and **129 materializer-wired
measurements**, live at `/v1/sources` and `/v1/bands`, spanning elevation
and NDVI through weather, forest change, surface water, and four open
foundation-model embeddings (Tessera, Clay v1.5, Prithvi-EO-2.0, and
Galileo; references at the end). **168 algorithms** and **27 topics**
are enumerated at `/v1/algorithms` and `/v1/topics`.

![the encoder runs at the source and emits an embedding; emem stores and signs the embedding on the ground](/docs/diagrams/31-encoders-in-orbit-decoders-on-ground.svg)
*Figure 6: the representation is the interface. emem signs the
embedding agents actually read, not the pixels, and the frozen encoder
checkpoint pins the Δ_encoder term of §1.1 by construction.*

### 10.1 Bands: the 1792-dimension voxel

The band ontology declares **43 cube slots** summing to exactly **1792
dimensions**. Offsets are contiguous, and reserved slots leave room for
new bands without breaking existing offsets.

```text
  offset  dims  key                family       tempo   privacy
  ------  ----  -----------------  -----------  ------  --------
       0   128  geotessera         vision       slow    public
     128    64  overture           human        slow    public
     192     7  air_quality        climate      ultra   public
     199   505  _reserved_512      reserved     static  public
     704    10  sentinel2_raw      optical      fast    public
```

The gap between 43 slots and 124 wired names is parametric expansion:
every Sentinel-2 reflectance band, every spectral index, every Tessera
vintage, and every Open-Meteo variant rides a fixed underlying slot.

The ontology is compiled into the binary (`include_str!` over
`bands-v0.json`) and read through one `LazyLock` by every consumer. There
is no env var, no file load, and no runtime override: a deployment that
wants a different ontology recompiles. The module's own header claims a
hot-swap capability that does not exist in the code, and §18 records
that.

### 10.2 Beyond satellites

Any observer with a location and a signing key can join the same attest,
recall, cite, verify path. `POST /v1/attest` is the multi-writer endpoint
and ships today. Substrate profiles for fixed sensors, drones, robot
fleets, industrial machines, and government registries are roadmap work,
not claims about this build. Location stays the first key for all of
them.

### 10.3 Change attribution: what a readout change means

The §1.1 decomposition becomes concrete on this substrate. Take one cell,
one band, two tslots, and ask why the value moved.

- **Δ_encoder, pinned.** An embedding fact names its model checkpoint and
  its versioned recipe (`derivation.fn_key` in the content-addressed
  algorithm registry), so an encoder upgrade is a visible property of the
  record, never a silent shift that masquerades as change on the ground.
- **Δ_sensor and Δ_geo, recorded but not attributed.** Scene selection is
  signed into provenance (`scenes_tried`, the chosen scene id, the SCL
  cloud class), and the processing-baseline guard pins the upstream's
  radiometric convention. What was observed through is on the record; no
  computation splits its contribution out of a delta.
- **Δ_env against the rest.** The bi-temporal bounds (§4.3) keep "the
  world changed" and "the memory learned" as separate questions with
  separate answers.

What ships as change surfaces today, stated plainly: `emem_diff` returns
a raw delta (`nd.delta@1`); `state_diff` returns residual, L2, and cosine
between two vintages of one encoder; `triple_consensus` reports
year-over-year agreement across three encoders behind a gate its own
output documents as uncalibrated. None of them attributes.

What ships as of 2026-07-16 is the LEDGER: `POST /v1/change_attribution`
(tool `emem_change_attribution`, registry key `change_attribution@1`)
returns per-term evidence for one cell, the observed Tessera
year-over-year change, the label-free index pairs with raw deltas, the
sources each visit was observed through, the encoder pinning proof, and
the scene-class per visit, every item carrying its fact cids under a
signed receipt, with `split` null and an in-band note saying why. The ledger
also persists: each run stores itself as a derivative fact (band
`change_attribution.ledger`, parents = every fact read) and returns its
`emem:fact:` token, so an attribution is cited and dereferenced like
any reading. What does not exist is the split itself: the estimator
and the calibrated cross-encoder, cross-sensor stability model it
needs are open work. The roadmap carries it.

### 10.4 The derived layer: algorithms

The algorithm registry holds 160 content-addressed entries in three
kinds: *solo* (one input band to one derived value), *combined*
(multi-band composites like a flood-risk score), and *embedding*
(operations over a foundation vector, such as similarity or novelty).
Each entry declares its inputs, a plain-math formula, its output, a
citation, and three provenance fields: `parameters` (typed, tunable
thresholds), `learned_from` (every tuned number cites the work it was
estimated against), and `prerequisites` (seed data whose absence
becomes a typed signed absence rather than a crash).

Deterministic entries carry an evaluation AST of fifteen variants
(arithmetic, `Linear`, `Clamp`, `Where`, `WeightedBlend`, and the usual
scalar maps). The AST evaluates against a recall snapshot in-process
and enumerates its own required bands, so the dispatcher can run every
algorithm whose inputs are present and skip the rest with a named
reason. A worked example: `flood_risk@2` blends surface-water
recurrence, elevation, and SAR backscatter as
`0.55 * (swr/100) + 0.25 * dem_agreement * (relu(50 - cop)/50) + 0.20 *
sigmoid((-15 - s1)/2)`, cites Pekel et al. 2016 for the water layer,
and round-trips byte-stably through canonical CBOR.

Two rules keep the registry honest. The *sensor tier rule*: an
algorithm claiming delivery resolution of 10 m must anchor on at least
one Sentinel-1, Sentinel-2, or Landsat input; coarser sources are
context, never the sole variance source for a fine-resolution claim.
And the ensemble caveat of §10.3 applies to the registry's central
change algorithm: the three encoders vote through per-encoder cosine
deltas behind a shared gate (default 0.15, adopted from the LandTrendr
ensemble convention in Healey et al. 2018), the design rationale being
that independent receptive fields (chip-scale, multi-temporal,
per-pixel) alias differently, so agreement is evidence of land-surface
change rather than encoder artifact. The gate is not per-encoder
calibrated, the output says so, and §14 reports what the ensemble
actually did on a live sample.

### 10.5 Read primitives

`recall` is *ensure*, not *get*: a miss with a registered connector
materializes on demand (upstream range read, compute, sign as the
responder, persist, return), bounded by a 30-second materializer
timeout and a 180-second gateway timeout, and a miss with no connector
returns a typed signed absence. A recall that names an unknown band
returns `bands_already_attested_at_cell`, the band keys actually
present, so a caller learns its name is wrong rather than that the
cell is empty. The responder signs materialized facts as itself, with
`derivation.fn_key` declaring exactly how each value was produced.

`find_similar` runs k-nearest-neighbour over a declared embedding band
in three modes: full-precision cosine, binary Hamming, and a triaged
Hamming-then-rerank. The binary sibling band packs each embedding
through a seeded, content-addressed rotation: a keyed blake3 stream
seeds Gaussian samples, Gram-Schmidt orthonormalizes them, and each
rotated dimension contributes its sign bit. The rotation matrix has a
content address of its own, so a verifier rebuilds it from the seed
text and byte-compares the packed vector. Results deduplicate per cell
(keeping the best vintage), a claim filter drops undecidable cells
rather than scoring them false, and `returned_k` is reported next to
`requested_k` rather than padded.

The remaining read surfaces (`diff`, `compare`, `trajectory`,
`query_region`) return raw deltas and series with receipts; none of
them attributes a change, which is exactly the §10.3 gap.

---

## 11. The agent-discoverable surface

`emem-server` serves HTTP/REST and MCP JSON-RPC on one port (default
`0.0.0.0:5051`): **156 documented REST paths under `/v1/*`** (162 total
in OpenAPI) and **107 MCP tools (16 core, 91 extended)**.

Discovery on first contact:

```text
  1. GET  /.well-known/emem.json         responder pubkey + capabilities
  2. GET  /.well-known/mcp.json          MCP transport advertisement
  3. GET  /.well-known/agent-card.json   metadata, recommended tool order
  4. GET  /v1/manifests                  bands_cid, algorithms_cid,
                                         sources_cid, schema_cid,
                                         registry_cid, topics_cid
  5. GET  /v1/grid_info                  cell pitch, honest warnings
  6. GET  /v1/verifier_spec              the signing rules, code-generated
  7. POST /v1/locate {q:"<place>"}       -> cell64
  8. POST /v1/recall                     -> signed facts + receipt
```

### 11.1 MCP tools are not read-only

v1 of this document stated that MCP tools are a strict read-only subset
of REST and that writes go through REST only. That is false, and the
responder refutes it from its own annotations: **8 of 102 tools are write
tools**. `memory_create`, `memory_str_replace`, `memory_insert`,
`memory_delete`, and `memory_rename` are destructive; `emem_entity`,
`emem_entity_link`, and `emem_derive` are non-destructive writes. An
agent can attest, mint an identity, and register a derivation over MCP
without touching REST.

### 11.2 Tiering is a listing decision, not a capability decision

An MCP host loads every advertised descriptor into the model's context at
connect. All 107 cost about 288 KB of every conversation whether or not it
ever touches Earth observation. So `POST /mcp` advertises the 16 tools of
the core loop in a single page, about 66 KB, and `POST /mcp/full`
advertises all 107. Both byte figures were measured on the wire on
2026-08-11 and move whenever a tool description is edited; re-measure
rather than quote.

Narrowing discovery removes no capability: **`tools/call` dispatches all
107 by name at either endpoint**, and an explicit
`{"tier":"core"|"extended"|"all"}` overrides the endpoint default. A tool
absent from a list is still callable. This matters because a tool an
agent cannot see is a tool it concludes does not exist, and the failure
mode of a 288 KB dump is the same conclusion reached for a different
reason: the agent stops reading.

`emem_tools` is itself core and maps the rest. With no arguments it
returns the core loop in order plus a bundle and shape menu in about 6
KB. `{"name":"emem_ndvi"}` returns one tool's input schema and a runnable
example in about 2 KB. `{"q":"ndvi"}` searches.

Every tool declares exactly one **shape** and any number of overlapping
**bundles** in MCP-standard `_meta`, as `dev.emem/shape` and
`dev.emem/bundles`. Shape is the form of the answer, because "which tool
do I use" is nearly always a question about the shape of the answer
rather than its topic: `scalar`, `timeseries`, `raster`, `geometry`,
`vector`, `identity`, `token`, `proof`, `plan`, `file`, `catalog`. A
bundle is a view by job: `tokenisation`, `verification`, `agent_to_agent`,
`long_horizon`, `robotics`, `satellites`, `agriculture`, `forestry`,
`climate_risk`. Both filter `emem_tools`, and bundle filters `tools/list`
itself.

---

## 12. Conformance

Two implementations conform when, given byte-identical inputs, they
produce byte-identical CIDs over the manifest set at `/v1/manifests`:

```text
  bands_cid        blake3 over canonical_cbor(BandsManifest)
                   (1792 dims, 43 cube slots)
  algorithms_cid   blake3 over canonical_cbor(AlgorithmsManifest)
                   (160 entries)
  sources_cid      blake3 over canonical_cbor(SourcesManifest)
                   (46 schemes)
  topics_cid       blake3 over canonical_cbor(TopicsManifest)
                   (27 topics)
  schema_cid       blake3 over canonical_cbor(SchemaBundle)
  registry_cid     blake3 over canonical_cbor(FunctionRegistry)
                   (20 primary / 2 derivative / 1 negative)
```

A receipt binds `schema_cid` and `registry_cid` as struct fields, and
binds the whole manifest set through the optional `manifest` segment
(§6.2) when `source_versions` is non-empty.

`GET /v1/verifier_spec` is the normative statement of the signing rules,
because it is generated from the signer's own compiled constants (§6.6).
Where this document and that endpoint disagree, that endpoint is right.

`cargo test --workspace` is the de-facto conformance check today. The
crate-internal tests in `emem-codec/src/geo.rs`, `tslot_text.rs`, and
`emem-attest/src/lib.rs` are the de-facto fixtures. Per-vector fixtures
under `spec/test_vectors/` are not yet extracted, and until they are,
"conformance" means "passes our tests", which is weaker than the word
implies.

---

## 13. Privacy

Four `PrivacyClass` variants (`crates/emem-core/src/privacy.rs`):
**Public** for open-data bands; **AggregateOnly { min_res }**, where
population-density bands snap to a coarser resolution and the receipt
carries `privacy_snapped: true`; **L2OnlyWithModelCid**, for
fine-resolution embeddings tied to a specific model checkpoint, which L1
responders must refuse; and **Prohibited**, reserved, declared by no band
today.

Legal surface, cited in SPEC.md §13: GDPR (Reg 2016/679), UK-GDPR,
DPDP-2023, CCPA-CPRA, RFC 9116. The canonical responder logs
`agent_ip_hash = base32_nopad_lower(blake3(client_ip)[..8])` rather than
the raw address. POST bodies are not captured. GET query strings are
captured for the 30-day journald retention window, which means a place
name in a query string is retained for 30 days and a place name in a POST
body is not.

---

## 14. Evaluation

Every number in this section is lifted from
[benchmarks.md](benchmarks.md), which states the method next to each
measurement; nothing is restated that was not measured there. Two
honesty rules carry over. Numbers marked SAMPLE come from the small
committed fixture and are illustrative, never the published result.
And single-node numbers make no scaling claim.

### 14.1 System performance

Measured against the production responder on its own serving host over
loopback (30 vCPU, 216 GB RAM, network-attached block storage, public
background load included), 2026-07-11:

```text
  Measurement                          Result
  -----------------------------------  --------------------------------
  Warm recall latency                  p50 2.5 ms · p95 6.1 ms · p99 9.1 ms
  Warm recall, deterministic filter    p50 2.4 ms (no measurable overhead)
  Cold recall (auto-materialize)       0.5 s to 1.6 s (n=3, upstream-bound)
  Receipt verification, server side    p50 1.0 ms · p99 4.2 ms
  Receipt verification, offline        p50 0.13 ms · p99 0.17 ms
  Token dereference                    p50 1.2 ms · p99 3.0 ms
  Sustained read throughput            632 requests/s (8 clients, warm)
```

The shape matters more than the absolute values: offline verification
is an order of magnitude cheaper than a warm read, so checking a
receipt is never the expensive step, and the cold path is dominated by
the upstream fetch, not by signing or persistence.

### 14.2 Memory-benchmark scorecard

`emem-scorecard` loads a LongMemEval-style corpus (Wu et al. 2024)
through the real write API and scores retrieval, test-time learning,
long-range understanding, and conflict resolution from the responder's
own output, embedding the responder's signed receipt in the scorecard
so a run is independently verifiable. The committed SAMPLE run (16
items, lexical fallback retrieval because the offline build ships no
embedder) scores a topline of 0.68; it is labelled
`dataset_provenance: "sample"` and is not a published benchmark
number. The open evaluation work, a full-dataset run with the semantic
read path, is tracked in the roadmap.

### 14.3 The change ensemble on a live sample

The 15 named places used by the site's own demos, run through the
triple-encoder ensemble against the production responder on
2026-07-11: 9 computed with all three encoders, 5 degraded to 2-of-3
with a typed reason, 1 failed at the geocoder. Ensemble change indices
ranged 0.047 to 0.579, and no place cleared the all-legs agreement
rule, the expected null result for stable landmarks compared year over
year. The two highest means came from single encoders firing without
corroboration, which is the case the agreement rule exists to hold
back. What this sample does not show, and a real evaluation still
needs, is a change-rich sample (burn scars, clearings, construction)
scored against ground truth.

### 14.4 Failure modes, typed

Failure surfaces are enumerated closed sets on the wire rather than
prose: absence reasons (`outside_coverage`, `gpu_unavailable`,
`no_auto_materializer_registered`, and kin), ensemble degradation
reasons (`partial_consensus_2_of_3`, `single_vintage`,
`gpu_sidecar_unavailable`), and typed request errors that teach the
accepted vocabulary in the message. A missing value is a signed
absence, never a bare 404, so the failure modes are themselves
testable claims.

---

## 15. Threat model

§16 lists the limits one by one; this section restates them as an
adversary model, since the useful question is always "who is lying,
and what do they gain." The receiver's trust base is small: the
correctness of blake3 and ed25519, the canonical-CBOR discipline of
§5, and the binding of a responder identity to a public key first seen
at contact. Everything else is checkable arithmetic.

**A malicious sender** (an agent handing over a token) gains nothing
from tampering: the identifier is the digest of the bytes, so altered
bytes stop resolving, and the receipt binds `(cell, fact_cid)` under
the responder's signature. What a sender can still do is select: cite
the facts that favour it and omit the rest. The defence is that the
address space is canonical, so the receiver can recall the same cell
itself and see everything the sender left out.

**A malicious responder** can sign a false value; the signature
attests attribution, never truth (§16.1), and the provenance class
plus contradiction scoring are the mitigations, not the signature.
What a responder cannot do is rewrite history undetectably for anyone
who holds a prior receipt: the cited bytes are content-addressed, so a
changed answer is a different identifier. What it *can* still do today
is equivocate, presenting different histories to different clients,
because the receipt does not chain to the transparency log (§8.4) and
the witness set is scaffolding, not a network (§8.5). Split-view
detection is exactly what the log's staged work buys, and until it
lands the honest statement is: one responder, one key, trust anchored
at first contact.

**A network attacker** who can intercept or replay gains nothing
against verification, which is offline and keyless; it can deny
service, and it can matter at first contact, when the responder's key
is learned. Key pinning after first contact, and the operator
attestation that binds the key to a build (§6), are the current
posture; a key-transparency mechanism is future work.

**A malicious writer** on the multi-writer path can attest false facts
under its own key. It cannot write into another attester's namespace
(the capability binding refuses), cannot overwrite (later records
supersede, disagreement is kept and scored), and cannot sign as anyone
but itself. The unauthenticated-write hole was closed in 2026-07: every
memory write requires a valid attester block, so an anonymous caller
can no longer plant a fact that surfaces in another agent's recall.

The one adversary this design does not address is a coercive one that
controls the responder key and every future receipt: against that,
only federation (several independent accounts of the same world,
cross-checked) helps, and that is roadmap, not build (§16).

---

## 16. Honest limits

Collected rather than scattered, because a limit mentioned once in
passing is a limit a reader misses.

### 16.1 What a signature does not mean

The signature proves who attested a record and that the bytes have not
changed. It does not make the value true, correct, current, or complete.
Confidence, uncertainty, and provenance travel with the value precisely
because the signature speaks to none of them.

### 16.2 Identity and tokens

- **Entity mints and links are not signed over their content.** The
  receipt binds the primitive and the cell, not the `entity_cid`, label,
  kind, or aliases. `intent` is a body field, never a preimage segment.
- **`emem:entity:` does not dereference to byte-identical bytes.** The
  identifier covers a narrow identity preimage; label, geometry, and (in
  the external-anchor branch) `cell64` sit outside it, so two nodes may
  hold different bodies under one identifier. There is no entity analogue
  of the fact cell-binding check.
- **No entity merge, and links do not conflict.** One alias maps to a
  growing candidate list with no precedence and no contradiction signal.
  Anyone may link any phrase to any entity: the endpoint takes no
  attester.
- **`entity_resolve` is an exact normalized-string lookup**, not a fuzzy
  or semantic match, and its candidates are unranked despite the tool
  description's wording.
- **`emem:cell:` does not dereference at all.** It is composed and never
  parsed.
- **`bundle_cid` is not a CBOR digest** (§3.5), and the family carries
  three identifier widths: 52 characters for a `fact_cid`, 26 for an
  `entity_cid` and a `bundle_cid`.
- **The responder does not re-verify a digest on read.** A client can
  detect a substituted body by recomputing the CID; the responder does
  not do it for them.

### 16.3 The transparency log

- **Not RFC 6962 conformant.** RFC 6962's tree construction with
  BLAKE3-256 substituted for SHA-256. No CT tooling interoperates.
- **It proves append-only-ness only to a client that pinned an earlier
  STH and returns.** A first-contact client has no baseline.
- **It does not resist split-view equivocation** (§8.3). No gossip
  exists; every artefact is served by the accused party.
- **Witness co-signing is scaffolding**: one witness, stale, no
  allowlist, no independence guarantee, self-submittable (§8.5).
- **The receipt does not chain to the log** (§8.4). You cannot prove a
  fact you were served is in it.
- **It cannot prove completeness.** A responder that never appends logs
  nothing, and the log cannot prove absence or non-omission.

### 16.4 Provenance

- **The class is an editorial per-band declaration**, never checked
  against the fact. `deterministic: true` means the band is *declared*
  recomputable, not that the value *was* recomputed.
- **`bands_cid` attests the table, not the classification's correctness**,
  and the manifest segment is optional, so some receipts are unbound.
- **Per-band, not per-fact**: a `deterministic_index` computed over a
  cloud-contaminated pixel keeps its class.
- **The caution is not universal per fact**: fan-out endpoints hoist band
  metadata to the response root at default verbosity.
- **The ontology is compile-time embedded.** The documented hot-swap does
  not exist.

### 16.5 Substrate and grid

- **Cells are not equal-area.** Square at the equator, taller than wide
  above it: longitude pitch narrows with cos(lat), so a cell at 60° is
  about 9.54 m by 4.77 m. The equal-area hex DGGS is a spec target, not
  this build.
- **Sub-metre imagery.** None. Sentinel-2 at 10 m is the finest pitch
  served.
- **Edge and onboard inference.** All inference is in-tenant. No
  spacecraft-bus encoder firmware ships with the protocol.
- **Federation.** Single primary; replicas are read-only. No
  multi-host clustering, no SOC 2 attestation.
- **Declared is not wired.** 46 declared source schemes is a catalog
  count, not a capability count, and several schemes remain
  declared-but-unwired.
- **JEPA v2 is trained, and does not beat persistence on real data.** This
  entry previously said the artefact was an untrained residual-zero
  identity baseline. That is false and `/v1/capabilities` refutes it:
  `jepa_predict_v2` reports `trained: true`. The deployed artefact carries
  `trained_at 2026-05-28` and 28,328 parameters. It was fitted on 12,000
  wholly synthetic sequences from a seasonality generator and validated on
  2,000 more, also synthetic (`synthetic_fraction_train: 1.0`), where it
  beats a Markov-1 "predict the last lag" baseline by 62.7% on NDVI MSE.
  On the 34 real NDVI pairs held for evaluation it does not beat that
  baseline: MSE 0.021124 against the baseline's 0.019858, a skill score of
  -0.064. The artefact's own `honesty_caveats` state the real corpus is
  too sparse to train end-to-end. Treat `jepa_predict_v2` as a research
  surface, not a forecast.
- **The change-attribution split does not exist.** The ledger ships
  (§10.3): per-term evidence with fact cids under a signed receipt. But
  `emem_diff` and `state_diff` still return raw deltas,
  `triple_consensus`'s agreement gate is uncalibrated with its strongest
  verdict unreachable, and nothing decomposes a delta numerically into
  environment, sensor, geometry, and encoder terms.

### 16.6 Specification

- **"Canonical CBOR" here is not RFC 8949 canonical.** Struct keys are in
  declaration order and are not sorted (§5). A generic canonical encoder
  computes the wrong digest.
- **Conformance means "passes our tests".** Per-vector fixtures are not
  yet extracted.

A request that names a missing surface returns a typed Absence or a
structured error code. No surface returns `verdict=false` for an absent
capability.

---

## 17. Comparison with adjacent work

**Certificate Transparency (RFC 6962).** The direct ancestor of §8, and
the comparison flatters CT. CT has an operating network of independent
logs, monitors, and auditors, plus gossip proposals to close split view.
emem borrows the tree construction, substitutes BLAKE3-256, and has one
stale witness. The mechanism is homage; the ecosystem is absent.

**Content-addressed stores (IPFS, IPLD, Git).** The `cid = hash(bytes)`
idea is theirs, and emem's fact layer is a conventional instance of it.
What differs is that emem addresses a *place and a time* as well as
bytes: `(cell, band, tslot)` is a canonical key that lets two parties who
have never met arrive at the same address for the same observable without
exchanging a CID first. A pure content store cannot answer "what is the
elevation here" because "here" is not a hash.

**Spatial indexes (H3, S2, geohash).** Peers to §4, and H3 is the
declared migration target. cell64 is a packed 64-bit lat/lng
quantisation, which is worse than H3 on equal-area properties and better
on token economy: four bigrams, about four BPE tokens. That trade is only
worth making for a memory whose primary consumer is a language model, and
it is a trade, not a win.

**Agent memory frameworks (MemGPT, Mem0, Zep, CoALA, vector stores).**
The kind taxonomy in the abstract is CoALA's, from the line of work on
agentic memory that Generative Agents opened. The engineering peers,
MemGPT's paged context, Mem0's extraction pipeline, Zep's temporal
knowledge graph, compete on recall quality and latency. The difference
here is verifiability: a memory store returns what it holds and asks to
be believed, while emem returns a receipt the receiver checks without
trusting the store. The cost is real and stated: emem cannot do
similarity over arbitrary prose, which those systems do well;
`find_similar` operates over declared band embeddings, not free text.

**Earth-observation foundation models (TESSERA, Clay, Prithvi-EO,
Galileo, AlphaEarth).** These are the representation layer, not rivals:
emem freezes four of them and signs what they emit (§10). AlphaEarth
Foundations is the strongest statement of the adjacent idea, a global
annual embedding field; like the others it ships unsigned artifacts
with no per-place identity that returns a checkable record and no
receipt. The papers in this family report classification and retrieval
scores; none measures whether an embedding preserves the identity of a
place across years, or decomposes why a readout moved, which is the gap
§1.1 and §10.3 name.

**Provenance and marking standards (C2PA, EU AI Act Article 50).**
Content credentials sign generated and edited *outputs*, and the EU AI
Act now requires machine-readable marking of synthetic media. Both
attest who produced content and when, not whether it is a true account
of the world, and neither covers the measured *input* state an agent
reads before it acts. emem sits on that other side: signed provenance
for measured world-state, with the same offline-check property the
marking standards require of outputs.

**Geospatial data platforms (Earth Engine, Planetary Computer, STAC).**
These are the incumbents for the substrate in §10 and are better at it:
deeper catalogs, real compute, mature tooling. None of them issues a
signed receipt that a third party can check offline, and none gives an
agent a token that another agent can dereference to identical bytes. emem
is not competing on catalog depth; it is adding an identity and trust
layer that the catalogs do not have.

---

## 18. Changes from v1

v1 is archived unedited at [/whitepaper/v1](/whitepaper/v1). It remains
the version cited by the DOI. This section exists so a reader of v1 can
see exactly which of its claims this responder does not honour, rather
than discovering it through a failed signature.

### 18.1 Claims v1 made that were false or became false

| v1 said | The code does |
|---|---|
| §1, §4: "Every CID is `base32_nopad_lower(blake3(canonical_cbor)[..16])`" and `FactCid = ...[..16]` | A `fact_cid` is **not truncated**: it is the full 32-byte blake3 digest, base32-encoded to **52** characters. The `[..16]` rule (26 characters) is what `entity_cid` and `bundle_cid` use. An implementation following v1 computes a 26-character identifier that can never match a real `fact_cid`. This is the most consequential of v1's errors: it breaks content addressing itself. |
| §5.2: the receipt preimage is a `\|`-joined concatenation of `request_id`, `served_at`, `primitive`, `cells`, `fact_cids` | That is the **v0** rule. Every new receipt is signed under **preimage v1**: domain-separated, every segment tagged and length-prefixed (§6.1). v0 is retained for verification only, so pre-cutover receipts still verify. |
| §5.2: "the `as_of` block sits outside the preimage ... does not change the signature math" | `as_of` **is** a tagged segment (`0x04`) and **is** signed (§6.2). |
| §5.2.1: `body_hash = blake3(canonical request body bytes)` | The responder never hashes the request body. `body_hash` is **per-verb** (§6.4). A client implementing v1's rule cannot produce an acceptable signature. |
| §17: "MCP tools are a strict read-only subset of REST; writes go through REST only" | **8 of 107 MCP tools write** (§11.1), including the five memory verbs, `emem_entity`, `emem_entity_link`, and `emem_derive`. |
| §17: 93 documented REST paths under `/v1/*` (96 total), 81 MCP tools (10 core, 71 extended) | **108** under `/v1/*` (**112** total), **91** tools (**14** core, **77** extended). |

### 18.2 Claims in v1's supporting material this document withdraws

These were not in v1's body but circulated in the README, module
headers, page copy, and tool descriptions alongside it. They are listed
because withdrawing a claim quietly is how the original error propagates.
Each has been corrected at its source as of this document's date; the
entries below record what was claimed, not what is still claimed.

- "An RFC 6962 Merkle tree." It is RFC 6962's **construction** with
  BLAKE3-256 (§8.1).
- "A responder cannot tell two clients two different histories." It can
  (§8.3).
- "With witness co-signing", read as an operating property. It is a
  callable endpoint with one stale participant (§8.5).
- "The text to canonical-id resolution decision is itself signed and
  citeable." It is not (§2.4).
- Any token "resolves to the byte-identical signed object", applied
  across the family. True for `emem:fact:`, false for `emem:entity:`,
  vacuous for `emem:cell:` (§3).
- "Four provenance classes." There are **five**; `unclassified` is the
  fail-safe default (§7).
- "Hot-swap the ontology without recompiling." The ontology is compiled
  in (§10.1).
- "`pip install ememdev`", presented as a working install path. Every
  wheel published to PyPI (0.0.9, 0.1.0, 1.0.0) contained no Python
  modules; the roadmap's SDK entry carries the full account and the
  release gates that now prevent a repeat.
- "46 declared sources and 124 wired measurements", read together as one
  capability count. Declared is a catalog count and several schemes remain
  unwired (§16.5); the README now scopes the two numbers apart.
- The homepage FAQ listing three foundation-model embeddings where the
  substrate wires four (Clay was omitted). The page is compiled into the
  binary, so that correction lands with the next server deploy rather
  than with this document.
- "base-1024 bigrams", which appeared in 14 places including the OpenAPI
  `Cell64` schema, where the same sentence went on to call the alphabet
  65,536-entry. The alphabet has **65,536** entries; four base-1024 digits
  would encode 40 bits and could not address a 64-bit cell (§4.1). Corrected
  everywhere as of this document's date.

### 18.3 What is new since v1

The entity registry and the token grammar (§2, §3); tamper-provenance
classes (§7); the transparency log and witness endpoints (§8);
caller-registered derivations (§9); the preimage v0 to v1 cutover (§6.3);
MCP tiering, shapes, and bundles (§11.2); and the code-generated verifier
spec (§6.6).

### 18.4 Why v1 rotted, and what changed

v1's counts were correct when written. They decayed because a number
re-typed into prose has no owner: nothing failed when a tool landed and
the paragraph did not move. Three of its five false claims are of that
kind. The other two, the preimage and the body-hash rules, decayed
because the protocol improved and the prose did not follow.

Two mechanisms now exist that did not. `GET /v1/verifier_spec` is
generated from the signer's compiled constants, so the normative
description of the signing rules cannot drift from the code that signs
(§6.6). And `scripts/sync_counts.py` verifies every count in this
repository against the live responder and the registries, failing rather
than warning.

Neither mechanism protects prose like this sentence. That is why §12
makes the machine-readable spec normative where it and this document
disagree, and why this document tries to state mechanisms rather than
totals wherever a total would do.

---

## 19. Open questions

- **Chain the receipt to the log.** A `fact_cid -> leaf_index` index and
  a log segment in the receipt preimage would close §8.4 and make
  "transparency" mean what it sounds like. This is the single highest-value
  increment on this substrate.
- **The change-attribution split.** The §10.3 ledger ships and persists
  as a derivative fact; the numeric split needs an estimator and a
  calibrated cross-encoder, cross-sensor stability model. The output
  shape is settled; the attribution logic is not.
- **An operating witness network.** Independent witnesses, a published
  trust policy, a refresh cadence, and gossip between clients. Until
  then, §8.5 stands.
- **Sign the entity decision.** An `intent` (or `object_cid`) segment in
  the preimage would make the mint and alias claims in §2.4 true rather
  than aspirational.
- **Per-fact provenance.** The class is per-band (§7.2). Per-fact lineage
  would let a cloud-contaminated index declare itself.
- **Extracted test vectors.** `spec/test_vectors/` would turn §12's
  "passes our tests" into conformance a second implementation can claim.
- **H3 migration.** Equal-area cells under a new mode prefix; receipts
  pin the active manifest CIDs, so historical answers do not drift.
- **Entity reconciliation.** A merge primitive, and a contradiction
  signal for conflicting links (§2.4).
- **Typed identity kinds.** An entity's `kind` is free text (§2.3). A
  small closed set (physical, material, ecological, functional,
  administrative) is the prerequisite for saying what kind of thing
  changed at an address, not only that something did.

---

## References

1. Laurie, B., Langley, A., Kasper, E. *Certificate Transparency.* RFC
   6962, IETF, 2013.
2. Bormann, C., Hoffman, P. *Concise Binary Object Representation
   (CBOR).* RFC 8949, IETF, 2020.
3. O'Connor, J., Aumasson, J.-P., Neves, S., Wilcox-O'Hearn, Z. *BLAKE3:
   one function, fast everywhere.* 2020.
4. Bernstein, D. J., Duif, N., Lange, T., Schwabe, P., Yang, B.-Y.
   *High-speed high-security signatures.* Journal of Cryptographic
   Engineering, 2012. (Ed25519)
5. Josefsson, S., Liusvaara, I. *Edwards-Curve Digital Signature
   Algorithm (EdDSA).* RFC 8032, IETF, 2017.
6. Sumbaly, R., et al. *CVE-2012-2459: Merkle tree duplicate-leaf
   ambiguity.* 2012.
7. Sumers, T., Yao, S., Narasimhan, K., Griffiths, T. *Cognitive
   Architectures for Language Agents (CoALA).* 2023.
8. Uber Technologies. *H3: A Hexagonal Hierarchical Geospatial Indexing
   System.* 2018.
9. Josefsson, S. *The Base16, Base32, and Base64 Data Encodings.* RFC
   4648, IETF, 2006.
10. Foundation for Public Code / Overture Maps Foundation. *GERS: Global
    Entity Reference System.* 2024.
11. Packer, C., et al. *MemGPT: Towards LLMs as Operating Systems.*
    arXiv:2310.08560, 2023.
12. Chhikara, P., et al. *Mem0: Building Production-Ready AI Agents
    with Scalable Long-Term Memory.* arXiv:2504.19413, 2025.
13. Rasmussen, P., et al. *Zep: A Temporal Knowledge Graph Architecture
    for Agent Memory.* arXiv:2501.13956, 2025.
14. Park, J. S., et al. *Generative Agents: Interactive Simulacra of
    Human Behavior.* arXiv:2304.03442, 2023.
15. Wu, D., et al. *LongMemEval: Benchmarking Chat Assistants on
    Long-Term Interactive Memory.* arXiv:2410.10813, 2024.
16. Benet, J. *IPFS: Content Addressed, Versioned, P2P File System.*
    arXiv:1407.3561, 2014.
17. Feng, Z., et al. *TESSERA: Temporal Embeddings of Surface Spectra
    for Earth Representation and Analysis.* arXiv:2506.20380, 2025.
18. Szwarcman, D., et al. *Prithvi-EO-2.0: A Versatile Multi-Temporal
    Foundation Model for Earth Observation Applications.*
    arXiv:2412.02732, 2024.
19. Tseng, G., et al. *Galileo: Learning Global and Local Features of
    Many Remote Sensing Modalities.* arXiv:2502.09356, 2025.
20. Brown, C. F., et al. *AlphaEarth Foundations: An embedding field
    model for accurate and efficient global mapping from sparse label
    data.* arXiv:2507.22291, 2025.
21. Clay Foundation. *Clay Foundation Model, v1.5.*
    https://clay-foundation.github.io/model/, 2024.
22. Coalition for Content Provenance and Authenticity. *C2PA Technical
    Specification.* https://c2pa.org/specifications/, 2024.
23. European Union. *Artificial Intelligence Act, Article 50:
    transparency obligations.* Regulation (EU) 2024/1689, 2024.
24. Healey, S. P., et al. *Mapping forest change using stacked
    generalization: An ensemble approach.* Remote Sensing of
    Environment 204, 717-728, 2018.
25. Pekel, J.-F., Cottam, A., Gorelick, N., Belward, A. S.
    *High-resolution mapping of global surface water and its long-term
    changes.* Nature 540, 418-422, 2016.
26. Google. *S2 Geometry Library.* https://s2geometry.io/, 2017.
