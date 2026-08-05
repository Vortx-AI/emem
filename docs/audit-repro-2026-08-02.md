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

## P0-0 · a `polygon_bbox` array was read positionally, sizing a 433 GiB window

**Status: FIXED** · tests `bbox_array_form_is_refused_with_the_object_form_named`,
`bbox_corners_outside_the_earth_are_refused`,
`window_cap_admits_real_callers_and_rejects_the_dos_window` · commit `6e9bfb4`

The one finding that outranked everything else: unauthenticated, one request,
whole node down until restart. Withheld from the public channel while it was
live, and reproduced here in an isolated instance rather than a third time
against production.

```bash
curl -s -X POST https://emem.dev/v1/recall_polygon -H 'content-type: application/json' \
  -d '{"polygon_bbox":[12.96,77.58,12.99,77.61],"bands":["copdem30m.elevation_mean"]}'
```

Before: connection dropped, `memory allocation of 432952345800 bytes failed`,
process aborted, every read dark until restart.

After: `HTTP 400` on REST, `-32602` on MCP, both naming the object form. The
process serves the next request.

**The cause was not the one assumed, and this matters for the fix.**
`#[derive(Deserialize)]` on a struct accepts a JSON array and binds it to the
fields in *declaration order*. So `[12.96, 77.58, 12.99, 77.61]`, a caller
writing the ordinary `[min_lat, min_lng, max_lat, max_lng]`, bound
`max_lat = 77.58`: a box spanning 64.62° of latitude instead of 0.03°. The
polygon prewarm then sized a COG window from that span with no ceiling and
asked for 232 635 × 232 635 px.

An allocation that size is refused by the allocator, which calls
`handle_alloc_error` → `abort()`. **That is not a panic.** `catch_unwind`
cannot intercept it and neither can any middleware, so the panic-catch layer
proposed as the fix would have left this exactly as it was. The class has to
be bounded where it is allocated. `cog::sample_window` now refuses a window
over `MAX_WINDOW_PX` (4096², 128 MiB of `f64`) before allocating, which is
what makes the other 104 tools safe rather than just this endpoint.

**The quiet half is worse than the crash.** Where the mangled box stayed small
enough to survive, the request returned signed, confident facts about a region
the caller never named, and said nothing. Our own two bbox types disagreed on
field order (`CellsBBox` is lat,lng,lat,lng against `RecallPolygonBbox`'s
lat,lat,lng,lng), so one array literal addressed two different regions across
two endpoints that accept the same `polygon_bbox` key.

The array form is therefore refused rather than reordered. GeoJSON/OGC write
`[west,south,east,north]`; Nominatim answers `[south,north,west,east]`. Any
convention picked here would silently read somewhere the caller did not mean.
Every `inputSchema` already declared `"type":"object"`; the deserializer was
the thing accepting more than we documented.

`CatchPanicLayer` was added anyway, for the ordinary unwrap-on-`None` class
across the tool surface, and documented at the call site for what it does not
cover.

---

## P0-3 · `cell` accepts any string, geocodes it, and mints a permanent fact

**Status: FIXED** (was PARTIAL) · tests `ambiguous_place_gate_tests`, `query_label_overlap_catches_confident_mismatch`, `confidence_floor_only_demotes_and_only_on_fuzzy_tiers` · commit `ff7ac83`

The remaining half, a string that resolved to ONE *confident* wrong place,
is closed. `"DROP TABLE facts"` matched "La Table Ronde" with importance 0.6,
and no floor on importance could ever have caught it: importance scores how
prominent the MATCHED FEATURE is, not how well it answers the question, and
that quarter of Bourg-lès-Valence genuinely is notable. The missing signal was
how much of the query the returned label accounts for. Below half the
substantive tokens, a fuzzy hit is no longer high confidence.

`/v1/ask` was separately checking that the resolved cell64 was well-SHAPED,
which says nothing about whether the right place was found, so one input was
refused by recall and answered confidently by ask, the surface a human reads.
Both now refuse.

