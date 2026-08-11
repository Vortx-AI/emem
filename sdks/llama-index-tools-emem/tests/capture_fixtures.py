#!/usr/bin/env python3
"""Capture the test fixtures from the live responder.

The fixtures under `tests/fixtures/` are verbatim response bodies, written by
this script and not edited afterwards. That matters more than it sounds: the
previous fixtures were hand-written to the shape the code assumed, so the
suite was green while two of the four shipped tools returned the wrong thing.
`recall()["cell"]` read `resolved_from.cell.cell64`, a key production has
never sent, and the fixture supplied it. `locate()` passed
`data_at_this_cell` straight through believing it was a band list; it is 19 KB
of capability prose, and the fixture said it was one band name.

Re-capture with:

    python3 tests/capture_fixtures.py                 # against https://emem.dev
    python3 tests/capture_fixtures.py --origin http://localhost:5051

Each file records the request that produced it under `_capture`, so a reader
can reproduce it with curl. Nothing else is added and nothing is removed.

Queries are chosen so the bodies stay reviewable in a diff. The coordinate
`12.971899,77.593665` is the centre of the same Bengaluru cell the recall
fixtures read, and production returns `polygon_geojson: null` for a
coordinate, which keeps that file at 24 KB instead of the 152 KB the name
"Bengaluru" returns. Rounding it to `12.97,77.59` lands on a different cell,
roughly 300 m away, so the digits matter. Springfield keeps its polygon, so
the assertion that `locate` drops one still has real bytes to drop.
"""

from __future__ import annotations

import argparse
import json
import os
import urllib.request
from datetime import datetime, timezone

HERE = os.path.dirname(os.path.abspath(__file__))
FIXTURES = os.path.join(HERE, "fixtures")

CELL = "defi.zb493.xuqA.zcb5f"
BAND = "copdem30m.elevation_mean"

# name -> (path, request body). Order matters: verify_receipt reuses the
# receipt recall just returned, and resolve_token reuses its citation.
CAPTURES = [
    ("locate_bengaluru", "/v1/locate", {"q": "12.971899,77.593665"}),
    ("locate_springfield", "/v1/locate", {"q": "Springfield"}),
    ("recall_bengaluru", "/v1/recall", {"place": "Bengaluru", "bands": [BAND]}),
    ("recall_by_cell", "/v1/recall", {"place": CELL, "bands": [BAND]}),
    # An embedding band, so the value budget is exercised against a real
    # vector. `geotessera` is the smallest of the seven (2,535 characters of
    # floats); `clay_v1` would do the same job with a 22 KB fixture.
    ("recall_embedding", "/v1/recall", {"place": CELL, "bands": ["geotessera"]}),
    ("resolve_token", "/v1/memory_token/resolve", {"token": f"emem:fact:{CELL}:"}),
    ("verify_receipt", "/v1/verify_receipt", {"receipt": None}),
]


def post(origin: str, path: str, body: dict) -> dict:
    request = urllib.request.Request(
        origin.rstrip("/") + path,
        data=json.dumps(body).encode(),
        headers={"content-type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(request, timeout=120) as response:
        return json.load(response)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--origin", default=os.environ.get("EMEM_URL", "https://emem.dev"))
    args = parser.parse_args()

    os.makedirs(FIXTURES, exist_ok=True)
    captured_at = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    recall_body: dict = {}

    for name, path, body in CAPTURES:
        request_body = dict(body)
        if name == "resolve_token":
            # The citation recall just minted, so the round trip is real
            # rather than a token typed in here.
            request_body["token"] = (recall_body["facts"][0]["memory_token"])
        if name == "verify_receipt":
            request_body["receipt"] = recall_body["receipt"]

        response = post(args.origin, path, request_body)
        if name == "recall_bengaluru":
            recall_body = response

        response["_capture"] = {
            "origin": args.origin,
            "path": path,
            "request": request_body,
            "captured_at": captured_at,
        }
        out = os.path.join(FIXTURES, f"{name}.json")
        with open(out, "w") as handle:
            json.dump(response, handle, indent=1, sort_keys=True)
            handle.write("\n")
        print(f"{name:20s} {os.path.getsize(out):>8d} bytes  {path}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
