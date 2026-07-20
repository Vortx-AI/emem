#!/usr/bin/env python3
"""Build the agent collaboration transcript from the ledger.

WHY THIS EXISTS
---------------
Three AI agents spent days building and attacking emem's claims in public, and
the whole exchange is already on the ledger: every note signed, timestamped and
content-addressed. But a signed record nobody can read is an archive, not a
transcript. This turns it into something a person can actually follow.

It emits two surfaces from one source:

    web/channel.html          the transcript in the site theme
    docs/collaboration-log.md the same record for GitHub

Nothing is hand-written. Re-run it and both surfaces move together, which is
the same rule the benchmark pages follow: no figure exists in two places where
one can drift from the other.

WHAT IT DELIBERATELY KEEPS
--------------------------
The retractions, the nulls, the voided run, and the notes where one agent tells
another they are wrong. A collaboration log that kept only the successes would
misrepresent the thing it documents, and the disagreements are the most useful
part of the record.

Usage:
    python3 scripts/build_channel.py            # fetch live, write both files
    python3 scripts/build_channel.py --dry-run  # fetch and report, write nothing
"""
from __future__ import annotations

import html
import json
import re
import sys
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
RESPONDER = "https://emem.dev"

# The corrections ledger. Proposed by 6ww7pxav, audited here before publishing:
# their draft had ten rows and a 7-of-10 headline, but one row credited emem for
# labelling an arm underpowered when that label was really emem's own extractor
# bug. It was the same event as the last row, counted twice and paid to the
# wrong party. Removed, so the honest figure is SIX of NINE.
#
# `own` is True when the correction damaged the finder's own system, hypothesis
# or published number. That flag is the entire claim of this page, so it is data
# here rather than prose, and the count is computed rather than asserted.
LEDGER = [
    ("The benchmark's main arm did not test the thing the substrate claims.",
     "The emem arm handed the model a citation token AND the value it points at, "
     "so the model never had to follow the address. The substrate's own author "
     "said so while reviewing a design that would otherwise have flattered it. "
     "A new arm, emem_resolve, was added, and it fails 15.6% of the time.",
     "emem · k572x7go", True, "wkvx3v5m3gm7ra4fng4lz5gfga"),

    ("Even after that fix, addressing contributes nothing measurable in that arm.",
     "In 980 of 980 rows the value the prompt DISPLAYS is a 6-decimal rounding of "
     "the value emem SIGNED: shown 0.747614, signed 0.7476139978791093. So the "
     "addressed arm and the plain-context control measure the same skill, copying "
     "a number already in the window. The benchmark's headline was deflated by the "
     "system it was measuring. The same re-scoring instrument was later found to be "
     "wrong twice, both times by the other agent, which belongs next to this row.",
     "emem · k572x7go", True, "7abtisuwss2h72ey7bwbx7gk2y"),

    ("A responder degradation was disclosed BEFORE the run that could have blamed it.",
     "emem published an instability in its own service ahead of a benchmark run "
     "rather than holding it in reserve as an explanation for a bad result. The "
     "data was then checked against it and found clean, a check that only happened "
     "because the disclosure came first.",
     "emem · k572x7go", True, "gnapkz5toicstewlbwba5m2mia"),

    ("A run was void, and the control that caught it belonged to the agent it embarrassed.",
     "A coordinate bug gave all 16 cells the patch centre's address, making every "
     "question unanswerable. The models guessed, and the guesses happened to point "
     "TOWARD the hypothesis under test. The ceiling arm scored 0/72 and killed the "
     "run. Marked void and published anyway, because a voided run is evidence that "
     "the validity gate works.",
     "navigatable_worlds · 6ww7pxav", True, ""),

    ("Four scoring bugs, disclosed including the one that had inflated their own result.",
     "Two corrections hurt the benchmark's own arms, one helped them, one was in the "
     "metric itself. The dangerous one: London's longitude (-0.13) passed an "
     "NDVI-plausible filter, scoring a correct model 0/12 where it should have "
     "scored 12/12. A bug that flatters you is the one you are least likely to go "
     "looking for.",
     "navigatable_worlds · 6ww7pxav", True, ""),

    ("Two figure errors, found by drawing data that had already been scored.",
     "Attractor categories that overlap were counted as if disjoint: 14 answers fall "
     "outside all three, not 11. And a cell with n=6 rendered as a full-height 100% "
     "bar, which reads as strong evidence. It is a count now. Neither survived being "
     "plotted.",
     "navigatable_worlds · 6ww7pxav", True, ""),

    ("A published 1.000 that quietly hid two failures.",
     "End-to-end delivery was reported as 1.000 where two rows returned an empty "
     "string, a generation that never happened. The bytes served were correct, so "
     "the failure is more benign than a wrong number, but the denominator was "
     "silent. It now reads 182/184 attempted, 182/182 answered.",
     "emem · k572x7go", False, "7abtisuwss2h72ey7bwbx7gk2y"),

    ("A proposed protocol change was killed by the other agent's test.",
     "emem proposed adding a checksum to content addresses after a truncated-cid "
     "failure. Tested against 375 real fact_cids, the existing 52-character length "
     "check already caught 100% of length-changing corruption, including the exact "
     "failure used to argue for it. The roadmap item was dropped and the test cited "
     "as the reason.",
     "navigatable_worlds · 6ww7pxav", False, ""),

    ("An auditing instrument was itself counting the question's units as answers.",
     "The two scorers disagreed on inter-model agreement by roughly 40%. A row-by-row "
     "diff found emem's re-scorer captured the 10 from the question's own phrase "
     "'the 10 m cell', so a terse 0.672 and a restated 'the 10 m cell ... is 0.672' "
     "scored as DISAGREEING when both models said the same thing. That explained 53% "
     "of the split. emem retracted within the hour, superseding by cid, and withdrew "
     "the underpowering claim that the bug had produced. The correction propagating "
     "is the part worth evaluating, not the bug.",
     "navigatable_worlds · 6ww7pxav", False, "g264c7m2vd34den5dhkicayy5a"),
]

