# emem error reference

The live catalogue is `GET /v1/errors` (schema `emem.errors.v1`), and it is
the source of truth: each row carries the code, its MCP error number, what
it means and how to recover. This page was checked against it and against
`crates/emem-core/src/error.rs` on 2026-09-29. When the two disagree, trust
the live endpoint.

## The envelope

Every error from a REST route is an `emem.error.v1` object:

```json
{
  "code":    "band_not_in_registry",
  "message": "none of the requested bands are known to this responder: elevation_mean. ...",
  "schema":  "emem.error.v1",
  "details": { "...": "optional, typed, per error" }
}
```

`details` is present when the responder has something machine-readable to
add: the field a deserializer rejected, the candidates for an ambiguous
place, or the exact digest a write should have signed. Some errors also
carry `path` or `did_you_mean`.

Over MCP a failed tool call comes back as a normal `result` with
`isError: true`. The text reads `tool error (-N): <code>: <message>`,
followed by the `details` JSON when there is any, so an MCP caller can
branch on the named code without knowing the number.

## How to branch

1. **Switch on `code`, then on `details.code` when it exists.** Several
   refusals share a top-level code and differ only in `details.code`. For
   example, an unsigned memory write is HTTP 401 with `code:
   invalid_argument` and `details.code: memory_attestation_required`.
   Messages change; codes do not.
2. **Do not switch on HTTP status alone.** The same code can come back with
   different statuses from different routes: `band_not_in_registry` is 400
   from `/v1/recall` and 404 from the storage layer.
3. **A signed Absence is not an error.** A `/v1/recall` fact with
   `kind: "absence"` and a `reason_cid` is the answer "this band has no
   data at this cell", signed like any other fact. Retrying will not change
   it.

## The catalogue (27 codes)

