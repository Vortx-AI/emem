#!/usr/bin/env python3
"""WCAG 2.5.8 failures a visitor actually hits: undersized AND unrescued.

    scripts/manual/tap_targets.py --page / --page /docs/whitepaper-v1.html
    scripts/manual/tap_targets.py --width 1280 --css patch.css --page /demos

production_audit.py has a `targets` column and it is a CANDIDATE LIST: it
counts anything under 24px, which over-reports about five to one because 2.5.8
has two exceptions. This implements both, so the number here is a verdict.

  inline   the target sits in a run of text, so its size is set by
           line-height rather than by a decision anyone made.
  spacing  a 24px circle centred on the target meets no other target and no
           other undersized target's circle. Note the shape of that test: it
           is about CIRCLE CENTRES, not the gap between boxes. Four links 12.8px
           apart passed here, because their centres were 52.8px apart.

Four things this had to learn the hard way, each of which had it reporting a
number that was true of what it measured and false of what it described:

  1. `a[href]` misses mdBook's fold toggle, an <a> with no href and a JS click
     handler. A target is what accepts a pointer action, not what navigates.
  2. `cursor: pointer` INHERITS, so every span inside a clickable row reports
     it. Counting them all turned one target into five: 299 findings where
     there were 32.
  3. An ancestor is not "another target". <main tabindex="-1"> is 920x1329 and
     collided with every small link on the page; a focusable scroll region did
     the same. That was 32 findings where there were 24.
  4. A rect says where an element WOULD be, not where it can be clicked. The
     homepage ticker scrolls its rows out of an overflow:auto parent; they
     still report full-size rects, hundreds of pixels away, over content they
     neither cover nor intercept. A closed <details> does the same by another
     route: it render-skips its content instead of laying it out at zero size,
     so the links inside report full-size rects and nothing is overflowing
     anything for a clip walk to catch. One of those phantom rows was the sole
     blocker behind the last finding on the homepage, and it was stable across
     1.2s, 6s and 12s, so waiting longer would never have exposed it. Every
     rect is now intersected with its clipping ancestors.

What it still cannot see: an element with a click handler, no interactive
semantics, and no cursor:pointer. Nothing in computed style distinguishes that
from a paragraph, so a zero here means "nothing that declares itself a target,
or paints itself as one, fails" -- not "the page conforms".

So it runs a control first, and refuses to report without it: a page built to
fail must fail and a page built to pass must pass. An audit where everything
passes may be a broken audit, and this one returned a confident 0 on a page
whose failures it simply was not collecting.

--css injects a stylesheet after load, so a fix is measured on the page the
site actually serves rather than on a local copy of it. The browser context
bypasses CSP for that; nothing else about the page changes.
"""
import argparse, sys
from playwright.sync_api import sync_playwright

