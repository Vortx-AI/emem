#!/usr/bin/env python3
"""Test the LIVE site the way a browser and a reader actually meet it.

Why this exists. A week of hand-auditing found bugs that every existing check
was structurally incapable of seeing:

  * All six inline scripts on all 26 /docs/ pages were CSP-blocked. Search,
    theme and the sidebar were dead. The pages still RENDERED, so nothing
    looked wrong, and the Rust guard could not see them because it greps this
    repo's source for `include_str!` and the docs book arrives via
    `include_dir!`.
  * 47 of 203 permalinks on /channel pointed at nothing.
  * One page stated three different REST path counts, none of them the number
    the server actually serves.
  * An editor's note shipped as visible body text under a masthead.
  * Two diagrams referenced by the site 404'd.

Every one of those is invisible to a unit test and obvious to a script that
fetches the site and reads what came back. So this checks OUTPUT, over HTTP,
against the running server:

  routes      every known route answers, with the content type it claims
  links       every internal href resolves (no 404s, no redirects to 404)
  anchors     every #fragment resolves to an id that exists on its target
  csp         every inline <script> is allowed by the CSP header sent with it
  assets      every <img src> resolves
  weight      no page exceeds its delivered-bytes budget
  counts      numbers pages assert about the API match what the API returns
  residue     no authoring residue, TODO, or placeholder is user-visible

Usage:
    python3 scripts/site_test.py                     # against production
    python3 scripts/site_test.py --base http://127.0.0.1:5051
    python3 scripts/site_test.py --only csp,links    # a subset
    python3 scripts/site_test.py --quiet             # failures only

Exit code is 1 if any check fails, so CI can gate on it.
"""

from __future__ import annotations

import argparse
import base64
import gzip
import hashlib
import json
import re
import ssl
import sys
import urllib.error
import urllib.parse
import urllib.request
from concurrent.futures import ThreadPoolExecutor

# Delivered-bytes budget, gzipped, per page. These are not aspirations: they
# are set just above what each page costs today, so the CHECK fails when a page
# grows rather than after someone notices it is slow. /channel had grown to
# 249 KB gzipped and rising with every note before anyone measured it.
WEIGHT_BUDGET_GZIP = {
    "/": 90_000,
    "/channel": 90_000,
    "default": 60_000,
}

# Pages whose inline scripts a browser must be allowed to run. Anything served
# as HTML is fair game; this list is the crawl seed, not the limit.
SEEDS = [
    "/", "/how-it-works", "/solutions", "/reference", "/verify", "/a2a",
    "/channel", "/collaboration", "/worlds", "/card", "/docs/gallery",
    "/gallery", "/demos", "/demos/ask-the-earth", "/demos/signed-answer",
    "/demos/state-cube", "/demos/find-similar", "/demos/trajectory",
    "/demos/recall-polygon", "/whitepaper", "/docs/", "/docs/diagrams",
]

# Served, useful, and deliberately not HTML. Listed so a future reader does not
# "fix" them into the seed list and get a spurious route failure.
NON_HTML_ROUTES = ["/api", "/openapi.json", "/llms.txt", "/.well-known/mcp.json"]

# Substrings that must never appear in served HTML. Each one shipped at least
# once, which is why it is here rather than in a style guide.
RESIDUE = [
    "same change at",
    "TODO:",
    "FIXME",
    "coming soon",
    "lorem ipsum",
    "PLACEHOLDER",
    "XXX:",
]

# These reproduce signed notes verbatim. A note that quotes "FIXME", or states
# the tool count as it stood the day it was written, is history rather than
# drift, and rewriting it would falsify the record.
VERBATIM_PAGES = {"/channel", "/collaboration"}

# The crawl is hundreds of independent GETs. Run sequentially it took over ten
# minutes, and a test nobody has time to run is a test nobody runs.
WORKERS = 12

# A page that does not answer in this many seconds is a FAILURE, not something
# to wait for. Unbounded patience is why the first version of this test could
# not finish: one slow route held a worker for 45 seconds and the suite never
# reached a verdict. Bounding it makes the whole run worst-case predictable and
# turns "slow" into a result instead of a hang.
REQUEST_TIMEOUT_S = 10

# Endpoints that materialise a fact from a third-party source on a COLD cell,
# and are therefore slow exactly once before the value is stored and signed.
# Measured: /v1/soil?place=Altamira takes 14s cold and 0.7s warm. These are
# linked from /solutions as "try it" examples, so a reader really can wait that
# long, and the honest handling is a longer allowance with the reason attached
# rather than a number nudged until the test stops complaining. A page that is
# slow for any other reason still fails at REQUEST_TIMEOUT_S.
COLD_FETCH_ALLOWANCE_S = 45
COLD_FETCH_PREFIXES = ("/v1/soil", "/v1/forest", "/v1/elevation", "/v1/water",
                       "/v1/weather", "/v1/ndvi")

