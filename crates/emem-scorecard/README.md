# emem-scorecard

`emem-scorecard` is a command-line tool that scores the memory side of an emem
responder. It grades four axes in the style of MemoryAgentBench: retrieval,
test-time learning, long-range recall and conflict handling. It also gives one
LongMemEval-S style topline, and writes the result as JSON.

You need this tool when you change how emem stores or finds memory notes, and
you want a number before and after. You do not need it to use emem. The tool
does not score Earth-observation facts.

| | |
|---|---|
| Version | 2.4.4 (workspace version in the root `Cargo.toml`) |
| Kind | one binary, `emem-scorecard`; no library, no other crate uses it |
| Depends on | `anyhow`, `clap`, `reqwest`, `serde`, `serde_json`, `tokio`; no emem crate |
| Tests | 15 unit tests, inline in `src/` |
| Method and sample score | [docs/benchmarks.md](../../docs/benchmarks.md) |

## Try it

The self-test needs no network and no responder. It grades an in-memory stub
against a small fixture inside the binary. From a checkout (Rust
1.91 or later):

```sh
cargo run -p emem-scorecard -- --self-test
```

We did not run this command for this page, because the host had no Rust
toolchain. From the code, the run grades 16 items and every axis gets 1.0. The
unit test `perfect_corpus_scores_one` in `src/main.rs` asserts this result.
The run writes `var/benchmarks/scorecard-self-test.json` and prints the same
JSON. A self-test number is a check of the scoring code, not a benchmark result.

## The two modes

| | `--self-test` (default) | `--live` |
|---|---|---|
| Graded against | `StubResponder`, in memory | a running responder at `--url` |
| Corpus | the fixture in `src/fixture.rs` | a JSON Lines file from `--dataset` |
| Network | none | HTTP to the responder |
| Writes | none | one memory note per item, through `memory_create` |
| Receipt in output | none | one receipt, from `memory_view` on the first note |
| Output file | `scorecard-self-test.json` | `scorecard-live.json` |

The flags are `--self-test`, `--live`, `--url`, `--dataset` and `--out-dir`.
`--out-dir` has the default `var/benchmarks`. If you give both mode flags, the
tool stops with an error. If you give neither, it runs the self-test.

The URL comes from `--url`, then from `EMEM_URL`, then from the default
`http://127.0.0.1:5051`. Without `--dataset`, live mode uses the sample in
`data/sample-longmemeval.jsonl`.

## Run live mode against a local responder

Live mode writes to the responder. Use a local responder that you can discard.
Do not point it at `https://emem.dev`.

A release build refuses unattested memory writes by default. The tool sends its
writes with no `attester` block. Thus, start the responder with
`EMEM_MEMORY_OPEN=1` (see `memory_write_policy` in
`crates/emem-api-rest/src/lib.rs`).

```sh
cargo build -p emem-cli --bin emem-server
EMEM_DATA=:memory: EMEM_BIND=127.0.0.1:5087 EMEM_OVERTURE_SKIP_WARMUP=1 \
  EMEM_MEMORY_OPEN=1 ./target/debug/emem-server &

cargo run -p emem-scorecard -- --live --url http://127.0.0.1:5087
```

We did not run these commands for this page. They need a Rust build and a local
responder. [docs/benchmarks.md](../../docs/benchmarks.md) shows the result of
one run on 30 May 2026: topline 0.68 on the sample.

## What live mode does

1. It writes each item's `content` to `/memories/scorecard/<id>.md` with
   `memory_create` over `POST /mcp`.
2. It sends one query to `POST /v1/memory/search`. If the reply has
   `model_loaded: true`, it uses semantic search. If not, it lists the notes
   with `memory_view` and ranks them by shared words. The output field
   `retrieval_path` names the path it used.
3. It grades each axis. An answer is correct when the retrieved note contains
   the expected answer. The check ignores case and extra spaces.
4. It reads the first note again with `memory_view` and puts that receipt in
   the output.

| Axis | Items it grades (live) | How |
|---|---|---|
| `retrieval_accuracy` | every item | query, then check the top note |
| `long_range_understanding` | the first third of the items (at least 1) | the same query again for the earliest notes |
| `test_time_learning` | items with `update` and `update_answer` | write `update` to the same path, query, expect `update_answer` |
| `conflict_resolution` | items with `conflicts_with` | write the second value to `<id>__b.md`, then ask whether the disagreement shows |

