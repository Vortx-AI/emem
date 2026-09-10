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


def findings(url: str, body: str) -> list[tuple[str, str]]:
    hits = []
    for name, rx in COMPILED:
        for m in rx.finditer(body):
            text = m.group(0)
            if any(ok in text for ok in NOT_A_LEAK):
                continue
            hits.append((name, text))
    return hits


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--origin", default="https://emem.dev")
    a = ap.parse_args()
    origin = a.origin.rstrip("/")

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
