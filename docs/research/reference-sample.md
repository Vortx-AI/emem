# An independent reference: a probability sample interpreted by people

*Protocol draft, 30 October 2026. It needs the operator's decisions in section 6 before it can start.
Companion to [Method selection](method-selection.md).*

## 1. Why the program needs it

Every score so far is agreement with another automated product. Hansen Global Forest Change and JRC TMF are both made
from Landsat. They agree with each other at an MCC of only 0.39 to 0.48 (companion paper, section 6.5), and Hansen
shares an input with one of the candidates (the Landsat version of the memory detector). Neither dates a loss more
finely than its year, so neither can measure how fast an alert arrives.

Rule of evidence 13 says that only independent references decide. The strongest independent reference is one that
people make, by looking, on a sample drawn by chance so that its estimates generalise.

## 2. Population and sample

- **Population.** Every forest cell of the experiments' forests, at the cell size each experiment reads (400 m now,
  240 m in the finer-cells experiment). A cell is forest when its mean 2000 tree cover is 30% or more.
- **Strata.** (A) cells that any candidate method flagged in the window; (B) cells Hansen marks as lost and no
  candidate flagged; (C) every other forest cell. A rare class is invisible to a simple random sample of a few
  hundred, and these strata keep enough loss in the sample to score it.
- **Allocation.** 150 cells in A, 100 in B and 250 in C, drawn without replacement by a fixed seed after the methods
  and the strata are frozen. 500 cells in all; at a minute and a half per cell for each of two interpreters, about
  25 hours of interpretation.
- **Estimation.** Stratified estimators of each method's precision and recall, and of the area lost, with their
  standard errors (Olofsson et al. 2014; Stehman 2014), so the sample speaks for the whole population and not only
  for the cells drawn.

## 3. Response design: what an interpreter records

For each sampled cell, two interpreters work independently, without seeing any method's output or Hansen's:

| field | values |
|---|---|
| disturbed in the window | yes, no, cannot tell |
| month of first visible change | month, or the interval between the last clear look before and the first after |
| type | clearing, selective logging or degradation, fire, plantation harvest, road or mining, flooding, other |
| area changed inside the cell | estimated hectares, by digitising the changed patch |
| confidence | high, medium, low |
| imagery used | the mosaics and scenes, by date and id |

Imagery: monthly very-high-resolution mosaics where they exist (for the tropics, the NICFI basemaps at about 4.8 m,
if that programme still serves them when the work starts; that is to be checked, not assumed),
with the Sentinel-2 record of the same cell as context. A disagreement between the interpreters is adjudicated by a
third, and the agreement before adjudication is reported.

## 4. How it enters emem

- Each interpretation is a signed note under the interpreter's own key, naming the cell, the imagery and the record
  above. The adjudicated label is a signed note under the adjudicator's key, citing both.
- The sample design (strata, allocation, seed) is signed before the first cell is drawn.
- The lab reads the adjudicated labels as an independent reference, kind 3 under rule 13. The lab's guard lists
  independent products by band; adjudicated sample notes are added to that list by their signer keys, never by a
  key of the fleet.

## 5. What it makes measurable

- Precision and recall of every candidate, and of Hansen and TMF themselves, against an independent reference.
- Detection delay, from each method's first as-of alert to the interpreted month of change.
- Which kinds of change each method finds: clearing against degradation, fire, plantation harvest.
- The size of clearing each method can see, scored on interpreted patches rather than on Hansen's.

## 6. What the operator decides

1. Who interprets, and who adjudicates. Two interpreters and an adjudicator, each with an emem key.
2. Which imagery. The NICFI basemaps, if still served, need an account accepted under their terms of use;
   commercial imagery needs a licence. Either is the operator's to hold.
3. When. Once the selection experiment's later period is read, its forests are a natural first population.

## References

Olofsson, P., Foody, G.M., Herold, M., Stehman, S.V., Woodcock, C.E. & Wulder, M.A. 2014. Good practices for
estimating area and assessing accuracy of land change. Remote Sensing of Environment 148:42-57.

Stehman, S.V. 2014. Estimating area and map accuracy for stratified random sampling when the strata are different
from the map classes. International Journal of Remote Sensing 35:4923-4939.
