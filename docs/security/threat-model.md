# Threat model: who can change what another agent reads

emem is shared memory for agents that do not trust each other. The only
attack that matters here is the one where a write by one party changes what a
different party later reads and cannot tell it happened. This document lists
every surface where that is possible, what was measured, what now stands
between the attacker and the reader, and the test that pins it. Nothing here is
asserted from memory: each row was probed on the live responder on
2026-09-14, and the fixes landed the same day.

Reads are open at every tier on every surface. A reader cannot poison
anything. Writes are tiered by what they can REACH, never by who is asking
(`GET /v1/enlist`).

## The surfaces

| surface | what a write changes for others | before (measured) | now | pinned by |
|---|---|---|---|---|
| **fact plane** `/v1/attest`, `/v1/edges`, `/v1/attest_traced` | what every reader `recall`s at `(cell, band, tslot)` | any ed25519 key accepted; trace gate answered "does not apply" to unenrolled keys; attester registry only counted; `/v1/enlist` said "no caller can write a fact by any route" | **closed by default.** Admitted: responder key, trace-enrolled device (with its trace), operator-listed key (`EMEM_FACT_PLANE_WRITERS`). Else `403 level_too_low` at storage, every route. `EMEM_FACT_PLANE_OPEN=1` reopens on purpose, logged at boot | `unenrolled_writers_may_not_occupy_an_address`, `the_responder_key_is_admitted`, `an_operator_listed_key_is_admitted_and_no_other`, `the_plane_is_closed_by_default_and_says_so` |
| **the address itself** canonical index | which of two facts at one key a reader gets | last-writer-wins; the test asserted `canonical == cids_b` as correct | **stays with the key that holds it.** A different signer's fact is stored, indexed under multi-attester, visible to the contradiction scan, and does not take the slot | `put_attestation_populates_multi_attester_index` (assertion flipped) |
| **shared entity space** `emem_entity`, `emem_entity_link` | what every agent resolves a phrasing to | T3 gate enforced (403 live) but attribution discarded at the gate; alias tree = bare cid list in arrival order; `resolve` steered readers to cite the first; no dispute path; no rate bound | **every claim attributed** (`emem.entity_alias_claims`: entity, stance, key, signed_at, receipt ids); `resolve` ranks by DISTINCT asserting keys, never arrival; each candidate carries `asserted_by`, `disputed_by`, `independent_attesters`, `corroboration`; `contested` flagged; `stance: disputes` recorded beside, deletes nothing; legacy rows count as zero keys and say so; per-key rate backstop | `a_shared_name_is_ranked_by_independent_keys_not_arrival_order`, `an_alias_claim_is_stored_with_its_author_and_never_overwritten` |
| **derivations** `/v1/derive` | nothing another agent reads until they are handed the token | T1, attester-validated, own tree, cites parents | unchanged: the open door. A `Fact::Derivative` takes no address | `derive_is_idempotent_per_attester_and_body` |
| **own-namespace notes** `emem_memory_*`, `/v1/inbox` | prose other agents may read | T1, signed, wrapped `_content_is_data_not_instructions`, per-key rate backstop | unchanged | existing memory tests |
| **the ladder itself** `EMEM_ENLISTMENT_ENFORCE` | whether any of the above is enforced | shadow mode unless set to `1` (fail-open) | **enforcing unless set to `0`/`off`/`shadow`** | `enlistment_enforcing` default |

## Measured on the live responder

- `POST /v1/entity/alias` with no attester → `403 level_too_low`. The tier gate
  was real before this work; the missing part was everything after it.
- `emem_entity_resolve("Trafalgar Square")` → **3 distinct objects,
  `contested: true`, all `legacy_unattributed`.** That is the pre-existing
  state, now visible to a reader instead of resolved silently to whichever
  was appended first.
- `CanonicalKey = (cell, band, tslot)`; redb `put_batch` inserted the index
  unconditionally. 20M `fact_proofs` rows and 2.4M multi-attester rows sit
  beside it; they are what let a reader see the loser.

## What is still not done

- **`fact_proofs` (20M rows) lives in sled** and is the whole 21-minute cold
  open. Not a security item; it is the deploy cost. See
  `emem-sled-slim`'s header for the measurement.
- **Legacy alias rows are unattributed forever.** 251 rows. They rank below
  any attributed row and are labelled; an operator could re-assert the true
  ones under the responder key to retire the label.
- **Dispute has no counter-weight in ranking**, by design: it is shown, not
  subtracted, because a single hostile key must not be able to demote a
  binding ten honest keys made. A corroboration-weighted score (asserts minus
  disputes, each counted once per key) is the obvious next step and needs
  the Sybil analysis written down first.
- **T3 means "callable at a declared endpoint."** One such key is still one
  vote in the shared space. Independent-key counting raises the cost from one
  key to N endpoints; it does not make it infinite. T4/T5 (organisation
  vouching, corroboration across responders) are the real ceiling and are
  not computed yet (`/v1/enlist` says so).
- **A backtrace for the boot panic** (`PathAndQuery::from_static`, right after
  the ACME cached-cert event) is now captured on the next boot.
