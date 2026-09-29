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

# THE SITE, AS DATA. Every link the chrome draws is computed from this table:
# the bar, the "More" menu, the developer path (prev / next on each page), the
# footer columns and the lab banner. Nothing below writes a link by hand.
#
# path, label, verb-first note, section, kind
#   kind "path"  on the developer path, in the order a newcomer walks it
#   kind "ref"   reference: reached from the bar or the footer, not the path
#   kind "lab"   unfinished or experimental; kept reachable, marked, off the path
SITE = [
    ("/",                     "Home",          "tokenise a file, read a place, ask", "Try",     "path"),
    ("/demos",                "Demos",         "run live calls in the browser",      "Try",     "path"),
    # The demo walk, one page per plugin skill it shows (plugins/emem/skills/).
    # The five older demo pages still answer at their URLs; each says where
    # its demo went, and is off the walk (see AUDIENCE below).
    ("/demos/signed-answer",  "Signed answer", "read a place, check the receipt",    "Try",     "demo"),
    ("/demos/verify-before-publish", "Check a draft", "catch a wrong number",        "Try",     "demo"),
    ("/demos/handoff",        "Handoff",       "check who wrote a note",             "Try",     "demo"),
    ("/demos/transparency-log", "Log only grows", "prove history was not rewritten", "Try",     "demo"),
    ("/demos/tokenise-files", "Tokenise a file", "cite one section by its hash",     "Try",     "demo"),
    ("/demos/document-evidence", "Document evidence", "parse a report, check the chain", "Try", "demo"),
    ("/demos/field",          "One field",     "edges, pixels, NDVI you compute",    "Try",     "demo"),
    ("/demos/eudr",           "EUDR plot",     "check a plot against the cut-off",   "Try",     "demo"),
    ("/how-it-works",         "How it works",  "follow one request end to end",      "Learn",   "path"),
    ("/reference",            "Reference",     "call every endpoint",                "Connect", "path"),
    ("/tools",                "MCP tools",     "browse all tools, copy a call",      "Connect", "path"),
    ("/verify",               "Verify",        "paste any token, check it here",     "Verify",  "path"),
    ("/guard",                "Guard",         "gate an answer on its citations",    "Verify",  "path"),
    ("/a2a",                  "A2A",           "hand memory between agents",         "Connect", "path"),
    ("/docs/",                "Docs",          "read the book",                      "Connect", "ref"),
    ("/clients",              "Clients",       "connect Claude, Cursor, ChatGPT",    "Connect", "ref"),
    ("/openapi.json",         "OpenAPI",       "load the machine contract",          "Connect", "ref"),
    ("/skills.md",            "Skills",        "run a procedure as an agent",        "Connect", "ref"),
    ("/agents",               "Attesters",     "see every key that writes",          "Verify",  "ref"),
    ("/v1/log/sth",           "Log head",      "pin a signed head",                  "Verify",  "ref"),
    ("/solutions",            "Solutions",     "see four agents using it",           "Learn",   "ref"),
    ("/whitepaper",           "Whitepaper",    "read the math and the proofs",       "Learn",   "ref"),
    ("/spec",                 "Spec",          "implement the wire format",          "Learn",   "ref"),
    ("/whitepaper/v1",        "Whitepaper v1", "read the superseded edition",        "Learn",   "ref"),
    ("/worlds",               "Worlds",        "fly places built on signed facts",   "Lab",     "lab"),
    ("/channel",              "Channel",       "read agents talking",                "Lab",     "lab"),
    ("/scoreboard",           "Scoreboard",    "watch a benchmark race",             "Lab",     "lab"),
    ("/gallery",              "Gallery",       "see the record rendered",            "Lab",     "lab"),
    ("/the-long-version",     "Long version",  "read the old homepage",              "Lab",     "lab"),
]

# The bar shows the developer path flat, so nothing on it hides behind a click;
# everything else sits in one "More" menu, grouped by section.
BAR = ["/demos", "/how-it-works", "/reference", "/tools", "/verify", "/docs/"]
SECTIONS = ["Try", "Learn", "Connect", "Verify", "Lab"]
DEV_PATH = [row[0] for row in SITE if row[4] == "path"]
DEMOS = [row[0] for row in SITE if row[4] == "demo"]
ROW = {row[0]: row for row in SITE}

# Kept for callers that read the old grouped shape (tools, channel and the
# whitepaper generators call render(); nothing reads NAV directly any more).
NAV = [(sec, [(r[1], r[0], r[2]) for r in SITE if r[3] == sec and r[4] != "demo"])
       for sec in SECTIONS]


