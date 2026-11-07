# Spatial memory traces: what `/v1/ask` gives a model

*Status: a research and design record. The maintainers wrote and measured the
design on 13 September 2026 (commit d2f13f1). Part of it ships. We checked the
status table and the measurements marked "7 November 2026" live on emem.dev on
that day. Everything else on this page records the September design and its
reasoning.*

## Status on 7 November 2026

| Item | State | Evidence on 7 November 2026 |
| --- | --- | --- |
| `spatial_trace` in every `/v1/ask` answer, schema `emem.spatial_trace.v1` | Ships | REST answer for Trafalgar Square: 19 recalled points and 4 camera points in 5 layers |
| Point fields `t` (tslot), `conf`, `class`, `f` | Ships | All 19 recalled points carry them; the 4 `ground` camera points have no `t`, `conf` or `f` |
| `ranges` from the band registry | Ships | `indices.ndvi` gives `[-1.0, 1.0]` |
| Cap of 48 points, question-matched bands first | Ships | `SPLAT_CAP` and the `topic_matched` sort in `spatial_trace()`, `crates/emem-api-rest/src/lib.rs` |
| One point per family before the rest of the cap | Does not ship | The sort uses `topic_matched` only |
| `splat` event in the stream (`Accept: text/event-stream`) | Ships | One `emem.spatial_trace_event.v1` event at the `recalled` stage |
| `GET /v1/state/<cid>` | Ships | 200; every stage has `verifiable_today: true` |
| MCP `structuredContent` for `emem_ask` | Ships | Keys `answer`, `cell`, `question`, `schema`, `spatial_trace` |
| A2A answer as an artifact on a task | Ships | `kind: task`, 1 artifact, `TASK_STATE_COMPLETED` |
| `scripts/protocol_conformance.py` | Ships | Ran; 1 check failed (see the last section) |
| MCP `resource_link` for large parts | Does not ship | The MCP answer has one `text` block and no link |
| Neighbourhood read (9 cells, stored facts only) | Does not ship | The trace has one `cell` and one `at` |
| Schema `emem.spatial_trace.v2` (cell and fact indices) | Does not ship | No code emits it |
| Prose (`_means`) moved out of each answer | Does not ship | `reasoning._means` and `spatial_trace._means` are still in each answer |
| Per-point contribution to the answer | Does not ship | No algorithm reports it |

## The goal

A model asks one question. In the same answer, it gets three things:

- The evidence as small records it can plot. Each record gives place, band,
  value, unit, time, kind of claim, and the fact it rests on.
- Addresses that resolve. Each stage of the answer has an `emem:state:` token.
  The token resolves to the bytes it commits to. A second ask can skip a stage
  that did not change, and a third party can check a stage that did.
- Typed structure. MCP `structuredContent` carries the trace as fields, so a
  model does not parse a JSON string.

The September design found the byte budget for this inside material that no
model asked for.

## The answer size on 13 September 2026

The question was "how busy is it right now?" at Trafalgar Square, London.

| Channel | Bytes | `reasoning` | Stages |
| --- | --- | --- | --- |
| REST | 99,321 | full | 4 |
| MCP | 22,684 | cut to 679 bytes | 4 |
| A2A | 106,575 | full | in text |

The largest fields of a second REST answer (99,908 bytes):

| Field | Bytes | Share |
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

One reading cost 83 bytes, for example
`{"age_s": 1924904, "band": "weather.temperature_2m", "unit": "degC", "value": 16.2}`.
Twelve readings were 1.6% of the answer. The `reasoning` block was 22% and held
no readings. Citations for algorithms that did not run were another 20.6%.

The record names three wastes:

1. Each grounded fact cid was in the answer three times: in `fact_cids`, in each
   step's `new_fact_cids`, and in each state's `derived_from`. With 33 cids of
   52 characters, one copy was about 5 KB.
2. `reasoning` repeated the same two sentences once per stage, 1,404 bytes in
   total.
3. `algorithms_for_question` carried 20,561 bytes of citations for algorithms
   that did not run.

Over MCP, `emem_ask` returned one `text` block with JSON in a string. It had no
`structuredContent` and no `resource_link`, although the protocol version that
emem served supported both.

## The same question on 7 November 2026

The answer grew because the recall stage now grounds many more facts at this
cell.

