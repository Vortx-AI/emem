#!/usr/bin/env python3
"""Spacing must come from the scale, because a scale nobody uses is decoration.

Why this exists
---------------
tokens.css defines a golden-ratio spacing scale and has for a long time:

    --s-1 .382   --s-2 .618   --s-3 1   --s-4 1.618   --s-5 2.618   --s-6 4.236

Those are powers of phi, and the type scale next to them is built the same way.
Then the pages were written with 1393 hand-picked rem values against 561 from
the scale: 71% ad-hoc, 353 of them on the homepage alone. `.5rem .8rem` next to
`.55rem .7rem` next to `.7rem .8rem`, differences too small to be intentional
and too many to be rhythm.

That is what makes a page feel restless without a reader being able to name it.
Every gap is slightly different, so nothing lines up and the eye finds no
interval to trust. The ratio was chosen once, written down, and then not used.

This does not ask for beauty. It asks that a spacing value be one of the six,
so the page has an interval a reader can learn.

    python3 scripts/spacing_scale.py            # report
    python3 scripts/spacing_scale.py --budget N # fail above N ad-hoc values

Exit: 0 within budget, 1 above it.
"""

from __future__ import annotations

import argparse
import glob
import re
import sys

SCALE = {".382rem", ".618rem", "1rem", "1.618rem", "2.618rem", "4.236rem"}
# Zero, auto and the inherited keywords are not spacing choices.
FREE = {"0", "auto", "inherit", "initial", "unset", "revert"}
PROP = re.compile(r"(?:padding|margin|gap|row-gap|column-gap)(?:-[a-z]+)?:\s*([^;\"}\n]+)")


def audit(path: str) -> tuple[int, int, list[str]]:
    text = open(path, encoding="utf-8").read()
    on = off = 0
    examples: list[str] = []
    for m in PROP.finditer(text):
        parts = [p for p in m.group(1).split() if p.endswith("rem")]
        if not parts:
            continue
        if all(p in SCALE for p in parts):
            on += 1
        else:
            off += 1
            if len(examples) < 3:
                examples.append(m.group(0).strip()[:52])
    return on, off, examples


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--budget", type=int, default=None,
                    help="fail if more than this many ad-hoc values remain")
    a = ap.parse_args()

    total_on = total_off = 0
    rows = []
    for path in sorted(glob.glob("web/*.html")) + sorted(glob.glob("web/*.css")):
        on, off, ex = audit(path)
        if on or off:
            rows.append((path, on, off, ex))
            total_on += on
            total_off += off

    rows.sort(key=lambda r: -r[2])
    for path, on, off, ex in rows[:10]:
        if off:
            print(f"  {path:<28} on-scale {on:>3}   ad-hoc {off:>4}   e.g. {'; '.join(ex)}")
    pct = total_off * 100 // max(1, total_on + total_off)
    print(f"\n  {total_on} values on the phi scale, {total_off} off it ({pct}% ad-hoc)")

    if a.budget is None:
        return 0
    if total_off > a.budget:
        print(f"\nspacing: {total_off} ad-hoc values, budget is {a.budget}.")
        print("  Use --s-1 .382, --s-2 .618, --s-3 1, --s-4 1.618, --s-5 2.618, --s-6 4.236.")
        print("  A scale the pages do not use is decoration, and the restlessness a")
        print("  reader feels but cannot name is every gap being slightly different.")
        return 1
    print(f"  within the budget of {a.budget}.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
