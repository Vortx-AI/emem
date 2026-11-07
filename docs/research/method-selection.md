# Method selection: when the method an agent trusted is wrong for this place

*Design, 30 October 2026. On 7 November 2026 the lab has registered and scored two experiments under this
design (sections 9 and 10). Companion to [the research program](program.md).*

## 1. The question

From 30 October the research program asks one question, and every experiment is a step towards answering it:

> Can an AI agent discover that the method it trusted yesterday is wrong for this place today, choose a better method
> using independent evidence, and let another agent verify the entire decision without trusting the first agent?

The question has three parts, and each needs something the program did not have.

- **Discover.** An agent needs a record of where each method is competent, kept per place and per kind of place, and
  updated as independent evidence arrives. A single pooled score cannot say that a method is wrong *here*.
- **Choose.** The choice may read only evidence that no candidate method produced, and only evidence that existed
  before the decision. It may abstain.
- **Verify.** The choice is a signed decision record. It names its evidence, its rule and its output by content id.
  A second agent with its own key can then resolve every input. It re-runs the rule and the chosen method from their
  pinned sources, and signs that it agrees or disputes.

The first two parts are statistics. The third is what emem's signed memory is for. None of the components is new
(harmonic regression, change points, CUSUM, empirical nulls, meta-analysis). A decision that a stranger can
re-derive from signed bytes is new.

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
3. **Independent references**: products and adjudications that no candidate method produced. Examples are Hansen
   Global Forest Change, JRC TMF, and later a probability sample that people interpret.

Selection and promotion read only the third kind. A hypothesis is never a reference, even when another agent
agreed with it. Agreement among descendants of one prediction is not evidence about the ground. A reference can
share an input with a candidate: Hansen derives from Landsat, and version 4 reads Landsat. The lab uses such a
reference, but the plan states the dependence. The lab enforces the first part in code. The bands it reads as truth
are on a list of independent products. A test fails if that list ever holds a band the fleet derives.

## 4. A decision may use only what existed before it

The experiments so far are retrospective. Some detectors confirm a loss with later passes. The temporal cloud screen
reads both neighbouring passes. A memory detector dates a change from the whole window. Those are valid for a
retrospective map, but a retrospective score is not the score of an alert issued on the day.

- **As-of replay.** A detector is re-run on each cell's series truncated at a sequence of decision dates, so every
  output uses only the passes that existed then. Each cell reports the first date it raised an alert and the date
  the alert held for good. It also reports every reversal and the reference's eventual label. Beside them is the
  retrospective score on the same cells, so the report measures the gap and does not assume it.
- **Selection as of a date.** The competence evidence that a choice reads must have a date before the period the
  choice acts on. It must also come from places or periods that the lab does not score the choice on.

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

A verifier is an agent with a different key. It resolves every token, and checks every signature and every evidence
kind. It re-runs the rule on the cited evidence, and the chosen method on the cited inputs from their pinned sources.
Then it signs a `verification@1` record: agree, or dispute with the first part that differs. The verifier publishes a
dispute beside the decision and does not hide it. The experiment reports how many decisions verified, and how
long and how many bytes a verification took.

## 6. The first experiment, as registered

**A first design was dropped before anything was signed.** It would have used the ten forests of the two-reference
experiment, scored for 2022 and 2023, as the earlier period. A dry run showed why that was too thin. Those forests
held 42 loss cells, 26 of them in one forest. At a forest with one loss cell, the plan flagged the champion as wrong
and replaced it on the strength of that single cell. A resample of cells cannot show how little one cell says. Two
changes followed. A forest is local evidence only with five or more loss cells. The experiment moved to new forests
with real loss in the earlier period.

**Forests.** The lab's pool held nine unused forests with ten or more 400 m cells of Hansen loss in 2021 to 2023. The
lab read this count before either period. The nine are Sao Felix do Xingu, Boca do Acre, Sud-Ubangi, Sagaing,
Zambezia, Sarawak, Attapeu, Masoala and Cujubim. (This Sao Felix do Xingu box lies about 70 km west of the box an
earlier experiment read under the same name; they share no cell.) The Koppen class at each box centre gives the
climate. Five are monsoon rainforest (Am) and two are rainforest (Af). One is savanna and dry forest (Aw), and one is
dry-winter subtropical (Cw).

**Why a second earlier experiment.** The paper's section 5.18 found, post hoc, that competence follows the climate.
In seasonally dry forest the registry rule keeps its score, and the desk's rule is worse than it. In evergreen forest
the memory detectors win by a wide margin. That suggests a policy an agent can apply at a forest it has never scored:
choose by the record of forests of the same climate. The table in section 5.18 cannot be that record. The lab scored
most of its forests for 2024 and 2025, the years on which this experiment scores its decisions. A choice may read
only evidence dated before the period it acts on (rule 14). Without that rule, a shared year-level cause, such as a
drought year, could reach the choice through its peers. So the context comes from a pool of forests of the same
climates, scored for 2022 and 2023. The nine forests alone could not give this context: the Aw and Cw forests have no
other forest of their class.

