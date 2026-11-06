# Sleep-time agent (`emem-sleep-agent`)

_Plan reference: `docs/plans/v0.0.8-and-v0.0.9.md`, "Change 6: Sleep-time
agent loop". v0.0.9 connect-and-evolve lane._

The sleep-time agent is an **opt-in loop that uses an LLM to merge notes**.
It adds to the deterministic refinement loop that already ships in emem.

| Loop | What it does when it finds a problem |
|---|---|
| Deterministic refinement (`EMEM_REFINEMENT_ENABLED`) | Draws a `disagrees_with` edge between the facts that disagree. |
| Sleep-time agent | In idle periods, asks an LLM to reconcile the conflicting or duplicate notes into one cleaner note. It writes that note back and keeps the old versions. |

The agent ships as the standalone crate `crates/emem-sleep-agent`, with one
binary, `emem-sleep-agentd`. The crate links no emem internals. It drives a
running responder through the same public REST and MCP surface that any
external agent uses.

> **It cannot write to a default responder today.** The agent sends
> `emem_memory_create` with no `attester` block. A release build refuses
> every unattested note write with 401 `memory_attestation_required`. The
> operator must set `EMEM_MEMORY_OPEN=1` or `EMEM_MEMORY_HARDEN_DESTRUCTIVE=1`
> to accept such writes. emem.dev sets `EMEM_MEMORY_REQUIRE_ATTESTER=1`, so
> emem.dev refuses the agent's writes. On an open responder, the agent still
> cannot write under `/memories/by_attester/`. It also cannot write over an
> open-root path that another key created (403 `memory_namespace_violation`).
> Dry-run and candidate selection work on any responder. The agent cannot
> sign its writes with a persisted key: that part is not built.

## How a pass works

```
            ┌─────────────────────────── emem-sleep-agentd ───────────────────────────┐
            │                                                                          │
  POST /v1/memory_contradictions ─┐                                                    │
  POST /mcp emem_memory_list_by_kind ──┼─► select_candidates ─► cluster near-dups ─► top-N  │
  POST /mcp emem_memory_view ──────────┘                              │                     │
            │                                                    ▼                     │
            │                       LlmTransport.propose_merge(cluster)  (REAL call)   │
            │                                                    │                     │
            │                                                    ▼                     │
            │              POST /mcp emem_memory_create(canonical_path, merged_text)        │
            │              → new file_cid shadows old via bi-temporal supersession     │
            └──────────────────────────────────────────────────────────────────────┘
```

Each pass has three steps.

1. **Select candidates** (`candidates.rs`, pure functions with unit tests).
   The agent finds candidates in two ways:
   - Contradictions. `POST /v1/memory_contradictions` returns
     `(cell, band, tslot)` triples where two or more attesters disagree.
     Each triple has a severity in `[0,1]`. The agent does not send
     `include_same_attester_sources`, so the server default (`true`)
     applies. The scan reports two kinds of disagreement: two or more
     different attesters, and one attester that answered the same address
     from two different upstreams. A triple with a severity equal to or more than
     `EMEM_SLEEP_AGENT_MIN_SEVERITY` (default 0.3) is a candidate. A higher severity ranks the candidate
     higher.
   - Churn. The agent lists the notes of the configured kinds
     (`emem_memory_list_by_kind`). It groups them by a normalized path stem:
     `notes/site-1`, `notes/site-2` and `notes/site.md` all become
     `notes/site`. A group with two or more notes is a candidate. A group
     whose total version count reaches the churn threshold is also a
     candidate.

   The agent ranks contradictions above churn. It keeps the first
   `EMEM_SLEEP_AGENT_TOPN` candidates.

2. **Rewrite and merge** (`llm.rs`). The agent puts the candidate group into
   a reconcile prompt and sends it to the configured `LlmTransport`. The
   prompt tells the model to keep every distinct fact. It also tells the
   model to state each disagreement, and not to pick one side silently.

3. **Write back** (`merge.rs`). The agent adds a provenance trailer to the
   merged text. The trailer names each source as `path@file_cid`. The agent
   then sends the text with `emem_memory_create` to the canonical path of
   the group.

After each pass, the agent prints a `PassSummary`. It shows the mode, the
candidate counts by source, the merges written, the new CIDs, the spend and
the plan lines.

## Configure the LLM route

The merge call goes to a real model. The agent never fakes it. You can
configure one of two routes.

### OpenAI-compatible chat completions

```bash
export EMEM_SLEEP_AGENT_LLM_URL=https://api.openai.com/v1/chat/completions
export EMEM_SLEEP_AGENT_LLM_MODEL=gpt-4o-mini
export OPENAI_API_KEY=sk-...            # or EMEM_SLEEP_AGENT_LLM_API_KEY
```

