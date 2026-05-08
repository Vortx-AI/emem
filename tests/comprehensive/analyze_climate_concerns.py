#!/usr/bin/env python3
"""Post-hoc analysis on the comprehensive eval results.

Reads results/summary.json and per-question q*.json bodies (if present)
to produce verified climate-concern reports:
  - climate_worry_report.md  : what emem actually said about high-severity
                               climate questions, with cell64 + facts where available
  - high_volume_report.md    : the "very_high" + "high" volume questions and their
                               protocol-level outcome
  - regional_climate.md      : per-region climate vulnerability rollup
"""
from __future__ import annotations

import json
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent
RESULTS = ROOT / "results"
SUMMARY = RESULTS / "summary.json"


def load_results() -> list[dict]:
    if not SUMMARY.exists():
        return []
    return json.loads(SUMMARY.read_text())


def emit_climate_worry(results: list[dict]) -> None:
    high_sev = [r for r in results if r["severity"] >= 4]
    high_sev.sort(key=lambda r: (-r["severity"], r["domain"], r["id"]))
    lines = [
        "# Climate worry report — what emem says about the questions that matter most",
        "",
        "Filtered to severity >= 4. These are the questions where a wrong or absent answer "
        "has real human cost: home-purchase decisions, evacuation, food security, hajj heatstroke, "
        "low-lying island survival.",
        "",
        f"Sample size: **{len(high_sev)}** high-severity questions.",
        "",
        "Each row shows: the consumer question, the place emem resolved (cell64), the topics it "
        "routed to, how many signed facts came back, and the routing verdict. Cells that returned "
        "facts are *verifiable* — the responder signed the fact CIDs and any client can re-fetch "
        "the same bytes.",
        "",
        "| Sev | Domain | Question | Place resolved (cell64) | Topics matched | Facts | Pass |",
        "|---|---|---|---|---|---|---|",
    ]
    for r in high_sev:
        cell = r.get("cell64") or "—"
        place_label = r.get("place_label") or r["place"]
        topics = ", ".join(f"`{t}`" for t in (r.get("matched_topics") or [])[:3]) or "—"
        flag = "OK" if r["routing_pass"] else "FAIL"
        lines.append(
            f"| {r['severity']} | {r['domain']} | {r['q'][:55]} | "
            f"{place_label[:35]} `{cell}` | {topics} | {r['facts_n']} | {flag} |"
        )
    lines.append("")

    # Group failures by domain for the "fix list"
    fail_by_domain: dict[str, list[dict]] = defaultdict(list)
    for r in high_sev:
        if not r["routing_pass"]:
            fail_by_domain[r["domain"]].append(r)
    if fail_by_domain:
        lines += [
            "## High-severity routing misses (fix list)",
            "",
            "Each bullet is one consumer who got the wrong primitive routed for a high-stakes question.",
            "",
        ]
        for d, rs in sorted(fail_by_domain.items()):
            lines.append(f"### {d}")
            lines.append("")
            for r in rs:
                lines.append(
                    f"- **{r['q']}**  \n"
                    f"  place: `{r['place']}`, expected one of `{r['expect']}`, "
                    f"got `{r.get('matched_topics') or '[]'}`, out_of_scope={r.get('out_of_scope')}"
                )
            lines.append("")
    (RESULTS / "climate_worry_report.md").write_text("\n".join(lines))


def emit_high_volume(results: list[dict]) -> None:
    high_vol = [r for r in results if r["volume_signal"] in ("very_high", "high")]
    high_vol.sort(key=lambda r: (r["volume_signal"], -r["severity"]))
    lines = [
        "# High-volume questions — what emem says about what users actually ask",
        "",
        "Filtered to questions whose phrasing matches the high-volume patterns AI agents "
        "see most: 'should I buy here', 'is the air safe', 'will my home flood', 'is X a "
        "climate haven', 'how bad is the smoke today'. These are the AI-agent queries that "
        "drive real protocol traffic, not GIS-analyst queries.",
        "",
        f"Sample size: **{len(high_vol)}** very_high + high volume questions.",
        "",
        "| Vol | Sev | Question | Place | Top match | Facts | Materialize | Lat | Pass |",
        "|---|---|---|---|---|---|---|---|---|",
    ]
    for r in high_vol:
        top = r["matched_with_score"][0] if r.get("matched_with_score") else None
        top_s = f"`{top[0]}` ({top[1]})" if top else "—"
        flag = "OK" if r["routing_pass"] else "FAIL"
        lines.append(
            f"| {r['volume_signal']} | {r['severity']} | {r['q'][:50]} | "
            f"{r['place'][:25]} | {top_s} | {r['facts_n']} | "
            f"{r['materialize_attempts_n']} | {r['elapsed_s']:.1f}s | {flag} |"
        )
    lines.append("")
    n = len(high_vol)
    p = sum(1 for r in high_vol if r["routing_pass"])
    fact_yield = sum(1 for r in high_vol if r["facts_n"] > 0)
    lines += [
        "## Headline numbers",
        "",
        f"- High-volume routing accuracy: **{p}/{n} ({100*p//n if n else 0}%)**",
        f"- High-volume questions returning >=1 signed fact: **{fact_yield}/{n}**",
        f"- Average latency: **{sum(r['elapsed_s'] for r in high_vol)/max(n,1):.2f}s**",
    ]
    (RESULTS / "high_volume_report.md").write_text("\n".join(lines))


def emit_regional(results: list[dict]) -> None:
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

    lines = [
        "# Regional climate-question rollup",
        "",
        "How well does emem serve climate questions across regions?  Coverage equity matters: "
        "a protocol that answers Florida flood questions but stalls on Bangladesh delta questions "
        "is unfit for the global agent ecosystem.",
        "",
    ]
    for region, rs in sorted(by_region.items(), key=lambda x: -len(x[1])):
        n = len(rs)
        p = sum(1 for r in rs if r["routing_pass"])
        place_ok = sum(1 for r in rs if r["place_resolved"])
        any_facts = sum(1 for r in rs if r["facts_n"] > 0)
        avg_lat = sum(r["elapsed_s"] for r in rs) / n
        lines.append(f"## {region}  ({n} questions)")
        lines.append("")
        lines.append(f"- Routing accuracy: **{p}/{n} ({100*p//n}%)**")
        lines.append(f"- Geocoder resolved: {place_ok}/{n}")
        lines.append(f"- Returned >=1 signed fact: {any_facts}/{n}")
        lines.append(f"- Avg latency: {avg_lat:.2f}s")
        lines.append("")
        lines.append("| Sev | Domain | Question | Place | Pass |")
        lines.append("|---|---|---|---|---|")
        for r in sorted(rs, key=lambda r: (-r["severity"], r["domain"])):
            flag = "OK" if r["routing_pass"] else "FAIL"
            lines.append(f"| {r['severity']} | {r['domain']} | {r['q'][:50]} | {r['place'][:30]} | {flag} |")
        lines.append("")
    (RESULTS / "regional_climate.md").write_text("\n".join(lines))


def main() -> int:
    results = load_results()
    if not results:
        print("No results found at", SUMMARY)
        return 1
    emit_climate_worry(results)
    emit_high_volume(results)
    emit_regional(results)
    print("Wrote:")
    for f in ("climate_worry_report.md", "high_volume_report.md", "regional_climate.md"):
        print(" -", RESULTS / f)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
