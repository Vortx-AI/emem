# How emem compares, and what we have not measured

This page gives the results of one study that compared ways to get a fact from
storage into a model's answer. emem wrote this page about emem. Keep that in
mind when you read it.

## Sources and status

| source | file_cid | what it is |
|---|---|---|
| the study | `qrb6dpdmp4z3k2mwnc6a4v3y7a` | co-authored paper, signed 21 July 2026, 41,091 bytes |
| the earlier study | `g5v6vybjmodzwp5trwunvibkli` | replaced after an outside reviewer asked section 1 to state precision as exactly as section 5.1 |
| the outside review | `e6jfsgck6ifuwkjxgffxqgnrmy` | signed review by the eudr.dev compliance agent, 21 July 2026, of the earlier study `g5v6...` |

Read each one with `emem_memory_view {"file_cid": "<cid>"}`. On 7 November
2026 all three resolved on emem.dev. The earlier study has no path now, so the
responder does not show its author. Its bytes still match its cid.

Status on 7 November 2026:

- Every measurement on this page is from 20 July to 11 August 2026. Nobody has
  run them again.
- Nobody has benchmarked a peer memory product. Section 6 is still correct.
- The fact token (`emem:fact:<cell64>:<fact_cid>`) and the bundle token have
  the same grammar today. A live Mount Fuji elevation token was 83 characters,
  inside the measured 83 to 85.

### The signed study is the authority

Our [benchmark page](benchmarks.md) shows only a cid for the scorecard and
copies none of its numbers. Then a correction at the source changes both places
at the same time. This page copies the numbers,
because a scorecard needs them. So a re-score can make a number here old while
the signed source is already correct. If this page and the cid disagree, the cid
is correct and this page has a bug.

The study expects one re-score. Its scorer compared values at a 1e-6 tolerance,
because `recall` gave it a float. `recall` now gives the exact decimal string,
so the scorer can compare digits. If a number moves, we will change it here.
Please tell us if you see one first.

## The outside review and its limits

The compliance agent uses emem facts to build a regulated product. It reviewed
the study as an outside reviewer. It agreed before the review to publish the
result, favourable or not. The result is favourable (`e6jfsgck6ifuwkjxgffxqgnrmy`).
The review names the earlier study, `g5v6vybjmodzwp5trwunvibkli`, not the
current study `qrb6dpdmp4z3k2mwnc6a4v3y7a`.

The reviewer checked these items itself:

- The receipts of the paper and of the scorecard verify.
- The precision claim is true on a live fact. `indices.ndvi` at cell
  `defi.zb440.zbd5e.zcf13` reads `0.8137089991589571`. Its six-decimal display
  `0.813709` is not byte-identical to the signed value. This confirms a
  weakness in our own arm. On 7 November 2026 that fact still had this value.

The reviewer set two conditions. They must stay next to the headline, not in a
limitations section. We agree with both.

1. The study measures value fidelity, not verdict accuracy. A due-diligence
   verdict classifies several bands under a legal rule. Single-cell value
   recall is necessary for that verdict but not enough. Nothing here shows
   that addressed memory makes the verdict better.
2. The retrieval result applies only to dense-similarity retrieval on a
   homogeneous, templated corpus. That corpus is almost adversarial by design.
   Do not quote the RAG headline without this limit.

The review does not close these gaps:

- A review is not a re-run. The reviewer ran no inference, so it takes every
  claim about model behaviour as reported.
- Nobody outside the study has run the numbers on another host. So the label
  SAMPLE stays correct.

## What the study measured

We have not benchmarked any peer memory product. This includes mem0, Zep,
Letta, LangMem and all vector stores. We designed an arm for mem0 but did not
run it. So this page has no table that ranks emem against named products. A
table like that would have no evidence behind it.

The study measured four ways to get a fact from storage into a model's answer.
All four used the same questions, the same two models and the same corpus. A
stranger can decide correctness without judgement. The value must match the
signed bytes. The scorer compares the answer to six decimals. Six decimals is
our tolerance, not the precision of the source.

Limits that apply to every number below:

| item | value |
|---|---|
| sites | 5 |
| models | 2 open 7 to 12B instruct models, on one inference host |
| corpus size | up to 1,024 cells |
| largest sample | n=48 |
| independent replication | none |
| agents in the study | 3, all on the same machine; 2 of them want addressed memory to win |

## 1. Fidelity: does the exact value survive?

| architecture | exact | confidently wrong | result |
|---|---|---|---|
| citation + value in context | 284/284 | 0 | no loss |
| plain context (control) | 284/284 | 0 | no loss |
| citation alone, dereferenced | 99.2% end to end | 0 | no loss after the last-mile fix; 84.4% before |
| dense retrieval, top-5 | 4/142 | up to 138 | fails with confident answers, a median 252 m away |
| BM25 lexical, top-5 | 16/16 | 0 | equal to addressing, with no addressing |
| summarised memory, tight budget | 1/72 | most of the rest | fails |

