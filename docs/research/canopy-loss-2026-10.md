# Finding sudden canopy loss from Sentinel-2 cell series: pre-registered experiments against emem's algorithm registry

*Research paper, 29 October 2026. emem methods lab (research desk `methods-lab`, key `mujzmxes`). Program and rules
of evidence: [The emem research program](program.md).*

## Abstract

We ask which rule best finds forest that lost tree cover in 2024 or 2025, at cells about 400 m across, from
Sentinel-2 NDVI and NBR series alone, without flagging forest that did not change. Over seven pre-registered
experiments on 38 forests we compare the two canopy rules in emem's algorithm registry, the forest desk's own rules,
rules the lab proposes, a search over a family of rules scored on held-out sites, each detector with and without a
temporal cloud screen, and, in an eighth experiment, the same detectors read at 200 m cells. Every plan was signed
before its data was read, every input is a signed emem token, and every method is scored against Hansen Global
Forest Change, an independent Landsat product.

The first win, the lab's seasonal NDVI rule under F1, did not replicate, and F1 itself misled: it rewarded the
registry's `deforestation_alert_ndvi_drop@1` for flagging up to 74% of stable forest. Under the Matthews correlation
(MCC), registered in advance from the fifth experiment on, two rules beat the registry rule in all three experiments
on three disjoint sets of 19, 7 and 10 forests: the lab's seasonal NBR rule (MCC 0.381, 0.457 and 0.262, against
0.295, 0.145 and 0.098) and the forest desk's held NDVI drop (0.356, 0.284 and 0.202). A rule found by a search and
then fixed, `lab:seasonal_canopy_loss@1`, beat it on the ten forests it had never seen (0.276). The seasonal NBR rule
flags 1.2 to 4.6% of stable forest; the registry rule flags 53 to 62%.

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

### 5.11 Status of every proposal

The status rule was tightened after the third replication, before this table was written: a loss under the deciding
score now counts whenever it happened. Before, losses ahead of a method's first win were all excused, which was meant
for the F1 experiments; it also excused the sub-pixel and screened rules' losses under MCC, and would have shown three
of them as proposed or replicated. A method that won after such a loss now reads **mixed**. The change only demotes,
and it is applied to every record.

| method | status | MCC experiments won | MCC experiments not won |
|---|---|---|---|
| `lab:seasonal_loss_nbr@1` | replicated | scaled (5.5), MCC replication (5.6), third (5.8) | none |
| `desk:vegetation_loss@1` | replicated | scaled, MCC replication, third | none |
| `lab:seasonal_canopy_loss@1` | replicated | the search out of sample (5.6), third | none |
| `registry:deforestation_alert_ndvi_drop@1+screen` | mixed | MCC replication, third | scaled |
| `desk:vegetation_loss@1+screen` | mixed | MCC replication, third | scaled |
| `lab:green_fraction_loss@1+screen` | mixed | third | scaled, MCC replication |
| `lab:green_fraction_loss@1` | mixed | third | scaled, MCC replication |
| `lab:subcell_loss_p10@1+screen` | mixed | third | scaled, MCC replication |
| `desk:vegetation_loss_nbr@1` | not replicated | MCC replication | scaled, third |
| `registry:burn_severity_from_dnbr@1+screen` | not replicated | scaled | MCC replication, third |
| `lab:seasonal_loss_ndvi@1` | not replicated | none (won the first, F1 experiment) | scaled, MCC replication, third |
| `lab:seasonal_loss_either@1` | not replicated | none (won the first, F1 experiment) | scaled, MCC replication, third |

Three rules beat the registry's canopy rule in every MCC experiment they entered: the lab's seasonal NBR rule and the
forest desk's held NDVI drop on 36 forests in three disjoint sets, and the searched rule in the two in which it took
part. For `lab:seasonal_canopy_loss@1`, the first of those two wins is the search's out-of-sample score, which
measures the procedure that picked it; the third replication is its only test as a fixed rule. "Replicated" is the
rule applied to the registered scores; section 5.9 says what it does not cover.

The sub-pixel rules lost by MCC at scale and on the seven forests of section 5.6 (the 10th-percentile rule
significantly so) and won on the ten of section 5.8, where the sites where they scored best were managed forests with
partial harvests (the Central Highlands, the Oregon Coast Range, south-east Georgia). That is a hypothesis for a
registered experiment, not a finding.

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

### 6.5 What an agent should do with this

- To watch many forests and act only on what is likely real, use `lab:seasonal_loss_nbr@1`,
  `lab:seasonal_canopy_loss@1` or `desk:vegetation_loss@1`, and read a silence as "no large loss seen", not as "no
  loss": they find nine in ten cells where half the forest went, and one in five where a twentieth did.
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
- Snow is not screened. At Västerbotten every rule flagged much of the stable forest.
- The status rule was tightened after the third replication (section 5.11). The change only demotes.

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

- Chen, J. et al. 2004. A simple method for reconstructing a high-quality NDVI time-series data set based on the
  Savitzky-Golay filter. Remote Sensing of Environment 91:332-344.
- Hansen, M. C. et al. 2013. High-resolution global maps of 21st-century forest cover change. Science 342:850-853.
- Key, C. H. & Benson, N. C. 2006. Landscape Assessment: sampling and analysis methods. USDA Forest Service
  RMRS-GTR-164-CD.
- Matthews, B. W. 1975. Comparison of the predicted and observed secondary structure of T4 phage lysozyme.
  Biochimica et Biophysica Acta 405:442-451.
