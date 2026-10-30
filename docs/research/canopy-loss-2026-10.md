# Finding sudden canopy loss from Sentinel-2 cell series: pre-registered experiments against emem's algorithm registry

*Research paper, 29 October 2026. emem methods lab (research desk `methods-lab`, key `mujzmxes`). Program and rules
of evidence: [The emem research program](program.md).*

## Abstract

We ask which rule best finds forest that lost tree cover, at cells about 400 m across, from satellite series
(Sentinel-2 first; then Sentinel-1 and Landsat 8 and 9, read into each cell's own multi-year memory), without flagging
forest that did not change. Over thirteen pre-registered experiments registering 89 forests (loss in 2024 and 2025;
in 2022 and 2023 for one) we compare the two canopy rules in emem's algorithm registry, the forest desk's own rules,
rules the lab proposes, a search over a family of rules scored on held-out sites, and each detector with and without a
temporal cloud screen; two of the experiments test the reading itself, at 200 m cells and with snow masked. Every plan was signed
before its data was read, every input is a signed emem token, and every method is scored against Hansen Global
Forest Change, an independent Landsat product.

The first win, the lab's seasonal NDVI rule under F1, did not replicate, and F1 itself misled: it rewarded the
registry's `deforestation_alert_ndvi_drop@1` for flagging up to 74% of stable forest. Under the Matthews correlation
(MCC), registered in advance from the fifth experiment on, two rules beat the registry rule in all four experiments,
on four disjoint sets of 19, 7, 10 and 8 forests: the lab's seasonal NBR rule (MCC 0.381, 0.457, 0.262 and 0.262,
against 0.295, 0.145, 0.098 and 0.013) and the forest desk's held NDVI drop (0.356, 0.284, 0.202 and 0.301). A rule
found by a search and then fixed, `lab:seasonal_canopy_loss@1`, beat it on the 18 forests it had never seen. The
seasonal NBR rule flags 1.2 to 4.6% of stable forest; the registry rule flags 46 to 62%. In managed forests a
registered hypothesis held: the sub-pixel rules after a cloud screen scored best of all (MCC 0.389 for the
10th-percentile rule).

Detectors built on each cell's own memory came next. Each keeps a seasonal normal per band over five years, applies a
matched filter across bands and sensors, and calibrates its alarm on the cell's remembered noise. Under Hansen in
2022 and 2023 the first memory detectors scored MCC 0.25 to 0.30 against 0.06. The version with Landsat 8 and 9 in the
same optical memory scored 0.42 against 0.06 on nine forests that had loss before the window. It beat the registry rule
with forests as the resampled unit (+0.03 to +0.69); against the same detector without Landsat, the interval spans
zero.

The record also shows how such results mislead. One experiment's MCC of 0.93 was a single clearing holding 56 of its
59 loss cells, and intervals that resampled cells made it look certain; resampled by forest, the same lead runs from
-0.03 to +1.16. From then on the forest is the unit of evidence.

The status rule since 29 October is a random-effects meta-analysis over disjoint site sets. Under it, five methods
are replicated:
- the first memory detector, by +0.21 MCC (forest-dependent);
- the searched seasonal rule, by +0.15;
- the seasonal NBR rule and the desk's rule, both forest-dependent;
- the screened registry rule, by +0.04.

The later memory detectors pool higher but disagree too much across forests to be called better. A second reference,
JRC TMF, marked too few of the cells as lost to judge any method.

The record also bounds these rules. Their wins are pooled: site by site, each beats the registry rule at 38% to 80%
of the sites with both loss and stable forest, depending on the rule and the site set. They find 91% of the 400 m cells where half the forest or more was cleared, as the
registry rule does, but only 20% of those where 5 to 10% was. Combining them does not help, because they miss the
same small clearings. The cloud screen helped detectors that compare single passes or read percentiles in 7 of 12
tests and never hurt them, and never helped the rule that waits for a drop to hold. Sub-pixel rules won once and
failed to beat the registry rule twice. Read at 200 m cells instead of 400 m, scored on the same Hansen pixels, every
detector found more of the lost forest with no more false alarms, and the 10th-percentile rule gained with an interval
clear of zero; on forests with small clearings, the seasonal NBR rule went from finding none of the loss to pixel MCC
0.343.

## 1. Question

A canopy-loss alert is useful to an agent only if it finds loss where it happened and stays quiet where it did not.
The registry offers two rules that read the Sentinel-2 signal emem serves:

- `deforestation_alert_ndvi_drop@1`: an alert when NDVI fell more than 0.20 from about 60 days earlier and now reads
  under 0.5 (after Hansen et al. 2013 and the GLAD-S2 alert documentation);
- `burn_severity_from_dnbr@1`: the drop in NBR between two passes, with 0.27 or more as moderate severity or worse
  (Key & Benson 2006).

Neither entry states how well it does. We measure that, and test whether other rules do better.

## 2. Data

**Series.** For each site, a grid of 12 x 12 square cells over a box 4.8 km a side (each cell about 0.16 km²), and
one signed `field_series` response per index (NDVI and NBR) over 1 January 2023 to 28 October 2026, from
Sentinel-2 L2A scenes spread evenly in time. The lab asked for 96 scenes; the server's budget of 12,000 cell-scenes
per request caps 144 cells at 83, and in the two MCC experiments each site kept 63 to 83 clear passes. Each cell's row per pass gives its median and its 10th and 90th
percentiles over pixels the scene classification left clear. Passes `field_series` marks suspect are left out.

**Reference.** Hansen Global Forest Change v1.13 loss year and 2000 tree cover (Landsat, 30 m), read for the same box
through `band_raster` (`forest_raster@1`), and decoded from emem's canonical grid encoding.

**Labels.** A cell is forest when its mean 2000 tree cover is 30% or more; other cells are left out. A forest cell
is LOSS when 5% or more of its pixels lost cover in 2024 or 2025, and STABLE when none did and none lost cover in
2023 (a 2023 clearing first seen in January 2024 is neither). Cells in between are left out and counted per site.

**Scoring window.** A method flags a cell when it sees loss dated 1 January 2024 to 31 March 2026; the reference is
annual, so a loss seen early in 2026 can be 2025's.

## 3. Methods under test

Each method is named in every plan by the blake3 of its own source (`agents/lab.py` in emem-world).

| key | origin | rule |
|---|---|---|
| `registry:deforestation_alert_ndvi_drop@1` | registry | NDVI more than 0.20 below the pass nearest 60 days earlier (40 to 80 days), and under 0.5 now |
| `registry:burn_severity_from_dnbr@1` | registry | NBR 0.27 or more below the previous clear pass |
| `desk:vegetation_loss@1` | desk | NDVI 0.25 below the previous pass from at least 0.4, still 0.125 down two passes later |
| `desk:vegetation_loss_nbr@1` | desk | the same on NBR: 0.27 from at least 0.3 |
| `lab:seasonal_loss_ndvi@1` | lab | the median NDVI of the 45 days from t at least 0.20 below both the median of the 90 days before t and the median of the same 45 days a year earlier, from at least 0.4 |
| `lab:seasonal_loss_nbr@1` | lab | the same on NBR, 0.27 from at least 0.3 |
| `lab:seasonal_loss_either@1` | lab | either seasonal rule |
| `lab:subcell_loss_p10@1` | lab | the seasonal rule on the cell's NDVI 10th percentile, 0.20 from at least 0.3 |
| `lab:green_fraction_loss@1` | lab | sub-pixel: the share of the cell's pixels above NDVI 0.4, read from its percentiles, 0.15 below both references, from at least 0.7 |
| `lab:seasonal_canopy_loss@1` | lab, from the search | the seasonal rule on NBR, 0.20 from at least 0.4, post window 45 days: the search's pick in section 5.6, fixed so that later experiments test this rule and not the search |
| `<detector>+screen` | lab | a detector run after the temporal cloud screen: drop a pass more than 0.1 below both neighbours (Chen et al. 2004) |

The year-ago reference is the point of the seasonal rules: a dry season that browns forest every year browns it a
year earlier too, so it is not mistaken for loss. The sub-pixel rules exist because a clearing over a fifth of a 400 m
cell barely moves the cell's median.

**Search.** The search experiment also tries 32 variants of the seasonal rule (index NDVI or NBR; drop 0.15 to 0.30;
starting level 0.3 or 0.4; post window 30 or 45 days) and scores them by leave-one-site-out cross-validation: each
site is scored by the variant with the best score on the other sites, so every scored cell comes from a site its
rule never saw.

## 4. Protocol

Each experiment's plan was published as a signed note before any data was read, with the question, the reference,
the labels, every method's source hash and the deciding score. A method beats the registry baseline
`deforestation_alert_ndvi_drop@1` when the 95% interval of its score minus the baseline's, from a paired bootstrap of
2,000 resamples of cells with a fixed seed, lies wholly above zero. The first three experiments registered F1 as the
deciding score; the scaled experiment and every one since registered the Matthews correlation (MCC), for the reason in
section 6.1.

A method that wins is proposed and waits. It is **replicated** when a later experiment on sites that no earlier
experiment scored also finds it better, and **not replicated** when any later experiment does not. A rerun on sites
already scored reads the same data, so it is listed but cannot replicate anything. Each experiment runs once.

## 5. Results

### 5.1 First experiment: seven forests (F1 decides)

Rondonia, São Félix do Xingu, Chiquitania (Bolivia), Central Kalimantan, Jasper, Prey Lang, Ituri. 194 loss and 441
stable cells.

| method | F1 | precision | recall | false alarm rate | MCC (not registered) | F1 minus baseline, 95% interval |
|---|---|---|---|---|---|---|
| `lab:seasonal_loss_ndvi@1` | 0.651 | 0.564 | 0.768 | 0.261 | 0.474 | +0.045 to +0.120 |
| `lab:seasonal_loss_either@1` | 0.651 | 0.564 | 0.768 | 0.261 | 0.474 | +0.045 to +0.120 |
| `desk:vegetation_loss_nbr@1` | 0.628 | 0.769 | 0.531 | 0.070 | 0.520 | -0.008 to +0.131 |
| `registry:burn_severity_from_dnbr@1` | 0.612 | 0.575 | 0.655 | 0.213 | 0.427 | -0.010 to +0.099 |
| `desk:vegetation_loss@1` | 0.609 | 0.715 | 0.531 | 0.093 | 0.482 | -0.024 to +0.105 |
| `lab:seasonal_loss_nbr@1` | 0.570 | 0.900 | 0.418 | 0.020 | 0.524 | -0.076 to +0.082 |
| `registry:deforestation_alert_ndvi_drop@1` | 0.568 | 0.409 | 0.928 | 0.590 | 0.338 | baseline |

