#!/usr/bin/env python3
"""
Verifiable lending due-diligence against emem.

A bank underwrites a loan against a parcel. The credit memo asserts physical
facts about the land. This script grounds each of those facts:

    place -> canonical cell64 address -> signed facts -> published algorithms
          -> a memo where every physical claim carries a resolvable token

...then runs both the naive memo and the grounded memo through emem-guard and
reports, from the actual responses, what the guard did and did not catch.

No API key. No account. Python 3 standard library only.

    python3 run.py
    python3 run.py --probe
"""

import argparse
import json
import math
import sys
import urllib.error
import urllib.request

DEFAULT_BASE = "https://emem.dev"
DEFAULT_LAT = 12.9352
DEFAULT_LNG = 77.6245

# Band keys verified to auto-materialize on the public responder. Family roots
# ("dem", "soilgrids") are NOT band keys and come back as unknown_band.
BANDS = [
    "indices.ndvi",
    "modis.ndvi_mean",
    "copdem30m.elevation_mean",
    "surface_water.recurrence",
    "sentinel1_raw",
    "weather.temperature_2m",
    "weather.precipitation_mm",
    "weather.cloud_cover",
    "overture.buildings.count",
    "overture.places.count",
    "overture.transportation.road_length_m",
    "gmrt.topobathy_mean",
    "geotessera",
    "koppen",
]

# Output ordinal of vegetation_class_from_ndvi@1, per its published evaluation tree.
VEG_CLASSES = ["water_or_snow", "bare", "sparse", "moderate", "dense"]

B32 = "abcdefghijklmnopqrstuvwxyz234567"
WIDTH = 92


# ---------------------------------------------------------------- transport --


def die(msg):
    """Fail honestly and loudly. Never continue with a fabricated value."""
    sys.stderr.write("\nERROR: %s\n" % msg)
    sys.stderr.write("This example never substitutes a placeholder for a value it "
                     "could not fetch, so it stops here.\n")
    sys.exit(1)


def _open(req, timeout, what):
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            raw = resp.read()
    except urllib.error.HTTPError as e:
        body = ""
        try:
            body = e.read().decode("utf-8", "replace")[:400]
        except Exception:
            pass
        die("%s -> HTTP %s %s\n%s" % (what, e.code, e.reason, body))
    except urllib.error.URLError as e:
        die("%s -> network failure: %s" % (what, e.reason))
    except Exception as e:  # socket timeouts, DNS, TLS
        die("%s -> %s: %s" % (what, type(e).__name__, e))
    try:
        return json.loads(raw.decode("utf-8"))
    except ValueError as e:
        die("%s -> responder did not return JSON: %s" % (what, e))


def post(base, path, body, timeout):
    req = urllib.request.Request(
        base + path,
        data=json.dumps(body).encode("utf-8"),
        headers={"content-type": "application/json",
                 "accept": "application/json",
                 "user-agent": "emem-verifiable-lending-example/1"},
        method="POST",
    )
    return _open(req, timeout, "POST " + path)


def get(base, path, timeout):
    req = urllib.request.Request(
        base + path,
        headers={"accept": "application/json",
                 "user-agent": "emem-verifiable-lending-example/1"},
        method="GET",
    )
    return _open(req, timeout, "GET " + path)


# ------------------------------------------------------------- presentation --


def rule(ch="-"):
    print(ch * WIDTH)


def head(n, title):
    print()
    rule("=")
    print("STEP %s  %s" % (n, title))
    rule("=")


def fmt_val(v, maxlen=44):
    """Exact rendering. No rounding in the evidence table."""
    if isinstance(v, list):
        first = json.dumps(v[0])[:10] if v else "-"
        return "[vector %d dims, first %s]" % (len(v), first)
    s = json.dumps(v)
    return s if len(s) <= maxlen else s[: maxlen - 3] + "..."


