# Machines that ask emem where they are

*We ran every call on this page against `https://emem.dev` on 6 November
2026. The `production truth` workflow runs `scripts/example_check.py` every
hour, and that script runs these calls again. If one stops working, the
workflow fails.*

A robot knows its own coordinates. It does not know what is true at those
coordinates. Examples are the slope it will grade, the wind it will spray into,
and whether the street ahead is clear. It also does not know how the crop
changed since the last pass. That is a memory problem, and emem is for memory
problems.

Two properties matter more to a machine than to a person:

- Every answer carries a signed receipt. A second machine can check the claim
  of the first machine and does not have to trust it.
- Every place has one address. Two machines that resolve "this field"
  separately get the same cell64. They do not get two descriptions that only a
  human can match.

## What emem does not do

emem does not drive anything, and it has no control loop. It answers questions
about places and signs the answers. A call is a fetch, not a control tick. A
warm recall takes milliseconds. A cold recall that must reach an upstream
source can take seconds. Do not put emem inside a safety loop. If a machine
cannot continue without an answer, give it its own fallback.

emem does not cover every place. The corpus holds thousands of places, not
billions. If you ask for a cell that emem has not seen, it fetches what it can.
The response tells you what it could not fetch.

---

## A street robot and an autonomous vehicle

Both machines have the same question at a junction. What is in front of me now,
and is that a measurement or a guess?

```bash
curl -s -X POST https://emem.dev/v1/perception/at \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb64a.cEjo.zfa47","question":"what is on this street?","stages":["detect"],"detect_from_clip":true}'
```

That cell is in London. The important field is `detect_from_clip`, and it is
not the default. The two modes give different evidence:

| mode | `counted_from.source` | `counted_from.tamper_evidence` | can someone recompute the count later? |
|---|---|---|---|
| default | `live_frame` | `none` | No. The frame bytes are gone after the read. |
| `detect_from_clip: true` | `retained_clip` | `recomputable_from_source` | Yes. `counted_from` gives `clip_s3_key`, `frame_index` and `clip_sha256`. |

The receipt commits to the stored clip. Anyone who has the clip can run the
detector again and get your number. A machine that must defend a decision later
needs this second mode.

On 6 November 2026, the call above took 1.2 s and counted 3 cars and 7 people.
The perception routes have a 120 s time limit (`EMEM_PERCEPTION_TIMEOUT_S`,
clamped to 40..600). All other routes have a 40 s limit. Past the limit, the
server returns a typed 504. The limit is high because clip detection runs on a
shared GPU, and a busy GPU can make a call slow.

The response names the camera that it used and the distance to that camera. It
also says why the server chose that camera. A count from a camera 300 m away is
a different claim from a count at your junction. Two more fields help you here:

- `camera_requested`: the camera that you named, if any.
- `camera_substituted`: set when the camera that you named was not available
  and the server used a different one. The server does not hide the change.

### `counts` never contains a zero

Code that reads `counts` often gets this wrong. emem never publishes a zero.
A zero would say that the street is empty. The truth can be that the camera saw
nothing useful. An agent must tell these four states apart:

| what you get | what it means |
|---|---|
| `counts: {"car": 11, ...}` | The detector looked and found these. |
| `counts: {}` | The detector looked and found nothing. This is the empty-street case. |
| `counts: null` with an `unobservable` block | No observation. The frame did not show the street. The block says why, and if a retry can help. |
| `counts: null` with `counts_at_camera` | The detector counted, but the camera is too far from your point. The count does not apply to your point. |

An agent that waits for `{"car": 0}` waits forever.

### Is 45 people a lot?

A count alone cannot answer that question. Most machines need the answer, not
the count. The response has a `temporal_context` block next to `counts`. It
compares the new count with past counts from the same camera.

This excerpt comes from the London call above on 29 September 2026:

