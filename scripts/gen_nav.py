#!/usr/bin/env python3
"""One navigation, generated into every page from one definition.

Why this exists
---------------
The site had SEVEN different navigation bars across 22 pages, and between them
they reached 13 of 142 human-facing routes. Which pages you could get to
depended on which page you were standing on. The homepage's nav was unique and
linked to none of /how-it-works, /solutions, /reference, /demos, /a2a, /verify
or /docs. The main variant, on 16 pages, linked to none of /guard, /worlds,
/tools, /channel, /scoreboard, /gallery or /whitepaper. /guard appeared in
exactly one nav: its own.

That is what happens when a nav is copied. So it is generated now, and a gate
compares every page against this file.

Shape
-----
Four groups, named for what a reader wants to do rather than for how the
project is organised, because the reader does not know how the project is
organised. Each group is a <details> element with a shared name=, which makes
them an exclusive group in HTML: opening one closes the rest, no script
involved. Without name= every <details> is independent and the menus stack on
top of each other, which is what shipped first.

The whole nav works with JavaScript disabled: it opens on click and on Enter
and is in the tab order for free. Six lines of progressive enhancement add the
two things HTML has no answer for, closing on Escape and on a click outside.
A menu a keyboard user cannot dismiss is not finished, and those six lines
change nothing if they never run.

Editing
-------
Add a destination to NAV below and run this. Never edit the nav in a page: the
gate regenerates and compares, so a hand edit is reported as drift.

Usage
-----
  python3 scripts/gen_nav.py --check      report drift, write nothing
  python3 scripts/gen_nav.py              rewrite every page's nav
"""

from __future__ import annotations

import argparse
import glob
import os
import re
import sys

START = "<!--nav:start-->"
END = "<!--nav:end-->"

# label, href, one-line note shown under the label.
NAV = [
    ("Understand", [
        ("How it works", "/how-it-works", "the address, the fact, the receipt"),
        ("Solutions", "/solutions", "four agents already running on it"),
        ("Whitepaper", "/whitepaper", "the long form, with the proofs"),
        ("The spec", "/spec", "wire format, preimages, canonical bytes"),
    ]),
    ("See it run", [
        ("Demos", "/demos", "eight, each a real call against production"),
        ("Walk a world", "/worlds", "signed terrain you can fly through"),
        ("The channel", "/channel", "agents talking, every message addressable"),
        ("Scoreboard", "/scoreboard", "the live benchmark, two heats"),
        ("Gallery", "/gallery", "what the record looks like rendered"),
    ]),
    ("Verify", [
        ("Check a receipt", "/verify", "paste any token, the proof runs in your browser"),
        ("emem-guard", "/guard", "allow or deny, signed, with a reason"),
        ("Transparency log", "/v1/log/sth", "pin a signed head, prove it only ever grew"),
        ("Who writes here", "/agents", "every attester, discovered not curated"),
    ]),
    ("Build", [
        ("Reference", "/reference", "the read surface, endpoint by endpoint"),
        ("OpenAPI", "/openapi.json", "the machine contract"),
        ("Skills", "/skills.md", "runnable procedures for an agent"),
        ("Clients", "/clients", "connect Claude, Cursor, or your own"),
        ("A2A", "/a2a", "how two agents share one memory"),
        ("Docs", "/docs", "the book"),
    ]),
]