def wrap(text, indent="  ", width=WIDTH):
    """Minimal word wrap; textwrap would also be stdlib but this keeps it local."""
    out, line = [], indent
    for word in text.split():
        if len(line) + len(word) + 1 > width and line.strip():
            out.append(line.rstrip())
            line = indent
        line += word + " "
    if line.strip():
        out.append(line.rstrip())
    return "\n".join(out)


def sigmoid(x):
    if x >= 0:
        return 1.0 / (1.0 + math.exp(-x))
    e = math.exp(x)
    return e / (1.0 + e)


def relu(x):
    return x if x > 0.0 else 0.0


def clamp01(x):
    return 0.0 if x < 0.0 else (1.0 if x > 1.0 else x)


def corrupt_cid(cid, n=4, at=10):
    """Well-formed but wrong: rotate n characters within the base32 alphabet."""
    out = list(cid)
    for i in range(at, min(at + n, len(out))):
        c = out[i]
        out[i] = B32[(B32.index(c) + 1) % len(B32)] if c in B32 else "a"
    return "".join(out)


# ------------------------------------------------------------------ helpers --


def guard(base, texts, timeout):
    return post(base, "/v1/guard/verdict?shape=native",
                {"texts": texts, "claim_gating": True}, timeout)


def verdict_line(v):
    r = v.get("receipt") or {}
    return "action=%-5s code=%-16s checked=%-2s citations_found=%-2s fact_cids=%d" % (
        v.get("action"), str(v.get("code")), v.get("checked"),
        v.get("citations_found"), len(r.get("fact_cids") or []))


def need(facts, band):
    """Return the fact for a band or stop. Never invent a value."""
    f = facts.get(band)
    if f is None:
        die("band %r did not materialize, so the arithmetic below cannot be "
            "computed honestly. See the materialize_notes printed above." % band)
    return f


# ------------------------------------------------------------- main scenario --


