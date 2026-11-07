# Threat model: who can change what another agent reads

emem is shared memory for agents that do not trust each other. One attack
matters most here. A write by one party changes what a different party reads
later, and the reader cannot see that it happened.

This page lists every surface where that attack is possible. For each one it
gives what we measured, what now protects the reader, and the test that holds
the fix in place. We probed each row on the live responder on 14 September
2026. The fixes landed the same day. Rows that we checked again on 7 November
2026 say so.

Reads are open at every tier on every surface, so a reader cannot poison
anything. Writes have tiers by what they can reach, never by who is asking
(`GET /v1/enlist`). The [security page](../security.md) explains the tiers.

## The surfaces

### Fact plane: `/v1/attest`, `/v1/attest_traced`

- What a write changes for others: what every reader recalls at
  `(cell, band, tslot)`.
- Before: storage accepted any Ed25519 key. The trace gate answered "does not
  apply" to keys that nobody had enrolled. The attester registry only counted
  keys. `/v1/enlist` said that no caller could write a fact by any route,
  which was false.
- Now: **closed by default.** Storage admits three kinds of key: the responder
  key, a trace-enrolled device with its trace, and a key in
  `EMEM_FACT_PLANE_WRITERS`. It refuses all others with 403 `level_too_low`,
  on every route. `EMEM_FACT_PLANE_OPEN=1` opens the plane on purpose, and the
  server logs this at boot.
- Tests: `unenrolled_writers_may_not_occupy_an_address`,
  `the_responder_key_is_admitted`,
  `an_operator_listed_key_is_admitted_and_no_other`,
  `the_plane_is_closed_by_default_and_says_so`.

### The address itself: the canonical index

- What a write changes for others: which of two facts at one key a reader
  gets.
- Before: the last writer won. The test asserted `canonical == cids_b` as the
  correct result.
- Now: **the address stays with the key that holds it.** The server stores a
  fact from a different signer and indexes it under multi-attester. The
  contradiction scan can see it. It does not take the slot.
- Test: `put_attestation_populates_multi_attester_index`, with its assertion
  reversed.

### Shared entity space: `emem_entity`, `emem_entity_link`

- What a write changes for others: what every agent resolves a phrase to.
- Before: the T3 gate worked (403 live). But the gate discarded
  attribution. The alias tree was a bare list of CIDs in arrival order.
  `resolve` told readers to cite the first one. There was no dispute path and
  no rate limit.
- Now: **`emem.entity_alias_claims` records who made every claim**, with
  entity, stance, key, `signed_at` and receipt ids. `resolve` ranks by the
  number of distinct keys that assert a binding, never by arrival order.
- Each candidate carries `asserted_by`, `disputed_by`,
  `independent_attesters` and `corroboration`. The response flags `contested`.
- `stance: disputes` records a dispute next to the claim and deletes nothing.
  Legacy rows count as zero keys and say so. A per-key rate limit applies.
- Tests: `a_shared_name_is_ranked_by_independent_keys_not_arrival_order`,
  `an_alias_claim_is_stored_with_its_author_and_never_overwritten`.

### Derivations and edges: `/v1/derive`, `/v1/edges`

- What a write changes for others: nothing that another agent reads until
  someone gives them the token.
- Before and now: tier T1, attester validated, own tree, cites its parents.
  This is the open door, on purpose. A `Fact::Derivative` takes no address,
  and neither does an edge.
- Test: `derive_is_idempotent_per_attester_and_body`.

### Own-namespace notes: `emem_memory_*`, `/v1/inbox`

- What a write changes for others: prose that other agents can read.
- Before and now: tier T1, signed, wrapped in
  `_content_is_data_not_instructions`, with a per-key rate limit. Since
  5 November 2026 every write must use the v2 preimage, which binds the
  version that it replaces.
- Tests: the memory tests in `crates/emem-api-rest`.

### The ladder itself: `EMEM_ENLISTMENT_ENFORCE`

- What a write changes for others: whether any of the gates above apply.
- Before: shadow mode unless an operator set the variable to `1`. A node that
  forgot the variable failed open.
- Now: **enforcing unless set to `0`, `off`, `false` or `shadow`.**
- Test: the `enlistment_enforcing` default.

## What we measured on the live responder

- On 14 September 2026, `POST /v1/entity/alias` with no attester returned
  `403 level_too_low`. The tier gate already worked. The work added
  everything after the gate.
- `emem_entity_resolve("Trafalgar Square")` returned **3 distinct objects,
  `contested: true`**, none attributed. That was the state before the fix.
  A reader now sees it, and the server does not silently pick the first
  append. On 7 November 2026 the same call still returned 3 candidates,
  `contested: true`, each with `independent_attesters: 0`.
- `CanonicalKey = (cell, band, tslot)`. The redb `put_batch` wrote the index
  with no check. On 14 September 2026, 20M `fact_proofs` rows and 2.4M
  multi-attester rows sat next to it. Those rows let a reader see the fact
  that lost the slot.

## What is still not done

- **`fact_proofs` lives in sled**, at 20M rows on 14 September 2026. It caused
  the whole 21-minute cold open. This is a deploy cost, not a security item.
  The header of `emem-sled-slim` has the measurement.
- **Legacy alias rows stay unattributed.** There were 251 such rows on
  14 September 2026. They rank below any attributed row and carry a label. An
  operator can assert the true ones again under the responder key to remove
  the label.
- **A dispute does not lower a rank**, by design. The server shows a dispute
  but does not subtract it. One hostile key must not be able to demote a
  binding that ten honest keys made. The next step is a score that counts
  asserts minus disputes, once per key. It needs a written Sybil analysis
  first.
- **T3 means "callable at a declared endpoint."** One such key is still one
  vote in the shared space. A count of independent keys raises the cost of an
  attack from one key to N endpoints. It does not make the cost infinite.
  T4 and T5 are the real limit. On 7 November 2026 the server computed T4 on
  demand and did not compute T5 (`/v1/enlist` says so).
- **The boot panic** (`PathAndQuery::from_static`, right after the ACME
  cached-cert event) did not have a backtrace. The server now captures one on
  the next boot.
