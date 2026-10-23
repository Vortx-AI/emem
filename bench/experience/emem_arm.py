"""The emem arm: verified procedures, stored and retrieved through emem.

publish(): each discovery task's procedure (its reference solution and its own
tests) becomes a signed memory note, together with the results of running those
tests under each library release, signed by the same verifier key. A procedure
whose tests fail under a release is recorded as STALE there.

EmemProcedures: before solving, retrieve the top-k procedure notes by the task
text alone (emem memory_search, no task ids or family labels), and give the
model each procedure with its verification status under the task's version.
"""
from __future__ import annotations

import json
import re
import urllib.request

from arms import BaselineFresh, SYSTEM

EMEM = "https://emem.dev"


def _post(path: str, body: dict, timeout: float = 120.0) -> dict:
    req = urllib.request.Request(EMEM + path, data=json.dumps(body).encode(),
                                 headers={"content-type": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as f:
        return json.load(f)


def _mcp(tool: str, args: dict) -> dict:
    out = _post("/mcp", {"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                         "params": {"name": tool, "arguments": args}})
    text = out["result"]["content"][0]["text"]
    if out["result"].get("isError"):
        raise RuntimeError(text[:300])
    return json.loads(text)


def procedure_note(task, verified: dict) -> str:
    """`verified` maps a release (e.g. pydantic==2.14.1) to True/False."""
    status = "\n".join(f"- {rel}: {'PASS' if ok else 'FAIL (stale under this release)'}"
                       for rel, ok in sorted(verified.items()))
    tests = "\n\n".join(t.read_text() for t in sorted((task.dir / "tests_hidden").glob("test_*.py")))
    return (f"---\nkind: procedure\nlibrary: {task.library}\nwritten_for: {task.version_spec}\n"
            f"family: {task.family}\n---\n# Procedure: {task.spec_text.splitlines()[0][:150]}\n\n"
            f"## What it solves\n\n{task.spec_text.strip()}\n\n"
            f"## Verified by running its tests\n\n{status}\n\n"
            f"## Code\n\n```python\n{task.reference.strip()}\n```\n\n"
            f"## Its tests\n\n```python\n{tests.strip()[:3000]}\n```\n")


def publish(agent, tasks: list, verified_by_task: dict) -> list:
    """Write one signed procedure note per discovery task with the verifier agent's key."""
    paths = []
    for t in tasks:
        path = f"/memories/by_attester/{agent.pk8}/procedures/{t.family}/{t.id}.md"
        agent.overwrite(path, procedure_note(t, verified_by_task[t.id]))
        paths.append(path)
    agent.save()
    return paths


class EmemStore:
    def __init__(self, prefix: str, k: int = 3):
        self.prefix, self.k = prefix, k

    def retrieve(self, query: str) -> list:
        R = _post("/v1/memory/search", {"q": query, "k": self.k, "path_prefix": self.prefix})
        out = []
        for h in (R.get("hits") or [])[: self.k]:
            p = h.get("path") or ""
            if "/procedures/" not in p:
                continue
            v = _mcp("memory_view", {"path": p})
            content = v.get("content") if isinstance(v.get("content"), str) else ""
            out.append({"path": p, "content": content, "attester": h.get("attester_pubkey_b32")})
        return out


def _status_for(content: str, version: str) -> str:
    m = re.search(r"^- (\S+==" + re.escape(version) + r"): (PASS|FAIL[^\n]*)$", content, re.M)
    return m.group(2) if m else "not verified under this release"


class EmemProcedures(BaselineFresh):
    """Task text plus the procedures emem retrieves for it, each with its status under the task's release."""

    name = "emem_procedures"

    def __init__(self, adapter, store: EmemStore, versions: dict, max_turns: int = 8):
        super().__init__(adapter, max_turns)
        self.store, self.versions = store, versions
        self.retrieved_log: list = []

    def solve(self, task_spec: str, context: dict) -> tuple:
        version = self.versions[context["version_spec"]]
        procs = self.store.retrieve(task_spec)
        self.retrieved_log.append({"task_id": context["task_id"], "paths": [p["path"] for p in procs]})
        blocks = []
        for p in procs:
            body = re.sub(r"^---.*?---\n", "", p["content"], flags=re.S)
            blocks.append(f"### {p['path'].rsplit('/', 1)[-1]} — under your release ({version}): "
                          f"{_status_for(p['content'], version)}\n\n{body[:6000]}")
        system = SYSTEM
        if blocks:
            system += ("\n\nVerified procedures retrieved from shared memory. Each was signed by a "
                       "verifier that ran its tests; its status under YOUR installed release is in "
                       "its heading. A procedure marked FAIL is stale under your release: reuse what "
                       "still holds and adapt the rest.\n\n" + "\n\n".join(blocks))
        text, usage = self._loop(system, task_spec, context.get("docs"))
        from arms import extract_code
        return extract_code(text), usage