| Measure | 7 November 2026 |
| --- | --- |
| REST answer | 975,392 bytes |
| Fact cids in `fact_cids` | 3,102, all grounded at the `recalled` stage |
| `reasoning` | 429,764 bytes (44.1%) |
| `facts_summary` | 176,235 bytes (18.1%) |
| `receipt` | 175,558 bytes (18.0%), of which `receipt.fact_cids` is 173,712 |
| `fact_cids` | 173,712 bytes (17.8%) |
| `algorithms_for_question` | 12,908 bytes (1.3%) |
| `spatial_trace` | 4,596 bytes (0.5%) |
| MCP `emem_ask` result | 186,614 bytes |
| A2A `message/send` result | 253,399 bytes |

The first waste is larger now. The cid list is in `fact_cids`, in
`receipt.fact_cids`, in `reasoning.steps` (178,888 bytes) and in
`reasoning.states` (249,915 bytes).

The MCP result keeps the whole receipt (175,556 bytes). To fit, it sets seven
fields to `null` in the text block, `spatial_trace` among them, and cuts
`live_perception`. The `_emem_truncation.omitted_fields` list names each one
with a stub. Its `fetch` block gives the REST call that returns the full
answer. The trace itself still arrives in `structuredContent` (3,824 bytes).

## The trace that ships

`spatial_trace` is in every `/v1/ask` answer. This is the Trafalgar Square
trace on 7 November 2026, with the point lists shortened:

```json
{
  "schema": "emem.spatial_trace.v1",
  "cell": "defi.zb64a.cAzU.zfa27",
  "at": [51.5084502, -0.1284499],
  "stage": "recalled",
  "layers": [
    { "layer": "surface", "points": [
      { "band": "indices.urban_canopy_index", "value": 0.07209548749348182,
        "age_s": 440286, "t": 20757, "conf": 0.949999988079071,
        "class": "deterministic_index", "f": 71 } ] }
  ],
  "absent": [],
  "ranges": { "indices.ndvi": [-1.0, 1.0] },
  "counts": { "present": 19, "absent": 0, "points_shown": 19, "absent_shown": 0 },
  "truncated": false
}
```

A point has these fields:

| Field | Meaning |
| --- | --- |
| `band`, `value`, `unit` | The reading. A vector longer than 16 values becomes its dimension and L2 norm. |
| `age_s` | How old the reading is at answer time. |
| `t` | The tslot. This is the time the reading is about, not its age. |
| `conf` | The confidence this responder gives its own reading. |
| `class` | The provenance class from the band registry. |
| `f` | The index of the fact in the answer's `fact_cids`. |

The layers come from the band registry's `family` field. A new band goes into a
layer with no edit to the projection.

| Layer | What it holds | Families |
| --- | --- | --- |
| `surface` | What the ground is | optical, radar, terrain, soil, vegetation, land cover, water, vision |
| `built` | What stands on the ground | human |
| `now` | What happens there now | climate |
| `embedding` | Vectors for similarity | foundation, encoding |
| `other` | Bands with no family match | none |
| `ground` | What a ground camera saw | added from `live_perception` after the other layers |

The Trafalgar Square trace had `surface` 10, `built` 5, `now` 3, `other` 1 and
`ground` 4 points. A missing layer means the question did not reach that kind
of evidence. A band that emem looked for and did not find goes in `absent`.
These are two different claims, and a model can read the difference.

The cap keeps 48 points and 48 absences. The sort puts question-matched bands
(`topic_matched`) first and is stable, so two same asks give the same
trace. The old plan also kept one point per family before the rest of the
cap. That step does not ship. The server computes `truncated` from the counts before the cap. A count taken
from a list after the cap cannot show that the cap removed anything. That fault
shipped twice in one week in September.

## Design rules the record keeps

- A trace is a projection of signed evidence. It is not a picture of the world.
  Each point is a reading at an address.
- Absence ships as data beside presence, so a reader sees coverage and does not
  guess it.
- A stage address stays the same for the same inputs and changes when the facts
  under it change. This makes it safe to skip a stage.
- Read only stored facts. Never fetch new data only to fill a picture.
- Contribution (which evidence moved the answer) stays out until an algorithm
  reports per-input sensitivity. An invented weight would be the most damaging
  number in the answer, because a reader most wants to believe it.

## What did not ship, and why the record proposed it

### The neighbourhood read

The v1 trace has one cell. On 13 September 2026, one live answer had 31 band
observations and no observation had its own cell. So 31 points drew 31 dots at
one coordinate.

`/v1/locate` returns the centre cell and eight neighbours. Three neighbours
already held signed `indices.ndvi` and `weather.temperature_2m` facts. The
record measured two ways to read them:

| Read | Cells x bands | Time | Result |
| --- | --- | --- | --- |
| `recall_many`, with materialization | 9 x 4 | 14.19 s | 0 usable points |
| Parallel recall, stored bands only | 9 x 2 | 0.02 s | 40 points at 9 positions |

