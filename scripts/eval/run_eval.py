#!/usr/bin/env python3
"""Run a 51-question routing+materialize evaluation against the
running emem-server. Calls /v1/ask sequentially through the local
loopback (127.0.0.1:5051) to skip the public TLS layer and
keep-alive overhead. Records routed topics, materialize attempts,
fact yield, and end-to-end latency for each question."""

import json
import time
import urllib.request
import urllib.error
from pathlib import Path

ROOT = Path("/home/ubuntu/emem/scripts/eval")
QUESTIONS = json.loads((ROOT / "questions.json").read_text())
OUT_DIR = ROOT / "results"
OUT_DIR.mkdir(exist_ok=True)
SUMMARY = OUT_DIR / "summary.json"
REPORT = OUT_DIR / "report.md"

ENDPOINT = "http://127.0.0.1:5051/v1/ask"
TIMEOUT = 90

def call_ask(q, place):
    body = json.dumps({"q": q, "place": place}).encode()
    req = urllib.request.Request(
        ENDPOINT, data=body,
        headers={"content-type":"application/json"}, method="POST")
    t0 = time.perf_counter()
    try:
        with urllib.request.urlopen(req, timeout=TIMEOUT) as r:
            raw = r.read()
            elapsed = time.perf_counter() - t0
            return r.status, elapsed, json.loads(raw)
    except urllib.error.HTTPError as e:
        return e.code, time.perf_counter() - t0, {"_error": e.read().decode()[:500]}
    except Exception as e:
        return 0, time.perf_counter() - t0, {"_error": f"{type(e).__name__}: {e}"}

results = []
for q in QUESTIONS:
    print(f"[{q['id']:2d}/{len(QUESTIONS)}] {q['domain']:11} {q['q'][:55]:<55}", end=" ", flush=True)
    status, elapsed, body = call_ask(q['q'], q['place'])
    tr = body.get('topic_routing', {}) if isinstance(body, dict) else {}
    matched = tr.get('matched_topics', []) or []
    matched_with_score = [(m.get('topic'), round(m.get('score', 0), 3), m.get('via'))
                           for m in tr.get('matched_keywords', [])]
    routed_via = tr.get('routing', {}).get('method')
    facts_n = len(body.get('facts', []) or [])
    notes_n = len(body.get('materialize_notes', []) or [])
    place_resolved = bool(body.get('place_resolved'))
    expect = q.get('expect', [])
    overlap = [t for t in matched if t in expect]
    routing_pass = bool(overlap) or any(t in expect for t in
        [a.get('topic') for a in (body.get('algorithms_for_question') or [])])
    result = {
        "id": q['id'],
        "domain": q['domain'],
        "q": q['q'],
        "place": q['place'],
        "expect": expect,
        "status": status,
        "elapsed_s": round(elapsed, 2),
        "routed_via": routed_via,
        "matched_topics": matched,
        "matched_with_score": matched_with_score[:5],
        "routing_pass": routing_pass,
        "expected_overlap": overlap,
        "place_resolved": place_resolved,
        "facts_n": facts_n,
        "materialize_attempts_n": notes_n,
    }
    results.append(result)
    flag = "OK " if routing_pass else "FAIL"
    print(f"{status} {elapsed:5.1f}s {flag} facts={facts_n} via={routed_via} matched={matched[:3]}")
    # Save intermediate so we can resume on interrupt
    SUMMARY.write_text(json.dumps(results, indent=2))
    # Save full body per question for inspection
    (OUT_DIR / f"q{q['id']:02d}.json").write_text(json.dumps(body, indent=2))

# Final summary
total = len(results)
passed = sum(1 for r in results if r['routing_pass'])
http_ok = sum(1 for r in results if r['status'] == 200)
with_facts = sum(1 for r in results if r['facts_n'] > 0)
avg_t = sum(r['elapsed_s'] for r in results) / max(total, 1)
print()
print(f"=== EVAL SUMMARY ===")
print(f"questions: {total}  routed_correct: {passed}/{total} ({100*passed//total}%)")
print(f"http 200:  {http_ok}/{total}    with facts: {with_facts}/{total}")
print(f"avg latency: {avg_t:.2f}s")

# Markdown report
lines = ["# emem.dev routing + answer evaluation",
         "",
         f"- Questions: {total}",
         f"- Routing-correct (any expected topic in matched): **{passed}/{total} ({100*passed//total}%)**",
         f"- HTTP 200: {http_ok}/{total}",
         f"- Returned ≥1 fact: {with_facts}/{total}",
         f"- Average end-to-end latency: {avg_t:.2f}s",
         "",
         "## By domain",
         "",
         "| Domain | Pass | Total | Avg latency | Avg facts |",
         "|---|---|---|---|---|"]
by_domain = {}
for r in results:
    by_domain.setdefault(r['domain'], []).append(r)
for d, rs in sorted(by_domain.items()):
    p = sum(1 for r in rs if r['routing_pass'])
    t = len(rs)
    l = sum(r['elapsed_s'] for r in rs) / t
    f_avg = sum(r['facts_n'] for r in rs) / t
    lines.append(f"| {d} | {p} | {t} | {l:.1f}s | {f_avg:.1f} |")

lines += ["", "## Per-question results", "",
          "| ID | Domain | Question | Routed via | Top match | Pass? | Facts | Latency |",
          "|---|---|---|---|---|---|---|---|"]
for r in results:
    top = r['matched_with_score'][0] if r['matched_with_score'] else None
    top_s = f"`{top[0]}` ({top[1]})" if top else "—"
    lines.append(f"| {r['id']} | {r['domain']} | {r['q'][:50]} | {r['routed_via']} | {top_s} | {'✓' if r['routing_pass'] else '✗'} | {r['facts_n']} | {r['elapsed_s']:.1f}s |")

REPORT.write_text("\n".join(lines))
print(f"report: {REPORT}")
