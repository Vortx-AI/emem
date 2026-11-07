# Experience-transfer benchmark

This directory holds a benchmark. It measures whether an agent that reuses
verified experience from emem solves new programming tasks for less money.
It is for people who want to measure that claim, not for people who call emem.
To call emem, read [docs/agents.md](../../docs/agents.md).

Try the offline part first. It needs no model, no key and no emem write:

```sh
cd bench/experience
python3 validate_suite.py
```

## What ships and what does not

| Part | State on 7 November 2026 |
|---|---|
| 40 tasks with hidden pytest suites | Ships. `validate_suite.py` passes all 40. |
| Dry run of the pipeline (`run_stream.py --dry-run`) | Ships. It uses each reference solution, so its 100% success rate measures nothing. |
| Baseline arms `baseline_fresh` and `baseline_notes` | Code ships. `run_stream.py` has no model adapter, so a real run stops at the first model call. |
| emem arm, pilot, repair and technique scripts | Code ships. They need operator files that are not in this repository (see [The pilot](#the-pilot-operator-only)). |
| Results | None in the repository. `runs/` is in `.gitignore`. |
| Tests 2, 4 and 5 of the claim | Not built. |

## The claim under test

> The claim is about a stream of new and changing programming tasks. In it,
> an agent that reuses verified experience reaches the same success rate as
> the strongest practical baseline, checked independently, at half the total cost.

Hidden pytest suites check success. The agent never sees them. Cost is dollars
from recorded token usage and tool calls, not a proxy.

The benchmark is to cover five tests. Only 1 and 3 have tasks now.

1. Transfer to unseen tasks. The `transfer` tasks need knowledge from a `discovery` task.
2. A receiving model that is different from the model that collected the experience.
3. A library change in the stream. The `post_change` tasks move from pydantic 1.10 to pydantic 2.
4. Incorrect contributions with valid signatures. A signature says who wrote a note, not that it is true.
5. A large and noisy store. Reuse must still pay when the experience volume grows.

## The task suite

Each task is a directory `tasks/<id>/` with `task.md`, `tests_hidden/`,
`reference.py` and `meta.json`. Each `post_change` task also has
`reference_v1.py`, a solution in pydantic 1.10 style.

There are 40 tasks in 8 families: coercion, config, customtypes, jsonschema,
optional, parsing, serialization and validators. Each family has these tasks:

| Phase | Tasks per family | Total | Library |
|---|---|---|---|
| `discovery` | 2 | 16 | `pydantic==1.10.*` |
| `transfer` | 2 | 16 | `pydantic==1.10.*` |
| `post_change` | 1 | 8 | `pydantic==2.*` |

## Files

| File | What it does |
|---|---|
| `taskset.py` | Loads the tasks and puts them in stream order. |
| `runner.py` | Makes one cached venv per version spec and runs the hidden tests. It records `pip freeze` and its sha256 as `env_fingerprint`. |
| `validate_suite.py` | Checks the task suite (see below). |
| `arms.py` | The two baseline arms, the docs tool and an adapter that has no model. |
| `cost.py`, `prices.json` | Prices token usage. Every price for a real model in `prices.json` is `null`. |
| `run_stream.py` | Runs one arm over the stream and writes `runs/`. |
| `adapters.py` | `ShimAdapter`: a chat client for the LLM shim on `127.0.0.1:5015` (Gemma 3 12B). See [python/llm_shim](../../python/llm_shim/README.md). |
| `emem_arm.py` | Publishes procedures as signed emem notes and retrieves them with `/v1/memory/search`. |
| `pilot.py` | Compares `baseline_fresh` with the emem arm on the transfer and post-change tasks. |
| `repair.py` | Ports procedures that fail under pydantic 2 and publishes each port that passes its own tests. |
| `techniques.py` | Writes short technique cards for each procedure (`publish`) and measures retrieval with them (`measure`). |
| `repeats.sh`, `repeats2.sh` | Run `pilot.py` three times per configuration to measure noise. They use fixed operator paths. |

## Run the offline checks

You need Python 3.10 or later and the `venv` module with `ensurepip`. On
Debian and Ubuntu, that is the `python3-venv` package. You need the network
once, to build two venvs. They go in `.venv_cache/`. Set `EMEM_BENCH_CACHE`
to put them somewhere else.

```sh
cd bench/experience
python3 validate_suite.py
python3 run_stream.py --arm baseline_fresh --dry-run
```

`validate_suite.py` does three checks, and it exits non-zero if one fails:

- Every reference solution passes its own tests.
- Each `post_change` task's `reference_v1.py` fails under pydantic 2 and
  passes under 1.10. Thus the library change causes the failure.
- No `transfer` test is a near copy of a `discovery` test.

On 7 November 2026 it printed
`OK: 40 tasks valid; every post_change v1-era reference fails under pydantic 2.`
in 31 seconds. The venvs held pydantic 1.10.27 and 2.14.3.

The dry run gives each `reference.py` to the pipeline as the answer, with
zero usage. It tests the pipeline, not a model. It writes
`runs/<stamp>-<arm>-dry.jsonl` with one line per task. It also writes
`runs/<stamp>-<arm>-dry.summary.json` with the success rate and cost per arm
and per phase. On 7 November 2026 it wrote 40 lines in 18 seconds.

If your Python has no `ensurepip`, `runner.py` cannot make the venvs. We
made them with `uv venv --seed`. Then we installed `pytest>=8,<10` and the
pydantic spec in each venv and wrote its `pip freeze` to `freeze.txt`.
`runner.py` uses a venv again when it has `bin/python` and `freeze.txt`.

## Before a real baseline run

- Write a model adapter with a `complete(system, messages, tools)` method.
  `run_stream.py` now builds arms with the adapter from `arms.py`, which
  raises `NotImplementedError`. `adapters.ShimAdapter` is one adapter that
  works, but `run_stream.py` does not use it.
- Put the prices in `prices.json` and record the date in `_as_of`.
  `cost.py` refuses to price non-zero usage without a price.
- Choose the retry and turn budget per task. Use the same budget in each arm.

## The pilot (operator only)

`pilot.py` compares two arms on the 16 transfer tasks and the 8 post-change
tasks. Each arm gets 3 tries per task by default (`--attempts`).

| Arm | What the model gets |
|---|---|
| `baseline_fresh` | The task and a docs tool. |
| `emem_procedures` | The same, plus the top procedures that emem returns for the task text. Each procedure says if it passed its tests under the task's pydantic release. With `--cards`, the arm is `emem_cards`. |

You cannot run the pilot from this repository alone. It needs these items:

- The `librarian` agent from `/home/ubuntu/emem-world/agents` (`ememagentd`
  and `roster.json`). That directory is not in this repository. The agent's
  key signs the notes.
- The LLM shim on `127.0.0.1:5015`.
- Signed writes to emem.dev. Without `--skip-publish`, the pilot writes one
  note per discovery task. `repair.py` and `techniques.py publish` also write.
- Operator paths. `pilot.py` keeps its venvs in
  `/home/ubuntu/emem-world/agents/state/bench-venvs` unless `EMEM_BENCH_CACHE`
  is set. `repeats.sh` and `repeats2.sh` run from `/home/ubuntu/emem-bench`
  with `/home/ubuntu/.emem-witness/venv/bin/python`.

`pilot.py` prices Gemma 3 12B at USD 0.09 per million input tokens and
USD 0.29 per million output tokens. These rates are in the code. It does not
read `prices.json`.

The published procedures are public, so anyone can read them. On
7 November 2026, emem.dev held 18 procedure notes under
`/memories/by_attester/3qsr47pc/procedures/`: 16 discovery procedures and 2
repaired ports (`optional-01-v2.md` and `parsing-02-v2.md`). This read lists
them:

```sh
curl -s -X POST https://emem.dev/v1/memory/search \
  -H 'content-type: application/json' \
  -d '{"q":"pydantic procedure","k":100,"path_prefix":"/memories/by_attester/3qsr47pc/procedures/"}' \
  | jq -r '[.hits[].path] | unique | .[]'
```

## Known limits

A model that knows both pydantic versions can solve discovery tasks without
help. Run the fresh baseline first. Then remove the tasks that it always
solves. The tasks are small, so documentation lookups and turns cause most
of the cost difference.
