# Verifiable lending due-diligence

A bank underwrites a loan against a piece of land. The credit memo asserts things
about that land: its elevation, its vegetation, whether it floods. This example
makes those assertions twice, once the way a model writes them by default and
once with every physical claim bound to a signed fact, and runs both through
emem-guard.

```sh
python3 run.py            # the full walkthrough
python3 run.py --probe    # four checks on what the guard does and does not catch
python3 run.py --lat 19.0760 --lng 72.8777    # any coordinate you like
```

Python 3 standard library only. No `pip install`, no API key, no account. It
talks to `https://emem.dev` over plain HTTP and takes about a minute.

## What it walks through

1. **Ground the parcel.** A coordinate becomes one canonical `cell64`. Two agents
   that locate the same point get the same address, so two memos about the parcel
   are about the parcel and not about two paraphrases of it.
2. **Recall signed facts.** Fourteen bands, each with a value, a unit, a
   confidence and an `emem:fact:` token, under one ed25519 receipt.
3. **Apply published algorithms.** `vegetation_class_from_ndvi@1` and
   `flood_risk@2` come out of the content-addressed registry, not out of this
   script. It prints the arithmetic term by term and cross-checks its own result
   against the value the responder computed. Both must agree or it says so.
4. **Two memos.** Naive and grounded, same parcel, same numbers.
5. **Two verdicts.** From `POST /v1/guard/verdict`.
6. **Findings**, computed from the responses rather than written in advance.

## What emem-guard proves, and what it does not

This is the part worth reading twice, because the failure mode is a control that
sounds stronger than it is.

`--probe` runs four checks:

| | claim | citation | verdict |
|---|---|---|---|
| A | measurable, anchored to a place | none | **deny** `CLAIM_UNGROUNDED` |
| B | same | a real token with 4 characters changed | **deny** `CLAIM_UNGROUNDED` |
| C | same | a token that resolves | allow |
| D | **false number** | the correct token for that fact | **deny** `PROV_VALUE` |

**`action: allow` is not clearance.** It means no rule fired. The field that
carries information is `receipt.fact_cids`, and the test that separates a real
citation from an invented one is:

```
len(receipt.fact_cids) == citations_found
```

**That test passes D, and D is false.** The arithmetic balances because the
citation is real: right cell, right band, resolves, verifies. What catches D is
`PROV_VALUE`, a rule that reads the value *inside* the fact and compares it with
the number in the sentence. Its fix is `correct_value`, never "drop the
citation", because the citation is the sound half.

Agreement is judged at the precision you wrote. Reporting `889.6439208984375 m`
as `889.6 m` is correct; `5000 m` is not.

**What is still not checked: relevance.** A citation that resolves is not
evidence that it is a citation *about the place the sentence names*. The probe
measures this and says so — every check says "at Jagraon" while citing a fact
from a different `cell64`, and the guard allows it. Step 3 is the honest answer
to that: recompute the published algorithms and compare against the responder's
own value.

Guard verdicts from `emem.dev` are **advisory**: `blocks_nothing: true`. To
enforce, run your own node (`GET /v1/guard/selfhost`), which also lets you check
a corpus this responder does not hold.

## About `expected_output.txt`

A real captured run, not a hand-written mock. It will not match yours line for
line, and that is the point: NDVI, weather and cloud cover are live readings that
move, and fact cids move with them. Elevation, surface-water recurrence and the
`cell64` are stable.

Behaviour B changed on 2026-08-08: an unresolvable citation used to suppress the
denial the sentence had earned, so pasting any well-formed token bought an
`allow` with an empty `fact_cids`. It is now a deny. If your run disagrees with
the capture, the script prints the difference rather than asserting the
expectation, so trust your run over this file.

## Honest limits

- One cell is roughly 10 m across. A claim about hectares needs many cells, and
  the script says so instead of implying one reading settles it.
- A band that does not materialize is reported with its `reason_class`. A
  `no_materializer` or an `upstream_error` is **silence**, not an observation.
  Only a signed `Absence` is the corpus answering "we looked, nothing there".
- Nothing here prices land or judges a borrower. It checks physical claims about
  a place, which is a smaller thing than a credit decision.
