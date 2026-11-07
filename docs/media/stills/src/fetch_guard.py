"""Fetch the live guard verdicts that guard_check.py draws.

Five read-only POSTs to /v1/guard/verdict, each receipt checked at
/v1/verify_receipt.
Usage: python3 fetch_guard.py   (writes data/guard/*.json)
"""

import json
import pathlib
import urllib.request

BASE = "https://emem.dev"
TOKEN = (
    "emem:fact:defi.zb27d.xobA.lAkA:"
    "p3dv6dvjbxzdm72fsxg73ass7yxnc7zrreebzwz5x4f4rrj2epvq"
)
OUT = pathlib.Path(__file__).parent / "data" / "guard"

DRAFTS = {
    "allow": ("", f"Table Mountain rises to 1069.01 m per {TOKEN}"),
    "deny": ("", f"Table Mountain rises to 1250 m per {TOKEN}"),
    "nocite_default": ("", "Elevation at Table Mountain is 1250 m."),
    "nocite_gated": ("?claim_gating=true", "Elevation at Table Mountain is 1250 m."),
    "nocite": ("?claim_gating=true", "Table Mountain rises to 1250 m."),
}


def post(path, body):
    req = urllib.request.Request(
        BASE + path,
        data=json.dumps(body).encode(),
        headers={"content-type": "application/json", "user-agent": "emem-docs-still"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.load(r)


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    for name, (query, text) in DRAFTS.items():
        body = post("/v1/guard/verdict" + query, {"texts": [text]})
        body["_draft"] = text
        (OUT / f"{name}.json").write_text(json.dumps(body, indent=2))
        print(name, body.get("action"), body.get("checked"))
        if "receipt" in body:
            v = post("/v1/verify_receipt", {"receipt": body["receipt"]})
            (OUT / f"{name}_verify.json").write_text(json.dumps(v, indent=2))
            print("  verify", v.get("valid"), v.get("status"))


if __name__ == "__main__":
    main()
