"""Surface + wire parity between Client and AsyncClient.

Two invariants guarded here:

1. **Method surface.** Every endpoint method on `Client` has a
   coroutine sibling on `AsyncClient` with the same name and same
   signature (parameter names + defaults). Catches the AttributeError
   trap a contributor would otherwise hit at runtime.

2. **Wire body parity.** For a representative spread of endpoints,
   firing both clients with identical arguments produces byte-for-byte
   identical request bodies (and identical URLs / query strings for
   GET endpoints). Catches subtle drift the surface test misses —
   e.g. one client dropping `None` differently or one wrapping
   `bands` in `list()` and the other not.

Adding a new endpoint requires extending both clients _and_ the
WIRE_PARITY_CASES list below; the surface test catches the former,
the wire test catches the latter.
"""

from __future__ import annotations

import asyncio
import inspect
import json
from typing import Any

import httpx
import pytest

from ememdev.client import AsyncClient, Client


# ── public-method enumeration helpers ──────────────────────────────


def _endpoint_methods(cls: type) -> set[str]:
    """Names of public methods on `cls` that are HTTP endpoints.

    Excludes lifecycle (`close`/`aclose`, dunders) and the private
    transport helpers (`_post`/`_get`/`_request`/`_boring_get`).
    """
    excluded = {"close", "aclose"}
    out: set[str] = set()
    for name, member in cls.__dict__.items():
        if name.startswith("_"):
            continue
        if name in excluded:
            continue
        if not (inspect.isfunction(member) or inspect.iscoroutinefunction(member)):
            continue
        out.add(name)
    return out


def _signature_shape(fn: Any) -> list[tuple[str, Any]]:
    """Return [(param_name, default_or_inspect.empty), ...] in declaration order.

    `inspect.iscoroutinefunction` reports True for the async siblings;
    the signature shape is otherwise identical, so we can compare both
    on equal footing.
    """
    sig = inspect.signature(fn)
    # Drop `self` — every method has it; it's noise for the comparison.
    return [(p.name, p.default) for p in sig.parameters.values() if p.name != "self"]


# ── surface parity ─────────────────────────────────────────────────


def test_endpoint_surfaces_match() -> None:
    sync = _endpoint_methods(Client)
    asy = _endpoint_methods(AsyncClient)
    only_sync = sorted(sync - asy)
    only_async = sorted(asy - sync)
    assert not only_sync and not only_async, (
        f"Client / AsyncClient method drift.\n"
        f"  on Client only:      {only_sync}\n"
        f"  on AsyncClient only: {only_async}"
    )


def test_async_methods_are_coroutines() -> None:
    for name in _endpoint_methods(AsyncClient):
        fn = getattr(AsyncClient, name)
        assert inspect.iscoroutinefunction(fn), (
            f"AsyncClient.{name} is not declared `async def` — the docstring "
            "guarantees every endpoint method is awaitable."
        )


def test_sync_methods_are_not_coroutines() -> None:
    for name in _endpoint_methods(Client):
        fn = getattr(Client, name)
        assert not inspect.iscoroutinefunction(fn), (
            f"Client.{name} is declared `async def` but should be synchronous."
        )


def test_signatures_match_per_method() -> None:
    """Each (name, default) pair must match between the two clients."""
    drift: list[str] = []
    for name in sorted(_endpoint_methods(Client)):
        sync_sig = _signature_shape(getattr(Client, name))
        async_sig = _signature_shape(getattr(AsyncClient, name))
        if sync_sig != async_sig:
            drift.append(
                f"{name}:\n  sync : {sync_sig}\n  async: {async_sig}"
            )
    assert not drift, "Signature drift in:\n\n" + "\n\n".join(drift)


# ── wire body parity ──────────────────────────────────────────────