def scenario(args):
    base, timeout = args.base, args.timeout

    print()
    rule("=")
    print("VERIFIABLE LENDING DUE-DILIGENCE".center(WIDTH))
    print("underwriting a parcel against emem".center(WIDTH))
    rule("=")
    print(wrap("A credit memo makes physical assertions about a piece of land. "
               "Below, the same assertions are made twice: once the way a model "
               "writes them by default, and once with every physical claim bound "
               "to a signed fact. Then both go through emem-guard."))
    print()
    print("  responder : %s" % base)
    print("  parcel    : lat %s, lng %s" % (args.lat, args.lng))

    # -- 1. ground the parcel ------------------------------------------------
    head(1, "GROUND THE PARCEL  (POST /v1/locate)")
    loc = post(base, "/v1/locate", {"lat": args.lat, "lng": args.lng}, timeout)
    cell = loc.get("cell64") or loc.get("cell")
    if not cell:
        die("/v1/locate returned no cell64. Keys present: %s" % sorted(loc.keys()))
    label = loc.get("place_label")
    centre = loc.get("centre") or {}
    print("  cell64      : %s" % cell)
    print("  place_label : %s" % (label if label else
                                  "null  (the responder returns no label for a direct "
                                  "lat/lng; it is not a geocoded name)"))
    if centre:
        print("  cell centre : lat %.7f, lng %.7f"
              % (centre.get("lat_deg", float("nan")), centre.get("lng_deg", float("nan"))))
    print("  resolved by : %s" % loc.get("via"))
    print()
    print(wrap("This address is the join key. Any agent that locates the same "
               "coordinate gets the same cell64, so two memos about this parcel "
               "are about the same parcel and not about two paraphrases of it."))

    # -- 2. recall signed facts ----------------------------------------------
    head(2, "RECALL SIGNED FACTS  (POST /v1/recall, %d bands)" % len(BANDS))
    rec = post(base, "/v1/recall", {"cell": cell, "bands": BANDS}, timeout)
    fact_list = rec.get("facts") or []
    facts = {f["band"]: f for f in fact_list}
    notes = rec.get("materialize_notes") or []
    note_by_band = {n.get("band"): n for n in notes}

    print("  %-38s %-24s %-6s %s" % ("BAND", "VALUE", "UNIT", "CONF"))
    rule()
    for band in BANDS:
        f = facts.get(band)
        if f is None:
            n = note_by_band.get(band)
            if n is None:
                print("  %-38s NOT RETURNED (and no materialize_note was given)" % band)
            else:
                print("  %-38s %s / %s" % (band, n.get("status"),
                                           n.get("reason_class", "no reason_class")))
                reason = n.get("reason")
                if reason:
                    print(wrap(str(reason), indent="      "))
            continue
        conf = f.get("confidence")
        print("  %-38s %-24s %-6s %s" % (
            band, fmt_val(f.get("value")), str(f.get("unit") or "-"),
            ("%.2f" % conf) if isinstance(conf, (int, float)) else "-"))
        print("      %s" % f.get("memory_token"))

    got, miss = len(facts), len(BANDS) - len(facts)
    print()
    print("  %d of %d bands materialized%s." % (got, len(BANDS),
          ("; %d did not, with reasons above" % miss) if miss else ""))

    skipped = [n for n in notes if n.get("status") != "materialized"]
    if skipped:
        print("  materialize_notes reporting a non-materialized band:")
        for n in skipped:
            print("    %-38s %s / %s" % (n.get("band"), n.get("status"),
                                         n.get("reason_class", "-")))

    # receipt
    r = rec.get("receipt") or {}
    mp = r.get("merkle_proof") or {}
    print()
    print("  RECEIPT")
    print("    fact_cids            : %d" % len(r.get("fact_cids") or []))
    print("    merkle_proof         : %s"
          % ("present (leaf_index=%s, path_len=%s, root %d bytes)"
             % (mp.get("leaf_index"), len(mp.get("path") or []),
                len(mp.get("root") or [])) if mp else "absent"))
    print("    signature_b32        : %s" % str(r.get("signature_b32"))[:52] + "...")
    print("    responder_pubkey_b32 : %s" % r.get("responder_pubkey_b32"))
    print("    served_at            : %s" % r.get("served_at"))
    print()
    print(wrap("The receipt is signed by the responder's key. The signature covers "
               "the fact set, so a third party can check this recall offline "
               "without trusting the bank, the borrower, or emem."))

    # -- 3. published algorithms --------------------------------------------
    head(3, "APPLY PUBLISHED ALGORITHMS  (GET /v1/algorithms/<key>)")
    print(wrap("The scores below are not invented here. Both recipes are published "
               "in the content-addressed algorithm registry, and the responder "
               "returns its own computed value alongside the facts, so this "
               "script's arithmetic can be checked against the responder's."))

    responder_vals = {a.get("algorithm_key"): a.get("value")
                      for a in (rec.get("applicable_algorithms") or [])}

    # --- vegetation_class_from_ndvi@1
    veg_def = get(base, "/v1/algorithms/vegetation_class_from_ndvi@1", timeout)
    ndvi_f = need(facts, "indices.ndvi")
    n = float(ndvi_f["value"])
    print()
    print("  vegetation_class_from_ndvi@1")
    print(wrap("published formula: " + str(veg_def.get("formula")), indent="    "))
    print("    inputs:")
    print("      indices.ndvi = %r" % n)
    print("      %s" % ndvi_f["memory_token"])
    if n < 0:
        veg_i = 0
    elif n < 0.1:
        veg_i = 1
    elif n < 0.3:
        veg_i = 2
    elif n < 0.5:
        veg_i = 3
    else:
        veg_i = 4
    veg = VEG_CLASSES[veg_i]
    print("    arithmetic:")
    print("      n = %.6f -> %s -> class ordinal %d = %r"
          % (n, "n < 0.3 and n >= 0.1" if veg_i == 2 else "threshold match",
             veg_i, veg))
    rv = responder_vals.get("vegetation_class_from_ndvi@1")
    if rv is None:
        print("    cross-check: responder returned no value for this key.")
    elif abs(float(rv) - veg_i) < 1e-9:
        print("    cross-check: responder computed %s -> AGREES with this script." % rv)
    else:
        print("    cross-check: DIFFERS. responder=%s, this script=%s. "
              "Reporting the difference rather than the expectation." % (rv, veg_i))

    # --- flood_risk@2
    fr_def = get(base, "/v1/algorithms/flood_risk@2", timeout)
    elev_f = need(facts, "copdem30m.elevation_mean")
    rec_f = need(facts, "surface_water.recurrence")
    s1_f = need(facts, "sentinel1_raw")
    gmrt_f = need(facts, "gmrt.topobathy_mean")
    elev = float(elev_f["value"])
    recur = float(rec_f["value"])
    s1 = float(s1_f["value"])
    gmrt = float(gmrt_f["value"])

    print()
    print("  flood_risk@2")
    print(wrap("published formula: " + str(fr_def.get("formula")), indent="    "))
    print("    inputs:")
    for f in (rec_f, elev_f, gmrt_f, s1_f):
        print("      %-38s = %-20s" % (f["band"], fmt_val(f["value"], 20)))
        print("        %s" % f["memory_token"])

    dem_gap = abs(elev - gmrt)
    dem_agreement = 1.0 if dem_gap <= 5.0 else 0.5
    t1 = 0.55 * (recur / 100.0)
    t2 = 0.25 * dem_agreement * (relu(50.0 - elev) / 50.0)
    sig_in = (-15.0 - s1) / 2.0
    t3 = 0.20 * sigmoid(sig_in)
    total = t1 + t2 + t3
    score = clamp01(total)

    print("    arithmetic:")
    print("      dem_agreement : |%.4f - %.4f| = %.4f %s 5  ->  %.1f"
          % (elev, gmrt, dem_gap, "<=" if dem_gap <= 5 else ">", dem_agreement))
    print("      term 1 water  : 0.55 * (%.4f / 100)                  = %.8f" % (recur, t1))
    print("      term 2 elev   : 0.25 * %.1f * (relu(50 - %.4f) / 50)  = %.8f"
          % (dem_agreement, elev, t2))
    print("      term 3 radar  : 0.20 * sigmoid((-15 - %.4f) / 2)      = %.8f" % (s1, t3))
    print("      sum                                                   = %.8f" % total)
    print("      clamp01                                               = %.8f" % score)
    band_name = "low" if score <= 0.2 else ("moderate" if score <= 0.5 else "high")
    print("      band          : %.8f -> %s" % (score, band_name))

    rv2 = responder_vals.get("flood_risk@2")
    if rv2 is None:
        print("    cross-check: responder returned no value for this key.")
    elif abs(float(rv2) - score) <= 1e-9:
        print("    cross-check: responder computed %.17g -> AGREES with this script "
              "(within 1e-9)." % float(rv2))
    else:
        print("    cross-check: DIFFERS. responder=%.17g, this script=%.17g, delta=%.3g. "
              "Reporting the difference rather than the expectation."
              % (float(rv2), score, float(rv2) - score))

    # -- 4. two memos --------------------------------------------------------
    head(4, "TWO CREDIT MEMOS ABOUT THE SAME PARCEL")

    place = "Koramangala"
    naive = [
        "Collateral for application APP-2291 is a parcel at %s, Bengaluru." % place,
        "The collateral parcel at %s sits at %.1f m elevation, well clear of any "
        "flood plain." % (place, elev),
        "Vegetation cover at the parcel is %s, with an NDVI of %.2f." % (veg, n),
        "Surface water recurrence at the site is %.0f percent, so flood risk is "
        "negligible." % recur,
        "Underwriting recommends approval at 70 percent LTV.",
    ]
    grounded = [
        "Collateral for application APP-2291 is the parcel at cell64 %s." % cell,
        "The collateral parcel at %s sits at %.1f m elevation [%s], well clear of "
        "any flood plain." % (place, elev, elev_f["memory_token"]),
        "Vegetation cover at the parcel is %s, with an NDVI of %.2f [%s], per "
        "published algorithm vegetation_class_from_ndvi@1."
        % (veg, n, ndvi_f["memory_token"]),
        "Surface water recurrence at the site is %.0f percent [%s]; flood_risk@2 "
        "scores %.5f (%s) from that recurrence, elevation [%s] and radar "
        "backscatter [%s]."
        % (recur, rec_f["memory_token"], score, band_name,
           elev_f["memory_token"], s1_f["memory_token"]),
        "Underwriting recommends approval at 70 percent LTV.",
    ]

    print("  MEMO A - naive. Confident physical claims, no evidence.")
    rule()
    for s in naive:
        print(wrap(s, indent="    "))
    print()
    print("  MEMO B - grounded. Every physical claim carries its emem:fact: token.")
    rule()
    for s in grounded:
        print(wrap(s, indent="    "))

    # -- 5. guard both -------------------------------------------------------
    head(5, "GUARD VERDICTS  (POST /v1/guard/verdict?shape=native)")
    v_naive = guard(base, naive, timeout)
    v_grounded = guard(base, grounded, timeout)

    print("  MEMO A (naive)    %s" % verdict_line(v_naive))
    print("  MEMO B (grounded) %s" % verdict_line(v_grounded))
    print()
    for name, v in (("MEMO A", v_naive), ("MEMO B", v_grounded)):
        c = v.get("claim")
        print("  %s" % name)
        print("    action : %s" % v.get("action"))
        print("    code   : %s" % v.get("code"))
        print("    fix    : %s" % v.get("fix"))
        if c:
            print("    claim it named:")
            print("      sentence    : %s" % c.get("sentence"))
            print("      magnitude   : %s   quantity: %s" % (c.get("magnitude"),
                                                             c.get("quantity")))
            print("      anchor      : %s   source_band: %s" % (c.get("anchor"),
                                                                c.get("source_band")))
        else:
            print("    claim it named: none")
        print()
    adv = v_naive.get("advisory")
    if adv:
        print(wrap("The responder marks this verdict advisory=%s. %s"
                   % (adv, v_naive.get("advisory_note") or "")))

    # -- 6. findings ---------------------------------------------------------
    head(6, "FINDINGS  (computed from the responses above)")

    per_sentence = []
    for s in naive:
        vv = guard(base, [s], timeout)
        per_sentence.append((s, vv))

    n_denied = sum(1 for _, vv in per_sentence if vv.get("action") == "deny")
    numeric = [(s, vv) for s, vv in per_sentence if any(ch.isdigit() for ch in s)]
    allowed_numeric = [(s, vv) for s, vv in numeric if vv.get("action") != "deny"]

    print("  1. The naive memo verdict was %s%s."
          % (str(v_naive.get("action")).upper(),
             " (%s)" % v_naive.get("code") if v_naive.get("code") else ""))
    cn = v_naive.get("claim")
    if cn:
        print(wrap("The guard named exactly one claim, and reported only that one: "
                   "%r, attributed to source_band %r as quantity %r."
                   % (cn.get("sentence"), cn.get("source_band"), cn.get("quantity")),
                   indent="     "))
    print()
    print("  2. Sentence by sentence, the naive memo scores like this:")
    for s, vv in per_sentence:
        c = vv.get("claim") or {}
        print("       %-5s %-12s %s" % (vv.get("action"),
                                        str(c.get("quantity") or "-"),
                                        (s[:60] + "...") if len(s) > 63 else s))
    print(wrap("%d of %d sentences contain a numeral; the guard denied %d of them "
               "and allowed %d."
               % (len(numeric), len(per_sentence), n_denied, len(allowed_numeric)),
               indent="     "))
    if allowed_numeric:
        print(wrap("These uncited numeric claims were ALLOWED. They are exactly as "
                   "unevidenced as the one that was denied:", indent="     "))
        for s, _ in allowed_numeric:
            print(wrap("- %s" % s, indent="       "))
        quants = sorted({(vv.get("claim") or {}).get("quantity")
                         for _, vv in per_sentence
                         if vv.get("action") == "deny"})
        print(wrap("The quantity type the guard did fire on was %s. The detector "
                   "needs both a place-name anchor and a magnitude it recognises; "
                   "ratios, percentages and counts did not trip it here."
                   % (", ".join(repr(q) for q in quants) if quants else "none"),
                   indent="     "))

    rg = v_grounded.get("receipt") or {}
    gf, gc = len(rg.get("fact_cids") or []), v_grounded.get("citations_found")
    rn = v_naive.get("receipt") or {}
    nf, nc = len(rn.get("fact_cids") or []), v_naive.get("citations_found")
    print()
    print("  3. The grounded memo verdict was %s. citations_found=%s, "
          "receipt.fact_cids=%d." % (str(v_grounded.get("action")).upper(), gc, gf))
    if gc == gf and gf > 0:
        print(wrap("Every token in the memo resolved to a fact this responder holds "
                   "and re-signed into the verdict receipt.", indent="     "))
    elif gf < (gc or 0):
        print(wrap("WARNING: %d citations were found in the prose but only %d "
                   "resolved. Unresolvable tokens do not lower the action."
                   % (gc, gf), indent="     "))

    print()
    print("  4. The discriminator, len(receipt.fact_cids) == citations_found:")
    print("       naive    : %d == %s -> %s%s"
          % (nf, nc, nf == nc, "  (vacuously true: nothing was cited)"
             if nf == 0 and nc == 0 else ""))
    print("       grounded : %d == %s -> %s" % (gf, gc, gf == gc))
    print()
    print(wrap("What this run did NOT show is what the guard misses on citations "
               "themselves. Run `python3 run.py --probe` for the four controlled "
               "checks that isolate that."))
    print()


