# emem-airgap

An emem node for a machine with no network. Files arrive in one directory. The
node signs a small custody record for each file and writes the records to a
second directory. It opens no socket and uses no database.

| | |
| --- | --- |
| Who needs it | Operators of hardware with no route out. Examples: a satellite payload, a robot, a field Jetson, a container on a host you do not own |
| What ships | Two static binaries in one crate, `emem-airgap` (the decoder) and `emem-encode` (the optional trace encoder), and two container images |
| Version | 2.4.5 (the workspace version in the root `Cargo.toml`) |
| Needs | Nothing on the node. Docker to use the images, or Rust 1.91 to build from source |

## Try it in one command

On a machine with Docker, this script goes from nothing to a signed and
verified record in six steps. It prints each command before it runs it.

```bash
curl -fsSL https://raw.githubusercontent.com/Vortx-AI/emem/main/crates/emem-airgap/quickstart.sh -o quickstart.sh
less quickstart.sh
sh quickstart.sh ./mynode
```

We did not run the quickstart for this page, because the host had no Docker.
We ran the `emem-airgap` and `emem-encode` binaries directly on 7 November 2026.
We took them from the published `latest` images (linux/amd64). We did not run
any `docker` command on this page.

We ran the quickstart steps with these binaries and the script defaults.
The defaults are `exec.trace.v1` and `generic.linux-host`. On a host with no
readable `/sys/kernel/tracing`, the record verifies, but it cites no trace.
The `exec.trace.v1` profile requires the `syscall` layer. The run report lists
the trace under `refused_traces` and names the missing layer. To see a cited
trace on such a host, set the profile first:

```bash
EMEM_AIRGAP_PROFILE=host.counters.v1 sh quickstart.sh ./mynode
```