The transport sends a standard `messages` body and reads
`choices[0].message.content`. The agent estimates the cost from the `usage`
token counts that the model returns. The rate is fixed in `llm.rs`: USD 0.50
for each million prompt tokens and USD 1.50 for each million completion
tokens. Only the budget cap uses this estimate. It never changes the text.

### The responder's own `/v1/ask` route

```bash
export EMEM_SLEEP_AGENT_USE_ASK=1
```

The agent sends the merge prompt to the `/v1/ask` endpoint of the
responder. The operator's responder configuration then chooses the model,
and you do not need a separate key. The agent records the cost as 0.0,
because the responder does its own metering.

`/v1/ask` answers questions about places. Nobody designed it to rewrite
notes. The transport uses the `answer` text, or else the `summary` text, as
the merge. When neither field has text, the transport refuses. Read the
merged text before you trust this route.

## Budget and loop caps

| Env var | Default | Meaning |
|---|---|---|
| `EMEM_SLEEP_AGENT` | _(unset)_ | `1` or `true` starts the long-running loop. Otherwise the agent runs one pass and exits. `--once` forces one pass. |
| `EMEM_SLEEP_AGENT_URL`, then `EMEM_URL` | `http://127.0.0.1:5051` | Responder base URL. The `--url` flag overrides both. |
| `EMEM_SLEEP_AGENT_INTERVAL_SECS` | 600 | Seconds between passes (minimum 1). |
| `EMEM_SLEEP_AGENT_TOPN` | 8 | Candidates per pass (minimum 1). |
| `EMEM_SLEEP_AGENT_BUDGET_USD` | 0.50 | Spend cap per pass, in USD. |
| `EMEM_SLEEP_AGENT_MIN_SEVERITY` | 0.3 | Contradiction severity floor. |
| `EMEM_SLEEP_AGENT_CHURN_THRESHOLD` | 3 | Version count that makes a path high-churn (minimum 2). |
| `EMEM_SLEEP_AGENT_CHURN_KINDS` | `semantic,episodic,resource` | Note kinds that the churn scan reads. |
| `EMEM_SLEEP_AGENT_ATTESTER` | `sleep_agent_v1` | The agent reads this value into its config. It does not send the value with any write, and it signs nothing. |

Before each LLM call, the agent compares the spend of the pass with the
cap. When the spend is equal to or more than the cap, the pass stops and
adds a note to the summary. The next pass selects candidates again. The
check happens before the call, so the last call of a pass can take the
spend above the cap.

## Old versions stay readable

This section applies when the responder accepts the write. An
`emem_memory_create` to an existing `path` does two things:

- It sets `memory_files[path]` to the new CID. `emem_memory_view` and any
  `as_of:now` read then return the merged text.
- It appends the new CID to the append-only `memory_file_history[path]`.
  You can still read each earlier version by its CID (`emem_memory_view`
  with `file_cid`) and replay it.

The agent deletes nothing. The provenance trailer of the merged note names
every source note, so an auditor can go back to the originals. This is the
bi-temporal shadow that the plan asks for. The newer note wins for
`as_of` now, and the originals stay replayable. The agent does not call
`emem_memory_supersede`. The source notes at other paths therefore carry no
`superseded_by` pointer to the merge.

## No invented rewrites

The agent never writes a rewrite when it has no real LLM to call.

- `emem-sleep-agentd --dry-run` connects, selects candidates, prints the
  merges that it would do, and exits 0. It makes **no** LLM call and **no**
  write. It works offline against a local responder. When the corpus has no
  candidates, the summary says "empty candidate set".
- In live mode with no transport configured, the agent runs as a dry run.
  It adds a note to the summary and writes nothing.

The live rewrite path is real code. Without a key, nothing runs it.

## Dry-run, end to end

These commands need a local Rust build. This page did not run them on
6 November 2026.

```bash
# Start a responder:
cargo build -p emem-cli --bin emem-server
EMEM_DATA=:memory: EMEM_BIND=127.0.0.1:5088 EMEM_OVERTURE_SKIP_WARMUP=1 \
  ./target/debug/emem-server &

# One dry pass: connects, selects, plans, writes nothing.
EMEM_URL=http://127.0.0.1:5088 \
  cargo run -p emem-sleep-agent --bin emem-sleep-agentd -- --dry-run
```

## Tests

```bash
cargo test -p emem-sleep-agent
```

- `merge::tests::mock_llm_merge_writes_superseding_memory` uses a mock
  `LlmTransport` that returns fixed merged text. It runs a full pass
  against a mock responder. It checks that the agent writes the merged
  text and provenance to the canonical path, with the inherited kind. It
  calls no real API.
- `merge::tests::dry_run_selects_but_never_writes` checks that a dry run
  selects a candidate but makes no LLM call and no write. This is true
  even when a transport is available.
- `candidates::tests::*` test candidate selection over a JSON fixture:
  churn grouping, the severity floor, ranking and an empty corpus.