On the registered rule the lab's seasonal NDVI rule beat the baseline, and it was proposed for the registry.

### 5.2 The search, scored out of sample

On the same seven forests, the variant picked on the other six sites scored F1 0.592 on each held-out site, pooled;
its difference from the baseline had a 95% interval of -0.027 to +0.073, so it was not shown better. The picks
changed from site to site (an NDVI variant for Rondonia and Jasper, an NBR variant with a 0.15 drop for the other
five), which is what fitting to the training sites looks like.

### 5.3 Replication: seven new forests (F1 decides)

Madre de Dios, Sinop, Gran Chaco, Riau, Tanintharyi, Taï forest edge, Western Ghats. 79 loss and 527 stable cells.

| method | F1 | precision | recall | false alarm rate | MCC (not registered) | F1 minus baseline, 95% interval |
|---|---|---|---|---|---|---|
| `registry:deforestation_alert_ndvi_drop@1` | 0.361 | 0.229 | 0.848 | 0.427 | 0.284 | baseline |
| `registry:burn_severity_from_dnbr@1` | 0.206 | 0.148 | 0.342 | 0.296 | 0.034 | -0.219 to -0.087 |
| `lab:seasonal_loss_ndvi@1` | 0.166 | 0.182 | 0.152 | 0.102 | 0.053 | -0.290 to -0.105 |
| `lab:seasonal_loss_either@1` | 0.164 | 0.179 | 0.152 | 0.104 | 0.051 | -0.291 to -0.106 |
| `desk:vegetation_loss@1` | 0.110 | 0.121 | 0.101 | 0.110 | -0.010 | -0.334 to -0.162 |
| `desk:vegetation_loss_nbr@1` | 0.081 | 0.111 | 0.063 | 0.076 | -0.016 | -0.366 to -0.185 |
| `lab:seasonal_loss_nbr@1` | 0.022 | 0.091 | 0.013 | 0.019 | -0.016 | -0.414 to -0.262 |

The first result did not replicate. Both proposals were updated to "not replicated".

### 5.4 Sub-cell rule: seven more forests (F1 decides)

Guaviare, Ucayali, Merauke, Menabe, Bosawas, Bokeo, Acre; Merauke and Menabe could not be scored (the server answered
502 and 400). 213 loss and 216 stable cells.

| method | F1 | precision | recall | false alarm rate | MCC (not registered) | F1 minus baseline, 95% interval |
|---|---|---|---|---|---|---|
| `registry:deforestation_alert_ndvi_drop@1` | 0.619 | 0.511 | 0.784 | 0.741 | 0.051 | baseline |
| `registry:burn_severity_from_dnbr@1` | 0.543 | 0.707 | 0.441 | 0.181 | 0.282 | -0.152 to +0.004 |
| `desk:vegetation_loss@1` | 0.532 | 0.838 | 0.390 | 0.074 | 0.375 | -0.162 to -0.006 |
| `lab:subcell_loss_p10@1` | 0.527 | 0.664 | 0.437 | 0.218 | 0.234 | -0.157 to -0.027 |
| `lab:seasonal_loss_either@1` | 0.492 | 0.617 | 0.408 | 0.250 | 0.169 | -0.193 to -0.063 |
| `lab:seasonal_loss_ndvi@1` | 0.466 | 0.600 | 0.380 | 0.250 | 0.140 | -0.221 to -0.087 |
| `desk:vegetation_loss_nbr@1` | 0.323 | 0.894 | 0.197 | 0.023 | 0.279 | -0.379 to -0.205 |
| `lab:seasonal_loss_nbr@1` | 0.312 | 0.930 | 0.188 | 0.014 | 0.290 | -0.387 to -0.224 |

The sub-cell rule did not beat the baseline on the registered score.

### 5.5 Scaled: all 21 forests, with and without the cloud screen (MCC decides)

All 21 forests of the three earlier experiments, pooled; Merauke and Menabe again could not be scored. 486 loss
and 1,184 stable cells. Registered: MCC decides; every detector is also scored after the temporal cloud screen, and
the screen's effect on each detector is the paired difference on the same cells.

| method | MCC | F1 | precision | recall | false alarm rate | MCC minus baseline, 95% interval |
|---|---|---|---|---|---|---|
| `lab:seasonal_loss_nbr@1` | 0.381 | 0.390 | 0.854 | 0.253 | 0.018 | +0.031 to +0.138 |
| `registry:burn_severity_from_dnbr@1+screen` | 0.365 | 0.472 | 0.677 | 0.362 | 0.071 | +0.011 to +0.126 |
| `desk:vegetation_loss@1` | 0.356 | 0.489 | 0.632 | 0.399 | 0.095 | +0.003 to +0.117 |
| `lab:seasonal_loss_either@1` | 0.327 | 0.520 | 0.528 | 0.512 | 0.188 | -0.015 to +0.080 |
| `desk:vegetation_loss_nbr@1` | 0.325 | 0.421 | 0.664 | 0.309 | 0.064 | -0.033 to +0.088 |
| `lab:seasonal_loss_ndvi@1` | 0.317 | 0.511 | 0.523 | 0.500 | 0.188 | -0.025 to +0.070 |
| `registry:deforestation_alert_ndvi_drop@1+screen` | 0.298 | 0.516 | 0.481 | 0.556 | 0.246 | -0.043 to +0.048 |
| `registry:deforestation_alert_ndvi_drop@1` | 0.295 | 0.539 | 0.395 | 0.848 | 0.533 | baseline |
| `lab:green_fraction_loss@1+screen` | 0.287 | 0.465 | 0.533 | 0.412 | 0.148 | -0.059 to +0.044 |
| `desk:vegetation_loss@1+screen` | 0.282 | 0.395 | 0.609 | 0.292 | 0.077 | -0.072 to +0.045 |
| `registry:burn_severity_from_dnbr@1` | 0.265 | 0.489 | 0.466 | 0.514 | 0.242 | -0.087 to +0.029 |
| `lab:green_fraction_loss@1` | 0.263 | 0.500 | 0.448 | 0.566 | 0.286 | -0.076 to +0.016 |
| `lab:subcell_loss_p10@1+screen` | 0.220 | 0.398 | 0.492 | 0.333 | 0.141 | -0.134 to -0.015 |
| `lab:subcell_loss_p10@1` | 0.178 | 0.420 | 0.415 | 0.424 | 0.245 | -0.172 to -0.058 |

Three rules beat the registry baseline on the registered rule: the lab's seasonal NBR rule, the registry's own dNBR
rule after the cloud screen, and the forest desk's held NDVI drop. The baseline has the highest recall of any rule
(0.848) and flags 53% of stable forest; the seasonal NBR rule flags 1.8%, at the cost of finding a quarter of the
loss. For an agent that must not raise false alarms, that trade is the point.

**The cloud screen.** Its effect on each detector, same cells, MCC:

| detector | unscreened | screened | difference, 95% interval | verdict |
|---|---|---|---|---|
| `registry:burn_severity_from_dnbr@1` | 0.265 | 0.365 | +0.056 to +0.141 | helps |
| `lab:subcell_loss_p10@1` | 0.178 | 0.220 | +0.005 to +0.078 | helps |
| `lab:green_fraction_loss@1` | 0.263 | 0.287 | -0.015 to +0.063 | no clear effect |
| `registry:deforestation_alert_ndvi_drop@1` | 0.295 | 0.298 | -0.043 to +0.048 | no clear effect |
| `desk:vegetation_loss@1` | 0.356 | 0.282 | -0.105 to -0.042 | hurts |

The screen helps the rules that compare single passes, which a lone hazy pass can trip, and hurts the desk rule,
which already demands that a drop hold for two more passes: for it, removing a low pass removes evidence rather than
noise. A correction is not good or bad in general; it is good for some detectors, and the record now says which.

**Sub-pixel readings.** Neither sub-pixel rule beat the baseline at scale. The 10th-percentile rule was the weakest
detector and the share-of-green rule was level with the baseline; per site, both were best where clearings were
small and scattered (Gran Chaco, Tanintharyi, Acre). Reading the distribution changes which losses a rule sees more
than how well it sees them overall.

**The search in this run** selected and scored its variants by F1 rather than the registered MCC: the code that
ran it had not been changed with the plan. Its out-of-sample F1 was 0.432 (95% interval of the difference -0.140 to
-0.072), so it was not shown better on either score. The code now uses the registered score; the next run's search
is decided by MCC. The plan template's search paragraph was not changed with it: the plans of section 5.6 and of the
third replication still say the search picks by F1, while their decision sections say MCC decides and the search
used MCC. That is a deviation from the letter of those plans, stated here; the template names the registered score
for every plan written after them.

**Status after this experiment.** The two rules that won here for the first time are proposed and await
replication; the seasonal NDVI rule and its either variant remain not replicated.

### 5.6 Replication under MCC: seven fresh forests (MCC decides)

Petén (Guatemala), Ketapang (West Kalimantan), Jambi (Sumatra), Tshopo (DR Congo), Niassa (Mozambique), Flin Flon
(Manitoba), Chaco (Argentina): none scored by any earlier experiment. 248 loss and 330 stable cells. Registered: the
same methods, labels and decision as section 5.5, the search now selecting and scoring by MCC.

