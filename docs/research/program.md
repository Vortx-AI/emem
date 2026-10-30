# The emem research program: algorithms for agents, settled by experiment

*Whitepaper, 29 October 2026. Companion paper: [Finding sudden canopy loss from Sentinel-2 cell series](canopy-loss-2026-10.md).*

**From 30 October the program asks one question:** can an AI agent discover that the method it trusted yesterday is
wrong for this place today, choose a better method using independent evidence, and let another agent verify the
entire decision without trusting the first agent? The design is in
[Method selection](method-selection.md); this paper is the discipline that question is answered under.

## Abstract

emem publishes a registry of algorithms that compose signed Earth-observation facts into answers. Until now the
registry said how each recipe computes and never how well: none of its entries carries a measured accuracy. This
paper sets out how emem now develops and judges algorithms. Every new algorithm is a piece of research held in emem
memory: a plan signed before any data is read, inputs that are signed emem tokens, a reference that is independent of
the method, a deciding score fixed in advance, a signed result, and a proposal whose evidence table every later
experiment updates. The readers are agents, not people, so every part is machine-readable and citeable by path,
content id or token, and an agent can decide whether to trust a method from its record alone. The same discipline
covers the raw-data work underneath the algorithms (sub-pixel measurement, cloud and haze correction) and the
forecasts the research desks publish. The first programme, sudden canopy loss, ran eleven experiments on 57 forests
in one day and shows both halves of the discipline. A rule that won its first experiment failed to replicate, and the
score fixed in advance rewarded a detector that flagged most of the stable forest. Under a better score registered in
advance, two rules then beat the registry's canopy rule on four disjoint sets of forests, until a fifth, smaller
experiment where they tied; by meta-analysis across all five they are better by about 0.17 MCC, with limits stated:
they often say nothing at a single forest, and find nine in ten large clearings but one in five small ones. A
memory-driven detector that fuses radar and optical readings over each cell's own five-year past won the one
experiment it entered. The record also shows the program's own rules being corrected as the evidence came in.

## 1. Why the registry needs evidence

An agent choosing an algorithm from a registry is trusting three claims: that the recipe computes what it says, that
its inputs exist, and that its output is right often enough to act on. emem already makes the first claim checkable,
because each recipe is content-addressed and its inputs are signed facts. The other two were unexamined. An audit of
the algorithm registry (`crates/emem-core/data/algorithms-v0.json`) on 29 October 2026 found:

| finding | entries |
|---|---|
| carry any measured accuracy, skill or validation | none |
| state the formula as prose only, with no machine-evaluable expression | 95 |
| do not run on this deployment: read an encoder or band it has retired (Clay, Prithvi, Galileo, Tessera, DMSP nightlights), name a route it no longer serves (the learned JEPA-v2 head, the three-encoder consensus), or read a band no materializer serves | 44 |
| cite their own shaping as "editorial" or heuristic | 14 |

The drought index entry is a sharp example: it needs thirty years of `weather.precipitation_mm`, and that band holds
the current hour only, so the recipe cannot run here at all. A registry that cannot say how good its entries are
leaves an agent to trust names. The research program exists to replace that trust with records.

## 2. The unit of research is an experiment held in emem memory

Each experiment is a small set of signed notes and one signed derivation, written by the methods-lab research desk
under its own key (`mujzmxes`), in this order:

1. **The plan.** `research/experiments/<id>/plan.md`, written before any data is read. (The code that runs the experiments is in the emem-world
   repository, private until peer review; every note and record it writes is public.) It states the question, the
   reference, the labelling rule, every method under test with the blake3 of its own source, and the deciding score
   with its rule (a paired bootstrap of 2,000 resamples of cells, fixed seed). The transparency log orders the plan
   before the result, so a plan cannot be edited to fit what was found.
2. **The inputs.** Every series and raster the experiment reads is a signed emem response, cited by token: per-cell
   Sentinel-2 series from `field_series`, and the reference raster from `band_raster` (`forest_raster@1`).
3. **The reference.** Independent of every method. In the first programme the methods read Sentinel-2 and the
   reference is Hansen Global Forest Change, read from Landsat.
