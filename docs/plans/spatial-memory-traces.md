# Spatial memory traces: what `/v1/ask` delivers to a model

*Status: research + design. Every number in section 1 is measured live against
emem.dev on 2026-09-13 (d2f13f1). Section 3 onward is proposal.*

## The deliverable, first

A model asks one question and receives, inside the same wire budget it already
pays:

- **The evidence as primitives it can draw.** One per reading: where (emem's own
  cell address and a coordinate), what (band, value, unit), when (age), what
  kind of claim it is (provenance class), which stage introduced it, and the
  fact it rests on. ~120 bytes each. Forty of them is 4.8 KB.
- **Addresses that resolve.** Each stage carries an `emem:state:` token that
  dereferences to the bytes it commits to, so a second ask skips what did not
  change and a third party can check what did.
- **Typed structure, not a JSON string.** `structuredContent` for the splat,
  `resource_link` for anything heavy, so a model reads fields instead of parsing
  a blob, and fetches the bulk only if it wants it.

That is the sentence that matters, and none of it is rhetoric: the pieces are
measured below, and the budget for them already exists inside what we currently
spend on material no model asked for.

## 1. Where the budget goes today, measured

One question (`Trafalgar Square, London`, "how busy is it right now?"):

| channel | bytes | reasoning | states |
| --- | --- | --- | --- |
| REST | 99,321 | yes | 4 |
| MCP | 22,684 | slimmed to 679 B | 4 |
| A2A | 106,575 | yes | in text |

REST envelope, 99,908 bytes:

| field | bytes | share |
| --- | --- | --- |
| `reasoning` | 22,066 | 22.1% |
| `algorithms_for_question` | 20,561 | 20.6% |
| `live_perception` | 15,977 | 16.0% |
| `facts_summary` | 9,120 | 9.1% |
| `receipt` | 8,273 | 8.3% |
| `fact_cids` | 6,608 | 6.6% |
| `algorithm_outcomes_summary` | 4,083 | 4.1% |
| `freshness` | 2,463 | 2.5% |
| `topic_routing` | 2,403 | 2.4% |
| `band_observations_summary` | 1,573 | 1.6% |
| `answer` | 1,519 | 1.5% |

**The ratio this document exists for.** A reading costs 83 bytes:
`{"age_s": 1924904, "band": "weather.temperature_2m", "unit": "degC", "value":
16.2}`. Twelve of them are 1.6% of the envelope. The block describing how we
found them is 22%, and contains none of them. Reference material for recipes
that never ran is another 20.6%.

Three structural wastes, each measured:

1. Grounded fact cids are carried three times: envelope `fact_cids`, per-step
   `new_fact_cids`, per-state `derived_from`. At 33 cids of 52 characters, ~5 KB
   per copy.
2. Repeated explanatory prose inside `reasoning`: 1,404 bytes of the same two
   sentences, once per stage.
3. `algorithms_for_question`: 20,561 bytes of citations for algorithms that did
   not run on this question.

## 2. What the transport already offers and we do not use

Over MCP `emem_ask` returns exactly one `text` item containing JSON as a string.
No `structuredContent`. No `resource_link`. Both are in the protocol version we
serve. A model therefore parses a blob, and pays for every byte of it whether it
wanted that section or not.

## 3. `emem.spatial_trace.v1`

One primitive per piece of evidence:

    { "cell": "defi.zb64a.cAzU.zfa27",
      "at": [51.5084, -0.1284],
      "band": "weather.temperature_2m",
      "value": 16.2, "unit": "degC",
      "age_s": 1924904,
      "class": "direct_sensor",
      "stage": "recalled",
      "fact": "7662nrfj..." }

Absence is a primitive too, because a picture of only what was found asserts a
coverage nobody measured:

    { "cell": "...", "band": "hansen.loss_year", "absent": true }

And the stage skeleton, unchanged from what ships today: `stage`, `at_ms`,
`grounded_total`, `state`.

A model can plot these, colour by class or age, show found against absent, and
cite any point by its fact. Two asks diff by address: same stage token, same
inputs, skip it.

## 4. Build order

1. **`GET /v1/state/<cid>`.** The addresses we already emit say
   `verifiable_today: false` on every response. The way to stop publishing that
   sentence is to make it untrue, not to delete it. Order, and it is not ours to
   shortcut: canonicalisation spec, published test vector, then the route. A
   fetchable record under an unspecified encoding turns "I cannot check this"
   into "I checked it and it failed".

