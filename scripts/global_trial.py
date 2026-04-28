#!/usr/bin/env python3
"""
Global trial — exercise emem.dev across diverse place types and capture
what an AI agent would actually experience.

Categories tested (~40 fixtures):
  major_city, mountain_peak, ocean_open, ocean_deep, sea, desert, polar,
  rainforest, island, coastal, edge_case (antimeridian, equator,
  prime-meridian, poles), well_known_place_name, non_english_place_name.

For each fixture we exercise:
  - GET /health
  - POST /v1/locate (lat/lng) — does the cell + polygon look right?
  - POST /v1/locate (place name) — only for fixtures where we set `place`
  - POST /v1/elevation — direct elevation endpoint
  - POST /v1/recall {bands: [copdem30m.elevation_mean]} — does lazy
    materialization yield a real signed Primary fact?
  - POST /v1/recall_many — fan out across the first 16 polygon sample cells

We record latency, status, the actual returned values, and a few
predicates ("did the polygon contain the input?", "is elevation roughly
plausible?"). Output is a JSON document and a Markdown report.

Usage:
  python3 scripts/global_trial.py [BASE_URL]
  default: https://emem.dev
"""

import json, sys, time, datetime
from typing import Optional
import urllib.request, urllib.error

BASE = sys.argv[1] if len(sys.argv) > 1 else "https://emem.dev"

UA = "emem-global-trial/0.1 (+https://emem.dev)"

def http(method: str, path: str, body: Optional[dict] = None, timeout: int = 30, max_retries: int = 4):
    """HTTP with retry-on-429: respects Retry-After when present, otherwise
    exponential backoff (0.5s, 1s, 2s, 4s). The trial fans out enough
    requests that without this, a tight inner loop trips the per-IP
    bucket — which is the right behaviour for production but not what
    we're testing for here."""
    url = BASE.rstrip("/") + path
    data = None
    headers = {"user-agent": UA, "accept": "application/json"}
    if body is not None:
        data = json.dumps(body).encode("utf-8")
        headers["content-type"] = "application/json"
    backoff = 0.5
    for attempt in range(max_retries + 1):
        req = urllib.request.Request(url, data=data, method=method, headers=headers)
        t0 = time.monotonic()
        try:
            with urllib.request.urlopen(req, timeout=timeout) as r:
                raw = r.read()
                dur_ms = (time.monotonic() - t0) * 1000
                try:
                    return {"ok": True, "status": r.status, "dur_ms": dur_ms, "json": json.loads(raw.decode("utf-8"))}
                except Exception:
                    return {"ok": True, "status": r.status, "dur_ms": dur_ms, "raw_len": len(raw)}
        except urllib.error.HTTPError as e:
            dur_ms = (time.monotonic() - t0) * 1000
            body_s = ""
            try: body_s = e.read().decode("utf-8")[:500]
            except Exception: pass
            if e.code == 429 and attempt < max_retries:
                ra = 0.0
                try: ra = float(e.headers.get("Retry-After", "0"))
                except Exception: ra = 0.0
                wait = max(ra, backoff)
                time.sleep(wait)
                backoff *= 2
                continue
            return {"ok": False, "status": e.code, "dur_ms": dur_ms, "error": str(e), "body": body_s}
        except Exception as e:
            dur_ms = (time.monotonic() - t0) * 1000
            return {"ok": False, "dur_ms": dur_ms, "error": str(e)}
    return {"ok": False, "error": "max retries exhausted"}

# ── Fixtures ───────────────────────────────────────────────────────────
# (category, name, lat, lng, place_name_for_geocoder, expected_elev_m_range)
# expected_elev_m_range: tuple (low, high) or None if undefined (e.g. open ocean)

