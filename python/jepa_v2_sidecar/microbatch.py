"""Dynamic micro-batching for the GPU sidecar.

Coalesces concurrent single-item inference requests into one batched
forward pass, amortising kernel-launch + CUDA dispatch when several
recalls land at once. Deliberately torch-free so the batching machinery
(queue / window / ordering / error fan-out) is unit-testable without a
GPU — see ``test_microbatch.py``.
"""

from __future__ import annotations

import queue
import threading
import time
from typing import Any, Callable


class MicroBatcher:
    """Coalesce concurrent single-item inference into one batched forward.

    One daemon worker thread per model. ``run_batch(list[payload]) ->
    list[result]`` MUST return one result per payload, in the same order.
    A failure in the batched forward propagates to every waiter in that
    batch (so one bad input never silently hangs the others).

    The worker blocks for the first request, then drains up to
    ``max_batch - 1`` more that arrive within ``window_ms`` before running
    a single ``run_batch``. With ``max_batch == 1`` this degenerates to
    one-request-per-forward (callers normally bypass the batcher entirely
    in that case).
    """

    def __init__(
        self,
        name: str,
        run_batch: Callable[[list[Any]], list[Any]],
        max_batch: int,
        window_ms: float,
    ) -> None:
        self._name = name
        self._run_batch = run_batch
        self._max_batch = max(1, int(max_batch))
        self._window_s = max(0.0, window_ms / 1000.0)
        self._q: "queue.Queue[tuple[Any, dict]]" = queue.Queue()
        self._worker = threading.Thread(
            target=self._loop, name=f"batch-{name}", daemon=True
        )
        self._worker.start()

    def submit(self, payload: Any) -> Any:
        """Enqueue one payload and block until its batch has run."""
        slot: dict = {"event": threading.Event(), "result": None, "error": None}
        self._q.put((payload, slot))
        slot["event"].wait()
        if slot["error"] is not None:
            raise slot["error"]
        return slot["result"]

    def _loop(self) -> None:
        while True:
            payload, slot = self._q.get()
            batch = [(payload, slot)]
            deadline = time.monotonic() + self._window_s
            while len(batch) < self._max_batch:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    break
                try:
                    batch.append(self._q.get(timeout=remaining))
                except queue.Empty:
                    break
            payloads = [p for (p, _slot) in batch]
            try:
                results = self._run_batch(payloads)
                if len(results) != len(batch):
                    raise RuntimeError(
                        f"run_batch returned {len(results)} results for "
                        f"{len(batch)} inputs"
                    )
                for (_p, s), r in zip(batch, results):
                    s["result"] = r
                    s["event"].set()
            except Exception as exc:  # noqa: BLE001 — fan failure to all waiters
                for (_p, s) in batch:
                    s["error"] = exc
                    s["event"].set()