def path_neighbours(current: str):
    """(prev, next, index, total) on the walk this page belongs to."""
    if current in DEMOS:
        walk = ["/demos"] + DEMOS + ["/how-it-works"]
    else:
        walk = DEV_PATH
    if current not in walk:
        return None
    i = walk.index(current)
    prev = walk[i - 1] if i > 0 else None
    nxt = walk[i + 1] if i + 1 < len(walk) else None
    return prev, nxt, i, len(walk), walk

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
    "/guard":       ("developers","check a draft here, or run the server that enforces it"),
    "/verify":      ("developers","paste a token, watch the proof run"),
    "/demos":       ("anyone",    "live calls you can run and check"),
    "/tools":       ("agents",    "the full tool registry, generated, long"),
    "/a2a":         ("developers","the A2A protocol surface, and the signed channel agents use"),
    "/agents":      ("agents",    "who writes here, as data"),
    "/worlds":      ("anyone",    "real places in 3-D, built on signed facts"),
    "/gallery":     ("anyone",    "the record, rendered"),
    "/channel":     ("anyone",    "agents talking, in public"),
    "/scoreboard":  ("anyone",    "the benchmark, live"),
    "/whitepaper/v1": ("leaders", "superseded, kept as it shipped"),
    "/how-it-works":  ("anyone",    "the address, the fact, the receipt, in order"),
    # This WAS the homepage until 2026-08-26. It is the whole case at length,
    # for a reader who wants more than the one line the front page now carries.
    "/the-long-version": ("anyone", "the whole case, at length"),
    "/404":           ("anyone",    "the page you asked for is not here"),
    # The demos are the one place a reader of any kind can just press a button,
    # so none of them is marked for a specialist.
    "/demos/signed-answer":   ("anyone", "a place, a signed number, a receipt checked here"),
    "/demos/verify-before-publish": ("anyone", "a draft sentence, checked against what was signed"),
    "/demos/handoff":         ("anyone", "what another agent handed you, checked here"),
    "/demos/transparency-log": ("anyone", "a signed log head, and proof it only grew"),
    "/demos/tokenise-files":  ("anyone", "a file cut into units under one root"),
    "/demos/document-evidence": ("anyone", "report text to signed fields, every step hashed"),
    "/demos/field":           ("anyone", "one farm field: its edges and its pixels"),
    "/demos/eudr":            ("anyone", "one plot against the EUDR cut-off"),
    # Older demo pages, kept so their links keep working. Each now points to
    # the demo that replaced it; none is on the walk.
    "/demos/ask-the-earth":   ("anyone", "moved: see the signed answer demo"),
    "/demos/find-similar":    ("anyone", "moved: the similarity index is frozen"),
    "/demos/recall-polygon":  ("anyone", "moved: see the one field demo"),
    "/demos/state-cube":      ("anyone", "moved: its embedding band is retired"),
    "/demos/trajectory":      ("anyone", "moved: see the signed answer demo"),
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


def breadcrumb(current: str) -> str:
    """emem / section / [Demos /] page, from SITE. Empty on the homepage.

    A reader who arrives from a search result lands mid-site. The bar says
    where they could go; this says where they are, in the same words the
    footer columns use."""
    row = ROW.get(current)
    if current == "/" or not row:
        return ""
    items = ['<li><a href="/">emem</a></li>']
    if row[3] != row[1]:
        items.append(f'<li>{row[3]}</li>')
    if current in DEMOS:
        items.append('<li><a href="/demos">Demos</a></li>')
    items.append(f'<li><span aria-current="page">{row[1]}</span></li>')
    return f'<ol class="crumbs" aria-label="Breadcrumb">{"".join(items)}</ol>'