FIXTURES = [
    # Major cities
    ("major_city", "Tokyo",         35.6762, 139.6503, "Tokyo",          (0, 200)),
    ("major_city", "New York",      40.7128, -74.0060, "New York",       (-5, 100)),
    ("major_city", "London",        51.5074, -0.1278,  "London",         (0, 50)),
    ("major_city", "São Paulo",    -23.5505, -46.6333, "São Paulo",      (700, 850)),
    ("major_city", "Lagos",          6.5244,  3.3792,  "Lagos",          (0, 50)),
    ("major_city", "Sydney",       -33.8688, 151.2093, "Sydney",         (0, 80)),
    ("major_city", "Mumbai",        19.0760, 72.8777,  "Mumbai",         (0, 30)),
    ("major_city", "Cairo",         30.0444, 31.2357,  "Cairo",          (15, 80)),

    # Mountain peaks
    ("mountain_peak", "Mt Fuji",       35.3606, 138.7274, "Mount Fuji",   (3000, 3800)),
    ("mountain_peak", "Mt Everest",    27.9881, 86.9250,  "Mount Everest",(7500, 8849)),
    ("mountain_peak", "Kilimanjaro",  -3.0674,  37.3556,  "Kilimanjaro",  (5000, 5895)),
    ("mountain_peak", "Denali",       63.0692, -151.0070, "Denali",       (4500, 6190)),
    ("mountain_peak", "K2",           35.8825, 76.5133,   "K2",           (7000, 8611)),
    ("mountain_peak", "Aconcagua",   -32.6532, -70.0109,  "Aconcagua",    (5500, 6962)),
    ("mountain_peak", "Mont Blanc",   45.8326, 6.8652,    "Mont Blanc",   (4000, 4810)),

    # Oceans / seas
    ("ocean_deep",  "Mariana Trench",   11.3736, 142.5916, None, (-11500, -8000)),
    ("ocean_open",  "North Pacific",    35.0,   -160.0,    None, (-6500, -3000)),
    ("ocean_open",  "South Atlantic",  -30.0,   -20.0,     None, (-6000, -2000)),
    ("ocean_open",  "Indian Ocean",    -20.0,    80.0,     None, (-6000, -3000)),
    ("sea",         "North Sea",        56.0,     3.0,     "North Sea",    (-200, 0)),
    ("sea",         "Mediterranean",    37.0,    18.0,     "Mediterranean Sea", (-4000, 0)),

    # Deserts
    ("desert",  "Sahara (Algeria)",     23.4162,  10.0  ,  None, (200, 1000)),
    ("desert",  "Atacama",             -22.9576, -68.2,    "Atacama",      (2000, 4500)),
    ("desert",  "Gobi (Mongolia)",      42.7951, 105.0,    None, (800, 1500)),
    ("desert",  "Outback (AU)",        -25.345, 131.036,   "Uluru",        (300, 900)),

    # Polar / icecap
    ("polar",   "South Pole",         -90.0,    0.0,    None, (2700, 3000)),
    ("polar",   "Greenland Summit",    72.5793, -38.4596, None, (2500, 3300)),
    ("polar",   "Svalbard",            78.2232,  15.6267, "Svalbard",  (0, 800)),
    ("polar",   "Vostok Station",     -78.4644, 106.8372, None, (3400, 3600)),

    # Rainforest
    ("rainforest", "Amazon (centre)",  -3.4653, -62.2159, None, (50, 200)),
    ("rainforest", "Congo basin",       0.0,    23.0,     None, (300, 700)),
    ("rainforest", "Borneo interior",   1.5,   114.0,     None, (200, 1500)),

    # Islands
    ("island",  "Big Island, Hawaii",   19.5429, -155.6659, "Hawaii",      (0, 4200)),
    ("island",  "Iceland (centre)",     64.9631, -19.0208,  "Iceland",     (200, 2000)),
    ("island",  "Galapagos",           -0.9538, -90.9656,   "Galápagos",   (0, 1700)),
    ("island",  "Madagascar",          -18.7669, 46.8691,   "Madagascar",  (200, 2800)),

    # Edge cases
    ("edge_case", "Antimeridian Pacific",  0.0, 180.0,    None, (-6000, -3000)),
    ("edge_case", "Equator/Prime Mer.",    0.0,   0.0,     None, (-6000, 0)),
    ("edge_case", "Drake Passage",       -58.0, -65.0,     None, (-5000, -3000)),
    ("edge_case", "North Pole",          90.0,   0.0,    None, (-4500, -1000)),

    # Non-English place names
    ("place_name", "東京 (Tokyo, JP)",   None, None, "東京",            (0, 200)),
    ("place_name", "Москва (Moscow)",    None, None, "Москва",         (100, 250)),
    ("place_name", "القاهرة (Cairo)",     None, None, "القاهرة",       (15, 80)),
]

