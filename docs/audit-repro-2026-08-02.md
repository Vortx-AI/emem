# Audit reproductions, 2 August 2026

This page is a record. It gives one command for each finding of the
three-round external audit, and the output we saw when we ran it. Each entry
was a test that we did not have before.

The record text below keeps the state of 2 August 2026 and the fixes that
followed in August. The tool counts in it (104 and 105 tools) are the counts of
that time. The section [State on 7 November 2026](#state-on-7-november-2026)
gives the result of each command when we ran it again.

## How to read an entry

The `Status` of an entry has one of these values:

| Status | Meaning |
|---|---|
| `FIXED` | The command now gives the correct answer. The entry names the regression test. |
| `OPEN` | We reproduced the finding. We did not fix it yet. |
| `PARTIAL` | We fixed one part, and one part is open. The entry says which. |
| `NOT REPRODUCED` | The behaviour we saw is different from the claim. The entry says what we saw. |

We mark an entry fixed only when a command on this page shows the fix.

`$C` is `defi.zb493.xuqA.zcb5f` (Bengaluru) unless the entry says otherwise.
We cut each response down to the fields under test.

## State on 7 November 2026

We ran the commands again against https://emem.dev on 7 November 2026. The
script is a set of read-only calls. It sends no writes.

| Finding | Status in the record | Result on 7 November 2026 |
|---|---|---|
| P0-1 | FIXED | REST gives HTTP 400 `invalid_argument`. MCP gives `isError: true` with the same message. The MCP code is now `-25`, not `-24`. With `"provenance":["model_output"]`, REST returns 235 facts and echoes the filter. |
| P0-0 | FIXED | The array `polygon_bbox` gives HTTP 400 `invalid_argument`. The message names the object form and the two array orders. |
| X-1 | FIXED | `emem_ndvi` accepts the cell64 from `/v1/locate` (`defi.zb493.xuqA.zcb5f`) and returns `indices.ndvi`. |
| P0-3 | FIXED, one case open | All three bad inputs give `isError: true`, `no_geocoder_match`. The open case is also closed: the server now refuses `DROP TABLE facts` with the reason `query_label_overlap_below_floor_not_a_place`. `emem_recall {}` gives `-25` `invalid_argument: no location provided`. |
| P0-4 | FIXED | A genuine receipt gives `valid: true`. A receipt with `merkle_proof` removed gives `valid: false`. A receipt with the proof removed and `preimage_version` set to 1 gives `valid: false`. The reason for both is now `receipt_reshaped_after_signing`, not `signature_invalid`. |
| P0-5 | OPEN | Not reproduced. `/v1/ask` with "elevation of Bengaluru; also DROP TABLE facts" resolves to Bengaluru and answers 915.07 m. `LOCATE_RESOLVER_VERSION` is now 17 in the code. |
| P1-1 | PARTIAL | 16 of 116 tools declare `outputSchema`. |
| P1-2 | FIXED | 114 of 116 tools carry the `emem_` prefix. The two others are `search` and `fetch`. |
| P1-7 | FIXED | A recall of `weather.temperature_2m` at `$C` gives `receipt.cost.source_freshness_s` 14,512,094 over 235 facts. The value is not 0. |
| P1-8 | FIXED | The same recall gives `fact_order: "tslot_ascending"` and a `current_by_band` entry for the band. |
| P1-15 | FIXED | `EMEM_MATERIALIZE_PER_MIN` still has the default 600 in the code. |
| P1-16 | PARTIAL | `GET /v1/log/witnesses`: tree size 3,245,621. `head_is_witnessed` is false. One independent operator (`eudr.dev`) is 11 entries behind the head. |
| P1-19 | FIXED | 51 of 116 tools declare `readOnlyHint: false`. |

We did not run the other P1 entries again.

## P0-1: the deterministic and provenance filters did nothing over MCP

**Status: FIXED**. Test `recall_flag_parity_tests`. Commit `894f1f2`.

The README presents `deterministic` as the control that separates evidence
from model prose. On the MCP path the flag did nothing, and the call reported
success.

```bash
# REST: correct. The band is model_output, so a deterministic read excludes it.
curl -s -X POST https://emem.dev/v1/recall -H 'content-type: application/json' \
  -d '{"cell":"defi.zb493.xuqA.zcb5f","bands":["weather.temperature_2m"],"deterministic":true}'

# MCP: the same arguments
curl -s -X POST https://emem.dev/mcp -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"emem_recall",
       "arguments":{"cell":"defi.zb493.xuqA.zcb5f","bands":["weather.temperature_2m"],"deterministic":true}}}'
```

Before: REST gave `0 facts`. MCP gave `2 facts`. The first value was `24.0`
from met.no, and our own taxonomy classes met.no as `model_output`.

After: both refuse in the same way, and name the band and its class.

```
REST -> HTTP 400 invalid_argument
MCP  -> tool error (-24)
both -> "provenance filter [direct_sensor, deterministic_index] excludes every
         requested band: weather.temperature_2m (model_output)."
```

When the classes match, both return the same facts and echo the filter:

```
provenance:["model_output"]  ->  REST 2 facts, filter=['model_output']
                                 MCP  2 facts, filter=['model_output']
```

**Cause.** `From<RecallApiReq> for RecallReq` left `provenance: None`, because
`post_recall` set the field later, inline. The MCP arm called `.into()` and
went directly to the primitive. One behaviour had two code paths.

**Fix.** We extracted `recall_req_with_provenance`, and both paths call it. A
repair of the MCP arm only would have kept the drift that caused the bug.

**Regression tests.** `deterministic_true_narrows_provenance`,
`an_explicit_provenance_list_is_carried_through`,
`absent_flags_leave_the_recall_unfiltered`,
`an_unknown_provenance_class_is_refused`.

## P0-2: a truncated field was truthy, so an absent verdict looked present

**Status: FIXED**. Test `truncation_falsy_tests`.

The MCP wire budget is 24 KB, and `emem_eudr_dds` returns about 120 KB. The
compliance verdict was one of the fields that the slimmer dropped. The slimmer
put a descriptive object in place of each dropped field, and that object is
truthy.

```bash
curl -s -X POST https://emem.dev/mcp -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"emem_locate",
       "arguments":{"place":"Bangalore"}}}' \
| python3 -c "import json,sys;t=json.loads(json.load(sys.stdin)['result']['content'][0]['text']);
print(t['_emem_truncation']['omitted_fields'][0])"
```

Before: `{"field":"data_at_this_cell","stub":{"_keys":11,"_kind":"object","_omitted":true}}`,
and the field itself held that stub. The test `if result["statement_of_compliance"]:`
passes on the stub. So a caller could report a plot as compliant when no
verdict existed.

After: the field is `null`, which is falsy in every client language. The
description moved to `_emem_truncation.omitted_fields[].stub`. The key stays
in the response, so shape checks still find it.

**Still open on the same tool.** The slimmer does not drop fields in priority
order. So it sheds the verdict because of its size, not its importance, and it
sends a partial DDS. See P1-6.

## P0-0: the server read a `polygon_bbox` array by position and sized a 433 GiB window

**Status: FIXED**. Tests `bbox_array_form_is_refused_with_the_object_form_named`,
`bbox_corners_outside_the_earth_are_refused`,
`window_cap_admits_real_callers_and_rejects_the_dos_window`. Commit `6e9bfb4`.

This finding was more serious than all the others. One request with no
authentication stopped the whole node until a restart. We kept it out of the
public channel while it was live. We reproduced it here on an isolated
instance, not a third time against production.

```bash
curl -s -X POST https://emem.dev/v1/recall_polygon -H 'content-type: application/json' \
  -d '{"polygon_bbox":[12.96,77.58,12.99,77.61],"bands":["copdem30m.elevation_mean"]}'
```

Before: the connection dropped with `memory allocation of 432952345800 bytes failed`.
The process aborted, and no read worked until a restart.

After: REST gives `HTTP 400` and MCP gives `-32602`. Both name the object
form. The process serves the next request.

**The cause was not the assumed one, and this changes the fix.**
`#[derive(Deserialize)]` on a struct accepts a JSON array. It binds the array
items to the fields in *declaration order*. A caller wrote the usual
`[min_lat, min_lng, max_lat, max_lng]` as `[12.96, 77.58, 12.99, 77.61]`. The
binding gave `max_lat = 77.58`, a box of 64.62° of latitude, not 0.03°. The
polygon prewarm then sized a COG window from that span with no limit. It asked
for 232 635 × 232 635 px.

The allocator refuses an allocation of that size and calls
`handle_alloc_error`, which calls `abort()`. **This is not a panic.**
`catch_unwind` cannot stop it, and no middleware can stop it. So a panic-catch
layer, the proposed fix, would not have changed this case. The fix must put a
limit at the place of the allocation. `cog::sample_window` now refuses a window
larger than `MAX_WINDOW_PX` (4096², 128 MiB of `f64`) before it allocates. This
limit makes the other 104 tools safe, not only this endpoint.

**The silent half is worse than the crash.** Sometimes the wrong box stayed
small enough for the process to survive. Then the request returned signed facts
about a region that the caller did not name, with no warning. Our own two bbox
types also had different field orders. `CellsBBox` is lat,lng,lat,lng and
`RecallPolygonBbox` is lat,lat,lng,lng. Both endpoints accept the same
`polygon_bbox` key, so one array addressed two different regions.

For this reason the server refuses the array form and does not reorder it.
GeoJSON and OGC write `[west,south,east,north]`. Nominatim answers
`[south,north,west,east]`. Any order that we chose would silently read a place
that some callers did not mean. Every `inputSchema` already declared
`"type":"object"`. The deserializer accepted more than the documents said.

We added `CatchPanicLayer` too. It covers the usual unwrap-on-`None` class
across the tool surface. A comment at the call site says what it does not
cover.

## X-1: `emem_locate` made an address that eight tools refused

**Status: FIXED**. Test `a_cell64_from_locate_is_accepted_where_a_place_is`.

An external party reported this finding, and we reproduced it before the fix.
It is not from the three audit rounds. We put it here because it is in the same
class: the surface makes a claim and does not keep it.

```bash
CELL=$(curl -s -X POST https://emem.dev/v1/locate -H 'content-type: application/json' \
  -d '{"place":"Bengaluru"}' | python3 -c "import json,sys;print(json.load(sys.stdin)['cell64'])")
# defi.zb493.xuqA.zcb5f

curl -s -X POST https://emem.dev/mcp -H 'content-type: application/json' \
  -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{
       \"name\":\"emem_ndvi\",\"arguments\":{\"cell\":\"$CELL\"}}}"
```

Before: `tool error (-24): supply (lat, lon|lng) or place`.

`emem_locate` turns a place into a cell64. Eight tools took only `place`, or
`lat` and `lng`: `emem_ndvi`, `emem_at`, `emem_air`, `emem_lst`, `emem_soil`,
`emem_water`, `emem_forest` and `emem_weather`. They refused the address that
locate had just returned. So the loop that the protocol advertises did not
close for these tools. A caller who used the grounded address got a worse
result than a caller who sent free text.

After: all eight accept the cell64. We checked this end to end from a live
`emem_locate` call. The tool decodes the cell locally and does not send it to
the geocoder again. So it makes no upstream call, and it cannot resolve to a
different place. `via` reports `input_cell64`, and the confidence is
`direct_cell64`. The area settings (`radius_m`, `n_cells`) work as they do for
lat and lng.

The tools refuse a string in `cell` that does not have the cell64 shape, and do
not geocode it. So this fix does not open P0-3 again. `"not-a-cell"` and
`"DROP TABLE facts"` both return a typed error that names the field.

## P0-3: `cell` accepted any string, geocoded it and made a permanent fact

**Status: FIXED** (was PARTIAL). Tests `ambiguous_place_gate_tests`,
`query_label_overlap_catches_confident_mismatch`,
`confidence_floor_only_demotes_and_only_on_fuzzy_tiers`. Commit `ff7ac83`.

We closed the second half of the finding: a string that resolved to ONE
*confident* wrong place. `"DROP TABLE facts"` matched "La Table Ronde" with
importance 0.6. No floor on importance could catch this case. Importance
measures how prominent the MATCHED FEATURE is, not how well it answers the
query. That quarter of Bourg-lès-Valence is in fact notable. The missing signal
was how much of the query the returned label explains. If the label explains
less than half of the substantive tokens, a fuzzy match is no longer high
confidence.

`/v1/ask` had a separate check: it made sure that the resolved cell64 had the
correct SHAPE. That check does not show that the geocoder found the correct
place. So recall refused one input, and ask answered it with confidence on the
surface that a human reads. Now both refuse it.

```
recall {"cell":"DROP TABLE facts"}  ->  no_geocoder_match (query_label_overlap_below_floor)
ask "elevation of Bengaluru; also DROP TABLE facts"  ->  "could not extract a location"
ask "what is the elevation of Bengaluru"  ->  918 m, unaffected
```

We changed `LOCATE_RESOLVER_VERSION` from 3 to 4. Without that change, the
stored verdict replays from the cache for the full 30-day TTL, and the fix
looks as if it did not deploy.

The original report used this command:

```bash
for bad in "not-a-cell" "DROP TABLE facts" "../../etc/passwd"; do
  curl -s -X POST https://emem.dev/mcp -H 'content-type: application/json' \
    -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"emem_recall\",
         \"arguments\":{\"cell\":\"$bad\",\"bands\":[\"indices.ndvi\"]}}}"
done
```

We saw the same result for all three: `isError: false`, signed facts in the
response, and no field that said a geocode occurred. `../../etc/passwd` made a
new signed fact.

This had three results, in order of severity:

1. An agent that passes the wrong variable gets a confident signed answer about
   the wrong place. The tool built to stop referential drift caused it.
2. The read path writes. Junk input adds an attestation to an append-only
   RFC 6962 log, which cannot be pruned by design. The read side has no
   authentication and no rate limit for this.
3. It contradicts the README claim "20 of 20 refusals name the missing field".

**Fixed.** `resolve_cell_field` computed the confidence triple of the geocoder
and then ignored it. Now it refuses when the geocoder reports that the match is
not high confidence. The refusal names the reason, the best candidate and the
remedy. The gate is the verdict of the geocoder itself. We did not add a
heuristic here.

We measured this live after the fix:

| input | before | after |
|---|---|---|
| `not-a-cell` | 2 facts, "Buvette club de Football Pont-a-Celles" | refused, `ambiguous_top_two_candidates` |
| `../../etc/passwd` | 1 fact, "Passadumkeag, Maine" | refused, `ambiguous_top_two_candidates` |
| `Springfield` | 1 fact, silently "Springfield, MO US" | refused, candidates offered |
| `Nashik` | resolved | resolved, `admin3_region_match` |
| `Bengaluru` | resolved | resolved, `embedded_gazetteer_hit` |
| `Mount Everest` | resolved | resolved, `well_known_poi_match` |

The refusal of `Springfield` is correct and is not a regression. `emem_locate`
already marked it `disambiguation_required`, and the read path ignored that.

**Still open on 2 August.** `DROP TABLE facts` resolves to "La Table Ronde"
with `is_high_confidence: true` (`geocoder_high_importance`). A substring of
the query is a real toponym of high importance. Confidence cannot separate this
from a genuine query. Only the field contract can. The remaining fix is to make
`cell` accept only a cell64 and move place names to the `place` parameter. That
is a breaking change to the most-called tool, so it needs its own release with
a deprecation period. Until then, `resolved_from` shows the substitution. On
7 November 2026 the server refuses this input (see the state table).

**Also fixed with this change.** `emem_recall {}` now returns
`tool error (-24) no location provided: pass 'cell' ...`, not a 200. The
empty-cell check reached MCP through the P0-1 helper.

## P0-4: `valid: true` on a receipt whose Merkle proof fails

**Status: FIXED** (was OPEN until a migration). Tests
`v2_detects_a_stripped_merkle_proof`, `v2_receipt_downgraded_to_v1_does_not_verify`,
`verify_receipt_detects_stripped_or_forged_proof`, `v2_merkle_binding_vectors`.
Commit `ff7ac83`.

This entry is the fix. The original filing follows it.

A foreign proof already failed. The open case was STRIPPING. When a caller
deleted `merkle_proof`, the receipt reported `valid: true` and
`merkle_proof_valid: null`. This was a downgrade by removal, and it left no
trace.

The server attached the proof AFTER it signed the receipt, so the signature
could not cover the proof. That order was the bug. The reverse order makes the
binding possible.

`preimage_version` 2 adds a MERKLE segment over (root, leaf_index, path,
rule_version). **When there is no proof, the segment hashes an explicit ABSENT
marker. It does not skip the segment.** If the encoding showed absence by
omission, a deleted proof would hash the same as a receipt that never had one.
Then stripping would stay invisible, as it was in v1. A v2 receipt states under
its signature "here is my proof" or "I have none".

A downgrade needs no separate defence. If a caller relabels a v2 receipt as v1,
the verifier rebuilds the preimage without the MERKLE segment. The result is a
digest that the responder did not sign.

We saw this against production after the deploy:

```
genuine v2 receipt          valid=True  signature_valid=True
merkle_proof deleted        valid=False signature_valid=False  reason=signature_invalid
deleted + version downgraded valid=False signature_valid=False  reason=signature_invalid
```

v1 receipts still verify byte for byte under the v1 rules. The JavaScript copy
of the verifier on the `/verify` page also learned v2. We checked its
cross-language vectors against an independent reimplementation, then froze them
in `emem-attest`. The new "verifier drift" state reports a silent difference
between the two verifiers.

## P0-4, original filing: `valid: true` on a receipt whose Merkle proof fails

**Status at filing: FIXED for a failing proof, OPEN for a stripped one**

The receipt signature did not cover `merkle_proof`.

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

We saw this:

| receipt | `valid` | `merkle_proof_valid` |
|---|---|---|
| untouched | `true` | `true` |
| `merkle_proof` removed | `true` | `null` |
| `merkle_proof` swapped from another receipt | `true` | **`false`** |

The third row was the dangerous one. A receiver that checked `valid` accepted a
receipt whose log-inclusion proof failed. Any party in transport could strip
the proof and downgrade the receipt silently.

The core ed25519 cryptography was sound. A change to `served_at`, `cells`,
`fact_cids` or `primitive`, or one flipped byte in the signature, made the
check fail. Only the log binding had no authentication.

**Fixed: `valid` is now a conjunction.** Before, `valid` was the same as
`signature_valid`. `merkle_proof_valid` came beside it as separate advice. So a
receipt with a failed proof still gave `valid: true`, and a receiver that
checked `valid` accepted it. We measured this after the fix:

| receipt | `valid` | `merkle_proof_valid` | `reason` |
|---|---|---|---|
| untouched | `true` | `true` | none |
| proof swapped from another receipt | **`false`** | `false` | `merkle_proof_invalid` |
| proof removed | `true` | `null` | none |

**Still open at filing: a stripped proof.** Row three did not change, and this
fix could not change it. The signature did not cover `merkle_proof`. So a proof
removed in transport left `merkle_proof_valid: null`. A receiver could not tell
this from a receipt that never had a proof (facts older than the proof tree).

The fix needed the proof inside the signed preimage. That means a
`receipt_preimage_v2` and a new `preimage_version`. The server must compute the
proof before it signs, not attach it after. Every receipt issued under v1 must
verify under the v1 rule forever. This is a protocol change with a migration,
on the guarantee that the rest of the system depends on. So we made it a
separate piece of work, not part of a patch. The entry above records that work.

## P0-5: `emem_ask` extracted the wrong entity with confidence

**Status: OPEN** (partly mitigated). On 7 November 2026 we did not reproduce
it (see the state table).

```bash
curl -s -X POST https://emem.dev/v1/ask -H 'content-type: application/json' \
  -d '{"q":"elevation of Bengaluru; also DROP TABLE facts"}'
```

Reported: `place_resolved.input = "DROP TABLE"`, label
`"La Table Ronde - est, Bourg-lès-Valence, France"`, and the answer `115.00 m`
with 12 signed `fact_cids`. The response had no caveat, no alternatives and no
confidence field.

`emem_locate` sets `disambiguation_required: true` for an ambiguous name, so
the mechanism exists. `emem_ask` did not use it.

**Partly mitigated on 31 July 2026.** The prepositional-anchor window now stops
at a clause boundary (`LOCATE_RESOLVER_VERSION` 3). This fixes the class
"around X right now, and what does it mean". The confidence gate was still
missing, and that gate is the general fix.

**Fix direction.** One confidence gate for `recall`, `ask` and `locate`. It
refuses, or returns alternatives, when the extracted span is ambiguous or has a
low score. The root cause is the same as P0-3: greedy geocoding with no gate.

## P1: findings we reproduced, with lower severity

| # | Finding | Repro | Status |
|---|---|---|---|
| P1-1 | No `outputSchema` on any of 105 tools, no `structuredContent` in responses | `tools/list`, then inspect any descriptor | **PART NOT REPRODUCED, rest PARTIAL**. Every result whose two-copy envelope fits the wire budget DOES carry `structuredContent`. 8 tools now declare `outputSchema`. We checked their live output with a Draft-7 validator: 8 conform, 0 fail. A tool that declares an `outputSchema` keeps its copy and is not slimmed, so the descriptor stays true. Tools that exceed the budget on every call declare no schema. A declaration would force the server to cut a real answer to carry a second copy. |
| P1-2 | Seven `memory_*` tools have no service prefix. Three are destructive, and the names collide with the namespace of Claude's own memory tool | `tools/list`, then look for a name without `emem_` | **FIXED**. We renamed them to `emem_memory_*`, so 105 of 105 advertised tools had the prefix. The bare names still dispatch until 3.0, so no caller breaks during a session. Signatures do not change: the attester preimage binds a bare verb (`create`, `delete`), never the tool name. |
| P1-3 | `when_to_use` appears word for word in both `annotations` and `description`, which almost doubles the catalog size | measure the bytes of a descriptor | **FIXED**. The text was in the description on 105 of 105 tools already. The copy cost 44,983 bytes (15.9% of a 282,896-byte catalog). We removed it from `annotations`, and moved `category`, `level` and `tier` to `_meta`. Now `annotations` holds only the hint set of the specification. |
| P1-4 | `signer` and `signature` come back as raw integer arrays, but other places use base32 | any recall receipt | **FIXED**. Receipts now carry `responder_pubkey_b32` and `signature_b32` next to the arrays. The arrays stay, because verifiers (the /verify JavaScript is one) already read them. The server omits both new fields when they are empty, so a receipt decoded from an older payload round-trips byte for byte. |
| P1-5 | The fact object has no provenance class, even with `include:["provenance"]` | `emem_recall`, then inspect a fact | **FIXED**. `provenance` was not a known `include` token, so it silently did nothing. Now it is, and it attaches the class, deterministic, tamper_evidence, trust_rank and the caution to each fact. `deterministic:true` and the `provenance` filter select on this class. Before, a caller could filter by class and not learn which class remained. Like freshness, it comes after the receipt, so the signed preimage does not change. `bands_cid` already attests the class. |
| P1-6 | EUDR truncation is not in priority order. The server sends a partial DDS and does not refuse | `emem_eudr_dds` over MCP and over REST | **FIXED**. Over MCP, the server now REFUSES an oversized due-diligence statement and names `POST /v1/eudr_dds`, which has no wire budget. The verdict was already on the never-drop list, and that is the dangerous shape. A DDS cut down to its verdict still reads like a statement, but the plots, evidence and caveats that qualify it are gone. Someone can file it or rely on it. Truncation is correct for an answer and wrong for a document. Other tools of the same size still slim. |
| P1-7 | Old values have no freshness signal: a temperature from 2026-05-23 served today with `source_freshness_s: 0` | recall `weather.temperature_2m` | **FIXED**. The signer had a hardcoded `0`, under a doc comment that promised "age of the stalest source". The server now measures it from the `captured_at` of the returned facts. It is `null` (never 0) when the facts have no date. A 2021 DEM tile reports 166,192,340 s (5.27 years). Weather reports 1,955 s. Both read 0 before. |
| P1-8 | A response with many facts has no order contract: 10 NDVI facts from 0.326 to 0.855, and no current marker | recall `indices.ndvi` at a warm cell | **FIXED**. `fact_order: "tslot_ascending"` states the contract that was only implied before. `current_by_band` names the `fact_cid` with the highest tslot for each band. Facts with no slot (`tslot: 0`) are not candidates. Zero means undated, not oldest, but it sorts to the front, where it looks like the earliest observation. |
| P1-9 | A past `as_of_signed_at` starts a materialization that cannot satisfy it, then returns nothing | bound at `1999-01-01` | **FIXED** `ff7ac83`. The server skips it with a typed note. A fact fetched now is signed now, so the write could never answer the query that caused it. |
| P1-10 | Six POST endpoints have no `requestBody` schema in `openapi.json`, `/v1/recall_polygon` is one | grep the specification | **FIXED** `ff7ac83`. All six now declare one. The one POST with no body is `/cancel`. It takes no body, and the specification says so. |
| P1-11 | Three of eight resource templates never resolve | `resources/templates/list`, then read each | **FIXED** `ff7ac83`. The state was available one frame up. The router sent the URIs to a function that did not take the state. |
| P1-12 | `emem://band/{band_key}` refuses the qualified form (`indices.ndvi`) that tools and documents use | read that template | **FIXED** `ff7ac83`. The registry uses the family as its key, so the one spelling that a caller had was the one refused. |
| P1-13 | `openapi.json` has no A2A surface | grep for `/v1/a2a` | **FIXED** `ff7ac83`. We added six paths. Before, a caller could not find in the machine contract the front door that the .well-known descriptor advertises. |
| P1-14 | Resources have no size guard, but tools have a 24 KB budget | read `whitepaper.md` as a resource | **FIXED**. `emem://docs/whitepaper.md` was 93,945 bytes against a 24,000-byte budget, through the same transport to the same hosts that truncate silently. `resources/read` now obeys the tool budget. It cuts on a line boundary and names the URL of the full document in `_emem_truncation`. |
| P1-15 | No rate limit on the read path | 12 rapid `/v1/recall` calls | **FIXED, in a different way than filed**. Reads DO have a limit per IP, but at a burst of 1200 requests, so 15 rapid calls pass. A request count is the wrong unit. A warm recall is a cache lookup. A cold recall costs an upstream fetch and adds to a log that cannot be pruned. A ceiling on cold materializations for the whole responder (`EMEM_MATERIALIZE_PER_MIN`, default 600) now limits this growth. A read above the ceiling does not fail. Warm facts come back as usual, and the cold part returns a typed deferral. |
| P1-16 | One log witness. Its last cosignature is at tree_size 476131, and the tree is past 763000 | `GET /v1/log/witnesses` | **PARTIAL** `ff7ac83`. The surface no longer implies that the head is fresh (`head_is_witnessed`, and `entries_behind_current` for each witness). To fix the staleness, an independent party must co-sign. If we made such a party ourselves, it would defeat the purpose. |
| P1-17 | `llms.txt` has no install line, and the bare name `emem` on PyPI is a memory library from another vendor | read `llms.txt` | **FIXED** `51d8f8e`. The file gives `pip install ememdev` and `npm i @vortxai/emem`, and says why a caller cannot guess either name. `/llms-full.txt` includes it. |
| P1-18 | `resources/list` also returns templates, but templates belong in `resources/templates/list` | `resources/list` | **FIXED**. `resources/list` returns only concrete resources. A caller cannot read a URI template, so a template in that list invites a host to try. |
| P1-19 | `emem_backfill` declares `readOnlyHint: true`, but it materializes and signs | inspect its annotations | **FIXED**. 10 tools whose descriptions say that they mint, sign or persist now declare `readOnlyHint: false`. `destructiveHint: false` with `idempotentHint: true` means "safe to auto-approve", which is the purpose of these hints. The test `no_tool_claims_read_only_while_authoring_state` enforces it. |
| P1-20 | Tools accept undeclared parameters silently | send `{"fetch": 1}` to any tool | **FIXED**. A name that the inputSchema of the tool does not declare comes back under `_unrecognised_arguments`, with a note that the tool ignored it. serde drops unknown fields. So `{"celll": ...}` ran on a default value as if the caller sent nothing, and the answer looked correct for the wrong thing. The server reports the name and does not refuse the call, because a refusal would break hosts that add their own arguments. `_meta` is exempt, because it is the slot of the host. |
| P1-21 | Reads of content from an attester that is not pinned have no data-not-instructions marker | `memory_view` on any note | **FIXED**. `memory_view` and `memory_search` put `_content_is_data_not_instructions` before `content`. The marker is on ALL content that agents write. A marker that appears only sometimes teaches readers that its absence is an endorsement. |

## Checks that passed

We record these so that the criticism stays calibrated. The auditors probed
these points and they held:

- `query_region` aggregation is exact and has correct weights. The server gave
  `910.2979038994231` and an independent unweighted mean gave
  `910.2979736328125`. `cos_lat_weighting_applied: true` explains all of the
  difference.
- `memory_bundle` keeps its claim of 38 characters at any N. It gives typed
  errors at 0 and at 257 triples.
- Bitemporal ordering is correct across four bounds.
- The A2A JSON-RPC surface conforms to the specification. It gives `-32001`
  for `TaskNotFound`, and refuses `message/stream` correctly with a pointer.
- The refusal on the write path teaches the caller. The 401 returns
  `digest_hex` and `body_hash_hex`. An independent BLAKE3 matched both byte for
  byte, and an offline ed25519 check of the author of a posted note passed.
- All 18 static resources resolve with the correct MIME types. All 8 prompts
  run and give correct errors.
- Transport hardening: HSTS with preload, nosniff, a CSP pinned by hash, and
  `frame-ancestors` limited to our own properties.
- `/v1/limits` separates enforced caps from measured ceilings, and names the
  one ceiling that fails silently.

## The pattern in the findings

The findings fall into two clusters. Both come from the structure of the code,
not from chance.

**Input reaches execution without a check against a declared schema.** P0-3,
P0-5, P1-10, P1-20. A cell64 has a shape, and the tool schema has a regex for
it. Nothing enforced the regex at the boundary.

**The MCP layer silently drops guarantees that the REST layer keeps.** P0-1,
P0-2, P1-5, P1-6. MCP is the distribution surface, so this is the wrong way
round. The fix for P0-1 is the template for the others: one code path that both
layers call. Two separate copies of the code agree only until they do not.
