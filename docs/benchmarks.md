# Benchmarks

`emem-scorecard` scores a running emem responder against a
LongMemEval-S / MemoryAgentBench-style memory benchmark and emits a signed
JSON scorecard. It has two modes:

- **`--self-test`**: grades an in-memory stub against a built-in synthetic
  fixture. No network, no responder. Exercises the scoring code in CI.
- **`--live --url <responder>`**: loads a dataset corpus into a *running*
  responder over its real write API, answers every query over the real read
  API, and computes the four axes + topline **from the responder's own
  output**. No score is hardcoded.

## Methodology

Each dataset item is `{id, content, query, expected_answer}` (with optional
`update`/`update_answer` and `conflicts_with`). The live path:

1. **Load (write API).** Each item's `content` is written as one signed
   memory file via `POST /mcp` → `tools/call` → `memory_create`
   (`/memories/scorecard/<id>.md`, `kind: "fact"`). Every create returns a
   server-signed receipt with a `file_cid`.
2. **Pick a read path.** A one-shot probe of `POST /v1/memory/search`
   reports `model_loaded`. If the BGE embedder is loaded, retrieval uses
   **`memory_search`** (semantic). If not (the default offline build ships
   no embedder weights), retrieval falls back to **`recall_fallback`**: list
   the loaded files via `memory_view` and rank them with a lexical
   token-overlap scorer. The chosen path is recorded as `retrieval_path`;
   a fallback run is never presented as a semantic-search run.
3. **Score four axes** from real retrieval output, with the standard
   answer-recall criterion (the retrieved content **contains** the
   ground-truth answer, case/whitespace-normalised, numerically canonical):
   - **retrieval_accuracy**: query every item, grade the retrieved content.
   - **test_time_learning**: for items with an `update`, rewrite the same
     path (last-write-wins), re-query, require the *post-update* answer.
   - **long_range_understanding**: re-query the earliest-loaded third of
     the corpus (front of the transcript), where a recency-biased store
     would have dropped the needle.
   - **conflict_resolution**: store a disagreeing companion under a second
     path and require the responder to surface the disagreement. The
     responder's `/v1/memory_contradictions` scan is tried first
     (`conflict_method: "contradiction_scan"`); since that index scans
     EO-fact `(cell, band, tslot)` triples and not free-text memory files,
     the honest text-surface check is whether both disagreeing values were
     stored **non-destructively** and read back distinct
     (`conflict_method: "stored_distinct_fallback"`).
4. **Topline.** `longmemeval_topline` is the item-weighted fraction of all
   graded questions answered correctly across the four axes, the
   LongMemEval-S convention.

The scorecard is written to `var/benchmarks/scorecard-live.json` and printed
to stdout, with the responder's signed receipt embedded.

## Honesty: SAMPLE vs FULL

A small **SAMPLE** dataset (`crates/emem-scorecard/data/sample-longmemeval.jsonl`,
~15 items) is committed so `--live` runs end-to-end with no download. Its
score is labelled `dataset_provenance: "sample"` and is **illustrative
only, not the published benchmark number**. A user-supplied dataset is
labelled `"full"`. Never quote a sample score as the published result.

## Run `--live` against a local responder

```bash
export CARGO_TARGET_DIR=/path/to/shared/target   # optional, to share builds

# 1. Boot a fresh in-memory responder.
cargo build -p emem-cli --bin emem-server
EMEM_DATA=:memory: EMEM_BIND=127.0.0.1:5087 EMEM_OVERTURE_SKIP_WARMUP=1 \
  ./target/debug/emem-server &

# 2. Score it against the committed sample (no download).
EMEM_URL=http://127.0.0.1:5087 \
  cargo run -p emem-scorecard -- --live \
  --dataset crates/emem-scorecard/data/sample-longmemeval.jsonl

# 3. Or score against the full public dataset (see below).
EMEM_URL=http://127.0.0.1:5087 \
  cargo run -p emem-scorecard -- --live --dataset /path/to/longmemeval-s.jsonl
```

`--url` overrides `$EMEM_URL` (default `http://127.0.0.1:5051`). Omit
`--dataset` to use the committed sample.

## Get the full dataset

The loader reads JSON Lines, one item per line, in the canonical emem schema
with aliases for the two public corpora:

```json
{"id":"q1","content":"...stored memory...","query":"...question...","expected_answer":"...","update":"...optional...","update_answer":"...","conflicts_with":"...optional second value..."}
```

Accepted aliases: `id` ← `question_id`/`qid`; `content` ←
`context`/`text`/`memory`; `query` ← `question`; `expected_answer` ←
`answer`/`expected`.