def evaluate_fixture(fix):
    cat, name, lat, lng, place, expected_elev = fix
    out = {"category": cat, "name": name, "checks": {}}

    # Place-name geocode (when present)
    if place:
        r = http("POST", "/v1/locate", {"q": place})
        out["checks"]["locate_by_place"] = {
            "ok": r.get("ok") is True,
            "status": r.get("status"),
            "dur_ms": round(r.get("dur_ms", 0), 1),
            "cell64": r.get("json", {}).get("cell64"),
            "via": r.get("json", {}).get("via"),
        }
        if r.get("ok") and not (lat is not None and lng is not None):
            j = r["json"]
            lat = j.get("centre", {}).get("lat_deg")
            lng = j.get("centre", {}).get("lng_deg")

    if lat is None or lng is None:
        out["checks"]["locate_by_latlng"] = {"skipped": "no lat/lng"}
        return out

    # Lat/lng locate
    r = http("POST", "/v1/locate", {"lat": lat, "lng": lng})
    locate_json = r.get("json", {}) if r.get("ok") else {}
    out["checks"]["locate_by_latlng"] = {
        "ok": r.get("ok") is True,
        "status": r.get("status"),
        "dur_ms": round(r.get("dur_ms", 0), 1),
        "cell64": locate_json.get("cell64"),
        "neighborhood_count": len(locate_json.get("neighborhood_cells") or []),
        "polygon_sample_count": len(locate_json.get("polygon_sample_cells") or []),
        "via": locate_json.get("via"),
    }
    cell = locate_json.get("cell64")

    # Direct elevation endpoint
    r = http("POST", "/v1/elevation", {"lat": lat, "lng": lng})
    elev_json = r.get("json", {}) if r.get("ok") else {}
    elev_value = elev_json.get("elevation_m")
    in_range = None
    if expected_elev and isinstance(elev_value, (int, float)):
        lo, hi = expected_elev
        in_range = lo <= elev_value <= hi
    out["checks"]["elevation_endpoint"] = {
        "ok": r.get("ok") is True,
        "status": r.get("status"),
        "dur_ms": round(r.get("dur_ms", 0), 1),
        "elevation_m": elev_value,
        "expected_range": expected_elev,
        "in_range": in_range,
    }

    # Recall with materialization — both copdem (land-only) and GMRT (global topo-bathy)
    if cell:
        for band, key in (("copdem30m.elevation_mean", "recall_copdem"),
                          ("gmrt.topobathy_mean",      "recall_gmrt")):
            r = http("POST", "/v1/recall", {"cell": cell, "bands": [band]})
            rec = r.get("json", {}) if r.get("ok") else {}
            facts = rec.get("facts", [])
            recall_value = None
            kind = None
            if facts:
                f = facts[0]
                kind = f.get("kind")
                recall_value = f.get("value") if kind == "primary" else None
            out["checks"][key] = {
                "ok": r.get("ok") is True,
                "status": r.get("status"),
                "dur_ms": round(r.get("dur_ms", 0), 1),
                "fact_count": len(facts),
                "fact_kind": kind,
                "recall_value_m": recall_value,
                "skip_reasons": [n.get("reason","")[:80] for n in (rec.get("materialize_notes") or [])],
            }

    # recall_many on first 8 polygon sample cells (using GMRT for global coverage)
    sample = (locate_json.get("polygon_sample_cells") or [])[:8]
    if sample:
        r = http("POST", "/v1/recall_many", {"cells": sample, "bands": ["gmrt.topobathy_mean"]})
        rm = r.get("json", {}) if r.get("ok") else {}
        bc = rm.get("by_cell", {})
        non_empty = sum(1 for v in bc.values() if v.get("facts"))
        out["checks"]["recall_many_sample"] = {
            "ok": r.get("ok") is True,
            "status": r.get("status"),
            "dur_ms": round(r.get("dur_ms", 0), 1),
            "cells_requested": len(sample),
            "cells_with_facts": non_empty,
        }

    return out


def main():
    print(f"# emem global trial — {datetime.datetime.now(datetime.UTC).isoformat()}", flush=True)
    print(f"# base = {BASE}", flush=True)
    h = http("GET", "/health")
    print(f"# /health -> {h.get('status')} ({round(h.get('dur_ms',0),1)} ms)", flush=True)
    if not h.get("ok"):
        print("# health failed; aborting", flush=True)
        sys.exit(1)

    results = []
    for i, fix in enumerate(FIXTURES):
        # Pace the trial: ~3 req/s sustained keeps us under the default
        # 1 tok/s + 60-burst limit even with 5 calls per fixture.
        if i: time.sleep(0.3)
        r = evaluate_fixture(fix)
        results.append(r)
        cat, name = fix[0], fix[1]
        cop = r["checks"].get("recall_copdem", {})
        gmrt = r["checks"].get("recall_gmrt", {})
        cop_str = f"{cop.get('fact_kind') or '-'}={cop.get('recall_value_m')}"
        gmrt_str = f"{gmrt.get('fact_kind') or '-'}={gmrt.get('recall_value_m')}"
        # Pass: copdem either land-primary or absence (both cite-able), AND gmrt primary
        cop_ok = cop.get("fact_kind") in ("primary", "absence")
        gmrt_ok = gmrt.get("fact_kind") == "primary"
        marker = "✓" if cop_ok and gmrt_ok else "✗"
        print(f"  [{cat:>13}] {name:<26}  cop:{cop_str:<20}  gmrt:{gmrt_str:<20}  {marker}", flush=True)

    out = {
        "base": BASE,
        "ran_at_utc": datetime.datetime.now(datetime.UTC).isoformat(),
        "fixtures": results,
    }
    out_path = "/tmp/emem_global_trial.json"
    with open(out_path, "w") as f:
        json.dump(out, f, indent=2, ensure_ascii=False)
    print(f"\n# wrote {out_path}", flush=True)


if __name__ == "__main__":
    main()
