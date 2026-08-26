#!/usr/bin/env python3
"""Text contrast against WCAG 1.4.3, measured in the browser that renders it.

Why this exists in this shape
-----------------------------
This site's computed colours are oklch. `canvas.fillStyle` does NOT normalise
oklch in Chromium: assigning `oklch(0.72 0 0)` hands the same string back, so
reading its three numbers as if they were RGB turns every colour into
near-black and every ratio into exactly 1.00. That reports the whole page as
failing, which reads as a page problem and is really a broken instrument. It
happened here on 2026-08-26.

So colours are PAINTED into a 1x1 canvas and read back as pixels, which forces
the browser to resolve whatever syntax it was handed.

Two controls run before any result is trusted, and the second is the one that
matters: a broken converter returns exactly 1.00 for the oklch pair, so its
bound is "strictly greater than 1.05", which the broken state cannot satisfy.
The first version of that control was written as "must be < 1.5" and 1.00
passed it, so the instrument stayed broken through a run that printed nothing
but failures and looked like a styling problem.

Usage
-----
  python3 scripts/manual/contrast.py --page / --width 390
  python3 scripts/manual/contrast.py --origin http://127.0.0.1:8899 --page /solutions
"""
from __future__ import annotations

import argparse
import sys

MEASURE = r"""
(sel) => {
  const cv = document.createElement('canvas').getContext('2d', { willReadFrequently: true });
  function px(c){
    cv.clearRect(0,0,1,1);
    cv.fillStyle = '#000'; cv.fillStyle = c;        // invalid input stays #000
    cv.fillRect(0,0,1,1);
    const d = cv.getImageData(0,0,1,1).data;
    return [d[0], d[1], d[2], d[3]/255];
  }
  const lum = c => { const s = px(c);
    const f = s.slice(0,3).map(v => { v/=255; return v <= .03928 ? v/12.92 : Math.pow((v+.055)/1.055, 2.4); });
    return { l: .2126*f[0] + .7152*f[1] + .0722*f[2], a: s[3] }; };
  function ratio(fg, bg){ const a = lum(fg), b = lum(bg);
    if (a.a < 0.99 || b.a < 0.99) return null;
    return (Math.max(a.l,b.l)+.05) / (Math.min(a.l,b.l)+.05); }
  function bgOf(el){
    for (let p = el; p; p = p.parentElement) {
      const c = getComputedStyle(p).backgroundColor;
      if (c && lum(c).a > 0.99) return c;
    }
    return getComputedStyle(document.body).backgroundColor || '#fff';
  }
  const rows = [], skipped = [];
  document.querySelectorAll(sel).forEach(el => {
    if (el.children.length && !el.childNodes.length) return;
    const t = (el.textContent||'').trim(); if (!t) return;
    const r = el.getBoundingClientRect(); if (!r.width || !r.height) return;
    const cs = getComputedStyle(el);
    if (cs.visibility === 'hidden' || cs.display === 'none') return;
    const size = parseFloat(cs.fontSize), bold = (parseInt(cs.fontWeight)||400) >= 700;
    const large = size >= 24 || (size >= 18.66 && bold);
    const cr = ratio(cs.color, bgOf(el));
    if (cr === null) { skipped.push(t.slice(0,40)); return; }
    rows.push({ t: t.slice(0,44), px: +size.toFixed(1), large,
                cr: +cr.toFixed(2), bound: large ? 3.0 : 4.5 });
  });
  return { rows, skipped,
           ctlMax: +ratio('#ffffff', '#000000').toFixed(2),
           ctlOklch: +ratio('oklch(0.72 0 0)', 'oklch(0.78 0 0)').toFixed(3) };
}"""

DEFAULT_SEL = ("h1, h2, h3, h4, p, li, a, button, code, figcaption, "
               "summary, label, td, th, span")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--origin", default="https://emem.dev")
    ap.add_argument("--page", action="append", required=True)
    ap.add_argument("--width", type=int, default=390)
    ap.add_argument("--height", type=int, default=844)
    ap.add_argument("--selector", default=DEFAULT_SEL)
    ap.add_argument("--scroll", action="store_true",
                    help="step down the page so lazily-drawn sections are measured too")
    a = ap.parse_args()

    try:
        from playwright.sync_api import sync_playwright
    except ImportError:
        print("playwright is not installed; this is a manual tool", file=sys.stderr)
        return 2

    total_fail = 0
    with sync_playwright() as pw:
        br = pw.chromium.launch()
        for theme in ("light", "dark"):
            for path in a.page:
                pg = br.new_page(viewport={"width": a.width, "height": a.height},
                                 color_scheme=theme)
                url = a.origin.rstrip("/") + path
                try:
                    pg.goto(url, wait_until="domcontentloaded", timeout=30000)
                except Exception as e:
                    print(f"[{theme}] {path}: could not load ({type(e).__name__}); "
                          f"NOT checked, which is not the same as clean")
                    total_fail += 1
                    pg.close(); continue
                pg.wait_for_timeout(900)

                positions = [0]
                if a.scroll:
                    h = pg.evaluate("document.documentElement.scrollHeight")
                    positions = list(range(0, max(1, h - a.height + 1), a.height // 2))
                worst, seen, fails = 99.0, 0, []
                ctlMax = ctlOk = None
                for y in positions:
                    pg.evaluate(f"window.scrollTo(0,{y})")
                    pg.wait_for_timeout(350 if a.scroll else 0)
                    res = pg.evaluate(MEASURE, a.selector)
                    ctlMax, ctlOk = res["ctlMax"], res["ctlOklch"]
                    for r in res["rows"]:
                        seen += 1
                        worst = min(worst, r["cr"] - r["bound"])
                        if r["cr"] < r["bound"]:
                            fails.append(r)

                ok = ctlMax is not None and abs(ctlMax - 21.0) < 0.05 and ctlOk > 1.05
                print(f"[{theme}] {path} @{a.width}px  controls: white/black={ctlMax} "
                      f"(want 21.00), oklch={ctlOk} (want >1.05) -> "
                      f"{'ok' if ok else 'INSTRUMENT BROKEN, results below mean nothing'}")
                if not ok:
                    total_fail += 1
                    pg.close(); continue
                print(f"          {seen} text runs, {len(fails)} below bound, "
                      f"worst margin {worst:+.2f}")
                for r in fails[:20]:
                    print(f"    {r['cr']:.2f} < {r['bound']}  {r['px']}px  {r['t']!r}")
                total_fail += len(fails)
                pg.close()
        br.close()

    print(f"\nTOTAL below bound (including instrument failures): {total_fail}")
    # WHAT THIS DID NOT LOOK AT, printed rather than left in the docstring. A
    # count of runs that pass reads as "the page passes"; it means "the elements
    # this selector names, at the positions I visited, pass".
    print(f"  scope: selector {a.selector!r}")
    print(f"         {'stepped down the page' if a.scroll else 'ONE viewport, no scrolling (pass --scroll to sweep)'}"
          f", widths {a.width}px, themes light+dark.")
    print("  NOT covered: text this selector does not name, text behind a closed")
    print("  <details>, and any colour with alpha (those are skipped, not passed).")
    return 1 if total_fail else 0


if __name__ == "__main__":
    raise SystemExit(main())