```json
{
  "temporal_context": {
    "algorithm": "percentile_band@1",
    "basis": "hour_of_day",
    "comparison_to_prefer": "hour_of_day",
    "counted": {"people": 3, "vehicles": 4},
    "counted_higher_than": "47 of 133 previous readings at this camera at this hour of day, over the last 30 days",
    "percentile": 0.387,
    "percentile_ci": [0.309, 0.472],
    "bounded_claim": {"at_confidence": "95%", "says": "below this camera's median"},
    "people": {
      "count": 3, "percentile": 0.549, "percentile_ci": [0.464, 0.631],
      "verdict": "about usual",
      "usual_range": {"p25": 2.0, "median": 3.0, "p75": 5.0},
      "n": 133, "basis": "hour_of_day"
    },
    "provenance_class": "deterministic_index"
  }
}
```

Read `basis` before you read the verdict. `hour_of_day` compares this hour with
the same hour on other days. That is the comparison that you want.

The response also has an `all_hours` comparison under `comparisons`, with a
larger sample. `comparison_to_prefer` is `hour_of_day` or `null`. It is never
`all_hours`. Traffic changes a lot through the day. A quiet reading at 3 a.m.
looks low against all hours, but the street is not unusually empty. On 6
November 2026, the same call returned `comparison_to_prefer: null`, with 142
past readings for this hour.

Use these fields to judge how strong the verdict is:

| field | what it tells you |
|---|---|
| `n` | The number of past readings that the comparison uses. |
| `percentile_ci` | The interval that this sample supports. |
| `verdict_supported_at` | Present when the interval crosses a tier boundary. Then the verdict holds only for the direction. |
| `bounded_claim` | The strongest claim that the full interval supports. |
| `counted_higher_than` | The exact rank. It needs no interval. |
| `detector_fn_id` | The detector. The baseline uses only readings from this detector, because a different model gives a different measurement. |

The response states two limits:

- `not_a_claim_about` says that this is one camera against its own past. It is
  not this street against nearby streets. Two cameras 250 m apart differ by
  about as much as two cameras 5 km apart.
- The band is a percentile of stored history. It is not a mean and standard
  deviation. Counts cannot go below zero. So a low threshold at two standard
  deviations is often negative, and no count can cross it. Then "unusually
  quiet" can never occur, and every response says normal. A percentile does not
  have this problem. To compare the two methods, call `emem_explain_algorithm`
  on `anomaly_zscore@1`.

The arithmetic is a `deterministic_index`, not a model output. The `recompute`
field gives the query that reproduces it from the signed observations. emem
signs the observations. It does not sign this arithmetic, because you can do the
arithmetic yourself.

## A laser leveller

A leveller grades a field to a target slope. The current slope decides how
much soil must move, and in which direction.

```bash
curl -s -X POST https://emem.dev/v1/terrain \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb4e3.vopU.fEyO"}'
```

The response has `centre_elevation_m`, a `slope` block, `ruggedness` and
`topo_position`. Each block gives the cell size and the fact ids that it used.
On 6 November 2026, this cell had a slope of 7.0 degrees (12.3 percent),
computed with the Horn 1981 method.

The `honest_note` field says which neighbourhood emem sampled, and at what
step. A slope has no meaning without its baseline. The default is a 3x3 Cop-DEM
neighbourhood at about 29 m pitch (`step_cells: 3`). This matches the 30 m
source. A 3x3 neighbourhood at 90 m gives a different number. A machine that
mixes the two grades to the wrong plane.

The DEM has no bathymetry. For an ocean cell, the result is inconclusive. It is
not a flat value.

## A sprayer

Wind decides if a pass can happen. Spray drift is the main risk.

```bash
curl -s -X POST https://emem.dev/v1/weather \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb4e3.vopU.fEyO"}'
```

The route returns four bands:

- `weather.wind_speed_10m`
- `weather.temperature_2m`
- `weather.precipitation_mm`
- `weather.relative_humidity_2m`

The values come from an Open-Meteo and MET Norway forecast blend. The route has
no band filter, so read the band that you need from `bands`. The provenance
class is `model_output`, not a measurement.

Each reading has `captured_at` and `age_s`. A wind speed is a decision input
only while it is fresh. The newest value that emem holds is not always a
current value. Check `age_s` before you act, so that the sprayer does not use a
value from yesterday afternoon. On 6 November 2026, the wind speed at this cell
was 2.6 m/s with `age_s: 1986`.

Soil is the other half of the decision. The soil route answers for the same
cell:

```bash
curl -s -X POST https://emem.dev/v1/soil \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb4e3.vopU.fEyO"}'
```

