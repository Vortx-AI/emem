#!/usr/bin/env python3
"""Independently score the agreement-vs-accuracy inversion from a pressure run.

WHAT THIS TESTS
---------------
Whether two models AGREEING is evidence that they are RIGHT. The claim under
test, pre-registered by emem before the run and measured by the
navigatable_worlds agent:

    Under compression, agreement decouples from accuracy. Two models reading
    the same lossily-compacted memory agree with each other far more often
    than either is correct, because the summariser drops individual values and
    keeps salient ones (range endpoints, round numbers), and both readers then
    answer with the same surviving artifact.

If that holds, then "the models agree, so the answer is probably right" is
unsafe exactly where agents share a compacted context, which is most
long-horizon multi-agent work.

WHY THE CONTROL ARM DECIDES EVERYTHING
--------------------------------------
`context16` shows both models all 16 values verbatim. It must score ~1.0. A
first run of this experiment scored the ceiling at 0.000 because every cell had
been given the patch centre's coordinates, so the question was unanswerable and
the models were guessing. The "inversion" in that run was agreement on guesses,
and it pointed the same way as the hypothesis. **A ceiling arm below 0.5 means
the harness is the explanation, not the finding**, and this script refuses to
report an inversion when that happens.

THE ARTIFACT THIS SCRIPT LOOKS FOR
----------------------------------
An inversion can be manufactured by scoring agreement loosely (two models
"roughly agree") while scoring accuracy strictly (exact float equality). So the
same verdict rule is applied to both, at several tolerances. An inversion that
appears only at a loose agreement rule is a scoring artifact. One that survives
strict equality is not.

WHAT IT CANNOT TELL YOU
-----------------------
In the run this was written against, both readers receive a byte-identical
prompt: one note, written by one model, read by both. So it measures CORRELATED
ERROR FROM SHARED MEMORY, which is real and common. It does NOT measure
independent convergent loss, where two agents each compact separately and both
land on the same wrong value. That needs a counterbalanced run and is a
stronger claim.

Usage:
    python3 score_inversion.py /path/to/navigatable_worlds/runs/pressure_lahaul_v2.json
"""
from __future__ import annotations

import json
import math
import re
import sys
from collections import defaultdict

from differential_scorer import numbers_in, question_coords, question_numbers, is_abstention

CEILING_ARM = "context16"
CEILING_MIN = 0.5


def fisher_greater(a: int, b: int, c: int, d: int) -> float:
    """One-sided Fisher exact test: P(agreement this high | same underlying rate).

    Replaces a confidence-interval overlap check, which this script used first and
    which is the wrong instrument. Non-overlapping intervals do imply a
    difference, but OVERLAPPING intervals do not imply the absence of one: the
    test is conservative and reports "not established" for effects that are real.
    It did exactly that here, calling the pressure arm unsupported at p = 0.035.

    Intervals are still printed, because they show the precision of each estimate,
    which a p-value hides. The verdict comes from this.
    """
    from math import comb

    n, r1, c1 = a + b + c + d, a + b, a + c
    return sum(
        comb(c1, x) * comb(n - c1, r1 - x) / comb(n, r1)
        for x in range(a, min(r1, c1) + 1)
    )


def wilson(k: int, n: int, z: float = 1.96) -> tuple[float, float]:
    """Wilson score interval. Normal approximation is useless at k=0, which is
    exactly where the pressure arm sits."""
    if n == 0:
        return (0.0, 0.0)
    p = k / n
    d = 1 + z * z / n
    centre = (p + z * z / (2 * n)) / d
    half = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / d
    return (max(0.0, centre - half), min(1.0, centre + half))


def close(a, b, tol) -> bool:
    if a is None or b is None:
        return False
    try:
        return float(a) == float(b) if tol is None else abs(float(a) - float(b)) <= tol
    except (TypeError, ValueError):
        return False


def load(run_path: str):
    run = json.load(open(run_path))
    side = json.load(open(run_path.replace(".json", ".sidecar.json")))
    return run, side


def split_turn(turn_id: str, arms: list[str]):
    """`q0-compaction_pressure_neutral-gemma` -> (q0, neutral, compaction_pressure)."""
    parts = turn_id.split("-")
    if len(parts) < 2:
        return None
    q, mid = parts[0], parts[1]
    for arm in sorted(arms, key=len, reverse=True):
        if mid.startswith(arm + "_"):
            return q, mid[len(arm) + 1 :], arm
    return None


def ground_truth(run, side) -> dict:
    """Truth comes from the CEILING arm's prompt, which lists every cell with its
    own coordinates. Derived here rather than taken from the harness."""
    truth = {}
    for st in run["steps"]:
        if st.get("arm") != CEILING_ARM:
            continue
        prompt = side.get(st["prompt_cid"], "")
        coords = question_coords(prompt.split("\n")[0])
        if not coords:
            continue
        lat, lon = coords
        m = re.search(rf"cell at {re.escape(lat)},{re.escape(lon)}:[^\d\-]*(-?[\d.]+)", prompt)
        if not m:
            continue
        parts = st["turn_id"].split("-")
        truth[(parts[0], parts[1].replace(CEILING_ARM + "_", ""))] = m.group(1)
    return truth


