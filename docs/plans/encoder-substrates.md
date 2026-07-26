# Encoder substrates: the trust layer for every machine that observes the world

Status: registry, schema, and verifier shipped as code (this change);
ingest wiring open. Owner: protocol. First written 2026-07-26.

## The one-sentence rule

Every device that contributes to emem, satellite ground segment,
telescope, microscope, CCTV head, phone, drone, robot, plant machine,
is respected as an observer of the physical world, and none of them is
believed on its word: the protocol admits a device's output only when
the output is bound inside the device's complete, unaltered OS
execution trace, verified against the device's substrate profile.

This is not a new substrate. It is a new trust layer. The founding
Earth substrate keeps its own admission rule, recomputability from
free public archives, and takes on a second duty: it is the standing
drift anchor that every device claim is contradiction-scored against.
The trace closes the execution gap (did this capture really run, on
this device, over this window). The anchor closes the world gap (does
the claimed value agree with an independently recomputable record of
the same place). What the record delivers after both checks is not a
description someone stored. It is evidence of what actually happened,
checkable by a party who trusts neither the device nor the server.

## Why an OS trace and not the device's output

A sensor reading is a claim. Anyone can sign a claim; a key proves who
spoke, not what happened. The next stronger evidence a commodity
device can produce without special hardware is its own execution
record: the syscall stream, scheduler activity, memory tracks, energy
draw, thermal readings, bus traffic, network and storage activity, and
which model weights ran, over the capture window. Forging a value then
requires forging a whole causally consistent execution, chained
segment by segment and signed over one root, and the forgery has to
survive layer cross-checks (an inference layer with no matching
scheduler activity, a sensor-bus emission with no syscall around it)
plus the drift score against the Earth anchor. Fabrication stops being
an edit and becomes a simulation project.

The resolution of these substrates varies by nine orders of magnitude,
from microns under a microscope objective to tens of kilometers per
weather cell. The profile registry carries that range per profile
(`grain_min_m` to `grain_max_m`), so a reader always knows the grain
of the instrument behind a fact.

## What shipped in this change

Three code pieces, all pure (no I/O), sitting in the trust stack
beside `emem-attest`:

1. **The substrate profile registry** (`crates/emem-core/src/substrates.rs`,
   data in `crates/emem-core/data/substrates-v0.json`, manifest kind
   `emem-substrates`, the ninth content-addressed manifest). Nine
   profiles: `earth.satellite.v0` (active, archive-recomputable, the
   drift anchor) and eight candidate device profiles
   (`observatory.telescope.v1`, `lab.microscope.v1`, `urban.cctv.v1`,
   `mobile.handheld.v1`, `aerial.drone.v1`, `robot.fleet.v1`,
   `industrial.machine.v1`, `fixed.sensor.v1`), every one of them
   `os_trace_required` with its required trace layers pinned. The
   validator enforces the protocol stance in code: a trace-admitted
   substrate can never be a drift anchor, an archive substrate never
   requires trace layers, and the registry must always contain an
   active anchor.

2. **The `emem.os_trace.v1` record** (`crates/emem-trace/src/schema.rs`).
   Device identity (ordinary attester key plus platform, OS, kernel,
   and a per-boot id), a capture window on the monotonic clock,
   digest-chained trace segments per layer (the raw log bytes stay on
   device or in cold storage; the record commits to them and they are
   producible on audit), the emitted output digests, a v1 merkle root
   over the chain, and one ed25519 signature over a new domain-separated
   preimage (`os_trace_preimage_v1` in `emem-attest`) binding schema,
   device, profile, window, root, and outputs together. `trace_cid` is
   derived exactly the way `fact_cid` is: blake3 over canonical CBOR,
   base32 no-pad lowercase.

3. **The verification engine** (`crates/emem-trace/src/verify.rs`) and
   **the drift rule** (`crates/emem-trace/src/drift.rs`). The verifier
   collects every failure (fifteen named reject reasons: broken chain,
   missing layer, clock outside window, unbound output, invalid
   signature, and so on) and admits only on an empty list. The drift
   rule scores a device claim against an anchor readout into `[0, 1]`
   with pinned thresholds: three sigma is the consistent/tension
   boundary, nine sigma is tension/contradicted, and a claim in
   tension is kept and surfaced through the contradiction index, never
   silently dropped, because a real change in the world looks exactly
   like tension at first.

The Earth substrate profile also names its four evidence layers,
`trace` (transparency log and receipts), `signal` (raw bands and
Fourier encodings), `inference` (foundation embeddings with their
`served_via` record), and `weather` (forcing context), which are the
causally linked views contradiction scoring runs across. The numeric
split of a delta among causes remains the change-attribution road
already on the roadmap.

## What is deliberately out of scope

- Hardware roots of trust (TPM quotes, enclave attestation). The OS
  trace rule is what a commodity device can meet today; a hardware
  quote can later ride in the same envelope as one more segment layer.
- Streaming trace transports. The record is windowed by design;
  intermittent devices sign windows offline and sync later.
