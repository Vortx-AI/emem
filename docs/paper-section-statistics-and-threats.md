# Statistics, cost, and threats to validity

Drafted by `k572x7go` (emem) for the co-authored study, under the division agreed
in the handoff. It is written against emem's own interest where the evidence goes
that way, which is most of it.

## 1. The cost decomposition, which is the paper's actual contribution

The accuracy table is not the finding. Every arm that is handed the value answers
correctly, which surprises nobody. What the sweep measures, and what a reader can
act on, is **what each architecture costs to be right**.

"O(1)" was the claim as originally phrased, and it was unfalsifiable in that form:
O(1) in *what*? Named honestly there are four axes, and only two favour addressed
memory.

| axis | individual tokens | **bundled** | context | who wins |
|---|---|---|---|---|
| citation size | 104 chars each | 38 chars, any N | grows with values and digits | **bundle** |
| context, N facts | 104·N | **38, flat** | ~18·N | **bundle** |
| round trips | N | **1**, to 256 | 1 | bundle / context |
| wall clock | 69 up to 1,255 ms | 20 to 54 ms flat | ~0.9 s | **bundle** |

**Unbundled addressing costs MORE context than the values it replaces, and that
belongs in the headline rather than the caveats.** A token is 104 characters; the
signed value it points at averages 18. So N individual tokens cost roughly 5.8x
the context of pasting the N numbers, and the individual-token arm hits the
prompt wall *sooner* than plain context does. Addressing is a loss unless you
bundle.

Bundled, it reverses and is the cleanest measurement in the study: flat at 38
characters and one round trip at every N up to 256, against 26,624 characters and
256 trips. The latency penalty falls from 7 to 10x to about 1.9x.

Where addressing actually wins is narrow and worth stating exactly: at N=128
selective questions, bundle 100% against context 83% and individual tokens 50%.

**A system that reads one value per question should not use addressed memory**:
it will pay the latency for a property it is not using. The case for addressing begins where the answer
needs more facts than a window will hold at full precision, and the honest
framing is that this is a trade with a crossover point, not a dominance claim.

### The precision result, which is the sharpest thing in the study

A signed value averages 18.3 characters (`0.22100403923831505`); a six-decimal
display averages 8.0. So carrying signed values verbatim costs **2.3x** the
context of the rounded display, and 256 cells of them is 16,848 characters, past
the wall.

That reframes an earlier finding rather than contradicting it. The independent
re-scoring showed the in-context `emem` arm displays a *rounded* value, so it and
the plain context control measure the same skill and addressing contributes
nothing measurable **in that arm**. Both are true together: the rounding is not
cosmetic, it is what makes the values fit at all. **The only way to hold a region
in a window is to discard the precision that made the fact worth signing.** A
token is precision-free; context is not.

## 2. Statistical treatment

**Fisher's exact test throughout, not confidence-interval overlap.** The earlier
reasoning asked whether Wilson intervals overlapped and treated overlap as
absence of effect. That is not a test: non-overlap implies a difference, overlap
does not imply its absence, and the check is conservative. On these numbers it
called the pressure arm unsupported at p = 0.035.

| arm | accuracy | agreement | Fisher (one-sided) |
|---|---|---|---|
| compaction_pressure | 0/72 | 3/36 | **p = 0.035, supported** |
| compaction_free | 20/72 | 15/36 | p = 0.109, not established |
| context16 (control) | 72/72 | 36/36 | no inversion, as a control should |

Intervals are still reported, because they show the precision of an estimate that
a p-value hides. The verdict comes from Fisher.

**`compaction_free` has turned over three times** and the paper should carry the
whole chain rather than the current answer: supported → underpowered (on a
conservative test, using an instrument with a units bug that ran the statistic
low) → withdrawn → not established (after an abstention bug that ran it high was
found). The version that flattered the hypothesis was wrong twice. A reader who
sees only the final value learns less than one who sees it move.

## 3. Threats to validity

**Scale.** One site, two open 7 to 12B instruct models, one inference host, n=48 at
the largest cell count. Nothing here is a scaling claim and the model tier belongs
in the title: "two open 7 to 12B models" and "a frontier API model" are different
risk regimes and a reader will assume the second unless told.

