# Audit reproductions, 2026-08-02

One command per finding from the three-round external audit, with the output
observed when it was run. Every entry is a test we did not have.

**How to read a row.** `Status` is one of `FIXED` (repro now returns the right
answer, regression test named), `OPEN` (reproduced, not yet fixed), or
`NOT REPRODUCED` (claimed, but the behaviour observed here differs, with what
was seen instead). Nothing is marked fixed without a command in this file that
demonstrates it.

`$C` below is `defi.zb493.xuqA.zcb5f` unless stated. Responses are trimmed to
the fields under test.

---

## P0-1 · deterministic and provenance filters were no-ops over MCP

**Status: FIXED** · test `recall_flag_parity_tests` · commit `894f1f2`

The flag the README sells as the control separating evidence from model prose
did nothing on the MCP path, and reported success.

```bash
# REST: correct, the band is model_output so a deterministic read excludes it
curl -s -X POST https://emem.dev/v1/recall -H 'content-type: application/json' \
  -d '{"cell":"defi.zb493.xuqA.zcb5f","bands":["weather.temperature_2m"],"deterministic":true}'

# MCP: same arguments
curl -s -X POST https://emem.dev/mcp -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"emem_recall",
       "arguments":{"cell":"defi.zb493.xuqA.zcb5f","bands":["weather.temperature_2m"],"deterministic":true}}}'
```

Before: REST `0 facts`. MCP `2 facts`, first value `24.0` from met.no, which is
`model_output` by our own taxonomy.

After: both refuse identically, naming the band and its class.

```
REST -> HTTP 400 invalid_argument
MCP  -> tool error (-24)
both -> "provenance filter [direct_sensor, deterministic_index] excludes every
         requested band: weather.temperature_2m (model_output)."
```

And when the classes do match, both return the same facts and echo the filter:

```
provenance:["model_output"]  ->  REST 2 facts, filter=['model_output']
                                 MCP  2 facts, filter=['model_output']
```

**Cause.** `From<RecallApiReq> for RecallReq` leaves `provenance: None` because
`post_recall` set it afterwards, inline. The MCP arm called `.into()` and went
straight to the primitive. Two code paths for one behaviour.

**Fix.** Extracted `recall_req_with_provenance`; both paths call it. Repairing
only the MCP arm would have left the drift that caused this.

**Regression tests.** `deterministic_true_narrows_provenance`,
`an_explicit_provenance_list_is_carried_through`,
`absent_flags_leave_the_recall_unfiltered`,
`an_unknown_provenance_class_is_refused`.

---

## P0-2 · truncated fields were truthy, so an absent verdict read as present

**Status: FIXED** · test `truncation_falsy_tests`

The MCP wire budget is 24 KB and `emem_eudr_dds` returns roughly 120 KB, so the
compliance verdict was among the fields dropped. The slimmer replaced each with
a descriptive object, and that object is truthy.

```bash
curl -s -X POST https://emem.dev/mcp -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"emem_locate",
       "arguments":{"place":"Bangalore"}}}' \
| python3 -c "import json,sys;t=json.loads(json.load(sys.stdin)['result']['content'][0]['text']);
print(t['_emem_truncation']['omitted_fields'][0])"
```

Before: `{"field":"data_at_this_cell","stub":{"_keys":11,"_kind":"object","_omitted":true}}`
and the field itself held that stub. `if result["statement_of_compliance"]:`
passes on it, so a caller could report a plot compliant when no verdict was
computed.

After: the field is `null`, which is falsy in every client language, and the
description moved to `_emem_truncation.omitted_fields[].stub` where it belongs.
The key stays present so shape checks still find it.

**Still open on the same tool.** Truncation is not priority-ordered, so the
verdict is shed on size rather than importance, and a partial DDS is emitted at
all. See P1-6.

---

## P0-3 · `cell` accepts any string, geocodes it, and mints a permanent fact

**Status: OPEN**

```bash
for bad in "not-a-cell" "DROP TABLE facts" "../../etc/passwd"; do
  curl -s -X POST https://emem.dev/mcp -H 'content-type: application/json' \
    -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"emem_recall\",
         \"arguments\":{\"cell\":\"$bad\",\"bands\":[\"indices.ndvi\"]}}}"
done
```

