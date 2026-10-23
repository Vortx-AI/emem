# Experience-transfer benchmark (step 1: baselines only)

## The claim under test

> Across a stream of new and changing programming tasks, an agent that reuses
> verified experience reaches the same independently checked success rate at
> half the total cost of the strongest practical baseline.

Success is checked by hidden pytest suites the agent never sees. Cost is
dollars from recorded token usage and tool calls, not a proxy.

## Five tests the harness will eventually cover

1. Transfer to unseen tasks (`transfer` phase: variants needing knowledge from a `discovery` task).
2. A different receiving model than the one that gathered the experience.
3. A library change mid-stream (`post_change` phase, pydantic 1.10 to 2.x).
4. Incorrect but validly signed contributions (a signature says who, not that it is true).
5. Growing experience volume (does reuse still pay when the store is large and noisy?).

## What step 1 covers

Tests 1 and 3 are exercised by the task suite and the baselines. Nothing here
uses emem yet: there is no emem arm, and tests 2, 4 and 5 need one. The
baselines are `baseline_fresh` (task spec plus a docs-lookup tool) and
`baseline_notes` (the same plus free-text notes the arm wrote itself earlier
in the stream, unverified). No model or API is called anywhere.

## Layout

- `tasks/<id>/`: `task.md`, `tests_hidden/`, `reference.py`, `meta.json`; post_change tasks also have `reference_v1.py`, the v1-style solution.
- 40 tasks: 8 families (validators, serialization, config, parsing, optional, customtypes, coercion, jsonschema), each 2 discovery + 2 transfer (pydantic 1.10.x) + 1 post_change (pydantic 2.x).
- `runner.py`: one cached venv per version spec, pinned install, `pip freeze` recorded, results with `env_fingerprint` (sha256 of the freeze). Offline once venvs exist.
- `validate_suite.py`, `arms.py`, `cost.py` + `prices.json`, `run_stream.py`.

## Run

Needs Python 3.10+ and network once, to build the venvs (cached in `.venv_cache/`, override with `EMEM_BENCH_CACHE`).

```sh
cd bench/experience
python validate_suite.py
python run_stream.py --arm baseline_fresh --dry-run
```

Validation checks that every reference passes its tests, that each
post_change task's v1-style solution fails under pydantic 2 (and passes under
1.10, so the failure is the library change), and that transfer tests are not
near-copies of discovery tests. It exits non-zero on any violation.

The dry run feeds each `reference.py` as the solution with zero usage. It
proves the pipeline, not a model: its 100% success rate means nothing. It
writes `runs/<stamp>-<arm>-dry.jsonl` (one line per task) and a
`.summary.json` (success rate and cost, per arm and per phase).

## Fill in before a real run

- A model adapter: implement `ModelAdapter.complete(system, messages, tools)` and pass it to the arm (`run_stream.py` currently builds arms with the unwired adapter, which raises `NotImplementedError`).
- `prices.json`: every price is `null` and marked TO BE FILLED. `cost.py` refuses to price non-zero usage without one.
- Decide the retry and turn budget per task, and hold it equal across arms.

## Known limits

A model that already knows both pydantic versions may pass the discovery
tasks without help; measure the fresh baseline first and prune tasks it
always solves. Tasks are small, so cost differences are driven by
documentation lookups and turns.
