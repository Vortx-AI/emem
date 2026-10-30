# Method selection: when the method an agent trusted is wrong for this place

*Design, 30 October 2026. Not yet pre-registered: the first experiment below is signed only when its plan is final.
Companion to [the research program](program.md).*

## 1. The question

From 30 October the research program asks one question, and every experiment is a step towards answering it:

> Can an AI agent discover that the method it trusted yesterday is wrong for this place today, choose a better method
> using independent evidence, and let another agent verify the entire decision without trusting the first agent?

The question has three parts, and each needs something the program did not have.

- **Discover.** An agent needs a record of where each method is competent, kept per place and per kind of place, and
  updated as independent evidence arrives. A single pooled score cannot say that a method is wrong *here*.
- **Choose.** The choice may read only evidence that no candidate method produced, and only evidence that existed
  before the decision. It may abstain.
- **Verify.** The choice is a signed decision record that names its evidence, its rule and its output by content id,
  so that a second agent with its own key can resolve every input, re-run the rule and the chosen method from their
  pinned sources, and sign that it agrees or disputes.

The first two parts are statistics. The third is what emem's signed memory is for: none of the components is new
(harmonic regression, change points, CUSUM, empirical nulls, meta-analysis), but a decision that a stranger can
re-derive from signed bytes is.

## 2. What the record already shows

The canopy-loss experiments (companion paper, sections 5.13 to 5.16) already contain the problem this question names.

- `lab:seasonal_loss_nbr@1` beat the registry rule in four experiments in a row on 2024-2025 forests, and scored MCC
  0.014 against 0.058 on ten forests for 2022-2023. An agent that trusted it from the first four would have been
  wrong there.
- Most methods' results disagree across sets of forests far more than chance allows (I² 78 to 99%). The first memory
  detector is replicated but forest-dependent: right for some kinds of forest, not for others.
- One experiment's best score was one clearing. Pooled scores hide where a method fails.

So the program should learn where each method is competent, not only which method has the highest average.

## 3. Three kinds of evidence (rule of evidence 13)

1. **Observations**: signed facts a registered upstream produced (a Sentinel-2 series, a Hansen raster).
2. **Hypotheses**: anything a method produced, however carefully: a desk's `canopy_alert@1`, an `experiment@1`
   record's predictions, an inference. Signing a hypothesis makes it auditable, not true.
3. **Independent references**: products and adjudications that no candidate method produced, such as Hansen Global
   Forest Change, JRC TMF, and later a probability sample interpreted by people.

Selection and promotion read only the third kind. A hypothesis is never a reference, even when another agent
agreed with it, because agreement among descendants of one prediction is not evidence about the ground. A
reference that shares an input with a candidate (Hansen is made from Landsat; version 4 reads Landsat) is used, but
the dependence is stated in the plan. The lab enforces the first part in code: the bands it reads as truth are on a
list of independent products, and a test fails if that list ever holds a band the fleet derives.

## 4. A decision may use only what existed before it

The experiments so far are retrospective. Some detectors confirm a loss with later passes, the temporal cloud
screen reads both neighbouring passes, and a memory detector dates a change from the whole window. Those are valid
for a retrospective map, but a retrospective score is not the score of an alert issued on the day.

- **As-of replay.** A detector is re-run on each cell's series truncated at a sequence of decision dates, so every
  output uses only the passes that existed then. Reported per cell: the first date it raised an alert, the date the
  alert held for good, every reversal, and the reference's eventual label; beside them, the retrospective score on
  the same cells, so the gap is measured rather than assumed.
- **Selection as of a date.** The competence evidence a choice reads must be dated before the period the choice
  acts on, and come from places or periods the choice is not scored on.

## 5. The decision record

A choice is signed as one record (`decision@1`) with these parts, each named by content id:

| part | what it holds |
|---|---|
| question | place, period, and the decision date |
| candidates | each method's key and source hash |
| evidence | each record the choice read, with its kind (observation, hypothesis, reference) and date |
| rule | the selection rule's source hash and its settings |
| competence | each candidate's estimate for this place, with its interval and what it was shrunk towards |
| choice | the method chosen, or an abstention and why |
| output | the chosen method's signed result for the place |

