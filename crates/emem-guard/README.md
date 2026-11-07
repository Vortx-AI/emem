# emem-guard

`emem-guard` is a verdict server. It reads a transcript and returns allow or
deny. It signs each verdict and writes it to an append-only log before it
answers. It checks claims about the physical world: do the `emem:` citations
in the text still verify, and does a measurable claim cite anything at all.

You need it if you run AI agents and want a gate in front of them that you
can audit later. It is a separate binary in the [emem](../../README.md)
workspace. The responder's `/v1/guard/verdict` route uses the same crate.

Try the same engine on emem.dev first. This call needs no key and blocks
nothing:

```bash
curl -s -X POST https://emem.dev/v1/guard/verdict \
  -H 'content-type: application/json' \
  -d '{"texts":["Elevation at Mount Fuji is 3539 m per emem:fact:defi.zb592.nUkO.zEzE:qejpz7cgvk347ordd6vrdtmwzprqlzcnkmotsjton2x4y2jjbplq"]}'
```

On 7 November 2026 it returned `"action":"allow"` and `"state":"verified"` for
the one citation, with a signed receipt. The MCP tool for the same call is
`emem_guard_verdict`.

Read more: [SKILL.md](SKILL.md) is the self-host procedure, written for an
agent to run unattended. [emem.dev/guard](https://emem.dev/guard) shows each
step with its real output.

<p align="center">
  <a href="https://www.youtube.com/watch?v=ajGu5IovxIM">
    <img src="../../web/video-emem-guard.png" width="820"
         alt="Video: emem-guard. Lit paths converging across a dark hexagonal grid toward a single gate. Click to watch on YouTube." />
  </a>
</p>

## Why a separate gate

Anthropic [Inference hooks](https://platform.claude.com/docs/en/manage-claude/inference-hooks)
hold each governed prompt until a server your organisation runs returns allow
or deny. The named destinations are DLP vendors. They check content, for
example card numbers, secrets or classified markings. They do not hold signed
observations of the world, so they cannot check if a claim about a place is
still true. `emem-guard` fills that gap.

## Run your own node

```bash
cargo build --release -p emem-guard
./target/release/emem-guard                              # bare node: signs and logs, verifies no citation
./target/release/emem-guard --responder https://emem.dev # verifies citations against a corpus
```

We did not run these commands for this page, because this host has no Rust
toolchain. The flags and defaults come from `src/bin/emem-guard.rs`.

On first run the binary makes a node key, opens a log in `./var/guard` and
listens on `127.0.0.1:8080`. Use `--bind ADDR` to change the address. If you
set `PORT` and no `--bind`, the node listens on `0.0.0.0:$PORT`, which is every
interface. `emem-guard --help` lists every flag.

The examples below write `emem-guard` for `./target/release/emem-guard`.

Without `--responder` the node holds no corpus. It signs and logs every
verdict, but it verifies no citation. `GET /health` then reports
`"verifies_citations": false`.

## Checkpoints

The node has nine verdict routes into one engine. The same evidence gives the
same verdict through each route. Seven routes are open. Two are for one vendor.
`server.rs` registers 16 routes in total. The others are health, the contract
and the log reads.

| Checkpoint | Who calls it | Route |
|---|---|---|
| emem native | any agent, on any model or framework | `POST /verdict` |
| emem native, explicit name | the same handler as `POST /verdict` | `POST /verdict/native` |
| MCP tools/call | any MCP host or proxy, for a tool call or a tool result | `POST /verdict/mcp` |
| OpenAI shape | any OpenAI-compatible client, chat or moderations body | `POST /verdict/openai` |
| CloudEvents 1.0 | Knative, Dapr, Argo Events and other event meshes | `POST /verdict/cloudevent` |
| OPA-style policy point | OPA-compatible clients, Envoy external authorisation | `POST /verdict/policy` |
| Batch | many transcripts in one request, for example an archive scan | `POST /verdict/batch` |
| Anthropic Inference hooks | claude.ai, Cowork and Claude Code in a Claude Enterprise org | `POST /verdict/anthropic-hook` |
| Claude Code client hooks | agents on the Platform API, Bedrock and Vertex, which Inference hooks do not see | `POST /verdict/claude-code` |

A denial names a log leaf. Anyone can fetch it with `GET /log/entry/{leaf}`
and check the verdict without trust in the node.

`GET /.well-known/emem-guard.json` publishes the full contract: routes, deny
codes, fixes and active rules. An agent can integrate from that document
alone. A test checks that every advertised route answers.

The Anthropic Inference hooks route needs a public HTTPS host on port 443.
That platform refuses private and loopback addresses and does not follow
redirects. The open routes work on localhost.

## Read a denial

The node writes a denial for the agent that must fix it:

```text
EMEM-GUARD DENY PROV_SIG token=emem:fact:cell:cid fix=refresh_token leaf=leaf_41
```

`leaf` names the log entry. Anyone can verify that entry without the node
that issued it. `fix` tells the agent what to do next:

| `fix` | Meaning |
|---|---|
| `refresh_token` | Resolve the token again and retry with the current value. |
| `remove_reference` | Drop the citation. Nothing can make it verify. |
| `contact_admin` | A person restricted this, not the evidence. |
| `redact_and_retry` | A detection module objected to one span. Remove it and retry. |
| `cite_observation` | Resolve the observation through emem and cite the token it returns. |
| `correct_value` | The text disagrees with the cited fact. Restate the value as the fact reports it. |

## Audit the log

The node signs each verdict and appends it to the log before it answers.
Each entry chains to the entry before it. The signatures show that each
verdict is genuine. The chain shows that nobody removed a verdict.

```bash
emem-guard --audit --data ./var/guard    # exits non-zero if a verdict was changed or deleted
```

## Gate claims that cite nothing

Claim gating is off by default. When on, it denies a transcript that cites
nothing and still states a measurable quantity about a place or a time. A
unit table decides what is measurable. Each row names the band that reports
the unit, so `800 ms` and `10 MB` never match.

The crate measured this rule over its own repository prose on 6 August 2026:
3 firings in 8739 sentences. Two of the three are the rule's own positive test
fixtures (`src/claim.rs`). Measure it on your own traffic before you enforce
it:

```bash
emem-guard --claim-gating --shadow    # every rule runs and is signed; nothing is blocked
emem-guard --report                   # counts "would have blocked" from the log on disk
```

## Add your own detection

`emem-guard` does not classify content. A detection module does that, and the
node signs and logs the module's findings like its own.

```bash
emem-guard --module secret-patterns --module webhook:https://your-classifier
curl -s localhost:8080/modules      # loaded modules, their health and their cost
```

| Module declaration | What the node does |
|---|---|
| `slow` | Never runs it on the enforcing path. |
| `fast` | Demotes it to shadow if it takes more than 50 ms three times. It can then no longer block. |
| `digests_only` | Gives it an empty transcript. |

The log records the module id, version and an evidence digest. It never
records what matched. The digest of the loaded module set is part of the
verdict preimage, so each verdict names the pipeline that made it.

A third party can ship a module with a signed manifest. The operator loads it
with `--signed-module FILE` and accepts its key with `--trust-publisher KEY`.
A closed-source engine can load over a unix socket with
`--module sidecar:/run/engine.sock`, without a link against the binary.

## Check a deployment

Unit tests check the handlers. They do not check the server you deployed.

```bash
emem-guard --conformance https://your-node.example
```

This runs twelve checks over the wire and exits non-zero on any failure. The
checks are in `src/conformance.rs`. They include a large body, a replayed
request, a fetch of the logged leaf, a 1000 ms deadline and a load test. The
load test sends 96 requests from 16 threads (`LOAD_REQUESTS` is 100, and 100 / 16
rounds down to 6 per thread). The first run against this project's own node found a
9 MB body that returned 413.

## What it does not do

- It is not a DLP scanner and does not classify content.
- It never denies a citation only because the node has not cached it. That
  citation looks the same as a token from another responder. A deny would
  block legitimate agents.
- The advisory check on emem.dev (`POST /v1/guard/verdict`) blocks nothing.
  To enforce, run your own node.

## Status

The engine and the server run and have tests. Nobody has pointed them at a
live organisation yet. The next step is the conformance suite against the
platform's own failure table. The project invites no design partner before
that suite passes.

## Read more

- [SKILL.md](SKILL.md): self-host procedure, also served at
  `GET /v1/guard/selfhost` and as the MCP tool `emem_guard_selfhost`.
- [emem.dev/guard](https://emem.dev/guard): the procedure, run end to end.
- Diagrams: [nine checkpoints, one decision](https://emem.dev/docs/diagrams/40-guard-checkpoints.svg),
  [one verdict, in order](https://emem.dev/docs/diagrams/41-guard-verdict-path.svg),
  [the chassis your DLP runs on](https://emem.dev/docs/diagrams/42-guard-dlp-chassis.svg),
  [three deployments](https://emem.dev/docs/diagrams/43-guard-deployments.svg).
- Video: [watch the gate refuse a claim](https://www.youtube.com/watch?v=ajGu5IovxIM).
