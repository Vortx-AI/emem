"""Technique-level retrieval for procedures.

A procedure note is task prose plus code, and a task names its own domain
(Point, Counter, Cart), so embedding either one matches nouns, not the
technique: on 2026-10-24 the right family ranked first for about half the
eval tasks, with relevant and irrelevant hits interleaved between 0.70 and
0.88 similarity. Here both sides are abstracted to the technique:

- publish: one technique card per procedure, a few verb-first lines with no
  names from its task, signed beside the procedure and pointing at it;
- retrieve: the task rewritten to the techniques it needs, then searched
  against the cards only.

Both rewrites are model calls; the query rewrite is charged to the emem arm.

Usage: techniques.py publish | measure [--k 2]
"""
from __future__ import annotations

import argparse
import json
import re
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import taskset  # noqa: E402
from adapters import ShimAdapter  # noqa: E402
from emem_arm import _post  # noqa: E402
from pilot import librarian  # noqa: E402

CARD_SYSTEM = ("You name the library technique a piece of code demonstrates. Reply with 1 to 3 short "
               "lines, each starting with a verb, naming the capability in the library's own terms. "
               "Use no names from the task: no class, field, function or variable names, and no "
               "example values. Example: `make a model immutable so that assigning a field raises`.")
QUERY_SYSTEM = ("You list the library techniques a programming task needs. Reply with 1 to 3 short "
                "lines, each starting with a verb, naming the capability in the library's own terms. "
                "Use no names from the task: no class, field, function or variable names, and no "
                "example values.")


def _lines(text: str) -> str:
    out = [re.sub(r"^[\s\-*\d.)`]+|`$", "", ln).strip() for ln in text.splitlines()]
    return "\n".join(ln for ln in out if ln)[:400]


def card_path(pk8: str, proc_path: str) -> str:
    return proc_path.replace(f"/by_attester/{pk8}/procedures/", f"/by_attester/{pk8}/techniques/")


def proc_path(pk8: str, card: str) -> str:
    return card.replace(f"/by_attester/{pk8}/techniques/", f"/by_attester/{pk8}/procedures/")


class TechniqueRetriever:
    """Rewrites a task to its techniques and searches the technique cards."""

    def __init__(self, adapter, pk8: str, k: int = 2):
        self.adapter, self.pk8, self.k = adapter, pk8, k
        self.prefix = f"/memories/by_attester/{pk8}/techniques/"
        self.last_usage = None

    def rewrite(self, task_spec: str) -> str:
        reply = self.adapter.complete(QUERY_SYSTEM, [{"role": "user", "content": task_spec}], [])
        self.last_usage = reply
        return _lines(reply.text) or task_spec

    def procedure_paths(self, task_spec: str) -> list:
        q = self.rewrite(task_spec)
        R = _post("/v1/memory/search", {"q": q, "k": self.k * 2, "path_prefix": self.prefix})
        out = []
        for h in R.get("hits") or []:
            p = proc_path(self.pk8, h.get("path") or "")
            if "/procedures/" in p and p not in out:
                out.append(p)
        return out[: self.k]


def publish(adapter, lib) -> None:
    prefix = f"/memories/by_attester/{lib.pk8}/procedures/"
    R = _post("/v1/memory/search", {"q": "pydantic procedure", "k": 100, "path_prefix": prefix})
    paths = sorted({h["path"] for h in R.get("hits") or [] if h.get("path", "").endswith(".md")})
    from emem_arm import _mcp
    for p in paths:
        body = _mcp("memory_view", {"path": p}).get("content") or ""
        solves = re.search(r"## What it solves\n\n(.*?)\n\n## ", body, re.S)
        code = re.search(r"## Code\n\n```python\n(.*?)```", body, re.S)
        msg = (f"What it solved:\n{solves.group(1) if solves else ''}\n\nThe code:\n"
               f"```python\n{code.group(1) if code else ''}```")
        reply = adapter.complete(CARD_SYSTEM, [{"role": "user", "content": msg}], [])
        card = _lines(reply.text)
        lib.overwrite(card_path(lib.pk8, p), f"---\nkind: technique\nprocedure: {p}\n---\n{card}\n")
        print(json.dumps({"procedure": p.rsplit("/", 1)[-1], "technique": card}), flush=True)


def measure(adapter, lib, k: int) -> None:
    prefix = f"/memories/by_attester/{lib.pk8}/procedures/"
    tech = TechniqueRetriever(adapter, lib.pk8, k=k)
    score = {"plain": [0, 0], "technique": [0, 0]}
    n = 0
    for t in taskset.load_tasks():
        if t.phase == "discovery":
            continue
        rel = {r for r in json.load(open(t.dir / "meta.json")).get("discovery_of", [])}
        name = lambda p: p.rsplit("/", 1)[-1].replace(".md", "").replace("-v2", "")
        R = _post("/v1/memory/search", {"q": t.spec_text, "k": k, "path_prefix": prefix})
        plain = [name(h["path"]) for h in (R.get("hits") or [])[:k]]
        techn = [name(p) for p in tech.procedure_paths(t.spec_text)]
        n += 1
        for arm, got in (("plain", plain), ("technique", techn)):
            score[arm][0] += bool(got) and got[0] in rel
            score[arm][1] += sum(g in rel for g in got) / max(1, len(got))
        print(f"{t.id:18s} plain={plain} technique={techn}", flush=True)
    for arm, (top1, prec) in score.items():
        print(f"{arm:10s} top-1 relevant {top1}/{n}, precision@{k} {prec / n:.2f}")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("action", choices=["publish", "measure"])
    ap.add_argument("--k", type=int, default=2)
    args = ap.parse_args()
    adapter, lib = ShimAdapter(), librarian()
    if args.action == "publish":
        publish(adapter, lib)
        time.sleep(1)
    else:
        measure(adapter, lib, args.k)
    return 0


if __name__ == "__main__":
    sys.exit(main())
