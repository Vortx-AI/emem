"""Pilot: does verified, shared experience let a DIFFERENT model solve unseen
and changed tasks for less?

Discovery: the 16 discovery tasks' reference procedures, with their own tests,
are published to emem as signed notes, and a verifier key records whether
their tests pass under each pydantic release (stale where they fail).
Evaluation: the 16 transfer tasks (pydantic 1.10) and the 8 post-change tasks
(pydantic 2), none of which is in the store, solved by the receiving model in
two arms:

  baseline_fresh   the task, a docs tool, nothing carried over
  emem_procedures  the same, plus the procedures emem retrieves by the task
                   text alone, each marked verified or stale under the
                   task's release

Each arm gets up to K attempts at the same temperature; a task counts as solved
at the first attempt whose solution passes the task's hidden tests, run in a
clean venv by this harness. Tokens and dollars are summed over every attempt.

Usage: pilot.py [--attempts 3] [--limit N] [--skip-publish]
"""
from __future__ import annotations

import argparse
import json
import os
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, "/home/ubuntu/emem-world/agents")

import runner  # noqa: E402
import taskset  # noqa: E402
from adapters import ShimAdapter  # noqa: E402
from arms import BaselineFresh, PydocDocs  # noqa: E402
from emem_arm import EmemProcedures, EmemStore, publish  # noqa: E402

PRICE_IN, PRICE_OUT = 0.09 / 1e6, 0.29 / 1e6  # Gemma 3 12B on Bedrock, the shim's own rates
CACHE = Path(os.environ.get("EMEM_BENCH_CACHE", "/home/ubuntu/emem-world/agents/state/bench-venvs"))


def cost(u) -> float:
    return u.input_tokens * PRICE_IN + u.output_tokens * PRICE_OUT


def librarian():
    import ememagentd as m
    cfg = json.load(open("/home/ubuntu/emem-world/agents/roster.json"))
    a_cfg = next(a for a in cfg["agents"] if a["slug"] == "librarian")
    agent = m.Agent(a_cfg, cfg["desks"]["librarian"])
    # The running daemon owns this state file; a save from here could overwrite
    # its newer state, so the pilot signs with the key and never saves.
    agent.save = lambda: None
    return agent


