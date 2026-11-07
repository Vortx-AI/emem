"""Fetch the live answer that ask_trail.py draws.

Reads only: one POST /v1/ask, POST /v1/verify_receipt on its receipt, GET /v1/state/<cid> for each
reasoning stage, and GET /v1/facts/<cid> for each fact the answer rests on.
Usage: python3 fetch_ask.py   (writes data/ask/*.json)
"""

import json
import pathlib
import urllib.request

BASE = "https://emem.dev"
Q = "what is the NDVI in Central Park, New York?"
OUT = pathlib.Path(__file__).parent / "data" / "ask"
UA = {"user-agent": "emem-docs-still"}


def call(path, body=None):
    data = None if body is None else json.dumps(body).encode()
    headers = dict(UA, **({"content-type": "application/json"} if data else {}))
    req = urllib.request.Request(BASE + path, data=data, headers=headers)
    with urllib.request.urlopen(req, timeout=120) as r:
        return json.load(r)


def save(name, body):
    (OUT / f"{name}.json").write_text(json.dumps(body, indent=2))


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    ans = call("/v1/ask", {"q": Q})
    save("ask", ans)
    print(ans["answer"])
    v = call("/v1/verify_receipt", {"receipt": ans["receipt"]})
    save("ask_verify", v)
    print("verify", v.get("valid"), v.get("signature_valid"))
    states = {}
    for s in ans["reasoning"]["states"]:
        cid = s["state"].split(":")[-1]
        states[s["stage"]] = call(f"/v1/state/{cid}")
    save("states", states)
    facts = {cid: call(f"/v1/facts/{cid}") for cid in ans["fact_cids"]}
    save("facts", facts)
    for cid, f in facts.items():
        print(cid[:8], f["band"], f["value"], f["sources"][0].get("captured_at"))


if __name__ == "__main__":
    main()
