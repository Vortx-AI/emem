# emem-airgap

An emem node for a machine with no route out.

Files arrive in one directory. The node reads them and writes signed records to
a second directory. That is the whole interface, because that is the whole
environment: a container on hardware you do not own, no network, no database,
no chance to ask a server anything.

## What it gives you

For every payload that arrives, a small signed record saying: **these bytes,
under this name, at this size, arrived here at this time, and the holder of
this key says so.** Anyone who later has the record and the payload can check
both, offline, with no account and nothing from this repository.

The payload never leaves. Only the record does, and it is roughly three orders
of magnitude smaller, which is the point on a link where bandwidth is the
expensive part.

## What is actually delivered, in one table

| You get | You do not get |
| --- | --- |
| A signed statement that these exact bytes, under this name and size, arrived at this node at this time | Any statement that the bytes are *correct*, or that the sensor was calibrated, or that the clock was right |
| A record that verifies offline against the node's published key, with no server and nothing from this repository | Encryption. Records are signed, not sealed; anyone who has one can read it |
| A digest you can check the payload against wherever the payload actually lives | The payload. It never leaves through this node |
| A stage label, so a record says which point in your pipeline it covers | Lineage between stages. See the note below, because this is the one people assume |
| A citation of an `emem.os_trace.v1` when an encoder traced the payload, and a stronger assurance sentence to match | The execution claim itself. That belongs to the trace, which you fetch and verify separately |

### The lineage gap, stated plainly

If your pipeline turns a raw capture into a corrected image and then into an
analysis product, this node can take custody at every step: run it against each
directory with its own `--stage` label. What it **cannot** do is tell you that
the analysis product came from that particular raw capture. Only your pipeline
knows that, and it does not tell us.

So a record says "this artefact was here, at this stage". It does not say "this
artefact was derived from that one". Anyone reading a set of records should not
infer a chain that nothing signed.

The one case where derivation IS attested is when an encoder traced the run:
the trace names the payload digests it emitted, and the custody record cites
the trace by content id. That is evidence rather than inference, which is why
it is the only form of it here.

## What it deliberately does not claim

A custody record is **not** an `emem.os_trace.v1` execution record, and it says
so in its own signed body:

> `custody_only: the holder of this node key states these bytes arrived under
> this name at this time. Nothing here attests how the payload was produced.`

That limit is not an oversight, it is the design. An OS trace asserts *verified
execution*: its verifier rejects an empty segment set, and the satellite
substrate profile requires eight trace layers. A decoder running on its own
captures none of them, so emitting a trace here would mean fabricating eight
layers of evidence to satisfy a schema. When the OS encoder ships on the same
machine, the payload gains a real trace and rises to attested execution.
Custody is the floor, not the ceiling.

## Build it

```bash
cargo build --release -p emem-airgap
```

One binary, about 3 MB. It compiles 42 crates where the emem server compiles
507, and none of them is a networking or database crate: there is nothing in
here that could open a socket.

## Run it

```bash
emem-airgap \
  --input  /in \
  --output /out \
  --data   /data \
  --profile  orbital.satellite.v1 \
  --platform nvidia.jetson-orin \
  --observed-at 2026-08-20T09:00:00Z
```

Every flag also reads an environment variable (`EMEM_AIRGAP_INPUT`, `_OUTPUT`,
`_DATA`, `_PROFILE`, `_PLATFORM`, `_OBSERVED_AT`, `_HWMODEL`,
`_MAX_PAYLOAD_BYTES`, `_MAX_TRACE_BYTES`, `_MAX_FILES`, `_TRACES`, `_STAGE`).
Either spelling works, with a space or an equals sign, and a flag the command
does not have is refused rather than ignored. Run `emem-airgap --help` for the
list; a test checks that this file names every flag it does.

The three caps exist because the input directory belongs to the host:
`--max-payload-bytes` (256 MiB) is the largest single payload, `--max-files`
(10,000) the most in one run, and `--max-trace-bytes` (16 MiB) the largest file
read from the traces directory. Each reports what it refused rather than
dropping it silently. `--hwmodel` is the EAT hardware-model string that goes
into the join request, defaulting to the platform id.

