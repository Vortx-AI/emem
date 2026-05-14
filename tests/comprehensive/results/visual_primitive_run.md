# Visual-primitive subset run — 2026-05-14

Self-contained run of the 27 questions added in id-band 31x..37x
(domain = `visual_primitive`). Distinct from the full-corpus run in
`report.md`, which covers all 132 questions across 22 domains.

## Result

- **Endpoint**: `http://127.0.0.1:5051` (local responder, post-renderer
  rewrite, commit 524b3fa)
- **Questions**: 27
- **Routing pass**: **21 / 27 (77%)**
- **HTTP 200**: 27 / 27
- **Place resolved**: 27 / 27
- **Returned ≥1 fact**: 27 / 27
- **Latency**: avg 28.3 s, p50 23.8 s, p95 59.4 s (cold materialiser
  pays the long tail; warm reads are sub-second)
- **Volume-weighted accuracy**: 80.8%

Against the achievable subset (27 minus the six known router gaps
catalogued in `questions_v2.json` `_meta.known_router_gaps_2026_05_14`):
**21 / 21 = 100%**.

## What the six failures actually surface

These are documented in `questions_v2.json` so future
expect-list tightening doesn't paper over them:

| id  | question (abridged)                            | what the router returned                          | the real gap |
|-----|------------------------------------------------|---------------------------------------------------|--------------|
| 311 | "show me the dem of manhattan as an overlay"   | optical_raw_reflectance, built_up_human_geography | `dem` literal isn't a strong elevation cue; Manhattan biases to optical/built |
| 316 | "render night lights of pyongyang vs seoul"    | optical_raw_reflectance, scene_classification     | no `nightlights` topic in the 26-topic registry |
| 320 | "where do you have signed facts on earth"      | built_up_human_geography, soil_bare               | coverage-audit queries have no dedicated topic; should route to `/v1/coverage_map.svg` via a meta surface |
| 321 | "show me the global coverage map"              | vegetation_condition, optical_raw_reflectance     | same coverage-audit gap |
| 322 | "how dense is your corpus over sub-saharan africa" | scene_classification, vegetation_condition    | same coverage-audit gap |
| 342 | "trajectory of nightlights over kyiv 2022"     | analytics, weather_now                             | nightlights gap, again |

## Latency notes

- Cold materialiser cost dominates the long tail: q317 Amazon arc of
  deforestation cold-loaded 24 facts in 59 s; q331 Singapore/KL
  comparison cold-loaded 35 facts in 60 s; q361 Brittany dairy belt
  hit field-boundaries PMTiles in 71 s.
- Warm reads are sub-second once the corpus is hydrated.

## What this run does NOT prove

- It does not verify that the `scene_overlay.svg` renderer produces
  the right pixel values — that's regression-tested via the per-cell
  recall path, not via `/v1/ask`.
- It does not verify that the static snapshots committed under
  `docs/gallery/` match the live renderer byte-for-byte — they were
  generated from the live endpoint at the same commit; if the
  renderer changes, those snapshots will drift.
- Routing-pass measures only that the topic router classified the
  question into a plausible set of topics — not that the answer is
  scientifically correct.

## How to reproduce

```sh
cd tests/comprehensive
# Subset run, local responder, ~6 min
python3 run_tests.py \
  --endpoint http://127.0.0.1:5051 \
  --filter-domain visual_primitive \
  --timeout 90 --pace-s 1.0
```
