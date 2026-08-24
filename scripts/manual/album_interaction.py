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

def run_stalled():
    """The failure path, which is the one that never gets exercised.

    The painter serves one request at a time, so "blocked behind something slow"
    is a normal state rather than an outage -- and while it lasts our proxy waits
    out its full forty-second gateway timeout before returning 504. That is
    forty seconds of "Reading the painter's index..." on the second section of
    the front page.

    This hangs the index call and asserts the page gives up on its own deadline
    and says something true.
    """
    import time
    with sync_playwright() as pw:
        b = pw.chromium.launch()
        pg = b.new_page(viewport={"width": 1400, "height": 950})
        pending = []
        def route(r):
            u = r.request.url
            if u.endswith("/local-index"):
                r.fulfill(status=200, content_type="text/html", body=PAGE)
            elif u.endswith(".css"):
                name = u.rsplit("/", 1)[-1]
                try:
                    r.fulfill(status=200, content_type="text/css",
                              body=open(f"/home/ubuntu/emem/web/{name}", "rb").read())
                except OSError:
                    r.fulfill(status=404, body=b"")
            elif "/v1/perception/cards" in u and "thumb" not in u:
                # LEAVE IT PENDING rather than sleeping. `time.sleep(30)` here
                # blocked playwright's own route-handling thread, so the page
                # could not receive anything until it returned and the measured
                # give-up time was 30s rather than 8 -- my test of a blocked
                # single-threaded upstream, blocked by a single-threaded test.
                # Never fulfilling is what a hung upstream actually looks like.
                pending.append(r)
            elif ".png" in u or ".svg" in u:
                r.fulfill(status=200, content_type="image/png", body=THUMB)
            else:
                r.fulfill(status=404, body=b"{}")
        pg.route("**/*", route)
        # t0 BEFORE the scroll that triggers the load. It was after, so the
        # eight seconds elapsed while playwright was still scrolling and the
        # measurement read 0.0s -- a control passing for a reason unrelated to
        # the property, which is the thing this whole suite is about.
        t0 = time.time()
        pg.goto("http://x/local-index", wait_until="domcontentloaded", timeout=45000)
        pg.locator("#album-split").scroll_into_view_if_needed()
        pg.wait_for_selector(".album-retry", timeout=25000)
        waited = time.time() - t0
        msg = pg.eval_on_selector(".album-wait", "e => e.textContent")
        print(f"  gave up after   : {waited:.1f}s (deadline is 8s, gateway is 40s)")
        print(f"  message         : {' '.join(msg.split())[:96]}")
        pg.locator("#album-split").screenshot(path=f"{HERE}/stalled.png")
        # Release the routes left hanging on purpose. Without this playwright
        # prints a CancelledError at teardown, and a test that ends in a
        # traceback is one people stop reading -- the same reason a checker that
        # cries wolf gets switched off.
        for r in pending:
            try:
                r.abort()
            except Exception:
                pass
        b.close()
        # A floor as well as a ceiling: giving up instantly would also satisfy
        # "under fifteen seconds" and would mean the deadline never ran.
        ok = 6.0 < waited < 16.0 and "did not answer" in msg
        if not ok:
            print(f"  FAIL: expected to give up between 6s and 16s, not {waited:.1f}s")
        return 0 if ok else 1


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
            elif u.endswith(".css"):
                # THE PAGE'S OWN STYLESHEETS. Without these the render is the
                # unstyled fallback -- readable, and nothing like what a visitor
                # sees. I nearly judged the page's layout from one.
                name = u.rsplit("/", 1)[-1]
                try:
                    body = open(f"/home/ubuntu/emem/web/{name}", "rb").read()
                    r.fulfill(status=200, content_type="text/css", body=body)
                except OSError:
                    r.fulfill(status=404, body=b"")
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
        # The top of the page, since the album now opens it: the question is no
        # longer "does the split work" but "does landing here make sense".
        pg.evaluate("window.scrollTo(0,0)")
        pg.wait_for_timeout(500)
        pg.screenshot(path=f"{HERE}/top.png")
        # And the seam: does the page's second screen follow from its first.
        pg.evaluate("window.scrollTo(0, window.innerHeight - 80)")
        pg.wait_for_timeout(600)
        pg.screenshot(path=f"{HERE}/fold.png")
        print(f"  page errors    : {errs or 'none'}")
        b.close()
        return 0 if (n == 12 and not errs) else 1

if "--stalled" in sys.argv:
    sys.exit(run_stalled())
sys.exit(run())