| method | MCC | F1 | precision | recall | false alarm rate | MCC minus baseline, 95% interval |
|---|---|---|---|---|---|---|
| `lab:seasonal_loss_nbr@1` | 0.457 | 0.509 | 0.956 | 0.347 | 0.012 | +0.231 to +0.391 |
| `desk:vegetation_loss_nbr@1` | 0.337 | 0.422 | 0.833 | 0.282 | 0.042 | +0.100 to +0.279 |
| `desk:vegetation_loss@1` | 0.284 | 0.515 | 0.656 | 0.423 | 0.167 | +0.051 to +0.225 |
| `desk:vegetation_loss@1+screen` | 0.273 | 0.442 | 0.702 | 0.323 | 0.103 | +0.036 to +0.218 |
| `registry:deforestation_alert_ndvi_drop@1+screen` | 0.254 | 0.548 | 0.594 | 0.508 | 0.261 | +0.039 to +0.184 |
| `registry:burn_severity_from_dnbr@1+screen` | 0.217 | 0.387 | 0.660 | 0.274 | 0.106 | -0.019 to +0.163 |
| `registry:burn_severity_from_dnbr@1` | 0.204 | 0.508 | 0.567 | 0.460 | 0.264 | -0.023 to +0.136 |
| `lab:seasonal_loss_either@1` | 0.172 | 0.511 | 0.535 | 0.488 | 0.318 | -0.050 to +0.102 |
| `registry:deforestation_alert_ndvi_drop@1` | 0.145 | 0.577 | 0.484 | 0.714 | 0.573 | baseline |
| `lab:green_fraction_loss@1` | 0.054 | 0.478 | 0.458 | 0.500 | 0.445 | -0.167 to -0.013 |
| `lab:seasonal_loss_ndvi@1` | 0.047 | 0.406 | 0.462 | 0.363 | 0.318 | -0.184 to -0.015 |
| `lab:green_fraction_loss@1+screen` | 0.037 | 0.325 | 0.463 | 0.250 | 0.218 | -0.204 to -0.013 |
| `lab:subcell_loss_p10@1` | -0.092 | 0.345 | 0.370 | 0.323 | 0.412 | -0.332 to -0.137 |
| `lab:subcell_loss_p10@1+screen` | -0.111 | 0.211 | 0.322 | 0.157 | 0.248 | -0.357 to -0.151 |

The seasonal NBR rule beat the baseline again, on forests it had not seen, and by a wider margin, mostly because the
baseline did worse here (0.145 against 0.295 at scale). Its precision was 0.956: of the 90 cells it flagged, 4 were
stable forest. The forest desk's held NDVI drop also beat the baseline for the second time. The
registry's dNBR rule after the screen, which won at scale, did not win here, and its proposal now reads not
replicated. Three rules won here for the first time: the desk's NBR rule, the desk's NDVI rule after the screen, and
the registry's NDVI rule after the screen. The baseline still has the best F1 (0.577), and a lower MCC than every rule
except the sub-pixel ones and the seasonal NDVI rule; section 6.1 is why that is not a contradiction.

Two sites carried one label only: Niassa had no loss cells (44 stable) and Flin Flon no stable ones (56 loss). Their
cells count in the pooled score; per site, MCC is undefined there and is reported as 0.

**The cloud screen**, same cells, MCC:

| detector | unscreened | screened | difference, 95% interval | verdict |
|---|---|---|---|---|
| `registry:deforestation_alert_ndvi_drop@1` | 0.145 | 0.254 | +0.039 to +0.184 | helps |
| `registry:burn_severity_from_dnbr@1` | 0.204 | 0.217 | -0.056 to +0.083 | no clear effect |
| `desk:vegetation_loss@1` | 0.284 | 0.273 | -0.061 to +0.041 | no clear effect |
| `lab:green_fraction_loss@1` | 0.054 | 0.037 | -0.094 to +0.062 | no clear effect |
| `lab:subcell_loss_p10@1` | -0.092 | -0.111 | -0.090 to +0.050 | no clear effect |

At scale the screen helped the dNBR rule and hurt the desk rule; here it helped only the NDVI-drop rule and did
neither to the other two. No detector's response to the screen replicated. What held is the direction: the screen
helps only rules that compare single passes, and never helped the rule that waits for a drop to hold.

**The search**, selecting by MCC on six sites and scoring the seventh, scored **MCC 0.458** out of sample, and its
difference from the baseline had a 95% interval of +0.233 to +0.392. Unlike the first search, it picked the same
variant (NBR, a drop of 0.20 from at least 0.4, over 45 days) for six of the seven held-out sites, and a 0.25 drop
for Petén. That is what a stable choice looks like. The search's score measures the search, not that variant: the
variant was also picked on all seven sites, so its own score there is in-sample (0.518) and decides nothing. It is
fixed as `lab:seasonal_canopy_loss@1`, and its first real test is the next experiment.

### 5.7 Site by site (post hoc, decides nothing)

Both MCC experiments cached their signed inputs, so every method can be rescored per site without reading anything
new. The recomputation reproduces every pooled score and interval in sections 5.5 and 5.6 exactly. None of this was
registered, so it decides nothing; it says where the pooled results come from.

**Pooled wins are not site wins.** Counting only sites with both loss and stable cells, the seasonal NBR rule's MCC
exceeds the baseline's at 7 of 16 sites at scale and 3 of 5 in section 5.6; the desk's NDVI rule at 8 of 16 and 4 of
5. At many sites the replicated rules flag nothing, and their MCC there is 0. The baseline flags every stable cell at
Chiquitania, Jasper, Gran Chaco and Acre, so its MCC there is 0 as well. Pooled, the baseline's false alarms add up
across sites and the quiet rules' do not.

| site (section 5.6) | loss | stable | `seasonal_loss_nbr@1` MCC (false alarms) | `vegetation_loss@1` | baseline |
|---|---|---|---|---|---|
| Petén | 84 | 17 | 0.152 (0.118) | 0.050 (0.294) | -0.027 (0.882) |
| Ketapang | 38 | 55 | 0.255 (0.000) | 0.423 (0.055) | 0.486 (0.200) |
| Jambi | 48 | 20 | 0.112 (0.000) | 0.112 (0.000) | 0.032 (0.300) |
| Tshopo | 5 | 126 | -0.025 (0.016) | -0.085 (0.159) | -0.170 (0.778) |
| Chaco | 17 | 68 | 0.000 (0.000) | 0.125 (0.044) | 0.069 (0.221) |

**The rules lose where clearings are small and rare.** Recomputed by MCC from the signed records, the second
experiment (section 5.3: 79 loss and 527 stable cells, mostly clearings a few percent of a cell) scores the seasonal
NBR rule at -0.016 and the desk's NDVI rule at -0.010, against 0.284 for the baseline. Both replicated rules would
have lost there by the score they now win on. Neither was registered to be judged that way then, so it does not
change their status. It does bound them: they do best where losses are large or common (fire at Jasper, clearing
fronts at Acre, Petén and Bokeo) and worse where a few small clearings sit in a large stable forest (Sinop,
Tanintharyi, Madre de Dios), where the registry rule's sensitivity sometimes pays. Prey Lang, with five loss cells
among 140, is the exception: the seasonal NBR rule scored 0.626 there with no false alarm, the baseline 0.307.

**The fixed search pick on the scaled forests.** `lab:seasonal_canopy_loss@1` was picked on the seven forests of
section 5.6, so the 19 scored forests of section 5.5 are new to it. There it scores MCC 0.360 against 0.295 for the
baseline (95% interval of the difference +0.009 to +0.119), and level with the seasonal NBR rule (-0.058 to +0.017).
The researchers had seen those forests, and the comparison was not registered, so this is a reason to expect the
third replication to go one way, not evidence that it will.

### 5.8 Third replication: ten fresh forests (MCC decides)

Novo Progresso (Pará), Lábrea (Amazonas), Caquetá (Colombia), Mai-Ndombe (DR Congo), Lomié (East Cameroon), Kapit
(Sarawak), the Central Highlands of Vietnam, the Oregon Coast Range, south-east Georgia and Västerbotten (Sweden):
none scored by any earlier experiment, and four of them temperate or boreal. 226 loss and 883 stable cells. The plan
(`c73ovt5juw6ebmjsuista46624`) registered the same methods, labels and decision as section 5.6, with
`lab:seasonal_canopy_loss@1` among the methods.

| method | MCC | F1 | precision | recall | false alarm rate | MCC minus baseline, 95% interval |
|---|---|---|---|---|---|---|
| `lab:green_fraction_loss@1+screen` | 0.311 | 0.446 | 0.464 | 0.429 | 0.127 | +0.140 to +0.285 |
| `lab:seasonal_canopy_loss@1` | 0.276 | 0.399 | 0.465 | 0.350 | 0.103 | +0.103 to +0.248 |
| `lab:seasonal_loss_nbr@1` | 0.262 | 0.321 | 0.554 | 0.226 | 0.046 | +0.086 to +0.246 |
| `lab:subcell_loss_p10@1+screen` | 0.227 | 0.405 | 0.352 | 0.478 | 0.225 | +0.059 to +0.201 |
| `desk:vegetation_loss@1` | 0.202 | 0.381 | 0.343 | 0.429 | 0.211 | +0.036 to +0.172 |
| `lab:green_fraction_loss@1` | 0.201 | 0.395 | 0.318 | 0.522 | 0.287 | +0.041 to +0.162 |
| `desk:vegetation_loss@1+screen` | 0.177 | 0.348 | 0.342 | 0.354 | 0.174 | +0.007 to +0.150 |
| `desk:vegetation_loss_nbr@1` | 0.166 | 0.305 | 0.366 | 0.261 | 0.116 | -0.007 to +0.147 |
| `registry:deforestation_alert_ndvi_drop@1+screen` | 0.162 | 0.354 | 0.311 | 0.412 | 0.233 | +0.006 to +0.127 |
| `lab:seasonal_loss_ndvi@1` | 0.147 | 0.361 | 0.284 | 0.496 | 0.320 | -0.010 to +0.109 |
| `lab:seasonal_loss_either@1` | 0.147 | 0.362 | 0.282 | 0.504 | 0.328 | -0.010 to +0.106 |
| `registry:burn_severity_from_dnbr@1+screen` | 0.138 | 0.298 | 0.326 | 0.274 | 0.145 | -0.031 to +0.115 |
| `lab:subcell_loss_p10@1` | 0.115 | 0.349 | 0.257 | 0.544 | 0.403 | -0.050 to +0.086 |
| `registry:deforestation_alert_ndvi_drop@1` | 0.098 | 0.354 | 0.233 | 0.739 | 0.623 | baseline |
| `registry:burn_severity_from_dnbr@1` | 0.045 | 0.298 | 0.226 | 0.438 | 0.384 | -0.120 to +0.017 |

