# emem-attest

`emem-attest` holds the hash rules behind every emem signature. It makes
the 32-byte digests that a key signs, and it builds the two Merkle trees
that emem publishes. The crate does not sign and does not store. Other
crates do that with the digests it returns.

You need this crate when you write a verifier in Rust, or when you change
what a receipt, an attestation or a device record binds. To check a receipt
from another language, you do not need it: use `POST /v1/verify_receipt`,
the `/verify` page, or the rules in [docs/protocol.md](../../docs/protocol.md).

| | |
|---|---|
| Version | 2.4.4 (workspace version in the root `Cargo.toml`) |
| Size | `src/lib.rs` (1,378 lines) and `src/translog.rs` (976 lines) |
| Used in code by | `emem-storage`, `emem-api-rest`, `emem-fact`, `emem-trace`, `emem-airgap`, `emem-cli` (`emem-demo-device`) |
| Used in tests only by | `emem-primitives` |
| Tests | 36 unit tests: 18 in `lib.rs`, 18 in `translog.rs` |
| I/O | None. The only hash is BLAKE3-256. |

## Try it

The live responder publishes known answers for both trees. A Rust or
JavaScript verifier must give the same roots:

```sh
curl -s https://emem.dev/v1/log/test_vectors | jq -c 'keys'
# ["_means","batch","log","schema"]
```

`batch` holds roots and paths for the batch tree. `log` holds them for the
log tree.

It needs `curl` and `jq`, and no key. The command ran on 7 November 2026.

Then read the signed head of the append-only log. Its tree rule comes from
`src/translog.rs`:

```sh
curl -s https://emem.dev/v1/log/sth | jq -c '.sth | {tree_size, root_b32, signed_at}'
# {"tree_size":3247646,"root_b32":"vkdu74qaff34hdevheev7bx4nro6jsfblra7ab246cw3ivy3acja","signed_at":"2026-11-07T07:18:54Z"}
```

The size and the root grow with the log. The values above are from
7 November 2026. To prove that the log only grew, ask for a consistency
proof between two sizes:

```sh
curl -s 'https://emem.dev/v1/log/consistency?first=3247000&second=3247646' \
  | jq -c '{first_size, second_size, steps: (.consistency_proof_b32 | length)}'
# {"first_size":3247000,"second_size":3247646,"steps":14}
```

The [emem-transparency-log](../../plugins/emem/skills/emem-transparency-log/)
skill ships a Python verifier for these proofs.

## The two Merkle trees

emem signs two kinds of tree. They use different rules, so a proof for one
does not verify under the other.

| | Batch tree (`lib.rs`) | Log tree (`translog.rs`) |
|---|---|---|
| Covers | The fact ids in one attestation batch | Every entry of the append-only attestation log |
| Leaf order | Sorted bytewise, no duplicates | Append order. Two equal entries are two leaves. |
| Leaf hash (v1) | `blake3(0x00 \|\| leaf)` | `blake3(0x00 \|\| entry)` |
| Node hash (v1) | `blake3(0x01 \|\| left \|\| right)` | `blake3(0x01 \|\| left \|\| right)` |
| Odd layer | The last node pairs with itself | The last node moves up unchanged (RFC 6962) |
| Proofs | Inclusion of one fact in its batch | Inclusion, and consistency between two sizes |
| Functions | `merkle_root_v1`, `merkle_root_and_paths_v1`, `verify_merkle_path_v1`, `has_adjacent_duplicate` | `merkle_tree_hash`, `inclusion_path`, `verify_inclusion`, `consistency_proof`, `verify_consistency`, `IncrementalTree` |

Because the batch tree pairs the last node with itself,
`root([A,B,C])` is equal to `root([A,B,C,C])`. A v1 verifier must refuse a
batch with a duplicate leaf. `has_adjacent_duplicate` does this check, and
`emem-storage` calls it before it accepts an attestation.

The functions without a version suffix (`merkle_root`,
`merkle_root_and_paths`, `verify_merkle_path`) are the v0 batch rule. v0
hashes a leaf as `blake3(leaf || leaf)` and has no prefix bytes. It stays so
that attestations signed before v1 still verify.

## Preimage rules

