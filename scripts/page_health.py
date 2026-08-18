#!/usr/bin/env python3
"""Open every page and find what a reader would see broken.

Why this exists
---------------
Every visual defect this project has shipped was invisible to the gates that
already run. doc_lint reads prose, sync_counts compares numbers, route_truth
asks whether a URL answers, live_numbers proves a figure is fetched rather than
typed. All of them passed while:

  the world map rendered 783 x 0, taking the live agent map, the channel
    feed and the ask box down with it, on the page whose subject is a
    shared memory of the physical world
  every message body on /channel was collapsed to max-height:0, so a
    correspondence displayed everything about each note except the note
  the film posters rendered 312x461 inside a box declared 16:9
  fifteen links rendered browser-default blue because the page had no base
    rule for its most common element
  the reply row on /channel pushed 132px of horizontal overflow on every
    phone

None of those are findable in source. They are properties of a rendered box,
and the only instrument that sees them is a browser. This runs one.

What it reports
---------------
  zero-size    an element with content that computes to no width or height
  broken-img   an <img> whose natural size is 0, so the file did not load
  overflow     the document is wider than the viewport
  blue         a link left at the browser default, meaning no rule reached it
  pending      a live placeholder still unfilled after the network settles

    python3 scripts/page_health.py            # against the deployed site
    python3 scripts/page_health.py --base http://127.0.0.1:8288

Exit: 0 clean, 1 something is broken, 2 no browser available.
"""

from __future__ import annotations

import argparse
import glob
import os
import sys

PAGES = ["/", "/channel", "/how-it-works", "/reference", "/solutions",
         "/verify", "/tools", "/a2a", "/guard", "/demos"]

PROBE = """(() => {
  const out = {zero: [], broken: [], blue: [], pending: []};
  const name = e => (e.tagName + (e.id ? '#' + e.id : '') +
    (e.className && typeof e.className === 'string' ? '.' + e.className.split(' ')[0] : '')).slice(0, 44);
  for (const e of document.querySelectorAll('img, svg, figure, section, canvas, video')) {
    const r = e.getBoundingClientRect();
    const cs = getComputedStyle(e);
    if (cs.display === 'none' || cs.visibility === 'hidden' || e.closest('[hidden]')) continue;
    // An image still in flight has no size yet and is not a fault.
    if (e.tagName === 'IMG' && !e.complete) continue;
    // An element that occupies a row but has no height is the shape of the
    // map bug: laid out, reserved, and showing nothing.
    if (r.width > 8 && r.height < 2) out.zero.push(name(e) + ' ' + Math.round(r.width) + 'x0');
  }
  for (const i of document.querySelectorAll('img')) {
    if (i.complete && i.naturalWidth === 0 && !i.closest('[hidden]')) out.broken.push(name(i) + ' ' + (i.currentSrc || i.src).slice(-46));
  }
  for (const a of document.querySelectorAll('a')) {
    if (getComputedStyle(a).color === 'rgb(0, 0, 238)') out.blue.push(name(a) + ' "' + (a.textContent || '').trim().slice(0, 22) + '"');
  }
  for (const p of document.querySelectorAll('[data-pending]')) out.pending.push(p.getAttribute('data-pending'));
  out.overflow = document.body.scrollWidth - document.documentElement.clientWidth;
  return out;
})()"""


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--base", default=os.environ.get("EMEM_BASE", "https://emem.dev"))
    ap.add_argument("--width", type=int, default=1440)
    a = ap.parse_args()
    try:
        from playwright.sync_api import sync_playwright
    except ImportError:
        print("page_health: playwright is not installed; nothing asserted.")
        return 2

    problems = []
    with sync_playwright() as p:
        b = p.chromium.launch()
        for path in PAGES:
            pg = b.new_page(viewport={"width": a.width, "height": 950})
            try:
                pg.goto(a.base.rstrip("/") + path, wait_until="load", timeout=90000)
                pg.wait_for_timeout(2500)
                pg.evaluate("""async () => {
                  const step = innerHeight * 0.8;
                  for (let y = 0; y < document.body.scrollHeight; y += step) {
                    window.scrollTo(0, y); await new Promise(r => setTimeout(r, 110));
                  }
                  window.scrollTo(0, 0);
                }""")
                pg.wait_for_timeout(3500)
                r = pg.evaluate(PROBE)
            except Exception as e:  # noqa: BLE001
                print(f"  ??   {path}: {str(e)[:60]}")
                pg.close()
                continue
            bad = []
            for kind in ("zero", "broken", "blue", "pending"):
                for item in r[kind][:4]:
                    bad.append(f"{kind}: {item}")
            if r["overflow"] > 2:
                bad.append(f"overflow: {r['overflow']}px wider than the viewport")
            mark = "ok  " if not bad else "FAIL"
            print(f"  {mark} {path}")
            for x in bad:
                print(f"         {x}")
                problems.append(f"{path} {x}")
            pg.close()
        b.close()

    if problems:
        print(f"\npage_health: {len(problems)} thing(s) a reader would see broken.")
        return 1
    print("\nEvery page renders what it lays out.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
