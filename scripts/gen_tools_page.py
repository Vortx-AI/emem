#!/usr/bin/env python3
"""Generate web/tools.html, the static tool explorer, from the live
tool registry, so it can never be hand-maintained or drift.

Runs inside the deploy ritual (before cargo build; the page is baked via
include_str). Groups: the core loop pinned first (its size comes from the registry), then every
remaining tool by category. Each row is name, what question it answers,
and a copy-paste call. A model-mediated reader gets capabilities, not a
count.
"""
from __future__ import annotations

import html
import importlib.util as _ilu
import json
import sys
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent

# The site nav, from the one place that owns it.
#
# This generator predated the design-token and nav consolidation and was never
# brought forward, so its output was the pre-consolidation page: literal font
# sizes instead of the scale, no tokens.css, no nav at all. web/tools.html had
# been fixed BY HAND, which meant every `scripts/redeploy.sh` quietly reverted
# it before `cargo build` baked it back in via include_str — the live /tools
# page lost its site chrome on each deploy and only a gate run afterwards
# would have said so. Importing render() the way render_whitepaper.py and
# build_channel.py already do makes the generated page the same page
# gen_nav.py --check and design_tokens.py expect.
_spec = _ilu.spec_from_file_location("gen_nav", str(Path(__file__).with_name("gen_nav.py")))
_gen_nav = _ilu.module_from_spec(_spec)
_spec.loader.exec_module(_gen_nav)
SOURCES = ["http://127.0.0.1:5051/v1/tools", "https://emem.dev/v1/tools"]

CATEGORY_ORDER = ["read", "write", "verify", "introspect", "plan"]
CATEGORY_TITLE = {
    "read": "Read the world",
    "write": "Write and attest",
    "verify": "Verify and prove",
    "introspect": "Introspect the surface",
    "plan": "Plan and compose",
}


def fetch() -> dict:
    last = None
    for u in SOURCES:
        try:
            with urllib.request.urlopen(u, timeout=30) as r:
                return json.load(r)
        except Exception as exc:  # noqa: BLE001, try the next source
            last = exc
    raise SystemExit(f"could not fetch the tool registry: {last}")


def example(t: dict) -> str:
    args = t.get("example_args") or {}
    body = json.dumps({"name": t["name"], "arguments": args})
    return (
        "curl -s -X POST https://emem.dev/mcp -H 'content-type: application/json' "
        + "-d '"
        # A single quote in an example (an English possessive) would close the
        # shell string, so it is written as '\'' and the line still pastes.
        + json.dumps(
            {"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": json.loads(body)}
        ).replace("'", "'\\''")
        + "'"
    )


def row(t: dict) -> str:
    what = (t.get("when_to_use") or t.get("description") or "").split(". ")[0].rstrip(".")
    name = html.escape(t['name'])
    return f"""
<details class="tool" id="{name}" data-q="{html.escape((t['name'] + ' ' + (t.get('title') or '') + ' ' + (t.get('description') or '')).lower())}">
  <summary><code>{name}</code><span class="tt">{html.escape(t.get('title') or '')}</span><span class="tw">{html.escape(what)}.</span></summary>
  <p>{html.escape(t.get('description') or '')}</p>
  <pre><code>{html.escape(example(t))}</code></pre>
  <p class="tl"><a href="#{name}">link to this tool</a></p>
</details>"""


