---
name: selfhost-emem-guard
description: Stand up an emem-guard verdict server, verify it against the conformance checks, and point any agent at it. Use when asked to self-host emem-guard, add a grounding gate to an agent on any model or framework, wire a checkpoint into Claude Code or Claude Enterprise or MCP, or run a signed allow/deny server for physical-world claims.
---

# Self-host emem-guard

You are the operator. This procedure is written to be run by an agent with a
shell, unattended, in one session. Every step is a command you can execute and
a check you can evaluate. If a step cannot be run verbatim, that is a defect in
this file: report it rather than improvising around it.

## What you are building

A server that answers one question: should this prompt proceed, given the
claims it makes about the physical world. It returns allow or deny, signs the
verdict, and appends it to a log before answering, so the decision can be
verified later by someone who does not trust you.

It is not a DLP scanner. It does not classify content. It checks whether the
`emem:` citations in a transcript still verify, and optionally whether
measurable claims about places carry a citation at all.

## Before you start

You need:

- Rust 1.91 or newer (`rustc --version`).
- Nothing else to run it locally.

For the Anthropic Enterprise checkpoint specifically you also need a publicly
reachable HTTPS URL on port 443 with a certificate that validates against the
public CA trust store, because that platform refuses private, loopback and
carrier-grade NAT ranges at connect time and does not follow redirects. The
open routes have no such requirement: localhost is fine.

You do NOT need an emem account, an API key, or network access to emem.dev.

## Step 1: build

```bash
git clone https://github.com/Vortx-AI/emem
cd emem
cargo build --release -p emem-guard
```

Check: the build exits 0. If it fails on `ort-sys` or a GPU crate, you have
built the wrong package; `-p emem-guard` pulls no inference dependencies.

## Step 2: run the tests before you trust it

```bash
cargo test --release -p emem-guard
```

Check: every test passes; there are 136 of them. These are not smoke tests. They include the two
signature-verification mistakes the platform documentation singles out as the
most common (hashing a re-encoded body, and decoding the secret with a
URL-safe alphabet), and they fail loudly if either is reintroduced.

Do not skip this step because the build succeeded. A guard that compiles and
mis-verifies signatures accepts requests from anyone.

## Step 3: start it

```bash
./target/release/emem-guard --data ./var/guard
```

It generates a node key at 0600 on first run, opens a log, and serves. The
startup banner tells you the one thing that matters most:

```
resolve  null
         no responder configured: citations are not verified, only logged.
```

A bare node signs and logs every verdict and verifies no citation, because it
holds no corpus. That is honest behaviour, not a broken install, and it is
still half the product: the signed, append-only record of every decision.

To make it verify, point it at a responder that holds facts:

```bash
./target/release/emem-guard --responder https://emem.dev
```

Check: `curl -s localhost:8080/health | grep verifies_citations` says `true`.

## Step 4: learn the whole contract in one request

```bash
curl -s localhost:8080/.well-known/emem-guard.json
```

Every route, every deny code, every remedy, the reason grammar, and which
rules are on. You do not need to read the rest of this file to integrate; you
need to read that document. It is served by the node itself so it cannot drift
from the binary.

## Step 5: pick your checkpoint

A checkpoint is any place your system pauses and asks whether to proceed. The
node serves nine, all reaching the same engine. **Two are vendor-specific and
seven belong to nobody**, which is deliberate: a gate reachable only through
one company's product is a gate for that company's customers.

| Route | Speaks to |
|---|---|
| `POST /verdict` | any agent, any model, any framework. The native shape. |
| `POST /verdict/mcp` | any MCP host or proxy, gating a tool call or a tool result |
| `POST /verdict/openai` | anything holding an OpenAI-shaped client |
| `POST /verdict/cloudevent` | CloudEvents 1.0 producers, Knative, Dapr, Argo Events |
| `POST /verdict/policy` | OPA-compatible clients, Envoy external authorisation |
| `POST /verdict/batch` | many transcripts at once, for scanning an archive |
| `POST /verdict/anthropic-hook` | claude.ai, Cowork and Claude Code inside a Claude Enterprise org |
| `POST /verdict/claude-code` | agents on the Platform API, Bedrock and Vertex, which Inference hooks cannot see |

The simplest possible integration, which works from anything that can make an
HTTP request:

```bash
curl -sS -X POST localhost:8080/verdict \
  -H 'content-type: application/json' \
  -d '{"texts":["Elevation there is 918 m per emem:fact:defi.zb493.xuqA.zcb5f:abc123"]}'
```

Check: HTTP 200 and a JSON body with `action`, `checked` and `leaf`.

### Gating MCP tool calls