4. **The result.** `research/experiments/<id>/result.md`, citing the plan by path and content id, with every
   method's score, the interval on its difference from the registry baseline, the per-site breakdown and the
   verdict the pre-registered rule gives.
5. **The record.** An `experiment@1` derivation whose inputs are the experiment's tokens and whose value holds the
   scores, the intervals, the method hashes and the plan's content id, so an agent can read the outcome without
   parsing prose.
6. **The proposal.** A method that wins on the pre-registered rule is written up at `research/algorithms/<key>.md`
   as a registry-shaped entry citing the experiment. Every later experiment that scores the method appends a row to
   its evidence table and recomputes its status.

A proposal moves through these states, and only the operator moves it into the registry:

| status | meaning (rule v2, from 29 October) |
|---|---|
| proposed, awaiting replication | won a pre-registered experiment, with fewer than three disjoint sets of forests behind it |
| replicated | over three or more disjoint sets of forests, the random-effects pooled MCC difference from the registry rule has its 95% lower bound above zero, and no experiment found it significantly worse |
| replicated, forest-dependent | replicated, with I² of 75% or more: right for some kinds of forest, wrong for others |
| not shown better | three or more sets of forests, and the pooled lower bound is not above zero or one experiment found it significantly worse |
| registered | an operator added it to the registry, citing its record |

Each proposal note prints the earlier, strict rule's verdict beside it (replicated only while every later experiment
won; mixed; not replicated), so a reader can see where the two disagree.

Every status so far is **exploratory**. Rule v2 was adopted after the experiments it judges had been seen, and most
of its evidence was weighed with intervals that resampled cells (rule 12). A status becomes **confirmatory** only
from an experiment whose methods, selection policy and score were frozen before it was registered, scored on a set
of places and a period that no earlier experiment, search or rule change has touched.

## 3. What an agent reads, and how it decides

The entry point is one signed note per task, the catalogue: for canopy loss,
`research/algorithms/index.md` under the lab's key. It lists every proposal with its status, its latest score and
interval against the baseline, the experiments it won and lost, the notes that qualify its status, and the measured
accuracy of the registry's own rules for the same task, with the same content in a JSON block. The lab rewrites it
after every experiment, so it is the current state, and every number in it resolves to a signed record.

Every step above is addressed for a machine. An agent that meets a method name can:

- resolve the proposal note and read its status and evidence table;
- resolve the `experiment@1` token and read the scores, the intervals and the method's source hash from the signed
  value;
- fetch the plan and check that its content id matches the one the result cites, and that the method's source hash
  matches the code it would run;
- resolve each input token and rerun the experiment, because every input is a signed response and every method is
  pinned by its source.

A rule of thumb for an agent: use a proposed method only when its status is replicated and the lower end of its
interval against the baseline is above zero, and prefer the registry entry otherwise. Then read the notes under its
evidence table: they say what the status does not, such as how the method does site by site. The record says which
case it is in.

## 4. Research tracks

### 4.1 Detection algorithms

The first programme asks which rule best finds forest that lost canopy in 2024 or 2025, from Sentinel-2 series
alone. It compares the registry's `deforestation_alert_ndvi_drop@1` and `burn_severity_from_dnbr@1`, the forest
desk's own rules, rules the lab proposes, a search over a family of rules scored on held-out sites, and every
detector with and without a cloud screen. The full record is in the [companion paper](canopy-loss-2026-10.md).

### 4.2 Raw-data enhancement

Most of what a detection rule can do is decided before the rule runs, by how the signal was read. The program treats
the reading itself as research.

