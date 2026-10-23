"""Task loading. A task is a directory under tasks/ (see README)."""
from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path

HERE = Path(__file__).resolve().parent
TASKS_DIR = HERE / "tasks"
PHASES = ("discovery", "transfer", "post_change")


@dataclass(frozen=True)
class Task:
    id: str
    family: str
    library: str
    version_spec: str
    phase: str
    discovery_of: tuple
    dir: Path

    @property
    def spec_text(self) -> str:
        return (self.dir / "task.md").read_text()

    @property
    def starter(self) -> str | None:
        p = self.dir / "starter.py"
        return p.read_text() if p.exists() else None

    @property
    def reference(self) -> str:
        return (self.dir / "reference.py").read_text()

    @property
    def reference_v1(self) -> str | None:
        p = self.dir / "reference_v1.py"
        return p.read_text() if p.exists() else None

    def test_sources(self) -> list:
        return [p.read_text() for p in sorted((self.dir / "tests_hidden").glob("test_*.py"))]


def load_tasks(root: Path = TASKS_DIR) -> list:
    out = []
    for d in sorted(p for p in root.iterdir() if (p / "meta.json").exists()):
        m = json.loads((d / "meta.json").read_text())
        out.append(Task(m["id"], m["family"], m["library"], m["version_spec"], m["phase"],
                        tuple(m.get("discovery_of", [])), d))
    return out


def stream_order(tasks: list) -> list:
    """Phase order (discovery, transfer, post_change); stable by id inside a phase."""
    return sorted(tasks, key=lambda t: (PHASES.index(t.phase), t.id))