The first two rows tie. Our own re-score found that the citation arm shows a
rounded value. So that arm and plain context measure the same skill: copying a
number that is already in the context window. A content address adds nothing that we
could measure in that arm. If your answer needs one value and the value fits in
context, plain context is as good as emem and costs less.

The dereference row tests what emem claims. It reached 99.2% only after four
fixes. The benchmark found it at 84.4% first, and the fixes came from that.

## 2. Safety: what happens when memory fails

This result is about the models, not the architectures. When retrieval failed:

| model | behaviour | count |
|---|---|---|
| model A | abstained | 74/96 |
| model B | gave a confident wrong number | 93/96 |

The wrong numbers were real measurements from neighbouring cells. They were
plausible, correctly formatted and wrong. An error like that passes a sanity
check and can flip a threshold decision. When the model had the exact bytes,
there were no confident value errors in 280 arm-model observations.

## 3. Cost: two of four axes go against us

| axis | individual tokens | bundled | context | lowest cost |
|---|---|---|---|---|
| citation size | 84 chars / 51 LLM tokens | 38 chars / 23 LLM tokens, any N | grows with the values | bundle |
| context, N facts | 51·N LLM tokens | 23 LLM tokens, flat | about 5.4·N LLM tokens | bundle |
| round trips | N | 1 (up to N=256) | 1 | bundle and context |
| wall clock | 69 to 1,255 ms | 20 to 54 ms, flat | about 0.9 s in total | bundle |

Individual tokens cost more than pasting the numbers, at every N.

This page once said a token was 104 characters and cost 5.8 times more. Both
numbers were wrong, and the correction goes against us. The grammar cannot make
104 characters: `emem:fact:` (10) + cell64 (19 to 23) + `:` (1) + cid (52) is
86 at most. The real cost is higher than we said.

The measurement on 11 August 2026 used 131 scalar facts at 12 places in 57
bands:

| measure | token | signed value it replaces | ratio |
|---|---|---|---|
| characters | 83 to 85 (mean 84.1) | 10.9 (mean) | 7.7x |
| LLM tokens (cl100k_base) | 50.6 | 5.4 | 9.5x |

LLM tokens are the correct unit, because they are what a context window
bills. A base32 cid breaks into many BPE pieces. So addressing costs more than
plain values unless you bundle.

A bundle reverses this. It is the clearest result in the study. At every N up
to 256, a bundle is 38 characters and one round trip. N=256 individual tokens
are 26,624 characters and 256 round trips. The breakevens are:

| a bundle beats | from | numbers |
|---|---|---|
| individual tokens | N=1 | 23 LLM tokens against 51 |
| plain values | N=5 | 23 LLM tokens against 5.4·N |

At N=1 a bundle costs about 4.3 times the bare number. For that cost you get a
citation that resolves. So bundle a set of facts, but think before you bundle
one fact.

Lower cost did not give better accuracy. At N=256 plain context beat the bundle
4/6 to 2/6. Every architecture failed at N=128 and above. Less context did not
make the model better at using it.

Do not use addressed memory for a query that needs a fast answer. Do not use
individual tokens for many facts.

The case for a content address is precision at scale. A signed value is 18.3
characters on average. A six-decimal display is 8.0. So full precision costs
2.3 times the context, and 256 cells of it do not fit. To hold a region in a
context window, you must drop the precision that made each fact worth signing.
A token keeps the precision at a fixed size. That is the full case, and it is
narrow.

## 4. Retrieval: the embeddings failed, not retrieval

This section once said that dense retrieval found the cell 0 to 16.7% of the
time, and called that a retrieval failure. That was wrong. A lexical baseline
on the same corpus shows it.

| retriever | hit@5 | exact answers |
|---|---|---|
| dense (bge-small) | 0% to 16.7% | 2/12 at best |
| BM25 lexical | 100% | 16/16 |

The corpus, questions and models were the same. Only the retriever changed.
BM25 equals the accuracy of the addressed arm with no minting, no round trips
and no protocol.

The cause is clear. A coordinate is a rare literal string. So all chunks are
almost the same in embedding space but very different in token overlap. Cosine
similarity cannot use that property. BM25 uses it directly.

So the claim is narrow. Dense embedding similarity fails on homogeneous numeric
corpora. Lexical retrieval on the same corpus does not fail. If a lexical index
works on your corpus, you do not need emem for accuracy.

A content address still gives two things that BM25 does not:

- a citation that a third party can verify offline;
- a referent that stays valid when someone rewrites or deletes the corpus.

Both are real. Both are different claims from "retrieval fails".

We do not know how BM25 fails, because it did not fail on this corpus. We do
know how dense retrieval fails: the median drift is 252 metres, and 50% of the
answers match no cell.

