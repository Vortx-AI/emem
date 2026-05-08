#!/usr/bin/env python3
"""Comprehensive consumer-focused test suite for emem /v1/ask.

Differs from scripts/eval/run_eval.py in three ways:
  1. New questions, new locations (intentionally disjoint).
  2. Per-question metadata (intent, severity, volume_signal) so we can
     compute volume-weighted accuracy and severity-weighted miss cost.
  3. Multiple report artefacts: summary.json, report.md, by_domain.md,
     by_volume.md, by_region.md, coverage_gaps.md, latency.csv.

Defaults to https://emem.dev. Override with --endpoint.
Designed to be safe to run repeatedly: intermediate results are
streamed to disk and the script can resume on KeyboardInterrupt.
"""
from __future__ import annotations

import argparse
import json
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path
from collections import defaultdict
from datetime import datetime, timezone

ROOT = Path(__file__).resolve().parent
QUESTIONS_FILE = ROOT / "questions_v2.json"
RESULTS_DIR = ROOT / "results"
RESULTS_DIR.mkdir(parents=True, exist_ok=True)

DEFAULT_ENDPOINT_BASE = "https://emem.dev"


def post_json(url: str, body: dict, timeout: int, retries: int = 3) -> tuple[int, float, dict | str]:
    data = json.dumps(body).encode()
    backoff = 4.0
    last: tuple[int, float, dict | str] = (0, 0.0, {"_error": "no attempt"})
    for attempt in range(retries + 1):
        req = urllib.request.Request(
            url,
            data=data,
            headers={"content-type": "application/json", "user-agent": "emem-comprehensive-eval/1.0"},
            method="POST",
        )
        t0 = time.perf_counter()
        try:
            with urllib.request.urlopen(req, timeout=timeout) as r:
                raw = r.read()
                elapsed = time.perf_counter() - t0
                try:
                    return r.status, elapsed, json.loads(raw)
                except json.JSONDecodeError:
                    return r.status, elapsed, raw.decode("utf-8", "replace")[:1000]
        except urllib.error.HTTPError as e:
            elapsed = time.perf_counter() - t0
            last = (e.code, elapsed, {"_error_body": e.read().decode("utf-8", "replace")[:500]})
            if e.code in (429, 502, 503, 504) and attempt < retries:
                time.sleep(backoff)
                backoff *= 2
                continue
            return last
        except urllib.error.URLError as e:
            last = (0, time.perf_counter() - t0, {"_error": f"URLError: {e.reason}"})
            if attempt < retries:
                time.sleep(backoff)
                backoff *= 2
                continue
            return last
        except Exception as e:
            return 0, time.perf_counter() - t0, {"_error": f"{type(e).__name__}: {e}"}
    return last


def safe_get(d, *path, default=None):
    cur = d
    for k in path:
        if not isinstance(cur, dict):
            return default
        cur = cur.get(k)
        if cur is None:
            return default
    return cur