The seasonal NBR rule and the desk's held NDVI drop beat the baseline for the third time, each time on forests no
earlier experiment had scored. The fixed search pick `lab:seasonal_canopy_loss@1`, on its first test as a rule of its
own, beat it too, a point above the seasonal NBR rule it differs from only in its settings (0.276 against 0.262; the
two were not compared by interval). The registry rule had
its lowest MCC yet (0.098), flagging 62% of stable forest.

Three things went against the record so far. The desk's NBR rule, which won in section 5.6, did not win here. The
best score belongs to a sub-pixel rule: the share of green pixels, after the cloud screen, which lost in both earlier
MCC experiments. And this run's own search did not beat the baseline out of sample (MCC 0.135, 95% interval -0.035 to
+0.110): its picks ranged over drops of 0.15 to 0.30, all from a starting level of 0.3, so the search is not a stable
procedure on ten sites even though the rule it picked last time held up.

Two of the ten boxes held no loss cells (Lábrea, 144 stable; Kapit, 139 stable). The sites were chosen by region, not
by known loss, and the boxes fell on standing forest. Their cells count as stable in the pooled score.

**The cloud screen**, same cells, MCC: it helped the NDVI-drop rule (+0.006 to +0.127), the dNBR rule (+0.040 to
+0.147), the 10th-percentile rule (+0.074 to +0.150) and the green-fraction rule (+0.063 to +0.160), and had no clear
effect on the desk's held drop (-0.062 to +0.011).

### 5.9 Across the three MCC experiments: 36 forests (post hoc, decides nothing)

The three MCC experiments cover 36 scored forests on three disjoint site sets, and all their inputs are cached, so
the questions below are answered from signed data without reading anything new. Every recomputation first reproduced
the published pooled scores and intervals exactly. None was registered, so none decides anything.

**Detection depends on how much of the cell was cleared.** Over the 960 loss cells:

| share of the 400 m cell cleared | loss cells | found by a quiet rule | found by the baseline |
|---|---|---|---|
| 5 to 10% | 249 | 20% | 71% |
| 10 to 25% | 292 | 36% | 76% |
| 25 to 50% | 188 | 56% | 78% |
| 50% or more | 231 | 91% | 92% |

A quiet rule here is any of the seasonal NBR rule, the desk's held NDVI drop and the desk's NBR rule, the three that
flag the least stable forest. Where half the cell or more was cleared, they find as much as the registry rule while
raising a fraction of its false alarms. The baseline's 71% on the smallest clearings is not much above the 57% of stable cells it flags anyway in
these experiments. The cells all three missed had a median of 12% of the cell cleared; those found, 43%.
The rules differ in what they flag on stable forest far more than in what they find.

**Combining the replicated rules does not help.** On the two experiments of sections 5.5 and 5.6, an OR of the
seasonal NBR rule and the desk's NDVI rule scores MCC 0.383 against 0.412 for the seasonal rule alone (difference
-0.064 to +0.007); an AND scores 0.381 (-0.053 to -0.010), at a precision of 0.94 and 0.7% false alarms. The two rules
miss the same cells, the ones with little clearing, so the limit is the measurement, not the choice of rule.

**Site by site.** In section 5.8 the seasonal NBR rule's MCC exceeds the baseline's at 5 of the 8 sites with both
labels (7 of 16 at scale, 3 of 5 in section 5.6). The new failure mode is snow: at Västerbotten every rule flags much
of the stable forest (the desk's NDVI rule all of it, the baseline 80%, the seasonal NBR rule 28%). A likely reason,
not yet tested: a pass with snow on the canopy reads low in NDVI and NBR, and the cloud screen keeps a low pass when
the passes either side are low too.

**The cloud screen, three times.** Across the three MCC experiments it helped a single-pass or sub-pixel detector in 7
of 12 tests and never hurt one, and it never helped the desk's held drop (it hurt it at scale and had no clear effect
twice). That is the one finding about the screen that held: apply it before rules that compare single passes or read
percentiles, and not before a rule that already waits for a drop to hold.

### 5.10 Cell size: 200 m against 400 m on the same pixels

Section 5.9 says the quiet rules miss clearings that are a small share of a 400 m cell. This experiment asks whether
a finer cell is the fix. Its plan (`w5sfgf4sczudavcmkgaj7x564a`) was signed at 12:04 UTC, before the third
replication's result existed, so the choice of group A could not follow that result.

**Design.** Over the central 2.4 km of each forest, 6 x 6 cells of 400 m and 12 x 12 cells of 200 m nested inside
them, all 180 read in one `field_series` request per index, so both sizes see the same scenes (66 of them, the
server's budget for 180 cells). Each detector runs on every cell as before. A flag is a claim about every pixel in
its cell, so each Hansen pixel takes its 400 m cell's flag and, separately, its 200 m cell's: the two sizes are scored
on the same ground, by the Matthews correlation over pixels. A pixel is LOSS if it lost cover in 2024 or 2025 and
STABLE if it never did; others are left out. The interval resamples 400 m blocks, each carrying its pixels at both
sizes. Group A, which decides, is the third replication's ten forests: 2,874 loss and 53,603 stable pixels. Group B is
the seven forests of section 5.3, chosen because their clearings were small, so it decides nothing: 910 loss and
25,023 stable pixels.

**Group A (decides).**

| method | pixel MCC, 400 m | 200 m | 200 m minus 400 m, 95% interval | loss pixels found, 400 / 200 m | stable pixels flagged, 400 / 200 m |
|---|---|---|---|---|---|
| `lab:seasonal_loss_nbr@1` | 0.217 | 0.235 | -0.063 to +0.096 | 32% / 36% | 6.1% / 6.6% |
| `desk:vegetation_loss@1` | 0.164 | 0.231 | -0.004 to +0.154 | 48% / 59% | 18.1% / 17.6% |
| `lab:seasonal_canopy_loss@1` | 0.112 | 0.192 | -0.003 to +0.172 | 26% / 40% | 9.9% / 11.3% |
| `lab:subcell_loss_p10@1+screen` | 0.121 | 0.202 | **+0.007 to +0.155** | 25% / 33% | 8.9% / 7.2% |
| `lab:subcell_loss_p10@1` | 0.100 | 0.181 | **+0.023 to +0.142** | 37% / 50% | 18.7% / 17.4% |
| `lab:green_fraction_loss@1+screen` | 0.177 | 0.203 | -0.026 to +0.079 | 49% / 52% | 17.2% / 16.6% |
| `lab:seasonal_loss_ndvi@1` | 0.197 | 0.176 | -0.109 to +0.054 | 41% / 42% | 11.3% / 13.4% |
| `registry:deforestation_alert_ndvi_drop@1` | -0.012 | 0.039 | -0.003 to +0.105 | 47% / 57% | 49.7% / 48.1% |

(Seven more detectors are in the result note; none has an interval clear of zero.) On the registered rule, 200 m
cells help two detectors, the 10th-percentile rule with and without the screen, and hurt none. The pixel MCC is
higher at 200 m for 14 of the 15 detectors and every detector finds more of the lost pixels, while the share of
stable pixels flagged barely moves. For the other twelve the interval crosses zero, so the effect there is not shown.

**Group B (reported, decides nothing).** Where the clearings were small the change is larger. The seasonal NBR rule
found none of the loss pixels at 400 m and finds 15% at 200 m while flagging 0.1% of stable pixels (pixel MCC 0.000
to 0.343, interval +0.129 to +0.478). The 10th-percentile rules, the screened dNBR rule and the screened desk rule
also gain with intervals clear of zero; no detector loses.

**Reading it.** The 10th-percentile rule is the one built for clearings that move a cell's edge before its median,
and it is the one that gains: at 200 m a clearing of 1 ha is a quarter of the cell, enough to drag its darkest tenth
down. Pixel scores are not comparable with the cell scores of earlier sections, which judge different units. The
block bootstrap treats 400 m blocks as independent, but blocks within one forest are not, and Mai-Ndombe holds 42% of
group A's loss pixels, so the intervals may be narrower than the truth. The rules were tuned at 400 m; at 200 m each
cell has a quarter of the pixels and noisier medians, so these are the rules' scores at a size they were not built
for. A full experiment at 200 m cells, scored per cell against each method's baseline, is the next test.

### 5.11 Snow: rejecting Sen2Cor's snow class (a reading experiment)

`field_series` keeps pixels that Sen2Cor's scene classification calls snow (SCL 11) unless the caller rejects them,
and at Västerbotten every rule flagged much of the stable forest. This experiment (plan `w54isezafoxudzsqffpcbazcbu`)
read each forest twice from the same scenes, with the default mask and also rejecting SCL 11, and scored every
detector on both readings of the same cells. Group A, which decides, is seven snowy managed forests no experiment had
scored (Kainuu, Dalarna, Arkhangelsk, Abitibi, Hearst, Prince George and Tokachi; North Woods in Maine could not be
read, `band_raster` answering 502 four times). Group B, reported only, is Jasper, Flin Flon and Västerbotten.

In group A (50 loss and 710 stable cells), every detector was near useless at 400 m with either mask: MCC between
-0.11 and +0.12. The default reading flagged 81% of stable forest by the registry rule and 98% by the desk's held
drop. Rejecting snow lowered the false-alarm rate of all 15 detectors, by a fifth to a half (the registry rule 81% to
66%, the desk rule 98% to 59%, the seasonal NBR rule 12% to 6%), while dropping only one to four of about 72 passes a
site. But the loss found fell with the false alarms, so MCC changed for three detectors only: the mask helped the
screened dNBR rule (+0.045 to +0.172) and hurt both versions of the desk's held drop (-0.192 to -0.045 and -0.229 to
-0.072).

So the default mask stays. The finding an agent needs is the other one: none of these rules finds canopy loss in
snowy boreal forest at 400 m, with or without the snow mask, and their flags there are mostly false.

### 5.12 Managed forests: the sub-pixel rules where harvests take part of a cell (MCC decides)

The sub-pixel rules lost by MCC at scale and on the seven forests of section 5.6, and won on the ten of section 5.8,
where they scored best at managed forests. This experiment (plan `2qi3f5hui526dvsoiz7ryv5voy`) registered that as a
hypothesis and tested every method on eight plantation and managed forests no experiment had scored: Kaingaroa (New
Zealand), Biobío (Chile), Três Lagoas (Mato Grosso do Sul), Águeda (Portugal), Guangxi (China), the Landes (France),
Mpumalanga (South Africa) and north-east Tasmania. 119 loss and 464 stable cells.