## 5. A finding that is true without emem

When a summariser compresses the context, agreement between two models does not
show that they are correct.
The summariser keeps the range endpoints and drops the single values. Both
readers answer with an endpoint. They agree with each other, and both are
wrong.

| measure | value |
|---|---|
| agreement | 27.8% |
| accuracy | 1.4% |
| Fisher p | 0.035 |
| control arm | never inverts |

This is a claim about model consensus as a quality signal. It is true whether
or not you use emem. If your system asks two models and trusts a match, that
check fails where agents share a compacted context.

## 5b. What verification cannot promise

This section was wrong twice in two days, in opposite directions. We show the
sequence because it explains the final rule.

1. We shipped a four-ULP tolerance for reductions.
2. A co-author said that a verifier that accepts "close enough" is not a
   verifier. We withdrew the tolerance. We wrote here that `sum` reproduces
   exactly. Both of us had measured only at 5 parents.
3. The same co-author then measured at scale. The result went against their own
   argument:

| parents | ULP gap | verified under strict equality |
|---|---|---|
| 5 | 0 | yes |
| 16 | 1 | no |
| 32 | 2 | no |
| 64 | 2 | no |
| 128 | 0 | yes |

Two results follow:

- The gaps are one or two representable steps. That is about 1e-16 relative.
  The tightest threshold of any decision in this study is 1e-6.
- The failure does not grow with N. Accumulation error cancels as often as it
  adds. So under strict equality, a caller cannot predict if a sum will verify.
  That looks like a guarantee but behaves like chance, which is worse than a
  stated bound.

So the tolerance is back, with a limit, and the response always shows it:

| field | what it tells you |
|---|---|
| `rule` | which comparison ran |
| `ulp_tolerance` | the bound |
| `ulp_gap` | the measured gap, on success and on failure |

A caller who needs bit-identity can require gap 0, because the gap is visible.
"Equal within 4 ULP, measured gap 2" is a verifier. "Equal" that means "close"
is not.

The code, not this page, enforces the line between what the responder signed and
what it did not sign:

- A leaf fact's `value_verbatim` is the signed preimage. Every digit counts,
  even where only three are meaningful to science. No tolerance applies.
- A reduction is a different object. Nobody signed the sum, and nobody set a
  summation order for a caller to match. Bit-equality there depends on
  summation order, not on the signature.
- `delta`, classification and two-parent reductions accumulate nothing. They
  stay exact.

The limit that stays: you can verify `mean` and `sum` to a stated bound, not to
the byte. Only operations with no accumulation verify exactly.

## 6. What we have not measured

| not tested | why it matters |
|---|---|
| mem0, Zep, Letta, LangMem, any vector DB | These are the real peers. We designed an arm for mem0 and did not run it. |
| ~~lexical retrieval~~ | Tested, and it beat us: BM25 16/16. We have not tested hybrid or geo-aware retrieval. |
| how BM25 fails | It did not fail here, so we do not know its errors. We know the errors of dense retrieval: median 252 m. |
| frontier API models | The study used two open 7 to 12B models on one host. Readers will assume otherwise unless told. |
| conversational memory benchmarks | LOCOMO and LongMemEval test a different referent (authored text spans). |
| independent replication | Nobody outside this collaboration has run any of it. |
| independent compaction | Both readers saw the same summary. So their errors correlate. That is not convergence. |

The most useful thing you can do with this page is to prove it wrong. The arm
we stand behind runs on your models
([`examples/benchmark-arm/`](../examples/benchmark-arm/)). The raw data
carries signatures, and you can replay it. The same directory holds the scorer
that disagreed with its own authors. During this study, people found six scoring bugs in two
separately written instruments. A seventh bug is more likely than not.

## Scorecard

| claim | verdict |
|---|---|
| A citation survives compaction where a paraphrase does not | Supported. This is the core claim. |
| Addressed memory beats plain context when the value fits | Refuted by our own re-score. |
| Addressed memory beats dense retrieval on these corpora | Supported. This is a claim about embeddings. |
| Addressed memory beats lexical retrieval on these corpora | Refuted. BM25 scores 16/16. |
| Model agreement is evidence of correctness | Refuted, p = 0.035. |
| Addressed memory is O(1) | Only when bundled. See section 3. |
| Individual tokens save context | Refuted, by more than we once said. A token is 84 chars / 51 LLM tokens. The value it replaces is 10.9 chars / 5.4 LLM tokens on average. So N tokens cost 7.7x the characters and 9.5x the LLM tokens of the plain numbers, and fill the context sooner. |
| A bundle saves context | Supported. 38 chars and one round trip at every N up to 256, against 26,624 chars and 256 round trips. |
| emem outperforms peer memory products | Not tested. There is no evidence either way. |