[Which profile to capture under](#which-profile-to-capture-under) gives the
reason.

## Read more

- [What a custody record proves](#what-a-custody-record-proves) and what it does not.
- [Run the decoder](#run-the-decoder) and [check a record](#check-a-record).
- [Add the encoder sidecar](#add-the-encoder-sidecar) for execution traces.
- [docs/emem-in-depth.md](../../docs/emem-in-depth.md) puts this crate next to the other emem parts.

## What a custody record proves

The node writes one record per payload. The record says that these bytes,
under this name and size, arrived here at this time. The holder of the node
key signs it. Anyone with the record and the payload can check both offline.
They need no account and nothing from this repository.

The payload stays where it is. Only the record leaves. A record is under 1 KB
for any payload size: in the run below, two records and the run report took
1,517 bytes. On a
slow link, this is the important part.

| You get | You do not get |
| --- | --- |
| A signed statement that these exact bytes, under this name and size, arrived at this node at this time | A statement that the bytes are correct, that the sensor was calibrated, or that the clock was right |
| A record that verifies offline against the node's public key | Encryption. The node signs records but does not seal them. Anyone with a record can read it |
| A digest to check the payload against, wherever the payload is | The payload. It never leaves through this node |
| A stage label (`--stage`) that says which point of your pipeline the record covers | Lineage between stages. See [The lineage gap](#the-lineage-gap) |
| A citation of an `emem.os_trace.v1` record when an encoder traced the payload | The execution claim itself. That belongs to the trace. Fetch the trace and verify it separately |

Each record states its limit in its own signed body:

> `custody_only: the holder of this node key states these bytes arrived under
> this name at this time. Nothing here attests how the payload was produced,
> and this is NOT an emem.os_trace.v1 execution record.`

A custody record is not an `emem.os_trace.v1` execution record. An OS trace
claims verified execution. Its verifier rejects an empty segment set. The
`orbital.satellite.v1` profile requires eight trace layers. The decoder alone
captures none of them. To write a trace here, it would have to invent eight
layers of evidence. When the encoder runs on the same machine, a payload gets a
real trace and the record cites it. Custody is the minimum claim, not the
maximum.

### The lineage gap

A pipeline can turn a raw capture into a corrected image, then into an analysis
product. The node can take custody at each step. Run it on each directory with
its own `--stage` label. The node cannot tell you that the analysis product
came from that raw capture. Only your pipeline knows that.

So a record says "this artefact was here, at this stage". It does not say "this
artefact came from that one". Do not infer a chain that nobody signed.

There is one case where a record carries evidence of derivation. An encoder
traced the run, and the trace names the payload digests that the run wrote.
The custody record then cites the trace by content id.

## Run the decoder

You can use a release binary, an image, or a source build. See
[Install it](#install-it). This run uses three local directories:

```bash
mkdir -p in/orbit_1 out data
cp frame_001.tif in/                # 30,000 bytes
echo hello > in/orbit_1/scene.tif   # 6 bytes, in a subdirectory
emem-airgap \
  --input  in \
  --output out \
  --data   data \
  --profile  orbital.satellite.v1 \
  --platform nvidia.jetson-orin \
  --observed-at 2026-11-07T09:00:00Z
```

On 7 November 2026 this printed:

```text
emem-airgap  new node identity ibed2l3i written to data/node_identity.json
emem-airgap  node ibed2l3iisilexjdw3juf7scpzkerwp2utby74scvdwrrry3i5ta
  2 recorded (0 citing an encoder trace), 0 skipped, 30006 bytes read, 1517 bytes written
```

The decoder writes the summary to stderr and the run report as JSON to stdout.

### Required flags

| Flag | Environment variable | What it sets |
| --- | --- | --- |
| `--input` | `EMEM_AIRGAP_INPUT` | The directory the payloads arrive in |
| `--output` | `EMEM_AIRGAP_OUTPUT` | The directory the records go to |
| `--profile` | `EMEM_AIRGAP_PROFILE` | The substrate profile id from the registry, for example `orbital.satellite.v1` |
| `--platform` | `EMEM_AIRGAP_PLATFORM` | The device platform id, for example `nvidia.jetson-orin` |
| `--observed-at` | `EMEM_AIRGAP_OBSERVED_AT` | The time to sign into every record: `YYYY-MM-DDTHH:MM:SSZ`, or the word `now` |

The node signs `--observed-at` into every record, so it never takes the time
from the system clock by default. A node with a wrong clock would then sign a
false statement. The node checks only the shape of the value. It accepts exactly
`YYYY-MM-DDTHH:MM:SSZ`, with no fractional seconds and no offset. A value such
as `yesterday` stops the run. An unattended node on a timer passes the literal
`now`, which uses the machine clock on purpose. If the platform gives you a
better time than the payload's own clock, pass that time. Only you can know if
the clock is right.

### Optional flags

| Flag | Environment variable | Default | What it does |
| --- | --- | --- | --- |
| `--data` | `EMEM_AIRGAP_DATA` | `.` | The directory for `node_identity.json` |
| `--traces` | `EMEM_AIRGAP_TRACES` | none | The directory where an encoder writes `emem.os_trace.v1` records |
| `--stage` | `EMEM_AIRGAP_STAGE` | none | Free text, signed. The point in your pipeline these payloads are at |
| `--hwmodel` | `EMEM_AIRGAP_HWMODEL` | the platform id | The EAT hardware-model string in the join request |
| `--max-payload-bytes` | `EMEM_AIRGAP_MAX_PAYLOAD_BYTES` | 256 MiB | The largest payload the node reads |
| `--max-files` | `EMEM_AIRGAP_MAX_FILES` | 10,000 | The most payloads in one run |
| `--max-trace-bytes` | `EMEM_AIRGAP_MAX_TRACE_BYTES` | 16 MiB | The largest file the node reads from `--traces` |
| `--max-depth` | `EMEM_AIRGAP_MAX_DEPTH` | 32 | How deep the walk goes into `--input` |
| `--flat` | `EMEM_AIRGAP_FLAT=1` | off | Write every record into the top of `--output`, with the path in the filename |
| `--seed-file` | `EMEM_AIRGAP_SEED_FILE` | none | Accepted, but the code does not read its value. Use `EMEM_AIRGAP_SEED_FILE` to read the node seed from a file |

Each flag takes its value after a space or an equals sign. The node refuses a
flag that it does not have, and suggests a close match if there is one. Run
`emem-airgap --help` for the full list. A unit test checks that this page names
every flag that the decoder and the encoder accept.

The three caps protect the node from the host, which owns the input directory.
The run report names each file that a cap refused. A directory deeper than
`--max-depth` also appears in the report by name.

`--stage` has no fixed vocabulary. Each host names its pipeline differently,
and a fixed list would force someone to mislabel theirs.

`--data` must be on storage that survives a restart. The node creates the
identity once and never makes a new one. A new key orphans every record signed
under the old key.

### What lands in the output directory

The run above wrote these files:

```text
out/
  frame_001.tif.ibed2l3i.20261107T090000Z.custody.json         one per payload, per observation
  orbit_1/scene.tif.ibed2l3i.20261107T090000Z.custody.json     the input's directories, mirrored
  join_request.ibed2l3i.json                                   carry this out to get it endorsed
  run.ibed2l3i.20261107T090000Z.json                           what the run did, and every skip
```

`ibed2l3i` is the node's short key. `20261107T090000Z` is `--observed-at`
without its punctuation.

The observation time is in each filename so that a second run does not erase
the first. Two runs at 09:00Z and 10:00Z make two true statements about the
same bytes. A rerun with the same `--observed-at` overwrites its own records,
because it makes the same statement.

The short key is in each filename because a host can run several containers
against one output mount. Two nodes can receive payloads with the same name.
Both records are true, and both stay.

The run report names each file that the node did not record, and why. It also
names any `.part` files that an unfinished run left behind at the top of
`--output`. It does not look in subdirectories. The node does not
delete them. On a host with parallel containers, a `.part` file can belong to a
node that is writing now.

A record looks like this:

```json
{
  "schema": "emem.custody.v1",
  "node": {
    "node_key": "ibed2l3iisilexjdw3juf7scpzkerwp2utby74scvdwrrry3i5ta",
    "profile": "orbital.satellite.v1",
    "platform": "nvidia.jetson-orin"
  },
  "name": "frame_001.tif",
  "payload_digest": "jrlkbw6ra4jrtbjgocevpzpymvnm33mvfmlh4bwgz24zt4fm65ia",
  "size_bytes": 30000,
  "observed_at": "2026-11-07T09:00:00Z",
  "assurance": "custody_only: the holder of this node key states ...",
  "signature": "zr5nvnf3lmgxz4zv65xjyhpcc235i52m746jrq6gfcczm4bde6mfowe7i7yleto3tul3ye3iyjmadukkrhcjdmgj37uwgg2cdpejsaa"
}
```

### A file that is still being written

The node signs a payload only if the file did not change while the node read
it. If the host is still writing `frame_002.tif`, the run report shows this:

```text
"skipped": [{
  "name": "frame_002.tif",
  "reason": "changed while it was being read: it grew from 30000000 to
             41000000 bytes. It was still being written, so nothing was
             signed; it will be recorded next run, once the host has
             finished with it."
}]
```

This is normal. The next run records the file. Before this check, a 200 MB
frame written during a run got a valid record over its first 30 MB, and the run
reported `0 skipped`. That record verified, named a real file, and had a digest
that did not match the file. Downstream, that looks the same as tampering.

If the node skips a file on every run, the host writes into the input directory
all the time. Make the host write to a temporary file and rename it into place.

## Check a record

These commands need only the binary. They need no network and no server.

```bash
emem-airgap identity --data data
emem-airgap verify out/frame_001.tif.ibed2l3i.20261107T090000Z.custody.json in/frame_001.tif
emem-airgap verify-join out/join_request.ibed2l3i.json
```

`identity` prints the node's public key for the endorser. `verify` checks a
record, and the file it covers. `verify-join` checks a join request. Put your
own node's short key in place of `ibed2l3i`, and your own `--observed-at`
time in place of `20261107T090000Z`. Give `verify` the full record
name. A later run writes a second record for the same file, and a `*` pattern
then matches both.

On 7 November 2026, `verify` printed this and exited 0:

```text
signature  VALID, signed by ibed2l3iisilexjdw3juf7scpzkerwp2utby74scvdwrrry3i5ta
name       frame_001.tif
...
payload    MATCHES in/frame_001.tif
```

With a different file, it printed `payload    DOES NOT MATCH` and exited 1. A
bad signature prints `signature  INVALID` and also exits 1. So `verify` can
gate a script or a CI job. You can leave out the payload argument
to check only the signature.

The identity file is mode 600 and belongs to the user that created it. If the
node ran in a container, run `identity` in a container too:

```bash
docker run --rm -v ./data:/data ghcr.io/vortx-ai/emem-airgap:latest identity --data /data
```

A permission error from the host side is correct behaviour. A private key that
every account can read would be the real fault. `verify` and `verify-join` read
only public records, so they work from either side.

In Rust, the library gives the same two checks:

```rust
let record: emem_airgap::Custody = serde_json::from_slice(&bytes)?;
record.verify()?;                       // signed by the node it names
assert!(record.covers(&payload_bytes)); // and it is about this file
```

`verify` asks if the record is genuine. `covers` asks if the file in front of
you is the file the record describes. A reader with only the record can answer
the first question.

## Install it

The node needs no Rust, no clone of this repository and no network access.

### Pull the images

The images are on GitHub Container Registry for `linux/amd64` and
`linux/arm64`. Docker picks your architecture.

```bash
docker pull ghcr.io/vortx-ai/emem-airgap:latest   # the decoder
docker pull ghcr.io/vortx-ai/emem-encode:latest   # the encoder sidecar
docker run --rm ghcr.io/vortx-ai/emem-airgap:latest --help
```

The tags follow the server image: `:latest` and `:main` on the default branch,
`:<short-sha>` for one commit, and semver tags on a release. To run the exact
bytes you tested, pin a digest:

```bash
docker pull ghcr.io/vortx-ai/emem-airgap@sha256:<digest>
```

We read the published `latest` images from the registry on 7 November 2026:

| Image | Architecture | Download | Binary inside |
| --- | --- | --- | --- |
| `emem-airgap` | arm64 | 574 KB | 1,066 KB, `ELF 64-bit ARM aarch64, statically linked, stripped` |
| `emem-airgap` | amd64 | 642 KB | 1,365 KB, `ELF 64-bit x86-64, static-pie linked` |
| `emem-encode` | arm64 | 511 KB | 932 KB, `ELF 64-bit ARM aarch64, statically linked, stripped` |
| `emem-encode` | amd64 | 558 KB | 1,168 KB, `ELF 64-bit x86-64, static-pie linked` |

Each image is `FROM scratch` and holds one static binary: `/emem-airgap` or
`/emem-encode`. There is no shell, no package manager and no libc.

### Take the image across the gap

Pull on a connected machine, then carry a tarball to the node:

```bash
docker pull --platform linux/arm64 ghcr.io/vortx-ai/emem-airgap:latest
docker save ghcr.io/vortx-ai/emem-airgap:latest | gzip > emem-airgap-arm64.tar.gz
sha256sum emem-airgap-arm64.tar.gz > emem-airgap-arm64.sha256

# on the node, after you check the digest you received
sha256sum -c emem-airgap-arm64.sha256
gunzip -c emem-airgap-arm64.tar.gz | docker load
```

The quickstart does not pull an image that is already loaded. So it also works
on a node with no network.

### Use only the binary

You can copy the binary out of the image and run it with no container runtime:

```bash
cid=$(docker create ghcr.io/vortx-ai/emem-airgap:latest)
docker cp "$cid:/emem-airgap" ./emem-airgap    # the encoder image holds /emem-encode
docker rm "$cid"
chmod +x ./emem-airgap && ./emem-airgap --help
```

### Build from source

```bash
git clone https://github.com/Vortx-AI/emem
cd emem
cargo build --release -p emem-airgap
./target/release/emem-airgap --help
```

This builds both binaries. The crate has nine direct dependencies. None of them
is an HTTP server, an async runtime or a database (`Cargo.toml`). We did not
run `cargo` for this page.

### Build the images yourself

The published images come from this Dockerfile:

```bash
# the decoder
docker build -f crates/emem-airgap/Dockerfile -t emem-airgap:latest .

# the encoder sidecar: the same Dockerfile, one argument different
docker build --build-arg ROLE=encode -f crates/emem-airgap/Dockerfile -t emem-encode:latest .
```

To build for an aarch64 board such as a Jetson on an x86 laptop, add
`--build-arg TARGETARCH=arm64`. The build cross-compiles, so you need no buildx
and no qemu. On the board itself, leave out `TARGETARCH`. The Dockerfile then
reads `uname -m`.

If you change the Dockerfile, keep three things true for the cross build:

1. blake3 uses its portable code on aarch64, so the build needs no C cross
   toolchain for its SIMD paths.
2. The link uses `rust-lld`, because the host `ld` refuses aarch64 objects.
3. The compiler strips symbols, because the host `strip` cannot read the
   result.

The build context is the workspace manifests and sources. It matters only if
you build instead of pull.

## Run it hardened in a container

```bash
docker run --rm \
  --network none \
  --read-only \
  --cap-drop ALL \
  --security-opt no-new-privileges \
  --user 65532:65532 \
  -v /host/in:/in:ro \
  -v /host/out:/out \
  -v /host/data:/data \
  ghcr.io/vortx-ai/emem-airgap:latest \
    --input /in --output /out --data /data \
    --profile orbital.satellite.v1 \
    --platform nvidia.jetson-orin \
    --observed-at now
```

`--network none` is the important flag, and the build agrees with it. The
`:ro` input mount means that you do not have to trust the node to leave your
input alone.

## A host with two mounts and no third

A flight platform often gives a payload one uplinked directory, one downlinked
directory, a read-only root file system and an unprivileged user. There is no
third place to keep a key.

Do not put `--data` in the downlink directory. `node_identity.json` holds
`seed_hex`, the node's private Ed25519 seed. That configuration seems to work
until someone reads a downlink. The node refuses to create an identity inside
`--output` for this reason.

Instead, make the key on the ground and give it to the node in the environment:

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

This run needs no `--data` and writes nothing outside the results mount. The
private key never touches storage. The encoder sidecar reads the same variable,
because it must sign as the same node. `EMEM_AIRGAP_SEED_FILE` reads the seed
from a path, for a platform that can mount a secret but cannot set an
environment variable.

The node walks nested capture directories. The signed `name` of a payload is
its path relative to `--input`, so `orbit_1/band_2/scene.tif` and
`orbit_2/band_2/scene.tif` stay different. If your host collects only the files
at the top of the results directory, pass `--flat`.

What the host must still give, and the cost if it does not:

| The host gives | Needed for | Without it |
| --- | --- | --- |
| A read-only input mount | custody | Nothing works |
| A writable output mount | custody | Nothing works |
| `EMEM_AIRGAP_SEED_HEX`, or a writable `--data` outside `--output` | a stable identity | A new key on each run, or the private key in the downlink |
| `/sys/kernel/tracing` and `CAP_DAC_READ_SEARCH` | the `syscall` layer, and with it `exec.trace.v1` | The encoder still reads scheduler, memory, storage, network and thermal from `/proc` and `/sys`. Its traces then pass `orbital.counters.v1` and `host.counters.v1`, but no profile that requires `syscall` |

Negotiate the last row if you want traces that more profiles admit. A
two-mount contract already gives everything above it.

Two more points matter on a hosted-payload platform:

- Size. Each binary is under 1.4 MB. Hosts often cap an uploaded application at
  tens of megabytes. Some offer a large shared base image and delta uploads to
  stay under the cap. This node is smaller than a typical delta, so it can ship
  whole.
- Parallel runs. Temporary files carry the process id, and per-run outputs
  carry the node's short key. Nodes do not collide, and no node deletes the
  work of another.

## Join the network

The node cannot enrol itself. The endorser signs a platform attestation, not
the device. So an air-gapped node cannot make one about itself.

The node can prove that it holds its key, and ask. That is the join request.
The flow uses removable media on purpose:

1. On each run, the node writes `join_request.<node8>.json`. The node signs it
   with its own key. It proves that the node holds the key and nothing more.
   The platform and hardware model in it are claims that the node makes about
   itself.
2. Carry the request to a connected machine that holds your endorser key.
3. Run `emem-airgap verify-join` on it. Then make sure that the hardware claim
   is true. Usually you know because you installed the machine.
4. Issue an `emem.platform_attestation.v0` for that node key. POST it to
   `/v1/enroll_attested`.
5. Put the attestation back in the node's input directory.

You are the trust root here, and the record says so. An enrolment admitted this
way carries `endorsed_by: operator.local.v0`, never a vendor anchor. The vendor
did not vouch for your board. You did. For that reason the platform stays
`candidate`.

## Add the encoder sidecar

The encoder `emem-encode` reads what the kernel shows it, signs an
`emem.os_trace.v1` trace over that, and writes the trace to a directory. The
decoder reads the same directory with `--traces`. The two processes never talk
to each other.

```text
  [ emem-encode ]  privileged sidecar     reads /proc, /sys and, if allowed, /sys/kernel/tracing
        |                                 writes a signed emem.os_trace.v1
        |  shared directory: /traces
        v
  [ emem-airgap ]  hardened decoder       reads /in and /traces
                                          writes signed custody records to /out
```

A trace names the payload digests that the run wrote in its window. When a
trace covers a payload's digest, the custody record cites that trace and
carries the stronger `custody_with_trace` assurance.

You can use either half alone. A developer who wants only custody runs the
decoder. A developer who wants only traces runs the encoder.

### Why two images

The decoder runs with `--cap-drop ALL`, and its claim is that it can do almost
nothing. The encoder needs a tracefs mount and the privilege to read it. With
one image, an operator could copy the encoder's flags onto the decoder and lose
the decoder's claim with no warning. Two images keep the two postures apart.
The two binaries share the schema, the identity file and the signing code from
one crate, so they duplicate no code.

### Run the encoder once

This ran on 7 November 2026 on a host with no readable tracefs. It used the
identity that the decoder made in `data`:

```bash
mkdir -p traces
emem-encode --out traces --payloads in --data data \
            --profile host.counters.v1 --platform nvidia.jetson-orin
```

```text
emem-encode  node ibed2l3i
  trace frgwzvs4mrmbqhkjlqpnr4ldo23tetojppsybsuyfxcq3jgwlqxa
  captured  thermal, scheduler, memory, storage, network
  absent    energy      /sys/class/powercap is readable but exposed nothing for this layer
  absent    syscall     /sys/kernel/tracing exists but is not readable by uid 1000. ...
  no source sensor_bus  no source in this encoder: the registry's only encoding for it is ros2.bag.v2, ...
  no source signal      no source in this encoder: as above, ros2.bag.v2 is the only registered encoding
  no source inference   no source in this encoder: registered encodings are linux.ebpf.raw and nvidia.nsys.v1, ...
  outputs   2 payload digest(s) bound
  5 layer(s) captured, 2 absent. Accepted by: orbital.counters.v1, host.counters.v1.
```

Then point the decoder at the traces, with the same profile and platform:

```bash
emem-airgap --input in --output out --data data \
  --profile host.counters.v1 --platform nvidia.jetson-orin \
  --observed-at now --traces traces --stage L0
```

That run reported `2 recorded (2 citing an encoder trace)`.

The encoder's `--profile` and `--platform` must match the decoder's exactly. A
record cites only a trace that this node captured under the same profile on the
same platform. The run report lists any other trace under `refused_traces`,
with the mismatch. Without this check, a trace dropped into the directory could
make a node cite the evidence of another device. A node under a strict profile
could also cite a capture taken under a weaker one.

### Encoder flags

| Flag | Environment variable | What it does |
| --- | --- | --- |
| `--out` | `EMEM_ENCODE_OUT` | Required. The directory the trace goes to. The decoder reads it with `--traces` |
| `--profile` | `EMEM_ENCODE_PROFILE` | Required. Must match the decoder |
| `--platform` | `EMEM_ENCODE_PLATFORM` | Required. Must match the decoder |
| `--payloads` | `EMEM_ENCODE_PAYLOADS` | The files this window wrote. The trace binds their digests |
| `--data` | `EMEM_ENCODE_DATA` | The directory that holds the decoder's `node_identity.json`. Default `.` |
| `--interval` | `EMEM_ENCODE_INTERVAL` | Capture every N seconds instead of once |
| `--prev-trace` | `EMEM_ENCODE_PREV_TRACE` | The trace this window follows, to resume a chain by hand |
| `--max-depth` | `EMEM_ENCODE_MAX_DEPTH` | How deep to walk `--payloads`. Default 32, the same walk as the decoder |

The encoder uses the same `--max-depth` default and walk as the decoder. So a
nested capture tree gets custody and a trace citation. The encoder lists any
file that it did not walk under `payloads_skipped`, with the reason. Before
the two walks matched, nested payloads got custody but no citation, and the
traced count still looked correct.

The encoder shares the decoder's identity and never creates one. Two halves of
a node must sign as one node. Give the encoder the identity in one of two ways:

- Set `EMEM_ENCODE_SEED_HEX` (or `EMEM_AIRGAP_SEED_HEX`) to the 64 hex
  characters from `emem-airgap keygen --print-seed`. The encoder then needs no
  `--data`. `EMEM_ENCODE_SEED_FILE` reads the seed from a path.
- Or run the decoder once with the same `--data`. That creates the file the
  encoder reads.

### Run the encoder as a sidecar

This command is for Docker. We did not run it, because the host had no Docker.

```bash
docker run --rm \
  --cap-add DAC_READ_SEARCH \
  --network none \
  -v /sys/kernel/tracing:/sys/kernel/tracing:ro \
  -v /sys/class/thermal:/sys/class/thermal:ro \
  -v /host/traces:/traces \
  -v /host/results:/payloads:ro \
  -v /host/data:/data \
  ghcr.io/vortx-ai/emem-encode:latest \
    --out /traces --payloads /payloads --data /data \
    --profile orbital.counters.v1 --platform nvidia.jetson-orin \
    --interval 60
```

The decoder that reads `/host/traces` must then use
`--profile orbital.counters.v1` too.

### Capture on a timer and survive restarts

With `--interval`, the encoder captures one window every N seconds. Each window
chains to the last by content id, so you can detect a dropped or reordered
window. Without `--interval`, it captures one window and exits. Use that form
when a task scheduler owns the timing.

The encoder keeps the chain head in `<--out>/.state/stream_head.json`. It does
not use `--data`, because on a read-only host the traces directory is the one
place the encoder can always write. The decoder does not read
subdirectories of `--traces`, so the state file does not get in its way.

The head lets a chain survive a stopped sidecar, a rescheduled container or a
brown-out. On restart, the encoder continues the same stream.

The head carries the boot id. After a reboot, the old head names a stream that
this kernel never ran. A chain to it would claim a continuity that did not
happen. So a new boot starts a new stream, which is what the ground verifier
expects. The encoder keeps the old head file, because an operator who
reconstructs the device history wants it.

The encoder writes the head after the trace that it names, and atomically. If
power fails between the two writes, the head still names the older trace. The
next window then chains from there, and the new trace stays unreferenced. An
operator can see that. Two windows with the same predecessor would be a fork
that nobody could tell from tampering.

The encoder has no signal handler, on purpose. SIGTERM stops the loop between
windows. The encoder writes each window atomically, so it leaves nothing half
finished. `stream_head.json` already records how many windows the stream made.
A kill between a trace and its head update has the same result as a power cut,
described above.

### Which profile to capture under

On an orbital payload, use `orbital.counters.v1`. On any other Linux host with
no kernel tracing, use `host.counters.v1`. Both require four layers: scheduler,
memory, storage and network. Each comes from `/proc` on every Linux system.
The encoder reads eight files: `/proc/schedstat`, `/proc/loadavg`,
`/proc/stat`, `/proc/meminfo`, `/proc/vmstat`, `/proc/diskstats`,
`/proc/net/dev` and `/proc/net/snmp`. Any user can read them, with no mount and no extra privilege.

The registry (`crates/emem-core/data/substrates-v0.json`) explains why other
profiles do not work here. `exec.trace.v1` requires `syscall`, and only
kernel tracing gives that layer. Every other profile that takes a trace requires
`sensor_bus`, `signal` or `inference`. This encoder has no source for any of them:

| Profile | Required layers | Can this encoder satisfy it |
| --- | --- | --- |
| `orbital.counters.v1` | scheduler, memory, storage, network | Yes, from `/proc` |
| `host.counters.v1` | scheduler, memory, storage, network | Yes, from `/proc` |
| `exec.trace.v1` | syscall, scheduler, memory | Only with a tracefs mount and `CAP_DAC_READ_SEARCH` |
| `orbital.satellite.v1` | syscall, scheduler, memory, sensor_bus, signal, energy, thermal, storage | No. This encoder has no source for `sensor_bus` or `signal` |

Two profile names exist for one evidence level because the node signs the
profile into every record, and nobody can correct it later. The profile must be
true about what the node is and about what evidence it made.
`orbital.counters.v1` copies each substrate field from `orbital.satellite.v1`.
It differs in three places. Its required layers are the four an orbital node
can produce. Its `evidence_layers` is `[trace]`, not `[trace, signal]`. Its `provenance_class` is `direct_sensor`, not
`attested_execution`, because counters do not attest execution. A spacecraft
that used `host.counters.v1` would sign records that say they came from a
generic host.

You do not have to work this out by hand. Each capture prints the profiles it
satisfies. It also says so when it does not satisfy the `--profile` you gave.

### What the encoder captures

| Layer | Source | Needs |
| --- | --- | --- |
| thermal | `/sys/class/thermal`, `/sys/class/hwmon` | nothing |
| energy | `/sys/class/powercap`, `/sys/class/hwmon` | nothing |
| scheduler, memory, storage, network | `/proc` counters (`linux.procfs.v1`), or `/sys/kernel/tracing` events (`linux.ftrace.v1`) | nothing for `/proc`; a mount and `CAP_DAC_READ_SEARCH` for tracefs |
| syscall | `/sys/kernel/tracing` | a mount and `CAP_DAC_READ_SEARCH` |
| sensor_bus, signal, inference | none | see below |

The label on each segment tells a reader which kind of evidence it holds.
`linux.ftrace.v1` is an event log: what happened, in order. `linux.procfs.v1` is
a counter read at two instants: totals since boot. A counter delta is real
evidence that a machine did work in that window. It binds the payload digests
to a device key and a clock. It is weaker than an event log. It does
not show which code ran. For that, you need a profile that requires `syscall`,
and the host must grant tracefs.

The encoder has no source for three layers. The only registered encoding for
`sensor_bus` and `signal` is `ros2.bag.v2`, which a robotics stack writes.
`inference` needs a profiler attached to the workload (`linux.ebpf.raw` or
`nvidia.nsys.v1`). A segment labelled with an encoding the encoder did not run
would be a false statement inside a provenance record.

The report keeps two cases apart, because they need different fixes:

- `absent`: the encoder could not read the layer on this host. This is a
  configuration problem, and the report says what to grant.
- `no source`: no source for the layer exists in this encoder. No permission
  fixes it.

A layer the encoder could not read is absent from the trace. A profile that
requires that layer will refuse the trace, and that refusal is correct. Custody
stays the claim that applies until the capture is complete.

## What it survives, and what it does not

The design expects a power bus that browns out and flash memory that flips
bits:

- The node writes each record to a temporary file and renames it, with fsync
  before and after. A power cut leaves the old record or the new one, never
  half of one.
- The node reads each record back from disk and verifies it again before the
  run counts it. The node is the last party that can see corruption while a
  second copy of the payload still exists.
- The node refuses symlinks in both directories. On a host you do not own, an
  attacker can control both.
- The node checks the size before it reads a payload, and the read itself has a
  bound. A file that grows after the check costs a skip line, not the process.
- The node also caps trace files (`--max-trace-bytes`). A separate process
  writes the traces directory, so other files collect there. Before this cap,
  400 MB of such files took the decoder to 383 MB resident memory. On an 8 GB
  module that shares memory with a GPU, that can stop custody completely.
- The node signs a payload only if it did not change during the read. The node
  also checks that the descriptor it reads is the file it inspected, so nobody
  can swap the path for a symlink in between.
- Several containers that start together on an empty data directory agree on
  one identity. The loser of that race reads the winner's key. The node writes
  the identity complete, fsyncs it and its directory, and never overwrites it.
- The node refuses a flag that it does not have. An older binary accepted
  `--window-ms 300` with no such flag, applied the default and reported
  success. On hardware nobody can log into, a setting that silently did not
  apply is worse than a refusal.
- The node refuses the same directory for `--input` and `--output`. Otherwise
  it takes custody of its own records, and the count grows each run: one
  payload became two records, then five, then eleven. The node makes this
  check after it writes `join_request.<node8>.json` into `--output`. If you
  give one directory twice, delete that join request from it before the next
  run. Otherwise the next correct run records the join request as a payload.
  We saw this on 7 November 2026.

The node does not encrypt anything. It does not decide if your clock is right.
It does not make the payload trustworthy. It records custody, and it says which
claims belong to other parts of the system.
