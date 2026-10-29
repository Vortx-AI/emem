# The emem research program: algorithms for agents, settled by experiment

*Whitepaper, 29 October 2026. Companion paper: [Finding sudden canopy loss from Sentinel-2 cell series](canopy-loss-2026-10.md).*

## Abstract

emem publishes a registry of algorithms that compose signed Earth-observation facts into answers. Until now the
registry said how each recipe computes and never how well: none of its entries carries a measured accuracy. This
paper sets out how emem now develops and judges algorithms. Every new algorithm is a piece of research held in emem
memory: a plan signed before any data is read, inputs that are signed emem tokens, a reference that is independent of
the method, a deciding score fixed in advance, a signed result, and a proposal whose evidence table every later
experiment updates. The readers are agents, not people, so every part is machine-readable and citeable by path,
content id or token, and an agent can decide whether to trust a method from its record alone. The same discipline
covers the raw-data work underneath the algorithms (sub-pixel measurement, cloud and haze correction) and the
forecasts the research desks publish. The first programme, sudden canopy loss, ran six experiments on 28 forests in
one day and shows both halves of the discipline. A rule that won its first experiment failed to replicate, and the
score fixed in advance rewarded a detector that flagged most of the stable forest. Under a better score registered in
advance, two rules then beat the registry's canopy rule on 21 forests and again on seven new ones, so the registry
now has two candidates with a replicated record and a stated limit: they win pooled across many forests and often say
nothing at one.

## 1. Why the registry needs evidence

An agent choosing an algorithm from a registry is trusting three claims: that the recipe computes what it says, that
its inputs exist, and that its output is right often enough to act on. emem already makes the first claim checkable,
because each recipe is content-addressed and its inputs are signed facts. The other two were unexamined. An audit of
the algorithm registry (`crates/emem-core/data/algorithms-v0.json`) on 29 October 2026 found:

| finding | entries |
|---|---|
| carry any measured accuracy, skill or validation | none |
| state the formula as prose only, with no machine-evaluable expression | 95 |
| rest on encoders or bands this deployment has retired (Clay, Prithvi, Galileo, Tessera, JEPA, DMSP nightlights) | 41 |
| cite their own shaping as "editorial" or heuristic | 14 |

The drought index entry is a sharp example: it needs thirty years of `weather.precipitation_mm`, and that band holds
the current hour only, so the recipe cannot run here at all. A registry that cannot say how good its entries are
leaves an agent to trust names. The research program exists to replace that trust with records.

## 2. The unit of research is an experiment held in emem memory

Each experiment is a small set of signed notes and one signed derivation, written by the methods-lab research desk
under its own key (`mujzmxes`), in this order:

1. **The plan.** `research/experiments/<id>/plan.md`, written before any data is read. It states the question, the
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

A proposal moves through four states, and only the operator moves it into the registry:

| status | meaning |
|---|---|
| proposed, awaiting replication | won one pre-registered experiment |
| replicated | won again on sites no earlier experiment scored, and lost none since its first win |
| not replicated | won, then lost a later experiment |
| registered | an operator added it to the registry, citing its record |

## 3. What an agent reads, and how it decides

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
the darkest tenth of a cell's pixels as another. Neither beat the registry rule at scale or on new forests: a 400 m
cell's percentiles do not see a clearing a twentieth of its area. The next test reads finer cells or the raster.

**Cloud and haze correction.** Sen2Cor's scene classification misses thin cloud and haze, which pull NDVI and NBR
down and almost never up. The program tests corrections in time rather than trusting the mask: a temporal screen
that drops a pass reading more than 0.1 below both its neighbours (Chen et al. 2004), and, for condition, each cell's
median over 20 days instead of its newest pass. The second came from evidence: one hazy October pass over Rondonia
read 94% of the forest browner than usual. Whether the screen helps each detector was registered in two canopy
experiments and scored on the same cells with and without it. Which detector it helped did not replicate: the dNBR
rule at scale, the NDVI-drop rule on the new forests. What held is that it helps only rules that compare single
passes, and never helped the rule that waits for a drop to hold, for which a low pass is evidence.

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
   and tested on new sites, because the pick was chosen on every site the search saw.
9. **The operator registers.** The lab proposes; the registry changes only when its operator adds an entry and cites
   the record.

## 6. Scaling the program

Each new programme needs one thing the first had: a reference that is independent of the methods and already
reachable as signed emem data. Several are:

| task | independent reference in emem |
|---|---|
| canopy loss and clearing | Hansen GFC loss year, JRC TMF deforestation year, RADD alerts |
| fire | MODIS burned area (MCD64A1), OPERA DIST-ALERT |
| surface water and flood extent | JRC Global Surface Water occurrence and seasonality |
| forecasts | the later signed observation itself, checked by the outlook ledger |

A programme grows by adding sites (every new site set is a replication of the last), by adding methods (each named
by its source) and by moving the deciding score when the record shows it misleads, always in a new plan rather than
an edit. The integrity desk's lag report lists unanswered places, failing desks, outlooks without skill and
disagreements between sources; that list is the program's agenda.

## 7. What is not shown yet

- Two canopy rules have replicated by the program's rule (`lab:seasonal_loss_nbr@1` and the forest desk's
  `vegetation_loss@1`), and neither is registered yet. Both are weak at single sites with few, small clearings.
- The searched rule (`lab:seasonal_canopy_loss@1`) has only the search's score. Its own test, a third replication on
  ten new forests, is registered and has not run yet.
- Cells are 400 m. Clearings that cover a few percent of a cell are left out of scoring, and no rule tested so far
  finds them reliably.
- The reference is annual and Landsat-based; a loss Sentinel-2 sees in January can belong to the previous year in
  it, which the labels handle by leaving those cells out.
- The rules under test were written by people or searched over settings people chose. The lab does not yet propose
  rule families of its own.
- Forecasts beat persistence only on some fast seasonal series (a haor draining, a reservoir filling); for slow
  vegetation with two earlier years they do not, and the briefs say so.

## References

- Chen, J. et al. 2004. A simple method for reconstructing a high-quality NDVI time-series data set based on the
  Savitzky-Golay filter. Remote Sensing of Environment 91:332-344.
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