2. **The splat projection**, beside `reasoning`, with a hard byte budget and a
   `truncated` computed from the pre-cap count. (Twice this week a count taken
   from an already-capped list could not witness its own capping.)

3. **`structuredContent` + `resource_link` over MCP.** Splat as typed output;
   `algorithms_for_question`, full `reasoning` and `live_perception.temporal_context`
   as links fetched on demand. This is the budget fix and the model-native fix in
   one move.

4. **Prose out of the per-response envelope.** The `_means`, `_why_not` and
   `does_not_cover` sentences are schema documentation charged to every answer.
   They belong in the schema a model fetches once, leaving the response carrying
   values, addresses and typed flags.

5. **Contribution, only when computable.** Which evidence moved the answer is
   the most valuable attribute a splat could carry, and we cannot derive it
   today. It stays absent until an algorithm reports per-input sensitivity. An
   invented weight would be the most damaging number in the payload, because it
   is the one a reader most wants to believe.

## 5. The line we hold

A splat is a projection of signed evidence, not a rendering of the world: the
points are readings at addresses. Absence ships as data beside presence, so a
consumer can see coverage rather than infer it. And a stage address is stable
for the same inputs and moves when the facts under it move, which is the
property that makes skipping safe.

## 6. What the first cut missed, measured against itself

`emem.spatial_trace.v1` shipped the evidence as primitives, and then the same
instrument that justified it showed it is not yet worth the name.

**It is geometrically degenerate.** A band observation carries no cell of its
own: measured on a live answer, 31 observations, `distinct cell fields: {None}`.
Every point therefore inherits the answer's single coordinate, so a splat of 31
points draws 31 coincident dots. A picture with no spatial extent is a list with
extra steps.

**It drops fields the evidence already carries.** Of 31 observations: `tslot`
31/31, `confidence` 31/31 (0.85, 0.80, 0.75 on the first three),
`derivation_fn_key` 31/31. Confidence is the attribute a model weighs most
directly, and the first cut left it on the floor.

**It cannot be normalised.** Bands declare `value_range` in the registry. Without
it a consumer cannot colour-map or compare two bands, and we do not send it.

**Its truncation is arbitrary.** The cap keeps the first 48 in recall order, not
the 48 that carry the most information. The count is honest; the selection is
not considered.

**It is not a stream.** `Accept: text/event-stream` emits stages as they
complete. Points ground during `recalled` and could be emitted as they land; the
splat exists only in the final envelope.

## 7. The neighbourhood is already there, and it is nearly free

`/v1/locate` returns nine neighbourhood cells. Sampled neighbours already hold
signed facts: `indices.ndvi` and `weather.temperature_2m` came back from three
of them with no materialisation.

Measured cost of reading them:

| read | cells x bands | time | result |
| --- | --- | --- | --- |
| `recall_many`, materialising | 9 x 4 | 14.19 s | 0 usable, fetched what was missing |
| parallel recall, already-stored bands only | 9 x 2 | **0.02 s** | 40 points at 9 positions |

Twenty milliseconds against an ask that already takes about four seconds. The
difference between the two rows is the whole design rule: **read what is stored,
never fetch to fill a picture.** A splat that triggers materialisation turns a
question into a bill.

Server-side the non-materialising path is `lookup_canonical_many` for the
(cell, band) keys followed by `get_facts_many` on the cids that came back. Keys
with nothing stored return nothing, which is the honest shape: absence is
already how this protocol says "looked, not there".

`/v1/coverage_matrix` is not the instrument for this. It answers which bands
exist as a catalogue, 50,332 bytes over nine cells, and carries no values.

## 8. v2, and what it is worth

    { "schema": "emem.spatial_trace.v2",
      "frame": { "centre": [51.5084, -0.1284], "cells": 9, "tslot": 496497 },
      "points": [ { "c": 0,            // index into `cells`, not a repeated cell64
                    "band": "weather.temperature_2m",
                    "value": 16.2, "unit": "degC",
                    "conf": 0.85,      // what the responder thinks of its own reading
                    "tslot": 496497,   // when the measurement refers to
                    "age_s": 1924904,  // how stale it is at answer time
                    "class": "direct_sensor",
                    "f": 7 } ],        // index into this envelope's fact_cids
      "cells": ["defi.zb64a.cAzU.zfa27", "..."],
      "absent": [ { "c": 3, "band": "hansen.loss_year" } ],
      "ranges": { "weather.temperature_2m": [-60, 60] } }

