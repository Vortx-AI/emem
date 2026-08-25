#!/usr/bin/env python3
"""Does /v1/ask answer every KIND of question, or only the ones we demo?

    scripts/manual/question_types.py [--origin https://emem.dev] [--only weather]

Why this exists
---------------
The demos on this site ask about air, vegetation, heat and water, and they pass.
That is four topics out of the twenty two this responder declares algorithms
for, chosen by whoever wrote the demos, which makes "the protocol answers
questions about the physical world" a claim resting on the sample most likely to
work. A surface is not stable across question types until someone has asked it
one of each and looked at what came back.

So the questions here are keyed to the topic registry the responder publishes at
`/v1/topics`, not to a list I made up. If a topic is added there and no question
exists for it, this says so rather than quietly covering twenty two of twenty
three.

What counts as answered
-----------------------
Not HTTP 200. A 200 carrying `emem.error.v1` is a failure with a good status
line, and `needs_location` on a question that named a London junction is a
refusal, not an answer. A question is answered when the envelope carries a
non-empty `answer`, a `place_resolved` for the place that was named, and at
least one `fact_cid` behind it. The last one is the point of the whole system:
prose with nothing to cite is what this protocol exists to replace.

The control
-----------
A question with no place in it must NOT come back answered. Without that, a
responder that answered everything from a default location would pass every row
above, and a battery where everything passes is indistinguishable from a battery
that is not looking.
"""
import argparse
import json
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from lib_patience import patient  # noqa: E402

# One natural question per declared topic. Phrased the way somebody would ask,
# not in the vocabulary of the band, because a router that only works when the
# question already contains the answer's field name is not routing.
QUESTIONS = {
    "weather_now": "What is the weather at Trafalgar Square, London?",
    "public_health": "What is the air quality in Marble Arch, London?",
    "vegetation_condition": "How green is Hyde Park, London?",
    "water_quality": "What is the water like at Tower Bridge, London?",
    "topography": "How high above sea level is Hampstead Heath, London?",
    "elevation_global_topobathy": "What is the elevation at Primrose Hill, London?",
    "soil_intelligence": "What is the soil like at Kew Gardens, London?",
    "soil_bare": "How much bare soil is there at Wormwood Scrubs, London?",
    "urban_livability": "How walkable is Soho, London?",
    "built_up_human_geography": "How built up is Canary Wharf, London?",
    "real_estate": "What is the built environment like around Old Street, London?",
    "agriculture": "What are the crops doing near Cambridge, England?",
    "analytics": "What changed at Battersea Power Station, London?",
    "carbon_credits": "How much tree cover is there at Epping Forest, England?",
    "esg": "What is the environmental profile of Canary Wharf, London?",
    "fire_burn_severity": "Has there been a fire at Saddleworth Moor, England?",
    "flood_history_long_term": "Has Tewkesbury, England flooded before?",
    "flood_risk_composite": "What is the flood risk at Tewkesbury, England?",
    "flood_water_event_window": "Is there standing water at the Somerset Levels, England?",
    "snow": "Is there snow at Ben Nevis, Scotland?",
    "parametric_insurance": "What rainfall has Tewkesbury, England had?",
    "foundation_embedding": "What places look like Canary Wharf, London?",
}

# No place. Must not come back answered.
CONTROL = "What is the meaning of a signed fact?"


def ask(origin: str, q: str, timeout: int = 180):
    req = urllib.request.Request(
        origin + "/v1/ask",
        data=json.dumps({"q": q}).encode(),
        headers={"content-type": "application/json",
                 "User-Agent": "emem-question-types-check"},
    )
    try:
        with patient(req, timeout=timeout) as r:
            return r.status, json.load(r)
    except urllib.error.HTTPError as e:
        try:
            return e.code, json.load(e)
        except Exception:
            return e.code, {}
    except Exception as e:
        return None, {"_transport": str(e)}


