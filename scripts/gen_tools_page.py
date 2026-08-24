#!/usr/bin/env python3
"""Generate web/tools.html, the static tool explorer, from the live
tool registry, so it can never be hand-maintained or drift.

Runs inside the deploy ritual (before cargo build; the page is baked via
include_str). Groups: the 15-tool core loop pinned first, then every
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
        + json.dumps(
            {"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": json.loads(body)}
        )
        + "'"
    )


def row(t: dict) -> str:
    what = (t.get("when_to_use") or t.get("description") or "").split(". ")[0].rstrip(".")
    return f"""
<details class="tool" id="{html.escape(t['name'])}">
  <summary><code>{html.escape(t['name'])}</code><span class="tt">{html.escape(t.get('title') or '')}</span><span class="tw">{html.escape(what)}.</span></summary>
  <p>{html.escape(t.get('description') or '')}</p>
  <pre><code>{html.escape(example(t))}</code></pre>
</details>"""


def main() -> int:
    reg = fetch()
    tools = reg["tools"]
    core = [t for t in tools if t.get("tier") == "core"]
    rest = [t for t in tools if t.get("tier") != "core"]
    sections = [
        (
            f"The core loop ({len(core)} tools)",
            "What /mcp advertises by default: enough to name, ground, read, cite, and verify.",
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

    parts = []
    for title, sub, group in sections:
        parts.append(f'<h2>{html.escape(title)}</h2>')
        if sub:
            parts.append(f'<p class="sub">{html.escape(sub)}</p>')
        parts.extend(row(t) for t in group)
    body = "\n".join(parts)

    total = len(tools)
    site_nav = _gen_nav.render("/tools")
    page = f"""<!doctype html>
<html lang=en>
<head>
<meta charset=utf-8>
<meta name=viewport content="width=device-width,initial-scale=1">
<title>Every tool · emem</title>
<meta name=description content="All {total} emem MCP tools, generated from the registry: what question each answers and the exact call to make. The {len(core)}-tool core loop first. Every one callable with no key.">
<link rel=canonical href="https://emem.dev/tools">
<link rel=icon type="image/gif" href="/vortxgola.gif">
<link rel=preconnect href="https://fonts.googleapis.com"><link rel=preconnect href="https://fonts.gstatic.com" crossorigin>
<link rel=stylesheet href="https://fonts.googleapis.com/css2?family=JetBrains+Mono:ital,wght@0,200..800;1,200..800&family=Newsreader:ital,opsz,wght@0,6..72,300..700;1,6..72,300..700&display=swap">
<link rel=stylesheet href="/tokens.css">
<link rel=stylesheet href="/nav.css">
<style>
/* The muted levels clear WCAG AA on all three grounds. They did not: the
   light --mute-2 measured 2.35:1 and the dark one 2.26:1 on --paper-3, both
   carrying 10px text. Fixing web/tools.html alone would have been undone by
   the next deploy, because this generator rewrites that file. */
:root{{--paper:#f8f7f3;--paper-2:#f2f0ec;--paper-3:#eae8e3;--ink:#171613;--ink-2:#343330;--mute:#646360;--mute-2:#686664;--rule:#d3d1cb;--rule-strong:#a7a49e;--accent:#326234;--accent-bg:#d8efd8;}}
@media (prefers-color-scheme:dark){{:root{{--paper:#0e0d0b;--paper-2:#151411;--paper-3:#1e1d1a;--ink:#e9e8e4;--ink-2:#bfbdba;--mute:#878683;--mute-2:#888682;--rule:#2b2924;--rule-strong:#4a4742;--accent:#80cd82;--accent-bg:#132a14}}}}
*{{box-sizing:border-box}}body{{margin:0;background:var(--paper);color:var(--ink);font-family:var(--mono);line-height:1.55}}
.wrap{{max-width:1080px;margin:0 auto;padding:2.2rem 1.5rem 4rem}}
h1{{font-size:var(--t-xl);margin:.2rem 0 .4rem}}
.lede{{color:var(--ink-2);max-width:70ch;margin:0 0 .4rem}}
a{{color:var(--accent)}}
.lede a,p a{{text-decoration:underline;text-decoration-thickness:1px;text-underline-offset:.18em}}
.crumb{{font-size:var(--t-2xs)}}.crumb a{{color:var(--accent)}}
h2{{font-size:var(--t-md);margin:2rem 0 .2rem;border-bottom:1.6px solid var(--ink);padding-bottom:.3rem}}
.sub{{color:var(--mute);font-size:var(--t-xs);margin:.3rem 0 .6rem}}
.tool{{border:1px solid var(--rule);background:var(--paper-2);margin:.4rem 0}}
.tool summary{{display:flex;gap:.8rem;align-items:baseline;padding:.5rem .8rem;cursor:pointer;flex-wrap:wrap}}
.tool summary code{{color:var(--accent);font-weight:600;flex:0 0 auto}}
.tool .tt{{color:var(--ink-2);font-size:var(--t-xs)}}
.tool .tw{{color:var(--mute);font-size:var(--t-xs)}}
.tool p{{padding:0 .9rem;color:var(--ink-2);font-size:var(--t-xs);max-width:90ch}}
.tool pre{{margin:.4rem .9rem .8rem;padding:.6rem .7rem;background:var(--paper-3);border:1px solid var(--rule);overflow-x:auto;font-size:var(--t-2xs)}}
</style>
</head>
<body>
{site_nav}
<div class="wrap">
<p class="crumb"><a href="/">emem</a> / tools</p>
<h1>Every tool, generated from the registry</h1>
<p class="lede">All {total} MCP tools this responder dispatches, rendered from the same registry the server serves at <a href="/v1/tools">/v1/tools</a>, so this page cannot drift from the code. Every call below runs with no key. The same skills answer over the A2A protocol at <a href="/.well-known/agent-card.json">the agent card</a>.</p>
{body}
</div>
</body></html>
"""
    (REPO / "web" / "tools.html").write_text(page)
    print(f"wrote web/tools.html: {total} tools, {len(sections)} sections")
    return 0


if __name__ == "__main__":
    sys.exit(main())
