# Benchmarks

`emem-membench` scores a running emem responder against a
LongMemEval-S / MemoryAgentBench-style memory benchmark and emits a signed
JSON scorecard. It has two modes:

- **`--self-test`** — grades an in-memory stub against a built-in synthetic
  fixture. No network, no responder. Exercises the scoring code in CI.
- **`--live --url <responder>`** — loads a dataset corpus into a *running*
  responder over its real write API, answers every query over the real read
  API, and computes the four axes + topline **from the responder's own
  output**. No score is hardcoded.

## Methodology

Each dataset item is `{id, content, query, expected_answer}` (with optional
`update`/`update_answer` and `conflicts_with`). The live path:

1. **Load (write API).** Each item's `content` is written as one signed
   memory file via `POST /mcp` → `tools/call` → `memory_create`
   (`/memories/membench/<id>.md`, `kind: "fact"`). Every create returns a
   server-signed receipt with a `file_cid`.
2. **Pick a read path.** A one-shot probe of `POST /v1/memory/search`
   reports `model_loaded`. If the BGE embedder is loaded, retrieval uses
   **`memory_search`** (semantic). If not (the default offline build ships
   no embedder weights), retrieval falls back to **`recall_fallback`**: list
   the loaded files via `memory_view` and rank them with a lexical
   token-overlap scorer. The chosen path is recorded as `retrieval_path` —
   a fallback run is never presented as a semantic-search run.
3. **Score four axes** from real retrieval output, with the standard
   answer-recall criterion (the retrieved content **contains** the
   ground-truth answer, case/whitespace-normalised, numerically canonical):
   - **retrieval_accuracy** — query every item, grade the retrieved content.
   - **test_time_learning** — for items with an `update`, rewrite the same
     path (last-write-wins), re-query, require the *post-update* answer.
   - **long_range_understanding** — re-query the earliest-loaded third of
     the corpus (front of the transcript), where a recency-biased store
     would have dropped the needle.
   - **conflict_resolution** — store a disagreeing companion under a second
     path and require the responder to surface the disagreement. The
     responder's `/v1/memory_contradictions` scan is tried first
     (`conflict_method: "contradiction_scan"`); since that index scans
     EO-fact `(cell, band, tslot)` triples and not free-text memory files,
     the honest text-surface check is whether both disagreeing values were
     stored **non-destructively** and read back distinct
     (`conflict_method: "stored_distinct_fallback"`).
4. **Topline.** `longmemeval_topline` is the item-weighted fraction of all
   graded questions answered correctly across the four axes — the
   LongMemEval-S convention.

The scorecard is written to `var/benchmarks/membench-live.json` and printed
to stdout, with the responder's signed receipt embedded.

## Honesty: SAMPLE vs FULL

A small **SAMPLE** dataset (`crates/emem-membench/data/sample-longmemeval.jsonl`,
~15 items) is committed so `--live` runs end-to-end with no download. Its
score is labelled `dataset_provenance: "sample"` and is **illustrative
only — not the published benchmark number**. A user-supplied dataset is
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
  cargo run -p emem-membench -- --live \
  --dataset crates/emem-membench/data/sample-longmemeval.jsonl

# 3. Or score against the full public dataset (see below).
EMEM_URL=http://127.0.0.1:5087 \
  cargo run -p emem-membench -- --live --dataset /path/to/longmemeval-s.jsonl
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

- **LongMemEval** — <https://github.com/xiaowu0162/LongMemEval>. The
  `LongMemEval-S` split ships as JSON of objects with
  `question_id` / `question` / `answer` over `haystack_sessions`. Flatten
  each item's relevant session text into `content` and emit one JSONL line
  per question. Dataset: <https://huggingface.co/datasets/xiaowu0162/longmemeval>.
- **MemoryAgentBench** — <https://github.com/HillZhang1999/MemoryAgentBench>.
  Items carry `context` / `query` / `answer`; map directly onto the schema
  above (the loader already accepts `context` and `answer` as aliases).

A line that fails to parse is a hard error with its line number — rows are
never silently dropped (a dropped row would inflate the score).

## Scorecard from the committed SAMPLE run

Produced by the run in this repo against a fresh `:memory:` responder
(`emem-server` on `127.0.0.1:5087`), dataset = the committed sample,
read path = `recall_fallback` (the offline build ships no BGE embedder, so
retrieval is lexical, not semantic). **This is a SAMPLE score — illustrative
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