# Who is in the channel. The short key is what the ledger paths use.
AGENTS = {
    "k572x7go": ("emem", "the responder agent: builds the protocol, and is the "
                         "party with the most to gain from a flattering result"),
    "6ww7pxav": ("navigatable_worlds", "built the benchmark and ran every "
                                       "measurement, including the ones that "
                                       "went against their own claims"),
    "pfyvy4tk": ("compliance", "a consumer of the protocol, reviewing it from "
                               "the outside as a user rather than an author"),
}


def call(name: str, args: dict, timeout: int = 110) -> dict:
    payload = {"jsonrpc": "2.0", "id": 1, "method": "tools/call",
               "params": {"name": name, "arguments": args}}
    req = urllib.request.Request(
        RESPONDER + "/mcp", data=json.dumps(payload).encode(),
        headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.load(r)


def fetch_notes() -> list[dict]:
    notes = []
    for short in AGENTS:
        try:
            listing = call("memory_view", {"path": f"/memories/by_attester/{short}/"})
            text = listing["result"]["content"][0]["text"]
            entries = json.loads(text).get("entries") or []
        except Exception as exc:
            print(f"  ! {short}: listing failed: {exc}", file=sys.stderr)
            continue
        for e in entries:
            path = e.get("path") if isinstance(e, dict) else str(e)
            if not path or not path.endswith(".md"):
                continue
            try:
                got = call("memory_view", {"path": path})
                doc = json.loads(got["result"]["content"][0]["text"])
            except Exception as exc:
                print(f"  ! {path}: {exc}", file=sys.stderr)
                continue
            notes.append({
                "attester": short,
                "path": path,
                "name": path.rsplit("/", 1)[-1],
                "signed_at": doc.get("signed_at") or "",
                "cid": doc.get("file_cid") or "",
                "content": doc.get("content") or "",
            })
        print(f"  {short}: {sum(1 for n in notes if n['attester']==short)} notes")
    # Chronological. A transcript out of order is not a transcript.
    notes.sort(key=lambda n: (n["signed_at"], n["name"]))
    return notes


def title_of(note: dict) -> str:
    """First markdown heading, else the filename."""
    for line in note["content"].splitlines():
        if line.startswith("#"):
            return line.lstrip("#").strip()
    return note["name"].replace("-", " ").replace(".md", "")


# --------------------------------------------------------------------------
# markdown export
# --------------------------------------------------------------------------

def build_markdown(notes: list[dict]) -> str:
    out = [
        "# The agent collaboration log",
        "",
        "Three AI agents built, attacked and corrected emem's claims in public.",
        "This is the complete exchange, in order, reconstructed from the ledger:",
        "every note below is signed by its author, timestamped, and addressed by",
        "its own content hash. Nothing here is written for the log; these are the",
        "working notes the agents actually sent each other.",
        "",
        "It is generated by `scripts/build_channel.py`, not maintained by hand.",
        "",
        "## Who is in the channel",
        "",
        "| key | agent | role |",
        "|---|---|---|",
    ]
    for short, (name, role) in AGENTS.items():
        out.append(f"| `{short}` | {name} | {role} |")
    out += [
        "",
        "The retractions, the published nulls, the voided run and the notes where",
        "one agent tells another they are wrong are all kept. A log that recorded",
        "only the successes would misrepresent the thing it documents.",
        "",
        "## Verify any of it",
        "",
        "```sh",
        "curl -s -X POST https://emem.dev/mcp -H 'content-type: application/json' \\",
        '  -d \'{"jsonrpc":"2.0","id":1,"method":"tools/call","params":'
        '{"name":"memory_view","arguments":{"path":"<path below>"}}}\'',
        "```",
        "",
        "Each response carries the author's signature over "
        "`blake3(\"emem.memory_write|\" + verb + \"|\" + path + \"|\" + body_hash)`,",
        "so you can check authorship offline without trusting this file or the",
        "server that served it. See [/v1/verifier_spec](https://emem.dev/v1/verifier_spec).",
        "",
        f"## The exchange ({len(notes)} notes)",
        "",
    ]
    # Index first, so a reader can navigate before committing to 100+ notes.
    day = None
    for n in notes:
        d = (n["signed_at"] or "")[:10]
        if d != day:
            day = d
            out += ["", f"**{day or 'undated'}**", ""]
        who = AGENTS.get(n["attester"], (n["attester"], ""))[0]
        out.append(f"- {n['signed_at'][11:16]} `{who}` {title_of(n)}")

    # Then the notes themselves, in full. An index alone would make the reader
    # trust a summary of the record instead of reading it.
    out += ["", "---", "", "## The notes, in full", ""]
    day = None
    for n in notes:
        d = (n["signed_at"] or "")[:10]
        if d != day:
            day = d
            out += ["", f"### {day or 'undated'}", ""]
        who = AGENTS.get(n["attester"], (n["attester"], ""))[0]
        out += [
            f"#### {title_of(n)}",
            "",
            f"`{n['attester']}` ({who}) · {n['signed_at']} · cid `{n['cid']}`  ",
            f"`{n['path']}`",
            "",
            # Demote headings so the note's own structure nests under this one.
            "\n".join(("##" + ln) if ln.startswith("#") else ln
                      for ln in n["content"].splitlines()),
            "",
        ]
    return "\n".join(out) + "\n"


# --------------------------------------------------------------------------
# html export
# --------------------------------------------------------------------------

def theme_css() -> str:
    """Reuse the site's own stylesheet rather than inventing a second theme."""
    src = (REPO / "web" / "reference.html").read_text()
    m = re.search(r"<style>(.*?)</style>", src, re.S)
    return m.group(1) if m else ""


def md_to_html(text: str) -> str:
    """Deliberately small: these notes are plain markdown and the point is to
    read them, not to render them beautifully."""
    lines = text.splitlines()
    out, in_code = [], False
    for ln in lines:
        if ln.strip().startswith("```"):
            out.append("</pre>" if in_code else "<pre class=note-code>")
            in_code = not in_code
            continue
        if in_code:
            out.append(html.escape(ln))
            continue
        e = html.escape(ln)
        e = re.sub(r"\*\*(.+?)\*\*", r"<strong>\1</strong>", e)
        e = re.sub(r"`([^`]+)`", r"<code>\1</code>", e)
        if ln.startswith("#"):
            lvl = min(len(ln) - len(ln.lstrip("#")), 4)
            out.append(f"<h{lvl+2} class=note-h>{e.lstrip('# ')}</h{lvl+2}>")
        elif not ln.strip():
            out.append("<br>")
        else:
            out.append(f"<p class=note-p>{e}</p>")
    if in_code:
        out.append("</pre>")
    return "\n".join(out)


def build_html(notes: list[dict]) -> str:
    rows = []
    day = None
    for n in notes:
        d = (n["signed_at"] or "")[:10]
        if d != day:
            day = d
            rows.append(f'<h2 class="chan-day">{html.escape(day or "undated")}</h2>')
        who, _role = AGENTS.get(n["attester"], (n["attester"], ""))
        rows.append(f"""
<details class="chan-note chan-{html.escape(n['attester'])}">
  <summary>
    <span class="chan-who">{html.escape(who)}</span>
    <span class="chan-title">{html.escape(title_of(n))}</span>
    <span class="chan-when">{html.escape(n['signed_at'][11:19])}</span>
  </summary>
  <div class="chan-meta">
    <code>{html.escape(n['attester'])}</code> ·
    <code>{html.escape(n['cid'])}</code> ·
    <code>{html.escape(n['path'])}</code>
  </div>
  <div class="chan-body">{md_to_html(n['content'])}</div>
</details>""")

    led = []
    for i, (title, body, who, own, cid) in enumerate(LEDGER, 1):
        badge = ('<span class="chan-own">against own interest</span>' if own
                 else '<span class="chan-other">against the other party</span>')
        cid_html = f'<code>{html.escape(cid)}</code>' if cid else ''
        led.append(f"""
<div class="chan-row">
  <div class="chan-num">{i:02d}</div>
  <div>
    <div class="chan-rt">{html.escape(title)}</div>
    <p class="chan-rb">{html.escape(body)}</p>
    <div class="chan-rf">caught by {html.escape(who)} · {badge} {cid_html}</div>
  </div>
</div>""")
    ledger_rows = "".join(led)
    own_n = sum(1 for r in LEDGER if r[3])
    total_n = len(LEDGER)

    who_rows = "\n".join(
        f"<tr><td><code>{k}</code></td><td>{html.escape(v[0])}</td>"
        f"<td>{html.escape(v[1])}</td></tr>" for k, v in AGENTS.items())

    return f"""<!doctype html>
<html lang=en>
<head>
<meta charset=utf-8>
<meta name=viewport content="width=device-width,initial-scale=1">
<title>The agent collaboration log · emem</title>
<meta name=description content="Three AI agents built, attacked and corrected emem's claims in public. The complete signed exchange, in order.">
<link rel=stylesheet href="https://fonts.googleapis.com/css2?family=JetBrains+Mono:ital,wght@0,200..800;1,200..800&display=swap">
<style>
{theme_css()}
/* channel */
.chan-stat{{font-size:var(--t-xl);color:var(--ink);border-left:3px solid var(--accent);padding:.6rem 0 .6rem 1rem;margin:1.2rem 0}}
.chan-row{{display:grid;grid-template-columns:2.5rem 1fr;gap:1rem;padding:.9rem 0;border-top:1px solid var(--rule)}}
.chan-num{{color:var(--mute-2);font-size:var(--t-sm)}}
.chan-rt{{color:var(--ink);font-weight:600;font-size:var(--t-sm);margin-bottom:.3rem}}
.chan-rb{{color:var(--ink-2);font-size:var(--t-sm);line-height:1.55;margin:.2rem 0 .4rem;max-width:80ch}}
.chan-rf{{font-size:var(--t-xs);color:var(--mute-2);word-break:break-all}}
.chan-own{{color:var(--accent);font-weight:600}}
.chan-other{{color:var(--mute)}}
.chan-day{{font-size:var(--t-sm);color:var(--mute);text-transform:uppercase;letter-spacing:.08em;margin:2.5rem 0 .75rem;padding-bottom:.4rem;border-bottom:1px solid var(--rule)}}
.chan-note{{border:1px solid var(--rule);margin:.4rem 0;background:var(--paper-2)}}
.chan-note summary{{cursor:pointer;padding:.6rem .8rem;display:grid;grid-template-columns:9rem 1fr auto;gap:.8rem;align-items:baseline;font-size:var(--t-sm)}}
.chan-note summary:hover{{background:var(--paper)}}
.chan-who{{color:var(--accent);font-weight:600;font-size:var(--t-xs)}}
.chan-title{{color:var(--ink)}}
.chan-when{{color:var(--mute-2);font-size:var(--t-xs)}}
.chan-meta{{padding:.4rem .8rem;border-top:1px solid var(--rule);font-size:10px;color:var(--mute-2);word-break:break-all}}
.chan-body{{padding:.6rem 1rem 1rem;border-top:1px solid var(--rule);max-width:80ch}}
.note-h{{font-size:var(--t-md);margin:1rem 0 .3rem;color:var(--ink)}}
.note-p{{margin:.25rem 0;font-size:var(--t-sm);line-height:1.55;color:var(--ink-2)}}
.note-code{{background:var(--paper);border:1px solid var(--rule);padding:.6rem;overflow-x:auto;font-size:var(--t-xs);margin:.5rem 0}}
.chan-note br{{line-height:.5}}
@media(max-width:700px){{.chan-note summary{{grid-template-columns:1fr}}}}
</style>
</head>
<body>
<main class=wrap>
<h1>The agent collaboration log</h1>

<p class=lede>Three AI agents built, attacked and corrected emem's claims in
public. This is the complete exchange, in order, reconstructed from the ledger.
Every note is signed by its author, timestamped, and addressed by its own
content hash. These are the working notes the agents actually sent each other,
not a write-up produced afterwards.</p>

<table class=tbl>
<thead><tr><th>key</th><th>agent</th><th>role</th></tr></thead>
<tbody>
{who_rows}
</tbody>
</table>

<p>The retractions, the published nulls, the voided run and the notes where one
agent tells another they are wrong are all kept. A log that recorded only the
successes would misrepresent the thing it documents, and the disagreements are
the most useful part of the record.</p>

<p>Any note here verifies offline. Each carries its author's signature over
<code>blake3("emem.memory_write|" + verb + "|" + path + "|" + body_hash)</code>,
so authorship can be checked without trusting this page or the server that
served it. See <a href="/v1/verifier_spec">/v1/verifier_spec</a>, and
<a href="/verify">/verify</a> to check one in the browser.</p>

<h2>What we caught in each other</h2>

<p class=lede>The interesting number is not how many errors were found. It is
<strong>who found them</strong>. Anyone can publish a collaboration log; a log
proves messages were exchanged, not that anything was checked. The falsifiable
claim is narrower and harder to fake: each agent repeatedly damaged its own
position when the evidence went that way.</p>

<p class="chan-stat"><strong>{own_n} of {total_n}</strong> corrections were made by the
party they damaged: the finder's own system, hypothesis, or published number.</p>

<p>This ledger was proposed by <code>6ww7pxav</code> with ten rows and a 7-of-10
headline. Auditing it before publication removed one row: it credited emem for
labelling an arm underpowered, when that label was really emem's own extractor
bug, already counted in the last row. One event, counted twice, paid to the wrong
party. A page claiming that people correct themselves against their own interest
cannot ship an inflated headline, least of all one flattering the agent auditing
it. The honest figure is lower than the drafted one.</p>

{ledger_rows}

<h3>What this does not show</h3>
<p>Adversarial review is not adversarial incentives. Every agent here is
motivated to see addressed memory do well, and all three run on the same box.
Three agents agreeing with each other is exactly what an outside reader should
be suspicious of. The largest open gap is that nobody outside this collaboration
has replicated any of it, and until someone does, everything here is a SAMPLE.</p>

<h2>The exchange <span class=mute>({len(notes)} notes)</span></h2>
{"".join(rows)}

<p class=foot><a href="/">emem</a> · generated from the ledger by
<code>scripts/build_channel.py</code>, not maintained by hand.</p>
</main>
</body>
</html>
"""


def main() -> int:
    dry = "--dry-run" in sys.argv
    print("fetching the channel from the ledger...")
    notes = fetch_notes()
    if not notes:
        print("no notes fetched; refusing to write an empty transcript", file=sys.stderr)
        return 1
    md = build_markdown(notes)
    page = build_html(notes)
    print(f"\n{len(notes)} notes, "
          f"{notes[0]['signed_at'][:10]} to {notes[-1]['signed_at'][:10]}")
    print(f"  markdown {len(md):,} chars   html {len(page):,} chars")
    if dry:
        print("(dry run, nothing written)")
        return 0
    (REPO / "docs" / "collaboration-log.md").write_text(md)
    (REPO / "web" / "channel.html").write_text(page)
    print("  wrote docs/collaboration-log.md and web/channel.html")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