# Who each surface is for, and what it will feel like when you open it.
#
# The site mixes three readerships with no marking at all. /tools is 14,000
# words and 321 code blocks; /reference is nine tables; /llms.txt and
# /openapi.json are machine formats a person can open by accident. A leader
# deciding whether to adopt lands on a wall of JSON and concludes the project
# is not for them, and a developer looking for the wire format wades through
# positioning. Neither is a content problem: both pages are good at their job.
# Nobody was told whose job it was.
#
# Three audiences, named for the reader rather than for us, and a fourth for
# the pages that genuinely suit anyone.
AUDIENCE = {
    "/":            ("anyone",    "memory two agents can agree on"),
    "/solutions":   ("leaders",   "what it is used for, and what holds up under audit"),
    "/whitepaper":  ("leaders",   "the long argument, with the proofs"),
    "/spec":        ("developers","the wire format, byte by byte"),
    "/reference":   ("developers","every endpoint, with worked calls"),
    "/docs":        ("developers","the book"),
    "/guard":       ("developers","a server you run, with the commands to run it"),
    "/verify":      ("developers","paste a token, watch the proof run"),
    "/demos":       ("anyone",    "eight things you can click"),
    "/tools":       ("agents",    "the full tool registry, generated, long"),
    "/a2a":         ("agents",    "how two agents agree before they start"),
    "/agents":      ("agents",    "who writes here, as data"),
    "/worlds":      ("anyone",    "signed terrain you can fly through"),
    "/gallery":     ("anyone",    "the record, rendered"),
    "/channel":     ("anyone",    "agents talking, in public"),
    "/scoreboard":  ("anyone",    "the benchmark, live"),
    "/whitepaper-v1": ("leaders", "superseded, kept as it shipped"),
    "/how-it-works":  ("anyone",    "the address, the fact, the receipt, in order"),
    "/404":           ("anyone",    "the page you asked for is not here"),
    # The demos are the one place a reader of any kind can just press a button,
    # so none of them is marked for a specialist.
    "/demos/ask-the-earth":   ("anyone", "ask a question, get a signed answer"),
    "/demos/find-similar":    ("anyone", "find places that resemble this one"),
    "/demos/recall-polygon":  ("anyone", "read a whole area at once"),
    "/demos/signed-answer":   ("anyone", "watch a receipt get built in four steps"),
    "/demos/state-cube":      ("anyone", "the full state vector of one place"),
    "/demos/trajectory":      ("anyone", "one cell, seven steps through its history"),
}

# Rows above that no page in web/ is served at, so render() never reads them.
#
# The gate has always checked that every rendered page HAS a row. It never
# checked the other direction, and a row nothing renders is the same defect one
# table over: it reads as a decision about a page that was classified, when in
# fact that page carries no audience strip at all. Measured against
# https://emem.dev on 2026-08-10: /, /reference and every other web/*.html page
# serve `class="audience aud-…"`; /agents, /docs and /spec serve none.
#
# These three stay listed because the classification is the right one and this
# is where a reader looks for it. They are named here so the row is understood
# as an intent for a responder-rendered route rather than as a claim that the
# strip is on the page.
#   /agents  built by the responder from the attester table, not from web/.
#   /docs    mdbook output, baked with include_dir!; it carries mdbook chrome.
#   /spec    served from the spec source, not from a page in web/.
# /api-redoc and /arcade were also unread, but for a different reason: both
# pages exist in web/ and both are in SKIP below, so their rows described a
# strip that was deliberately never going to be rendered. Removed, since a row
# for a skipped page is indistinguishable from a row for a page nobody checked.
UNRENDERED_AUDIENCE = {"/agents", "/docs", "/spec"}

# What the chip says about the reader it names.
AUDIENCE_NOTE = {
    "agents":     "machine-first: expect JSON, long listings, and no hand-holding",
    "developers": "expect commands you can paste and formats you can implement",
    "leaders":    "expect the argument and the evidence, not the wire format",
    "anyone":     "no prior knowledge assumed",
}

GITHUB = "https://github.com/Vortx-AI/emem"


def render(current: str) -> str:
    """`current` is the path of the page being rendered, for the on state."""
    out = [START, '<header class="sitebar"><nav class="sitebar-in" aria-label="Site">',
           '<a href="/" class="brand"><img src="/vortxgola.gif" alt="">emem</a>']
    for group, items in NAV:
        hit = any(h == current for _, h, _ in items)
        # name= makes these an exclusive group: opening one closes the rest,
        # in HTML, with no script. Without it every <details> is independent
        # and the menus stack on top of each other.
        out.append(f'<details class="navgrp{" on" if hit else ""}" name="sitenav">')
        out.append(f'<summary>{group}</summary>')
        out.append('<div class="navmenu">')
        for label, href, note in items:
            mark = ' aria-current="page"' if href == current else ''
            out.append(f'<a href="{href}"{mark}><b>{label}</b><span>{note}</span></a>')
        out.append('</div></details>')
    out.append('<span class="sitebar-gap"></span>')
    out.append('<a class="navplain" href="/mcp">MCP</a>')
    out.append(f'<a class="navplain" href="{GITHUB}" rel="noopener">GitHub</a>')
    out.append('</nav></header>')
    # The audience strip. One line, directly under the bar, so a reader knows
    # whose page this is before they start reading it.
    who, what = AUDIENCE.get(current, ("anyone", ""))
    note = AUDIENCE_NOTE.get(who, "")
    out.append(f'<div class="audience aud-{who}">'
               f'<span class="aud-for">for {who}</span>'
               f'<span class="aud-what">{what}</span>'
               f'<span class="aud-note">{note}</span>'
               f'</div>')
    # Progressive enhancement only. Escape and outside-click are the two
    # dismissals <details> has no answer for; everything else is HTML.
    out.append('<script>(function(){'
               'function shut(){document.querySelectorAll(".navgrp[open]")'
               '.forEach(function(d){d.open=false;});}'
               'document.addEventListener("keydown",function(e){'
               'if(e.key==="Escape")shut();});'
               'document.addEventListener("click",function(e){'
               'if(!e.target.closest(".navgrp"))shut();});'
               '})();</script>')
    out.append(END)
    return "\n".join(out)