```
recall {"cell":"DROP TABLE facts"}  ->  no_geocoder_match (query_label_overlap_below_floor)
ask "elevation of Bengaluru; also DROP TABLE facts"  ->  "could not extract a location"
ask "what is the elevation of Bengaluru"  ->  918 m, unaffected
```

LOCATE_RESOLVER_VERSION 3 → 4, or the stored verdict replays from cache for the
full 30 d TTL and the fix looks like it never deployed.

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

**Fixed.** `resolve_cell_field` computed the geocoder's confidence triple and
then discarded it. It now refuses when the geocoder itself reports the match is
not high-confidence, naming the reason, the best candidate and the remedy. The
gate is the geocoder's own verdict, not a heuristic added here.

Measured live, after:

| input | before | after |
|---|---|---|
| `not-a-cell` | 2 facts, "Buvette club de Football Pont-a-Celles" | refused, `ambiguous_top_two_candidates` |
| `../../etc/passwd` | 1 fact, "Passadumkeag, Maine" | refused, `ambiguous_top_two_candidates` |
| `Springfield` | 1 fact, silently "Springfield, MO US" | refused, candidates offered |
| `Nashik` | resolved | resolved, `admin3_region_match` |
| `Bengaluru` | resolved | resolved, `embedded_gazetteer_hit` |
| `Mount Everest` | resolved | resolved, `well_known_poi_match` |

Refusing `Springfield` is the correct behaviour rather than a regression:
`emem_locate` already flagged it `disambiguation_required`, and the read path
was ignoring that.

**Still open, and stated rather than hidden.** `DROP TABLE facts` resolves
`is_high_confidence: true` (`geocoder_high_importance`) to "La Table Ronde",
because a substring is a real high-importance toponym. Confidence cannot
separate that from a genuine query; only the field contract can. The remaining
fix is to make `cell` mean cell64 and move place names to the existing `place`
parameter, which is a breaking change to the most-called tool on the surface
and belongs in its own release with a deprecation window. `resolved_from`
discloses the substitution in the meantime.

**Already fixed in passing:** `emem_recall {}` now returns
`tool error (-24) no location provided: pass 'cell' ...` rather than a 200. The
empty-cell check reached MCP with the P0-1 helper.

---

## P0-4 · `valid: true` on a receipt whose Merkle proof fails

**Status: FIXED** (was OPEN, held for a migration) · tests
`v2_detects_a_stripped_merkle_proof`, `v2_receipt_downgraded_to_v1_does_not_verify`,
`verify_receipt_detects_stripped_or_forged_proof`, `v2_merkle_binding_vectors` ·
commit `ff7ac83`

The foreign-proof case already failed. The one still open was STRIPPING: delete
`merkle_proof` and the receipt reported `valid: true`, `merkle_proof_valid: null`
a downgrade by removal, with no trace.

The proof was attached AFTER signing, so the signature could not cover it. That
ordering is the bug; reversing it is what makes binding possible.

`preimage_version` 2 adds a MERKLE segment over (root, leaf_index, path,
rule_version). **Absence hashes an explicit ABSENT marker rather than skipping
the segment.** If absence were encoded by omission, a deleted proof would hash
identically to a receipt that never had one and stripping would stay invisible:
the v1 behaviour being replaced. A v2 receipt states, under signature, either
"here is my proof" or "I have none".

Downgrade needs no separate defence: relabelling a v2 receipt as v1 sends the
verifier down a rebuild that omits the MERKLE segment, producing a digest the
responder never signed.

Observed against production after deploy:

```
genuine v2 receipt          valid=True  signature_valid=True
merkle_proof deleted        valid=False signature_valid=False  reason=signature_invalid
deleted + version downgraded valid=False signature_valid=False  reason=signature_invalid
```

v1 receipts keep verifying byte-for-byte under v1 rules. The `/verify` page's JS
mirror learns v2 too, and its cross-language vectors are frozen in `emem-attest`
after checking them against an independent reimplementation. A silent
divergence there is exactly what the new "verifier drift" state reports.

---

## P0-4 (original filing) · `valid: true` on a receipt whose Merkle proof fails

**Status: FIXED for a failing proof, OPEN for a stripped one**

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

