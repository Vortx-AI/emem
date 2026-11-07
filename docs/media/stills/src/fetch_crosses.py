"""Fetch the live reads behind what-crosses.webp. Read-only calls to emem.dev.

One token per source kind in the README table "What crosses, and what stays":
  a file:   GET  /v1/tree/amii4tnpkfoofsyzrbbs2vxsj4?row=1   (a 107-row index of GPT-2 safetensors)
  a device: POST /v1/trace_resolve, then /v1/trace_verify on the record it returns
  a device list: GET /v1/devices
The archive row reuses the Mount Fuji reads in data/log/ and data/hero/.
usage: python fetch_crosses.py    (writes data/crosses/*.json)
"""
import json
import urllib.request
from pathlib import Path

D = Path(__file__).resolve().parent / "data" / "crosses"
D.mkdir(parents=True, exist_ok=True)
API = "https://emem.dev/v1"
TREE = "amii4tnpkfoofsyzrbbs2vxsj4"
TRACE = "emem:trace:mxyer5c2oxn4ud3xqbtnaxcxhdv4kkxiu67q32inbwxgpa7r3s6a"


def call(path, body=None):
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(f"{API}/{path}", data=data, headers={"content-type": "application/json"})
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.load(r)


def save(name, obj):
    (D / f"{name}.json").write_text(json.dumps(obj, indent=1))
    return obj


save("tree_row1", call(f"tree/{TREE}?row=1"))
save("tree", call(f"tree/{TREE}"))
tr = save("trace_resolve", call("trace_resolve", {"token": TRACE}))
save("trace_verify", call("trace_verify", {"trace": tr["trace"], "profile": tr["trace"]["device"]["substrate_profile"]}))
save("devices", call("devices"))
print("wrote", sorted(p.name for p in D.glob("*.json")))
