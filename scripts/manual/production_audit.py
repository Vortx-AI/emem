#!/usr/bin/env python3
"""Every page, swept for the failures a visitor sees before they read a word.

    scripts/manual/production_audit.py                     the live site
    scripts/manual/production_audit.py --origin http://127.0.0.1:5051
    scripts/manual/production_audit.py --page /guard --page /verify
    scripts/manual/production_audit.py --width 1440

Six checks, chosen because each one is invisible from the source and obvious in
a browser:

  overflow    the page scrolls sideways on a phone, and WHICH element is wider
              than the viewport. The single most common way a site that looks
              finished on a laptop looks broken on the device most people open
              it on.
  errors      anything thrown while loading. A page that half-runs still paints.
  contrast    every run of text against its real background -- the first
              ancestor that actually paints one -- as a WCAG ratio. Reported
              against 4.5:1, or 3:1 for large text.
  document    title, meta description, one h1, lang, viewport. The things an
              enterprise reader's tooling reads before a human does.

              The title and description LENGTHS are soft guides, not defects,
              and two findings here are standing decisions rather than work not
              yet done. The whitepaper titles run 73 and 74 characters against a
              70 guide; every way of trimming them drops a load-bearing word
              ("layer", "verifiable"), and search engines truncate around 60
              anyway, so a trim buys a shorter claim and not an untruncated one.
              Seventeen meta descriptions run over 175: none can be cut at a
              sentence boundary, so trimming is rewriting, and rewriting means
              authoring new claims about the protocol in the one place nobody
              proofreads. Both are left long and accurate on purpose. They stay
              in the output rather than being exempted, because a check that
              hides a decision is how the decision gets forgotten.
  images      an <img> with no alt attribute at all. An empty alt is a
              DECISION (decorative) and is not flagged; a missing one is a
              question nobody answered.
  targets     links and buttons smaller than 24px in either direction. Read
              this as UNDERSIZED, not as "fails WCAG 2.5.8": the criterion has a
              SPACING exception, and an undersized target passes if a 24px
              circle centred on it meets no other target. Measured with the
              exception implemented, 189 undersized elements on this site were
              52 inline-exempt, 116 spacing-exempt and 21 real failures. So this
              number over-reports by roughly 5x and must not be quoted as a
              compliance count -- I nearly "fixed" two dozen already-compliant
              targets on the strength of it, which would have moved layout for
              nothing. The spacing pass lives in the other agent's harness; this
              column is a candidate list for it, not a verdict.

Why the self-test is not optional
---------------------------------
A sweep that reports "26 pages clean" is indistinguishable from a sweep whose
selectors stopped matching, and the second is likelier: these detectors read
computed styles and a rename upstream silently empties them. So every run first
loads a page built to FAIL all six, and a page built to pass, and refuses to
report on the real site unless the known-bad trips exactly the checks it should
and the known-good trips none. That is the control this repository has learned
to demand: an audit where everything passes may be a broken audit.
"""
import argparse
import json
import re
import urllib.error
import urllib.request
import sys

try:
    from playwright.sync_api import sync_playwright
except ImportError:
    raise SystemExit("playwright is not installed; this is a manual tool, "
                     "not a gate, and it needs a browser")

# HTML pages only. /spec, /clients and /skills.md are served as text/markdown
# and /mcp as JSON: a browser shows them as plain text in a <pre>, so "no
# title, no h1, no lang" is a true statement about a document that was never
# an HTML page. Auditing them here would report five findings each, forever,
# about a deliberate decision -- which is how a checker teaches people to skim
# past it.
# The hand-written list this file used to carry, kept ONLY as the pages that are
# checked when the sitemap cannot be read. It is twenty, and the site serves
# forty one HTML pages.
#
# That gap was the whole finding. This sweep reported 24 undersized targets and
# read as a survey of the site; a neighbouring responder's accessibility pass
# over all forty one found 940 real failures, 885 of them under /docs, which
# this list has never contained. The twenty pages it enumerates are the twenty
# with almost none of the problem, and a count from them was being quoted as if
# it were the site's.
#
# So the list is DISCOVERED now, from the sitemap this responder publishes,
# filtered to the ones that actually serve HTML. A page added to the site is
# audited without anyone remembering to add it here.
FALLBACK_PAGES = [
    "/", "/how-it-works", "/solutions", "/whitepaper", "/whitepaper/v1",
    "/demos", "/demos/signed-answer", "/demos/state-cube", "/demos/trajectory",
    "/worlds", "/scoreboard", "/gallery", "/verify", "/guard", "/agents",
    "/reference", "/a2a", "/tools", "/card", "/404-does-not-exist",
]


