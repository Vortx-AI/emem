#!/usr/bin/env python3
"""Assert that emem can still see the ground, and fail loudly when it cannot.

Why this exists
---------------
`live_perception` disappeared from every answer for the length of a migration
and nothing reported it. The cause was a loopback default pointing at a host
that no longer ran the perception service, and the reason it was invisible is
structural, not an oversight:

    fetch_live_perception(...) -> Option<JsonValue>

Every failure returns `None` -- connection refused, non-2xx, malformed JSON --
and so does the perfectly ordinary case of a place with no cameras near it.
Absence of the service and absence of cameras are the same value, so there is
nothing for a log line to distinguish. The answer stayed well-formed, correctly
signed, and quietly stopped describing the street.

A probe fixes that only if it asserts against a place where the answer is known
in advance. Trafalgar Square has 267 cameras; if it reports none, that is not a
quiet place, it is a broken pipeline.

Three distinct faults, three exit codes, because the remedies differ:

  2  no live_perception block   -> perception service unreachable
                                   (tunnel down, or geoqa-perception stopped)
  3  block present, no cameras  -> service up, camera registry empty or the
                                   query is wrong. A database fault, not a
                                   network one.
  4  cameras but stale clips    -> registry fine, INGESTER dead. This is the
                                   one that will matter after the TfL frame
                                   fetcher moves off the OCI box: cameras stay
                                   listed forever, clips silently stop.

Exit 0 is the only success. Anything non-zero trips OnFailure.
"""
import json
import os
import sys
import urllib.request

# A cell with cameras, chosen because the answer is known. A probe pointed at a
# place that might legitimately have none cannot tell "broken" from "quiet".
CELL = os.environ.get("EMEM_PROBE_CELL", "defi.zb64a.cAzU.zfa27")
QUESTION = os.environ.get("EMEM_PROBE_QUESTION", "what is happening at Trafalgar Square")
ORIGIN = os.environ.get("EMEM_PROBE_ORIGIN", "http://127.0.0.1:5051")
MIN_CAMERAS = int(os.environ.get("EMEM_PROBE_MIN_CAMERAS", "1"))
# Clip cadence upstream is ~1230 s. Three generations of silence is a stopped
# ingester rather than an unlucky sample; tighter than that alarms on jitter.
MAX_CLIP_AGE_S = int(os.environ.get("EMEM_PROBE_MAX_CLIP_AGE_S", "3600"))
TIMEOUT_S = int(os.environ.get("EMEM_PROBE_TIMEOUT_S", "60"))


def main() -> int:
    body = json.dumps({"question": QUESTION}).encode()
    req = urllib.request.Request(
        ORIGIN.rstrip("/") + "/v1/ask", data=body, method="POST",
        headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=TIMEOUT_S) as r:
            ask = json.loads(r.read())
    except Exception as e:  # noqa: BLE001
        print(f"FAIL: /v1/ask unreachable at {ORIGIN}: {type(e).__name__}: {e}")
        return 5

    lp = ask.get("live_perception")
    if not lp:
        print(f"FAIL: no live_perception for {CELL}. The perception service is "
              f"unreachable -- check emem-perception-tunnel here and "
              f"geoqa-perception on the geo.qa box. Answers are still correct "
              f"and signed, they just describe the surface, not the street.")
        return 2

    cams = lp.get("cameras_near") or 0
    if cams < MIN_CAMERAS:
        print(f"FAIL: live_perception present but cameras_near={cams} "
              f"(expected >= {MIN_CAMERAS}) for {CELL}. Service is up, so this "
              f"is the camera registry or the spatial query, not the network.")
        return 3

    age = lp.get("newest_clip_age_s")
    if age is None:
        print(f"FAIL: {cams} cameras but no clip carries a usable age for {CELL}. "
              f"Nothing has been retained recently -- check the frame ingester.")
        return 4
    if age > MAX_CLIP_AGE_S:
        print(f"FAIL: {cams} cameras but freshest clip is {age}s old "
              f"(limit {MAX_CLIP_AGE_S}s) for {CELL}. The registry is fine and "
              f"the INGESTER is not writing -- check geoqa-frames.")
        return 4

    print(f"ok: {cams} cameras, {lp.get('cameras_with_retained_clips')} with "
          f"retained clips, freshest {age}s")
    return 0


if __name__ == "__main__":
    sys.exit(main())
