# emem-guard

*A yes/no gate for claims about the world. Part of [emem](../../README.md);
this page is its own front door, because it is a separate product with a
separate decision to make about it.*

<p align="center">
  <a href="https://www.youtube.com/watch?v=ajGu5IovxIM">
    <img src="web/video-emem-guard.png" width="820"
         alt="Video: emem-guard. Lit paths converging across a dark hexagonal grid toward a single gate. Click to watch on YouTube." />
  </a>
</p>

<p align="center"><a href="https://www.youtube.com/watch?v=ajGu5IovxIM"><b>Watch the gate refuse a claim</b></a></p>

Anthropic's [Inference hooks](https://platform.claude.com/docs/en/manage-claude/inference-hooks) hold every governed prompt for an allow or deny verdict from a server your organisation runs, before the model sees it. The named destinations are DLP vendors, and they all evaluate content: does this text carry a card number, a secret, a classified marking. None of them can evaluate whether a claim about the physical world still holds, because none of them hold signed observations of it.

`emem-guard` is that server. Input: a transcript. Output: allow or deny, signed, logged, with a reason an agent can act on.

```bash
cargo build --release -p emem-guard
./target/release/emem-guard          # generates a key, opens a log, serves
```

It answers nine checkpoints from one engine, and the same evidence gives the same verdict through every one. **Seven of the nine belong to no vendor**, which is the point: a gate reachable only through one company's product is a gate for that company's customers.

| Checkpoint | Reaches | Route |
|---|---|---|
| emem native | any agent, on any model, through any framework | `POST /verdict` |
| MCP tools/call | any MCP host or proxy, gating a tool call **or a tool result** | `POST /verdict/mcp` |
| OpenAI-shaped | anything holding an OpenAI-compatible client | `POST /verdict/openai` |
| CloudEvents 1.0 | Knative, Dapr, Argo Events, any eventing mesh | `POST /verdict/cloudevent` |
| OPA-style policy point | OPA-compatible clients, Envoy external authorisation | `POST /verdict/policy` |
| Batch | many transcripts at once, for scanning an archive offline | `POST /verdict/batch` |
| Log read | anyone checking a verdict without trusting the node that issued it | `GET /log/entry/{leaf}` |
| Anthropic Inference hooks | claude.ai, Cowork, Claude Code in a Claude Enterprise org | `POST /verdict/anthropic-hook` |
| Claude Code client hooks | agents on the Platform API, Bedrock and Vertex, which Inference hooks cannot see | `POST /verdict/claude-code` |

`GET /.well-known/emem-guard.json` publishes the whole contract, so a cold agent integrates without being handed a document by a person. A test asserts every route it advertises answers, and that the open ones outnumber the vendor ones.

A denial is machine-first, because the reader who can fix it is the agent:

```text
EMEM-GUARD DENY PROV_SIG token=emem:fact:cell:cid fix=refresh_token leaf=leaf_41
```

`fix` is the actionable part: `refresh_token` means re-resolve and retry, `remove_reference` means the citation cannot be made to verify, `contact_admin` means a person restricted this rather than the evidence, `cite_observation` means resolve it through emem and cite the token. `leaf` is the log entry, which anyone can verify without asking the server that issued it.

**Every verdict is signed and logged before it is returned**, and each entry chains to the one before it. Signatures alone would prove each verdict genuine; the chain is what proves none were removed. Check any log, including ours, with the binary itself:

```bash
emem-guard --audit --data ./var/guard    # exits non-zero if a verdict was altered or deleted
```

**Claim gating denies on absence, so it ships off behind a measurement rather than an opinion.** The rule fires when a transcript cites nothing at all and still asserts a measurable quantity about a place or a time. The discriminator is a unit table where every row names the band that reports it, so `800 ms` and `10 MB` never reach it: no band measures them, and a claim this node could not have verified is not one it will gate. Measured over this repository's own prose, 3 firings in 8739 sentences, two of which are the detector's own positive test fixtures. Measure it on your own traffic before enforcing:

```bash
emem-guard --claim-gating --shadow    # every rule runs and is signed; nobody is blocked
emem-guard --report                   # "would have blocked", counted off disk
```

**Bring your own detection.** emem-guard is deliberately bad at content classification and will stay that way. What it has that no detection engine ships is the half after the verdict, so a module plugs in and its findings get signed and logged like a native one:

```bash
emem-guard --module secret-patterns --module webhook:https://your-classifier
curl -s localhost:8080/modules      # what is loaded, and what it actually cost
```

Two declarations decide where a module may run, and neither is taken on trust. A module declaring `slow` never runs on the enforcing path. A module declaring `fast` that exceeds 50 ms three times is demoted and stops being able to block. A module declaring `digests_only` is handed an empty transcript rather than asked not to read it. The log records module id, version and an evidence digest, never what matched, and the loaded set's digest enters the verdict preimage so a verdict names the exact pipeline that produced it.

A third party ships a module nobody here compiled by publishing its manifest **signed**, and the operator decides whether that key counts: `--signed-module` plus `--trust-publisher`. A closed-source engine does not have to link against the binary at all, and loads over a unix socket with `--module sidecar:/run/engine.sock`.

**Check the deployment, not just the code.** `emem-guard --conformance <url>` runs twelve checks over the wire, because unit tests prove the handlers and prove nothing about the server you stood up. Its first run against this project's own node found a 9 MB body returning 413.

**What it will not do.** It is not a DLP scanner and does not classify content itself. A citation this node has not cached is never a denial: that is indistinguishable from a token minted by another responder, and blocking on it would deny legitimate agents.

Diagrams: [nine doors, one decision](https://emem.dev/docs/diagrams/40-guard-checkpoints.svg) · [one verdict, in order](https://emem.dev/docs/diagrams/41-guard-verdict-path.svg) · [the chassis your DLP runs on](https://emem.dev/docs/diagrams/42-guard-dlp-chassis.svg) · [three deployments](https://emem.dev/docs/diagrams/43-guard-deployments.svg).

Walk it: [emem.dev/guard](https://emem.dev/guard) is the self-host skill run end to end with the real output of each step. Self-host guide written for an agent to run unattended: [crates/emem-guard/SKILL.md](crates/emem-guard/SKILL.md), also served at `GET /v1/guard/selfhost` and as the MCP tool `emem_guard_selfhost`.

To consult a verdict without running anything, `POST /v1/guard/verdict` on this responder answers with the same engine over the shared corpus. It is advisory and blocks nothing; the MCP tool is `emem_guard_verdict`.

**Status: the engine and the server run and are tested; they have not yet been pointed at a live organisation.** The conformance suite against the platform's own failure table is next, and no design partner is invited before it is green.

