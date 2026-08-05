# Sleep-time agent (`emem-sleep-agent`)

_Plan reference: `docs/plans/v0.0.8-and-v0.0.9.md`, "Change 6: Sleep-time
agent loop". v0.0.9 connect-and-evolve lane._

The sleep-time agent is the **opt-in LLM rewrite/merge loop** that layers
on top of the deterministic refinement that already ships in emem. The
deterministic loop (`EMEM_REFINEMENT_ENABLED`) reacts to a contradiction by
drawing a `disagrees_with` edge; the sleep-time agent goes one step further
and, during idle periods, asks an LLM to actually *reconcile* the
conflicting / duplicated memory into a single cleaner note, then writes
that note back non-destructively.

It ships as the standalone crate `crates/emem-sleep-agent` with one binary,
`emem-sleep-agentd`. The crate links no emem internals: it drives a running
responder over the same public REST + MCP surface any external agent uses.

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

Each pass:

1. **Candidate selection** (`candidates.rs`, pure + unit-tested):
   - **Contradictions**: `POST /v1/memory_contradictions` returns
     `(cell, band, tslot)` triples where ≥2 attesters disagree, each with a
     severity in `[0,1]`. Anything above `EMEM_SLEEP_AGENT_MIN_SEVERITY`
     (default 0.3) is a candidate; severity scores its urgency.
   - **Churn**: memory files of the configured kinds are enumerated
     (`emem_memory_list_by_kind`) and clustered by a normalized path stem
     (`notes/site-1`, `notes/site-2`, `notes/site.md` → `notes/site`).
     A cluster with ≥2 members, or a single path rewritten past the churn
     threshold, is a candidate.
   - Candidates are ranked (contradictions outrank churn) and truncated to
     `EMEM_SLEEP_AGENT_TOPN`.

2. **Rewrite + merge** (`llm.rs`): the candidate cluster is rendered into a
   reconcile prompt and handed to the configured `LlmTransport`. The
   instruction preserves every distinct fact and states disagreements
   explicitly rather than silently picking one.

3. **Write-back** (`merge.rs`): the merged text (plus a provenance trailer
   naming each source `path@file_cid`) is written via `emem_memory_create` to the
   cluster's canonical path.

A `PassSummary` is printed per pass: mode, candidate counts by source,
merges written, new CIDs, spend, and the plan lines.

## LLM gateway configuration

The merge call is **real**, never faked. Two configurable routes:

### OpenAI-compatible chat-completions

```bash
export EMEM_SLEEP_AGENT_LLM_URL=https://api.openai.com/v1/chat/completions
export EMEM_SLEEP_AGENT_LLM_MODEL=gpt-4o-mini
export OPENAI_API_KEY=sk-...            # or EMEM_SLEEP_AGENT_LLM_API_KEY
```

The transport POSTs a standard `messages` body and reads
`choices[0].message.content`. Cost is estimated from reported `usage`
tokens at a conservative blended small-model rate; the estimate only drives
the budget cap, it never alters the text.

### Responder's own `/v1/ask` gateway

```bash
export EMEM_SLEEP_AGENT_USE_ASK=1
```

The merge prompt is routed through the responder's `/v1/ask`, so the model
choice follows the operator's responder configuration rather than a
separately-pinned key. Cost is reported as 0.0 (the responder owns metering).

## Budget and loop caps

| Env var | Default | Meaning |
|---|---|---|
| `EMEM_SLEEP_AGENT` | _(unset)_ | `1` enables the long-running loop; otherwise one pass and exit |
| `EMEM_URL` / `EMEM_SLEEP_AGENT_URL` | `http://127.0.0.1:5051` | responder base URL |
| `EMEM_SLEEP_AGENT_INTERVAL_SECS` | 600 | seconds between passes |
| `EMEM_SLEEP_AGENT_TOPN` | 8 | candidates per pass |
| `EMEM_SLEEP_AGENT_BUDGET_USD` | 0.50 | per-pass USD cap, checked before each call |
| `EMEM_SLEEP_AGENT_MIN_SEVERITY` | 0.3 | contradiction severity floor |
| `EMEM_SLEEP_AGENT_CHURN_THRESHOLD` | 3 | versions before a path is high-churn |
| `EMEM_SLEEP_AGENT_CHURN_KINDS` | `semantic,episodic,resource` | kinds scanned for churn |
| `EMEM_SLEEP_AGENT_ATTESTER` | `sleep_agent_v1` | attester label the merge is written under |

The budget cap is evaluated **before** each LLM call, so a pass never starts
a call that would breach it; the remaining candidates are deferred to the
next pass with a logged note.

## Non-destructive supersession guarantee

`emem_memory_create` to an existing `path`:

- updates `memory_files[path] → new_cid`, so `emem_memory_view` and any
  `as_of:now` read return the merged text, and
- appends `new_cid` to the append-only `memory_file_history[path]`, so the
  prior versions are still resolvable by CID and replayable.

Nothing is deleted. The merged entry's provenance trailer records every
source folded in, so an auditor can walk back to the originals. This is the
exact bi-temporal shadow the plan calls for: the newer entry wins under
`as_of` now, the originals stay replayable.

## Honesty: no fabrication

The agent never invents a rewrite when it has no real LLM to call:

- `emem-sleep-agentd --dry-run`: connect, select candidates, print exactly
  what would be merged, and exit 0. **No** LLM call, **no** write. Works
  fully offline against a local responder. An empty corpus is reported
  honestly ("empty candidate set") rather than silently passing.
- Live mode with no transport configured degrades to dry-run with a logged
  note instead of fabricating output.

The real rewrite path is real code; it is simply unexercised without a key.

## Dry-run, end to end

```bash
# Boot a responder:
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

- `merge::tests::mock_llm_merge_writes_superseding_memory`: injects a mock
  `LlmTransport` returning fixed merged text, runs a full pass against a
  mock responder, and asserts the agent writes the merged text + provenance
  to the canonical path with the inherited kind. No real API.
- `merge::tests::dry_run_selects_but_never_writes`: dry-run selects a
  candidate but performs no LLM call and no write even when a transport is
  available.
- `candidates::tests::*`: candidate selection over a JSON fixture
  (churn clustering, severity floor, ranking, empty corpus).