**Sub-pixel measurement.** A `field_series` cell carries the 10th, 50th and 90th percentiles of its pixels on each
pass, not only the median. Reading the distribution through those three points gives the share of the cell's pixels
above a line, so a change a tenth of a cell wide shows before it moves the median. The coast desk uses this to place
a shoreline from 10 m pixels: at Marina Beach in Chennai it reads the shore advancing 4.8 m a year, with a 95%
interval of 1.8 to 8.6 m (Sen's rank-based interval on the Theil-Sen slope), over 52 clear passes since November
2023. The canopy programme tested the same reading as a detector, the share of a cell's pixels that stay green, and
the darkest tenth of a cell's pixels as another. Neither beat the registry rule at scale or on the seven forests of
the MCC replication; after the cloud screen, both beat it on the ten of the third replication, where their best
sites were managed forests with partial harvests. Their record is mixed. Over all three MCC experiments the quiet
rules found 91% of cells where half the forest was cleared and 20% where a twentieth was, so the cell-size experiment
reads the same detectors at 200 m, scored on the same pixels.

**Cloud and haze correction.** Sen2Cor's scene classification misses thin cloud and haze, which pull NDVI and NBR
down and almost never up. The program tests corrections in time rather than trusting the mask: a temporal screen
that drops a pass reading more than 0.1 below both its neighbours (Chen et al. 2004), and, for condition, each cell's
median over 20 days instead of its newest pass. The second came from evidence: one hazy October pass over Rondonia
read 94% of the forest browner than usual. Whether the screen helps each detector was registered in two canopy
experiments and scored on the same cells with and without it. Across the three MCC experiments it helped a detector
that compares single passes or reads percentiles in 7 of 12 tests and never hurt one, and it never helped the rule
that waits for a drop to hold, for which a low pass is evidence. Snow is the next correction: at a boreal site every
rule flagged much of the stable forest, and the screen keeps a low pass whose neighbours are low too.

**Novel readings.** The program keeps a place for logic that is not a threshold on one index: comparing a cell with
the same weeks a year earlier so that a dry season is not mistaken for loss, reading the darkest tenth of a cell
instead of its median, and the seasonal phase shift the outlooks use (section 4.3).

### 4.3 Forecasts that state their skill

The research desks forecast each measured series two and four weeks ahead with three methods: the season plus
today's anomaly fading at the rate the series has shown; the season slid in time to fit this year; and persistence.
Every method is hindcast over the series' own past using only data before each origin, and the brief states the
method's error against persistence and against the season alone. Each outlook is signed (`outlook@1`), kept in a
ledger and checked against the pass that lands on its date (`outlook_check@1`). The integrity desk's lag report
counts how many series carry an outlook that beats both free forecasts; on 29 October that was none of six, which is
a finding about slow vegetation with two earlier years of history, not a failure to report.

### 4.4 Drivers, and disagreement as a lead

The `rain_series` route gives every desk the rain behind and ahead of a place, ranked against the same weeks of the
last 30 years (a nonparametric standardized precipitation index over 90 days). A condition brief now says when rain
and vegetation disagree: at Maradi in Niger the last 90 days of ERA5 rain are the driest of those weeks since 1996
while 44% of the land is greener than usual. One of the two is wrong there, and the record says so rather than
choosing. Disagreements are listed by the integrity desk as leads for the next experiment.

### 4.5 Memory-driven detection: one timeline per place, every sensor as evidence

The rules in sections 4.1 and 4.2 are simple upgrades of one idea: compare one sensor's reading with a fixed
threshold. They know nothing about the place they are reading, and each sensor is read alone. emem already holds the
two things that idea lacks: the signed history of every place it has read, and machinery for a timeline that changes
its mind (`supersedes` and `disagrees_with` edges, contradictions, consolidation). The memory-driven methods use both.

**Memory.** A place's normal is its own past, not a global threshold. For each sensor, the method fits the cell's
observations from before the window with an annual harmonic and a robust spread, so a dry season the cell always has
is remembered as normal and an equally large drop out of season is not.

**One timeline, every sensor.** Each observation, from any sensor, is evidence about the same question: is this
place still what it was? An optical pass adds the log likelihood ratio of "disturbed" against "as remembered" for its
reading, and a radar pass does the same for backscatter, in date order on one timeline. A pass lost to cloud is
replaced by a radar pass; a hazy pass is one capped piece of evidence, not a verdict.

**Temporal bifurcation.** When evidence of change starts to build, the place's timeline forks into "still as
remembered" and "disturbed since t0". Later observations move weight between the branches until one is confirmed (at
odds of 39 to 1) or the fork collapses. This is Wald's sequential test run as Page's CUSUM, the form the published
optical-radar fusion detectors take (Reiche et al. 2015, 2018). In emem terms, a fork is a provisional fact, its
confirmation supersedes it, and its collapse is recorded rather than erased, so a reader can see when a change was
first suspected and how it was settled.