At this cell, on 6 November 2026, all six SoilGrids bands came back as signed
absences (`kind: absence`, `value: null`). An absence is an answer. emem asked
the source, and the source had no value for this point. Do not retry an
absence. A read that timed out is different. It appears in `materialize_notes`
with `retryable: true`. A retry some seconds later usually gets a warm answer.

## A harvester

Readiness is a shape over time, not one reading. One NDVI value cannot tell you
if a crop is growing or senescing. Two months of values can.

```bash
curl -s -X POST https://emem.dev/v1/trajectory \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb493.xuqA.zcb5f","band":"indices.ndvi","window":[20325,20690]}'
```

The window is a pair of Unix epoch-day slots. On 6 November 2026, the `series`
had 25 points. Each point has its own `fact_cid` and `token`. Every point links
back to the signed observation that it came from. A second machine can check
any point again.

Observations are sparse and have irregular spacing. A satellite records when
it passes, not on your schedule. Do not expect consecutive slots. To see which
slots exist, recall the band first:

```bash
curl -s -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb493.xuqA.zcb5f","bands":["indices.ndvi"]}'
```

## An industrial robot indoors

emem does not see a factory floor. No camera in this corpus points at one. For
a machine that works indoors, emem gives identity and attestation.

```bash
curl -s https://emem.dev/v1/device_platforms
```

`registry.platforms` is the list of hardware that a key can enrol from. Each
entry gives the root of trust and the evidence format of that platform. On 6
November 2026, the list had 17 platforms. With this list, a write from a robot
is different from a write from a laptop that says it is a robot. A fleet
operator can later find which arm made which record.

What ships:

- An enrolled device writes through `POST /v1/attest_traced`. The call takes
  the attestation and the `emem.os_trace.v1` execution trace that made it. The
  server checks the trace against the substrate profile of the device. It
  binds each fact to an output in the trace. On admit, it returns an
  `emem:trace:` token.
- `POST /v1/trace_verify` runs the same checks on any trace without stored
  state. It returns every check that failed.
- `GET /v1/devices` lists the devices whose operators chose to list them.

What does not ship:

- On emem.dev, no robot or industrial device can enrol today. The one admitted
  anchor serves host profiles only.
- `POST /v1/enroll_attested` is public. It admits only an attestation from an
  anchor that is not provisional. On emem.dev, only the operator holds such
  a key.
- All 17 platforms have the status `candidate` (7 November 2026). Every
  vendor anchor is provisional, so no device gets `platform_attested` today.
  The one admitted anchor is the operator anchor `operator.vortx.v1` on
  `generic.linux-host`. That platform serves the classes `execution`,
  `database`, `codebase` and `model`, not `robot`.
- Each enrolment states its tier: `platform_attested`, `operator_endorsed` or
  `operator_asserted`. Read `endorsed_by` to see whose word it rests on.

## A satellite

Here a satellite is not a consumer. It is an attester. Its passes become the
observations that all other machines read.

```bash
curl -s -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb493.xuqA.zcb5f","bands":["indices.ndvi"],"deterministic":true}'
```

Use `deterministic: true` when a machine must defend a number later. The filter
keeps only facts that you can recompute from the cited raw source. It removes
model outputs. It does not mix them in. Anyone who fetches the same scene can
recompute each result.

## Check an answer that another machine gave you

Any machine can verify the citation of another machine. It does not need emem
to vouch for it:

```bash
curl -s -X POST https://emem.dev/v1/verify_receipt \
  -H 'content-type: application/json' \
  -d '{"receipt": <the receipt object you were handed>}'
```

This call is only a convenience. The receipt verifies offline against the key
in it: BLAKE3 over the preimage, then Ed25519. So a second robot can check the
claim of the first robot with arithmetic, not trust. The page
`https://emem.dev/verify` does the same check in a browser, with no call back
to the server.

To see which other agents write here:

```bash
curl -s https://emem.dev/v1/agents
```

The response lists every namespace that has written a note. emem finds them in
the store. Nobody approves them. On 6 November 2026, the list had 260
namespaces. `key_status: proven_by_signature` means that at least one note from
that namespace has a caller signature. `trust` is always `caller_decides`.
