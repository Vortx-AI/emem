# emem-storage

`emem-storage` is the storage layer of the emem responder. It finds a fact
in the cache, or makes it on a miss. It writes each attestation to an
append-only log on disk. It also signs the receipt that goes out with each
read.

You need this crate when you change how the responder stores, signs or
logs facts. Examples are a new receipt field, a new sled tree, or a change
to who may write a fact. To read facts and check receipts, you do not need
it. The responder serves the receipt, and any Ed25519 library checks it.

| | |
|---|---|
| Version | 2.4.4 (workspace version in the root `Cargo.toml`) |
| Used by | `emem-api-rest`, `emem-cli`, `emem-primitives` |
| Uses | `emem-cache`, `emem-fetch`, `emem-attest`, `emem-trace`, `emem-fact`, `emem-core`, `emem-codec` |
| Tests | 65 unit tests inline in `src/` (24 `#[test]`, 41 `#[tokio::test]`) |
| Specification | [docs/protocol.md](../../docs/protocol.md), sections 7 to 9 (receipt, Merkle proof, log) |

## Try it

Recall the Mount Fuji elevation fact and look at its receipt. The receipt
names the preimage rule and holds the Merkle proof that this crate adds:

```sh
curl -s https://emem.dev/v1/recall -H 'content-type: application/json' \
  -d '{"cell":"defi.zb592.nUkO.zEzE","bands":["elevation"]}' \
  | jq '.receipt | {preimage_version, fact_cids, proof: (.merkle_proof | del(.root))}'
```

On 7 November 2026 this gave `preimage_version` 2 and one fact id,
`qejpz7cgvk347ordd6vrdtmwzprqlzcnkmotsjton2x4y2jjbplq`. The proof had
`leaf_count` 1, an empty `path`, and `log_entry_hash`
`quzqrplqfultmmku5efrp4pnghwqpf37rimscveyxbonqxnsk33a`. The `root` field,
left out above, is an array of 32 bytes.

The `log_entry_hash` names the attestation in the log. Ask the
transparency log where that entry is:

```sh
curl -s 'https://emem.dev/v1/log/inclusion?fact_cid=qejpz7cgvk347ordd6vrdtmwzprqlzcnkmotsjton2x4y2jjbplq' \
  | jq '{entry_hash_b32, leaf_index, tree_size, path_len: (.audit_path_b32 | length)}'
```

On 7 November 2026 the entry was leaf 2,463,431 in a tree of 3,246,184
entries, with an audit path of 22 hashes. Both commands need `curl` and
`jq`. Both ran on that day.

To run the crate tests from a checkout (needs Rust 1.91 or later):

```sh
cargo test -p emem-storage
```

We did not run `cargo test` for this page, because the host had no Rust
toolchain. The test counts come from the test attributes in the source.

## What each module does

| Module | Holds |
|---|---|
| `lib.rs` | The `Storage` trait that all read primitives use. `MaterializingStorage`, which joins the cache, the fetch dispatcher, the registries and the log. `FactPlanePolicy`, which decides who may write a fact at an address. The names of the sled trees. |
| `server.rs` | `Server`, which holds the storage, the responder key and the manifest ids. `sign_receipt` and its variants. `receipt_message`, which rebuilds the signed 32 bytes from a receipt. |
| `merkle_log.rs` | `AttestationLog`: the append-only log of attestations on disk |
| `attesters.rs` | `AttesterRegistry`: per-key counts of batches, facts, citations and cells |
| `trace_gate.rs` | `TraceGate`: device enrolment, and the check that an enrolled device sends a valid execution trace with its facts |
| `artifacts.rs` | `ArtifactStore`: a content-addressed blob store with a byte limit, for the pixel bytes of raster fields |

## How a read becomes a fact

1. A primitive asks for `(cell, band, tslot)` keys. The crate looks for
   their fact ids in the cache.
2. On a hit, it returns the stored fact id.
3. On a miss, `materialize_many` returns `MaterializeMiss` (wire code
   `CidNotFound`). It does not fetch. The inline materializers in
   `emem-api-rest` read the upstream data with `emem-fetch`, compute the
   value, sign it, and store it with `put_attestation`.
4. `put_attestation` checks the Merkle root over the fact ids and the
   signature before it stores anything. It then appends the attestation
   to the log and saves one proof per fact.

`MaterializingStorage::rooted(dir, ...)` keeps data on disk:
`<dir>/cache.sled` for the cache and `<dir>/log/` for the log.
`MaterializingStorage::ephemeral(...)` uses a temporary cache and log, for
tests and short runs.

## Receipts

Each call to `sign_receipt` or a variant uses one private function.
It gets the Merkle proof for the first cited fact before it signs, because
preimage version 2 includes that proof in the signed bytes. Thus a relay
cannot remove the proof and keep a valid signature. If no proof exists, v2
signs that absence.

The preimage functions are in `emem-attest`. `receipt_message` in this
crate picks v2, v1 or the old v0 rule from the receipt's own
`preimage_version`. Use that one function in each verifier. Do not write a
second copy of a preimage rule.

## The log on disk

Each record is a 4-byte little-endian length, the CBOR of the
attestation, and the BLAKE3 hash of that CBOR. The crate names each segment file
`merkle.log.<n>`. When a segment reaches 1 GiB, the crate closes it and
writes a 32-byte BLAKE3 hash of the full segment at its end.
`AttestationLog::verify` checks those hashes for the closed segments. It
does not check the open segment.

`emem-api-rest` builds the RFC 6962 transparency log at `/v1/log/*` from
the record hashes of this log. The tree rules are in `emem-attest`.

## Who may write a fact

By default, `FactPlanePolicy` accepts facts that take an
address only from these keys:

- the responder's own key
- a device that the trace gate enrolled (it must also send its trace)
- keys that the operator lists in `EMEM_FACT_PLANE_WRITERS`, separated by
  commas

`EMEM_FACT_PLANE_OPEN=1` lets any signing key write. A derivative fact and
an edge do not take an address, so this policy does not stop them.

## Rules for changes

- Keep three places in step. A change to `sign_receipt` needs the same
  change in [docs/protocol.md](../../docs/protocol.md) and in the browser
  verifier in `web/humans.html`. The root [AGENTS.md](../../AGENTS.md) gives
  this rule.
- Do not change the bytes of a v0, v1 or v2 preimage. Old receipts must
  still verify. A new signed field needs a new preimage version.
- Do not delete or rewrite `var/emem/sled/` or the log segments. They are
  the live record, and other people debug from them.
- Do not read `var/emem/identity.secret.b32`. It is the responder's secret
  key.

## What does not ship

- `merkle_log.rs` defines a `SegmentBackup` trait to copy closed segments
  to S3 or IPFS. No type in the workspace uses this trait yet.
- A receipt has one Merkle proof, for the first cited fact only. A
  verifier gets the other fact ids from the signed list in the receipt.
- The doc comment on `sign_receipt` still says that it signs preimage v1.
  The code signs v2, as the first command above shows.

## Read more

- [docs/protocol.md](../../docs/protocol.md), section 7: the receipt and a
  worked preimage.
- [docs/protocol.md](../../docs/protocol.md), sections 8 and 9: the Merkle
  inclusion proof and the append-only log.
- [docs/glossary.md](../../docs/glossary.md): the terms receipt and
  `fact_cid`.
- [crates/emem-fact/README.md](../emem-fact/README.md): the fact,
  attestation and receipt types that this crate stores and signs.
- [crates/emem-codec/README.md](../emem-codec/README.md): the `cell64`
  and `tslot` in each cache key.