**Version 1** (`lab:memory_optical@1`, `lab:memory_radar@1`, `lab:memory_fusion@1`) remembers one index per sensor
from the year before the window and confirms a fork at fixed odds of 39 to 1.

**Version 2** is the same idea with the mathematics it needs:

- *Memory across bands and years.* For each cell and sensor group (Sentinel-2 NDVI, NBR and NDMI; Sentinel-1 VH),
  each band's normal is a least-squares annual harmonic fit over five years (2019 to 2023; two harmonics when the
  memory spans two years), with a robust scale (1.4826 times the median absolute residual about the fit; the
  pinned source calls it a MAD, but it is not centred on the residuals' median as the conventional MAD is, and
  field_series' served `memory` block uses the conventional one). The bands' joint noise
  is a rank correlation, 2 sin(πρ/6) of Spearman's ρ, shrunk 20% toward the identity so that a few dozen passes
  give a usable matrix R.
- *A matched filter.* Canopy loss lowers the bands together, in the ratio s = (0.15, 0.20, 0.15) for NDVI, NBR and
  NDMI, and lowers VH. In the cell's own units, a pass's standardised drop z is projected on the whitened direction:
  y = sᵀR⁻¹z / √(sᵀR⁻¹s), which is N(0, 1) under the memory. Correlated bands then count as the evidence they are,
  not three times over, and s fixes only the direction of a loss, not its size.
- *Losses of any size.* Three CUSUM charts run in parallel for shifts of μ = 1.5, 3 and 6 standard deviations
  along that direction, each pass adding μy − μ²/2, capped at ±3 so no pass decides alone.
- *A false-alarm rate each cell states in advance.* The cell's p-value compares its largest CUSUM value over the
  window with 99 replays of the window's own pass schedule built from its memory. The cell is flagged when p ≤ 0.05
  and dated when its strongest fork opened. A variant applies Benjamini-Hochberg across a forest's cells at q = 0.10.
  Another weights each cell by its neighbours' evidence (Genovese, Roeder and Wasserman 2006), because clearings
  spread.
- *Memory as an emem fact.* Each forest's per-cell memory (seasonal values, scales, correlations) is minted as a
  signed `cell_memory@1` derivation whose parents are the series it was learned from. It is the problem-specific
  counterpart of a learned embedding such as AlphaEarth or GeoTessera: every coordinate has a meaning and a source,
  and any agent can recompute it.

**Version 3: the forest calibrates the null.** A cell's memory cannot see a departure every cell shares: a drought
year browns a whole forest a little, and each cell, judged against its own past, reads that as loss. On real forests
version 2 flagged 9.2% of stable cells against a stated 5%. Version 3 (`lab:memory_mv_fusion_emp@3`) scores each cell
continuously against its own null replays, then decides across the forest under an empirical null (Efron 2004):
most cells are unchanged, so together they say what "no change" looks like this year. The null's location and scale
are read from the lower tail of the cells' scores, which loss never reaches, so up to three quarters of the cells may
have changed; the scale is never below 1, so the forest cannot make a cell more sensitive than its own null. On a
synthetic forest of 60 stable and 10 cleared cells:

| | version 2 false alarms | version 3 false alarms | clearings found by each |
|---|---|---|---|
| no shared departure | 4 of 60 | 3 of 60 (its null is the identity) | 10 of 10 |
| a drought year, every band 1.25 noise sd lower | 60 of 60 | 5 of 60 | 10 of 10 |

This is memory and relations together: the cell's own past says what is normal for it, and its forest says what is
normal this year. Version 3 is registered in the two-reference experiment, against version 2.

**Calibration had to be earned.** On synthetic stable cells, three versions of the null failed before one held, and
each failure is a general lesson for memory-driven detectors:

| null | stable cells flagged | why it failed |
|---|---|---|
| residuals resampled in blocks of three passes | 30% with year-to-year offsets | a drier or wetter year shifts every band for months; short blocks do not carry it, and a CUSUM adds it up |
| whole remembered years replayed | 31% with no offsets at all | in-sample residuals lack the fit's own error, which the window sees as a steady offset (the estimated-parameter problem; Jones, Champ and Rigdon 2001) |
| leave-one-year-out residuals replayed | 19% | five remembered years cannot reach the tail of a new year's offset |
| leave-one-year-out within-year noise plus a new year's offset from its predictive distribution | 2 to 5% across four noise conditions | holds |

The predictive draw is normal with the remembered years' offset covariance, scaled by (n − 1)/χ²ₙ₋₁ so that it
carries the uncertainty of estimating that covariance from n years.

**First result.** All seven memory methods were registered in one experiment on ten tropical forests, of which six
could be read. The fused version-2 detector was the only method to beat the registry rule (MCC 0.366 against 0.225;
+0.029 to +0.257) and the best of every method by point score. The comparisons that separate what each upgrade adds
all point the right way (version 2 over version 1, radar over optical alone, memory over the fixed seasonal rule)
and none is decisive at six forests. False-discovery control across a forest's cells was too strict. And the stated
5% false-alarm rate did not hold on real forests: 9.2% for the fused detector, 13.3% for optical alone. Real forests
change in ways five remembered years do not hold, and some cells Hansen calls stable did lose canopy; until a second
reference separates the two, the stated rate is a floor. The next experiment needs more forests read, which needs the
server to finish heavy reads (gap ledger).

**How high can accuracy go?** The score is bounded by the reference. On 4,391 tropical forest cells, Hansen's loss
year and JRC TMF's deforestation year agree on which cells lost forest at an MCC of only 0.39 to 0.48 (2021 to 2023;
paper, section 6.5), partly because they define loss differently. The best detectors score about that against
Hansen. So the next large gain is not a cleverer detector scored against the same references: it is a better
reference, a probability sample of cells checked on very-high-resolution imagery (Olofsson et al. 2014), held in emem
memory like everything else. Claims of near-perfect accuracy will be stated against such a reference, or not at all.