Observed, all three: `isError: false`, signed facts returned, no field in the
response saying a geocode happened. `../../etc/passwd` materialised a new signed
fact.

Three consequences, in severity order:

1. An agent passing a wrong variable gets a confident signed answer about the
   wrong place. That is referential drift produced by the thing built to stop it.
2. The read path writes. Junk input mints an attestation into an append-only
   RFC 6962 log that by design cannot be pruned, unauthenticated and unrated on
   the read side.
3. It contradicts the README's "20 of 20 refusals name the missing field".

**Fix direction.** Validate the cell64 shape and refuse a non-conforming string
with a typed error. If geocoding on recall is wanted, make it an explicit
`place` parameter and echo `resolved_from` in the response.

**Already fixed in passing:** `emem_recall {}` now returns
`tool error (-24) no location provided: pass 'cell' ...` rather than a 200. The
empty-cell check reached MCP with the P0-1 helper.

---

## P0-4 · `valid: true` on a receipt whose Merkle proof fails

**Status: OPEN**

The receipt signature does not cover `merkle_proof`.

```bash
# take a receipt, strip its proof, verify
curl -s -X POST https://emem.dev/v1/recall -H 'content-type: application/json' \
  -d '{"cell":"defi.zb493.xuqA.zcb5f","bands":["weather.temperature_2m"]}' \
| python3 -c "import json,sys;print(json.dumps({'receipt':json.load(sys.stdin)['receipt']}))" \
| python3 -c "
import json,sys,urllib.request
b=json.load(sys.stdin); b['receipt'].pop('merkle_proof',None)
r=urllib.request.Request('https://emem.dev/v1/verify_receipt',data=json.dumps(b).encode(),
                         headers={'Content-Type':'application/json'})
print(json.load(urllib.request.urlopen(r)))"
```

Observed:

| receipt | `valid` | `merkle_proof_valid` |
|---|---|---|
| untouched | `true` | `true` |
| `merkle_proof` removed | `true` | `null` |
| `merkle_proof` swapped from another receipt | `true` | **`false`** |

The third row is the dangerous one: a receiver checking `valid` accepts a
receipt whose log-inclusion proof demonstrably fails. Anyone in transport can
strip the proof and downgrade silently.

The core ed25519 crypto is sound, which is worth stating: `served_at`, `cells`,
`fact_cids`, `primitive` and single-byte signature flips all correctly fail. It
is only the log binding that is unauthenticated.

**Fix direction.** Bind `merkle_proof` into the signed preimage, and make
`valid` a conjunction over everything actually checked rather than the signature
alone. Note this is a preimage change: it needs a `preimage_version` bump and
old receipts must keep verifying under the old rule.

---

## P0-5 · `emem_ask` extracts the wrong entity, confidently

**Status: OPEN** (partially mitigated)

```bash
curl -s -X POST https://emem.dev/v1/ask -H 'content-type: application/json' \
  -d '{"q":"elevation of Bengaluru; also DROP TABLE facts"}'
```

Reported: `place_resolved.input = "DROP TABLE"`, label
`"La Table Ronde - est, Bourg-lès-Valence, France"`, answer `115.00 m` with 12
signed `fact_cids`. No caveat, no alternatives, no confidence field.

`emem_locate` sets `disambiguation_required: true` for an ambiguous name, so the
machinery exists; `emem_ask` does not use it.

**Partially mitigated 2026-07-31**: the prepositional-anchor window now stops at
a clause boundary (`LOCATE_RESOLVER_VERSION` 3), which fixes the
"around X right now, and what does it mean" class. The confidence gate is still
missing, and that is the general fix.

**Fix direction.** A confidence gate shared by `recall`, `ask` and `locate`:
refuse, or return alternatives, when the extracted span is ambiguous or scores
low. Same root cause as P0-3: greedy geocoding with no gate.

---

## P1 · reproduced, lower severity

