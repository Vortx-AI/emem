#!/usr/bin/env python3
"""Every skill carries its own emem_crypto.py so it runs when copied alone.
The copies must be byte-identical: one that drifted would verify a
signature another refuses. Exit 1 on any difference or a missing copy."""
import glob, hashlib, os, sys

ROOT = os.path.join(os.path.dirname(__file__), "..", "plugins", "emem", "skills")
SIGNER = os.path.join(ROOT, "..", "..", "emem-signer", "signer")
copies = sorted(glob.glob(os.path.join(ROOT, "*", "scripts", "emem_crypto.py"))
                + glob.glob(os.path.join(SIGNER, "emem_crypto.py")))
needs = sorted({os.path.dirname(p) for p in glob.glob(os.path.join(ROOT, "*", "scripts", "*.py"))}
               | {SIGNER})
missing = [d for d in needs if not os.path.exists(os.path.join(d, "emem_crypto.py"))]
digests = {hashlib.sha256(open(p, "rb").read()).hexdigest() for p in copies}
if missing or len(digests) > 1 or not copies:
    for d in missing:
        print(f"missing emem_crypto.py in {os.path.relpath(d)}")
    if len(digests) > 1:
        print(f"{len(digests)} different emem_crypto.py copies; make them identical")
    sys.exit(1)
# A script a second skill ships for itself must stay the same file.
for name in ("verify_note.py",):
    same = {hashlib.sha256(open(p, "rb").read()).hexdigest()
            for p in glob.glob(os.path.join(ROOT, "*", "scripts", name))}
    if len(same) > 1:
        print(f"{len(same)} different {name} copies; make them identical")
        sys.exit(1)
print(f"{len(copies)} emem_crypto.py copies, identical")