| method | MCC | F1 | precision | recall | false alarm rate | MCC minus baseline, 95% interval |
|---|---|---|---|---|---|---|
| `lab:subcell_loss_p10@1+screen` | 0.389 | 0.477 | 0.603 | 0.395 | 0.067 | +0.265 to +0.484 |
| `lab:green_fraction_loss@1+screen` | 0.322 | 0.438 | 0.505 | 0.387 | 0.097 | +0.205 to +0.414 |
| `desk:vegetation_loss@1` | 0.301 | 0.402 | 0.520 | 0.328 | 0.078 | +0.163 to +0.410 |
| `lab:seasonal_loss_nbr@1` | 0.262 | 0.248 | 0.692 | 0.151 | 0.017 | +0.138 to +0.355 |
| `lab:seasonal_canopy_loss@1` | 0.258 | 0.339 | 0.517 | 0.252 | 0.060 | +0.143 to +0.343 |
| `lab:green_fraction_loss@1` | 0.237 | 0.409 | 0.367 | 0.462 | 0.205 | +0.136 to +0.313 |
| `lab:subcell_loss_p10@1` | 0.234 | 0.400 | 0.375 | 0.429 | 0.183 | +0.108 to +0.328 |
| `registry:deforestation_alert_ndvi_drop@1+screen` | 0.096 | 0.260 | 0.292 | 0.235 | 0.147 | -0.000 to +0.163 |
| `registry:deforestation_alert_ndvi_drop@1` | 0.013 | 0.292 | 0.210 | 0.479 | 0.463 | baseline |
| `registry:burn_severity_from_dnbr@1` | -0.079 | 0.227 | 0.167 | 0.353 | 0.450 | -0.174 to -0.004 |

Five more methods are in the result note; all beat the baseline, by less. The hypothesis held: in managed forest the
10th-percentile rule after the cloud screen had the best MCC of any method in any experiment, and the share-of-green
rule after the screen was second. The registry rule scored 0.013, barely above chance, while flagging 46% of stable
forest. The three replicated rules won again: for the fourth time for the seasonal NBR rule and the desk's NDVI rule,
and for the second time as a fixed rule for the searched one. The search itself, scored out of sample, also beat the
baseline (MCC 0.225, +0.099 to +0.322), picking a 0.30 NBR drop at six of eight sites.

The loss cells were concentrated: Águeda and Guangxi held 88 of the 119, Mpumalanga none and Biobío one. The screen
helped the 10th-percentile (+0.100 to +0.207), share-of-green and dNBR rules and hurt the desk's held drop again
(-0.231 to -0.044).

**What an agent should take from it.** The rule to use depends on the forest. Where harvests take part of a cell, the
sub-pixel rules after the screen are the best tested; on tropical clearing frontiers (sections 5.5 and 5.6) they were
among the weakest. Their status reads mixed, which is the record saying exactly that.

### 5.13 Memory and fusion: detectors built on each cell's own past (MCC decides)

Every rule above compares one or two optical indices with a fixed threshold. This experiment (plan
`s6ld3nyqfaeekcoaotbxl2geny`) registered two generations of memory-driven detector (program, section 4.5) on ten
tropical forests no experiment had scored. Version 1 remembers one index per sensor from the year before the window.
Version 2 remembers five years across NDVI, NBR, NDMI and Sentinel-1 VH, scores each pass with a matched filter
under the cell's own covariance, and calibrates its alarm on the cell's remembered noise, stating a 5% false-alarm
chance per stable cell.

Four of the ten forests could not be read: Apuí, Ogooué-Ivindo and Kachin returned 502 after retries, and Équateur a
400. Heavy multi-year, multi-index reads met the server's own timeouts and upstream errors (the gap ledger records
it). The experiment scored the other six: 104 loss and 391 stable cells, so its intervals are wide.

| method | MCC | precision | recall | false alarm rate | MCC minus baseline, 95% interval |
|---|---|---|---|---|---|
| `lab:memory_mv_fusion@2` | 0.366 | 0.550 | 0.423 | 0.092 | **+0.029 to +0.257** |
| `lab:memory_fusion@1` | 0.298 | 0.329 | 0.798 | 0.432 | -0.013 to +0.151 |
| `lab:seasonal_canopy_loss@1` | 0.290 | 0.438 | 0.442 | 0.151 | -0.038 to +0.176 |
| `lab:memory_mv_optical@2` | 0.290 | 0.453 | 0.413 | 0.133 | -0.035 to +0.170 |
| `lab:seasonal_loss_nbr@1` | 0.273 | 0.655 | 0.183 | 0.026 | -0.077 to +0.175 |
| `lab:memory_optical@1` | 0.254 | 0.351 | 0.587 | 0.289 | -0.067 to +0.125 |
| `lab:memory_radar@1` | 0.237 | 0.355 | 0.519 | 0.251 | -0.095 to +0.119 |
| `registry:deforestation_alert_ndvi_drop@1` | 0.225 | 0.268 | 0.913 | 0.665 | baseline |
| `lab:memory_mv_fusion_fdr@2` | 0.181 | 0.452 | 0.183 | 0.059 | -0.167 to +0.077 |
| `lab:memory_mv_fusion_rel@2` | 0.156 | 0.425 | 0.163 | 0.059 | -0.194 to +0.049 |
| `desk:vegetation_loss@1` | 0.112 | 0.333 | 0.192 | 0.102 | -0.230 to +0.004 |

The fused version-2 detector was the only method to beat the registry rule, and the best by point score. The
registered comparisons were all inconclusive at this size except one: false-discovery control across a forest's
cells was worse than per-cell calibration (-0.276 to -0.091), because at q = 0.10 it flagged a fifth of the loss.
The point estimates favour version 2 over version 1 (+0.068), radar over optical alone within version 2 (+0.076)
and memory over the fixed seasonal rule (+0.093), but every one of those intervals crosses zero.

**The stated false-alarm rate did not hold on real forests.** The fused detector flagged 9.2% of stable cells (Wilson
interval 6.7 to 12.5%) and the optical one 13.3% (10.3 to 17.0%), against a stated 5%. On synthetic stable cells the
same calibration held at 2 to 5%. Two causes are possible and the record cannot yet separate them: real forests
change in ways five remembered years do not contain, and some cells Hansen calls stable did lose canopy. Until a
second reference separates them, read the stated rate as a floor.

**The replicated rules did not win here.** The seasonal NBR rule and the searched rule tied with the baseline
(intervals -0.077 to +0.175 and -0.038 to +0.176); the desk's NDVI rule scored below it. Under the program's rule a
failure to win after a first win means not replicated, so all three now read not replicated (section 5.19).

### 5.14 Two references: loss in 2022 and 2023, judged by Hansen and by JRC TMF (MCC decides)

Every result above is agreement with Hansen, and section 6.5 measures how far Hansen and JRC TMF agree. This
experiment scored every method three ways on ten tropical forests no experiment had scored, for loss in 2022 and 2023
(TMF ends in 2023): against Hansen, which decides; against TMF's deforestation year by the same rule of 5% of a 400 m
cell, with Hansen's forest mask; and on the cells where the two agree (`canopy-loss-two-references-2022-2023-2026-10-29`,
plan content id `nppiqq7e6wznwzwpa4kfxuvele`).

**An erratum, signed before the result.** The plan's labels and decision paragraphs had been copied from the
2024-2025 experiments and named their years. The code scores by the experiment's parameters (years 2022 and 2023,
2021 adjacent, flags dated 2022-01-01 to 2024-03-31), which match the plan's question. An erratum stating this was
signed beside the plan while the forests were being read and before any result existed
(`erratum.md`, content id `tkdjsvkaslmwr7dlt6maddpcgy`); the transparency log orders it before the result. The lab now
refuses to register a plan whose labels or decision name a year or a date its parameters do not contain.

**Under Hansen the memory detectors lead.** 42 cells were labelled loss and 1,062 stable.

| method | MCC | false-alarm rate | MCC minus the registry rule, 95% interval |
|---|---|---|---|
| `lab:memory_mv_optical@2` | 0.303 | 1.2% | +0.085 to +0.400 |
| `lab:memory_fusion@1` | 0.269 | 8.2% | +0.115 to +0.303 |
| `lab:memory_optical@1` | 0.253 | 10.0% | +0.105 to +0.276 |
| `lab:seasonal_canopy_loss@1` | 0.190 | 5.1% | +0.024 to +0.238 |
| `lab:memory_mv_fusion@2` | 0.188 | 1.8% | +0.004 to +0.271 |
| `lab:memory_mv_fusion_emp@3` | 0.186 | 1.3% | -0.007 to +0.277 |
| `lab:seasonal_loss_nbr@1` | 0.014 | 1.5% | -0.121 to +0.053 |
| `registry:deforestation_alert_ndvi_drop@1` | 0.058 | 49.2% | baseline |

The fixed seasonal NBR rule, replicated (forest-dependent) on the 2024-2025 forests, found almost nothing here, and the
memory rules beat it on the same cells (`memory_fusion@1` minus `seasonal_loss_nbr@1`: +0.136 to +0.364). The
calibrated detectors stated a 5% chance of flagging a stable cell and flagged 1.2 to 1.8%, so their calibration errs
toward silence. The registered comparisons of version 2 against version 1, and of version 3 against version 2, show
no clear difference. The two variants with false-discovery control across a forest flagged no cell at all, which makes
them significantly worse than per-cell calibration (-0.324 to -0.061).

The loss is concentrated. 26 of the 42 loss cells are in Pacajá (Pará), 7 in Palawan, 5 north of Porto Velho, and
four of the ten forests (Paragominas, Lacandona, Imataca, Dja) had none, so the pooled score leans on one forest. The
per-site lines are in the result note.

**TMF could not judge.** By the same cell rule TMF marked 5 of these cells as loss in 2022 and 2023, against Hansen's
42. No method can be shown better or worse with five positives, and every method reads "no" for "beats the baseline
under all three references" for that reason, not for its own. TMF's deforestation year records the conversion of
humid tropical forest, and Hansen records any loss of tree cover, so on forests that lose canopy to fire, logging or
plantation cycles more than to conversion the two references count different events. The question this experiment
asked is therefore still open; it needs forests chosen from TMF's own record of conversion before any series is read.