`PreimageV1` builds every signed digest. The stream starts with
`emem.preimage.v1\0` and a length-prefixed domain name. Each field is then
one segment: a tag byte, a `u32` little-endian length, and the bytes. A
list segment writes a count, then each item with its length. The digest is
the BLAKE3 of the stream.

The crate uses 9 domains:

| Domain | Function | Who signs the digest |
|---|---|---|
| `receipt` | `receipt_preimage_v1`, `receipt_preimage_v2` | The responder |
| `merkle` | `merkle_binding_v2` (one segment of a v2 receipt) | Part of the receipt |
| `field` | `field_binding_v1` (one segment of a field receipt) | Part of the receipt |
| `attestation` | `attestation_preimage_v1` | The responder |
| `os_trace` | `os_trace_preimage_v1` | A device key ([emem-trace](../emem-trace/README.md)) |
| `custody` | `custody_preimage_v1` | A node key ([emem-airgap](../emem-airgap/README.md)) |
| `platform_attestation` | `platform_attestation_preimage_v1` | An endorser key |
| `join_request` | `join_request_preimage_v1` | A node key that asks to enrol |
| `publish_decision` | `publish_decision_preimage_v1` | A device key that consents to a public list |

The tag constants live in one module per domain, for example
`receipt_tag` and `os_trace_tag`. The responder builds
`GET /v1/verifier_spec` from these constants, so the published spec and
the signer use the same numbers.

### Receipt v1 and v2

A v2 receipt adds one segment, `MERKLE` (tag `0x0b`), after the v1
segments. It holds the digest of the receipt's inclusion proof. When the
receipt has no proof, the segment holds a signed "absent" marker. Thus a
person in the middle cannot remove the proof and keep a valid signature.
A verifier must pick the rule from the receipt's own `preimage_version`. It
must not check a v2 receipt under the v1 rule.

On 7 November 2026, a recall at Mount Fuji returned a receipt with
`preimage_version` 2:

```sh
curl -s https://emem.dev/v1/recall -H 'Content-Type: application/json' \
  -d '{"cell":"Mount Fuji","bands":["copdem30m.elevation_mean"]}' \
  | jq -c '.receipt | {preimage_version, proof_version: .merkle_proof.version}'
# {"preimage_version":2,"proof_version":1}
```

`proof_version` 1 means that the batch tree used the v1 hash rule.

## Rules for changing this crate

- Tags are wire constants. Do not renumber a tag. Add a new tag at the
  end of its module.
- Add an optional segment only after the existing segments, and write it
  only when it has a value. Then every digest signed before the change
  stays the same. `FIELD`, `PREV_TRACE`, `STAGE` and `TRACE` follow this
  rule.
- Do not change a v0 or v1 function. Add a new version next to it, as
  `receipt_preimage_v2` does.
- A change to the receipt preimage also needs a change to
  [docs/protocol.md](../../docs/protocol.md) section 7 and to the
  in-browser verifier. The root `AGENTS.md` names the three places that
  must agree.

## Where the code and the description differ

We checked these points in the code on 7 November 2026:

| The text says | The code does |
|---|---|
| `Cargo.toml` description: "stake state (stubbed v2.0)" | The crate has no stake type or stake function. |
| `lib.rs` header: "signing" | The crate signs nothing. It returns digests, and the caller signs. |
| `Cargo.toml` lists `emem-core`, `ed25519-dalek`, `serde`, `ciborium` and `thiserror` | `src/` uses only `blake3`. |

## What ships and what does not

| Ships | Does not ship |
|---|---|
| The v0 and v1 batch trees, and the RFC 6962 log tree | Stake or slashing state |
| Digest rules for 9 domains, with v1 and v2 receipts | Signing, key storage or log storage. `emem-storage` does those. |
| Inclusion and consistency proofs for the log | The signed tree head. `emem-api-rest` signs it under the domain `emem.translog.sth.v1`. |

## Read more

- [docs/protocol.md](../../docs/protocol.md) sections 6 to 9: the
  attestation, the receipt, the inclusion proof and the append-only log.
- [emem-storage](../emem-storage/README.md): the crate that signs the
  digests and keeps the log.
- [emem-transparency-log skill](../../plugins/emem/skills/emem-transparency-log/):
  an offline verifier for the log.
- [docs/glossary.md](../../docs/glossary.md): receipt, attestation and
  fact id.