def resolved_versions(agent) -> dict:
    sw = agent.st.get("software") or {}
    if "1.10" in sw and "2" in sw:
        return {"pydantic==1.10.*": sw["1.10"]["version"], "pydantic==2.*": sw["2"]["version"]}
    import re
    import urllib.request
    rel = json.load(urllib.request.urlopen("https://pypi.org/pypi/pydantic/json", timeout=60))["releases"]
    stable = [v for v in rel if re.fullmatch(r"\d+\.\d+\.\d+", v) and rel[v]]
    key = lambda v: tuple(int(x) for x in v.split("."))
    latest = lambda pre: max((v for v in stable if v.startswith(pre)), key=key)
    return {"pydantic==1.10.*": latest("1.10."), "pydantic==2.*": latest("2.")}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--attempts", type=int, default=3)
    ap.add_argument("--limit", type=int, default=0)
    ap.add_argument("--skip-publish", action="store_true")
    ap.add_argument("--cards", action="store_true", help="compact verified cards, k=2, stale as a warning")
    ap.add_argument("--k", type=int, default=3)
    ap.add_argument("--phase", choices=["transfer", "post_change"], help="evaluate one phase only")
    args = ap.parse_args()

    tasks = taskset.load_tasks()
    discovery = [t for t in tasks if t.phase == "discovery"]
    evals = [t for t in taskset.stream_order(tasks) if t.phase in ("transfer", "post_change")]
    if args.phase:
        evals = [t for t in evals if t.phase == args.phase]
    if args.limit:
        evals = evals[: args.limit]
    lib = librarian()
    versions = resolved_versions(lib)
    exact = {spec: f"pydantic=={v}" for spec, v in versions.items()}
    prefix = f"/memories/by_attester/{lib.pk8}/procedures/"

    if not args.skip_publish:
        verified = {}
        for t in discovery:
            verified[t.id] = {rel: runner.run_candidate(t.dir, t.reference, rel, cache=CACHE)["ok"]
                              for rel in exact.values()}
        paths = publish(lib, discovery, verified)
        print(f"published {len(paths)} procedures under {prefix}", flush=True)
        stale = sum(1 for v in verified.values() for ok in v.values() if not ok)
        print(f"verification: {stale} procedure-release pairs stale", flush=True)
        store = EmemStore(prefix)
        for _ in range(40):  # the memory index re-reads every 60 s
            if store.retrieve(discovery[0].spec_text):
                break
            time.sleep(15)

    adapter = ShimAdapter()
    store = EmemStore(prefix, k=args.k)
    arms = [BaselineFresh(adapter), EmemProcedures(adapter, store, versions, compact=args.cards)]
    stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    out = HERE / "runs" / f"pilot-{stamp}{'-cards' if args.cards else ''}.jsonl"
    out.parent.mkdir(exist_ok=True)
    records = []
    for t in evals:
        rel = exact[t.version_spec]
        env = runner.ensure_env(rel, CACHE)
        for arm in arms:
            total, solved_at, attempts = None, None, []
            for k in range(1, args.attempts + 1):
                ctx = {"task_id": t.id, "phase": t.phase, "version_spec": t.version_spec,
                       "docs": PydocDocs(env["python"])}
                try:
                    source, usage = arm.solve(t.spec_text, ctx)
                    r = runner.run_candidate(t.dir, source, rel, cache=CACHE)
                    ok = r["ok"]
                except Exception as e:  # a model or network failure is a failed attempt, recorded
                    from arms import Usage
                    usage, ok = Usage(model_id=adapter.model_id), False
                    print(f"  {t.id} {arm.name} attempt {k} error: {e!r}"[:200], flush=True)
                attempts.append({"ok": ok, **usage.as_dict(), "cost_usd": cost(usage)})
                if ok:
                    solved_at = k
                    break
            rec = {"task_id": t.id, "family": t.family, "phase": t.phase, "arm": arm.name,
                   "model": adapter.model_id, "release": rel, "solved_at": solved_at,
                   "attempts": attempts,
                   "input_tokens": sum(a["input_tokens"] for a in attempts),
                   "output_tokens": sum(a["output_tokens"] for a in attempts),
                   "cost_usd": sum(a["cost_usd"] for a in attempts)}
            if arm.name.startswith("emem_") and arm.retrieved_log:
                rec["retrieved"] = arm.retrieved_log[-1]["paths"]
            records.append(rec)
            with open(out, "a") as f:
                f.write(json.dumps(rec) + "\n")
            print(f"{t.id:16} {t.phase:11} {arm.name:16} solved_at={solved_at} "
                  f"cost=${rec['cost_usd']:.5f}", flush=True)

    summary = {}
    for arm in arms:
        for phase in ("transfer", "post_change", "all"):
            rs = [r for r in records if r["arm"] == arm.name and (phase == "all" or r["phase"] == phase)]
            if not rs:
                continue
            n = len(rs)
            summary[f"{arm.name}/{phase}"] = {
                "tasks": n,
                "success_at_1": sum(1 for r in rs if r["solved_at"] == 1) / n,
                f"success_at_{args.attempts}": sum(1 for r in rs if r["solved_at"]) / n,
                "total_cost_usd": round(sum(r["cost_usd"] for r in rs), 6),
                "input_tokens": sum(r["input_tokens"] for r in rs),
                "output_tokens": sum(r["output_tokens"] for r in rs),
            }
    (out.with_suffix(".summary.json")).write_text(json.dumps(summary, indent=1))
    print(json.dumps(summary, indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main())
