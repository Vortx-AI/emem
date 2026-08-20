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

`--max-depth` bounds how far the walk descends (default 32); a directory past
it is reported by name rather than skipped quietly. `--flat` writes every
record into the top of `--output` instead of mirroring the input's shape.
`--seed-file` supplies the identity from a path, as `EMEM_AIRGAP_SEED_HEX`
does from the environment.

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

Nothing here needs Rust, a clone of this repository, or network access from
the node itself.

**Pull the images.** Both halves are published for `linux/amd64` and
`linux/arm64`; the manifest picks your architecture:

```bash
docker pull ghcr.io/vortx-ai/emem-airgap:latest   # the decoder
docker pull ghcr.io/vortx-ai/emem-encode:latest   # the encoder sidecar

docker run --rm ghcr.io/vortx-ai/emem-airgap:latest --help
```

Tags follow the server image: `:latest` and `:main` on the default branch,
`:<short-sha>` for any particular commit, and the semver forms on a release.
Pin a digest if you want the exact bytes you tested to be the exact bytes that
run:

```bash
docker pull ghcr.io/vortx-ai/emem-airgap@sha256:<digest>
```

**Or run the quickstart**, which goes from nothing to a signed, verified record
in six steps and prints every command it runs:

```bash
curl -fsSL https://raw.githubusercontent.com/Vortx-AI/emem/main/crates/emem-airgap/quickstart.sh -o quickstart.sh
less quickstart.sh          # short, and it prints every command it runs
sh quickstart.sh ./mynode
```

**Carry it across the gap.** The node itself never needs a registry. Fetch on a
connected machine, hand over a tarball:

```bash
docker pull --platform linux/arm64 ghcr.io/vortx-ai/emem-airgap:latest
docker save ghcr.io/vortx-ai/emem-airgap:latest | gzip > emem-airgap-arm64.tar.gz
sha256sum emem-airgap-arm64.tar.gz > emem-airgap-arm64.sha256

# on the node, having checked the digest you were given
sha256sum -c emem-airgap-arm64.sha256
gunzip -c emem-airgap-arm64.tar.gz | docker load
```

The quickstart skips the pull for an image that is already loaded, so it works
unchanged on a node with no route out.

**Just the binary, no container runtime.** The image is `FROM scratch` and holds
exactly one static file, so you can take it out and run it anywhere:

```bash
cid=$(docker create ghcr.io/vortx-ai/emem-airgap:latest)
docker cp "$cid:/emem-airgap" ./emem-airgap    # the encoder image holds /emem-encode
docker rm "$cid"
chmod +x ./emem-airgap && ./emem-airgap --help
```

Measured: `ELF 64-bit LSB, statically linked, stripped`. No libc, no
interpreter, no shared objects to satisfy.

The binary inside each image is named for the half it is, `/emem-airgap` or
`/emem-encode`. Both used to be `/emem-node`, which after a `docker load` left
an operator with two images that looked identical and took different flags.

**From source, with Rust**, if you would rather build what you run:

```bash
git clone https://github.com/Vortx-AI/emem
cd emem
cargo build --release -p emem-airgap
./target/release/emem-airgap --help
```

**Or build the images yourself**, from the same Dockerfile the published ones
come from:

```bash
# the decoder
docker build -f crates/emem-airgap/Dockerfile -t emem-airgap:latest .

# the encoder sidecar, same Dockerfile, one argument different
docker build --build-arg ROLE=encode -f crates/emem-airgap/Dockerfile -t emem-encode:latest .
```

Measured: decoder image 1.9 MB, encoder image 1.48 MB.

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

Measured against the published images, both arches pulled and opened:

| | download | binary inside |
| --- | --- | --- |
| `emem-airgap` arm64 | 526 KB | 973 KB, `ELF 64-bit ARM aarch64, statically linked, stripped` |
| `emem-airgap` amd64 | 594 KB | 1,260 KB, `ELF 64-bit x86-64, static-pie linked` |
| `emem-encode` arm64 | 427 KB | |
| `emem-encode` amd64 | 472 KB | |