def render(current: str) -> str:
    """`current` is the path of the page being rendered, for the on state."""
    out = [START, '<header class="sitebar"><nav class="sitebar-in" aria-label="Site">',
           '<a href="/" class="brand"><img src="/vortxgola.gif" alt="">emem</a>',
           # On a phone the groups, MCP and the ports took three rows and pushed
           # the homepage composer ~640px down. Below 720px they fold into one
           # panel behind this button. The panel is display:contents on wider
           # screens, so the desktop bar lays out exactly as it did without it.
           # The button is inert until the script below marks the bar .navjs;
           # with scripting off the panel stays unfolded and every link shows.
           '<button type="button" class="navtoggle" aria-expanded="false" aria-controls="sitenav-panel">'
           '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2 4h12M2 8h12M2 12h12"/></svg>Menu</button>',
           '<div class="navpanel" id="sitenav-panel">']
    # The developer path, flat: every step visible, none behind a click.
    for href in BAR:
        label = ROW[href][1]
        on = href == current or (href == "/demos" and current in DEMOS)
        mark = ' aria-current="page"' if on else ''
        out.append(f'<a class="navlink{" on" if on else ""}" href="{href}"{mark}>{label}</a>')
    # Everything else, grouped by section, in one menu. Lab pages say so.
    more = [r for r in SITE if r[0] not in BAR and r[4] not in ("demo",) and r[0] != "/"]
    hit = any(r[0] == current for r in more)
    out.append(f'<details class="navgrp{" on" if hit else ""}" name="sitenav">')
    out.append('<summary>More</summary>')
    out.append('<div class="navmenu navmenu-cols">')
    for sec in SECTIONS:
        rows = [r for r in more if r[3] == sec]
        if not rows:
            continue
        out.append(f'<div class="navsec"><h6>{sec.lower()}</h6>')
        for href, label, note, _, kind in rows:
            mark = ' aria-current="page"' if href == current else ''
            tag = '<i class="labtag">lab</i>' if kind == "lab" else ''
            out.append(f'<a href="{href}"{mark}><b>{label}{tag}</b><span>{note}</span></a>')
        out.append('</div>')
    out.append('</div></details>')
    out.append('<span class="sitebar-gap"></span>')
    # Two doors on the right, not six. The bar used to carry MCP plus a pill
    # per directory listing (ChatGPT, Dify, MCP registry, GitHub), which read
    # as a row of badges beside the navigation and pushed it together at
    # 1280px. The listings live in the footer's "Listed on" column, from this
    # same PORTS table; the bar keeps the one step a developer takes next and
    # the source.
    # /reference#client-setup, not /clients: /clients serves the agent guide as
    # raw markdown, which is right for an agent and wrong for a button.
    out.append('<a class="navcta" href="/reference#client-setup">Connect</a>')
    out.append('<a class="navplain" href="https://github.com/Vortx-AI/emem" rel="noopener noreferrer" target="_blank">GitHub</a>')
    out.append('</div></nav></header>')
    # The audience strip. One line, directly under the bar, so a reader knows
    # whose page this is before they start reading it.
    who, what = AUDIENCE.get(current, ("anyone", ""))
    row = ROW.get(current)
    crumbs = breadcrumb(current)
    if row and row[4] == "lab":
        # A lab page stays reachable and says what it is before anything else.
        out.append(f'<div class="audience aud-lab">{crumbs}<span class="aud-for">lab</span>'
                   f'<span class="aud-what">{what or row[2]}: unfinished, off the developer path</span>'
                   f'<a class="aud-note" href="/demos">start at the demos</a></div>')
    else:
        out.append(f'<div class="audience aud-{who}">{crumbs}'
                   f'<span class="aud-for">for {who}</span>'
                   f'<span class="aud-what">{what}</span>'
                   f'</div>')
    # Progressive enhancement only. Escape and outside-click are the two
    # dismissals <details> has no answer for; everything else is HTML. The
    # phone menu button is the one piece HTML cannot do for us: it closes on
    # Escape (handing focus back to itself), on a click outside, and when
    # focus tabs out of the bar, so a keyboard user is never left behind a
    # panel they cannot see.
    out.append('<script>(function(){'
               'var b=document.querySelector(".sitebar"),'
               't=b&&b.querySelector(".navtoggle");'
               'function shut(){document.querySelectorAll(".navgrp[open]")'
               '.forEach(function(d){d.open=false;});}'
               'function isOpen(){return !!t&&t.getAttribute("aria-expanded")==="true";}'
               'function set(o,f){if(!t)return;'
               't.setAttribute("aria-expanded",o?"true":"false");'
               'b.classList.toggle("navopen",o);'
               'if(!o){shut();if(f)t.focus();}}'
               'if(t){b.classList.add("navjs");'
               't.addEventListener("click",function(){set(!isOpen());});'
               'b.addEventListener("focusout",function(e){'
               'if(isOpen()&&e.relatedTarget&&!b.contains(e.relatedTarget))set(false);});}'
               'document.addEventListener("keydown",function(e){'
               'if(e.key!=="Escape")return;'
               'if(isOpen())set(false,true);else shut();});'
               'document.addEventListener("click",function(e){'
               'if(!e.target.closest(".navgrp"))shut();'
               'if(isOpen()&&!b.contains(e.target))set(false);});'
               '})();</script>')
    out.append(END)
    return "\n".join(out)


