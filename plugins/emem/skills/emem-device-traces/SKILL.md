---
name: emem-device-traces
description: Reads and checks the signed activity records (OS traces) emem holds for devices such as robots, drones, satellites and sensor hosts. Lists enrolled devices and their assurance tier, and resolves an emem:trace token to its signed OS trace. Re-verifies the trace (segment chain, clocks, required layers, output binding, device signature). Reads the platform registry to say which hardware roots of trust emem admits. Use when a fact or claim is said to come from a device. Use when an agent receives an emem:trace or emem:attestation token. Also use when someone asks if a robot or satellite can prove what it ran. Read-only: it sends and receives JSON, runs nothing it receives, and says plainly what is roadmap.
---

# emem-device-traces

| | |
|---|---|
| Use it when | A fact is said to come from a device, or an agent receives an `emem:trace:` or `emem:attestation:` token. Also when someone asks if a robot can prove what it ran. |
| You need | An `emem:trace:` token, or nothing to list the devices. No key. |
| You get | The enrolled devices and their assurance tier, the signed trace record, and a verdict (`admit` or `reject`) with the reason for each failed check. |
| Calls | `GET /v1/devices`, `POST /v1/trace_resolve`, `POST /v1/trace_verify` (MCP `emem_trace_verify`), `GET /v1/device_platforms`, `GET /v1/trace_encodings`. |

> **Network use.** The calls in this skill send JSON to `https://emem.dev` and receive JSON from it. `https://emem.dev` is the service that the MCP server of the plugin connects to. An answer is saved as a JSON file only so that `jq` can print a summary of it. Nothing received is run. A trace is data: a JSON list of event counts and hashes, not a program.

A device can sign more than a reading. It can also sign a trace of
what its operating system did while it made the reading. The trace
records scheduler, memory, storage and network activity in fixed
windows. Each segment links to the one before it, and the trace binds
the outputs that the device emitted. emem stores these records as
`emem.os_trace.v1` and names them `emem:trace:<cid>`.

## What ships and what does not

The verifier side is live. The platform registry lists hardware roots of trust (TPM, DICE, TDX,
SEV-SNP, Secure Enclave, Android StrongBox and others) as candidates
with provisional anchors. They admit no device yet.

emem has one enrolled device: the operator's own software-only Linux host.
Do not tell a user that their robot can enrol with hardware
attestation. That path ends in a rejection today.

## Step 1: list the enrolled devices

```sh
curl -s https://emem.dev/v1/devices \
  | jq -c '{count, devices: [.devices[] | {device_key, platform_id, profile_id, assurance, endorsed_by, traces}]}'
```

On 7 November 2026 this returned `count: 1`. The device has platform
`generic.linux-host`, profile `host.counters.v1`, assurance
`operator_endorsed` by `operator.vortx.v1`, and one trace.

| `assurance` | Meaning |
|---|---|
| `platform_attested` | A hardware anchor vouched for the device key. No device has this tier yet. |
| `operator_endorsed` | A software-only platform. An operator anchor vouched for the device key. |
| `operator_asserted` | The device presented no evidence. The operator enrolled it directly. |

## Step 2: resolve a trace token

```sh
curl -s -X POST https://emem.dev/v1/trace_resolve -H 'content-type: application/json' \
  -d '{"token":"emem:trace:mxyer5c2oxn4ud3xqbtnaxcxhdv4kkxiu67q32inbwxgpa7r3s6a"}' -o trace.json
jq -c '{resolved, profile: .trace.device.substrate_profile, outputs: .trace.outputs}' trace.json
jq -c '.trace.segments[] | {seq, layer, encoding, event_count}' trace.json
```

On 7 November 2026 the token resolved to profile `host.counters.v1`
with `outputs: []` and four `linux.ftrace.v1` segments:

| `seq` | `layer` | `event_count` |
|---|---|---|
| 0 | `scheduler` | 4744 |
| 1 | `memory` | 8382 |
| 2 | `storage` | 77 |
| 3 | `network` | 667 |

The answer also has `trace_cid`, `prev_trace_cid` and a `reverify`
string that names the call for step 3.

## Step 3: re-verify the trace

Send the record back with its profile. `jq` builds the body from the
saved answer:

```sh
jq -c '{trace: .trace, profile: .trace.device.substrate_profile}' trace.json \
  | curl -s -X POST https://emem.dev/v1/trace_verify -H 'content-type: application/json' -d @- \
  | jq '{verdict, coverage: .report.coverage, reasons: .report.reasons}'
```

