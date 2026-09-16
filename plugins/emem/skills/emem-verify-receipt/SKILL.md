---
name: emem-verify-receipt
description: Verify an emem receipt's Ed25519 signature offline by rebuilding the canonical BLAKE3 preimage and checking against the responder's published pubkey. Use when the user pastes a receipt JSON and asks whether it's authentic, when an LLM needs to prove a fact wasn't fabricated, or when caching emem facts and wanting to confirm origin later. Runs without re-contacting the responder.
allowed-tools: Bash(curl:*) Bash(python3:*) Bash(pip:*) Read Write
---

# emem-verify-receipt

This skill rebuilds the canonical preimage of an emem receipt
byte-for-byte, BLAKE3s it, and runs Ed25519 verification — all
locally. The math matches `emem-attest::receipt_preimage_v1` (crates/emem-attest/src/lib.rs)
in the emem source; if your verification passes, the receipt was
signed by the responder pubkey and has not been tampered with.

## When to invoke

- "Is this emem receipt authentic?"
- "Verify this fact_cid offline."
- "I cached an emem response from last year — can I prove it's real
  without calling the server?"
- The user pastes a JSON blob with `request_id`, `served_at`,
  `primitive`, `cells`, `fact_cids`, `signature`, `responder` (or
  `responder_pubkey_b32`) fields.

## How to invoke

### Quick one-shot via the bundled Python script

```sh
python3 .claude/skills/emem-verify-receipt/verify.py path/to/receipt.json
```

Or pipe a receipt directly:

```sh
curl -sf -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb493.xoso.zcb6a","bands":["weather.temperature_2m"]}' \
  | jq '.receipt' \
  | python3 .claude/skills/emem-verify-receipt/verify.py -
```

The script prints `VALID` and the BLAKE3 digest hex if the signature
checks out, or `INVALID` with the reason if not.

### What the math does

Receipts carry `preimage_version: 1`. The preimage is **domain-separated
and length-prefixed** so no two different receipts can collide without a
BLAKE3 collision:

```
blake3( "emem.preimage.v1\0"
        || u32le(len("receipt")) || "receipt"
        || segment*  )
```

Each scalar segment is `tag || u32le(len) || bytes`; a list segment is
`tag || u32le(count) || (u32le(len) || bytes)*`. Segments appear in this
fixed order, and optional ones are emitted only when present:

| tag | segment | when |
|----|----------|------|
| 1 | request_id | always |
| 2 | served_at | always |
| 3 | scope | scoped reads only |
| 4 | as_of | bi-temporal reads only |
| 5 | edges | `include:["edges"]` only |
| 6 | manifest | `blake3(cbor(source_versions))`, when non-empty |
| 7 | primitive | always |
| 8 | cells | always (list) |
| 9 | fact_cids | always (list) |
| 10 | field | field tokens (raster/cube): `blake3` of the `(aoi_cid, derivation_cid)` binding |

BLAKE3 over that stream produces the 32-byte digest; the responder's
ed25519 key signs the digest, and
`ed25519.verify(signature, digest, responder_pubkey)` checks it. The
bundled `verify.py` rebuilds this exactly for the common cases (recall,
field tokens); a receipt carrying a scope/as_of/edges segment is sent to
the server verifier instead of guessed at.

The pubkey decodes from `responder_pubkey_b32` via base32-nopad-lowercase.

## Dependencies

The script needs `blake3` and either `cryptography` or `nacl` for
Ed25519. If they're missing:

```sh
pip install --user blake3 cryptography
```

## Server fallback

If the user can't run Python locally, point them at:

```sh
curl -sf -X POST https://emem.dev/v1/verify_receipt \
  -H 'content-type: application/json' \
  -d '{"receipt": <receipt_json>}'
```

This re-runs the same math server-side and returns
`{valid: bool, preimage_blake3_hex, signer_pubkey_b32, ...}`.
Fundamentally less trustworthy than the offline path (you're trusting
the responder to be honest about the verification), but useful as a
sanity check.

## Worked example

```
USER: Here's an emem receipt I got last week — is it real?
      {"request_id":"01JBQ...","served_at":"2026-05-01T10:00:00Z",
       "primitive":"emem.recall","cells":["defi.zb493.xoso.zcb6a"],
       "fact_cids":["qi3jo4...l2hgjtwm"],
       "signature":[<64 bytes>],"responder":[<32 bytes>],
       "responder_pubkey_b32":"777er3yihgifqmv5..."}

CLAUDE invokes this skill:
  python3 verify.py /tmp/receipt.json
    → VALID
    → preimage_v1: 260 bytes  (domain-separated, length-prefixed)
    → digest:      c88485ab2a09...
    → signer:      777er3yihgifqmv5... (matches /.well-known/emem.json)

CLAUDE replies: "Yes — the signature verifies against the responder
pubkey at /.well-known/emem.json. The receipt is authentic. The
fact 'temperature_2m at Bengaluru' was signed by emem.dev at
2026-05-01T10:00:00Z."
```

## What to do on a mismatch

- **Signature does not verify** — receipt was tampered with, or it
  was signed by a different responder than the one at the given
  pubkey. Show the user both the expected pubkey and the one
  embedded in the receipt; let them decide.
- **Pubkey doesn't match `/.well-known/emem.json`** — the responder
  rotated keys. Each receipt carries `responder_key_epoch`; an
  out-of-date receipt was signed by an older epoch and can still be
  verified against the historical pubkey if you have it.
- **Preimage hash mismatch** — almost always a serialisation issue
  (whitespace in the JSON, wrong byte order on `signature` /
  `responder` arrays). Re-fetch the receipt with `jq -c` to
  guarantee canonical JSON.
