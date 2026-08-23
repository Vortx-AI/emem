"""Drive the album in a real browser, against a captured real response.

    python3 scripts/manual/album_interaction.py

NOT IN CI: it needs playwright and a chromium download, which the workflow does
not carry. Run it by hand after touching the album. It is the only test here
that drives the page rather than reading its markup, and it has already earned
that -- it showed "27 personseen 6 min ago" running together in a caption that
every markup assertion called correct.

The API fixture is `cards-real.json`, taken verbatim from the live
/v1/perception/cards a minute before this ran -- derived from what the route
produces, not invented, which is the difference between a fixture that proves
the feature and one that proves my model of it. Images are stubbed with one real
thumbnail so twelve of them do not turn a two-second test into a two-minute one.
"""
import json, pathlib, sys
from playwright.sync_api import sync_playwright

HERE = str(pathlib.Path(__file__).resolve().parent)
PAGE = open("/home/ubuntu/emem/web/index.html").read()
CARDS = open(f"{HERE}/cards-fixture.json", "rb").read()
# One real thumbnail, so twelve image loads do not make this a two-minute test.
THUMB = open(f"{HERE}/thumb-fixture.png", "rb").read()

def run():
    with sync_playwright() as pw:
        b = pw.chromium.launch()
        pg = b.new_page(viewport={"width": 1400, "height": 950})
        def route(r):
            u = r.request.url
            if u.endswith("/local-index"):
                r.fulfill(status=200, content_type="text/html", body=PAGE)
            elif "/v1/perception/cards" in u and "thumb" not in u and ".svg" not in u and "story" not in u:
                r.fulfill(status=200, content_type="application/json", body=CARDS)
            elif "thumb.png" in u or ".png" in u or ".svg" in u:
                r.fulfill(status=200, content_type="image/png", body=THUMB)
            else:
                r.fulfill(status=404, body=b"{}")
        pg.route("**/*", route)
        errs = []
        pg.on("pageerror", lambda e: errs.append(str(e)[:140]))
        pg.goto("http://x/local-index", wait_until="domcontentloaded", timeout=45000)
        pg.locator("#album-split").scroll_into_view_if_needed()
        pg.wait_for_selector(".album-card", timeout=30000)
        pg.wait_for_timeout(1500)

        n = pg.locator(".album-card").count()
        cols = pg.eval_on_selector("#album-split", "e => getComputedStyle(e).gridTemplateColumns")
        names = pg.eval_on_selector_all(".album-card b", "els => els.map(e => e.textContent)")
        sel = lambda: pg.eval_on_selector_all(".album-card",
            "els => els.map(e => e.getAttribute('aria-current')).indexOf('true')")
        print(f"  cards          : {n}")
        print(f"  split columns  : {cols}")
        print(f"  order          : {', '.join(names[:4])} ...")
        print(f"  selected at load: index {sel()}, hash {pg.evaluate('location.hash')}")

        pg.locator(".album-card").nth(0).focus()
        pg.keyboard.press("ArrowRight"); pg.wait_for_timeout(400)
        pg.keyboard.press("ArrowRight"); pg.wait_for_timeout(900)
        print(f"  after 2x Right : index {sel()}, hash {pg.evaluate('location.hash')}")
        pg.keyboard.press("End"); pg.wait_for_timeout(900)
        print(f"  after End      : index {sel()} of {n}")
        pg.keyboard.press("Home"); pg.wait_for_timeout(900)
        print(f"  after Home     : index {sel()}")
        pg.keyboard.press("ArrowLeft"); pg.wait_for_timeout(400)
        print(f"  Left at index 0: index {sel()}  (must not wrap)")

        pg.locator("#album-split").screenshot(path=f"{HERE}/album.png")
        print(f"  page errors    : {errs or 'none'}")
        b.close()
        return 0 if (n == 12 and not errs) else 1

sys.exit(run())
