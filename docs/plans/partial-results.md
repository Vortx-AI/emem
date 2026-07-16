# Partial results: the answer is the store, not a job queue

**Status: design for owner sign-off, 2026-07-16. Nothing here ships.
This is the deliberate answer the roadmap's partial-results item said
the API change deserves, and it gates three other items: the memory
preparer, scheduled densification, and the channel's retryable-errors
ask.**

## The problem, restated from the roadmap

A cold polygon cannot finish inside the 40 second gateway: worst case
is roughly 31 sequential upstream round trips for one cell, and
`recall_polygon` runs 64 cells in waves. An agent handed 40 of 64
cells plus a list of what is still materializing can proceed; a 504
teaches it to stop calling emem. Bounding the materializer shrinks the
window but does not change the shape.

## The insight the design turns on

Materialization PERSISTS. Every cell a request warms is signed and
stored before the response goes out, so the identical request, retried,
returns strictly more cells from cache and strictly fewer from
upstream. That makes convergence a property of the store, not of a job
scheduler: there is nothing to enqueue, no job id to mint, no state to
reap, and nothing that breaks when the responder restarts mid-way. The
"resume token" agents ask for is the request they already hold.

## The design

### 1. One request knob

Area reads (`recall_polygon`, `recall_many`, and `emem_backfill` when
the preparer lands) accept `budget_ms`, an optional soft compute
budget. Absent, behaviour is unchanged. Present, the responder fills
cells until the budget expires, then answers with what it has. The
budget bounds materialization work, not response assembly; a fully
warm request ignores it.

### 2. The partial response is a first-class 200

```jsonc
{
  "facts": [ /* every fact that is ready, signed as always */ ],
  "receipt": { /* binds exactly the returned facts, nothing more */ },
  "converged": false,
  "pending": [
    { "cell": "defi...", "band": "indices.ndvi",
      "state": "materializing",           // closed set, below
      "reason": "upstream fetch in flight" }
  ],
  "progress": { "ready": 40, "pending": 24, "total": 64 },
  "retry": {
    "how": "repeat the identical request; each retry returns strictly more from cache",
    "suggested_after_ms": 4000
  }
}
```

The `pending[]` states are a closed set, append-only:
`materializing` (an upstream fetch this request started),
`budget_exhausted` (not attempted this pass), and
`upstream_failed` (attempted, failed, will be retried; carries the
typed upstream error). This folds the agent channel's W6 ask
(retryable errors with a remedy) into the same shape: every pending
entry IS the remedy, stated.

### 3. What the receipt attests, said precisely

The receipt binds the facts RETURNED, exactly as today, and nothing
about `pending`. A pending entry is advisory and unsigned, because it
is a statement about the responder's work queue, not about the world.
In particular a pending cell is NOT a signed absence: absence means
"the source was consulted and there is nothing there", signed;
pending means "not consulted yet", unsigned. Conflating them would
poison the absence semantics that recall relies on, so the design
keeps them disjoint by type, not by convention.

### 4. Convergence is monotone, and stated

`converged: true` appears when `pending` is empty. Two properties hold
by construction and are documented as guarantees: retries never lose
ground (persistence), and the same request converges to the same fact
set regardless of how many retries it took (content addressing). An
agent's loop is therefore three lines: call, use `facts`, retry later
if `converged` is false.

### 5. What this unlocks, in order

1. **The memory preparer**: `emem_backfill` gains the same
   `budget_ms` + `pending` contract over a cell list times a window,
   and "warm this area before I reason over it" becomes a loop over
   one idempotent call. No new tool.
2. **Scheduled densification**: the supply-side warmer is the
   preparer on a timer with a declared priority list. Its honesty
   requirement (never pretend to be synchronous) is exactly the
   partial contract.
3. **The fetch verbs stay out of the core 14**: with partiality
   first-class, the core loop's `emem_recall` already communicates
   cold-start honestly, and none of the fetch verbs needs promoting.

### 6. What this deliberately does not do

- No job ids, no `/v1/jobs/*` surface, no server-side cursors. The
  store is the state; anything else is a second source of truth that
  can disagree with it.
- No streaming (SSE progress) in v1. The retry loop above is enough
  for agents; a stream is ergonomics that can ride later without
  changing the contract.
- No change to single-cell recall. One cell either answers inside the
  gateway or fails with today's typed error; partiality is an
  area-read concept.

## Build order, once signed off

1. `budget_ms` + `pending[]` + `converged` on `recall_polygon`
   (the surface whose 504 taught agents to leave).
2. The same contract on `recall_many`.
3. `emem_backfill` grows the cell-list form: the preparer.
4. The densification warmer, riding the preparer on a schedule.
5. Docs flip in the same commits, as always.