def verdict(code, d):
    """(answered, why-not). Answered means citable, not merely 200."""
    if code is None:
        return False, f"unreachable ({d.get('_transport','')[:60]})"
    if d.get("schema") == "emem.error.v1" or d.get("code"):
        return False, f"error envelope: {str(d.get('message'))[:70]}"
    status = d.get("status")
    if status and status != "ok":
        return False, f"{status}: {str(d.get('confidence_reason') or '')[:50]}"
    answer = (d.get("answer") or "").strip()
    if not answer:
        return False, "no answer text"
    if not (d.get("place_resolved") or {}).get("cell64"):
        return False, "no place resolved"
    if not d.get("fact_cids"):
        # WHY there is nothing to cite, because the two reasons are different
        # claims and only one of them is about the question.
        #
        # A band that timed out or hit the cold-fetch ceiling says so in
        # materialize_notes, and that is a statement about how busy this
        # responder was, not about whether it can answer this kind of question.
        # The first run of this file reported eleven topics as unanswerable
        # while the site was rebuilding its channel at three requests a second;
        # asked again on a quiet responder the same question came back with
        # twelve fact_cids. A checker that cannot tell those apart manufactures
        # a protocol failure out of its own load.
        notes = d.get("materialize_notes") or []
        deferred = [n.get("band") for n in notes
                    if isinstance(n, dict) and not n.get("absence")
                    and any(k in str(n.get("reason", "")).lower()
                            for k in ("budget", "ceiling", "deferred", "too slow"))]
        if deferred:
            return False, (f"nothing to cite; {len(deferred)} band(s) deferred under load "
                           f"({', '.join(str(b) for b in deferred[:3])}) - ask again when quiet")
        return False, "answered with nothing to cite, and no band says why"
    return True, ""


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--origin", default="https://emem.dev")
    ap.add_argument("--only", help="substring filter over topic names")
    ap.add_argument("--pace-s", type=float, default=1.0)
    args = ap.parse_args()

    # The registry is the source of the list, so a topic added there without a
    # question here is reported rather than silently uncovered.
    try:
        with patient(urllib.request.Request(f"{args.origin}/v1/topics"), timeout=60) as r:
            declared = set(json.load(r)["topics"]["algorithms_for_topic"])
    except Exception as e:
        print(f"  could not read {args.origin}/v1/topics ({e}); undetermined, not clean.")
        return 1
    missing = sorted(declared - set(QUESTIONS))
    extra = sorted(set(QUESTIONS) - declared)

    code, d = ask(args.origin, CONTROL)
    ok, _ = verdict(code, d)
    if ok:
        print("  THE CONTROL WAS ANSWERED. A question naming no place came back with a")
        print("  resolved place and a citable answer, which means every row below would")
        print("  pass against a responder answering from somewhere it chose. Not run.")
        return 1

    rows = {k: v for k, v in QUESTIONS.items() if not args.only or args.only in k}
    failed = []
    for i, (topic, q) in enumerate(sorted(rows.items())):
        if i:
            time.sleep(args.pace_s)
        code, d = ask(args.origin, q)
        ok, why = verdict(code, d)
        routed = (d.get("topic_routing") or {}).get("matched_keywords") or []
        top = routed[0].get("topic") if routed and isinstance(routed[0], dict) else "?"
        print(f"  {'ok ' if ok else 'FAIL'} {topic:28} -> {str(top):24} {why}")
        if not ok:
            failed.append((topic, q, why))

    print(f"\n  {len(rows)} question type(s) against {args.origin}, control refused as it must")
    if missing:
        print(f"  {len(missing)} declared topic(s) have no question here: {', '.join(missing)}")
    if extra:
        print(f"  {len(extra)} question(s) name a topic the responder no longer declares: "
              f"{', '.join(extra)}")
    if failed:
        print(f"\n  {len(failed)} did not come back with a citable answer:")
        for topic, q, why in failed:
            print(f"    {topic}: {why}")
            print(f"      asked: {q}")
        return 1
    if missing or extra:
        return 1
    print("  Every declared topic answers, with a place and something to cite.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
