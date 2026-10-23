"""Validate the task suite. Exit non-zero on any violation.

(a) every reference.py passes its hidden tests under its own version spec;
(b) every post_change task's v1-era reference (reference_v1.py) FAILS under
    pydantic 2.x, and passes under 1.10.x (control: the failure is the library
    change, not a broken test);
(c) transfer-task hidden tests are not copies of a discovery task's tests.
"""
from __future__ import annotations

import argparse
import difflib
import hashlib
import re
import sys
from collections import Counter

from runner import run_candidate
from taskset import PHASES, load_tasks

V1, V2 = "pydantic==1.10.*", "pydantic==2.*"
EXPECTED_TASKS = 40
SIMILARITY_LIMIT = 0.80


def _norm(src: str) -> list:
    src = re.sub(r"#.*", "", src)
    return re.findall(r"\w+|[^\w\s]", src)


def _tests_text(task) -> str:
    return "\n".join(task.test_sources())


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--timeout", type=float, default=60.0)
    args = ap.parse_args()
    tasks = load_tasks()
    by_id = {t.id: t for t in tasks}
    problems = []
    rows = []

    for t in tasks:
        row = {"id": t.id, "phase": t.phase, "ref": "", "v1_under_v2": "-", "v1_control": "-", "dup": "-"}
        r = run_candidate(t.dir, t.reference, t.version_spec, args.timeout)
        row["ref"] = f"pass({r['passed']})" if r["ok"] else f"FAIL({r['passed']}p/{r['failed']}f/{r['errors']}e)"
        if not r["ok"]:
            problems.append(f"{t.id}: reference fails under {t.version_spec}")

        if t.phase == "post_change":
            if t.reference_v1 is None:
                problems.append(f"{t.id}: post_change task has no reference_v1.py")
            else:
                under2 = run_candidate(t.dir, t.reference_v1, V2, args.timeout)
                row["v1_under_v2"] = "fails" if not under2["ok"] else "PASSES"
                if under2["ok"]:
                    problems.append(f"{t.id}: v1-era reference still passes under pydantic 2 (change is not real)")
                under1 = run_candidate(t.dir, t.reference_v1, V1, args.timeout)
                row["v1_control"] = "pass" if under1["ok"] else "FAIL"
                if not under1["ok"]:
                    problems.append(f"{t.id}: v1-era reference fails under pydantic 1.10 (broken control)")
            if t.version_spec != V2:
                problems.append(f"{t.id}: post_change must be pinned to {V2}")
        elif t.version_spec != V1:
            problems.append(f"{t.id}: {t.phase} must be pinned to {V1}")

        if t.phase == "transfer":
            if not t.discovery_of or any(by_id.get(d) is None or by_id[d].phase != "discovery" for d in t.discovery_of):
                problems.append(f"{t.id}: discovery_of must name discovery tasks")
            worst = 0.0
            mine = _tests_text(t)
            for d in tasks:
                if d.phase != "discovery":
                    continue
                theirs = _tests_text(d)
                if hashlib.sha256(mine.encode()).digest() == hashlib.sha256(theirs.encode()).digest():
                    worst = 1.0
                else:
                    worst = max(worst, difflib.SequenceMatcher(None, _norm(mine), _norm(theirs), autojunk=False).ratio())
            row["dup"] = f"{worst:.2f}"
            if worst >= SIMILARITY_LIMIT:
                problems.append(f"{t.id}: hidden tests are {worst:.2f} similar to a discovery task's tests")
        rows.append(row)

    print(f"{'task':<18}{'phase':<13}{'reference':<18}{'v1ref@v2':<10}{'v1ref@v1':<10}{'max sim'}")
    for r in rows:
        print(f"{r['id']:<18}{r['phase']:<13}{r['ref']:<18}{r['v1_under_v2']:<10}{r['v1_control']:<10}{r['dup']}")

    print("\ntasks per family x phase")
    fams = sorted({t.family for t in tasks})
    c = Counter((t.family, t.phase) for t in tasks)
    print(f"{'family':<14}" + "".join(f"{p:<13}" for p in PHASES) + "total")
    for f in fams:
        print(f"{f:<14}" + "".join(f"{c[(f, p)]:<13}" for p in PHASES) + str(sum(c[(f, p)] for p in PHASES)))
    print(f"{'ALL':<14}" + "".join(f"{sum(c[(f, p)] for f in fams):<13}" for p in PHASES) + str(len(tasks)))

    if len(tasks) != EXPECTED_TASKS:
        problems.append(f"expected {EXPECTED_TASKS} tasks, found {len(tasks)}")
    for f in fams:
        if (c[(f, "discovery")], c[(f, "transfer")], c[(f, "post_change")]) != (2, 2, 1):
            problems.append(f"family {f}: expected 2 discovery, 2 transfer, 1 post_change")

    if problems:
        print("\nVIOLATIONS")
        for p in problems:
            print(" -", p)
        return 1
    print(f"\nOK: {len(tasks)} tasks valid; every post_change v1-era reference fails under pydantic 2.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