JS_LIB = r"""
  const __collect = () => {
  // `a[href]` alone misses mdBook's fold toggle, which is an <a> with no href
  // and a JS click handler. A target is what accepts a pointer action, not what
  // navigates, so collect bare <a>, [tabindex], and leaf elements the page
  // itself paints as clickable (cursor:pointer).
  // Text is not an identifier: two elements can share it and a truncated
  // textContent points at the wrong one. Carry a real path -- and the same
  // one for measured and for out-of-view targets, so coverage is a set
  // difference over comparable keys rather than two vocabularies.
  const pathOf = e => {
    const seg = [];
    for (let n = e; n && n.nodeType === 1 && seg.length < 5; n = n.parentElement) {
      let t = n.tagName.toLowerCase();
      if (n.id) { seg.unshift('#' + n.id); break; }
      if (n.className && typeof n.className === 'string')
        t += '.' + n.className.trim().split(/\s+/).slice(0, 2).join('.');
      const sibs = n.parentElement ? [...n.parentElement.children].filter(c => c.tagName === n.tagName) : [];
      if (sibs.length > 1) t += ':nth(' + (sibs.indexOf(n) + 1) + ')';
      seg.unshift(t);
    }
    return seg.join(' > ');
  };
  const SEL = 'a,button,input,select,textarea,[role="button"],[role="link"],[onclick],[tabindex],summary';
  const els = new Set();
  for (const e of document.querySelectorAll(SEL)) {
    // tabindex="-1" is programmatic focus (skip-link landing, scroll region),
    // not a pointer target. Counting it made <main tabindex="-1"> a 920x1329
    // "target" that engulfed every small link on the page.
    if (e.getAttribute('tabindex') === '-1' && !e.matches('a,button,input,select,textarea,summary,[role],[onclick]')) continue;
    els.add(e);
  }
  for (const e of document.querySelectorAll('div,span,li,td,p,section,header')) {
    if (getComputedStyle(e).cursor !== 'pointer') continue;
    if (e.closest(SEL) !== null) continue;       // already inside a target
    // A clickable row that CONTAINS a link used to be skipped as "a wrapper,
    // not the target". It is both: the row responds to a click anywhere in it
    // and the link is a separate target inside it. Skipping it meant never
    // measuring the row's own size. Nesting is handled by the outermost rule
    // below, not by refusing containers -- cursor:pointer is not inherited
    // upward from a child <a>, so a div only computes it when something set
    // it on the div or above.
    // cursor:pointer INHERITS, so every span inside a clickable row reports
    // it too. Counting them all turned one target into five and inflated the
    // total ~5x. Keep only the outermost element of each pointer region:
    // the row is the target, its inner spans are its text.
    const p = e.parentElement;
    if (p && getComputedStyle(p).cursor === 'pointer') continue;
    els.add(e);
  }
  const boxes = [];
  // A rect is where an element WOULD be, not where it can be clicked. A
  // ticker row translated out of its overflow:hidden parent still reports a
  // full-size rect at a position where nothing is visible or clickable.
  // Intersect with every clipping ancestor and use what survives.
  //
  // Any non-visible overflow counts, `auto` and `scroll` included, not only
  // `hidden`/`clip`. The objection to that is false negatives: content
  // scrolled out of a scrollable box is reachable by scrolling to it. True,
  // but at the rect it reports WHILE scrolled out it is clickable by nobody,
  // so a spatial collision test must not use it -- this responder's homepage
  // ticker is overflow:auto, and its out-of-view rows were the phantom
  // blocker behind a finding on a link nothing was near. Intersecting keeps
  // whatever is currently in view, so a crowded pair inside a scroll box
  // still collides; only what is out of view right now is dropped. What this
  // does NOT do is measure the other scroll positions, which is the same
  // limitation as measuring one fold state.
  // Two different questions, and conflating them cost a round in each
  // direction. PRESENCE: is any of this element in view right now? Every
  // clipping ancestor counts, scrollable ones included -- a ticker row
  // scrolled out of an overflow:auto box is clickable by nobody at the rect
  // it reports, and using that rect invented a blocker for a link nothing
  // was near. SIZE: how big is the target? Only PERMANENT clipping
  // (hidden/clip) shrinks it. A sidebar link half-scrolled past the edge of
  // a scrollable box is a full-size target one scroll away, and measuring
  // the visible sliver reported 1.3px-tall "failures" on twelve docs pages
  // that were nothing of the kind.
  const CLIPS_FOREVER = /^(hidden|clip)$/;
  const rects = e => {
    const r = e.getBoundingClientRect();
    let vx1 = r.left, vy1 = r.top, vx2 = r.right, vy2 = r.bottom;
    let sx1 = r.left, sy1 = r.top, sx2 = r.right, sy2 = r.bottom;
    for (let p = e.parentElement; p; p = p.parentElement) {
      const cs = getComputedStyle(p);
      const axes = [cs.overflowX, cs.overflowY];
      if (axes.every(v => v === 'visible')) continue;
      const pr = p.getBoundingClientRect();
      vx1 = Math.max(vx1, pr.left); vy1 = Math.max(vy1, pr.top);
      vx2 = Math.min(vx2, pr.right); vy2 = Math.min(vy2, pr.bottom);
      if (vx2 <= vx1 || vy2 <= vy1) {
        // WHY it vanished decides whether it is coverable. A scrollable
        // ancestor can be scrolled to bring it back; a collapsed section
        // clipped to zero by an overflow:hidden ancestor cannot, and asking
        // scrollIntoView to fix that spins forever. mdBook's closed chapters
        // are the second kind, and they are most of them.
        const scrollable = p.scrollHeight > p.clientHeight || p.scrollWidth > p.clientWidth;
        return { hiddenBy: scrollable ? 'scroll' : 'state' };
      }
      if (axes.some(v => CLIPS_FOREVER.test(v))) {
        sx1 = Math.max(sx1, pr.left); sy1 = Math.max(sy1, pr.top);
        sx2 = Math.min(sx2, pr.right); sy2 = Math.min(sy2, pr.bottom);
        if (sx2 <= sx1 || sy2 <= sy1) return { hiddenBy: 'state' };
      }
    }
    return { left: sx1, top: sy1, width: sx2 - sx1, height: sy2 - sy1 };
  };
  const visibleRect = rects;
  let offscreen = 0, stateHidden = 0;
  const offscreenEls = [], offscreenPaths = [];
  for (const e of els) {
    const r = visibleRect(e);
    // Not in view at THIS scroll position. Dropping it is right -- it is
    // clickable by nobody where its rect claims to be -- but it is also
    // uncovered, so it gets counted rather than silently discarded. A sweep
    // that bounds its own coverage and does not say so reads as "all clear".
    if (r === null || r.hiddenBy) {
      // Not in view here. Dropping it is right -- it is clickable by nobody
      // where its rect claims to be -- but only the scrollable kind is a
      // coverage gap this sweep can close, so they are counted apart.
      if (r && r.hiddenBy === 'scroll') { offscreen++; offscreenEls.push(e); offscreenPaths.push(pathOf(e)); }
      else { stateHidden++; }
      continue;
    }
    if (r.width <= 0 || r.height <= 0) continue;
    const cs = getComputedStyle(e);
    if (cs.visibility === 'hidden' || cs.display === 'none' || cs.pointerEvents === 'none') continue;
    // A CLOSED <details> render-skips its content rather than laying it out at
    // zero size, so links inside still report full-size rects and neither a
    // size test nor the clip walk sees anything wrong: nothing is overflowing
    // anything. checkVisibility catches this and does NOT catch the clipping
    // case, so both are needed. Left at defaults -- widening it starts
    // dropping things that are genuinely tappable.
    if (typeof e.checkVisibility === 'function' && !e.checkVisibility()) continue;
    if (e.disabled) continue;
    boxes.push({
      el: e,
      contains: null,   // filled below
      w: r.width, h: r.height, x: r.left, y: r.top,
      cx: r.left + r.width / 2, cy: r.top + r.height / 2,
      tag: e.tagName.toLowerCase(),
      cls: (e.getAttribute('class') || '').slice(0, 60),
      txt: (e.textContent || '').trim().replace(/\s+/g, ' ').slice(0, 40),
      path: pathOf(e),
      // inline exception: the target sits in a run of text, so its size is
      // constrained by line-height rather than by a decision anyone made.
      inline: (() => {
        const d = cs.display;
        if (d !== 'inline' && d !== 'inline-block' && d !== 'contents') return false;
        const p = e.parentElement;
        if (!p) return false;
        // a sibling text node with real words next to it
        return [...p.childNodes].some(n => n.nodeType === 3 && n.textContent.trim().length > 1);
      })(),
    });
  }
  // Mark, for each collected target, which other collected targets it
  // contains. Identity is the INDEX in this list, never a property stashed
  // on the element: a stashed one survives into the next measurement of the
  // same page and the second run cannot resolve it.
  boxes.forEach((b, i) => { b.uid = i; });
  for (const b of boxes) {
    const inner = [];
    for (const o of boxes) {
      if (o.uid === b.uid) continue;
      if (b.el.contains(o.el)) inner.push(o.uid);
    }
    b.contains = inner.length ? inner : null;
  }
  // `el` is a live node and cannot cross the serialisation boundary.
  const out = boxes.map(({ el, ...rest }) => rest);
  return { boxes: out, offscreen, stateHidden, offscreenEls, offscreenPaths };
};
"""

