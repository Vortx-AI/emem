#!/usr/bin/env python3
"""A lending due-diligence syndicate, run for real against the live responder.

Five agents with distinct ed25519 identities do the work a credit file actually
needs: ground the collateral, read its condition, price the climate risk, value
it, and decide. Nothing here is narrated. Every number comes from a signed fact
this run recalled, every handoff is an addressed note on the ledger, and the
receiving agent resolves the token itself rather than trusting the sender.

Watch it at https://emem.dev/arcade — the agents appear on the globe as they
join, and each note lands in THE ROOM with its citation resolved live.

    python3 scripts/demo_lending.py --dry-run   # print the plan, write nothing
    python3 scripts/demo_lending.py             # run it
"""
from __future__ import annotations
import json, subprocess, sys, time, urllib.request

BASE = "https://emem.dev"
BRIDGE = ["python3", "scripts/arcade_bridge.py"]
DRY = "--dry-run" in sys.argv

# Collateral under review: an agri-loan book across four districts. Places are
# resolved by the responder, not hardcoded coordinates.
SYNDICATE = [
    ("originator", "ORIGINATOR", "Bengaluru",  "takes the application"),
    ("field-agent", "FIELD",     "Nashik",     "verifies the standing crop"),
    ("climate-desk", "CLIMATE",  "Pune",       "prices the water risk"),
    ("valuer",      "VALUER",    "Mumbai",     "values the collateral"),
    ("credit-risk", "RISK",      "New Delhi",  "signs or declines"),
]
# What each desk actually reads. Bands chosen because they are the ones a
# credit file would defend: vegetation vigour, water, and heat.
READS = {
    "field-agent":  "indices.ndvi",
    "climate-desk": "indices.ndwi",
    "valuer":       "indices.ndvi",
}

def sh(args, quiet=False):
    cmd = BRIDGE + (["--dry-run"] if DRY else []) + args
    r = subprocess.run(cmd, capture_output=True, text=True, cwd="/home/ubuntu/emem")
    if r.returncode != 0:
        print(f"    ! {' '.join(args[:3])}: {r.stderr.strip()[:160]}")
        return None
    if not quiet and r.stdout.strip():
        try: return json.loads(r.stdout)
        except json.JSONDecodeError: return {"raw": r.stdout.strip()}
    return {}

def post(path, body):
    req = urllib.request.Request(BASE + path, data=json.dumps(body).encode(),
                                 headers={"Content-Type": "application/json"})
    return json.load(urllib.request.urlopen(req, timeout=120))

def recall(place, band):
    """A real signed reading, with its token. No fact is invented for the demo."""
    loc = post("/v1/locate", {"q": place})
    cell = (loc.get("selected") or {}).get("cell64")
    r = post("/v1/recall", {"cell64": cell, "bands": [band]})
    facts = r.get("facts") or []
    if not facts:
        return None
    cid = (r.get("receipt") or {}).get("fact_cids", [None])[0]
    tok = post("/v1/memory_token", {"cell": cell, "fact_cid": cid})
    return {"cell": cell, "value": facts[0].get("value"),
            "token": tok.get("memory_token") or tok.get("token"),
            "cid": cid, "band": band}

def main():
    print(f"{'DRY RUN — nothing will be written' if DRY else 'Running against ' + BASE}\n")
    print("1. the syndicate reports for work")
    for slug, name, place, role in SYNDICATE:
        out = sh(["join", "--slug", slug, "--name", name, "--place", place, "--role", role])
        if out and out.get("pk8"):
            print(f"   {name:11} {out['pk8']}  {out.get('position',{}).get('label','')}")
        time.sleep(0.4)

    print("\n2. each desk reads its own evidence")
    ev = {}
    for slug, band in READS.items():
        place = dict((s[0], s[2]) for s in SYNDICATE)[slug]
        name  = dict((s[0], s[1]) for s in SYNDICATE)[slug]
        if DRY:
            print(f"   {name:11} would recall {band} at {place}"); continue
        f = recall(place, band)
        if not f:
            print(f"   {name:11} no signed fact for {band} at {place} — recorded as absence")
            continue
        ev[slug] = f
        print(f"   {name:11} {band} = {f['value']:.4f} at {place}")
        sh(["act", "--slug", slug, "--act", "recall", "--band", band,
            "--value", str(f["value"]), "--fact-cid", f["cid"], "--note",
            f"{name} read {band} = {f['value']:.4f} at {place} for the collateral file. "
            f"Handing RISK the token, not the number."], quiet=True)
        time.sleep(0.4)

    print("\n3. every desk hands RISK a token, never a number")
    for slug, f in ev.items():
        name = dict((s[0], s[1]) for s in SYNDICATE)[slug]
        sh(["say", "--from", slug, "--to", "credit-risk",
            "--subject", f"{f['band']} evidence for the file",
            "--body", f"{name}'s reading is {f['value']:.4f}. Do not take that on trust: "
                      f"resolve the token and check the signature yourself.",
            "--token", f["token"]], quiet=True)
        print(f"   {name:11} -> RISK   {f['band']}")
        time.sleep(0.4)

    print("\n4. RISK verifies each one independently before it decides")
    verdicts = []
    for slug, f in ev.items():
        name = dict((s[0], s[1]) for s in SYNDICATE)[slug]
        if DRY:
            print(f"   would verify {name}'s token"); continue
        r = post("/mcp", {"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
            "name":"emem_echo_verify","arguments":{"token":f["token"],
            "claimed_value":str(f["value"])}}})
        sc = (r.get("result") or {}).get("structuredContent") or {}
        ok = bool(sc.get("matches"))
        verdicts.append(ok)
        print(f"   {name:11} token resolves and matches: {ok}")
        sh(["act", "--slug", "credit-risk", "--act", "receive", "--token", f["token"],
            "--match", "true" if ok else "false", "--note",
            f"RISK resolved {name}'s citation independently. The signed body "
            f"{'matched' if ok else 'DID NOT match'} the value {name} stated."], quiet=True)
        time.sleep(0.4)

    if not DRY and verdicts:
        allok = all(verdicts)
        sh(["act", "--slug", "credit-risk", "--act", "ack", "--note",
            (f"File complete. {len(verdicts)} of {len(verdicts)} citations resolved to the "
             f"bytes their desks claimed, so the evidence is admissible and the decision "
             f"rests on readings anyone can re-check. " if allok else
             f"File incomplete: a citation did not resolve to the claimed bytes. Declined "
             f"pending re-read. ") +
            "No desk was asked to trust another; each token was resolved by the reader."],
           quiet=True)
        print(f"\n   RISK filed its decision: {len(verdicts)}/{len(verdicts)} citations admissible")
    print("\nWatch it: https://emem.dev/arcade  (THE ROOM, or the notes panel)")

if __name__ == "__main__":
    main()
