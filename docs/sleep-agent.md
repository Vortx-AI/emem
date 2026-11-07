# Sleep-time agent (`emem-sleep-agent`)

_Plan reference: `docs/plans/v0.0.8-and-v0.0.9.md`, "Change 6: Sleep-time
agent loop". v0.0.9 connect-and-evolve lane._

The sleep-time agent is an **opt-in loop that uses an LLM to merge notes**.
It adds to the deterministic refinement loop that already ships in emem.

| Loop | What it does when it finds a problem |
|---|---|
| Deterministic refinement (`EMEM_REFINEMENT_ENABLED`) | Draws a `disagrees_with` edge between the facts that disagree. |
| Sleep-time agent | In idle periods, asks an LLM to merge duplicate notes or to describe a contradiction. It writes the result as a new note and keeps the old versions. |

The agent ships as the standalone crate `crates/emem-sleep-agent`, with one
binary, `emem-sleep-agentd`. The crate links no emem internals. It drives a
running responder through the same public REST and MCP surface that any
external agent uses.

> **It cannot write to a default responder today.** The agent sends
> `emem_memory_create` with no `attester` block. A release build refuses
> every unattested note write with `memory_attestation_required`. The
> operator must set `EMEM_MEMORY_OPEN=1` or `EMEM_MEMORY_HARDEN_DESTRUCTIVE=1`
> to accept such writes. emem.dev sets `EMEM_MEMORY_REQUIRE_ATTESTER=1`, so
> emem.dev refuses the agent's writes. On an open responder, the agent still
> cannot write under `/memories/by_attester/`. It also cannot write over an
> open-root path that another key created (`memory_namespace_violation`).
> Both refusals arrive as an MCP tool result with `isError: true`, not as
> an HTTP status. See [Write your first signed note](memory.md#write-your-first-signed-note).
> Dry-run and candidate selection work on any responder. The agent cannot
> sign its writes with a persisted key: the crate has no code for that yet.

## How a pass works

```
emem-sleep-agentd, one pass

  POST /v1/memory_contradictions -----+
  POST /mcp emem_memory_list_by_kind -+--> select candidates --> keep top N
  POST /mcp emem_memory_view ---------+            |
                                                   v
                         LlmTransport.propose_merge (a real model call)
                                                   |
                                                   v
                POST /mcp emem_memory_create (canonical path, merged text)
                the new file_cid wins for as_of now; old versions stay
```

Each pass has three steps.

1. **Select candidates** (`candidates.rs`, pure functions with unit tests).
   The agent finds candidates in two ways:
   - Contradictions. `POST /v1/memory_contradictions` returns
     `(cell, band, tslot)` triples where two or more attesters disagree.
     Each triple has a severity in `[0,1]`. [Contradiction detection](memory.md#contradiction-detection)
     describes the scan. The agent sends this request (`client.rs`,
     `candidates.rs`):

     | Field | What the agent sends | Effect |
     |---|---|---|
     | `min_severity` | `EMEM_SLEEP_AGENT_MIN_SEVERITY` (default 0.3) | A triple with this severity or more is a candidate. |
     | `limit` | `EMEM_SLEEP_AGENT_TOPN` or 8, whichever is larger | The server returns the highest severities first. |
     | `cell_prefix` | not sent | The scan starts at the start of the whole corpus. |
     | `include_same_attester_sources` | not sent | The server default (`true`) applies. |
     | `resume_after` | not sent, and not read from the response | The agent sees only the keys scanned before the time budget ends. |

     Thus the scan reports two kinds of disagreement: two or more
     different attesters, and one attester that answered the same address
     from two different upstreams. A higher severity ranks the candidate
     higher. On 7 November 2026 at 08:15 UTC, this request to emem.dev
     scanned 44,705 keys in 8.0 s and returned `scan_truncated: true`.
   - Churn. The agent lists the notes of the configured kinds
     (`emem_memory_list_by_kind`). It groups them by a normalized path stem.
     The stem keeps the parent path. It cuts the last path part at its
     first dot. Then trailing digits, trailing `-`, `_` or spaces, and a
     trailing `(copy)` come off (`normalize_stem` in `lib.rs`). Thus
     `notes/site-1`, `notes/site-2` and `notes/site.md` all become
     `notes/site`. The first dot is not always the extension:
     `notes/v1.2-final` becomes `notes/v`.
     A group with two or more notes is a candidate. A group
     whose total version count reaches the churn threshold is also a
     candidate. The agent does not read the version history of a note. It
     counts each note as one version (`history_len: 1` in `candidates.rs`).
     Thus only a group of two or more notes becomes a candidate, and
     `EMEM_SLEEP_AGENT_CHURN_THRESHOLD` has no effect today. When the
     responder refuses a list for one kind, or a view for one note, the
     agent skips it. The pass summary does not show the skip.

   The agent ranks candidates by a score (`candidates.rs`). A
   contradiction scores `1 + severity`, so at least 1.3 with the default
   minimum. A churn group scores `0.5 + 0.01 × n`, where `n` is the number
   of notes. Thus contradictions usually rank first, but a churn group of
   more than 80 notes outranks a contradiction of severity 0.3. The agent
   keeps the first `EMEM_SLEEP_AGENT_TOPN` candidates.

2. **Rewrite and merge** (`llm.rs`). The agent puts the candidate group into
   a reconcile prompt and sends it to the configured `LlmTransport`. The
   prompt tells the model to keep every distinct fact. It also tells the
   model to state each disagreement, and not to pick one side silently.

3. **Write back** (`merge.rs`). The agent adds a provenance trailer to the
   merged text. The trailer names each source as `path@file_cid`. The agent
   then sends the text with `emem_memory_create` to the canonical path of
   the group. For a churn group, the canonical path is the first member
   path in sort order. The other members stay as they are.

   A contradiction candidate carries no note. Its canonical path is
   `/memories/reconciled/<cell>/<band>`. The prompt gives the model only
   that path, not the values that disagree. The agent writes the result to
   that path as kind `semantic`, with no provenance trailer. Read such a
   note as text from a model, not as a check of the facts. The path has
   no tslot. Two triples with the same cell and band but different tslots
   therefore write to one path, and the later write becomes the current
   version.

After each pass, the agent prints a `PassSummary`. It shows the mode, the
candidate counts by source, the merges written, the new CIDs, the spend and
the plan lines.

## Configure the LLM route

The merge call goes to a real model. The agent never fakes it. You can
configure one of two routes. When you set both, the `/v1/ask` route wins
(`config.rs`, `resolve_llm_route`). When you set neither, the agent runs
dry.

### OpenAI-compatible chat completions

```bash
export EMEM_SLEEP_AGENT_LLM_URL=https://api.openai.com/v1/chat/completions
export EMEM_SLEEP_AGENT_LLM_MODEL=gpt-4o-mini
export OPENAI_API_KEY=sk-...            # or EMEM_SLEEP_AGENT_LLM_API_KEY
```

This route needs all three values: the URL, the model and a key. An empty
URL or model counts as not set. If `OPENAI_API_KEY` and
`EMEM_SLEEP_AGENT_LLM_API_KEY` both exist, the agent uses `OPENAI_API_KEY`.
This is also true when `OPENAI_API_KEY` is empty. Then the agent does not
read `EMEM_SLEEP_AGENT_LLM_API_KEY`, and it runs without an LLM.

The transport sends a `POST` with the key as a bearer token. The body has
the model, a system message, a user message and `temperature: 0.2`. The
transport reads `choices[0].message.content` and trims it. A status that
is not a success, or empty text, is an error. The agent estimates the cost from the `usage`
token counts that the model returns. `llm.rs` sets a fixed rate: USD 0.50
for each million prompt tokens and USD 1.50 for each million completion
tokens. Only the budget cap uses this estimate. It never changes the text.

### The responder's own `/v1/ask` route

```bash
export EMEM_SLEEP_AGENT_USE_ASK=1
```

The agent also accepts `true`, in any letter case.

The agent sends the merge prompt to the `/v1/ask` endpoint of the
responder that it reads notes from. The body is `{"question": ...}`, with
the system text and the user text joined by a blank line. `/v1/ask`
accepts `question` as another name for `q`. The operator's responder configuration then chooses the model,
and you do not need a separate key. The agent records the cost as 0.0,
because the responder does its own metering.

`/v1/ask` answers questions about places. Nobody designed it to rewrite
notes. The transport uses the `answer` text as the merge. When the response
has no `answer` field, the transport uses the `summary` text. When it gets
no text, the transport refuses, and the pass stops (see
[When a call fails](#when-a-call-fails)). An `answer` string that is empty
or only spaces also gives a refusal, even when `summary` has text.

On 7 November 2026, emem.dev did not answer a merge request. We sent
`POST /v1/ask` with a two-note merge question. The response had
`status: needs_location` and no `answer` or `summary` field. With that
response, the transport refuses. A question that resolves to a place gets
an `answer`, but that text describes the place, not a merge. Read the
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

The agent does not report a value that it cannot parse. It uses the
default for that variable. For example, `EMEM_SLEEP_AGENT_TOPN=eight` gives
8 candidates.

Each pass makes one `emem_memory_view` call for each listed note. The
agent lists up to 256 notes for each kind in `EMEM_SLEEP_AGENT_CHURN_KINDS`.
With the three default kinds, one dry pass sends up to 772 calls: one
contradiction scan, three lists and 768 views. Point the agent at your own
responder, not at a shared public one.

Before the first pass, the agent sends `GET` to `/v1/health`, then
`/health`, then `/`. If no probe returns a success status, the agent exits
with code 2. In single-pass mode, a failed pass exits with code 1 and a
completed pass exits with code 0. In loop mode, the agent logs a failed
pass and tries again at the next interval. The loop never stops by
itself. `run_loop` (`lib.rs`) stops only for a `SleepAgentError::Config`
error, and no code in the crate makes that error today. To stop the
loop, stop the process.

Before each LLM call, the agent compares the spend of the pass with the
cap. When the spend is equal to or more than the cap, the pass stops and
adds a note to the summary. The next pass selects candidates again. The
check happens before the call, so the last call of a pass can take the
spend above the cap.

## When a call fails

One failed LLM call or one refused write stops the whole pass
(`run_pass` in `lib.rs` returns the error with `?`). The agent does not
skip to the next candidate. A failed contradiction scan also stops the
pass, in a dry run too. A failed list or view does not (see step 1).

- Merges that the pass wrote before the error stay written.
- The agent prints the error, not the `PassSummary`. The message starts
  with `LLM transport error:` or `responder transport error:`.
- A single pass then exits with code 1.
- In loop mode, the next pass selects the same candidates in the same
  order. If the first candidate fails each time, no later candidate gets
  a merge.

A responder that refuses unattested writes, such as emem.dev, refuses the
write of the first candidate (see the note at the top of this page). With
a paid LLM route, each pass then pays for one model call and writes
nothing.

## Old versions stay readable

This section applies when the responder accepts the write. An
`emem_memory_create` to an existing `path` does two things:

- It sets `memory_files[path]` to the new CID. `emem_memory_view` and any
  `as_of:now` read then return the merged text.
- It appends the new CID to the append-only `memory_file_history[path]`.
  You can still read each earlier version by its CID (`emem_memory_view`
  with `file_cid`) and replay it.

The agent deletes nothing. The provenance trailer of the merged note names
every source note, so an auditor can go back to the originals. The plan
(Change 6) asks that the originals stay readable behind the newer version.
The path history does this. The newer note wins for `as_of` now, and the
originals stay replayable. The agent does not call
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

These commands need a local Rust build. This page did not run these commands.

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

The crate has 15 unit tests. None of them calls a real LLM or emem.dev.

| Test | What it checks |
|---|---|
| `merge::tests::mock_llm_merge_writes_superseding_memory` | A full pass with a mock `LlmTransport` and a mock responder. The agent writes the merged text and the provenance trailer to the canonical path, with the inherited kind. |
| `merge::tests::dry_run_selects_but_never_writes` | A dry run selects a candidate and prints a plan, but makes no LLM call and no write, even when a transport is available. |
| `merge::tests::provenance_trailer_is_appended_not_replacing` | It builds a cluster of two notes and asserts that the cluster has two notes and that the first CID is `cid-n/x-1`. It does not call `apply_merge`, so only the first test checks the trailer text. |
| `candidates::tests::*` (4 tests) | Candidate selection over a JSON fixture: churn grouping, the severity floor, contradiction before churn, and an empty corpus. |
| `lib::tests::*` (3 tests) | `normalize_stem`, the stem groups, and that the pass summary says `DRY-RUN`. |
| `llm::tests::*` (4 tests) | The cost estimate from `usage`, and zero cost with no `usage`. No transport when the config names no LLM. The merge prompt holds every cluster entry. |
| `config::tests::defaults_when_env_absent` | An explicit dry run and URL give a config with `top_n` of 1 or more and a budget above 0. |
