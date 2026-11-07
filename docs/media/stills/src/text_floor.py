"""Measure the smallest text in each still as a phone shows it.

Each still renderer runs as usual, but writes to a temporary file. Before the
screenshot, the page reports every text run with its rendered size in CSS px
(an SVG text size is scaled by its transform to the screen). A phone shows the
page width in a column of PHONE_COLUMN px, so a size of s px shows as
s * PHONE_COLUMN / page width.

Usage, from the repository root (needs Playwright and Pillow):

    python docs/media/stills/src/text_floor.py            # all stills
    python docs/media/stills/src/text_floor.py hero.py    # one renderer

Exit status 1 when a still has text smaller than FLOOR on the phone.
"""

import runpy
import sys
import tempfile
from pathlib import Path

from playwright.sync_api import Page

HERE = Path(__file__).resolve().parent
PHONE_COLUMN = 360
FLOOR = 9.9  # 22 CSS px on a page 800 px wide

# Renderers and the arguments that draw the phone copy, when there is one.
# The 1800 px desktop copies of hero and what-crosses are not shown on a phone.
RUNS = {
    "hero.py": ["--phone"],
    "what_crosses.py": ["--phone"],
}
SKIP = {"check_media.py", "recheck.py", "text_floor.py", "receipt_data.py", "wire_data.py"}

MEASURE = """() => {
  const out = [];
  const walk = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  while (walk.nextNode()) {
    const t = walk.currentNode.textContent.trim();
    if (!t) continue;
    const el = walk.currentNode.parentElement;
    const cs = getComputedStyle(el);
    if (cs.display === 'none' || cs.visibility === 'hidden') continue;
    let size = parseFloat(cs.fontSize);
    const svgText = el.closest('text');
    if (svgText && svgText.getScreenCTM) {
      const m = svgText.getScreenCTM();
      size *= Math.sqrt(m.a * m.a + m.b * m.b);
    }
    out.push([size, t.slice(0, 40)]);
  }
  return [document.documentElement.clientWidth, out];
}"""

found = []
_orig = Page.screenshot


def _measure(self, *a, **k):
    width, runs = self.evaluate(MEASURE)
    found.append((width, runs))
    return _orig(self, *a, **k)


def measure(name):
    found.clear()
    Page.screenshot = _measure
    with tempfile.TemporaryDirectory() as tmp:
        argv = [str(HERE / name), str(Path(tmp) / "out.webp"), *RUNS.get(name, [])]
        saved = sys.argv
        sys.argv = argv
        try:
            runpy.run_path(argv[0], run_name="__main__")
        finally:
            sys.argv = saved
            Page.screenshot = _orig
    width, runs = found[-1]
    scale = PHONE_COLUMN / width
    size, text = min(runs)
    return round(size * scale, 1), round(size, 1), width, text


def main():
    names = sys.argv[1:] or sorted(
        p.name for p in HERE.glob("*.py") if p.name not in SKIP and not p.name.startswith("fetch_")
    )
    bad = 0
    for name in names:
        phone, css, width, text = measure(name)
        flag = "LOW" if phone < FLOOR else "ok"
        bad += flag == "LOW"
        print(f"{flag:3} {name:22} {phone:5} px on a {PHONE_COLUMN} px column "
              f"({css} px on a {width} px page): {text!r}", flush=True)
    print(f"{len(names)} stills measured, {bad} below {FLOOR} px")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