# Each case is (method_name, args, kwargs). For every case both clients
# must emit byte-identical request bodies (POST) / query params (GET).
#
# Adding a new endpoint MUST add at least one case here. The test scans
# `_endpoint_methods(Client)` and asserts every method appears in at
# least one case, so omissions fail loudly rather than slipping by.
WIRE_PARITY_CASES: list[tuple[str, tuple[Any, ...], dict[str, Any]]] = [
    # Geocoder
    ("locate", ("Mount Everest",), {}),
    ("locate", (None,), {"lat": 27.99, "lng": 86.92}),
    # Read primitives
    ("recall", ("defi.zb493.xoso.zcb6a",), {"bands": ["copdem30m.elevation_mean"]}),
    ("recall", ("defi.zb493.xoso.zcb6a",), {}),  # bands=None
    ("recall_many", (["c1", "c2"],), {"bands": ["b1"]}),
    (
        "recall_polygon",
        ("Bengaluru",),
        {"bands": ["weather.temperature_2m"], "max_cells": 64},
    ),
    ("find_similar", ("defi.zb493.xoso.zcb6a",), {"k": 5}),
    ("compare", ("a", "b"), {}),
    ("compare_bands", ("c", "a", "b"), {"predicate": {"max_delta": 0.1}}),
    ("trajectory", ("c", "b"), {"window": (1_700_000_000, 1_720_000_000)}),
    ("diff", ("c", "b"), {"tslot_a": 1, "tslot_b": 2}),
    ("query_region", (), {"bbox": [0.0, 0.0, 1.0, 1.0], "bands": ["b1"]}),
    ("verify", ({"claim": 1},), {"cell": "c"}),
    ("ask", ("what is at this place?",), {"place": "Mumbai"}),
    ("fetch", (), {"cid": "cxjiu7l54ujzrpnekp24n4534yojpue4mprddbvevnqtti3lh5bq"}),
    ("backfill", ("c", "b"), {"start_unix": 1, "end_unix": 2}),
    ("intent", ("find flood risk",), {"place": "Chennai"}),
    # Physics
    ("heat_solve", ("c",), {}),
    ("wave_solve", ("c", 2.0, 8.0), {}),
    ("jepa_predict", ("c",), {}),
    ("jepa_predict_v2", ("c",), {}),
    # Boring lat/lng GET shortcuts
    ("ndvi", (), {"place": "Bengaluru"}),
    ("elevation", (), {"lat": 12.97, "lng": 77.59}),
    ("air", (), {"place": "Delhi"}),
    ("lst", (), {"lat": 13.0, "lng": 80.0}),
    ("soil", (), {"place": "Nairobi"}),
    ("water", (), {"place": "Lake Erie"}),
    ("forest", (), {"place": "Mato Grosso"}),
    ("weather", (), {"place": "Tokyo"}),
    # Introspection GETs
    ("bands", (), {}),
    ("algorithms", (), {}),
    ("algorithms", ("clay_prithvi_tessera_triple_consensus@1",), {}),
    ("sources", (), {}),
    ("schema", (), {}),
    ("manifests", (), {}),
    ("topics", (), {}),
    ("grid_info", (), {}),
    ("coverage_matrix", (), {}),
    ("agent_card", (), {}),
    ("discover", (), {}),
    ("openapi", (), {}),
    ("health", (), {}),
    # extended (v0.0.8)
("at", (), {}),
    ("verify_receipt", ({'x': 1},), {}),
    ("hunt", ('x',), {}),
    ("eudr_dds", ([{"plot_id": "p1"}],), {}),
    ("attest", ({'x': 1}, 'x', 'x'), {}),
    ("state", ('x',), {}),
    ("state_multi", ('x',), {}),
    ("state_diff", ('x',), {'tslot_a': 1, 'tslot_b': 1}),
    ("field_boundaries", (), {}),
    ("temporal_route", ('x',), {}),
    ("memory_token", ('x', 'x'), {}),
    ("memory_token_resolve", ('x',), {}),
    ("memory_contradictions", ('x',), {}),
    ("reviews", ('x', 'x'), {}),
    ("benchmark_grade", ({'x': 1},), {}),
    ("cell_info", ("x",), {}),
    ("cell_geojson", ("x",), {}),
    ("cell_recall_geojson", ("x",), {}),
    ("fact_by_cid", ("x",), {}),
    ("review_by_subject", ("x",), {}),
    ("contributor_by_pubkey", ("x",), {}),
    ("agent_quickref", (), {}),
    ("agent_stats", (), {}),
    ("capabilities", (), {}),
    ("contributors", (), {}),
    ("corpus_state_stats", (), {}),
    ("coverage", (), {}),
    ("data_availability", (), {}),
    ("demos", (), {}),
    ("errors", (), {}),
    ("fleet", (), {}),
    ("functions", (), {}),
    ("materializers", (), {}),
    ("quickstart", (), {}),
    ("tools", (), {}),
    ("benchmark", (), {}),
]


