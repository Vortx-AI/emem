# Errors

Every error response is a small JSON object with a stable `code`, a
human-readable `message`, and (where useful) a `hint` pointing at the
next call to make. emem never returns an empty error body. If you get a
non-JSON 5xx, the responder is broken — please open an issue.

## Shape

```json
{
  "code":    "<stable string, kebab-case>",
  "message": "<human-readable explanation>",
  "hint":    "<optional, what to call next>",
  "request_id": "<base32, useful in bug reports>"
}
```

`code` is the field you switch on. `message` is what you show the user.
`hint` is what your code does next. The codes below are stable across
0.0.x; new codes may be added, never renamed.

## Common codes

### `cid_not_found` (404)

The fact CID you asked for is not in this responder's storage.

```json
{ "code":"cid_not_found",
  "message":"no fact stored for CID wbqyx…m5q",
  "hint":"call POST /v1/recall to materialise this (cell, band) — auto-materialize will fetch upstream and persist" }
```

What to do: call `/v1/recall` with the original `(cell, band)`. The
responder will fetch from open data, sign, persist, and serve it. The
recall is idempotent — repeat callers see the same fact.

### `cell_not_found` (404)

You called `/v1/locate` with a query that doesn't resolve to a cell.

```json
{ "code":"cell_not_found",
  "message":"could not geocode 'NotAReadPlace'",
  "hint":"try a more specific name, or POST /v1/locate with {\"lat\":…, \"lng\":…} directly" }
```

What to do: be specific ("South Mumbai" beats "Mumbai", which beats
"India"), or pass coordinates directly.

### `band_unknown` (422)

The band key you asked for isn't in the registry.

```json
{ "code":"band_unknown",
  "message":"band 'sentinel2.ndvi_avg' is not declared",
  "hint":"GET /v1/bands for the registry; correct spelling is 'sentinel2_l2a.ndvi'" }
```

What to do: pull `/v1/bands` once at startup and cache the keys. Bands
are content-addressed: the response carries `bands_cid` so you can
detect schema drift.

### `band_not_materialized` (404)

The band is declared but no materializer is wired on this responder.
Emem returns a **signed Absence** — that's a citable receipt, not a
failure. Use it.

```json
{ "code":"band_not_materialized",
  "message":"band 'radd.alert' is declared but no live connector is wired here",
  "hint":"check GET /v1/materializers; signed Absence is in receipt — cite it as 'no data at this place'" }
```

What to do: surface the absence to your user as "no data here", don't
retry. A signed Absence has the same CID semantics as a positive fact.

### `polygon_too_large` (422)

Your `/v1/recall_polygon` polygon exceeds the per-call cell budget.

```json
{ "code":"polygon_too_large",
  "message":"polygon spans 18421 cells; max per call is 4096",
  "hint":"split into tiles, or use GET /v1/query_region for aggregate-only" }
```

What to do: split or aggregate. Each tile gets its own receipt.

### `signature_invalid` (422)

Receipt failed signature verification.

```json
{ "code":"signature_invalid",
  "message":"ed25519.verify returned false against responder pubkey 777er…womvka",
  "hint":"check the receipt wasn't truncated; preimage rules at /docs/whitepaper.html#trust-receipts" }
```

What to do: re-fetch the receipt. If it still fails, the responder pubkey
may have rolled — pull `/.well-known/emem.json` for the current key.

### `rate_limited` (429)

You're hitting the public responder too hard. Backoff per the
`retry-after` header.

```json
{ "code":"rate_limited",
  "message":"too many requests; retry after 5 s",
  "hint":"self-host for unlimited throughput: docker run ghcr.io/vortx-ai/emem:latest" }
```

What to do: exponential backoff (start 1 s, cap at 30 s). Self-host if
your workload sustains > 50 rps.

### `internal_error` (5xx)

Something went wrong on the responder. The `request_id` in the body is
what to paste into a GitHub issue.

```json
{ "code":"internal_error",
  "message":"upstream connector 'jrc_tmf' timed out after 8 s",
  "hint":"retry once; if it persists, open an issue with this request_id",
  "request_id":"r-c3vbhf2x6m" }
```

