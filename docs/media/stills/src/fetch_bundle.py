"""Fetch the live reads that bundle_handoff.py draws.

All calls are reads. The bundle was minted on 23 September 2026 and is
resolved here, not minted again (a new bundle is stored on the responder).
  GET  /v1/memory_bundle/<token>                  -> data/bundle/bundle.json
  POST /v1/verify_receipt {bundle receipt}        -> data/bundle/bundle_verify.json
  POST /v1/memory_token/resolve {token} (x4)      -> data/bundle/fact_<i>.json
  POST /v1/verify_receipt {fact receipt} (x4)     -> data/bundle/fact_<i>_verify.json
Usage: python3 fetch_bundle.py
"""

import json
import pathlib
import urllib.parse
import urllib.request

BASE = "https://emem.dev"
BUNDLE = "emem:bundle:y2k6h2laeo4libfsiebucdw4mu"
OUT = pathlib.Path(__file__).parent / "data" / "bundle"
UA = {"user-agent": "emem-docs-still"}


def get(path):
    req = urllib.request.Request(BASE + path, headers=UA)
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.load(r)


def post(path, body):
    req = urllib.request.Request(
        BASE + path,
        data=json.dumps(body).encode(),
        headers={"content-type": "application/json", **UA},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.load(r)


def save(name, body):
    (OUT / name).write_text(json.dumps(body, indent=2))


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    bundle = get("/v1/memory_bundle/" + urllib.parse.quote(BUNDLE, safe=":"))
    save("bundle.json", bundle)
    v = post("/v1/verify_receipt", {"receipt": bundle["receipt"]})
    save("bundle_verify.json", v)
    print("bundle", bundle["members"], bundle["resolved"], "verify", v.get("valid"))
    for i, c in enumerate(bundle["citations"]):
        fact = post("/v1/memory_token/resolve", {"token": c["memory_token"]})
        save(f"fact_{i}.json", fact)
        fv = post("/v1/verify_receipt", {"receipt": fact["receipt"]})
        save(f"fact_{i}_verify.json", fv)
        print(c["band"], "verify", fv.get("valid"))


if __name__ == "__main__":
    main()