On 7 November 2026 this returned `verdict: "admit"`, all four
required layers in `coverage.present`, and no reasons.

Over MCP, call `emem_trace_verify` with the same object as its
arguments. The plugin's MCP server does not list this tool, but
`tools/call` runs it by name. `emem_tools` returns its schema. No MCP
tool lists devices or resolves a trace token, so use REST for steps 1
and 2.

To see a failure, change one count before you send the record:

```sh
jq -c '{trace: (.trace | .segments[2].event_count += 1), profile: .trace.device.substrate_profile}' trace.json \
  | curl -s -X POST https://emem.dev/v1/trace_verify -H 'content-type: application/json' -d @- \
  | jq -c '{verdict, reasons: .report.reasons}'
```

On 7 November 2026 this returned `verdict: "reject"` with
`{"code":"chain_broken","seq":3}`. Each segment commits to the digest
of the segment before it. An edit to segment 2 changes its digest, so
the link in segment 3 breaks.

`trace_verify` is stateless. It checks these items:

- the schema and the profile
- the segment order and the digest chain
- clocks that increase inside the capture window
- the required layers, and duplicate digests
- the Merkle `trace_root` over the segment digests
- that the trace binds each output inside the window
- the Ed25519 signature of the device key (strict)

The report names each failed check, for example `chain_broken`,
`missing_layer`, `output_unbound` or `signature_invalid`.

## What a verdict proves, and what it does not

| Claim | Proved by `admit` |
|---|---|
| Some key signed a trace that is internally consistent. | Yes. |
| The key belongs to an enrolled or real device. | No. Compare the key with `device_key` in `/v1/devices`. |
| A fact came from this execution. | Only if `outputs` names that fact's cid. A trace with `outputs: []` binds no facts. No public trace binds an output yet. |

Enrolment evidence (`emem.platform_attestation.v0`) is one endorser
signature, checked against a registered anchor. emem does not yet
parse certificate chains, TPM quotes, measurement matches or fresh
nonces. `POST /v1/enroll_verify` checks a piece of evidence and does
not store it. It returns HTTP 200 for any JSON body.

| Evidence | Answer |
|---|---|
| Parses as `emem.platform_attestation.v0` | `verdict`, a `report` with the reasons, `attestation_cid` and an `emem:attestation:` token. emem does not store the evidence, so the token does not resolve. |
| Does not parse, or names an unknown platform | `verdict: "reject"` and an `error` string. No token. |

```sh
curl -s -X POST https://emem.dev/v1/enroll_verify -H 'content-type: application/json' \
  -d '{"platform":"nvidia.jetson-orin","attestation":{"schema":"emem.platform_attestation.v0"}}'
```

On 7 November 2026 this returned `verdict: "reject"` with the error
``missing field `eat_profile` ``.

## Step 4: read the registries

```sh
curl -s https://emem.dev/v1/device_platforms \
  | jq -c '{manifest_cid, platforms: [.registry.platforms[] | {id, status, root_of_trust, anchors: [.trust_anchors[]? | {id, provisional}]}]}'
curl -s https://emem.dev/v1/trace_encodings | jq -c '{manifest_cid, ids: [.registry.encodings[].id]}'
```

On 7 November 2026 the platform registry had 17 platforms, all with
status `candidate`. The only anchor that was not provisional was
`operator.vortx.v1` on `generic.linux-host` (`software_only`). Its
`manifest_cid` was
`gi6lw3ekbpd7soeperslxjgeljl6c3h7q6ngdcykfp2yucpcbsya`.

The encoding registry had 8 encodings: `linux.ftrace.v1`,
`linux.ebpf.raw`, `linux.hwmon.v1`, `zephyr.ctf.v1`, `ros2.bag.v2`,
`android.perfetto.v1`, `nvidia.nsys.v1` and `apple.os_signpost.v1`.
Its `manifest_cid` was
`fmyxah55fjr2vfzmktjdghutg37g2qdt2zmu7ub2dtgtpn64otga`.

Each registry is content-addressed. When you say what the registry admitted on
a date, cite the `manifest_cid` of that date.

## Related

For the design and the roadmap, see `docs/robots.md` in the emem
repository. For a fact that a device made, verify its receipt with
[`emem-verify-receipt`](../emem-verify-receipt/SKILL.md), as for any
other fact.
