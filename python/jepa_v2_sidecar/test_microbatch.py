"""Unit tests for the GPU-sidecar micro-batcher (torch-free).

Proves the coalescing / ordering / error-fan-out logic without a GPU, so
the batching machinery is verifiable independent of the encoders it wraps.
Run: `python test_microbatch.py` (or under pytest).
"""

import threading
import time

from microbatch import MicroBatcher


def test_results_correct_and_in_order():
    # run_batch doubles each payload; results must map back per-submitter.
    b = MicroBatcher("double", lambda xs: [x * 2 for x in xs], max_batch=8, window_ms=20)
    out: dict[int, int] = {}
    lock = threading.Lock()

    def worker(i: int):
        r = b.submit(i)
        with lock:
            out[i] = r

    threads = [threading.Thread(target=worker, args=(i,)) for i in range(50)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    assert out == {i: i * 2 for i in range(50)}, "each submitter must get its own doubled result"


def test_actually_coalesces():
    # Record the batch size of each forward; with concurrent submits and a
    # generous window, at least one forward must batch >1 (else batching is
    # a no-op). max_batch caps the largest batch.
    sizes: list[int] = []

    def run_batch(xs):
        sizes.append(len(xs))
        return xs

    b = MicroBatcher("coalesce", run_batch, max_batch=8, window_ms=50)
    threads = [threading.Thread(target=lambda: b.submit(1)) for _ in range(16)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    assert max(sizes) > 1, f"expected a coalesced batch >1, got sizes {sizes}"
    assert max(sizes) <= 8, f"batch must never exceed max_batch=8, got {sizes}"
    assert sum(sizes) == 16, "every submitted item must be processed exactly once"


def test_max_batch_one_is_unbatched():
    sizes: list[int] = []

    def run_batch(xs):
        sizes.append(len(xs))
        return xs

    b = MicroBatcher("single", run_batch, max_batch=1, window_ms=50)
    threads = [threading.Thread(target=lambda: b.submit(1)) for _ in range(8)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    assert sizes == [1] * 8, f"max_batch=1 must run one item per forward, got {sizes}"


def test_error_fans_out_to_every_waiter():
    class Boom(RuntimeError):
        pass

    def run_batch(_xs):
        raise Boom("forward failed")

    b = MicroBatcher("boom", run_batch, max_batch=8, window_ms=20)
    errors: list[BaseException] = []
    lock = threading.Lock()

    def worker():
        try:
            b.submit(1)
        except BaseException as e:  # noqa: BLE001
            with lock:
                errors.append(e)

    threads = [threading.Thread(target=worker) for _ in range(10)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    assert len(errors) == 10, "every waiter in a failed batch must see the error"
    assert all(isinstance(e, Boom) for e in errors), "the original exception type must propagate"


def test_length_mismatch_is_an_error():
    # A run_batch that returns the wrong count must fail loudly (caught and
    # fanned out), never silently mis-map results to submitters.
    b = MicroBatcher("badlen", lambda xs: xs[:-1] if len(xs) > 1 else xs, max_batch=8, window_ms=30)
    errors = []
    lock = threading.Lock()

    def worker():
        try:
            b.submit(1)
        except Exception as e:  # noqa: BLE001
            with lock:
                errors.append(e)

    threads = [threading.Thread(target=worker) for _ in range(6)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    # When a batch of >1 drops a result, that batch's waiters all error.
    assert errors, "a result-count mismatch must surface as an error, not a silent hang"


if __name__ == "__main__":
    for name, fn in sorted(globals().items()):
        if name.startswith("test_") and callable(fn):
            fn()
            print(f"ok  {name}")
    print("all microbatch tests passed")
