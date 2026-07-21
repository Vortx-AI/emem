# How emem compares, and what we have not measured

Scored from the co-authored study (`g5v6vybjmodzwp5trwunvibkli`), the runs behind
it, and the independent re-scoring. Written by emem, about emem, which is the
first thing you should hold against it.

## Read this before the tables

**We have not benchmarked a single peer memory product.** Not mem0, not Zep, not
Letta, not LangMem, not any vector store. An arm for mem0 was designed and never
run. Anyone publishing a table that ranks emem above named competitors on this
evidence would be inventing it, so there is no such table here.

What was measured is narrower and more useful: **four ways of getting a fact from
storage into a model's answer**, run head to head on the same questions, the same
two models, and the same corpus, where correctness is decidable to six decimals
by a stranger rather than judged.

Scope that bounds every number below: 5 sites, 2 open 7-12B instruct models on
one inference host, up to 1,024 cells, n=48 at the largest size. **No independent
replication.** Two of the three participating agents want addressed memory to
win, and all three run on the same machine.

## 1. Fidelity: does the exact value survive?

| architecture | exact | confidently wrong | what it means |
|---|---|---|---|
| citation + value in context | 284/284 | 0 | lossless |
| plain context (control) | 284/284 | 0 | lossless |
| citation alone, dereferenced | 99.2% end-to-end | 0 | lossless after the last-mile fix, 84.4% before |
| dense retrieval, top-5 | 4/142 | up to 138 | fails, and fails *confidently* |
| summarised memory, tight budget | 1/72 | most of the rest | fails |

**The first two rows tie, and that matters more than it looks.** Our own
re-scoring found the citation arm displays a *rounded* value, so it and plain
context are measuring the same skill: copying a number already in the window.
**Addressing contributes nothing measurable in that arm.** If your answer needs
one value and it fits, context is not worse than emem. It is the same, and
cheaper.

The dereference row is the one that tests what emem claims, and it only reaches
99.2% *after* four fixes prompted by the benchmark finding it at 84.4%.

## 2. Safety: what happens when memory fails

The most operationally important result, and it is about the models rather than
the architectures. On retrieval failure:

- one model **abstained** 74/96
- the other **emitted a confident wrong number** 93/96

The wrong numbers were real measurements from *neighbouring cells*: plausible,
correctly formatted, and wrong. That is the failure that survives a sanity check
and flips a threshold decision. Possession of the exact bytes eliminated
confident value errors in 280 arm-model observations.

## 3. Cost: two of four axes go against us

The part a vendor usually buries.

| axis | addressed memory | context | winner |
|---|---|---|---|
| citation size | O(1). A token is the same length whether the value has 6 digits or 17 | grows with values and digits | **emem** |
| context consumed | O(1). 90 tokens at 256 cells and at 1,024 | O(N). 4,212 → 15,786, then a hard wall | **emem** |
| round trips | 1 per fact; a bundle covers 256, so ceil(N/256) | 1 | **context** |
| wall clock | 6–9 s | ~0.9 s | **context, by 7–10x** |

**So: do not use addressed memory for a query you must answer fast.** If one
value per question fits in your window, you are paying 7–10x latency for a
property you are not using.

The crossover is precision at scale. A signed value averages 18.3 characters
against 8.0 for a six-decimal display, so carrying real precision costs 2.3x the
context, and 256 cells of it overflows. **The only way to hold a region in a
window is to throw away the precision that made the fact worth signing.** A token
is precision-free. That is the whole case, and it is narrow.

## 4. Where retrieval actually stands, both halves

Dense retrieval recovered the queried cell at **hit@5 ≤ 8.3%** on a homogeneous
corpus, **16.7%** on a diverse one at equal size, then **0%** as that corpus grew.

Both halves must be quoted together. The first corpus was near-adversarial for
embeddings by construction and we said so before running; the second shows
retrieval doing better when there is semantic diversity to exploit, and then
degrading as the corpus approaches the scale a real deployment has. The honest
claim is **"retrieval reliability is corpus-size dependent and degrades at scale
on this task"**, never "RAG fails".

## 5. The finding that does not need emem to be true

Under compression, two models agreeing is **not** evidence they are right. The
summariser keeps the range endpoints and drops the individual values; both
readers answer with an endpoint, agree with each other, and are both wrong.
Agreement 27.8% against accuracy 1.4%, Fisher p = 0.035, with a control arm that
never inverts.

This is a claim about *model consensus as a quality signal*, and it holds whether
or not you ever use emem. If your system asks two models and trusts a match, that
check fails exactly where agents share a compacted context.

## 6. What we have not measured, stated so it can be closed

| not tested | why it matters |
|---|---|
| mem0, Zep, Letta, LangMem, any vector DB | these are the actual peers. An arm was designed for mem0 and never run |
| lexical, hybrid, or geo-aware retrieval | only dense similarity was tested, so "retrieval" is unproven beyond that |
| frontier API models | two open 7-12B models on one host. A reader will assume otherwise unless told |
| conversational memory benchmarks | LOCOMO and LongMemEval evaluate a different referent (authored text spans) |
| independent replication | nobody outside this collaboration has run any of it |
| independent compaction | both readers saw the same summary, so this is correlated error, not convergence |

**The most useful thing anyone can do with this document is contradict it.** The
arm we would defend runs on your models
([`examples/benchmark-arm/`](../examples/benchmark-arm/)), the raw data is signed
and replayable, and the scorer that disagreed with its own authors is in the same
directory. Six scoring bugs were found across two independently written
instruments during this study; a seventh is more likely than not.

## Scorecard, honestly

| claim | verdict |
|---|---|
| A citation survives compaction where a paraphrase does not | **supported**, and it is the core claim |
| Addressed memory beats plain context when the value fits | **refuted by our own re-scoring** |
| Addressed memory beats dense retrieval on these corpora | **supported**, with the corpus caveat attached |
| Model agreement is evidence of correctness | **refuted**, p = 0.035 |
| Addressed memory is O(1) | **only in size and context.** Round trips cap at 256; latency is 7–10x worse |
| emem outperforms peer memory products | **not tested. No evidence either way** |
