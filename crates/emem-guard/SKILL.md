---
name: selfhost-emem-guard
description: Stand up an emem-guard verdict server for AI inference hooks, verify it against the conformance checks, and point a checkpoint at it. Use when asked to self-host emem-guard, add a grounding gate to Claude Code or Claude Enterprise, or run a signed allow/deny server for physical-world claims.
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
`emem:` citations in a transcript still verify.

## Before you start

You need:

- Rust 1.91 or newer (`rustc --version`).
- A publicly reachable HTTPS URL on port 443 with a certificate that validates
  against the public CA trust store. Anthropic refuses private, loopback and
  carrier-grade NAT ranges at connect time, and does not follow redirects.
- The signing secret from your organisation's Inference hooks configuration,
  if you are wiring the Enterprise checkpoint. It begins `whsec_`.

You do NOT need an emem account, an API key, or network access to emem.dev.
A node with no upstream still verifies every token it holds locally, and
`--offline` is a supported mode.

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

Check: every test passes. These are not smoke tests. They include the two
signature-verification mistakes the platform documentation singles out as the
most common (hashing a re-encoded body, and decoding the secret with a
URL-safe alphabet), and they fail loudly if either is reintroduced.

Do not skip this step because the build succeeded. A guard that compiles and
mis-verifies signatures accepts requests from anyone.

## Step 3: understand what it will and will not block

Read this before you enable enforcement, because the defaults are deliberate.

On by default:

- `provenance`: a cited token whose signature fails, or that resolves to
  different bytes than the transcript claims. Deny codes `PROV_SIG` and
  `PROV_BYTES`.
- `freshness`: a cited reading that has drifted past its band threshold.
  Deny code `PROV_DRIFT`.

Off by default:

- `geo_restriction`: needs your own cell64 zone list to mean anything.
- `claim_gating`: denies on the ABSENCE of a citation, which will block
  ordinary conversation until you have measured the rate on your own traffic.

Never a denial, whatever you configure:

- A token this node has not cached. It is indistinguishable from a token
  minted by another responder, and blocking on it would deny honest agents for
  citing something you have not seen.

## Step 4: verify the failure semantics yourself

This is the step most integrations get wrong, so check it rather than assume
it.

A webhook failure is **not** a deny. If your server returns a non-200, times
out, or sends an unparseable body, the platform applies your organisation's
failure-handling setting instead of your verdict. Under "allow the request"
that means the prompt reaches the model uninspected.

So: your server must answer 200 with a well-formed verdict in every case,
including ones it does not understand. Confirm your deployment does by sending
a body it cannot evaluate and checking it still gets a verdict:

```bash
curl -sS -X POST http://127.0.0.1:8080/verdict/anthropic-hook \
  -H 'content-type: application/json' \
  -d '{"type":"some_event_type_that_does_not_exist_yet","request_id":"probe"}'
```

Check: HTTP 200 and `{"action":"allow"}`. If you get a 4xx or 5xx, your
deployment will trip the circuit breaker under real traffic and stop enforcing
entirely.

## Step 5: raise the body limit

Transcripts are sent untruncated, up to 10 MB. Several common defaults are far
smaller: nginx `client_max_body_size` is 1 MB, and Express `express.json()` is
100 kB. A rejected body counts as a webhook failure, so under allow-on-failure
an oversized prompt reaches the model uninspected.

If you are behind nginx:

```
client_max_body_size 10m;
```

Check: post a 9 MB body and confirm you still get a verdict rather than a 413.

## Step 6: point the checkpoint at it

For Claude Enterprise Inference hooks, an administrator sets your HTTPS URL in
the organisation configuration and presses Test connection. The test sends a
synthetic prompt with `source.application` of `config-test` and reports the
verdict you returned.

The first connection test arrives **unsigned**, because the signing secret does
not exist until the first save. Accept unsigned requests until your
administrator confirms the secret exists, then reject them.

For Claude Code, the client posts hook input and you block by returning **2xx
with a deny decision in the body**. A non-2xx is non-blocking. This is the
inverse of the usual HTTP intuition and it is the reach that matters: Claude
Code hooks cover agents on the Platform API, Bedrock and Vertex, none of which
Inference hooks can see.

## Step 7: rotation

Secret rotation is an immediate cutover on the platform side, but requests
signed with the previous secret keep arriving for about a minute, plus anything
already in flight. Configure both secrets during the switchover. A server
holding one secret drops those stragglers, and a dropped request is a webhook
failure.

## What "done" looks like

- `cargo test --release -p emem-guard` is green.
- An unknown event type returns 200 with an allow.
- A 9 MB body returns a verdict rather than a 413.
- Test connection reports your allow verdict.
- A tampered token in a transcript produces a deny whose reason parses as
  `EMEM-GUARD DENY <CODE> token=<token> fix=<fix> leaf=<leaf>`.

## The reason grammar, for agents

Denials are machine-first on purpose. The line your agent receives is:

```
EMEM-GUARD DENY PROV_SIG token=emem:fact:cell:cid fix=refresh_token leaf=leaf_01HX
```

`fix` is the actionable part. `refresh_token` means re-resolve and retry;
`remove_reference` means the citation cannot be made to verify; `contact_admin`
means a human restricted this, not the evidence. `leaf` is the log entry, which
you can fetch and verify without asking the server that issued it.

An agent that reads `fix` and acts on it starts carrying valid tokens. That is
the point of the gate.

## If you get stuck

Report the step number and the exact command output. Do not work around a
failing check: every check here exists because skipping it produces a server
that appears to enforce and does not.