| # | Finding | Repro | Status |
|---|---|---|---|
| P1-1 | No `outputSchema` on any of 105 tools, no `structuredContent` in responses | `tools/list` and inspect any descriptor | OPEN |
| P1-2 | Seven `memory_*` tools carry no service prefix; three are destructive and collide with Claude's own memory tool namespace | `tools/list \| grep '"name": "memory_'` | OPEN |
| P1-3 | `when_to_use` duplicated verbatim into both `annotations` and `description`, roughly doubling catalog size | measure a descriptor's bytes | OPEN |
| P1-4 | `signer` and `signature` returned as raw integer arrays where base32 is used elsewhere | any recall receipt | OPEN |
| P1-5 | Provenance class absent from the fact object even with `include:["provenance"]` | `emem_recall` and inspect a fact | OPEN |
| P1-6 | EUDR truncation is not priority-ordered; a partial DDS is emitted rather than refused | `emem_eudr_dds` over MCP vs REST | OPEN |
| P1-7 | Stale values carry no freshness signal: a 2026-05-23 temperature served today with `source_freshness_s: 0` | recall `weather.temperature_2m` | OPEN |
| P1-8 | Multi-fact returns have no ordering contract; 10 NDVI facts 0.326 to 0.855, no current marker | recall `indices.ndvi` at a warm cell | OPEN |
| P1-9 | A past `as_of_signed_at` triggers materialization that cannot satisfy it, then returns nothing | bound at `1999-01-01` | OPEN |
| P1-10 | Six POST endpoints have no `requestBody` schema in `openapi.json`, including `/v1/recall_polygon` | grep the spec | OPEN |
| P1-11 | Three of eight resource templates never resolve | `resources/templates/list` then read each | OPEN |
| P1-12 | `emem://band/{band_key}` rejects the qualified form (`indices.ndvi`) that tools and docs use | read that template | OPEN |
| P1-13 | A2A surface absent from `openapi.json` | grep for `/v1/a2a` | OPEN |
| P1-14 | Resources have no size guard while tools budget 24 KB | read `whitepaper.md` as a resource | OPEN |
| P1-15 | No read-path rate limiting | 12 rapid `/v1/recall` calls | OPEN |
| P1-16 | One log witness, last cosignature at tree_size 476131 while the tree is past 763000 | `GET /v1/log/witnesses` | OPEN |
| P1-17 | `llms.txt` carries no install line, and bare `emem` on PyPI is another vendor's memory library | read `llms.txt` | OPEN |
| P1-18 | `resources/list` returns templates as well; templates belong in `resources/templates/list` | `resources/list` | OPEN |
| P1-19 | `emem_backfill` is `readOnlyHint: true` but materializes and signs | inspect its annotations | OPEN |
| P1-20 | Undeclared parameters are accepted silently across tools | pass `{"fetch": 1}` to any tool | OPEN |
| P1-21 | Reads of unpinned-attester content carry no data-not-instructions marker | `memory_view` any note | OPEN |

---

## Verified clean

Worth recording so the criticism stays calibrated. These were probed and held:

- `query_region` aggregation is exact and honestly weighted: server
  `910.2979038994231` against an independent unweighted `910.2979736328125`,
  the difference fully explained by `cos_lat_weighting_applied: true`.
- `memory_bundle` holds its 38-character-at-any-N claim, with typed errors at 0
  and 257 triples.
- Bitemporal ordering is correct across four bounds.
- The A2A JSON-RPC surface is spec-conformant, including `-32001` for
  `TaskNotFound` and a correct refusal of `message/stream` with a pointer.
- The write path's teaching refusal works: the 401 returns `digest_hex` and
  `body_hash_hex`, an independently computed blake3 matches both byte for byte,
  and offline ed25519 authorship verification of a posted note passes.
- All 18 static resources resolve with correct MIME types; all 8 prompts execute
  and error correctly.
- Transport hardening: HSTS with preload, nosniff, hash-pinned CSP,
  `frame-ancestors` scoped to our own properties.
- `/v1/limits` separates enforced caps from measured ceilings and names the one
  ceiling that fails silently.

---

## The pattern

Two clusters, and both are structural rather than incidental.

**Input reaches execution without passing a declared schema.** P0-3, P0-5,
P1-10, P1-20. A cell64 has a shape and a regex for it exists in the tool schema;
nothing enforces it at the boundary.

**The MCP layer silently drops guarantees the REST layer keeps.** P0-1, P0-2,
P1-5, P1-6. MCP is the distribution surface, so that is backwards. P0-1's fix
is the template for the rest: one code path, called by both, rather than two
implementations that agree until they do not.
