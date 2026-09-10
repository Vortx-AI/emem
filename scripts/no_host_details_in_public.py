#!/usr/bin/env python3
"""No keyless surface leaks a private address or a host filesystem path.

Three sessions found the same class on 2026-09-10 from three directions, and
two of the instances were addresses:

  * `materialize_notes[].reason` carried `connect "/run/emem/jepa_sidecar.sock":
    No such file or directory (os error 2)` to anyone who asked a place
    question. A caller cannot act on an errno and should not be shown our
    filesystem.
  * `counted_from.detector.backend_url` carried `http://127.0.0.1:5019`,
    republished from an upstream whose object we copied wholesale.

It also reads served text for indentation folded into a sentence, which is the
wire-side half of scripts/no_padded_prose.py. That one reads the source and is
stronger, because it covers every path whether or not a caller can reach it.
This one covers what the source cannot see: a string assembled at runtime.

Neither was written on purpose. Both arrived by copying something inward-facing
into something outward-facing, which is how this always happens, so the check
is on the OUTPUT rather than on anyone's care.

Surfaces are taken from /openapi.json -- every GET that needs no parameters --
plus the well-known documents, so a route added tomorrow is covered without
anyone remembering to add it here.

WHAT THIS DOES NOT COVER, so that a clean run is not read as more than it is:
POST bodies and MCP tool results, which is where both of the instances above
actually appeared. A GET sweep would not have caught either of them. It catches
the documents, and the documents are where a leak lives longest because nobody
re-reads them. The two named cases are covered by tests beside the code that
produced them.

DECLARED EXCEPTIONS live in docs/registries/public-host-details.json, next to
this, with a reason each. An exception is a decision someone can read and
argue with; a pattern quietly tuned until the check goes green is not.

  python3 scripts/no_host_details_in_public.py
  python3 scripts/no_host_details_in_public.py --origin http://127.0.0.1:5051

Exit 0 clean, 1 a leak, 2 the responder did not answer, 3 a fault on our side.
"""
from __future__ import annotations

import argparse
import json
import pathlib
import re
import sys
import urllib.request

REPO = pathlib.Path(__file__).resolve().parent.parent
EXCEPTIONS = REPO / "docs" / "registries" / "public-host-details.json"
UA = "emem-no-host-details/1 (+https://emem.dev)"

# Private and loopback addresses, and paths that only mean something on the
# machine that wrote them.
PATTERNS = [
    # Indentation folded into a sentence. scripts/no_padded_prose.py catches
    # this in the source and is the stronger check, because it covers every
    # path whether or not anyone can reach it. It cannot see a string ASSEMBLED
    # at runtime -- a format! that pads, a join over indented parts -- and this
    # can, because it reads what was served rather than what was written. The
    # geo.qa agent made the argument for the pair: they widened their own
    # confirmation from one field to the whole response body because "the other
    # twenty are on paths I cannot reach from a single ask", and a body-wide
    # regex costs nothing and covers the ones neither of us thought to call.
    ("indentation in a sentence", r"\S {8,}\S"),
    ("loopback", r"\b(?:127\.\d{1,3}\.\d{1,3}\.\d{1,3}|localhost|0\.0\.0\.0|\[::1\])\b"),
    ("rfc1918", r"\b(?:10\.\d{1,3}|192\.168|172\.(?:1[6-9]|2\d|3[01]))\.\d{1,3}\.\d{1,3}\b"),
    ("container-host", r"host\.docker\.internal"),
    ("host path", r"(?<![\w/])/(?:home|root|srv|var/lib|run|tmp)/[\w./-]{3,}"),
    ("unix socket", r"[\w./-]*\.sock\b"),
    ("errno", r"\bos error \d+\b"),
]
COMPILED = [(name, re.compile(p)) for name, p in PATTERNS]

# Ordinary prose and spec text that these patterns match and that is not a
# leak. Narrow on purpose: each is a literal, not a wildcard.
NOT_A_LEAK = (
    "127.0.0.1:5051",   # the self-host docs' own bind example
    "localhost:5051",
    "http://localhost",  # OAuth/redirect examples in discovery documents
)


