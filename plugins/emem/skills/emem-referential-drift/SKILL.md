---
name: emem-referential-drift
description: Stops two agents, or one agent in two sessions, from reporting different numbers for the same reading. Pins a value to a citation token that another party resolves to the same signed bytes. Grades a value that you are about to say against the signed value. Asks why a number moved when the words did not. Finds where the corpus disagrees with itself. Use it when a figure moves between contexts. Use it when two sources give different values for one place. Use it when a number changed and nobody can say why. Use it before you publish a number that you did not read from a token in this turn.
---

# emem-referential-drift

| | |
|---|---|
| Use it when | A number moves between agents, sessions or documents, and it must stay the signed number. |
| You need | A cell64, or an `emem:fact:` token. No key. |
| You get | A citation token, a match grade (`matches`, `drift`), a per-term change ledger, and a list of signed disagreements. |
| Calls | `emem_recall`, `emem_memory_token_resolve`, `emem_echo_verify`, `emem_change_attribution`, `emem_memory_contradictions`. Each has a REST route. |

> **Network use.** The commands in this skill send JSON to `https://emem.dev` and receive JSON from it. Some steps also receive an image or a raster file. `https://emem.dev` is the service that the MCP server of the plugin connects to. The commands do not execute what they download. This skill ships no scripts. The commands read no environment variable, key or credential. They read only local files that an earlier step saved from `https://emem.dev`.

Referential drift has two sides. On the words side, "the north field",
"plot 14" and a cell64 are one object, or one phrase means two objects.
On the values side, agents copy, round and summarise a number until
nobody can trace it.

The words side is [`emem-shared-identity`](../emem-shared-identity/SKILL.md).
This skill is the values side.

Each call below is a tool of the MCP server of the plugin. Without MCP,
send the same JSON body to the REST route that the step names.
`emem_change_attribution` is not in the short tool list that the server
shows. The server runs it by name through `tools/call`. If your host
calls only listed tools, use the REST route.

## Step 1: get the token for the reading

Each fact that a recall returns has its citation in `memory_token`.
`current_by_band` names the fact cid of the latest reading for each band.
Call `emem_recall` with this body:

```json
{"cell": "defi.zb493.zezo.zcb35", "bands": ["weather.temperature_2m"]}
```

Without MCP, save the result and pick the token of the current reading:

```sh
curl -s -X POST https://emem.dev/v1/recall -H 'content-type: application/json' \
  -d '{"cell":"defi.zb493.zezo.zcb35","bands":["weather.temperature_2m"]}' -o recall.json
jq -r '.current_by_band["weather.temperature_2m"] as $c
       | .facts[] | select(.fact_cid == $c) | .memory_token' recall.json
```

`facts[]` is in order from oldest to newest. Thus `.facts[0]` is the
oldest reading, not the current one. On 7 November 2026 this cell held
12 temperature facts, from 23 July to 6 November 2026.

If you have a cell and a fact cid from a different source, build the
same token with `POST https://emem.dev/v1/memory_token`. Send
`{"cell": ..., "fact_cid": ..., "band": ...}` and read `memory_token`.
The result also has the provenance block of the band.

## Step 2: resolve the token, quote the signed string

The token is `emem:fact:<cell64>:<fact_cid>`. Each agent that resolves
it gets the same signed object. Call `emem_memory_token_resolve`, or:

```sh
curl -s -X POST https://emem.dev/v1/memory_token/resolve -H 'content-type: application/json' \
  -d '{"token":"emem:fact:defi.zb493.zezo.zcb35:nflpddk7zsncywguwjzk5koksseqfyx4jnngkuryrnd4aykqlpfq"}' \
  | jq -c '{value_verbatim, unit, band, kind}'
```

On 7 November 2026 this returned
`{"value_verbatim":"28.0","unit":"degC","band":"weather.temperature_2m","kind":"primary"}`.
A signed fact does not change, so this token gives the same answer on
each day.

Quote `value_verbatim`. It is the exact decimal string that the responder signed.
If you type the JSON number again, you can lose precision.

To resolve many tokens in one call, send `{"tokens": [...]}` to
`POST https://emem.dev/v1/memory_token/resolve_many`. The limit is
256 tokens (`max_tokens`). Each entry of `items[]` has `ok` and
`resolution`. The value is at `items[].resolution.value_verbatim`.

## Step 3: grade the number before you say it

`emem_echo_verify` (REST `POST https://emem.dev/v1/echo_verify`)
compares the value that you are about to write with the fact that your
token names:

```sh
curl -s -X POST https://emem.dev/v1/echo_verify -H 'content-type: application/json' \
  -d '{"token":"emem:fact:defi.zb493.zezo.zcb35:nflpddk7zsncywguwjzk5koksseqfyx4jnngkuryrnd4aykqlpfq","claimed_value":28.0}' \
  | jq -c '{matches, drift}'
```

The check compares the strings first, then the numbers. Results on
7 November 2026:

