"""Capture computed styles for every rendering element, to prove a CSS change moved nothing.

    python3 scripts/manual/computed_styles.py before.json    # with the old CSS
    ...make the change...
    python3 scripts/manual/computed_styles.py after.json
    python3 scripts/manual/computed_styles.py --diff before.json after.json

NOT IN CI: needs playwright. Run it around any refactor of index.html CSS.

Pixel diffing is the obvious way to prove this and the wrong one: the album
paints live data, so two shots of an unchanged page differ. Computed styles are
exact, data-independent, and are what a cascade change actually moves.

Four things had to be right before it measured the property rather than
something else, and every one of them was wrong first:

  the DOM must be DETERMINISTIC -- serving the album fixture gave 828 elements
    one run and 816 the next, so the index request is refused on purpose
  keys must be PATHS, not ordinals -- an ordinal shifts when the element count
    changes, reporting 690 elements "gone" that had merely renumbered
  the path must index RENDERING siblings -- removing twelve <style> elements
    shifted nth-child for everything after them: 676 paths "changed" with no
    cascade involved
  and the edit that fixed the keys had to actually LAND -- the first attempt
    was a str.replace that matched nothing, silently, and the diff it produced
    looked like a finding
"""
import json, sys
from playwright.sync_api import sync_playwright

PAGE = open("/home/ubuntu/emem/web/index.html").read()
FIX  = "/home/ubuntu/emem/scripts/manual"
CARDS = open(f"{FIX}/cards-fixture.json","rb").read()
THUMB = open(f"{FIX}/thumb-fixture.png","rb").read()
if "--diff" in sys.argv:
    import json as _j
    a = _j.load(open(sys.argv[sys.argv.index("--diff") + 1]))
    b = _j.load(open(sys.argv[sys.argv.index("--diff") + 2]))
    oa, ob = set(a) - set(b), set(b) - set(a)
    diffs = [(k, p, v, b[k].get(p)) for k in set(a) & set(b)
             for p, v in a[k].items() if b[k].get(p) != v]
    print(f"  elements {len(a)} -> {len(b)};  paths only-before {len(oa)}, only-after {len(ob)}")
    print(f"  computed-style differences: {len(diffs)}")
    for k, p, x, y in diffs[:12]:
        print(f"    {k[-46:]:48} {p}: {x!r} -> {y!r}")
    raise SystemExit(0 if not diffs and not oa and not ob else 1)

OUT = sys.argv[1]

PROPS = ["display","position","grid-template-columns","font-size","font-family",
         "color","background-color","margin","padding","border","width","max-width",
         "line-height","letter-spacing","text-align","flex-direction","gap","z-index"]

with sync_playwright() as pw:
    b = pw.chromium.launch(); pg = b.new_page(viewport={"width":1400,"height":950})
    def route(r):
        u = r.request.url
        if u.endswith("/local"): r.fulfill(status=200, content_type="text/html", body=PAGE)
        elif u.endswith(".css"):
            n=u.rsplit("/",1)[-1]
            try: r.fulfill(status=200, content_type="text/css", body=open(f"/home/ubuntu/emem/web/{n}","rb").read())
            except OSError: r.fulfill(status=404, body=b"")
        elif "/v1/perception/cards" in u and "thumb" not in u:
            # 404 ON PURPOSE, so the DOM is DETERMINISTIC.
            # Serving the fixture lets the album build twelve cards plus a
            # strip, and the resulting DOM differed between runs -- 828 elements
            # against 816 -- which shifted every ordinal key and reported 690
            # elements "gone". Differences that were misalignment, not cascade.
            # With the album refusing, the only thing varying between the two
            # captures is the CSS, which is the thing under test.
            r.fulfill(status=404, body=b"{}")
        elif ".png" in u or ".svg" in u: r.fulfill(status=200, content_type="image/png", body=THUMB)
        else: r.fulfill(status=404, body=b"{}")
    pg.route("**/*", route)
    pg.goto("http://x/local", wait_until="load", timeout=60000)
    pg.wait_for_timeout(2500)
    got = pg.evaluate("""(props) => {
      const out = {};
      const els = Array.prototype.filter.call(
        document.querySelectorAll('body *'),
        e => e.tagName !== 'STYLE' && e.tagName !== 'SCRIPT');
      let i = 0;
      for (const el of els) {
        // a stable key: tag + classes + ordinal, so it survives a CSS-only change
        // A DOM PATH, not an ordinal. The ordinal shifts whenever the element
        // count changes -- removing twelve <style> elements moved every key
        // after them -- and the diff then reports hundreds of elements as
        // "gone" that simply renumbered. A path names a position in the tree.
        let path = [], n = el;
        while (n && n !== document.body) {
          const parent = n.parentNode;
          if (!parent) break;
          // Index among RENDERING siblings. <style> and <script> draw nothing,
          // and removing twelve <style> elements from the body shifted the
          // nth-child index of everything after them -- 676 paths "changed"
          // for a reason that has nothing to do with the cascade.
          const sibs = Array.prototype.filter.call(parent.children,
            c => c.tagName !== 'STYLE' && c.tagName !== 'SCRIPT');
          path.unshift(n.tagName.toLowerCase() + ':' + (sibs.indexOf(n) + 1));
          n = parent;
        }
        const key = path.join('>');
        i++;
        const cs = getComputedStyle(el);
        const rec = {};
        for (const p of props) rec[p] = cs.getPropertyValue(p);
        out[key] = rec;
      }
      return out;
    }""", PROPS)
    json.dump(got, open(OUT,"w"), indent=0, sort_keys=True)
    print(f"  captured {len(got)} elements -> {OUT}")
    b.close()
