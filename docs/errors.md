# emem error reference

An emem error is a JSON object with a stable `code`. Programs branch on that
code. The `message` is for people and can change.

There are three layers of codes:

- 27 protocol codes. These are the `ErrorCode` enum in
  `crates/emem-core/src/error.rs`. Each has a fixed MCP error number.
- A few codes that the router and the load layer send. They are not in the
  enum.
- Route codes in `details.code`. A route uses them to say which of its own
  checks refused the request. The top-level `code` is then one of the 27.

The live catalogue is `GET /v1/errors` (schema `emem.errors.v1`). It lists the
27 protocol codes with their MCP numbers, meanings and recovery steps. We
checked this page against that endpoint and against the code on 7 November
2026. When this page and the live endpoint disagree, trust the live endpoint.

## The error envelope

Every error from a REST route is an `emem.error.v1` object:

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
`isError: true`. The text is `tool error (-N): <code>: <message>`, then the
`details` JSON if there is any. An MCP caller can branch on the code name
without the number.

## How to branch on an error

1. Switch on `code`. Then switch on `details.code` when it exists. Several
   refusals share one top-level code and differ only in `details.code`. For
   example, an unsigned memory write is HTTP 401 with `code:
   invalid_argument` and `details.code: memory_attestation_required`.
2. Do not switch on the HTTP status alone. One code can have different
   statuses on different routes. `band_not_in_registry` is 400 from
   `/v1/recall` and 404 from the storage layer.
3. Do not treat a signed absence as an error. A `/v1/recall` fact with
   `kind: "absence"` and a `reason_cid` is a signed answer: this band has
   no data at this cell. A retry gives the same answer.
4. Read `materialize_notes` on a successful recall. A band that the
   responder skipped or could not fetch appears there, not in the error.

## Examples from emem.dev

We ran these requests on 7 November 2026.

A path that does not exist gives HTTP 404 with `code: not_found`:

```sh
curl -s https://emem.dev/v1/no-such-route
```

The body is short. All error bodies have `schema: emem.error.v1`:

```json
{"code":"not_found","did_you_mean":[],"message":"no route matched /v1/no-such-route. ...","path":"/v1/no-such-route","schema":"emem.error.v1"}
```

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

## Protocol codes (27)

These are all the variants of `ErrorCode`. The MCP number is the enum
discriminant, negated.

