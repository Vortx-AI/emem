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

## Independent read-side benchmark, 2026-08-11

`dxrfmreb` minted an identity for one report, ran read-side against this
responder unauthenticated for four hours, and published the result by cid
rather than sending it to us. No commercial relationship, nothing built on
emem, one WAN client. It is cited here because the standard above says third
party results are the point, and because our other independent scorecard comes
from an agent that runs on this machine.

- report: `ftievssgcklmyqvqn636vc6zeu`
- correction, against two of its own findings: `xupp4kbgllah56hjjcvl6bw23e`
- changeset it proposed: `4l225nmsha3tu7as4eqbqkrbpu`
- amendment to one clause of the changeset: `jtgrfgvunnf5f4l3r2xvat5dtm`

Log head pinned at report time: tree_size 1096727, root
`gzijfg53tbd6zwk4vmbjrutzlff62d2tddok2362wih36oxrjsea`.

Its crypto claims were tested with a clean-room verifier written from
`/v1/verifier_spec` rather than from our source, which reproduced our golden
preimage digest on a live receipt. Five receipt tampers rejected including the
v1-to-v2 downgrade, five inclusion proofs and a live consistency proof verified
under an independent RFC 6962 implementation, 725 requests at 1x to 16x
concurrency with zero errors, and 14 of 14 genuine refusals typed with no 5xx.

**Eleven findings, and eight of them were defects.** Every one reproduced
against production before it was touched; the fixes and what each cost are in
the CHANGELOG. Two it withdrew itself after reading the source, which is the
part worth copying: it had scored a deliberate recovery path as leniency
because it tested the wire and inferred the cause, and it had filed an unsigned
`unknown` as a dishonest absence when refusing to sign "I could not look" is
the correct behaviour.

**What it does not cover**, in its own framing: the write path, self-hosting,
federation, the device gate, model-in-the-loop accuracy, multi-node scaling,
and any peer memory product. Server-side latency is untestable from outside.
One client, one day, one node. SAMPLE.

**The finding we would not have thought to publish**, and it is theirs rather
than ours: between this README being written and that benchmark running, the
live value at the flagship cell moved from 918.0 to 915.0712280273438 because
the band's upstream changed provider. The token published in May still resolves
to 918.0 and still verifies. Nobody designed that demonstration, it happened in
production against a real drift, and it is better evidence for addressed memory
than anything either party could have constructed.

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

**The scope of that rule, because this page does state other numbers and a
reader deserves to know why.** It binds figures from *their* scorecard, which we
do not control: if they re-score, our page must not keep asserting the old value.
Numbers produced by our own instruments, the BM25 baseline and the inversion
p-values below, are ours to state and ours to correct, and the record shows how
badly that is needed: the `compaction_free` verdict below has turned over three
times. Where a number here disagrees with the signed source, the cid wins and
this page has a bug.

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

### The baseline we should have run first: BM25 beat us

The dense-retrieval arm recovers the queried cell **0% to 16.7%** of the time at
hit@5, and we framed that as *retrieval fails*. That framing was wrong. A plain
lexical BM25 baseline over the **identical** corpus, the same questions and the
same two models, with only the retriever changed, recovered the cell **100%** of
the time and answered **16/16 exact**. It matches the addressed arm's accuracy
with no protocol, no minting and no round trips.

| retriever | hit@5 | answers exact |
|---|---|---|
| dense (bge-small) | 0% to 16.7% | 2/12 at best |
| **BM25 lexical** | **100%** | **16/16** |

These are our own baseline numbers, not the third-party scorecard's, so the
no-restatement rule above does not apply to them. They are stated in full in
[how emem compares, section 4](how-emem-compares.md), and the lexical retriever
itself ships in `crates/emem-primitives/src/memory_search/mod.rs`.

The mechanism is legible, which is why nobody should be surprised twice. A
coordinate is a rare literal string. Every chunk in this corpus is therefore
near-identical in embedding space and wildly different in token overlap. The
exact property that defeats cosine similarity is the property BM25 keys on.

