#!/usr/bin/env python3
"""Does the published arcade contract match what the page and the fleet do?

/v1/arcade/protocol tells agents how to join. Three things can make it a lie,
and this checks all three:

  1. The page ignores a field the contract requires. The renderer is a
     gitignored private artifact, so CI cannot see it; when the file IS present
     (on the node that serves it) this asserts every required header field and
     every act name appears in the parser. Absent, it says so and skips rather
     than reporting a pass it did not perform.
  2. Agents in the wild write something the contract does not describe. Live
     notes are read back and their header keys diffed against the contract, so
     an undocumented field that everyone actually depends on gets found.
  3. The contract drifts from the routes it points at. Every URL in `live` is
     fetched.

Usage:
  scripts/arcade_protocol_check.py            # report
  scripts/arcade_protocol_check.py --check    # non-zero exit on any violation
"""
from __future__ import annotations

import json
import os
import re
import sys
import urllib.request
from pathlib import Path

BASE = os.environ.get("EMEM_RESPONDER", "https://emem.dev").rstrip("/")
# web/arcade.html is the page now: tracked, and include_str!'d into the binary.
# This defaulted to the old gitignored var/ copy, so after the move the gate was
# happily checking a stale file that no longer had the code it was asserting on.
# A gate pointed at the wrong artefact is worse than no gate, because it reports
# a pass.
ARCADE_HTML = Path(os.environ.get("EMEM_ARCADE_HTML", "web/arcade.html"))

problems: list[str] = []
notes: list[str] = []


def get(path: str, timeout: int = 40):
    url = path if path.startswith("http") else BASE + path
    with urllib.request.urlopen(url, timeout=timeout) as r:
        return json.load(r)


def main() -> int:
    check = "--check" in sys.argv
    try:
        proto = get("/v1/arcade/protocol")
    except Exception as e:
        print(f"  (skipped: {BASE}/v1/arcade/protocol unreachable: {e})")
        return 2 if check else 0

    required = list((proto["header_fields"]["required"]).keys())
    placement = list((proto["header_fields"]["placement"]).keys())
    acts = list(proto["acts"].keys())
    print(f"contract {proto['protocol']} v{proto['version']}: "
          f"{len(required)} required fields, {len(acts)} acts")

    # 1. the renderer
    if ARCADE_HTML.exists():
        html = ARCADE_HTML.read_text(encoding="utf-8", errors="replace")
        if "ARCADE " not in html:
            problems.append("the page never looks for the `ARCADE ` prefix the contract mandates")
        for f in required + placement:
            # `o.<field>` or `'field'` / "field" is how the parser reads it
            if f"o.{f}" not in html and f"'{f}'" not in html and f'"{f}"' not in html:
                problems.append(f"page never reads header field {f!r}, but the contract lists it")
        # An act is renderable if the page names it at all: as a quoted literal
        # in a comparison, or as a bare object key like `ask:'\U0001f52e'` in the
        # icon table. Checking only the quoted forms reported `ask` as
        # unhandled while the page had `ask:` in FACTS_ICON and rendered it
        # through the generic path, which is a gate crying wolf about its own
        # regex. Acts the page never mentions still get reported, because those
        # really would arrive with no icon and no label.
        for a in acts:
            if not re.search(rf"(['\"]){a}\1|\b{a}\s*:", html):
                notes.append(f"act {a!r} is published and the page never names it; "
                             f"it will render through the generic path with no icon")
        print(f"  renderer: checked {ARCADE_HTML}")
    else:
        notes.append(f"renderer not present at {ARCADE_HTML} (gitignored build artifact); "
                     f"page conformance NOT checked this run")

    # 2. the fleet in the wild
    seen: set[str] = set()
    sampled = 0
    try:
        geo = get("/v1/channel/geo")
        writers = []
        for m in geo.get("messages", []):
            if m["from"] not in writers:
                writers.append(m["from"])
        for pk8 in writers[:6]:
            try:
                with urllib.request.urlopen(
                        f"{BASE}/memories/by_attester/{pk8}/arcade/state.md", timeout=25) as r:
                    first = r.read().decode("utf-8", "replace").split("\n", 1)[0]
            except Exception:
                continue
            if not first.startswith("ARCADE "):
                problems.append(f"{pk8} state.md exists but has no ARCADE header")
                continue
            try:
                seen |= set(json.loads(first[7:]).keys())
                sampled += 1
            except json.JSONDecodeError:
                problems.append(f"{pk8} ARCADE header is not valid JSON on one line")
    except Exception as e:
        notes.append(f"fleet sample skipped: {e}")

    if sampled:
        documented = set()
        for group in proto["header_fields"].values():
            # header_fields also carries prose notes alongside the field groups;
            # only the objects are field tables.
            if isinstance(group, dict):
                documented |= set(group.keys())
        # fields the contract names but nobody writes are fine (optional);
        # fields everybody writes but the contract omits are the real drift.
        undocumented = seen - documented
        for f in sorted(undocumented):
            problems.append(f"header field {f!r} appears in the {sampled} sampled live "
                            f"presence notes, and /v1/arcade/protocol does not document it")
        missing_required = [f for f in required if f not in seen]
        for f in missing_required:
            problems.append(f"contract requires {f!r} but no sampled live agent writes it")
        print(f"  fleet: sampled {sampled} live presence notes, {len(seen)} distinct header fields")

    # 3. the routes it points at
    for name, url in (proto.get("live") or {}).items():
        target = url.split("?")[0]
        try:
            req = urllib.request.Request(BASE + target, method="GET")
            with urllib.request.urlopen(req, timeout=25) as r:
                ok = r.status == 200
        except Exception as e:
            ok = False
            notes.append(f"live.{name} -> {target}: {e}")
        if not ok and "sse" not in target:
            problems.append(f"contract advertises live.{name} -> {target}, which does not answer")
    print(f"  routes: checked {len(proto.get('live') or {})} advertised endpoints")

    for n in notes:
        print(f"  note: {n}")
    if problems:
        print("\nVIOLATIONS:")
        for p in problems:
            print(f"  x {p}")
        return 1 if check else 0
    print("\nThe published contract, the renderer and the live fleet agree.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
