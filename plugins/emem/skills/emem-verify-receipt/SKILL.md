---
name: emem-verify-receipt
description: Verifies the Ed25519 signature of an emem receipt. It checks the receipt through the emem_verify_receipt tool. It also states the canonical BLAKE3 preimage (v1 or v2) and the published key of the responder, so that anyone can check the signature again offline with tools that they already trust. Use when the user pastes a receipt or an emem response and asks if it is authentic. Also use it when an agent must prove that the responder signed a fact, or must check a cached answer later. The server check is fast but is not independent. The offline check needs no trust in the responder and no new call to it.
---

# emem-verify-receipt

| | |
|---|---|
| Use it when | You must show that an emem response is authentic, now or long after you received it. |
| You need | A saved emem response or a bare receipt. No key. For the independent check, a BLAKE3 tool and an Ed25519 tool that you already trust. |
| You get | From emem: `valid`, `reason`, the digest and the signer key. From the independent check: your own digest and your own signature result. |
| Calls | `emem_verify_receipt` (MCP) or `POST /v1/verify_receipt`, and `GET /.well-known/emem.json` for the published key. |

> **Network use.** The commands in this skill send JSON to `https://emem.dev` and receive JSON from it. `https://emem.dev` is the service that the MCP server of the plugin connects to. The commands do not execute what they download. This skill ships no code. Its commands read no environment variable, key or credential. The only local files that they read are responses that an earlier step saved, or a receipt that the user gives you.

Each emem response that serves facts has a `receipt`. The responder
signs a BLAKE3 digest of the fields of the receipt with its Ed25519 key.
This skill checks that signature in two ways:

- **Through emem.** `emem_verify_receipt` rebuilds the preimage and
  checks the signature on the server. It is fast. But the responder
  reports on its own signature, so this check is not independent.
- **Independently.** You rebuild the preimage byte for byte, hash it
  with BLAKE3 and check the Ed25519 signature against the published
  key. You use tools that you already trust. Then you trust only the
  key.

The math is the same as `receipt_preimage_v1` and `receipt_preimage_v2`
in `crates/emem-attest/src/lib.rs`. The signer and the server check use
that one code path.

The emem repository publishes a reference offline verifier at
https://github.com/Vortx-AI/emem/tree/main/tools/verifiers/emem-verify-receipt
for anyone who wants to read or run it themselves. This plugin ships no
code.

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
and save the result as `recall.json`.

Any saved emem response with a top-level `receipt` works. A bare
receipt also works.

## Step 2: get the published key

Get the published key of the responder:

```sh
curl -fsSL https://emem.dev/.well-known/emem.json
```

Read `responder.pubkey_b32` in the answer.

On 12 November 2026 it was
`777er3yihgifqmv5hmc2wwmyszgddzderzhsx6rex4yoakwomvka`, the same as
`responder_pubkey_b32` in the receipt.

This answer also comes from the responder. For a check that does not
depend on the responder at all, compare the key with a copy that you
saved earlier, or with a copy from a different source. A receipt can
verify under a key that you do not expect. Then it proves only that the
holder of that key signed it.

## Step 3: check the signature through emem

Call `emem_verify_receipt` with the whole `receipt` object. Do not
change it. Add the key from step 2 as `pubkey_b32`:

```json
{"receipt": {"primitive": "emem.recall", "request_id": "...", "served_at": "...", "...": "..."},
 "pubkey_b32": "777er3yihgifqmv5hmc2wwmyszgddzderzhsx6rex4yoakwomvka"}
```

With `pubkey_b32`, the server checks the signature against that key,
not against the key in the receipt. Without it, the server uses the key
in the receipt.

Without MCP, build the body from the saved response:

```sh
jq -c '{receipt: .receipt, pubkey_b32: "777er3yihgifqmv5hmc2wwmyszgddzderzhsx6rex4yoakwomvka"}' recall.json > verify_req.json
```

