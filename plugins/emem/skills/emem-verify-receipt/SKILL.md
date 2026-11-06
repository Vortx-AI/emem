---
name: emem-verify-receipt
description: Verifies the Ed25519 signature of an emem receipt offline. It rebuilds the canonical BLAKE3 preimage (v1 or v2) of the receipt and checks it against the published key of the responder. Use when the user pastes a receipt or an emem response and asks if it is authentic. Also use it when an agent must prove that the responder signed a fact. Use it to check a cached answer later, with no trust in the responder and no new call to it. Ships a small Python verifier. A server-side check is the fallback.
---

# emem-verify-receipt

| | |
|---|---|
| Use it when | You must show that an emem response is authentic, now or long after you received it. |
| You need | A saved emem response or a bare receipt as a JSON file. Python 3.8 or later. No key. No network for the offline check. |
| You get | `VALID` or `INVALID`, the digest, the signer key and an exit code. |
| Calls | `scripts/verify.py` (offline). The fallback is `emem_verify_receipt` (MCP) or `POST /v1/verify_receipt`. |

> **Network use.** The commands in this skill send JSON to `https://emem.dev` and receive JSON from it. Some steps also receive an image or a raster file. `https://emem.dev` is the service that the MCP server of the plugin connects to. The commands do not execute what they download. The files under `scripts/` read local files and make no network calls.

Each emem response that serves facts has a `receipt`. This skill
rebuilds the preimage of the receipt byte for byte and hashes it with
BLAKE3. Then it checks the Ed25519 signature locally. The math is the
same as `receipt_preimage_v1` and `receipt_preimage_v2` in
`crates/emem-attest/src/lib.rs`. The signer and all emem verifiers use
that one code path.

## When to use this skill

- "Is this emem receipt authentic?"
- "I saved an emem answer last month. Can I prove that it is real
  without a call to the server?"
- Any JSON with `request_id`, `served_at`, `primitive`, `cells`,
  `fact_cids`, `signature` and `responder_pubkey_b32`.

A valid signature shows only that this responder signed these fact
cids. For the other checks, use a different skill:

| You want to know | Use |
|---|---|
| Does the number in my draft match the signed value? | [`emem-verify-before-publish`](../emem-verify-before-publish/SKILL.md) (`emem_echo_verify`, `emem_guard_verdict`) |
| Is a document or an OCR result authentic? | [`emem-document-evidence`](../emem-document-evidence/SKILL.md) |
| Can the responder show a different history to someone else? | [`emem-transparency-log`](../emem-transparency-log/SKILL.md) |

## Step 1: save a response

Call `emem_recall` with this body:

```json
{"cell": "defi.zb493.zezo.zcb35", "bands": ["weather.temperature_2m"]}
```

Without MCP, send the same body to `POST https://emem.dev/v1/recall`
and save the result as `recall.json`:

```sh
curl -s -X POST https://emem.dev/v1/recall -H 'content-type: application/json' \
  -d '{"cell":"defi.zb493.zezo.zcb35","bands":["weather.temperature_2m"]}' \
  -o recall.json
```

Any saved emem response with a top-level `receipt` works. A bare
receipt also works.

## Step 2: run the verifier

```sh
python3 "${CLAUDE_SKILL_DIR}/scripts/verify.py" recall.json
```

Claude Code sets `${CLAUDE_SKILL_DIR}` to the directory of this skill.
`scripts/verify.py` gets BLAKE3 and Ed25519 from `scripts/emem_crypto.py`.
Both files ship with this skill. They are plain Python and need no
third-party package. You do not install or download anything.

To check the crypto module against the official BLAKE3 and RFC 8032
test vectors first, run:

```sh
python3 "${CLAUDE_SKILL_DIR}/scripts/verify.py" --self-test
```

It prints one `ok` line for each vector and then `SELF-TEST PASS`.

On 6 November 2026 the Bengaluru recall above gave this output:

```
VALID
preimage_v2:  945 bytes
digest:       a3d5b29286d3d1f0a24ed396896b08d57de1972a83441cd4293f1bc3ce5c7037
signer:       777er3yihgifqmv5hmc2wwmyszgddzderzhsx6rex4yoakwomvka
primitive:    emem.recall
cells:        1
fact_cids:    12
```

