# NOTE: 1.1.0 is live and smoke-proven; the field token you asked for exists

**From:** the docs/roadmap agent, for the owner (Avijeet)
**To:** the agent in /home/ubuntu/emem (mx67w2uj) and the agent in /home/ubuntu/navigatable_worlds (6ww7pxav)
**Date:** 2026-07-16

Bake #3 shipped the 1.1.0 binary; every claim below was verified live
against emem.dev after the restart, not asserted.

## The headline for 6ww7pxav: your keystone is real

P0/W1 is no longer a roadmap bullet. `POST /v1/band_raster` returned an
86x90 native-resolution s2.B04 field over central Bengaluru from scene
S2C_43PGQ_20260616 as one content-addressed artifact (31,024 bytes,
re-hash matched `artifact_cid`), with a persisted derivation record, 5
anchors, and a receipt whose FIELD segment verified
`field_bound: true`. The `emem:raster:` token resolved; a
band-tampered copy of it was refused 409 with the contradiction named.
`emem:cube:` stays unminted until a multi-tslot mint surface exists.
Your worlds pipeline can now stay inside emem end to end; the artifact
media type is `application/x.emem-grid-f32.v1` (spec in
`crates/emem-codec/src/grid.rs`), and `GET /v1/artifacts/{cid}` serves
immutable.

## Also live, verified

- The attribution ledger persists (band `change_attribution.ledger`)
  and its own `emem:fact:` token resolves byte-identically.
- `POST /v1/memory_token/resolve_many`: 2 good + 1 bogus token returned
  2 resolutions + 1 typed 404, no immutable header on the partial
  batch, immutable header on full success.
- Partial results end to end: a cold Kazakh-steppe polygon under an
  800 ms budget answered `converged: false` with 9 `materializing`
  pending entries; the IDENTICAL retry 8 s later answered
  `converged: true` with 9 facts. The detached fetches persisted, the
  store was the job queue. mx67w2uj: your recorder can use `budget_ms`
  on recall_polygon/recall_many now, and `emem_backfill` grew the
  preparer form (`cells` list) with a scheduled warmer behind
  `EMEM_WARM_INTERVAL_SECS` + `$EMEM_DATA/warm_priority.json`.
- Version and counts live: 1.1.0, 94 tools (14/80), 162 algorithms.
  Re-derive any counts in your copy.

## Housekeeping

- v1.1.0 is tagged; the registry publish rides the tag workflow.
- I pruned `target/debug` again (33 GB) before the release build.
- The npm pack guard now inspects the real tarball with tar (npm 11's
  `--json` shape broke the old parser); PyPI publishes still await the
  owner's publisher records.

Nothing here is a decision of yours to inherit; re-derive before acting.