So the honest claim is narrow: **dense embedding similarity fails on homogeneous
numeric corpora; lexical retrieval on the same corpus does not.** On a corpus
where a lexical index works, you do not need emem for accuracy, and a reader who
takes "retrieval fails" away from this page has been misled by us.

What addressing still has that BM25 does not, on the same corpus where BM25 won:

- a citation a third party can verify offline, against the signed bytes rather
  than against whatever the index returns today;
- a referent that survives the corpus being rewritten or deleted, because the
  address does not point into the corpus.

Both are real. Both are different claims from "retrieval fails", and neither is
an accuracy claim.

**One measurement that runs in BM25's favour and that we cannot yet match.** We
have not characterised its failure mode, because on this corpus it did not fail.
Dense retrieval's failures we can put a unit on: median drift **252 metres**,
with 50% of answers matching no cell at all. Until someone finds the corpus where
BM25 breaks, the fair statement is that we know how our loser fails and not how
the winner does.

### The differential re-score

One of those NOs was *no independent re-scoring*. We closed it, against
ourselves: [`examples/benchmark-arm/differential_scorer.py`](../examples/benchmark-arm/differential_scorer.py)
recomputes their headline figures from their published bytes using scoring
semantics written from scratch. It deliberately imports none of their scoring
code, because a re-scorer that shares the original's helpers can only reproduce
the original's bugs.

Its results are signed at `7abtisuwss2h72ey7bwbx7gk2y` and, by the same rule as
above, are not restated here. What the reader should know is the *shape* of what
it found:

- **The integrity leg could not be made to fail.** Every published run, sidecar
  and code file matches its recorded hash, and every prompt and answer satisfies
  `cid == base32(blake3(bytes))`. Nothing was scored from unaddressed bytes.
- **The pre-intervention rate reproduces exactly**, to three decimals, from an
  independent implementation.
- **One disagreement with their headline**, and it is a denominator: a small
  number of rows recorded an *empty* generation. Counting those rows as attempts
  gives a lower end-to-end figure than excluding them. emem served correct bytes
  in every such case. The failure is a model that returned nothing, which is
  categorically different from a model that returned a wrong number, and the
  published claim should name which denominator it uses.
- **A double-count trap for re-users**: two of the published runs carry identical
  resolve-arm rows. Their own arithmetic does not double-count them; a third
  party pooling every run naively would.
- **A finding that cuts against us.** In every row where both are known, the
  value the prompt *displays* is a rounded form of the value emem *signed*. So
  the in-context emem arm and the plain context control measure the same skill,
  copying a number already in the window, and addressing contributes nothing
  measurable *in that arm*. The dereference arm is the only one that tests what
  emem actually claims. We would rather deflate our own headline than have a
  reviewer find this first.

The scorer states its own limits: its retrieval-hit detection is more
conservative than theirs, so it can neither confirm nor refute their
conditioned-on-a-hit claim, and it says so rather than reporting a weaker
instrument's disagreement as a contradiction.

### Does agreement between models mean they are right?

No, and this is the result we would most like developers to take away, because
it argues against a habit almost every multi-agent system relies on.

emem pre-registered a prediction before any data existed: under compression,
two models reading the same lossily-summarised memory would **agree with each
other more often than either was correct**. If true, "both agents said the same
thing, so it is probably right" is unsafe exactly where agents share a
compacted context, which is most long-horizon work.

We expected it to fail. We said so in the pre-registration, and gave the
reasons we thought our own hypothesis was weak. It did not fail.

The mechanism is legible in the data. Asked to compact sixteen observations
into a tight budget, the summariser keeps the **range endpoints** and drops
every individual value. A note that says "NDVI values range from -0.14 to 0.79"
is true, useful-sounding, and cannot answer a question about one specific cell.
Both readers then answer with an endpoint, so they agree with each other and
are both wrong. Wrong answers cluster on round numbers and on the most salient
value, and never on the set mean.

Run the numbers yourself rather than reading ours:

```sh
python3 examples/benchmark-arm/score_inversion.py <run>.json
```

There is no hand-maintained figure on this page to drift out of date. The
script recomputes everything from the signed run, and it is deliberately built
to make the result hard to believe rather than easy:

- **It refuses to report at all if the control arm fails.** An earlier run of
  this experiment was voided because every observation had been given the same
  coordinates, so the question was unanswerable and the models were guessing.
  The guessing pointed the same way as the hypothesis. The control arm is the
  only reason a false confirmation was not published, and it was in the design
  because emem asked for it before the data existed.
- **It applies the same verdict rule to both statistics**, at three tolerances.
  Scoring agreement loosely while scoring accuracy strictly would manufacture
  this result. The inversion survives strict equality, and the control never
  inverts at any tolerance.
- **It reports a one-sided Fisher exact test.** The pressure arm's inversion is
  supported (p = 0.035). The `compaction_free` arm's is not (p = 0.109), and the
  script says so rather than presenting both as findings.

  This bullet has now been wrong twice, in opposite directions, and the history
  is more useful than the current value. It first said one arm was underpowered,
  which was our extractor reading the "10" in "the 10 m cell" as the model's
  answer. Corrected, both arms looked supported, so we withdrew the criticism.
  Then the benchmark's author found that a refusal quoting a range contains
  numbers, so first-number extraction scores a model *declining* as a model
  *asserting*. We had that bug too, and checked our own instrument rather than
  taking their word for it. With abstentions excluded, `compaction_free` is not
  established after all.

  We also had the wrong test. Deciding significance by asking whether confidence
  intervals overlap is conservative and reports "not established" for real
  effects: on these numbers it would have called the pressure arm unsupported at
  p = 0.035. Intervals are still printed, because they show the precision of an
  estimate that a p-value hides, but the verdict comes from Fisher.
- **It checks whether both readers saw the same note** and prints the
  consequence: where they did, this measures *correlated error from shared
  memory*, not two agents independently converging on the same mistake. The
  first is real and common. The second is a stronger claim and is untested.

| | |
|---|---|
| Pre-registration (before any data) | `l44rdbk7lcpjt2abzmlkgzpdee` |
| The run | `mtce2egrv5oqf4d2tbwv7cufb6sv3oc7xkt2xqfu5vit4bbc2vtq` |
| Their reading of it | published by attester `6ww7pxav` |
| emem's independent score, including where we disagree with them | `e6ymbtkypniy45sxcgzjkuzxdm` |
| The voided run, published rather than deleted | `ucidjnjp44wsx4rn32kbl3dhd574w36ofjf4i72fp7himxsconbq` |

The two instruments did disagree, and the diff is the most useful thing on this
page. Our agreement figures came out materially lower than theirs in both arms.
We published that as a disagreement and said neither party should quote an
agreement number until it was resolved. They resolved it: **the gap was a bug in
our scorer**, and once fixed our numbers reproduce theirs to three decimals. We
had also called one of their arms underpowered on the strength of it, which was
false and which we have withdrawn ([retraction
`g264c7m2vd34den5dhkicayy5a`](https://emem.dev/channel)).

What remains is a genuine definitional difference, not a bug in either
implementation: they ask whether two models *asserted the same answer*, we ask
whether they *produced the same output*. For a claim about voting mechanisms
theirs is the right notion, so theirs is the one published.

The one criticism of ours that survived: their headline said agreement *rises*
as accuracy collapses. It does not rise. It falls, more slowly than accuracy
falls, which is the real and less dramatic result.

The larger lesson from that exchange is not any single number. The same bug
class, a refusal read as an assertion, was found independently in two
separately-written instruments, and neither party found it alone. Anyone
building a scorer over model output should assume they have it: check for
abstention *before* extracting a value, never after, because a refusal carries
the numbers it is quoting.

The scorecard carries its own reproducibility checklist including six explicit
NOs (no independent replication, no independent re-scoring, no pre-specified
power, no DOI, no model diversity beyond two 7-12B open models on one host).
Independent re-scoring is now closed; the rest need someone outside this
collaboration. If you are that someone,
the arm above runs on your models and the raw data is signed and replayable:
disagreeing with us is the most useful thing you can do here, and
[`tell_us_where_it_hurts`](https://emem.dev/.well-known/mcp.json) is the path in.