### 5.15 Two optical sensors: Landsat in the memory, on forests that had loss (MCC decides, forests resampled)

Version 4 (`lab:memory_hls_emp@4`) puts Landsat 8 and 9 into the same optical memory as Sentinel-2. Each cell puts
Landsat on Sentinel-2's scale with a per-band offset from its own remembered passes, then both sensors' passes form one
record with one normal and one year offset per year. Sentinel-1 VH stays its own sensor, and the forest-wide empirical
null decides, as in version 3. On a synthetic forest where Sentinel-2 was clouded after the clearings, version 3 found
1 of 10 and version 4 found 10 of 10, with no false alarm in 40 stable cells. That shows the mechanism works; it is not
evidence about real forests.

The experiment (`canopy-loss-two-optical-sensors-2024-2025-2026-10-30`, plan content id
`mmsumwdn3d4ohgiiqiih4tnnbm`) was planned, registered, run and written up by the lab desk on its own between 01:43
and 04:50 UTC on 30 October. Two choices were revised before it was signed:

- **Forests with loss.** It read the nine unused forests of the desk's pool within 7 degrees of the equator that
  lost forest in 2021 to 2023 by Hansen, the three years before the scored ones (13 were candidates; 4 had none).
- **Forests as the unit.** It decides with the forest as the resampled unit.

86 cells were labelled loss and 740 stable, spread over the forests: Lobaye 30, Cavally 17, Kapuas Hulu 11, Sinoe 10,
Meta 7, and 3 or fewer elsewhere.

| method | MCC | precision | recall | false-alarm rate | minus the registry rule, forests resampled (decides) | cells resampled |
|---|---|---|---|---|---|---|
| `lab:memory_hls_emp@4` | 0.420 | 0.71 | 0.29 | 1.4% | +0.031 to +0.685 | +0.227 to +0.492 |
| `lab:memory_mv_fusion_emp@3` | 0.323 | 0.62 | 0.21 | 1.5% | -0.003 to +0.550 | +0.116 to +0.399 |
| `lab:memory_mv_fusion@2` | 0.259 | 0.43 | 0.23 | 3.6% | -0.069 to +0.493 | +0.066 to +0.327 |
| `lab:memory_optical@1` | 0.228 | 0.24 | 0.47 | 16.8% | +0.010 to +0.335 | +0.069 to +0.264 |
| `lab:seasonal_canopy_loss@1` | 0.107 | 0.23 | 0.14 | 5.4% | -0.052 to +0.164 | -0.049 to +0.150 |
| `lab:seasonal_loss_nbr@1` | 0.052 | 0.19 | 0.06 | 2.8% | -0.154 to +0.118 | -0.104 to +0.097 |
| `registry:deforestation_alert_ndvi_drop@1` | 0.058 | 0.12 | 0.69 | 59.3% | baseline | baseline |

Version 4 scored best and beat the registry rule with forests resampled. So did `memory_optical@1`; no other method
did. The registered question is still open. Version 4 minus version 3 runs from -0.066 to +0.203: Landsat added about
0.10 MCC here, but nine forests cannot tell that from zero. Version 4 beat version 2 (+0.008 to +0.278) and the
fixed seasonal NBR rule (+0.140 to +0.594).

The calibrated detectors stated a 5% chance of flagging a stable cell and flagged 1.4 to 3.6%, so they err toward
silence. Version 4 finds 29% of the loss cells, and 71% of what it flags is loss.

As the plan stated, Hansen maps loss from Landsat, so part of version 4's gain may be one sensor agreeing with itself.
As the plan promised, each forest's clear passes were counted afterwards from the cached series; the counts decide
nothing. Over 2023 to October 2026, a cell's median clear passes were 21.5 to 63 from Sentinel-2 and 48 to 119 with
Landsat added. Landsat about doubled every forest's clear looks. Sinoe, the cloudiest (21.5 Sentinel-2 passes), is
one of the three forests where version 4 did best (MCC 0.30; Sankuru 0.57, Cavally 0.93). Nine forests show no
gradient one way or the other.

### 5.16 As of the day, and checked by another agent (post hoc on section 5.15, deciding nothing)

The program now asks whether an agent can tell that a method it trusted is wrong for a place, choose another on
independent evidence, and have a second agent check the whole decision ([Method selection](method-selection.md)).
Two of the measurements that question needs were first taken on the two-optical-sensor experiment. Its data had
already been scored, so both are post hoc.

**As-of replay.** Each detector was re-run on every cell's series truncated at nine quarterly dates from March
2024 to March 2026, with the scoring window ending at that date, so that each output used only passes that existed
then. The retrospective run had read passes up to October 2026, seven months after the window closed.

| method | MCC, retrospective | MCC as of 31 March 2026 | 2024 loss flagged by the end of 2024 | false alarms by then | alerts withdrawn later (loss, stable) |
|---|---|---|---|---|---|
| `lab:memory_hls_emp@4` | 0.420 | 0.420 | 25% | 0.4% | 4, 12 of 51 cells ever alerted |
| `lab:memory_mv_fusion_emp@3` | 0.342 | 0.342 | 20% | 0.9% | 9, 13 of 54 |
| `lab:memory_optical@1` | 0.228 | 0.228 | 35% | 8.8% | none of 164 |
| `lab:seasonal_canopy_loss@1` | 0.107 | 0.107 | 15% | 2.3% | 0, 1 of 53 |
| `registry:deforestation_alert_ndvi_drop@1` | 0.058 | 0.058 | 47% | 37.0% | none of 498 |

The later passes changed no final decision. Every method's score as of the window's end equals its retrospective
score, so this experiment's results did not depend on reading past the window. The memory detectors raise most of
what they will ever find within the loss year: version 4 flagged a quarter of the 2024 loss by the end of 2024, at
0.4% false alarms.

What the retrospective score hid is churn. Versions 3 and 4 decide each cell against the forest's empirical null,
and that null moves as passes arrive. Version 4 withdrew 16 of the 51 alerts it ever raised. Twelve of those were
false alarms it corrected, and four were real losses it dropped. Version 3 withdrew 22, nine of them real losses.
The first memory detector, a sequential test with a fixed null, never withdrew an alert, but by the end of 2025 it
had flagged 15.8% of stable forest. An alert product built on versions 3 or 4 would need to state that a share of
its alerts are later retracted, and how many of those retractions are wrong.

**Re-derived by an agent that does not trust the lab.** `verify_experiment.py` (emem-world) re-derived the experiment
from its signed records alone, without the lab's cache or state. It read the plan and the result from emem memory and
checked their signer. It checked that each method's source and the scoring code hash to the values the plan
registered. For each of the 270 inputs it checked the receipt offline against the responder key, and checked that
the input's canonical bytes hash to the content id it is cited by. It then rebuilt every forest's series and Hansen
rasters from those bytes, reran every method and the scoring, and compared its numbers with the published ones. All
24 published MCCs reproduced, to the third decimal.

A receipt alone would not have been enough. It signs the content id, not the value: in the control, a series with
one value changed still passed the receipt check, and only the hash of the bytes caught it.

One part of the experiment could not be checked from signed records: its years, windows and sites lived in the lab's
private roster. Plans now print those parameters and the scoring code's hash, so later experiments close the gap. The
integrity desk now runs this verification on each new experiment under its own key and signs the outcome as
`verification@1`.

### 5.17 Clearings, by size, against chance (post hoc on section 5.15, deciding nothing)

A 400 m cell is 16 hectares, and the label rule calls a cell lost at 5% of it, about 0.8 ha. A smaller clearing is
never counted, although a clearing that size can matter to the owner of the land or to a compliance check. To see
what the methods find at that scale, the unit of truth was changed from the cell to the clearing. Hansen draws each
clearing as 8-connected 30 m pixels that lost cover in 2024 or 2025 and were forest in 2000. A method finds a clearing
when any cell the clearing touches was flagged. Every forest cell is judged, including those the cell labels left
out as ambiguous.

That measure alone flatters a method that flags everywhere, as F1 did (section 6.1): a cell that is flagged at
random still touches clearings. So beside each count stands the number the method's own flag rate on stable forest
would find if its flags fell at random, 1 - (1 - rate)^k for a clearing touching k cells.

The nine forests of section 5.15 held 305 clearings: 191 under 0.5 ha, 55 of 0.5 to 1 ha, 53 of 1 to 5 ha, 4 of 5
to 20 ha and 2 over 20 ha. Each count below is clearings found, with the number expected by chance in brackets.

| method | under 0.5 ha | 0.5 to 1 ha | 1 to 5 ha | 5 to 20 ha | over 20 ha |
|---|---|---|---|---|---|
| `lab:memory_hls_emp@4` | 18 (4) | 9 (1) | 14 (1) | 4 of 4 (0) | 2 of 2 (0) |
| `lab:memory_mv_fusion_emp@3` | 16 (3) | 4 (1) | 9 (1) | 3 (0) | 2 (0) |
| `lab:memory_optical@1` | 77 (48) | 23 (13) | 32 (15) | 3 (2) | 2 (0) |
| `lab:seasonal_canopy_loss@1` | 35 (13) | 7 (3) | 7 (4) | 2 (1) | 1 (0) |
| `lab:seasonal_loss_nbr@1` | 8 (8) | 1 (1) | 3 (1) | 2 (1) | 1 (0) |
| `registry:deforestation_alert_ndvi_drop@1` | 136 (124) | 40 (38) | 42 (39) | 3 (3) | 1 (2) |

At 400 m cells, then, emem finds large clearings and misses most small ones. Version 4 found all six clearings over
5 ha. It found a quarter of those of 1 to 5 ha, against one expected by chance, and a tenth of those under 0.5 ha.
The first memory detector finds more small clearings above chance (29 more under 0.5 ha than chance would give,
against 14 for version 4), at the cost of flagging 17% of stable forest.

The registry rule's apparent reach is chance: its 136 small clearings stand against 124 from its flag rate alone.
The fixed seasonal NBR rule finds small clearings no better than chance.

