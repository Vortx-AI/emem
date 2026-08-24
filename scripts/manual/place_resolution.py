#!/usr/bin/env python3
"""Does a question about a place get answered ABOUT THAT PLACE?

    scripts/manual/place_resolution.py [--origin https://emem.dev] [--verbose]

Why this is not the test I wrote the first time
-----------------------------------------------
The first regression test for the exonym refusal asserted that each query
came back answered rather than `needs_location`. It passed while the
responder was answering about the wrong country, because "it answered" and
"it answered about Seoul" are different claims and only the first one was
being checked. The second version was worse: it compared the resolved label
against alternate names I had written down from memory, so it was really
asking whether the geocoder agreed with me about what Seoul is called, and
I had invented some of them.

Coordinates fix both problems. Where these cities are is not a fact about
our geocoder, our label handling, or my recall of Korean; it is a fact about
the world that any atlas settles. So every row below carries a latitude and
longitude, and the check is a distance. A resolution that lands 9,000 km
away fails whatever it called the place, and a resolution that lands in the
right city passes even if the label comes back in a script I cannot read --
which is the whole point, because 서울특별시 is the correct answer.

What it exercises
-----------------
Both halves of the path, because the failures were in different places:

  ask       POST /v1/ask, a natural question. This is where the place
            extractor lives, and where "the population of Seoul, South
            Korea" once had `Korea` pulled out of it and answered from
            Côte d'Ivoire.
  locate    POST /v1/locate, the bare name. This is where the confidence
            floor lives, and where an English query against a local-script
            label scored no overlap and refused.

The rows are endonym/exonym pairs on purpose. Both spellings of one city
must land in the same city; a pass on `Munich` and a refusal on `München`
is the defect this file exists to catch, and the pair makes it visible
without anyone having to remember which spelling was the broken one.

RADIUS_KM is deliberately loose. A geocoder is entitled to put Prague at the
castle or at the main station, and this is not a check on which; it is a
check on which continent.
"""
import argparse
import json
import math
import sys
import time
import urllib.error
import urllib.request

# lat, lon, and how far the resolution may land from it. Coordinates are
# the world's, not ours: any atlas settles them, which is exactly why they
# can referee our geocoder.
PLACES = [
    # (query-as-typed, endonym-or-exonym partner, lat, lon, radius_km)
    ("Munich", "München", 48.137, 11.576, 40),
    ("Cologne", "Köln", 50.938, 6.960, 40),
    ("Vienna", "Wien", 48.208, 16.373, 40),
    ("Prague", "Praha", 50.088, 14.420, 40),
    ("Copenhagen", "København", 55.677, 12.569, 40),
    ("Lisbon", "Lisboa", 38.722, -9.139, 40),
    ("Warsaw", "Warszawa", 52.230, 21.011, 40),
    ("Naples, Italy", "Napoli", 40.852, 14.268, 40),
    ("The Hague", "Den Haag", 52.078, 4.310, 40),
    ("Turin", "Torino", 45.071, 7.687, 40),
    ("Milan", "Milano", 45.464, 9.190, 40),
    ("Florence, Italy", "Firenze", 43.770, 11.256, 40),
    ("Seville", "Sevilla", 37.389, -5.984, 40),
    ("Gothenburg", "Göteborg", 57.709, 11.974, 40),
    # Non-Latin script on the responder's side: the label cannot overlap an
    # English query at all, which is the case that was refusing outright.
    ("Seoul, South Korea", "서울", 37.567, 126.978, 40),
    ("Tokyo, Japan", "東京", 35.690, 139.692, 60),
    ("Beijing", "北京", 39.906, 116.391, 60),
    ("Moscow, Russia", "Москва", 55.756, 37.617, 60),
    ("Athens, Greece", "Αθήνα", 37.984, 23.728, 40),
    # Renamed cities, where the old exonym is still what many questions say.
    ("Mumbai", "Bombay", 19.076, 72.878, 50),
    ("Kolkata", "Calcutta", 22.573, 88.364, 50),
    ("Chennai", "Madras", 13.083, 80.271, 50),
    ("Guangzhou", "Canton, China", 23.129, 113.264, 60),
    # The formal country name, which is where a refusal turns into a wrong
    # answer rather than a refusal: the longer span geocodes to an embassy, the
    # confidence gate refuses that class correctly, and the next candidate is a
    # bare country word that matches a village somewhere else exactly.
    ("Seoul, Republic of Korea", "Seoul, South Korea", 37.567, 126.978, 40),
    ("Munich, Federal Republic of Germany", "Munich, Germany", 48.137, 11.576, 40),
    # A country rather than a city: a much larger target, and the one whose
    # rescue previously laundered embassies and consulates into place hits.
    ("Côte d'Ivoire", "Ivory Coast", 7.54, -5.55, 700),
]

