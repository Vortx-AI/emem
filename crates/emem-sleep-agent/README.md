# emem-sleep-agent

The opt-in **sleep-time agent** for emem: an LLM rewrite/merge loop that
layers on top of the deterministic contradiction→`disagrees_with`-edge
refinement loop that already ships (`EMEM_REFINEMENT_ENABLED`).

During idle periods a background worker (`emem-sleep-agentd`, **default
OFF**) drives a running emem responder over its public REST + MCP API:

1. **Select candidates** — top-N memory paths ranked by multi-attester
   contradiction severity (`POST /v1/memory_contradictions`) and/or by
   write-churn (clusters of near-duplicate memory paths).
2. **Rewrite + merge** — for each candidate cluster, ask an
   operator-configured LLM to propose one reconciled memory.
3. **Write back non-destructively** — the merged text is written as a NEW
   signed memory via `memory_create`. emem's bi-temporal model means the
   new write *shadows* the old (latest `signed_at` wins under `as_of:now`)
   while every prior version stays in the append-only
   `memory_file_history` and is still resolvable by CID.

This crate is **standalone**: it links no emem internals, it only speaks
the same wire any agent does. It modifies no other crate.

## Enable

```bash
# One-shot pass (no loop) against a local responder:
EMEM_URL=http://127.0.0.1:5051 emem-sleep-agentd

# Long-running loop:
EMEM_SLEEP_AGENT=1 EMEM_URL=http://127.0.0.1:5051 emem-sleep-agentd
```

## LLM gateway (configurable, real)

Two routes; pick one. If **neither** is configured the agent refuses to
fabricate and runs dry (see Honesty below).

**OpenAI-compatible endpoint:**

```bash
export EMEM_SLEEP_AGENT_LLM_URL=https://api.openai.com/v1/chat/completions
export EMEM_SLEEP_AGENT_LLM_MODEL=gpt-4o-mini
export OPENAI_API_KEY=sk-...
```

**Responder's own `/v1/ask` gateway** (model follows the responder config):

```bash
export EMEM_SLEEP_AGENT_USE_ASK=1
```

## Budget + loop caps

| Env var | Default | Meaning |
|---|---|---|
| `EMEM_SLEEP_AGENT_INTERVAL_SECS` | 600 | seconds between passes |
| `EMEM_SLEEP_AGENT_TOPN` | 8 | candidates considered per pass |
| `EMEM_SLEEP_AGENT_BUDGET_USD` | 0.50 | per-pass spend cap (stops early) |
| `EMEM_SLEEP_AGENT_MIN_SEVERITY` | 0.3 | contradiction floor |
| `EMEM_SLEEP_AGENT_CHURN_THRESHOLD` | 3 | versions before a path is high-churn |
| `EMEM_SLEEP_AGENT_CHURN_KINDS` | semantic,episodic,resource | kinds scanned for churn |
| `EMEM_SLEEP_AGENT_ATTESTER` | `sleep_agent_v1` | attester label for merges |

The budget cap is checked **before** each LLM call, so a pass never starts a
call that would breach it.

## Non-destructive supersession guarantee

`memory_create` on an existing `path` is last-write-wins on the path **and**
append-only on history. Writing the merged text:

- updates `memory_files[path] → new_cid` (so `as_of:now` returns the merge),
- appends `new_cid` to `memory_file_history[path]` (so every prior version
  is still resolvable and replayable).

Originals are **never deleted**. The merged entry carries a provenance
trailer naming the `path@file_cid` of every source it folded in.

## Honesty — no fabrication

This project forbids stubs and fake data. If no LLM endpoint is configured,
or it is unreachable, the agent **does not invent a rewrite**. Instead:

- `--dry-run` (flag) — select candidates and print exactly what would be
  merged. No LLM call, no write. Works fully offline against a local
  responder. Exits 0 even on an empty corpus (it says so honestly).
- When live mode is requested but no transport resolves, the pass degrades
  to dry-run with a logged note rather than fabricating.

The real rewrite path is real code; it is simply unexercised without a key.

## Test

```bash
cargo test -p emem-sleep-agent
```

The merge-application test injects a **mock** `LlmTransport` (fixed merged
text) and asserts the agent writes the right superseding memory against a
mock responder — no real API is called. Candidate selection is unit-tested
against a small JSON fixture.

See [`docs/sleep-agent.md`](../../docs/sleep-agent.md) for the full design.
