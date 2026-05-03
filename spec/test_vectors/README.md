# emem Protocol — Test Vectors

This directory is the **conformance gate** for the emem protocol (spec §19).
A conforming implementation MUST pass all vectors marked at its claimed
conformance level (L0 / L1 / L2).

## File format

Every vector is a single JSON file matching:

```json
{
  "id":       "cell.cell64.roundtrip.0001",
  "kind":     "cell64",
  "spec":     "v0.0.3",
  "level":    "L0",
  "input":    { "...": "kind-specific" },
  "expected": { "...": "kind-specific" },
  "notes":    "Optional human-readable rationale."
}
```

`id` is `<group>.<subgroup>.<test>.<seq>`, lowercase, dot-separated.

## Vector kinds

| Kind          | What it tests                                                           | Directory      |
|---------------|--------------------------------------------------------------------------|----------------|
| `cell64`      | cell encode/decode round-trip; spatial-locality property                 | `cell64/`      |
| `tslot`       | tslot text encode/decode; tempo-class snapping                           | `tslot/`       |
| `vec64`       | vec64 derivation from a 1792D fp16 vector                                | `vec64/`       |
| `cbor`        | canonical CBOR encoding of a Fact / Attestation / Receipt                | `cbor/`        |
| `cid`         | fact CID computation (`base32(blake3(canonical_cbor(fact))[:32])`)       | `cid/`         |
| `sig`         | ed25519 attestation signature                                            | `sig/`         |
| `claim_eval`  | claim evaluation against a fact bundle                                   | `claim_eval/`  |
| `derivation`  | function registry entry produces expected output for fixed inputs        | `derivation/`  |

## Adding vectors

New protocol features MUST ship with new vectors before merge. New `Derivation.fn`
entries (spec §16) MUST ship with at least one `derivation/` vector each.

A vector is considered authoritative once its CID appears in the schema CID
manifest (`/.well-known/emem.json#manifests.schema_cid`).

## Generating from agri ground truth

Many vectors will be generated from the AgriSynth bootstrap cubes (spec §17):

```bash
# from emem repo root
cargo run -p emem-cli -- ingest --cube ../agri/farms/IOWA_US/cube_10m.npz \
  --attester-key ~/.emem/key \
  --registry-cid <cid> \
  --emit-vectors spec/test_vectors/derivation/iowa_us/
```

Generated vectors are committed to the repo so any implementation can replay them
without access to the source cubes.
