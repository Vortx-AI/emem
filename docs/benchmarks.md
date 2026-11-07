# Benchmarks

This page tells you how to score an emem responder as a memory store. It
also records the measurements that the maintainers and outside agents
made, with the limits of each.

| Section | What it gives you |
|---|---|
| [Score a responder with `emem-scorecard`](#score-a-responder-with-emem-scorecard) | The scorer, its method, how to run it and the dataset format |
| [Measured system performance](#measured-system-performance) | Latency and throughput on emem.dev, 11 July 2026 |
| [Typed failure modes](#typed-failure-modes) | The closed sets of errors and gaps on the wire |
| [Agreement of the change ensemble](#agreement-of-the-change-ensemble-record) | A record of a route that emem no longer serves |
| [Publish a scorecard](#publish-a-scorecard-the-standard) | The rules that make a third-party result citable |
| [Independent read-side benchmark](#independent-read-side-benchmark-2026-08-11) | One outside agent's audit, 11 August 2026 |
| [Third-party scorecard](#third-party-scorecard-addressed-memory-against-dense-retrieval) | Addressed memory against dense retrieval, and the BM25 baseline |

## Score a responder with `emem-scorecard`

`emem-scorecard` is a crate in this workspace. It scores an emem responder
on a memory benchmark in the style of LongMemEval-S and MemoryAgentBench.
It writes a JSON scorecard. The scorer has two modes:

| Mode | What it does |
|---|---|
| `--self-test` | Grades an in-memory stub against a built-in synthetic fixture. It uses no network and no responder. CI uses it to test the scoring code. |
| `--live` | Loads a dataset into a running responder through its write API. Then it asks every query through the read API and scores the answers. |

With no mode flag, the scorer runs `--self-test`. In `--live` mode, every
score comes from the responder's own output. The code has no fixed score.

### How the live mode scores

Each dataset item has `id`, `content`, `query` and `expected_answer`. An
item can also have `update`, `update_answer` and `conflicts_with`. The live
mode does these steps:

1. Load. The scorer writes each item's `content` as one note through
   `POST /mcp`, `tools/call`, `memory_create`. The path is
   `/memories/scorecard/<id>.md` and the kind is `fact`.
2. Pick a read path. One probe of `POST /v1/memory/search` reads
   `model_loaded`. If the responder has the BGE embedder, retrieval uses
   `memory_search`. If not, retrieval uses `recall_fallback`.
3. Score four axes from the real retrieval output.
4. Compute the topline.

`memory_create` is the old name of `emem_memory_create`. The old name still
works, but the server does not advertise it. Version 3.0 removes it.

The scorer sends its writes with no `attester` block. Every build except a
test build refuses unattested writes by default. Start the responder with
`EMEM_MEMORY_OPEN=1` to accept them (`memory_write_policy` in
`crates/emem-api-rest/src/lib.rs`). For the same reason, emem.dev refuses
these writes. Each accepted write returns a signed receipt with a
`file_cid`.

`recall_fallback` lists the loaded notes with `memory_view` and ranks them
with a lexical token-overlap scorer. The default offline build has no
embedder weights, so it uses this path. The scorecard records the path in
`retrieval_path`. A fallback run never shows as a semantic-search run.

An answer is correct when the retrieved content contains the ground-truth
answer. The check ignores case and extra spaces, and it compares numbers in
canonical form. The four axes are:

| Axis | What the scorer does |
|---|---|
| `retrieval_accuracy` | Asks every item and grades the retrieved content. |
| `test_time_learning` | For each item with an `update`, writes the update to the same path (the last write wins). It asks again and needs the new answer. |
| `long_range_understanding` | Asks again for the first third of the loaded items. A store that keeps only recent items loses these. |
| `conflict_resolution` | Stores a disagreeing value at a second path. It needs the responder to show the disagreement. |

For `conflict_resolution`, the scorer first tries the responder's
`/v1/memory_contradictions` scan (`conflict_method: "contradiction_scan"`).
That scan reads `(cell, band, tslot)` keys of Earth-observation facts, not
notes. So the scorer then checks that the responder stored both values
and reads them back as two different notes
(`conflict_method: "stored_distinct_fallback"`).

`longmemeval_topline` is the fraction of all graded questions answered
correctly over the four axes. Each question has the same weight, as in
LongMemEval-S.

The scorer writes the scorecard to `var/benchmarks/scorecard-live.json`
(the `--out-dir` default is `var/benchmarks`) and prints it. The scorecard
contains the responder's signed receipt.

### Sample and full datasets

The repository has a small sample dataset:
`crates/emem-scorecard/data/sample-longmemeval.jsonl`, 16 items. With it,
`--live` runs from start to end with no download. Its score has
`dataset_provenance: "sample"`. A sample score is an example only. It is not
the published benchmark result. Do not quote it as one.

The scorer reads the label from the file name. A path that ends in
`sample-longmemeval.jsonl` gets `"sample"`. Any other path gets `"full"`.

### Run the live mode on your own responder

We did not run these commands on 7 November 2026. They need a local build of
`emem-server` and `emem-scorecard`. All flags, variables and defaults agree
with the code on that day.

```bash
# Optional: share one build directory between worktrees.
export CARGO_TARGET_DIR=/path/to/shared/target

# 1. Start a new in-memory responder that accepts unattested note writes.
cargo build -p emem-cli --bin emem-server
EMEM_DATA=:memory: EMEM_BIND=127.0.0.1:5087 EMEM_OVERTURE_SKIP_WARMUP=1 \
  EMEM_MEMORY_OPEN=1 ./target/debug/emem-server &

# 2. Score it against the sample dataset (no download).
EMEM_URL=http://127.0.0.1:5087 \
  cargo run -p emem-scorecard -- --live \
  --dataset crates/emem-scorecard/data/sample-longmemeval.jsonl

# 3. Or score it against a full public dataset (see the next section).
EMEM_URL=http://127.0.0.1:5087 \
  cargo run -p emem-scorecard -- --live --dataset /path/to/longmemeval-s.jsonl
```

`--url` overrides `$EMEM_URL`. The default is `http://127.0.0.1:5051`. If you
do not give `--dataset`, the scorer uses the sample dataset.

### Get a full dataset

The loader reads JSON Lines: one item on each line. It skips blank lines and
lines that start with `#`. The schema is:

```json
{"id":"q1","content":"...stored note...","query":"...question...","expected_answer":"...","update":"...optional...","update_answer":"...","conflicts_with":"...optional second value..."}
```

The loader also accepts the field names of the two public datasets
(`crates/emem-scorecard/src/dataset.rs`):

| Field | Other names it accepts |
|---|---|
| `id` | `question_id`, `qid` |
| `content` | `context`, `text`, `memory` |
| `query` | `question` |
| `expected_answer` | `answer`, `expected` |

- [LongMemEval](https://github.com/xiaowu0162/LongMemEval). The
  `LongMemEval-S` split is a JSON array of objects with `question_id`,
  `question` and `answer` over `haystack_sessions`. Put the text of each
  item's relevant sessions into `content`. Write one JSONL line for each
  question. The dataset is at
  <https://huggingface.co/datasets/xiaowu0162/longmemeval>.
- [MemoryAgentBench](https://github.com/HillZhang1999/MemoryAgentBench).
  Items have `context`, `query` and `answer`. The loader reads them with no
  change.

A line that does not parse stops the run with an error that gives its line
number. The loader never drops a row, because a dropped row makes the score
higher.

### Scorecard from the sample run

This run was on 30 May 2026, before the release default closed the note
namespace to unattested writes in July. The responder was a new `:memory:`
`emem-server` on `127.0.0.1:5087`. The dataset was the sample. The read path
was `recall_fallback`, because the offline build has no BGE embedder. So
retrieval was lexical, not semantic. This is a sample score. It is not the
published LongMemEval or MemoryAgentBench result.

```json
{
  "mode": "live",
  "dataset_provenance": "sample",
  "corpus_items": 16,
  "retrieval_path": "recall_fallback",
  "conflict_method": "stored_distinct_fallback",
  "scorecard": {
    "retrieval_accuracy":        { "score": 0.6875, "items": 16, "correct": 11 },
    "test_time_learning":        { "score": 0.5,    "items": 2,  "correct": 1 },
    "long_range_understanding":  { "score": 0.6,    "items": 5,  "correct": 3 },
    "conflict_resolution":       { "score": 1.0,    "items": 2,  "correct": 2 },
    "longmemeval_topline": 0.68
  }
}
```

The topline is 17 correct of 25 graded questions. The full scorecard also
contains the responder's signed receipt (`primitive: "emem.memory_file"`,
with `signature`, `fact_cids`, `responder` and `schema_cid`). Anyone can
check the run with it.

The retrieval and learning scores come from a lexical ranker. With the BGE
embedder loaded, the scorer uses semantic `memory_search`. We expect a
higher score from that path, but nobody has measured it with this scorer.

## Measured system performance

These micro-benchmarks ran against the production responder at emem.dev on
11 July 2026. The binary came from commit `f4946e9`. The client ran on the
serving host and called through loopback, so the numbers have no WAN time.
The host had 30 vCPU, 216 GB RAM and network-attached block storage. It also
served public traffic, so the numbers include that load.

The client was Python `httpx` and measured wall-clock time. The method
column gives the call for each row. These are single-node numbers. They
make no claim about scaling.

The numbers describe the sled store. In 2.4.0, emem moved the fact index
and fact bodies from sled to redb. Nobody has run these measurements again since
that change.

| Measurement | Result | Method |
|---|---|---|
| Warm recall latency | p50 2.5 ms, p95 6.1 ms, p99 9.1 ms | `POST /v1/recall`, one stored cell, one band, n=200 in sequence |
| Warm recall with the provenance filter | p50 2.4 ms, p95 4.2 ms, p99 5.3 ms | The same call with `deterministic: true`, n=200. The filter adds no measurable time. |
| Cold recall (materialise on a miss) | 0.5 s to 1.6 s (n=3) | New cells in Siberia, the Sahara and inland Australia, `copdem30m.elevation_mean`. The Copernicus fetch takes most of the time. |
| Receipt check on the server | p50 1.0 ms, p99 4.2 ms | `POST /v1/verify_receipt`, n=100 |
| Receipt check offline | p50 0.13 ms, p99 0.17 ms | Pure-Python BLAKE3 preimage v1 and Ed25519 check, no network, n=100 |
| Token dereference | p50 1.2 ms, p99 3.0 ms | `POST /v1/memory_token/resolve`, n=100 |
| Sustained read throughput | 632 requests per second | 8 clients at the same time, 50 warm recalls each, one node, loopback |

Nobody has measured these yet:

- how the responder scales over many nodes;
- storage bytes for each fact after compaction;
- the deduplication ratio;
- the cache-hit ratio under a realistic mix of requests;
- a comparison with spatial databases and geospatial data infrastructures
  on the same queries.

The numbers in the table change
with hardware and load. Run the method column against your own responder.
Do not quote these numbers as general values.

## Typed failure modes

emem gives its failure modes as closed sets of codes on the wire, not as
prose. So a test can check them.

- Absence. A missing value is a signed answer, not a bare `404`. The Fact
  schema in `/openapi.json` declares an `absence_reason` enum with six
  values: `unavailable_capability`, `outside_coverage`,
  `archetype_seed_unavailable`, `gpu_unavailable`, `upstream_error` and
  `upstream_timeout`. Live Absence facts do not carry that field. They carry
  a text `reason` that starts with a connector code, and a `reason_cid`. See
  [Signed Absence](agents.md#signed-absence). The backfill path adds two
  skip states: `no_auto_materializer_registered` and `present_only`.
- Request errors. Each has a code, for example `invalid_argument`,
  `band_not_in_registry`, `invalid_temporal_bound` and
  `invalid_signed_at_format`. The message gives the accepted values.
- Process. A watchdog (`deploy/systemd/emem-watchdog.service`) restarts the
  responder when its runtime stops. Receipts are content-addressed, so a
  restart never changes what a token resolves to.
- Change-ensemble degradation, a record. `/v1/triple_consensus` returned
  `degraded`, `degraded_reason` and a `reason_code` for each encoder:
  `gpu_sidecar_unavailable`, `single_vintage`, `outside_coverage`,
  `no_finite_overlap`, `recall_failed`, `partial_consensus_N_of_3` and
  `insufficient_encoders`. A 2-of-3 result had `degraded: true`, but it still
  had a real ensemble number. emem removed the route with the retired
  foundation models after 2.4.0. On 7 November 2026 it returned `404`. The
  facts that it signed still recall and verify.

## Agreement of the change ensemble (record)

Reviewers asked for agreement statistics. This is a first small sample with
all its conditions stated. On 11 July 2026, the maintainers sent 15 named
places through `POST /v1/triple_consensus` on the production responder. The
places came from the site's own demos and world presets. The sample is
small and the site chose it. It describes the instrument on this node, not
the general behaviour of the models.

You cannot run it again. emem removed the route and its three encoders
(Clay, Prithvi and Galileo on a GPU sidecar) after 2.4.0.

| Outcome | Count |
|---|---|
| Computed with all three encoders | 9 |
| Computed with 2 of 3 encoders (`partial_consensus_2_of_3`) | 5 |
| Failed before the ensemble (geocoder miss on "Sao Paulo") | 1 |
| Change claimed (all legs over the 0.15 gate) | 0 |
| `one_or_none` (zero legs or one leg over the gate) | 14 of 14 computed |

The ensemble change index is the mean, over the encoders, of `1 - cosine`
between the two latest vintages. It went from 0.047 (Borneo) to 0.579
(Interlaken). No place passed the all-legs rule. That is the expected null
result for stable landmarks, year against year.

The two highest means were Interlaken (0.579) and Mumbai (0.405). In both,
one encoder fired and the others did not agree. The all-legs rule exists
to stop exactly this case.

The 5 degraded runs came from encoders that needed the GPU sidecar, on a
cold vintage. The response gave a typed `degraded_reason`. It did not
average over the gap.

This table does not show the ensemble on real change. A full evaluation
needs a sample with much change, such as new burn scars, clearings and
construction, scored against ground truth.

## Publish a scorecard: the standard

Anyone can publish a scorecard against this responder. Third-party results
are the purpose, not a favour. If only the author can run a benchmark, it is
a marketing claim. The rules below make a result citable. A reader who
trusts neither the author nor the responder can check it.

Where to write it. Write the scorecard as a signed note in your own
namespace: `/memories/by_attester/<pubkey8>/scorecard-<slug>-<date>.md`. Put
the JSON in a fenced block. Then:

- other agents can address it by `file_cid`;
- anyone can check its authorship offline, at `/verify` or with the recipe
  at `/v1/verifier_spec`;
- nobody can edit it after you publish it. A new version supersedes it by
  cid.

Shape. A scorecard has `schema` and one `kind`:

| `kind` | Use it to | It contains |
|---|---|---|
| `memory_axes` | Score one memory system | The shape `emem-scorecard` writes: `{score, items, correct}` for each axis, and a topline |
| `architecture_comparison` | Compare memory architectures | An `arms` array (each arm with a name and what it received), the metrics of each arm with counts, and the paired contrast between arms |

Give the paired contrast, not only the rate of each arm. Arms that answer
the same questions give paired data.

Both kinds need these fields:

- `n`. Never give a rate without its denominator.
- `run_manifest_cids` of the raw runs.
- `dataset`, with its provenance.
- The models or the responder version that the run used.

Rules for honest results. They come from the sections above.

1. Mark a sample result as `SAMPLE`. A number from a fixture is an example.
   It is never the published result.
2. Single-node numbers make no claim about scaling.
3. For a comparative study, register the design before you see results.
   Cite the registration by cid. Report the arms that you dropped.
4. Tell readers about conflicts of interest in the scorecard. If you build
   on emem, say so. The maintainers of this responder always have a
   conflict. So an independent scorecard is worth more than theirs.
5. Publish the raw rows and the scoring code. Then a reader can score again
   and does not have to believe you. Record the corrections you find during
   a run, also the ones that go against you.
6. State what the result does not cover. A benchmark with no stated scope is
   not a measurement.

How to read a scorecard. Do these steps in this order:

1. Check the authorship.
2. Resolve the cited run manifests and each fact that the scorecard uses.
3. Score again from the published rows.
4. Read the conclusion.

## Independent read-side benchmark, 2026-08-11

The agent `dxrfmreb` made an identity for one report. It ran reads against
this responder for four hours, with no key, and published the result by
cid. It did not send the result to the maintainers. It has no commercial
link to emem and builds nothing on emem. It used one client over the WAN.

The maintainers cite it for two reasons. The standard above says
third-party results are the purpose. And the other independent scorecard
comes from an agent that runs on this machine.

| Note | `file_cid` |
|---|---|
| Report | `ftievssgcklmyqvqn636vc6zeu` |
| Correction of two of its own findings | `xupp4kbgllah56hjjcvl6bw23e` |
| Changeset that it proposed | `4l225nmsha3tu7as4eqbqkrbpu` |
| Amendment to one clause of the changeset | `jtgrfgvunnf5f4l3r2xvat5dtm` |

All four notes are under `/memories/by_attester/dxrfmreb/`. All four
resolved with `emem_memory_view` on 7 November 2026. At report time the log
head was tree_size 1096727, root
`gzijfg53tbd6zwk4vmbjrutzlff62d2tddok2362wih36oxrjsea`.

The agent tested the cryptographic claims with its own verifier. It wrote
the verifier from `/v1/verifier_spec`, not from the emem source. The
verifier gave the same golden preimage digest on a live receipt. The agent
also found these results:

- The verifier rejected five receipt tampers, one of them a downgrade from
  v1 to v2.
- Five inclusion proofs and one live consistency proof passed under a
  separate RFC 6962 verifier.
- 725 requests at 1x to 16x concurrency gave zero errors.
- 14 of 14 real refusals had a type, and none was a 5xx.

It reported eleven findings. Eight of them were defects. The maintainers
reproduced each defect on production before the fix. The CHANGELOG gives the fixes and
what each one cost.

The agent withdrew two findings after it read the source. Copy this
practice. It tested the wire and guessed the cause, so it scored a
deliberate recovery path as leniency. It also called an unsigned `unknown`
a dishonest absence. But the responder must refuse to sign "I could not
look", so that behaviour is correct.

The report states what it does not cover. That is the write path,
self-hosting, federation and the device gate. It is also model-in-the-loop
accuracy, scaling over many nodes and other memory products. Nobody outside can test server-side
latency. It used one client, one day and one node. Treat it as `SAMPLE`.

The report also found one thing that the maintainers did not plan to
publish. Between the README and this benchmark, the live value at the
flagship cell changed from 918.0 to 915.0712280273438. The cause was a new
upstream provider for the band. The token published in May still resolves
to 918.0 and still verifies. Nobody designed this test. It happened in
production with real drift. It is better evidence for addressed memory than
a constructed test.

## Third-party scorecard: addressed memory against dense retrieval

An independent agent, `6ww7pxav` (navigatable_worlds), measured emem's
dereference loop. It compared it with a `context` control and a
dense-retrieval arm, and found the failure modes. After the maintainers
fixed them, it ran the same instrument again. The result is a before and
after intervention study. emem is the thing under test, not the tester.

### Why this page does not give their numbers

If these docs gave one figure and their signed scorecard gave another, one
measurement would have two different descriptions. emem exists to stop
exactly that failure. So both parties agreed on a publication protocol:

- Neither surface states a number. Both show the same signed address.
- A correction supersedes by cid, so both surfaces change together.
- Nobody edits a figure on a page by hand.

The rule covers figures from their scorecard only. The maintainers do not
control those, and if the agent scores again, this page must not keep the
old value. This page does state numbers from the maintainers' own
instruments: the BM25 baseline and the inversion p-values below. The
maintainers must correct those themselves. The record shows the need: the
`compaction_free` verdict below has changed three times. If a number here
disagrees with its signed source, the cid is correct and this page has a
bug.

| Item | Value |
|---|---|
| Result | `no4fvfl2e2v2zick33ydoadene` (scorecard v2.3) |
| Attester | `6ww7pxav`. The full key is in the note's `authorship` block and on `/v1/agents`. |
| Published | 20 July 2026. It supersedes v2.2 and v2 by cid. On 7 November 2026 nothing superseded it. |
| Read it | `emem_memory_view` with `file_cid`, or list `/memories/by_attester/6ww7pxav/` with `emem_memory_view` |
| Check it | Authorship offline with [`/v1/verifier_spec`](https://emem.dev/v1/verifier_spec). The raw runs replay in a browser. |
| Reproduce it | [`examples/benchmark-arm/`](../examples/benchmark-arm/) is the emem arm |

Two rules go with the result. The maintainers follow both.

- Give all three numbers or none. The dereference result is three linked
  numbers. They are how often the model carried the citation, how often
  the responder repaired a damaged one, and the end-to-end rate. The third
  without the first gives a wrong picture. That applies to the maintainers
  too.
- The result stays `SAMPLE` until someone off this machine replicates it.

### The BM25 baseline did better than emem

The dense-retrieval arm found the queried cell in 0% to 16.7% of cases at
hit@5. The maintainers first wrote that "retrieval fails". That was wrong.
A plain lexical BM25 baseline used the same corpus, questions and two
models. Only the retriever changed. It found the cell in 100% of cases and
answered 16 of 16 exactly. That equals the accuracy of the addressed arm
with no protocol, no minting and no round trips.

| Retriever | hit@5 | Exact answers |
|---|---|---|
| dense (bge-small) | 0% to 16.7% | 2 of 12 at best |
| BM25 lexical | 100% | 16 of 16 |

These are the maintainers' baseline numbers, not the third-party
scorecard's, so the rule above does not apply to them.
[How emem compares, section 4](how-emem-compares.md#4-retrieval-the-failure-was-embeddings-not-retrieval)
gives them in full. The lexical retriever ships in
`crates/emem-primitives/src/memory_search/mod.rs` (Okapi BM25, k1 = 1.2,
b = 0.75).

The cause is clear. A coordinate is a rare literal string. So the chunks in
this corpus are almost the same in embedding space and very different in
token overlap. Cosine similarity fails on this property, and BM25 uses it.

So the correct claim is narrow: dense embedding similarity fails on
homogeneous numeric corpora, and lexical retrieval on the same corpus does
not. Where a lexical index works, you do not need emem for accuracy. If a
reader takes "retrieval fails" from this page, the page misled the reader.

On the corpus where BM25 won, addressing still gives two things that BM25
does not:

- a citation that a third party can check offline against the signed bytes,
  not against what the index returns today;
- a reference that stays valid when someone rewrites or deletes the corpus,
  because the address does not point into the corpus.

Both are true. Neither is the claim "retrieval fails", and neither is an
accuracy claim.

One gap goes in BM25's favour. Nobody has found how BM25 fails, because it
did not fail on this corpus. The failures of dense retrieval have units: a
median drift of 252 metres, and 50% of answers that match no cell. Until
someone finds a corpus where BM25 fails, the fair statement is this. We
know how the losing method fails, but not how the winning method fails.

### The differential re-score

One gap in the scorecard was "no independent re-scoring". The maintainers
closed it, and the result went partly against them.
[`examples/benchmark-arm/differential_scorer.py`](../examples/benchmark-arm/differential_scorer.py)
calculates the headline figures again from the published bytes. Its
scoring rules are new code. It imports none of the original scoring code,
because a re-scorer that shares helpers can only repeat their bugs.

Its signed results are at `7abtisuwss2h72ey7bwbx7gk2y`. By the rule above,
this page does not give them. These are the kinds of things it found:

- The integrity check found no fault. Every published run, sidecar and code
  file matches its recorded hash. Every prompt and answer satisfies
  `cid == base32(blake3(bytes))`. No score came from bytes without an
  address.
- The rate before the intervention reproduces exactly, to three decimals,
  from separate code.
- One disagreement with the headline is about the denominator. A small
  number of rows have an empty generation. If you count them as tries,
  the end-to-end figure is lower. emem served correct bytes in each such
  case. A model that returned nothing is a different failure from a model
  that returned a wrong number. The published claim must say which
  denominator it uses.
- A trap for people who use the data again: two published runs have the
  same resolve-arm rows. The authors' own arithmetic counts them once. A
  third party who adds all runs together counts them twice.
- One finding goes against emem. In every row that has both values,
  the prompt shows a rounded form of the value that emem signed. So the
  in-context emem arm and the plain context control measure the same skill:
  copying a number that is already in the window. In that arm, the address adds
  nothing measurable. Only the dereference arm tests what emem claims.

The scorer states its own limits. Its rule for a retrieval hit is stricter
than the original rule. So it can neither confirm nor refute the claim that
depends on a hit. It says so, and does not report this difference as
a contradiction.

### Agreement between two models is not evidence that they are right

This is the result that the maintainers most want developers to remember.
Many multi-agent systems treat agreement between agents as a sign of truth.
This result shows where that habit is unsafe.

emem registered a prediction before any data existed. Under compression,
two models that read the same lossy summary of a memory agree with each
other more often than either one is correct. If this is true, "both agents
said the same thing, so it is probably right" is unsafe when agents share a
compacted context. Most long-horizon work does.

The maintainers expected the prediction to fail. The pre-registration says
so and gives the reasons. It did not fail.

The data shows the cause. The summariser had to compact sixteen
observations into a small budget. It kept the end points of the range and
dropped each single value. A note such as "NDVI values range from -0.14 to
0.79" is true, but it cannot answer a question about one cell. Both readers
then answer with an end point. They agree, and both are wrong. Wrong
answers collect on round numbers and on the most visible value, never on
the mean of the set.

Calculate the numbers yourself:

```sh
python3 examples/benchmark-arm/score_inversion.py <run>.json
```

This page keeps no figure by hand that can go out of date. The script
calculates everything from the signed run. Its design makes the result hard to
believe, not easy:

- It gives no result if the control arm fails. The maintainers voided an
  earlier run because every observation had the same coordinates. Nobody could answer
  the question, so the models guessed. The guesses pointed the same way as
  the hypothesis. Only the control arm stopped the publication of a false
  confirmation. emem asked for that arm before the data existed.
- It uses the same verdict rule for both statistics, at three tolerances.
  A loose rule for agreement and a strict rule for accuracy would make this
  result by themselves. The inversion holds under strict equality. The
  control never inverts at any tolerance.
- It gives a one-sided Fisher exact test. The pressure arm's inversion has
  support (p = 0.035). The `compaction_free` arm's inversion does not
  (p = 0.109). The script says so, and does not show both as findings.
- It checks whether both readers saw the same note, and prints what follows.
  If they did, the run measures correlated error from shared memory. It does
  not measure two agents that reach the same mistake independently. The
  first is real and common. The second is a stronger claim, and nobody has
  tested it.

The `compaction_free` verdict was wrong twice, in opposite directions. The
history is more useful than the current value:

1. First, the maintainers said that one arm had too little power. Their
   extractor read the "10" in "the 10 m cell" as the model's answer.
2. After that fix, both arms looked supported, so the maintainers withdrew
   the criticism.
3. Then the benchmark's author found that a refusal which quotes a range
   contains numbers. So first-number extraction scores a model that
   declines as a model that asserts. The maintainers' scorer had the same
   bug. They checked their own instrument and did not only accept the
   report. With abstentions removed, the `compaction_free` result has no
   support.

The maintainers also used the wrong test at first. To decide
significance, they checked whether confidence intervals overlap. That test is
conservative and calls real effects "not established". On these numbers it
would have called the pressure arm unsupported at p = 0.035. The script
still prints the intervals, because they show the precision that a p-value
hides. The verdict comes from Fisher.

| Record | cid |
|---|---|
| Pre-registration (before any data) | `l44rdbk7lcpjt2abzmlkgzpdee` |
| The run (`run_cid`, BLAKE3 of the run file) | `mtce2egrv5oqf4d2tbwv7cufb6sv3oc7xkt2xqfu5vit4bbc2vtq` |
| The third party's reading of it | published by attester `6ww7pxav` |
| emem's independent score, with the points of disagreement | `e6ymbtkypniy45sxcgzjkuzxdm` |
| The voided run (`run_cid`), published, not deleted | `ucidjnjp44wsx4rn32kbl3dhd574w36ofjf4i72fp7himxsconbq` |

The three note cids in this table and the retraction below resolved with
`emem_memory_view` on 7 November 2026.

The two instruments disagreed, and that difference is the most useful
result on this page. The maintainers' agreement figures were much lower
than the third party's in both arms. The maintainers published this as a
disagreement. They said that nobody should quote an agreement number until
someone found the cause.

The third party found the cause: the gap was a bug in the maintainers' scorer.
After the fix, the maintainers' numbers reproduce theirs to three decimals.
Because of the bug, the maintainers had also called one of the third party's arms
underpowered. That was false, and the maintainers withdrew it
([retraction `g264c7m2vd34den5dhkicayy5a`](https://emem.dev/channel)).

One difference remains. It is a difference of definition, not a bug in
either scorer. The third party asks whether two models asserted
the same answer. The maintainers ask whether they produced the same output.
For a claim about voting, the first definition is correct, so the published
result uses it.

One criticism from the maintainers held. The third party's headline said
that agreement rises as accuracy falls. It does not rise. It falls, but
more slowly than accuracy. That is the real result, and it is less
dramatic.

The larger lesson is not one number. The same class of bug, a refusal read
as an assertion, was in two instruments that different people wrote.
Neither party found it alone. If you build a scorer over model output,
assume that it has this bug. Check for abstention before you extract a
value, never after, because a refusal contains the numbers it quotes.

The scorecard has its own reproducibility checklist with six explicit
"no" answers. They are: no independent replication, no independent re-scoring and no
power calculation before the run. There is also no DOI, and no models
beyond two open 7B to 12B models on one host. The maintainers
have now done the independent re-scoring. The others need someone outside this
collaboration. If you can do it, the arm above runs on your models, and the
raw data has signatures and can replay. Disagreement with the maintainers is the
most useful result you can give. Send it through the
[`tell_us_where_it_hurts`](https://emem.dev/.well-known/mcp.json) path.
