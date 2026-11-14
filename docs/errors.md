# emem error reference

An emem error is a JSON object with a stable `code`. Programs branch on that
code. The `message` is for people and can change.

A REST route can send codes from three layers:

- 27 protocol codes. These are the `ErrorCode` enum in
  `crates/emem-core/src/error.rs`. Each has a fixed MCP error number.
- Three codes that the router and the load layer send: `not_found`,
  `method_not_allowed` and `overloaded`. They are not in the enum.
- Route codes in `details.code`. A route uses them to say which of its own
  checks refused the request. The top-level `code` is then one of the 27.

The JSON-RPC endpoints `/mcp` and `/a2a/tasks` also send JSON-RPC error
numbers. See [JSON-RPC errors](#json-rpc-errors-from-mcp-and-a2atasks).
A guard verdict uses its own deny codes. See
[Deny codes from the guard](#deny-codes-from-the-guard).

The live catalogue is `GET /v1/errors` (schema `emem.errors.v1`). It lists the
27 protocol codes with their MCP numbers, meanings and recovery steps. We
checked this page against that endpoint and against the code on 7 November
2026. When this page and the live endpoint disagree, trust the live endpoint.

On this page:

| Section | Use it to |
|---|---|
| [The error envelope](#the-error-envelope) | Read the shape of an `emem.error.v1` body. |
| [How to branch on an error](#how-to-branch-on-an-error) | Write the client logic in the correct order. |
| [Examples from emem.dev](#examples-from-ememdev) | Cause each common error with one command. |
| [Skip notes on a successful recall](#skip-notes-on-a-successful-recall) | Read `reason_class` on a band that the responder did not fetch. |
| [Protocol codes (27)](#protocol-codes-27) | Find a top-level `code` and its MCP number. |
| [Codes from the router and the load layer](#codes-from-the-router-and-the-load-layer) | Handle 404, 405 and 503 before a handler runs. |
| [Memory and entity write refusals](#memory-and-entity-write-refusals) | Fix a refused signed write. |
| [Derivation refusals](#derivation-refusals-post-v1derive) | Fix a refused `POST /v1/derive`. |
| [Other route codes](#other-route-codes) | Find a `details.code` from another route. |
| [Refusals from the history, comparison and region reads](#refusals-from-the-history-comparison-and-region-reads) | Fix a refused trajectory, diff, edge, contradiction, region or similarity read. |
| [JSON-RPC errors](#json-rpc-errors-from-mcp-and-a2atasks) | Handle errors from `/mcp` and `/a2a/tasks`. |
| [Responses that are not `emem.error.v1`](#responses-that-are-not-ememerrorv1) | Handle quota, plain-text, proxy, `/splats` and OAuth bodies. |
| [Routes that only a self-hosted node answers](#routes-that-only-a-self-hosted-node-answers) | Understand a 501 from emem.dev. |
| [Deny codes from the guard](#deny-codes-from-the-guard) | Act on a guard `deny`. |
| [Endpoints that help you recover](#endpoints-that-help-you-recover) | Find the live catalogues. |

## The error envelope

Almost every error from a REST route is an `emem.error.v1` object. A few
routes send other bodies. See
[Responses that are not `emem.error.v1`](#responses-that-are-not-ememerrorv1).
This is the shape:

```json
{
  "code":    "band_not_in_registry",
  "message": "none of the requested bands are known to this responder: elevation_mean. ...",
  "schema":  "emem.error.v1",
  "details": { "...": "optional, typed, per error" }
}
```

The responder adds `details` when it has machine-readable data. Examples are
the field that the deserializer refused, the candidates for an ambiguous
place, and the digest that a write had to sign. Some errors also carry `path`
or `did_you_mean`.

Over MCP, a failed tool call comes back as a normal `result` with
`isError: true`. The text is `tool error (-N): <code>: <message>`. `N` is
the MCP number of the code. When the error has `details`, the text then has
a blank line, the line `details:`, and the details as indented JSON. An MCP
caller can branch on the code name without the number.

## How to branch on an error

1. Switch on `code`. Then switch on `details.code` when it exists. Several
   refusals share one top-level code and differ only in `details.code`. For
   example, an unsigned memory write is HTTP 401 with `code:
   invalid_argument` and `details.code: memory_attestation_required`.
2. Do not switch on the HTTP status alone. One code can have different
   statuses on different routes. `band_not_in_registry` is 400 from
   `/v1/recall`. It is 404 from `/v1/backfill` with one `cell`, and from
   every route that passes on a storage error.
3. Do not treat a signed absence as an error. A `/v1/recall` fact with
   `kind: "absence"` and a `reason_cid` is a signed answer: this band has
   no data at this cell. A retry gives the same answer.
4. Read `materialize_notes` on a successful recall. A band that the
   responder skipped or could not fetch appears there, not in the error.
   See [Skip notes on a successful recall](#skip-notes-on-a-successful-recall).

## Examples from emem.dev

We ran these requests on 7 November 2026.

A path that does not exist gives HTTP 404 with `code: not_found`:

```sh
curl -s https://emem.dev/v1/no-such-route
```

The body is short and has `schema: emem.error.v1`. Some routes send other
bodies; see [Responses that are not
`emem.error.v1`](#responses-that-are-not-ememerrorv1):

```json
{"code":"not_found","did_you_mean":[],"message":"no route matched /v1/no-such-route. ...","path":"/v1/no-such-route","schema":"emem.error.v1"}
```

curl sends `Accept: */*`, so it gets JSON. A request with no `Accept`
header gets an HTML page with the same 404 status. Python `urllib` sends no
`Accept` header by default. Send `Accept: application/json` to get the JSON
body.

A path that exists, called with the wrong method, gives HTTP 405 with
`code: method_not_allowed`. `/v1/errors` answers only `GET`:

```sh
curl -s -i -X DELETE https://emem.dev/v1/errors
```

The `allow` header and the `allow` body field both list the methods that
the path answers. Here that is `GET`. Use `allow`. The `example` and
`_legacy_message` fields always name a method that is not `GET`, `HEAD` or
`OPTIONS`. On a path that answers only `GET`, they name `POST`, which is
wrong. The `message` field is correct:

```json
{"allow":["GET"],"code":"method_not_allowed","message":"/v1/errors is a wired endpoint but does not answer DELETE. It accepts GET.","method_tried":"DELETE","schema":"emem.error.v1","...":"..."}
```

A malformed cell64 gives HTTP 400 with `code: invalid_cell`:

```sh
curl -s -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell":"not.a.cell"}'
```

A body with a wrong field type gives HTTP 400 with `code:
invalid_argument`. `details.deserializer_error` names the field (`cell:
invalid type: integer`):

```sh
curl -s -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell":5}'
```

A value out of its range gives HTTP 400 with `code: invalid_argument` and
`details.error: invalid_field`. The details also have `field`, `allowed`
and `got`. For `lat` and `lng` routes such as `/v1/at`, `allowed` has one
`{min, max}` for each coordinate, and `got` holds both values.
`/v1/heat_solve`, `/v1/wave_solve` and `/v1/jepa_predict` send this body
for their numeric fields, with one `min` and `max`. `/v1/jepa_predict`
also sends it for `band`, with an `enum`.

```sh
curl -s 'https://emem.dev/v1/at?lat=95&lng=10'
```

An unknown band key needs care, because the answer depends on the other
bands in the request:

| Request | Answer |
|---|---|
| `POST /v1/recall`, no band in `bands` is in the registry | HTTP 400, `code: band_not_in_registry` |
| `POST /v1/recall`, some bands are in the registry | HTTP 200 with the known facts. Each unknown band is in `materialize_notes` with `reason_class: unknown_band` and `retryable: false`. The top-level `unknown_bands` array also lists them. |
| `POST /v1/recall`, a legacy short key such as `dem` | HTTP 200 with `facts: []` and no `unknown_bands` field. Only `materialize_notes` names the band, with `reason_class: unknown_band`. The handler finds a materializer entry for the key, so it does not count the key as unknown. Use the namespaced key (`copdem30m.elevation_mean`). |
| MCP `emem_recall`, any mix | No tool error. Unknown bands are in `materialize_notes`, as for REST. The MCP result has no `unknown_bands` field. |

This request has only the unknown band, so it returns HTTP 400 with `code:
band_not_in_registry`:

```sh
curl -s -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb4d9.pefa.zf619","bands":["elevation_mean"]}'
```

This request has one unknown and one known band, so it returns HTTP 200.
`facts` holds the `copdem30m.elevation_mean` fact, and `unknown_bands` is
`["elevation_mean"]`:

```sh
curl -s -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb4d9.pefa.zf619","bands":["elevation_mean","copdem30m.elevation_mean"]}'
```

`/v1/backfill` with one `cell` sends the same code with HTTP 404, not 400.
It refuses the unknown band before it fetches anything:

```sh
curl -s -X POST https://emem.dev/v1/backfill \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb4d9.pefa.zf619","band":"elevation_mean"}'
```

A fact token whose cid lost characters gives HTTP 400 with `code:
invalid_argument` and `details.code: fact_cid_malformed_length`. This cid
has 50 characters, not 52. `details.recoverable` is `false`:

```sh
curl -s -X POST https://emem.dev/v1/memory_token/resolve \
  -H 'content-type: application/json' \
  -d '{"token":"emem:fact:defi.zb4d9.pefa.zf619:4zstrktfkxaxm7hvdw5tuz2khxf53y5f55ucn5wk2rvjfxpam7"}'
```

The `details` block of that refusal is:

```json
{"code":"fact_cid_malformed_length","expected_length":52,"observed_length":50,"recoverable":false,"why":"..."}
```

Over MCP, the malformed cell gives a result with `isError: true` and the
text `tool error (-27): invalid_cell: ...`. The JSON-RPC call itself
succeeds, so read `result.isError`, not the JSON-RPC `error` field:

```sh
curl -s -X POST https://emem.dev/mcp \
  -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"emem_recall","arguments":{"cell":"not.a.cell"}}}'
```

## Skip notes on a successful recall

A recall can return HTTP 200 and still fail to fetch a band. The responder
then adds an entry to `materialize_notes[]` with `status: "skipped"`. The
responder signs nothing for that band, and the entry is not an absence. A
signed absence comes back as `status: "materialized"` with a `fact_cid`.
The recall, polygon and find-similar routes use the same note shape.

A per-band skip note has these keys: `band`, `status`, `reason`,
`reason_class`, `retryable` and `absence` (always `false`). The `reason` is
free text. Branch on `reason_class`. The function `classify_skip_reason` in
`crates/emem-api-rest/src/lib.rs` makes the class from the reason text. It
tests the rows of this table in order and takes the first match:

| `reason_class` | `retryable` | What happened |
|---|---|---|
| `deferred` | `true` | The responder reached its limit of cold fetches per minute. The limit is `EMEM_MATERIALIZE_PER_MIN`, default 600. |
| `retired` | `false` | This deployment withdrew the band. Another responder can still serve it. |
| `not_geographic` | `false` | The cell is not a geographic cell, so no upstream can answer for it. |
| `unknown_band` | `false` | This responder does not know the band key. |
| `no_data` | `false` | Both input bands read zero at this pixel in the newest scene. A new overpass can change this. |
| `upstream_gone` | `false` | A fixed upstream URL answered 404 or 410. The upstream moved or withdrew the data. |
| `no_materializer` | `false` | This responder has no materializer for this band. |
| `timeout` | `true` | The upstream did not answer in time. |
| `upstream_error` | `true` | Any other upstream failure. |

A `true` value means that a later call can succeed. A `false` value means
that the same call to this responder gives the same skip.

One skip has the wrong class in version 2.4.4. A key such as
`vegetation_condition.ndvi` is a topic and a dimension, not a band. The
`reason` starts with `topic_name_used_as_band` and lists the bands of that
topic. No row of the table matches this text, so the class is
`upstream_error` with `retryable: true`. A retry gives the same skip. Recall
one of the listed bands instead. On 7 November 2026, emem.dev sent this note
when the request also named a known band. When the responder knows no band
in the request, the recall refuses with 400 `band_not_in_registry`.

Two skip notes cover more than one band. They have `bands[]` in place of
`band`, and they have no `absence` key:

| `reason` | `reason_class` | When |
|---|---|---|
| `band_retired_at_this_responder` | `retired` | The request names a band in `EMEM_RETIRED_BANDS`. The responder does not try it. |
| `as_of_signed_at_in_the_past` | none | `as_of_signed_at` is before the current time. A fact signed now cannot match that bound, so the responder fetches nothing. The note adds `as_of_signed_at` and `responder_now`. |

To read a value from the past, use `as_of_tslot`, not `as_of_signed_at`.
`as_of_tslot` bounds the observation time. `as_of_signed_at` bounds the
signing time.

The `ask` path can also send `status: "deferred"` with only `band` and
`reason`. That entry means the call reached its cap on cold bands. Fetch
the band with `POST /v1/recall`.

We ran these two requests on 7 November 2026. Each gave HTTP 200 with
`facts: []` and one note of the second table:

```sh
curl -s -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb4d9.pefa.zf619","bands":["nightlights"]}'

curl -s -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb4d9.pefa.zf619","bands":["weather.temperature_2m"],"as_of_signed_at":"2020-01-01T00:00:00Z"}'
```

## Protocol codes (27)

These are all the variants of `ErrorCode`. The MCP number is the enum
discriminant, negated. The first HTTP status is the default. It comes from
the store error map in `crates/emem-api-rest/src/lib.rs`
(`impl From<StorageError> for ApiError`). The other statuses come from
handlers that set their own. On 7 November 2026, no route sends 13 of these
codes. The next subsection names them and says what you get instead.

| Code | MCP | HTTP | Meaning | First thing to try |
|---|---|---|---|---|
| `invalid_cell` | -27 | 400 | The cell64 string did not parse. The responder does not geocode a string with a `.` in it as a place. So it refuses a mistyped cell and does not send it somewhere random. | Re-derive it with `POST /v1/locate`. |
| `invalid_resolution` | -1 | 400 | Resolution out of range. | Use the cell64 `/v1/locate` returns; see `/v1/grid_info`. |
| `tslot_mismatch` | -2 | 400 | The tslot does not sit on the band's tempo grid. | Read the band's `tempo` in `/v1/bands`. |
| `band_not_in_registry` | -3 | 404; 400 from `/v1/recall` when it knows none of the bands | The band key is not in the active manifest. For a band family, `details.did_you_mean` names its wired keys. | `GET /v1/bands`. Band keys have a namespace: `copdem30m.elevation_mean`, not `elevation_mean`. |
| `function_not_in_registry` | -4 | 404 | No registry entry matches the derivation `fn_key`. | `GET /v1/functions`. |
| `source_scheme_unknown` | -5 | 400 | `source.scheme` is not in `/v1/sources`. | `GET /v1/sources`. |
| `cid_not_found` | -6 | 404 | This responder holds nothing under that content id. Memory reads say whether a tombstone exists. So you can tell "never written" from "written then removed". | Check the cid came from this responder (`responder_pubkey_b32` against `/health`). |
| `no_geocoder_match` | -7 | 404; 400 (`ambiguous_place`); 504 when an entity anchor lookup passes 12 s | Every geocoder answered and none knows the place (HTTP 404). Or the name was too ambiguous to pick one: HTTP 400 with `details.code: ambiguous_place`. | Add a region or country, or pass `lat` and `lng`. |
| `registry_cid_unknown` | -8 | 404 | The responder does not know the bound `registry_cid`. | `GET /v1/manifests`. |
| `schema_cid_unknown` | -9 | 404 | The responder has not loaded the fact's `schema_cid`. | `GET /v1/manifests`. |
| `privacy_refused` | -10 | 403 | The band's privacy class blocks this query at this resolution. On 7 November 2026 no route returns it: the responder does not enforce privacy classes ([protocol §10](protocol.md#10-privacy-classes)). | Query a coarser cell; see `privacy.class` for the band in `/v1/bands`. |
| `level_too_low` | -11 | 403 | The operation needs a higher tier. The fact plane refuses any attestation from a key that is not the responder's materializer, an enrolled device, or an operator-listed writer. | `GET /v1/enlist` for what each tier can write. |
| `attester_revoked` | -12 | 401 | The operator revoked the attester key. | Sign with a current key. |
| `unauthorized` | -13 | 401 | A write has no attester signature, or the signature is not valid. | Sign again. The refusal says what the responder expected. |
| `claim_undecidable` | -14 | 400 | The claim cannot resolve to true or false (a missing value, NaN, a type mismatch). | Recall the fact and check its value and type. |
| `bad_signature` | -15 | 422 (also a bad key in `/v1/verify_receipt`); 401 on memory, derive and witness writes; 403 when a `vault_capability` does not verify | An ed25519 signature did not verify. | For receipts, `/v1/verify_receipt` returns `preimage_blake3_hex` to compare with your own. For writes, see the tables below. |
| `unaddressable_subject` | -16 | 400 | A fact's subject is neither a cell64 nor an `emem:entity:` identity. The responder refuses the write before the log. Your signature is not the problem. | Send a cell64 or an entity id as the subject. |
| `bad_merkle_proof` | -17 | 422 | An inclusion proof has a bad shape, or it does not reach its root. | Rebuild it from the canonical leaf order; check `leaf_index`. |
| `canonical_encoding_divergence` | -18 | 500; 400 | HTTP 400: an attestation body did not decode (`/v1/attest_cbor`, `/v1/attest_traced`). HTTP 500: the responder could not encode or decode its own stored CBOR. No route compares your bytes with a canonical encoding. | On a 400, check the body against [protocol §6](protocol.md#6-attestation-envelope). On a 500, report it. |
| `source_fetch_failed` | -19 | 502; 504 when a fan-out passes its budget; 422 from the physics routes | An upstream fetch failed at the transport (non-2xx, DNS, timeout, bad JSON) before the upstream gave an answer. On a place lookup this means the geocoder is down, not that the place is absent. | Retry with backoff. |
| `source_format_mismatch` | -20 | 502 | The fetched bytes do not match the declared format. | Report it; the provider may have changed format. |
| `compute_timeout` | -21 | 504 | The request ran past the transport budget (default 40 s, `EMEM_TIMEOUT_SECS`). `details.budget_secs` gives that budget. Some routes have a different budget: see [Codes from the router and the load layer](#codes-from-the-router-and-the-load-layer). Over MCP the same cause reads `exceeded the <N>s call budget`, with the budget in seconds. | Send a smaller input, or retry. A band read that passes the 14 s dispatch cap keeps running. A retry often finds the value ready. |
| `compute_quota_exceeded` | -22 | 402 | You used all of a per-key, per-day compute quota. | No route sends this code in version 2.4.4 (see [Codes that no route sends](#codes-that-no-route-sends)). |
| `rate_limited` | -23 | 429; 503 when the memory SSE subscriber cap is full | Too many requests. Per IP the default is 1,800 per minute with a burst of 300 (`EMEM_RATE_LIMIT_RPS`, `EMEM_RATE_LIMIT_BURST`). Memory writes also have a per-attester backstop (`details.code: write_rate_limited`). | Honour `Retry-After` (seconds) or `details.retry_after_s`. |
| `cache_error` | -24 | 500; 503 when the node has no durable store | The responder's store returned an internal error. | Retry once. |
| `invalid_argument` | -25 | 400 on most routes; the route tables below give the others | A field is syntactically valid but semantically wrong, or the body did not match the route's schema (`details.deserializer_error` names the field). Also the top-level code for most route refusals below. | Read `message` and `details`; the request shape is in `/openapi.json`. |
| `internal` | -26 | 500; 503 or 501 on some routes | A responder bug. Receipts already signed stay valid. | File the request and response at [github.com/Vortx-AI/emem/issues](https://github.com/Vortx-AI/emem/issues). |

### Codes that no route sends

`/v1/errors` lists all 27 codes. The code maps each of these 13 codes to
an HTTP status, but no handler or store error produces it:

| Code | What you get instead |
|---|---|
| `invalid_resolution`, `tslot_mismatch` | Address errors come back as `invalid_cell` or `invalid_argument`. |
| `function_not_in_registry` | The store has a `StorageError::UnknownFunction` variant, but no code path creates it. `/v1/derive` does not check `fn_key` against a registry. |
| `source_scheme_unknown`, `source_format_mismatch` | Upstream problems come back as `source_fetch_failed`, or as a `materialize_notes` entry on a 200. |
| `registry_cid_unknown`, `schema_cid_unknown` | No route reads a caller's registry or schema CID. |
| `privacy_refused` | The responder does not enforce privacy classes ([protocol §10](protocol.md#10-privacy-classes)). |
| `attester_revoked` | The responder has no revocation check on writes. |
| `unauthorized` | An unsigned write gets HTTP 401 with top-level `invalid_argument` and `details.code` such as `memory_attestation_required` or `derive_attestation_required`. A signature that does not verify gets HTTP 401 with `bad_signature`. |
| `claim_undecidable` | `/v1/verify` answers HTTP 200 with a verdict ([protocol §11](protocol.md#11-claim-algebra)). |
| `bad_merkle_proof` | No route accepts a caller's proof. Check proofs yourself ([protocol §9.6.1](protocol.md#961-check-an-inclusion-proof-by-hand)). |
| `compute_quota_exceeded` | The quotas that exist answer with their own bodies (see [Responses that are not `emem.error.v1`](#responses-that-are-not-ememerrorv1)). |

Write your client to handle these codes anyway. A later release or
another node can send them.

## Codes from the router and the load layer

The router and the load layer send these envelopes before any handler runs.
Their codes are not in the enum.

| Code | HTTP | When |
|---|---|---|
| `not_found` | 404 | No route matches the path. `did_you_mean` suggests near paths. With no `Accept` header, or an `Accept` header that contains `text/html`, the body is an HTML page with no `code`. curl sends `Accept: */*` by default, so curl gets JSON. |
| `method_not_allowed` | 405 | The path exists but not for that method. The `allow` header and body field list the methods it answers. With an `Accept` header that contains `text/html`, the body is an HTML page. With no `Accept` header, the body is JSON. |
| `overloaded` | 503 | The responder is at its cap of requests in flight: `EMEM_MAX_INFLIGHT`, default 128, limits 8 to 4096. It refused the request at once and did not queue it. `Retry-After: 1`. |
| `overloaded` | 503 | Only `POST /v1/eudr_dds`, `/v1/field_burn_scar` and `/v1/field_actual_et`. These routes run 6 requests at a time (`EMEM_EUDR_MAX_INFLIGHT`, limits 1 to 64). Up to 4 times that number wait for a slot. A request gets this 503 when the wait is full, or when no slot frees in 60 s (`EMEM_EUDR_QUEUE_SECS`, at most 120). `Retry-After: 1`. |

Each router has its own transport budget. When a request passes it, the
router sends 504 `compute_timeout`.

| Routes | Default budget | Variable | Limits |
|---|---|---|---|
| All routes not named below | 40 s | `EMEM_TIMEOUT_SECS` | 1 to 600 |
| `POST /v1/eudr_dds`, `/v1/field_burn_scar`, `/v1/field_actual_et` | 610 s | `EMEM_EUDR_TIMEOUT_SECS` | 15 to 600, then the router adds 10 s |
| `/splats`, `/splats/`, `/splats/*path` | 180 s | `EMEM_SPLATS_TIMEOUT_S` | 40 to 600 |
| `/v1/perception/*path` | 120 s | `EMEM_PERCEPTION_TIMEOUT_S` | 40 to 600 |

The 504 body always takes `details.budget_secs` and the number in its
message from `EMEM_TIMEOUT_SECS`. On the last three rows that number is
not the budget that applied. Use this table instead.

## Memory and entity write refusals

Memory verbs name the refusal in `details.code`. A refusal about a signature
also carries `details.how_to_sign`. It holds the exact digest the responder
verifies for this write, the encoding rules and a worked example.

| `details.code` | Top-level `code` | HTTP | Meaning |
|---|---|---|---|
| `memory_attestation_required` | `invalid_argument` | 401 | The write has no `attester` block. emem.dev refuses unsigned writes. |
| `memory_attestation_invalid` | `bad_signature` | 401 | The key has a bad shape (`reason: bad_pubkey`) or the signature does not verify (`reason: bad_signature`). The message names the expected digest. Every verb verifies only the v2 preimage, `emem.memory_write.v2\|verb\|path\|body_hash\|base`. The responder refuses a v1 signature with `reason: v1_retired` (since 2026-11-05). |
| `memory_namespace_violation` | `invalid_argument` | 403 | Your key may not change this path. `details.reason` says why. The table below lists the reasons. |
| `write_rate_limited` | `rate_limited` | 429 | The per-attester write backstop tripped. The defaults are 240 writes per minute with a burst of 60 (`EMEM_WRITE_RATE_PER_MIN`, `EMEM_WRITE_RATE_BURST`). |
| `vault_immutable` | `invalid_argument` | 409 | You sent an in-place edit to a sealed vault entry. Create the entry again instead. |
| `vault_capability_invalid` | `bad_signature` | 403 | A `vault_capability` did not let you view that vault path. |

A `memory_namespace_violation` refusal comes after the signature verified.
The `details.reason` field names the ownership rule that failed.

| `details.reason` | Cause |
|---|---|
| `reserved_namespace` | The path is under `/memories/.well-known/`. No key can write under that prefix. |
| `namespace_mismatch` | The path is in the `/memories/by_attester/<pubkey8>/` namespace of another key. `details.expected_short` gives your own prefix. |
| `owner_key_mismatch` | A different key that has the same 8-character prefix wrote this file. A file answers to the full key that wrote it. |
| `vault_owner_mismatch` | Another key wrote this vault entry. Only that key can change or delete it. |
| `open_namespace_owner_mismatch` | Another key created this file in the open namespace. |
| `open_namespace_unowned_record` | The file is older than stored authorship, so it has no owner. It stays frozen while the operator requires signed writes. Copy it into your own namespace. |
| `source_namespace` | A rename moves a file out of the namespace of another key. `details.expected_short` gives your own prefix. |
| `root_directory_delete` | A directory delete named `/memories/` itself. |
| `directory_holds_foreign_files` | A directory delete covers files that your key does not own. `details.foreign_files` gives the count and `details.example` one path. |

The entity routes `/v1/entity` and `/v1/entity/alias` use a top-level code.
They refuse with `level_too_low` (HTTP 403) when the key has not reached the
tier that the shared entity space needs. `details.tier` gives the tier you
reached. `GET /v1/enlist` says what each tier check proves. When the mint
had no signature, `details.how_to_sign` carries the digest to sign, and its
`code` is `entity_attestation_required`.

The fact plane also refuses with `level_too_low` (HTTP 403). This occurs
when a key that the fact plane does not admit tries to write a fact at a
`(cell, band, tslot)` address. The signature verified, but the key does not
have the tier. The message starts with `LevelTooLow: fact_plane_closed:` and
names `POST /v1/derive` as the route to add a value. The body has no
`details` field.

## Derivation refusals (`POST /v1/derive`)

A derivation is a value that you computed over signed parent facts. The
responder checks the body, then the parents, then your signature. Each
refusal names itself in `details.code`.

| `details.code` | Top-level `code` | HTTP | Meaning |
|---|---|---|---|
| `derive_invalid_argument` | `invalid_argument` | 400 | `fn_key`, `cell`, `band` or `op` is empty. |
| `derive_invalid_cell` | `invalid_argument` | 400 | `cell` does not have the cell64 shape. Get the address from `/v1/locate`. |
| `derive_no_inputs` | `invalid_argument` | 400 | `inputs` is empty. A derivation must name at least one parent token. |
| `derive_too_many_inputs` | `invalid_argument` | 400 | `inputs` names more than 1024 parents. Put them in one `emem:bundle:` token and cite that. |
| `derive_invalid_tslot_window` | `invalid_argument` | 400 | `tslot_window` is not in order. It is inclusive `[start, end]`. |
| `derive_invalid_confidence` | `invalid_argument` | 400 | `confidence` is not a finite number from 0 to 1. |
| `derive_provenance_class_refused` | `invalid_argument` | 400 | A caller can declare only `model_output`, `human_curated` or `estimator`. The classes `direct_sensor` and `deterministic_index` are for the responder only. `details.allowed` lists the three. |
| `derive_input_unparseable` | `invalid_argument` | 400 | `inputs[details.index]` is not a valid token. |
| `derive_parent_unresolved` | `cid_not_found` | 404 | This responder does not hold that parent fact. Recall it here first, or send the derivation to the responder that holds it. |
| `derive_parent_cell_mismatch` | `invalid_argument` | 409 | The token names a cell that is not the cell of the signed fact. The message gives the correct token. |
| `derive_attestation_required` | `invalid_argument` | 401 | The body has no `attester` block. `details.how_to_sign` carries the digest to sign. |
| `derive_attestation_invalid` | `bad_signature` | 401 | The key has a bad shape (`reason: bad_pubkey`) or the signature does not verify (`reason: bad_signature`). The message names the expected digest. |
| `derive_namespace_violation` | `invalid_argument` | 403 | The code maps this case, but it cannot occur in version 2.4.4. `/v1/derive` is not under a `by_attester` path. |

Three more derive refusals have no `details` block. The route code is the
start of `message`.

| Start of `message` | Top-level `code` | HTTP | Meaning |
|---|---|---|---|
| `derive_uncanonicalisable_body` | `invalid_argument` | 400 | The responder cannot encode the body as canonical CBOR, so it cannot compute `body_hash`. |
| `derive_attestation_failed` | `internal` | 500 | The responder could not build or sign the attestation for the new fact. Report it. |
| `derive_store_returned_no_cid` | `internal` | 500 | The store accepted the attestation but returned no fact cid. Report it. |

## Other route codes

These routes also put a route code in `details.code`. The top-level code is
`invalid_argument` unless the table says otherwise.

| Route | `details.code` | HTTP | Meaning |
|---|---|---|---|
| `/v1/memory_token/resolve` and the other token readers | `fact_cid_malformed_length` | 400 | The fact cid is not 52 characters. Something cut the token in transit. A retry does not help. Read the token again from its source. A value with no cid shape at all gets `invalid_argument` with no `details.code`. |
| `/v1/log/witness` | `witness_root_mismatch` | 409 | The `root_b32` you co-signed is not the root this responder computes at that `tree_size`. |
| `GET /v1/tree/{file_cid}` | `cid_not_found` | 404 | Top-level `cid_not_found`. The responder holds no note with that `file_cid`. |
| `GET /v1/tree/{file_cid}` | `not_a_tree`, `no_rows` | 422 | The note is not `pointer.v1` or `directory.v1`, or no table row parsed. |
| `GET /v1/tree/{file_cid}` | `root_mismatch` | 409 | The root that the note states does not match the tree over its rows. The route gives no path. |
| `GET /v1/tree/{file_cid}` | `row_not_found` | 404 | No row has that index or label. `details.rows` gives the row count. |
| `GET /v1/tree/{file_cid}` | `path_self_check_failed` | 500 | Top-level `internal`. The computed path did not fold to the root. Report it. |
| `GET /v1/tree/{file_cid}` | none | 503 | Top-level `cache_error`, no `details`. The responder runs without a durable store, so it holds no notes. |
| `POST /v1/tree/path` | `invalid_argument` | 400 | The body must have exactly one of `leaves` and `chunks`. Each hash must be 52 lowercase base32 characters (32 bytes). `index` must be in range, with at most 1048576 leaves. The message says which check failed. A body that is not JSON gets a [plain-text refusal](#plain-text-refusals-of-a-bad-request-body). |
| `/v1/derived` | `derived_attester_required`, `derived_attester_malformed` | 400 | `attester_pubkey_b32` is missing, or it is not a 52-character base32 Ed25519 key. This read lists the derivations of one key only. |
| `/v1/derived` | `derived_band_needs_cell` | 400 | You sent `band` without `cell`. Send `cell` too, or filter by band on your side. |
| `/v1/derived` | none | 503 | Top-level `cache_error`, no `details`. The message starts with `derived_index_unavailable`. The responder has no durable store. |
| `/v1/derived` | none | 500 | Top-level `cache_error`, no `details`. The message starts with `derived_index_open_failed` or `derived_index_scan_failed`. The store could not open or read the derivation index. Retry once. |
| `/v1/inferences` | `inferences_place_required`, `inferences_invalid_coordinates` | 400 | Send `cell`, or `lat` and `lng` together. The route does not take a place name. `lat` must be in [-90, 90] and `lng` in [-180, 180]. |
| `/v1/inferences` | `inferences_invalid_cell`, `inferences_cell_latlng_disagree` | 400 | `cell` is not a cell64, or `cell` and `lat`/`lng` name different cells. Send one of them. |
| `/v1/inferences` | `inferences_too_many_attesters`, `inferences_attester_malformed` | 400 | `attesters` names more than 64 keys, or one entry is not a 52-character base32 Ed25519 key. |
| `/v1/inferences` | `inferences_unknown_tier` | 400 | `min_tier` is not one of `T0_anonymous` to `T5_corroborated`. The route also takes the short form `T4`, and upper or lower case. |
| `/v1/inferences` | `inferences_invalid_radius`, `inferences_radius_too_wide` | 400 | `radius_m` is not in [0, 50000]. Or, near a pole, the circle touches more than 4096 index tiles. Send a smaller radius. |
| `/v1/inferences` | `inferences_index_unavailable` | 503 | Top-level `cache_error`. The responder has no durable store, so it has no place index. Resolve derivations by token. |
| `/v1/device_publish` | `device_key_malformed`, `device_key_invalid`, `signature_malformed`, `signature_invalid` | 400 | The device key or the device signature is bad. Only the device can sign its own listing decision. |
| `/v1/a2a/tasks` | `a2a_skill_required`, `a2a_skill_unknown` | 400 | The `skill` field is empty, or it names no skill on the agent card. |
| `/v1/a2a/tasks` | none | 503 | Top-level `internal`, no `details`. The message starts with `a2a_task_spawn_failed`. The task registry is full of running tasks. The limit is 256 (`EMEM_MCP_MAX_TASKS`). Retry later. `/a2a/tasks` answers in the same call when the skill ends in 20 s; a slower skill needs a registry slot there too. |
| `/v1/memory/search` | none | 503 | Top-level `internal`, no `details`. The node has no text embedder, so it cannot search notes. The message starts with `memory_search: text embedder unavailable`. |
| `/v1/memory/search` | none | 500 | Top-level `internal`, no `details`. The search index failed. The message starts with `memory_search: index error`. An empty `q` gets 400 `invalid_argument`. |
| `/v1/building_footprints` | `needs_location` | 400 | Send `polygon_bbox`, or a `place` that resolves to a bounding box. |
| `/v1/range_hash` | `invalid_argument`, `range_too_large`, `url_refused` | 400 | The body is bad, the length is 0 or over the cap, or the URL is not public HTTPS. The cap is 16 MiB on 7 November 2026 (`EMEM_RANGE_HASH_MAX_BYTES`, at most 256 MiB). |
| `/v1/range_hash` | `unresolvable`, `fetch_failed`, `too_many_redirects`, `redirect_refused`, `range_not_honoured`, `short_read` | 502 | Top-level `source_fetch_failed`. The upstream host failed or did not return the byte range. |
| `/v1/range_hash` | `busy` | 503 | The responder closed its fetch pool. A normal request waits for one of 4 fetch slots, so you do not usually see this code. |
| `/v1/range_hash`, `/v1/read`, `/v1/ocr` | `client` | 500 | The responder could not build its HTTP client. Report it. |
| `/v1/read` | `invalid_argument` | 400 | The body is over 8 KiB, or it has no `url`. |
| `/v1/read` | `not_text` | 422 | The page is not text, HTML, JSON or XML. Use `/v1/ocr` for an image or `/v1/range_hash` for bytes. |
| `/v1/ocr` | `invalid_argument` | 400 | `lang` is not a tesseract language code, `image_b64` does not decode, or the body does not have exactly one of `url` and `image_b64`. A body over 11188906 bytes also gets this code. That limit holds an 8 MiB image in base64. |
| `/v1/ocr` | `too_large` | 400 | The uploaded image is over 8 MiB. |
| `/v1/ocr` | `ocr_unavailable` | 501 | The node has no OCR engine. emem.dev has one on 7 November 2026. |
| `/v1/ocr` | `not_an_image`, `ocr_failed` | 422 | The bytes are not PNG, JPEG, TIFF, WebP, GIF or BMP, or tesseract failed on them. |
| `/v1/ocr` | `busy` | 503 | The responder closed its OCR pool. A normal request waits for one of 2 OCR slots. |
| `/v1/lab_report_parse`, `/v1/land_record_parse` | `invalid_argument`, `too_large` | 400 | `invalid_argument`: the body has none of `text`, `ocr`, `url` and `image_b64`, `ocr.text` is missing, or the body is over 11188906 bytes. `too_large`: the text is over 800000 bytes of UTF-8. |
| `/v1/lab_report_parse`, `/v1/land_record_parse` | `ocr_receipt_invalid`, `text_hash_mismatch` | 422 | The `ocr` object is not a receipt that this responder signed, or `ocr.text` does not hash to `ocr.text_blake3_b32`. |
| `/v1/decide` | `invalid_argument` | 400 | The body is over 32 KiB, has no `state`, has 0 or more than 8 questions, or `state` is over 8000 characters. A question can also be bad. Its `kind` must be `choice`, `bool` or `score`. A `choice` needs 2 to 8 `options`, and a `score` scale needs 2 to 8 steps. The message starts with `question <i>:`. |
| `/v1/decide` | `busy`, `model_unavailable` | 503 | The responder closed its decision pool, or the model did not answer. |
| `POST /v1/reviews` | none | 400 | Top-level `invalid_argument`, no `details`. `subject_kind` is not `fact`, `cell`, `request_id`, `session`, `band`, `endpoint` or `other`. Or `outcome` is not `success`, `partial`, `failed` or `noisy`, `task` is empty, or `rating` is over 5. |
| `POST /v1/reviews` | none | 413 | Top-level `invalid_argument`, no `details`. `task` is over 512 bytes of UTF-8, or `comment` is over 4096 bytes. The message says "chars", but the check counts bytes. |
| `POST /v1/reviews` | none | 503 | Top-level `cache_error`. The responder has no durable store, so it cannot keep reviews. |

The fetch codes of `/v1/range_hash` (`url_refused`, `unresolvable`,
`too_many_redirects`, `redirect_refused`, `fetch_failed`) also come from
`/v1/read`, from `/v1/ocr` with a `url`, and from the two parse routes with a
`url`. These routes share one fetcher. `/v1/read`, `/v1/ocr` and the parse
routes with a `url` add two 502 codes. `upstream_status` means the upstream
did not send a 2xx status. `too_large` means it sent more than the cap:
4 MiB for `/v1/read`, 8 MiB for `/v1/ocr`. A parse route with `url` or
`image_b64` runs OCR first, so it can also send every `/v1/ocr` code.

The top-level code of all these routes follows one rule in `range_hash::refuse`.
HTTP 502 gives `source_fetch_failed`. Every other status gives
`invalid_argument`, also 422, 500, 501 and 503. Branch on `details.code`.

## Refusals from the history, comparison and region reads

These reads send a top-level code with no `details.code`. Branch on `code`
and the HTTP status. The message says which check failed. We caused each row
marked "live" on emem.dev on 7 November 2026. The other rows come from the
code in `crates/emem-primitives/src/` and `post_*` handlers in `lib.rs`.

| Route | Cause | HTTP | `code` |
|---|---|---|---|
| `/v1/trajectory` | No `window` and not both `from_date` and `to_date` (live). | 400 | `invalid_argument` |
| `/v1/trajectory` | A `from_date` or `to_date` is not `YYYY-MM-DD` (live). | 400 | `invalid_argument` |
| `/v1/recall`, `/v1/trajectory`, `/v1/query_region`, `/v1/find_similar` | `as_of_signed_at` is not RFC 3339 (live). The message contains `invalid_signed_at_format`. | 400 | `invalid_argument` |
| `/v1/recall` | You sent a `tslot` and an `as_of_tslot` that is less than it (live). The message contains `invalid_temporal_bound`. | 400 | `invalid_argument` |
| `/v1/diff` | No `tslot_a` or `date_a`, or no `tslot_b` or `date_b` (live). | 400 | `invalid_argument` |
| `/v1/diff` | A `date_a` or `date_b` is not `YYYY-MM-DD` (live). | 400 | `invalid_argument` |
| `/v1/diff` | `tslot_a` and `tslot_b` are equal, so the delta is always zero (live). | 400 | `invalid_argument` |
| `/v1/diff` | No fact at one of the two tslots (live). Call `/v1/trajectory` to list the tslots that hold facts. | 404 | `cid_not_found` |
| `/v1/compare_bands` | You sent a tslot, and no fact is at that tslot (live). | 404 | `cid_not_found` |
| `/v1/compare_bands` | You did not send a tslot, and the band has no fact at the cell. This is not an error (live, also for an unknown band). The response has `bands_with_no_history` with `reason` `EmptyHistory`, and `metric` is null. | 200 | none |
| `/v1/compare_bands` | One value is a scalar and one is a vector. Or two vectors have different lengths, or they come from two different vector bands. | 400 | `invalid_argument` |
| `/v1/diff`, `/v1/compare_bands` | A fact is not a primary fact, or its value is not a number or a vector. | 500 | `internal` |
| `/v1/compare_same_doy` | `doy` is not in 1 to 366 (live), or `years` is empty. | 400 | `invalid_argument` |
| `/v1/query_region` | The body has no `geometry` and no `bbox` (live). | 400 | `invalid_argument` |
| `/v1/query_region` | The `bbox` array has a value that is not finite, or it is not `[west, south, east, north]` with west <= east and south <= north (live). Or it samples zero cells. | 400 | `invalid_argument` |
| `/v1/query_region` | A `geometry` string in the form `bbox:lon_min,lat_min,lon_max,lat_max` is bad. It does not have 4 numbers (live), a latitude or longitude is out of range, or a minimum is more than its maximum. Split a box that crosses the antimeridian into two. | 400 | `invalid_cell` |
| `/v1/query_region` | A `geometry` covers more than 4096 cells. This applies to a `cells:` list and to a `bbox:` string. | 400 | `invalid_cell` |
| `/v1/query_region` | A `geometry` starts with `{`. The route does not read GeoJSON. Send a `bbox` or a `cells:` list. | 400 | `invalid_cell` |
| `/v1/backfill` | The body has no `cell` and no `cells` (live). | 400 | `invalid_argument` |
| `/v1/backfill` | The `cells` list is empty or has more than 64 entries (live). Send a longer list in pages. | 400 | `invalid_argument` |
| `/v1/backfill` | You sent `cells`, and one cell fails, for example with an unknown band (live). This is not an error. The response is 200 with `converged` false. The cell is in `pending` with `state` `upstream_failed`, and `reason` holds the error message. | 200 | none |
| `/v1/edges/recall` | `direction` is not `out` or `in` (live). | 400 | `invalid_argument` |
| `/v1/edges/recall` | Direction `out` (the default) with an empty `subj` (live), or with `obj` also set (live). | 400 | `invalid_argument` |
| `/v1/edges/recall` | Direction `in` with an empty `obj`, or with `subj` also set (live). The responder uses `in` when you send only `obj`. | 400 | `invalid_argument` |
| `/v1/edges` (write) | The signed attestation has an empty `edges` array. | 400 | `invalid_argument` |
| `/v1/memory_contradictions` | `min_severity` is not in 0 to 1 (live). | 400 | `invalid_argument` |
| `/v1/memory_contradictions` | The window low value is more than the high value (live). POST sends `window_unix_s`; GET sends `window_lo` and `window_hi`. | 400 | `invalid_argument` |
| `/v1/find_similar` | The responder holds no vector for the key in that band. The message tells you which bands the cell has. | 404 | `cid_not_found` |
| `/v1/find_similar` | An `inline:` vector has a value that is not a number, or it has more than 16384 values. | 400 | `invalid_argument` |

When a check is in a primitive, the message starts with `InvalidArgument:`.
The trajectory window and date checks are in the handler, so their messages
do not have this prefix.

The same bad box gets two codes. A `bbox` array gets `invalid_argument`. A
`bbox:` string in `geometry` gets `invalid_cell`. A `bbox` array with a
value that is not a number fails before the handler runs. It gets 400
`invalid_argument` with `details.deserializer_error` (live).

## JSON-RPC errors from `/mcp` and `/a2a/tasks`

The two JSON-RPC endpoints have a fourth layer of codes. These are numbers
in the JSON-RPC range -32768 to -32000. They come back in the top-level
`error` object, not in `result`.

The MCP endpoint applies one rule to a failed tool call. A code in that
range goes to `error`. Usually this is a protocol failure: the tool name is
unknown, or the arguments do not deserialize, or the response did not
serialize. Two tools also send -32000 there (see the table). Every other
failure is a tool error. It comes back as a `result`
with `isError: true`, and the number is the negative of the protocol code
(-27 is `invalid_cell`).

`POST /mcp` (and `/mcp/full`):

| Code | HTTP | When |
|---|---|---|
| -32600 | 200 | The body is not valid JSON-RPC. `id` is `null`. |
| -32601 | 200 (404 when the request sends `MCP-Protocol-Version: 2026-07-28`) | The method does not exist. |
| -32602 | 200 | The tool name is unknown (the message names close matches), or the arguments have the wrong type. `tasks/get` and `tasks/result` also send it for a missing or unknown `taskId`. |
| -32603 | 200 | The responder could not serialize its own answer, or a tool task panicked or stopped before it finished. |
| -32000 | 403 | The `Origin` header is not on the allowlist, and the call can write. Unlisted origins can call the handshake, the listings and read-only tools. Operators add origins with `EMEM_MCP_ALLOWED_ORIGINS`. An unlisted origin that sends a body that is not JSON-RPC also gets this 403, not -32600. |
| -32000 | 200 | The task registry is full of running tasks. Retry later, or call the tool without a task. A REST error whose `code` is not one of the 27 also comes back as -32000. |
| -32000 | 200 | `emem_cell_geojson` cannot decode the cell, or `emem_cell_scene_rgb` finds no scene or cannot read it. These come back in `error`, not as a tool error. The message gives the reason. |
| -32002 | 200 | You called `tasks/result` before the task finished. Poll `tasks/get` first. |
| -32022 | 400 | The `MCP-Protocol-Version` header names a version this responder does not support. `data.supported` lists the versions. |
| -32020 | 400 | With header version `2026-07-28`, the `Mcp-Method` header or `_meta` protocol version disagrees with the body. |
| -32050 | 200 | A model or perception service did not answer, or refused. When only the optional model reading fails, the code is in a `model_answer` field and the signed `answer` is still there. |

`POST /a2a/tasks`:

| Code | HTTP | When |
|---|---|---|
| -32009 | 400 | The `A2A-Version` header names a version other than 0.3 or 1.0. No header means 0.3. |
| -32601 | 200 | This endpoint does not have the method (`data.supported` lists the methods). Or the skill answers only over HTTP (`data.call_it_here` gives the URL). |
| -32602 | 200 | The skill refused the call, and `data.mcp_error_code` carries the tool error number. A bad request shape also causes it. The message has no parts, or no skill and no question. Or a text part for a structured skill is not JSON, or the text names no confident place. These refusals have no `data.mcp_error_code`. |
| -32001 | 200 | The task id is unknown. Tasks stay in memory only, so a restart clears them. Send the task again. |
| -32000 | 200 | The responder could not start a background task for the call. |

The agent card also lists -32002 `TaskNotCancelable`. No A2A path sends it.

`GET /mcp` with `Accept: text/event-stream` gives HTTP 405 and the header
`allow: POST, OPTIONS`. The body is plain JSON, not JSON-RPC, with `error:
sse_stream_not_offered`. The endpoint is stateless and has no stream. Send
JSON-RPC over POST to the same URL. A `GET /mcp` without that `Accept` value
gives the discovery document.

We sent requests to emem.dev on 7 November 2026. Each gave the code and
status in the tables. From `/mcp` we got -32600, -32601, -32602 (three
causes), -32022, -32020, the 403 -32000 and the tool error -27. We also
got the 403 for a bad body, and the `emem_cell_scene_rgb` -32000. From
`/a2a/tasks` we got -32009, -32001 and -32601. We did not cause -32603,
-32002, -32050 or the task registry -32000 live. Those rows come from
the code in
`crates/emem-api-rest/src/lib.rs`.

```sh
curl -s -X POST https://emem.dev/mcp -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"emem_nope","arguments":{}}}'
```

```json
{"error":{"code":-32602,"message":"Unknown tool: emem_nope. Did you mean emem_ndvi or emem_ocr? emem_tools lists the catalog."},"id":1,"jsonrpc":"2.0"}
```

## Responses that are not `emem.error.v1`

Some routes send a smaller JSON body. It has a `code`, and sometimes `error`
in place of `message`. Most of these bodies have no `schema`. The
`/postcard` bodies are the exception: `painting` carries
`emem.postcard_pending.v1`, and the two error bodies (`upstream_unavailable`,
`upstream_error`) carry `emem.error.v1`.

| Route | `code` | HTTP | Meaning |
|---|---|---|---|
| `/v1/range_hash` | `range_hash_quota` | 429 | This IP used its daily quota (default 500, `EMEM_RANGE_HASH_DAILY_QUOTA`). `Retry-After: 86400`. |
| `/v1/decide` | `decide_quota` | 429 | This IP used its daily quota (default 2000, `EMEM_DECIDE_DAILY_QUOTA`). `Retry-After: 86400`. |
| `/v1/read` | `read_quota` | 429 | This IP used its daily quota (default 500, `EMEM_READ_DAILY_QUOTA`). `Retry-After: 86400`. |
| `/v1/ocr`, and the parse routes with `url` or `image_b64` | `ocr_quota` | 429 | This IP used its daily OCR quota (default 200, `EMEM_OCR_DAILY_QUOTA`). `Retry-After: 86400`. |
| `/v1/lab_report_parse`, `/v1/land_record_parse` with `text` | `doc_parse_quota` | 429 | This IP used its daily parse quota (default 2000, `EMEM_DOC_PARSE_DAILY_QUOTA`). `Retry-After: 86400`. |
| `/v1/artifacts/<cid>` | `artifact_evicted_or_unknown` | 404 | This responder holds no bytes under that cid. Send the same `POST /v1/band_raster` again to make the bytes again. |
| `/v1/artifacts/<cid>` | `invalid_argument` | 400 | The value is not a 52-character artifact cid. |
| `/v1/cells/<cell64>/scene.png`, `/v1/cells/<cell64>/scene.rgb` | `no_clear_scene` | 404 | No Sentinel-2 L2A scene passed any of the searches. |
| `/v1/cells/<cell64>/scene.png`, `/v1/cells/<cell64>/scene.rgb` | `scene_upstream_failed` | 502 | The scene search upstream failed. |
| `/postcard` | `painting` | 202 | Not an error. The picture is still in progress. Ask again later. |
| `/postcard` | `upstream_unavailable`, `upstream_error` | 502 or upstream status | The service that makes the picture is down, or it failed with no reason. |

### Plain-text refusals of a bad request body

Most POST routes catch a bad JSON body and send `400 invalid_argument` with
`details.deserializer_error`. The routes below do not. They use the plain
`axum` JSON extractor, so a bad body gets a `text/plain` answer with no
`code`. Branch on the HTTP status.

| HTTP | When | Text starts with |
|---|---|---|
| 400 | The body is not valid JSON. | `Failed to parse the request body as JSON:` |
| 415 | The `content-type` header is not `application/json`. | `` Expected request with `Content-Type: application/json` `` |
| 422 | The JSON is valid, but a field has the wrong type or is missing. | `Failed to deserialize the JSON body into the target type:` |

The routes are `/v1/locate`, `/v1/guard/verdict`, `/verdict` and its six
shape routes, `/v1/tree/path`, `/v1/trace_verify`, `/v1/enroll_verify`,
`/v1/trace_resolve`, `/v1/attest_traced`, `/v1/enroll_attested`,
`/v1/log/witness` and `/v1/benchmark/grade`. The list comes from the handlers
that take `Json<..>` in place of `EmemJson<..>` in `crates/emem-api-rest/src/`.
On 7 November 2026, `/v1/locate` gave all three statuses. `/v1/tree/path`,
`/v1/trace_verify` and `/verdict/mcp` gave the 400.

Every route also has a body size cap. A body over the cap gets `413` with
the plain-text body `length limit exceeded`. The cap is `EMEM_BODY_LIMIT_MB`
mebibytes: default 16, at least 1, at most 256 (`body_limit_bytes` in
`crates/emem-api-rest/src/lib.rs`). On 7 November 2026, emem.dev sent 413
for a `Content-Length` of 16777217 bytes. It did not refuse 16777216.

### Errors from the perception proxy

`/v1/perception/<path>` forwards requests to a separate camera service.
emem.dev does not run that service. The proxy adds three bodies of its own.
Two of them have no `code` field, so branch on the HTTP status.

| Case | HTTP | Body |
|---|---|---|
| The path is not on the allowlist. | 404 | `error`, `fronted` (allowed paths), `fronted_trees` (allowed prefixes), `why`. No `code`. |
| The service answered with a 5xx status. | 502 | `error`, `code: "upstream_failed"`, `upstream_status`, `upstream_body` (first 512 bytes), `fronted_by`, `note`. The `x-emem-upstream-status` header repeats the upstream status. |
| The service did not answer. | 502 | `error`, `detail` (the transport error), `note`. No `code`. |

A path under a prefix in `fronted_trees` also gets the 404 when it breaks a
segment rule. The path can have 8 segments at most. A segment must not be
empty or `..`. It can use only ASCII letters, digits, `-`, `_` and `.`. An
operator can add single-segment paths with `EMEM_PERCEPTION_EXTRA_PATHS`, a
comma-separated list. These rules are in `perception_proxy` in
`crates/emem-api-rest/src/lib.rs`.

A status below 500 from the service comes back unchanged, with the body of
the service. Three other failures send a bare status with no body:

- 500: the proxy cannot build its HTTP client.
- 405: the method is HEAD. Other methods such as DELETE get the JSON
  `method_not_allowed` body of the router, with `allow: ["GET","POST"]`.
- 502: the proxy cannot read the body that the service sent.

On 7 November 2026, `GET /v1/perception/nope` gave the 404 body.
`GET /v1/perception/clips/a%20b` also gave it, because a space breaks the
segment rule. `POST /v1/perception/health` gave the 405 of the service.

### Errors from the `/splats` static files

`GET /splats/<path>` serves the files of the 3D viewer. The files are in
`EMEM_SPLATS_DIR` (default `<EMEM_DATA>/splats-site`). These handlers are
`serve_splats_file` and `splats_post` in `crates/emem-api-rest/src/lib.rs`.

| Case | HTTP | Body |
|---|---|---|
| A path segment is empty, `.` or `..`. | 404 | `{"code":"not_found","message":"bad path"}`. No `schema`. |
| No file is at the path. | 404 | `{"code":"not_found","message":"not found"}`. No `schema`. |
| The responder cannot read the file. | 404 | `{"code":"not_found","message":"read error"}`. No `schema`. |
| The `Range` header asks for bytes after the end of the file. | 416 | Empty. The `content-range` header is `bytes */<file length>`. |
| `POST` to a path other than `api/gemma`. | 404 | `{"code":"not_found","message":"not found"}`. |
| `POST /splats/api/gemma` and the local model bridge does not answer in time, or the responder cannot read its body. | 502 | Empty. |

The responder does not refuse every bad `Range` header. A header with more
than one range, or a header that does not parse, gets the whole file with 200.
An `If-Range` value that is not the current `etag` also gets the whole file.
`POST /splats/api/gemma` forwards the body to a model bridge on
`127.0.0.1:8080`. It returns the status and body of the bridge unchanged. The
time limit is `EMEM_SPLATS_GEMMA_TIMEOUT_S` seconds (default 120).

On 7 November 2026, emem.dev sent 416 with `content-range: bytes */8009` for
`Range: bytes=999999999-` on `/splats/viewer/temporal.html`. It sent 206 for
`bytes=0-9` and 200 for `bytes=0-1,4-5`. `/splats/a//b` (sent with
`curl --path-as-is`) gave the `bad path` body.

### Errors from the OAuth routes

`POST /oauth/token` uses the OAuth 2.0 error body of RFC 6749: an `error`
field and an `error_description` field. It has no `code` and no `schema`.
Every refusal is HTTP 400.

| `error` | Cause |
|---|---|
| `unsupported_grant_type` | `grant_type` is not `authorization_code` or `refresh_token`. |
| `invalid_grant` | The `code` is missing, does not verify or has expired. Or `redirect_uri` does not match the code, or the PKCE S256 check failed. For a refresh, the `refresh_token` is missing, wrong or expired. |

`GET /oauth/authorize` sends an `emem.error.v1` body with `invalid_argument`
when `client_id` does not verify, or when the client did not register that
`redirect_uri`. After
those two checks pass, it sends a 302 redirect to your `redirect_uri`. A
`response_type` other than `code` adds `error=unsupported_response_type` to
that redirect. A `code_challenge_method` other than `S256` adds
`error=invalid_request`. `POST /oauth/register` sends `invalid_argument`
when `redirect_uris` has more than 10 entries, or an entry is empty or longer
than 512 characters. These handlers are `oauth_token`, `oauth_authorize` and
`oauth_register` in `crates/emem-api-rest/src/lib.rs`.

On 7 November 2026, `POST /oauth/token` with `grant_type=password` gave
`unsupported_grant_type`. With `grant_type=refresh_token&refresh_token=x` it
gave `invalid_grant`. `GET /oauth/authorize?client_id=nope` gave the 400
`invalid_argument` body.

## Routes that only a self-hosted node answers

emem.dev answers these paths with HTTP 501 and an `emem.error.v1` body. They
work on a node that you run yourself. All six paths gave these codes on
emem.dev on 7 November 2026.

| Route | `code` |
|---|---|
| `POST /verdict/anthropic-hook`, `POST /verdict/claude-code` | `requires_your_own_node` |
| `GET /log/head`, `GET /log/entries`, `GET /log/entry/<leaf>` | `no_verdict_log_here` |
| `GET /modules` | `no_modules_here` |

## Deny codes from the guard

A guard verdict is not an error. The guard answers HTTP 200 with
`action: "allow"` or `action: "deny"`, also when it denies. A deny has a
`code` in capital letters and a `fix` in lower case. Branch on `fix` to
decide what to do next. The `reason` line has a fixed grammar:

```text
EMEM-GUARD DENY <code> token=<token or -> fix=<fix> leaf=<leaf id or ->
```

Two servers send these codes. `POST /v1/guard/verdict` on emem.dev is
advisory and blocks nothing. The `emem-guard` binary is a checkpoint that you
run yourself. The codes and fixes are in `crates/emem-guard/src/policy.rs`,
and `GET /v1/guard/capabilities` lists them. The rules run in the order of
this table, and the first match wins.

| `code` | Meaning | `fix` | Sent by emem.dev |
|---|---|---|---|
| `PROV_SIG` | The signature of a cited token did not verify. | `refresh_token` | No. See the note below the table. |
| `PROV_BYTES` | A cited fact token names a different cell than the fact it resolves to. | `remove_reference` | Yes |
| `PROV_VALUE` | A sentence states a number that does not agree with the one fact that it cites. | `correct_value` | Yes |
| `POLICY_MODULE` | A detection module that the operator loaded denied. The verdict names the module. | Set by the module, for example `redact_and_retry` | No. Self-hosted, with `--module` or `--signed-module`. |
| `PROV_DRIFT` | A cited reading drifted past the threshold of its band. | `refresh_token` | No. See the note below the table. |
| `GEO_ZONE` | A cited cell is in a zone that the operator restricted. | `contact_admin` | No. Self-hosted, with `--restrict-cell`. |
| `CLAIM_UNGROUNDED` | A measurable claim about a place has no citation that resolved. | `cite_observation` | Only with `claim_gating: true` in the body or `?claim_gating=true` in the query. The query wins. Self-hosted, with `--claim-gating`. |

No resolver in the code sets the token states that give `PROV_SIG` or
`PROV_DRIFT`. Only unit tests make them. The two codes are in the published
list, but no shipped server sends them on 7 November 2026.

A citation that the responder does not hold is never a deny. In the
`citations` array of an emem.dev verdict, its `state` is `not_held_here`.
In shadow mode (`--shadow`), a self-hosted guard answers `allow` with
`shadowed: true` and keeps the `code` that it would have sent.

The guard does not refuse a bad request. The `emem-guard` webhook routes
answer `allow` when they cannot parse the body. On emem.dev, a body that is
not JSON gets a plain-text 400. A wrong `content-type` gets a plain-text 415
(see [Plain-text refusals](#plain-text-refusals-of-a-bad-request-body)). A
JSON object with no text in it gets `allow` with `checked: 0`. Two routes of
the `emem-guard` binary send a small error body:

| Route | HTTP | Body |
|---|---|---|
| `GET /log/entry/<leaf>`, leaf is not `leaf_<n>` or `<n>` | 400 | `{"error": "leaf must be leaf_<n> or <n>"}` |
| `GET /log/entry/<leaf>`, no such entry | 404 | `{"error": "no such leaf", "leaf": "<leaf>"}` |

On 7 November 2026, `POST /v1/guard/verdict` on emem.dev gave these results:

| Text sent | `action` | `code` | `fix` |
|---|---|---|---|
| `Elevation in Leh is 3500 m.` with `"claim_gating": true` | `deny` | `CLAIM_UNGROUNDED` | `cite_observation` |
| `Mount Fuji mean elevation is 3539 m per <token>.` | `allow` | none | none |
| The same sentence with `2100 m` | `deny` | `PROV_VALUE` | `correct_value` |
| `<token>` with the cell changed to `defi.zb493.xuqA.zcb5f` | `deny` | `PROV_BYTES` | `remove_reference` |
| A token with a cid that the responder does not hold | `allow` | none | none |

The token was
`emem:fact:defi.zb592.nUkO.zEzE:qejpz7cgvk347ordd6vrdtmwzprqlzcnkmotsjton2x4y2jjbplq`,
the `copdem30m.elevation_mean` fact at Mount Fuji (value 3539.04 m). This is
the request for the first row:

```sh
curl -s https://emem.dev/v1/guard/verdict -H 'content-type: application/json' \
  -d '{"texts":["Elevation in Leh is 3500 m."],"claim_gating":true}'
```

## Endpoints that help you recover

- `GET /v1/errors`: the protocol-code catalogue, live
- `GET /v1/manifests`: current registry, schema, band and source cids
- `GET /v1/bands`: the band catalogue, with `tempo` and `privacy.class` for
  each band
- `GET /v1/limits`: the caps in `enforced` (batch sizes, the per-attester
  write rate, the MCP result size) and the slowest calls seen in `measured`.
  It does not list the read rate limit.
- `POST /v1/verify_receipt`: debug a `bad_signature`. Compare its
  `preimage_blake3_hex` with the digest that you computed.
