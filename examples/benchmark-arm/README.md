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

- `published_byte_identical` — what the loop emitted. This is the number the
  loop is designed to drive to 1.0.
- `model_byte_identical_first_pass` — what the model managed unaided. This is
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