**Next, in order.** Each step is a registered experiment, not a release:

1. Write each alert into emem memory as a signed fact whose lineage carries its history. Running for the forest desk
   since 29 October: the desk reads the methods lab's signed catalogue, runs the method its evidence supports for the
   place's kind of watch, and writes each new or changed alert as a `canopy_alert@1` fact whose parents are the series
   it was read from and the cell's previous alert, with the method's source hash, status and pooled evidence in its
   value, and a per-place index note. Research and operations meet in memory: the lab's records choose the desk's
   method, and the desk's alerts are evidence the next experiment can read. Forks that are open but not yet confirmed
   are the next to be written.
2. Let the memory learn: update a cell's normal from observations its timeline has confirmed as undisturbed, so slow
   change is remembered and a confirmed change resets the normal.
3. Let neighbours speak: a fork confirmed next door raises the prior of a fork here, because clearings spread.
4. Add sensors as emem serves them: Landsat or HLS for memory before 2017 and a third optical reading, and per-pixel
   stacks from `band_cube` instead of cell medians (the cell-size experiment says finer reading finds more).

## 5. Rules of evidence

These are the rules the program holds itself to, each learned or confirmed on the first day it ran:

1. **Register before reading.** The plan, the methods' source hashes and the deciding score are signed before data
   is read.
2. **A win counts only on sites the rule has not seen.** A rule that wins one experiment is proposed and waits. On 29
   October the lab's seasonal rule won on seven forests (F1 0.651 against 0.568, interval +0.045 to +0.120) and lost
   on seven new ones (0.166 against 0.361); its proposal now reads not replicated.
3. **A search is scored out of sample.** When the lab searches a family of rules, each site is scored by the variant
   chosen on the other sites. The first search scored 0.59 out of sample and was not shown better than the baseline;
   its picks changed from site to site, which is what fitting looks like.
4. **Choose a score that cannot be won by flagging everything.** F1 ignores the stable cells a method correctly left
   alone. Where loss was common it let the registry rule win while it flagged 74% of stable forest. Recomputed from
   the signed records, by the Matthews correlation the same rule ranked last in two of the three experiments. That
   recomputation was not pre-registered, so it is not a win for anything; it is why every experiment since registers
   MCC.