| Code | MCP | Meaning | First thing to try |
|---|---|---|---|
| `invalid_cell` | -27 | The cell64 string did not parse. A string with a `.` in it is not geocoded as a place, so a mistyped cell is refused rather than sent somewhere random. | Re-derive it with `POST /v1/locate`. |
| `invalid_resolution` | -1 | Resolution out of range. | Use the cell64 `/v1/locate` returns; see `/v1/grid_info`. |
| `tslot_mismatch` | -2 | The tslot does not sit on the band's tempo grid. | Read the band's `tempo` in `/v1/bands`. |
| `band_not_in_registry` | -3 | The band key is not in the active manifest. For a band family, `details.did_you_mean` names its wired keys. | `GET /v1/bands`. Band keys are namespaced: `copdem30m.elevation_mean`, not `elevation_mean`. |
| `function_not_in_registry` | -4 | A derivation `fn_key` is not registered. | `GET /v1/functions`. |
| `source_scheme_unknown` | -5 | `source.scheme` is not in `/v1/sources`. | `GET /v1/sources`. |
| `cid_not_found` | -6 | Nothing is stored under that content id here. Memory reads say whether a tombstone exists, so "never written" and "written then removed" are told apart. | Check the cid came from this responder (`responder_pubkey_b32` against `/health`). |
| `no_geocoder_match` | -7 | Every geocoder answered and none knows the place, or the name was too ambiguous to pick one (`details.code: ambiguous_place`). | Add a region or country, or pass `lat` and `lng`. |
| `registry_cid_unknown` | -8 | The bound `registry_cid` is not recognised. | `GET /v1/manifests`. |
| `schema_cid_unknown` | -9 | The fact's `schema_cid` is not loaded. | `GET /v1/manifests`. |
| `privacy_refused` | -10 | The band's `privacy_class` blocks this query at this resolution. | Query a coarser cell; see `privacy_class` in `/v1/bands`. |
| `level_too_low` | -11 | The operation needs a higher tier. The fact plane refuses any attestation from a key that is not the responder's materialiser, an enrolled device, or an operator-listed writer. | `GET /v1/enlist` for what each tier can write. |
| `attester_revoked` | -12 | The attester key is revoked. | Sign with a current key. |
| `unauthorized` | -13 | Missing or invalid attester signature on a write. | Re-sign; the refusal says what was expected. |
| `claim_undecidable` | -14 | The claim cannot resolve to true or false (a missing value, NaN, a type mismatch). | Recall the fact and check its value and type. |
| `bad_signature` | -15 | An ed25519 signature did not verify. | For receipts, `/v1/verify_receipt` returns `preimage_blake3_hex` to compare with your own. For memory writes, see below. |
| `unaddressable_subject` | -16 | A fact's subject is neither a cell64 nor an `emem:entity:` identity, so the write is refused before the log. Your signature is not the problem. | Send a cell64 or an entity id as the subject. |
| `bad_merkle_proof` | -17 | An inclusion proof is malformed or does not reach its root. | Rebuild it from the canonical leaf order; check `leaf_index`. |
| `canonical_encoding_divergence` | -18 | The CBOR sent is not deterministic per RFC 8949 section 4.2.1. | Encode with sorted map keys and shortest-form integers. |
| `source_fetch_failed` | -19 | An upstream fetch failed at the transport (non-2xx, DNS, timeout, bad JSON) before any answer was reached. On a place lookup this means the geocoder is down, not that the place is absent. | Retry with backoff. |
| `source_format_mismatch` | -20 | The fetched bytes do not match the declared format. | Report it; the provider may have changed format. |
| `compute_timeout` | -21 | A derivation ran past its deadline. | Send a smaller input, or retry: a band read cut off by the 14 s dispatch cap keeps running and a retry often answers warm. |
| `compute_quota_exceeded` | -22 | A per-key, per-day compute quota was hit. | Throttle; the windows are published at `/v1/limits`. |
| `rate_limited` | -23 | Too many requests. Per IP the default is 600 per minute with a burst of 120; memory writes also have a per-attester backstop of about 240 per minute (`details.code: write_rate_limited`). | Honour `Retry-After` (seconds) or `details.retry_after_s`. |
| `cache_error` | -24 | The responder's store returned an internal error. | Retry once. |
| `invalid_argument` | -25 | A field is syntactically valid but semantically wrong, or the body did not match the route's schema (`details.deserializer_error` names the field). Also the top-level code for most memory-write refusals. | Read `message` and `details`; the request shape is in `/openapi.json`. |
| `internal` | -26 | A responder bug. Receipts already signed stay valid. | File the request and response at [github.com/Vortx-AI/emem/issues](https://github.com/Vortx-AI/emem/issues). |

## Codes outside the catalogue

A few envelopes come from the router and the load layer rather than from a
handler, and use codes the catalogue does not list:

| Code | HTTP | When |
|---|---|---|
| `not_found` | 404 | No route matches the path. `did_you_mean` suggests near paths. |
| `method_not_allowed` | 405 | The path exists but not for that method. `allow` lists the methods it does answer. |
| `overloaded` | 503 | The server is at its in-flight request cap and shed the request at once rather than queue it. `Retry-After: 1`. |

## Memory and entity write refusals

Memory-verb refusals name themselves in `details.code`. Each one that
concerns a signature carries `details.how_to_sign`: the exact digest the
responder will verify for this write, the encoding rules and a worked
example. The entity routes answer with a top-level code instead, shown in
the last row.

| `details.code` | HTTP | Meaning |
|---|---|---|
| `memory_attestation_required` | 401 | The write has no `attester` block. emem.dev refuses unsigned writes. |
| `memory_attestation_invalid` | 401 | The key is malformed (`reason: bad_pubkey`) or the signature does not verify (`reason: bad_signature`). The message names the digest that was expected. `delete` and `rename` verify only the v2 preimage, `emem.memory_write.v2\|verb\|path\|body_hash\|base`. |
| `memory_namespace_violation` | 403 | The path is in another key's `/memories/by_attester/<pubkey8>/` namespace, is owned by another key, or is under the reserved `/memories/.well-known/` prefix. |
| `memory_signature_replayed` | 409 | A v1 signature for a state-dependent verb was used twice. Sign v2, which binds the version it replaces. |
| `write_rate_limited` | 429 | The per-attester write backstop tripped. |
| `vault_immutable` | 409 | An in-place edit was sent to a sealed vault entry. Re-create it instead. |
| `level_too_low` on `/v1/entity`, `/v1/entity/alias` | 403 | The shared entity space needs a higher tier than your own namespace (`details.tier` says which you reached; `GET /v1/enlist` says what each check proves). When the mint was unsigned, `details.how_to_sign` (its `code` is `entity_attestation_required`) carries the digest to sign. |

## Recovery endpoints

- `GET /v1/errors`: this catalogue, live
- `GET /v1/manifests`: current registry, schema, band and source cids
- `GET /v1/bands`: the band catalogue with tempo and `privacy_class`
- `GET /v1/limits`: batch sizes, rate limits and timeouts
- `POST /v1/verify_receipt`: debug a `bad_signature` with `preimage_blake3_hex`
