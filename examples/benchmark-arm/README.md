# The canonical emem arm for memory benchmarks

`emem_arm.py` is the addressed-memory arm emem would defend, in the repo emem
owns. It has no dependencies beyond the standard library and no framework
opinion, so an Inspect AI solver, a bare script, or someone else's harness can
call it.

## Why it exists

Third parties benchmark emem against RAG and agent-memory products, and they
write the emem arm themselves. An arm implemented as "hand the model a token
and hope" measures a loop emem does not claim. This is the loop emem does
claim, written down and runnable, so a comparison is against the design rather
than a guess at it.

## The measurement that shaped it

The navigatable_worlds agent ran a real dereference arm on 2026-07-20 (n=56,
two sites, gemma-4-12B and Qwen2.5-7B, greedy) against a responder **without**
the fixes this arm depends on:

| | |
|---|---|
| token copied verbatim, resolved | 46/56 &nbsp; 82.1% |
| of those, value copied verbatim | 36/46 &nbsp; 78.3% |
| **end-to-end byte-identical** | **36/56 &nbsp; 64.3%** |

emem never served a wrong byte. Every loss was in the last mile, and nothing
verified that step. Two distinct failures, which need two distinct fixes:

1. **Head dropped** (17.9%). The model keeps the opaque base32 tail and
   discards `emem:fact:<descriptor>:`, so nothing resolves. Fixed responder
   side: `resolve` now accepts a bare cid and answers `degraded: true`.
2. **Value retyped** (21.7% of resolves). The model re-emits a JSON float from
   its own tokenizer, `0.2411` for `0.241103`. Fixed by `value_verbatim`, an
   exact decimal **string** to copy, and caught by `echo_verify`.

## The loop

```
resolve  ->  value_verbatim, an exact decimal STRING (never a float)
answer   ->  instruct the model to QUOTE it, digit for digit
echo     ->  POST /v1/echo_verify {token, claimed_value}
correct  ->  on drift: re-ask once, then substitute, then re-verify
```

## The claim, stated narrowly

**A single model pass is not reliable. The loop is.** `echo_verify` is a gate,
so a value only leaves the loop after emem has confirmed it matches the signed
fact. Drift becomes a caught event instead of a silent wrong number.

That is a claim about the OUTPUT, not the model, and the report keeps the two
apart on purpose:

- `published_byte_identical`: what the loop emitted. This is the number the
  loop is designed to drive to 1.0.
- `model_byte_identical_first_pass`: what the model managed unaided. This is
  the number that will stay well below 1.0, and it is the honest one to quote
  when comparing MODELS rather than architectures.

Conflating those two would flatter the model with the loop's work. The stub in
`__main__` demonstrates the gap deliberately: a model that rounds on the first
pass yields `model_byte_identical_first_pass: 0.0` and
`published_byte_identical: 1.0`.

## What it will not do

- It does not invent a value the model never produced *silently*. If the model
  refuses twice, the arm substitutes the signed value and reports
  `corrected: true`, so a reader can subtract those cases.
- It does not hide a degraded citation: a bare cid that resolves is reported as
  `degraded_citation: true`.
- It does not fix retrieval. This arm is about dereferencing a citation you
  already hold.

## Use

```python
from emem_arm import EmemArm

arm = EmemArm(answer_fn=my_model)          # answer_fn(prompt) -> str
out = arm.answer(token, "What is the NDVI here?",
                 exclude=("32.57272", "77.03276"))   # question's own coords
print(out["value"], out["verified"], out["first_pass_ok"], out["drift"])
print(arm.report())
```

`exclude` exists because of a real scoring bug: at a London site the question's
own longitude (-0.13) is itself a plausible NDVI, so naive first-number
extraction scored correct answers as wrong. Pass the question's coordinates.

Self-check against the live responder:

```sh
python3 examples/benchmark-arm/emem_arm.py
```

## The differential scorer

`differential_scorer.py` re-scores the navigatable_worlds agent's published runs
from their recorded bytes, using scoring semantics written here rather than
imported from the harness that produced them. It exists because their scorecard
listed *no independent re-scoring* as an open gap and emem is the party that
asked for it.

```sh
python3 differential_scorer.py /path/to/navigatable_worlds --manifest-only
python3 differential_scorer.py /path/to/navigatable_worlds --offline   # skip live resolves
```

It runs three legs in order, and refuses to score bytes that fail the first:

1. **Integrity.** Every run, sidecar and code file against `PROVENANCE.json`,
   and every prompt/answer against its own content address,
   `cid == base32(blake3(bytes))`.
2. **Ground truth, twice.** The value the prompt *displayed*, parsed from the
   prompt bytes, and the value emem *signed*, resolved live. These are not the
   same object: a prompt may display `0.747614` for a signed
   `0.7476139978791093`. A scorer with one notion of "expected" silently
   mis-scores one arm or the other.
3. **Scoring, decomposed.** `exact`, `numeric_equal` (right value, wrong
   bytes: the rounding failure), `confidently_wrong`, and `abstain`, kept
   separate because collapsing them is how a benchmark flatters itself.

Requires `blake3` and `pynacl` only for the integrity leg; `--offline` drops
the live responder calls.

Its own two bugs, found by running it and reported rather than quietly fixed:
citation grading originally compared against the *first* token in a prompt that
offered several, marking correct citations as misses; and retrieval-hit
detection counted differently-shaped handoff prompts as hits, inflating the hit
rate threefold. Both are fixed and commented at the site of the fix.

## The inversion scorer

`score_inversion.py` tests whether two models agreeing is evidence that they are
right. Under compression it is not, and that is the finding this directory
exists to let you check rather than believe.

```sh
python3 score_inversion.py /path/to/runs/pressure_lahaul_v2.json
```

It is built to resist its own conclusion:

- **A ceiling gate.** If the control arm (every value shown verbatim) scores
  below 0.5, the script refuses to print anything. The first run of this
  experiment had a ceiling of 0.208 because a coordinate bug made every
  question unanswerable, and the resulting noise happened to point the same way
  as the hypothesis. Without the control, a false confirmation would have been
  published.
- **One verdict rule for both statistics**, at three tolerances, because
  scoring agreement loosely and accuracy strictly manufactures an inversion.
- **Wilson intervals**, which say plainly when an arm is underpowered rather
  than presenting it alongside an established one.
- **A shared-prompt check.** Where both readers saw a byte-identical note, the
  result is correlated error from shared memory, not independent convergence.
  The script prints that caveat rather than leaving it to the writeup.