| Signed value | `claimed_value` | `matches` | `drift` |
|---|---|---|---|
| `28.0` | `28.0` | `true` | `null` |
| `28.0` | `28` | `true` | `null` |
| `28.0` | `"28"` with `"strict": true` | `false` | `reformatted` |
| `23.9` | `24` | `false` | `rounded` |
| `28.0` | `0.74` | `false` | `wrong` |

`rounded` means that you lost precision. `wrong` means that you have a
different number. Use `strict` when the digits themselves matter, for
example in a compliance report. The receipt in the result signs the
resolution of the token. It does not sign your claim.

This is the cheapest check in this skill. It finds a failure that you
cannot see yourself: a number that you paraphrased some turns before.
Run it on each figure that you did not read in this turn from a
resolved token.

## Step 4: ask why a number moved

Use `emem_change_attribution` (REST
`POST https://emem.dev/v1/change_attribution`) when the words held and
the readout changed between visits:

```sh
curl -s -X POST https://emem.dev/v1/change_attribution -H 'content-type: application/json' \
  -d '{"cell":"defi.zb493.zezo.zcb35"}' \
  | jq -c '{observed, split, terms: (.terms | keys)}'
```

The result is a ledger of evidence for each term. Each term cites its
fact cids:

| Term | What it holds |
|---|---|
| `terms.env` | Index pairs (NDVI, NBR, NDWI) from Sentinel-2, with raw deltas. |
| `terms.sensor` | The observation path of each visit, per band, and if that path changed. |
| `terms.noise` | The scene classification (`s2.scl`) at each visit, and if the class changed. |
| `terms.encoder` | If the embedding recipe changed between the two vintages. |
| `terms.geo` | Not estimated. This build has no registration-residual surface. |
| `observed` | The change in the multi-year embedding, where the store holds a vector. |

On 7 November 2026 this call returned all six keys for this cell, with
`split: null` and `n_env_evidence_bands: 3`.

Two limits apply:

- The result has no numeric split. `split` is `null`, and
  `attribution_note` says that the split is roadmap work. Do not report
  a share such as "60% environmental".
- `observed` needs a stored embedding. emem.dev no longer computes the
  embedding band, so `observed` is present only where the store already
  holds a vector. `terms.env` comes from Sentinel-2 indices and does not need
  the embedding. Use it first.

On a cold cell the call can return `compute_timeout`, because it fetches
several bands for two visits. This occurred on 28 September 2026. An
immediate retry does not help. Recall the index bands of the cell first,
wait for the background fetch, then call again. This tool does not
accept the MCP `task` parameter. Only `emem_eudr_dds` and `emem_hunt`
run as tasks.

## Step 5: find where the corpus disagrees

`emem_memory_contradictions` (REST
`POST https://emem.dev/v1/memory_contradictions`) finds two or more
signed values that disagree for the same place, band and time:

```sh
curl -s -X POST https://emem.dev/v1/memory_contradictions -H 'content-type: application/json' \
  -d '{"cell":"defi.zb493.zezo.zcb35","min_severity":0.2,"limit":20}' \
  | jq -c '{found: (.contradictions | length), corpus_scanned, scan_truncated}'
```

On 7 November 2026 this returned 0 contradictions, with
`corpus_scanned: 6` and `scan_truncated: false`.

All fields are optional. `cell_prefix` limits the scan to a region.
`band` limits it to one measurement. `window_unix_s` limits it to a
period. Without a scope, the scan stops at a cap. On 7 November 2026 a
call with only `{"limit": 3}` scanned 44682 facts and returned
`scan_truncated: true`. Read `scan_truncated` before you report "none
found".

Each entry has a `severity` from 0 to 1 and a `disagreement_scope`:

| `disagreement_scope` | Meaning |
|---|---|
| `multi_attester` | Two or more different keys signed different values. |
| `same_attester_provider_substitution` | One key signed values from two different upstream sources. This is one witness that changed instruments, not two witnesses. |

Do not hide a contradiction. Report each signed value and its
`attester_pubkey_b32`. Do not choose a winner, and do not average. No
key signed the mean of two values that disagree.

## When you must prove the log itself

Sometimes the question is: "Could this responder show a different
history to someone else?" For that question, use
[`emem-transparency-log`](../emem-transparency-log/SKILL.md).

## Common mistakes

| Mistake | Result | Do this |
|---|---|---|
| You quote a number without its token. | Nobody downstream can find drift. | Put the `emem:fact:` token next to the number. |
| You resolve once, then paraphrase. | The paraphrase is the drift. | Resolve again, or quote `value_verbatim`. |
| You take `facts[0]` as the current value. | You report the oldest reading. | Use `current_by_band`. |
| You read a split out of change attribution. | You report shares that no key signed. | Report the evidence for each term. |
| You read `same_attester_provider_substitution` as two witnesses. | You count one source twice. | Read `disagreement_scope` first. |
