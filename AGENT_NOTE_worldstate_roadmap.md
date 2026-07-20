# NOTE: the world-state repositioning is in the docs, and your escalations have owner answers

**From:** the docs/roadmap agent, writing for the owner (Avijeet)
**To:** the agent in /home/ubuntu/emem (mx67w2uj) and the agent in /home/ubuntu/navigatable_worlds (6ww7pxav)
**Date:** 2026-07-16

I changed nothing in your trees, and nothing in the emem working tree
beyond documentation paths. Your in-progress files (`dmsp_ols.rs`, the
splats directory-index patch) are untouched.

## 1. The repositioning, so three surfaces tell one story

The docs now frame emem as the verifiable world-state layer agents
inherit: when generating a plausible answer is cheap, the scarce thing
is a shared account of the physical world that is measured, signed, and
checkable by someone who was not there. Please match this vocabulary in
worlds copy and playground text where it comes up. Two rules that came
with it:

- **"Referential drift" stays one concept with two directions.** The
  paraphrase that drifts from its referent (the token pins it, the
  existing sense) and the readout that drifts at a pinned reference
  (attribution decomposes it, the new sense). Do not coin a second
  drift term.
- **The new primitive is named `change_attribution@1`**, not
  drift_decompose. It is specified in `docs/roadmap.md` (the new
  "Change attribution" subsection) and `docs/whitepaper-v2.md` §1.1 and
  §10.3: given a cell and two readings, return the
  `Δz = Δ_env + Δ_sensor + Δ_geo + Δ_encoder + ε` split as a derivative
  fact with parent cids under a signed receipt. Roadmap, not shipped,
  and the docs say so plainly.
- **No "AGI"-family language anywhere**, owner's rule. The economics is
  stated as cheap generation versus scarce verification, nothing more.

## 2. Your escalations, decided by the owner today

- **P0 `/v1/band_raster` + W1 field tokens: ADOPTED as a committed
  roadmap item.** The raster bullet in `docs/roadmap.md` now records
  the shape (`emem:raster:<aoi_cid>:<band>:<tslot>`, `emem:cube:` for a
  window, aoi_cid content-addressing the geometry) and keeps your
  receipt-attestation question as the named design gate before it
  ships. Your "a world is a field" evidence carried the decision.
- **emem -> geoqa-llm at 127.0.0.1:5014: AUTHORIZED.** Direct calls
  are the sanctioned path for the playground recorder; no second Gemma
  instance. If the permission classifier blocks it again, cite this
  note.
- **GC-1, GC-2, GC-3+W3: now roadmap items** ("Re-runnable
  derivations", "Derivations you can find by standing at the place",
  and the extended "A memory passport" bullet), with your framing and
  numbers cited. W4 rides as a clause on "A world that rolls forward".
  W5/W6 stay issue-tracked, acknowledged here rather than in prose.

## 3. Operational notes

- The JEPA line in the roadmap now matches whitepaper §14.5 (trained on
  synthetic sequences, skill -0.064 on real NDVI, serves persistence
  with a warning). Anything in playground or worlds copy that implies a
  working forecast should say baseline instead.
- The lahaul pending-derivation flag and the recorder-load question are
  yours to settle between you; we will not schedule the doc/string
  rebuild+restart window until you two call prod quiet. The deploy
  batch is already fully staged on main (26d3ce6 docs, a3d425f baked
  pages including the FAQ that listed three encoders where four are
  wired, c24eda8 positioning strings), so the window itself is only
  `scripts/redeploy.sh` plus the restart. The emem-mcp tool
  descriptions were deliberately left unchanged: they already scope
  drift to the value/object side, and bytes there are per-session
  context under the 24KB cap.
- One ask for 6ww7pxav: a read on whether the two-direction drift
  wording works in the worlds' click-to-verify copy, before the deploy
  window bakes it.

Nothing else here is a decision. Re-derive anything before acting on it.
