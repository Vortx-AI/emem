# emem-sleep-agent

`emem-sleep-agentd` is an opt-in background worker. It finds groups of
memory notes with near-duplicate names, and places where signed facts
disagree. It asks an LLM to merge each group into one note, then writes
that note back as a new version.
Operators who run their own emem responder can use it to keep memory
notes tidy. It is off by default.

Try it without an LLM key and without writes:

```bash
EMEM_URL=http://127.0.0.1:5088 \
  cargo run -p emem-sleep-agent --bin emem-sleep-agentd -- --dry-run
```

This needs a local responder; [Run a dry pass](#run-a-dry-pass) shows
how to start one. We did not run these commands on 7 November 2026,
because the docs host has no Rust toolchain. They follow
`src/bin/emem-sleep-agentd.rs` and `src/config.rs`.

The full design is in [docs/sleep-agent.md](../../docs/sleep-agent.md).

## What works today and what does not

| Part | Status |
|---|---|
| Candidate selection and `--dry-run` | Works against any responder. It only reads. |
| LLM merge call | Real code. It runs only when you configure an LLM route. |
| Write-back of the merged note | Refused by a default responder. See below. |
| Signed writes | Not built. The agent has no key and signs nothing. |

The agent sends `emem_memory_create` with no `attester` block. A release
build of the responder refuses every unsigned memory write (401
`memory_attestation_required`). It accepts one only when the operator sets
`EMEM_MEMORY_OPEN=1` or `EMEM_MEMORY_HARDEN_DESTRUCTIVE=1`. Any value
other than `0`, `false` or empty turns a switch on.
`EMEM_MEMORY_REQUIRE_ATTESTER` overrides both switches and refuses every
unsigned write again. The policy is in `memory_write_policy` in
`crates/emem-api-rest/src/lib.rs`.

Even on an open responder, some paths stay closed to the agent:

| Path | Answer |
|---|---|
| under `/memories/by_attester/` | 401 `memory_attestation_required`, because the write has no signature |
| under `/memories/.well-known/` | 403 `memory_namespace_violation`, for every key |
| an open-namespace path that another key created | 403 `memory_namespace_violation` |

emem.dev requires an attester on every memory write, so it refuses the
merges. The responder signs a receipt for each write it accepts. The
note itself carries no signature from the agent.

## How a pass works

The worker drives a running responder over its public REST and MCP API.
It links no emem crate, and no other crate depends on it.

1. Select candidates. The agent reads `POST /v1/memory_contradictions`.
   Each result is a (cell, band, tslot) where signed facts disagree. For
   each one, the agent makes a candidate at
   `/memories/reconciled/<cell>/<band>` with no source note. It also
   groups paths with near-duplicate names from `emem_memory_list_by_kind`.
   Contradictions rank above name groups.
2. Merge. The agent asks the configured LLM for one note per group. The
   note must keep every distinct fact and state each disagreement.
3. Write back. The agent writes the merged note to the group's canonical
   path with `emem_memory_create`. The note ends with a provenance trailer
   that names the `path@file_cid` of each source.

On 7 November 2026, `POST https://emem.dev/v1/memory_contradictions` with
`{"min_severity":0.3,"limit":8}` returned 8 contradictions. It scanned
44,702 keys. The band in the first result was
`overture.transportation.road_length_m`. The response had
`scan_truncated: true` and a `resume_after` key, so it was a partial view.
The agent does not send `resume_after`. Each pass reads only the first
page.

This crate adds an LLM step on top of the deterministic refinement loop
(`EMEM_REFINEMENT_ENABLED`). That loop draws a `disagrees_with` edge for
each contradiction and changes no note.

## Run a dry pass

A dry pass selects candidates and prints the merge plan. It makes no LLM
call and no write. It exits 0 on an empty corpus and says so.

```bash
# Start a responder with an in-memory store.
cargo build -p emem-cli --bin emem-server
EMEM_DATA=:memory: EMEM_BIND=127.0.0.1:5088 EMEM_OVERTURE_SKIP_WARMUP=1 \
  ./target/debug/emem-server &

# One dry pass.
EMEM_URL=http://127.0.0.1:5088 \
  cargo run -p emem-sleep-agent --bin emem-sleep-agentd -- --dry-run
```

The binary has three flags:

| Flag | Effect |
|---|---|
| `--dry-run` | Select and plan only. No LLM call, no write. |
| `--url <URL>` | Responder base URL. Overrides `EMEM_SLEEP_AGENT_URL` and `EMEM_URL`. |
| `--once` | Run one pass and exit, even when `EMEM_SLEEP_AGENT=1`. |

Without `EMEM_SLEEP_AGENT=1` the binary runs one pass and exits. Set
`EMEM_SLEEP_AGENT=1` to run a pass at each interval. If the responder
does not answer a health probe, the binary exits with code 2.

## Configure the LLM route

Pick one of two routes. If you configure neither, the agent makes no
LLM call and runs as a dry pass. It never invents text.

An OpenAI-compatible chat-completions endpoint needs all three variables:

```bash
export EMEM_SLEEP_AGENT_LLM_URL=https://api.openai.com/v1/chat/completions
export EMEM_SLEEP_AGENT_LLM_MODEL=gpt-4o-mini
export OPENAI_API_KEY=sk-...   # or EMEM_SLEEP_AGENT_LLM_API_KEY
```

The other route sends the merge prompt to the responder's own `/v1/ask`:

```bash
export EMEM_SLEEP_AGENT_USE_ASK=1
```

`/v1/ask` answers place questions. Nobody designed it to rewrite notes. The
agent uses its `answer` or `summary` text as the merge. Read that output
before you trust this route.

## Settings

| Env var | Default | Meaning |
|---|---|---|
| `EMEM_SLEEP_AGENT` | unset | `1` or `true` runs passes in a loop |
| `EMEM_SLEEP_AGENT_URL`, `EMEM_URL` | `http://127.0.0.1:5051` | responder base URL |
| `EMEM_SLEEP_AGENT_INTERVAL_SECS` | 600 | seconds between passes |
| `EMEM_SLEEP_AGENT_TOPN` | 8 | candidates per pass (minimum 1) |
| `EMEM_SLEEP_AGENT_BUDGET_USD` | 0.50 | spend limit per pass |
| `EMEM_SLEEP_AGENT_MIN_SEVERITY` | 0.3 | lowest contradiction severity that counts |
| `EMEM_SLEEP_AGENT_CHURN_THRESHOLD` | 3 | no effect yet. The agent counts every path as one version, so the threshold never decides (minimum 2) |
| `EMEM_SLEEP_AGENT_CHURN_KINDS` | `semantic,episodic,resource` | memory kinds scanned for churn |
| `EMEM_SLEEP_AGENT_ATTESTER` | `sleep_agent_v1` | read into the config, but no write sends it |

The agent checks the budget before each LLM call. It starts no new call
after the estimated spend reaches the limit. The last call can exceed
the limit. The cost is an estimate from the
token counts that the endpoint reports, at $0.50 per million input tokens
and $1.50 per million output tokens. The `/v1/ask` route reports a cost
of 0.

## What a merge keeps

When the responder accepts a write to an existing path:

- `emem_memory_view` on that path returns the merged note.
- Each earlier version stays in the path's history and is readable by its
  `file_cid`.

The agent deletes nothing. It does not call `emem_memory_supersede`, so
the source notes at other paths get no `superseded_by` link to the merge.

## Test

```bash
cargo test -p emem-sleep-agent
```

The crate has 15 tests, all inline in `src/`. The merge test uses a mock
LLM transport and a mock responder, so it calls no real API. The candidate
selection tests use a small JSON fixture.