**Four experiments, run by the lab desk in order.** Once one is done, the next due starts within the hour.

1. `canopy-loss-selection-forests-2022-2023`: the earlier period, on the two-optical-sensor design, with Landsat.
   These forests are new to the lab, so this period is also a set of forests in every method's evidence.
2. `canopy-loss-context-forests-2022-2023`: the context pool, on the same design for the same years. It holds ten
   forests that the lab scored for 2024 and 2025 before. They are in the classes of the selection forests, except Am.
   Am has five selection forests of its own. The pool looks for forests with eight or more loss cells in 2021 to
   2023. A class with at least three such forests gives up to five, the largest loss first. Aw gives Chiquitania,
   Bokeo, Lobaye, Prey Lang and Sinop. Af gives Jambi, Ketapang, Madre de Dios, Ituri and Caqueta. Cw had two, so the
   pool leaves it out. The Cw forest, Sagaing, gets the global choice. These forests are already in other experiments'
   evidence, so the pool enters no method's status evidence.
3. `canopy-loss-method-selection-2022-2025`: runs once (1) and (2) are scored. Its plan prints every forest's
   decision. It also prints the climate class of every forest it read, and the fact each class comes from. The lab
   signs each decision as `decision@1` before it reads the later period.
4. `canopy-loss-selection-forests-2024-2025`: the later period, held until (3)'s decisions are signed. It covers
   the same forests, so it enters no method's status evidence.

Then (3) runs again and scores its decisions on (4).

**Candidates.** Versions 1 to 4 of the memory detectors, the two seasonal rules, the forest desk's rule and the
registry rule. Each is named by the blake3 of its source.

**Policies**, frozen in the plan. Every policy reads the same earlier record, (1) and (2) together, so the
difference between P2 and P0 is the context and nothing else.

- P0, global: the candidate with the best mean MCC over the forests of the earlier record with five or more loss
  cells.
- P1, local: each forest's MCC shrunk towards that mean by empirical Bayes.
- P2, context: the candidate with the best mean MCC over the other forests of the earlier record in the same
  climate class. The class is the first two letters of the Koppen class. P2 counts only forests with five or more
  loss cells. With fewer than three such forests, it takes P0's choice. It never reads the forest's own record.
- P3, abstaining: P1, but only where the choice's shrunk lower bound beats the registry rule.

The plan also says when the champion is flagged as wrong at a forest.

**Scores on the later period**, with forests as the resampled unit:

- P1 against P0, and P2 against P0. These are two registered comparisons, and the lab adjusts neither for the other.
  The report gives the number of forests where P2 chose other than P0.
- P3's coverage, and its MCC beside P0's on the same forests;
- how often the "champion wrong here" flag held.

**Limits stated in advance.** Hansen is the reference in both periods, and it is a Landsat product. Nine forests is
few, and five of them share one class, so P2 can differ from P0 at only a few forests. The Koppen class is read at
one point, the box centre, from a 1 km map. Climate is confounded with continent. The scoring is retrospective inside
each period; as-of replay comes with it as a second measurement.

**What the verifier checks.** The integrity desk re-derives both earlier experiments from their signed records. It
reads every Koppen fact from its canonical bytes. The hash must be the content id, the band must be `koppen`, and the
value must be the class the plan prints. Then the desk re-runs every policy, and signs agreement or the first
difference. A dry run used the two-optical-sensor forests, split into a stand-in earlier period and a stand-in pool.
It found that the selection plan did not print the parameters block its verifier needs. The plan prints it now.

## 7. What it needs that the lab cannot provide alone

- **An independently designed reference.** A probability sample of stable forest, small clearings, plantations and
  cloud-heavy places, interpreted by people on very-high-resolution imagery (Olofsson et al. 2014). The protocol is
  in [An independent reference](reference-sample.md); who interprets, and on what imagery, is the operator's
  decision.
- **Competitors.** GLAD-S2, RADD and DIST-ALERT where they overlap. DIST-ALERT needs an Earthdata login, which is the
  operator's to provide.

## 8. Steps

1. Rule 13 in the program and the lab's reference guard (done with this note).
2. As-of replay. Done: first post hoc on the two-optical-sensor experiment (companion paper, section 5.16). It is now
   in every result, for the leading detectors and the registry rule.
3. `verification@1` and a verifier desk. Done: the integrity desk re-derives each new experiment from its signed
   records. It checks every input's receipt and canonical bytes offline, and signs the outcome under its own key.
   From the second selection on, the lab signs `decision@1` for every forest, with each policy's choice and the evidence
   it read. The first selection signed none (companion paper, section 5.19).