# Measure at the current scroll position.
MEASURE = JS_LIB + r"""
() => { const { boxes, offscreen, stateHidden, offscreenPaths } = __collect();
        return { boxes, offscreen, stateHidden, offscreenPaths }; }
"""

# Bring one currently-uncovered target into view and say which one, so the
# caller can tell progress from a stall. Returns null when everything has been
# seen at least once.
# Every scrollable container that clips a target, with its scroll range.
# Enumerating containers beats targeting elements: scrollIntoView did nothing
# on mdbook-sidebar-scrollbox (scrollTop stayed 0 through every round) and
# walking up from an element to "its" scroller picked one that would not move.
# A container either scrolls or it does not, and that is checkable.
SCROLLERS = JS_LIB + r"""
() => {
  const { offscreenEls } = __collect();
  const set = new Map();
  for (const e of offscreenEls) {
    for (let p = e.parentElement; p; p = p.parentElement) {
      const cs = getComputedStyle(p);
      if (cs.overflowX === 'visible' && cs.overflowY === 'visible') continue;
      if (p.scrollHeight <= p.clientHeight && p.scrollWidth <= p.clientWidth) continue;
      if (!p.__tapScroller) p.__tapScroller = 'sc' + set.size;
      set.set(p.__tapScroller, {
        id: p.__tapScroller,
        maxTop: p.scrollHeight - p.clientHeight,
        maxLeft: p.scrollWidth - p.clientWidth,
        step: Math.max(40, Math.floor(p.clientHeight / 2)),
      });
      break;
    }
  }
  return [...set.values()];
}
"""

