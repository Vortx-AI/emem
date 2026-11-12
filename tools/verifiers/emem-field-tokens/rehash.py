#!/usr/bin/env python3
"""Re-hash a downloaded file and compare it to an emem artifact cid.

An artifact is named by base32 (lowercase, no padding) of the BLAKE3-256
digest of its bytes, so a fetched copy either hashes to its name or it
does not. Fetch the artifact to a file first, then check the file:

    curl -sf -o artifact.bin "https://emem.dev/v1/artifacts/$CID"
    python3 rehash.py "$CID" artifact.bin

    python3 rehash.py --self-test     # check the bundled BLAKE3

Prints the computed cid, then MATCH (exit 0) or MISMATCH (exit 1).
BLAKE3 comes from emem_crypto.py beside this script, plain Python shipped with the
plugin; nothing here touches the network.
"""
import base64
import sys
from pathlib import Path

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent))
try:
    from emem_crypto import blake3, self_test
except ImportError:
    sys.stderr.write("emem_crypto.py not found: it ships beside this script in the skill's scripts/ directory\n")
    sys.exit(2)


def main() -> int:
    args = sys.argv[1:]
    if args == ["--self-test"]:
        return 0 if self_test() else 1
    want = args[0].strip().lower() if args else ""
    if len(args) >= 2 and args[1] != "-":
        data = Path(args[1]).read_bytes()
    else:
        data = sys.stdin.buffer.read()
    got = base64.b32encode(blake3(data).digest()).decode().lower().rstrip("=")
    print(got)
    if want:
        print("MATCH" if got == want else "MISMATCH")
        return 0 if got == want else 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