The next gain at small clearings will come from a finer unit (section 5.10 found 200 m cells better than 400 m),
not from a sharper threshold at 400 m. Every result now carries this table, and its signed record the counts.

### 5.18 Where each method is competent, by climate (post hoc, deciding nothing)

Section 5.9 found that the methods' results differ between forests far more than chance allows. To see how much of
that difference follows the kind of place, every forest of the eight experiments scored by MCC in sections 5.5 to 5.15
(79 forests; the cell-size and snow experiments aside) was put in the Koppen climate class at its box centre. That class is a signed emem fact (Beck et al.
2018) that no method produced. Each method is scored only on the forests it ran on that had five or more loss cells:
its pooled MCC minus the registry rule's on the same forests, and the interval with forests resampled.

| class | forests with loss | best | `lab:memory_optical@1` | `lab:seasonal_loss_nbr@1` | `desk:vegetation_loss@1` |
|---|---|---|---|---|---|
| Am, monsoon rainforest | 15 | `lab:memory_mv_fusion@2` +0.58 (7 forests; +0.21 to +0.83) | +0.37 (+0.17 to +0.52) | +0.15 (-0.02 to +0.28) | +0.18 (+0.05 to +0.31) |
| Af, rainforest | 13 | `lab:memory_mv_fusion@2` +0.32 (5 forests; +0.23 to +0.39) | +0.16 (+0.03 to +0.28) | +0.02 (-0.12 to +0.14) | +0.03 (-0.09 to +0.13) |
| Aw, savanna and dry forest | 8 | `lab:memory_optical@1` +0.11 | +0.11 (+0.00 to +0.28) | -0.06 (-0.22 to +0.17) | -0.19 (-0.29 to -0.02) |
| Cf, temperate | 3 | `lab:memory_optical@1` +0.54 | +0.54 (+0.46 to +0.64) | +0.43 | +0.26 |
| Df, boreal | 3 | `lab:memory_optical@1` +0.48 | +0.48 (0.00 to +0.89) | +0.35 | -0.29 |

Competence follows the climate. In evergreen forest (Am, Af, Cf) the detectors that compare a cell with its own
past beat the registry rule by a wide margin. In the seasonally dry forests of class Aw, where leaves fall each dry
season and a cell browns without being cleared, the registry rule's plain drop holds up (MCC 0.375). Only the first
memory detector is above it there, and the desk's own rule, which wins in class Am, is significantly worse than it.
In class Af the fixed seasonal rules have no edge at all. The first memory detector is the one method above the
registry rule in every class it ran in.

This is post hoc. The classes were not registered, climate is confounded with continent, the fusion detectors ran
on two to seven forests per class, and most of these forests are scored for 2024 and 2025. That last point is why the
table does not feed a decision. A choice may read only evidence dated before the period it acts on (rule 14), and
the method-selection experiment scores its decisions on 2024 and 2025. Its context policy instead reads a pool of
forests of the same climates scored for 2022 and 2023 ([Method selection](method-selection.md), section 6).

### 5.19 Status of every proposal

**The status rule was changed on 29 October, in the open.** The rule in force until then asked every pre-registered
experiment after a method's first win to win as well. Section 5.13 showed what that does: one experiment that read six
of its ten forests demoted three methods, two of them after four wins in a row. Given enough experiments a rule like
that reads *not replicated* for any method, however good. From 29 October a method's status is decided by all of its
evidence (rule v2, signed in emem memory at `research/rules/status-v2.md`). Over three or more disjoint MCC site sets
it is **replicated** when the random-effects pooled MCC difference from the registry rule (DerSimonian and Laird 1986)
has its 95% lower bound above zero and no experiment was significantly worse, and **forest-dependent** when I² is 75%
or more. The rule was adopted after the strict rule demoted those methods, and every proposal note prints both
verdicts.

| method | status (rule v2) | pooled MCC difference, 95% interval | I² | MCC site sets |
|---|---|---|---|---|
| `lab:memory_optical@1` | replicated, forest-dependent | +0.209, +0.026 to +0.393 | 92% | 4 |
| `lab:seasonal_canopy_loss@1` | replicated | +0.153, +0.092 to +0.214 | 59% | 6 |
| `lab:seasonal_loss_nbr@1` | replicated, forest-dependent | +0.119, +0.037 to +0.201 | 86% | 8 |
| `desk:vegetation_loss@1` | replicated, forest-dependent | +0.078, +0.012 to +0.144 | 78% | 8 |
| `registry:deforestation_alert_ndvi_drop@1+screen` | replicated | +0.039, +0.008 to +0.070 | 40% | 8 |
| `lab:memory_hls_emp@4` | proposed, awaiting replication | one site set (section 5.15) | | 1 |
| `lab:memory_mv_fusion@2` | not shown better | +0.341, -0.098 to +0.781 | 98% | 4 |
| `lab:memory_mv_optical@2` | not shown better | +0.311, -0.086 to +0.708 | 97% | 4 |
| `lab:memory_fusion@1` | not shown better | +0.248, -0.008 to +0.503 | 96% | 4 |
| `lab:memory_radar@1` | not shown better | +0.165, -0.125 to +0.456 | 97% | 4 |
| `lab:memory_mv_fusion_fdr@2`, `_rel@2` | not shown better | about +0.21, -0.38 to +0.79 | 99% | 4 |
| `lab:memory_mv_fusion_emp@3` | not shown better | two site sets | | 2 |
| the other ten lab, desk and registry rules | not shown better | between -0.03 and +0.09 | | 8 |

Five methods are replicated. The first memory detector, `memory_optical@1`, is replicated and forest-dependent, by +0.21 MCC. The searched seasonal rule is replicated by +0.15 over six site sets. The seasonal NBR rule and the desk's rule are replicated and forest-dependent, and the screened registry rule by a small, steady 0.04. Each proposal note also prints the strict rule's verdict. The later memory detectors pool to the largest differences, +0.17 to +0.34, and are not shown better, because their site sets disagree by far more than their intervals allow (I² 96 to 99%). With four studies the between-study variance is itself poorly estimated. The next paragraph shows where the disagreement comes from.

**The replication, completed, and what one clearing does to a pooled score.** The replication of section 5.13 on its
four unread forests and six new ones (`canopy-loss-memory-fusion-replication-2024-2025-2026-10-29`, plan content id
`u24aifkusddf2zyyhoyhvygwtm`) ran into three faults in turn. The first was the LZW decoder, which refused a Hansen
block whose pixels were all there. The second was a forest raster across a 10-degree tile line. The third was a
catalogue rate limit that failed a task. Each fault is in the gap ledger and fixed. Once all three were fixed, the run
was completed under its signed plan. The earlier result, from six forests and 3 loss cells, stays in the result note's
history.

Completed, it scored 59 loss and 1,175 stable cells. The best detector reached MCC 0.93 (`lab:memory_mv_fusion_rel@2`;
`memory_mv_fusion@2` 0.80, `memory_mv_optical@2` 0.73), against -0.08 for the registry rule. But 56 of the 59 loss
cells are one clearing at Humaitá (Amazonas), and seven of the ten forests had no loss.

The registered interval resamples cells, which counts that clearing as 56 independent findings: it put the best
detector's lead at +0.93 to +1.08. With forests resampled instead (post hoc, deciding nothing, after the pooled numbers
were reproduced exactly from the run's own inputs), the same lead runs from -0.03 to +1.16. With Humaitá left out, the
version-2 detectors' edge is gone, while version 1's memory detectors keep +0.09 to +0.15.

The same check on section 5.14 gives a steadier picture. Pacajá holds 26 of that experiment's 42 loss cells. Without
it, `memory_mv_optical@2` keeps +0.21 and `memory_optical@1` +0.11. `memory_optical@1` is the one method whose
forest-resampled interval lies above zero (+0.036 to +0.28).

The unit of evidence is the forest, or the clearing in it, not the cell. Every result now reports both intervals. The
two-optical-sensor experiment, registered next, decides by forests. It reads the nine unused equatorial forests of the
desk's pool that lost forest in 2021 to 2023, the three years before its window, so that its reads land where there is
loss to find.

## 6. Discussion

### 6.1 The score decided the winner

F1 counts loss found and false alarms raised but not the stable forest a method correctly left alone. At Bosawas,
135 cells lost cover and two were stable, so a rule that flags almost everything scores well there, and pooling
carries that into the total. The registry rule's false alarm rate was 0.59, 0.43 and 0.74 in the three F1
experiments. The Matthews correlation uses all four cells of the confusion table and cannot be won that way. We
recomputed it from the signed records after the fact (the columns marked "not registered"); because that was not the
registered score, it decides nothing about those experiments, and the scaled experiment registered MCC in advance.

### 6.2 The cell is the limit

Losses in the replication were mostly small clearings, a few percent to a fifth of a cell. They barely move a 400 m
cell's median, which every rule except the two sub-pixel ones reads. That is why recall collapsed for the seasonal
rules, which wait for the median to fall, while the single-difference registry rule kept recall by alarming often.
The sub-pixel readings (the 10th percentile, the share of green pixels) were the program's answer to this. Neither
beat the baseline at scale or on the seven fresh forests, and on those the 10th-percentile rule scored below zero.
Reading a 400 m cell's percentiles is not enough to see a clearing a twentieth of its area; the next step is a finer
cell or the raster itself (section 7).

### 6.3 What the first win was

The first experiment's pooled result was carried by Jasper, where the July 2024 fire removed canopy from 80 of its
cells: a large, whole-cell loss that the seasonal rule finds cleanly. Seven forests chosen by hand are not a sample of
forests, and the replication was the test that showed it.

### 6.4 Pooled is the unit of an alert feed

A rule that raises an alarm on fewer than two stable cells in a hundred and finds a third of the loss is worth more to an agent
watching many forests than a rule that finds three quarters of the loss and raises an alarm on more than half the
stable forest. Pooled MCC scores the first kind of agent: one reading alerts across many places, where every false
alarm costs a look. Section 5.7 shows that it does not score the second kind, an agent watching one forest, for whom
the replicated rules often say nothing at all. Both readings are in the record, and the proposals say which one their
status rests on.

Part of the registry rule's false-alarm rate comes from the window, not the rule. It compares single passes, and over
27 months a cell has dozens of passes, so one hazy pass or one harvest is enough to flag it. At four of the 19 forests
scored at scale it flagged every stable cell. A rule that compares medians over weeks, or waits for a drop to hold,
does not add up false alarms that way. That is the property the replicated rules share.

