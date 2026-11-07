# emem-fact

`emem-fact` holds the data types that emem signs: the fact, the attestation
that carries facts in, and the receipt that goes out with every read. It
also defines canonical CBOR, the byte form that a fact id is the hash of.

You need this crate when you add a field to a fact or a receipt. You also
need it when you write a second encoder that must give the same bytes. To only read
and check facts, you do not need it: the responder serves the bytes, and
any BLAKE3 library checks them.

| | |
|---|---|
| Version | 2.4.4 (workspace version in the root `Cargo.toml`) |
| Used by | `emem-api-rest`, `emem-cache`, `emem-claim`, `emem-cli`, `emem-mcp`, `emem-primitives`, `emem-storage` |
| Tests | 33 unit tests inline in `src/`, 7 in `tests/round_trip.rs` |
| Specification | [docs/protocol.md](../../docs/protocol.md), sections 3 to 7 |

## Try it

A fact id is the BLAKE3 hash of the fact's canonical CBOR. Get the CBOR of
the Mount Fuji elevation fact, then hash it yourself:

```sh
curl -s -H 'Accept: application/cbor' -o fuji.cbor \
  https://emem.dev/v1/facts/qejpz7cgvk347ordd6vrdtmwzprqlzcnkmotsjton2x4y2jjbplq

python3 -c "import base64, blake3; b = open('fuji.cbor', 'rb').read(); \
print(base64.b32encode(blake3.blake3(b).digest()).decode().rstrip('=').lower())"
# qejpz7cgvk347ordd6vrdtmwzprqlzcnkmotsjton2x4y2jjbplq
```

The second command needs the `blake3` Python package. Both commands ran on
7 November 2026. The file was 743 bytes, and its hash was equal to the id in
the URL.

Without the `Accept` header, the same URL gives a JSON view of the fact. Do
not hash the JSON. JSON shows the value as `3539.0422`, but the CBOR holds
`3539.042236328125`. JSON also loses the width of `confidence`, which CBOR
writes as a 32-bit float. Only the CBOR bytes give the fact id.

To run the crate tests from a checkout (needs Rust 1.91 or later):

```sh
cargo test -p emem-fact
```

We did not run `cargo test` for this page, because the host had no Rust
toolchain. The test counts come from the `#[test]` lines in the source.

## What each module does

| Module | Holds |
|---|---|
| `fact` | `Fact`, with three kinds on the wire: `primary`, `derivative` and `absence` (the Rust type is `NegativeFact`). Also `Source`, `Derivation`, `Uncertainty` and `ServedVia`. |
| `cbor` | `to_canonical_cbor`, `blake3_32`, and `canonical_float`, which turns every NaN into one NaN and -0.0 into 0.0 |
| `cid` | String types for ids, so that a fact id and a registry id cannot change places by mistake: `FactCid`, `RegistryCid`, `SchemaCid`, `ReasonCid`, `BatchCid`, `CoverageCid`, `EdgeCid`, `StateCid` |
| `attest` | `Attestation`: facts in, with a Merkle root over their ids and the attester's signature |
| `receipt` | `Receipt`, `MerkleProof`, `Cost`, `AsOfReceipt` and `FieldBinding`: the signed proof that goes out with a read |
| `scope` | `Scope`: the optional `user_id`, `agent_id`, `run_id` and `org_id` of a read or a write |
| `edge` | `EdgeFact`: a signed relation between two facts, valid over a time interval |
| `state` | `StateRecord`: one step on the path to an answer, as an `emem:state:` address |

## How the crate makes the bytes

`to_canonical_cbor` calls `ciborium` and nothing more. Thus the field order
in a Rust `struct` is the key order in the bytes. For a primary fact, the
order is `kind`, `cell`, `band`, `tslot`, `value`, `unit`, `confidence`,
`uncertainty` (optional), `sources`, `derivation`, `privacy_class`,
`schema_cid`, `signer`, `signed_at`, `served_via` (optional). The encoder
leaves out an empty optional field. It does not write null for it.

A fact id is the full 32-byte hash, written as 52 lower-case base32
characters. The function that makes it is `fact_cid_of` in
`crates/emem-cache/src/sled_hot.rs`, not this crate. Edges and states make
their ids in this crate, with the same rule.

The fact includes `signer` and `signed_at`. Thus, if emem signs the same
value again at a later time, that fact gets a new id.

## Rules for changes

- Keep three places in step. A change to canonical CBOR here needs the same
  change in [docs/protocol.md](../../docs/protocol.md) and in the browser
  verifier in `web/humans.html`. The root [AGENTS.md](../../AGENTS.md) gives
  this rule.
- Do not move, rename or remove a field of a signed type. The field order is
  the byte order, so each such change gives every old fact a new id. Old
  signatures then fail.
- Add a new field as optional, with `skip_serializing_if`. Then old records
  keep the same bytes. `scope`, `edges` and `as_of` came in this way.
- Call `canonicalize_floats` on facts that the responder makes, before it
  signs them. Do not call it on an attestation that someone else signed,
  because their signature would then fail.
- A freeform `ciborium::Value::Map` keeps the key order you give it. Sort
  its keys before you encode it. The encoder does not sort them for you.

## What does not ship

- The encoding is deterministic, but it is not the RFC 8949 core
  deterministic encoding. That standard sorts map keys. This crate keeps
  the field order of the Rust type, as shown above.
- `cbor.rs` defines four CBOR tags: 65000 (cell), 65001 (time slot), 65002
  (vector id) and 42 (IPLD CID). No code in the workspace uses them. A fact
  writes its `cell64` as a text string and its time slot as a plain integer.
- This crate does not sign. The responder signs receipts in
  `crates/emem-storage/src/server.rs` (`sign_receipt`). The preimage rules
  are in `emem-attest`.

## Read more

- [docs/protocol.md](../../docs/protocol.md), sections 3 and 4: fact ids and
  canonical CBOR.
- [docs/protocol.md](../../docs/protocol.md), sections 5 to 7: the three
  fact kinds, the attestation and the receipt, with a worked preimage.
- [docs/glossary.md](../../docs/glossary.md): the terms fact, `fact_cid`,
  canonical CBOR, absence and receipt.
- [crates/emem-codec/README.md](../emem-codec/README.md): the `cell64` text
  that a fact holds in its `cell` field.