Send `verify_req.json` as the body of
`POST https://emem.dev/v1/verify_receipt`. Read these fields in the
answer:

| Field | Meaning |
|---|---|
| `valid` | `true` only when the signature verifies, the Merkle proof did not fail, and each fact that you sent in `facts` is in the receipt. |
| `reason` | `null` when valid. Otherwise `signature_invalid`, `receipt_reshaped_after_signing`, `merkle_proof_invalid` or `fact_mismatch`. |
| `failure_detail` | What the reason means and what to do. |
| `signature_valid` | The Ed25519 result alone. |
| `merkle_proof_valid` | `true` or `false` when the responder walked the proof. `null` when the receipt has no proof, or when this responder does not hold the first fact. |
| `preimage_blake3_hex` | The 32-byte digest that the signature covers, in hex. |
| `signer_pubkey_b32` | The key that the check used. |
| `preimage_version`, `primitive`, `fact_cids_count`, `manifest_bound`, `field_bound` | What the receipt carries. |

To check that a fact value you rely on is one that the receipt signs,
also send that fact in `facts`, as `emem_recall` returned it. A fact
that is not in the receipt makes `valid` false with
`reason: fact_mismatch`.

On 12 November 2026 at 19:45 UTC, the recall in step 1 returned a
receipt with 16 fact cids. The server check returned `valid: true`,
`reason: null`, `signature_valid: true`, `merkle_proof_valid: true`,
`preimage_version: 2` and `preimage_blake3_hex`
`b768c835bae1607e427c972d0fadc330cdbcd791e2651759d15fb732a0133a4f`.
Then each change below went to the same check:

| Change | `valid` | `reason` |
|---|---|---|
| `served_at` one second later | `false` | `signature_invalid` |
| `merkle_proof` deleted | `false` | `receipt_reshaped_after_signing` |
| `pubkey_b32` set to a different key | `false` | `signature_invalid` |

Your digest and counts will be different, because each call has a new
`request_id` and `served_at`.

**What this check proves.** The responder rebuilt the preimage and
found that the signature matches it under the key that you named. It
does not prove this to someone who does not trust the responder. A
responder that lies can report `valid: true` for anything. Use step 4
when the answer must not depend on emem.

A person can also paste a receipt into the page
`https://emem.dev/verify`. The page checks it in the browser. Its code
also comes from the responder.

## Step 4: check it independently

Use this step when the result must not depend on emem. Use tools that
you already trust for BLAKE3-256 and Ed25519. If you have none, report
the result of step 3 and say that the responder checked its own
signature.

Section 7.1 of https://github.com/Vortx-AI/emem/blob/main/docs/protocol.md
gives the preimage. Section 7.2 gives a worked example byte for byte.
`GET /v1/verifier_spec` serves the same segment table from the compiled
constants, with a golden vector for the manifest digest.

```
stream = "emem.preimage.v1\0" || u32le(7) || "receipt" || segment*
digest = blake3(stream)                              32 bytes
valid  = ed25519_verify(responder_pubkey, signature, digest)
```

The encodings:

- `u32le(n)` is `n` as 4 bytes, little-endian. A tag is 1 byte.
- A scalar segment is `tag || u32le(len) || bytes`. A list segment is
  `tag || u32le(count) || (u32le(len) || bytes)*`.
- A text field enters as its UTF-8 bytes.
- A digest segment (manifest, field, merkle) enters as its 64 lowercase
  hex characters, as ASCII text. It does not enter as 32 raw bytes.
- `signature` is 64 bytes and `responder` is 32 bytes. The JSON gives
  them as byte arrays. It gives them again as `signature_b32` and
  `responder_pubkey_b32`: base32 with the RFC 4648 alphabet, lowercase,
  no padding.
- Ed25519 signs the 32-byte digest, not the stream.

The segments come in this order. An optional segment is present only
when the "When" column applies.