- **LongMemEval** (<https://github.com/xiaowu0162/LongMemEval>). The
  `LongMemEval-S` split ships as JSON of objects with
  `question_id` / `question` / `answer` over `haystack_sessions`. Flatten
  each item's relevant session text into `content` and emit one JSONL line
  per question. Dataset: <https://huggingface.co/datasets/xiaowu0162/longmemeval>.
- **MemoryAgentBench** (<https://github.com/HillZhang1999/MemoryAgentBench>).
  Items carry `context` / `query` / `answer`; map directly onto the schema
  above (the loader already accepts `context` and `answer` as aliases).

A line that fails to parse is a hard error with its line number; rows are
never silently dropped (a dropped row would inflate the score).

## Scorecard from the committed SAMPLE run

Produced by the run in this repo against a fresh `:memory:` responder
(`emem-server` on `127.0.0.1:5087`), dataset = the committed sample,
read path = `recall_fallback` (the offline build ships no BGE embedder, so
retrieval is lexical, not semantic). **This is a SAMPLE score: illustrative
only, not the published LongMemEval/MemoryAgentBench result.**

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

The full scorecard embeds the responder's signed receipt
(`primitive: "emem.memory_file"`, with `signature`, `fact_cids`,
`responder`, `schema_cid`) so the run is independently verifiable. The
`recall_fallback` retrieval and learning scores reflect a purely **lexical**
ranker; running against a responder with the BGE embedder loaded switches
the read path to semantic `memory_search` and is expected to score higher.

## Measured system performance

Micro-benchmarks against the production responder at emem.dev, run on the
serving host itself over loopback (no WAN in the numbers), 2026-07-11,
binary built from commit `f4946e9`. Host: 30 vCPU / 216 GB RAM /
network-attached block storage, the same machine that answers public
traffic, so background load is included rather than idealised away.
Client wall-clock timing from a Python `httpx` client; the measurement
script method is stated per row. These are single-node numbers; no
scaling claim is made.

| Measurement | Result | Method |
|---|---|---|
| Warm recall latency | p50 2.5 ms · p95 6.1 ms · p99 9.1 ms | `POST /v1/recall`, one attested cell, one band, n=200 sequential |
| Warm recall with provenance filter | p50 2.4 ms · p95 4.2 ms · p99 5.3 ms | same call with `deterministic: true`, n=200; the filter adds no measurable overhead |
| Cold recall (auto-materialize) | 0.5 s to 1.6 s (n=3) | fresh never-attested cells (Siberia, Sahara, Australian interior), `copdem30m.elevation_mean`; dominated by the upstream Copernicus fetch, then signed and persisted |
| Receipt verification, server side | p50 1.0 ms · p99 4.2 ms | `POST /v1/verify_receipt`, n=100 |
| Receipt verification, offline | p50 0.13 ms · p99 0.17 ms | pure-Python blake3 preimage-v1 + ed25519 check, no network, n=100 |
| Token dereference | p50 1.2 ms · p99 3.0 ms | `POST /v1/memory_token/resolve`, n=100 |
| Sustained read throughput | 632 requests/s | 8 concurrent clients x 50 warm recalls, single node, loopback |

Not yet measured, tracked as open evaluation work: multi-node scaling,
storage bytes per fact under compaction, deduplication ratio, cache-hit
ratio under a realistic access mix, and a head-to-head against spatial
databases and geospatial data infrastructures on the same queries. Numbers
above will drift with hardware and load; re-run the method column against
your own responder rather than quoting these as universal.

## Failure modes, typed

Reviewers ask for failure modes; emem's are enumerated closed sets on the
wire rather than prose, so they are testable:

- **Absence reasons** (a missing value is a signed answer, never a bare
  404): `unavailable_capability`, `outside_coverage`, `gpu_unavailable`,
  `archetype_seed_unavailable`, `no_auto_materializer_registered`,
  `present_only`.
- **Change-ensemble degradation** (`/v1/triple_consensus` carries
  `degraded`, `degraded_reason`, and per-encoder `reason_code`):
  `gpu_sidecar_unavailable`, `single_vintage`, `outside_coverage`,
  `no_finite_overlap`, `recall_failed`, `partial_consensus_N_of_3`,
  `insufficient_encoders`. A 2-of-3 result reports `degraded: true` even
  though it carries a real ensemble number.
- **Request errors** are typed (`invalid_argument`,
  `band_not_in_registry`, `invalid_temporal_bound`,
  `invalid_signed_at_format`, ...) and teach the accepted vocabulary in
  the message.
- **Process level**: a watchdog restarts the responder if the runtime
  stalls; receipts are content-addressed, so a restart never changes
  what a token resolves to.

## Agreement statistics for the change ensemble

Reviewers asked for agreement statistics, so here is a first, small,
fully stated sample rather than a claim: the 15 named places used across
the site's own demos and world presets, run through
`POST /v1/triple_consensus` against the production responder on
2026-07-11. The sample is site-chosen and small; it characterises the
instrument on this node, not global model behaviour.

| Outcome | Count |
|---|---|
| Computed, all three encoders | 9 |
| Computed, degraded 2-of-3 (`partial_consensus_2_of_3`) | 5 |
| Failed before the ensemble (geocoder miss on "Sao Paulo") | 1 |
| Change claimed (`all` legs over the 0.15 gate) | 0 |
| `one_or_none` (zero or one leg over the gate) | 14 of 14 computed |

Ensemble change indices (mean of per-encoder `1 - cosine` between the
two latest vintages) ranged 0.047 (Borneo) to 0.579 (Interlaken). No
place cleared the all-legs rule, which is the expected null result for
stable landmarks compared year over year; the two highest means
(Interlaken 0.579, Mumbai 0.405) show single encoders firing without
corroboration, exactly the case the all-legs rule exists to hold back.
The 5 degraded runs are the honest cost of sidecar-gated encoders on a
cold vintage: the response says so in a typed `degraded_reason` instead
of averaging over the gap. What this table does not show, and what a
real evaluation still needs: a change-rich sample (recent burn scars,
clearings, construction) where the ensemble should fire, scored against
ground truth.

## Publishing a scorecard: the standard

Anyone may publish a scorecard against this responder, and third-party results
are the point rather than a courtesy: a benchmark only an author can run is a
marketing claim. The convention below is what makes a result citeable and
checkable by someone who trusts neither the author nor the responder.

**Where.** Write the scorecard as a signed memory in your own namespace,
`/memories/by_attester/<pubkey8>/scorecard-<slug>-<date>.md`, with the JSON in a
fenced block. It is then addressable by `file_cid`, its authorship verifies
offline (`/verify`, or the recipe at `/v1/verifier_spec`), and it cannot be
edited under you: a revision supersedes by cid.

**Shape.** A scorecard carries `schema` plus one `kind`:

- `kind: "memory_axes"`: the harness shape `emem-scorecard` emits: per-axis
  `{score, items, correct}` plus a topline. Use for scoring one memory system.
- `kind: "architecture_comparison"`: for comparing memory architectures. Carry
  an `arms` array (each with a name and what it received), the per-arm metrics
  with counts, and the paired contrast between arms rather than only per-arm
  rates, because arms compared on the same questions are paired data.

Both require: `n` (never a rate without its denominator), the `run_manifest_cids`
of the raw runs, `dataset` with its provenance, and the models or responder
version actually exercised.

**Honesty rules, carried from the sections above.**

1. Mark `SAMPLE` results as SAMPLE. A number from a fixture is illustrative and
   is never the published result.
2. Single-node numbers make no scaling claim.
3. For a comparative study, pre-register the design before you see results and
   cite the registration by cid. Report the arms you dropped.
4. Disclose conflicts of interest. If you build on emem, say so in the scorecard
   itself. The maintainers of this responder are conflicted by construction,
   which is why an independent scorecard is worth more than ours.
5. Publish the raw rows and the scoring code, so a reader can rescore rather
   than believe. Corrections found during a run belong in the record, including
   the ones that moved the result against you.
6. State what the result does not cover. A benchmark with no stated scope is not
   a measurement.

**What a reader should do with one.** Verify authorship, resolve the cited run
manifests and any fact the scorecard rests on, rescore from the published rows,
and only then read the conclusion. That order is the whole protocol in miniature.

## Third-party scorecard: addressed memory vs. dense retrieval

An independent agent (`6ww7pxav`, navigatable_worlds) measured emem's
dereference loop against a `context` control and a dense-retrieval arm,
diagnosed its failure modes, and re-ran the same instrument after we shipped
fixes for them. The result is a before/after intervention study in which emem
is the thing being tested, not the thing doing the testing.

**This page does not restate their numbers, on purpose.**

If our docs said one figure and their signed scorecard said another, we would
have produced two diverging descriptions of one measurement, which is precisely
the failure emem exists to abolish, committed by its own authors in public. So
under a publication protocol we agreed with them, neither surface states a
number: both render the same signed address, and a correction supersedes by cid
so both surfaces move together. Nobody hand-edits a figure on a page.

| | |
|---|---|
| Canonical result | `no4fvfl2e2v2zick33ydoadene` (scorecard v2.3) |
| Attester | `6ww7pxav`, full key in the contacts registry |
| Published | 2026-07-20, superseding v2.2 and v2 by cid |
| Read it | `POST /v1/memory_token/resolve`, or `memory_view` the attester's namespace |
| Verify it | authorship offline per [`/v1/verifier_spec`](https://emem.dev/v1/verifier_spec); raw runs replay in-browser |
| Reproduce it | [`examples/benchmark-arm/`](../examples/benchmark-arm/) is the canonical emem arm |

Two rules travel with it, and we hold ourselves to both:

- **The triple travels or nothing travels.** The dereference result is three
  bound numbers: how often the model carried the citation, how often the
  responder recovered a damaged one, and the end-to-end rate. Any presentation
  of the third without the first misstates it. That includes ours.
- **It is marked SAMPLE** until someone off this box replicates it.

The scorecard carries its own reproducibility checklist including six explicit
NOs (no independent replication, no independent re-scoring, no pre-specified
power, no DOI, no model diversity beyond two 7-12B open models on one host).
Four of those need someone outside this collaboration. If you are that someone,
the arm above runs on your models and the raw data is signed and replayable:
disagreeing with us is the most useful thing you can do here, and
[`tell_us_where_it_hurts`](https://emem.dev/.well-known/mcp.json) is the path in.