def evaluate_one(q: dict, endpoint: str, timeout: int) -> dict:
    body = {"q": q["q"], "place": q["place"]}
    status, elapsed, resp = post_json(f"{endpoint}/v1/ask", body, timeout)

    if not isinstance(resp, dict):
        return {
            **{k: q[k] for k in ("id", "domain", "intent", "volume_signal", "severity", "q", "place")},
            "expect": q.get("expect", []),
            "status": status,
            "elapsed_s": round(elapsed, 2),
            "error": str(resp)[:500],
            "routing_pass": False,
            "place_resolved": False,
            "facts_n": 0,
            "materialize_attempts_n": 0,
            "matched_topics": [],
            "out_of_scope": None,
            "caveats_n": 0,
        }

    tr = resp.get("topic_routing") or {}
    matched = tr.get("matched_topics") or []
    matched_kw = tr.get("matched_keywords") or []
    matched_with_score = [
        (m.get("topic"), round(m.get("score") or 0, 3), m.get("via"))
        for m in matched_kw
    ][:5]
    routing_method = safe_get(tr, "routing", "method")
    out_of_scope = tr.get("out_of_scope")
    place_resolved = bool(resp.get("place_resolved"))
    cell64 = safe_get(resp, "place_resolved", "cell64")
    place_label = safe_get(resp, "place_resolved", "label")

    facts_block = resp.get("facts") or {}
    facts = facts_block.get("facts") if isinstance(facts_block, dict) else None
    facts_n = len(facts or [])
    bands_attested = facts_block.get("bands_already_attested_at_cell") if isinstance(facts_block, dict) else None
    bands_attested_n = len(bands_attested or []) if bands_attested else 0

    materialize_notes = resp.get("materialize_notes") or []
    notes_n = len(materialize_notes)
    materialize_ok = sum(1 for n in materialize_notes if isinstance(n, dict) and (n.get("ok") or n.get("status") == "ok"))

    expect = q.get("expect", []) or []
    overlap = [t for t in matched if t in expect]
    algos_for_q = resp.get("algorithms_for_question") or []
    algos_topics = [a.get("topic") for a in algos_for_q if isinstance(a, dict)]
    algo_overlap = [t for t in algos_topics if t in expect]
    routing_pass = bool(overlap) or bool(algo_overlap) or (not expect and out_of_scope is True)

    return {
        **{k: q[k] for k in ("id", "domain", "intent", "volume_signal", "severity", "q", "place")},
        "expect": expect,
        "status": status,
        "elapsed_s": round(elapsed, 2),
        "routed_via": routing_method,
        "matched_topics": matched,
        "matched_with_score": matched_with_score,
        "out_of_scope": out_of_scope,
        "expected_overlap": overlap,
        "algo_overlap": algo_overlap,
        "routing_pass": routing_pass,
        "place_resolved": place_resolved,
        "cell64": cell64,
        "place_label": place_label,
        "facts_n": facts_n,
        "bands_attested_n": bands_attested_n,
        "materialize_attempts_n": notes_n,
        "materialize_ok_n": materialize_ok,
        "caveats_n": len(resp.get("caveats") or []),
        "band_observations_n": resp.get("band_observations") or 0,
        "algorithm_outcomes_n": resp.get("algorithm_outcomes") or 0,
    }


VOLUME_WEIGHT = {"very_high": 4, "high": 3, "medium": 2, "low": 1}