# Per-request, not per-run. Some zero-parameter GETs on this surface scan the
# corpus and take minutes, and a scan of every public answer must not become a
# load test. A surface that does not answer inside this is UNREAD, and the
# summary says so rather than counting it as clean.
REQUEST_TIMEOUT_S = 15


def get(url: str) -> tuple[int, str]:
    req = urllib.request.Request(url, headers={"user-agent": UA})
    try:
        with urllib.request.urlopen(req, timeout=REQUEST_TIMEOUT_S) as r:
            return r.status, r.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as e:
        return e.code, ""
    except Exception:  # noqa: BLE001
        return 0, ""


def surfaces(origin: str, oa: dict) -> list[str]:
    """Keyless GETs, from the document rather than from a list here."""
    out = [
        "/.well-known/agent-card.json",
        "/.well-known/mcp.json",
        "/.well-known/emem.json",
        "/.well-known/did.json",
        "/llms.txt",
    ]
    for path, item in (oa.get("paths") or {}).items():
        op = item.get("get")
        if not isinstance(op, dict):
            continue
        if "{" in path:
            continue
        needs = [p for p in (op.get("parameters") or [])
                 if isinstance(p, dict) and p.get("required")]
        if needs:
            continue
        out.append(path)
    return [origin + p for p in dict.fromkeys(out)]


# Aligned columns are deliberate, here as in the source scanner: a route table
# in a help payload, a label-and-value report. Same three exemptions, so the two
# checks agree about what is damage rather than one reporting what the other
# allows.
VERB = re.compile(r"^\s*(GET|POST|PUT|DELETE|PATCH|HEAD)\b")
COLUMN = re.compile(r" {8,}[,|{]")


def findings(url: str, body: str) -> list[tuple[str, str]]:
    """Scan the STRING VALUES of a JSON body, not the JSON text.

    Reading the raw text would report the pretty-printer's own indentation as
    padding on every response that is formatted, and report nothing at all on
    the ones that are not. The values are what a caller reads.
    """
    hits = []

    def look(text: str):
        for name, rx in COMPILED:
            for m in rx.finditer(text):
                found = m.group(0)
                if any(ok in found for ok in NOT_A_LEAK):
                    continue
                if name == "indentation in a sentence" and (
                    "\n" in text or VERB.match(text) or COLUMN.search(text)
                ):
                    continue
                hits.append((name, found if name != "indentation in a sentence"
                             else text[:110]))

    try:
        doc = json.loads(body)
    except json.JSONDecodeError:
        look(body)          # llms.txt and friends are not JSON
        return hits

    def walk(node):
        if isinstance(node, dict):
            for v in node.values():
                walk(v)
        elif isinstance(node, list):
            for v in node:
                walk(v)
        elif isinstance(node, str):
            look(node)

    walk(doc)
    return hits


# One case per pattern that MUST fire, and four that must stay silent.
#
# The first version of this self-tested the indentation pattern and nothing
# else, which is the fault the geo.qa agent named in their own scan: a pattern
# proved when it was written is proved for that day and nothing after. Six of
# the seven here were in exactly that state, including the two that started
# this -- the socket path in an error and the loopback backend_url -- and a
# regex that has stopped matching produces output identical to a clean sweep.
#
# Each entry is a real value or a close copy of one, not a synthetic that only
# the pattern's author would think to write.
SELF_TEST = [
    # (label, one JSON string value, which pattern must fire, or None for silence)
    ("the note emem served on 2026-09-10",
     "sidecar unavailable: this responder does not run the GPU inference sidecar "
     "this band needs.                  It is an optional extension.",
     "indentation in a sentence"),
    ("a pad assembled at runtime, which the source scanner cannot see",
     "the value is 42           and the unit is metres",
     "indentation in a sentence"),
    ("the backend_url we were republishing",
     "http://127.0.0.1:5019", "loopback"),
    ("a private-range address",
     "the encoder answers at 10.0.3.14:5019", "rfc1918"),
    ("a container host name",
     "upstream http://host.docker.internal:5019/predict", "container-host"),
    ("the socket path that started this",
     "connect /run/emem/jepa_sidecar.sock failed", "host path"),
    ("a socket by name alone",
     "bind emem-guard.sock before starting", "unix socket"),
    ("the errno beside it",
     "No such file or directory (os error 2)", "errno"),
    # Silence.
    ("an aligned route table", "GET  /v1/bands          , band catalogue", None),
    ("a label and value report line", "fired rate            {0.4231}", None),
    ("a block laid out on purpose",
     "type    | needs\nwhere_is        | description", None),
    ("ordinary prose",
     "Ground cameras see what a satellite cannot: people and vehicles.", None),
]


