# emem error reference
_Generated from `/v1/errors` on 2026-05-28. Schema: `emem.errors.v1`._

This catalog is the wire truth. Every code below is something the
running responder will actually emit; nothing aspirational. Error
responses follow the `emem.error.v1` envelope:

```json
{
  "code":   "<one of the codes below>",
  "message": "human-readable detail",
  "path":   "/v1/...",
  "schema": "emem.error.v1"
}
```

An agent should `switch (error.code)` rather than match on `message`
or status. Messages may evolve; codes are stable.

## Catalog (28 codes)

| Code | HTTP | Where it surfaces | Meaning |
|---|---|---|---|
| `invalid_cell` | 422 | * | cell64 string failed grammar check (4 base-1024 bigrams joined by dots, with a valid prefix). |
| `invalid_resolution` | 422 | /v1/grid_info | resolution argument out of range for the cell tier. |
| `tslot_mismatch` | 422 | /v1/recall, /v1/trajectory | Requested tslot is not at the band's tempo cadence. |
| `band_not_in_registry` | 404 | * | Band name is not declared in the band manifest. |
| `function_not_in_registry` | 404 | /v1/algorithms | Algorithm function id is not declared in the algorithm registry. |
| `source_scheme_unknown` | 404 | /v1/fetch | Source URL scheme is not declared in the source registry. |
| `cid_not_found` | 404 | /v1/recall, /v1/recall_polygon, /v1/memory_bundle/{token} | No fact or bundle stored under the given content id at this responder. |
| `no_geocoder_match` | 404 | /v1/locate, /v1/ask | Geocoder cascade exhausted without a confident match for the place query. |
| `place_not_found` | 404 | /v1/locate | Place name not found in any embedded layer or network fallback. |
| `geocoder_transport_down` | n/a | n/a | (see /v1/errors live payload for description) |
| `invalid_argument` | 422 | * | Request payload failed validation; see message for the specific field. |
| `registry_cid_unknown` | 404 | /v1/manifests | Registry CID does not resolve at this responder. |
| `schema_cid_unknown` | 404 | /v1/manifests | Schema CID does not resolve at this responder. |
| `privacy_refused` | 403 | /v1/recall | Caller does not meet the privacy_class gate declared for this band. |
| `level_too_low` | 403 | /v1/attest | Caller trust level is below the band's declared min level. |
| `attester_revoked` | 403 | /v1/recall, /v1/memory_contradictions | Attester pubkey is on the revocation list maintained by this responder. |
| `unauthorized` | 401 | /v1/attest, /v1/memory/* | Missing or malformed bearer / signature. |
| `claim_undecidable` | 422 | /v1/verify_claim | The claim has no decidable resolution at the supplied tslot bound. |
| `bad_signature` | 401 | /v1/verify_receipt, /v1/memory/* | Ed25519 verification failed against the supplied pubkey. |
| `bad_merkle_proof` | 422 | /v1/verify_receipt | Receipt was signed but the merkle inclusion proof does not chain. |
| `canonical_encoding_divergence` | 422 | /v1/verify_receipt | Re-encoded CBOR does not match the bytes the receipt was signed over. |
| `source_fetch_failed` | 502 | /v1/recall, /v1/fetch | Upstream source connector returned a network or HTTP error. |
| `source_format_mismatch` | 502 | /v1/recall, /v1/fetch | Upstream returned a payload that does not match the registered driver schema. |
| `compute_timeout` | 504 | /v1/recall, /v1/state | Inference sidecar or upstream exceeded the per-request deadline. |
| `compute_quota_exceeded` | 429 | /v1/recall, /v1/state | Caller is past the per-window compute budget. |
| `rate_limited` | 429 | * | Caller is over the request-rate budget for the window. |
| `cache_error` | 500 | * | sled or LanceDB cache returned an unexpected error; transient, retry once. |
| `internal` | 500 | * | Unclassified server-side error; receipts already signed are still valid. |

## Recovery hints

- GET /v1/manifests  : current registry/schema/bands/sources CIDs
- GET /v1/bands      : band catalogue with tempo + privacy_class
- POST /v1/verify_receipt : debug a bad_signature with `preimage_blake3_hex`

## How agents should handle errors

1. **Switch on `code`, not on status or message.** Status is a coarse
   bucket; the same `400/422` can carry a dozen distinct codes.
2. **`signed Absence` is not an error.** A `/v1/recall` 200 carrying
   `Fact::Absence { reason_cid, signed_at, … }` is the canonical
   answer for "this band has no data at this cell." Retrying will
   not change the result.
3. **`rate_limited` / `compute_quota_exceeded`** carry a `retry_after`
   hint in the response body (seconds, integer). Honour it.
4. **`bad_signature` + `canonical_encoding_divergence`** mean the
   client and server disagree on the canonical CBOR encoding. Most
   often the client serialised maps with non-canonical key order.
5. **`internal` + `cache_error`** are transient. Retry once with a
   fresh request id; if it persists, file a bug with the receipt.