# Put one container at one position. Returns what it actually reached, so a
# container that refuses to move is visible as a stall rather than assumed.
SET_SCROLL = JS_LIB + r"""
([id, top]) => {
  const p = [...document.querySelectorAll('*')].find(x => x.__tapScroller === id);
  if (!p) return null;
  p.scrollTop = top;
  return p.scrollTop;
}
"""

def circle_hits_box(cx, cy, b, r=12.0):
    nx = max(b["x"], min(cx, b["x"] + b["w"]))
    ny = max(b["y"], min(cy, b["y"] + b["h"]))
    return (cx - nx) ** 2 + (cy - ny) ** 2 < r * r

def failures(boxes):
    out = []
    for i, b in enumerate(boxes):
        if min(b["w"], b["h"]) >= 24:
            continue
        if b["inline"]:
            continue
        # spacing exception: this target's 24px circle must miss every OTHER
        # target's box, and every other undersized target's circle.
        clear = True
        for j, o in enumerate(boxes):
            if i == j:
                continue
            # An ancestor is not "another target". A link inside a focusable
            # table or a clickable card does not collide with its own
            # container, and treating it as though it does marks every small
            # link in a scroll region as a failure.
            if o["contains"] and b["uid"] in o["contains"]:
                continue
            if circle_hits_box(b["cx"], b["cy"], o):
                clear = False; break
            if min(o["w"], o["h"]) < 24:
                d2 = (b["cx"] - o["cx"]) ** 2 + (b["cy"] - o["cy"]) ** 2
                if d2 < 24 * 24:
                    clear = False; break
        if not clear:
            out.append(b)
    return out