Both indices exist for the same reason: a cell64 is 21 characters and a cid is
52, and neither should be repeated once per point when the envelope can carry
each one once and point at it.

What that buys a model, in one call and inside the existing budget: a field it
can plot rather than a list it must join; a time axis (`tslot`) separate from
staleness (`age_s`); a weight (`conf`) it can reason with; a normalisation
(`ranges`) so two bands are comparable; and a citation per point that resolves
to signed bytes.

Build order, revised by what the measurements say:

1. **Neighbourhood read, non-materialising**, bounded by a deadline, inside the
   `recalled` stage. If the read misses its deadline the splat still ships with
   the centre cell, because a late picture is worse than a smaller one.
2. **v2 shape**: `conf`, `tslot`, cell indices, `ranges` for the bands present.
3. **Importance-ordered truncation**: question-matched bands first
   (`topic_matched` is already on every observation), then one per family, then
   the rest. The cap stays; what survives it stops being an accident.
4. **Stream the points.** They ground during `recalled`; emitting them as they
   land is what makes this a reasoning *stream* rather than a reasoning summary.
5. **`GET /v1/state/<cid>`** and MCP `structuredContent`/`resource_link`, as in
   section 4, unchanged.

## 9. Built, and what the name settled

Shipped as `emem.spatial_trace.v1`. The name matters and was changed on the
owner's call before anything went live: "splat" is a renderer's word for a
primitive a renderer draws, and the consumer here is the model. "Synthetic
satellite image" was considered and rejected: nothing here is synthesised,
every point is a signed measurement with a provenance class, and borrowing the
word would invite exactly the misreading this protocol exists to prevent.

Points are grouped into layers taken from the band registry's own `family`, so
a band added to the registry lands in a layer without anyone editing the
projection: `surface` is what the ground is, `built` is what stands on it,
`now` is what is happening there, `embedding` is the vectors that answer
similarity, and `ground` is what a camera saw. A layer missing from the list
means the question never reached that kind of evidence; a band looked for and
not found is in `absent`. Those are different claims, and a model can read the
difference, which is what makes a place with cameras reason differently from a
mid-Pacific cell without being told so in prose.

Each point carries `value`, `unit`, `age_s` (how stale), `t` (the tslot, what
moment it is about), `conf` (what this responder thinks of its own reading),
`class` (what kind of claim it is) and `f`, an index into the answer's
`fact_cids`. `ranges` carries the registry's declared range for the bands
present, so two bands are comparable without fetching the catalogue.
Question-matched readings are offered to the cap first, so what survives
truncation stops being an accident.

## 10. One answer, three protocols, measured

The drift on 2026-09-13, before the fix: REST 99,735 bytes, MCP 22,770 with ten
fields NULL, A2A 107,039 as a message carrying a 98 KB data part and no
artifacts.

The MCP nulling was the sharpest fault in the surface. `null` is this
protocol's word for "there is no observation", so a field nulled to fit a wire
budget said something false about the world, and every key was present, which
is why it went unnoticed. The fix is to convert by MEANING before a budget can
convert by size: an agent gets the facts, the trace, the receipt and the
counts; it does not get a second prose rendering, a freshness table repeating
an age every point already carries, or 20,561 bytes of citations for algorithms
that did not run on that question. `_projection` names what was left out and
where it lives.

A2A moved to artifacts on a completed task, because the spec is explicit
("Results SHOULD BE returned using Artifacts… Messages SHOULD NOT be used to
deliver task outputs") and our own async path already did it, so a synchronous
caller was receiving a shape an asynchronous caller was not.

## 11. Conformance, as a gate rather than a claim

`scripts/protocol_conformance.py --origin <node>` checks the rules against a
live responder: the MCP transport header and tools capability; every tool's
name length, title and read-only/destructive annotation (the Claude directory's
two deciding criteria, with a public privacy policy as the other); that every
declared `outputSchema` is a valid Draft 2020-12 schema and that returned
`structuredContent` VALIDATES against it; the A2A agent card's required fields,
task shape, artifacts and proto-enum task state.

Exit codes follow this repo's convention: 0 conforms, 1 a rule is violated, 2
the responder did not answer (waived: it says nothing about the code), 3 our
own side could not run the check.
