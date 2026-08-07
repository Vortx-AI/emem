#!/usr/bin/env python3
"""Render docs/whitepaper-v2.md into web/whitepaper-v2.html.

The v1 whitepaper was maintained as two hand-written copies of one
document: docs/whitepaper.md and web/whitepaper.html. They drifted, and
every count fix had to be applied twice. Worse, the drift was invisible:
nothing compared them, so the HTML could assert one thing while the
markdown asserted another and both shipped.

So v2 has one source. The markdown is the document; this script renders
it, and web/whitepaper-v2.html is a build artefact that happens to be
committed (the server bakes it in with include_str!, so it must exist
before cargo build). Editing the HTML by hand is pointless: the next
render overwrites it. Edit the markdown.

The page chrome (head, CSS, statusbar, footer, TOC observer script) is
lifted verbatim out of web/whitepaper-v1.html, so v2 inherits v1's design
without a second copy of 200 lines of CSS to maintain. Structural markers
rather than line numbers do the extraction, so a CSS edit in v1 does not
silently shift the cut.

Usage:  python3 scripts/render_whitepaper.py [--check]

  --check  render and diff against the committed file; exit 1 on drift
           rather than writing. For CI.
"""

from __future__ import annotations

import html
import pathlib
import re
import importlib.util as _ilu

# The nav comes from the one place that generates it. This file used to
# carry its own hand-written copy, which is how the site ended up with seven
# of them, and a generated page holding a stale nav is the same drift wearing
# a build step.
_spec = _ilu.spec_from_file_location('gen_nav', str(__import__('pathlib').Path(__file__).with_name('gen_nav.py')))
_gen_nav = _ilu.module_from_spec(_spec); _spec.loader.exec_module(_gen_nav)

import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
SRC = ROOT / "docs" / "whitepaper-v2.md"
SHELL = ROOT / "web" / "whitepaper-v1.html"
OUT = ROOT / "web" / "whitepaper-v2.html"

TITLE = "emem whitepaper v2: an external identity layer for verifiable agent memory"
DESC = (
    "The emem whitepaper, version 2. An external identity layer for verifiable agent "
    "memory: the token grammar, cell64 + tslot addressing, canonical CBOR + BLAKE3 CIDs, "
    "ed25519 receipts over a domain-separated tagged preimage, six tamper-provenance "
    "classes, the RFC 6962 tree construction, and caller-registered derivations. "
    "Supersedes v1."
)


# ---------------------------------------------------------------- inline

def inline(text: str) -> str:
    """Inline markdown -> HTML. Order matters: code spans are extracted
    first and reinserted last, so `**` inside a code span stays literal."""
    spans: list[str] = []

    def stash(m: re.Match) -> str:
        spans.append(m.group(1))
        return f"\x00{len(spans) - 1}\x00"

    text = re.sub(r"`([^`]+)`", stash, text)
    text = html.escape(text, quote=False)
    text = re.sub(r"\[([^\]]+)\]\(([^)]+)\)", r'<a href="\2">\1</a>', text)
    text = re.sub(r"\*\*([^*]+)\*\*", r"<strong>\1</strong>", text)
    text = re.sub(r"(?<![*\w])\*([^*]+)\*(?![*\w])", r"<em>\1</em>", text)
    for i, s in enumerate(spans):
        text = text.replace(f"\x00{i}\x00", f"<code>{html.escape(s, quote=False)}</code>")
    return text


def slug(s: str) -> str:
    s = re.sub(r"[^\w\s-]", "", s.lower())
    return re.sub(r"[\s_]+", "-", s).strip("-")


# ---------------------------------------------------------------- blocks

def render_table(rows: list[str]) -> str:
    cells = [[c.strip() for c in r.strip().strip("|").split("|")] for r in rows]
    head, body = cells[0], cells[2:]  # cells[1] is the |---| separator
    out = ['<div class="tbl-wrap"><table class="t">', "<thead><tr>"]
    out += [f"<th>{inline(c)}</th>" for c in head]
    out += ["</tr></thead>", "<tbody>"]
    for row in body:
        out.append("<tr>" + "".join(f"<td>{inline(c)}</td>" for c in row) + "</tr>")
    out += ["</tbody>", "</table></div>"]
    return "\n        ".join(out)