def write_report(results: list[dict], endpoint: str, started: str, finished: str) -> None:
    total = len(results)
    if total == 0:
        return
    passed = sum(1 for r in results if r.get("routing_pass"))
    http_ok = sum(1 for r in results if r.get("status") == 200)
    with_facts = sum(1 for r in results if r.get("facts_n", 0) > 0)
    place_ok = sum(1 for r in results if r.get("place_resolved"))
    avg_t = sum(r.get("elapsed_s", 0) for r in results) / total
    p50 = sorted(r["elapsed_s"] for r in results)[total // 2]
    p95 = sorted(r["elapsed_s"] for r in results)[max(int(total * 0.95) - 1, 0)]

    # Volume-weighted accuracy
    vw_num = sum(VOLUME_WEIGHT.get(r["volume_signal"], 1) for r in results if r["routing_pass"])
    vw_den = sum(VOLUME_WEIGHT.get(r["volume_signal"], 1) for r in results)
    vw_acc = (100.0 * vw_num / vw_den) if vw_den else 0.0

    # Severity-weighted miss cost (sum severity of failures)
    miss_severity = sum(r["severity"] for r in results if not r["routing_pass"])

    lines: list[str] = []
    lines += [
        "# emem comprehensive consumer evaluation",
        "",
        f"- Endpoint: `{endpoint}`",
        f"- Started: {started}",
        f"- Finished: {finished}",
        f"- Questions: **{total}**",
        f"- Routing-correct: **{passed}/{total}** ({100*passed//total}%)",
        f"- Volume-weighted routing accuracy: **{vw_acc:.1f}%**  (weights very_high=4, high=3, medium=2, low=1)",
        f"- HTTP 200: {http_ok}/{total}",
        f"- Place resolved: {place_ok}/{total}",
        f"- Returned >=1 fact: {with_facts}/{total}",
        f"- Severity-weighted miss cost (sum of severity for failures, lower is better): **{miss_severity}**",
        f"- Latency: avg {avg_t:.2f}s, p50 {p50:.2f}s, p95 {p95:.2f}s",
        "",
        "## How to read this",
        "",
        "**Routing-correct** means the responder's `topic_routing.matched_topics` (or "
        "`algorithms_for_question.topic`) overlapped the expected topic set, **or** the "
        "question was out_of_scope and we expected nothing. This measures whether the LLM-facing "
        "router on `/v1/ask` is sending the question to the right Earth-observation primitives. "
        "It does **not** measure whether materialization succeeded or whether the answer is "
        "scientifically correct — it measures whether the protocol *understood the question*.",
        "",
        "**Place resolved** means the geocoder returned a `cell64`. **Facts** count signed "
        "facts attached to the response — usually attested-band reads from the local cache. "
        "**Materialize attempts** count cold-band fan-outs, which can be slow on first call.",
        "",
    ]

    # By domain
    by_domain: dict[str, list[dict]] = defaultdict(list)
    for r in results:
        by_domain[r["domain"]].append(r)
    lines += [
        "## By domain",
        "",
        "| Domain | Pass | Total | % | Avg latency | Avg facts | Avg materialize |",
        "|---|---|---|---|---|---|---|",
    ]
    for d, rs in sorted(by_domain.items(), key=lambda x: -len(x[1])):
        n = len(rs)
        p = sum(1 for r in rs if r["routing_pass"])
        l = sum(r["elapsed_s"] for r in rs) / n
        f_avg = sum(r["facts_n"] for r in rs) / n
        m_avg = sum(r["materialize_attempts_n"] for r in rs) / n
        lines.append(f"| {d} | {p} | {n} | {100*p//n}% | {l:.1f}s | {f_avg:.1f} | {m_avg:.1f} |")
    lines.append("")

    # By volume signal
    by_vol: dict[str, list[dict]] = defaultdict(list)
    for r in results:
        by_vol[r["volume_signal"]].append(r)
    lines += [
        "## By volume signal (where users actually ask)",
        "",
        "Higher volume = the protocol must answer this *well* because it's what users ask AI agents most.",
        "",
        "| Volume | Pass | Total | % | Sample |",
        "|---|---|---|---|---|",
    ]
    for v in ("very_high", "high", "medium", "low"):
        rs = by_vol.get(v, [])
        if not rs:
            continue
        n = len(rs)
        p = sum(1 for r in rs if r["routing_pass"])
        sample = next((r["q"] for r in rs if not r["routing_pass"]), rs[0]["q"])
        lines.append(f"| {v} | {p} | {n} | {100*p//n}% | _{sample[:60]}_ |")
    lines.append("")

    # By region
    def region_of(place: str) -> str:
        p = place.lower()
        regions = {
            "South Asia": ["india", "pakistan", "bangladesh", "sri lanka", "nepal", "bhutan", "maldives", "delhi", "mumbai", "chennai", "bangalore", "kolkata", "noida", "gurgaon", "jaipur", "karachi", "lahore", "dhaka"],
            "East/SE Asia": ["china", "japan", "korea", "vietnam", "thailand", "indonesia", "philippines", "malaysia", "singapore", "taiwan", "mongolia", "hong kong", "hanoi"],
            "Middle East": ["uae", "dubai", "saudi", "iraq", "iran", "egypt", "libya", "sudan", "morocco", "qatar", "kuwait", "lebanon", "israel", "syria", "yemen", "mecca"],
            "Africa": ["nigeria", "kenya", "ethiopia", "somalia", "ghana", "drc", "congo", "senegal", "south africa", "tanzania", "uganda", "mali", "madagascar", "niger"],
            "Europe": ["spain", "italy", "france", "germany", "uk", "scotland", "ireland", "greece", "portugal", "netherlands", "switzerland", "austria", "poland", "russia", "ukraine", "iceland", "norway", "sweden", "finland", "denmark", "belgium", "czech", "hungary", "romania"],
            "North America": ["united states", "usa", "canada", "mexico", "florida", "california", "texas", "new york", "illinois", "ohio", "minnesota", "arizona", "louisiana", "north carolina", "washington", "iowa", "wyoming", "nevada", "utah", "manhattan", "los angeles", "chicago"],
            "South America": ["brazil", "argentina", "chile", "peru", "colombia", "bolivia", "paraguay", "uruguay", "ecuador", "venezuela"],
            "Oceania": ["australia", "new zealand", "fiji", "tuvalu", "kiribati", "papua", "samoa", "tonga"],
            "Polar": ["antarctica", "greenland", "arctic"],
        }
        for r, kws in regions.items():
            if any(k in p for k in kws):
                return r
        return "Other"

    by_region: dict[str, list[dict]] = defaultdict(list)
    for r in results:
        by_region[region_of(r["place"])].append(r)
    lines += [
        "## By region (geographic equity of coverage)",
        "",
        "| Region | Pass | Total | % | Avg latency | Place-resolve % |",
        "|---|---|---|---|---|---|",
    ]
    for region, rs in sorted(by_region.items(), key=lambda x: -len(x[1])):
        n = len(rs)
        p = sum(1 for r in rs if r["routing_pass"])
        pl = sum(1 for r in rs if r["place_resolved"])
        l = sum(r["elapsed_s"] for r in rs) / n
        lines.append(f"| {region} | {p} | {n} | {100*p//n}% | {l:.1f}s | {100*pl//n}% |")
    lines.append("")

    # Failures with severity
    fails = [r for r in results if not r["routing_pass"]]
    fails_sorted = sorted(fails, key=lambda r: (-r["severity"], -VOLUME_WEIGHT.get(r["volume_signal"], 0)))
    lines += [
        "## Top routing failures (sorted by severity x volume — these are the ones to fix first)",
        "",
        "| Sev | Volume | Domain | Question | Place | Matched (if any) | Out-of-scope? |",
        "|---|---|---|---|---|---|---|",
    ]
    for r in fails_sorted[:30]:
        topm = r["matched_topics"][0] if r["matched_topics"] else "—"
        lines.append(
            f"| {r['severity']} | {r['volume_signal']} | {r['domain']} | {r['q'][:55]} | {r['place'][:30]} | `{topm}` | {r['out_of_scope']} |"
        )
    lines.append("")

    # Per-question table
    lines += [
        "## All questions",
        "",
        "| ID | Vol | Sev | Domain | Question | Routed via | Top match | Pass | Facts | Lat |",
        "|---|---|---|---|---|---|---|---|---|---|",
    ]
    for r in results:
        top = r["matched_with_score"][0] if r.get("matched_with_score") else None
        top_s = f"`{top[0]}` ({top[1]})" if top else "—"
        flag = "OK" if r["routing_pass"] else "FAIL"
        lines.append(
            f"| {r['id']} | {r['volume_signal']} | {r['severity']} | {r['domain']} | {r['q'][:48]} | {r.get('routed_via','—')} | {top_s} | {flag} | {r['facts_n']} | {r['elapsed_s']:.1f}s |"
        )

    (RESULTS_DIR / "report.md").write_text("\n".join(lines))

    # Latency CSV
    csv_lines = ["id,domain,intent,volume_signal,severity,elapsed_s,status,facts_n,materialize_attempts_n,routing_pass"]
    for r in results:
        csv_lines.append(
            f"{r['id']},{r['domain']},{r['intent']},{r['volume_signal']},{r['severity']},"
            f"{r['elapsed_s']},{r['status']},{r['facts_n']},{r['materialize_attempts_n']},{int(r['routing_pass'])}"
        )
    (RESULTS_DIR / "latency.csv").write_text("\n".join(csv_lines))

    # Coverage gaps
    gap_lines = [
        "# Coverage gaps and follow-ups",
        "",
        "Domains where routing accuracy is below 70% — these are concrete next-step targets for adding aliases or topics.",
        "",
    ]
    for d, rs in sorted(by_domain.items()):
        n = len(rs)
        p = sum(1 for r in rs if r["routing_pass"])
        if n and (100 * p // n) < 70:
            gap_lines.append(f"## {d}  ({p}/{n})")
            gap_lines.append("")
            for r in rs:
                if not r["routing_pass"]:
                    gap_lines.append(
                        f"- _{r['q']}_  — place `{r['place']}` — matched `{r['matched_topics'] or '—'}` — expected `{r['expect']}`"
                    )
            gap_lines.append("")
    (RESULTS_DIR / "coverage_gaps.md").write_text("\n".join(gap_lines))


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--endpoint", default=DEFAULT_ENDPOINT_BASE, help="emem base URL")
    ap.add_argument("--timeout", type=int, default=120)
    ap.add_argument("--limit", type=int, default=0, help="0 = all")
    ap.add_argument("--filter-domain", default="", help="comma-separated domains to run only")
    ap.add_argument("--start-id", type=int, default=0)
    ap.add_argument("--pace-s", type=float, default=2.0, help="sleep between requests")
    args = ap.parse_args()

    qs = json.loads(QUESTIONS_FILE.read_text())["questions"]
    if args.filter_domain:
        wanted = {d.strip() for d in args.filter_domain.split(",") if d.strip()}
        qs = [q for q in qs if q["domain"] in wanted]
    if args.start_id:
        qs = [q for q in qs if q["id"] >= args.start_id]
    if args.limit > 0:
        qs = qs[: args.limit]

    started = datetime.now(timezone.utc).isoformat(timespec="seconds")
    print(f"Endpoint: {args.endpoint}")
    print(f"Running {len(qs)} questions, timeout={args.timeout}s, started {started}")
    print("-" * 80)

    summary_path = RESULTS_DIR / "summary.json"
    results: list[dict] = []
    if summary_path.exists():
        try:
            results = json.loads(summary_path.read_text())
            done_ids = {r["id"] for r in results if r.get("status") == 200}
            qs = [q for q in qs if q["id"] not in done_ids]
            results = [r for r in results if r["id"] in done_ids]
            if results:
                print(f"Resuming: {len(results)} previously-passed questions found, {len(qs)} remaining (failures will be retried).")
        except Exception:
            results = []

    try:
        for i, q in enumerate(qs, 1):
            print(f"[{i:3d}/{len(qs)}] id={q['id']:3d} {q['domain']:32} | {q['q'][:60]}", flush=True)
            r = evaluate_one(q, args.endpoint, args.timeout)
            flag = "OK  " if r["routing_pass"] else "FAIL"
            print(
                f"          {flag} status={r.get('status'):3} "
                f"lat={r['elapsed_s']:5.1f}s facts={r['facts_n']:2d} "
                f"matched={r['matched_topics'][:2]}"
            )
            results.append(r)
            summary_path.write_text(json.dumps(results, indent=2))
            (RESULTS_DIR / f"q{r['id']:03d}.json").write_text(json.dumps(r, indent=2))
            time.sleep(args.pace_s)
    except KeyboardInterrupt:
        print("\nInterrupted — partial results saved.")

    finished = datetime.now(timezone.utc).isoformat(timespec="seconds")
    write_report(results, args.endpoint, started, finished)

    total = len(results)
    if total:
        passed = sum(1 for r in results if r["routing_pass"])
        avg_t = sum(r["elapsed_s"] for r in results) / total
        print()
        print("=== EVAL SUMMARY ===")
        print(f"questions:    {total}")
        print(f"routed pass:  {passed}/{total} ({100*passed//total}%)")
        print(f"avg latency:  {avg_t:.2f}s")
        print(f"reports:      {RESULTS_DIR}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