What to do: one retry. If it persists, file
[github.com/Vortx-AI/emem/issues](https://github.com/Vortx-AI/emem/issues)
with the `request_id`.

### `memory_attestation_invalid` (401)

A memory write supplied an `attester: {pubkey_b32, sig_b32}` block
whose signature does not verify against the supplied pubkey over the
canonical preimage `blake3("emem.memory_write|" + verb + "|" + path +
"|" + body_hash)`. Either the caller signed the wrong bytes, the
pubkey doesn't match the private key, or the body got rewritten in
flight.

```json
{ "code":"memory_attestation_invalid",
  "message":"attester.sig_b32 does not verify over blake3(emem.memory_write|create|/memories/by_attester/wosdtqio/note.md|<body_hash>)",
  "hint":"recompute the preimage and re-sign with the matching ed25519 private key" }
```

What to do: regenerate `body_hash = blake3(canonical request body
bytes)`, recompute the preimage, sign with the corresponding private
key, retry. Reference implementation in
`crates/emem-primitives/src/memory_acl.rs` (`attester_preimage` +
`verify_attester`).

### `memory_namespace_violation` (403)

A memory write supplied a valid attester signature, but the path
falls under a namespace owned by a *different* attester. Paths under
`/memories/by_attester/<pubkey_short>/...` are write-restricted to
the holder of the corresponding ed25519 key.

```json
{ "code":"memory_namespace_violation",
  "message":"signature verified, but path /memories/by_attester/zzzzzzzz/foo.md is under a different attester's namespace. Use /memories/by_attester/wosdtqio/...",
  "hint":"either change the path to your own namespace, or write to bare /memories/... which accepts anyone" }
```

What to do: either prefix the path with your own
`/memories/by_attester/<your_pubkey_short>/...`, or drop the
`by_attester` segment entirely and write to `/memories/...` which
stays anyone-writable.

### `invalid_temporal_bound` (400)

A read primitive received both an explicit `tslot` and an
`as_of_tslot` whose values conflict (the `as_of` bound excludes the
explicit tslot). Bi-temporal reads must intersect, not contradict.

```json
{ "code":"invalid_temporal_bound",
  "message":"explicit tslot=99999 but as_of_tslot=12345 excludes it",
  "hint":"drop tslot and use only as_of_tslot, or set as_of_tslot >= tslot" }
```

What to do: pick one. If you want "the latest fact ≤ T," set
`as_of_tslot = T`. If you want "the fact at exactly tslot T," set
`tslot = T` and leave `as_of_tslot` unset.

### `invalid_signed_at_format` (400)

A read primitive received `as_of_signed_at` that doesn't parse as
RFC 3339. The validator is strict — it checks calendar ranges,
clock ranges, optional fractional seconds, and the offset
(`Z` or `±HH:MM`).

```json
{ "code":"invalid_signed_at_format",
  "message":"as_of_signed_at must be RFC 3339 (e.g. '2026-05-15T00:00:00Z'); got 'not-a-date' (too short)",
  "hint":"format as YYYY-MM-DDThh:mm:ssZ or with explicit offset" }
```

What to do: format the timestamp as RFC 3339. Examples:
`2026-05-15T00:00:00Z`, `2026-05-15T18:30:00+05:30`,
`2026-05-15T18:30:00.500Z`.

## Status codes at a glance

| HTTP | What it usually means | Retryable? |
|------|----------------------|------------|
| 200  | OK                                                                | n/a |
| 304  | Not modified — your `If-None-Match` matched current ETag         | n/a |
| 400  | Malformed JSON, missing required field, conflicting bi-temporal bounds, malformed RFC 3339 | no — fix the call |
| 401  | `memory_attestation_invalid` (memory writes only; reads stay open) | no — re-sign |
| 403  | `memory_namespace_violation` (memory writes to someone else's namespace) | no — change path |
| 404  | `cid_not_found`, `cell_not_found`, or `band_not_materialized`    | depends on `code` |
| 422  | Validation failure (`band_unknown`, `polygon_too_large`, etc.)   | no — fix the call |
| 429  | `rate_limited`                                                   | yes, with backoff |
| 500  | `internal_error`                                                 | once |
| 502  | Upstream connector unreachable                                   | yes, with backoff |
| 504  | Upstream timeout                                                 | yes, with backoff |

## Honest absences vs. errors

A 404 with `code:"cid_not_found"` says *the fact isn't in this
responder's hot storage* — call `/v1/recall` and it will materialize.
A signed Absence inside a 200 response says *there is no data at this
cell for this band, and we've signed that fact*. Treat the second as a
real answer (cite the receipt). Treat the first as a hint to call recall.

If you find an error path that doesn't follow the shape above, file an
issue — it's a bug, not a feature.