CTX = ssl.create_default_context()
CTX.check_hostname = False
CTX.verify_mode = ssl.CERT_NONE


class Result:
    def __init__(self) -> None:
        self.failures: list[tuple[str, str]] = []
        self.checked = 0

    def fail(self, check: str, detail: str) -> None:
        self.failures.append((check, detail))

    def ok(self, n: int = 1) -> None:
        self.checked += n


def _lower(headers) -> dict:
    """HTTP header names are case-insensitive; a plain dict is not.

    Wrapping them in `dict()` and then asking for "Content-Type" returns None,
    because the wire spells it "content-type". That silently made every page
    look like a non-HTML response, so the CSP check skipped all of them and
    reported success over an empty set. A test that checks nothing passes
    forever, which is the whole reason `--self-check` below exists.
    """
    return {k.lower(): v for k, v in headers.items()}


def get(base: str, path: str, gz: bool = False):
    """Fetch a path. Returns (status, headers, body_text, raw_bytes)."""
    url = base.rstrip("/") + path
    req = urllib.request.Request(url, headers={"User-Agent": "emem-site-test"})
    if gz:
        req.add_header("Accept-Encoding", "gzip")
    timeout = (COLD_FETCH_ALLOWANCE_S
               if path.split("?")[0].startswith(COLD_FETCH_PREFIXES)
               else REQUEST_TIMEOUT_S)
    try:
        with urllib.request.urlopen(req, timeout=timeout, context=CTX) as r:
            raw = r.read()
            body = raw
            if (r.headers.get("Content-Encoding") or "").lower() == "gzip":
                try:
                    body = gzip.decompress(raw)
                except Exception:
                    pass
            return r.status, _lower(r.headers), body.decode("utf8", "replace"), raw
    except urllib.error.HTTPError as e:
        raw = e.read()
        return e.code, _lower(e.headers), raw.decode("utf8", "replace"), raw
    except Exception as e:  # noqa: BLE001 - a network error IS a test failure
        return 0, {}, f"{type(e).__name__}: {e}", b""


# Endpoints that hold the connection open by design. Fetching one to
# completion never returns, so a link checker must recognise them rather than
# time out and call a working feature broken.
STREAMING = ("/v1/memory/sse", "/v1/stream", "/sse")


def is_streaming(path: str) -> bool:
    return any(path.split("?")[0].endswith(sfx) or path.split("?")[0] == sfx
               for sfx in STREAMING)


def is_html(headers: dict) -> bool:
    return "text/html" in (headers.get("content-type") or "")


