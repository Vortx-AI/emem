#!/usr/bin/env python3
"""A directory submission must not describe tools differently from the server.

Why this exists
---------------
integrations/chatgpt/chatgpt-app-submission.json declares MCP tool annotations
to a directory. Those annotations are also declared by the server itself, in
crates/emem-mcp/src/lib.rs, and the two had drifted: the submission told the
directory that emem_ask, emem_recall, emem_band_raster, emem_band_cube and
emem_change_attribution were `readOnlyHint: true`, while the server says false.

The server is right. Each of those can materialize and SIGN new facts into a
publicly readable store on a cold miss, which is a state change other readers
see, and `no_tool_claims_read_only_while_authoring_state` already enforces that
inside the crate. The submission was the copy that was wrong, and it was wrong
in the one direction that matters: it told a reviewer a writing tool was safe
to call without consent.

That drift was found only because a reviewer raised an unrelated question about
openWorldHint. Nothing was watching this pair.

What this checks
----------------
Every tool named in the submission exists in the server's descriptor table, and
its readOnlyHint, destructiveHint and openWorldHint match exactly.

Exit codes
----------
  0  the submission says what the server says
  1  a tool is missing from the server, or an annotation disagrees
  2  could not run
"""
import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
SUB = ROOT / "integrations/chatgpt/chatgpt-app-submission.json"
SRC = ROOT / "crates/emem-mcp/src/lib.rs"
FIELDS = (("read_only_hint", "readOnlyHint"),
          ("destructive_hint", "destructiveHint"),
          ("open_world_hint", "openWorldHint"))


def server_annotations(src: str, name: str):
    m = re.search(rf'name:\s*"{re.escape(name)}"', src)
    if not m:
        return None
    chunk = src[m.end():m.end() + 9000]
    nxt = re.search(r'name:\s*"emem_[a-z0-9_]+"', chunk)
    if nxt:
        chunk = chunk[:nxt.start()]
    out = {}
    for rust, js in FIELDS:
        mm = re.search(rf'{rust}:\s*(true|false)', chunk)
        if mm:
            out[js] = mm.group(1) == "true"
    return out if len(out) == len(FIELDS) else None


def main() -> int:
    try:
        tools = json.loads(SUB.read_text(encoding="utf-8"))["tools"]
        src = SRC.read_text(encoding="utf-8")
    except Exception as e:
        print(f"submission-match: cannot run: {e}", file=sys.stderr)
        return 2

    problems = []
    for name, declared in tools.items():
        actual = server_annotations(src, name)
        if actual is None:
            problems.append(f"{name} is submitted but the server has no such tool")
            continue
        for _, js in FIELDS:
            if declared.get(js) != actual.get(js):
                problems.append(
                    f"{name}.{js}: the submission says {declared.get(js)}, "
                    f"the server says {actual.get(js)}")

    print(f"  {len(tools)} tools in the submission, checked against the server")
    if problems:
        print("\nA submission that describes a tool differently from the server "
              "misinforms whoever reads the listing, and the read-only flag is "
              "the one nobody can afford to have wrong.")
        for p in problems:
            print(f"  {p}")
        return 1
    print("Every submitted tool declares what the server declares.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
