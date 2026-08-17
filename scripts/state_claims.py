#!/usr/bin/env python3
"""Prose that asserts a mutable system state must agree with the responder.

Why this exists
---------------
Counts have a gate (`sync_counts`). Routes have one (`route_truth`). Live
figures on the homepage have one (`live_numbers`). Nothing watched the
sentences that assert what a *component is currently doing*, and those rot the
same way, except more quietly: a count looks like a fact and invites checking,
while "JEPA v2 is untrained today" reads like settled background.

On 2026-08-17 five documents said the JEPA v2 dynamics head was untrained and
that its receipt carries `untrained_baseline`. The head had since been trained.
Every one of those sentences was false, and the failure they described had been
replaced by a different one: the head is trained and *loses to persistence*, so
the receipt carries `NEGATIVE_SKILL` and serves every band from
`persistence_fallback_negative_skill`.

Both statements warn you off the output, which is why nobody noticed. But an
agent that read the docs would look for `untrained_baseline` in the receipt,
not find it, and reasonably conclude the caveat no longer applied, when in fact
a different caveat did. A stale warning is worse than no warning, because it
spends the reader's trust on the wrong thing.

The general shape
-----------------
A claim of the form "X is currently Y" is a measurement with no timestamp and
no owner. This gate gives each one a probe: the phrase may appear only while
the responder still answers the way the phrase says it does. When the state
flips, the build fails and names the files.

Adding a claim is three fields: how to probe, how to read the answer, and which
phrasings each state forbids. Keep the phrasings narrow. A regex broad enough
to catch every wording will also catch the historical notes, and a gate that
cries wolf gets an exemption list, which is how the rot gets back in.

    python3 scripts/state_claims.py
    python3 scripts/state_claims.py --origin http://127.0.0.1:5051

Exit codes: 0 every state claim matches the responder, 1 at least one is
stale, 2 the responder could not be reached (nothing asserted).
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
import urllib.error
import urllib.request

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEFAULT_ORIGIN = "https://emem.dev"

# Generated renders and frozen archives. docs/book/ is mdbook output rebuilt on
# every deploy, so fixing the source fixes it; whitepaper-v1 is the archived DOI
# record and must keep saying what it said; the collaboration log is a
# transcript, where a superseded statement is the point rather than a defect.
SKIP = (
    "docs/book/",
    "docs/whitepaper-v1.md",
    "docs/collaboration-log.md",
    "target/",
    ".git/",
)


def probe_jepa_trained(origin):
    """Is the dynamics head the zero-init sentinel, or a trained model?

    Read from the honesty warnings rather than a `trained` boolean on purpose:
    the warnings are what an agent actually receives, so this checks the thing
    the docs describe rather than an internal flag that happens to agree.
    """
    req = urllib.request.Request(
        origin + "/v1/jepa_predict_v2",
        data=json.dumps({"cell": "defi.zb493.xuqA.zcb5f"}).encode(),
        headers={"content-type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=90) as r:
        body = json.loads(r.read())
    warnings = json.dumps(body.get("model", {}).get("honesty_warnings", []))
    return "untrained" if "untrained_baseline" in warnings else "trained"


CLAIMS = [
    {
        "name": "jepa_v2 training state",
        "probe": probe_jepa_trained,
        # Phrasing that may only appear while the state on the left holds.
        "forbidden_when": {
            "trained": [
                (r"jepa[^.\n]{0,60}\bis\b[^.\n]{0,30}\buntrained\b",
                 "says the head is untrained; it is trained and loses to "
                 "persistence (NEGATIVE_SKILL, persistence_fallback_negative_skill)"),
                (r"\buntrained today\b",
                 "asserts an untrained head as today's state"),
            ],
            "untrained": [
                (r"jepa[^.\n]{0,60}\bis\b[^.\n]{0,30}\btrained\b(?![^.\n]{0,20}on )",
                 "says the head is trained; the receipt carries untrained_baseline"),
            ],
        },
    },
]


def prose_files():
    for root, dirs, files in os.walk(REPO):
        rel_root = os.path.relpath(root, REPO).replace("\\", "/")
        if any(rel_root.startswith(s.rstrip("/")) for s in SKIP):
            dirs[:] = []
            continue
        dirs[:] = [d for d in dirs if not d.startswith(".") and d != "target"]
        for f in files:
            if not f.endswith((".md", ".rs")):
                continue
            rel = os.path.relpath(os.path.join(root, f), REPO).replace("\\", "/")
            if any(rel.startswith(s) for s in SKIP):
                continue
            yield rel


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--origin", default=os.environ.get("EMEM_ORIGIN", DEFAULT_ORIGIN))
    a = ap.parse_args()
    origin = a.origin.rstrip("/")

    files = list(prose_files())
    if not files:
        print("state-claims: read no documents, so nothing was checked.",
              file=sys.stderr)
        return 2

    problems = []
    print(f"state claims against {origin}\n")

    for claim in CLAIMS:
        try:
            state = claim["probe"](origin)
        except Exception as e:
            print(f"  ?? {claim['name']}: responder did not answer ({e})",
                  file=sys.stderr)
            print("state-claims: nothing asserted this run.", file=sys.stderr)
            return 2

        print(f"  {claim['name']}: responder says {state!r}")
        rules = claim["forbidden_when"].get(state, [])
        for rel in files:
            try:
                text = open(os.path.join(REPO, rel), encoding="utf-8",
                            errors="replace").read()
            except OSError:
                continue
            for pattern, why in rules:
                for m in re.finditer(pattern, text, re.I):
                    line = text[:m.start()].count("\n") + 1
                    snippet = " ".join(m.group(0).split())[:78]
                    problems.append(
                        f"{rel}:{line}: {why}\n      {snippet!r}")

    if os.environ.get("GITHUB_ACTIONS") == "true":
        for p in problems:
            print(f"::error title=state-claims::"
                  f"{p.replace(chr(10), ' ').replace('::', ' ')}")

    print(f"\nstate-claims: {len(CLAIMS)} claim(s) probed over {len(files)} "
          f"documents, {len(problems)} stale")
    if problems:
        print("\nA sentence describing what a component currently does is a "
              "measurement. These no longer match the responder:")
        for p in problems:
            print(f"  x {p}")
        return 1
    print("Every state a document asserts is the state the responder is in.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