def discovered_pages(origin: str):
    """Every path in /sitemap.xml that serves text/html, plus the 404 probe."""
    try:
        with urllib.request.urlopen(f"{origin}/sitemap.xml", timeout=60) as r:
            xml = r.read().decode("utf-8", "replace")
    except Exception as e:
        print(f"  could not read {origin}/sitemap.xml ({e}); falling back to the "
              f"{len(FALLBACK_PAGES)} written-down pages, which is NOT the site")
        return list(FALLBACK_PAGES)
    locs = re.findall(r"<loc>\s*([^<]+?)\s*</loc>", xml)
    paths = []
    for u in locs:
        path = re.sub(r"^https?://[^/]+", "", u) or "/"
        # JSON, markdown and the event stream are in the sitemap and are not
        # pages; asking a browser to audit them measures nothing.
        if re.search(r"\.(json|md|txt|xml|png|svg)$", path) or path.endswith("/stream"):
            continue
        paths.append(path)
    if "/404-does-not-exist" not in paths:
        paths.append("/404-does-not-exist")

    # And ask each one what it actually serves. Filtering by extension leaves
    # paths that carry no suffix and answer with JSON or a redirect; a browser
    # dutifully loads those and measures nothing, and the page COUNT in the
    # report is then a number about URLs rather than about pages. Cheap: one
    # HEAD each, and it makes the total honest.
    html = []
    for path in sorted(set(paths)):
        req = urllib.request.Request(f"{origin}{path}", method="HEAD",
                                     headers={"User-Agent": "emem-production-audit"})
        try:
            with urllib.request.urlopen(req, timeout=30) as r:
                ct = r.headers.get("content-type", "")
        except urllib.error.HTTPError as e:
            ct = e.headers.get("content-type", "") if e.headers else ""
        except Exception:
            continue
        if "text/html" in ct.lower():
            html.append(path)
    return html