def test_every_endpoint_is_wire_tested() -> None:
    """Every public endpoint method must appear in at least one wire case."""
    covered = {name for name, _, _ in WIRE_PARITY_CASES}
    missing = sorted(_endpoint_methods(Client) - covered)
    assert not missing, (
        "WIRE_PARITY_CASES is missing entries for: "
        + ", ".join(missing)
        + "\n(Add at least one case per new endpoint so byte-level drift is "
        "caught here, not in production.)"
    )


def _capture_sync(method: str, args: tuple[Any, ...], kwargs: dict[str, Any]) -> httpx.Request:
    """Run a sync Client method through a no-network mock transport and
    return the actual `httpx.Request` it would have sent."""
    captured: dict[str, httpx.Request] = {}

    def handler(request: httpx.Request) -> httpx.Response:
        captured["req"] = request
        return httpx.Response(
            200,
            headers={"content-type": "application/json"},
            json={},
        )

    transport = httpx.MockTransport(handler)
    with Client(base_url="https://test.invalid", transport=transport) as c:
        getattr(c, method)(*args, **kwargs)
    return captured["req"]


def _capture_async(
    method: str, args: tuple[Any, ...], kwargs: dict[str, Any]
) -> httpx.Request:
    captured: dict[str, httpx.Request] = {}

    async def handler(request: httpx.Request) -> httpx.Response:
        captured["req"] = request
        return httpx.Response(
            200,
            headers={"content-type": "application/json"},
            json={},
        )

    async def run() -> None:
        transport = httpx.MockTransport(handler)
        async with AsyncClient(base_url="https://test.invalid", transport=transport) as c:
            await getattr(c, method)(*args, **kwargs)

    asyncio.run(run())
    return captured["req"]


@pytest.mark.parametrize(("method", "args", "kwargs"), WIRE_PARITY_CASES)
def test_wire_body_parity(method: str, args: tuple[Any, ...], kwargs: dict[str, Any]) -> None:
    sync_req = _capture_sync(method, args, kwargs)
    async_req = _capture_async(method, args, kwargs)

    # Same URL + method.
    assert str(sync_req.url) == str(async_req.url), (
        f"{method}: URL drift\n  sync : {sync_req.url}\n  async: {async_req.url}"
    )
    assert sync_req.method == async_req.method

    # Same User-Agent (defends against accidentally diverging the UA pin).
    assert (
        sync_req.headers.get("user-agent") == async_req.headers.get("user-agent")
    ), f"{method}: User-Agent drift"

    # Same body (POST) or no body (GET).
    if sync_req.method == "POST":
        sync_body = json.loads(sync_req.content) if sync_req.content else None
        async_body = json.loads(async_req.content) if async_req.content else None
        assert sync_body == async_body, (
            f"{method}: POST body drift\n  sync : {sync_body}\n  async: {async_body}"
        )
    else:
        assert not sync_req.content and not async_req.content


# ── lifecycle parity ──────────────────────────────────────────────


def test_sync_client_context_manager_closes() -> None:
    """Entering and exiting Client must close the underlying httpx client."""
    c = Client(base_url="https://test.invalid")
    with c:
        pass
    assert c._http.is_closed  # type: ignore[attr-defined]


def test_async_client_context_manager_closes() -> None:
    async def run() -> bool:
        c = AsyncClient(base_url="https://test.invalid")
        async with c:
            pass
        return c._http.is_closed  # type: ignore[attr-defined]

    assert asyncio.run(run())
