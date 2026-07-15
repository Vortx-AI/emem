"""The world at its true resolution, rendered four ways: the arms under test.

WHY THIS DOES NOT READ THE SCENE DIGEST
---------------------------------------
`world_london/scene_digest.json` is a rendering artifact, and it discloses what
it is (`scene.grid.note`): "signed scalars anchored on the coarser grid and
inherited". Measured, that means:

    256 rendered tiles          ->  64 distinct cell64
    each signed fact inherited across 4 tiles
    two tiles sharing one cell64 sit up to 151 m apart
    a tile's own lat/lng locates to a DIFFERENT cell64 than the one it lists,
        by up to 74 m

So a benchmark built on the digest asks "what is the NDVI at <tile centre>" and
answers it with an observation from up to 74 m away, anchored on a 212 m grid.
Worse, minting a descriptor token for it stamps a 5-decimal coordinate (~1 m)
onto a value that was averaged over a neighbourhood. The 5dp floor exists to
stop a citation pointing at the neighbouring 10 m cell; using it to assert 1 m
precision on a 212 m-anchored inherited scalar is that same failure, committed
deliberately, in the demo that is supposed to prove the opposite.

emem holds the real thing. Two adjacent 10 m cells in Westminster:

    defi.zb649.zf09f.zf998   ndvi = 0.285271
    defi.zb649.zf0df.zf998   ndvi = 0.108051     (~10 m north)

NDVI swings 2.6x across 10 m. That is the signal the coarse grid destroyed, and
it is exactly the signal the benchmark needs: at 10 m, *which cell* is the whole
question, so retrieving the neighbour is a real error and addressing it is a
real answer. Built on the digest, every neighbour looked identical and the
effect under test could not have appeared at all.

Every cell here is therefore recalled from the responder at a true cell64, and
every question is asked at the cell's OWN coordinates, derived from its cell64
rather than from a tile centre.

THE ARMS
--------
    emem     one `emem:fact:` token per observation, resolved from the responder
    context  the same observation bodies, pasted in, with no address at all
    rag      the observations reached by dense retrieval over the whole patch

Everything except the addressing is held fixed, so a difference in the answers
is attributable to the addressing. `context` is the control that keeps that
honest: same bytes as `emem`, addressing removed. The retriever is a real dense
one (bge-small-en-v1.5); geoqa-embed is CLIP (512-d, 77-token cap, image-caption
trained) and would have lost for reasons unrelated to retrieval, which is a
strawman worth less than no arm.
"""

from __future__ import annotations

import json
import math
import urllib.request
from typing import Any

RESPONDER = "https://emem.dev"

#: ~10 m in degrees latitude. Longitude is scaled by cos(lat) at use.
DEG_10M_LAT = 10.0 / 111_320.0

BAND_PROSE = {
    "indices.ndvi": "NDVI, vegetation greenness",
    "indices.ndwi": "NDWI, surface water index",
    "indices.ndbi": "NDBI, built-up index",
    "copdem30m.elevation_mean": "mean elevation in metres",
    "surface_water.recurrence": "surface water recurrence, percent",
}


def _post(path: str, body: dict, timeout: int = 180) -> dict[str, Any]:
    req = urllib.request.Request(
        RESPONDER + path, data=json.dumps(body).encode(),
        headers={"Content-Type": "application/json", "Connection": "close"})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.loads(r.read())


def locate(lat: float, lng: float) -> str:
    """The canonical cell64 for a coordinate.

    Uses ?lat=&lng= rather than ?q=<lat,lng>: until b431941 the q form was
    geocoded as a place name and landed ~300 m away with a 200.
    """
    return _post("/v1/locate", {"lat": lat, "lng": lng})["cell"]


def recall(cell64: str, bands: list[str]) -> dict[str, dict]:
    """Signed facts at one true 10 m cell, keyed by band. Auto-fetches on miss."""
    rec = _post("/v1/recall", {"cell": cell64, "bands": bands})
    out = {}
    for f in rec.get("facts") or []:
        out[f["band"]] = f
    return out


def cell_latlng(cell64: str, fact_cid: str, band: str, date: str) -> tuple[float, float]:
    """The cell's OWN coordinates, taken from the responder's descriptor token.

    Derived from the cell64 rather than from any grid the caller invented, so a
    question asked at these coordinates is asked at the place the fact actually
    describes. This is the exact step the scene-digest build got wrong.
    """
    body = {"cell": cell64, "fact_cid": fact_cid, "band": band}
    if date:
        body["observed_on"] = date
    m = _post("/v1/memory_token", body)
    tok = m.get("descriptor_token")
    if not tok:
        raise RuntimeError(f"no descriptor token for {cell64}/{band}: cannot site the question")
    coords = tok.split(":", 2)[2].split("@")[0]
    lat, lng = (float(x) for x in coords.split(","))
    return lat, lng


def build_patch(lat0: float, lng0: float, n: int, bands: list[str]) -> dict[str, dict]:
    """An n x n lattice of TRUE 10 m cells anchored at (lat0, lng0).

    Deduplicated by cell64: the lattice oversamples slightly to guarantee every
    10 m cell is hit, and two lattice points landing in one cell must collapse
    to one entry rather than weight it twice.
    """
    out: dict[str, dict] = {}
    for i in range(n):
        for j in range(n):
            lat = lat0 + i * DEG_10M_LAT
            lng = lng0 + j * DEG_10M_LAT / math.cos(math.radians(lat0))
            c = locate(lat, lng)
            if c in out:
                continue
            facts = recall(c, bands)
            if not facts:
                continue
            out[c] = {"cell64": c, "facts": facts}
    return out


def cell_prose(entry: dict) -> str:
    """One retrievable chunk describing one true 10 m cell.

    The same prose feeds `rag` (indexed) and `context` (handed over), so the two
    differ only in how the chunk is reached and neither wins on wording.
    """
    lat, lng = entry["lat"], entry["lng"]
    lines = [f"The 10 m cell at latitude {lat:.5f}, longitude {lng:.5f}."]
    for band, fact in entry["facts"].items():
        v = fact.get("value")
        if isinstance(v, float):
            v = round(v, 6)
        lines.append(f"{BAND_PROSE.get(band, band)}: {v}.")
    return " ".join(lines)


def fact_line_emem(band: str, token: str, fact: dict) -> str:
    v = fact.get("value")
    if isinstance(v, float):
        v = round(v, 6)
    return f"{token}\n  {BAND_PROSE.get(band, band)} = {v}"


def fact_line_context(band: str, fact: dict) -> str:
    v = fact.get("value")
    if isinstance(v, float):
        v = round(v, 6)
    return f"{BAND_PROSE.get(band, band)} = {v}"
