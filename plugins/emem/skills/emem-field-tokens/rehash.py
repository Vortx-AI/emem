#!/usr/bin/env python3
"""Re-hash bytes read from stdin and compare them to an emem artifact cid.

An artifact is named by base32 (lowercase, no padding) of the BLAKE3-256
digest of its bytes, so a fetched copy either hashes to its name or it
does not.

    curl -sf https://emem.dev/v1/artifacts/$CID | python3 rehash.py $CID
"""
import base64
import sys

from blake3 import blake3


def main() -> int:
    want = sys.argv[1].strip().lower() if len(sys.argv) > 1 else ""
    got = base64.b32encode(blake3(sys.stdin.buffer.read()).digest()).decode().lower().rstrip("=")
    print(got)
    if want:
        print("MATCH" if got == want else "MISMATCH")
        return 0 if got == want else 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
