#!/usr/bin/env python3
"""Record a REAL cross-model relay for the homepage decay demo.

One signed fact leaves emem, passes Gemma -> Qwen -> Gemma as a summary
(the way a long multi-agent session compacts it), and separately as a
token. Every frame below is what the models actually wrote; nothing is
authored by hand. Output: web/data/relay-recording.json
"""
import json, subprocess, urllib.request, time
from pathlib import Path

LLM = "http://127.0.0.1:5014/v1/chat/completions"
# Eight hops, alternating families, with the budget tightening the way a
# long session's compaction budget does. Three hops lost only the tail
# digits; the interesting failure is what a chain does over its length.
# Alternating families, budget tightening the way a long session's compaction
# budget does. The chain runs until the carried value falls below 0.1 (the
# point where the number is no longer the measurement in any useful sense) or
# MAX_HOPS is reached. Whatever it does is what gets published: if it never
# falls, the recording says so.
# The second family is Cosmos, not Qwen.
#
# Qwen/Qwen2.5-7B-Instruct's weights were evicted from this box on 2026-08-17
# and its registry rows are archived, so this recording could no longer be
# reproduced: the alternation would die on the first cold load of hop two. The
# point of the relay is that the chain is re-runnable by anyone reading the
# published result, and a hop pointing at a model nobody can load is a
# recording nobody can check.
#
# Cosmos runs on its own service, so the two hops now cross a process boundary
# as well as a family one, which if anything makes the drift test stronger.
HOPS_PAIR = [
    ("google/gemma-4-12B-it", "gemma", "http://127.0.0.1:5014/v1/chat/completions"),
    ("nvidia/Cosmos3-Edge", "cosmos3_edge", "http://127.0.0.1:5017/v1/chat/completions"),
]
MAX_HOPS = 24
STOP_BELOW = 0.1

def call(base, fam, prompt, timeout=600, url=None):
    body = {"base_model": base, "family": fam,
            "messages": [{"role":"user","content":prompt}],
            "temperature": 0.0}
    # max_tokens only where it is a ceiling the model stops before. Cosmos
    # grows to meet any cap it is given, so one does not shorten its answer, it
    # removes it: 1600 produced 5,465 characters of deliberation and no
    # conclusion where unthrottled completed in 990 tokens.
    if fam == "gemma":
        body["max_tokens"] = 160
    out = subprocess.run(["curl","-s","-m",str(timeout),"-X","POST",url or LLM,
                          "-H","Content-Type: application/json","-d",json.dumps(body)],
                         capture_output=True, text=True).stdout
    try:
        return json.loads(out)["choices"][0]["message"]["content"].strip()
    except Exception as e:
        return f"ERROR: {out[:200]}"

# 1. the real fact, live from prod
req = urllib.request.Request("https://emem.dev/v1/recall",
        data=json.dumps({"place":"Manali, Himachal Pradesh","bands":["indices.ndvi"]}).encode(),
        headers={"content-type":"application/json"})
rec = json.load(urllib.request.urlopen(req, timeout=120))
fact = rec["facts"][0]
cell, cid, val = fact["cell"], fact["fact_cid"], fact["value"]
token = f"emem:fact:{cell}:{cid}"
print("fact:", val, token[:50])

import re as _re
def carried_value(t):
    m = _re.search(r"\d*\.\d+", t or "")
    return float(m.group(0)) if m else None

frames = []
carried = f'NDVI at cell {cell} is {val}'
stop_reason = "max_hops"
for i in range(MAX_HOPS):
    base, fam, hop_url = HOPS_PAIR[i % 2]
    # Tighten the budget the way a compaction window tightens, then hold.
    budget = max(6, 24 - i * 2)
    prompt = (f"You are agent {i+1} in a long chain. Compress this handoff note for the next "
              f"agent in ONE sentence, under {budget} words. Write it naturally, the way you "
              f"would brief a colleague; do not add caveats.\n\n{carried}")
    t0 = time.time()
    out = call(base, fam, prompt, url=hop_url)
    ms = int((time.time() - t0) * 1000)
    cv = carried_value(out)
    frames.append({"hop": i + 1, "model": base, "family": fam, "text": out,
                   "ms": ms, "value": cv})
    print(f"hop{i+1} {fam} {ms}ms val={cv}: {out[:100]}")
    carried = out
    if cv is None:
        stop_reason = "value_dropped_from_the_summary"
        break
    if cv < STOP_BELOW:
        stop_reason = "value_below_0.1"
        break

# 2. the token arm: same chain, carrying only the handle
tok_carried = f"Reference: {token}"
tok_frames = []
for i in range(len(frames)):
    base, fam, hop_url = HOPS_PAIR[i % 2]
    prompt = (f"You are agent {i+1} in a long chain. Pass this reference to the next agent "
              f"EXACTLY as written, with one short sentence of context. Under 25 words.\n\n{tok_carried}")
    out = call(base, fam, prompt, url=hop_url)
    tok_frames.append({"hop": i+1, "model": base, "family": fam, "text": out,
                       "token_intact": token in out})
    print(f"tok hop{i+1} {fam} intact={token in out}: {out[:90]}")
    tok_carried = out

