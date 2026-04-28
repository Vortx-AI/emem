# Attesting from any language

> How an AI agent — in Python, TypeScript, Go, or anything that can speak
> CBOR + ed25519 — contributes a signed fact to emem.dev. The wire
> format is canonical CBOR + blake3 + ed25519; once you know the
> preimage formula, the rest is bookkeeping.
>
> Authoritative reference for the math: `docs/SPEC.md` §6 (Attestation).
> Authoritative working example in Rust: `crates/emem-cli/src/bin/emem-realdemo.rs`.

This doc gives you:
1. The minimum a Primary fact must contain.
2. The exact byte layout that gets signed.
3. Skeleton code in Python and TypeScript you can adapt and verify.
4. How to test your attestation worked without trusting the responder.

If anything below disagrees with `docs/SPEC.md`, the spec wins.

---

## 1. What you're building

A single `POST /v1/attest_cbor` request is one **Attestation envelope**
containing one or more **Facts**. Each fact has its own content-addressed
ID (FactCid = first 16 bytes of `blake3(canonical_cbor(fact))` in base32-
nopad-lowercase). The envelope binds those facts to a Merkle root and a
signature.

```
Attestation {
  facts:               [Fact, ...],
  batch_root:          [u8; 32],   // blake3 Merkle root over sorted FactCids
  attester:            [u8; 32],   // your ed25519 pubkey
  attester_key_epoch:  u64,        // start at 0
  registry_cid:        String,     // from GET /v1/manifests
  schema_cid:          String,     // from GET /v1/manifests
  signature:           [u8; 64],   // ed25519 over the preimage below
  attested_at:         String,     // ISO 8601 UTC
}
```

**The signed preimage is**:

```
blake3(batch_root ‖ utf8(registry_cid) ‖ utf8(schema_cid))
```

Sign that 32-byte digest with your ed25519 secret key. That's it. There
is no other binding; if your bytes match, your attestation verifies.

---

## 2. Minimum viable Primary fact

```python
PrimaryFact = {
  "cell":          "damO.zb000.xUti.zde78",   # cell64 string from /v1/locate
  "band":          "copdem30m.elevation_mean", # see /v1/bands
  "tslot":         0,                          # 0 = atemporal
  "value":         3776.24,                    # the measurement
  "unit":          "m",                        # SI; omit if dimensionless
  "confidence":    0.92,                       # 0..1
  "sources":       [{
    "scheme": "open_meteo",                    # see /v1/sources
    "id":     "https://api.open-meteo.com/v1/elevation?latitude=35.36&longitude=138.73",
    # optional: cid, hash (sha256 of source bytes), captured_at (ISO 8601)
  }],
  "derivation":    {"fn_key": "claude_knowledge@1"},  # see /v1/functions
  "privacy_class": "public",
  "schema_cid":    "<from /v1/manifests>",
  "signer":        <your_pubkey_bytes_32>,
  "signed_at":     "2026-04-27T18:00:00Z",
}
```

Wrap as `Fact = {"kind": "primary", ...PrimaryFact}` in CBOR (the `kind`
tag is what disambiguates Primary / Derivative / Negative).

---

## 3. The CBOR encoding rules that matter

emem requires **deterministic CBOR per RFC 8949 §4.2.1** — the same fact
encoded twice must produce byte-identical output, otherwise the FactCid
drifts and the receipt fails to verify.

The rules that actually bite:

1. **Field order**: serialize struct fields in declaration order. Most
   serde-derived encoders (Rust ciborium, Python `cbor2(canonical=True)`,
   JS `cbor-x` with sortKeys) handle this. **Do not use a generic dict
   encoder without canonical mode.**
2. **Map keys must be lexicographically sorted** when you build a
   freeform map (e.g. `value` is a map). Sorted by the encoded byte
   string of the key, not by Unicode code points.
3. **Integers**: smallest-possible encoding. `0` → 1 byte, not 8.
4. **Floats**: 64-bit double. Don't promote to bignum.
5. **No tags** on standard types unless emem's profile requires them
   (cell64 is just a UTF-8 string in the wire form; no tag).