# A page built to FAIL both exceptions: two 16px targets stacked 0px apart, so
# neither is inline and each circle sits inside the other's box.
CONTROL_BAD = """<!doctype html><meta charset=utf-8><body style="margin:0">
<div style="padding:40px"><a href=# style="display:block;width:16px;height:16px">a</a>
<a href=# style="display:block;width:16px;height:16px">b</a></div></body>"""
# And one built to pass: 40px targets, 40px apart.
CONTROL_GOOD = """<!doctype html><meta charset=utf-8><body style="margin:0">
<div style="padding:40px"><a href=# style="display:block;width:40px;height:40px">a</a>
<a href=# style="display:block;width:40px;height:40px;margin-top:40px">b</a></div></body>"""
# One small link ALONE inside a focusable scroll region. The region's box
# contains it, so without the ancestor exclusion this is flagged and nothing
# on the page is wrong. Must report nothing.
CONTROL_REGION_ALONE = """<!doctype html><meta charset=utf-8><body style="margin:0">
<div tabindex=0 style="overflow:auto;width:400px;height:200px;padding:60px">
<a href=# style="display:block;width:16px;height:16px">a</a></div></body>"""
# Two small links CROWDED inside that same region. The ancestor exclusion must
# skip the region and nothing else: these two still block each other. Without
# this control, widening the exclusion from "my own ancestors" to "anything
# sharing a region" looks identical and empties the whole check.
#
# 16px tall with a 4px gap puts the centres 20px apart, inside 24. Written
# first with a 15px gap, which puts them 31px apart and passes -- the same
# gap-versus-centre-distance mistake this checker exists to avoid making, and
# it cost a control that could not fail.
CONTROL_REGION_CROWDED = """<!doctype html><meta charset=utf-8><body style="margin:0">
<div tabindex=0 style="overflow:auto;width:400px;height:200px;padding:60px">
<a href=# style="display:block;width:16px;height:16px">a</a>
<a href=# style="display:block;width:16px;height:16px;margin-top:4px">b</a></div></body>"""


# A scrollable sidebar holds far more links than fit. Measuring one scroll
# position and reporting a number for "the page" covers whatever happened to
# be on screen: 83 of 176 targets on /docs/whitepaper-v1.html were out of view
# at load, and a clean 0 said nothing about them.
SCROLL_POSITIONS_CAP = 40


def sweep_scroll_positions(pg):
    """Findings across every scroll position needed to see every target once.

    A scrollable sidebar holds far more links than fit: 83 of 176 targets on
    /docs/whitepaper-v1.html were out of view at load, and a clean 0 said
    nothing about them.

    Findings are unioned by element path, and each is computed against its
    neighbours AT THE SAME position -- the spacing exception compares a target
    to what is beside it, so mixing positions would compare things that are
    never on screen together.

    Half-steps, not whole ones: a target taller than the step can straddle
    every position and be measured at none of them, and a target caught
    halfway past an edge measures as a sliver. Half a container-height means
    everything lands whole in at least one position.
    """
    found = {}
    seen, never = set(), set()
    off = 0

    def absorb():
        nonlocal off
        m = pg.evaluate(MEASURE)
        for b in failures(m["boxes"]):
            found.setdefault(b["path"], b)
        for b in m["boxes"]:
            seen.add(b["path"])
        never.update(m.get("offscreenPaths", []))
        off = m.get("offscreen", 0)

    absorb()
    if not off:
        return list(found.values()), 0, 1, False

    scrollers = pg.evaluate(SCROLLERS)
    if not scrollers:
        # Targets are out of view and nothing scrolls: not coverable here.
        return list(found.values()), len(never - seen), 1, False

    positions = 1
    capped = False
    for sc in scrollers:
        top, reached_end = 0, False
        while not reached_end:
            got = pg.evaluate(SET_SCROLL, [sc["id"], top])
            pg.wait_for_timeout(80)
            positions += 1
            absorb()
            if got is None or got < top - 1:
                # Asked for a position it would not take: at the end, or it
                # does not really scroll. Either way, stop rather than spin.
                break
            top += sc["step"]
            reached_end = top > sc["maxTop"] + sc["step"]
            if positions >= SCROLL_POSITIONS_CAP:
                capped = True
                break
        pg.evaluate(SET_SCROLL, [sc["id"], 0])
        if capped:
            break
    absorb()
    return list(found.values()), len(never - seen), positions, capped