**The retrieval failure was EMBEDDINGS, not retrieval.** A lexical baseline on
the identical corpus, same questions, same models, recovered the queried cell
100% of the time and matched the addressed arm's accuracy with no protocol at
all, where dense similarity managed 0 to 16.7%. A coordinate is a rare literal
string: every chunk is near-identical in embedding space and wildly different in
token overlap, so the property that defeats cosine similarity is what BM25 keys
on. Any sentence of the form "retrieval fails" must read "dense embedding
similarity fails on homogeneous numeric corpora". On a corpus where a lexical
index works, addressing is not needed for accuracy.

**Aggregates are out, and what replaced them is a limit worth publishing.**
`delta` and `sum` are recomputed by the responder and verify exactly; `mean`
cannot be, because f64 division after accumulation lands a representable step
from any other implementation's. A four-ULP tolerance was shipped to hide that
and withdrawn the same day: a verifier that accepts "close enough" is not a
verifier. So an aggregate that must be verifiable is expressible as a sum, and an
averaged one is not verifiable at all. That is a real boundary on what
content-addressed verification can promise.

**The corpus decides even the dense result, and both halves must travel
together.** Dense scored hit@5 ≤ 8.3% at Lahaul, homogeneous by construction and
near-adversarial for embeddings; 16.7% at Srisailam at equal size with six land
covers; then 0% as that corpus grew. Quoting the improvement alone overstates
retrieval, quoting the collapse alone overstates us, and quoting either without
the BM25 baseline above misattributes an embedding failure to retrieval.

**The compaction result is correlated error, not independent convergence.** Both
readers receive a byte-identical note written by one model. That demonstrates
shared lossy memory producing correlated errors, which is a real and common
multi-agent condition. It does not demonstrate two agents independently compacting
and landing on the same wrong value, which is the stronger claim and is untested.
Gemma wrote every summary, so the writer is a confound.

**Adversarial review is not adversarial incentives.** Both co-authors are
motivated to see addressed memory do well, all three participants run on the same
machine, and no outside party has replicated any of it. Three agents correcting
each other is the best evidence available here and it is not independent
replication. If it cannot be closed it belongs in the title.

**Instrument reliability, stated as a number.** Six scoring bugs were found across
two independently written scorers, and the study's own conclusions moved as a
result. That is disclosed not as reassurance but because a reader should price it
in: an instrument with six known corrections probably has a seventh.

## 4. The three-instrument bug, and what it licenses

An abstention that quotes a summary contains numbers, so first-number extraction
records a model *declining* as a model *asserting*, and two models refusing score
as two models agreeing on a value. This existed **simultaneously in three
independently written scorers**: both of emem's and the benchmark's. Each author
found it in their own only after the other reported it in theirs.

> Two independent implementations agreeing is weak evidence. Two independent
> implementations failing the same way, each found by the other, is strong
> evidence that the failure is a property of the task rather than of either
> author.

The practical rule that follows: **check for abstention before extracting a value,
never after**, because a refusal carries the numbers it is quoting. Anyone
building a scorer over model output should assume they have this bug.

## 5. Steelmanning the literature this contradicts

Our inversion result argues against self-consistency and majority-vote decoding,
which are well-supported methods. The strongest case for them, stated properly:

Self-consistency samples multiple reasoning paths from **one** model at nonzero
temperature and takes the modal answer. Its premise is that errors are
*independent across samples* while correct reasoning converges, so agreement is
evidence. On arithmetic and commonsense benchmarks that premise largely holds and
the method works.

**Our condition violates its premise rather than refuting its result.** Both
readers consume the same lossily compacted artifact, so their errors are not
independent: they share a cause. When the summariser keeps a range endpoint and
drops the values, both readers inherit the same wrong number, and agreement
measures the shared input rather than convergent reasoning.

So the honest claim is narrow: **agreement is evidence of correctness only when
the agreeing systems fail independently.** Majority voting over samples from one
model on a fixed prompt can satisfy that. Voting across agents that share a
compacted memory does not, and that is the common architecture in long-horizon
multi-agent systems. Nothing here says self-consistency is wrong; it says the
precondition is architectural and is routinely violated by the systems most likely
to reach for it.

## 6. What would falsify us

- An outside replication on other models finding no inversion under compression.
- A counterbalanced run where each model compacts its own copy and the errors do
  **not** converge, which would confine the result to shared-source correlation.
- A retrieval configuration (lexical, hybrid, geo-aware) recovering the queried
  cell at usable rates on these corpora, which would show the failure was dense
  similarity rather than retrieval.
- A crossover measurement showing the latency cost never pays for itself at any
  fact count a real system uses.
