#!/usr/bin/env python3
"""Headlines cut in half by the sticky bar AT REST, under BOTH motion prefs.

The symptom, not the property. `scroll-padding-top` was set correctly and the
headline was still clipped, because that property governs scroll-INTO-view and
cannot govern a reader stopping mid-scroll. So this measures pixels of headline
overlapping the bar's rectangle after the scroll has settled, at arbitrary
offsets a reader might actually stop at.

It runs under `prefers-reduced-motion: reduce` as well as `no-preference`,
because a fix placed behind that media query ships the original defect to the
readers least able to tolerate a page that moves.

Usage:
  python3 scripts/manual/headline_clip.py --origin http://127.0.0.1:8899 --page /index.html
"""
from __future__ import annotations

import argparse
import sys

# Twenty-four rests. A seven-step sample reported 0 clipped on a build a denser
# sweep found 3 in: the SAMPLE was clean, not the page. A detector coarser than
# the thing it looks for reports the same green as one that found nothing.
OFFSETS = tuple(range(24))

MEASURE = """() => {
  const bar = document.querySelector('header.sitebar');
  if (!bar) return { nobar: true };
  const b = bar.getBoundingClientRect();
  const out = [];
  let onscreen = 0;
  document.querySelectorAll('main .line').forEach(e => {
    const r = e.getBoundingClientRect();
    if (r.bottom <= 0 || r.top >= innerHeight) return;
    onscreen++;
    const hidden = Math.max(0, Math.min(b.bottom, r.bottom) - Math.max(b.top, r.top));
    if (hidden > 2) out.push([Math.round(hidden), (e.textContent||'').trim().slice(0,34)]);
  });
  // `onscreen` is what makes a zero readable. Without it, "0 clipped" from a
  // page full of headlines and "0 clipped" from a selector that matched nothing
  // print identically, and the broken one looks like the clean one.
  return { barH: Math.round(b.height), clipped: out, onscreen: onscreen,
           total: document.querySelectorAll('main .line').length };
}"""


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--origin", default="https://emem.dev")
    ap.add_argument("--page", default="/")
    ap.add_argument("--widths", default="1440,2560")
    a = ap.parse_args()
    try:
        from playwright.sync_api import sync_playwright
    except ImportError:
        print("playwright is not installed; this is a manual tool", file=sys.stderr)
        return 2

    url = a.origin.rstrip("/") + a.page
    total, seen, vacuous = 0, 0, 0
    with sync_playwright() as pw:
        br = pw.chromium.launch()
        for motion in ("no-preference", "reduce"):
            for w in (int(x) for x in a.widths.split(",")):
                pg = br.new_page(viewport={"width": w, "height": 900},
                                 reduced_motion=motion)
                pg.goto(url, wait_until="domcontentloaded")
                pg.wait_for_timeout(1100)
                worst, hits, barh, examined = 0, 0, 0, 0
                # A WHEEL GESTURE, not window.scrollTo. Scripted jumps do not
                # engage scroll snapping at all, so a measurement built on them
                # reports every snap-fixed position as still broken: this file
                # said "3 of 24 clipped" on a build where a real wheel scroll
                # rests cleanly 14 times out of 14. The instrument was scrolling
                # in a way no reader does.
                pg.mouse.move(w // 2, 500)
                for _ in OFFSETS:
                    pg.mouse.wheel(0, 640)
                    pg.wait_for_timeout(850)          # let the snap settle
                    r = pg.evaluate(MEASURE)
                    if r.get("nobar"):
                        print("  no sticky bar on this page; nothing to measure")
                        pg.close(); br.close(); return 1
                    barh = r["barH"]
                    examined += r.get("onscreen", 0)
                    seen += 1
                    if r["clipped"]:
                        hits += 1
                        worst = max(worst, max(c[0] for c in r["clipped"]))
                        total += 1
                if examined == 0:
                    print(f"  reduced-motion={motion:13} {w:5}px  VACUOUS: no 'main .line'")
                    print("       was on screen at any rest, so this measured nothing. A")
                    print("       zero here would be a fact about the selector, not the page.")
                    vacuous += 1
                else:
                    print(f"  reduced-motion={motion:13} {w:5}px  bar {barh}px  "
                          f"worst clip {worst:3}px  {hits} of {len(OFFSETS)} offsets "
                          f"({examined} headline-sightings examined)")
                pg.close()
        br.close()
    # Reaching nothing is not agreement.
    if seen == 0:
        print("no scroll positions were measured. Undetermined, not clean.")
        return 1
    if vacuous:
        print(f"\n{vacuous} configuration(s) measured no headline at all. That is")
        print("undetermined, not clean: a selector that matches nothing reports the")
        print("same zero as a page with nothing wrong.")
        return 1
    print(f"\n{total} clipped positions out of {seen} measured")
    print(f"  scope: wheel gestures at widths {a.widths}, both motion preferences,")
    print(f"         {len(OFFSETS)} rests each, selector 'main .line'.")
    print("  NOT covered: any headline this selector does not name, and any rest")
    print("  a wheel gesture does not produce. A programmatic scrollTo does not")
    print("  engage scroll snapping, so it is deliberately not used here.")
    return 1 if total else 0


if __name__ == "__main__":
    raise SystemExit(main())
