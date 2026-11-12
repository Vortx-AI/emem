#!/usr/bin/env python3
"""Every reference verifier folder carries its own emem_crypto.py so it runs
when copied alone, and so does the signer. The copies must be byte-identical:
one that drifted would verify a signature another refuses. Exit 1 on any
difference or a missing copy."""
import glob, hashlib, os, sys

HERE = os.path.dirname(__file__)
ROOT = os.path.join(HERE, "..", "tools", "verifiers")
SIGNER = os.path.join(HERE, "..", "plugins", "emem-signer", "signer")
copies = sorted(glob.glob(os.path.join(ROOT, "*", "emem_crypto.py"))
                + glob.glob(os.path.join(SIGNER, "emem_crypto.py")))
needs = sorted({os.path.dirname(p) for p in glob.glob(os.path.join(ROOT, "*", "*.py"))}
               | {SIGNER})
missing = [d for d in needs if not os.path.exists(os.path.join(d, "emem_crypto.py"))]
digests = {hashlib.sha256(open(p, "rb").read()).hexdigest() for p in copies}
if missing or len(digests) > 1 or not copies:
    for d in missing:
        print(f"missing emem_crypto.py in {os.path.relpath(d)}")
    if len(digests) > 1:
        print(f"{len(digests)} different emem_crypto.py copies; make them identical")
    sys.exit(1)
# A verifier that two folders carry must stay the same file.
for name in ("verify_note.py",):
    same = {hashlib.sha256(open(p, "rb").read()).hexdigest()
            for p in glob.glob(os.path.join(ROOT, "*", name))}
    if len(same) > 1:
        print(f"{len(same)} different {name} copies; make them identical")
        sys.exit(1)
print(f"{len(copies)} emem_crypto.py copies, identical")
