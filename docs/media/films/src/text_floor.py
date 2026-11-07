"""Measure the smallest text in each film as a phone shows it.

The film pages are 1080 px wide. A phone plays the film in a column of
PHONE_COLUMN px, so a size of s CSS px shows as s * PHONE_COLUMN / 1080.
The check seeks each page every STEP seconds and measures every text run that
is on screen and at least half opaque. The size includes CSS and SVG
transforms, so a label that is scaled down in a move counts at its drawn size.

Usage, from the repository root (needs Playwright with Chromium):

    python docs/media/films/src/text_floor.py                      # all films
    python docs/media/films/src/text_floor.py emem-one-byte.html   # one film

Exit status 1 when a film has text smaller than FLOOR on the phone.
"""

import sys
from pathlib import Path

from playwright.sync_api import sync_playwright

HERE = Path(__file__).resolve().parent
W, H = 1080, 960
PHONE_COLUMN = 360
FLOOR = 9.9  # the same floor as docs/media/stills/src/text_floor.py
STEP = 0.5

MEASURE = """() => {
  const out = [];
  const walk = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  while (walk.nextNode()) {
    const t = walk.currentNode.textContent.trim();
    if (!t) continue;
    const el = walk.currentNode.parentElement;
    const cs = getComputedStyle(el);
    if (cs.display === 'none' || cs.visibility === 'hidden') continue;
    let alpha = 1;
    for (let e = el; e; e = e.parentElement) alpha *= parseFloat(getComputedStyle(e).opacity);
    if (alpha < 0.5) continue;
    const range = document.createRange();
    range.selectNodeContents(walk.currentNode);
    const r = range.getBoundingClientRect();
    if (r.width === 0 || r.height === 0) continue;
    if (r.right <= 0 || r.bottom <= 0 || r.left >= innerWidth || r.top >= innerHeight) continue;
    let size = parseFloat(cs.fontSize);
    const svgText = el.closest('text');
    if (svgText && svgText.getScreenCTM) {
      const m = svgText.getScreenCTM();
      size *= Math.sqrt(m.a * m.a + m.b * m.b);
    } else {
      const m = new DOMMatrix(getComputedStyle(el).transform === 'none' ? undefined : getComputedStyle(el).transform);
      let s = Math.sqrt(m.a * m.a + m.b * m.b);
      for (let e = el.parentElement; e; e = e.parentElement) {
        const tf = getComputedStyle(e).transform;
        if (tf && tf !== 'none') { const p = new DOMMatrix(tf); s *= Math.sqrt(p.a * p.a + p.b * p.b); }
      }
      size *= s;
    }
    out.push([size, t.slice(0, 40)]);
  }
  return out;
}"""


def measure(pg, page):
    pg.goto(page.as_uri(), wait_until="load")
    pg.evaluate("window.film.ready")
    dur = pg.evaluate("window.film.duration")
    worst = (1e9, "", 0.0)
    t = 0.0
    while t < dur:
        pg.evaluate(f"window.film.seek({t})")
        for size, text in pg.evaluate(MEASURE):
            if size < worst[0]:
                worst = (size, text, t)
        t = round(t + STEP, 3)
    return worst, dur


def main():
    names = sys.argv[1:] or sorted(p.name for p in HERE.glob("emem-*.html"))
    bad = 0
    with sync_playwright() as p:
        b = p.chromium.launch(args=["--font-render-hinting=none"])
        pg = b.new_page(viewport={"width": W, "height": H}, device_scale_factor=1)
        for name in names:
            (css, text, at), dur = measure(pg, HERE / name)
            phone = round(css * PHONE_COLUMN / W, 1)
            flag = "LOW" if phone < FLOOR else "ok"
            bad += flag == "LOW"
            print(f"{flag:3} {name:26} {phone:5} px on a {PHONE_COLUMN} px column "
                  f"({css:.1f} px at {at:.1f} s of {dur} s): {text!r}", flush=True)
        b.close()
    print(f"{len(names)} films measured, {bad} below {FLOOR} px")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