4. The lab registered the experiment (section 6), and the lab desk runs it without help. The lab signs its
   decisions before it reads the later period.
5. The context policy (P2), from a pool scored for the years before the later period (section 6).

## 9. What the first experiment found

The picture shows both selections side by side: which method each policy chose per forest, and how each policy
scored. It reads the lab's two signed result notes. We drew it on 7 November 2026.

<p align="center"><img src="../media/stills/method-selection.webp" width="420" alt="Two method selections, from two signed result notes of key mujzmxes. Selection 1, 9 forests: the champion memory_hls_emp@4 at every forest; the local policy chose another method at 3 forests; pooled MCC for 2024 and 2025 is 0.145 global, 0.656 local and 0.145 context; local minus global lies between -0.052 and +0.750; no decision@1 was signed. Selection 2, 10 forests: the champion memory_hls_emp@5; the local policy chose another method at 7, two of them version 4, which scored the same; pooled MCC 0.314 global, 0.261 local and 0.308 context; local minus global -0.162 to +0.041, context minus global -0.035 to +0.028; 10 decision@1 facts signed. Both note signatures are valid when checked offline."></p>

Results are in the companion paper, section 5.19. In short:

- **A first, narrow yes.** The local policy chose, from each forest's own earlier record, a different method at three
  forests. At one of them, Sao Felix do Xingu, the global champion went on to fail. It scored MCC 0.02, against 0.41
  for the local choice. Pooled over the later period, the local policy scored 0.656 against 0.145. The interval with
  forests resampled crosses zero, so it does not decide: one forest carries the difference.
- **Why the champion failed there.** Its empirical null is fitted to the forest's own cells and assumes most are
  unchanged. Where most of the box was cleared, the null absorbed the clearing. The record of where a method is
  competent has to include the conditions under which its calibration holds, not only its scores.
- **The context policy added nothing** in this experiment: it chose as the global policy at every forest.
- **Verification works end to end.** Another agent re-derived every input, score and decision from signed records,
  and its first disagreement was its own bug, which it found.

What the next selection changes:

1. Forests are screened on the years the labels score, so that every forest gives local evidence.
2. A calibrated detector falls back to its theoretical null when the fitted null departs far from it. This is a new
   version with its own source hash, registered before it is scored.
3. Each decision is signed as `decision@1`; a selection short of one holds its later period (now enforced).

**Registered 5 November, before any of its forests was read.** The second selection runs as three experiments in
order: `canopy-loss-selection2-forests-2022-2023`, `canopy-loss-method-selection2-2022-2025` and
`canopy-loss-selection2-forests-2024-2025`. The lab ranked 45 forests by the number of 400 m cells lost in 2022 and
2023. They were 31 unused forests of the lab's pool and 14 placed on known clearing frontiers. The top ten have
59 to 16 loss cells each. They are Bafwasende, Djoum, Loreto, Trairao, Placas, Puerto Inca, Laguna del Tigre,
Cartagena del Chaira, Jacareacanga and Apiacas. The earlier record adds the two experiments already scored for 2022
and 2023. Version 5, `lab:memory_hls_emp@5`, is a candidate. It gives version 4's scores, with one change. Sometimes
more than three quarters of a forest's cells read as changed against their own nulls. There it uses the theoretical
null. The lab checked this rule post hoc over its 38 forest-periods with Landsat. It changes a decision only at Sao
Felix in 2024-2025. These forests test it.

## 10. What the second experiment found

Results are in the companion paper, section 5.20. In short:

- **No yes against Hansen.** With local evidence at all ten forests, the local policy scored 0.261 against 0.314 for the
  global champion, and the context policy 0.308. Neither interval with forests resampled excludes zero (-0.162 to +0.041
  and -0.035 to +0.028), so neither decides. The first experiment's gain, carried by one forest, did not repeat.
- **The registered independent test is still to come.** CL1 is a panel of model readers over these forests. For this
  same selection, it tests the local policy against the global one. The emem responder agent drew CL1 on 6 November 2026.
  Nobody has scored it yet.
- **The flag.** The champion was flagged wrong at three forests, and at all three it scored below its own mean. That
  count decides nothing, and without the unflagged forests' rate it is not evidence that the flag works.
- **Version 5 was not tested.** It scored exactly as version 4 in both periods, so its rule never changed a decision.
- **Verification** re-derived the decisions and both forest experiments in full, with one trust gap on the climate
  class P2 chooses by. The selection's own result, the policy scores and intervals, was re-derived too, after this
  section was first written, and reproduces; so does the first selection's.

What the next selection changes:

1. Candidates that score identically on the earlier record count as one, so a choice is a real change of method.
2. The verifier re-derives the selection's result too, not only its decisions and inputs (done since, on 6 November).
3. The flag is reported beside the unflagged forests' rate, so it can be judged.
4. The lab scores it against CL1's kind of reference and also against Hansen.