**Fixed: `valid` is now a conjunction.** It was `signature_valid` alone, with
`merkle_proof_valid` reported beside it as separate advice, so a receipt whose
proof demonstrably failed still came back `valid: true` and a receiver checking
the field named `valid` accepted it. Measured after:

| receipt | `valid` | `merkle_proof_valid` | `reason` |
|---|---|---|---|
| untouched | `true` | `true` | none |
| proof swapped from another receipt | **`false`** | `false` | `merkle_proof_invalid` |
| proof removed | `true` | `null` | none |

**Still open: a stripped proof.** Row three is unchanged and cannot be fixed
this way. The signature does not cover `merkle_proof`, so a proof removed in
transport leaves `merkle_proof_valid: null`, which is indistinguishable from a
receipt that legitimately never carried one (facts predating the proof tree).

Closing it means binding the proof into the signed preimage, which is a
`receipt_preimage_v2` and a `preimage_version` bump: the proof must be computed
before signing rather than attached after, and every receipt already issued
under v1 has to keep verifying under the v1 rule forever. That is a protocol
change with a migration, on the one guarantee the rest of the system rests on,
and it is the next piece of work rather than something to append to a patch.

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
| P1-9 | A past `as_of_signed_at` triggers materialization that cannot satisfy it, then returns nothing | bound at `1999-01-01` | **FIXED** `ff7ac83`: skipped with a typed note; a fact fetched now is signed now, so the write could never answer the query that caused it |
| P1-10 | Six POST endpoints have no `requestBody` schema in `openapi.json`, including `/v1/recall_polygon` | grep the spec | **FIXED** `ff7ac83`: all six declared; the one POST still without a body is `/cancel`, which takes none, and says so |
| P1-11 | Three of eight resource templates never resolve | `resources/templates/list` then read each | **FIXED** `ff7ac83`: the state was available one frame up; the URIs were routed to a function that did not take it |
| P1-12 | `emem://band/{band_key}` rejects the qualified form (`indices.ndvi`) that tools and docs use | read that template | **FIXED** `ff7ac83`: the registry is keyed by family, so the one spelling a caller holds was the one refused |
| P1-13 | A2A surface absent from `openapi.json` | grep for `/v1/a2a` | **FIXED** `ff7ac83`: six paths added; the front door the .well-known descriptor advertises was undiscoverable from the machine contract |
| P1-14 | Resources have no size guard while tools budget 24 KB | read `whitepaper.md` as a resource | OPEN |
| P1-15 | No read-path rate limiting | 12 rapid `/v1/recall` calls | OPEN |
| P1-16 | One log witness, last cosignature at tree_size 476131 while the tree is past 763000 | `GET /v1/log/witnesses` | **PARTIAL** `ff7ac83`: the surface no longer implies freshness (`head_is_witnessed`, `entries_behind_current` per witness). The staleness itself needs an independent party to co-sign; manufacturing one would defeat the purpose |
| P1-17 | `llms.txt` carries no install line, and bare `emem` on PyPI is another vendor's memory library | read `llms.txt` | **FIXED** `51d8f8e`: `pip install ememdev` / `npm i @vortxai/emem` with why both names are unguessable; `/llms-full.txt` concatenates it |
| P1-18 | `resources/list` returns templates as well; templates belong in `resources/templates/list` | `resources/list` | OPEN |
| P1-19 | `emem_backfill` is `readOnlyHint: true` but materializes and signs | inspect its annotations | **FIXED**: 10 tools whose own descriptions say they mint, sign or persist now declare `readOnlyHint: false`. `destructiveHint: false` + `idempotentHint: true` carry "safe to auto-approve", which is what that vocabulary is for. Enforced by `no_tool_claims_read_only_while_authoring_state` |
| P1-20 | Undeclared parameters are accepted silently across tools | pass `{"fetch": 1}` to any tool | OPEN |
| P1-21 | Reads of unpinned-attester content carry no data-not-instructions marker | `memory_view` any note | **FIXED**: `_content_is_data_not_instructions` on `memory_view` and `memory_search`, emitted before `content`. Marked on ALL agent-authored content, since a marker that appears only sometimes teaches readers to treat its absence as endorsement |

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