def collect(run, side, arms):
    """(question, variant, arm) -> {model_family: extracted_value}"""
    cells = defaultdict(dict)
    shared = defaultdict(set)
    for st in run["steps"]:
        arm = st.get("arm", "")
        if arm.endswith("_writer"):
            continue
        key = split_turn(st.get("turn_id", ""), arms)
        if not key:
            continue
        prompt = side.get(st["prompt_cid"], "")
        answer = side.get(st["answer_cid"], "")
        # Abstention is checked FIRST. A refusal that quotes the summary's range
        # contains numbers, and taking the first one turns a model declining to
        # answer into a model asserting -0.14. Two models refusing then score as
        # two models agreeing, which inflates exactly the statistic this script
        # exists to measure.
        family = "gemma" if "gemma" in st.get("model", "").lower() else "qwen"
        if is_abstention(answer):
            cells[key][family] = None
        else:
            nums = numbers_in(answer, exclude=question_numbers(prompt.split("\n")[0]))
            cells[key][family] = nums[0] if nums else None
        shared[key].add(st["prompt_cid"])
    return cells, shared


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    run, side = load(sys.argv[1])
    arms = sorted({s["arm"] for s in run["steps"] if not s.get("arm", "").endswith("_writer")})
    truth = ground_truth(run, side)
    cells, shared = collect(run, side, arms)

    print(f"# independent inversion score: {sys.argv[1]}")
    print(f"# run_cid: {(run.get('run') or {}).get('run_cid')}")
    print(f"# arms: {arms}")
    print()

    # -- the gate. Nothing below this line is interpretable if it fails.
    ck = [k for k in cells if k[2] == CEILING_ARM]
    hit = tot = 0
    for k in ck:
        t = truth.get((k[0], k[1]))
        if t is None:
            continue
        for v in cells[k].values():
            tot += 1
            hit += close(v, t, None)
    ceiling = hit / max(tot, 1)
    print(f"## ceiling ({CEILING_ARM}) = {hit}/{tot} = {ceiling:.3f}")
    if ceiling < CEILING_MIN:
        print(f"\nREFUSING TO REPORT. A ceiling below {CEILING_MIN} means the task was")
        print("unanswerable and the models were guessing. The harness is the")
        print("explanation, not the hypothesis. Fix the instrument and re-run.")
        return 1
    print("   instrument sound, results below are interpretable\n")

    print(f"## same verdict rule applied to BOTH statistics")
    print(f"   {'arm':<22}{'rule':<12}{'accuracy':>10}{'agreement':>11}{'inverted':>10}")
    for arm in arms:
        ks = [k for k in cells if k[2] == arm]
        for label, tol in (("strict", None), ("tol 0.005", 0.005), ("tol 0.05", 0.05)):
            ex = n = ag = pairs = 0
            for k in ks:
                t = truth.get((k[0], k[1]))
                if t is None:
                    continue
                pairs += 1
                g, q = cells[k].get("gemma"), cells[k].get("qwen")
                for v in (g, q):
                    n += 1
                    ex += close(v, t, tol)
                ag += close(g, q, tol)
            acc, agr = ex / max(n, 1), ag / max(pairs, 1)
            flag = "YES" if agr > acc else "no"
            print(f"   {arm:<22}{label:<12}{acc:>10.3f}{agr:>11.3f}{flag:>10}")
        print()

    print("## is it statistically supported? (one-sided Fisher exact; Wilson shown for precision)")
    for arm in arms:
        ks = [k for k in cells if k[2] == arm]
        ex = n = ag = pairs = 0
        for k in ks:
            t = truth.get((k[0], k[1]))
            if t is None:
                continue
            pairs += 1
            g, q = cells[k].get("gemma"), cells[k].get("qwen")
            for v in (g, q):
                n += 1
                ex += close(v, t, None)
            ag += close(g, q, None)
        alo, ahi = wilson(ex, n)
        glo, ghi = wilson(ag, pairs)
        acc, agr = ex / max(n, 1), ag / max(pairs, 1)
        p = fisher_greater(ag, pairs - ag, ex, n - ex)
        if agr <= acc:
            # Not a weak inversion: no inversion at all. For the control arm
            # this is the intended result, and calling it "not established"
            # would read as a failed test rather than a passed one.
            verdict = "no inversion (as a control should)"
        elif p < 0.05:
            verdict = f"inversion supported (Fisher p={p:.3f})"
        else:
            verdict = f"NOT established at this n (Fisher p={p:.3f})"
        print(f"   {arm:<22} acc {ex}/{n} [{alo:.3f},{ahi:.3f}]  "
              f"agr {ag}/{pairs} [{glo:.3f},{ghi:.3f}]  {verdict}")
    print()

    print("## is the note SHARED between readers? (bounds what agreement means)")
    multi = [k for k, v in shared.items() if len(v) == 1 and k[2] != CEILING_ARM]
    print(f"   pairs whose two readers saw a byte-identical prompt: {len(multi)}")
    if multi:
        print("   Agreement here measures two readers of ONE lossy artifact.")
        print("   That is correlated error from shared memory, NOT independent")
        print("   convergent loss. Do not claim the latter from this run.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
