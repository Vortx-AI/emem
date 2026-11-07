# emem protocol test vectors

This directory holds known-answer files for the emem protocol. Each file
gives an input and the exact output that correct code must
produce. Use these files if you write a second emem responder, client or
verifier.

On 7 November 2026 one kind ships: `os_trace/`, with 4 vectors. The other
kinds in the table below have no files yet. They are on the plan.

Try one vector against the live responder. The command needs `curl` and
`jq`, and no key:

```sh
jq .input spec/test_vectors/os_trace/trace.os_trace.chain_broken.0002.json \
  | curl -s https://emem.dev/v1/trace_verify -H 'Content-Type: application/json' -d @- \
  | jq -c '{verdict, codes: [.report.reasons[].code], trace_cid: .report.trace_cid}'
```

Output on 7 November 2026, which matches the `expected` block of the file:

```json
{"verdict":"reject","codes":["chain_broken"],"trace_cid":"n4viesyenopyotvxoibkr4xie2p65szlmi6nqcjvrmf6q323bg6a"}
```

Read more: [docs/protocol.md §13](../../docs/protocol.md#13-test-vectors)
(test vectors) and [§12](../../docs/protocol.md#12-conformance-gates-the-four-cids)
(the four manifest CIDs).

## What ships and what is on the plan

| Kind | What it tests | Status on 7 November 2026 |
|---|---|---|
| `os_trace` | Verdict of an `emem.os_trace.v1` execution trace against a substrate profile | Ships: 4 vectors in `os_trace/` |
| `cell64` | Cell encode and decode round trip | Planned, no files |
| `tslot` | Tslot text encode and decode, tempo-class snapping | Planned, no files |
| `vec64` | vec64 derivation from a 1792-D fp16 vector | Planned, no files |
| `cbor` | Canonical CBOR of a Fact, Attestation or Receipt | Planned, no files |
| `cid` | Fact CID: `base32(blake3(canonical_cbor(fact))[:32])` | Planned, no files |
| `sig` | Ed25519 attestation signature | Planned, no files |
| `claim_eval` | Claim evaluation against a set of facts | Planned, no files |
| `derivation` | A registry function gives the expected output for fixed inputs | Planned, no files |

Until the planned kinds ship, use these known answers from other places:

- The four manifest CIDs in [docs/protocol.md §12](../../docs/protocol.md#12-conformance-gates-the-four-cids).
  `GET https://emem.dev/v1/manifests` returns the live values.
- `GET https://emem.dev/v1/log/test_vectors` gives known answers for the
  transparency-log tree and the attestation batch tree.
- The worked examples in docs/protocol.md §1.4 and §7.2.

## The four os_trace vectors

| File | Profile | Expected verdict | Reason code |
|---|---|---|---|
| `trace.os_trace.admit.0001.json` | `robot.fleet.v1` | `admit` | none |
| `trace.os_trace.chain_broken.0002.json` | `robot.fleet.v1` | `reject` | `chain_broken` |
| `trace.os_trace.output_unbound.0003.json` | `robot.fleet.v1` | `reject` | `output_unbound` |
| `trace.os_trace.archive_refused.0004.json` | `earth.satellite.v0` | `reject` | `admission_not_trace_based` |

On 7 November 2026 we sent the `input` of all four files to
`POST https://emem.dev/v1/trace_verify`. The verdict, the reason codes and
the `trace_cid` matched the `expected` block in each file.

## File format

Each vector is one JSON file. The `os_trace` files have this shape (the
example shortens the `trace` object):

```json
{
  "id": "trace.os_trace.admit.0001",
  "kind": "os_trace",
  "spec": "v1.3.0",
  "level": "L1",
  "input": {
    "profile": "robot.fleet.v1",
    "claimed_payload_digest": "...",
    "trace": { "...": "the full emem.os_trace.v1 record" }
  },
  "expected": {
    "verdict": "admit",
    "reason_codes": [],
    "trace_cid": "umplvo2hpglpnmu3qhw6g4jiujdhwjmp6bdjzmnnywm3plp5t5cq"
  },
  "notes": "A complete robot.fleet.v1 trace whose emitted output binds the claimed payload digest."
}
```

| Field | Meaning |
|---|---|
| `id` | `<group>.<subgroup>.<test>.<seq>`, lower case, dot-separated. The file name is `<id>.json`. |
| `kind` | The vector kind. It is also the directory name. |
| `spec` | The spec version that the vector tests. |
| `level` | The conformance level that the vector belongs to. All four `os_trace` vectors are `L1`. |
| `input` | Kind-specific input. For `os_trace`, it is the request body of `POST /v1/trace_verify`. |
| `expected` | Kind-specific output that correct code must give. |
| `notes` | One sentence about what the vector shows. |

## Replay the vectors with Rust

The test `crates/emem-trace/tests/vectors.rs` reads each committed file,
runs `emem_trace::verify_os_trace`, and compares the verdict, the reason
codes and the `trace_cid`. It also checks that there are exactly 4 files.

```sh
cargo test -p emem-trace --test vectors
```

We did not run this command on 7 November 2026: the host had no Rust
toolchain. The command comes from the test file. The live check above uses
the same `verify_os_trace` function through `/v1/trace_verify`.

## Regenerate the os_trace vectors

The vectors come from a fixed key and fixed clocks, so the output is
deterministic. Regenerate them only after an intentional schema change:

```sh
EMEM_REGEN_VECTORS=1 cargo test -p emem-trace --test vectors
```

Review the diff as a wire change. A vector that moves without a spec bump
is a bug.

## Why the os_trace vectors were re-minted on 2026-08-20

The signatures and trace CIDs were re-minted on 2026-08-20 to close a gap
in signature coverage. Before, the trace preimage bound only the
`payload_digest` of each output. A signed trace still verified after an
edit to `band`, `layer` or `emitted_at_ns`. Now the preimage binds the
digest of the whole output record.

No verdict and no reason code changed in that regeneration. That shows it
was a re-signing and not a change in behaviour.

The change stayed in `emem.os_trace.v1` and did not make a v2. No trace
signed outside this repository existed, because the device roster was
empty. A v2 would only leave a v1 with a known hole that verifiers must
still accept.

## Add vectors

The rule for the project: a new protocol feature ships with new vectors
before merge. A new function in the derivation registry ships with at
least one `derivation/` vector. Features from before this rule have no
vectors yet, as the status table shows.

The committed JSON files are the reference. A test replays them; the
`os_trace` test is the model to copy.
