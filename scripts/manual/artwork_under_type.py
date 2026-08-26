"""How much artwork ink falls inside the box of each on-screen text element.

Not "who wins each half", which is what I was measuring: a reader does not
experience halves, they experience whether the drawing runs through the
sentence they are reading. Isolation again (hide the layer, diff) so this is
the artwork and not the type itself.
"""
import argparse
import io
import sys
from playwright.sync_api import sync_playwright
from PIL import Image, ImageChops

SEED = "(function(){var s=987654321;Math.random=function(){s^=s<<13;s^=s>>>17;s^=s<<5;return((s>>>0)/4294967296);};})();"

ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
ap.add_argument("--origin", default="https://emem.dev")
ap.add_argument("--page", default="/")
ap.add_argument("--reach", default="#checkable",
                help="which reach to scroll to before measuring")
ARGS = ap.parse_args()
URL = ARGS.origin.rstrip("/") + ARGS.page

WIDTHS = (768, 1024, 1440, 1920, 2560)

with sync_playwright() as pw:
    br = pw.chromium.launch()
    worst = 0.0
    for w in WIDTHS:
        pg = br.new_page(viewport={"width": w, "height": 900}, color_scheme="light")
        pg.add_init_script(SEED)
        pg.goto(URL, wait_until="domcontentloaded"); pg.wait_for_timeout(1100)
        pg.eval_on_selector(ARGS.reach, "e=>e.scrollIntoView({block:'center'})")
        pg.evaluate("window.scrollBy(0, -window.innerHeight*0.10)")
        pg.wait_for_timeout(1200)
        base = Image.open(io.BytesIO(pg.screenshot())).convert("RGB")
        boxes = pg.evaluate("""() => [...document.querySelectorAll(
            'main .line, main .sub, main .out, .names .side, .reach-names > span')]
          .map(e => { const r = e.getBoundingClientRect();
            return { t:(e.textContent||'').trim().slice(0,34),
                     x:Math.round(r.left), y:Math.round(r.top),
                     w:Math.round(r.width), h:Math.round(r.height) }; })
          .filter(b => b.h > 0 && b.y > -b.h && b.y < innerHeight)""")
        pg.evaluate("""document.querySelectorAll('.scenery').forEach(g=>g.style.display='none');
                       document.querySelectorAll('.bank [data-s]').forEach(n=>n.style.display='none');""")
        pg.wait_for_timeout(500)
        blank = Image.open(io.BytesIO(pg.screenshot())).convert("RGB")
        pg.close()
        diff = ImageChops.difference(base, blank).convert("L")
        rows = []
        for b in boxes:
            x0, y0 = max(0, b["x"]), max(0, b["y"])
            x1, y1 = min(base.size[0], b["x"] + b["w"]), min(base.size[1], b["y"] + b["h"])
            if x1 <= x0 or y1 <= y0: continue
            crop = diff.crop((x0, y0, x1, y1))
            px = list(crop.get_flattened_data())
            if not px: continue
            pct = 100.0 * sum(1 for p in px if p > 6) / len(px)
            rows.append((pct, b["t"]))
        rows.sort(reverse=True)
        top = rows[0] if rows else (0.0, "-")
        worst = max(worst, top[0])
        print(f"  {w:5}  worst {top[0]:5.1f}% of box  {top[1]!r}   ({len(rows)} text boxes)")
    br.close()
    # WHAT THIS DID NOT LOOK AT. The number above is one scroll position per
    # width, and the whole reason this file exists is that a caption below the
    # mobile breakpoint is IN THE FLOW and comes to rest wherever the scroll
    # stops: two of us measured the same page at different rests and got 0.1%
    # and 12.8%. A clean line here means "clean at these rests", and saying
    # otherwise would be the defect this tool was written to catch.
    print(f"\nworst artwork-under-type anywhere: {worst:.1f}%")
    print(f"  scope: {len(WIDTHS)} width(s), ONE scroll position each "
          f"(reach {ARGS.reach}, nudged up 10%), selector: main .line, main .sub,")
    print("         main .out, .names .side, .reach-names > span.")
    print("  NOT covered: any other rest, and any text this selector does not name.")
    print("  A caption in the flow moves with the scroll, so re-run with --reach")
    print("  at other sections before reading a clean line as a clean page.")
