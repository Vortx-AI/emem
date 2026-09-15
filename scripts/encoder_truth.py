#!/usr/bin/env python3
"""Every encoder this responder advertises must actually return a vector.

    scripts/encoder_truth.py [--origin https://emem.dev]

Why this exists
---------------
`/v1/data_availability` tells an agent which bands it can ask for, and
`/v1/state_multi` fans out to the foundation encoders by default. Both are
built from lists in the source tree. Whether the model behind an encoder is
actually running is a fact about the DEPLOYMENT, and no list in the repository
can know it.

On 2026-09-15 that gap was live: `clay_v1`, `prithvi_eo2` and `galileo` were
advertised by `/v1/data_availability`, fanned out by `/v1/state_multi`, and
answered `missing` at every cell sampled, because the GPU behind them had been
removed. The unit test beside `FOUNDATION_ENCODERS` passed the whole time, and
could not have failed: it compared a typed list to a typed list. An agent
reading our surface would have designed a feature against nothing.

What this checks
----------------
For each advertised encoder, ask the responder for a state vector at a cell
known to be warm. An encoder that cannot answer anywhere is not a capability,
and advertising it is the failure this file exists to make loud. A cold cell is
not proof, so a miss is only a finding when the encoder misses EVERYWHERE.
"""
import argparse
import json
import sys
import urllib.request

# Cells that have carried facts for a long time. If an encoder answers at none
# of them it is not the address that is cold.
WARM_CELLS = [
    "defi.zb64a.cAzU.zfa27",   # Trafalgar Square, the worked-example cell
    "defi.zb2d8.wAlI.zca2e",
]


def rpc(origin: str, name: str, args: dict) -> dict:
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                       "params": {"name": name, "arguments": args}}).encode()
    req = urllib.request.Request(
        origin.rstrip("/") + "/mcp", data=body,
        headers={"Content-Type": "application/json",
                 "Accept": "application/json, text/event-stream"})
    with urllib.request.urlopen(req, timeout=180) as r:
        d = json.load(r)
    res = d.get("result") or {}
    text = (res.get("content") or [{}])[0].get("text", "")
    try:
        return json.loads(text)
    except ValueError:
        return {"_error": text[:300]}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--origin", default="https://emem.dev")
    args = ap.parse_args()

    answered: dict[str, bool] = {}
    advertised: set[str] = set()
    for cell in WARM_CELLS:
        o = rpc(args.origin, "emem_state_multi", {"cell": cell})
        if "_error" in o:
            print(f"  {cell}: state_multi did not answer: {o['_error']}")
            continue
        for e in o.get("encoders") or []:
            name = e.get("encoder")
            if name:
                advertised.add(name)
                answered[name] = True
        for m in o.get("missing") or []:
            name = m.get("encoder")
            if name:
                advertised.add(name)
                answered.setdefault(name, False)

    if not advertised:
        print("  no encoder was named by state_multi at any warm cell; this check "
              "could not run and is NOT a pass.")
        return 2

    dead = sorted(e for e in advertised if not answered.get(e))
    live = sorted(e for e in advertised if answered.get(e))
    print(f"  advertised: {len(advertised)}  answering: {len(live)} {live}")
    if dead:
        print(f"  NOT answering at any warm cell: {dead}")
        print("  These are advertised by /v1/data_availability and fanned out by")
        print("  /v1/state_multi while producing nothing. An agent cannot tell them")
        print("  from a cold address. Either wire them or stop advertising them.")
        return 1
    print("  Every advertised encoder returns a vector.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
