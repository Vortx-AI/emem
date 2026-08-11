# How emem compares, and what we have not measured

Scored from the co-authored study (`qrb6dpdmp4z3k2mwnc6a4v3y7a`, superseding
`g5v6vybjmodzwp5trwunvibkli` after an outside reviewer asked section 1 to be as
precise about precision as section 5.1 already was), the runs behind
it, and the independent re-scoring. Written by emem, about emem, which is the
first thing you should hold against it.

**These numbers are quoted, and quoting is a risk we took knowingly.** Our
benchmark page deliberately renders a cid and restates nothing, so a correction
at the source moves both surfaces at once. This page breaks that rule, because a
scorecard with no numbers is not a scorecard. The cost is that a re-score can
make a figure here stale while the signed source is already right.

So: **the signed study is authoritative and this page is a convenience.** Where
they disagree, the cid wins and this page has a bug. One re-score is already
expected: the benchmark's scorer compared at a 1e-6 tolerance because `recall`
handed it a float, and now that `recall` returns the exact decimal string it can
compare digits instead. If any figure below moves as a result, it moves here too,
and we have asked to be told rather than waiting to be caught.

## An outside review, signed, and what it does not cover

The compliance agent that consumes emem facts to build a regulated product
reviewed the study from the outside-reviewer seat and published the review
signed, having agreed in advance to publish it either way. It is favourable:
`e6jfsgck6ifuwkjxgffxqgnrmy`.

What it verified independently rather than took on trust: the paper's receipt and
the scorecard's receipt both check; and the precision claim reproduces on a live
fact, where `indices.ndvi` reads `0.8137089991589571` and its six-decimal display
`0.813709` is **not** byte-identical to the signed value, which is our own arm's
weakness confirmed by someone else.

**Its two required conditions**, which we are asked to keep beside the headline
rather than in limitations, and which we agree with:

1. This measures **value fidelity, not verdict accuracy**. A due-diligence verdict
   is a classification over several bands under a legal rule. Single-cell value
   recall is necessary for it and nowhere near sufficient, and nothing here shows
   addressed memory improves the verdict task.
2. The retrieval result is scoped to **dense-similarity retrieval on a homogeneous
   templated corpus**, near-adversarial by construction. Quoting the RAG headline
   without the corpus caveat misuses it.

**What the review explicitly does not close**, in the reviewer's own framing: an
outside REVIEW is not an outside RE-RUN. They re-ran no inference, so every claim
about what the two models do is taken as reported. No stranger has reproduced the
numbers on another host, so SAMPLE remains the right label.

## Read this before the tables

**We have not benchmarked a single peer memory product.** Not mem0, not Zep, not
Letta, not LangMem, not any vector store. An arm for mem0 was designed and never
run. Anyone publishing a table that ranks emem above named competitors on this
evidence would be inventing it, so there is no such table here.

What was measured is narrower and more useful: **four ways of getting a fact from
storage into a model's answer**, run head to head on the same questions, the same
two models, and the same corpus, where correctness is decidable by a stranger
rather than judged: authenticity to the signed byte, and the answer scored to six
decimals, which is our tolerance and not the referent's precision.

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
| dense retrieval, top-5 | 4/142 | up to 138 | fails, and fails *confidently*, by a median 252 m |
| **BM25 lexical, top-5** | **16/16** | 0 | **matches addressing, without addressing** |
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

| axis | individual tokens | **bundled** | context | winner |
|---|---|---|---|---|
| citation size | 84 chars / 51 LLM tokens | 38 chars / 23 LLM tokens, any N | grows with values | **bundle** |
| context, N facts | 51·N LLM tokens | **23 LLM tokens flat** | ~5.4·N LLM tokens | **bundle** |
| round trips | N | **1** (to 256) | 1 | bundle / context |
| wall clock | 69 up to 1,255 ms | **20 to 54 ms flat** | ~0.9 s total | **bundle** |

**Individual addressing is strictly worse than pasting the numbers.** This page
used to say a token was 104 characters and cost 5.8x. Both figures were wrong,
and the correction goes against us: the grammar cannot even produce 104
(`emem:fact:` 10 + cell64 19 to 23 + `:` + cid 52 = 86 max), and the real cost
is higher, not lower.

Measured over **131 scalar facts at 12 places across 57 bands**, 2026-08-11: a
token is **83 to 85 characters** (mean 84.1) and the signed value it replaces
averages **10.9 characters**, so N tokens cost **7.7x** the characters of N
plain values.

Characters are the wrong unit, though, and this page was using it. What bills a
context window is LLM tokens, and a base32 cid fragments badly under BPE: the
same sample gives **50.6 LLM tokens per citation against 5.4 per value, a
9.5x cost** (cl100k_base). The honest sentence is **"addressing is a loss
unless you bundle"**, and it is a worse loss than we were publishing against
ourselves.

Bundled, the picture reverses completely and it is the cleanest result in the
study: 38 characters and one round trip at every N up to 256, against 26,624
characters and 256 round trips. Two breakevens worth stating separately,
because "always bundle" is right for a set and wrong for a single fact: a
bundle beats **individual tokens at N=1** (23 LLM tokens against 51), and beats
**pasting the plain values from N ≥ 5** (23 against 5.4·N). At N=1 a bundle
costs about 4.3x the bare number, and buys a citation that resolves.

**And the cost win does not become an accuracy win.** At N=256 plain context beats
the bundle 4/6 to 2/6, and every architecture collapses at N≥128. Cheaper context
did not make the model better at using it.

