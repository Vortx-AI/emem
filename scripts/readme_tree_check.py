#!/usr/bin/env python3
"""The README's Content address footer names a signed tree. Check it covers this README.

The footer says every section above it is a unit of one signed tree, and names
the note that holds the index. That is a claim about these exact bytes, so it
goes stale the moment anyone edits the README without re-signing: on
2026-10-10 an install line changed and the footer kept naming the old tree.

This rebuilds the tree from the committed README, offline, with the plugin's
own tree_proof.py, and compares the whole-body BLAKE3 and the Merkle root with
the published index. A mismatch means: re-sign the README (cut at the footer,
rebuild, post the index, rewrite the footer).

    python3 scripts/readme_tree_check.py [--origin https://emem.dev]
"""
from __future__ import annotations

import argparse
import re
import subprocess
import sys
import tempfile
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
TREE_PROOF = REPO / "tools/verifiers/emem-tokenise-files/tree_proof.py"
FOOTER = "\n## Content address\n"


def field(text: str, key: str) -> str | None:
    m = re.search(rf"^{key}:\s*(\S+)\s*$", text, re.M)
    return m.group(1) if m else None


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--origin", default="https://emem.dev")
    a = ap.parse_args()

    readme = (REPO / "README.md").read_text()
    cut = readme.find(FOOTER)
    if cut < 0:
        print("README has no '## Content address' footer; nothing claims a tree.")
        return 1
    body, footer = readme[:cut], readme[cut:]

    note_path = re.search(r"\(https://emem\.dev(/memories/by_attester/[^)\s]+\.md)\)", footer)
    tree = re.search(r"`emem:tree:([a-z2-7]+)`", footer)
    if not note_path or not tree:
        print("the footer names no index note or no emem:tree token")
        return 1

    try:
        with urllib.request.urlopen(a.origin + note_path.group(1), timeout=40) as r:
            note = r.read().decode()
    except Exception as e:  # network, 404
        print(f"could not fetch the index note {note_path.group(1)}: {e}")
        return 2

    with tempfile.TemporaryDirectory() as d:
        src = Path(d) / "readme_body.md"
        src.write_text(body)
        built = subprocess.run(
            [sys.executable, str(TREE_PROOF), "build", str(src), "--source", "check"],
            capture_output=True, text=True, check=True,
        ).stdout

    bad = []
    for key in ("blake3", "root"):
        want, got = field(note, key), field(built, key)
        if want != got:
            bad.append(f"{key}: the published index says {want}, this README builds {got}")
    if bad:
        print("THE README IS NOT THE FILE ITS FOOTER SIGNED:")
        for b in bad:
            print("  " + b)
        print("Re-sign it: rebuild the tree over everything above the footer, post the "
              "index under the same key, and rewrite the footer to name the new tree.")
        return 1
    print(f"README matches its signed tree emem:tree:{tree.group(1)} "
          f"(root {field(note, 'root')}).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