`longmemeval_topline` is the number correct over the number graded, summed
over all four axes.

The tool exits with code 1 when the retrieval axis or the long-range axis
graded 0 items. It only warns when the learning axis or the conflict axis
graded 0 items, because those fields are optional in a dataset.

## Dataset format

One JSON object per line. The loader skips blank lines and lines that start
with `#`. A line that does not parse stops the run with its line number.

```json
{"id":"q1","content":"...","query":"...","expected_answer":"...","update":"...","update_answer":"...","conflicts_with":"..."}
```

| Field | Also accepted as | Required |
|---|---|---|
| `id` | `question_id`, `qid` | yes |
| `content` | `context`, `text`, `memory` | yes |
| `query` | `question` | yes |
| `expected_answer` | `answer`, `expected` | yes |
| `update`, `update_answer` | none | no; both needed for the learning axis |
| `conflicts_with` | none | no |

The committed sample has 16 items. Two have `update` and `update_answer`, and
two have `conflicts_with`. Thus a live run on the sample grades 16, 5, 2 and 2
items on the four axes. A path that ends in `sample-longmemeval.jsonl` gets
`dataset_provenance: "sample"`. Any other path gets `"full"`. A sample score is
illustrative. It is not a published LongMemEval or MemoryAgentBench score.

## Limits of the scores

These limits come from the code in `src/live.rs` and `src/score.rs`.

- The conflict axis in live mode grades only items that should conflict. It
  has no case that should not conflict. A run that flags every pair gets 1.0.
  The self-test fixture does have one such case.
- The conflict check first calls `POST /v1/memory_contradictions`. Any result
  in that list counts as a hit, for every item. The list holds contradictions
  between signed Earth-observation facts, not between notes. On a responder
  that already holds facts, the axis can pass for a reason not related to the
  dataset.
- When that list is empty, the check passes if both notes read back with
  different text (`conflict_method: "stored_distinct_fallback"`). This shows
  that the store kept both values. It does not show that the responder found
  the disagreement.
- The learning axis rewrites a note on the same path, so the old text is gone.
  The axis checks that a rewrite replaces the value. It does not test a store
  that keeps both versions.
- The long-range axis asks the same queries as the retrieval axis, for the
  first third of the items. A memory store has no transcript order, so the two
  axes give the same result on those items.
- The tool sends `kind: "fact"` on every write. That is not a memory kind. The
  responder stores the note with the default kind, `resource`.
- The output embeds a responder receipt for one note. The scorecard JSON itself
  has no signature.

## What does not ship

- No CI job runs the binary. The 15 unit tests run with
  `cargo test --workspace`.
- No converter for the public datasets. You must convert LongMemEval-S or
  MemoryAgentBench to the format above yourself. docs/benchmarks.md says how.
- No published full-dataset score.
- The crate has no `#![forbid(unsafe_code)]` line yet. It uses no `unsafe`.

## Code issues we found for this page

- The doc comment of `conflict_verdict` in `src/live.rs` says the code checks
  the scan result for "this id's namespace" and names the method
  `value_compare_over_stored`. The code does not filter by namespace, and the
  method name it writes is `stored_distinct_fallback`.
- docs/benchmarks.md says the tool emits a "signed JSON scorecard" and that the
  self-test runs in CI. Neither is true of the code on 7 November 2026.

## The zstd-0.13-compat folder

`crates/zstd-0.13-compat` is not a workspace member and is not part of the
scorecard. It is a crate named `zstd` at version 0.13.4 that re-exports zstd
0.14.3. The root `Cargo.toml` patches every `zstd ^0.13` dependency to it.
This way `arrow-ipc`, `parquet` and `lance-encoding` link the zstd release that
fixes RUSTSEC-2026-0391, -0398 and -0403. Its `Cargo.toml` says to remove it
when those crates move to zstd 0.14.

## Read more

- [docs/benchmarks.md](../../docs/benchmarks.md): method, the sample run,
  how to get the full datasets, and how to publish a scorecard.
- [crates/emem-mcp](../emem-mcp/README.md): the `memory_create` and
  `memory_view` tools the live mode calls.
