"""Run one arm over the task stream (discovery, transfer, post_change).

Writes runs/<stamp>-<arm>[-dry].jsonl (one line per task) and a summary JSON
next to it. `--dry-run` uses each task's reference.py as the solution with zero
usage: it proves the pipeline, not any model.
"""
from __future__ import annotations

import argparse
import json
import sys
from collections import defaultdict
from datetime import datetime, timezone
from pathlib import Path

from arms import ARMS, DryRunArm, PydocDocs
from cost import DEFAULT_PRICES, cost_usd, load_prices
from runner import ensure_env, run_candidate
from taskset import PHASES, load_tasks, stream_order

HERE = Path(__file__).resolve().parent


def summarise(records: list) -> dict:
    def agg(rs):
        n = len(rs)
        return {"tasks": n, "passed": sum(r["passed"] for r in rs),
                "success_rate": round(sum(r["passed"] for r in rs) / n, 4) if n else None,
                "total_cost_usd": round(sum(r["cost_usd"] for r in rs), 6),
                "input_tokens": sum(r["usage"]["input_tokens"] for r in rs),
                "output_tokens": sum(r["usage"]["output_tokens"] for r in rs)}
    arms = defaultdict(list)
    for r in records:
        arms[r["arm"]].append(r)
    out = {}
    for a, rs in arms.items():
        out[a] = {"overall": agg(rs),
                  "by_phase": {p: agg([r for r in rs if r["phase"] == p]) for p in PHASES}}
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--arm", required=True, choices=sorted(ARMS))
    ap.add_argument("--dry-run", action="store_true", help="use reference.py as the solution, zero usage")
    ap.add_argument("--prices", default=str(DEFAULT_PRICES))
    ap.add_argument("--out", default=str(HERE / "runs"))
    ap.add_argument("--timeout", type=float, default=60.0)
    ap.add_argument("--limit", type=int, default=0, help="only the first N tasks of the stream")
    args = ap.parse_args()

    tasks = stream_order(load_tasks())
    if args.limit:
        tasks = tasks[: args.limit]
    by_id = {t.id: t for t in tasks}
    prices = load_prices(args.prices)

    if args.dry_run:
        arm = DryRunArm(args.arm, lambda tid: by_id[tid].reference)
    else:
        arm = ARMS[args.arm]()  # unwired adapter: raises NotImplementedError on the first model call

    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    stem = f"{stamp}-{args.arm}{'-dry' if args.dry_run else ''}"
    records = []
    with open(out_dir / f"{stem}.jsonl", "w") as fh:
        for t in tasks:
            started = datetime.now(timezone.utc).isoformat()
            env = ensure_env(t.version_spec)
            context = {"task_id": t.id, "phase": t.phase, "docs": PydocDocs(env["python"])}
            source, usage = arm.solve(t.spec_text, context)
            res = run_candidate(t.dir, source, t.version_spec, args.timeout)
            arm.observe(t.id, res["ok"])
            rec = {"task_id": t.id, "phase": t.phase, "arm": args.arm,
                   "model": usage.model_id, "passed": res["ok"], "usage": usage.as_dict(),
                   "cost_usd": cost_usd(usage, prices), "env_fingerprint": res["env_fingerprint"],
                   "started_at": started, "dry_run": args.dry_run,
                   "tests": {k: res[k] for k in ("passed", "failed", "errors", "duration_s", "timed_out")},
                   "env_freeze": res["freeze"]}
            fh.write(json.dumps(rec) + "\n")
            records.append(rec)
            print(f"{t.id:<18}{t.phase:<13}{'PASS' if res['ok'] else 'FAIL'}  ${rec['cost_usd']:.4f}")
    summary = {"stamp": stamp, "dry_run": args.dry_run, "summary": summarise(records)}
    (out_dir / f"{stem}.summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary["summary"], indent=2))
    print(f"wrote {out_dir / (stem + '.jsonl')}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
