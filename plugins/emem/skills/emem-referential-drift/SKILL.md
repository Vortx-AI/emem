---
name: emem-referential-drift
description: Stop two agents, or one agent across two sessions, from silently meaning different things by the same words or reporting different numbers for the same reading. Pin a value to a citation another party resolves to byte-identical bytes, grade what you are about to say against what was actually signed, ask why a number moved when the words held, and find where the corpus disagrees with itself. Use when a figure is being carried between contexts, when two sources report different values for one place, when a number changed and nobody can say which part of the world changed, or before you publish a number you did not compute yourself.
allowed-tools: Bash(curl:*) Bash(jq:*) Read
---

# emem-referential-drift

Referential drift is the failure this protocol exists for, and it has two
sides. **Words move**: "the north field", "plot 14" and a cell64 turn out
to be one object, or worse, two agents use one phrase for two objects.
**Values move**: a number is copied from a context, paraphrased, rounded,
re-summarised, and arrives somewhere as a fact nobody can trace.

The words side is [`emem-shared-identity`](../emem-shared-identity/SKILL.md).
This skill is the values side.

## The one habit that matters

**Do not carry a number. Carry a token to the number.**

Minting takes the cell and the **fact_cid of the reading you mean**, so
recall first and mint from what came back — the token names one signed
observation, not "whatever is current at this cell":

```sh
CID=$(curl -sf -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb64a.cAzU.zfa27","bands":["indices.ndvi"]}' \
  | jq -r '.facts[0].fact_cid')

curl -sf -X POST https://emem.dev/v1/memory_token \
  -H 'content-type: application/json' \
  -d "{\"cell\":\"defi.zb64a.cAzU.zfa27\",\"fact_cid\":\"$CID\",\"band\":\"indices.ndvi\"}" | jq .
```

`cell` and `fact_cid` are required; passing `band` adds the band's
provenance block (class, deterministic, tamper evidence, trust rank) to
the response — not to the token string.

You get `emem:fact:<cell64>:<fact_cid>` — a handle any agent or model
resolves to the byte-identical signed object. The receiving party does
not have to trust you, or the channel, or your paraphrase. It resolves
the handle and reads the same bytes you read.

Resolving is one round trip, and `value`, `unit`, `band` and `kind` come
back at the top level beside the full signed body:

```sh
curl -sf -X POST https://emem.dev/v1/memory_token/resolve \
  -H 'content-type: application/json' \
  -d '{"token":"emem:fact:defi.zb64a.cAzU.zfa27:<cid>"}' \
  | jq '{value, unit, band, kind}'
```

## Grade yourself before you speak

`emem_echo_verify` compares the value you are about to emit against the
fact your citation points at. It returns `matches`, and when it does not,
the `drift` between what you were going to say and what emem holds.

This is the cheapest check in the protocol and the one an LLM most needs,
because the failure it catches is the one you cannot feel: you paraphrased
a number three turns ago and have been carrying the paraphrase since.

```sh
curl -sf -X POST https://emem.dev/v1/echo_verify \
  -H 'content-type: application/json' \
  -d '{"token":"emem:fact:defi.zb64a.cAzU.zfa27:<cid>","claimed_value":0.74}' \
  | jq '{matches, drift}'
```

`matches: true` and you may quote it. `matches: false` comes back with a
`drift` naming the failure — `"wrong"` when the value simply is not the
one signed. Both were run against the live responder while writing this.

Run it on any figure you did not read in this turn from a resolved token.

## When the number moved and the words did not

`emem_change_attribution` answers "why is this readout different from last
year's" as a per-term **evidence ledger**, not as a split:

- `terms.env` — label-free index pairs (NDVI, NBR, NDWI) with raw deltas
  and both fact cids: evidence a future estimator would read.
- `terms.sensor` — what each visit was observed through, source scheme and
  scene id per band, and whether the path changed between visits.
- `observed` — the year-over-year embedding change.

```sh
curl -sf -X POST https://emem.dev/v1/change_attribution \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb64a.cAzU.zfa27"}' | jq '{observed, terms}'
```

**Expect this to time out on a cold cell.** Run against a place nobody
has asked about recently it returns `compute_timeout` — it has to
materialise both visits across several bands, and that exceeds the
responder's 40 s transport budget. It is not broken and retrying the same
call will not help. Narrow it (a cell already warm from a recent recall,
or fewer bands), or send it as an MCP tool call with a `task` param and
poll `tasks/get`. Recall the cell first and the call usually lands.

Two honesty notes, both load-bearing:

**There is no numeric split.** Δz = Δ_env + Δ_sensor + Δ_geo + Δ_encoder + ε
is the decomposition this is the first runnable surface of, and the split
itself is roadmap. Report the ledger; do not invent the shares.

**The `observed` term depends on a retired band.** The embedding encoders
are withdrawn on emem.dev, so `observed` is available only where a vector
was already stored. The `terms.env` evidence is computed from Sentinel-2
indices and is unaffected — it is the part to lean on.

## Where the corpus disagrees with itself

`emem_memory_contradictions` surfaces competing evidence: two or more
independent attesters who signed **different values for the same place,
band and time**, with a 0–1 severity.

```sh
curl -sf -X POST https://emem.dev/v1/memory_contradictions \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb64a.cAzU.zfa27","min_severity":0.2,"limit":20}' | jq .
```

Every field is optional: no `cell` scans broadly, `cell_prefix` scopes to
a region, `band` to one measurement, `window_unix_s` to a period.

A contradiction is not an error and must not be hidden. It is the most
informative thing a shared corpus can tell you, and the one an averaging
pipeline destroys. When you find one, report both signatures and who
signed them, rather than picking a winner or taking a mean.

## And when you need to prove the ledger itself

`emem_log_sth`, `emem_log_inclusion`, `emem_log_consistency` and
`emem_log_witnesses` expose an RFC 6962 transparency log: a signed tree
head, an inclusion proof for one entry, a consistency proof between two
heads, and the independent witnesses that countersigned. Reach for these
when the question stops being "is this fact signed" and becomes "could
this responder have shown someone else a different history".

## Pitfalls

- **Quoting a number without its token.** The moment the number leaves
  with no handle, drift starts and nothing downstream can detect it.
- **Resolving once and paraphrasing after.** The paraphrase is the drift.
  Re-resolve, or quote the resolved value verbatim.
- **Averaging a contradiction.** Two attesters disagreeing is a finding.
  A mean of two disagreeing signatures is signed by neither.
- **Reading a split out of change_attribution.** It returns evidence per
  term, not shares. Saying "60% of the change was environmental" is
  inventing the thing the surface deliberately does not compute.
