#!/usr/bin/env python3
"""Alarm when no INDEPENDENT witness has seen this log recently.

Why the built-in flag is not enough
-----------------------------------
`/v1/log/witnesses` reports `head_is_witnessed`, and on emem.dev it is almost
always true. Measured 2026-09-05, at one instant:

    head_is_witnessed: True   (freshest behind: 0)
      k572x7go  n=159  behind=0     <- this host's own key
      ttecxvf3  n=41   behind=124   <- the only external witness

The zero-behind signature is `storage_liveness.py`, the watchdog's
write-liveness canary, which co-signs the current head every two minutes to time
the write and catch a wedge that stops writes while reads still answer. That is
a good probe and its choice of an idempotent co-signature is deliberate. But it
shares a key and an endpoint with federation witnessing, so the flag ends up
answering "did anything sign this head" when the question worth asking is "did
anyone ELSE".

Split-view detection is exactly the property a self-signature cannot provide: a
node cannot catch itself showing two histories. So this measures the lag of
witnesses that are not us, and ignores the rest.

It does not replace the flag; it reports the number the flag obscures.

Exit codes, distinct because the repairs differ:
  0  an independent witness is within --max-behind
  2  independent witnesses exist but all are staler than --max-behind
  3  NO independent witness at all -- the mesh is not running
  4  the endpoint could not be read
"""
import argparse
import json
import sys
import urllib.request


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--origin", default="http://127.0.0.1:5051")
    ap.add_argument("--self-key", action="append", default=[],
                    help="a witness key belonging to THIS host (the canary, the "
                         "local agent). Repeatable. These are excluded.")
    ap.add_argument("--max-behind", type=int, default=5000,
                    help="entries the freshest independent witness may lag. emem.dev "
                         "grows fast, so this is entries rather than seconds.")
    ap.add_argument("--limit", type=int, default=200)
    a = ap.parse_args()

    try:
        with urllib.request.urlopen(
                f"{a.origin.rstrip('/')}/v1/log/witnesses?limit={a.limit}", timeout=45) as r:
            d = json.loads(r.read())
    except Exception as e:
        print(f"FAIL: cannot read witnesses at {a.origin}: {type(e).__name__}: {e}")
        return 4

    cur = d.get("current_tree_size")
    ws = d.get("witnesses") or []
    mine = {k[:12] for k in a.self_key}
    ext = [w for w in ws if w["witness_pubkey_b32"][:12] not in mine]

    if not ext:
        print(f"FAIL: tree_size {cur}: {len(ws)} co-signature(s), ALL from this host. "
              f"head_is_witnessed={d.get('head_is_witnessed')} is self-satisfied and "
              f"means nothing. No peer is witnessing this log.")
        return 3

    best = min(ext, key=lambda w: w["entries_behind_current"])
    behind = best["entries_behind_current"]
    who = best["witness_pubkey_b32"][:12]
    if behind > a.max_behind:
        print(f"FAIL: tree_size {cur}: freshest INDEPENDENT witness ({who}...) is "
              f"{behind} entries behind, over the {a.max_behind} limit, last at "
              f"{best['cosigned_at']}. The peer's witness timer may have stopped.")
        return 2

    print(f"ok: tree_size {cur}; independent witness {who}... is {behind} behind "
          f"(limit {a.max_behind}); {len(ext)} of {len(ws)} co-signatures are not ours")
    return 0


if __name__ == "__main__":
    sys.exit(main())