A verifier is an agent with a different key. It resolves every token, checks every signature and every evidence
kind, re-runs the rule on the cited evidence and the chosen method on the cited inputs from their pinned sources, and
signs a `verification@1` record: agree, or dispute with the first part that differs. A dispute is published beside
the decision, not hidden. The experiment reports how many decisions verified, and how long and how many bytes a
verification took.

## 6. The first experiment, as registered

**A first design was dropped before anything was signed.** It would have used the ten forests of the two-reference
experiment, scored for 2022 and 2023, as the earlier period. A dry run showed why that was too thin. Those forests
held 42 loss cells, 26 of them in one forest. At a forest with one loss cell, the champion was flagged wrong and
replaced on the strength of that single cell, because resampling cells cannot show how little one hit or miss says.
Two changes followed: a forest is local evidence only with five or more loss cells, and the experiment moved to new
forests with real loss in the earlier period.

**Forests.** The nine unused forests of the lab's pool with ten or more 400 m cells of Hansen loss in 2021 to 2023,
read before either period: Sao Felix do Xingu, Boca do Acre, Sud-Ubangi, Sagaing, Zambezia, Sarawak, Attapeu,
Masoala and Cujubim. They span humid rainforest, monsoon forest and dry miombo, so competence that differs by kind
of forest has room to show.

**Three experiments, run by the lab desk in order.**

1. `canopy-loss-selection-forests-2022-2023`: the earlier period, on the two-optical-sensor design, with Landsat.
   These forests are new to the lab, so this period is also a set of forests in every method's evidence.
2. `canopy-loss-method-selection-2022-2025`: runs once (1) is scored. Its plan prints every forest's decision, and
   each decision is signed as `decision@1` before the later period is read.
3. `canopy-loss-selection-forests-2024-2025`: the later period, held until (2)'s decisions are signed. It covers
   the same forests, so it enters no method's status evidence.

Then (2) runs again and scores its decisions on (3).

**Candidates.** Versions 1 to 4 of the memory detectors, the two seasonal rules, the forest desk's rule and the
registry rule. Each is named by the blake3 of its source.

**Policies**, frozen in the plan:

- P0, global: the candidate with the best mean earlier-period MCC over the forests with five or more loss cells.
- P1, local: each forest's MCC shrunk towards that mean by empirical Bayes.
- P3, abstaining: P1, but only where the choice's shrunk lower bound beats the registry rule.

The plan also says when the champion is flagged as wrong at a forest.

**Scores on the later period**, with forests as the resampled unit:

- P1 against P0;
- P3's coverage, and its MCC beside P0's on the same forests;
- how often the "champion wrong here" flag held.

**Limits stated in advance.** Hansen is the reference in both periods, and it is a Landsat product. Nine forests is
few. The scoring is retrospective inside each period; as-of replay comes with it as a second measurement.

## 7. What it needs that the lab cannot provide alone

- **An independently designed reference.** A probability sample of stable forest, small clearings, plantations and
  cloud-heavy places, interpreted by people on very-high-resolution imagery (Olofsson et al. 2014). Who interprets,
  and on what imagery, is the operator's decision.
- **Competitors.** GLAD-S2, RADD and DIST-ALERT where they overlap. DIST-ALERT needs an Earthdata login, which is the
  operator's to provide.

## 8. Steps

1. Rule 13 in the program and the lab's reference guard (done with this note).
2. As-of replay, run first on the cached forests (done post hoc on the two-optical-sensor experiment; companion
   paper, section 5.17). As an experiment kind of its own, next.
3. `verification@1` and a verifier desk (done: the integrity desk re-derives each new experiment from its signed
   records, with every input's receipt and canonical bytes checked offline, and signs the outcome under its own
   key). `decision@1`, next.
4. The experiment is registered (section 6) and runs from the lab desk on its own; its decisions are
   signed before the later period is read.
