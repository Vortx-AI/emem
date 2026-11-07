# emem-guard listings from release 2.1.0

This page is a record. On 10 and 11 August 2026, release 2.1.0 added
`emem-guard`, a verdict server. The maintainers then planned where to list
it, and drafted the entry lines, PR text and announcement posts. The
drafts are in [Drafts from August 2026](#drafts-from-august-2026), as
written then.

We checked each venue and each guard fact again on 7 November 2026.
We used the GitHub API, crates.io and the live `https://emem.dev` routes. The general listing log is
[`registry_claude.md`](../../registry_claude.md).

## State of each venue on 7 November 2026

The August plan had nine venues in a first wave and ten targets in a
second wave. Four targets are in both lists. This table holds all 15.
A search for `emem` and for the author `avijeetsingh1` in each repository
found the issues and PRs below, and no others.

| Venue | August plan | State on 7 November 2026 |
|---|---|---|
| [inference-gateway/awesome-a2a](https://github.com/inference-gateway/awesome-a2a) | Second wave, A2A | Listed. PR #7 merged on 17 August 2026. The README lists `emem`, linked to emem.dev. |
| [ottosulin/awesome-ai-security](https://github.com/ottosulin/awesome-ai-security) | Submit a PR | PR #382 open since 11 August 2026, with no comment or review. |
| [efij/awesome-claude-code-security](https://github.com/efij/awesome-claude-code-security) | Submit a PR from a fork | Issue #27 open since 11 August 2026, with no comment. It is an issue, not the PR that the plan names. |
| [tamish560/awesome-mcp-security](https://github.com/tamish560/awesome-mcp-security) | Submit a PR | Not filed. No issue or PR found. |
| [hesreallyhim/awesome-claude-code](https://github.com/hesreallyhim/awesome-claude-code) | A person files it through the web issue form | Not filed. |
| [enguard-ai/awesome-ai-guardrails](https://github.com/enguard-ai/awesome-ai-guardrails) | Optional, weak fit | Not filed. |
| [fuzzylabs/awesome-secure-mcp-servers](https://github.com/fuzzylabs/awesome-secure-mcp-servers) | Ask for a scan of `https://emem.dev/mcp` | Not filed. |
| [bh-rat/awesome-mcp-enterprise](https://github.com/bh-rat/awesome-mcp-enterprise) | Does not qualify | Not filed, as planned. |
| [ant-research/awesome-mllm-guardrails](https://github.com/ant-research/awesome-mllm-guardrails) | Wrong list | Not filed, as planned. |
| [rust-unofficial/awesome-rust](https://github.com/rust-unofficial/awesome-rust) | First held, then marked as filed | Not filed. No issue or PR found. |
| [pab1it0/awesome-a2a](https://github.com/pab1it0/awesome-a2a) | Second wave, A2A | Not filed. |
| [ai-boost/awesome-a2a](https://github.com/ai-boost/awesome-a2a) | Second wave, A2A | Not filed. |
| [wong2/awesome-mcp-servers](https://github.com/wong2/awesome-mcp-servers) | Second wave, MCP | Not filed. |
| [appcypher/awesome-mcp-servers](https://github.com/appcypher/awesome-mcp-servers) | Second wave, MCP | Not filed. The owner archived the repository, so it takes no new entries. |
| [capizziemanuele/useful-geospatial-tools](https://github.com/capizziemanuele/useful-geospatial-tools) | Second wave, geospatial | Not filed. The last push was on 22 July 2024. |

## What the August record said that is not true now

| August text | State on 7 November 2026 |
|---|---|
| Ten sibling sessions are running, one for each second-wave target. | Three of the ten targets have a trace: PR #382, issue #27 and the merged A2A PR. The other seven have no issue or PR. |
| The awesome-rust submission is in. | No issue or PR names emem in that repository. |
| emem has 52 GitHub stars. | `Vortx-AI/emem` has 89 stars. |
| `https://emem.dev/release.png` serves the 2.1.0 card. | It serves the current card. Its bytes are the same as `web/release-2.4.4.png`, not `web/release-2.1.0.png`. |
| The guard denies with `PROV_SIG`, `PROV_BYTES` and `PROV_DRIFT`. | These three codes still exist. The contract now lists seven codes (see below). |

The venue rules that we checked again still hold.
The `hesreallyhim/awesome-claude-code` CONTRIBUTING file still asks for the
web issue form and says "Do not open a PR". Its bar is a repository at
least 14 days old with active development, or 100 stars. The
`rust-unofficial/awesome-rust` bar is still 50 GitHub stars or 2000
crates.io downloads. At 89 stars, emem clears it. We did not check the
rules of the other venues again.

## Guard facts that the drafts cite

The drafts make claims about `emem-guard`. This table gives each claim
and its state on 7 November 2026. The live contract is
`GET https://emem.dev/.well-known/emem-guard.json`.

| Claim | State on 7 November 2026 | Source |
|---|---|---|
| Nine checkpoints | True. The contract lists nine routes. emem.dev hosts seven. `/verdict/anthropic-hook` and `/verdict/claude-code` run only on a node that you run. | Live contract |
| Deny codes | Seven: `PROV_SIG`, `PROV_BYTES`, `PROV_DRIFT`, `GEO_ZONE`, `PROV_VALUE`, `CLAIM_UNGROUNDED`, `POLICY_MODULE`. | Live contract |
| Reason line `EMEM-GUARD DENY <CODE> token=... fix=... leaf=...` | True. | Live contract, `reason_grammar` |
| A citation that the node does not hold is never a denial | True. | Live contract, `never_denies_on` |
| The verdict log and `emem-guard --audit` | True on a node that you run. emem.dev keeps no verdict log. `/log/head` and `/log/entries` are in `not_served_here`. | Live contract |
| Flags `--audit`, `--shadow`, `--report`, `--module`, `--signed-module`, `--conformance` | True. | `crates/emem-guard/src/bin/emem-guard.rs` |
| `--conformance` runs twelve checks | True. | `CHECKS` in `crates/emem-guard/src/conformance.rs` |
| A `fast` module that exceeds 50 ms three times can no longer block | True. | `FAST_BUDGET` and `FAST_STRIKES` in `crates/emem-guard/src/module.rs` |
| Apache-2.0, built with `cargo build --release -p emem-guard` | True. The workspace licence is Apache-2.0. | `Cargo.toml` |
| Not yet used by a live organisation | The crate README still says so. | `crates/emem-guard/README.md` |
| `emem`, `emem-guard` and `emem-core` are not on crates.io | True for all three. | crates.io API |
| `https://emem.dev/release-2.1.0.png` returns 404 | True. | Live GET |
| `https://emem.dev/guard` and the A2A routes answer | True. `/guard`, `/.well-known/agent-card.json` and `/v1/a2a/skills?q=elevation` return 200. | Live GET |

## Venue rules (August 2026)

The maintainers read these rules in the `CONTRIBUTING.md` or README of each
venue in August 2026. Read them again before you file.

| Venue | Rule |
|---|---|
| `tamish560/awesome-mcp-security` | Entry format `- [owner/repo](https://github.com/owner/repo) - Description. By Author/Org.` Alphabetical in the section. One or two sentences. No marketing words. Review takes 3 to 5 days. |
| `ottosulin/awesome-ai-security` | Entry is a bare name, a dash and a lowercase description with no full stop at the end. Open a PR, or contact the owner. |
| `efij/awesome-claude-code-security` | Entry format `- [Title](https://example.com/tool) - description.` with 15 to 30 words. Capital first letter, full stop at the end, alphabetical, one resource for each line. Send from a fork. Review target is two weeks. |
| `hesreallyhim/awesome-claude-code` | A person must use the [web issue form](https://github.com/hesreallyhim/awesome-claude-code/issues/new?template=recommend-resource.yml). No PR, no `gh` CLI, no agent. One resource for each submission. |
| `enguard-ai/awesome-ai-guardrails` | A table row in the shape `name \| scope \| "description"`. |
| `fuzzylabs/awesome-secure-mcp-servers` | Entries are servers that passed their `mcp-scan` pipeline. The README gives no self-submission path. Open an issue that asks them to scan `https://emem.dev/mcp`. Do not add a row with a score that nobody computed. |
| `bh-rat/awesome-mcp-enterprise` | The listing template needs two of four proofs. They are named customers, a checked compliance standard (SOC 2, ISO 27001 or HIPAA), 6 months of general availability, or two documented production deployments. emem-guard has none. |
| `rust-unofficial/awesome-rust` | At least 50 stars, 2,000 downloads on crates.io, or an equal level of other popularity. crates.io downloads are optional. The list takes applications. |

For the A2A lists, give these endpoints. Both answered 200 on 7 November 2026.

- The agent card: `https://emem.dev/.well-known/agent-card.json`.
- The skill query: `https://emem.dev/v1/a2a/skills?q=elevation`.
- Tasks: `https://emem.dev/a2a/tasks` (message send) and
  `https://emem.dev/v1/a2a/tasks` (create, get, cancel).

Do not put the 2.1.0 release card in `README.md`. A release card at the top
would date the page at each new version.

## Next steps

A person must do these steps. On 7 November 2026, nobody had done them.

1. Ask for a review on [ottosulin/awesome-ai-security#382](https://github.com/ottosulin/awesome-ai-security/pull/382).
2. Change [efij/awesome-claude-code-security#27](https://github.com/efij/awesome-claude-code-security/issues/27) into a PR from a fork, because the venue asks for one.
3. File `hesreallyhim/awesome-claude-code` through the [web issue form](https://github.com/hesreallyhim/awesome-claude-code/issues/new?template=recommend-resource.yml). Do not use an agent or the `gh` CLI.
4. Send the PR to `tamish560/awesome-mcp-security`.
5. Send the `rust-unofficial/awesome-rust` PR. Say in it that emem is not on crates.io.
6. Before you use a draft, update its facts from the table above. The deny codes and the card changed.

## Drafts from August 2026

The text in this section is as the maintainers wrote it in August 2026. We
did not edit it. Some facts in it changed; the tables above give the
current state.

### tamish560/awesome-mcp-security

Section: Firewalls and Guardrails. Entry line, PR title and PR body:

```markdown
- [Vortx-AI/emem](https://github.com/Vortx-AI/emem/tree/main/crates/emem-guard) - emem-guard answers allow/deny on a transcript by re-resolving the signed observations it cites, denying on a failed signature, changed bytes, or drift past a band threshold. Every verdict is ed25519-signed and hash-chained into an append-only log that `emem-guard --audit` re-checks without trusting the node that issued it. By Vortx AI.
```

```
Add Vortx-AI/emem to Firewalls and Guardrails
```

```markdown
Adding `emem-guard` under Firewalls and Guardrails.

**What it is.** A verdict server: input is a transcript, output is allow or
deny with a machine-readable reason. It differs from the scanners already in
this list in what it looks at. A content scanner asks whether a card number or
a secret is present. emem-guard asks whether the `emem:` citations in the
transcript still resolve to the bytes they were signed over, denying `PROV_SIG`
on a failed signature, `PROV_BYTES` when the token resolves to different bytes,
and `PROV_DRIFT` when the value moved past a band threshold.

**Why it is a security entry and not a data entry.** Every verdict is
ed25519-signed and appended to a hash-chained log *before* it is returned.
Signatures alone would prove each verdict genuine; the chain is what proves
none were removed. `emem-guard --audit --data ./var/guard` exits non-zero if a
verdict was altered or deleted, and anyone can run it against a log they did
not produce.

**MCP relevance.** `POST /verdict/mcp` gates an MCP `tools/call` **or a tool
result**, which is the injection path a gateway sitting in front of the server
does not see. Eight further checkpoints answer the same engine in other shapes:
Anthropic Inference hooks, Claude Code client hooks, OpenAI-compatible,
CloudEvents 1.0, OPA-style policy input, batch, and a log read. Seven of the
nine belong to no vendor.

**Checklist**

- Open source, Apache-2.0, pure Rust: `cargo build --release -p emem-guard`
- Not already listed (searched `emem` and `Vortx`)
- Contract published at `GET /.well-known/emem-guard.json`, so the claims
  above are checkable without taking my word for them
- Description is two sentences, no marketing language, alphabetically placed

**Scope, stated plainly.** It is not a DLP scanner and does not classify
content itself; it hosts third-party modules and signs their findings instead.
A citation the node has not cached is never a denial, because that is
indistinguishable from a token minted by another responder. The engine and
server are tested but have not yet been pointed at a live organisation.
```

### ottosulin/awesome-ai-security

Section: Input/Output Guardrails. Entry line, PR title and PR body:

```markdown
- [emem-guard](https://github.com/Vortx-AI/emem/tree/main/crates/emem-guard) - allow/deny verdict server for claims about the physical world; every verdict is ed25519-signed and hash-chained into an offline-auditable log, and one engine answers Anthropic Inference hooks, Claude Code hooks, MCP tools/call, OpenAI-shaped, CloudEvents and OPA-style inputs
```

```
Add emem-guard to Input/Output Guardrails
```

```markdown
Both current entries in this section evaluate the *text* of a message.
`emem-guard` evaluates whether the observations a message cites still hold: it
re-resolves each `emem:` token and denies on a failed signature, on bytes that
changed, or on a value that drifted past a band threshold. No detection engine
holds signed observations of the physical world, so none of them can answer
that question.

The half that makes it a security control rather than a lookup is what happens
after the verdict. Each one is ed25519-signed and appended to a hash-chained
log before it is returned, so `emem-guard --audit` detects an altered or
deleted verdict in a log you did not produce.

Apache-2.0, pure Rust, self-hosted, no account. It is deliberately bad at
content classification and stays that way: third-party detectors load as
modules (`--module`, `--signed-module`, or a sidecar over a unix socket) and
their findings get signed and logged like a native rule. A module declaring
`fast` that exceeds 50 ms three times is demoted and stops being able to block.

Happy to move it to Agent Runtime Security or MCP Security if you would rather
file it there.
```

### efij/awesome-claude-code-security

Section: Hooks and Guardrails. Entry line (21 words), PR title and PR body:

```markdown
- [emem-guard](https://github.com/Vortx-AI/emem/tree/main/crates/emem-guard) - Self-hosted verdict server for Claude Code hooks and Anthropic Inference hooks; signs and hash-chains every allow/deny so decisions stay auditable offline.
```

```
Add emem-guard to Hooks and Guardrails
```

```markdown
**Category:** 🪝 Hooks and Guardrails

**Why it meets the criteria**

- *Relevant.* Two of its nine checkpoints are Claude-specific:
  `POST /verdict/anthropic-hook` speaks the Inference hooks contract for a
  Claude Enterprise org, and `POST /verdict/claude-code` covers Claude Code
  client hooks on the Platform API, Bedrock and Vertex, which Inference hooks
  do not reach.
- *Practically useful.* A platform owner runs one binary
  (`cargo build --release -p emem-guard`) and points the hook at it. It
  generates a key, opens a log, and serves.
- *High signal.* The control it adds is not detection, it is the audit half:
  every verdict is ed25519-signed and appended to a hash-chained log before it
  is returned, so a deleted verdict is detectable, not just a modified one.
  `emem-guard --audit` verifies a log without trusting the node that wrote it.
  A denial is machine-first, `EMEM-GUARD DENY PROV_SIG token=... fix=refresh_token
  leaf=leaf_41`, where `fix` tells the agent what to do rather than the human.
- *Current.* Shipped in v2.1.0, Apache-2.0, active development.
- *Unique.* Nothing in this section evaluates whether a cited claim still
  holds; the existing entries cover secrets and permissions.

Checked for duplicates. One entry, alphabetically placed, 21-word description.

**What it does not do.** It is not a DLP scanner and does not classify content
itself. Claim gating ships off, behind `--shadow` and `--report`, because a
rule that denies on absence should be measured on your traffic before it
blocks anything. The engine and server are tested but have not yet been
pointed at a live organisation.
```

### hesreallyhim/awesome-claude-code

Two separate web-form submissions. Submission A goes in Security.

```text
emem-guard, a self-hosted server that answers Claude Code hooks and
Anthropic Inference hooks with allow or deny. It checks whether the
observations a transcript cites still resolve to the bytes they were signed
over, and signs and hash-chains every verdict into a log you can audit
offline with `emem-guard --audit`.
```

Submission B is for emem itself and goes in Memory & Context Persistence.

```text
emem, memory that outlives the context window. An agent keeps one
`emem:fact:` token instead of a paraphrase; after compaction, a handoff, or
a model swap, the token resolves to the byte-identical signed record and the
signature still checks. Reads need no key or account. MCP endpoint at
`https://emem.dev/mcp`.
```

### enguard-ai/awesome-ai-guardrails

A row for the Organisations/Companies, Open Source table:

```
| emem-guard | all | "Signed allow/deny verdicts on claims about the physical world, with a hash-chained log." |
```

### Announcement posts

Show HN title and body:

```
Show HN: Emem-guard, a signed allow/deny server for claims about the world
```

```text
Anthropic's Inference hooks hold every governed prompt for an allow or deny
verdict from a server your organisation runs, before the model sees it. The
named destinations are DLP vendors, and they all evaluate content: does this
text carry a card number, a secret, a classified marking. None of them can
evaluate whether a claim about the physical world still holds, because none
of them hold signed observations of it.

emem-guard is that server. Input is a transcript, output is allow or deny.
It re-resolves the emem: tokens the transcript cites and denies PROV_SIG on a
failed signature, PROV_BYTES when a token resolves to different bytes, and
PROV_DRIFT when the value moved past a band threshold.

The part I think is actually interesting is the half after the verdict. Every
verdict is ed25519-signed and appended to a hash-chained log before it is
returned. Signatures alone prove each verdict genuine; the chain is what
proves none were removed. `emem-guard --audit` exits non-zero on an altered or
deleted entry, and it works on a log you did not produce, including ours.

Nine checkpoints answer from one engine, and seven of them belong to no
vendor: emem native, MCP tools/call (gating a call or a result), an
OpenAI-shaped route, CloudEvents 1.0, an OPA-style policy point, batch, log
read, plus the two Claude ones. A gate reachable only through one company's
product is a gate for that company's customers.

Two things it deliberately does not do. It does not classify content, it is
bad at that and will stay that way; third-party detectors load as modules and
their findings get signed and logged like native rules. And a citation this
node has not cached is never a denial, because that is indistinguishable from
a token minted by another responder.

Honest status: the engine and the server run and are tested, and they have not
yet been pointed at a live organisation. Claim gating ships off, behind
--shadow and --report, because a rule that denies on absence should be
measured on your own traffic first. On this repo's own prose it fired 3 times
in 8739 sentences, two of which were its own test fixtures.

Apache-2.0, pure Rust, no account, no API key.
cargo build --release -p emem-guard && ./target/release/emem-guard

https://github.com/Vortx-AI/emem  ·  https://emem.dev/guard
```

Lobsters comment (tags `ai`, `security`, `rust`):

```text
Authored. emem-guard is a verdict server for AI inference hooks: it takes a
transcript and returns allow or deny, based on whether the observations the
transcript cites still resolve to the bytes they were signed over. Rust,
Apache-2.0, self-hosted.

The design point I would most like criticism on: every verdict is signed and
hash-chained into an append-only log before it is returned, so the audit
artefact is produced by the enforcement path rather than alongside it. That
buys tamper-evidence over deletion, not just modification, and it costs a
write on the hot path. I think the trade is right for a gate whose whole
argument is auditability, and I would like to hear why it is not.
```

r/mcp post:

```text
Title: emem-guard: gating MCP tools/call (and tool results) on whether the
cited evidence still verifies

POST /verdict/mcp takes an MCP tools/call, or a tool *result*, which is the
path a gateway in front of the server never sees, and returns allow or deny.
The check is not content classification. It re-resolves the emem: tokens in
the payload and denies if a signature fails, if the token now resolves to
different bytes, or if the value drifted past a band threshold.

Every verdict is ed25519-signed and hash-chained into an append-only log
before it is returned, so you can audit the gate offline with the same binary,
including against a node you do not run.

Eight other checkpoints answer the same engine: Anthropic Inference hooks,
Claude Code client hooks, OpenAI-shaped, CloudEvents, OPA-style policy input,
batch, log read, and the native route. The contract is at
GET /.well-known/emem-guard.json so an agent can integrate without being
handed a doc.

Apache-2.0, Rust, self-hosted, no account. Tested, not yet run against a live
org, saying so up front.
```

r/rust post:

```text
Title: emem-guard: a hash-chained, ed25519-signed verdict log on the
enforcement path (Rust, Apache-2.0)

Built a policy server where the audit artefact is produced by the enforcement
path rather than emitted next to it. Each allow/deny is signed and appended to
a hash-chained log before the response goes out, so removing a verdict is
detectable and not just modifying one. `--audit` re-verifies a log the binary
did not write.

Two bits that were more interesting to build than expected:

- Module isolation without a sandbox. A module declares `fast`, `slow` or
  `digests_only`. `slow` never runs on the enforcing path; a `fast` module
  that exceeds 50 ms three times is demoted and loses the ability to block;
  `digests_only` is handed an empty transcript rather than asked politely not
  to read it. The loaded set's digest enters the verdict preimage, so a
  verdict names the pipeline that produced it. Out-of-process modules load
  over a unix socket, so a closed-source engine never links against the binary.

- Conformance over the wire, not just unit tests. `--conformance <url>` runs
  twelve checks against a deployed server, because handler tests prove the
  handlers and prove nothing about what you stood up. Its first run against
  our own node found a 9 MB body returning 413.

cargo build --release -p emem-guard
https://github.com/Vortx-AI/emem/tree/main/crates/emem-guard
```

X and LinkedIn post, with the card image attached:

```text
emem 2.1.0 ships emem-guard: a server that answers allow or deny on a
transcript, before an agent asserts.

Not content classification. It re-resolves the observations the transcript
cites and denies when a signature fails, when the bytes changed, or when the
value drifted past a threshold. DLP vendors can tell you a message contains a
secret. None of them can tell you a claim about the world still holds.

Every verdict is signed and hash-chained before it is returned, so the log
proves not just that no verdict was altered but that none were removed, and
you can check ours without asking us.

Nine checkpoints, seven of which belong to no vendor. Apache-2.0, Rust, self-
hosted, no account.

Tested; not yet pointed at a live organisation. Saying so because a gate that
oversells itself is the one you should not install.

https://emem.dev/guard
```

MCP Discord, channel `#showcase`:

```text
emem 2.1.0 adds emem-guard, allow/deny on an MCP tools/call or tool result,
based on whether the evidence it cites still verifies, not on what the text
looks like. Signed and hash-chained verdicts, auditable offline with the same
binary. Apache-2.0, Rust, self-hosted, no account.
https://emem.dev/guard
```