# A question that names the place naturally, so the extractor is exercised
# rather than bypassed. Air quality because every cell has it.
ASK_TEMPLATE = "What is the air quality in {}?"


def haversine_km(a_lat, a_lon, b_lat, b_lon):
    r = 6371.0088
    p1, p2 = math.radians(a_lat), math.radians(b_lat)
    dp = math.radians(b_lat - a_lat)
    dl = math.radians(b_lon - a_lon)
    h = math.sin(dp / 2) ** 2 + math.cos(p1) * math.cos(p2) * math.sin(dl / 2) ** 2
    return 2 * r * math.asin(math.sqrt(h))


def post(origin, path, body, timeout=120):
    req = urllib.request.Request(
        origin + path,
        data=json.dumps(body).encode(),
        headers={"content-type": "application/json",
                 "User-Agent": "emem-place-resolution-check"},
    )
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, json.load(r)
    except urllib.error.HTTPError as e:
        try:
            return e.code, json.load(e)
        except Exception:
            return e.code, {}
    except Exception as e:
        return None, {"_transport": str(e)}


def get(origin, path, timeout=60):
    req = urllib.request.Request(
        origin + path, headers={"User-Agent": "emem-place-resolution-check"})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return json.load(r)
    except Exception:
        return {}


def cell_centre(origin, cell):
    d = get(origin, f"/v1/cells/{cell}/info")
    c = (d or {}).get("centre") or {}
    if "lat_deg" in c and "lng_deg" in c:
        return c["lat_deg"], c["lng_deg"]
    return None


def resolve_via_ask(origin, query):
    """(verdict, cell-or-None, label) for the natural-question path.

    TWO ENVELOPES, and the first version of this only knew one. A refusal
    carries `status: needs_location` with `candidate_cell` beside it; an
    answer carries neither, because it has nothing to apologise for -- the
    place it used is under `place_resolved`. Reading only the refusal shape
    made every successful answer look like "no place extracted", which is
    the failure inverted: the check reported the fix as broken on the run
    that proved it worked.
    """
    code, d = post(origin, "/v1/ask", {"q": ASK_TEMPLATE.format(query)})
    if code is None:
        return "unreachable", None, d.get("_transport", "")
    pr = d.get("place_resolved") or {}
    if pr.get("cell64"):
        # lat/lng are RIGHT HERE. The first version threw them away and then
        # spent a second request asking /v1/cells/{cell}/info for the same
        # numbers, which is a third of this check's traffic bought for nothing.
        if pr.get("lat") is not None and pr.get("lng") is not None:
            return "answered", (pr["lat"], pr["lng"]), pr.get("label", "")
        return "answered", pr["cell64"], pr.get("label", "")
    status = d.get("status", "")
    cell = d.get("candidate_cell") or d.get("cell64") or d.get("cell")
    label = d.get("candidate_label") or ""
    if not cell:
        return "no_place", None, f"status={status} extracted={d.get('extracted')!r}"
    return status or "ok", cell, label


def resolve_via_locate(origin, query):
    code, d = post(origin, "/v1/locate", {"query": query})
    if code is None:
        return None, d.get("_transport", "")
    c = d.get("centre") or {}
    if "lat_deg" not in c:
        return None, f"HTTP {code}: {str(d)[:80]}"
    return (c["lat_deg"], c["lng_deg"]), d.get("label") or d.get("place") or ""