5. **Publish what went against you.** Failed replications, searches that overfit and metrics that misled are in the
   record, in the same format and at the same prominence as wins.
6. **A rerun is not a replication.** An experiment rerun on sites already scored reads the same data. Each experiment
   runs once, and a win counts as a replication only on sites no earlier experiment scored.
7. **Report per site as well as pooled.** A pooled score can be won by the false alarms a method does not raise. On 29
   October both replicated canopy rules beat the baseline at only about half the sites, and both would have lost, by
   the score they won on, on one earlier site set. Analyses like these are post hoc and decide nothing, and they are
   written on the proposal beside its status.
8. **A search's score is the search's.** When a search wins out of sample, its pick is fixed as a method of its own
   and tested on new sites, because the pick was chosen on every site the search saw. On 29 October the fixed pick
   won its own test on ten new forests, while that run's search did not win out of sample: the procedure was unstable
   and the rule it had picked held.
9. **A failure under the deciding score stands.** Losses before a method's first win are excused only when another
   score judged them. The rule was first written to excuse every earlier loss, which was meant for the F1 experiments,
   and it was tightened when three sub-pixel and screened rules won after losing under MCC. Changes to the status rule
   are made in the open, apply to every record, and so far have only demoted.
10. **A status rule must not punish running more experiments.** The rule in force asks every later experiment to
   win, so a method that is better on average will in time tie in a small experiment and read not replicated; with
   enough experiments, every good method does. The record keeps that rule's verdicts as registered and reports a
   random-effects meta-analysis across disjoint site sets beside them (DerSimonian and Laird 1986). On 29 October the
   meta-analysis became the status rule (rule v2, signed at `research/rules/status-v2.md`), with the strict rule's
   verdicts kept beside it.
11. **The operator registers.** The lab proposes; the registry changes only when its operator adds an entry and cites
   the record.
12. **The forest is the unit of evidence.** A clearing is one event seen in many cells, so an interval that resamples
   cells counts it many times. On 29 October one clearing held 56 of an experiment's 59 loss cells, and the best
   detector's lead ran +0.93 to +1.08 with cells resampled and -0.03 to +1.16 with forests resampled. Every result
   reports both; a plan that registers forests as the unit decides by them; and forests are chosen where loss was
   under way before the scored years, since a forest with no loss adds stable cells and no evidence.
13. **Three kinds of evidence, and only one decides.** Observations are signed facts an upstream produced. Hypotheses
   are anything a method produced, however carefully, such as a desk's signed `canopy_alert@1`. Independent
   references are products and adjudications no candidate method made. Selection and promotion read only the third:
   a signed hypothesis is auditable, not true, and agreement among descendants of one prediction is not evidence
   about the ground. The lab reads its truth only through a list of independent products, and a test fails if a
   fleet derivation is ever on it. A reference that shares an input with a candidate (Hansen and version 4 both use
   Landsat) is used with the dependence stated.
14. **A decision may use only what existed before it.** A retrospective score, where a detector reads passes after
   the change and a cloud screen reads both neighbours, is not the score of an alert issued on the day. Alerting
   claims are measured by as-of replay: the detector re-run on series truncated at each decision date, reporting
   the first alert, the date it held, every reversal, and the retrospective score beside it. Every result carries
   this table for the leading detectors and the registry rule, and its signed record the figures.

## 6. Scaling the program

Each new programme needs one thing the first had: a reference that is independent of the methods and already
reachable as signed emem data. Several are:

| task | independent reference in emem |
|---|---|
| canopy loss and clearing | Hansen GFC loss year and JRC TMF deforestation year (to 2023), both as rasters (`band_raster`); RADD alerts registered but unreadable (Requester-Pays) |
| fire | MODIS burned area (MCD64A1) through the field burn-scar route; OPERA DIST-ALERT registered but needs an Earthdata token |
| surface water and flood extent | JRC Global Surface Water occurrence and seasonality |
| forecasts | the later signed observation itself, checked by the outlook ledger |

A programme grows by adding sites (every new site set is a replication of the last), by adding methods (each named
by its source) and by moving the deciding score when the record shows it misleads, always in a new plan rather than
an edit. The integrity desk's lag report lists unanswered places, failing desks, outlooks without skill and
disagreements between sources; that list is the program's agenda.

