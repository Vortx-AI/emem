#!/usr/bin/env python3
"""Does every example we publish actually work when you run it?

    python3 scripts/example_check.py                  run them all
    python3 scripts/example_check.py --list           show what was found
    python3 scripts/example_check.py --origin http://127.0.0.1:5051

Why this exists
---------------
Documentation is a promise about behaviour, and nothing here was checking it.
`route_truth.py` asserts the routes ANSWER; that is a weaker claim than the
examples being runnable, and the gap between them is where a caller lives. A
worked example that 400s teaches an agent that the surface is broken, and it
teaches it at exactly the moment the agent is deciding whether to keep going.

The failure this is aimed at is not a 404. It is the plausible repair: a
published line that returns 200 and does not carry what the text around it
promised, so the caller follows the advice, gets a success, and is no further
forward. That shape has now been found twice on this pair of systems in one day
-- once where `GET /at` was advertised for counts that only `POST /at` returns,
and once where a refusal told a caller to rephrase a question whose place had
already been extracted and distrusted. Both answered 200. Neither helped.

What counts as a pass
---------------------
The call is made and the response is NOT an `emem.error.v1` envelope. A 2xx
carrying an error body is a failure here, because that is precisely the shape
that reads as success to everything except a human.

Placeholders are SKIPPED AND COUNTED, never silently dropped: `<cid>`, `{cell}`,
`YOUR_KEY`, an elided body. A harness that quietly ignores what it cannot run
reports a coverage it does not have, which is the same defect in the checker
that it exists to find in the docs.
"""
import argparse
import glob
import html
import json
import pathlib
import re
import shlex
import subprocess
import time
import sys

REPO = pathlib.Path(__file__).resolve().parent.parent

SOURCES = ["docs/**/*.md", "web/*.html", "SKILL.md", "README.md", "AGENTS.md"]

# Files that record what a call looked like ON A DATE, or reproduce another
# agent's signed words. Rewriting or re-running those is not a check, it is
# damage. Same list-with-reasons discipline as sync_counts.
FROZEN = (
    "collaboration-log.md",   # reproduces signed notes verbatim
    "audit-repro-",           # a dated reproduction of one run
    "whitepaper-v1",          # the DOI-cited archive
    "CHANGELOG.md",           # every entry is a past release
)

# `curl` at a line start, after whitespace, or straight after a tag. The last
# case is the one that was missing: a served page writes `<pre><code>curl ...`,
# so the character before the command is `>` and a pattern wanting whitespace
# matched nothing at all. Twelve commands on the homepage, none of them seen.
CURL_START = re.compile(r"(?:^|[\s>])(?P<c>curl\b)")

PLACEHOLDER = re.compile(r"<[a-z_]+>|\{[a-z_0-9]+\}|YOUR_|\.\.\.|…|\$\{|\bexample\.com\b|xxxx", re.I)


def sources():
    out = []
    for pat in SOURCES:
        for f in glob.glob(str(REPO / pat), recursive=True):
            name = pathlib.Path(f).name
            if any(fr in f for fr in FROZEN):
                continue
            out.append(pathlib.Path(f))
    return sorted(set(out))


# COMMANDS THE FILES DO NOT CONTAIN.
#
# The homepage's film strip is rendered by the responder: twelve frames, each
# with a command parameterised by that place's real cell64, none of which exists
# in web/index.html. A gate that reads the repository therefore checks every
# published example EXCEPT the ones a reader is most likely to try, which is the
# wrong twelve to miss.
#
# So the served page is scanned too, as one more source. It is fetched, not
# built, because what is being checked is what a reader receives.
SERVED = ["/"]


def served_sources(origin: str):
    """The pages whose commands only exist after the responder fills them in."""
    out = []
    for path in SERVED:
        code, body = _run_once(f"curl -s {origin}{path}", origin, timeout=60)
        if code and code.isdigit() and 200 <= int(code) < 300 and body:
            out.append((f"{origin}{path} (served)", body))
        else:
            # Undetermined, and said so: a served page that did not arrive is
            # not a page with no examples in it.
            print(f"  note: could not read {origin}{path} (status {code}), so any "
                  f"commands the responder injects there went unchecked")
    return out


def extract(path: pathlib.Path):
    """Every curl invocation, with the file and line it came from."""
    return _extract_from(path.read_text(encoding="utf-8", errors="ignore"))