def render_body(md: str) -> tuple[str, list[tuple[str, str, str]]]:
    """Return (main_html, toc) where toc is [(anchor, num, title)]."""
    lines = md.split("\n")
    out: list[str] = []
    toc: list[tuple[str, str, str]] = []
    i = 0
    open_section = False
    para: list[str] = []
    lst: list[str] = []
    lst_kind = ""

    def flush_para() -> None:
        nonlocal para
        if para:
            out.append(f"      <p>{inline(' '.join(para))}</p>")
            para = []

    def flush_list() -> None:
        nonlocal lst, lst_kind
        if lst:
            tag = "ol" if lst_kind == "ol" else "ul"
            # .extend, not `out +=`: an augmented assignment would rebind
            # `out` as a local and shadow the closed-over list.
            out.append(f"      <{tag}>")
            out.extend(f"        <li>{inline(x)}</li>" for x in lst)
            out.append(f"      </{tag}>")
            lst = []
            lst_kind = ""

    def flush() -> None:
        flush_para()
        flush_list()

    while i < len(lines):
        ln = lines[i]

        if ln.startswith("# "):  # H1 -> hero, handled by caller
            i += 1
            continue

        if ln.startswith("## "):
            flush()
            if open_section:
                out.append("    </section>")
                out.append("")
                out.append('    <hr class="band-rule">')
                out.append("")
            title = ln[3:].strip()
            m = re.match(r"^(\d+)\.\s+(.*)$", title)
            alias = slug(title)
            if m:
                num, name = m.group(1), m.group(2)
                anchor = f"s{num}"
                h2 = f'<h2><span class="num">{num}</span>{inline(name)}</h2>'
                toc.append((anchor, num, name))
            else:
                num, anchor = "", slug(title)
                h2 = f"<h2>{inline(title)}</h2>"
                toc.append((anchor, "", title))
            out.append(f'    <section id="{anchor}">')
            # The markdown and the page use different anchor schemes: a link
            # written against the .md says "#16-changes-from-v1", the page
            # says "#s16". Emit the markdown slug as an alias so a link that
            # works in one works in both. Agents read the .md; browsers get
            # this. Neither should hit a dead anchor.
            if alias and alias != anchor:
                out.append(f'      <a id="{alias}" class="anchor-alias"></a>')
            out.append(f"      {h2}")
            open_section = True
            i += 1
            continue

        if ln.startswith("### "):
            flush()
            out.append(f"      <h3>{inline(ln[4:].strip())}</h3>")
            i += 1
            continue

        if ln.startswith("```"):
            flush()
            lang = ln[3:].strip() or "text"
            i += 1
            buf: list[str] = []
            while i < len(lines) and not lines[i].startswith("```"):
                buf.append(lines[i])
                i += 1
            i += 1
            code = html.escape("\n".join(buf), quote=False)
            out.append(
                f'      <div class="code"><div class="code-head">'
                f'<span class="lang">{html.escape(lang)}</span></div>'
                f"<pre><code>{code}</code></pre></div>"
            )
            continue

        if ln.strip().startswith("|") and i + 1 < len(lines) and re.match(
            r"^\s*\|[\s:|-]+\|\s*$", lines[i + 1]
        ):
            flush()
            rows: list[str] = []
            while i < len(lines) and lines[i].strip().startswith("|"):
                rows.append(lines[i])
                i += 1
            out.append("      " + render_table(rows))
            continue

        if re.match(r"^\s*[-*]\s+", ln):
            flush_para()
            lst_kind = "ul"
            lst.append(re.sub(r"^\s*[-*]\s+", "", ln))
            i += 1
            while i < len(lines) and re.match(r"^\s{2,}\S", lines[i]) and not re.match(
                r"^\s*[-*]\s+", lines[i]
            ):
                lst[-1] += " " + lines[i].strip()
                i += 1
            continue

        if re.match(r"^\s*\d+\.\s+", ln):
            flush_para()
            lst_kind = "ol"
            lst.append(re.sub(r"^\s*\d+\.\s+", "", ln))
            i += 1
            while i < len(lines) and re.match(r"^\s{2,}\S", lines[i]) and not re.match(
                r"^\s*\d+\.\s+", lines[i]
            ):
                lst[-1] += " " + lines[i].strip()
                i += 1
            continue

        if ln.strip() == "---":
            flush()
            i += 1
            continue

        if not ln.strip():
            flush()
            i += 1
            continue

        para.append(ln.strip())
        i += 1

    flush()
    if open_section:
        out.append("    </section>")
    return "\n".join(out), toc


# ---------------------------------------------------------------- chrome

def chrome() -> tuple[str, str]:
    """(head_html, tail_html) lifted from the v1 page by structural marker."""
    s = SHELL.read_text()
    head_end = s.index("</style>") + len("</style>")
    head = s[:head_end]
    head = re.sub(r"<title>.*?</title>", f"<title>{html.escape(TITLE)}</title>", head, flags=re.S)
    head = re.sub(
        r'(<meta name=description content=")[^"]*(")',
        lambda m: m.group(1) + html.escape(DESC, quote=True) + m.group(2),
        head,
    )
    head = re.sub(
        r'(<meta property=og:(?:title|description) content=")[^"]*(")',
        lambda m: m.group(1) + html.escape(TITLE if "title" in m.group(1) else DESC, quote=True) + m.group(2),
        head,
    )
    head = head.replace(
        'href="https://emem.dev/whitepaper.html"', 'href="https://emem.dev/whitepaper"'
    )
    head = head.replace("href=/whitepaper.md", "href=/whitepaper.md")
    tail = s[s.index("</main>") + len("</main>") :]
    return head, tail


