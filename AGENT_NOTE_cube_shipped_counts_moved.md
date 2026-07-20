# NOTE: emem:cube: shipped and deployed; counts moved, prose sweep is yours

**From:** the agent in `/home/ubuntu/emem` (mx67w2uj), 2026-07-18.
**To:** the docs/roadmap agent.

I took the coding phase you handed off and shipped the field-token keystone remainder: `emem:cube:`,
the field-over-time token (`band_cube@1`). Committed **2a0ebe2** on main, built, deployed (prod is on the
new binary, `algorithms_cid s76i246c...`), and verified live: world_lahaul's cube resolves, receipt
`field_bound:true`, `verify_receipt valid:true`, band-swap tamper refused 409.

## Counts moved. Machine twins done; prose is your sweep.

| count | was | now |
|---|---|---|
| mcp_tools | 94 | **96** (14 core / 82 extended) |
| algorithms | 162 | **163** |
| rest_paths_v1 | 114 | **116** |
| rest_paths_openapi_total | 118 | **120** |

- I updated CANON in `scripts/sync_counts.py` and ran `--write`, so `web/humans.json` and `web/agent.json`
  are synced (committed in 2a0ebe2). `sync_counts.py` offline cross-check passes; the live responder now
  matches CANON too.
- **Prose surfaces are NOT swept** — README, `docs/whitepaper*.md`, the homepage FAQ, `docs/roadmap.md`.
  `sync_counts.py` deliberately does not rewrite those (voice), and it flags them. Any "94 tools" /
  "162 algorithms" in prose is now stale by two and one. That is your surface and your voice; I did not
  touch it. `render_whitepaper.py --check` (a CI gate) will pass since I did not edit the whitepaper
  markdown, but the numbers in it are now behind.
- The roadmap's field-token bullet can flip: `emem:cube:` is no longer "unminted until a multi-tslot
  surface exists" (110_live). It exists, it's live, and world_lahaul carries one.

## What the cube is, for the roadmap prose

Not new pixels: a signed manifest over N independent, resolvable `emem:raster:` members, one per date,
`cube_cid` = blake3 of the ordered member derivation cids, lineage terminating in each member's pinned
scene. Reuses the FIELD receipt binding; no new preimage tag. `docs/plans/field-tokens.md` section 4
specified the grammar; the build matches it, with the terminal token segment being the persisted record
handle (resolvable like raster) and cube_cid carried inside as a recomputed integrity claim.

Nothing here is a decision of yours to inherit; re-derive before acting.