def _extract_from(text: str):
    """The same, from markup already in hand (a served page, say)."""
    # Unescape with the stdlib rather than a hand-written table: my table
    # covered &#39; and not &#x27;, so every example in web/tools.html came out
    # carrying literal entity text and would have "failed" as a shell parse
    # error -- a checker reporting the docs broken when it was the reader.
    text = html.unescape(text)
    # FOLLOW BACKSLASH CONTINUATIONS. Stopping at the first newline reported 30
    # published examples as broken when every one of them was correct: the
    # extractor was cutting the command before its -H and -d lines, then
    # blaming the docs for the 400 that a bodyless POST obviously returns. A
    # harness that truncates its input finds faults it created.
    out = []
    lines = text.split("\n")
    i = 0
    while i < len(lines):
        if not CURL_START.search(lines[i]):
            i += 1
            continue
        start = i
        parts = []
        while i < len(lines):
            seg = lines[i].rstrip()
            parts.append(seg.rstrip("\\").strip())
            i += 1
            joined = " ".join(parts)
            # Continue while the line asks to (trailing backslash) OR while a
            # quote is still open -- several examples put a JSON body across
            # lines inside one pair of quotes with no backslashes at all, and
            # cutting at the newline turned a correct example into a bodyless
            # POST that obviously 400s.
            if seg.endswith("\\"):
                continue
            if joined.count("'") % 2 or joined.count('"') % 2:
                continue
            break
        joined = " ".join(p for p in parts if p).strip()
        # EVERY command on the line, not the first.
        #
        # A file puts one command per block, so taking the first `curl` and
        # stopping was right for files and wrong the moment a page was rendered
        # by the responder: the homepage's film strip is twelve commands on one
        # 14,900-character line, and eleven of them went unchecked while the
        # report said everything was checked.
        for m in CURL_START.finditer(joined):
            cmd = joined[m.start("c"):]
            # strip a trailing ``` or prose that shares the closing line
            cmd = cmd.split("```")[0].strip()
            # HTML pages carry the command inside <code>; the closing tags ride
            # along on the last line and became part of the URL.
            cmd = re.split(r"</code>|</pre>|</div>|<br\s*/?>", cmd)[0].strip()
            # SYNTAX HIGHLIGHTING IS NOT PART OF THE COMMAND.
            #
            # Three pages wrap every token of their examples in a <span> for
            # colour, so the command reads `curl</span> <span
            # class="tok-flag">-sX</span> ...`. The old pattern wanted whitespace
            # before `curl` and those start with `>`, so those examples were
            # never extracted and never checked -- silently, while the report
            # said every published example answers. They are extracted now, and
            # the tags come out here, which is what makes them runnable rather
            # than four new false findings.
            cmd = re.sub(r"<[^>]+>", "", cmd)
            cmd = html.unescape(cmd).strip()
            if "http" in cmd:
                out.append((start + 1, cmd))
    return out


def runnable(cmd: str):
    """(ok, why-not). Placeholders are a reason, not a silent skip."""
    if PLACEHOLDER.search(cmd):
        return False, "carries a placeholder"
    if "|" in cmd and "jq" not in cmd:
        return False, "piped into something that is not jq"
    # A step in a sequence, not a standalone example: it needs a value the
    # previous step produced. Counted, not silently dropped -- these are
    # exactly the examples a reader runs in order, and claiming to have
    # checked them would be worse than saying I did not.
    if re.search(r"\$\(|\$[A-Z_]{2,}", cmd):
        return False, "depends on a value from an earlier step"
    # A command that lives inside a SCRIPT STRING, which the page renders and
    # this file cannot. The giveaway is JavaScript escaping: `\'` is how a quote
    # survives inside a single-quoted JS literal and is never valid inside a
    # single-quoted shell word. Reconstructing it would mean writing a JS string
    # parser into a curl checker, and guessing at the unescaping is how a
    # checker starts inventing the commands it then reports on.
    if "\\'" in cmd or "\\n" in cmd:
        return False, "lives inside a script string; the page renders it, the file cannot"
    # A stream has no end, so "did it finish" is the wrong question for it.
    if " -N" in cmd or "/sse" in cmd or "text/event-stream" in cmd:
        return False, "a stream: correct behaviour is not to finish"
    # Reads its body from stdin, which a runner has no way to supply.
    if "@-" in cmd:
        return False, "reads its body from stdin"
    return True, ""


# A published example is judged by what the responder SAYS about it. When the
# responder says nothing -- a refused connection during a restart, or a 429
# because this suite and everything else on the box share one source address --
# the example was never exercised, and reporting it as broken sends somebody to
# fix a document that is correct. Both of those clear on their own within
# seconds, so they are waited out before they are believed.
# Same sizing as lib_patience, and for the same reason: the limiter's window is
# per minute and CI shares one source address with whatever else is running.
# This failed in CI at 08:07:54 on five seconds of patience while the same suite
# passed on the box minutes either side.
TRANSPORT_RETRIES = 5
TRANSPORT_PAUSE_S = (1.0, 4.0, 10.0, 25.0)