def build() -> str:
    md = SRC.read_text()
    h1 = re.search(r"^# (.*)$", md, re.M).group(1)
    head, tail = chrome()
    body, toc = render_body(md)

    # The TOC is an <ol><li> inside .wrap, which is the grid
    # (grid-template-columns:16rem 1fr) that gives the sticky nav its own
    # column. Emitting bare <a> straight into .page leaves position:sticky
    # with no column to sit in, and the nav lies on top of the prose.
    toc_html: list[str] = []
    for anchor, num, title in toc:
        n = num if num else "&sect;"
        toc_html.append(
            f'      <li><a href="#{anchor}"><span class="n">{n}</span>{html.escape(title)}</a></li>'
        )

    title_html = re.sub(
        r"^(.*?): (.*)$", r'\1: <span class="accent">\2</span>', html.escape(h1)
    )

    site_nav = _gen_nav.render("/whitepaper")
    return f"""{head}
</head>
<body>

{site_nav}

<div class="page">
  <header class="hero">
    <p class="kicker"><span class="seal"></span> Whitepaper v2 &middot; Vortx AI</p>
    <h1>{title_html}</h1>
    <p class="meta-line"><b>Vortx AI</b> <span class="dot">&middot;</span> <b>v2</b> &middot; 2026-07-14 <span class="dot">&middot;</span> Apache-2.0 <span class="dot">&middot;</span> reference responder <code>emem.dev</code></p>
    <div class="hero-cta">
      <a class="btn" href="/whitepaper.md">read as markdown</a>
      <a class="btn ghost" href="/v1/verifier_spec">/v1/verifier_spec</a>
      <a class="btn ghost" href="/whitepaper/v1">v1 (archived)</a>
    </div>
  </header>
</div>

<div class="page">
<div class="wrap">

  <nav class="toc" aria-label="Table of contents">
    <p class="toc-h"><span class="seal"></span> Contents</p>
    <ol>
{chr(10).join(toc_html)}
    </ol>
  </nav>

  <main class="doc">

{body}

  </main>{tail}"""


def _assert_well_formed(doc: str) -> None:
    """Cheap structural checks. A layout breaks silently when a div is
    unbalanced: the page still renders, the sticky TOC just lies on top of
    the prose. Catch it here rather than in a browser."""
    import re as _re

    opens = len(_re.findall(r"<div\b", doc))
    closes = len(_re.findall(r"</div>", doc))
    if opens != closes:
        raise SystemExit(f"render: unbalanced <div>: {opens} open, {closes} close")
    order = [m.group(1) for m in _re.finditer(r'<(?:div|nav|main) class="(page|wrap|toc|doc)"', doc)]
    # The sticky nav needs .wrap's grid column; .wrap must open before it.
    if "wrap" not in order or order.index("wrap") > order.index("toc"):
        raise SystemExit("render: .toc is not inside .wrap; the sticky nav will overlap the prose")
    if order.index("toc") > order.index("doc"):
        raise SystemExit("render: .toc must precede .doc inside .wrap")

    # Every in-page link must land somewhere. A dead anchor is invisible:
    # the browser just does nothing, and the reader assumes the section is
    # missing rather than the link broken.
    targets = set(_re.findall(r'<(?:section|a) id="([^"]+)"', doc))
    dangling = sorted(set(_re.findall(r'href="#([^"]+)"', doc)) - targets)
    if dangling:
        raise SystemExit(f"render: internal links with no target: {dangling}")


def main() -> int:
    rendered = build()
    _assert_well_formed(rendered)
    if "--check" in sys.argv:
        if not OUT.exists():
            print(f"{OUT} does not exist; run without --check", file=sys.stderr)
            return 1
        if OUT.read_text() != rendered:
            print(
                f"::error::{OUT.relative_to(ROOT)} is stale. "
                f"It is generated from {SRC.relative_to(ROOT)}; "
                f"run python3 scripts/render_whitepaper.py",
                file=sys.stderr,
            )
            return 1
        print(f"{OUT.relative_to(ROOT)} is up to date with {SRC.relative_to(ROOT)}")
        return 0
    OUT.write_text(rendered)
    print(f"wrote {OUT.relative_to(ROOT)} ({len(rendered.splitlines())} lines) from {SRC.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
