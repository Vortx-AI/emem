"""Live read-only calls for one-place-many-names.webp.

Five phrasings of one mountain go to POST /v1/locate. Each distinct cell
then gets one POST /v1/recall for the elevation band, and one receipt is
checked with POST /v1/verify_receipt. Output: data/names/*.json.
"""

import json
import pathlib
import urllib.request

BASE = "https://emem.dev"
OUT = pathlib.Path(__file__).parent / "data" / "names"
PHRASINGS = ["Mount Fuji", "mount fuji", "Fujisan", "35.3606, 138.7274", "Fuji-san, Japan"]
BAND = "copdem30m.elevation_mean"


def post(path, body):
    req = urllib.request.Request(
        BASE + path,
        data=json.dumps(body).encode(),
        headers={"content-type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.load(r)


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    rows = []
    for p in PHRASINGS:
        d = post("/v1/locate", {"place": p})
        s = d["selected"]
        rows.append({
            "phrasing": p,
            "cell64": d["cell64"],
            "lat": s["lat"],
            "lng": s["lng"],
            "label": s.get("label"),
            "source": s.get("source"),
            "via": d.get("via"),
            "confidence_reason": s.get("confidence_reason"),
            "is_high_confidence": s.get("is_high_confidence"),
            "disambiguation_required": d.get("disambiguation_required"),
            "wikidata": (d.get("alternatives") or [{}])[0].get("wikidata"),
        })
    recalls = {}
    for cell in dict.fromkeys(r["cell64"] for r in rows):
        rec = post("/v1/recall", {"cell": cell, "bands": [BAND]})
        recalls[cell] = rec
    first = rows[0]["cell64"]
    check = post("/v1/verify_receipt", {"receipt": recalls[first]["receipt"]})
    (OUT / "locate.json").write_text(json.dumps(rows, ensure_ascii=False, indent=1))
    (OUT / "recalls.json").write_text(json.dumps(recalls, ensure_ascii=False, indent=1))
    (OUT / "verify.json").write_text(json.dumps(check, indent=1))
    for r in rows:
        facts = recalls[r["cell64"]].get("facts", [])
        vals = [f.get("value") for f in facts if f.get("band") == BAND]
        print(r["phrasing"], r["cell64"], r["source"], r["disambiguation_required"], vals)
    print("verify:", {k: check.get(k) for k in ("valid", "signature_valid")})


if __name__ == "__main__":
    main()
