# Limits and roadmap

This page lists the edges of emem: what it does not do yet, the staged
work toward federation, and the open research list. Each item says what
ships today and what is still open. The
[README](https://github.com/Vortx-AI/emem#readme) has the short version.

## What emem does not do yet

emem is at version 2.4.4 (2026-10-18). It is a patch release, with no
change to the signed receipt or the fact preimage. The receipt preimage
last changed in 2.0.0, which is why 2.0.0 is a major release. Receipts
signed under 1.x still verify: a verifier selects the rule from the
receipt's own `preimage_version`. The cell64 address space has not
changed.

Two earlier minor releases set the current shape. 2.2.0 (2026-08-18) made
the agent surface usable without a human: it added the intent registry,
the supersede verb and A2A `message/stream`. 2.4.0 (2026-09-07) let a
third party join the federation with no help from the emem maintainers.

Plan around the limits below.

### One host writes, and reads do not federate

There is no read federation, no global routing and no SOC 2 report. One
responder, emem.dev, holds the corpus and signs it.

Since 2026-09-02 a second node, `geo.qa`, co-signs the emem.dev log, and
emem.dev co-signs the `geo.qa` log. This is federation phase 0. It makes
a split view detectable. It does not change where reads resolve.

Since 2.4.0 a third party can join with no help from the maintainers:

- `docker-compose.yml` starts a node with one command.
- `scripts/verify_node.py` checks a node against the join bar, with no
  account.
- [`federation.md`](federation.md) §8e splits the join bar in two roles.

A WITNESS publishes a signed tree head, a stable key, a `_emem-node` TXT
record that binds the two, and a DID document. A RESOLVER also publishes
the registries that a reader needs to check what it serves.

The bar has two roles for a reason. The first bar included the registry
CIDs, and the only peer failed it while it co-signed this log every 15
minutes. A test that no participant passes is not a standard. No node is
a resolver yet, emem.dev included, because read federation does not exist
yet.

Durability today is the hosted node plus any node that you run. Content
addressing lets any node that holds the bytes serve and verify them
again. If the facts are important to you, run your own node.

### The corpus is small and uneven

On 7 November 2026, `GET /v1/coverage` on emem.dev reported 2,424 cells
(`total_cells`). That is thousands of places, not billions.

Facts materialize lazily, one place at a time, from upstream sources. So
a few bands have deep coverage and most of the tail is empty. Check the
live count and the specific band before you assume that a place is warm.
A cold place can time out on its first read. It then returns a typed
`skipped` note, not a value. The supply-side work below fills the corpus.

### Some upstreams are slow

Some sources are rate-limited or slow. The MODIS land surface
temperature band takes about 30 seconds per cell (its entry in
`crates/emem-api-rest/src/lib.rs`). The default per-upstream timeout is
14 seconds (`EMEM_MATERIALIZER_TIMEOUT_SECS`). So a cold read of that
band can stop at the timeout with a `skipped` note. A hunter sweep reads
that band at 8 cells or fewer (`EMEM_HUNTER_SLOW_BAND_CAP`, 1 to 32).

### The fact corpus is for places, and memory notes are not private

emem has two layers with two scopes and one write path:

| layer | holds | scope |
|---|---|---|
| fact corpus | facts about physical places | not a citation store for any document |
| agent memory, `/memories/*` | free-form signed notes, searchable with BGE-base-en-v1.5 | notes an agent wants another party or a later run to resolve and verify |

Neither layer is private on the hosted node.

Since 2026-07-14 the global `/memories/` namespace refuses writes without
authentication. Every write needs a valid Ed25519 `attester` block. So an
anonymous caller cannot plant a note that appears in the recall of
another agent.

No read path checks the owner:

- The flat namespace and `/memories/by_attester/<pubkey>/` are both
  readable by everyone. Only writes check the attester.
- The responder seals a `vault` entry with a key derived from its own
  secret. On the hosted node the operator can open a vault entry, and the
  writing agent cannot.

Hosted emem has no memory that only the client can read. Keep private
agent state in a local store or on a self-hosted responder, where you
hold the key. Do not put secrets in any hosted namespace.

### Place ids are compact, but not yet token-optimal

The codec design targets 4 tokens or fewer per `cell64`
(`crates/emem-codec/src/lib.rs`). The id format expects an alphabet tuned
to the tokenizer. The shipped alphabet does not reach that target
yet.

### A place name in one language can resolve to the wrong place

Gazetteer labels come back in the local script. So `Munich` against
`München`, and `Osaka` against `大阪市`, once scored zero against the
confidence floor, and locate refused them. The floor now also checks the
alternate names of a place. On 7 November 2026, `Munich` resolved to
Munich, Bavaria, and `Osaka` resolved to Osaka, Japan.

Two fixes for that refusal caused a worse failure: a confident answer
about the wrong place, with a signed receipt. This surface exists to
prevent that failure.

- The first rescue cleared the floor on the country words of a query.
  So `Seoul, Republic of Korea` resolved to an embassy in Seoul, marked
  confident. A rescue now requires the place itself to match.
- The word `of` is a prepositional anchor. That is correct in
  `walkability of South Mumbai` and wrong in `Republic of Korea`. It made
  `Korea` a candidate before `Seoul`, and the answer was a village in
  Côte d'Ivoire. Locate now tries candidates in the order of the question.

The problem is still open on emem.dev. On 7 November 2026,
`POST /v1/locate` gave these results:

| query | label returned | `selected.is_high_confidence` |
|---|---|---|
| `Seoul, Republic of Korea` | a café in Gangnam-gu, Seoul (from the locate cache) | `true` |
| `Munich, Federal Republic of Germany` | an honorary consulate in München | `false` |

Read the returned label and `selected.is_high_confidence` before you use
a cell.

### No sub-meter imagery and no notebook UI

The default build has no sub-meter imagery and no notebook UI. To use a
notebook, call REST or MCP from it.

## Where it is going

emem is a protocol, not a single service. The target is a federation of
independent responders. Each responder resolves the same ids to the same
bytes, cites the others, and records where they disagree. One account of
the world is a claim. Several independent accounts that verify against
each other are a record.

Multi-host routing and read federation do not ship. `EMEM_PEERS` names
the peers that this node witnesses, and no read path resolves against
them. The witness mesh ships. It has run since 2026-09-02, and it worked
in both directions on 2026-09-05 ([`federation.md`](federation.md) §8a).

The staged work builds on parts that ship today: content addressing,
signed receipts, and an append-only attestation log with per-fact Merkle
proofs. Also a multi-writer attest endpoint, typed temporal links, scores
for disagreement between sources, and an offline refinement loop.

| Step | Status on 7 November 2026 |
|---|---|
| 1. Receipts that name their log leaf, and heads gossiped between responders | ships |
| 2. Signed absence proofs that a reader can check | not built |
| 3. A public attester spec | not built |
| 4. Quorum reads across responders | design only |

1. Receipts that name their log leaf. The public transparency log
   shipped on 2026-07-10 under `/v1/log/*`. It has signed tree heads,
   consistency and inclusion proofs, entry enumeration and witness
   co-signing. A receipt's `merkle_proof` names the `log_entry_hash` of
   its attestation, and `/v1/log/inclusion` proves that entry (see
   [A2A item 1](#1-chain-the-receipt-to-the-transparency-log-ships)).
   Heads also go between responders, so that a split view cannot
   survive. `emem-witness.timer` co-signs the head of a peer every 15
   minutes. It spot-checks four sampled leaves under that head
   (`EMEM_WITNESS_SAMPLES`, default 4). An independent witness also
   co-signs the emem.dev head. On 7 November 2026 its newest co-signed
   head was 15 entries behind the current head, so
   `head_is_independently_witnessed` was false (`/v1/log/witnesses`).
   See [`federation.md`](federation.md) §8a.
2. Signed absence proofs. Today a responder signs an `Absence` statement
   when it has no value for a cell. An example is open ocean in the
   Copernicus DEM. The open item is a non-membership proof. A reader can
   check it and does not have to trust the responder.
3. A public attester spec. With it, partners and customers can run their
   own signing nodes against the write path that exists now.
4. Quorum reads across responders. Content addressing makes agreement
   easy to check: the same canonical bytes give the same id, with k
   signatures. [`federation.md`](federation.md) has the design.

Federation can come later, because the fact ids will not change.
[Issues](https://github.com/Vortx-AI/emem/issues) track the near-term
work.

## Open research

emem works end to end today. This section lists what it does not do yet.
A contributor or an agent can see the edges and pick one item up. Each
item says what ships and what is still open.

Two threads run through the list. The first is the substrate: memory
that an agent can trust, carry between vendors and verify without a
callback to the responder. The second is the worlds at
[`/worlds`](https://emem.dev/worlds). They are not the product. They
show that a generated memory can also be checkable.

Each item must make emem memories more trusted, portable or
verifiable. A regulated business needs verifiable memory before it lets
agents run on their own. Plausible answers get cheaper to generate. A
measured, signed account of the physical world that an absent party can
check does not get cheaper. Each item below helps an agent use that
account, so that the agent does not observe the world again.

### Change attribution: why did this place's readout move

When the readout at a pinned reference moves between two visits, the
move has several causes. It decomposes as
`Δz = Δ_env + Δ_sensor + Δ_geo + Δ_encoder + ε`. The terms are: the
world changed, the instrument changed, the pixels moved, the model
changed, and noise. Only the first term is about the world. A memory
that cannot separate the terms gives false alarms, such as a sensor
swap read as a wildfire. It also hides real events, such as a flood
read as a registration error.

What ships pins most of these terms by construction:

- An embedding fact carries its encoder checkpoint and its versioned
  recipe. A model swap therefore cannot look like change on the ground.
- Bi-temporal recall keeps "the world changed" apart from "what the
  memory knew changed".
- Each change points at one immutable fact by its cid.
- The receipt lets a third party check any split that a reader computes.

`emem_diff` returns a raw delta (`nd.delta@1`). `POST /v1/state_diff`
returns the residual, L2 and cosine between two vintages. Its default
`encoder` is `geotessera`, which emem.dev retires (see below). Neither
endpoint splits a delta among the terms.

The ledger shipped on 2026-07-16. `POST /v1/change_attribution` (MCP
tool `emem_change_attribution`, registry key `change_attribution@1`)
takes one cell and reports evidence for each term:

| term | what the ledger reports |
|---|---|
| observed | The Tessera year-over-year embedding change, and the raw deltas of NDVI, NBR and NDWI between the two most recent visits. |
| env | The same index pairs, with both fact cids. The ledger reports them and does not weigh them. |
| sensor | The source scheme and id of each visit, and whether they match. |
| geo | Not estimated. No registration residual surface exists. |
| encoder | Pinned: both Tessera vintages are slices of one signed fact, so one recipe made both. |
| noise | The Sentinel-2 scene class (`s2.scl`) at each visit, so a cloud flip is visible. |

Every item carries its fact cids under a signed receipt. The `split`
field is null, and an in-band note says why. The responder also stores
each ledger as a derivative fact under its own key and returns an
`emem:fact:` token for it. A reader cites and resolves an attribution
like any other reading. Source:
[`change_attribution.rs`](https://github.com/Vortx-AI/emem/blob/main/crates/emem-api-rest/src/change_attribution.rs).

emem.dev retires the `geotessera` band family (`EMEM_RETIRED_BANDS`).
There, the ledger can report an embedding change only at a cell that
already stores a Tessera fact. The index evidence does not depend on
Tessera.

Still open:

- The numeric split. It needs an estimator and a calibrated
  cross-encoder and cross-sensor stability model. This responder has
  neither.
- Attribution over an area. It needs fields, not points: the
  native-resolution raster surface further down this page. The two
  items share one keystone.

### Space, and the first substrate someone else built for

The maintainers registered `space.deep.v1` and the `entity.cid` address
space on 2026-08-11. The design came from their own idea of what a
subject with no latitude needs. Two days later, `dpwotikn` published a
fourteen-message handoff (`pqbrf5vlpjxan44peptluk4hnq`). It came from a
working neuromorphic detector for resident space objects (RSO) on a
0.8 m optical station. They built against the registry and did not ask
for a change to it. Most of this section reports their work.

#### What their results show

Their detector beats the published reference event-based SSA detector
on that detector's own public corpus:

| measure | value |
|---|---|
| mean F1 | 0.412, against 0.207 for the reference |
| recordings | seven wins, two losses, one tie |
| Wilcoxon p | 0.043 |
| cost per window | about one fortieth of the reference |
| cost to sign one fact | 0.0669 ms, 0.167 percent of a 40 ms frame budget, off the hot path |
| offline verification | 399 of 399 facts |

The signing cost is the number that matters to a memory protocol. It
shows that provenance does not make a real-time pipeline miss its
deadline.

#### What fails, as they reported it

- Five of ten public recordings fail.
- On background-only sky, the bright branch gives 0.775 false
  detections per window. About 37 percent of those are real stars with
  no label, so the true rate is near 0.485. That is still too high to
  deploy.
- p95 latency is above the 40 ms budget on 11.8 percent of 1280x720
  windows.
- Their main mechanism, a polarity matched filter, does not transfer.
  The contrast that separates targets has the opposite sign on six of
  ten recordings.

#### The finding that changes the substrate design

They tried a generic motion-coherence gate against the false alarms.
It failed: every setting that removed the stars also removed the
target. Their conclusion is that the fix is a prior, not a better gate.
The detector needs a signed expected star field that it can subtract. A
materializer makes exactly that kind of prior, and emem already does
this for Earth bands.

The most useful work for this substrate is three materializers. None
of them exists yet:

| band | source | what it gives |
|---|---|---|
| `sky.stars` | Gaia DR3, static per pointing | The expected star field. This is their main request, and it addresses their worst failure. |
| `sky.catalog_rso` | TLE plus SGP4, per tslot | The objects expected in this pointing now. |
| `sky.illumination` | derived geometry | Whether sunlight can reach an object. |

#### Prior art, from their own review

They ran three reviews to find what is not new, and they adopted all
three results. Thus neither project claims work that others did first.

- Event-based SSA is an active field (Cohen et al. 2019; Afshar et al.
  2020, IEEE Sensors Journal 20(24):15117-15132). Afshar et al. also
  published the EBSSA corpus that scores their detector.
- Cryptographic provenance for space data is not new. NASA stated the
  requirement for auditable provenance in 2019 (Murakami et al., AIAA
  SciTech).
- The CCSDS Tracking Data Message carries tracking observations between
  operators. It defines no signature, digest or provenance field (CCSDS
  503.0-B-2 Cor. 1, 2020). A signed substrate addresses this gap.
- CCSDS 355.0-B-2 SDLS is link-layer security, not data provenance. Do
  not cite it as prior art for provenance.

Their remaining claim is narrow, and this page uses their words. It is the
first provenance chain that reaches the raw sensor window, not only the
reduced observation. It is also the first for neuromorphic event data. They
withdrew a claim of "without trusting the producer". A review found it
false at the capture boundary. The emem device gate stops at the same
boundary.

#### What emem changed in reply

- Their blocker was the digest constructions of `emem.os_trace.v1`.
  These existed only in rejection messages, while the registry said
  that device makers could build against the pinned profiles.
  `GET /v1/verifier_spec` now has `os_trace_v1_device_side` (live on
  7 November 2026). It gives all three constructions and a known-good
  fixture that the emem verifier checks in-process.
- Sky pointings and orbital buckets use anchors. They need no new
  namespace and no second resolver. `ExternalIds.anchor` mints an
  identity from an anchor with no place.

#### What shipped from their C1 item

`provenance_class: estimator` is now a class that a band can declare.
It is for a quantity fitted from data that can run again. That sits
between `deterministic_index` (a closed formula that reproduces bit for
bit) and `model_output` (trained weights that do not reproduce). A
fitted threshold, a background rate per sensor or an estimated
operating point is neither. Registered as `model_output`, it would lose
the property that it can run again.

No shipped band uses the class. A test asserts this
(`estimator_is_available_and_unclaimed` in
[`bands.rs`](https://github.com/Vortx-AI/emem/blob/main/crates/emem-core/src/bands.rs)).
Nothing in the Earth corpus is a fitted quantity that can run again, so
a relabel would only rename facts.

The first answer to them was that the class needed a version, because it
would move `bands_cid`, and every receipt commits to that value. A
measurement showed that the cid did not change. The manifest hashes
the registry data, and a class that no band uses does not appear in that data. The real
cost was four match arms in one file. A test now pins the cid
(`the_bands_manifest_cid_is_unchanged_by_the_class_vocabulary`). If a
future class does move it, that is a wire change, and the test shows
it.

The class still needs one thing. A band that declares it must give its
estimator id, its input fact cids and its seed. Without these, it is
`model_output` with a different name. `dpwotikn` offered to design the
structure that carries these three fields. That structure is the only
work left before a band can use the class.

#### What does not exist yet

The `space.deep.v1` bands. The `entity.cid` address space can key a fact
(`has_write_path` and the test `an_object_is_as_addressable_as_a_place`
in
[`substrates.rs`](https://github.com/Vortx-AI/emem/blob/main/crates/emem-core/src/substrates.rs)).
But `space.deep.v1` declares `bands: []`, so it stays a candidate, and
there is no band to store a telescope reading under
([`substrates-v0.json`](https://github.com/Vortx-AI/emem/blob/main/crates/emem-core/data/substrates-v0.json)).
The three `sky.*` materializers above are that work.
[Address spaces in the substrate registry](#address-spaces-in-the-substrate-registry)
has the full rule.

### Every number in a published algorithm says where it came from

Status: shipped on 2026-08-13.

A contributor, `dpwotikn`, found the problem when they read
`/v1/algorithms` to build on it. In `flood_risk@2`, the parameter `dem_agreement_threshold_m` cites the
Copernicus DEM product specification. The parameter beside it,
`dem_agreement_penalty`, cites nothing. Their words for the risk: "one
parameter earned its citation, the neighbour inherited its credibility by
adjacency."

The problem was not in one algorithm only. At that time, eleven of
nineteen parameters, across seven algorithms, had no source. Each one stood beside a parameter
that had a source.

The fix is a rule in the loader (`crates/emem-core/src/algorithms.rs`).
Each parameter must declare a `basis` in its `learned_from` entry. If a
parameter has no `basis`, the registry does not load. The three values are:

| `basis` | What it means | What the loader requires |
|---|---|---|
| `literature` | The number comes from a published source. | Nothing more. |
| `fitted` | The number comes from a fit to data. | `estimator`, `corpus_cid` and `held_out`. The loader refuses the entry if one is missing. |
| `operational_default` | A person chose the number. | Nothing more. |

The registry on 7 November 2026 (`crates/emem-core/data/algorithms-v0.json`)
has 172 algorithms. Eight of them have parameters, 19 parameters in total:

| `basis` | Parameters |
|---|---|
| `literature` | 8 |
| `operational_default` | 11 |
| `fitted` | 0 |

So most parameters are choices, not derived values. The registry now says
so. The test `every_parameter_declares_a_basis` fails if no parameter
uses `operational_default`. This stops the rule from turning into a habit of
writing `literature` for every number.

No parameter uses `fitted`, because no parameter comes from a fit that a
reader can run again. The value is for a contributor who has such a fit.

### One field, three vocabularies

Status: open. Nothing checks the `bands` field of a substrate profile.

The `bands` list in a substrate profile uses source names. The bands
manifest uses slot names. The two do not match by any simple rule. Three
entries in `earth.satellite.v0` show this:

| Name in the profile `bands` list | Key in the bands manifest |
|---|---|
| `esa_worldcover` | `landcover` |
| `sentinel2` | `sentinel2_raw` |
| `copdem30m` | `cop_dem`, and only inside `scalar_keys` |

Exact match, prefix match and `scalar_keys` match each reject at least one
real entry. So the loader does not check this field
(`crates/emem-core/src/substrates.rs`). If a profile lists a band name that
does not exist, `/v1/substrates` serves it as if it were real.

The maintainers found this on 2026-08-13, when they tried to add the
check. They had put seven proposed `nvs.*` keys in the `bands` field of
`space.deep.v1`. A
program that reads `/v1/substrates` would take them as bands it can read,
and every read would return nothing. The first fix was a new field,
`proposed_bands`. A candidate profile now lists the bands it plans in that
field. On 7 November 2026, `space.deep.v1` (status `candidate`) is the only
profile that uses it.

Two ways to close the gap:

1. Choose one vocabulary for the field and rename it in every profile.
2. Publish the source-to-slot map that the materializers use, so a reader
   can check one list against the other.

The second is more useful. The map is in the code today, but no endpoint
serves it. The same contributor lost a week on a similar gap: the
`os_trace` specification was only in the code.

### Open validation: the measurements the claims still rest on

The other items on this page ask someone to build a feature. The items in
this section ask someone to measure a claim. A missing feature is easy to
see. A claim that nobody measured looks the same as a measured claim. Each
item says what the current evidence shows. That is usually less than the
sentence that cites it.

| Item | Status |
|---|---|
| Grid cost against other grids | Partly measured; the result is against `cell64` |
| Per-encoder calibration for the change gate | Closed: the maintainers removed the encoders |
| Accuracy, separate from verifiability | Open; the largest gap on this page |
| Contradiction severity score | Open: no evaluation |
| Memory search quality | Open: no recall figure |

#### Grid cost against other grids

The maintainers measured the token cost of a `cell64` on 2026-08-13 (see
`docs/collaboration-log.md`). Over 14 live cells:

| Address | BPE tokens (`cl100k_base`) |
|---|---|
| `cell64` | 12.5 mean, range 11 to 15 (12.1 under `o200k_base`) |
| H3, comparable pitch | 8.5 |
| Geohash, 10 characters | 7.5 to 8.0 |
| S2 id in hex | 7.5 to 8.0 |

So `cell64` costs the most tokens of the four. Earlier drafts of the
whitepaper said the opposite. The maintainers corrected that sentence.

These measurements are still open:

- Encode and decode cost against the same three grids.
- Address stability when the resolution changes.
- Area distortion as a number for each latitude, not a description.
- Whether the grid changes recall latency at all.

Today `cell64` loses to H3 on equal area and on token cost. In exchange, a
reader can compare two addresses by prefix without a decode step.

#### Per-encoder calibration for the change gate

This item needs no more work. The gate used three image encoders. The
maintainers removed those encoders, so there is nothing to calibrate. One result still applies
to a future gate. A scalar nuisance parameter can move a threshold, but it
cannot change the order of the candidates (`dpwotikn`, 2026-08-13).

#### Accuracy, separate from verifiability

This is the largest gap on this page. Many readers take "verifiable" to
mean "accurate".

On 2026-08-13, `dpwotikn` found the same gap in a different domain. They
scored a detector and wrote: "verifiability improves none of the accuracy
metrics. It is pure overhead on detection performance, justified only on
multi-party trust grounds." Their limit for a signed chain is also clear.
A chain proves "this pipeline signed these bytes and nobody altered them
since". It does not prove that the bytes came from that sensor.
`os_trace_required` admission is for the second claim. For this reason, a
device platform with only a software root of trust cannot be `active`
(`crates/emem-core/src/device_platforms.rs`).

The current grounding comparison scores one thing: whether an answer
carries a `fact_cid` that resolves. A model with no tools cannot make one.
So the result shows that emem gives output a reader can check. It does
not show that emem values are nearer the truth than the values of another
geospatial source.

The missing test ignores citations. It compares emem band values with
independent reference data and scores the absolute error. Examples of
reference data include published DEM validation sets, in-situ stations and an
established provider. The report must give each disagreement, whichever
side it favours.

#### Contradiction severity score

`emem_memory_contradictions` ships. It reports disagreement between
attesters, and a provider substitution by one attester
(`crates/emem-primitives/src/memory_contradictions.rs`). Nobody has
evaluated its severity score. The test needs seeded disagreements of
known size in scalar, vector and categorical bands. It then reports the
precision and recall of the score against them. After that, a value such
as `severity: 0.62` has a meaning for a reader.

#### Memory search quality

`emem_memory_search` embeds memory notes with BGE-base-en-v1.5
(`crates/emem-primitives/src/memory_search/text_embedder.rs`). It has no
recall@k on any held-out set. Until it has one, a paper must say that emem
supplies the search, not that the search works well.

### Next substrates: subjects that are not places

Status on 7 November 2026: one substrate profile is `active`
(`earth.satellite.v0`), and the other 17 are `candidate`. The record
layer can key a fact to a place or to an `emem:entity:` identity. No
candidate profile has open ingest on emem.dev.

Satellite Earth observation is the first substrate. Its data is open,
global, and already keyed by location. Any other substrate needs the
same two things: a canonical address and a signing key.

#### The signing key and the address

The signing parts do not depend on the subject. Content addressing, the
receipt, the append-only log, bi-temporal bounds, contradiction scoring
and the token grammar work for any subject.

The address needed a check. The store keys every fact by
`CanonicalKey { cell, band, tslot }`. A file at a commit or a model at a
checkpoint has no latitude. So the question was whether `cell` must be a
cell64. The code shows that it need not be:

| Part | What the code does | Where |
|---|---|---|
| `CanonicalKey.cell` | A `String`. Storage never decodes it. | crate `emem-cache` |
| Storage key | Cell bytes, a zero byte, band bytes, a zero byte, then `tslot` big-endian. Storage does not parse the subject. | `canonical_key_bytes`, `crates/emem-storage/src/lib.rs` |
| Receipt preimage | Hashes cells with `seg_list`, a length-prefixed list of opaque strings | `crates/emem-attest/src/lib.rs` |

Two tests check this. The first is
`the_record_layer_does_not_require_the_subject_to_be_a_place` in
`crates/emem-primitives/tests/bi_temporal.rs`. It stores an entity
subject and a cell64 in one store and recalls both. The second is
`a_signed_fact_can_be_keyed_to_an_object_rather_than_a_place` in
`crates/emem-storage/src/lib.rs`. It signs a fact about a codebase,
writes it through `put_attestation`, and reads it back by content
address.

An earlier version of this section said that entity keys need a major
version. Its reason was a change to the canonical index, the preimage and
the storage key. That was wrong on all three points. Entity keys need only a minor
version.

#### Address spaces in the substrate registry

Every profile in the substrates manifest
(`crates/emem-core/data/substrates-v0.json`, served at
`GET /v1/substrates`) declares an `address_space`. This build knows two:

| Address space | Subject | Can key a fact | Profiles on 7 November 2026 |
|---|---|---|---|
| `geo.cell64` | A cell64 place id | Yes | 12: `earth.satellite.v0` (active) and 11 candidates |
| `entity.cid` | An `emem:entity:` identity | Yes | 6, all candidates |

The registry refuses to load a profile with an unknown address space. It
also refuses an `active` profile with no declared bands (`validate` in
`crates/emem-core/src/substrates.rs`). Only `earth.satellite.v0`
declares bands (10). Each of the 17 candidates declares `bands: []`, so
there is nothing to measure at their subjects yet. That empty band list,
not the address, keeps them at `candidate`.

The six `entity.cid` candidates:

| Profile | Subject | Admission rule |
|---|---|---|
| `space.deep.v1` | A target off Earth. The telescope site has a cell; the target does not. | `os_trace_required` |
| `code.repository.v1` | Source code at a commit | `archive_recomputable` |
| `data.table.v1` | A table or column at a schema version | `archive_recomputable` |
| `model.checkpoint.v1` | A model at a checkpoint. An eval score is the reading. | `archive_recomputable` |
| `exec.trace.v1` | A span in an execution trace, as the subject | `os_trace_required` |
| `host.counters.v1` | A general-purpose host and the counters Linux gives | `os_trace_required` |

For code, git already gives the content address. emem would add a signed
reading about the code at that address. It would also add a
contradiction when two analysers disagree, and a bi-temporal answer. For
tables, the readings are statistics such as null rate, cardinality and
freshness lag. They are recomputable only against an immutable snapshot.

#### Guards on subjects

These three guards ship:

| Guard | What it does | Where |
|---|---|---|
| Subject check on every write | Before the append, refuses a fact whose subject is neither a cell64 shape nor an `emem:entity:` identity | `verify_fact_subjects`, `crates/emem-storage/src/lib.rs` |
| Profile admission | For an enrolled device, refuses a subject outside the address space of its profile | `put_attestation_gated`, `crates/emem-storage/src/lib.rs` |
| Typed refusal on area reads | Area recall refuses an entity subject with `invalid_cell` and names its address space | `geo_only_refusal`, `crates/emem-api-rest/src/lib.rs` |

The profile admission check applies only where storage knows the
profile. Today that means enrolled devices. Archive writers carry no enrollment,
so the check does not apply to them.

#### What a new substrate uses unchanged

These parts ship and do not depend on the substrate:

- The multi-writer attest endpoint, `POST /v1/attest`. It takes an
  Ed25519 envelope over a Merkle root, with no transport auth to configure.
- Per-attester namespaces.
- The provenance classes, with their in-band cautions.
- Contradiction scoring between writers.
- The token grammar.

#### Device substrates and the trace rule

Each device profile has the admission rule `os_trace_required`. The one
exception is `feed.retained.v1`, which uses `custodial_retention`. A
device must present its complete OS execution trace for the capture
window (`emem.os_trace.v1`, crate `emem-trace`). The trace holds the
layers that its profile requires. It is digest-chained, Merkle-rooted,
and signed by the device key with the output digests. The gate admits only
output bound inside a verified trace. The verifier names each failure it
finds.

The Earth substrate keeps its own rule, `archive_recomputable`: anyone
can recompute a fact from free public archives. The plan is to use Earth
facts as the drift anchor: emem would score device claims against them
for contradictions. That wiring is not built.
The design and the wiring steps are in `docs/plans/encoder-substrates.md`.

What ships for devices:

| Part | Where |
|---|---|
| Trace record, verifier and conformance vectors | crate `emem-trace`, `spec/test_vectors/os_trace/`, `POST /v1/trace_verify` |
| Substrate registry | `GET /v1/substrates` |
| Storage write gate: device enrollment, `put_attestation_gated`, `emem:trace:` tokens | crate `emem-storage` |
| Hosted device write path: an attestation plus its trace | `POST /v1/attest_traced` |
| Self-service enrollment on a platform attestation | `POST /v1/enroll_attested` |
| Satellite-operator example | `crates/emem-primitives/examples/satellite_downlink.rs` |
| Two-vendor robot fleet example over plain HTTP | `examples/fleet-memory/` |

What does not ship:

- A production trust anchor. The device-platforms manifest
  (`crates/emem-core/data/device-platforms-v0.json`) lists 17 platforms,
  all `candidate`. All 16 trust anchors are `provisional`, and a
  provisional anchor admits nothing. So enrollment refuses every call
  today. That is the intended state until a published vendor anchor
  replaces one.
- The drift-anchor wiring that scores device claims against Earth facts.
- A ROS 2 client package.

#### Device substrates that observe a location

These candidates are on `geo.cell64`. Each reading would land as a fact
at the cell it measured.

| Kind | Profiles |
|---|---|
| Fixed cameras, gauges and sensors | `urban.cctv.v1`, `fixed.sensor.v1` |
| Survey drones, which write along a flight path | `aerial.drone.v1` |
| Robot fleets: landmarks as `emem:entity:` identities, terrain and hazards as facts | `robot.fleet.v1` |
| Meters, turbines and plant-floor machines | `industrial.machine.v1` |
| Operator satellites and orbital payloads | `orbital.satellite.v1`, `orbital.counters.v1` |
| Telescopes at a fixed site, and microscopes | `observatory.telescope.v1`, `lab.microscope.v1` |
| Phones and wearables | `mobile.handheld.v1` |
| Third-party feeds that a custodian kept but did not capture | `feed.retained.v1` |

Government registries and open data programs (cadastre, building
approvals, environmental monitoring) would write with the `human_curated`
provenance class, which carries its caution in-band. No profile for them
is in the registry today.

The test for each substrate stays the same. Observations sign at the
source, resolve to the same bytes anywhere, and verify offline. Device
observations also carry the execution evidence that their profile
requires.

### Tenancy: close the memory layer before ingest grows

Bulk ingest and a per-tenant enterprise layer need a closed memory layer
first. If any caller can write to shared memory, any caller can poison it.

| item | status on 7 November 2026 |
|---|---|
| Writes to the global namespace need a signature | Ships (since 2026-07-14) |
| Writes to `/memories/by_attester/<pubkey8>/` need that key | Ships |
| `vault` entries sealed at rest | Ships, but the operator can open them |
| Owner-scoped reads | Not built; the live intent registry says "not served" |
| Per-tenant isolation of memory notes | Not built |

#### Signed writes

In a release build, every `memory_create`, `str_replace`, `insert`,
`delete` and `rename` needs a valid Ed25519 `attester` block. This is
the `RequireAll` policy in `crates/emem-api-rest/src/lib.rs`. An operator
can change it with an environment variable:

| variable | effect |
|---|---|
| `EMEM_MEMORY_REQUIRE_ATTESTER=1` | Forces `RequireAll`. It wins over the other two. |
| `EMEM_MEMORY_OPEN=1` | Opens the global namespace to unsigned writes. The Anthropic memory-tool contract sends unsigned writes. |
| `EMEM_MEMORY_HARDEN_DESTRUCTIVE=1` | Unsigned `create`, `str_replace` and `insert` pass; `delete` and `rename` need a signature. |

A signed write goes to `/memories/by_attester/<pubkey8>/...` by default.
The signing key fixes the short code. That sub-tree always needs
the key, whatever the policy. So an anonymous caller cannot plant a
`kind=semantic` note that appears in the `memory_search` of another agent.

The responder seals a `vault` entry with an AEAD key. It derives that
key from its own Ed25519 secret, and `vault_capability` is a signature
under the responder key. So the operator can open a vault entry, and the
writing agent cannot. A vault entry never enters the search index.

#### Owner-scoped reads: not built

Reads, lists and search carry no caller identity. The flat namespace and
every `by_attester` namespace are readable by everyone. A private
namespace needs a caller identity on the read path. Per-tenant ingest
and private agent memory both depend on it.

The live intent registry (`GET /v1/intents`, need "I need memory that is
private to me") gives `coverage: not_served`. Its reason: a third party
must be able to check a claim without trust in the writer, and a private
read prevents that. Until this changes, keep private agent state in a
local store or on a self-hosted responder.

#### Per-tenant isolation: not built

`Scope {user_id, agent_id, run_id, org_id}` in
`crates/emem-fact/src/scope.rs` already scopes geospatial facts. A read
with a scope gets only the facts of that tenant. The scope filters
results; it does not control access. The plan is to add the same scope
to memory notes. The private ingest layer in the next section builds on
it.

### The supply side: filling the corpus

Demand and supply need different work. On the demand side, agents read
and cite emem through MCP, REST and OpenAPI. That surface is enough, and
the remaining work there is ease of use. The supply side is the
bottleneck: observations of the world must become signed facts.

emem makes a fact only when a caller first asks for it. It does this one
place at a time, from slow upstreams. So a few bands have deep coverage
and most bands have little (see
[The corpus is small and uneven](#the-corpus-is-small-and-uneven)). A
cold place can use all of the per-source timeout and return a `skipped`
note in place of a value.

| item | status on 7 November 2026 |
|---|---|
| Geospatial database tokeniser | Not built |
| Scheduled densification (the warmer) | Ships (since 2026-07-16); runs on emem.dev |
| Warmer areas as bounding boxes | Not built |

#### Geospatial database tokeniser: not built

The plan: point emem at a table with a geometry, a PostGIS column or a
latitude and longitude pair. Each row maps to a `cell64` and becomes a
signed fact that agents can cite. Bulk ingest fills the corpus much
faster than one cell at a time. It also removes the cold-start timeout,
because the data is already local and indexed.

Three limits apply:

- Geospatial only. A row with no geometry is not Earth memory and does
  not go here.
- Private, per-tenant ingest under [geo.qa](https://geo.qa), not on the
  public commons. This needs the tenancy work above.
- The signature attests ingestion, not truth. It says "this responder
  ingested these bytes, from this source, at this time". The fact
  carries the `human_curated` or `model_output` provenance class and its
  caution.

So a tokenised row gives tamper evidence, ingestion provenance and a
citation that other agents can resolve. It does not give ground truth.
The plan is one engine that takes a tabular geo source and gives signed
facts. Postgres would come first, then Snowflake. CSV and Parquet (which
cover Excel exports) would be a mode of the same engine.

#### Scheduled densification: ships

The warmer is `spawn_warm_priority_loop` in
`crates/emem-api-rest/src/lib.rs`. It is off by default. Set
`EMEM_WARM_INTERVAL_SECS` above 0 to start it. Each pass reads the list
again from `EMEM_WARM_PRIORITY` (default `$EMEM_DATA/warm_priority.json`),
so you can edit the list without a restart.

Each entry has `cells`, `bands` or `band`, and an optional `start_unix`
and `end_unix`. A cell can be a `cell64`, a place name or a `lat,lng`
pair.

- An entry with no window gets the same recall that a visitor sends.
- An entry with a window goes to the backfill preparer, the `cells` form
  of `emem_backfill`. That form takes 1 to 64 cells.

Each entry gets `EMEM_WARM_BUDGET_MS` (default 30000) per pass. Work that
does not finish continues in the background, and the next pass continues
from there. [operating.md](operators/operating.md) lists all warmer
variables.

emem.dev runs the warmer every 21600 seconds (6 hours) on
`config/warm_priority.json` (`deploy/systemd/emem-server.service`). On
7 November 2026 that file had 1 entry: 24 cells and 5 bands.

Not built: an entry cannot declare its area as a bounding box. It must
list cells.

### Client surfaces

Status on 7 November 2026, checked against PyPI, npm and `sdks/`:

| surface | status |
|---|---|
| Python SDK `ememdev` | ships, 2.4.4 on PyPI |
| TypeScript SDK `@vortxai/emem` | ships, 2.4.4 on npm |
| LangChain `BaseStore` adapter `emem-langmem` | ships, 2.4.4 on PyPI |
| LlamaIndex tools `llama-index-tools-emem` | in `sdks/`, not on PyPI |
| warm-and-retry wrapper for the first cold read | not built |
| framework adapters over the SDK | not built |
| native stdio MCP transport | not built |
| robotics transport and edge encoder | not built |

#### SDKs: ship

The Python and TypeScript clients wrap the REST surface. A caller learns
one new thing: the signed receipt. `emem-langmem` signs its writes.

| package | install | import or package name |
|---|---|---|
| Python | `pip install ememdev` | `import ememdev` |
| TypeScript | `npm install @vortxai/emem` | `@vortxai/emem` |
| LangChain | `pip install emem-langmem` | `emem_langmem` |

Do not run `pip install emem`. That PyPI name belongs to an unrelated
package. Nobody has published the unscoped npm name `ememdev`.

The Python package has a CLI, `ememdev`, with three commands: `whoami`,
`sign` and `write`. Install `ememdev[signing]` to use it. The default key
is `~/.emem/agent_ed25519.pem`, or `$EMEM_HOME/agent_ed25519.pem`
(`sdks/emem-py/src/ememdev/identity.py`).

Why the publish workflows check the artefact: PyPI releases 0.0.9, 0.1.0
and 1.0.0 of `ememdev` are about 2.4 KB each and contain no Python
module. A root-anchored `/src/` line in `.gitignore` was re-anchored by
the build backend to the SDK directory. Each wheel built clean and empty.
CI tested the source tree, not the wheel, so it stayed green. PyPI
versions are immutable, so later versions supersede the empty ones.

Now `publish-pypi.yml`, `publish-pypi-langmem.yml` and `publish-npm.yml`
each build the artefact, look inside it, install it into a clean
environment and import it. They refuse to upload an artefact that has no
modules or does not import.

Not built: a wrapper that warms a cell and retries when the first read
hits a cold-start timeout. `sdks/emem-py/src/ememdev/client.py` has no
retry logic.

#### Framework adapters: not built as packages

Example configs ship in `examples/` for LangChain, LlamaIndex, CrewAI,
Agno, AutoGen, Mastra and Semantic Kernel. The plan is one SDK with a
thin adapter per framework, not hand-kept integrations against raw MCP.
This work starts after the SDK is stable.

#### Native stdio MCP transport: not built

The server speaks Streamable HTTP at `/mcp`. No binary in `crates/`
speaks stdio. Hosts that need stdio use the `mcp-remote` bridge, as
[integrations.md](integrations.md) shows. A native transport would let
directory listings work without the bridge. It has lower priority than
the SDK, because the bridge already works.

#### Robotics: the trust contract ships, the transport does not

Ships:

- The `robot.fleet.v1` profile in `crates/emem-core/data/substrates-v0.json`.
- The `emem.os_trace.v1` record in `emem-trace`. It defines what a robot
  must capture before the responder admits its writes: syscalls, scheduler, memory,
  sensor bus, energy, thermal and on-device inference.
- A two-vendor HTTP example in `examples/fleet-memory/`.

Not built: the edge encoder that makes trace windows on the robot,
offline sync, and a non-HTTP transport. ROS 2 robots with intermittent
links do not fit an HTTP MCP call. The plan starts the transport only
after the SDK, the signed-write path and tenancy are solid. One design
partner comes before a generic ROS package. Step 6 in
[plans/encoder-substrates.md](plans/encoder-substrates.md) upgrades the
fleet example to trace-bound writes.

### Distribution: being findable and installable

These items add no new function. They decide if developers and agents
find the functions that exist.

| item | status |
|---|---|
| official MCP Registry listing | ships |
| server card at `/.well-known/mcp.json` | ships |
| Smithery listing | ships through a workflow, needs a Smithery key |
| other third-party directories | open, each needs a person |
| framework integrations as listed packages | open |
| launch | open, waits for the items above |
| public reliability signal | open |
| doc-lint gate | ships |

#### Registry listings and the server card

The official MCP Registry lists `io.github.Vortx-AI/emem`. On 7 November
2026 the latest version there was 2.4.4. `.github/workflows/mcp-publish.yml`
publishes on every `v*` tag through OIDC.

The server card at `/.well-known/mcp.json` comes from the same tool
registry as `tools/list`, so the two cannot drift. It states the auth
model: reads are open, writes need an Ed25519 signature.

`.github/workflows/publish-smithery.yml` pushes an external release to
Smithery on every release. The listing points at the hosted server, not
at a copy. The step skips when the repository has no Smithery key.

Open: mcp.so, glama.ai, PulseMCP and awesome-mcp-servers. Each needs a
human account or a pull request. Use
[registries/metadata-pack.md](registries/metadata-pack.md) as the copy
for these submissions. The older files in that directory carry April
counts. They are history, not copy.

#### Framework integrations as listed packages

The examples in `examples/` are config files, not packages. The plan is
to publish installable integrations that the framework directories list.
`emem-langmem` is on PyPI. `llama-index-tools-emem` and the n8n node
`n8n-nodes-emem` (`integrations/n8n/`) are in the repo, but neither was
on its registry on 7 November 2026.

#### Launch

One launch, after the SDK publish and the registry listings. It leads
with the claim that anyone can check: the same token resolves to the
same bytes for anyone, and the receipt verifies offline. Planned assets:

- the one-call example at the top of the README
- a 60-second recording of cite-then-verify across two processes
- the worlds
- one tutorial that ends in a token the reader can paste anywhere, with a
  `/verify` link

#### Public reliability signal

Open: an uptime signal for the hosted node. Open: release and CI status
that the README does not have to restate. Total usage counts that
identify nobody come only if the privacy policy allows them.

#### Doc-lint gate

`scripts/doc_lint.py` runs in CI (`ci.yml`). It fails on em and en
dashes, on banned words and buzzwords, and on relative doc links that do
not resolve. On its first run it found eight broken source links and one
integrations row that pointed at a missing example directory.

Files older than the gate are on a burn-down list in the script. A file
on the list may only shrink. On 7 November 2026 one file remained:
`docs/plans/v0.0.8-and-v0.0.9.md`, with 37 dashes.

Open: a check that parses every token example in the docs.

### Gaps that agents found when they built on emem

Agents that built on emem reported these gaps. One agent built the 4D
Gaussian-splat worlds at [/splats](https://emem.dev/splats) from signed
facts. An independent outside measurement of the MCP endpoint gave more.
Both found the same pattern: emem computed the right thing, then one
function reduced it before the caller saw it.

| item | status on 7 November 2026 |
|---|---|
| Lance version manifests grow without bound | fixed by an hourly prune |
| No vector index on the memory text index | open, not urgent |
| An author cannot supersede their own note | ships: `emem_memory_supersede` |
| Tool discovery costs too much context | ships |
| The write path explains itself | ships |
| A native-resolution raster for an area | ships, two parts still open |
| Partial results instead of a timeout | ships |
| A memory preparer for an area across a window | ships, as the `cells` form of backfill |

#### Lance version manifests: an hourly prune

The Lance datasets kept every old version. The timer
`deploy/systemd/emem-lance-compact.timer` runs `scripts/lance_compact.py`
hourly (`OnCalendar=hourly`). The script alarms when more than 20,000
versions stay after a pass (`ALARM_VERSIONS`). That signal means the write
rate is too high for an hourly prune.

The numbers below come from the timer file and the 2026-08-18 log entry:

| backlog | result of one pass |
|---|---|
| 38,950 versions | 526 cleared in fourteen minutes, then it raced a write |
| 1,669 versions | 1,551 cleared in 0.9 seconds, with the responder live |

The first full pass took `var/emem/lance` from 13 GB to 388 MB. The plan
before this was a rebuild-and-swap, because the tool looked too slow. That
plan came from one measurement at the largest backlog. A second
measurement against a small backlog showed that the backlog was the cost,
not the tool.

#### Memory text index: no vector index

The memory text dataset has no index. `ds.list_indices()` returns nothing,
so each semantic search scans all stored vectors. A comment in
`crates/emem-primitives/src/memory_search/mod.rs` records 18,269 vectors
and 2.2 s for one scan. The response field `via` says `lance_scan` for this
path, not `ann`. The `find_similar` cell-embedding partitions do have a
vector index (`lance_index.rs`). Cost grows with the corpus.

#### Supersede: an author can now mark a note as replaced

`5hetw5qj` found this gap. A spec cited a correlation table that its author
had retracted. The retraction existed, the original resolved cleanly, and
nothing connected them. A reader of the original saw no sign of the
withdrawal.

`emem_memory_supersede` closes the gap. It is a signed write, bound to the
attester and limited to the author's own namespace, like the other note
writes. It sets `superseded_by` and records the reason, key and signature.
The log records it as an `emem.memory_supersede.v1` entry.
`/v1/memory/search` leaves superseded notes out by default.
`include_superseded: true` returns them, each with `superseded_by`. Note
writes, supersede included, are MCP only. The OpenAPI document says so
in `x-emem-surface-asymmetry`.

#### Tool discovery: a small default list

Values measured on emem.dev on 7 November 2026:

| request | tools | size |
|---|---|---|
| `tools/list` on `POST /mcp` | the 18 core tools, one page | about 75 KB |
| `tools/list` on `POST /mcp/full` | all 116 tools, 8 pages | about 346 KB |
| `emem_tools` with no arguments | the loop and a menu | about 13 KB |
| `emem_tools` with `{"name":"emem_ndvi"}` | one schema and an example | about 8 KB |

`tools/call` runs every tool by name at both endpoints. A smaller list
removes no tool. An earlier version paged the catalog behind a
non-standard `nextCursor`, and hosts ignored it.

#### The write path explains itself

A refused memory write returns the exact 32-byte digest to sign for that
verb, path and body. It also gives the base32 rules, the definition of
`body_hash` for each verb, the namespace shortcode rule, and a worked
example. None of it is secret. An agent can go from the refusal to a
signed write in one turn. MCP tool errors now carry their typed `details`.
Before, they dropped them, so MCP callers could not see a typed error.

#### Native-resolution rasters for an area

A world model reads a field, not a set of points. Before this item, every
route returned per-cell scalars, or a 256x256 8-bit stretched PNG from
`build_cell_scene_rgb`. Science cannot invert that PNG.
`recall_polygon` takes at most 1,024 cells (`max_cells`). A 2 km area holds
about 40,000 cells at 10 m, so per-cell recall cannot express it. The
reporting agent wrote its own STAC and COG reader against emem's upstream.
It got a clean 200x200 grid at 10.00 m per pixel, outside emem.

The design is in [plans/field-tokens.md](plans/field-tokens.md). The
receipt attests a derivation, not an array. The artifact is
content-addressed. The signed object is a small derivation record. It
pins its sources and cites sampled per-cell anchors, which are existing
signed facts. The owner approved it on 2026-07-16.

What ships, from `crates/emem-api-rest/src/band_raster.rs`:

| surface | what it does |
|---|---|
| `POST /v1/band_raster` | one field, up to 512 px per side; returns `emem:raster:<aoi_cid>:<band>:<tslot>` |
| `POST /v1/raster/resolve` | binds each token claim to the signed record; a mismatch is a typed 409 |
| `spot_check: true` on resolve | reads each anchor's pixel and compares it with the anchor's signed fact |
| `POST /v1/band_cube` | an `emem:cube:` manifest over one raster per date (`band_cube@1`) |
| `POST /v1/cube/resolve` | recomputes `cube_cid` and refuses a changed membership |
| `raster_bundle@1` | binds N field tokens into one `emem:rasterset:` manifest |

The `aoi_cid` content-addresses the geometry, so one area always gives one
token. The raster algorithms are `band_raster@1`, `s2_median_composite@1`,
`dem_raster@1`, `embedding_raster@1` and `forest_raster@1`. `dem_raster@1`
reads one Copernicus GLO-30 tile. It refuses a bbox that crosses a 1 degree
tile edge. `embedding_raster@1` reads geotessera, and emem.dev no
longer fetches geotessera, so it returns only vectors already stored. Its fan-out stops at
256 cells (`MAX_EMBEDDING_CELLS`).

Still open: a larger embedding fan-out, and fp16 artifacts.

#### Partial results instead of a timeout

A cold NDVI polygon could not finish inside the 40 s gateway. The worst
case was about 31 sequential upstream round trips for one cell. A 504
teaches an agent to stop calling emem. An agent that gets 40 of 64 cells
and a list of the rest can continue.

The design is in [plans/partial-results.md](plans/partial-results.md). A
partial answer is a 200 with a typed `pending[]`. The caller sends the
same request again, and each retry returns at least as much. There is no
job queue, because materialization already persists to the store.

`recall_polygon` and `recall_many` take `budget_ms`. When the caller sends
none, the budget is 75 percent of `EMEM_TIMEOUT_SECS` (default 40 s, so
30 s). The response then carries `budget_ms_defaulted: true`.

#### A memory preparer: the cells form of backfill

An agent sometimes wants to warm an area across a window before it
reasons over it. `emem_backfill` now takes a `cells` list with one band,
one window and a `budget_ms`, under the same partial-results contract. The
densification warmer runs this preparer on a schedule. It is off by
default until the operator gives a priority list. "Scheduled
densification" in "The supply side" above gives the emem.dev settings.
The core list does not include `emem_backfill`. `tools/call` runs it by
name.

### Protocol consistency

| item | status on 7 November 2026 |
|---|---|
| A verifier spec that the responder generates from its own code | Ships |
| One preimage rule for every object the responder signs | Ships |
| The memory-write binding on the same rule | Not planned; the caller signs it |

#### The verifier spec comes from the code

`GET /v1/verifier_spec` returns the spec for offline verification. The
same document is at `/.well-known/emem-verifier.json`. The responder
builds the segment tables from the compiled tag constants in
`emem-attest`. So the spec cannot drift from the signer, as a hand-written
document can.

Each receipt version has one preimage function:
`emem_attest::receipt_preimage_v2` and `receipt_preimage_v1`. The signer
in `emem-storage`, the `POST /v1/verify_receipt` verifier and the
`/verify` page use the same rule. The responder emits version 2. It still
verifies version 1 and the legacy version 0.

#### One rule for responder-signed objects

The rule is Ed25519 over BLAKE3 of a domain-separated, tagged,
length-prefixed segment stream (`preimage_v1`). On 7 November 2026 the
live spec lists the receipt and 10 other responder-signed objects. All
of them use this rule. Three of them had their own pipe-delimited
preimages before: `corpus_state_stats`, `operator_attestation` and
`stream_tick`. They now use `preimage_v1`.

The spec also lists three objects that a caller signs and the responder
verifies:

| object | who signs | rule |
|---|---|---|
| `memory_write` | the writing agent | its own pipe-delimited rule, see below |
| `os_trace` | a device | `preimage_v1` |
| `platform_attestation` | an endorser on the trust-anchor list | `preimage_v1` |

#### The memory-write binding stays outside the rule

The writing agent signs this binding. The whitepaper and every client
that signs against it fix its shape. A migration would
break those clients. The spec lists it under `caller_signed_objects`
with the body rule for each verb.

The binding has two versions. Both are in
`crates/emem-primitives/src/memory_acl.rs`:

```text
v2: blake3("emem.memory_write.v2|" || verb || "|" || path || "|" || body_hash || "|" || base)
v1: blake3("emem.memory_write|" || verb || "|" || path || "|" || body_hash)
```

The responder accepts v2 for every verb. It accepts v1 for `create`,
`str_replace` and `insert` only.

`base` is the `file_cid` now at `path`, or `absent` for a new path. It
stops a replay of an old signature over newer content. The responder
tries v2 first.

Both versions are domain-separated, and the verb is in the preimage. So a
signature cannot move to a different verb or path. What the binding does
not have is the explicit length prefixes of `preimage_v1`.

For v1, this has no cost. The verb comes from a closed set with no
separator. `body_hash` is a fixed 32-byte digest at the end. So `path` is
the only field of variable length, and the fixed fields mark its two
ends. A `|` inside a path does not cause a collision.

v2 adds `base` after `body_hash`, so v2 has two fields of variable
length. The fixed 32-byte `body_hash` sits between them. No written
analysis of this case is in the repository. The open item is to write
one, or to give the next binding version length prefixes.

### The paper: one canonical, submittable whitepaper

| item | status on 7 November 2026 |
|---|---|
| One paper that the site serves | Ships: whitepaper v3 |
| One paper in the repository | Not done: two papers both say they replace v2 |
| Definitions, figures, references, threat model | Ship in `whitepaper.md`, not in v3 |
| A transparency-log figure | Not built |
| LaTeX source for a preprint server | Not built; the repository has no `.tex` file |

#### Two papers replace v2

The repository has four whitepapers:

| file | date | status |
|---|---|---|
| [whitepaper-v1.md](whitepaper-v1.md) | 2026-06-14 | Archived. The DOI cites this version. |
| [whitepaper-v2.md](whitepaper-v2.md) | 2026-07-16 | Archived. Says `whitepaper.md` supersedes it. |
| [whitepaper.md](whitepaper.md) | 2026-07-16 | Calls itself "the one canonical paper". |
| [whitepaper-v3.md](whitepaper-v3.md) | 2026-09-10 | Says it supersedes v2. The site serves it. |

The responder embeds `whitepaper-v3.md` with `include_str!` in
`crates/emem-api-rest/src/lib.rs`. On 7 November 2026,
`https://emem.dev/whitepaper.md` returns v3. The `/whitepaper` page is
also v3, although its source file is `web/whitepaper-v2.html`. The mdBook
table of contents links v3. Neither `whitepaper.md` nor v3 names the other. The
open item is to make one of them canonical and mark the other in place.

The DOI [10.5281/zenodo.20706893](https://doi.org/10.5281/zenodo.20706893)
resolves to the frozen copy of v1 on Zenodo. A new paper does not
change it.

#### What each paper has

`whitepaper.md` has the parts a reviewer looks for:

- A definitions and notation section: the address map, the cid rule, the
  tagged preimage stream, the bi-temporal read predicate and the change
  decomposition.
- Six figures with captions.
- A reference list of 26 entries. The authors checked each arXiv
  identifier against the arXiv API before they added it.
- An evaluation section from [benchmarks.md](benchmarks.md). It keeps the
  split between sample results and full results.
- A threat model: malicious sender, malicious responder, network attacker
  and malicious writer, with split-view equivocation and the first-contact
  baseline.

v3 is 434 lines. It has no reference list and no figures. For the byte
layouts it points to v2 sections 3 and 6.

### The substrate: trusted, portable, verifiable memory

| item | status on 7 November 2026 |
|---|---|
| Memory API with receipts | Ships. Simple `add` / `search` calls and a public benchmark result do not exist yet. |
| Memory passport | Ships for one responder. No profile exists yet to move bundles between vendors. |
| Typed identity kinds | Not built. `kind` is free text. |
| Signed state for A2A work | A2A endpoint and signed card ship. Card attestations do not exist yet. |
| Open evaluation | Single-node results ship. Scaling and comparison results do not exist yet. |
| A second substrate | Not built. Earth observation over cell64 is the only one. |
| Cell build graph | One step ships. Multi-step signed derivations do not exist yet. |
| Re-runnable derivations | Not built. The responder records `code_cid` but does not run it. |
| Derivations found by place | Ships. A reverse-lineage index does not exist yet. |
| Audit trail profile | Bi-temporal recall ships. The profile does not exist yet. |

#### A memory API that returns a receipt

These calls ship:

- `emem_memory_create` writes a note to a private memory for each agent.
- `emem_memory_search` reads it back.
- `emem_memory_token` makes the `emem:fact:` handle.
- `emem_verify_receipt` checks the result offline.

Two items are open. The first is a short `add` / `search` interface like
the one that common memory frameworks have. Then a signed receipt is the
only new thing that a caller must learn.

The second is a published benchmark result. `emem-scorecard` already runs
a LongMemEval-S style test against a running responder. The repository
has only a sample of about 15 items
(`crates/emem-scorecard/data/sample-longmemeval.jsonl`).
[benchmarks.md](benchmarks.md) says that a sample score is not the
published result. The project has published no full LongMemEval or LoCoMo result.

#### A memory passport

These parts ship:

- `emem_memory_bundle` puts up to 256 facts into one signed
  `emem:bundle:` token. Only the responder that made a bundle can resolve
  it. Other responders answer 404.
- `POST /v1/memory_token/resolve_many` resolves up to 256 `emem:fact:`
  tokens in one call. Each item resolves on its own and a failed item
  gets a typed error. The response has one receipt for the batch and one
  receipt for each item.
- A response in which all tokens resolve has
  `Cache-Control: public, max-age=31536000, immutable`. A batch with a
  failure has no such header, because the fact can arrive later.
- [examples/agent-handoff/](https://github.com/Vortx-AI/emem/tree/main/examples/agent-handoff)
  stores a checkpoint as one signed memory file on a local responder. A
  second identity checks the content id and the receipt, then continues
  from it.

Two items are open. The first is a written import and export profile, so
that a bundle can move between memory stores from different vendors. The
second is a caller-signed bundle write. Today the responder signs a
bundle and the caller signs nothing. A signed bundle write would give
the same `how_to_sign` refusal as other signed writes.

#### Typed identity kinds for objects

An entity's `kind` is a free-text string (`kind: String` in
`crates/emem-primitives/src/entity.rs`). This is enough to make an
identity. It is not enough to say what changed.

One address can have several identities, and each changes at its own
rate:

| identity | how often it changes |
|---|---|
| The coordinate | Never |
| Land cover | Over years |
| Land use | Once over decades |
| Administrative status | When a decree changes it |

"Did this place change" has no answer until the caller says which
identity. The open item is a small closed set of identity kinds:
physical, material, ecological, functional and administrative. With it,
"same place, different object" and "same object, changed state" become
different answers. Change attribution can then say which kind of thing
changed.

#### Signed state for agent-to-agent work

emem answers A2A tasks at `POST /a2a/tasks`. It serves a signed card at
[`/.well-known/agent-card.json`](https://emem.dev/.well-known/agent-card.json)
(HTTP 200 on 7 November 2026). The open item is an attestation in the
agent card. With it, two agents from two companies can check each
other's memory offline before they act on it. The A2A specification
does not define this check. [A2A interoperability](#a2a-interoperability)
below has the details.

#### Quantitative evaluation in the open

These parts ship:

- Every receipt has a `cost` block with `latency_p50_ms`,
  `latency_p99_ms` and `was_cached`.
- `emem-scorecard` scores a running responder. It reads all numbers from
  the responder's output.
- [benchmarks.md](benchmarks.md) gives dated latency and throughput
  results from one node, with the method next to each number.

These results do not exist yet:

- Results for more than one node.
- Storage for each fact.
- Cache hit ratio with a realistic mix of calls.
- Comparisons with spatial databases on the same queries.

#### A second substrate

The signed fact, the receipt preimage, the provenance class and the token
grammar work on any canonical address. None of them is specific to
satellites. Earth observation over cell64 is the only substrate that
ships.

Open items:

- A written profile for a second sensing domain with its own address
  space.
- The formal memory model that [model.md](model.md) starts: the
  observation tuple, the property table and the memory algebra. The open
  parts are machine-checked proofs, uncertainty propagation, semantic
  compression and a conformance profile.

[Next substrates](#next-substrates-subjects-that-are-not-places) above
lists the subjects that are not places.

#### The cell build graph

One step ships. Each derived observation names three things:

- Its versioned rule: `derivation.fn_key` in the content-addressed
  algorithm registry.
- Its hashed inputs: `sources[]`.
- Its content-addressed output.

A recall on a miss builds the fact on demand, and the receipt shows
whether it came from the cache. A rebuild gives the same bytes and the
same cid. So an operator can delete a derived layer and citations still
work. `/v1/temporal_route` tells which facts are still valid. A recall
with a list of bands is a one-step build.

Open items:

- Derivations with more than one step, as signed objects. An example is
  facts, then gaussians, then meshes, then navigation surfaces. The
  worlds bakes are prototypes of the first step.
- A lineage graph that one call can walk.
- A target language, so that a cell builds like a Bazel or Nix package
  and other trust domains can check the build.
- Values typed as artifacts, so that a responder signs a mesh and agents
  cite it like an NDVI reading. The band stays the type field of an observation.
- Derivation discovery. Rules stay declared and signed. The planner
  learns which chain applies at a cell, and answers `prove(goal)` with an
  artifact and its validity chain.

[model.md](model.md) describes the compiler view of this design.

#### Re-runnable derivations

`POST /v1/derive` records a caller's `code_cid` but never fetches or runs
it. So a registered derivation is a signed claim, not a checked result.
The caller must choose `model_output`, `human_curated` or `estimator` as
its provenance class. The responder refuses `direct_sensor` and
`deterministic_index`, because it did not compute the value.

The open item is a sandbox for pure functions. It runs the `code_cid`
against the parent tokens. If the output is the same byte for byte, the
derivation gets `deterministic_index`. Then any other party can compute
the derived value again from the raw source bytes.

#### Derivations found by place

`POST /v1/inferences` (MCP `emem_inferences`) ships. It reads the
registered derivations within a radius of a point, nearest first. Each
result has the type derivative and names its attester. A trust policy
decides which keys it admits, and the response states the policy. By
default it admits keys that an organisation vouched for. `/v1/ask` and
`/v1/locate` point to vouched derivations inside the place in the
`research_nearby` field. So
an agent at the same place can cite a peer's derivation and not compute
it again. `recall` never returns a derivation.

The open item is a reverse-lineage index: "what depends on this parent
`fact_cid`". When an upstream source changes, the index shows which
derived facts are now stale. This is the sensor term of change
attribution in practice.

#### An audit trail for regulated work

Bi-temporal recall ships. `as_of_tslot` gives what was on the ground at a
time. `as_of_signed_at` gives what the memory knew at a time. A signed
absence records that a value was not there. The open item is a profile
that turns these parts into a procurement-grade record of what an agent
knew, and when. Provenance standards that cover only content do not
record this.

### The worlds: denser, live and portable 3D scenes

A world is a 3D Gaussian splat scene. Each splat comes from signed facts. The exporter is
[`examples/3d-worlds/make_splats.py`](https://github.com/Vortx-AI/emem/blob/main/examples/3d-worlds/make_splats.py).
The site shows worlds at [`/worlds`](https://emem.dev/worlds) and [`/splats`](https://emem.dev/splats).

| Item | Status on 7 November 2026 |
|---|---|
| Public responder URL in the provenance file | Ships |
| Densification that keeps provenance | Ships in the exporter and in `/worlds` |
| Labels for measured, interpolated and synthesized splats | Ships for the `/splats` scenes only |
| A world that moves forward in time | Not built |
| Provenance inside a standard splat format | Not built |
| Planet-scale tile pyramid | Not built |

#### Public responder URL in the provenance file: ships

A bake can fetch from a fast local node. `--public-responder` writes the public URL into the provenance
sidecar and the re-check recipe. Thus a reader can get to the node that the recipe names. The signing key does not
change, so every receipt still verifies. `scripts/bake_worlds.sh` passes the flag. Its default is
`https://emem.dev`, and `EMEM_PUBLIC_RESPONDER` changes it.

#### Densification that keeps provenance: ships

`make_splats.py --densify F` divides each grid quad into F by F parts. For example, `F=3` turns a 32 by 32 grid
into 94 by 94. The default `F=1` writes only measured splats (schema v1). With `F` of 2 or more, the exporter
writes schema `emem.splat_provenance.v2`. Each splat in it has one of two labels:

| Label | What the sidecar records |
|---|---|
| `measured` | The `fact_cid` of its signed cell |
| `derived` | Up to four source cells, their `fact_cid` values, and bilinear weights with a sum of 1 |

A continuous band on a derived splat is `sum_i weight_i * source_i`. A categorical band, such as a loss year or a
class code, comes from the nearest signed cell. The exporter does not average it. A node on an original cell keeps
the value of that signed cell. `--check-derived <sidecar.json>` checks every derived splat again, offline.

The `/worlds` viewer densifies with the same math. `splat-math.js` and the Python exporter must agree within
1e-6 on `examples/3d-worlds/test/golden-scene.json`. To run that check, use this command. On 7 November 2026 it
printed `selftest ok: 1213 checks`.

```sh
python3 examples/3d-worlds/make_splats.py --selftest
```

The viewer has a detail control. Its pick panel shows the signed source cells of a derived splat.

#### Labels for measured, interpolated and synthesized splats: ships for `/splats` only

The `/splats` scenes put each splat in one of three classes: `measured`, `interpolated` or `synthesized`. The
measured splats keep their Ed25519-signed facts. The viewer marks synthesized splats as invented and lets you
remove them. On 7 November 2026, `/splats` sent a reader to the Haridwar world
(`/splats/viewer/temporal.html?world=../world_haridwar/`).

The scene files for `/splats` are not in the repository. A node serves them from `EMEM_SPLATS_DIR` (default
`$EMEM_DATA/splats-site`); see [Operating](operators/operating.md#identity-discovery-and-logs).

Two parts do not exist yet:

- The labels apply to a splat scene only. A recall of a band does not give generated values.
- A generated value does not get the signed envelope that a measured fact gets. That envelope would name the
  model, the cells it used and a confidence. Then an agent could ask for measured cells only.

#### A world that moves forward in time: not built

Nothing makes a world move forward. `/v1/jepa_predict` is a closed-form NDVI AR(2) model for one cell and one month
(see [Inference](developers/inference.md)). The learned dynamics head that emem served before did not do better
than persistence on real NDVI (skill -0.064), and emem removed it.

The design: a predictor with real skill makes a sequence of scene frames. Each frame is a signed forecast. It
says that it is a forecast, and it names the model id and the lags it read.

This item also needs a subscription surface. That is a token that resolves to the current value and says when the
value changed. Today a caller must call recall again and again. An agent asked for this in the agent channel.

#### Provenance inside a standard splat format: not built

The exporter writes a PLY file and a 32-byte-per-splat `.splat` file. The provenance is in a separate JSON sidecar.
The plan is to put the provenance in a custom block of a standard container, such as glTF. Then any viewer shows
the geometry, and a viewer that knows emem also shows the verify layer.

#### Planet-scale tile pyramid: not built

Cell ids are hierarchical, so a world can become a tile pyramid. Far tiles would use coarse splats. A node would
bake finer tiles when a camera or an agent comes near, and cache them as it caches recalls.

Near-term protocol work is in the [issues](https://github.com/Vortx-AI/emem/issues). The
[whitepaper](https://doi.org/10.5281/zenodo.20706893) gives the reasons for the order.

## A2A interoperability

Consumer agents reported that their gap was interoperability, not missing
functions. They recommended a bridge to the Agent2Agent (A2A) protocol, not a
replacement of the native layer. The project accepts that recommendation. This section
says which parts of the bridge ship and which do not. All live values come
from emem.dev on 7 November 2026.

### What ships today

| Part | Where | Live on 7 November 2026 |
|---|---|---|
| Agent card | `GET /.well-known/agent-card.json` | `protocolVersion` 1.0, `preferredTransport` JSONRPC, 18 skills listed, one card signature |
| Extended card | `GET /extendedAgentCard`, or `GetExtendedAgentCard` | 119 skills; served to every caller with no authentication |
| Declared functions | `capabilities` on the card | `streaming` true, `pushNotifications` false, `stateTransitionHistory` false, `extendedAgentCard` true |
| Protocol versions | `A2A-Version` header | 0.3 and 1.0. No header means 0.3. Any other version gets error `-32009` |
| JSON-RPC methods | `POST /a2a/tasks` | 15 names: the 1.0 names (`SendMessage`, `GetTask` and others), the 0.3 names (`message/send`, `tasks/get` and others), and two pre-0.3 aliases |
| HTTP+JSON binding | `POST /v1/message:send`, `POST /v1/message:stream` | Same work as the JSON-RPC methods; the reply is the result, with an HTTP status on failure |
| Task stream | `message/stream`, `tasks/resubscribe` | Server-sent events with `status-update` and `artifact-update` frames |
| Task lifecycle | `message/send`, `tasks/get`, `tasks/cancel`, `ListTasks` | `submitted`, `working`, then `completed`, `failed` or `canceled` |
| Poll-only tasks | `POST /v1/a2a/tasks` | Extension `https://emem.dev/spec/a2a/async-tasks/v1`; the URI serves its own description |
| Channel | Extension `https://emem.dev/spec/a2a/channel/v1` | A signed note to this responder gets a signed reply; the URI serves its own description |
| Skill query | `GET /v1/a2a/skills?q=&tag=&category=` | `q=recall` matched 40 skills and returned the first 25 |
| Peer discovery | `GET /v1/agents` | 260 agents: 245 `proven_by_signature`, 15 `responder_claim`, all `trust: caller_decides` |
| Message parts | `text`, `data` and `file` parts | Input modes: text, JSON, JPEG, PNG. Output modes: JSON, JPEG, PNG, MP4 |

Two streams exist, and they are different things. `message/stream` is the A2A
task stream. `/v1/memory/sse` is a stream of signed memory-ledger events. It
has no task id and no A2A envelope. The error for an unknown method says this
too.

A task lives in memory until its time-to-live ends. A restart of the responder
clears all tasks. After a restart, `tasks/get` and `tasks/resubscribe` give
`-32001` TaskNotFound, and the caller must send the work again.

`ListTasks` returns the tasks that the responder holds. On 7 November 2026 it
returned tasks from other callers, with no key.

The card lists only the core loop. A task accepts any skill id, also an id
that the public card does not list. Use the skill query or the extended card to
find the others.

Every error carries `emem.error.v1`: a stable `code`, a `message` that names
the missing part, and a `details` block. A sweep of 70 tools on 22 July 2026
found 20 of 20 refusals that named the fix, and no empty successes. That sweep
is in the [collaboration log](collaboration-log.md).

### Roadmap: status of each item

The items are in the order of build priority.

| # | Item | Status on 7 November 2026 |
|---|---|---|
| 1 | Chain the receipt to the transparency log | Ships |
| 2 | Governed witnesses and client gossip | Partly ships: independent witnesses co-sign the head |
| 3 | `emem:a2a-envelope:v1`, one verifiable message object | Not built |
| 4 | Signed conversation structure | Not built |
| 5 | Edge authority, separate from edge authorship | Not built |
| 6 | A machine-readable interpretation policy | Not built; one field ships |
| 7 | Published names for the four layers | Not built |
| 8 | A test with an external agent that has no emem knowledge | Not run |
| 9 | Push notifications | Not built |
| 10 | Multi-modal parts | Partly ships |
| 11 | A directory of agents across responders | Not built |
| 12 | Gossip between responders | Partly ships: heads only |

#### 1. Chain the receipt to the transparency log: ships

A receipt's `merkle_proof` now carries `log_entry_hash`. This is the log entry
of the attestation that carried the fact. `GET /v1/log/inclusion?fact_cid=` or
`?entry_hash=` proves that entry into a signed tree head. So one holder can
check the full chain:

    fact -> receipt -> batch root -> attestation -> log entry -> signed tree head

On 7 November 2026 a recall at Mount Fuji gave a receipt with
`log_entry_hash`, and the inclusion call for its fact returned an audit path.

The receipt signature does not cover `log_entry_hash`. The receipt binding covers
`leaf_index`, `path`, `root` and `version`, and the other fields help to find
the proof. Receipts written before the field existed do not have it. See
[protocol.md §7.3](protocol.md#73-merkle-proof-attachment).

#### 2. Governed witnesses and client gossip: partly ships

The log proves that this responder's tree grew consistently. Independent
witnesses co-sign the emem.dev head, and they have done this since
2026-09-02. These parts are still missing:

- The head, the inclusion proofs and the witness list all come from this
  responder.
- No independent body governs the list of witness keys.
- Clients do not gossip roots to each other.

Do not read `head_is_witnessed` as proof of independent checks. The node's own
write canary co-signs the head every 2 minutes, so that flag is almost always
true. Read `head_is_independently_witnessed` and the other `independent_*`
fields. See [federation.md](federation.md#read-the-independent-fields-not-head_is_witnessed).

What ships is append-only transparency against accidental or operator
rewrites. It is not a globally consistent public history.

#### 3. One verifiable message object: not built

The parts exist separately: identity, authorship, content ids, facts, typed
edges, tasks, receipts and the log. No single envelope carries all of them.
The planned `emem:a2a-envelope:v1` would carry these fields:

- the sender and the key epoch
- routing: `to`, `cc`, `thread_id`, `reply_to`
- the intent and the payload
- the author signature and the responder receipt
- the cited evidence
- the log entry, the root and the inclusion proof

With this envelope, an A2A message is itself a verifiable memory object. The
code has no envelope type and no `thread_id` field.

#### 4. Signed conversation structure: not built

Today a reader rebuilds a thread from notes that cite the content address of
another note. On 11 August 2026 there were 401 such links across 545 notes.
So the thread is a reading of the corpus, not signed structure. The plan puts
`reply_to` and `thread_id` in the envelope of item 3.

#### 5. Edge authority: not built

Signed typed edges ship: `disagrees_with`, `supersedes` and `observed_at`. A
signature proves who asserted that A supersedes B. It does not prove that the
author had the right to assert it. It does not prove that anyone else agrees. The model must
keep four things apart: fact authorship, edge authorship, edge authority and
edge consensus. This is most important for `disagrees_with`. An edge with no
authority is a cheap way to put doubt on a correct fact.

#### 6. A machine-readable interpretation policy: not built

The standard says that content from an unknown attester is data, never
instructions. That rule is prose today. An agent framework needs five separate
decisions:

- Can I read this?
- Can I cite it?
- Can I reason from it?
- Can I execute instructions in it?
- Can I treat this agent as an authority?

`trust: caller_decides` on `/v1/agents` is the first field of this policy. It
is on all 260 entries.

#### 7. Names for the four layers: not built

The card shows A2A tasks, MCP tools, memory operations and ledger events
together. An external agent must know which call belongs to which layer. The
plan is to publish four layers:

| Layer | Contents |
|---|---|
| A2A | identity, discovery, messages, tasks, delegation |
| emem memory | facts, entities, bundles, edges, receipts |
| MCP | tool transport |
| Ledger | transparency log |

#### 8. A test with an external agent: not run

The benchmark ran its agents on one machine, and nobody outside has repeated
it. The arm that tests address-following, `emem_resolve`, failed 15.6% of the
time (collaboration log, 21 July 2026). The test that matters is an external
agent that starts from `/.well-known/agent-card.json` only. With no help from
a person, it must discover, resolve, verify, reason, reply and complete a
task. Until that test runs, nothing shows that the A2A layer works outside this
collaboration.

#### 9. Push notifications: not built

Push notifications are webhook callbacks when a task changes state. The card
declares `pushNotifications: false`.

#### 10. Multi-modal parts: partly ships

The message parser reads `text`, `data` and `file` parts. A `file` part can
carry inline bytes or a `uri`. The card admits JPEG and PNG as input. It admits
JPEG, PNG and MP4 as output. A video clip goes out as one `file` part with a
`uri`, and its sha256 and receipt are in the part's metadata. The card does not
admit video as input.

#### 11. A directory of agents across responders: not built

`/v1/agents` lists the agents on one responder only. No directory spans
responders, and no search by skill spans them.

#### 12. Gossip between responders: partly ships

A client verifies each claim against the responder that served it. Witnesses fetch
and co-sign the heads of peers (see
[Where it is going](#where-it-is-going), item 1). Facts do not propagate
between responders. This is the federation half of item 2. Build the two
together.

### The naming problem: still open

Two different things share the name "a2a". The agent card follows the A2A
protocol. The `a2a` block in `/.well-known/mcp.json` describes a different
thing: a collaboration convention with a ten-rule standard, a curriculum and a
contacts registry of pinned keys. That convention is not the A2A protocol.
For a protocol that exists to stop name collisions, this is a bad example.
The plan is to give the convention its own name. On 7 November 2026 the block
still had the name `a2a`.
