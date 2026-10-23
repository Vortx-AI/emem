"""Arms: how an agent attempts a task. Only the two baselines exist so far.

The model call sits behind `ModelAdapter`. No adapter is wired: the default
raises NotImplementedError, so nothing here can reach a model API by accident.
"""
from __future__ import annotations

import re
import subprocess
import time
from dataclasses import asdict, dataclass, field
from typing import Callable, Optional, Protocol


@dataclass
class Usage:
    input_tokens: int = 0
    output_tokens: int = 0
    tool_calls: int = 0
    wall_seconds: float = 0.0
    model_id: str = ""

    def add(self, other: "Usage") -> None:
        self.input_tokens += other.input_tokens
        self.output_tokens += other.output_tokens
        self.tool_calls += other.tool_calls
        self.wall_seconds += other.wall_seconds
        self.model_id = self.model_id or other.model_id

    def as_dict(self) -> dict:
        return asdict(self)


@dataclass
class ToolCall:
    name: str
    arguments: dict


@dataclass
class ModelReply:
    text: str
    tool_calls: list = field(default_factory=list)
    input_tokens: int = 0
    output_tokens: int = 0


class ModelAdapter(Protocol):
    model_id: str

    def complete(self, system: str, messages: list, tools: list) -> ModelReply: ...


class UnwiredAdapter:
    """Placeholder. Replace with an adapter for the model under test."""

    model_id = "UNWIRED"

    def complete(self, system: str, messages: list, tools: list) -> ModelReply:
        raise NotImplementedError("no model adapter wired: implement ModelAdapter.complete()")


DOCS_TOOL = {
    "name": "docs_lookup",
    "description": "Look up Python documentation for a dotted name (for example 'pydantic.BaseModel') "
                   "in the library version installed for this task.",
    "parameters": {"query": "dotted name"},
}


class PydocDocs:
    """Offline docs tool: `pydoc` run inside the task's pinned venv, so it matches the version."""

    def __init__(self, python: str, max_chars: int = 6000):
        self.python, self.max_chars = python, max_chars

    def __call__(self, query: str) -> str:
        if not re.fullmatch(r"[A-Za-z_][\w.]*", query or ""):
            return "error: query must be a dotted Python name"
        p = subprocess.run([self.python, "-m", "pydoc", query], capture_output=True, text=True, timeout=30)
        return (p.stdout or p.stderr)[: self.max_chars]


def extract_code(text: str) -> str:
    blocks = re.findall(r"```(?:python)?\n(.*?)```", text, re.S)
    return (blocks[-1] if blocks else text).strip() + "\n"


class Arm:
    name = "arm"

    def solve(self, task_spec: str, context: dict) -> tuple:
        """Return (solution_source, Usage). `context` carries task_id, phase, docs (callable)."""
        raise NotImplementedError

    def observe(self, task_id: str, passed: bool) -> None:
        """Called by the driver after independent checking. Only pass/fail, never test content."""


SYSTEM = ("You write one Python file, solution.py, that satisfies the task. Hidden tests will check it. "
          "Use docs_lookup when unsure about library behaviour. Reply with the final file in one "
          "```python block.")


class BaselineFresh(Arm):
    """Task spec plus a docs-lookup tool. Nothing carries over between tasks."""

    name = "baseline_fresh"

    def __init__(self, adapter: Optional[ModelAdapter] = None, max_turns: int = 8):
        self.adapter = adapter or UnwiredAdapter()
        self.max_turns = max_turns

    def _system(self) -> str:
        return SYSTEM

    def _loop(self, system: str, first_user: str, docs: Optional[Callable[[str], str]]) -> tuple:
        usage = Usage(model_id=self.adapter.model_id)
        messages = [{"role": "user", "content": first_user}]
        text = ""
        for _ in range(self.max_turns):
            t0 = time.monotonic()
            reply = self.adapter.complete(system, messages, [DOCS_TOOL])
            usage.wall_seconds += time.monotonic() - t0
            usage.input_tokens += reply.input_tokens
            usage.output_tokens += reply.output_tokens
            text = reply.text
            messages.append({"role": "assistant", "content": reply.text, "tool_calls": reply.tool_calls})
            if not reply.tool_calls:
                break
            for call in reply.tool_calls:
                usage.tool_calls += 1
                t0 = time.monotonic()
                out = docs(call.arguments.get("query", "")) if docs else "docs tool not available"
                usage.wall_seconds += time.monotonic() - t0
                messages.append({"role": "tool", "name": call.name, "content": out})
        return text, usage

    def solve(self, task_spec: str, context: dict) -> tuple:
        text, usage = self._loop(self._system(), task_spec, context.get("docs"))
        return extract_code(text), usage


class BaselineNotes(BaselineFresh):
    """As baseline_fresh, plus free-text notes this arm wrote itself on earlier tasks in the stream.

    Notes are the arm's own words, unverified: no signing, no filtering. That is the point of the
    comparison with a verified-experience arm.
    """

    name = "baseline_notes"

    def __init__(self, adapter: Optional[ModelAdapter] = None, max_turns: int = 8, max_notes_chars: int = 12000):
        super().__init__(adapter, max_turns)
        self.notes: list = []
        self.max_notes_chars = max_notes_chars
        self._last: Optional[tuple] = None

    def _system(self) -> str:
        if not self.notes:
            return SYSTEM
        joined = "\n\n".join(self.notes)[-self.max_notes_chars:]
        return SYSTEM + "\n\nYour own notes from earlier tasks (unverified, may be wrong):\n" + joined

    def solve(self, task_spec: str, context: dict) -> tuple:
        source, usage = super().solve(task_spec, context)
        t0 = time.monotonic()
        reply = self.adapter.complete(
            self._system(),
            [{"role": "user", "content": task_spec}, {"role": "assistant", "content": source},
             {"role": "user", "content": "Write short notes for your future self about library behaviour "
                                         "you learned or relied on in this task. Plain text."}],
            [],
        )
        usage.wall_seconds += time.monotonic() - t0
        usage.input_tokens += reply.input_tokens
        usage.output_tokens += reply.output_tokens
        self._last = (context.get("task_id", "?"), reply.text.strip())
        return source, usage

    def observe(self, task_id: str, passed: bool) -> None:
        if self._last and self._last[0] == task_id:
            self.notes.append(f"[{task_id}] ({'passed' if passed else 'failed'} independent check) {self._last[1]}")
            self._last = None


class DryRunArm(Arm):
    """Plumbing check: returns the task's reference solution with zero usage. Not a model."""

    def __init__(self, name: str, reference_for: Callable[[str], str]):
        self.name = name
        self._ref = reference_for

    def solve(self, task_spec: str, context: dict) -> tuple:
        return self._ref(context["task_id"]), Usage(model_id="dry-run")


ARMS = {"baseline_fresh": BaselineFresh, "baseline_notes": BaselineNotes}
