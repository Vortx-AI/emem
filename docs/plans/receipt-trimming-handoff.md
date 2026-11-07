# A trimmed receipt reports authentic data as forged

Status: both work items shipped. This page started as a plan. It now records
the defect, the fix, and how to check the fix. The current reference for
receipts is [docs/protocol.md](../protocol.md) and `GET /v1/verifier_spec`.

PR #18 found the defect, and a check against emem.dev reproduced it on 11
August 2026. A second check confirmed the fix on 29 September 2026. The reproduction below ran
again on 7 November 2026 with the results shown.

## Reproduce it

```python
import json, urllib.request

def post(p, b):
    r = urllib.request.Request("https://emem.dev" + p, data=json.dumps(b).encode(),
                               headers={"Content-Type": "application/json"})
    return json.load(urllib.request.urlopen(r, timeout=90))

rec  = post("/v1/recall", {"cell64": "defi.zb493.xuqA.zcb5f", "bands": ["indices.ndvi"]})
full = rec["receipt"]
trimmed = {k: v for k, v in full.items() if k != "merkle_proof"}

post("/v1/verify_receipt", {"receipt": full})     # valid True, merkle_proof_valid True
post("/v1/verify_receipt", {"receipt": trimmed})  # valid False, reason receipt_reshaped_after_signing
```

Results on 7 November 2026:

| receipt sent | `valid` | `signature_valid` | `merkle_proof_valid` | `reason` |
|---|---|---|---|---|
| full, unchanged | `true` | `true` | `true` | `null` |
| `merkle_proof` removed | `false` | `false` | `null` | `receipt_reshaped_after_signing` |
| `preimage_version` removed | `false` | `false` | `true` | `receipt_reshaped_after_signing` |
| one character of a `fact_cids` entry changed | `false` | `false` | `null` | `signature_invalid` |

For a reshaped receipt, `failure_detail` names the changed segment. It tells
the caller to fix the serialiser and verify the original bytes again. For a
tampered receipt, `failure_detail` says the rebuilt preimage does not match
the signature.

## Why the signature fails

Version 2.0.0 moved receipts to `preimage_version: 2`. The v2 preimage binds
the inclusion proof into the signature. Under v1, the signature covered the
receipt fields but not the proof. An intermediary could delete `merkle_proof`
and the receipt still verified, with `merkle_proof_valid: null`. That was a
silent downgrade, and v2 closes it.

The result is that a receipt verifies byte for byte or not at all. An SDK or
wrapper that drops, renames or summarises a field makes the signature fail.
This is true when nobody tampered with the data.

## Why a false "forged" is a problem

A false "forged" teaches an agent to distrust data that it can prove. Before
the fix, a trimmed receipt and a tampered receipt gave the same answer:
`signature_valid: false` and `merkle_proof_valid: null`. An integrator had no
way to tell a serialiser bug from an attack.

## The two work items

### 1. State the contract where an integrator reads it

Shipped. These places now say that a receipt is byte for byte or nothing:

- the `emem_verify_receipt` tool description (MCP, checked on 7 November 2026)
- the `Receipt` schema in `GET /openapi.json`
- the `receipt` field description in `crates/emem-mcp/src/lib.rs`
- `sdks/emem-py/README.md` and `sdks/emem-ts/README.md`

### 2. Make the failure clear

Shipped in `crates/emem-api-rest/src/lib.rs` (search `reshaped_segment`).
When the signature fails, the verifier finds the inclusion proof that this
responder recorded for the first fact in `fact_cids`. It rebuilds the v2
preimage with that proof. If the signature then verifies, something reshaped
the receipt after signing, and `reason` is `receipt_reshaped_after_signing`. The changed
segment is `preimage_version` when the receipt no longer says v2. Otherwise
it is `merkle_proof`.

Limits of this check:

- It never accepts a receipt. `valid` and `signature_valid` stay `false`.
- It works only when this responder holds the fact. For a receipt from
  another responder, the answer stays `signature_invalid`.
- A receipt changed in any other field fails the rebuilt preimage too. The
  answer stays `signature_invalid`.

## What must not regress

- A tampered receipt must still fail. The reshape check must never become a
  path that accepts a bad signature.
- Receipts with `preimage_version` 0 and 1 still verify under their own
  rules. The verifier selects the rule from `preimage_version`.
- `POST /v1/verify_receipt`, the in-browser verifier at `web/verify.html`,
  and the byte-by-byte example in `docs/protocol.md` must agree.

The same class of defect appeared several times in August 2026: a true answer
that reads as a different, worse answer. Guard returned `allow` on a citation
it could not resolve. Callers read `checked: 1` as "verified". A core profile
declared 16 tools in a body that returned 12.
