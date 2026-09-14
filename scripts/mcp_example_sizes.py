#!/usr/bin/env python3
"""Measure the worked-example size distribution across the served registry.

    scripts/mcp_example_sizes.py [--origin https://emem.dev]

Why this exists
---------------
`MCP_INLINE_EXAMPLE_MAX_BYTES` in emem-api-rest is a measured number, and a
measured number in a comment rots. This is the command that comment names, so
the threshold can be re-derived rather than remembered.

Every tool's listing description ends with its worked example. One of them is
a document: a real signed receipt, byte-for-byte or nothing. Inlining it put
the core listing over the host ceiling, so examples above the cap are replaced
in the listing by a pointer at `emem_tools`. This prints the distribution the
cap sits in, and which tools the cap currently catches.
"""
import argparse
import json
import re
import statistics
import urllib.request

CAP = 512  # keep in step with MCP_INLINE_EXAMPLE_MAX_BYTES
POINTER = re.compile(r"Example arguments: ([\d,]+) bytes, too long to inline")
INLINE = re.compile(r"\n\nExample arguments: (.*)$", re.S)


def tools(origin: str) -> list:
    out, cursor = [], None
    while True:
        params = {"cursor": cursor} if cursor else {}
        body = json.dumps({"jsonrpc": "2.0", "id": 1,
                           "method": "tools/list", "params": params}).encode()
        req = urllib.request.Request(
            origin.rstrip("/") + "/mcp/full", data=body,
            headers={"content-type": "application/json",
                     "accept": "application/json, text/event-stream"})
        with urllib.request.urlopen(req, timeout=120) as r:
            res = json.load(r)["result"]
        out += res["tools"]
        cursor = res.get("nextCursor")
        if not cursor:
            return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--origin", default="https://emem.dev")
    args = ap.parse_args()

    listed = tools(args.origin)
    inline, pointed = [], []
    for t in listed:
        desc = t.get("description") or ""
        pointer = POINTER.search(desc)
        if pointer:
            pointed.append((int(pointer.group(1).replace(",", "")), t["name"]))
            continue
        m = INLINE.search(desc)
        if m:
            inline.append((len(m.group(1)), t["name"]))

    sizes = sorted(n for n, _ in inline)
    if not sizes:
        print("no inlined examples found; the listing shape changed.")
        return 1
    p90 = sizes[min(len(sizes) - 1, int(0.9 * len(sizes)))]
    print(f"  {len(listed)} tools, {len(inline)} examples inlined, "
          f"{len(pointed)} over the {CAP} B cap and pointed at emem_tools")
    print(f"  inlined: median {statistics.median(sizes):.0f} B, "
          f"p90 {p90} B, max {max(sizes)} B")
    for n, name in sorted(inline, reverse=True)[:5]:
        print(f"    {n:6,} B  {name}")
    for n, name in sorted(pointed, reverse=True):
        print(f"    {n:6,} B  {name}  (pointed, not inlined)")

    # The cap earns its place only if it separates the outliers from the rest.
    if pointed:
        gap = min(n for n, _ in pointed) - max(sizes)
        print(f"  the cap sits in a {gap:,} B gap between the two groups.")
        return 0
    over = [(n, name) for n, name in inline if n > CAP]
    if over:
        # Not a headroom figure: the responder is inlining something this cap
        # says it should not, which means the build being served predates it.
        print(f"  {len(over)} example(s) exceed the {CAP} B cap and are inlined "
              f"anyway: {', '.join(name for _, name in over)}.")
        print("  The served build predates the cap. Deploy, then re-run.")
        return 1
    print(f"  nothing exceeds the cap; headroom {CAP - max(sizes):,} B.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
