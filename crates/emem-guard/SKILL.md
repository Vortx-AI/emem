---
name: selfhost-emem-guard
description: Stand up an emem-guard verdict server, check it with the conformance suite, and point any agent at it. Use it to self-host emem-guard or to add a grounding gate to an agent on any model or framework. Also use it to wire a checkpoint into Claude Code, Claude Enterprise or MCP. Or use it to run a signed allow/deny server for physical-world claims.
---

# Self-host emem-guard

You are the operator. An agent with a shell can follow this procedure
unattended, in one session. Each step gives a command to run and a check to
make. If you cannot run a step exactly as written, this file has a defect.
Report the defect. Do not improvise around it.

## What you are building

Four diagrams show the shape before the commands do:

- [nine checkpoints, one decision](https://emem.dev/docs/diagrams/40-guard-checkpoints.svg)
- [one verdict, in order](https://emem.dev/docs/diagrams/41-guard-verdict-path.svg)
- [the chassis your DLP runs on](https://emem.dev/docs/diagrams/42-guard-dlp-chassis.svg)
- [three deployments](https://emem.dev/docs/diagrams/43-guard-deployments.svg)

The server answers one question: can this prompt continue, given the claims it
makes about the physical world? It returns allow or deny and signs the verdict.
It appends the verdict to a log before it answers. Someone who does not trust
you can then verify the decision later.

It is not a DLP scanner. It does not classify content. It checks whether the
`emem:` citations in a transcript still verify. As an option, it also checks
whether measurable claims about places carry a citation at all.

## Before you start

You need:

- Rust 1.91 or newer (`rustc --version`).
- Nothing else to run it locally.

The Anthropic Enterprise checkpoint needs more. It needs a public HTTPS URL on
port 443, with a certificate that the public CA trust store accepts. That
platform refuses private, loopback and carrier-grade NAT addresses when it
connects, and it does not follow redirects. The open routes have no such
requirement: localhost is fine.

You do NOT need an emem account, an API key, or network access to emem.dev.

## Step 1: build

```bash
git clone https://github.com/Vortx-AI/emem
cd emem
cargo build --release -p emem-guard
```

Check: the build exits 0. If it fails on `ort-sys` or a GPU crate, you built
the wrong package. `-p emem-guard` pulls no inference dependencies.

## Step 2: run the tests before you trust it

```bash
cargo test --release -p emem-guard
```

Check: every test passes. The crate source holds 143 tests (counted on
7 November 2026: 123 `#[test]` and 20 `#[tokio::test]`, none ignored).
These are not smoke tests. The platform documentation names two common
signature-verification mistakes: a hash over a re-encoded body, and a secret
decoded with a URL-safe alphabet. Tests cover both and fail if either comes
back.

Do not skip this step because the build succeeded. A guard that compiles and
verifies signatures incorrectly accepts requests from anyone.

## Step 3: start it

```bash
./target/release/emem-guard --data ./var/guard
```

On the first run it makes a node key with mode 0600, opens a log, and starts
to serve. The default listen address is `127.0.0.1:8080`. The startup banner
tells you the most important fact:

```
  resolve  null
           no responder configured: citations are not verified, only logged.
```

A bare node holds no corpus. It signs and logs every verdict, but it verifies
no citation. This is correct behaviour, not a broken install. The node still
keeps a signed, append-only record of every decision.

To make it verify citations, point it at a responder that holds facts:

```bash
./target/release/emem-guard --responder https://emem.dev
```

Check: `http://localhost:8080/health` reports `"verifies_citations": true`.

## Step 4: learn the whole contract in one request

```bash
curl -s http://localhost:8080/.well-known/emem-guard.json
```

This document lists every route, deny code and remedy, the reason grammar, and
the rules that are on. To integrate, read that document; you do not need the
rest of this file. The node itself serves the document, so it always matches
the binary.

## Step 5: pick your checkpoint

A checkpoint is any place where your system stops and asks whether to continue.
The node serves nine verdict routes, and all of them use the same engine.
`POST /verdict/native` is a second name for `POST /verdict`. Two
are specific to one vendor and seven belong to nobody. That is deliberate: a
gate that only one company's product can reach is a gate for that company's
customers only.

| Route | Speaks to |
|---|---|
| `POST /verdict` | any agent, any model, any framework. The native shape. |
| `POST /verdict/native` | the same handler as `POST /verdict`, under an explicit name |
| `POST /verdict/mcp` | any MCP host or proxy that gates a tool call or a tool result |
| `POST /verdict/openai` | anything that holds an OpenAI-shaped client |
| `POST /verdict/cloudevent` | CloudEvents 1.0 producers, Knative, Dapr, Argo Events |
| `POST /verdict/policy` | OPA-compatible clients, Envoy external authorisation |
| `POST /verdict/batch` | many transcripts at once, to scan an archive |
| `POST /verdict/anthropic-hook` | claude.ai, Cowork and Claude Code inside a Claude Enterprise org |
| `POST /verdict/claude-code` | agents on the Platform API, Bedrock and Vertex, which Inference hooks cannot see |

The simplest integration works from anything that can send an HTTP request:

```http
POST /verdict HTTP/1.1
Host: localhost:8080
Content-Type: application/json

{"texts":["Elevation there is 918 m per emem:fact:defi.zb493.xuqA.zcb5f:abc123"]}
```

Check: HTTP 200 and a JSON body with `action`, `checked` and `leaf`.

### Gate MCP tool calls

MCP is an open protocol with many independent hosts, so this option works in
the most places. Put a proxy between your host and its servers. Before the
proxy dispatches `tools/call`, it sends this:

```http
POST /verdict/mcp HTTP/1.1
Host: localhost:8080
Content-Type: application/json

{"jsonrpc":"2.0","id":7,"method":"tools/call",
 "params":{"name":"read_file","arguments":{"path":"notes.md"}}}
```

On `allow: false` the response carries `result`, already in the shape of a
`CallToolResult` with `isError` set. Return it instead of running the tool.
The model then sees the denial and the remedy where it already reads errors.
Your proxy does not need to know the reason grammar.

You can also gate the other direction, which most systems miss. Send
`params.result` instead of `params.arguments`. The guard then catches a
fabricated citation when it enters the context. Without this, you catch it one
turn later, after the model has already reasoned on it.

## Step 6: verify the failure behaviour yourself

Most integrations get this step wrong. Check it. Do not assume it.

A webhook failure is **not** a deny. Your server can return a non-200, time
out, or send a body that does not parse. In each case the platform applies
your organisation's failure-handling setting instead of your verdict. If that
setting is "allow the request", the prompt goes to the model without a check.

So your server must answer 200 with a well-formed verdict in every case. This
includes requests that it does not understand. Confirm that your deployment
does this:

```http
POST /verdict/anthropic-hook HTTP/1.1
Host: localhost:8080
Content-Type: application/json

{"type":"some_event_type_that_does_not_exist_yet","request_id":"probe"}
```

Check: HTTP 200 and `{"action":"allow"}`. A 4xx or 5xx means that real traffic
will trip the circuit breaker, and then the deployment stops enforcing.

## Step 7: know what it blocks and what it does not

Read this before you turn on enforcement. The defaults are deliberate.

On by default:

- `provenance`: a cited token whose signature fails, or that resolves to
  different bytes than the transcript claims. Deny codes `PROV_SIG` and
  `PROV_BYTES`.
- `freshness`: a cited reading that has drifted past its band threshold.
  Deny code `PROV_DRIFT`.

Off by default:

- `geo_restriction`: needs your own list of cell64 zones, given with
  `--restrict-cell`. It uses exact match only, never a geocode. A fuzzy
  resolution that lands on the wrong place would block innocent work with
  confidence.
- `claim_gating`: see the next step. It denies when a citation is ABSENT.

Never a denial, whatever you configure:

- A token that this node has not cached. The node cannot tell it apart from a
  token that another responder minted. A block on it would deny honest agents
  because they cite something you have not seen.

## Step 8: measure claim gating before you enforce it

`claim_gating` is the only rule that fires on something missing, not on
something that failed. It denies a sentence that states a measurable quantity
about a place or a time when the transcript cites nothing at all.

Do not turn it on directly. First run it in shadow mode. In shadow mode the
node evaluates, signs and logs every rule, but it blocks nothing:

```bash
./target/release/emem-guard --claim-gating --shadow --data ./var/guard
```

Send it a week of your own traffic. Then read the result from disk:

```bash
./target/release/emem-guard --report --data ./var/guard
```

The report looks like this (example values, not a measurement):

```
verdicts           4210
blocked            0
would have blocked 37
fired rate         0.0088
by code            enforced / observed
  CLAIM_UNGROUNDED        0 / 37
```

`would have blocked` is the number you decide on. It comes from the same
append-only file that an auditor reads, not from a counter in memory. So the
figure you quote internally is the figure an outsider can verify. The node also
serves the same numbers live at `GET /log/report`.

When you accept the rate, remove `--shadow`.

For reference, on 6 August 2026 the detector fired on 3 of 8739 sentences in
this repository's own documentation (source: `src/claim.rs`). Two of the three
are the crate's own positive test fixtures. That is one input, not a licence.
Your traffic is different from this documentation, so measure the rate where
the gate will run.

## Step 9: plug in your own detection

emem-guard checks grounding. It does not detect content. The one built-in
module, `secret-patterns`, finds only fixed-prefix credential shapes (AWS,
GitHub, Google) and PEM private-key headers. It is a reference module, not a
DLP product. What the node adds is a
signed, hash-chained record of each verdict that anyone can check offline.

So bring your own detector. The node signs and logs a module's verdicts in the
same way as a native verdict.

```bash
./target/release/emem-guard \
  --module secret-patterns \
  --module webhook:https://your-classifier.internal/scan
```

```bash
curl -s http://localhost:8080/modules
```

Each entry in `modules` carries `id`, `latency_class`, `can_block` and `observed`.

Two declarations decide where a module can run. The node does **not** take
either declaration on trust.

Latency: a module that declares `slow` never runs on the enforcing path. The
node evaluates, signs and logs it, but it cannot block. A module that declares
`fast` gets a 50 ms budget. After three overruns the registry demotes it to
shadow, and it stays there. `/modules` then shows `"demoted": true`. The node
measures this; it does not believe the declaration. A module that misses the
deadline causes a transport failure. A transport failure gives the decision to
your failure policy. It also trips the breaker that stops all enforcement.

Data: a module that declares `digests_only` gets an empty transcript. The node
does not ask it to ignore the text. It gives the module a different slice, so
the module cannot read text. Add `--modules-no-text` to give every module an
empty transcript, whatever it declared. Use this for a relay where text must
not cross a boundary.

### Write a module

```rust
use emem_guard::{
    DataClass, LatencyClass, ModuleContext, ModuleManifest, ModuleVerdict,
    PolicyModule, DenyCode, Fix,
};

pub struct MyModule { manifest: ModuleManifest }

impl MyModule {
    pub fn new() -> Self {
        Self {
            manifest: ModuleManifest {
                id: "my-module".into(),           // lowercase, appears in the log
                version: "0.1.0".into(),
                build_hash: String::new(),        // filled by .sealed()
                latency_class: LatencyClass::Fast,
                data_class: DataClass::NeedsText,
                deny_code: DenyCode::PolicyModule,
                fix: Fix::RedactAndRetry,
                description: "what it detects, in your words".into(),
            }
            .sealed(),
        }
    }
}

impl PolicyModule for MyModule {
    fn manifest(&self) -> &ModuleManifest { &self.manifest }

    fn evaluate(&self, ctx: &ModuleContext<'_>) -> ModuleVerdict {
        for text in ctx.texts {
            if let Some(matched) = self.find(text) {
                // The evidence is HASHED here, not stored. Nothing in this
                // crate ever sees what you matched.
                return ModuleVerdict::deny("what you found", matched.as_bytes(), 0.99);
            }
        }
        ModuleVerdict::allow()
    }
}
```

Three rules apply. The type system enforces the first two, not a reviewer:

- An abstain is never a block. If your sidecar is down, or your module cannot
  decide, return `ModuleVerdict::abstain`. If your outage denied requests,
  every operator who added your control would have a less reliable system.
- The log never carries what you matched. `ModuleVerdict::deny` hashes the
  evidence in the constructor, so you cannot leak it by accident. Also keep it
  out of your `reason` string, because that string goes to the operator log.
- Declare honestly, because the node measures it. A module that declares
  `fast` and runs slow gets demoted, not trusted.

`ModuleManifest::sealed()` fills `build_hash` from the declaration that it
covers. The registry refuses a manifest whose hash does not match. So nobody
can edit a module after review to claim a different latency class.

### Ship a module that nobody here compiled

Publish the manifest with a signature. The operator decides whether your key
counts.

```json
{
  "manifest": { "id": "your-engine", "version": "1.0.0", "build_hash": "...",
                "latency_class": "slow", "data_class": "needs_text",
                "deny_code": "PolicyModule", "fix": "RedactAndRetry",
                "description": "what it detects" },
  "publisher_b32": "<your ed25519 key, base32-nopad-lowercase>",
  "signature_b32": "<ed25519 over the preimage below>"
}
```

The preimage is domain-separated and covers the DECLARED hash. It does not
cover a re-serialisation of the JSON. So you and the verifier cannot disagree
about field order:

```
0x00 || len("emem.guard.module_manifest.v1") || "emem.guard.module_manifest.v1"
0x01 || len(build_hash)                       || build_hash
```

```bash
./target/release/emem-guard --signed-module your-engine.json --trust-publisher <your key>
```

The node refuses a signed module in three cases, and it names the case:

- the signature does not verify,
- the operator has not listed the publisher,
- the signed manifest does not match the binary.

**Nothing loads without a key that the operator typed.** A valid signature from
a stranger is still a stranger.

### If your engine is closed source

Your engine does not have to link against this binary. Run it on a unix socket
and send one line of JSON each way:

```bash
./target/release/emem-guard --module sidecar:/run/your-engine.sock
```

The request and the response are byte-identical to what the webhook module
posts. You write one shape, and the node can reach it over either transport:

```json
{"text_digest":"<hex>","texts":["..."],"token_count":0,"claim_count":0}
{"outcome":"deny","reason":"policy 12","confidence":0.8}
```

`text_digest` is a keyed BLAKE3 of the transcript, under the node's
`digest.key`. Your engine can use it to match calls with the same text. It
cannot use it to test a guess of the text.

The node connects to a socket. It does not spawn a process, because a process
brings restart policy, zombie reaping and crash loops for the operator to
handle. A missing socket counts as an abstain. A socket that accepts the write
and never answers is also an abstain, after your declared budget runs out.

### What modules change in your verdicts

The config digest of the registry goes into the verdict preimage. So every
signed record names the exact module set that produced it. If you change the
loaded modules, or only their order, the digest changes. Without this, an
operator could swap detectors, and one signed record could describe two
different evaluations.

A module denial carries the code `POLICY_MODULE` and your declared `fix`. The
reason line does not get a field for the module id, because the grammar is
fixed and agents parse it. No reply carries the module id either. The native
route (`POST /verdict`) returns `action`, `reason`, `code`, `fix`, `token`,
`leaf`, `checked`, `shadowed` and `claim`, and nothing about the module. To
find which module denied, get the log entry that `leaf` names
(`GET /log/entry/{leaf}`). Its record holds `module` (`id@version`) and
`evidence_digest`. The node keys the evidence digest with its `digest.key`
before it logs it. Step 11 tells why.

## Step 10: raise the body limit

Checkpoints send transcripts without truncation, up to 10 MB. Several common
defaults are much smaller: nginx `client_max_body_size` is 1 MB, and Express
`express.json()` is 100 kB. A rejected body counts as a webhook failure. Under
allow-on-failure, an oversized prompt then goes to the model without a check.

If you are behind nginx:

```
client_max_body_size 10m;
```

Check: post a 9 MB body and confirm that you get a verdict, not a 413.

## Step 11: prove a verdict to someone who does not trust you

Every verdict names a log leaf. Get it and check it yourself:

```bash
curl -s http://localhost:8080/log/entry/leaf_0
```

You get the record, the Ed25519 signature over its preimage, the public key
that signed, and the chain link to the entry before. The preimage is a list of
length-prefixed segments, documented in `/.well-known/emem-guard.json`. The
signature covers those bytes directly, not a BLAKE3 hash of them. Any Ed25519
library can check it. The check does not ask the node anything.

Then check the whole file:

```bash
./target/release/emem-guard --audit --data ./var/guard
```

```
entries        2
bad signature  []
broken chain   []
intact         true
```

Signatures alone prove that each verdict is genuine. The chain proves that
nobody removed a verdict. If you delete a line, every remaining signature still
verifies. But the chain breaks at that point, `intact` becomes false, and the
command exits with 1.

`GET /log/head` publishes the head, so a witness or a mirror can pin it.
`GET /log/entries?start=0` lets anyone mirror the log without your help.

### What the open log holds, and what it does not

`/log/entries` has no auth. Since 12 November 2026 (record domain
`emem.guard.verdict.v4`), it holds no text that anyone can test a guess
against:

| Field | What it holds |
|---|---|
| `input_digest` | a keyed BLAKE3 of the checked input. With `request_id`, it is the replay key. |
| `evidence_digest` | a keyed BLAKE3 of what a module matched. When the node withholds `token`, a keyed BLAKE3 of the restricted cell. |
| `token` | the cited token on a denial. Absent when the token names a restricted cell, whatever the code. A `GEO_ZONE` token is the cell, and a `PROV_*` token can hold it in its address (`emem:fact:<cell>:<cid>`). |

The key is `digest.key` in the data directory. The node makes it on the first
run, with mode 0600. It is not the signing key `guard.key`. Keep it private:
anyone with the key can test guesses against the digests. You hold the key, so
you can still show which input or which cell an entry is about. Compute the
same keyed digest and compare.

Before v4, `evidence_digest` was an unkeyed BLAKE3, and a reader can guess a
short match from it. A record also held the cited token in `token`, so the
open log listed each restricted cell that a request cited. Records written
before v4 keep their v3 preimage, and `--audit` still verifies them. If such
records are on a public node, treat their digests and cells as published.

The caller still gets the token in the reason line, because the caller sent
it. A retry of the same input gets the same reason line. One exception: when
the input cites two different tokens in the same restricted cell, the logged
digest cannot tell which one the rule named. Then the retry gets `token=-`,
not a wrong token.

## Step 12: set up the vendor checkpoints, if you want them

### Alert on unverified allows

A request whose signature does not verify still gets a 200 with an allow. The
reason is in step 6: a refusal would be a transport failure. That blocks
nothing and counts against the breaker. This behaviour is correct. It is also
the one case where the guard answers without knowing who asked.

The node logs this case with a distinct marker that grep can find. You can
alert on it without parsing every allow from disk:

```
emem-guard: UNVERIFIED_ALLOW inbound signature not verified: <why>
```

A steady flow of `UNVERIFIED_ALLOW` means a wrong secret or a stranger who
talks to your endpoint. Both need a page.

### Claude Enterprise Inference hooks

An administrator sets your HTTPS URL in
the organisation configuration and presses Test connection. The test sends a
synthetic prompt with `source.application` set to `config-test`. It reports the
verdict that you returned.

The first connection test arrives **unsigned**, because the signing secret does
not exist until the first save. Accept unsigned requests until your
administrator confirms that the secret exists. Then add `--require-signature`.

### Claude Code client hooks

The Claude Code client posts hook input to `POST /verdict/claude-code`. To block, return **2xx with a
deny decision in the body**. A non-2xx response does not block. This is the
opposite of what most HTTP clients expect.

The hook input has no id for each input, so the node uses `session_id` as the
request id. A retry gets the recorded verdict only when the request id AND a
keyed digest of the input are the same. So each input in a session gets its
own verdict and its own log entry. Before 12 November 2026 the node replayed
the first verdict of a session for every later input. A clean first message
then let a restricted cell through. Input with no `session_id` gets a new id,
so it never shares a verdict.

Check: in one session, send a clean prompt and then a prompt that cites a cell
from `--restrict-cell`. The second answer must be a deny, and `GET /log/head`
must count two entries.

## Step 13: rotate the secret

On the platform side, secret rotation is an immediate cutover. But requests
signed with the previous secret keep arriving for about a minute, plus any
request already in flight. Pass `--secret` twice during the switch. A server
with one secret drops those late requests, and a dropped request is a webhook
failure.

```bash
./target/release/emem-guard --secret <old secret> --secret <new secret> --require-signature
```

When the late requests stop, restart with only the new secret.

## Step 14: run the conformance suite before you point anything at it

Each step above checks one thing that you did. The conformance suite checks the
DEPLOYMENT, over the wire, in the same way a checkpoint reaches it.

```bash
./target/release/emem-guard --conformance https://your-node
```

Part of the output looks like this:

```
  pass  reachable                                   0 ms  GET /health answered
  pass  large_body_is_accepted                     72 ms  9 MB body: 200 OK with a verdict
  pass  leaf_is_fetchable_and_verifies              1 ms  leaf_9 fetched and its signature verifies offline
  pass  holds_up_under_concurrency                 69 ms  96 requests at 16 concurrent: worst 20 ms, no failures

12 of 12 checks passed. This deployment can be relied on by a checkpoint.
```

The suite runs twelve checks (`CHECKS` in `src/conformance.rs`) and exits
non-zero on any failure. Each failure says what breaks in production. The
suite runs against a URL because it looks for problems between the code and
the deployment. Examples: a proxy that rejects a 9 MB body, a terminator that
adds 400 ms, or a node that answers 502 while its own tests pass.

The suite has found real defects. Its first run against this project's own
server got a 413 for a 9 MB body. axum applies a 2 MB default, and that
default overrode the configured 10 MB layer. The docs, the comment and the
layer all said 10 MB, but the socket returned 413. Under a fail-open policy,
every oversized transcript would then go to the model without a check.

Do not point a checkpoint at a node that has not passed this suite.

## What "done" looks like

- `cargo test --release -p emem-guard` passes.
- `/health` reports whether this node verifies citations, and you know which
  answer you have.
- An unknown event type returns 200 with an allow.
- A 9 MB body returns a verdict, not a 413.
- A tampered token in a transcript produces a deny whose reason parses as
  `EMEM-GUARD DENY <CODE> token=<token> fix=<fix> leaf=<leaf>`.
- You can get the leaf that the denial names from `/log/entry/<leaf>`, and its
  signature verifies against the key in the entry.
- `emem-guard --audit` exits 0.
- `emem-guard --conformance <your-url>` exits 0.

## The reason grammar, for agents

Denials are machine-first on purpose. The line that your agent receives is:

```
EMEM-GUARD DENY PROV_SIG token=emem:fact:cell:cid fix=refresh_token leaf=leaf_01HX
```

`fix` tells the agent what to do. It has six values (`Fix` in `src/policy.rs`):

| `fix` | What the agent does |
|---|---|
| `refresh_token` | Resolve the token again and retry. |
| `remove_reference` | The citation cannot verify. Remove it. |
| `contact_admin` | A person restricted this, not the evidence. Ask an administrator. |
| `redact_and_retry` | A detection module matched one span. Remove that content and retry. |
| `cite_observation` | Resolve the observation through emem and cite the token. |
| `correct_value` | The prose disagrees with the cited fact. Restate the figure as the fact reports it. |

`leaf` is the log entry. You can get it and verify it without asking the server
that issued it.

The grammar is fixed and will not get more fields, because agents parse it.
When a denial has more to say, the native route says it. `POST /verdict`
returns `code`, `fix`, `token` and `leaf` as separate fields. It also returns
`claim`, which names the sentence, the size of the quantity and the emem band that would
have answered it.

An agent that reads `fix` and acts on it starts to carry valid tokens. That is
the purpose of the gate.

## Use the shared corpus without hosting it

Your node holds what you give it. To also check citations against the public
corpus, point it at a responder:

```bash
./target/release/emem-guard --responder https://emem.dev
```

Your verdicts stay yours. Your key signs them, your log stores them, and they
never leave your machine. The node asks the responder one thing: whether a
fact cid resolves. It asks under a hard timeout that cannot exceed the verdict
deadline.

To see an advisory verdict without enforcement, use emem.dev. It serves the
same engine at `POST /v1/guard/verdict`, and that route blocks nothing. This is
the fastest way to see what the guard does before you build anything.

## If you get stuck

Report the step number and the exact command output. Do not work around a
failing check. Each check exists because a server that skips it appears to
enforce and does not.