## 7. What is not shown yet

- Five canopy rules are replicated under rule v2 (the paper, section 5.18): the first memory detector
  (`lab:memory_optical@1`, +0.21 MCC, forest-dependent), the searched seasonal rule (+0.15), the seasonal NBR rule
  and the desk's rule (both forest-dependent) and the screened registry rule (+0.04). The later memory detectors pool
  higher, +0.17 to +0.34, but their sets of forests disagree too much (I² 96 to 99%) for them to be called better, and
  most of their evidence was weighed with intervals that resampled cells (rule 12). The Landsat version
  (`lab:memory_hls_emp@4`) won its first experiment with forests resampled and awaits replication. The lab's planner
  schedules that replication itself, on the Landsat design and on forests that had loss before the window.
- At 400 m they find 20% of cells with 5 to 10% cleared. Whether a 200 m reading closes that gap is the cell-size
  experiment's question.
- Clearings that cover under 5% of a 400 m cell are left out of scoring.
- The reference is annual and Landsat-based; a loss Sentinel-2 sees in January can belong to the previous year in
  it, which the labels handle by leaving those cells out.
- The rules under test were written by people or searched over settings people chose. The lab does not yet propose
  rule families of its own.
- Forecasts beat persistence only on some fast seasonal series (a haor draining, a reservoir filling); for slow
  vegetation with two earlier years they do not, and the briefs say so.

## References

- Olofsson, P. et al. 2014. Good practices for estimating area and assessing accuracy of land change. Remote
  Sensing of Environment 148:42-57.
- Page, E. S. 1954. Continuous inspection schemes. Biometrika 41:100-115.
- Reiche, J. et al. 2015. A Bayesian approach to combine Landsat and ALOS PALSAR time series for near real-time
  deforestation detection. Remote Sensing 7:4973-4996.
- Reiche, J. et al. 2018. Improving near-real time deforestation monitoring in tropical dry forests by combining
  dense Sentinel-1 time series with Landsat and ALOS-2 PALSAR-2. Remote Sensing of Environment 204:147-161.
- Wald, A. 1945. Sequential tests of statistical hypotheses. Annals of Mathematical Statistics 16:117-186.
- Chen, J. et al. 2004. A simple method for reconstructing a high-quality NDVI time-series data set based on the
  Savitzky-Golay filter. Remote Sensing of Environment 91:332-344.
- Benjamini, Y. & Hochberg, Y. 1995. Controlling the false discovery rate. Journal of the Royal Statistical
  Society B 57:289-300.
- Genovese, C. R., Roeder, K. & Wasserman, L. 2006. False discovery control with p-value weighting. Biometrika
  93:509-524.
- Jones, L. A., Champ, C. W. & Rigdon, S. E. 2001. The performance of exponentially weighted moving average charts
  with estimated parameters. Technometrics 43:156-167.
- DerSimonian, R. & Laird, N. 1986. Meta-analysis in clinical trials. Controlled Clinical Trials 7:177-188.
- Efron, B. 2004. Large-scale simultaneous hypothesis testing: the choice of a null hypothesis. Journal of the
  American Statistical Association 99:96-104.
- Farahmand, A. & AghaKouchak, A. 2015. A generalized framework for deriving nonparametric standardized drought
  indicators. Advances in Water Resources 76:140-145.
- Hansen, M. C. et al. 2013. High-resolution global maps of 21st-century forest cover change. Science 342:850-853.
- Key, C. H. & Benson, N. C. 2006. Landscape Assessment: sampling and analysis methods. USDA Forest Service
  RMRS-GTR-164-CD.
- Matthews, B. W. 1975. Comparison of the predicted and observed secondary structure of T4 phage lysozyme.
  Biochimica et Biophysica Acta 405:442-451.
- McKee, T. B., Doesken, N. J. & Kleist, J. 1993. The relationship of drought frequency and duration to time scales.
  8th Conference on Applied Climatology, AMS, 179-184.
- Sen, P. K. 1968. Estimates of the regression coefficient based on Kendall's tau. Journal of the American
  Statistical Association 63:1379-1389.