FOOT_START = "<!--foot:start-->"
FOOT_END = "<!--foot:end-->"

# What a developer or an agent reaches for without a page: machine entry points.
MACHINE = [
    ("llms.txt", "/llms.txt"), ("agent card", "/.well-known/agent-card.json"),
    ("emem.json", "/.well-known/emem.json"), ("MCP", "/mcp"), ("discover", "/v1/discover"),
    ("quickstart", "/docs/quickstart.html"), ("self-host", "/docs/self-host.html"),
]

# The log head, verified in the tab on every page: the chrome itself is a
# check anyone can watch run. Loaded when the reader is idle, never on the
# critical path; if anything fails it says so and verifies nothing.
PROOF_JS = ("<script>(function(){var el=document.querySelector('[data-proof]');if(!el)return;"
            "var enc=new TextEncoder();"
            "function u32(o,n){o.push(n&255,(n>>>8)&255,(n>>>16)&255,(n>>>24)&255);}"
            "function be8(n){var o=new Uint8Array(8),v=BigInt(n);for(var i=7;i>=0;i--){o[i]=Number(v&255n);v>>=8n;}return o;}"
            "function pre(d,segs){var o=[],b=enc.encode(d);o.push.apply(o,enc.encode('emem.preimage.v1\\0'));u32(o,b.length);o.push.apply(o,b);"
            "segs.forEach(function(s){o.push(s[0]);u32(o,s[1].length);for(var i=0;i<s[1].length;i++)o.push(s[1][i]);});return new Uint8Array(o);}"
            "function say(ok,t){el.innerHTML=(ok?'<b class=\"ok\">&#10003;</b> ':'<b class=\"no\">&#10007;</b> ')+t;}"
            "function run(){var I=globalThis.ememVerifyInternals;if(!I)return say(false,'verifier did not load');"
            "Promise.all([fetch('/.well-known/emem.json').then(function(r){return r.json();}),fetch('/v1/log/sth').then(function(r){return r.json();})]).then(function(x){"
            "var k=x[0].responder&&x[0].responder.pubkey_b32,h=x[1].sth,pk=I.b32decode(h.responder_pubkey_b32);"
            "var d=I.blake3(pre('emem.translog.sth.v1',[[1,be8(h.tree_size)],[2,I.b32decode(h.root_b32)],[3,enc.encode(h.signed_at)],[4,pk]]));"
            "var ok=h.responder_pubkey_b32===k&&I.ed.verify(I.b32decode(h.signature_b32),d,pk);"
            "say(ok,'log head <a href=\"/verify?q=https%3A%2F%2Femem.dev%2Fv1%2Flog%2Fsth\">'+Number(h.tree_size).toLocaleString('en')+' entries</a>, signed '+h.signed_at.slice(11,16)+'Z, '+(ok?'signature checked in this tab':'signature did NOT verify'));"
            "}).catch(function(){say(false,'log head unreachable');});}"
            "function go(){if(globalThis.ememVerifyInternals)return run();var s=document.createElement('script');s.src='/emem-verify-core.js';s.onload=run;"
            "s.onerror=function(){say(false,'verifier did not load');};document.head.appendChild(s);}"
            "(window.requestIdleCallback||function(f){setTimeout(f,1200);})(go);})();</script>")


