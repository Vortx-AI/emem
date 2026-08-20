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
`_DATA`, `_PROFILE`, `_PLATFORM`, `_OBSERVED_AT`, `_MAX_PAYLOAD_BYTES`,
`_MAX_FILES`). Run `emem-airgap --help` for the current list; that output is
the source of truth if this file ever falls behind it.

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

## What lands in the output directory

```
out/
  frame_001.tif.custody.json   one per payload, signed, verifies standalone
  join_request.<node8>.json    carry this out to be endorsed (see below)
  run.<node8>.json             what the run did, including every skip and why
```

The per-run files are keyed by the node's short key rather than named
`run.json`, because a host may run several containers in parallel against one
output mount. Two nodes writing a shared `run.json` meant the second silently
destroyed the first's report; keyed, they coexist, while the same node
re-running still overwrites its own.

`run.<node8>.json` also names any `.part` files left behind by a run that did
not finish. They are reported and deliberately **not** deleted: on a host with
parallel containers a temporary may belong to a node that is writing right now,
and tidying it away would corrupt a healthy write to clean up after a dead one.

`run.json` names everything that was **not** recorded and why. A run that
quietly ignored half its input would look identical to a clean one, so nothing
is skipped silently.

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
  capped at 10,000 files, with the overflow reported rather than dropped.

What it does **not** do: it does not encrypt anything, it does not decide
whether your clock is right, and it does not make the payload itself
trustworthy. It records custody. Everything else is somebody else's job, and
saying which is somebody else's job is most of what this crate is for.
