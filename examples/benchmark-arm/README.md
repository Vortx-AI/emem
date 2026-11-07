# The canonical emem arm for memory benchmarks

`emem_arm.py` is the emem arm for a memory benchmark, written by the emem
project. It uses only the Python standard library and no framework. An Inspect
AI solver, a plain script or another benchmark program can call it.

## What you need

- Python 3 (the checks on this page ran with Python 3.14.4 on 7 November 2026).
- Network access to `https://emem.dev`. The arm only reads; it does no writes.
- A model function `answer_fn(prompt) -> str`. The self-check uses a stub, so
  you do not need a model key to run it.
- For `differential_scorer.py` and `score_inversion.py`: the run files of the
  navigatable_worlds benchmark. They are not in this repository. `blake3`
  (`pip install blake3`) is optional; it turns on the content-address check.

## What you get

- One loop that resolves a citation, makes the model quote the value, checks
  the quote with emem, and corrects drift.
- A report with two numbers: what the loop published, and what the model got
  right without help.
- Two scorers that re-score recorded benchmark runs from their bytes.

## Why the arm exists

Other teams benchmark emem against RAG and agent-memory products. They write
the emem arm themselves. An arm that only gives the model a token measures a
loop that emem does not claim. This file is the loop that emem does claim. A
comparison against it tests the design, not a guess at the design.

## The measurement behind the design

On 20 July 2026 the navigatable_worlds agent ran a dereference arm. It used
n=56, two sites, gemma-4-12B and Qwen2.5-7B, and greedy decoding. The
responder did not yet have the fixes that this arm uses. The agent then ran
the same arm again after the fixes. Both runs are in
`docs/collaboration-log.md`.

| Step | Before the fixes | After the fixes |
|---|---|---|
| Token copied verbatim | 46/56 (82.1%) | 82.1% |
| Resolve failed | 17.9% | 1.8% |
| Resolved only through a bare cid (`degraded`) | not available | 16.1% |
| Value copied verbatim, of the resolves | 36/46 (78.3%) | 55/55 (100%) |
| End-to-end byte-identical | 36/56 (64.3%) | 98.2% |

In the first run, emem served no wrong byte. All losses were in the last step,
and nothing checked that step. There were two different failures, with two
different fixes:

1. The model dropped the head of the token (17.9%). It kept the base32 tail
   and dropped `emem:fact:<descriptor>:`, so nothing resolved. The fix is on
   the responder: `resolve` now accepts a bare cid and answers
   `degraded: true`.
2. The model retyped the value (21.7% of resolves). It wrote a JSON float in
   its own form, for example `0.2411` for `0.241103`. The fix is
   `value_verbatim`, an exact decimal string to copy. `echo_verify` catches
   the failure when it still occurs.

## The loop

```
resolve  ->  value_verbatim, an exact decimal STRING (never a float)
answer   ->  instruct the model to QUOTE it, digit for digit
echo     ->  POST /v1/echo_verify {token, claimed_value}
correct  ->  on drift: re-ask once, then substitute, then re-verify
```

## The claim and its limits

One model pass is not reliable. The loop is reliable, because `echo_verify` is
a gate. A value leaves the loop only after emem confirms that it matches the
signed fact. A drift becomes an event that the loop records, not a silent
wrong number.

This is a claim about the output of the loop, not about the model. The report
keeps the two numbers apart:

| Field | What it counts |
|---|---|
| `published_byte_identical` | Values the loop published that match the signed value. The loop aims to bring this to 1.0. |
| `model_byte_identical_first_pass` | Values the model got right on its first pass, without help. Quote this number when you compare models, not architectures. |

If you merge the two numbers, the model gets credit for the work of the loop.
The self-check shows the gap. Its stub model rounds on the first pass, so the
report gives `model_byte_identical_first_pass: 0.0` and
`published_byte_identical: 1.0`.

## What the arm does not do

- It does not hide a value that the model did not produce. If the model fails
  the first pass and the re-ask, the arm publishes the signed value and sets
  `corrected: true`. A reader can then remove those cases.
- It does not hide a degraded citation. If a bare cid resolves, the result has
  `degraded_citation: true`.
- It does not fix retrieval. The arm dereferences a citation that you already
  have.

## Run the self-check

Run this from the repository root. It reads one NDVI-delta fact from emem.dev
and uses a stub model that rounds on the first pass.

```sh
python3 examples/benchmark-arm/emem_arm.py
```

Output on 7 November 2026:

```
{
 "value": "-0.052199413489736016",
 "verified": true,
 "drift": "rounded",
 "corrected": false
}
{
 "n": 1,
 "resolved": 1,
 "degraded": 0,
 "verified_first_pass": 0,
 "verified_after_retry": 1,
 "corrected": 0,
 "failed": 0,
 "published_byte_identical": 1.0,
 "model_byte_identical_first_pass": 0.0
}
```

## Use the arm in your benchmark

Put `examples/benchmark-arm` on your import path, then:

```python
from emem_arm import EmemArm

arm = EmemArm(answer_fn=my_model)          # answer_fn(prompt) -> str
out = arm.answer(token, "What is the NDVI here?",
                 exclude=("32.57272", "77.03276"))   # the question's own coordinates
print(out["value"], out["verified"], out["first_pass_ok"], out["drift"])
print(arm.report())
```

`token` is an `emem:fact:` token, or a bare fact cid. The `exclude` argument
fixes a real scoring bug. At a London site the longitude of the question
(-0.13) is also a possible NDVI value. A scorer that takes the first number in
the answer then marked correct answers as wrong. Pass the coordinates of the
question in `exclude`.

## The differential scorer

`differential_scorer.py` re-scores the published runs of the
navigatable_worlds agent from their recorded bytes. This file holds its own
scoring rules. It does not import them from the program that made the runs.
The scorecard of that agent listed "no independent re-scoring" as an open gap,
and emem asked for this scorer.

You need the navigatable_worlds directory. It holds `runs/` and
`benchmark/PROVENANCE.json`. It is not in this repository. Without a path, the script
uses `/home/ubuntu/navigatable_worlds`.

```sh
python3 examples/benchmark-arm/differential_scorer.py /path/to/navigatable_worlds
python3 examples/benchmark-arm/differential_scorer.py /path/to/navigatable_worlds --manifest-only
python3 examples/benchmark-arm/differential_scorer.py /path/to/navigatable_worlds --offline
```

`--manifest-only` scores only the runs listed in the manifest. `--offline`
does not call the responder, so the signed values stay empty.

The script prints its checks in this order:

1. Integrity. It hashes each run file and sidecar against
   `benchmark/PROVENANCE.json`. It also recomputes the address of each
   sidecar entry (prompts and answers) as `cid == base32(blake3(bytes))`. Without the `blake3` package, it skips the
   address check and says so.
2. Ground truth, two times. It parses the value that the prompt showed from
   the prompt bytes. It resolves the value that emem signed from the
   responder. These can differ: a prompt can show `0.747614` for a signed
   `0.7476139978791093`. A scorer with only one expected value mis-scores one
   of the arms.
3. Scores, in separate classes: `exact`, `numeric_equal` (right value, wrong
   bytes, which is the rounding failure), `confidently_wrong` and `abstain`.
   If you merge these classes, the benchmark looks better than it is.

The script does not stop when an integrity check fails. It prints the
integrity counts first and then scores all rows. Read the integrity counts
before you use the scores.

When the authors ran the script, they found two bugs in it. They fixed both
and put a comment at each fix. First, citation grading compared the answer with the first token in a
prompt that offered several, so it marked correct citations as misses. Second,
retrieval-hit detection counted handoff prompts, which have a different shape,
as hits. That made the hit rate about three times too high (0.086 against
0.026).

## The inversion scorer

`score_inversion.py` tests one question: when two models agree, is that
evidence that they are right? Under compression it is not. The script lets
you check this finding yourself. Like the differential scorer, it needs a run
file from the navigatable_worlds directory.

```sh
python3 examples/benchmark-arm/score_inversion.py /path/to/navigatable_worlds/runs/pressure_lahaul_v2.json
```

Without an argument, the script prints its description and exits with code 2.

The script has these guards against its own conclusion:

- A ceiling gate. The control arm shows every value verbatim. If it scores
  below 0.5, the script does not report an inversion. The first run (v1) of
  this experiment scored the control arm at 0 of 72, because a coordinate bug
  made every question impossible to answer. The noise in that run pointed the
  same way as the hypothesis. Without the control arm, the authors would
  have published a false confirmation.
- One verdict rule for both statistics, at three tolerances: strict, 0.005 and
  0.05. Loose scoring of agreement with strict scoring of accuracy makes an
  inversion that is not real.
- Wilson intervals. They show when an arm has too few samples, instead of
  showing it next to an arm with enough.
- A shared-prompt check. If both readers saw a byte-identical note, their errors
  correlate through shared memory. That is not independent agreement. The
  script prints this caveat itself.
