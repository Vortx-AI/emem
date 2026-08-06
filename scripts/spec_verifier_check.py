#!/usr/bin/env python3
"""Run the spec's OWN reference verifier against a freshly signed receipt.

Why this exists
---------------
`docs/protocol.md` section 7.4 publishes a self-contained Python verifier and
calls it "the entire trust-rebinding path". It is the listing a third party is
most likely to copy. On 2026-08-06 `5ci64hps` rebuilt it from the segment
tables and found it rejected every current receipt: the stream stopped at tag
`0x09` and never emitted the v2 MERKLE segment at `0x0b`, so it computed a
digest the responder had never signed.

The tell was that it computed **the same digest as the downgrade attack** the
spec describes two sections earlier. The published reference verifier and the
attacker agreed with each other and disagreed with the signer.

`GET /v1/verifier_spec` was correct the whole time, because it is generated
from the same constants the signer uses. Only the prose drifted, in the one
document nobody runs.

So run it. This extracts the fenced Python block from section 7.4, executes it
verbatim against a receipt pulled live, and fails if it does not verify. A
listing that cannot verify a real receipt is not documentation, it is a trap.

Exit codes
----------
  0  the doc's own verifier verifies a live receipt
  1  it does not, or the block could not be found
  2  could not run (responder unreachable, missing dependency)

Usage
-----
  python3 scripts/spec_verifier_check.py
  python3 scripts/spec_verifier_check.py --origin http://127.0.0.1:5051
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import urllib.request

DOC = "docs/protocol.md"
DEFAULT_ORIGIN = "https://emem.dev"


def extract_block(text):
    """The python listing in section 7.4.

    Anchored on content rather than on a section number, so renumbering the
    document cannot silently make this check pass by testing nothing.
    """
    blocks = re.findall(r"```python\n(.*?)```", text, re.S)
    for b in blocks:
        if "emem.preimage.v1" in b and "VerifyKey" in b:
            return b
    return None


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--origin", default=DEFAULT_ORIGIN)
    a = ap.parse_args()
    origin = a.origin.rstrip("/")

    try:
        from blake3 import blake3  # noqa: F401
        from nacl.signing import VerifyKey  # noqa: F401
    except Exception as e:
        print(f"spec-verifier: missing a dependency the listing needs: {e}",
              file=sys.stderr)
        return 2

    try:
        doc = open(DOC, encoding="utf-8").read()
    except OSError as e:
        print(f"spec-verifier: cannot read {DOC}: {e}", file=sys.stderr)
        return 2

    code = extract_block(doc)
    if not code:
        print(f"spec-verifier: no reference verifier found in {DOC}. If it was "
              f"removed, remove this check with it; if it moved, this check has "
              f"been passing without testing anything.", file=sys.stderr)
        return 1

    # A receipt signed just now, so this tests the CURRENT rule rather than a
    # fixture someone remembered to update.
    req = urllib.request.Request(
        f"{origin}/v1/recall",
        data=json.dumps({"cell": "defi.zb493.xuqA.zcb5f",
                         "bands": ["copdem30m.elevation_mean"]}).encode(),
        headers={"content-type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=90) as r:
            receipt = json.loads(r.read())["receipt"]
    except Exception as e:
        print(f"spec-verifier: {origin} did not answer: {e}", file=sys.stderr)
        return 2

    version = receipt.get("preimage_version")
    print(f"spec-verifier: live receipt carries preimage_version={version}")

    # The listing opens `receipt.json` by name, exactly as a reader would.
    import tempfile, os
    tmp = tempfile.mkdtemp()
    cwd = os.getcwd()
    with open(os.path.join(tmp, "receipt.json"), "w") as f:
        json.dump(receipt, f)

    ok = False
    err = None
    try:
        os.chdir(tmp)
        # Verbatim. No edits, no shims: whatever a reader would get.
        exec(compile(code, f"{DOC} §7.4", "exec"), {"__name__": "__main__"})
        ok = True
    except SystemExit as e:
        err = f"the listing exited: {e}"
    except Exception as e:
        err = f"{type(e).__name__}: {e}"
    finally:
        os.chdir(cwd)

    if ok:
        print("spec-verifier: the published listing verifies a live receipt.")
        return 0
    print(f"\nspec-verifier: the published reference verifier FAILED on a "
          f"receipt this responder signed moments ago.\n  {err}\n\n"
          f"This is the listing third parties copy. A verifier that cannot "
          f"verify a genuine receipt sends them to the wrong conclusion about "
          f"real evidence. Check it against GET {origin}/v1/verifier_spec, "
          f"which is generated from the signer's own constants.", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