def check_routes_links_anchors_assets(base: str, res: Result, quiet: bool):
    """Crawl the seeds, then verify every internal link, anchor and image."""
    pages: dict[str, str] = {}
    with ThreadPoolExecutor(max_workers=WORKERS) as pool:
        fetched = list(pool.map(lambda p: (p, get(base, p)), SEEDS))
    for path, (status, headers, body, _) in fetched:
        res.ok()
        if status != 200:
            res.fail("routes", f"{path} -> HTTP {status}")
            continue
        if not is_html(headers):
            res.fail("routes", f"{path} -> not HTML ({headers.get('content-type')})")
            continue
        pages[path] = body
    for path in NON_HTML_ROUTES:
        status, _, _, _ = get(base, path)
        res.ok()
        if status != 200:
            res.fail("routes", f"{path} -> HTTP {status}")

    if len(pages) < len(SEEDS) - 2:
        res.fail("routes", f"only {len(pages)} of {len(SEEDS)} seed pages returned "
                           "HTML; everything downstream is measuring a fraction "
                           "of the site")
    if not quiet:
        print(f"  crawled {len(pages)} pages")

    # Every internal href, deduped, resolved once.
    targets: dict[str, set[str]] = {}
    for path, body in pages.items():
        # Strip <script> first: a JS string like "/verify/' + cid + '" is not a
        # link, and testing it produces a failure that no reader can ever hit.
        markup = re.sub(r"<script\b.*?</script>", " ", body, flags=re.S)
        for href in re.findall(r'href="(/[^"#]*)"', markup):
            href = href.split("#")[0]
            if not href or is_streaming(href):
                continue
            targets.setdefault(href, set()).add(path)
    hrefs = sorted(targets)
    with ThreadPoolExecutor(max_workers=WORKERS) as pool:
        statuses = list(pool.map(lambda h: get(base, h)[0], hrefs))
    seen = dict(zip(hrefs, statuses))
    for href, status in seen.items():
        res.ok()
        if status == 0:
            src = ", ".join(sorted(targets[href])[:3])
            limit = (COLD_FETCH_ALLOWANCE_S
                     if href.split("?")[0].startswith(COLD_FETCH_PREFIXES)
                     else REQUEST_TIMEOUT_S)
            res.fail("links", f"{href} did not answer within {limit}s "
                              f"(linked from {src})")
        elif status >= 400:
            src = ", ".join(sorted(targets[href])[:3])
            res.fail("links", f"{href} -> HTTP {status}  (linked from {src})")
    if len(seen) < 30:
        res.fail("links", f"only {len(seen)} internal links found; the crawl is not "
                          "reaching the site")
    if not quiet:
        print(f"  resolved {len(seen)} distinct internal links")

    # Anchors: #frag must exist as an id on the target page.
    id_cache: dict[str, set[str]] = {}
    for path, body in pages.items():
        id_cache[path] = set(re.findall(r'id="([^"]+)"', body)) | set(
            re.findall(r"id=([A-Za-z0-9_-]+)", body)
        )
    anchor_checks = 0
    for path, body in pages.items():
        for target, frag in re.findall(r'href="(/[^"#]*)?#([^"]+)"', body):
            tgt = target or path
            if tgt not in id_cache:
                status, headers, tbody, _ = get(base, tgt)
                if status != 200 or not is_html(headers):
                    continue
                id_cache[tgt] = set(re.findall(r'id="([^"]+)"', tbody)) | set(
                    re.findall(r"id=([A-Za-z0-9_-]+)", tbody)
                )
            anchor_checks += 1
            res.ok()
            if frag not in id_cache[tgt]:
                res.fail("anchors", f"{path} -> {tgt}#{frag} has no such id")
    if not quiet:
        print(f"  resolved {anchor_checks} anchors")

    # Images.
    imgs: dict[str, set[str]] = {}
    for path, body in pages.items():
        for src in re.findall(r'<img[^>]+src="(/[^"]+)"', body):
            imgs.setdefault(src, set()).add(path)
    srcs = sorted(imgs)
    with ThreadPoolExecutor(max_workers=WORKERS) as pool:
        img_status = list(pool.map(lambda u: get(base, u)[0], srcs))
    for src, status in zip(srcs, img_status):
        res.ok()
        if status >= 400:
            res.fail("assets", f"{src} -> HTTP {status}  (on {sorted(imgs[src])[0]})")
    if not quiet:
        print(f"  resolved {len(imgs)} images")
    return pages


def check_csp(base: str, res: Result, pages: dict[str, str], quiet: bool):
    """Every inline <script> must be hashed into the CSP sent WITH that page.

    This is the check that would have caught the docs outage. It does not care
    how a page got baked; it hashes what arrived and reads the header that
    arrived with it.
    """
    # The docs book is 26 pages that share one bootstrap; sample its index plus
    # a few chapters rather than crawling the whole book on every run.
    extra = ["/docs/", "/docs/intro.html", "/docs/quickstart.html",
             "/docs/benchmarks.html", "/docs/how-emem-compares.html"]
    checked = 0
    targets_csp = list(pages) + extra
    with ThreadPoolExecutor(max_workers=WORKERS) as pool:
        got = list(pool.map(lambda p: (p, get(base, p)), targets_csp))
    for path, (status, headers, body, _) in got:
        if status != 200 or not is_html(headers):
            continue
        csp = headers.get("content-security-policy") or ""
        if not csp:
            res.fail("csp", f"{path} served with NO CSP header")
            res.ok()
            continue
        for m in re.finditer(r"<script(?![^>]*\ssrc=)[^>]*>(.*?)</script>", body, re.S):
            block = m.group(1)
            if not block.strip():
                continue
            digest = base64.b64encode(hashlib.sha256(block.encode()).digest()).decode()
            checked += 1
            res.ok()
            if f"'sha256-{digest}'" not in csp:
                head = " ".join(block.split())[:60]
                res.fail("csp", f"{path} inline script BLOCKED: {head}")
    if checked < 20:
        res.fail("csp", f"only {checked} inline scripts examined; the check is not "
                        "covering the site. Header casing or content-type sniffing "
                        "has regressed, or the crawl came back empty.")
    if not quiet:
        print(f"  hashed {checked} inline scripts against their own CSP header")


def check_weight(base: str, res: Result, pages: dict[str, str], quiet: bool):
    worst = []
    paths = list(pages)
    with ThreadPoolExecutor(max_workers=WORKERS) as pool:
        sizes = list(pool.map(lambda p: len(get(base, p, gz=True)[3]), paths))
    for path, n in zip(paths, sizes):
        budget = WEIGHT_BUDGET_GZIP.get(path, WEIGHT_BUDGET_GZIP["default"])
        res.ok()
        worst.append((n, path))
        if n > budget:
            res.fail("weight", f"{path} is {n:,} B gzipped, budget {budget:,} B")
    if not quiet and worst:
        worst.sort(reverse=True)
        top = ", ".join(f"{p} {n // 1000}KB" for n, p in worst[:3])
        print(f"  heaviest: {top}")