The most portable option, because MCP is an open protocol with many
independent hosts. Put a proxy between your host and its servers, and before
dispatching `tools/call`:

```bash
curl -sS -X POST localhost:8080/verdict/mcp \
  -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":7,"method":"tools/call",
       "params":{"name":"read_file","arguments":{"path":"notes.md"}}}'
```

On `allow: false` the response carries `result`, already shaped as a
`CallToolResult` with `isError` set. Return it instead of running the tool and
the model sees the denial and the remedy in the place it already reads errors.
Your proxy does not need to know the reason grammar.

You can also gate the other direction, which is the one most systems miss.
Send `params.result` instead of `params.arguments` and a fabricated citation is
caught as it enters the context, rather than a turn later once the model has
already reasoned on it.

## Step 6: verify the failure semantics yourself

This is the step most integrations get wrong, so check it rather than assume
it.

A webhook failure is **not** a deny. If your server returns a non-200, times
out, or sends an unparseable body, a platform applies your organisation's
failure-handling setting instead of your verdict. Under "allow the request"
that means the prompt reaches the model uninspected.

So: your server must answer 200 with a well-formed verdict in every case,
including ones it does not understand. Confirm your deployment does:

```bash
curl -sS -X POST localhost:8080/verdict/anthropic-hook \
  -H 'content-type: application/json' \
  -d '{"type":"some_event_type_that_does_not_exist_yet","request_id":"probe"}'
```

Check: HTTP 200 and `{"action":"allow"}`. If you get a 4xx or 5xx, your
deployment will trip the circuit breaker under real traffic and stop enforcing
entirely.

## Step 7: understand what it will and will not block

Read this before you enable enforcement, because the defaults are deliberate.

On by default:

- `provenance`: a cited token whose signature fails, or that resolves to
  different bytes than the transcript claims. Deny codes `PROV_SIG` and
  `PROV_BYTES`.
- `freshness`: a cited reading that has drifted past its band threshold.
  Deny code `PROV_DRIFT`.

Off by default:

- `geo_restriction`: needs your own cell64 zone list, supplied with
  `--restrict-cell`. Exact match only, never a geocode, because a fuzzy
  resolution that lands on the wrong place is a confident block of innocent
  work.
- `claim_gating`: see the next step. It denies on the ABSENCE of a citation.

Never a denial, whatever you configure:

- A token this node has not cached. It is indistinguishable from a token
  minted by another responder, and blocking on it would deny honest agents for
  citing something you have not seen.

## Step 8: measure claim gating before you enforce it

`claim_gating` is the only rule that fires on something missing rather than
something failed. It denies a sentence that asserts a measurable quantity
about a place or a time when the transcript cites nothing at all.

Do not turn it straight on. Run it in shadow, where every rule is evaluated,
signed and logged, and nobody is blocked:

```bash
./target/release/emem-guard --claim-gating --shadow --data ./var/guard
```

Send it a week of your own traffic. Then read the answer off disk:

```bash
./target/release/emem-guard --report --data ./var/guard
```

```
verdicts           4210
blocked            0
would have blocked 37
fired rate         0.0088
by code            enforced / observed
  CLAIM_UNGROUNDED        0 / 37
```

`would have blocked` is the number you are deciding on. It comes from the same
append-only file an auditor reads, not from a counter in memory, so the figure
you quote internally and the figure an outsider can verify are the same figure.
The same numbers are served live at `GET /log/report`.

When the rate is one you accept, drop `--shadow`.

For reference, the detector fires on 3 sentences in 8739 across this
repository's own documentation, two of which are its own positive test
fixtures. That is one input, not a licence: your traffic
is not our changelog, which is exactly why you measure it where it will run.

## Step 9: plug in your own detection

emem-guard checks grounding and will never be good at content detection. That
race is thousands of patterns maintained by hundreds of engineers, and winning
it would add nothing here. What this node has that no detection engine ships is
the half after the verdict: a signed, hash-chained, offline-checkable record.

So bring your own. A module's verdicts get signed and logged exactly like a
native one.

```bash
./target/release/emem-guard \
  --module secret-patterns \
  --module webhook:https://your-classifier.internal/scan
```

```bash
curl -s localhost:8080/modules | jq '.modules[] | {id, latency_class, can_block, observed}'
```

Two declarations decide where a module is allowed to run, and **neither is
taken on trust**.

**Latency.** A module declaring `slow` never runs on the enforcing path at all.
It is evaluated, signed and logged, and cannot block. A module declaring `fast`
that exceeds 50 ms three times is demoted to shadow by the registry and stays
there, which shows up as `"demoted": true` at `/modules`. That is measured, not
promised: a module that misses the deadline is not slow, it is a transport
failure, and a transport failure hands the decision to your failure policy and
trips the breaker that stops enforcement for everything.