`--observed-at` is required and is never defaulted to the system clock. It is
signed into every record, so a node with a wrong clock stamping its own time
would be signing a false statement. It is shape-checked as RFC 3339 UTC;
whether the clock is *right* is something only you can know.

`--stage` labels what point in your pipeline these payloads sit at. It is free
text and signed. There is no fixed vocabulary because every host names its
pipeline differently, and one baked into this crate would force somebody to
mislabel theirs.

`--traces` points at a directory where an encoder on the same machine writes
`emem.os_trace.v1` records. That directory is the whole interface between the
two halves: a trace names the payload digests it emitted, so a payload the
encoder watched being produced gets that trace cited in its custody record and
the stronger `custody_with_trace` assurance. A node with no encoder points this
at nothing and keeps working; neither half needs to know the other exists.

`--data` must be on storage that survives a restart. The identity there is
created once and never regenerated: a new key orphans every record already
signed under the old one.

## Two halves, two images, one crate

```text
  [ emem-encode ]  privileged sidecar          reads /sys/kernel/tracing, /sys/class/thermal
        |                                      writes a signed emem.os_trace.v1
        |  shared volume: /traces
        v
  [ emem-airgap ]  hardened decoder            reads /in and /traces
                                               writes signed custody records to /out
```

They never talk. The folder is the whole interface: the encoder names the
payload digests it saw emitted, and a payload whose digest a trace covers gets
that trace cited in its custody record.

**Why two images rather than one with two entrypoints.** The decoder runs
`--cap-drop ALL` and its entire claim is that it cannot do anything. The
encoder needs a tracefs mount and the privilege to read it. In one image, an
operator who copies the encoder's flags onto the decoder silently throws that
claim away and nothing complains. Separate images make the postures impossible
to confuse; the crate keeps the schema, the identity file and the signing
shared, so there is no duplicated code to drift.

**Either half alone is fine.** A developer who only wants custody runs the
decoder and never builds the encoder. A developer who only wants traces runs
the encoder and points it anywhere. Neither requires the other to exist.

## Install it

Three ways in, depending on what you have.

**From this repository, with Rust:**

```bash
git clone https://github.com/Vortx-AI/emem
cd emem
cargo build --release -p emem-airgap
./target/release/emem-airgap --help
```

**As a container image, built here:**

```bash
git clone https://github.com/Vortx-AI/emem
cd emem

# the decoder
docker build -f crates/emem-airgap/Dockerfile -t emem-airgap:latest .

# the encoder sidecar, same Dockerfile, one argument different
docker build --build-arg ROLE=encode -f crates/emem-airgap/Dockerfile -t emem-encode:latest .

docker run --rm emem-airgap:latest --help
docker run --rm emem-encode:latest --help
```

Measured: decoder image 1.7 MB, encoder image 1.43 MB.

**Cross-built for an aarch64 board from an x86 laptop**, which is the usual
case when the target is a Jetson you cannot compile on. No buildx, no qemu:
the build cross-compiles rather than emulating, so plain `docker build` does
it.

```bash
docker build --build-arg TARGETARCH=arm64 \
  -f crates/emem-airgap/Dockerfile \
  -t emem-airgap:arm64 .

# Save it for a machine with no registry access
docker save emem-airgap:arm64 | gzip > emem-airgap-arm64.tar.gz

# On the target
gunzip -c emem-airgap-arm64.tar.gz | docker load
```

Building on the board itself needs no argument at all: with no `TARGETARCH`
the Dockerfile reads `uname -m` and targets the machine it is on.

Measured, both arches built and checked:

