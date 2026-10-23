"""Sandbox runner: one cached venv per version spec, one temp dir per run.

Network is needed only the first time a venv is created (pip install). After
that `run_candidate` never touches the network.
"""
from __future__ import annotations

import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
import time
import xml.etree.ElementTree as ET
from pathlib import Path

HERE = Path(__file__).resolve().parent
DEFAULT_CACHE = Path(os.environ.get("EMEM_BENCH_CACHE", HERE / ".venv_cache"))
PYTEST_SPEC = "pytest>=8,<10"


def _env_dir(spec: str, cache: Path) -> Path:
    tag = re.sub(r"[^A-Za-z0-9]+", "_", spec).strip("_")
    return cache / f"{tag}-{hashlib.sha256(spec.encode()).hexdigest()[:8]}"


def ensure_env(spec: str, cache: Path | None = None) -> dict:
    """Create (once) the venv for `spec`; return {python, freeze, fingerprint}."""
    cache = Path(cache or DEFAULT_CACHE)
    d = _env_dir(spec, cache)
    py = d / "bin" / "python"
    freeze_file = d / "freeze.txt"
    if not (py.exists() and freeze_file.exists()):
        d.mkdir(parents=True, exist_ok=True)
        subprocess.run([sys.executable, "-m", "venv", str(d)], check=True)
        subprocess.run(
            [str(py), "-m", "pip", "install", "--disable-pip-version-check", "-q", spec, PYTEST_SPEC],
            check=True,
        )
        out = subprocess.run([str(py), "-m", "pip", "freeze"], check=True, capture_output=True, text=True).stdout
        freeze_file.write_text(out)
    freeze = freeze_file.read_text()
    norm = "\n".join(sorted(l.strip() for l in freeze.splitlines() if l.strip()))
    return {
        "python": str(py),
        "freeze": freeze.strip().splitlines(),
        "fingerprint": hashlib.sha256(norm.encode()).hexdigest(),
    }


def run_candidate(task_dir: Path | str, source: str, spec: str, timeout: float = 60.0,
                  cache: Path | None = None) -> dict:
    """Run the hidden tests of `task_dir` against `source` (saved as solution.py) under `spec`."""
    task_dir = Path(task_dir)
    env = ensure_env(spec, cache)
    result = {"passed": 0, "failed": 0, "errors": 0, "duration_s": 0.0,
              "env_fingerprint": env["fingerprint"], "freeze": env["freeze"],
              "version_spec": spec, "timed_out": False}
    with tempfile.TemporaryDirectory(prefix="emem-bench-") as tmp:
        tmp = Path(tmp)
        (tmp / "solution.py").write_text(source)
        for t in sorted((task_dir / "tests_hidden").glob("test_*.py")):
            (tmp / t.name).write_text(t.read_text())
        junit = tmp / "junit.xml"
        cmd = [env["python"], "-m", "pytest", "-q", "-p", "no:cacheprovider", "--tb=no",
               f"--junitxml={junit}", "--rootdir", str(tmp), str(tmp)]
        child_env = {k: v for k, v in os.environ.items() if k not in ("PYTHONPATH", "VIRTUAL_ENV")}
        child_env["PYTHONDONTWRITEBYTECODE"] = "1"
        t0 = time.monotonic()
        try:
            subprocess.run(cmd, cwd=tmp, env=child_env, capture_output=True, text=True, timeout=timeout)
        except subprocess.TimeoutExpired:
            result["timed_out"] = True
            result["errors"] = 1
        result["duration_s"] = round(time.monotonic() - t0, 3)
        if junit.exists():
            suite = ET.parse(junit).getroot()
            suite = suite if suite.tag == "testsuite" else suite.find("testsuite")
            tests, fail = int(suite.get("tests", 0)), int(suite.get("failures", 0))
            err, skip = int(suite.get("errors", 0)), int(suite.get("skipped", 0))
            result.update(failed=fail, errors=err, passed=tests - fail - err - skip)
        elif not result["timed_out"]:
            result["errors"] = 1
    result["ok"] = result["passed"] > 0 and result["failed"] == 0 and result["errors"] == 0
    return result


if __name__ == "__main__":
    if len(sys.argv) != 4:
        sys.exit("usage: runner.py TASK_DIR SOLUTION.py VERSION_SPEC")
    r = run_candidate(sys.argv[1], Path(sys.argv[2]).read_text(), sys.argv[3])
    r.pop("freeze")
    print(json.dumps(r, indent=2))
    sys.exit(0 if r["ok"] else 1)