def run_one(cmd: str, origin: str, timeout: int = 90):
    for attempt in range(TRANSPORT_RETRIES):
        status, body = _run_once(cmd, origin, timeout)
        # 502/503/504 join 429 and the transport failures. All four say the
        # responder was busy or in front of something that was, and none of them
        # is a statement about the command. Both examples this caught were
        # correct: /v1/verify answered in 30 ms and the ask-with-a-model in 6 s
        # when asked again, while the suite that reported them broken was itself
        # the load. A persistent one still fails, because five attempts over
        # forty seconds is patience, not blindness.
        transient = (status is None or status in ("", "000", "429", "502", "503", "504"))
        if not transient or attempt + 1 == TRANSPORT_RETRIES:
            return status, body
        time.sleep(TRANSPORT_PAUSE_S[min(attempt, len(TRANSPORT_PAUSE_S) - 1)])
    return status, body


def _run_once(cmd: str, origin: str, timeout: int = 90):
    cmd = re.sub(r"https?://emem\.dev", origin, cmd)
    cmd = cmd.split("|")[0].strip()          # drop a trailing | jq
    try:
        argv = shlex.split(cmd)
    except ValueError as e:
        return None, f"unparseable: {e}"
    # force a body-only, status-reporting invocation
    argv = [a for a in argv if a not in ("-s", "--silent")]
    argv[1:1] = ["-s", "-o", "-", "-w", "\n__STATUS__%{http_code}"]
    try:
        # bytes, not text: several examples fetch a PNG or a CBOR body and
        # decoding those as utf-8 kills the run partway through, which looks
        # exactly like the harness passing everything before it.
        p = subprocess.run(argv, capture_output=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return None, f"timed out after {timeout}s"
    except OSError as e:
        return None, f"could not run: {e}"
    raw = p.stdout.decode("utf-8", "replace")
    body, _, status = raw.rpartition("\n__STATUS__")
    return (status.strip(), body)


def verdict(status, body):
    """A 2xx carrying an error envelope is a failure, not a pass."""
    if status is None:
        return "unrun"
    if not status.isdigit():
        return "no status"
    code = int(status)
    try:
        doc = json.loads(body)
    except Exception:
        doc = None
    if isinstance(doc, dict) and doc.get("schema") == "emem.error.v1":
        return f"{code} but the body is an emem.error.v1: {str(doc.get('message'))[:90]}"
    if isinstance(doc, dict) and isinstance(doc.get("error"), dict):
        return f"{code} but the body carries a JSON-RPC error: {str(doc['error'].get('message'))[:80]}"
    if code >= 400:
        return f"{code}"
    return "ok"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--origin", default="https://emem.dev")
    ap.add_argument("--list", action="store_true")
    args = ap.parse_args()

    found, skipped = [], []
    for path in sources():
        for line, cmd in extract(path):
            ok, why = runnable(cmd)
            (found if ok else skipped).append((path, line, cmd, why))

    # And the commands that only exist once the responder has filled the page
    # in. The homepage's film strip is twelve of them, each carrying a real
    # cell64, and none of them appears in any file in this repository.
    for label, body in served_sources(args.origin):
        for line, cmd in _extract_from(body):
            ok, why = runnable(cmd)
            (found if ok else skipped).append((label, line, cmd, why))

    if not found:
        print("MATCHED NOTHING: no runnable curl example found. Either the docs "
              "stopped carrying them or the extractor stopped matching; both are "
              "worth knowing and neither is a pass.")
        return 1

    if args.list:
        for path, line, cmd, _ in found:
            print(f"  {path.relative_to(REPO)}:{line}  {cmd[:110]}")
        print(f"\n  {len(found)} runnable, {len(skipped)} skipped")
        return 0

    failures = []
    for path, line, cmd, _ in found:
        status, body = run_one(cmd, args.origin)
        v = verdict(status, body)
        if v != "ok":
            failures.append((path, line, cmd, v))

    print(f"example check: {len(found)} runnable example(s) across "
          f"{len({p for p, _, _, _ in found})} file(s); {len(skipped)} skipped "
          f"for placeholders or pipes")
    if failures:
        print("\nA PUBLISHED EXAMPLE DOES NOT DO WHAT IT SAYS:")
        for path, line, cmd, v in failures:
            print(f"  {path.relative_to(REPO)}:{line}  -> {v}")
            print(f"      {cmd[:150]}")
        return 1
    print("Every runnable example we publish answers, and none of them answers "
          "with an error envelope.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
