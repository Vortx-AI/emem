"""Fetch the live band registry and the wired materializer list for wired_bands.py.

Read-only GETs to emem.dev: /v1/bands once, /v1/materializers six pages.
Writes data/bands/{bands,materializers}.json.
"""

import json
import pathlib
import urllib.request

BASE = "https://emem.dev"
OUT = pathlib.Path(__file__).parent / "data" / "bands"


def get(path):
    req = urllib.request.Request(BASE + path, headers={"User-Agent": "emem-docs-media"})
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.load(r)


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / "bands.json").write_text(json.dumps(get("/v1/bands"), indent=1))
    items, page, total = [], 1, None
    while True:
        d = get(f"/v1/materializers?summary=true&page={page}&page_size=20")
        items += d["items"]
        total = d["pagination"]["total"]
        if not d["pagination"]["has_next"]:
            break
        page += 1
    assert len(items) == total, (len(items), total)
    (OUT / "materializers.json").write_text(json.dumps({"total": total, "items": items}, indent=1))
    print(f"{total} materializers, {page} pages")


if __name__ == "__main__":
    main()