The build context is 722 MB (the workspace's manifests and sources), which
matters only if you build rather than pull.

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

It shares the decoder's identity and **never creates one**: two halves of a
node must sign as one node, and a key minted by a sidecar would quietly split it
in two. Give it that identity one of two ways:

* `EMEM_ENCODE_SEED_HEX` (or `EMEM_AIRGAP_SEED_HEX`, the same thing), 64 hex
  characters as `emem-airgap keygen --print-seed` prints them. Nothing is
  written and **no `--data` is needed at all**, which is the only option on a
  host with no writable mount. `EMEM_ENCODE_SEED_FILE` reads the same from a
  path.
* Or run the decoder once against the same `--data` directory, which creates
  the file the sidecar then reads.

**Stream state lives in `<--out>/.state/`, not `--data`.** It used to go to the
working directory, so on a read-only rootfs the encoder wrote its trace and
then exited 1 recording where the chain had got to: work done, run reported as
failed. The traces directory is the one place an encoder is guaranteed to be
able to write. The decoder skips subdirectories when it scans for traces, so
the state costs nothing there.

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

### Which profile to capture under

This is the question that decides whether the encoder does anything useful, and
it has a short answer and a long one.

**Short: on a general-purpose Linux host with no kernel tracing, use
`host.counters.v1`.** It requires scheduler, memory, storage and network, all of
which come from `/proc` on every Linux, readable by any uid, with no mount and
no capability.

**Long:** every other trace-admitted profile in the registry requires
`sensor_bus`, `signal` or `inference`, whose only registered encodings are
`ros2.bag.v2`, `linux.ebpf.raw` and `nvidia.nsys.v1`. This encoder has no source
for any of them on any machine, so it cannot satisfy those profiles anywhere.
The one exception, `exec.trace.v1`, requires `syscall`, which needs a tracefs
mount and the capability to read it. A hosted payload with neither could
therefore produce a signed, chained, payload-binding trace that **no** profile
would admit, which is exactly what happened on a real deployment before
`host.counters.v1` existed.

You do not have to work this out by hand. Every capture reports which profiles
it satisfies:

```json
"accepted_by": ["host.counters.v1"],
"admissibility": "5 layer(s) captured, 2 absent. Accepted by: host.counters.v1.
                  NOT by --profile orbital.satellite.v1, which this capture does
                  not cover; a verifier will refuse it under that profile and be
                  right to."
```

**What `host.counters.v1` is not.** Its segments carry `linux.procfs.v1`: a
counter read at two instants, not an ordered log of what happened. That binds
the payload digests emitted in a window to a device key and a clock, and it is
real evidence that a machine was doing something. It is not evidence of *which
code ran*. If you need that, you need a profile naming `syscall`, and the host
has to grant tracefs.

### On a host that grants no kernel tracing

Most layers have a second source that needs no mount and no capability:
`/proc/schedstat`, `/proc/meminfo`, `/proc/diskstats`, `/proc/net/dev` and the
thermal zones under `/sys`. Measured on a host with tracefs unreadable and no
capabilities: **five layers captured** (thermal, scheduler, memory, storage,
network), where before there was one and no trace was written at all.

Those segments are labelled `linux.procfs.v1`, not `linux.ftrace.v1`, and the
difference is the point. ftrace gives an event log: what happened, in order.
`/proc` gives counters: totals since boot, read at two instants. A counter
delta is real evidence about a machine and worth signing; it is weaker, and a
reader has to be able to tell which one they are holding.

This changes nothing about admission. `syscall` has no unprivileged source, and
every trace-admitted profile in the registry requires it, so a `/proc`-only
trace stays inadmissible. It is simply no longer empty, and no longer dead
weight on a host that will not grant tracefs.

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
  frame_001.tif.<node8>.<when>.custody.json      one per payload, per observation
  orbit_1/band_2/scene.tif.<node8>.<when>.custody.json   the input's shape, mirrored
  join_request.<node8>.json                      carry this out to be endorsed
  run.<node8>.<when>.json                        what the run did, and every skip
```

`<when>` is `--observed-at` with the punctuation removed, so `20260821T090000Z`.

**Records carry the observation time because a second pass used to erase the
first.** Two runs at 09:00Z and 10:00Z left only the 10:00Z records, and the
run report with them: two statements about the same bytes at different times
are both true, and the node had been keeping one. Re-running with the *same*
`--observed-at` still overwrites, and should, because that is the same
statement about the same bytes.

**Every** output is keyed by the node's short key as well, because a host may
run several containers in parallel against one output mount, and two nodes may
be handed payloads with the same filename. Two payloads sharing a name are not a
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

## A host with two mounts and no writable third

The shape a flight platform usually offers: one directory uplinked to the
payload, one downlinked to the ground, a read-only rootfs, and an unprivileged
uid. No third place to keep anything.

**Do not put `--data` in the downlink directory.** `node_identity.json` holds
`seed_hex`, this node's private ed25519 seed. Pointing `--data` at the results
folder publishes the node's private key with its records, and it looks like a
working configuration until someone reads a downlink. The node refuses to
create an identity anywhere inside `--output` for exactly this reason.

Instead, generate the key on the ground and hand it over in the environment:

```bash
# on a machine you control, once per node
emem-airgap keygen --print-seed
#   EMEM_AIRGAP_SEED_HEX=<64 hex characters>   -> your secret store
#   node <52 characters>                       -> the endorser
```

```bash
docker run --rm --network none --read-only --user 65532:65532 \
  -e EMEM_AIRGAP_SEED_HEX="$SEED" \
  -v /opt/host/data:/in:ro \
  -v /opt/host/results:/out \
  ghcr.io/vortx-ai/emem-airgap:latest \
    --input /in --output /out \
    --profile orbital.satellite.v1 --platform nvidia.jetson-orin \
    --observed-at now
```

No `--data`, nothing written outside the results mount, and the private key
never touches storage. The same variable works for the encoder sidecar, which
must run as the same node.

`EMEM_AIRGAP_SEED_FILE` reads the same 64 characters from a path instead, for a
platform that can mount a secret read-only but cannot set an environment.

**What to pass for `--observed-at`.** It is signed into every record, so it is
never defaulted silently. An unattended node on a timer passes the literal
`now`, which uses the machine's clock deliberately. If the platform gives you a
better time than the payload's own clock, pass that instead. The field says
when this node saw these bytes; it does not claim the clock is right, and only
you can know that.

**Nested capture directories are walked.** A payload's signed `name` is its
path relative to `--input`, so `orbit_1/band_2/scene.tif` is distinguishable
from `orbit_2/band_2/scene.tif`. Records mirror that shape under `--output`.
If your host collects only the files at the top of the results directory, pass
`--flat` and every record lands there with its path encoded into the filename
instead.

**What the host still has to grant, and what it costs if it does not:**

| | needed for | without it |
| --- | --- | --- |
| a read-only input mount | custody | nothing works |
| a writable output mount | custody | nothing works |
| `EMEM_AIRGAP_SEED_HEX` or a writable `--data` outside `--output` | a stable identity | either a new key every run, or the private key in the downlink |
| `/sys/kernel/tracing` + `CAP_DAC_READ_SEARCH` | the `syscall` layer, and with it `exec.trace.v1` | the encoder captures scheduler, memory, storage, network and thermal from `/proc` and `/sys`, and its traces are admitted under `host.counters.v1`; no profile requiring `syscall` will take them |

That last row is the one to negotiate if you want traces admitted rather than
merely signed. Everything above it is already satisfied by a two-mount
contract.

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
