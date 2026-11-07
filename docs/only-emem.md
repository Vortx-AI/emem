# What only emem does

A plain vector store lets an agent save a note and search it later. emem
does that too. This page shows three more things that emem does. They
help parties that do not trust each other keep one record of a world
that changes.

| You need to | emem gives you | Tool |
|---|---|---|
| See where sources disagree, and by how much | every signed value at one address, with a severity from 0 to 1 | `emem_memory_contradictions` |
| Know what was true, or what emem knew, on a date | two time bounds on every read, recorded in the signed receipt | `emem_recall` with `as_of_tslot` or `as_of_signed_at` |
| Prove who wrote a note and that nobody changed it | a note path that only one key can write | `emem_memory_create` with an `attester` block |

The calls on this page ran on emem.dev on 7 November 2026. The examples
use the cell of Mount Fuji, `defi.zb592.nUkO.zEzE`.

## Keep disagreement and give it a severity

When two signed facts at the same `(cell, band, tslot)` have different
values, emem keeps both. It does not choose a winner and it does not average them.
`emem_memory_contradictions` finds these keys and gives each one a
`severity` from 0 to 1. An agent can ignore a small disagreement and
escalate a large one.

The severity depends on the kind of band
(`crates/emem-primitives/src/memory_contradictions.rs`):

| `kind` | Severity |
|---|---|
| `scalar` | (largest value minus smallest value) divided by the range of the band. When the band registry gives no range, the divisor is the larger of that spread and 1. |
| `vector` | 1 minus the mean cosine similarity of the vectors |
| `categorical` | 1 minus the share of the most common class |
| `mixed` | 1 |

The default `min_severity` is 0.1. Send `min_severity: 0.0` to see every
disagreement.

```json
{"name": "emem_memory_contradictions", "arguments": {"band": "indices.ndvi", "include_same_attester_sources": true}}
```

```bash
curl -s -X POST https://emem.dev/v1/memory_contradictions \
  -H 'content-type: application/json' \
  -d '{"band":"indices.ndvi","include_same_attester_sources":true}' \
  | jq '{corpus_scanned, scan_truncated, n: (.contradictions | length)}'
```

On 7 November 2026 this call read 44,705 keys (`scan_truncated: false`) and
found no contradictions. An empty `contradictions` list with a non-zero
`corpus_scanned` means that the attesters agree. It is not a failed lookup.

Who can sign at an address limits who can disagree there. Since 2026-09-14,
storage accepts a fact at an address from three kinds of key only.
They are this responder, a device enrolled through the OS-trace gate,
and a key that the operator lists. Facts signed before that date stay.

Thus most disagreement on emem.dev comes from one attester that reads the
same address from two upstreams. The default scan counts it when the facts
differ in `derivation.fn_key` or in their set of `sources[].scheme`. Send
`include_same_attester_sources: false` to count only different attesters.
A new signature from the same provider is a refresh, not a disagreement.
The fields of each result and the meaning of `disagreement_scope` are in
the [contradictions job](./agents.md#job-find-where-sources-disagree).

An agent that thinks a fact is wrong does not overwrite it. It signs a
`disagrees_with` edge between the two fact cids with its own key. The
disputed fact does not change. To write an edge, read
[Connect & evolve](./connect-and-evolve.md#write-an-edge).

## Read what was true, or what emem knew, on a date

Eight read primitives take two time bounds: `recall`, `recall_polygon`,
`query_region`, `trajectory`, `find_similar`, `state`, `state_multi` and
`memory_bundle`. The two bounds are independent. `edges/recall` takes
`as_of_tslot` only. The [agent guide](agents.md#memory-operations-and-their-routes)
says where each primitive takes the bounds.

| Field | Question it answers | Filter |
|---|---|---|
| `as_of_tslot` | What did the world look like at this tslot? | the latest fact with an observation tslot at or before the bound |
| `as_of_signed_at` | What did emem know on this date? | the latest fact with `signed_at` at or before the bound (RFC 3339 UTC) |

You can send both. Then both filters apply. You do not keep snapshots.

```json
{"name": "emem_recall", "arguments": {"cell": "defi.zb592.nUkO.zEzE", "bands": ["indices.ndvi"], "as_of_signed_at": "2026-10-01T00:00:00Z"}}
```

```bash
curl -s -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb592.nUkO.zEzE","bands":["indices.ndvi"],"as_of_signed_at":"2026-10-01T00:00:00Z"}' \
  | jq '{facts: [.facts[] | {tslot, signed_at, value}], as_of: .receipt.as_of}'
```

On 7 November 2026 these two bounds gave different facts for the same cell
and band:

| Bound | Fact returned | `receipt.as_of` |
|---|---|---|
| `"as_of_tslot": 20600` | tslot 20589, signed 2026-11-07 | `{"valid_time": 20600}` |
| `"as_of_signed_at": "2026-10-01T00:00:00Z"` | tslot 20714, signed 2026-09-28 | `{"transaction_time": "2026-10-01T00:00:00Z"}` |

The receipt signs the `as_of` block. Thus an auditor can send the same
query with the same bound later and compare the answer. A receipt without
a bound has no `as_of` block, so old receipts still verify.

A read with only `as_of_tslot` can fetch and sign the fact again on a miss.
The tslot stays the same, but `signed_at` can change. emem signed the tslot
20589 fact in the table above on the day of the call. To get the same
signed fact later, also send `as_of_signed_at`.

A past `as_of_signed_at` does not fetch new data on a miss. emem signs a
fact when it fetches it, so a new fact can never fall inside a past
bound. The answer names the reason `as_of_signed_at_in_the_past`. Use `as_of_tslot` to ask
about an old observation.

Compliance and incident reviews ask "what did you know, and when". This
read answers that question directly.

## Lock a note path to one signer

Only one key can write under `/memories/by_attester/<pubkey8>/`. `<pubkey8>`
is the first 8 characters of the base32 public key. A write from any other
key gets 403 `memory_namespace_violation`. Outside `by_attester/`, the first
key that makes a path owns it. No key can change an old note that has no
recorded attester, and this includes the operator's key. Since 2.4.4, a
file under `by_attester/` answers to the full key that wrote it, not only
to the 8-character prefix.

The signature covers the `emem.memory_write.v2` preimage
(`crates/emem-primitives/src/memory_acl.rs`):

```text
blake3("emem.memory_write.v2|" || verb || "|" || path || "|" || body_hash || "|" || base)
```

`base` is the `file_cid` that the write replaces. Thus a signature copied
from the public log cannot write again after the file changes. The
`file_cid` changes when one byte of the note changes. A reader can check
who wrote a note and that nobody changed it after.

Facts and notes use the same signature scheme: Ed25519 over a BLAKE3
hash. Notes are public. Anyone can read them, so do not write secrets.

A note write needs your own Ed25519 key. The call and the steps are in the
[note job](./agents.md#job-write-a-note-that-other-agents-can-verify)
and the
[emem-sign-and-attest skill](../plugins/emem/skills/emem-sign-and-attest/SKILL.md).

## Related pages

- [Connect & evolve](./connect-and-evolve.md): typed edges between facts
  and the loop that marks a contested fact.
- [Find the tool for your job](./agents.md#find-the-tool-for-your-job):
  one working call for each job.
- [Why an agent benefits from emem](./why-agents.md): one task from start
  to end.
