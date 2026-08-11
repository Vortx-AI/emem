# A trimmed receipt reports authentic data as forged

Found in PR #18, reproduced against production 2026-08-11.

## The reproduction

```python
import json, urllib.request
def post(p, b):
    r = urllib.request.Request("https://emem.dev" + p, data=json.dumps(b).encode(),
                               headers={"Content-Type": "application/json"})
    return json.load(urllib.request.urlopen(r, timeout=90))

rec  = post("/v1/recall", {"cell64": "defi.zb493.xuqA.zcb5f", "bands": ["indices.ndvi"]})
full = rec["receipt"]
trimmed = {k: v for k, v in full.items() if k != "merkle_proof"}

post("/v1/verify_receipt", {"receipt": full})     # signature_valid True,  merkle_proof_valid True
post("/v1/verify_receipt", {"receipt": trimmed})  # signature_valid False, merkle_proof_valid None
```

## Why it happens, and why the cause is correct

2.0.0 moved receipts to `preimage_version: 2`, which binds the inclusion proof
into the signature. That change was right and the CHANGELOG explains why: under
v1 the signature covered a receipt's fields but not its proof, so an
intermediary could delete `merkle_proof` wholesale and the receipt still
verified, reporting itself valid with `merkle_proof_valid: null`. A downgrade
with no trace.

The consequence nobody wrote down is that receipts are now byte-for-byte or
nothing. Any SDK, wrapper or integration that reshapes a receipt, summarises it,
or drops a field it considers redundant produces `signature_valid: false` on
data that was never tampered with.

## Why it matters more than a normal false negative

A false "forged" is worse here than a missed forgery. It teaches an agent to
distrust the one thing that was actually provable, and it is indistinguishable
on the wire from real tampering: `signature_valid: false` with
`merkle_proof_valid: null` is exactly what a stripped proof looks like, because
that is exactly what it is.

## Two pieces of work

### 1. State the contract where an integrator will hit it

Receipts are byte-for-byte. Field-level reshaping invalidates the signature by
design, not by accident. That belongs in `emem_verify_receipt`'s tool
description, in the receipt schema in the OpenAPI, and in the SDK docs that
hand receipts to callers.

### 2. Make the failure legible

Today a trimmed receipt and a tampered one return the same shape. The verifier
knows the difference: `preimage_version: 2` requires the proof segment, so an
absent `merkle_proof` on a v2 receipt is a structural fault, not a signature
mismatch. Say so. An integrator should read "the proof segment is absent, and
v2 binds it" and go fix their serialiser, rather than read "forged" and go
hunting an attacker.

This is the same defect class this repo kept finding through 2026-08-10 and 11:
a true answer that reads as a different, worse answer. Guard returning `allow`
on an unresolvable citation. `checked: 1` reading as "verified". A core profile
declaring 16 tools in the same body that returned 12. Each was correct and each
misled.

## What must not regress

- A genuinely tampered receipt must still fail. Whatever distinguishes absence
  from mismatch must not become a path that accepts a bad signature.
- v0 and v1 receipts still verify under their own rules. The verifier selects
  by `preimage_version`; do not collapse that.
- `/v1/verify_receipt` and the in-browser verifier at `web/verify.html` must
  agree, along with the byte-by-byte example in `docs/protocol.md`. AGENTS.md
  pins those three as needing to stay in step.
