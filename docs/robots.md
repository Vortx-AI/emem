# Machines that ask emem where they are

*Every call on this page was run against `https://emem.dev` before it was
written down, and `scripts/example_check.py` runs them again on every CI
pass. If one stops working the build fails, not the reader.*

A robot already knows its own coordinates. What it does not know is what is
true at those coordinates: the slope it is about to grade, the wind it is about
to spray into, whether the street ahead is clear, how the crop has moved since
the last pass. That is a memory problem, and it is the one emem is for.

Two properties matter more to a machine than to a person. Every answer carries
a signed receipt, so a second machine can check the first one's claim without
trusting it. And every place has one address, so two machines that resolve
"this field" separately arrive at the same cell64 rather than two descriptions
that only a human can tell apart.

## What emem does not do

It does not drive anything, and it holds no control loop. It answers questions
about places and signs the answers. Latency is a fetch, not a tick: a warm
recall is milliseconds, a cold one that has to reach an upstream can be
seconds. Nothing here belongs inside a safety loop, and a machine that cannot
proceed without an answer should carry its own fallback.

It is also not everywhere. The corpus holds thousands of places, not billions.
Ask for a cell it has never seen and it will fetch what it can and tell you
what it could not.

---

## A street robot, and an autonomous vehicle

Both have the same question at a junction: what is in front of me right now,
and is that a measurement or a guess?

```bash
curl -s -X POST https://emem.dev/v1/perception/at \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb64a.cEjo.zfa47","question":"what is on this street?","stages":["detect"]}'
```

That cell is London. The response names the camera it used, how far away it
was, and why that camera was chosen, because a count from a camera 300 m away
is a different claim from a count at the junction you asked about. It also
carries `camera_requested` and `camera_substituted`: if you name a camera and
it is not there, you are told you were given another one rather than quietly
handed someone else's street.

Read `counts` with the caveat attached. A zero means the detector looked and
found nothing; it does not mean the street is empty, and the response says so
in its own words rather than leaving you to assume.

## A laser leveller

A leveller grades a field to a target slope. Before it starts, the slope that
is already there decides how much soil has to move and in which direction.

```bash
curl -s -X POST https://emem.dev/v1/terrain \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb4e3.zaeed.fEya"}'
```

Back comes `centre_elevation_m`, a `slope` block, `ruggedness`, and
`topo_position`, each with the cell dimensions they were computed over and the
fact ids they were computed from. The `honest_note` says which neighbourhood
was sampled and at what step, because a slope is meaningless without the
baseline it was measured across: 3x3 cells at 27 m is a different number from
3x3 at 90 m, and a machine that mixes them will grade to the wrong plane.

## A sprayer

Wind decides whether a pass happens at all. Drift is the whole argument.

```bash
curl -s -X POST https://emem.dev/v1/weather \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb4e3.zaeed.fEya","bands":["weather.wind_speed_10m"]}'
```

Every reading carries an `age_s`. A wind speed is only a decision input while
it is fresh, and "the newest we hold" is not the same claim as "current" -- the
response distinguishes them so a sprayer does not act on a number from
yesterday afternoon.

Soil is the other half of the same decision, and it answers on the same cell:

```bash
curl -s -X POST https://emem.dev/v1/soil \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb4e3.zaeed.fEya"}'
```

## A harvester

Readiness is a shape over time, not a reading. One NDVI value cannot tell you
whether a crop is climbing or senescing; two months of them can.

```bash
curl -s -X POST https://emem.dev/v1/trajectory \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb493.xuqA.zcb5f","band":"indices.ndvi","window":[20325,20690]}'
```

The window is a pair of Unix epoch-day slots. Each point in `series` carries
its own `fact_cid`, so the curve is not a picture: every point on it
dereferences back to the signed observation it came from, and a second machine
can re-check any of them.

Observations are sparse and unevenly spaced, because a satellite writes when it
passes and not on a schedule you chose. Do not assume consecutive slots. Recall
the band first if you need to know which slots exist:

```bash
curl -s -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb493.xuqA.zcb5f","bands":["indices.ndvi"]}'
```

## An industrial robot, indoors

emem does not see a factory floor, and no camera in this corpus is pointed at
one. What it offers a machine that works indoors is the other half: identity
and attestation.

```bash
curl -s https://emem.dev/v1/device_platforms
```

`registry.platforms` is the whitelist of hardware a key may enrol from, with
the attestation family each platform supports. That is what makes a write from
a robot different from a write from a laptop claiming to be one, and it is why
a fleet operator can later tell which arm produced which record.

The honest state: enrolment evidence is declared and the gate does not yet
verify it end to end. That is named in the roadmap rather than implied away.

## A satellite

A satellite is not a consumer here. It is an attester: the thing whose passes
become the observations everything else reads.

```bash
curl -s -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb493.xuqA.zcb5f","bands":["indices.ndvi"],"deterministic":true}'
```

`deterministic: true` is the filter that matters for a machine that has to
defend a number later. It keeps only facts recomputable from the cited raw
source, so anything a model produced is excluded rather than blended in. What
comes back can be recomputed by anyone who fetches the same scene.

## Checking an answer you were given

Any machine can verify another's citation without asking emem to vouch for it:

```bash
curl -s https://emem.dev/v1/agents
```

That is every key that has ever written here, discovered rather than approved.
A receipt verifies offline against the key in it, so a second robot checks the
first robot's claim with arithmetic instead of trust.