An ask took about 4 s at that time, so the second read costs almost nothing.
On the server, the stored-only read is `lookup_canonical_many` for the
(cell, band) keys, then `get_facts_many` on the cids that come back. Both are
methods in `crates/emem-storage/src/lib.rs`. A key with no stored fact returns
nothing, which is how emem already says "looked, not there".

`/v1/coverage_matrix` does not do this job. It lists which bands exist, with no
values, and it was 50,332 bytes for nine cells.

### Schema v2

The proposed v2 shape puts each cell64 (21 characters) and each cid (52
characters) in the answer once, and points refer to them by index:

```text
{ "schema": "emem.spatial_trace.v2",
  "frame": { "centre": [51.5084, -0.1284], "cells": 9, "tslot": 496497 },
  "points": [ { "c": 0, "band": "weather.temperature_2m",
                "value": 16.2, "unit": "degC", "conf": 0.85,
                "tslot": 496497, "age_s": 1924904,
                "class": "direct_sensor", "f": 7 } ],
  "cells": ["defi.zb64a.cAzU.zfa27", "..."],
  "absent": [ { "c": 3, "band": "hansen.loss_year" } ],
  "ranges": { "weather.temperature_2m": [-60, 60] } }
```

`c` is an index into `cells`. `f` is an index into the answer's `fact_cids`.
The v1 trace already has `f`, `t`, `conf` and `ranges`. It does not have `c`,
`cells` or `frame`.

The proposed order was: the stored-only neighbourhood read with a deadline
inside the `recalled` stage, then the v2 shape. If the read misses its
deadline, the trace ships with the centre cell only.

### MCP `resource_link` and prose out of the answer

The record proposed that MCP send the trace as typed output. It also
proposed links for `algorithms_for_question`, the full `reasoning` and
`live_perception.temporal_context`, so a model fetches them only when it needs
them. The typed output ships. The links do not.

The record also proposed that the `_means`, `_why_not` and `does_not_cover` sentences
move to the schema, which a model fetches once. `_means` sentences are still
in each answer, for example in `reasoning` and in `spatial_trace`.

## The name

The owner chose the name `spatial_trace` before anything went live. "Splat" is
a renderer word for a shape that a renderer draws, but the reader here is a
model. The stream event still uses `stage: "splat"`. "Synthetic satellite
image" was also a candidate. The owner rejected it because emem invents no
data here. Each point is a signed measurement with a provenance class.

## One answer on three protocols

On 13 September 2026, before the fix, the same question gave three
different answers:

| Channel | Bytes | Shape |
| --- | --- | --- |
| REST | 99,735 | full answer |
| MCP | 22,770 | ten fields set to `null` |
| A2A | 107,039 | a message with a 98 KB data part and no artifacts |

In emem, `null` means "there is no observation". So a field set to `null` to
fit a wire budget said something false about the world. Every key was present,
so nobody saw the fault. The fix in September dropped fields by meaning
before the size cut. It also added `_projection`, which names each dropped
field and where to get it.

On 7 November 2026 the MCP text block again has fields set to `null`
(`_projection` among them), because the 175,556-byte receipt does not fit. The
difference from September is that `_emem_truncation.omitted_fields` names each
dropped field. A reader must check that list before it reads a `null` as "no
observation".

A2A moved to artifacts on a completed task. The A2A specification says results
should be artifacts and messages should not deliver task outputs. The
asynchronous path already did this. On 7 November 2026, `message/send` returns
`kind: task` with one artifact and state `TASK_STATE_COMPLETED`.

## Check conformance

`scripts/protocol_conformance.py` checks a live responder against the MCP and
A2A specifications and the published directory rules. It needs the Python
package `jsonschema`.

```sh
python3 scripts/protocol_conformance.py --origin https://emem.dev
```

It checks these rules:

- The MCP transport header, `server/discover`, and the declared `tools` support.
- For each core tool: name length, title, and a read-only or destructive
  annotation.
- Each declared `outputSchema` is valid JSON Schema, and the returned
  `structuredContent` validates against it.
- The `search` and `fetch` connector pair.
- The A2A agent card fields, the task shape, artifacts, and the task state
  name.
- A public privacy policy.

| Exit code | Meaning |
| --- | --- |
| 0 | All checks pass |
| 1 | A check failed |
| 2 | The responder did not answer, so the script asserts nothing |

On 7 November 2026 the script exited with 1. One check failed: "a capped
search still says how much there was". It expects the first `search` result
title to contain "signed facts at". The live title starts "66 signed
indices.urban_canopy_index, ...". All other checks passed, A2A included.
