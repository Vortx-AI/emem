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
HOPS = [("google/gemma-4-12B-it","gemma"), ("Qwen/Qwen2.5-7B-Instruct","qwen"), ("google/gemma-4-12B-it","gemma")]

def call(base, fam, prompt, timeout=600):
    body = {"base_model": base, "family": fam,
            "messages": [{"role":"user","content":prompt}],
            "temperature": 0.0, "max_tokens": 160}
    out = subprocess.run(["curl","-s","-m",str(timeout),"-X","POST",LLM,
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

frames = []
carried = f'NDVI at cell {cell} is {val}'
for i,(base,fam) in enumerate(HOPS):
    prompt = (f"You are agent {i+1} in a chain. Summarize this handoff note in ONE short "
              f"sentence for the next agent, under 20 words. Do not add caveats.\n\n{carried}")
    t0=time.time()
    out = call(base, fam, prompt)
    ms = int((time.time()-t0)*1000)
    frames.append({"hop": i+1, "model": base, "family": fam, "text": out, "ms": ms})
    print(f"hop{i+1} {fam} {ms}ms: {out[:110]}")
    carried = out

# 2. the token arm: same chain, carrying only the handle
tok_carried = f"Reference: {token}"
tok_frames = []
for i,(base,fam) in enumerate(HOPS):
    prompt = (f"You are agent {i+1} in a chain. Pass this reference to the next agent "
              f"EXACTLY as written, with one short sentence of context. Under 25 words.\n\n{tok_carried}")
    out = call(base, fam, prompt)
    tok_frames.append({"hop": i+1, "model": base, "family": fam, "text": out,
                       "token_intact": token in out})
    print(f"tok hop{i+1} {fam} intact={token in out}: {out[:90]}")
    tok_carried = out

# 3. resolve the token back, live: the exact bytes return
req2 = urllib.request.Request("https://emem.dev/v1/memory_token/resolve",
        data=json.dumps({"token":token}).encode(), headers={"content-type":"application/json"})
res = json.load(urllib.request.urlopen(req2, timeout=120))
resolved = res.get("fact",{}).get("value")

out = {
  "schema":"emem.relay.recording.v1",
  "recorded_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
  "place":"Manali, Himachal Pradesh", "band":"indices.ndvi",
  "cell": cell, "fact_cid": cid, "token": token,
  "exact_value": val,
  "prose_chain": frames,
  "token_chain": tok_frames,
  "resolved_value": resolved,
  "resolved_matches_exact": resolved == val,
  "note":"Every frame is what the named model actually wrote at temperature 0. The prose chain is one signed fact carried as a summary through three hops across two model families; the token chain carries the handle instead. Re-run scripts/record_relay.py to regenerate."
}
Path("/home/ubuntu/emem/web/data").mkdir(parents=True, exist_ok=True)
Path("/home/ubuntu/emem/web/data/relay-recording.json").write_text(json.dumps(out, indent=1))
print("\nexact:", val, "| resolved:", resolved, "| match:", out["resolved_matches_exact"])
