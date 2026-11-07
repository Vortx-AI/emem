# Encoder substrates: the trust layer for devices that observe the world

This page is a plan and a record. It describes how emem admits output from
devices such as satellites, drones, robots and microscopes. Some steps ship.
Some steps are open. The status table below says which. The checks on this
page ran against the code and against https://emem.dev on 7 November 2026.
The protocol owner wrote the first version on 26 July 2026.

The architecture on one page, with the gaps labelled:
![Three columns. Orbit: the open archive (Sentinel-1, Sentinel-2, Cop-DEM, CHIRPS), admitted because anyone can recompute it. Operator satellites, a candidate profile, must ship a signed execution trace. emem: a trust gate that admits device output only inside a complete trace, and the shared memory of cells, signed facts and tokens. Ground: robot fleets, agent swarms and an auditor who verifies years later. A panel lists the gaps that do not ship yet. The first is that self-service enrollment refuses every call today.](../diagrams/39-orbit-to-ground-trust.svg)

The short summary of open items is in the substrates section of
[docs/roadmap.md](../roadmap.md#next-substrates-and-the-address-space-that-gates-all-of-them).

## Status on 7 November 2026

| Part | Status | Where |
|---|---|---|
| Substrate profile registry, 18 profiles | Ships | `crates/emem-core/data/substrates-v0.json`, `GET /v1/substrates`, MCP `emem_substrates` |
| Device-platform registry, 17 platforms in 7 families | Ships | `crates/emem-core/data/device-platforms-v0.json`, `GET /v1/device_platforms` |
| `emem.os_trace.v1` record and its signature preimage | Ships | `crates/emem-trace/src/schema.rs`, `os_trace_preimage_v1` in `emem-attest` |
| Trace verifier, 17 reject reasons | Ships | `crates/emem-trace/src/verify.rs` |
| Drift score function | Ships as a library function | `crates/emem-trace/src/drift.rs` |
| Write-path gate | Ships | `put_attestation_gated` in `emem-storage`; `POST /v1/attest` and `POST /v1/attest_traced` call it |
| Stateless trace check | Ships | `POST /v1/trace_verify`, MCP `emem_trace_verify` |
| Trace tokens `emem:trace:` and `emem:attestation:` | Ship | `emem-trace::token`, `POST /v1/trace_resolve` |
| Enrollment | Ships, admits through an operator endorsement only | `POST /v1/enroll_attested`, `POST /v1/enroll_verify` |
| Opt-in device roster | Ships | `GET /v1/devices`, `POST /v1/device_publish` |
| Conformance vectors, 4 | Ship | `spec/test_vectors/os_trace/` |
| A published vendor trust anchor | Open | every vendor anchor in the registry is `provisional` |
| Live check of the enrollment nonce | Open | the attestation signs the nonce, but no code checks it against a live challenge yet (`crates/emem-trace/src/enroll.rs`) |
| Drift-anchor wiring on admitted facts | Open | only `emem-trace` and `crates/emem-primitives/examples/satellite_downlink.rs` call `DriftAnchorCheck` |
| Copernicus BLAKE3 checksum in `sources[]` | Open | `cdse.traceability.blake3` is only a declared lineage key |
| Traces in the transparency log | Open | the storage code appends no trace leaves to the Merkle log |
| `emem-cli trace verify` | Open | `emem-cli` has no trace subcommand |
| os-trace section in `docs/protocol.md` | Open | no such section yet |
| Fleet-memory example with traces | Open | `examples/fleet-memory/` does not build traces |

## The admission rule

emem accepts each device as an observer of the physical world. It does not
believe any device on its word. emem admits a device's output only when two
conditions are true:

1. The complete OS execution trace of the device binds the output.
2. That trace verifies against the substrate profile of the device.

This rule adds a trust layer. It does not add a new substrate. The founding
Earth substrate keeps its own rule: emem must be able to recompute each fact
from free public archives. The Earth substrate also gets a second job. It is
the drift anchor that emem scores each device claim against.

The two checks close two different gaps:

| Check | Question it answers |
|---|---|
| OS trace | Did this capture run on this device, over this window? |
| Drift anchor | Does the value agree with a record of the same place that anyone can recompute? |

A third party can do both checks and trust neither the device nor the server.

## Why emem asks for an OS trace and not only the output

A sensor reading is a claim. Anyone can sign a claim. A key proves who spoke,
not what happened.

A commodity device can give stronger evidence without special hardware: its
own execution record over the capture window. That record holds the syscall
stream, scheduler activity, memory, energy, thermal readings, bus traffic,
network and storage activity, and the model weights that ran.

To forge a value, an attacker must then forge a full, consistent execution.
The record chains the segments and signs them over one root. The forgery must also pass
cross-checks between layers. An inference layer with no scheduler activity is
suspect. A sensor-bus output with no syscall around it is suspect. The drift
score against the Earth anchor is a further check.

The profiles cover a large range of grain. The live registry on 7 November 2026
goes from 0.0000001 m (`lab.microscope.v1`) to 1,000,000 m
(`observatory.telescope.v1`). Each profile states its range in `grain_min_m`
and `grain_max_m`, so a reader knows the grain of the instrument behind a fact.
The six profiles on the `entity.cid` address space have no grain.

## What the code contains

The three core parts are pure code with no I/O. They sit next to
`emem-attest` in the trust stack.

### The substrate profile registry

The code is `crates/emem-core/src/substrates.rs`. The data is
`crates/emem-core/data/substrates-v0.json`. The manifest kind is
`emem-substrates`, one of the 10 manifests in `GET /v1/manifests`.

On 7 November 2026 the registry has 18 profiles:

| Status | Admission | Address space | Profiles |
|---|---|---|---|
| active | `archive_recomputable` | `geo.cell64` | `earth.satellite.v0` (the drift anchor) |
| candidate | `os_trace_required` | `geo.cell64` | `orbital.satellite.v1`, `orbital.counters.v1`, `observatory.telescope.v1`, `lab.microscope.v1`, `urban.cctv.v1`, `mobile.handheld.v1`, `aerial.drone.v1`, `robot.fleet.v1`, `industrial.machine.v1`, `fixed.sensor.v1` |
| candidate | `custodial_retention` | `geo.cell64` | `feed.retained.v1` |
| candidate | `os_trace_required` | `entity.cid` | `space.deep.v1`, `exec.trace.v1`, `host.counters.v1` |
| candidate | `archive_recomputable` | `entity.cid` | `code.repository.v1`, `data.table.v1`, `model.checkpoint.v1` |

Each `os_trace_required` profile lists its `required_trace_layers`. The
satellite class appears twice on purpose. emem admits the public archive
because anyone can recompute it. An operator's own constellation is a device
like any other.

The registry pins a candidate profile so that device makers can build against it.
Candidate does not mean that ingest is open. A profile can be `active` only on
an address space that this build can key a fact by. Today that is
`geo.cell64` only.

The validator enforces three rules in code:

- A trace-admitted substrate cannot be a drift anchor.
- An archive substrate does not require trace layers.
- The registry always contains an active anchor.

The Earth profile names four evidence layers: `trace`, `signal`, `inference`
and `weather`. Contradiction scoring compares these layers. The split of a
change in value among its causes is still on the roadmap.

### The `emem.os_trace.v1` record

The code is `crates/emem-trace/src/schema.rs`. The record holds:

- The device identity: an attester key, platform, OS, kernel and a per-boot id.
- A capture window on the monotonic clock.
- Trace segments per layer, chained by digest. The raw logs stay on the device
  or in cold storage. The record commits to them, and an audit can ask for them.
- The digests of the emitted outputs.
- A v1 Merkle root over the chain.
- One Ed25519 signature over a domain-separated preimage
  (`os_trace_preimage_v1` in `emem-attest`). It binds the schema, device,
  profile, window, root and outputs.

emem computes `trace_cid` the same way as `fact_cid`: BLAKE3 over canonical CBOR,
in lowercase base32 with no padding.

### The verifier and the drift rule

The verifier is `crates/emem-trace/src/verify.rs`. It collects every failure
and admits a trace only when the list is empty. The `RejectReason` enum has 17
reasons, for example `chain_broken`, `missing_layer`, `clock_outside_window`,
`output_unbound` and `signature_invalid`.

The drift rule is `crates/emem-trace/src/drift.rs`. It scores a device value
against an anchor value with one-sigma uncertainty `sigma`:

- `z = |device - anchor| / (3 * sigma)`
- `score = z / (1 + z)`

A difference of three sigma scores 0.5, the boundary between consistent and
tension. Nine sigma is the boundary between tension and contradicted. A claim
in tension stays in the record and goes to the contradiction index. emem does
not drop it, because a real change in the world also looks like tension at
first.

## Check a trace on emem.dev

`POST /v1/trace_verify` takes a profile, a trace and an optional payload
digest. It returns the full report and does not store anything. This command
sends the committed `admit` vector:

```sh
python3 -c "import json; print(json.dumps(json.load(open('spec/test_vectors/os_trace/trace.os_trace.admit.0001.json'))['input']))" \
  | curl -s -X POST https://emem.dev/v1/trace_verify \
      -H 'content-type: application/json' --data-binary @- \
  | jq '.report | {verdict, trace_cid, reasons, missing: .coverage.missing}'
```

On 7 November 2026 the answer was:

```json
{
  "verdict": "admit",
  "trace_cid": "umplvo2hpglpnmu3qhw6g4jiujdhwjmp6bdjzmnnywm3plp5t5cq",
  "reasons": [],
  "missing": []
}
```

All four vectors gave the verdict, reason codes and `trace_cid` that their
`expected` block states:

| Vector | Profile | Verdict | Reason codes |
|---|---|---|---|
| `admit.0001` | `robot.fleet.v1` | admit | none |
| `chain_broken.0002` | `robot.fleet.v1` | reject | `chain_broken` (at `seq` 3) |
| `output_unbound.0003` | `robot.fleet.v1` | reject | `output_unbound` |
| `archive_refused.0004` | `earth.satellite.v0` | reject | `admission_not_trace_based` |

The MCP tool `emem_trace_verify` does the same check.

## What the open archives give (checked on 26 July 2026)

The direct sensors of the Earth substrate are Sentinel-1 and Sentinel-2. No
public satellite archive publishes the execution trace of its ground
processing. The design does not claim otherwise.

The archives publish declared processing lineage. The protocol owner checked
live Planetary Computer STAC items on 26 July 2026. This page did not repeat
that check.

- Sentinel-2 L2A items carry `s2:processing_baseline` (the value was `05.12`),
  `s2:generation_time`, `s2:datastrip_id`, `s2:granule_id` and
  `s2:product_uri`. The product URI names the SAFE product. Its
  `manifest.safe` records the processing facility and the software history.
- Sentinel-1 items carry `s1:processing_datetime`, `s1:orbit_source` (`RESORB`
  on the checked item), `s1:instrument_configuration_ID`,
  `s1:processing_level` and the `sat:` orbit block.
- The [Copernicus Data Space traceability
  service](https://dataspace.copernicus.eu/analyse/traceability) registers a
  BLAKE3 checksum for each Sentinel product. You can check a downloaded
  product against this register. emem uses BLAKE3 too.

This has three results for the design:

1. `earth.satellite.v0` keeps the `archive_recomputable` rule. The evidence of
   the archive is that anyone can fetch it again, plus the integrity register
   of the publisher. A SAFE manifest is not an execution trace. The
   `declared_lineage` field of the profile lists 11 lineage keys, so a reader
   knows what is checkable for each fact.
2. Today a fact cites its source by URL, id and capture time, not by a content
   hash. emem can fetch the registered checksum during materialization and put
   it in `sources[]`. That makes the URL pin a content pin. This is step 4a
   below and is open.
3. Lineage of the processing is a cause in change attribution. A change of
   `processing_baseline` can move a value while the ground stays the same.
   Baseline changes have shifted Sentinel-2 radiometry before. The lineage
   keys belong in the evidence that `emem_change_attribution` returns.

## Standards to align with

| Standard | Part of this design it touches |
|---|---|
| IETF RATS architecture (RFC 9334) | Roles: the device is the Attester, the verifier is the Verifier, the write gate is the Relying Party. The os_trace record is RATS Evidence. Hardware Evidence (TPM 2.0 quotes, DICE certificates) can later be one more segment layer. The device-platform registry uses these roles. |
| in-toto attestations and SLSA provenance | Same shape as an os_trace: subject is the payload digest, predicate is the execution evidence. An export from `emem.os_trace.v1` to an in-toto statement is a possible next step. |
| W3C PROV and ISO 19115 lineage | `declared_lineage` keys can map to PROV activities and ISO lineage steps in an export path. They stay out of the wire record. |
| STAC processing extension | `processing:software` and `processing:lineage` are the standard place for the lineage that Sentinel items carry in their own keys. The `StacPc` connector already reads STAC. |
| CTF, the Common Trace Format (LTTng, barectf, Zephyr) | Segment `encoding` strings name it (`zephyr.ctf.v1`). `ros2_tracing` writes LTTng/CTF sessions on robots, so an edge encoder can cut an existing session into segments. |
| eBPF, ftrace, rosbag2 | Linux capture methods behind the encodings, and the ROS 2 sensor-bus layer. |

Public data to test against:

- Live Sentinel STAC items (Planetary Computer, already a connector) and the
  Copernicus traceability register.
- LTTng example CTF traces and `ros2_tracing` sample sessions.
- The DARPA Transparent Computing and OpTC datasets: large, labelled OS
  provenance traces. They can test whether the layer cross-checks catch
  execution stories that do not fit together.
- The repo's own `spec/test_vectors/os_trace/` vectors.

## Out of scope

- Hardware roots of trust inside the trace. The OS trace rule is what a
  commodity device can meet today. A hardware quote can later be one more
  segment layer.
- A stream transport for traces. Each record covers one window. A device that is
  offline signs its windows and sends them later.
- Truth of the value. A valid trace does not make the value true. That is the
  job of the drift anchor. The two checks stay separate on purpose.

## Steps for maintainers to connect the layer

Each step is one change that a reviewer can check.

1. **Expose the registry.** Ships. `GET /v1/substrates` returns the registry and
   its manifest CID. The MCP tool `emem_substrates` is in the `robotics` and
   `satellites` bundles.
2. **Gate the write path.** Ships. `trace_gate::TraceGate` in `emem-storage`
   holds the enrollment tree: attester key to profile, for
   `os_trace_required` profiles only. `put_attestation_gated` applies the
   rule. An enrolled key must send a trace that verifies against its profile.
   The trace must bind the payload digest of each primary fact
   (`emem_trace::payload_digest_of_value`). If not, emem stores nothing and
   returns the full reason list. An enrolled key can write traced primary
   observations only. The gate refuses derivative facts, absences and edges
   from it, because no traced-derivation rule exists yet. A key that nobody
   enrolled keeps the ungated path. The gate stores each admitted trace by
   `trace_cid` with an audit edge from fact to trace. `POST /v1/attest` and
   `POST /v1/attest_traced` both call the gate.
   Open: append admitted traces to the transparency log, so that inclusion
   proofs cover them.
3. **Stateless checks.** Ships: `POST /v1/trace_verify`, `emem_trace_verify`
   and `POST /v1/enroll_verify`. Open: an `emem-cli trace verify`
   subcommand for a fully offline check.
4. **Drift-anchor wiring.** Open. For each admitted device fact, recall the
   anchor band at the cell of the fact and compute `DriftAnchorCheck`. Write
   the result to the contradiction index. Show it through
   `emem_memory_contradictions` and the change-attribution ledger.
   Step 4a is in the same change. It applies when the source of a fact is a
   Sentinel product. During materialization, fetch the registered BLAKE3
   checksum from the Copernicus traceability service. Put it in `sources[]`.
5. **Token grammar.** Ships. `emem:trace:<trace_cid>` composes and parses in
   `emem-trace::token`. `POST /v1/trace_resolve` resolves `emem:trace:` and
   `emem:attestation:` tokens. Open: bundle membership for trace tokens.
6. **Upgrade the robotics example.** Open. Change `examples/fleet-memory/` so
   that each vendor's writer builds and signs an `emem.os_trace.v1` window
   around its observations (`OsTrace::build_and_sign_v1`). The reader then
   checks the trace before it trusts the map update. After that, add a
   ROS 2 capture profile (`ros2.bag.v2` encoding) and an eBPF capture profile
   for Linux devices.
7. **Conformance.** Ships: the four vectors in `spec/test_vectors/os_trace/`
   (admit, chain broken, output unbound, archive refused). The `emem-trace`
   vector test replays them in CI. Open: the os-trace section in
   `docs/protocol.md` and the threat-model update in the whitepaper.

## Steps for a device maker

1. **Pick the profile.** Read `GET /v1/substrates` or
   `crates/emem-core/data/substrates-v0.json`. Find the profile for your
   class of machine. If none fits, propose a new entry in a pull request. The
   validator applies the admission rules to it.
2. **Make the device key.** Run `emem-cli keygen` to make one Ed25519 key pair
   per device. The public key names the device in each record. Keep the
   secret safe.
3. **Capture the required layers.** The profile lists them. On Linux, ftrace or
   eBPF covers syscall, scheduler and memory. RAPL or a power rail covers
   energy. hwmon covers thermal. The bus capture depends on your instrument.
   Cut each layer into segments with monotonic-clock extents. Put the BLAKE3
   of the raw bytes in `log_digest`. Keep the raw logs, because an audit can
   ask for them.
4. **Bind your outputs.** Before you sign, add each sensor payload as an
   `EmittedOutput`. Give its BLAKE3 digest and its emission time inside the
   window. The gate rejects an output that the trace does not bind.
5. **Build, sign and check.** `OsTrace::build_and_sign_v1` chains the segments
   and signs the preimage. Run `emem_trace::verify_os_trace` against your
   profile before you send it, or send it to `POST /v1/trace_verify`. The
   report lists every problem at once.
6. **Write and keep the tokens.** Send the attestation with the trace to
   `POST /v1/attest_traced`. On admit, keep the `emem:fact:` tokens and the
   `trace_cid`, not the payloads. Any other agent can resolve and check them.
   On reject, read the reason list.
7. **Expect a drift score, later.** Step 4 is open. When it ships, emem
   will score your claims against the Earth anchor where the bands overlap.
   Today a contradicted claim goes to the contradiction index. Traces are
   not in the transparency log yet.

## Can a satellite maker use emem today?

### Self-hosted: yes, end to end

A maker that runs the crates can do the full loop now.
`crates/emem-primitives/examples/satellite_downlink.rs` shows it:

1. Enroll a key per spacecraft under `orbital.satellite.v1`.
2. Capture the required layers on the payload computer, in CTF encodings. A
   ground-segment ingest host is an accepted first step while flight software
   integration grows.
3. Bind the downlink payload digest in the trace, sign it and write it
   through `put_attestation_gated`.
4. Keep three handles: the `emem:fact:` token, the `emem:trace:` token and the
   `emem:bundle:` handle.

One pass is one capture window. A constellation sends a stream of signed
windows. Passes made offline sync when a link is available.

### Hosted on emem.dev: reads and checks yes, self-service writes no

A maker can read `GET /v1/substrates` and `GET /v1/device_platforms`, and
check traces with `POST /v1/trace_verify` and `POST /v1/enroll_verify`.

Self-service enrollment through `POST /v1/enroll_attested` needs an
attestation signed by a whitelisted trust anchor. Every vendor anchor in
`device-platforms-v0.json` is `provisional`, and a provisional anchor
whitelists nothing. So this call refuses every vendor attestation today.

The only route in is an operator endorsement. A node operator lists the
machines that they vouch for in the file named by `EMEM_OPERATOR_ENDORSEMENTS`
(`crates/emem-core/src/device_platforms.rs`). On 7 November 2026 emem.dev has
one such anchor, `operator.vortx.v1`, on the `generic.linux-host` platform.
`GET /v1/devices` lists one device: a demo host with assurance
`operator_endorsed`, profile `host.counters.v1` and one trace. The demo writer
is `crates/emem-cli/src/bin/emem-demo-device.rs`. It claims no manufacturer
and no hardware root of trust.

A node with endorsements has a different `device_platforms` manifest CID from
the published one. That is correct, because its registry is different.

Two items must ship before a satellite maker can write on emem.dev on its
own. They are a published vendor anchor and a live check of the enrollment
nonce.

## What this layer changes

Before this layer, emem gave shared memory with one substrate. That substrate
is trustworthy because its sources are public archives. With this layer, emem
becomes a protocol that admits observers of the physical world one class at a
time. Each class meets the same standard of evidence: an execution that you
can verify, anchored to a record that you can recompute. The output is proof
of what happened. A reader can check it when nobody who was present is
available to ask.