AUDIT_JS = r"""() => {
  const out = {overflow: [], contrast: [], document: [], images: [], targets: []};

  // ---- overflow: name the widest offender, not just the symptom ----
  const de = document.documentElement;
  const vw = de.clientWidth;
  if (de.scrollWidth > vw + 1) {
    const wide = [];
    for (const el of document.querySelectorAll('body *')) {
      const r = el.getBoundingClientRect();
      if (r.width === 0 && r.height === 0) continue;
      const right = r.right + window.scrollX;
      if (right > vw + 1 || r.width > vw + 1) {
        const cs = getComputedStyle(el);
        // An element inside its own horizontal scroller is contained, not
        // overflowing: a wide table in an overflow-x:auto box is CORRECT.
        let contained = false;
        for (let p = el.parentElement; p; p = p.parentElement) {
          const pcs = getComputedStyle(p);
          if (pcs.overflowX === 'auto' || pcs.overflowX === 'scroll' ||
              pcs.overflowX === 'hidden') { contained = true; break; }
        }
        if (contained) continue;
        wide.push({tag: el.tagName.toLowerCase(),
                   cls: (el.className || '').toString().slice(0, 40),
                   id: el.id || '', w: Math.round(r.width),
                   right: Math.round(right)});
      }
    }
    wide.sort((a, b) => b.right - a.right);
    out.overflow = wide.slice(0, 5);
    out.overflowPx = de.scrollWidth - vw;
  }

  // ---- contrast ----
  // getComputedStyle does NOT resolve oklch() to rgb() -- Chrome hands back
  // `oklch(0.45 0.09 145)` verbatim, and this site's entire palette is
  // authored in oklch. The first version of this function ran /[\d.]+/ over
  // that string and read 0.45, 0.09, 145 as r,g,b: every pair came out around
  // 1:1, which is what a contrast checker prints when it is comparing a value
  // with itself. 451 "findings", almost all of them the detector failing.
  //
  // So convert through a canvas, which is the one thing in the platform that
  // takes any CSS colour string and gives back sRGB bytes.
  const cv = document.createElement('canvas');
  cv.width = cv.height = 1;
  const ctx = cv.getContext('2d', {willReadFrequently: true});
  const lum = (c) => {
    if (!c) return null;
    ctx.clearRect(0, 0, 1, 1);
    ctx.fillStyle = '#000';
    ctx.fillStyle = c;                 // ignored if the string does not parse
    if (ctx.fillStyle === '#000' && !/^#0{3,8}$|black|rgb\(0, 0, 0\)/i.test(c))
      return null;                     // unparseable: say so rather than guess
    ctx.fillRect(0, 0, 1, 1);
    const d = ctx.getImageData(0, 0, 1, 1).data;
    const [r, g, b] = [d[0], d[1], d[2]].map(v => {
      const s = v / 255;
      return s <= 0.03928 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4);
    });
    return 0.2126 * r + 0.7152 * g + 0.0722 * b;
  };
  const bgOf = (el) => {
    for (let e = el; e; e = e.parentElement) {
      const c = getComputedStyle(e).backgroundColor;
      const m = c.match(/[\d.]+/g);
      if (m && (m.length < 4 || Number(m[3]) > 0.5)) return c;
    }
    return getComputedStyle(document.body).backgroundColor;
  };
  const seen = new Set();
  for (const el of document.querySelectorAll(
        'p,li,h1,h2,h3,h4,span,a,button,td,th,figcaption,label,code,small,dd,dt')) {
    // Only elements with their OWN text, so a wrapper is not judged on its
    // children's colours.
    let text = '';
    for (const n of el.childNodes) if (n.nodeType === 3) text += n.textContent;
    text = text.trim();
    if (text.length < 3) continue;
    const r = el.getBoundingClientRect();
    if (r.width < 2 || r.height < 2) continue;
    const cs = getComputedStyle(el);
    if (cs.visibility === 'hidden' || cs.opacity === '0') continue;
    const fg = lum(cs.color), bg = lum(bgOf(el));
    if (fg === null || bg === null) continue;
    const ratio = (Math.max(fg, bg) + 0.05) / (Math.min(fg, bg) + 0.05);
    const px = parseFloat(cs.fontSize);
    const bold = parseInt(cs.fontWeight, 10) >= 700;
    const large = px >= 24 || (bold && px >= 18.66);
    const need = large ? 3 : 4.5;
    if (ratio < need) {
      const key = cs.color + '|' + bgOf(el) + '|' + Math.round(px);
      if (seen.has(key)) continue;
      seen.add(key);
      out.contrast.push({text: text.slice(0, 44), ratio: Math.round(ratio * 100) / 100,
                         need, px: Math.round(px), color: cs.color, bg: bgOf(el),
                         sel: el.tagName.toLowerCase() + (el.className ?
                              '.' + el.className.toString().split(/\s+/)[0] : '')});
    }
  }

  // ---- document-level ----
  const title = (document.title || '').trim();
  const desc = document.querySelector('meta[name="description"]');
  const h1s = document.querySelectorAll('h1');
  if (!title) out.document.push('no <title>');
  else if (title.length < 8) out.document.push(`title is ${title.length} chars: ${title}`);
  else if (title.length > 70) out.document.push(`title is ${title.length} chars, over 70`);
  if (!desc) out.document.push('no meta description');
  else if (!desc.content.trim()) out.document.push('meta description is empty');
  else if (desc.content.trim().length > 175)
    out.document.push(`meta description ${desc.content.trim().length} chars, over 175`);
  if (h1s.length === 0) out.document.push('no h1');
  else if (h1s.length > 1) out.document.push(`${h1s.length} h1 elements`);
  if (!document.documentElement.lang) out.document.push('no lang on <html>');
  if (!document.querySelector('meta[name="viewport"]')) out.document.push('no viewport meta');

  // ---- images ----
  for (const img of document.querySelectorAll('img')) {
    if (!img.hasAttribute('alt'))
      out.images.push((img.getAttribute('src') || '(no src)').slice(0, 60));
  }

  // ---- touch targets ----
  for (const el of document.querySelectorAll('a,button,input[type=submit]')) {
    const r = el.getBoundingClientRect();
    if (r.width === 0 || r.height === 0) continue;
    const cs = getComputedStyle(el);
    if (cs.display === 'inline') continue;   // inline links flow with text
    if (r.width < 24 || r.height < 24) {
      out.targets.push({t: (el.textContent || '').trim().slice(0, 26),
                        w: Math.round(r.width), h: Math.round(r.height),
                        // the selector, so 56 findings can be grouped by the
                        // handful of rules that actually cause them
                        sel: el.tagName.toLowerCase()
                             + (el.className ? '.' + el.className.toString().trim().split(/\s+/)[0] : ''),
                        disp: cs.display});
    }
  }
  return out;
}"""