| Tag | Segment | When |
|----|----------|------|
| 1 | request_id | Always |
| 2 | served_at | Always |
| 3 | scope: hex of the BLAKE3 of the canonical CBOR of `scope` | Scoped reads |
| 4 | as_of: the same rule over `as_of` | Bi-temporal reads |
| 5 | edges: the same rule over the sorted `edge_cids` | `include:["edges"]` |
| 6 | manifest: hex of `blake3(cbor(source_versions))` | When not empty |
| 7 | primitive | Always |
| 8 | cells | Always (list) |
| 9 | fact_cids | Always (list) |
| 10 | field: hex of the field binding | Field tokens |
| 11 | merkle: hex of the Merkle binding | v2 only, always |

The three sub-digests:

- **manifest.** A CBOR map of text keys to text values. The keys are in
  plain string order: `bands_cid`, `registry_cid`, `schema_cid`,
  `sources_cid`. This is not the RFC 8949 order, which puts shorter
  keys first. Test your encoder against the golden vector in
  `/v1/verifier_spec`.
- **field.** `blake3("emem.preimage.v1\0" || u32le(5) || "field" ||
  seg(1, aoi_cid) || seg(2, derivation_cid))`, with the two cids as
  text.
- **merkle.** With a proof: `blake3("emem.preimage.v1\0" || u32le(6) ||
  "merkle" || seg(1, root) || seg(2, u32le(leaf_index)) || seg(3, path)
  || seg(4, version))`. `root` and each path sibling are 32 raw bytes,
  and `path` joins the siblings into one segment. `version` is one
  byte, 0 when the JSON omits it. Without a proof, the only segment is
  `seg(5, "")`.

Read the version from the receipt. Receipts that emem serves today have
`preimage_version: 2`. A v1 receipt has no tag 11. A receipt with no
`preimage_version` uses the legacy rule in section 7.1. Use step 3 for
it. Tag 11 makes a deleted `merkle_proof` visible: the digest changes
and the signature fails. A change of `preimage_version` to 1 also
fails, because the v1 digest is different from the signed v2 stream.

Compare your digest with `preimage_blake3_hex` from step 3. Equal
digests show that your rebuild is correct. Then check the signature
over your digest with your own Ed25519 tool and the key from step 2.
OpenSSL 3 checks Ed25519 but has no BLAKE3. It reads a raw key as DER:
the 12 bytes `302a300506032b6570032100`, then the 32 key bytes.

On 12 November 2026 an independent rebuild of the receipt in step 3
gave the same digest as `preimage_blake3_hex`. OpenSSL 3.0 verified the
signature over that digest. With one bit of the digest changed, it
reported a failure.

To walk the Merkle proof yourself, see section 8 of the same document.

## If the check fails

| Result | What to do |
|---|---|
| `reason: signature_invalid`, or your own Ed25519 check fails | Someone changed the receipt, or a different key signed it. Show both keys to the user and let the user decide. |
| `reason: receipt_reshaped_after_signing` | The signed body is intact, but `merkle_proof` or `preimage_version` changed after signing. `failure_detail` names the field. Get the original bytes of the response again. |
| `reason: merkle_proof_invalid` | The signature is valid, but the inclusion proof does not walk to its root. Do not rely on the receipt. |
| The signer key is not the key in `/.well-known/emem.json` | The responder can rotate keys. `responder_key_epoch` on the receipt gives the epoch that signed it. A receipt from an older epoch verifies against its historical key. |
| Your digest is not `preimage_blake3_hex` | Your rebuild is wrong, not the receipt. Look for manifest keys in RFC 8949 order, a digest entered as raw bytes and not as hex text, or a missing tag 11. |
| The hash does not match after you serialised the receipt again | `signature` and `responder` are byte arrays. Keep them as arrays, or use `signature_b32`. Do not change the order of `cells` or `fact_cids`. |