### 6.5 How high can accuracy go? The reference sets the ceiling

Every score here is agreement with Hansen, and Hansen is itself a classification with errors. To see how far a
second expert product agrees with it, Hansen's loss year and JRC TMF's deforestation year were compared on the same
cells with the experiments' own rule (a forest cell is loss in a year when 5% or more of its pixels lost cover), over
the 37 cached tropical forests of the MCC experiments that TMF covers and that could be read. TMF ends in 2023, so the
years are 2021 to 2023. This is descriptive and decides nothing.

| year | forest cells | Hansen loss | TMF loss | agreement, MCC | TMF precision against Hansen | TMF recall against Hansen |
|---|---|---|---|---|---|---|
| 2021 | 4,391 | 357 | 112 | 0.385 | 0.73 | 0.23 |
| 2022 | 4,391 | 318 | 125 | 0.475 | 0.79 | 0.31 |
| 2023 | 4,391 | 432 | 149 | 0.407 | 0.75 | 0.26 |

Two products built by expert teams from the same Landsat archive agree on which 400 m cells lost forest at an MCC of
0.39 to 0.48. Part of the gap is definition: Hansen counts any loss of tree cover, fire and plantation harvest
included, and TMF's deforestation year counts conversion of moist tropical forest. The best detectors in this paper
score 0.26 to 0.46 against Hansen, which is about where a second expert reference scores. A claim of near-perfect
accuracy cannot be verified against these references at this scale, by any method. Progress past this point needs
a better reference before a better method: a probability sample of cells checked by eye on very-high-resolution
imagery, with the accuracy and area estimates that good practice prescribes (Olofsson et al. 2014).

### 6.6 What an agent should do with this

- To watch many tropical forests and act only on what is likely real, use `lab:seasonal_loss_nbr@1` or
  `lab:seasonal_canopy_loss@1`: over five disjoint sets of forests they beat the registry rule by about 0.17 MCC,
  though the strict status rule reads them not replicated after one tie (section 5.19). Read a silence as "no large
  loss seen", not as "no loss": they find nine in ten cells where half the forest went, and one in five where a
  twentieth did.
- Where harvests take part of a cell, use `lab:subcell_loss_p10@1+screen` (section 5.12), and not on clearing
  frontiers.
- In snowy boreal forest, do not trust any of them (section 5.11).
- `lab:memory_mv_fusion@2` won the one experiment it has entered; wait for its next.
- To find every clearing at one site, the registry's `deforestation_alert_ndvi_drop@1` finds more of it, and most of
  what it flags is not loss; check each flag against a second source.
- Do not use a rule whose status is mixed on the strength of its best result.
- Before using any of them, read the catalogue at `research/algorithms/index.md`: the status, the latest interval and
  the limits are current there, and this paper is not.

## 7. Limitations

- Cells are 400 m and the loss label needs 5% of a cell; smaller clearings are outside what was scored.
- Hansen is annual and Landsat-based; detection dates are compared with a year, not a day.
- Site sets were chosen by hand for known activity; pooled scores weight sites by their labelled cells.
- Two of 21 sites failed to load in the sub-cell experiment and are reported as unscored, not dropped silently.
- The rules were written or parameterised by people; the search explored settings, not new rule families.
- Pooling weights each site by its labelled cells, so a site with many loss cells (Bosawas, 135) moves the pooled score
  more than a site with five. Section 5.7 reports per site for that reason.
- In the signed `experiment@1` records of 29 October, the interval of each method against the baseline is stored
  under the key `f1_minus_baseline_95` even where the plan registered MCC, and the records do not name the score.
  The plan's deciding score is the one that was used, and the numbers match the result notes. Records written by the
  lab since then carry `score` and `minus_baseline_95`.
- The desk's two rules were scored in every experiment but entered the proposal ledger only after the sixth. Their
  ledger rows are taken from the five signed records, not recomputed.
- Sites were chosen by region, not by known loss: two of the third replication's ten boxes held no loss cells.
- Snow is not screened by default; rejecting it cuts false alarms in snow but adds no skill (section 5.11).
- Heavy reads fail: the fusion experiment lost four of ten forests to server timeouts and upstream errors, which cost
  it power.
- The program's status rule turns a single tie into not replicated (section 5.19); its verdicts are kept as
  registered, and a meta-analysis is reported beside them.
- The status rule was tightened after the third replication (section 5.13). The change only demotes.
- Cells are the unit the experiments score, and the label rule (5% of a 400 m cell) never counts a clearing under about 0.8 ha. Section 5.17 scores clearings by size against chance; at 400 m the methods miss most small ones.
- Every interval in sections 5.1 to 5.14 resamples cells. Where loss comes in clearings, that overstates certainty: a clearing is one event seen in many cells. Section 5.19 measures how much on two experiments; from the two-optical-sensor experiment on, forests are the resampled unit that decides, and every result reports both.

## 8. Reproducing this

Every artefact is in emem memory under the lab's key and readable with `memory_view`:

| experiment | plan (content id) | result | signed record |
|---|---|---|---|
| canopy-loss-2024-2025 | `research/experiments/canopy-loss-2024-2025-2026-10-29/plan.md` (`5rfzw25vhvlobb6mnj7v6me66e`) | `.../result.md` | `emem:fact:defi.zb391.mAfO.zcb6f:urkmemquo4oj3vymjulyvgrzelwdifo66lq7yuyp3dg5lzvxia3q` |
| canopy-loss-2024-2025-search | `research/experiments/canopy-loss-2024-2025-search-2026-10-29/plan.md` (`uycskbqdtzvtrhqjjp4ddwf4f4`) | `.../result.md` | `emem:fact:defi.zb391.mAfO.zcb6f:oo24dil4wskcmuwb3fn7qxpuawr2gslq4xiewjltqw3pgmyrjuha` |
| canopy-loss-replication-2024-2025 | `research/experiments/canopy-loss-replication-2024-2025-2026-10-29/plan.md` (`bwn6cnqcyqz5k4j66ivhuveafu`) | `.../result.md` | `emem:fact:defi.zb370.yUsU.vIwo:2spflvqzjr7nqzkifz7jnmvqxaoqhpq2sc2nbpznmviwtkabqr2q` |
| canopy-loss-subcell-2024-2025 | `research/experiments/canopy-loss-subcell-2024-2025-2026-10-29/plan.md` (`w2dyuyscw4svzyonyz56yzroz4`) | `.../result.md` | `emem:fact:defi.zb41a.hinI.dUdE:y6ig2nyrqbc7dun54yhaqvyn7lqpxjfdneeddc7unkhfmlsh6s4q` |
| canopy-loss-scaled-2024-2025 | `research/experiments/canopy-loss-scaled-2024-2025-2026-10-29/plan.md` (`ls7ckvg4tkwnxafpo3q2bgkktu`) | `.../result.md` | `emem:fact:defi.zb391.mAfO.zcb6f:mw6r3fd4cwvn6giqrk43a6hj4fnaklsku5fk3nsie7azsqinal3a` |
| canopy-loss-mcc-replication-2024-2025 | `research/experiments/canopy-loss-mcc-replication-2024-2025-2026-10-29/plan.md` (`phqhgqyqikkb3ezjfubwafmugq`) | `.../result.md` (`xwdaqkz4trxnlgxlbinf7ll6pe`) | `emem:fact:defi.zb4c5.zf90f.zedcb:qalxjsyhyianw5rd52geunqd25o3wxb5pwlqy56rumpkyrvd4gea` |
| canopy-loss-third-replication-2024-2025 | `research/experiments/canopy-loss-third-replication-2024-2025-2026-10-29/plan.md` (`c73ovt5juw6ebmjsuista46624`) | `.../result.md` (`sioynhojljozkjwzrha7wczlym`) | `emem:fact:defi.zb3b0.pebO.gIfo:dd6xcxxyybclpofvfmgnsyr7xqeygj27tedngpmoz7iwqtk3i3fq` |

The lab's code (`agents/lab.py`, the experiment runner and their tests) is in the emem-world repository, which stays
private until peer review; the plans, results, proposals and signed records it wrote are public in emem memory now.

Paths are under `/memories/by_attester/mujzmxes/`. The catalogue, `research/algorithms/index.md`, lists every
proposal with its current status and the registry rules' measured scores, and is rewritten after each experiment.
Each proposal and its evidence table is at
`research/algorithms/<key>.md`, where the key is the method name after its origin: `seasonal_loss_nbr@1`,
`vegetation_loss@1`, `vegetation_loss_nbr@1`, `vegetation_loss@1+screen`, `deforestation_alert_ndvi_drop@1+screen`,
`seasonal_canopy_loss@1`, `burn_severity_from_dnbr@1+screen`, `seasonal_loss_ndvi@1` and `seasonal_loss_either@1`.
Each result note lists every input token; each method's source hash in the plan matches `agents/lab.py` at the commit
that ran it. The per-site and post hoc figures in sections 5.7 and 5.9 are recomputed from the inputs the lab cached
for each site, by `lab.py` at the same commit, and reproduce every pooled number in sections 5.5, 5.6 and 5.8.

## References

- Beck, H. E. et al. 2018. Present and future Koppen-Geiger climate classification maps at 1-km resolution.
  Scientific Data 5:180214.
- Chen, J. et al. 2004. A simple method for reconstructing a high-quality NDVI time-series data set based on the
  Savitzky-Golay filter. Remote Sensing of Environment 91:332-344.
- DerSimonian, R. & Laird, N. 1986. Meta-analysis in clinical trials. Controlled Clinical Trials 7:177-188.
- Hansen, M. C. et al. 2013. High-resolution global maps of 21st-century forest cover change. Science 342:850-853.
- Key, C. H. & Benson, N. C. 2006. Landscape Assessment: sampling and analysis methods. USDA Forest Service
  RMRS-GTR-164-CD.
- Olofsson, P. et al. 2014. Good practices for estimating area and assessing accuracy of land change. Remote
  Sensing of Environment 148:42-57.
- Matthews, B. W. 1975. Comparison of the predicted and observed secondary structure of T4 phage lysozyme.
  Biochimica et Biophysica Acta 405:442-451.
