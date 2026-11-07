# emem-trace

`emem-trace` defines the signed execution trace of a device, `emem.os_trace.v1`,
and the code that checks it. A device is a robot, a drone, a sensor host or a
satellite. emem admits a device reading as a fact only when the reading is
bound inside a trace that passes this check.

You need this crate when you change the trace record, its checks or its
tokens. You also need it to build or check a trace in your own Rust code. To
check a trace from any language, you do not need it: send the trace to
`POST /v1/trace_verify`.

| | |
|---|---|
| Version | 2.4.4 (workspace version in the root `Cargo.toml`) |
| Used by | `emem-airgap`, `emem-api-rest`, `emem-cli`, `emem-storage`; `emem-primitives` in two examples only (`satellite_downlink.rs`, `orin_stream.rs`) |
| Uses | `emem-core` (substrate profiles, platform registry), `emem-attest` (preimage, Merkle root) |
| Tests | 13 unit tests in `src/`, 10 integration tests in `tests/` |
| Test vectors | 4 files in [spec/test_vectors/os_trace/](../../spec/test_vectors/os_trace/) |
| I/O | None. The crate has no network, disk or async code. |

## Try it

The live responder runs this verifier. Send a committed test vector with a
broken segment chain:

```sh
jq .input spec/test_vectors/os_trace/trace.os_trace.chain_broken.0002.json \
  | curl -s https://emem.dev/v1/trace_verify \
      -H 'Content-Type: application/json' -d @- \
  | jq -c '{verdict, codes: [.report.reasons[].code], trace_cid: .report.trace_cid}'
# {"verdict":"reject","codes":["chain_broken"],
#  "trace_cid":"n4viesyenopyotvxoibkr4xie2p65szlmi6nqcjvrmf6q323bg6a"}
```

Run it from the root of a checkout. It needs `curl` and `jq`, and no key. The
vector `trace.os_trace.admit.0001.json` gives `"verdict":"admit"` and no codes.
Both commands ran on 7 November 2026. We wrapped the output to fit the page.

To run the crate tests from a checkout (needs Rust 1.91 or later):

```sh
cargo test -p emem-trace
```

We did not run `cargo test` for this page, because the host had no Rust
toolchain. The test counts come from the `#[test]` lines in the source.

## What each module does

| Module | Contents |
|---|---|
| `schema` | The record: `OsTrace`, `DeviceIdentity`, `TraceSegment`, `EmittedOutput`. Builds and signs a trace (`build_and_sign_v1`, `build_and_sign_chained_v1`) and computes its `trace_cid`. |
| `verify` | `verify_os_trace(trace, profile, claimed_payload_digest)`. Returns a `VerificationReport` with the verdict, every failed check and the layer coverage. |
| `enroll` | `PlatformAttestation` (`emem.platform_attestation.v0`) and `verify_platform_attestation`. This checks that a whitelisted vendor or operator key endorsed a device key. |
| `token` | The `emem:trace:<trace_cid>` and `emem:attestation:<attestation_cid>` tokens: compose and parse. |
| `drift` | `drift_contradiction_score`. It compares a device value with an Earth-archive value at the same place. |

## How a trace is built

A trace covers one capture window on one device. It holds:

1. The device identity: key, key epoch, substrate profile, platform, OS,
   kernel and boot id.
2. One segment for each captured log layer, for example `syscall` or
   `sensor_bus`. A segment holds the BLAKE3 digest of the raw log, not the
   log. Each segment holds the digest of the segment before it, so the
   segments form a chain.
3. `trace_root`: a Merkle root over the segment digests, in chain order.
4. The outputs: each sensor payload the device sent in the window, with its
   digest, band, time and layer.
5. Optional `prev_trace_cid`: the previous trace of the same device, so
   windows also form a chain.
6. The device signature over a BLAKE3 preimage. The preimage binds the
   schema, the identity digest, the profile, the window and the root. It
   also binds the digest of each whole output record and `prev_trace_cid`.
   The preimage is
   `emem_attest::os_trace_preimage_v1`.

`trace_cid` is the BLAKE3 hash of the canonical CBOR of the whole signed
record, in base32. Facts use the same rule.

## What the verifier checks

`verify_os_trace` runs every check and returns every failure. It does not
stop at the first one. The verdict is `admit` only when the list is empty.
There are 17 reason codes:

| Area | Reason codes |
|---|---|
| Record and profile | `schema_mismatch`, `profile_mismatch`, `admission_not_trace_based`, `encoding_failed` |
| Window and segments | `window_empty`, `empty_trace`, `missing_layer`, `seq_out_of_order` |
| Chain and root | `chain_broken`, `duplicate_segment_digest`, `root_mismatch`, `prev_trace_malformed` |
| Clocks | `clock_non_monotonic`, `clock_outside_window`, `output_outside_window` |
| Binding and signature | `output_unbound`, `signature_invalid` |

The profile sets the required layers. The 18 profiles are in
`crates/emem-core/data/substrates-v0.json`. 13 of them use the rule
`os_trace_required`. For example, `robot.fleet.v1` requires `syscall`,
`scheduler`, `memory`, `sensor_bus`, `energy`, `thermal` and `inference`.
`fixed.sensor.v1` requires only `syscall` and `sensor_bus`.

`output_unbound` is the main write-path check. The write path passes the
payload digest of the new fact. If no output in the trace has that digest,
the fact is refused, even when the trace itself is sound.

`prev_trace_malformed` checks only the form of `prev_trace_cid`. The crate
has no store, so it cannot check that the previous trace exists.
`emem-storage` does that check.

## Where the checks run

| Caller | What it does |
|---|---|
| `emem-storage` (`trace_gate.rs`) | The write gate. A key enrolled as a device can write only through it. It stores admitted traces and links each fact to its trace. |
| `emem-api-rest` | `POST /v1/trace_verify` (stateless check), `POST /v1/trace_resolve` (token to stored trace), `POST /v1/enroll_verify`, `POST /v1/enroll_attested`, `POST /v1/attest_traced` (write), `GET /v1/devices`, `GET /v1/device_platforms`, `GET /v1/trace_encodings`. |
| `emem-airgap` | Encodes traces on a device with no network and checks them. |
| `emem-cli` | `emem-demo-device` builds and sends a real ftrace window from one host. |

## Rules for changing this crate

- The record and the preimage are wire formats. A change moves every
  `trace_cid` and breaks every device signature.
- The 4 committed vectors are the reference. `tests/vectors.rs` replays them
  and checks the verdict, the reason codes and the `trace_cid`. After a
  planned schema change, write them again with
  `EMEM_REGEN_VECTORS=1 cargo test -p emem-trace --test vectors`. Then review
  the diff as a wire change.
- `prev_trace_cid` is left out of the CBOR and the preimage when it is empty.
  This keeps a first trace byte-identical to a trace made before chaining.
- Keep the crate pure: no I/O and no async. Storage and HTTP belong to
  `emem-storage` and `emem-api-rest`.

## What ships and what does not

| Ships | Does not ship |
|---|---|
| The trace record, the verifier, the tokens and the 4 vectors | Self-service enrolment for vendor hardware. `POST /v1/enroll_attested` ships, but it admits a key only when an effective anchor endorses it. Today only the operator anchor is effective. |
| Platform attestation v0: one Ed25519 endorser signature over a BLAKE3 preimage | X.509 or DICE chain checks, and checks of boot measurements against reference values. The code calls these the v1 step. |
| An operator anchor, set with `EMEM_OPERATOR_ENDORSEMENTS` on a node | An effective vendor anchor. All 16 anchors in `device-platforms-v0.json` are provisional, so no vendor attestation is admitted. |
| The drift score and its three verdicts: `consistent` below 3 sigma, `tension` to 9 sigma, `contradicted` above | Code that calls the drift score on the write path. Only the example `crates/emem-primitives/examples/satellite_downlink.rs` calls it. |

On 7 November 2026, `GET /v1/devices` on emem.dev listed 1 device. Its
platform is `generic.linux-host`, its profile is `host.counters.v1` and its
assurance is `operator_endorsed` by `operator.vortx.v1`. No manufacturer
vouches for it.

## Read more

- [docs/robots.md](../../docs/robots.md): how devices use emem, and what
  ships for them.
- [spec/test_vectors/README.md](../../spec/test_vectors/README.md): the
  vector format and how to check a vector.
- [emem-device-traces skill](../../plugins/emem/skills/emem-device-traces/):
  how an agent inspects and checks device traces.
- [emem-airgap](../emem-airgap/README.md): traces on a device with no
  network.
- [emem-storage](../emem-storage/README.md): the write gate and the log.