# 3. resolve the token back, live: the exact bytes return
req2 = urllib.request.Request("https://emem.dev/v1/memory_token/resolve",
        data=json.dumps({"token":token}).encode(), headers={"content-type":"application/json"})
res = json.load(urllib.request.urlopen(req2, timeout=120))
resolved = res.get("fact",{}).get("value")


# ── The set arm: where prose actually breaks ─────────────────────────────
# One number survives a relay in any format; it only rounds. A SET is the
# regime the measurements say prose cannot hold, so record that too: six
# signed facts carried as a written list, against the same six behind one
# emem:bundle: handle.
import urllib.request as _u

SET_BANDS = ["copdem30m.elevation_mean", "indices.ndvi", "weather.temperature_2m",
             "weather.precipitation_mm", "cams.pm25", "modis.lst_day_8day"]

def _post(path, body, timeout=180):
    req = _u.Request("https://emem.dev" + path, data=json.dumps(body).encode(),
                     headers={"content-type": "application/json"})
    return json.load(_u.urlopen(req, timeout=timeout))

set_facts, triples = [], []
rec_set = _post("/v1/recall", {"place": "Manali, Himachal Pradesh", "bands": SET_BANDS})
for f in rec_set.get("facts", []):
    set_facts.append({"band": f["band"], "value": f["value"], "fact_cid": f["fact_cid"]})
    triples.append({"cell": f["cell"], "band": f["band"], "tslot": f["tslot"]})
print("set facts:", len(set_facts))

bundle_token = ""
if triples:
    bt = _post("/v1/memory_bundle", {"triples": triples, "purpose": "homepage relay recording"})
    bundle_token = bt.get("bundle_token") or bt.get("token") or (bt.get("tokens") or {}).get("bundle") or ""
    print("bundle:", bundle_token[:60] if bundle_token else json.dumps(bt)[:200])

def fmt_set(facts):
    return "; ".join(f"{f['band']} = {f['value']}" for f in facts)

set_prose, set_tok = [], []
carried_set = fmt_set(set_facts)
for i in range(len(frames)):
    base, fam, hop_url = HOPS_PAIR[i % 2]
    budget = max(18, 60 - i * 4)
    out = call(base, fam,
               f"You are agent {i+1} in a long chain. Compress this handoff note for the "
               f"next agent in under {budget} words. Keep it natural.\n\n{carried_set}",
               url=hop_url)
    kept = sum(1 for f in set_facts if str(f["value"])[:6] in out)
    set_prose.append({"hop": i + 1, "family": fam, "text": out,
                      "values_kept": kept, "of": len(set_facts)})
    print(f"set hop{i+1} {fam}: kept {kept}/{len(set_facts)}")
    carried_set = out

carried_tok = f"Reference: {bundle_token}"
for i in range(len(frames)):
    base, fam, hop_url = HOPS_PAIR[i % 2]
    out = call(base, fam,
               f"You are agent {i+1} in a long chain. Pass this reference to the next agent "
               f"EXACTLY as written, with one short sentence of context. Under 25 words.\n\n{carried_tok}")
    set_tok.append({"hop": i + 1, "family": fam, "token_intact": bundle_token in out})
    carried_tok = out

resolved_set = []
if bundle_token:
    try:
        rb = _post("/v1/memory_token/resolve", {"token": bundle_token})
        resolved_set = [f.get("fact_cid") for f in (rb.get("facts") or [])]
    except Exception as e:
        print("bundle resolve:", e)
print("bundle resolves", len(resolved_set), "of", len(set_facts))

out = {
  "schema":"emem.relay.recording.v1",
  "recorded_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
  "place":"Manali, Himachal Pradesh", "band":"indices.ndvi",
  "cell": cell, "fact_cid": cid, "token": token,
  "exact_value": val,
  "prose_chain": frames,
  "stop_reason": stop_reason,
  "hops": len(frames),
  "token_chain": tok_frames,
  "resolved_value": resolved,
  "resolved_matches_exact": resolved == val,
  "set": {
    "bands": SET_BANDS,
    "facts": set_facts,
    "bundle_token": bundle_token,
    "prose_chain": set_prose,
    "token_chain": set_tok,
    "resolved_count": len(resolved_set),
  },
  "note":"Every frame is what the named model actually wrote at temperature 0. The prose chain is one signed fact carried as a summary through three hops across two model families; the token chain carries the handle instead. Re-run scripts/record_relay.py to regenerate."
}
Path("/home/ubuntu/emem/web/data").mkdir(parents=True, exist_ok=True)
Path("/home/ubuntu/emem/web/data/relay-recording.json").write_text(json.dumps(out, indent=1))
print("\nexact:", val, "| resolved:", resolved, "| match:", out["resolved_matches_exact"])