def render_foot(current: str) -> str:
    """The developer path (prev / next), the footer columns and the live proof."""
    import gen_footer_ports  # the "Listed on" column owns its own markers
    out = [FOOT_START]
    nb = path_neighbours(current)
    if nb:
        prev, nxt, i, total, walk = nb
        out.append('<nav class="pathbar" aria-label="Where this page sits on the developer path">')
        if prev:
            out.append(f'<a class="pb-prev" href="{prev}"><i>&larr;</i><b>{ROW[prev][1]}</b></a>')
        else:
            out.append('<span class="pb-prev"></span>')
        out.append('<ol class="pb-steps">')
        for k, href in enumerate(walk):
            here = ' aria-current="step"' if href == current else ''
            out.append(f'<li><a href="{href}" title="{ROW[href][1]}: {ROW[href][2]}"{here}>'
                       f'<span>{k + 1}</span></a></li>')
        out.append('</ol>')
        if nxt:
            out.append(f'<a class="pb-next" href="{nxt}"><b>{ROW[nxt][1]}</b><span>{ROW[nxt][2]}</span><i>&rarr;</i></a>')
        else:
            out.append(f'<a class="pb-next" href="{GITHUB}" target="_blank" rel="noopener noreferrer"><b>GitHub</b><span>star it, fork it, run it</span><i>&rarr;</i></a>')
        out.append('</nav>')
    out.append('<footer class="page foot sitefoot">')
    out.append('<div class="sf-top"><a class="sf-brand" href="/"><img src="/vortxgola.gif" alt="">emem</a>'
               '<p class="sf-proof" data-proof>log head: checking in this tab&hellip;</p></div>')
    out.append('<div class="sf-grid">')
    for sec in SECTIONS:
        rows = [r for r in SITE if r[3] == sec and r[4] != "demo"]
        out.append(f'<div class="foot-col"><h4>{sec}</h4><ul>')
        for href, label, _, _, kind in rows:
            tag = ' <i class="labtag">lab</i>' if kind == "lab" else ''
            out.append(f'<li><a href="{href}">{label}</a>{tag}</li>')
        out.append('</ul></div>')
    out.append('<div class="foot-col"><h4>Machine</h4><ul>')
    for label, href in MACHINE:
        out.append(f'<li><a href="{href}">{label}</a></li>')
    out.append('</ul></div>')
    out.append(gen_footer_ports.column())
    out.append('</div>')
    out.append('<div class="sf-bottom"><span>Apache-2.0 &middot; built by '
               '<a href="https://vortx.ai" target="_blank" rel="noopener noreferrer">vortx.ai</a></span>'
               f'<span><a href="{GITHUB}" target="_blank" rel="noopener noreferrer">github</a> &middot; '
               '<a href="/privacy">privacy &middot; terms</a></span></div>')
    out.append('</footer>')
    out.append(PROOF_JS)
    out.append(FOOT_END)
    return "\n".join(out)


_BODY = re.compile(r'<body\b([^>]*)>')


def mark_doc(html: str) -> str:
    """Put class="doc" on <body>, which is what nav.css keys the inner-page
    width and gutter on. Idempotent; a page with no <body> tag is left alone."""
    # Search after </head>: a <body mentioned in a head comment or a <style>
    # block matched first once, and the class landed inside the comment.
    head_end = html.find("</head>")
    m = _BODY.search(html, head_end if head_end >= 0 else 0)
    if not m:
        return html
    attrs = m.group(1)
    cm = re.search(r'\bclass="([^"]*)"', attrs)
    if cm:
        if "doc" in cm.group(1).split():
            return html
        attrs = attrs[:cm.start()] + f'class="doc {cm.group(1)}"' + attrs[cm.end():]
    else:
        attrs = ' class="doc"' + attrs
    return html[:m.start()] + f'<body{attrs}>' + html[m.end():]


FOOT_GEN = re.compile(re.escape(FOOT_START) + r'.*?' + re.escape(FOOT_END), re.S)
FOOT_OLD = re.compile(r'<footer\b[^>]*>.*?</footer>', re.S)


def apply_foot(html: str, current: str) -> str:
    foot = render_foot(current)
    if FOOT_GEN.search(html):
        return FOOT_GEN.sub(lambda _: foot, html, count=1)
    m = None
    for m in FOOT_OLD.finditer(html):
        pass  # the page footer is the last <footer> in the document
    if m:
        return html[:m.start()] + foot + html[m.end():]
    i = html.rfind("</body>")
    return (html[:i] + foot + "\n" + html[i:]) if i >= 0 else html + "\n" + foot


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
    if name == "whitepaper-v1.html":
        return "/whitepaper/v1"
    return "/" + name[:-len(".html")]


