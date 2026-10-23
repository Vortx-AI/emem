"""Repair stale experience: port each procedure that fails under the new release,
keeping its contract, and publish the repaired version only once its own tests pass.

The oracle is the procedure's OWN tests (the discovery task's), which exercise its
behaviour through its interface and call no v1-only pydantic API (checked
2026-10-23). The post-change evaluation tasks are never read here, so a repaired
procedure carries no knowledge of the tasks it will later be scored on.

Usage: repair.py [--rounds 5]
"""
from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import runner  # noqa: E402
import taskset  # noqa: E402
from adapters import ShimAdapter  # noqa: E402
from arms import extract_code  # noqa: E402
from emem_arm import procedure_note  # noqa: E402
from pilot import CACHE, PRICE_IN, PRICE_OUT, librarian, resolved_versions  # noqa: E402

SYSTEM = ("You port Python code to a new library release without changing what it does. Reply with the "
          "whole new solution.py in one ```python block.")


def failures(task, source, spec) -> str:
    """Run the procedure's own tests and return pytest's short failure report."""
    import subprocess
    import tempfile
    env = runner.ensure_env(spec, CACHE)
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        (tmp / "solution.py").write_text(source)
        for t in sorted((task.dir / "tests_hidden").glob("test_*.py")):
            (tmp / t.name).write_text(t.read_text())
        p = subprocess.run([env["python"], "-m", "pytest", "-q", "-p", "no:cacheprovider", "--tb=short",
                            str(tmp)], cwd=tmp, capture_output=True, text=True, timeout=120)
        return (p.stdout + p.stderr)[-3500:]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--rounds", type=int, default=5)
    args = ap.parse_args()
    lib = librarian()
    versions = resolved_versions(lib)
    old_spec, new_spec = f"pydantic=={versions['pydantic==1.10.*']}", f"pydantic=={versions['pydantic==2.*']}"
    adapter = ShimAdapter()
    tasks = [t for t in taskset.load_tasks() if t.phase == "discovery"]
    report = []
    for t in tasks:
        if runner.run_candidate(t.dir, t.reference, new_spec, cache=CACHE)["ok"]:
            continue  # still holds under the new release: keep it as it is
        tests = "\n\n".join(p.read_text() for p in sorted((t.dir / "tests_hidden").glob("test_*.py")))
        source, cost, ok = t.reference, 0.0, False
        messages = [{"role": "user", "content":
                     f"This procedure was written for pydantic 1.10 and its tests now fail under "
                     f"{new_spec}. Port it so the same tests pass under {new_spec}.\n\nWhat it solves:\n"
                     f"{t.spec_text}\n\nThe procedure:\n```python\n{t.reference}```\n\nIts tests:\n"
                     f"```python\n{tests[:4000]}```\n\nThe failures under {new_spec}:\n"
                     f"```\n{failures(t, t.reference, new_spec)}```"}]
        for rnd in range(1, args.rounds + 1):
            reply = adapter.complete(SYSTEM, messages, [])
            cost += reply.input_tokens * PRICE_IN + reply.output_tokens * PRICE_OUT
            source = extract_code(reply.text)
            if runner.run_candidate(t.dir, source, new_spec, cache=CACHE)["ok"]:
                ok = True
                break
            messages += [{"role": "assistant", "content": reply.text},
                         {"role": "user", "content": "Still failing:\n```\n" + failures(t, source, new_spec)
                                                     + "```\nFix it and reply with the whole file."}]
        rec = {"task": t.id, "repaired": ok, "rounds": rnd, "cost_usd": round(cost, 6)}
        if ok:
            # Publish the repair beside the original: same contract, verified under the new
            # release, still failing under the old one if the port relies on v2-only API.
            still_old = runner.run_candidate(t.dir, source, old_spec, cache=CACHE)["ok"]
            repaired = type("R", (), {})()
            for attr in ("id", "family", "library", "dir", "spec_text"):
                setattr(repaired, attr, getattr(t, attr))
            repaired.version_spec, repaired.reference = new_spec, source
            note = procedure_note(repaired, {new_spec: True, old_spec: still_old})
            note = note.replace("---\nkind: procedure\n",
                                f"---\nkind: procedure\nrepairs: /memories/by_attester/{lib.pk8}/procedures/"
                                f"{t.family}/{t.id}.md\n", 1)
            path = f"/memories/by_attester/{lib.pk8}/procedures/{t.family}/{t.id}-v2.md"
            lib.overwrite(path, note)
            rec["path"] = path
        report.append(rec)
        print(json.dumps(rec), flush=True)
    out = HERE / "runs" / f"repair-{time.strftime('%Y%m%dT%H%M%SZ', time.gmtime())}.json"
    out.write_text(json.dumps(report, indent=1))
    print(f"repaired {sum(r['repaired'] for r in report)} of {len(report)} stale procedures, "
          f"${sum(r['cost_usd'] for r in report):.5f}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
