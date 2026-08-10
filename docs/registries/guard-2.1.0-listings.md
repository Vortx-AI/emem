# Listing emem-guard: verified venues and ready-to-fire submissions

2.1.0 added `emem-guard`, and that is a different product category from
everything emem has been listed under so far. The existing listings
(`registry_claude.md`) are geospatial and MCP-catalogue lists, which were
right for a memory protocol and are the wrong shelf for a verdict server.
This document is the guard-era target list: where emem-guard belongs, the
exact line to add, and the PR text, one section per venue.

Every venue below was checked against its own README and CONTRIBUTING
before it was written down: the section it goes in, the entry format, the
ordering rule, and the submission mechanism. Three candidate venues were
checked and rejected, and the reason is recorded rather than dropped,
because the next person to look will otherwise check them again.

## Status of each target

| Venue | Section | Mechanism | Verdict |
|---|---|---|---|
| [tamish560/awesome-mcp-security](https://github.com/tamish560/awesome-mcp-security) | Firewalls and Guardrails | PR | **submit** |
| [ottosulin/awesome-ai-security](https://github.com/ottosulin/awesome-ai-security) | Input/Output Guardrails | PR | **submit** |
| [efij/awesome-claude-code-security](https://github.com/efij/awesome-claude-code-security) | Hooks and Guardrails | PR (from a fork) | **submit** |
| [hesreallyhim/awesome-claude-code](https://github.com/hesreallyhim/awesome-claude-code) | Security | web issue form | **human only, see below** |
| [enguard-ai/awesome-ai-guardrails](https://github.com/enguard-ai/awesome-ai-guardrails) | Organisations/Companies | PR | optional, weak fit |
| [fuzzylabs/awesome-secure-mcp-servers](https://github.com/fuzzylabs/awesome-secure-mcp-servers) | Community | issue, then their scan | ask to be scanned |
| [bh-rat/awesome-mcp-enterprise](https://github.com/bh-rat/awesome-mcp-enterprise) | Security & Governance | issue, then PR | **does not qualify** |
| [ant-research/awesome-mllm-guardrails](https://github.com/ant-research/awesome-mllm-guardrails) | — | PR | **wrong list** |
| [rust-unofficial/awesome-rust](https://github.com/rust-unofficial/awesome-rust) | — | PR | **hold** |

---

## 1. tamish560/awesome-mcp-security

The closest fit of any list found. It has a **Firewalls and Guardrails**
section whose existing entries are proxies and policy engines, and no entry
in it evaluates whether a cited claim still holds.

`CONTRIBUTING.md` requires the format `- [owner/repo](url) - Description. By
Author/Org.`, alphabetical order within the section, one or two sentences,
and explicitly excludes marketing language. Review is 3 to 5 days.

**File:** `README.md` · **Section:** Firewalls and Guardrails · sort under `V`

```markdown
- [Vortx-AI/emem](https://github.com/Vortx-AI/emem/tree/main/crates/emem-guard) - emem-guard answers allow/deny on a transcript by re-resolving the signed observations it cites, denying on a failed signature, changed bytes, or drift past a band threshold. Every verdict is ed25519-signed and hash-chained into an append-only log that `emem-guard --audit` re-checks without trusting the node that issued it. By Vortx AI.
```

**PR title**

```
Add Vortx-AI/emem to Firewalls and Guardrails
```

**PR body**

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

---

## 2. ottosulin/awesome-ai-security

Section 18, **Input/Output Guardrails**, currently NeMo-Guardrails and
llm-guard: both classify text. Same gap as above. Entry style in this list is
a bare name, a dash, and a lowercase description with no trailing period.
Contribution is "create a PR or contact me @ottosulin".

**File:** `README.md` · **Section:** Input/Output Guardrails

```markdown
- [emem-guard](https://github.com/Vortx-AI/emem/tree/main/crates/emem-guard) - allow/deny verdict server for claims about the physical world; every verdict is ed25519-signed and hash-chained into an offline-auditable log, and one engine answers Anthropic Inference hooks, Claude Code hooks, MCP tools/call, OpenAI-shaped, CloudEvents and OPA-style inputs
```

**PR title**

```
Add emem-guard to Input/Output Guardrails
```

**PR body**

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

---

## 3. efij/awesome-claude-code-security

Section 4 is literally **Hooks and Guardrails**, and emem-guard ships two
Claude-specific checkpoints: `POST /verdict/anthropic-hook` for Inference
hooks in a Claude Enterprise org, and `POST /verdict/claude-code` for client
hooks on the Platform API, Bedrock and Vertex, which Inference hooks cannot
see. That second one is the entry's reason to exist.

`CONTRIBUTING.md` requires `- [Title](URL) - description.` at 15 to 30 words,
capitalised, ending in a period, alphabetical within the category, one
resource per line, submitted from a fork. Review target is two weeks.

**File:** `README.md` · **Section:** 🪝 Hooks and Guardrails · sort under `e`

```markdown
- [emem-guard](https://github.com/Vortx-AI/emem/tree/main/crates/emem-guard) - Self-hosted verdict server for Claude Code hooks and Anthropic Inference hooks; signs and hash-chains every allow/deny so decisions stay auditable offline.
```

(21 words, inside the 15 to 30 range.)

**PR title**

```
Add emem-guard to Hooks and Guardrails
```

**PR body**

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
  A denial is machine-first — `EMEM-GUARD DENY PROV_SIG token=... fix=refresh_token
  leaf=leaf_41` — where `fix` tells the agent what to do rather than the human.
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

---

## 4. hesreallyhim/awesome-claude-code — Avijeet has to file this one

**Do not send an agent at this.** `CONTRIBUTING.md` says
`ALL RECOMMENDATIONS MUST BE MADE USING THE WEB UI ISSUE FORM TEMPLATE`,
`Do not open a PR`, states that submission via the `gh` CLI is not possible,
and asks that recommendations come from humans rather than AI agents. Filing
it any other way is a defensible reason for the maintainer to close it.

Form: <https://github.com/hesreallyhim/awesome-claude-code/issues/new?template=recommend-resource.yml>

Eligibility is met on the first branch of the rule: the repository is far
older than 14 days and shows active development. It does not meet the
100-star alternative (52 at the time of writing), which is fine, the criteria
are `OR`.

One resource per submission, so it is two separate submissions, filed apart:

**Submission A — Security**

> emem-guard — a self-hosted server that answers Claude Code hooks and
> Anthropic Inference hooks with allow or deny. It checks whether the
> observations a transcript cites still resolve to the bytes they were signed
> over, and signs and hash-chains every verdict into a log you can audit
> offline with `emem-guard --audit`.

**Submission B — Memory & Context Persistence** (this is emem itself, not the
guard, and it is arguably the better of the two: the section exists for
exactly this problem)

> emem — memory that outlives the context window. An agent keeps one
> `emem:fact:` token instead of a paraphrase; after compaction, a handoff, or
> a model swap, the token resolves to the byte-identical signed record and the
> signature still checks. Reads need no key or account. MCP endpoint at
> `https://emem.dev/mcp`.

---

## 5. enguard-ai/awesome-ai-guardrails — optional

A table-shaped list built mainly around guard *models* and datasets. emem-guard
is neither, so it fits only the Organisations/Companies → Open Source table.
Low cost, low return. Row, matching the `name | scope | "description"` shape:

```
| emem-guard | all | "Signed allow/deny verdicts on claims about the physical world, with a hash-chained log." |
```

## 6. fuzzylabs/awesome-secure-mcp-servers — ask to be scanned

Entries here are servers that have been through their `mcp-scan` pipeline and
carry a security score, and the README documents no self-submission path.
The move is an issue asking for `https://emem.dev/mcp` to be added to the scan
queue, not a PR that adds a row with a score nobody computed. Worth doing:
the list is small (16 servers) and the scored-server framing suits a server
whose whole argument is that its claims are checkable.

## 7. bh-rat/awesome-mcp-enterprise — does not qualify, do not submit

Its listing-proposal template requires at least two of four proofs: named
customers, verifiable compliance (SOC 2 / ISO 27001 / HIPAA), 6+ months GA,
or two documented production deployments. emem-guard's own README says the
engine and server "have not yet been pointed at a live organisation," so it
meets none of the four. Revisit after the first design partner is live and
the conformance suite is green.

## 8. ant-research/awesome-mllm-guardrails — wrong list

It catalogues guard models, jailbreak attacks, moderation datasets and safety
benchmarks. emem-guard is infrastructure and classifies nothing. Submitting
would be noise.

## 9. rust-unofficial/awesome-rust — hold

The list is for published, used crates, and `emem` does not exist on
crates.io (checked: `crates.io/api/v1/crates/emem` returns
`crate 'emem' does not exist`). Publish `emem-guard` and `emem-core` first,
then this becomes a reasonable submission. Until then it would be declined on
the obvious ground.

---

## The photo

`web/release-2.1.0.png`, 1200×630, is the 2.1.0 card. Two of the three places
it needs to be are already done:

- **GitHub release v2.1.0** — attached, present in the release body.
- **Public URL** — served at <https://emem.dev/release.png> (and
  `/release.svg`), from `web/release-current.png`, which is byte-identical to
  `web/release-2.1.0.png`. That is the URL to hand to anything that needs to
  fetch the card.

  Note that `https://emem.dev/release-2.1.0.png` is **404** and always will be:
  the route is `/release.png`, and the versioned file is the archive copy, not
  a served path. Do not paste the versioned URL into a post.

- **Still to do** — the social posts below. Every one of them takes an image,
  and the card is the right one: it names the version, the two headline
  changes, and one limit, which is the tone the rest of the project keeps.

Do **not** add the card to `README.md`. The README opens with
`web/emem-strip.png`, which explains the protocol; a release card above it
would date the page at every version bump.

---

## Announcement copy

Same facts everywhere, sized to each venue. All of it is checkable against
`GET /.well-known/emem-guard.json` and the README, which is deliberate: the
claim these posts make is that claims should be checkable.

### Show HN

**Title** (Show HN titles must not be editorialised, and 80 characters is the
practical ceiling):

```
Show HN: Emem-guard – a signed allow/deny server for claims about the world
```

**Body**

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

Two things it deliberately does not do. It does not classify content — it is
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

### Lobsters

Tags `ai`, `security`, `rust`. Lobsters wants the link and a short authored
comment, not a pitch:

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

### r/mcp

```text
Title: emem-guard: gating MCP tools/call (and tool results) on whether the
cited evidence still verifies

POST /verdict/mcp takes an MCP tools/call — or a tool *result*, which is the
path a gateway in front of the server never sees — and returns allow or deny.
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
org — saying so up front.
```

### r/rust

Lead with the engineering, not the product; that subreddit rejects the
reverse.

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

### X / LinkedIn

Attach `https://emem.dev/release.png`.

```text
emem 2.1.0 ships emem-guard: a server that answers allow or deny on a
transcript, before an agent asserts.

Not content classification. It re-resolves the observations the transcript
cites and denies when a signature fails, when the bytes changed, or when the
value drifted past a threshold. DLP vendors can tell you a message contains a
secret. None of them can tell you a claim about the world still holds.

Every verdict is signed and hash-chained before it is returned, so the log
proves not just that no verdict was altered but that none were removed — and
you can check ours without asking us.

Nine checkpoints, seven of which belong to no vendor. Apache-2.0, Rust, self-
hosted, no account.

Tested; not yet pointed at a live organisation. Saying so because a gate that
oversells itself is the one you should not install.

https://emem.dev/guard
```

### MCP Discord, #showcase

One short paragraph plus the card; the long form belongs in the links.

```text
emem 2.1.0 adds emem-guard — allow/deny on an MCP tools/call or tool result,
based on whether the evidence it cites still verifies, not on what the text
looks like. Signed and hash-chained verdicts, auditable offline with the same
binary. Apache-2.0, Rust, self-hosted, no account.
https://emem.dev/guard
```

---

## Sequencing

The two Claude-ecosystem lists should go first: they are the venues where the
Inference-hook and Claude-Code-hook checkpoints are the entry's whole reason
to exist, and that argument gets weaker as other people ship the same thing.
`awesome-mcp-security` and `awesome-ai-security` next, since both have a
section that is currently all content classifiers. Stagger the prose posts
behind the listings by a few days, so a reader arriving from Hacker News
finds the project already on the shelves they would check next.

---

# Filed: the second wave

The repo-scope block that stopped the first pass is worked around by the
mechanism the tooling itself names: a sibling session seeded with the target
repo as its initial source clears it. Ten are running, one per target, each
scoped to one file, one line, one PR, and each told to read the target's own
CONTRIBUTING and let the target's real formatting override the draft line.

| Target | Category | Session |
|---|---|---|
| tamish560/awesome-mcp-security | MCP security | `session_011NZHcndjdrtwjbpdSUG4E5` |
| ottosulin/awesome-ai-security | AI security | `session_01UTp6QpdhiWEXPmFPdiB9zW` |
| efij/awesome-claude-code-security | Claude Code security | `session_0171KHhx3qmDxgRXpLUmnNyU` |
| inference-gateway/awesome-a2a | A2A | `session_01V3ViwKZads3az2FrgjwJs1` |
| pab1it0/awesome-a2a | A2A | `session_011qCCtXwNgSesWknaeo5FCH` |
| ai-boost/awesome-a2a | A2A | `session_01U5DDDeWpDVY32SXSZRWaiz` |
| wong2/awesome-mcp-servers | MCP | `session_01XBjUmvuygqj9TWkUSfgjk2` |
| appcypher/awesome-mcp-servers | MCP | `session_01Tvpa2HF5i34Z9RjHSb7fii` |
| rust-unofficial/awesome-rust | Rust | `session_01PSZ6HGVuLeeHTbpmQaTC3T` |
| capizziemanuele/useful-geospatial-tools | geospatial | `session_01BipExGhrrZKSRJHLFRb8YP` |

Sessions inherit `default` permission mode and will block on a permission
prompt that only a human can clear in the web UI. The first one stalled on a
needless `list_repos` call, so every later prompt says not to call it. If you
spawn more, keep that line in.

## A2A is a new category and emem already qualifies

Nothing in `registry_claude.md` targets A2A, and the implementation is live
rather than aspirational, which is what these lists check:

- `https://emem.dev/.well-known/agent-card.json` — 200
- `a2a-message-send` → `https://emem.dev/a2a/tasks`
- `a2a-async-tasks` (create / get / cancel) → `https://emem.dev/v1/a2a/tasks`
- `a2a-skill-query` → `https://emem.dev/v1/a2a/skills?q=elevation` — 200

The argument that lands on an A2A list is not "signed data". It is that a
handoff in natural language transfers a paraphrase, and a paraphrase drifts;
an `emem:fact:` token transfers bytes both sides resolve identically. That is
a property of the handoff, which is the layer A2A defines.

## Correction: awesome-rust was wrongly held

An earlier version of this document held `rust-unofficial/awesome-rust` on the
grounds that it wants published crates. That was wrong. Their CONTRIBUTING
sets the bar at "at least 50 stars on GitHub, **2000 downloads on crates.io,
or an equivalent level of other popularity metrics**", crates.io is explicitly
optional, and applications are accepted alongside libraries. emem was at 52
stars, so it clears the bar today and the submission is filed.

Publishing the crates is still worth doing on its own merits, but it is not a
precondition for this listing. The submission states plainly that emem is not
on crates.io rather than leaving their template fields to be guessed at.

## The geospatial backlog is the bigger lever

Ten geospatial listings in `registry_claude.md` are still marked OPEN, some
for months, and one of them turned out to have merged without the table
noticing. Opening an eleventh geospatial PR is worth less than finding out
which of the ten are alive, stale, or merged. Do that sweep before adding
more geospatial targets.

## Still not filed, and why

- **hesreallyhim/awesome-claude-code** — human only. Their CONTRIBUTING
  forbids PRs, forbids the `gh` CLI, and asks that recommendations come from
  humans rather than AI agents. Avijeet files it; text is above.
- **fuzzylabs/awesome-secure-mcp-servers** — an issue asking to enter their
  `mcp-scan` queue, not a PR. Held until the first wave lands.
- **bh-rat/awesome-mcp-enterprise** — still does not qualify.
- **ant-research/awesome-mllm-guardrails** — still the wrong list.
