---
name: emem-verify-receipt
description: Verifies an emem receipt's Ed25519 signature offline by rebuilding its canonical BLAKE3 preimage (v1 or v2) and checking it against the responder's published key. Use when the user pastes a receipt or an emem response and asks whether it is authentic, when an agent must prove a fact was not fabricated, or when checking a cached answer later without trusting or re-contacting the responder. Ships a small Python verifier; a server-side check is the fallback.
---

# emem-verify-receipt

> **Network use.** The commands in this skill send and receive JSON (and, where a step says so, an image or a raster file) to `https://emem.dev` only, the service the plugin's MCP server connects to. Nothing they download is executed. The files under `scripts/` read local files and make no network calls.

Every emem response that serves facts carries a `receipt`. This skill
rebuilds the receipt's preimage byte for byte, hashes it with BLAKE3,
and checks the Ed25519 signature locally. The math matches
`receipt_preimage_v1` and `receipt_preimage_v2` in
`crates/emem-attest/src/lib.rs`, the one implementation the signer and
every emem verifier share.

## When to invoke

- "Is this emem receipt authentic?"
- "I cached an emem answer last month. Can I prove it is real without
  calling the server?"
- Any JSON with `request_id`, `served_at`, `primitive`, `cells`,
  `fact_cids`, `signature` and `responder_pubkey_b32`.

## Run the bundled verifier

Save the response to a file, then run the shipped script on the file:

```sh
curl -sf -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb493.zezo.zcb35","bands":["weather.temperature_2m"]}' \
  -o recall.json
python3 "${CLAUDE_SKILL_DIR}/scripts/verify.py" recall.json
```

It accepts a bare receipt or a whole response with a top-level
`receipt`. `${CLAUDE_SKILL_DIR}` is this skill's directory, filled in
by Claude Code; `scripts/verify.py` ships with this skill and imports BLAKE3
and Ed25519 from `scripts/emem_crypto.py`, shipped with this skill, plain Python with no
third-party packages. Nothing is installed or downloaded to run it.
`python3 "${CLAUDE_SKILL_DIR}/scripts/verify.py" --self-test` checks that module
against the official BLAKE3 and RFC 8032 test vectors first.

Output on success (recorded 2026-09-28 on a Bengaluru recall):

```
VALID
preimage_v2:  385 bytes
digest:       27d147e5b452b4292e2cc97f405e38fa0a45efecfbeefe00902c89dab5f31340
signer:       777er3yihgifqmv5hmc2wwmyszgddzderzhsx6rex4yoakwomvka
primitive:    emem.recall
cells:        1
fact_cids:    2
```

The same file with `served_at` edited, or with `merkle_proof` deleted,
printed `INVALID` and exited 1.

Exit codes: `0` VALID, `1` INVALID, `2` bad input (not JSON, or an
`emem.error.v1` body), `3` a segment the script does not rebuild
(scope, as_of or edges): send those to the server check below.

Compare `signer` with the key at `https://emem.dev/.well-known/emem.json`
(`responder.pubkey_b32`). A receipt that verifies under a key you did
not expect proves only that the holder of that key signed it.

## What the math does

```
digest = blake3( "emem.preimage.v1\0" || u32le(len("receipt")) || "receipt" || segment* )
valid  = ed25519_verify(responder_pubkey, signature, digest)
```

A scalar segment is `tag || u32le(len) || bytes`; a list segment is
`tag || u32le(count) || (u32le(len) || bytes)*`. In order, optional ones
only when present:

| tag | segment | when |
|----|----------|------|
| 1 | request_id | always |
| 2 | served_at | always |
| 3 | scope | scoped reads |
| 4 | as_of | bi-temporal reads |
| 5 | edges | `include:["edges"]` |
| 6 | manifest: lowercase hex of `blake3(cbor(source_versions))`, map keys in plain string order | when non-empty |
| 7 | primitive | always |
| 8 | cells | always (list) |
| 9 | fact_cids | always (list) |
| 10 | field: hex of the `(aoi_cid, derivation_cid)` binding | field tokens |
| 11 | merkle: hex of the inclusion-proof binding, or of an explicit absence marker | v2 only, always |

Receipts served today carry `preimage_version: 2`. Tag 11 is what makes
a stripped `merkle_proof` detectable: delete the proof and the digest
changes, so the signature fails. Rewriting `preimage_version` to 1 does
not help an attacker either, because the v1 digest differs from the v2
stream that was signed. `GET /v1/verifier_spec` publishes these rules
from the compiled constants, including a golden vector for the manifest
encoding.

## Server fallback

```sh
jq '{receipt: .receipt}' response.json \
  | curl -sf -X POST https://emem.dev/v1/verify_receipt \
      -H 'content-type: application/json' -d @- \
  | jq '{valid, preimage_version, merkle_proof_valid, signer_pubkey_b32}'
```

`response.json` is any saved emem response. The server runs the same
math and reports `merkle_proof_valid` beside `valid`. It is weaker than
the offline path, because you are trusting the responder to report
honestly on its own signature, but it handles every segment and every
legacy version. The
browser page `https://emem.dev/verify` does the check locally in
JavaScript if you would rather paste than script.

## On a mismatch

- **INVALID**: the receipt was altered, or signed by a different key.
  Show the user both keys and let them decide.
- **Key differs from `/.well-known/emem.json`**: the responder may have
  rotated keys. `responder_key_epoch` on the receipt says which epoch
  signed it; an older epoch verifies against its historical key.
- **Hash mismatch on a receipt you re-serialised**: `signature` and
  `responder` are byte arrays; keep them as arrays (or use
  `signature_b32`), and do not reorder `cells` or `fact_cids`.

## This is one of three checks

A valid signature answers **did this responder sign these fact cids**.
It does not say you quoted the value correctly, or that the fact
supports your sentence. Those are `emem_echo_verify` and
`emem_guard_verdict`: see
[`emem-verify-before-publish`](../emem-verify-before-publish/SKILL.md).
For documents and the transparency log, which sign different objects,
see [`emem-document-evidence`](../emem-document-evidence/SKILL.md) and
[`emem-transparency-log`](../emem-transparency-log/SKILL.md).