BAD_PAGE = """<html><head><title>x</title></head><body style="background:#fff">
<div style="width:3000px;height:20px;background:#eee">too wide</div>
<p style="color:#bbb;background:#fff;font-size:14px">this text has no contrast at all</p>
<p style="color:oklch(0.88 0.01 85);background:oklch(0.99 0.002 85);font-size:14px">oklch, also far too pale</p>
<img src="/nope.png">
<a href="#" style="display:block;width:10px;height:10px">t</a>
<script>throw new Error("boom")</script>
</body></html>"""

GOOD_PAGE = """<html lang="en"><head><title>A sufficiently long page title</title>
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="description" content="A short, real description of the page."></head>
<body style="background:#fff;margin:0">
<h1 style="color:#111">One heading</h1>
<p style="color:#111;background:#fff;font-size:16px">Text with real contrast.</p>
<p style="color:oklch(0.2 0.005 85);background:oklch(0.99 0.002 85);font-size:16px">oklch with real contrast.</p>
<img src="/x.png" alt="a described image">
<a href="#" style="display:block;width:44px;height:44px;color:#111">ok</a>
</body></html>"""


def audit(pg, url, width):
    errors = []
    pg.on("pageerror", lambda e: errors.append(str(e)[:120]))
    pg.set_viewport_size({"width": width, "height": 900})
    try:
        pg.goto(url, wait_until="load", timeout=90000)
    except Exception as e:
        return {"unreachable": str(e)[:120]}
    pg.wait_for_timeout(2200)
    d = pg.evaluate(AUDIT_JS)
    d["errors"] = errors
    return d