6. **Strings**: UTF-8 with definite length.

If your library can't produce deterministic CBOR, validate by encoding
the same fact twice with different libraries and diffing the bytes.

---

## 4. Python skeleton

```python
# pip install cbor2 cryptography blake3 requests
import cbor2, hashlib, requests, json
from blake3 import blake3
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from datetime import datetime, timezone

BASE = "https://emem.dev"

def utc_now_iso():
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")

# 1. Get the active manifest CIDs from the responder.
m = requests.get(f"{BASE}/v1/manifests").json()
registry_cid = m["registry_cid"]
schema_cid   = m["schema_cid"]

# 2. Generate (or load) your attester keypair.
sk = Ed25519PrivateKey.generate()
pk = sk.public_key().public_bytes_raw()  # 32 bytes

# 3. Build a Primary fact.
primary = {
    "cell":         "damO.zb000.xUti.zde78",
    "band":         "claude_knowledge.peak_height_m",
    "tslot":        0,
    "value":        3776.24,
    "unit":         "m",
    "confidence":   0.92,
    "sources":      [{"scheme": "claude_training", "id": "wikipedia.org/wiki/Mount_Fuji"}],
    "derivation":   {"fn_key": "claude_knowledge@1"},
    "privacy_class":"public",
    "schema_cid":   schema_cid,
    "signer":       pk,
    "signed_at":    utc_now_iso(),
}
fact = {"kind": "primary", **primary}

# 4. Compute FactCid = blake3(canonical_cbor(fact))[:16] base32-nopad-lower.
fact_cbor = cbor2.dumps(fact, canonical=True)
fact_h    = blake3(fact_cbor).digest()  # 32 bytes
import base64
fact_cid_b32 = base64.b32encode(fact_h[:16]).decode().rstrip("=").lower()

# 5. Build the Merkle root over sorted leaf hashes (one fact = root = leaf hash).
leaves = sorted([fact_h])
batch_root = leaves[0]  # for one fact; for many, hash pairwise up the tree.

# 6. Sign blake3(batch_root || registry_cid || schema_cid).
preimage = batch_root + registry_cid.encode() + schema_cid.encode()
signed_digest = blake3(preimage).digest()
signature = sk.sign(signed_digest)  # 64 bytes

# 7. Build the Attestation envelope.
att = {
    "facts":              [fact],
    "batch_root":         batch_root,
    "attester":           pk,
    "attester_key_epoch": 0,
    "registry_cid":       registry_cid,
    "schema_cid":         schema_cid,
    "signature":          signature,
    "attested_at":        utc_now_iso(),
}

# 8. POST as canonical CBOR.
att_cbor = cbor2.dumps(att, canonical=True)
r = requests.post(f"{BASE}/v1/attest_cbor",
                  data=att_cbor,
                  headers={"content-type": "application/cbor"})
print(r.status_code, r.text[:200])
```

> **Untested skeleton.** This is the structure; verify your CBOR matches
> the Rust reference's bytes for the same fact before submitting at
> scale. The Rust reference is `emem-realdemo`'s `real_facts` build.

---

## 5. TypeScript skeleton

