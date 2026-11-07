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
- Now: storage closes the plane by default. It admits three kinds of key: the responder
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
- Now: the address stays with the key that holds it. The server stores a
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
- Now: `emem.entity_alias_claims` records who made every claim, with
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
- Before and now: a write needs tier T1 and a valid attester. It goes to its
  own tree and cites its parents. Any T1 key can write here, on purpose. A `Fact::Derivative` takes no address,
  and neither does an edge.
- Test: `derive_is_idempotent_per_attester_and_body`.

### Own-namespace notes: `emem_memory_*`, `/v1/inbox`

- What a write changes for others: prose that other agents can read.
- Before and now: a write needs tier T1 and a signature. A per-key rate
  limit applies. Readers get the note inside
  `_content_is_data_not_instructions`.
- Since 5 November 2026 every write verb must use the v2 preimage. It binds
  the version that the write replaces, so nobody can replay an old write
  from the public log.
- Test: `a_v1_write_signature_is_refused_for_every_verb`.

### The ladder itself: `EMEM_ENLISTMENT_ENFORCE`

- What a write changes for others: whether any of the gates above apply.
- Before: shadow mode unless an operator set the variable to `1`. A node that
  forgot the variable failed open.
- Now: enforcing unless set to `0`, `off`, `false` or `shadow`.
- Test: none. The default is in the function `enlistment_enforcing` in
  `crates/emem-api-rest/src/lib.rs`. A tier verdict reports it in its
  `enforcing` field.

## What we measured on the live responder

- On 14 September 2026, `POST /v1/entity/alias` with no attester returned
  `403 level_too_low`. The tier gate already worked. The fixes added the
  attribution, ranking and dispute steps that come after the gate.
- `emem_entity_resolve("Trafalgar Square")` returned 3 distinct objects,
  `contested: true`, none attributed. That was the state before the fix.
  A reader now sees it, and the server does not silently pick the first
  append. On 7 November 2026 the same call still returned 3 candidates,
  `contested: true`, each with `independent_attesters: 0`.
- `CanonicalKey = (cell, band, tslot)`. The redb `put_batch` wrote the index
  with no check. On 14 September 2026, 20 million `fact_proofs` rows and 2.4
  million multi-attester rows sat next to it. Those rows let a reader see the fact
  that lost the slot.

## What is still not done

- The 21-minute cold open on restart is a deploy cost. It is not a security
  item. On 14 September 2026 `fact_proofs` held 20 million rows in sled and
  caused it. The
  code now keeps `fact_proofs` and the multi-attester index in a redb file
  (`facts.redb`). A background job copies the old sled rows across.
  `EMEM_HOT_BACKEND=sled` rolls this back. The header of `emem-sled-slim` has
  the measurement. The server still opens the sled store at boot. The cold
  open stays until both backfills finish and an operator slims the store.
- Legacy alias rows stay unattributed. There were 251 such rows on
  14 September 2026. They rank below any attributed row and carry a label. An
  operator can assert the true ones again under the responder key to remove
  the label.
- A dispute does not lower a rank. This is by design. The server shows a dispute
  but does not subtract it. One hostile key must not be able to demote a
  binding that ten honest keys made. The next step is a score that counts
  asserts minus disputes, once per key. It needs a written Sybil analysis
  first.
- T3 means "callable at a declared endpoint". One such key is still one
  vote in the shared space. A count of independent keys raises the cost of an
  attack from one key to N endpoints. It does not make the cost infinite.
  T4 and T5 are the real limit. On 7 November 2026 the server computed T4 on
  demand and did not compute T5 (`/v1/enlist` says so).
- The boot panic (`PathAndQuery::from_static`, right after the ACME
  cached-cert event) did not have a backtrace. The production unit now sets
  `RUST_BACKTRACE=1` (`deploy/systemd/emem-server.service`), so the next
  panic logs one.