def self_test(browser, width) -> list[str]:
    """The detectors, against a page built to fail and one built to pass."""
    problems = []
    for label, html, expect_fail in (("known-bad", BAD_PAGE, True),
                                     ("known-good", GOOD_PAGE, False)):
        pg = browser.new_page(viewport={"width": width, "height": 900})
        errs = []
        pg.on("pageerror", lambda e: errs.append(str(e)))
        pg.route("**/*", lambda r: r.fulfill(
            status=200, content_type="text/html", body=html)
            if r.request.url.endswith("/st") else r.fulfill(status=404, body=b""))
        pg.goto("http://x/st", wait_until="load", timeout=30000)
        pg.wait_for_timeout(300)
        d = pg.evaluate(AUDIT_JS)
        d["errors"] = errs
        pg.close()
        fired = {k for k in ("overflow", "contrast", "images", "targets", "errors")
                 if d.get(k)}
        if expect_fail:
            for want in ("overflow", "contrast", "images", "targets", "errors"):
                if want not in fired:
                    problems.append(f"{label}: the {want} detector did not fire "
                                    f"on a page built to trip it")
            if len(d.get("document", [])) < 3:
                problems.append(f"{label}: document checks found "
                                f"{len(d.get('document', []))} of the 4 planted defects")
        else:
            if fired:
                problems.append(f"{label}: a clean page tripped {sorted(fired)}")
            if d.get("document"):
                problems.append(f"{label}: a clean page tripped document "
                                f"{d['document']}")
    return problems


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--origin", default="https://emem.dev")
    ap.add_argument("--page", action="append", default=[])
    # TWO VIEWPORTS, because a page can be clean at one and broken at the other
    # and this reported only the narrow one.
    #
    # The /docs sidebar is HIDDEN at mobile width, so its 885 undersized targets
    # were invisible to a 375px sweep: 86 at 1280, 2 at 375, on the same page.
    # Between running one viewport and enumerating twenty pages, this file was
    # reporting 24 findings for a site that had 940.
    ap.add_argument("--width", type=int, action="append",
                    help="viewport width; repeatable. Default: 375 and 1280.")
    ap.add_argument("--json", help="write the full result here")
    args = ap.parse_args()
    pages = args.page or discovered_pages(args.origin)

    with sync_playwright() as pw:
        browser = pw.chromium.launch()

        widths = args.width or [375, 1280]
        broken = self_test(browser, widths[0])
        if broken:
            print("THE DETECTORS ARE NOT WORKING, so nothing below would be found:")
            for b in broken:
                print(f"  {b}")
            browser.close()
            return 1
        print(f"  detectors verified against a known-bad and a known-good page\n")

        results = {}
        for path in pages:
            pg = browser.new_page()
            # Worst of the two: a finding at either width is a finding, and
            # reporting the narrow one alone is what hid the sidebar.
            merged = {}
            for w in widths:
                r = audit(pg, args.origin.rstrip("/") + path, w)
                for k, v in r.items():
                    if isinstance(v, list):
                        merged.setdefault(k, [])
                        for item in v:
                            tagged = f"{item}  [{w}px]" if isinstance(item, str) else item
                            if tagged not in merged[k]:
                                merged[k].append(tagged)
            results[path] = merged
            pg.close()
        browser.close()

    if args.json:
        with open(args.json, "w") as f:
            json.dump(results, f, indent=1)

    total = 0
    unreachable = 0
    print(f"  {len(pages)} pages at {' and '.join(str(w) for w in widths)}px wide, "
          f"{args.origin}\n")
    for path, d in results.items():
        if "unreachable" in d:
            unreachable += 1
            print(f"  {path:24} UNREACHABLE  {d['unreachable']}")
            continue
        n = (len(d["overflow"]) and 1) + len(d["contrast"]) + len(d["document"]) \
            + len(d["images"]) + len(d["targets"]) + len(d["errors"])
        total += n
        if n == 0:
            print(f"  {path:24} clean")
            continue
        print(f"  {path:24} {n} finding(s)")
        if d["overflow"]:
            w = d["overflow"][0]
            print(f"      overflow {d.get('overflowPx')}px past the viewport; widest: "
                  f"<{w['tag']}{('#' + w['id']) if w['id'] else ''}"
                  f"{('.' + w['cls'].split()[0]) if w['cls'] else ''}> {w['w']}px")
        for c in d["contrast"][:4]:
            print(f"      contrast {c['ratio']}:1 (needs {c['need']}) {c['px']}px "
                  f"{c['sel']:22} {c['text'][:34]!r}")
        if len(d["contrast"]) > 4:
            print(f"      contrast ... and {len(d['contrast']) - 4} more colour pairs")
        for x in d["document"]:
            print(f"      document {x}")
        for x in d["images"][:3]:
            print(f"      image with no alt attribute: {x}")
        if len(d["images"]) > 3:
            print(f"      ... and {len(d['images']) - 3} more images with no alt")
        for x in d["targets"][:3]:
            print(f"      target {x['w']}x{x['h']}px {x['t']!r}")
        if len(d["targets"]) > 3:
            print(f"      ... and {len(d['targets']) - 3} more small targets")
        for x in d["errors"][:3]:
            print(f"      js error {x}")

    # Reaching nothing is not passing.
    if unreachable == len(pages):
        print(f"\n  Every page was unreachable. Undetermined, not clean.")
        return 1
    print(f"\n  {total} finding(s) over {len(pages) - unreachable} page(s)"
          + (f", {unreachable} unreachable" if unreachable else ""))
    return 1 if total else 0


if __name__ == "__main__":
    raise SystemExit(main())