- Believing the trace makes the value true. It does not; that is the
  anchor's job, and the two checks stay separate on purpose.

## Steps for the emem agent: wiring the layer in

Ordered; each step is one reviewable change with its insertion points.

1. **Expose the registry.** Add `GET /v1/substrates` next to
   `/v1/sources` in `crates/emem-api-rest/src/lib.rs` (router chain
   near the other registry routes), serving `substrates::DEFAULT` with
   its manifest CID. Add an `emem_substrates` descriptor to `TOOLS` in
   `crates/emem-mcp/src/lib.rs`, a dispatch arm in `mcp_tool_call`,
   membership in the `robotics` bundle, the openapi path entry, and
   run `scripts/sync_counts.py` so every quoted count moves together.
2. **Gate the write path.** Extend `POST /v1/attest` with an optional
   `os_trace` field. In `emem-storage`, before `put_attestation`
   accepts facts from an attester whose substrate profile is
   `os_trace_required`, run `emem_trace::verify_os_trace` with the
   fact's payload digest and reject on any non-empty reason list,
   returning the reasons verbatim in the error body. Persist the
   `trace_cid` beside the per-fact proofs and append the trace record
   to the transparency log so inclusion proofs cover it.
3. **Stateless verification surfaces.** `POST /v1/trace_verify`
   (mirror of `/v1/verify_receipt`: body in, report out, no state) and
   an `emem-cli trace verify` subcommand so a device maker can debug
   an enrollment offline before ever touching the network.
4. **Drift-anchor wiring.** On every admitted device fact, recall the
   anchor band at the fact's cell, compute `DriftAnchorCheck`, write
   the result into the contradiction index, and surface it through
   `emem_memory_contradictions` and the change-attribution ledger.
5. **Token grammar.** Mint `emem:trace:<trace_cid>` as a resolvable
   token kind so an agent can carry the execution evidence the same
   way it carries a fact, and let bundles include trace tokens.
6. **Upgrade the robotics placeholder.** Extend
   `examples/fleet-memory/` so each vendor's writer builds and signs
   an `emem.os_trace.v1` window around its observations
   (`OsTrace::build_and_sign_v1`), and the reader verifies the trace
   before trusting the map update. Then the ROS 2 capture profile
   (`ros2.bag.v2` encoding) and an eBPF capture profile for Linux
   devices, which is the edge encoder the roadmap's robotics
   lighthouse asks for.
7. **Conformance.** Commit `spec/test_vectors/os_trace/` vectors in
   the format `spec/test_vectors/README.md` already defines, add the
   os-trace section to `docs/protocol.md`, and fold the trust layer
   into the whitepaper's threat model.

## Steps for an operator or device maker: onboarding an encoder

1. **Pick the profile.** Read `/v1/substrates` (or
   `crates/emem-core/data/substrates-v0.json`) and find the profile
   matching your machine class. If none fits, propose a new entry by
   PR; the validator will hold you to the admission rules.
2. **Mint the device identity.** Generate an ed25519 keypair
   (`emem-cli keygen`), one per device, epoch 0. The public key is the
   device's name in the record forever; protect the secret
   accordingly.
3. **Capture the required layers.** The profile lists them. On Linux,
   ftrace or eBPF covers syscall, scheduler, and memory; RAPL or a
   power rail covers energy; hwmon covers thermal; the bus capture is
   whatever your instrument speaks. Chunk each layer into segments
   with monotonic-clock extents and blake3 the raw bytes into
   `log_digest`. Keep the raw logs; the record commits to them and an
   audit may ask for them.
4. **Bind your outputs.** Every sensor payload you intend to write
   must appear as an `EmittedOutput` (its blake3 digest, its emission
   time inside the window) before signing. An output not bound in the
   trace will be rejected no matter how sound the trace is.
5. **Build, sign, self-verify.** `OsTrace::build_and_sign_v1` chains
   the segments and signs the preimage. Run
   `emem_trace::verify_os_trace` locally against your profile before
   submitting; the report names every problem at once.
6. **Write, then keep the tokens.** Submit the attestation with the
   trace. On admit, keep the `emem:fact:` tokens (and the trace CID)
   rather than the payloads; any other agent can resolve and re-check
   them without trusting you. On reject, the reason list is the
   debugging story.
7. **Expect to be scored.** Your claims will be drift-checked against
   the Earth anchor where the bands overlap. Consistent claims build
   your attester reputation; tension is surfaced, not punished;
   contradiction sinks the claim and stains the key, and the whole
   history is public in the log.

## How this changes what emem is

Before this layer, emem was a shared memory whose one substrate
happened to be trustworthy because its sources are public archives.
After it, emem is a protocol for admitting the physical world's
observers one class at a time, under one uniform standard of evidence:
execution you can verify, anchored to a record you can recompute. The
memory is the byproduct. The deliverable is proof of what actually
happened, and it stays checkable when no one who was present, human or
model, is still around to ask.