def run_control(ctx):
    """Refuse to report unless the detector still detects.

    These measurements read computed styles and geometry, and a rename or a
    tightened rule upstream empties them silently. A sweep that reports zero
    is indistinguishable from a sweep whose selectors stopped matching, and
    the second is likelier. So prove both directions before trusting a run.
    """
    pg = ctx.new_page()
    pg.set_viewport_size({"width": 1280, "height": 900})
    ok = True
    for name, html, want_failures in (
        ("known-bad", CONTROL_BAD, True),
        ("known-good", CONTROL_GOOD, False),
        ("lone-target-in-a-focusable-region", CONTROL_REGION_ALONE, False),
        ("crowded-inside-that-same-region", CONTROL_REGION_CROWDED, True),
    ):
        pg.set_content(html)
        pg.wait_for_timeout(100)
        n = len(failures(pg.evaluate(MEASURE)["boxes"]))
        got = n > 0
        print(f"control {name}: {n} real ({'as expected' if got == want_failures else 'WRONG'})")
        ok = ok and got == want_failures
    pg.close()
    return ok


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--origin", default="https://emem.dev")
    ap.add_argument("--page", action="append", required=True)
    ap.add_argument("--width", type=int, default=375)
    ap.add_argument("--css")
    a = ap.parse_args()
    css = open(a.css).read() if a.css else None
    total = 0
    errors = 0
    offscreen_total = 0
    with sync_playwright() as pw:
        br = pw.chromium.launch()
        # bypass_csp so --css can inject: the site serves a hash-based CSP
        # that (correctly) refuses an injected <style>. Only the measuring
        # harness bypasses it; nothing about the page changes.
        ctx = br.new_context(bypass_csp=True)
        if not run_control(ctx):
            br.close()
            raise SystemExit(
                "control failed: the detector no longer detects what it exists "
                "to detect, so nothing it reports about the real site means "
                "anything. Fix the checker before reading its output."
            )
        for path in a.page:
            pg = ctx.new_page()
            pg.set_viewport_size({"width": a.width, "height": 900})
            try:
                # NOT networkidle: the homepage's live tiles keep a request in
                # flight, so idle never arrives and the whole sweep dies on
                # page one. Load, then settle.
                r = pg.goto(a.origin.rstrip("/") + path, wait_until="load", timeout=45000)
                if r is not None and "text/html" not in (r.header_value("content-type") or ""):
                    print(f"{path}  -- not html, skipped")
                    pg.close(); continue
                pg.wait_for_timeout(1200)
                # Measure at the width that was asked for. A page loaded at
                # one width and measured at another is a layout no visitor
                # has ever seen, and it reports findings to match.
                got_w = pg.evaluate("() => window.innerWidth")
                if got_w != a.width:
                    raise RuntimeError(
                        f"viewport is {got_w}px, asked for {a.width}px"
                    )
                if css:
                    pg.add_style_tag(content=css)
                    pg.wait_for_timeout(250)
                f, off, rounds, capped = sweep_scroll_positions(pg)
            except Exception as e:
                print(f"{path}  !! {type(e).__name__}: {str(e).splitlines()[0][:90]}")
                errors += 1
                pg.close(); continue
            total += len(f)
            offscreen_total += off
            note = f"   [{rounds} scroll positions]" if rounds > 1 else ""
            if off:
                note += f"   !! {off} targets NEVER brought into view"
            if capped:
                note += "   !! position cap hit; coverage incomplete"
            print(f"{path}  {len(f)} real{note}")
            for b in f:
                print(f"    {b['w']:.1f} x {b['h']:.1f}  {b['path']}\n        {b['txt']!r}")
            pg.close()
        br.close()
    # A page that failed to load reported no findings, which is not the same
    # as having none. Say so, and never let a run of errors read as a clean
    # sweep.
    print(
        f"TOTAL {total} real"
        + (f"  ({errors} pages NOT MEASURED)" if errors else "")
        + (
            f"  -- {offscreen_total} targets were never brought into view at "
            "any scroll position and are NOT covered by this number"
            if offscreen_total
            else ""
        )
    )
    return total + errors

if __name__ == "__main__":
    sys.exit(0 if main() == 0 else 1)
