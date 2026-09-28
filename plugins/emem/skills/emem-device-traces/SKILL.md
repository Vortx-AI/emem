---
name: emem-device-traces
description: Inspects and verifies the execution evidence emem holds for devices such as robots, drones, satellites and sensor hosts. Lists enrolled devices and their assurance tier, resolves an emem:trace token to its signed OS trace, re-verifies the trace (segment chain, clocks, required layers, output binding, device signature), and reads the platform registry to say which hardware roots of trust are actually admitted. Use when a fact or claim is said to come from a device, when an agent receives an emem:trace or emem:attestation token, or when someone asks whether a robot or satellite can prove what it ran. Read-only; states plainly what is roadmap.
---

# emem-device-traces

A device can do more than sign a reading. It can sign a trace of what
its operating system did while producing it: scheduler, memory, storage
and network activity in fixed windows, chained segment by segment and
bound to the outputs it emitted. emem stores these as
`emem.os_trace.v1` records and names them `emem:trace:<cid>`.

**What is live today is the verifier side.** Hardware roots of trust
(TPM, DICE, TDX, SEV-SNP, Secure Enclave, Android StrongBox and others)
are registered as candidates with provisional anchors, and admit
nothing yet. One device is enrolled, the operator's own software-only
Linux host. Do not tell a user their robot can enrol with hardware
attestation; that path ends in a rejection today.

## Who is enrolled

```sh
curl -sf -o devices.json https://emem.dev/v1/devices
jq '{count, devices: [.devices[] | {device_key, platform_id, profile_id, assurance, endorsed_by, traces}]}' devices.json
```

Recorded on 2026-09-28: `count: 1`, platform `generic.linux-host`,
profile `host.counters.v1`, assurance `operator_endorsed` by
`operator.vortx.v1`, one trace.

| `assurance` | Means |
|---|---|
| `platform_attested` | a hardware anchor vouched for the device key (none admitted yet) |
| `operator_endorsed` | a software-only platform, vouched for by an operator anchor |
| `operator_asserted` | nothing was presented; enrolled by the operator directly |

## Resolve and re-verify a trace

```sh
curl -sf -X POST https://emem.dev/v1/trace_resolve -H 'content-type: application/json' \
  -d '{"token":"emem:trace:mxyer5c2oxn4ud3xqbtnaxcxhdv4kkxiu67q32inbwxgpa7r3s6a"}' -o trace.json
jq '{resolved, profile: .trace.device.substrate_profile,
     segments: [.trace.segments[] | {seq, layer, encoding, event_count}], outputs: .trace.outputs}' trace.json
jq '{trace: .trace, profile: .trace.device.substrate_profile}' trace.json > verify_req.json
curl -sf -X POST https://emem.dev/v1/trace_verify -H 'content-type: application/json' \
  -d @verify_req.json -o verdict.json
jq '{verdict, coverage: .report.coverage, reasons: .report.reasons}' verdict.json
```

Recorded on 2026-09-28: four `linux.ftrace.v1` segments (scheduler
4744 events, memory 8382, storage 77, network 667), `outputs: []`, and
`verdict: "admit"` with all four required layers present. With one
segment's `event_count` raised by one, the verdict was `reject` with
`{"code":"chain_broken","seq":2}`: each segment commits to the one
before it, so an edited window breaks every later link.

`trace_verify` checks the schema, the profile, segment order and chain,
monotonic clocks inside the capture window, required layers, duplicate
digests, the Merkle `trace_root` over segment digests, that outputs are
bound and inside the window, and the device key's Ed25519 signature
(strict). It is stateless.

## What a verdict proves, and what it does not

- `admit` proves **some key signed an internally consistent trace**. It
  does not prove the key belongs to an enrolled or real device; check
  the key against `/v1/devices` for that.
- A trace with `outputs: []` binds no facts. Only a trace whose outputs
  name fact cids links a reading to the execution that produced it.
  None of the public traces does yet.
- Enrolment evidence (`emem.platform_attestation.v0`) is a single
  endorser signature checked against a registered anchor. Certificate
  chains, TPM quotes, measurement matching and fresh nonces are not
  parsed yet. `POST /v1/enroll_verify` reports why a piece of evidence
  is refused, and the `emem:attestation:` token it returns for refused
  evidence is computed, not stored, so it will not resolve.

## The registries

```sh
curl -sf -o platforms.json https://emem.dev/v1/device_platforms
jq '[.registry.platforms[] | {id, status, root_of_trust,
     anchors: [.trust_anchors[]? | {id, provisional}]}]' platforms.json
curl -sf -o encodings.json https://emem.dev/v1/trace_encodings
```

On 2026-09-28: 17 platforms, every hardware platform `candidate` with
provisional anchors; the one non-provisional anchor was
`operator.vortx.v1` on `generic.linux-host` (`software_only`). And 8
trace encodings (`linux.ftrace.v1`,
`linux.ebpf.raw`, `linux.hwmon.v1`, `zephyr.ctf.v1`, `ros2.bag.v2`,
`android.perfetto.v1`, `nvidia.nsys.v1`, `apple.os_signpost.v1`).
Each registry is content-addressed (`manifest_cid`), so cite the cid
when you describe what was admitted on a given date.

For the design and the roadmap, see `docs/robots.md` in the emem
repository. For a fact a device produced, verify its receipt with
[`emem-verify-receipt`](../emem-verify-receipt/SKILL.md) as for any
other fact.
