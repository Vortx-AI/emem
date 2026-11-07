# satellite-downlink: one pass through the trust layer

This example shows how a satellite operator would write facts to emem.
A spacecraft, SAT-042, images three cells of Nile Delta cropland on pass 1881.
The example runs the full operator loop in five acts, and two of the acts are refusals.

## What ships and what does not

| Part | State on 7 November 2026 |
|---|---|
| The example program | Ships. It is Rust, offline and deterministic (fixed key, fixed clocks). |
| The `orbital.satellite.v1` profile on emem.dev | `candidate`. `GET /v1/substrates` lists it, but the ingest path is not open. emem.dev does not accept these writes today. |
| The trace gate and trace verifier | Ship in `emem-storage` and `emem-trace`. The example runs them against an in-memory store. |
| A live drift anchor | Not wired. The anchor value (NDVI 0.6402, sigma 0.02) is a constant in the source. |

Of the 18 profiles in `GET /v1/substrates` on 7 November 2026, only `earth.satellite.v0` is `active`.
In the code, `candidate` means "pinned so device makers can build against it, but the ingest path is not open yet"
(`crates/emem-core/src/substrates.rs`).

## What you need

- A Rust toolchain (MSRV 1.91) and a clone of this repository.
- `protoc`, the Protocol Buffers compiler. The `lance-encoding` dependency needs it at build time.
  On Debian or Ubuntu, install `protobuf-compiler`. Or set `PROTOC` to the path of a `protoc` binary.
- Network access for the first build only, to download crates. The program itself uses no network, no key and no account.

## Run it

```bash
cargo run -p emem-primitives --example satellite_downlink
```

We ran this command on 7 November 2026 with rustc 1.99.1 and libprotoc 30.2.
The first build took about 25 minutes on the docs host. Later runs took about 5 s.
Three runs gave byte-identical output. The last run was at 09:25 UTC.
CI compiles the example (`cargo clippy --workspace --all-targets`), but no CI job runs it.

## What you get

The program prints one block for each act and asserts the results.
The full output of the run on 7 November 2026:

```text
act 1  enrolled spacecraft key mwndc7zcesxqvw7bgxmbff3trktrynx3edprmwtzodv6qt45vqvq
       profile orbital.satellite.v1 requires 8 trace layers:
       [Syscall, Scheduler, Memory, SensorBus, Signal, Energy, Thermal, Storage]

act 2  write without a trace:
       attestation verification failed: os_trace gate: attester mwndc7zcesxqvw7bgxmbff3trktrynx3edprmwtzodv6qt45vqvq is enrolled under orbital.satellite.v1; writes require the device's OS execution trace and none was presented

act 3  smuggle one unemitted fact into a sound batch:
       attestation verification failed: os_trace gate: fact at band indices.ndvi carries payload digest cyzwaknc3wznnmpjc52er4sy4kxqkeq6ysflubbmahrxassy4jlq that is not bound in the trace's emitted outputs

act 4  admitted: 3 facts under one verified trace
       emem:fact:defi.zb55e.natI.tUpu:ol4ugzp4rmkc7nodxgfnm3bucamdbell6anysvt5syqlwhmzjtia
       emem:fact:defi.zb55e.nadU.tUra:wndl3zwt4b27q2ufzpwc6oxwqx7a2t3ysd3evwzjnewvz5wphefa
       emem:fact:defi.zb55e.mUgI.tUsE:asue5jcmxpwiur3ajfj7xil5u7uz7bvh4rrxbcqswkvjeop2lbhq
       emem:trace:3arfrney6ieeugzekzqfufwoayvqj3edhc6fbwaqzhydxn2hbogq
       emem:bundle:sfxaann374s7omyhfhbavl3qne

act 5  drift-anchor scoring against the open archive:
       defi.zb55e.natI.tUpu  device 0.6431 vs anchor 0.6402  score 0.05  Consistent
       defi.zb55e.nadU.tUra  device 0.6512 vs anchor 0.6402  score 0.15  Consistent
       defi.zb55e.mUgI.tUsE  device 0.2103 vs anchor 0.6402  score 0.88  Contradicted

       tampered segment 2 after signing; the verifier answers:
       reject: chain broken at seq 3

done   three facts, one trace, one bundle, and every forgery named.
```

The tokens are local to the in-memory store. They do not resolve on emem.dev.
The act 5 heading says "open archive", but the anchor is the fixed constant in the source.

What each act does:

1. Enroll. The spacecraft key enrolls under `orbital.satellite.v1`.
   The substrate registry sets the eight trace layers that the platform must capture.
   They are syscall, scheduler, memory, sensor_bus, signal, energy, thermal and storage.
2. Write without a trace. The store refuses the write.
   The device can contribute, but the protocol does not take its word.
3. Smuggle a fact. The program captures the pass window as a digest-chained OS trace and binds the three downlink payloads.
   It then adds a fourth fact that the execution did not emit.
   The store refuses the batch and names the forged digest.
4. Write the honest batch. The store admits three signed facts under one verified trace.
   The program prints the handles that stay valid outside any context window.
   It prints one `emem:fact:` token for each observation and one `emem:trace:` token for the execution evidence.
   It also prints one `emem:bundle:` token for the pass.
5. Score and tamper. The program scores each claim against the fixed anchor.
   The score is `z / (1 + z)` with `z = |device - anchor| / (3 * sigma)`.

| Device NDVI | Score | Verdict |
|---|---|---|
| 0.6431 | 0.05 | `Consistent` |
| 0.6512 | 0.15 | `Consistent` |
| 0.2103 | 0.88 | `Contradicted` |

A score of 0.5 or more is `Tension`, and 0.75 or more is `Contradicted` (`crates/emem-trace/src/drift.rs`).
The contradicted claim stays visible, and the store keeps it.
The anchor is not yet wired to archive recalls (`drift.rs` calls that ingest-side work).
After that wiring ships, a real harvest will move the anchor toward the device value at the next archive pass.

Last, the program changes the log digest of stored segment 2 and verifies the trace again.
The verifier answers `reject: chain broken at seq 3`, because segment 3 no longer points to the digest of segment 2.

## Where to read more

- Source: `crates/emem-primitives/examples/satellite_downlink.rs`.
- Design and onboarding steps: [docs/plans/encoder-substrates.md](../../docs/plans/encoder-substrates.md).
- A two-vendor robot handoff over plain HTTP: [examples/fleet-memory/](../fleet-memory/).