**Data.** A module declaring `digests_only` is handed an empty transcript. Not
asked politely: given a different slice, so it cannot read text. Add
`--modules-no-text` and every module gets an empty transcript regardless of what
it declared, for a relay where text may not cross a boundary.

### Writing one

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

Three rules, and the first two are enforced by the type rather than by review:

- **An abstain is never a block.** If your sidecar is down or you could not
  decide, return `ModuleVerdict::abstain`. Turning your outage into a denied
  request would make every operator running you strictly less reliable for
  having added a control.
- **The log never carries what you matched.** `ModuleVerdict::deny` hashes the
  evidence at the constructor, so you cannot leak it by accident. Keep it out of
  your `reason` string too; that goes to the operator log.
- **Declare honestly, because it is measured.** Declaring `fast` and being slow
  gets you demoted, not trusted.

`ModuleManifest::sealed()` fills `build_hash` from the declaration it covers.
The registry refuses a manifest whose hash does not match, so a module cannot be
edited after review to claim a different latency class than the one it was
reviewed under.

### Shipping a module nobody here compiled

Publish the manifest signed, and the operator decides whether your key counts.

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

The preimage is domain-separated over the DECLARED hash, not over a
re-serialisation of the JSON, so you and the verifier cannot disagree about
field order:

```
0x00 || len("emem.guard.module_manifest.v1") || "emem.guard.module_manifest.v1"
0x01 || len(build_hash)                       || build_hash
```

```bash
emem-guard --signed-module your-engine.json --trust-publisher <your key>
```

Three ways it is refused, and each says which: a signature that does not
verify, a publisher the operator has not listed, and a signed manifest paired
with a different binary. **Nothing loads without a key the operator typed.** A
valid signature from a stranger is still a stranger.

### If your engine is closed source

It does not have to link against this binary. Run it listening on a unix
socket and speak one line of JSON each way:

```bash
emem-guard --module sidecar:/run/your-engine.sock
```

Request in, response out, byte-identical to what the webhook module posts, so
you implement one shape and can be reached over either transport:

```json
{"text_digest":"<hex>","texts":["..."],"token_count":0,"claim_count":0}
{"outcome":"deny","reason":"policy 12","confidence":0.8}
```

A socket rather than a process we spawn, because process lifecycle means
restart policy, zombie reaping and a crash loop that becomes the operator's
incident. A missing socket is an abstain. A socket that accepts the write and
never answers is also an abstain, bounded by your declared budget.

### What this does to your verdicts

The registry's config digest enters the verdict preimage, so every signed record
names the exact module set that produced it. Change which modules are loaded, or
just their order, and the digest moves. Without that, an operator could swap
detectors and the same signed record would describe two different evaluations.

A module denial carries the code `POLICY_MODULE` and your declared `fix`. The
reason line does not grow a field for the module id, because that grammar is
fixed and agents parse it; the id, the version and the evidence digest reach the
caller through the native route and the log.

## Step 10: raise the body limit

Transcripts are sent untruncated, up to 10 MB. Several common defaults are far
smaller: nginx `client_max_body_size` is 1 MB, and Express `express.json()` is
100 kB. A rejected body counts as a webhook failure, so under allow-on-failure
an oversized prompt reaches the model uninspected.

If you are behind nginx:

```
client_max_body_size 10m;
```

Check: post a 9 MB body and confirm you still get a verdict rather than a 413.

## Step 11: prove a verdict to someone who does not trust you

Every verdict names a log leaf. Fetch it and check it yourself:

```bash
curl -s localhost:8080/log/entry/leaf_0
```

You get the record, the ed25519 signature over its preimage, the public key
that signed, and the chain link to the entry before. Verify the signature with
any ed25519 implementation over `blake3`-free, length-prefixed segments
documented in `/.well-known/emem-guard.json`. Nothing in that check involves
asking the node whether it is telling the truth.

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

Signatures alone prove each verdict is genuine. The chain is what proves none
were removed: delete a line and every remaining signature still verifies, but
the chain breaks at the seam and `intact` goes false with a non-zero exit.

`GET /log/head` publishes the head so a witness or a mirror can pin it, and
`GET /log/entries?start=0` lets anyone mirror the log without your help.

## Step 12: the vendor checkpoints, if you want them

### Alerting on unverified allows

A request whose signature does not verify still gets a 200 with an allow, for
the reason in step 6: refusing would be a transport failure, which blocks
nothing and counts against the breaker. That is right, and it is also the one
condition where the guard answers without knowing who asked.

It is logged at a distinct, greppable marker so you can alert on it without
parsing every allow off disk:

```
emem-guard: UNVERIFIED_ALLOW inbound signature not verified: <why>
```

Sustained `UNVERIFIED_ALLOW` is either a misconfigured secret or somebody else
talking to your endpoint. Both deserve a page.

For Claude Enterprise Inference hooks, an administrator sets your HTTPS URL in
the organisation configuration and presses Test connection. The test sends a
synthetic prompt with `source.application` of `config-test` and reports the
verdict you returned.

The first connection test arrives **unsigned**, because the signing secret does
not exist until the first save. Accept unsigned requests until your
administrator confirms the secret exists, then add `--require-signature`.

For Claude Code, the client posts hook input and you block by returning **2xx
with a deny decision in the body**. A non-2xx is non-blocking. This is the
inverse of the usual HTTP intuition.

## Step 13: rotation

Secret rotation is an immediate cutover on the platform side, but requests
signed with the previous secret keep arriving for about a minute, plus anything
already in flight. Pass `--secret` twice during the switchover. A server
holding one secret drops those stragglers, and a dropped request is a webhook
failure.

## Step 14: run the conformance suite before you point anything at it

Every step above checks one thing you just did. This checks the DEPLOYMENT, over
the wire, exactly as a checkpoint would reach it.

```bash
./target/release/emem-guard --conformance https://your-node
```

```
  pass  reachable                                   0 ms  GET /health answered
  pass  large_body_is_accepted                     72 ms  9 MB body: 200 OK with a verdict
  pass  leaf_is_fetchable_and_verifies              1 ms  leaf_9 fetched and its signature verifies offline
  pass  holds_up_under_concurrency                 69 ms  96 requests at 16 concurrent: worst 20 ms, no failures

12 of 12 checks passed. This deployment can be relied on by a checkpoint.
```

Twelve checks, non-zero exit on any failure, and each failure prints what
breaks in production rather than restating itself. It runs against a URL
because everything it looks for lives between the code and the deployment: a
proxy that rejects a 9 MB body, a terminator that adds 400 ms, a node that
answers 502 while its own tests are green.

It is not decoration. Its first run against this project's own server found a
9 MB body returning 413, because axum applies a 2 MB default that wins over the
10 MB layer that was configured. The docs said 10 MB, the comment said 10 MB,
the layer said 10 MB, and the socket said 413. Under a fail-open policy that
turns every oversized transcript into an uninspected request.

Do not point a checkpoint at a node that has not passed this.

## What "done" looks like

- `cargo test --release -p emem-guard` is green.
- `/health` reports whether this node verifies citations, and you know which
  answer you have.
- An unknown event type returns 200 with an allow.
- A 9 MB body returns a verdict rather than a 413.
- A tampered token in a transcript produces a deny whose reason parses as
  `EMEM-GUARD DENY <CODE> token=<token> fix=<fix> leaf=<leaf>`.
- The leaf that denial names fetches from `/log/entry/<leaf>` and its signature
  verifies against the key in the entry.
- `emem-guard --audit` exits 0.
- `emem-guard --conformance <your-url>` exits 0.

## The reason grammar, for agents

Denials are machine-first on purpose. The line your agent receives is:

```
EMEM-GUARD DENY PROV_SIG token=emem:fact:cell:cid fix=refresh_token leaf=leaf_01HX
```

`fix` is the actionable part:

- `refresh_token`: re-resolve the token and retry.
- `remove_reference`: the citation cannot be made to verify; drop it.
- `contact_admin`: a person restricted this, not the evidence.
- `cite_observation`: resolve the observation through emem and cite the token.

`leaf` is the log entry, which you can fetch and verify without asking the
server that issued it.

The grammar is fixed and will not grow fields, because agents parse it. When a
denial has more to say, the native route says it: `POST /verdict` returns
`code`, `fix`, `token` and `leaf` already split out, plus `claim` naming the
sentence, the magnitude and the emem band that would have answered it.

An agent that reads `fix` and acts on it starts carrying valid tokens. That is
the point of the gate.

## Using the shared corpus without hosting it

Your node holds whatever you give it. To check citations against the public
corpus as well, point it at a responder:

```bash
./target/release/emem-guard --responder https://emem.dev
```

Your verdicts stay yours: they are signed with your key, written to your log,
and never leave your machine. The responder is asked one thing, whether a fact
cid resolves, and it is asked under a hard timeout that the verdict deadline
cannot exceed.

If you only want to consult a verdict rather than enforce one, emem.dev serves
the same engine at `POST /v1/guard/verdict`, advisory and blocking nothing.
That is the fastest way to see what this does before you build anything.

## If you get stuck

Report the step number and the exact command output. Do not work around a
failing check: every check here exists because skipping it produces a server
that appears to enforce and does not.