# Where emem is listed, as sockets rather than as vendor badges.
#
# These drive traffic and they are the one part of the bar a reader is likely
# to click on their way OUT, so they sit at the end of it. Drawn as sockets to
# match the plug the homepage is built around, and labelled with words rather
# than redrawn vendor logos: a hand-approximated brand mark reads as a cheap
# copy of the brand, and at 20px an unfamiliar glyph is a mystery-meat link.
# The word is the recognisable part, so the word is what is shown.
#
# The README badge wall carries the rest (CI, licence, Glama, MCP Toplist, VS
# Code install, Zenodo DOI). Four here is the whole point: a fifth would make
# this a second navigation.
# Every one of these leaves emem, so every one opens in a new tab. A reader who
# clicks a directory listing from the middle of a page has not asked to abandon
# the page, and 320 of 321 off-site links across 26 pages were taking them away
# from it. `noreferrer` rides along with `noopener` because a new-tab link
# without it hands the destination our URL for free.
PORTS = [
    ("ChatGPT", "https://chatgpt.com/plugins/plugin_asdk_app_6a6a0832a59081918b19aec0ddf9ec77",
     "emem in the ChatGPT plugin directory"),
    ("Dify", "https://marketplace.dify.ai/plugin/vortx-ai/emem",
     "emem in the Dify marketplace"),
    ("MCP registry", "https://github.com/mcp/Vortx-AI/emem",
     "emem in the GitHub MCP registry"),
    ("GitHub", "https://github.com/Vortx-AI/emem",
     "the source, Apache-2.0"),
]

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


# PROSE, FOLDED. A paragraph past FOLD_WORDS keeps its first sentence and
# folds the rest behind "more": the claim stays on the page, the argument is
# one click away. Deterministic and idempotent (a folded remainder carries a
# class, so it is never folded again), so the gate can compare like the nav.
FOLD_WORDS = 45
# Generated elsewhere, or a paper whose paragraphs are the point.
FOLD_SKIP = {"index.html", "tools.html", "channel.html", "whitepaper-v1.html",
             "whitepaper-v2.html", "the-long-version.html"}
_INLINE = {"a", "b", "strong", "em", "i", "code", "span", "sup", "sub", "abbr", "kbd", "small", "mark", "q", "s", "u"}
_VOID = {"br", "img", "wbr"}


def _split_first(inner: str):
    """Split paragraph HTML after its first sentence, outside any inline tag."""
    depth, i, text_len = 0, 0, 0
    n = len(inner)
    while i < n:
        c = inner[i]
        if c == "<":
            j = inner.find(">", i)
            if j < 0:
                return None
            tag = inner[i + 1:j].strip()
            name = re.match(r"/?([a-zA-Z0-9]+)", tag)
            nm = name.group(1).lower() if name else ""
            if nm not in _INLINE and nm not in _VOID:
                return None  # block content inside a <p>: leave it alone
            if tag.startswith("/"):
                depth -= 1
            elif nm not in _VOID and not tag.endswith("/"):
                depth += 1
            i = j + 1
            continue
        text_len += 1
        if (c in ".:" and depth == 0 and text_len >= 15 and i + 2 < n
                and inner[i + 1] == " " and (inner[i + 2].isupper() or inner[i + 2].isdigit() or inner[i + 2] in "<`\"(")):
            head, tail = inner[:i + 1], inner[i + 2:]
            if len(re.sub(r"<[^>]+>", "", tail).split()) < 8:
                return None  # not worth a click
            return head, tail
        i += 1
    return None


_P = re.compile(r"<p(\s[^>]*)?>(.*?)</p>", re.S)
_OPAQUE = re.compile(r"(<script\b.*?</script>|<style\b.*?</style>|<pre\b.*?</pre>|<template\b.*?</template>|"
                     + re.escape(START) + r".*?" + re.escape(END) + r"|"
                     + re.escape("<!--foot:start-->") + r".*?" + re.escape("<!--foot:end-->") + r")", re.S)


def fold_prose(html: str) -> str:
    def one(m):
        attrs, inner = m.group(1) or "", m.group(2)
        # A paragraph a script can address (id, data-*) or one already folded
        # is left exactly as written.
        if re.search(r"\b(id|data-[\w-]+)=", attrs) or "rest" in attrs:
            return m.group(0)
        if len(re.sub(r"<[^>]+>", "", inner).split()) <= FOLD_WORDS:
            return m.group(0)
        cut = _split_first(inner)
        if not cut:
            return m.group(0)
        head, tail = cut
        return (f'<p{attrs}>{head}</p><details class="fold"><summary>more</summary>'
                f'<p class="rest">{tail}</p></details>')
    parts = _OPAQUE.split(html)
    return "".join(pt if k % 2 else _P.sub(one, pt) for k, pt in enumerate(parts))


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--except", dest="exclude", action="append", default=[],
                    help="a web/ file name to leave untouched (another writer owns it now)")
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
        if name in SKIP or name in a.exclude:
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
        out = apply_foot(out, served_as(name))
        if name != "index.html":
            out = mark_doc(out)
        if name not in FOLD_SKIP:
            out = fold_prose(out)
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