def check_one(origin, query, lat, lon, radius, verbose):
    """One spelling, both surfaces. Returns a list of failure strings."""
    bad = []

    pos, detail = resolve_via_locate(origin, query)
    if pos is None:
        bad.append(f"locate  {query!r}: no centre ({detail})")
    else:
        km = haversine_km(lat, lon, *pos)
        if km > radius:
            bad.append(f"locate  {query!r} landed {km:,.0f} km away "
                       f"({pos[0]:.3f},{pos[1]:.3f}), limit {radius} km")
        elif verbose:
            print(f"    locate  {query:<22} {km:6.1f} km  {detail[:40]}")

    status, cell, label = resolve_via_ask(origin, query)
    if status == "unreachable":
        bad.append(f"ask     {query!r}: unreachable ({label})")
    elif cell is None:
        bad.append(f"ask     {query!r}: no place extracted ({label})")
    else:
        centre = cell if isinstance(cell, tuple) else cell_centre(origin, cell)
        if centre is None:
            bad.append(f"ask     {query!r}: cell {cell} has no centre")
        else:
            km = haversine_km(lat, lon, *centre)
            if km > radius:
                bad.append(f"ask     {query!r} answered about somewhere "
                           f"{km:,.0f} km away: {label[:40]!r}")
            elif status != "answered":
                # Right place, still refused: the confidence floor rejecting a
                # correct match. This is the exonym defect exactly.
                bad.append(f"ask     {query!r} found the right cell "
                           f"({km:.0f} km) and REFUSED it: {label[:44]!r}")
            elif verbose:
                print(f"    ask     {query:<22} {km:6.1f} km  {status}")
    return bad


def self_test(origin):
    """A control, because a battery where everything passes may be a battery
    that is not measuring. Assert a deliberately WRONG position for a place
    the responder resolves well, and require this file to report it."""
    bad = check_one(origin, "Munich", 35.690, 139.692, 40, False)  # Munich at Tokyo
    if not bad:
        return ["the control did not fail: this check cannot detect a wrong "
                "place, so its passes below mean nothing"]
    return []


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--origin", default="https://emem.dev")
    ap.add_argument("--verbose", action="store_true")
    ap.add_argument("--only", help="substring filter over the queries")
    ap.add_argument("--pace-s", type=float, default=1.5,
                    help="seconds between spellings; this check is not a load test")
    args = ap.parse_args()

    broken = self_test(args.origin)
    if broken:
        for b in broken:
            print(f"  {b}")
        return 1

    rows = [r for r in PLACES
            if not args.only or args.only.lower() in (r[0] + r[1]).lower()]
    failures, checked = [], 0
    for primary, partner, lat, lon, radius in rows:
        for query in (primary, partner):
            if checked:
                time.sleep(args.pace_s)
            checked += 1
            failures += check_one(args.origin, query, lat, lon, radius, args.verbose)

    # A responder that went away mid-run has told us nothing about how it
    # resolves places, and listing every row as a resolution failure would say
    # the opposite. This happens on ordinary days here: a deploy takes the
    # port down for about ninety seconds, and the first run of this file was
    # cut in half by one. Unreachable is undetermined.
    unreachable = [f for f in failures if "unreachable" in f or "Connection refused" in f]
    if unreachable:
        print(f"\n  {len(unreachable)} of {checked * 2} probes could not reach "
              f"{args.origin}.")
        print("  Undetermined, not failed: a place check has to reach the")
        print("  responder to say anything about what it resolves. If a deploy")
        print("  is running, wait for the restart and run this again.")
        return 1

    print(f"\n  {checked} spellings across {len(rows)} places, "
          f"{args.origin}, control passed")
    if failures:
        print(f"\n  {len(failures)} did not resolve to the place they name:")
        for f in failures:
            print(f"    {f}")
        return 1
    print("  Every spelling resolved to the place it names, on both surfaces.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