def main() -> int:
    reg = fetch()
    tools = reg["tools"]
    core = [t for t in tools if t.get("tier") == "core"]
    rest = [t for t in tools if t.get("tier") != "core"]
    sections = [
        (
            f"Core loop ({len(core)})",
            "What /mcp lists by default: enough to name a thing, ground it to a place, read, cite and verify. Every other tool stays callable by name.",
            core,
        )
    ]
    for cat in CATEGORY_ORDER:
        group = sorted((t for t in rest if t.get("category") == cat), key=lambda t: t["name"])
        if group:
            sections.append((f"{CATEGORY_TITLE.get(cat, cat)} ({len(group)})", "", group))
    leftover = sorted(
        (t for t in rest if t.get("category") not in CATEGORY_ORDER), key=lambda t: t["name"]
    )
    if leftover:
        sections.append((f"Other ({len(leftover)})", "", leftover))

    parts, toc = [], []
    for k, (title, sub, group) in enumerate(sections):
        sid = f"sec-{k}"
        toc.append(f'<li><a href="#{sid}"><span>{k:02d}</span>{html.escape(title)}</a></li>')
        parts.append(f'<section class="tgroup" id="{sid}" aria-labelledby="{sid}-h">')
        parts.append(f'<h2 id="{sid}-h">{html.escape(title)}</h2>')
        if sub:
            parts.append(f'<p class="sub">{html.escape(sub)}</p>')
        parts.extend(row(t) for t in group)
        parts.append('</section>')
    body = "\n".join(parts)
    toc_html = "\n".join(toc)

    total = len(tools)
    site_nav = _gen_nav.render("/tools")
    page = f"""<!doctype html>
<html lang=en>
<head>
<meta charset=utf-8>
<meta name=viewport content="width=device-width,initial-scale=1">
<title>Every tool · emem</title>
<meta name=description content="All {total} emem MCP tools from the registry: what each answers and the exact call. The {len(core)}-tool core loop first. Free to read; writes are signed and tiered.">
<link rel=canonical href="https://emem.dev/tools">
<link rel=icon type="image/gif" href="/vortxgola.gif">
<link rel=preconnect href="https://fonts.googleapis.com"><link rel=preconnect href="https://fonts.gstatic.com" crossorigin>
<link rel=stylesheet href="https://fonts.googleapis.com/css2?family=JetBrains+Mono:ital,wght@0,200..800;1,200..800&family=Newsreader:ital,opsz,wght@0,6..72,300..700;1,6..72,300..700&display=swap">
<link rel=stylesheet href="/tokens.css">
<link rel=stylesheet href="/nav.css">
<style>
*{{box-sizing:border-box}}body{{margin:0;background:var(--paper);color:var(--ink);font-family:var(--mono);line-height:1.55}}
a{{color:var(--accent)}}
p a{{text-decoration:underline;text-decoration-thickness:1px;text-underline-offset:.18em}}
.btn{{display:inline-flex;align-items:center;min-height:44px;padding:0 var(--s-3);border:1px solid var(--ink);background:var(--ink);color:var(--paper);text-decoration:none;font-family:var(--mono);font-size:var(--t-sm);font-weight:600}}
.btn.ghost{{background:transparent;color:var(--ink)}}
.btn:hover{{background:var(--ink-2);color:var(--paper)}}
.tl-body{{display:grid;grid-template-columns:var(--col-meta) minmax(0,1fr);gap:var(--s-5);padding-top:var(--s-4);padding-bottom:var(--s-5)}}
.tl-side{{position:sticky;top:64px;align-self:start;max-height:calc(100vh - 80px);overflow:auto}}
.tl-side label{{display:block;font-size:var(--t-3xs);text-transform:uppercase;letter-spacing:.08em;color:var(--mute);margin:0 0 var(--s-1)}}
.tl-side input{{width:100%;min-height:44px;padding:0 var(--s-2);border:1px solid var(--rule-strong);background:var(--paper);color:var(--ink);font:inherit;font-size:var(--t-sm)}}
.tl-side .count{{font-size:var(--t-2xs);color:var(--mute);margin:var(--s-1) 0 var(--s-4)}}
.tl-side .doc-toc ol{{columns:1}}
.tgroup{{margin:0 0 var(--s-5)}}
.doc-head .tl-note{{font-family:var(--mono);font-size:var(--t-xs);line-height:1.55;border-left:1px solid var(--rule-strong);padding-left:var(--s-3)}}
h2{{font-family:var(--display);font-weight:420;font-size:var(--t-2xl);line-height:1.1;margin:0 0 var(--s-2);padding-bottom:var(--s-2);border-bottom:1px solid var(--rule-strong)}}
.sub{{color:var(--ink-2);font-size:var(--t-sm);margin:0 0 var(--s-3);max-width:var(--measure)}}
.tool{{border:1px solid var(--rule);border-top:0;background:var(--paper)}}
.tgroup h2+.tool,.tgroup .sub+.tool{{border-top:1px solid var(--rule)}}
.tool[open]{{background:var(--paper-2)}}
.tool summary{{display:grid;grid-template-columns:minmax(14rem,22rem) minmax(10rem,16rem) minmax(0,1fr);gap:var(--s-3);align-items:baseline;padding:var(--s-2) var(--s-3);cursor:pointer;min-height:44px}}
.tool summary code{{color:var(--accent);font-weight:600;overflow-wrap:anywhere}}
.tool .tt{{color:var(--ink);font-size:var(--t-xs)}}
.tool .tw{{color:var(--mute);font-size:var(--t-xs)}}
.tool p{{padding:0 var(--s-3);color:var(--ink-2);font-size:var(--t-sm);max-width:var(--measure)}}
.tool pre{{margin:var(--s-2) var(--s-3);padding:var(--s-2) var(--s-3);background:var(--paper-3);border:1px solid var(--rule);overflow-x:auto;font-size:var(--t-2xs)}}
.tool .tl{{font-size:var(--t-2xs);margin:0 0 var(--s-2)}}
.tool:target{{outline:2px solid var(--accent);outline-offset:-2px}}
.tool[hidden],.tgroup[hidden]{{display:none}}
@media (max-width:900px){{.tl-body{{grid-template-columns:minmax(0,1fr)}}.tl-side{{position:static;max-height:none}}.tool summary{{grid-template-columns:minmax(0,1fr)}}.tool .tw{{display:none}}}}
</style>
</head>
<body class="doc">
{site_nav}
<header class="doc-head">
  <div>
    <p class="eyebrow">Connect &middot; generated from /v1/tools</p>
    <h1>MCP tools</h1>
    <p class="purpose">All {total} tools this responder dispatches, each with what it answers and a call you can paste. The {len(core)}-tool core loop comes first.</p>
    <div class="actions">
      <a class="btn" href="#sec-0">Core loop</a>
      <a class="btn ghost" href="/v1/tools">/v1/tools JSON</a>
      <a class="btn ghost" href="/reference">API reference</a>
    </div>
    <dl class="facts">
      <div><dt>endpoint</dt><dd>https://emem.dev/mcp</dd></div>
      <div><dt>every tool</dt><dd>/mcp/full</dd></div>
      <div><dt>reads</dt><dd>no account, no API key</dd></div>
      <div><dt>A2A</dt><dd><a href="/.well-known/agent-card.json">agent card</a></dd></div>
    </dl>
  </div>
  <p class="purpose tl-note">This page is rendered from the registry the server serves at <a href="/v1/tools">/v1/tools</a>. Every call below runs without a key. Tools not listed by /mcp stay callable by name through <code>tools/call</code>.</p>
</header>
<div class="page tl-body">
<aside class="tl-side">
  <label for="tl-q">Filter tools</label>
  <input id="tl-q" type="search" placeholder="name or keyword" autocomplete="off" spellcheck="false">
  <p class="count" id="tl-count" aria-live="polite">{total} tools</p>
  <nav class="doc-toc" aria-label="On this page"><h2>Sections</h2><ol>
{toc_html}
  </ol></nav>
</aside>
<main class="tl-list">
{body}
</main>
</div>
<script>(function(){{var q=document.getElementById("tl-q"),c=document.getElementById("tl-count");if(!q)return;
var tools=[].slice.call(document.querySelectorAll(".tool")),groups=[].slice.call(document.querySelectorAll(".tgroup"));
q.addEventListener("input",function(){{var v=q.value.trim().toLowerCase(),n=0;
tools.forEach(function(t){{var hit=!v||t.dataset.q.indexOf(v)>=0;t.hidden=!hit;if(hit)n++;}});
groups.forEach(function(g){{g.hidden=!g.querySelector(".tool:not([hidden])");}});
c.textContent=n+(n===1?" tool":" tools")+(v?" match":"");}});}})();</script>
</body></html>
"""
    # the generated footer too, from the same site map as the bar
    (REPO / "web" / "tools.html").write_text(_gen_nav.apply_foot(page, "/tools"))
    print(f"wrote web/tools.html: {total} tools, {len(sections)} sections")
    return 0


if __name__ == "__main__":
    sys.exit(main())
