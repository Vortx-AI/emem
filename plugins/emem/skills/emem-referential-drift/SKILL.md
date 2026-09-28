---
name: emem-referential-drift
description: Stops two agents, or one agent across two sessions, from reporting different numbers for the same reading. Pins a value to a citation token another party resolves to byte-identical signed bytes, grades a value about to be said against what was signed, asks why a number moved when the words held, and finds where the corpus disagrees with itself. Use when a figure is carried between contexts, when two sources report different values for one place, when a number changed and nobody can say what changed, or before publishing a number the agent did not read from a token in this turn.
---

# emem-referential-drift

Referential drift has two sides. **Words move**: "the north field",
"plot 14" and a cell64 turn out to be one object, or one phrase ends up
meaning two. **Values move**: a number is copied, rounded, paraphrased
and re-summarised until it arrives somewhere as a fact nobody can trace.

The words side is [`emem-shared-identity`](../emem-shared-identity/SKILL.md).
This skill is the values side.

## Carry a token to the number, not the number

Every fact a recall returns already carries its citation in
`memory_token`. Take the one for the reading you mean; `current_by_band`
names the latest:

```sh
curl -sf -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb493.zezo.zcb35","bands":["weather.temperature_2m"]}' \
  | jq -r '.current_by_band["weather.temperature_2m"] as $c
           | .facts[] | select(.fact_cid == $c) | .memory_token'
```

`facts[]` is ordered oldest first, so `.facts[0]` is the oldest reading,
not the current one. If you hold a cell and a fact cid from somewhere
else, `POST /v1/memory_token {"cell":..,"fact_cid":..,"band":..}` builds
the same token (field `memory_token`) and adds the band's provenance
block.

The token is `emem:fact:<cell64>:<fact_cid>`. Any agent resolves it to
the byte-identical signed object:

```sh
curl -sf -X POST https://emem.dev/v1/memory_token/resolve \
  -H 'content-type: application/json' \
  -d '{"token":"emem:fact:defi.zb493.zezo.zcb35:nflpddk7zsncywguwjzk5koksseqfyx4jnngkuryrnd4aykqlpfq"}' \
  | jq '{value_verbatim, unit, band, kind}'
```

Recorded on 2026-09-28: `{"value_verbatim":"28.0","unit":"degC","band":"weather.temperature_2m","kind":"primary"}`.
Quote `value_verbatim`, the exact decimal string that was signed;
re-typing the JSON number is where precision gets lost.
`POST /v1/memory_token/resolve_many` resolves a list in one call.

## Grade yourself before you speak

`POST /v1/echo_verify` (MCP `emem_echo_verify`) compares the value you
are about to emit with the fact your token names:

```sh
curl -sf -X POST https://emem.dev/v1/echo_verify \
  -H 'content-type: application/json' \
  -d '{"token":"emem:fact:defi.zb493.zezo.zcb35:nflpddk7zsncywguwjzk5koksseqfyx4jnngkuryrnd4aykqlpfq","claimed_value":0.74}' \
  | jq '{matches, drift}'
```

Recorded on 2026-09-28: `claimed_value` 28.0 gave `matches: true`;
0.74 gave `matches: false, drift: "wrong"`.

This is the cheapest check here and the one a language model needs
most, because the failure it catches is one you cannot feel: you
paraphrased a number three turns ago and have carried the paraphrase
since. Run it on any figure you did not read in this turn from a
resolved token.

## When the number moved and the words did not

`POST /v1/change_attribution` (MCP `emem_change_attribution`) answers
"why is this readout different from last year's" as a per-term
**evidence ledger**:

- `terms.env`: label-free index pairs (NDVI, NBR, NDWI) with raw deltas
  and both fact cids.
- `terms.sensor`: what each visit was observed through, scene id per
  band, and whether the path changed between visits.
- `observed`: the year-over-year embedding change.

```sh
curl -sf -X POST https://emem.dev/v1/change_attribution \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb493.zezo.zcb35"}' | jq '{observed, terms}'
```

Expect `compute_timeout` on a cold cell: it materialises both visits
across several bands, which can pass the 40 s transport budget (it did
on 2026-09-28). Retrying the same call does not help. Recall the cell's
index bands first, or send it as an MCP tool call with a `task` param
and poll `tasks/get`.

Two limits, both load-bearing:

- **There is no numeric split.** The ledger is evidence per term, not
  shares of the change. Do not report "60% environmental".
- **`observed` needs a retired band.** The embedding encoders are
  withdrawn on emem.dev, so `observed` exists only where a vector was
  stored. `terms.env` comes from Sentinel-2 indices and is unaffected;
  lean on it.

## Where the corpus disagrees with itself

`POST /v1/memory_contradictions` surfaces two or more independent
attesters who signed **different values for the same place, band and
time**, with a 0 to 1 severity:

```sh
curl -sf -X POST https://emem.dev/v1/memory_contradictions \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb493.zezo.zcb35","min_severity":0.2,"limit":20}' \
  | jq '{n: (.contradictions | length), corpus_scanned, scan_truncated}'
```

Every field is optional: `cell_prefix` scopes to a region, `band` to one
measurement, `window_unix_s` to a period. Read `scan_truncated` before
you report "none found".

A contradiction is not an error to hide. Report both signatures and who
signed them rather than picking a winner or averaging; a mean of two
disagreeing signatures is signed by neither.

## When you need to prove the ledger itself

If the question becomes "could this responder have shown someone else a
different history", use the transparency log:
[`emem-transparency-log`](../emem-transparency-log/SKILL.md).

## Pitfalls

- **Quoting a number without its token.** Once the number leaves with no
  handle, nothing downstream can detect drift.
- **Resolving once and paraphrasing after.** The paraphrase is the
  drift. Re-resolve, or quote `value_verbatim`.
- **Taking `facts[0]` as current.** It is the oldest reading.
- **Reading a split out of change_attribution.** It returns evidence per
  term, not shares.