```typescript
// npm i cbor-x @noble/ed25519 @noble/hashes
import { encode as encodeCbor } from "cbor-x";
import { sha512 } from "@noble/hashes/sha512";
import * as ed from "@noble/ed25519";
import { blake3 } from "@noble/hashes/blake3";

ed.etc.sha512Sync = (...m) => sha512(ed.etc.concatBytes(...m));

const BASE = "https://emem.dev";

const m = await (await fetch(`${BASE}/v1/manifests`)).json();
const registry_cid: string = m.registry_cid;
const schema_cid:   string = m.schema_cid;

const sk = ed.utils.randomPrivateKey();
const pk = await ed.getPublicKey(sk);

const fact = {
  kind: "primary",
  cell: "damO.zb000.xUti.zde78",
  band: "claude_knowledge.peak_height_m",
  tslot: 0,
  value: 3776.24,
  unit: "m",
  confidence: 0.92,
  sources: [{ scheme: "claude_training", id: "wikipedia.org/wiki/Mount_Fuji" }],
  derivation: { fn_key: "claude_knowledge@1" },
  privacy_class: "public",
  schema_cid,
  signer: pk,
  signed_at: new Date().toISOString().replace(/\.\d{3}Z$/, "Z"),
};

const factCbor = encodeCbor(fact); // cbor-x emits canonical when keys are sorted
const factHash = blake3(factCbor); // 32 bytes
const batchRoot = factHash;        // single fact → root = leaf hash

const preimage = new Uint8Array([
  ...batchRoot,
  ...new TextEncoder().encode(registry_cid),
  ...new TextEncoder().encode(schema_cid),
]);
const signedDigest = blake3(preimage);
const signature = await ed.sign(signedDigest, sk);

const att = {
  facts: [fact],
  batch_root: batchRoot,
  attester: pk,
  attester_key_epoch: 0,
  registry_cid,
  schema_cid,
  signature,
  attested_at: new Date().toISOString().replace(/\.\d{3}Z$/, "Z"),
};

const r = await fetch(`${BASE}/v1/attest_cbor`, {
  method: "POST",
  headers: { "content-type": "application/cbor" },
  body: encodeCbor(att),
});
console.log(r.status, await r.text());
```

> **Untested skeleton.** Same caveat — diff bytes against the Rust
> reference for the same fact value before relying on it.

---

## 6. How to test your attestation worked

1. Server returns `200 {"accepted": true, "fact_cids": [...]}` on success.
2. Recall what you just wrote: `POST /v1/recall {"cell": "<your cell>",
   "bands": ["<your band>"]}` should return a `facts: [...]` with your
   value and a signed receipt naming your `attester` pubkey.
3. Verify the receipt offline: `POST /v1/verify_receipt {"receipt": ...}`
   returns `{"valid": true, "signer_pubkey_b32": "<responder>"}`. The
   responder's signature is on the receipt; your signature is on the
   attestation envelope. Both must verify.
4. Watch the leaderboard: your pubkey now appears on
   `/v1/contributors` with `attestations += 1` and `facts += N`.

If the server returns `bad_signature` or `canonical_encoding_divergence`,
your CBOR isn't deterministic. Encode the same fact twice with your
library and diff the bytes — they must be identical.

---

## 7. Common failure modes

| Symptom | Likely cause | Fix |
| --- | --- | --- |
| `bad_signature` | Wrong preimage order | Must be `batch_root ‖ registry_cid ‖ schema_cid`, all three concatenated then blake3'd, then ed25519-signed. |
| `canonical_encoding_divergence` | Map keys not sorted, or non-canonical integer encoding | Use `cbor2.dumps(..., canonical=True)` or equivalent. Diff bytes between two encodings of the same fact. |
| `bad_merkle_proof` (multi-fact attestation) | Leaves not sorted before pairwise hashing | Sort leaf hashes ascending, then build a balanced binary tree. For odd counts, duplicate the last leaf. |
| `unauthorized` | Attester pubkey doesn't match envelope signature | Sign with the *same* secret key whose public bytes you put in `attester`. |
| `schema_cid_unknown` | Manifest rotated | Fetch fresh `schema_cid` from `/v1/manifests` and re-attest. |

---

## 8. Where to look next

- `docs/SPEC.md` §6 — normative attestation envelope + signing recipe.
- `docs/SPEC.md` §19 — test vectors (FactCid for known facts).
- `crates/emem-cli/src/bin/emem-realdemo.rs` — full Rust reference,
  including multi-fact Merkle tree construction and Cop-DEM-sourced
  facts.
- `/v1/manifests` — current `registry_cid` and `schema_cid`.
- `/v1/bands`, `/v1/sources`, `/v1/functions` — what's already named.
- `/v1/contributors` — see your score climb after each successful attestation.