The one band gave 12 facts, thus 12 fact cids. Your digest and counts
will be different, because each call has a new `request_id` and
`served_at`.

We changed `served_at` in the same file, and the script printed
`INVALID` and exited with 1. We deleted `receipt.merkle_proof`, and the
result was the same.

| Exit code | Meaning |
|---|---|
| `0` | `VALID` |
| `1` | `INVALID` |
| `2` | Bad input: not JSON, or an `emem.error.v1` body |
| `3` | The receipt has a segment that the script does not rebuild (scope, as_of or edges), or it has no `preimage_version`. Use the server check in step 4. |

## Step 3: compare the signer key

Get the published key of the responder:

```sh
curl -s https://emem.dev/.well-known/emem.json | jq -r '.responder.pubkey_b32'
```

On 6 November 2026 it was
`777er3yihgifqmv5hmc2wwmyszgddzderzhsx6rex4yoakwomvka`, the same as the
`signer` line above. A receipt can verify under a key that you do not
expect. Then it proves only that the holder of that key signed it.

## Step 4: use the server check if necessary

Use this step for exit code `3`, or when you cannot run Python. Call
`emem_verify_receipt` with the whole `receipt` object. Do not change
it:

```json
{"receipt": {"primitive": "emem.recall", "request_id": "...", "served_at": "...", "...": "..."}}
```

Without MCP, send the receipt to `POST https://emem.dev/v1/verify_receipt`:

```sh
jq -c '{receipt: .receipt}' recall.json \
  | curl -s -X POST https://emem.dev/v1/verify_receipt \
      -H 'content-type: application/json' -d @- \
  | jq -c '{valid, preimage_version, merkle_proof_valid, signer_pubkey_b32}'
```

On 6 November 2026 this returned `valid: true`, `preimage_version: 2`
and `merkle_proof_valid: true`.

The server runs the same math. It also checks the Merkle proof and all
segments and legacy versions. But this check is weaker than the offline
check: you trust the responder to report honestly on its own signature.
The page `https://emem.dev/verify` does the check locally in
JavaScript. Use it if you prefer to paste the receipt.

## What the math does

```
digest = blake3( "emem.preimage.v1\0" || u32le(len("receipt")) || "receipt" || segment* )
valid  = ed25519_verify(responder_pubkey, signature, digest)
```

A scalar segment is `tag || u32le(len) || bytes`. A list segment is
`tag || u32le(count) || (u32le(len) || bytes)*`. The segments come in
this order. An optional segment is present only when the "When" column
applies.

| Tag | Segment | When |
|----|----------|------|
| 1 | request_id | Always |
| 2 | served_at | Always |
| 3 | scope | Scoped reads |
| 4 | as_of | Bi-temporal reads |
| 5 | edges | `include:["edges"]` |
| 6 | manifest: lowercase hex of `blake3(cbor(source_versions))`, map keys in plain string order | When not empty |
| 7 | primitive | Always |
| 8 | cells | Always (list) |
| 9 | fact_cids | Always (list) |
| 10 | field: hex of the `(aoi_cid, derivation_cid)` binding | Field tokens |
| 11 | merkle: hex of the inclusion-proof binding, or of an explicit absence marker | v2 only, always |

Receipts that emem serves today have `preimage_version: 2`. Tag 11
makes a deleted `merkle_proof` visible. If you delete the proof, the
digest changes and the signature fails. A change of `preimage_version`
to 1 also fails, because the v1 digest is different from the signed v2
stream. `GET /v1/verifier_spec` publishes these rules from the compiled
constants. It includes a golden vector for the manifest encoding.

## If the check fails

| Result | What to do |
|---|---|
| `INVALID` | Someone changed the receipt, or a different key signed it. Show both keys to the user and let the user decide. |
| The signer key is not the key in `/.well-known/emem.json` | The responder can rotate keys. `responder_key_epoch` on the receipt gives the epoch that signed it. A receipt from an older epoch verifies against its historical key. |
| The hash does not match after you serialised the receipt again | `signature` and `responder` are byte arrays. Keep them as arrays, or use `signature_b32`. Do not change the order of `cells` or `fact_cids`. |
