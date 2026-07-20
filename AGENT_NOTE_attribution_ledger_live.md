# NOTE: change_attribution@1 is live, and your signing surface shipped with it

**From:** the docs/roadmap agent, for the owner (Avijeet)
**To:** the agent in /home/ubuntu/emem (mx67w2uj) and the agent in /home/ubuntu/navigatable_worlds (6ww7pxav)
**Date:** 2026-07-16

Second bake of the day, from `616ffd0`, healthy in 12 s. It carried
both agents' work: the attribution ledger and mx67w2uj's `ememdev`
signing surface (`58a1b08`), which CI now gates correctly after I fixed
the one `import emem` check the module rename had missed in ci.yml.
Your SDK work imported clean on both Python versions.

## What the ledger is

`POST /v1/change_attribution {cell}` (tool `emem_change_attribution`,
registry key `change_attribution@1`): per-term EVIDENCE for
`Δz = Δ_env + Δ_sensor + Δ_geo + Δ_encoder + ε`, with `split` null by
design and an in-band note saying why. Verified live at the Bengaluru
reference cell: observed Tessera change 0.1339, an NDVI pair with raw
delta, NBR/NDWI/SCL honestly `single_vintage` at that cell, encoder
pinned by `geotessera_multi_year@2`, and the receipt (3 fact cids)
round-trips `signature_valid: true`.

Vocabulary for worlds and playground copy: the LEDGER ships, the SPLIT
is roadmap. Do not write "emem attributes change" unqualified; write
"emem reports the attribution evidence and refuses to fabricate the
split". Every doc surface says it that way as of this bake.

## Counts moved

92 tools (14 core, 78 extended), 161 algorithms, and the
`algorithms_cid` manifest advanced accordingly. sync_counts is green
against repo and live. If you carry counts in any copy, re-derive.

Nothing here is a decision of yours to inherit; re-derive before
acting on it.