# The nav a page carries today, in any of its seven shapes.
OLD = re.compile(r'<header class="statusbar">.*?</header>', re.S)
GEN = re.compile(re.escape(START) + r'.*?' + re.escape(END), re.S)

# Which path each file is served at, for the current-page state.
def served_as(name: str) -> str:
    if name == "index.html":
        return "/"
    if name == "demos-index.html":
        return "/demos"
    if name.startswith("demos-"):
        return "/demos/" + name[len("demos-"):-len(".html")]
    return "/" + name[:-len(".html")]


SKIP = {# channel.html was skipped for "owning its markup". It did own it, and it
        # froze: the channel was the only page of twenty-two still carrying the
        # old flat `statusbar` bar with nine links, while every other page had
        # moved to the four-group `sitebar` this file generates. It was missing
        # /worlds, /scoreboard, /gallery, /spec and seven more, and it
        # hand-rolled its own audience strip that nav.css already styles. A
        # generated page can import the generator: build_channel.py now calls
        # render() here the way render_whitepaper.py does, so the markers are
        # present and this gate checks it like any other page.
        "whitepaper-v2.html": "generated by render_whitepaper.py, which calls "
                              "render() here itself and marks /whitepaper as "
                              "the current page rather than /whitepaper-v2",
        "card.html": "a fixed-size share card with no site chrome",
        "api-redoc.html": "hosts a third-party renderer with its own frame",
        # A full-bleed WebGL stage: the globe fills the viewport and the page's
        # own brand mark is its only chrome. The shared nav is a horizontal bar
        # for document pages and would sit on top of the canvas, over the world
        # tags, with nothing to return to. It moved into web/ from a gitignored
        # path on 2026-08-10, which is why this gate can suddenly see it.
        # Skipping it puts the way back on the page itself: the notes panel
        # footer carries an explicit emem.dev link. Checked when this exemption
        # was written, because a skipped page with no link out is a dead end
        # and the nav exists precisely to prevent that.
        "arcade.html": "a full-bleed 3D stage; a document nav bar would sit on "
                       "top of the globe, so it carries its own link home",
        # Not a page anyone browses to. It is the MCP Apps view (SEP-1865) a
        # host renders inside a conversation, sized to a chat pane and loaded
        # under a CSP that blocks every external request, which is why its
        # blake3 and ed25519 are compiled in rather than fetched. A site nav
        # would offer a reader links their host cannot follow. Checked when
        # this exemption was written, because a skipped page with no link out
        # is a dead end: the card renders an "open the offline verifier" link
        # to emem.dev alongside the two commands that re-check it elsewhere.
        "mcp-fact-card.html": "an MCP Apps view rendered inside a host "
                              "conversation, not a page on this site"}


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--check", action="store_true")
    a = ap.parse_args()

    changed, drifted, skipped = [], [], []
    # A SKIP entry for a page that is gone stops being a decision and becomes a
    # comment, and it silently shrinks what this gate covers. Every entry was
    # re-checked on 2026-08-10 against the file it names: whitepaper-v2.html
    # does carry the markers render_whitepaper.py writes (it calls render()
    # here at line 295), card.html and api-redoc.html carry no nav markers and
    # no site chrome, and arcade.html still carries the explicit way back its
    # entry promises (href="/" plus emem.dev links in the notes footer).
    present = {os.path.basename(p) for p in glob.glob("web/*.html")}
    for name in sorted(SKIP):
        if name not in present:
            drifted.append(f"{name} is listed in SKIP but web/{name} does not "
                           f"exist; drop the entry or fix the name.")
    for path in sorted(glob.glob("web/*.html")):
        name = os.path.basename(path)
        if name in SKIP:
            skipped.append(name)
            continue
        s = open(path, encoding="utf-8").read()
        nav = render(served_as(name))
        if GEN.search(s):
            out = GEN.sub(lambda _: nav, s, count=1)
        elif OLD.search(s):
            out = OLD.sub(lambda _: nav, s, count=1)
        else:
            # /tools and /scoreboard were built as bare pages with no site
            # chrome at all, so there is nothing to replace and the nav is
            # inserted instead. A reader who lands on either had no way back
            # except the browser button.
            # /scoreboard has no <body> tag at all: it is a fragment and the
            # browser implies one. Fall back to the first content element.
            m = re.search(r'<body[^>]*>', s)
            if m:
                out = s[:m.end()] + "\n" + nav + s[m.end():]
            else:
                m = re.search(r'\n<(?:main|section|div|header)[ >]', s)
                if not m:
                    drifted.append(f"{name}: nowhere to insert a nav")
                    continue
                out = s[:m.start()] + "\n" + nav + s[m.start():]
        if "/nav.css" not in out:
            out = out.replace('<link rel=stylesheet href="/tokens.css">',
                              '<link rel=stylesheet href="/tokens.css">\n'
                              '<link rel=stylesheet href="/nav.css">', 1)
        if out != s:
            if a.check:
                drifted.append(f"{name}: nav does not match scripts/gen_nav.py")
            else:
                open(path, "w", encoding="utf-8").write(out)
                changed.append(name)

    # Every page the nav reaches must have an audience someone chose. A page
    # that falls through to the "anyone" default has not been classified; it
    # has been forgotten, and "for anyone" is exactly the claim the strip
    # exists to stop the site making by accident.
    unclassified = []
    for path in sorted(glob.glob("web/*.html")):
        name = os.path.basename(path)
        if name in SKIP:
            continue
        served = served_as(name)
        if served not in AUDIENCE:
            unclassified.append(f"{name} (served at {served}) has no audience "
                                f"in AUDIENCE; add one rather than letting it "
                                f"default.")
    drifted.extend(unclassified)

    # The other direction: a row render() never reads. Either the page moved,
    # or it was skipped and the row is describing a strip that will never be
    # drawn. Both are worth saying out loud; neither is visible otherwise.
    rendered = {served_as(os.path.basename(p)) for p in glob.glob("web/*.html")
                if os.path.basename(p) not in SKIP}
    # render_whitepaper.py renders whitepaper-v2.html marked as /whitepaper.
    rendered.add("/whitepaper")
    for path in sorted(set(AUDIENCE) - rendered - UNRENDERED_AUDIENCE):
        drifted.append(f"AUDIENCE classifies {path} but no page in web/ is served "
                       f"there, so the strip is never rendered; drop the row, or "
                       f"name it in UNRENDERED_AUDIENCE with the reason.")
    for path in sorted(UNRENDERED_AUDIENCE - set(AUDIENCE)):
        drifted.append(f"{path} is in UNRENDERED_AUDIENCE but has no AUDIENCE row "
                       f"to explain; drop it or add the row back.")

    # A PAGE CAN CARRY THE NAV AND NOT ITS STYLESHEET, and then the markup is
    # there, this check is satisfied, and the header renders as a bare list.
    # web/gallery.html shipped that way: the generated nav was present and
    # correct, `nav.css` was never linked, and `summary` fell back to
    # display:list-item -- an unstyled site header on a page we point people at.
    # Presence of the markup was never evidence that it was styled.
    for f in sorted(glob.glob("web/*.html")):
        name = os.path.basename(f)
        if name in SKIP:
            continue
        text = open(f, encoding="utf-8", errors="ignore").read()
        if START not in text:
            continue
        if "nav.css" not in text:
            drifted.append(f"{name} carries the generated nav and does not link "
                           f"/nav.css, so its header renders unstyled. The markup "
                           f"being present is not evidence that it is styled.")

    total = sum(len(i) for _, i in NAV) + 2
    print(f"nav: {len(NAV)} groups, {total} destinations, "
          f"{len(skipped)} pages skipped ({', '.join(sorted(skipped))})")
    if a.check:
        if drifted:
            print("\nA nav that is edited per page becomes seven navs. It did.")
            for d in drifted:
                print(f"  {d}")
            return 1
        print("Every page carries the generated nav.")
        return 0
    print(f"rewrote {len(changed)} pages")
    for d in drifted:
        print(f"  ! {d}")
    return 1 if drifted else 0


if __name__ == "__main__":
    sys.exit(main())