| Code | MCP | Meaning | First thing to try |
|---|---|---|---|
| `invalid_cell` | -27 | The cell64 string did not parse. The responder does not geocode a string with a `.` in it as a place. So it refuses a mistyped cell and does not send it somewhere random. | Re-derive it with `POST /v1/locate`. |
| `invalid_resolution` | -1 | Resolution out of range. | Use the cell64 `/v1/locate` returns; see `/v1/grid_info`. |
| `tslot_mismatch` | -2 | The tslot does not sit on the band's tempo grid. | Read the band's `tempo` in `/v1/bands`. |
| `band_not_in_registry` | -3 | The band key is not in the active manifest. For a band family, `details.did_you_mean` names its wired keys. | `GET /v1/bands`. Band keys have a namespace: `copdem30m.elevation_mean`, not `elevation_mean`. |
| `function_not_in_registry` | -4 | No registry entry matches the derivation `fn_key`. | `GET /v1/functions`. |
| `source_scheme_unknown` | -5 | `source.scheme` is not in `/v1/sources`. | `GET /v1/sources`. |
| `cid_not_found` | -6 | This responder holds nothing under that content id. Memory reads say whether a tombstone exists. So you can tell "never written" from "written then removed". | Check the cid came from this responder (`responder_pubkey_b32` against `/health`). |
| `no_geocoder_match` | -7 | Every geocoder answered and none knows the place (HTTP 404). Or the name was too ambiguous to pick one: HTTP 400 with `details.code: ambiguous_place`. | Add a region or country, or pass `lat` and `lng`. |
| `registry_cid_unknown` | -8 | The responder does not know the bound `registry_cid`. | `GET /v1/manifests`. |
| `schema_cid_unknown` | -9 | The responder has not loaded the fact's `schema_cid`. | `GET /v1/manifests`. |
| `privacy_refused` | -10 | The band's privacy class blocks this query at this resolution. On 7 November 2026 no route returns it: the responder does not enforce privacy classes ([protocol §10](protocol.md#10-privacy-classes)). | Query a coarser cell; see `privacy.class` for the band in `/v1/bands`. |
| `level_too_low` | -11 | The operation needs a higher tier. The fact plane refuses any attestation from a key that is not the responder's materialiser, an enrolled device, or an operator-listed writer. | `GET /v1/enlist` for what each tier can write. |
| `attester_revoked` | -12 | The operator revoked the attester key. | Sign with a current key. |
| `unauthorized` | -13 | A write has no attester signature, or the signature is not valid. | Sign again. The refusal says what the responder expected. |
| `claim_undecidable` | -14 | The claim cannot resolve to true or false (a missing value, NaN, a type mismatch). | Recall the fact and check its value and type. |
| `bad_signature` | -15 | An ed25519 signature did not verify. | For receipts, `/v1/verify_receipt` returns `preimage_blake3_hex` to compare with your own. For writes, see the tables below. |
| `unaddressable_subject` | -16 | A fact's subject is neither a cell64 nor an `emem:entity:` identity. The responder refuses the write before the log. Your signature is not the problem. | Send a cell64 or an entity id as the subject. |
| `bad_merkle_proof` | -17 | An inclusion proof has a bad shape, or it does not reach its root. | Rebuild it from the canonical leaf order; check `leaf_index`. |
| `canonical_encoding_divergence` | -18 | The CBOR sent is not deterministic per RFC 8949 section 4.2.1. | Encode with sorted map keys and shortest-form integers. |
| `source_fetch_failed` | -19 | An upstream fetch failed at the transport (non-2xx, DNS, timeout, bad JSON) before the upstream gave an answer. On a place lookup this means the geocoder is down, not that the place is absent. | Retry with backoff. |
| `source_format_mismatch` | -20 | The fetched bytes do not match the declared format. | Report it; the provider may have changed format. |
| `compute_timeout` | -21 | A derivation ran past its deadline. | Send a smaller input, or retry. A band read that passes the 14 s dispatch cap keeps running. A retry often finds the value ready. |
| `compute_quota_exceeded` | -22 | You used all of a per-key, per-day compute quota. | Slow down. `/v1/limits` publishes the windows. |
| `rate_limited` | -23 | Too many requests. Per IP the default is 600 per minute with a burst of 120. Memory writes also have a per-attester backstop (`details.code: write_rate_limited`). | Honour `Retry-After` (seconds) or `details.retry_after_s`. |
| `cache_error` | -24 | The responder's store returned an internal error. | Retry once. |
| `invalid_argument` | -25 | A field is syntactically valid but semantically wrong, or the body did not match the route's schema (`details.deserializer_error` names the field). Also the top-level code for most route refusals below. | Read `message` and `details`; the request shape is in `/openapi.json`. |
| `internal` | -26 | A responder bug. Receipts already signed stay valid. | File the request and response at [github.com/Vortx-AI/emem/issues](https://github.com/Vortx-AI/emem/issues). |

## Codes from the router and the load layer

The router and the load layer send these envelopes before any handler runs.
Their codes are not in the enum.

| Code | HTTP | When |
|---|---|---|
| `not_found` | 404 | No route matches the path. `did_you_mean` suggests near paths. |
| `method_not_allowed` | 405 | The path exists but not for that method. The `allow` header and body field list the methods it answers. With `Accept: text/html`, the body is an HTML page. |
| `overloaded` | 503 | The server is at its cap of requests in flight. It refused the request at once and did not queue it. `Retry-After: 1`. |

## Memory and entity write refusals

Memory verbs name the refusal in `details.code`. A refusal about a signature
also carries `details.how_to_sign`. It holds the exact digest the responder
verifies for this write, the encoding rules and a worked example.

| `details.code` | Top-level `code` | HTTP | Meaning |
|---|---|---|---|
| `memory_attestation_required` | `invalid_argument` | 401 | The write has no `attester` block. emem.dev refuses unsigned writes. |
| `memory_attestation_invalid` | `bad_signature` | 401 | The key has a bad shape (`reason: bad_pubkey`) or the signature does not verify (`reason: bad_signature`). The message names the expected digest. Every verb verifies only the v2 preimage, `emem.memory_write.v2\|verb\|path\|body_hash\|base`. The responder refuses a v1 signature with `reason: v1_retired` (since 2026-11-05). |
| `memory_namespace_violation` | `invalid_argument` | 403 | The path is in the `/memories/by_attester/<pubkey8>/` namespace of another key, or another key owns it, or it is under the reserved `/memories/.well-known/` prefix. |
| `write_rate_limited` | `rate_limited` | 429 | The per-attester write backstop tripped. The defaults are 240 writes per minute with a burst of 60. |
| `vault_immutable` | `invalid_argument` | 409 | You sent an in-place edit to a sealed vault entry. Create the entry again instead. |
| `vault_capability_invalid` | `bad_signature` | 403 | A `vault_capability` did not let you view that vault path. |

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
names `POST /v1/derive` as the route to add a value. `details` is empty.

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

Four more derive refusals have no `details` block. The route code is the
start of `message`.

| Start of `message` | Top-level `code` | HTTP | Meaning |
|---|---|---|---|
| `derive_uncanonicalisable_body` | `invalid_argument` | 400 | The responder cannot encode the body as canonical CBOR, so it cannot compute `body_hash`. |
| `derive_attestation_failed` | `internal` | 500 | The responder could not build or sign the attestation for the new fact. Report it. |
| `derive_store_returned_no_cid` | `internal` | 500 | The store accepted the attestation but returned no fact cid. Report it. |
| `derive_namespace_violation` | `invalid_argument` | 403 | The code maps this case, but it cannot occur today. `/v1/derive` is not under a `by_attester` path. |

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
| `/v1/inferences` | `inferences_index_unavailable` | 503 | Top-level `cache_error`. The responder has no durable store, so it has no place index. Resolve derivations by token. |
| `/v1/device_publish` | `device_key_malformed`, `device_key_invalid`, `signature_malformed`, `signature_invalid` | 400 | The device key or the device signature is bad. Only the device can sign its own listing decision. |
| `/v1/a2a/tasks` | `a2a_skill_required`, `a2a_skill_unknown` | 400 | The `skill` field is empty, or it names no skill on the agent card. |
| `/v1/building_footprints` | `needs_location` | 400 | Send `polygon_bbox`, or a `place` that resolves to a bounding box. |
| `/v1/range_hash` | `invalid_argument`, `range_too_large`, `url_refused` | 400 | The body is bad, the length is 0 or over the cap, or the URL is not public HTTPS. The cap is 16 MiB on 7 November 2026 (`EMEM_RANGE_HASH_MAX_BYTES`, at most 256 MiB). |
| `/v1/range_hash` | `unresolvable`, `fetch_failed`, `too_many_redirects`, `redirect_refused`, `range_not_honoured`, `short_read` | 502 | Top-level `source_fetch_failed`. The upstream host failed or did not return the byte range. |
| `/v1/range_hash` | `busy` | 503 | The responder closed its fetch pool. A normal request waits for one of 4 fetch slots, so you do not usually see this code. |
| `/v1/range_hash`, `/v1/read`, `/v1/ocr` | `client` | 500 | The responder could not build its HTTP client. Report it. |
| `/v1/read` | `invalid_argument` | 400 | The body is over 8 KiB, or it has no `url`. |
| `/v1/read` | `not_text` | 422 | The page is not text, HTML, JSON or XML. Use `/v1/ocr` for an image or `/v1/range_hash` for bytes. |
| `/v1/ocr` | `invalid_argument` | 400 | `lang` is not a tesseract language code, `image_b64` does not decode, or the body does not have exactly one of `url` and `image_b64`. |
| `/v1/ocr` | `too_large` | 400 | The uploaded image is over 8 MiB. |
| `/v1/ocr` | `ocr_unavailable` | 501 | The node has no OCR engine. emem.dev has one on 7 November 2026. |
| `/v1/ocr` | `not_an_image`, `ocr_failed` | 422 | The bytes are not PNG, JPEG, TIFF, WebP, GIF or BMP, or tesseract failed on them. |
| `/v1/ocr` | `busy` | 503 | The responder closed its OCR pool. A normal request waits for one of 2 OCR slots. |
| `/v1/lab_report_parse`, `/v1/land_record_parse` | `invalid_argument`, `too_large` | 400 | The body has none of `text`, `ocr`, `url` and `image_b64`, `ocr.text` is missing, or the text is too long. |
| `/v1/lab_report_parse`, `/v1/land_record_parse` | `ocr_receipt_invalid`, `text_hash_mismatch` | 422 | The `ocr` object is not a receipt that this responder signed, or `ocr.text` does not hash to `ocr.text_blake3_b32`. |
| `/v1/decide` | `invalid_argument` | 400 | The body is over 32 KiB, has no `state`, has 0 or more than 8 questions, or `state` is over 8000 characters. |
| `/v1/decide` | `busy`, `model_unavailable` | 503 | The responder closed its decision pool, or the model did not answer. |

The fetch codes of `/v1/range_hash` (`url_refused`, `unresolvable`,
`too_many_redirects`, `redirect_refused`, `fetch_failed`) also come from
`/v1/read`, from `/v1/ocr` with a `url`, and from the two parse routes with a
`url`. These routes share one fetcher. `/v1/read` and `/v1/ocr` add two 502
codes. `upstream_status` means the upstream did not send a 2xx status.
`too_large` means it sent more than the cap: 4 MiB for `/v1/read`, 8 MiB for
`/v1/ocr`.

The top-level code of all these routes follows one rule in `range_hash::refuse`.
HTTP 502 gives `source_fetch_failed`. Every other status gives
`invalid_argument`, also 422, 500, 501 and 503. Branch on `details.code`.

## Responses that are not `emem.error.v1`

Some routes send a smaller JSON body. It has a `code`, but no `schema`, and
sometimes `error` in place of `message`.

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

## Routes that only a self-hosted node answers

emem.dev answers these paths with HTTP 501 and an `emem.error.v1` body. They
work on a node that you run yourself. All six paths gave these codes on
emem.dev on 7 November 2026.

| Route | `code` |
|---|---|
| `POST /verdict/anthropic-hook`, `POST /verdict/claude-code` | `requires_your_own_node` |
| `GET /log/head`, `GET /log/entries`, `GET /log/entry/<leaf>` | `no_verdict_log_here` |
| `GET /modules` | `no_modules_here` |

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