| | |
| --- | --- |
| aarch64 binary | 0.8 MB, `ELF 64-bit ARM aarch64, statically linked, stripped` |
| aarch64 image | 1.36 MB |
| x86_64 image | 1.7 MB |
| build context | 722 MB (the workspace's manifests and sources) |

Three things had to be true for the cross build to work, and each is a
one-line reason worth knowing if you change the Dockerfile: blake3 builds its
SIMD paths in C, so aarch64 uses its portable implementation instead of
needing a cross C toolchain; the link goes through `rust-lld`, because the
host's `ld` refuses aarch64 objects; and symbols are stripped by the compiler,
because the host's `strip` cannot read the result either.

The image is `FROM scratch` and holds one static binary, so the tarball is a
few megabytes rather than a few gigabytes. If your host offers a large shared
base image and a delta-upload mechanism to stay under an application size cap,
you can skip it: this is already smaller than a typical delta.

### Check it before you trust it

```bash
emem-airgap identity --data /data     # the public key, for whoever endorses you
emem-airgap verify <record> [payload] # a record, and whether it covers a file
emem-airgap verify-join <request>     # a join request, for the endorser
```

If the node ran in a container, run `identity` in one too:

```bash
docker run --rm -v ./data:/data emem-airgap:latest identity --data /data
```

The identity file is mode 600 and owned by the uid the container ran as, so a
host user reading it directly gets a permission error. That is the file
behaving correctly rather than a problem to route around: a private key
readable by every account on the host would be the actual bug. `verify` and
`verify-join` read only public records and work from either side.

`verify` exits non-zero on a bad signature or a payload that does not match, so
it drops straight into a script or a CI job. Nothing in this list needs a
network, a server, or anything from this repository beyond the binary itself.

## In a container, hardened

```bash
docker build -f crates/emem-airgap/Dockerfile -t emem-airgap .

docker run --rm \
  --network none \
  --read-only \
  --cap-drop ALL \
  --security-opt no-new-privileges \
  --user 65532:65532 \
  -v /host/in:/in:ro \
  -v /host/out:/out \
  -v /host/data:/data \
  emem-airgap \
    --input /in --output /out --data /data \
    --profile orbital.satellite.v1 \
    --platform nvidia.jetson-orin \
    --observed-at "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
```

The image is `FROM scratch` and contains exactly one file. No shell, no package
manager, no libc: nothing to pivot into if the process is ever compromised.

`--network none` is the flag that matters, and it agrees with the build rather
than being merely requested. `-v ...:/in:ro` means you do not have to take the
node's word that it leaves your input alone.

The same Dockerfile cross-builds `aarch64` for a Jetson Orin and `x86_64` for a
laptop rehearsal.

## Running the encoder sidecar

```bash
docker run --rm \
  --cap-add DAC_READ_SEARCH \
  --network none \
  -v /sys/kernel/tracing:/sys/kernel/tracing:ro \
  -v /sys/class/thermal:/sys/class/thermal:ro \
  -v /host/traces:/traces \
  -v /host/results:/payloads:ro \
  -v /host/data:/data \
  emem-encode:latest \
    --out /traces --payloads /payloads --data /data \
    --profile orbital.satellite.v1 --platform jetson-orin-nx \
    --interval 60
```

It shares the decoder's `node_identity.json` and **never creates one**: two
halves of a node must sign as one node, and a key minted by a sidecar would
quietly split it in two. Run the decoder once against the same `--data`
directory first.

Then point the decoder at the traces:

```bash
emem-airgap --input /in --output /out --data /data --traces /traces --stage L2 ...
```

### Streaming, and surviving a restart

A sidecar runs for a mission, not for one window. Add `--interval`:

```bash
emem-encode --out /traces --payloads /payloads --data /data \
            --profile orbital.satellite.v1 --platform jetson-orin-nx \
            --interval 60
```

Each window chains to the last by content id, so a dropped or reordered window
is detectable rather than invisible. Where the chain has got to is kept in
`stream_head.json` beside the identity, which is what makes the chain survive
the things that break chains: the sidecar is stopped, the container is
rescheduled, the bus browns out. On the way back it resumes the same stream.

Keyed by boot id, and that is the load-bearing part. After a **reboot** the
previous head refers to a stream this kernel never ran, so chaining to it would
assert a continuity that did not happen. A fresh boot starts a fresh stream,
which is what the verifier on the ground expects. The stale head is kept rather
than deleted, because an operator reconstructing what a device did wants it.

The head is written **after** the trace it names, and atomically. Lose power
between the two and the head still points at the older trace, so the next
window chains from there and the just-written one is left unreferenced: visible
to an operator, and better than two windows claiming the same predecessor,
which is a fork nobody could tell from tampering.

`--prev-trace <cid>` names the trace this window follows, when you are driving
the chain yourself rather than letting `--interval` do it. The streaming loop
sets it from the previous window automatically, so you only need it to resume a
stream by hand.

Without `--interval` it captures one window and exits, which is the right shape
for a task scheduler that wants to own the cadence itself.

**There is no signal handler, on purpose.** SIGTERM ends the loop between
windows; a window in flight is written atomically, so nothing half-finished is
left, and `stream_head.json` already records how many windows the stream has
produced. Progress is durable on disk whether the process exits politely or is
killed outright, so a handler would add a dependency to print something the
operator can already read. If the encoder is killed between writing a trace and
updating the head, the trace is left unreferenced and the next window chains
from the older head, which is the same behaviour as a power cut and is
described above.

### What it captures, and what it will not pretend to

Privilege is not uniform, so neither is the capture:

| Layer | Source | Needs |
| --- | --- | --- |
| thermal | `/sys/class/thermal`, `/sys/class/hwmon` | nothing |
| energy | `/sys/class/powercap`, `/sys/class/hwmon` | nothing |
| syscall, scheduler, memory, storage, network | `/sys/kernel/tracing` | a mount and the capability to read it |
| sensor_bus, signal, inference | none | see below |

Three layers this encoder has **no source for at all**, and it says so rather
than letting you hunt for a permission that does not exist. The registry's only
encoding for `sensor_bus` and `signal` is `ros2.bag.v2`, which a robotics stack
produces and reading sysfs does not; `inference` needs a profiler attached to
the workload. Emitting a segment labelled with an encoding we did not run would
be a lie about provenance inside a provenance record.

The report separates the two cases, because they are different problems:

```text
absent    energy      /sys/class/powercap is readable but exposed nothing
no source sensor_bus  the registry's only encoding for it is ros2.bag.v2 ...
```

The first is a configuration fix. The second is not.

A layer the encoder could not read is **absent from the trace** and reported
with the reason:

```text
captured  thermal, syscall, scheduler, memory
absent    energy     /sys/class/powercap is readable but exposed nothing for this layer
4 layer(s) captured, 1 absent. A substrate profile requiring an absent layer
will REFUSE this trace, and that refusal is correct.
```

That refusal is the design working. `orbital.satellite.v1` requires eight
layers; a trace carrying four will not be admitted under it, and the honest
answer is to say so rather than to invent the other four. Custody remains the
operative claim until the capture is genuinely complete.

## What lands in the output directory

```
out/
  frame_001.tif.<node8>.custody.json   one per payload, signed, verifies standalone
  join_request.<node8>.json    carry this out to be endorsed (see below)
  run.<node8>.json             what the run did, including every skip and why
```

**Every** output is keyed by the node's short key, because a host may run
several containers in parallel against one output mount, and two nodes may be
handed payloads with the same filename. Two payloads sharing a name are not a
conflict to resolve: they are different bytes that different nodes took custody
of, and both records are true. Two nodes writing a shared `run.json` meant the second silently
destroyed the first's report; keyed, they coexist, while the same node
re-running still overwrites its own.

`run.<node8>.json` also names any `.part` files left behind by a run that did
not finish. They are reported and deliberately **not** deleted: on a host with
parallel containers a temporary may belong to a node that is writing right now,
and tidying it away would corrupt a healthy write to clean up after a dead one.

`run.json` names everything that was **not** recorded and why. A run that
quietly ignored half its input would look identical to a clean one, so nothing
is skipped silently.

### A skip you should expect to see

**A payload the host is still writing is skipped, not signed.** If the decoder
runs on a timer and the host is halfway through writing `frame_002.tif`, the
node reads the file, notices it moved underneath it, and leaves it alone:

```
"skipped": [{
  "name": "frame_002.tif",
  "reason": "changed while it was being read: it grew from 30000000 to
             41000000 bytes. It was still being written, so nothing was
             signed; it will be recorded next run, once the host has
             finished with it."
}]
```

This is normal and needs no action. The next run records it. The alternative is
worse than a skip: before this check existed, a 200 MB frame written during a
run produced a perfectly valid record over the first 30 MB, reported as
`0 skipped`. That record verifies, names a real file, and its digest does not
match it, which downstream is indistinguishable from tampering.

If a file is skipped this way on **every** run, the host is writing into the
input directory continuously. Write to a temporary and rename it in, and the
node will see the whole file the first time.

## Joining the network

The node cannot enrol itself. A platform attestation is signed by the
**endorser**, not the device, so an air-gapped node has no way to produce one
about itself, and self-issuing would be the same fabrication described above.

What it can do is prove it holds its key and ask. That is `join_request.json`,
and the flow is deliberately sneakernet:

1. The node writes `join_request.json` on every run. It is self-signed, so it
   proves possession of the node key and **nothing else** — the platform and
   hardware model in it are the node's own claims about itself.
2. Carry it to a connected machine that holds your endorser key.
3. Verify the self-signature, then satisfy yourself the hardware claim is true.
   Usually that means: you installed the machine.
4. Issue an `emem.platform_attestation.v0` for that node key and POST it to
   `/v1/enroll_attested`.
5. Return the attestation to the node's input directory.

You are the trust root here, and the record says so: an enrolment admitted this
way carries `endorsed_by: operator.local.v0`, never a vendor anchor. NVIDIA has
not vouched for your board; you have. The platform stays `candidate` for
exactly that reason.

## Checking a record on the ground

Every custody record verifies on its own. In Rust:

```rust
let record: emem_airgap::Custody = serde_json::from_slice(&bytes)?;
record.verify()?;                      // genuine: signed by the node it names
assert!(record.covers(&payload_bytes)); // and it is about THIS file
```

Those are two different questions on purpose. `verify` asks whether the record
is authentic; `covers` asks whether the file in front of you is the one it
describes. A reader holding only the record can answer the first.

## Fitting a hosted-payload platform

The crate hardcodes no host, path or vendor: input, output, data directory,
profile, platform and timestamp are all flags or environment variables, so it
adapts to whatever mount points a host provides rather than assuming any.

Two properties that tend to matter on such platforms:

* **Size.** The binary is about 4 MB. Hosts commonly cap an uploaded
  application in the tens of megabytes, and some offer a large shared base
  image plus a delta upload to stay inside that. This node is smaller than a
  typical delta, so it can ship whole and skip the mechanism entirely.
* **Parallelism.** Several containers may run at once against one output mount.
  Temporary files carry the writing process's id and per-run outputs carry the
  node's short key, so nodes do not collide, and no node ever deletes another's
  work.

## What it survives, and what it does not

Built for a bus that browns out and flash that flips bits:

* Records are written to a temporary and renamed, which is atomic, and fsynced
  before and after, so a power cut leaves the old record or the new one and
  never half of one.
* Every record is read back from disk and re-verified before the run counts it.
  The node is the last party who can notice corruption while a second copy of
  the payload still exists.
* Symlinks in either directory are refused, not followed. Both directories are
  attacker-controlled when you do not own the host.
* Payloads are size-checked before being read (256 MiB default) and runs are
  capped at 10,000 files, with the overflow reported rather than dropped. The
  size check is a hint, not the limit: the read itself is bounded, so a file
  that grows after being checked costs a skip line rather than the process.
* Trace files are bounded too (`--max-trace-bytes`, 16 MiB default). The
  traces directory is written by a **separate** process, so it collects debris
  for ordinary reasons; 400 MB of it took the decoder to 383 MB resident before
  this cap existed, which on an 8 GB module shared with a GPU is a decoder that
  records no custody at all.
* A payload is only signed if it held still while it was read, and the
  descriptor that is read is checked to be the file that was inspected, so the
  path cannot be swapped for a symlink in between.
* Several containers starting together on an empty data directory agree on one
  identity. The loser of that race reads the winner's key rather than failing;
  it used to exit with "File exists", which reads as a broken disk. The
  identity is written complete (never a moment where it exists and is empty),
  fsynced with its directory, and never overwritten: replacing it would orphan
  every record already signed under it and void any endorsement issued for it.
* A flag the command does not have is refused, not ignored, with a suggestion
  when one is close. `--window-ms 300` was accepted in silence by a binary with
  no such flag: the run applied the default and reported success. On hardware
  nobody can log into, a setting that silently did not apply is worse than a
  run that refuses to start.
* `--input` and `--output` being the same directory is refused. The node would
  take custody of its own records and the growth squares: one payload became
  two records, then five, then eleven. A typo in a unit file should not fill
  the output mount.

What it does **not** do: it does not encrypt anything, it does not decide
whether your clock is right, and it does not make the payload itself
trustworthy. It records custody. Everything else is somebody else's job, and
saying which is somebody else's job is most of what this crate is for.