def self_test() -> list[str]:
    """Run before the network, so a dead pattern cannot pass as a clean sweep.

    Verified by sabotage: replace the loopback pattern with one that cannot
    match and this reports `pattern went silent on: the backend_url we were
    republishing` and refuses, rather than printing clean over sixty-nine
    surfaces.
    """
    import json as _json
    wrong = []
    for label, value, want in SELF_TEST:
        names = {n for n, _ in findings("self-test", _json.dumps({"v": value}))}
        if want is None:
            if names:
                wrong.append(f"pattern fired on something deliberate: {label} "
                             f"({', '.join(sorted(names))})")
        elif want not in names:
            wrong.append(f"pattern went silent on: {label} "
                         f"(wanted `{want}`, got {sorted(names) or 'nothing'})")
    return wrong


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--origin", default="https://emem.dev")
    a = ap.parse_args()
    origin = a.origin.rstrip("/")

    wrong = self_test()
    if wrong:
        print(f"SELF-TEST FAILED, {len(wrong)} of {len(SELF_TEST)} case(s). "
              f"A sweep with these patterns would be silence, not a clean result:")
        for w in wrong:
            print("  ", w)
        return 3

    st, body = get(origin + "/openapi.json")
    if st != 200 or not body:
        print(f"could not read {origin}/openapi.json (http {st or 'no response'})")
        print("Undetermined, not clean: the surface list comes from that document.")
        return 2
    try:
        oa = json.loads(body)
    except json.JSONDecodeError as e:
        print(f"/openapi.json did not parse: {e}")
        return 3

    allowed: dict[str, list[str]] = {}
    if EXCEPTIONS.exists():
        try:
            allowed = json.loads(EXCEPTIONS.read_text(encoding="utf-8")).get("allow") or {}
        except Exception as e:  # noqa: BLE001
            print(f"{EXCEPTIONS.name} did not parse: {e}")
            return 3

    urls = surfaces(origin, oa)
    read = 0
    unread: list[str] = []
    leaks: list[str] = []
    excused = 0
    for url in urls:
        st, body = get(url)
        if st != 200 or not body:
            unread.append(f"{url[len(origin):]} (http {st or 'no answer'})")
            continue
        read += 1
        path = url[len(origin):]
        for name, text in findings(url, body):
            if any(text.startswith(a) or a in text for a in allowed.get(path, [])):
                excused += 1
                continue
            leaks.append(f"{path}: {name} {text!r}")

    if read < 5:
        print(f"read {read} surface(s); the responder answered almost nothing, so "
              f"a clean result here would mean nothing")
        return 2

    print(f"{read} of {len(urls)} keyless surface(s) read, "
          f"{excused} declared exception(s) matched")
    if unread:
        # Stated, not swallowed. "No leaks found" over a surface nobody read is
        # the same sentence as "no leaks", and they are not the same result.
        print(f"  {len(unread)} not read, so not checked: "
              f"{', '.join(unread[:8])}{' ...' if len(unread) > 8 else ''}")
    if leaks:
        print(f"\n{len(leaks)} host detail(s) in a public answer:")
        for row in sorted(set(leaks))[:40]:
            print("  ", row)
        print(f"\nRemove it, or declare it in {EXCEPTIONS.relative_to(REPO)} with a reason.")
        return 1
    print("nothing that only means something on this machine.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