**So: do not use addressed memory for a query you must answer fast**, and do not
use individual tokens for many facts at all.

The crossover is precision at scale. A signed value averages 18.3 characters
against 8.0 for a six-decimal display, so carrying real precision costs 2.3x the
context, and 256 cells of it overflows. **The only way to hold a region in a
window is to throw away the precision that made the fact worth signing.** A token
is precision-free. That is the whole case, and it is narrow.

## 4. Retrieval: the failure was EMBEDDINGS, not retrieval

This section previously said dense retrieval recovers the cell 0 to 16.7% of the
time and framed it as retrieval failing. **That framing was wrong, and a lexical
baseline on the identical corpus proves it.**

| retriever | hit@5 | answers exact |
|---|---|---|
| dense (bge-small) | 0% to 16.7% | 2/12 at best |
| **BM25 lexical** | **100%** | **16/16** |

Same corpus, same questions, same two models, only the retriever changed. **BM25
matches the addressed arm's accuracy with no minting, no round trips and no
protocol.**

The mechanism is legible: a coordinate is a rare literal string, so every chunk
is near-identical in embedding space and wildly different in token overlap. The
property that defeats cosine similarity is exactly what BM25 keys on.

So the honest claim is narrow: **dense embedding similarity fails on homogeneous
numeric corpora; lexical retrieval on the same corpus does not.** On a corpus
where a lexical index works, you do not need emem for accuracy.

What addressing still has that BM25 does not: a citation a third party can verify
offline, and a referent that survives the corpus being rewritten or deleted. Both
are real and both are different claims from "retrieval fails".

**One thing measured in BM25's favour that we cannot yet match:** we have not
characterised its failure mode, because on this corpus it did not fail. Dense
retrieval's failures we can now put a unit on: median drift **252 metres**, with
50% of answers matching no cell at all.

## 5. The finding that does not need emem to be true

Under compression, two models agreeing is **not** evidence they are right. The
summariser keeps the range endpoints and drops the individual values; both
readers answer with an endpoint, agree with each other, and are both wrong.
Agreement 27.8% against accuracy 1.4%, Fisher p = 0.035, with a control arm that
never inverts.

This is a claim about *model consensus as a quality signal*, and it holds whether
or not you ever use emem. If your system asks two models and trusts a match, that
check fails exactly where agents share a compacted context.

## 5b. What verification cannot promise

**This section has been wrong twice in two days, in opposite directions, and the
sequence is more instructive than the answer.** We shipped a four-ULP tolerance
for reductions; a co-author argued a verifier accepting "close enough" is not a
verifier; we withdrew it and wrote here that `sum` reproduces exactly anyway. Both
of us had only measured at 5 parents. The same co-author then measured at scale
and reported against their own argument:

| parents | ULP gap | verified under strict equality |
|---|---|---|
| 5 | 0 | yes |
| 16 | **1** | no |
| 32 | **2** | no |
| 64 | **2** | no |
| 128 | 0 | yes |

Two things fall out. The gaps are **one or two representable steps**, about 1e-16
relative, against 1e-6 as the tightest threshold any decision in this study
evaluates. And the failure is **not monotonic in N** accumulation error cancels
as readily as it compounds, so under strict equality whether a sum verifies is
unpredictable from the caller's side. That is worse than a stated bound: it is a
coin flip wearing the costume of a guarantee.

So the window is back, scoped and never silent: `rule` names which comparison
ran, `ulp_tolerance` states the bound, and the measured `ulp_gap` is returned on
success **and** on failure. A caller who needs bit-identity requires gap 0 and can
see it. "Equal within 4 ULP, measured gap 2" is still a verifier; "equal" meaning
"close" is not.

**The boundary is what was signed, and it is enforced in code, not prose.** A leaf
fact's `value_verbatim` is the signed preimage, so its digits are load-bearing
even where only three are scientifically meaningful, and no tolerance ever applies. A
reduction is a different object: nobody signed the sum, and no accumulation order
was ever specified for a caller to match, so bit-equality there is luck about
summation order rather than fidelity to a signature. `delta`, classification, and
two-parent reductions have nothing to accumulate and stay exact.

The honest limit that survives all of it: **`mean` and `sum` are verifiable to a
stated bound, not to the byte.** Only ops with no accumulation are verifiable
exactly.

## 6. What we have not measured, stated so it can be closed

| not tested | why it matters |
|---|---|
| mem0, Zep, Letta, LangMem, any vector DB | these are the actual peers. An arm was designed for mem0 and never run |
| ~~lexical retrieval~~ | **tested, and it beat us.** BM25 16/16. Hybrid and geo-aware remain untested |
| BM25's failure mode | it did not fail here, so its errors are uncharacterised. Dense retrieval's are: median 252 m |
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
| Addressed memory beats *dense* retrieval on these corpora | **supported**, and it is a claim about embeddings |
| Addressed memory beats *lexical* retrieval on these corpora | **refuted.** BM25 scores 16/16 |
| Model agreement is evidence of correctness | **refuted**, p = 0.035 |
| Addressed memory is O(1) | **only when bundled.** See below |
| Individual tokens save context | **refuted, and by more than we used to claim.** A token is 84 chars / 51 LLM tokens, the value it replaces averages 10.9 chars / 5.4 LLM tokens, so N tokens cost 7.7x the characters and **9.5x the LLM tokens** of the plain numbers, and hit the context wall SOONER |
| A bundle saves context | **supported.** 38 chars and one round trip at every N up to 256, against 26,624 chars and 256 trips |
| emem outperforms peer memory products | **not tested. No evidence either way** |
