# Partial results: the store holds the state, not a job queue

This page is a design record. The owner signed off the design on
16 July 2026, and all five build steps shipped the same day. The current
reference for each route is [docs/agents.md](../agents.md) and
`/openapi.json` on emem.dev.

## Status on 7 November 2026

The code is in `crates/emem-api-rest/src/lib.rs`. The examples below ran
on emem.dev on 7 November 2026.

| Item | Status | Where |
|---|---|---|
| `budget_ms`, `pending[]`, `converged` on an area read | Ships | `POST /v1/recall_polygon`, MCP `emem_recall_polygon` |
| The same contract on a cell list | Ships | `POST /v1/recall_many`, MCP `emem_recall_many` |
| The same contract on a grid | Ships | `POST /v1/grid`, MCP `emem_grid` |
| The preparer: backfill over a cell list | Ships | `POST /v1/backfill` with `cells`, MCP `emem_backfill` |
| The densification warmer | Ships, off by default | `EMEM_WARM_INTERVAL_SECS` and `warm_priority.json` |
| Job ids, `/v1/jobs/*`, server-side cursors | Not built, by design | See [What the design leaves out](#what-the-design-leaves-out) |
| Progress over a stream (SSE) | Not built | See [What the design leaves out](#what-the-design-leaves-out) |

Four points differ from the original design:

- `budget_ms` is not opt-in on `recall_polygon`, `recall_many` and
  `grid`. A call with no `budget_ms` gets 75% of the transport timeout.
  The timeout is `EMEM_TIMEOUT_SECS`, default 40 s, so the default
  budget is 30,000 ms (`default_fanout_budget_ms`).
- The preparer form of `/v1/backfill` has no default budget. Without
  `budget_ms` it runs until the transport timeout.
- `budget_exhausted` occurs only in the preparer. It means the cell hit
  `max_facts` before the end of the window.
- The design put one receipt on the area answer. `recall_polygon` and
  `recall_many` sign one receipt per cell, under `by_cell.<cell>.receipt`.
  `grid` signs one receipt that cites every fact in the grid.

## The problem

A cold area read can need many upstream fetches for each cell. The design
put the worst case at about 31 upstream round trips, one after the other,
for one cell.
`recall_polygon` reads 64 cells by default (`max_cells`, 1 to 1024). Such a
read cannot finish inside the 40 s transport timeout. A 504 gives the agent
nothing. An agent that gets 40 of 64 cells and a list of the rest can
continue.

## The idea: materialization persists

The responder signs and stores every fact that it fetches. It does this
before it sends the response. Thus the same request, sent again, gets more
cells from the cache and fewer from upstream.

So the store holds the progress. There is no job to queue, no job id, and
no state to clean up. A restart of the responder loses nothing. The
"resume token" is the request that the agent already holds.

## The contract

### One request field

An area read accepts `budget_ms`, a soft budget for fetch work. The
responder fills cells until the budget expires. Then it answers with the
cells it has. The responder detaches a fetch that is still in flight. It
does not stop it, so the fetch persists its fact when it completes. A fully warm request finishes before the budget
matters.

### A partial answer is a normal 200

This `recall_polygon` call asked for NDVI at 4 cold cells in Western
Australia with a 300 ms budget:

```sh
curl -s https://emem.dev/v1/recall_polygon -H 'content-type: application/json' -d '{
  "polygon_bbox": {"min_lat": -21.31, "min_lng": 129.61, "max_lat": -21.29, "max_lng": 129.63},
  "bands": ["indices.ndvi"], "max_cells": 4, "budget_ms": 300}'
```

The first answer, trimmed, on 7 November 2026:

```json
{
  "facts_returned": 0,
  "converged": false,
  "budget_ms": 300,
  "budget_ms_defaulted": false,
  "budget_expired": true,
  "progress": {"ready": 0, "pending": 4},
  "pending": [
    {"cell": "defi.zb30d.xEfA.cojA", "bands": ["indices.ndvi"], "state": "materializing",
     "reason": "the budget expired while this cell's fetch was in flight; the fetch was detached, not aborted, so it persists on completion and the identical retry finds it warm"}
  ],
  "retry": {
    "how": "repeat the identical request; every retry returns strictly more from cache, because everything materialized persists",
    "suggested_after_ms": 4000
  }
}
```

The same request about 5 seconds later returned `converged: true`,
`progress: {"ready": 4, "pending": 0}`, 4 facts and a receipt on each
cell. Two more retries returned the same answer.

The answer carries `budget_ms`, `progress`, `pending` and `retry` when the
call passed `budget_ms` or did not converge. `grid` copies them only when
it did not converge.

### The `pending[]` states

Each pending entry names the cell, the `band`, a state and a reason. The
entry tells the agent what to do next.

| State | Where | Meaning |
|---|---|---|
| `materializing` | All four routes | The budget expired while a fetch for this cell was in flight. The fetch continues and persists. |
| `upstream_failed` | All four routes | The fetch for this cell failed. The reason gives the upstream error. Retry later. |
| `skipped` | `recall_polygon`, `recall_many`, `grid` | The materializer skipped a band, with a retryable `reason_class` such as `timeout`. |
| `deferred` | `recall_polygon`, `recall_many`, `grid` | The cold-fetch limit held a band back. The next call fetches it. |
| `budget_exhausted` | `/v1/backfill` preparer | The cell reached `max_facts` before the end of the window. The retry continues from the stored slots. |

A skip that a retry cannot fix goes to `skipped[]`, not to `pending[]`.
Examples are an unknown band or a band with no materializer. Such an
answer can have `converged: true`. On 7 November 2026 a call with the
unknown band `elevation.copernicus_dem_glo30` returned `converged: true`,
0 facts, and four `skipped[]` entries with `reason_class: unknown_band`.
Read `skipped[]` before you trust `converged`.

### What the receipt attests

A receipt binds the facts that the answer returns. It says nothing about
`pending`. A pending entry is advisory and unsigned. It describes the work
of the responder, not the world.

A pending cell is not a signed absence. An absence says that the responder
read the source and found nothing there, and the responder signs it. A
pending entry says that the responder did not read the source yet, and
nobody signs it. The two have different types, so an agent cannot confuse
them.

### Convergence

`converged` is `true` when `pending` is empty. Two properties follow from
the design:

- A retry never loses ground, because fetched facts persist.
- The same request converges to the same fact set, however many retries
  it took, because facts are content-addressed.

The agent loop is: call, use the facts, and send the same request again
while `converged` is `false`.

## What the contract made possible

### The preparer

`/v1/backfill` with `cells` (1 to 64 entries) fills one band over one
window at each cell, under the same contract. "Fill this area before I
reason over it" is a loop over one idempotent call. It added no new tool.

```sh
curl -s https://emem.dev/v1/backfill -H 'content-type: application/json' -d '{
  "cells": ["Mount Fuji", "35.36,138.73"], "band": "era5.t2m",
  "start_unix": 1759276800, "end_unix": 1760486400,
  "max_facts": 3, "budget_ms": 2000}'
```

On 7 November 2026 this returned `schema: emem.backfill_prepare.v1`,
`converged: false`, and two `budget_exhausted` entries. Each cell wrote 3
facts, the `max_facts` limit, before the end of the window.

### The densification warmer

The warmer runs the preparer on a timer over a priority list that the
operator writes. It is off by default.

| Setting | Default | Effect |
|---|---|---|
| `EMEM_WARM_INTERVAL_SECS` | 0 (off) | Seconds between passes. A value above 0 starts the warmer. |
| `EMEM_WARM_PRIORITY` | `$EMEM_DATA/warm_priority.json` | The list of cells and bands. The warmer reads it again on each pass. |
| `EMEM_WARM_BUDGET_MS` | 30000 | The budget for each entry in one pass. |
| `EMEM_WARM_MAX_IO_PRESSURE` | 10 | A due pass waits while IO stall is above this percent. It waits for one interval at most. |

For an entry without a window, the warmer recalls the newest reading.
For an entry with a window, it runs the preparer. The warmer writes the
time of the last pass to `$EMEM_DATA/warm_state.json`, so a restart does
not skip or repeat a pass.
The format of the list is in the comment on `spawn_warm_priority_loop`.

### The fetch tools stay out of the core loop

The core `emem_recall` reports a skipped or deferred band in its
`materialize_notes[]`. Area reads report the same notes as `pending[]`. So
no fetch tool had to join the core tool list.

## What the design leaves out

- Job ids, a `/v1/jobs/*` surface, and server-side cursors. The store is
  the state. A second record of progress could disagree with it.
- Progress over a stream (SSE). The retry loop is enough for agents.
  A stream can come later without a change to the contract.
- A change to single-cell recall. One cell answers inside the timeout or
  fails with a typed error. Partial results apply to area reads only.

## Build order, as signed off

1. `budget_ms`, `pending[]` and `converged` on `recall_polygon`.
2. The same contract on `recall_many`.
3. The cell-list form of `emem_backfill`: the preparer.
4. The densification warmer, which runs the preparer on a schedule.
5. Documentation in the same commits.
