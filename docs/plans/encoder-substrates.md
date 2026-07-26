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

The resolution of these substrates spans more than eleven orders of
magnitude, from 100 nanometers under a microscope objective to tens of
kilometers per weather cell. The profile registry carries that range per profile
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
   collects every failure (sixteen named reject reasons: broken chain,
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

## What the open archives actually provide (checked live)

The question comes up immediately: the Earth substrate's direct
sensors are Sentinel-1 and Sentinel-2, so do those give us OS traces?
No. No public satellite archive publishes the execution trace of its
ground processing, and the design does not pretend otherwise. What
the archives do publish, checked against live Planetary Computer STAC
items on 2026-07-26, is **declared processing lineage**:

- Sentinel-2 L2A items carry `s2:processing_baseline` (live value
  `05.12`), `s2:generation_time`, `s2:datastrip_id`, `s2:granule_id`,
  and `s2:product_uri` naming the exact SAFE product, whose
  `manifest.safe` records the processing facility and software
  history.
- Sentinel-1 items carry `s1:processing_datetime`, `s1:orbit_source`
  (`RESORB` on the checked item, which orbit solution was used),
  `s1:instrument_configuration_ID`, `s1:processing_level`, and the
  full `sat:` orbit block.
- The [Copernicus Data Space traceability
  service](https://dataspace.copernicus.eu/analyse/traceability)
  registers a BLAKE3 checksum for each Sentinel product at creation
  and through its lifecycle, so a downloaded product can be checked
  against the publisher's own register. BLAKE3 is the hash this
  protocol already uses everywhere.

Three consequences, all leverage rather than compromise:

1. Declared lineage is exactly why `earth.satellite.v0` keeps the
   `archive_recomputable` admission rule instead of a retroactive
   trace rule: the archive's evidence is re-fetchability plus the
   publisher's own integrity register, and pretending a SAFE manifest
   is an execution trace would blur the line the registry exists to
   draw. The profile's `declared_lineage` field lists these keys so a
   reader knows what is checkable per fact.
2. The Copernicus BLAKE3 register closes a documented caveat: the
   provenance-class docs admit the cited source is pinned "by URL, id
   and capture time, not by a content hash". Fetching the product's
   registered checksum at materialization time and carrying it in the
   fact's `sources[]` turns that URL pin into a content pin, with the
   publisher on the hook for the digest. That is a small connector
   change with outsized trust value, and it is now wiring step 4a
   below.
3. Processing lineage feeds the sensor term of change attribution: a
   `processing_baseline` bump is a real, recorded cause of a value
   moving while the world stands still (baseline changes have shifted
   Sentinel-2 radiometry before), so the recalled lineage keys belong
   in the evidence ledger `emem_change_attribution` already serves.

## Standards and demo data to align with

Standards worth conforming to rather than reinventing, each mapped to
the piece of this design it touches:

- **IETF RATS architecture (RFC 9334).** The remote-attestation
  vocabulary: the device is the Attester, the responder's verification
  engine is the Verifier, the write gate is the Relying Party. The
  os_trace record is RATS Evidence; keeping the roles named this way
  keeps the door open for hardware Evidence (TPM 2.0 quotes, DICE
  layered certificates) to ride as an extra segment layer later.
- **in-toto attestations and SLSA provenance.** The supply-chain
  world's statement format (subject digest, predicate, DSSE
  envelope). An os_trace with its emitted output digests is the same
  shape: subject = payload digest, predicate = execution evidence. An
  export mapping from `emem.os_trace.v1` to an in-toto statement is
  cheap and makes device evidence legible to existing SLSA tooling.
- **W3C PROV and ISO 19115 lineage.** The archival lineage models the
  geospatial world already speaks; `declared_lineage` keys should map
  onto PROV activities and ISO lineage steps in the export path, not
  in the wire record.
- **STAC and its processing extension.** Already in the fetch path via
  the `StacPc` connector; the processing extension's
  `processing:software` and `processing:lineage` fields are the
  standardized home of the same declared lineage the Sentinel
  properties carry ad hoc.
- **CTF, the Common Trace Format (LTTng, barectf, Zephyr).** The
  segment `encoding` strings already name it (`zephyr.ctf.v1`). CTF is
  the practical on-disk format for the syscall, scheduler, and memory
  layers on Linux and on microcontroller OSes; `ros2_tracing` produces
  LTTng/CTF sessions on robots today, which makes the robot lighthouse
  concrete: the edge encoder chunks an existing CTF session into
  segments rather than inventing a capture stack.
- **eBPF and ftrace** as the Linux capture mechanisms behind those
  encodings, and **rosbag2** for the sensor-bus layer on ROS 2.

Demo and test data to build against, all public:

- Live Sentinel STAC items (Planetary Computer, already a connector)
  for declared-lineage extraction, and the Copernicus traceability
  register for content-pinning a real downloaded granule.
- LTTng's published example CTF traces and `ros2_tracing` sample
  sessions, as segment-layer inputs for the fleet-memory upgrade.
- The DARPA Transparent Computing / OpTC public datasets: large,
  labeled OS provenance traces (benign plus red-team activity), the
  right stress test for whether the verifier's layer cross-checks
  catch execution stories that do not hang together.
- The repo's own `spec/test_vectors/` format for conformance vectors
  (`os_trace/` is step 7 of the wiring list).

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
   Step 4a, same change surface: when a fact's source is a Sentinel
   product, fetch the product's registered BLAKE3 checksum from the
   Copernicus traceability service during materialization and carry it
   in the fact's `sources[]`, turning the URL pin into a content pin
   against the publisher's own register.
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