# ------------------------------------------------------------------- probe ---


def probe(args):
    base, timeout = args.base, args.timeout

    print()
    rule("=")
    print("PROBE: WHAT A GUARD VERDICT ACTUALLY PROVES".center(WIDTH))
    rule("=")
    print(wrap("Four texts. They differ only in the citation attached to one "
               "measurable claim. Everything below is measured live; where the "
               "responder disagrees with the documented expectation, the "
               "difference is printed rather than the expectation."))

    loc = post(base, "/v1/locate", {"lat": args.lat, "lng": args.lng}, timeout)
    cell = loc.get("cell64") or loc.get("cell")
    if not cell:
        die("/v1/locate returned no cell64.")
    rec = post(base, "/v1/recall",
               {"cell": cell, "bands": ["copdem30m.elevation_mean"]}, timeout)
    facts = {f["band"]: f for f in (rec.get("facts") or [])}
    f = facts.get("copdem30m.elevation_mean")
    if f is None:
        notes = rec.get("materialize_notes") or []
        die("copdem30m.elevation_mean did not materialize; notes: %s"
            % json.dumps(notes)[:400])
    elev = float(f["value"])
    good_tok = f["memory_token"]
    bad_tok = "emem:fact:%s:%s" % (cell, corrupt_cid(f["fact_cid"]))

    print()
    print("  cell64        : %s" % cell)
    print("  real fact     : copdem30m.elevation_mean = %r m" % elev)
    print("  valid token   : %s" % good_tok)
    print("  corrupted     : %s" % bad_tok)
    print(wrap("(the corrupted token is the valid one with 4 characters rotated "
               "inside the base32 alphabet: same shape, same length, wrong cid)",
               indent="  "))

    claim = "The parcel at Jagraon sits at %.1f m elevation." % elev
    cases = [
        ("A", "bare uncited measurable claim", claim,
         "deny CLAIM_UNGROUNDED"),
        ("B", "same claim + CORRUPTED token", "%s [%s]" % (claim, bad_tok),
         "deny CLAIM_UNGROUNDED, citations_found 1, fact_cids EMPTY"),
        ("C", "same claim + VALID token", "%s [%s]" % (claim, good_tok),
         "allow, fact_cids populated, merkle_proof present"),
        ("D", "VALID token, FALSE value in prose",
         "The parcel at Jagraon sits at 5000 m elevation. [%s]" % good_tok,
         "deny PROV_VALUE, fix correct_value"),
    ]

    results = []
    for key, desc, text, expect in cases:
        v = guard(base, [text], timeout)
        r = v.get("receipt") or {}
        results.append({
            "key": key, "desc": desc, "expect": expect, "verdict": v,
            "action": v.get("action"), "code": v.get("code"),
            "checked": v.get("checked"), "cites": v.get("citations_found"),
            "fact_cids": len(r.get("fact_cids") or []),
            "merkle": bool(r.get("merkle_proof")),
        })

    print()
    rule("=")
    print("  %-3s %-34s %-6s %-17s %-4s %-6s %s"
          % ("", "CHECK", "ACTION", "CODE", "CITE", "CIDS", "MERKLE"))
    rule()
    for r in results:
        print("  %-3s %-34s %-6s %-17s %-4s %-6s %s"
              % (r["key"], r["desc"], r["action"], str(r["code"]),
                 r["cites"], r["fact_cids"], "yes" if r["merkle"] else "no"))
    rule("=")

    print()
    print("  WHAT EACH CHECK PROVES")
    for r in results:
        print()
        print("  %s. %s" % (r["key"], r["desc"]))
        print("     documented expectation : %s" % r["expect"])
        print("     observed               : action=%s code=%s citations_found=%s "
              "fact_cids=%d" % (r["action"], r["code"], r["cites"], r["fact_cids"]))

    by = {r["key"]: r for r in results}
    a, b, c, d = by["A"], by["B"], by["C"], by["D"]

    # Observation-gated conclusions. Each line is printed only if measured.
    print()
    rule("=")
    print("  DERIVED RULE  (each line below is gated on what was just measured)")
    rule("=")

    conclusions = []
    if a["action"] == "deny" and a["fact_cids"] == 0:
        conclusions.append(
            "A denied with no citations. The guard does detect a bare measurable "
            "claim anchored to a place name.")
    else:
        conclusions.append(
            "A did NOT deny as documented. Observed action=%s code=%s. The "
            "documented expectation is reported, not asserted."
            % (a["action"], a["code"]))

    if b["action"] == "deny" and b["cites"] and b["fact_cids"] == 0:
        conclusions.append(
            "B is closed. A citation that is well-formed but does not resolve no "
            "longer buys an allow: citations_found=%s while fact_cids=%d, and the "
            "denial the sentence earned stands. Before 2026-08-08 this returned "
            "ALLOW, so pasting any plausible string defeated the gate more cheaply "
            "than grounding the claim did." % (b["cites"], b["fact_cids"]))
    elif b["action"] == "allow" and b["cites"] and b["fact_cids"] == 0:
        conclusions.append(
            "B is the uncomfortable one, and this responder still has it. A "
            "citation that is well-formed but does not resolve turned a DENY into "
            "an ALLOW. citations_found=%s while "
            "receipt.fact_cids is empty: the guard counted the token in the prose "
            "and could not resolve it, and still allowed."
            % b["cites"])
    else:
        conclusions.append(
            "B behaved differently from the documented expectation: action=%s, "
            "citations_found=%s, fact_cids=%d."
            % (b["action"], b["cites"], b["fact_cids"]))

    if c["action"] == "allow" and c["fact_cids"] > 0:
        conclusions.append(
            "C is the honest case: the token resolved, and the responder re-signed "
            "the resolved fact_cid into the verdict receipt%s."
            % (" with a merkle_proof" if c["merkle"] else ""))
    else:
        conclusions.append(
            "C behaved differently: action=%s, fact_cids=%d."
            % (c["action"], c["fact_cids"]))

    if d["action"] == "deny" and d["code"] == "PROV_VALUE":
        conclusions.append(
            "D is caught. The prose says 5000 m; the cited fact says %.1f m. The "
            "token resolves, verifies and binds to the right cell, so every "
            "provenance check passes and the receipt arithmetic balances. "
            "PROV_VALUE is the rule that reads the number inside the fact and "
            "compares it with the number in the sentence, and the fix is "
            "correct_value: the citation is the sound half, the prose is what "
            "disagrees with it." % elev)
    elif d["action"] == "allow":
        conclusions.append(
            "D was NOT caught on this responder. The prose says 5000 m; the cited "
            "fact says %.1f m, and the verdict is ALLOW. That means this node "
            "predates PROV_VALUE (shipped 2026-08-09), so it checks that a "
            "citation exists and resolves but not that the number beside it "
            "matches the number inside it." % elev)
    else:
        conclusions.append(
            "D behaved differently from either documented expectation: action=%s, "
            "code=%s." % (d["action"], d.get("code")))

    for i, line in enumerate(conclusions, 1):
        print()
        print(wrap("%d. %s" % (i, line), indent="  "))

    print()
    rule()
    print(wrap("The test that discriminates B from C is NOT the action. It is:",
               indent="  "))
    print()
    print("      len(receipt.fact_cids) == citations_found")
    print()
    for r in results:
        ok = r["fact_cids"] == (r["cites"] or 0)
        print("      %s : %d == %s -> %-5s %s"
              % (r["key"], r["fact_cids"], r["cites"], ok,
                 "(vacuous, nothing cited)" if (r["cites"] or 0) == 0 else ""))
    print()
    if d["action"] == "allow" and d["fact_cids"] == (d["cites"] or 0):
        print(wrap("And that test still passes D, which is false: the arithmetic "
                   "balances because the citation is real. A resolvable citation "
                   "proves the fact exists and was signed; it does not prove the "
                   "sentence reports it correctly. On a responder carrying "
                   "PROV_VALUE the guard makes that comparison itself.",
                   indent="  "))
    elif d["action"] == "deny" and d["code"] == "PROV_VALUE":
        print(wrap("Note that the arithmetic alone would have passed D: "
                   "fact_cids == citations_found, because the citation is real. "
                   "What catches it is PROV_VALUE reading the value inside the "
                   "fact. Receipt arithmetic answers \"is this citation real\"; "
                   "it never answered \"does the sentence agree with it\".",
                   indent="  "))
    print()
    # Same-cell check: does the guard care that the prose names a different place?
    if c["action"] == "allow":
        print(wrap("One more thing this run measured without being asked to: the "
                   "prose in every check says \"at Jagraon\", while the cited fact "
                   "belongs to cell64 %s, which is the coordinate this script "
                   "located. The guard allowed anyway. A resolvable token is not "
                   "evidence that it is a token about the place named in the "
                   "sentence." % cell, indent="  "))
    print()

    matched = 0
    for r, ok in ((a, a["action"] == "deny" and a["fact_cids"] == 0),
                  (b, b["action"] == "allow" and bool(b["cites"]) and b["fact_cids"] == 0),
                  (c, c["action"] == "allow" and c["fact_cids"] > 0),
                  (d, d["action"] == "allow")):
        matched += 1 if ok else 0
    print("  %d of 4 checks matched the documented expectation." % matched)
    if matched != 4:
        print("  The mismatches are printed above as observed, not as expected.")
    print()


# -------------------------------------------------------------------- main ---


def main():
    p = argparse.ArgumentParser(
        description="Verifiable lending due-diligence against emem. No key, no account.")
    p.add_argument("--lat", type=float, default=DEFAULT_LAT,
                   help="parcel latitude (default %(default)s)")
    p.add_argument("--lng", type=float, default=DEFAULT_LNG,
                   help="parcel longitude (default %(default)s)")
    p.add_argument("--base", default=DEFAULT_BASE,
                   help="responder base URL (default %(default)s). Point this at "
                        "your own node; see GET /v1/guard/selfhost.")
    p.add_argument("--timeout", type=float, default=60.0,
                   help="per-request timeout in seconds (default %(default)s)")
    p.add_argument("--probe", action="store_true",
                   help="run the four controlled checks on what a verdict proves")
    args = p.parse_args()
    args.base = args.base.rstrip("/")

    if args.probe:
        probe(args)
    else:
        scenario(args)


if __name__ == "__main__":
    main()