def check_counts(base: str, res: Result, pages: dict[str, str], quiet: bool):
    """Numbers a page asserts about the API must match what the API returns.

    reference.html once stated three different REST path counts on one page and
    none matched the server. A blacklist of known-stale strings cannot catch
    drift to a NEW wrong number; comparing against the live surface can.
    """
    status, _, body, _ = get(base, "/openapi.json")
    if status != 200:
        res.fail("counts", f"/openapi.json -> HTTP {status}")
        return
    try:
        spec = json.loads(body)
    except Exception as e:  # noqa: BLE001
        res.fail("counts", f"/openapi.json did not parse: {e}")
        return
    v1 = sum(1 for p in spec.get("paths", {}) if p.startswith("/v1/"))
    res.ok()

    # Any "<N> ... paths under /v1/*" claim on any page must equal v1.
    pat = re.compile(r"(\d{2,4})\s+(?:documented\s+)?paths?\s+under\s*(?:<[^>]+>)?\s*/v1/\*")
    for path, page in pages.items():
        if path in VERBATIM_PAGES:
            continue
        for m in pat.finditer(page):
            claimed = int(m.group(1))
            res.ok()
            if claimed != v1:
                res.fail("counts", f"{path} claims {claimed} REST paths under /v1/*, server serves {v1}")

    # MCP tool count.
    req = urllib.request.Request(
        base.rstrip("/") + "/mcp/full",
        data=json.dumps({"jsonrpc": "2.0", "id": 1, "method": "tools/list",
                         "params": {"tier": "all"}}).encode(),
        headers={"Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(req, timeout=REQUEST_TIMEOUT_S, context=CTX) as r:
            tools = json.load(r)["result"]["tools"]
    except Exception as e:  # noqa: BLE001
        res.fail("counts", f"tools/list failed: {e}")
        return
    n_tools = len(tools)
    res.ok()
    tool_pat = re.compile(r"(?:all|of)\s+(\d{2,4})\s+(?:tools|by name)")
    for path, page in pages.items():
        if path in VERBATIM_PAGES:
            continue
        for m in tool_pat.finditer(page):
            claimed = int(m.group(1))
            res.ok()
            if claimed != n_tools:
                res.fail("counts", f"{path} claims {claimed} tools, server advertises {n_tools}")
    if not quiet:
        print(f"  server: {v1} REST paths under /v1/*, {n_tools} MCP tools")


def check_residue(base: str, res: Result, pages: dict[str, str], quiet: bool):
    """Authoring residue must never be user-visible.

    Only text OUTSIDE script/style is checked: a note inside a signed message
    quoting the word TODO is content, not residue.
    """
    for path, page in pages.items():
        if path in VERBATIM_PAGES:
            continue
        visible = re.sub(r"<(script|style)\b.*?</\1>", " ", page, flags=re.S)
        visible = re.sub(r"<!--.*?-->", " ", visible, flags=re.S)
        visible = re.sub(r"<[^>]+>", " ", visible)
        res.ok()
        for token in RESIDUE:
            if token in visible:
                idx = visible.index(token)
                snippet = " ".join(visible[max(0, idx - 40):idx + 60].split())
                res.fail("residue", f"{path} shows {token!r}: ...{snippet}...")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--base", default="https://emem.dev")
    ap.add_argument("--only", default="", help="comma-separated subset of checks")
    ap.add_argument("--quiet", action="store_true")
    args = ap.parse_args()
    only = {c.strip() for c in args.only.split(",") if c.strip()}

    def want(name: str) -> bool:
        return not only or name in only

    res = Result()
    print(f"site test against {args.base}", flush=True)

    pages = check_routes_links_anchors_assets(args.base, res, args.quiet) \
        if want("links") or want("routes") or want("anchors") or want("assets") \
        else {p: get(args.base, p)[2] for p in SEEDS}

    if want("csp"):
        check_csp(args.base, res, pages, args.quiet)
    if want("weight"):
        check_weight(args.base, res, pages, args.quiet)
    if want("counts"):
        check_counts(args.base, res, pages, args.quiet)
    if want("residue"):
        check_residue(args.base, res, pages, args.quiet)

    print()
    if not res.failures:
        print(f"PASS  {res.checked} checks, 0 failures")
        return 0
    by_check: dict[str, list[str]] = {}
    for check, detail in res.failures:
        by_check.setdefault(check, []).append(detail)
    for check in sorted(by_check):
        items = by_check[check]
        print(f"FAIL  {check}  ({len(items)})")
        for d in items[:12]:
            print(f"        {d}")
        if len(items) > 12:
            print(f"        ... and {len(items) - 12} more")
    print()
    print(f"FAIL  {res.checked} checks, {len(res.failures)} failures")
    return 1


if __name__ == "__main__":
    sys.exit(main())
