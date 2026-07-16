"""Thin synchronous + asynchronous HTTP client for emem.dev.

Covers ~90% of the live REST surface (75 of 83 endpoints in OpenAPI).
Each REST endpoint maps 1:1 to a method on both ``Client`` (sync) and
``AsyncClient`` (async); the wire body for identical inputs is
byte-for-byte identical across the two — enforced by the parity test
suite at ``sdks/emem-py/tests/test_client_parity.py``.

Endpoints intentionally not wrapped (non-JSON or non-HTTP-1.1
semantics — use ``httpx`` directly): ``/v1/stream`` (SSE),
``/v1/cells/{cell64}/scene.{png,rgb}`` (binary imagery),
``/v1/coverage_map.svg`` (SVG), ``/v1/attest_cbor`` (raw CBOR body),
``/mcp`` (JSON-RPC), ``/.well-known/emem.json`` (discovery).

All wrapped calls return parsed JSON. HTTP non-2xx responses raise
``EmemHTTPError`` with the response status, URL, and parsed body
(when JSON).
"""

from __future__ import annotations

import os
from importlib.metadata import PackageNotFoundError, version as _pkg_version
from typing import Any, Iterable, Mapping, Sequence

import httpx

DEFAULT_BASE_URL = os.environ.get("EMEM_BASE_URL", "https://emem.dev")
DEFAULT_TIMEOUT = float(os.environ.get("EMEM_TIMEOUT_SECS", "180"))

# Derive the User-Agent version from the installed distribution (pyproject.toml
# is the single source), so it never drifts from the shipped package version.
try:
    _VERSION = _pkg_version("ememdev")
except PackageNotFoundError:  # running from a source checkout, not installed
    _VERSION = "0+unknown"
USER_AGENT = f"emem-py/{_VERSION} (+https://emem.dev)"


class EmemError(Exception):
    """Base class for emem client errors."""


class EmemHTTPError(EmemError):
    """Raised when the responder returns a non-2xx HTTP status."""

    def __init__(self, status_code: int, url: str, body: Any) -> None:
        self.status_code = status_code
        self.url = url
        self.body = body
        super().__init__(f"emem responder returned {status_code} for {url}: {body!r}")


def _strip_none(d: Mapping[str, Any]) -> dict[str, Any]:
    return {k: v for k, v in d.items() if v is not None}


class Client:
    """Synchronous HTTP client for emem.dev.

    Parameters
    ----------
    base_url:
        Responder root. Defaults to ``$EMEM_BASE_URL`` or ``https://emem.dev``.
    timeout:
        Request timeout in seconds. Defaults to ``$EMEM_TIMEOUT_SECS`` or 180 s
        (matches the responder's gateway timeout).
    transport:
        Optional pre-built ``httpx.BaseTransport`` for testing.
    """

    def __init__(
        self,
        base_url: str = DEFAULT_BASE_URL,
        timeout: float = DEFAULT_TIMEOUT,
        transport: httpx.BaseTransport | None = None,
    ) -> None:
        self.base_url = base_url.rstrip("/")
        self._http = httpx.Client(
            base_url=self.base_url,
            timeout=timeout,
            headers={"user-agent": USER_AGENT, "accept": "application/json"},
            transport=transport,
        )

    def __enter__(self) -> "Client":
        return self

    def __exit__(self, *_exc: object) -> None:
        self.close()

    def close(self) -> None:
        self._http.close()

    def _post(self, path: str, body: Mapping[str, Any]) -> Any:
        return self._request("POST", path, json=_strip_none(body))

    def _get(self, path: str, params: Mapping[str, Any] | None = None) -> Any:
        return self._request("GET", path, params=_strip_none(params or {}))

    def _request(self, method: str, path: str, **kwargs: Any) -> Any:
        url = path if path.startswith("/") else f"/{path}"
        resp = self._http.request(method, url, **kwargs)
        if resp.status_code >= 400:
            try:
                body: Any = resp.json()
            except ValueError:
                body = resp.text
            raise EmemHTTPError(resp.status_code, str(resp.url), body)
        if resp.headers.get("content-type", "").startswith("application/json"):
            return resp.json()
        return resp.text

    # ── Geocoder ────────────────────────────────────────────────────────
    def locate(
        self,
        place: str | None = None,
        *,
        lat: float | None = None,
        lng: float | None = None,
    ) -> Any:
        """Resolve a place name (or lat/lng) to a cell64."""
        return self._post("/v1/locate", _locate_body(place, lat, lng))

    # ── Read primitives ────────────────────────────────────────────────
    def recall(
        self,
        cell: str,
        bands: Sequence[str] | None = None,
        tslot: int | None = None,
    ) -> Any:
        return self._post("/v1/recall", _recall_body(cell, bands, tslot))

    def recall_many(self, cells: Sequence[str], bands: Sequence[str] | None = None) -> Any:
        return self._post("/v1/recall_many", _recall_many_body(cells, bands))

    def recall_polygon(
        self,
        place: str | None = None,
        *,
        polygon_bbox: Sequence[float] | None = None,
        polygon_geojson: Mapping[str, Any] | None = None,
        bands: Sequence[str] | None = None,
        max_cells: int | None = None,
        cells_per_sqkm: float | None = None,
        drill_on_water: bool | None = None,
    ) -> Any:
        return self._post(
            "/v1/recall_polygon",
            _recall_polygon_body(
                place,
                polygon_bbox,
                polygon_geojson,
                bands,
                max_cells,
                cells_per_sqkm,
                drill_on_water,
            ),
        )

    def find_similar(
        self,
        key: str,
        k: int = 10,
        band: str = "geotessera",
        mode: str = "cosine",
    ) -> Any:
        return self._post("/v1/find_similar", _find_similar_body(key, k, band, mode))

    def compare(self, a: str, b: str, family: str | None = None) -> Any:
        return self._post("/v1/compare", _compare_body(a, b, family))

    def compare_bands(
        self,
        cell: str,
        a: str,
        b: str,
        *,
        tslot_a: int | None = None,
        tslot_b: int | None = None,
        predicate: Mapping[str, Any] | None = None,
    ) -> Any:
        return self._post(
            "/v1/compare_bands",
            _compare_bands_body(cell, a, b, tslot_a, tslot_b, predicate),
        )

    def trajectory(self, cell: str, band: str, window: tuple[int, int]) -> Any:
        return self._post("/v1/trajectory", _trajectory_body(cell, band, window))

    def diff(self, cell: str, band: str, tslot_a: int, tslot_b: int) -> Any:
        return self._post("/v1/diff", _diff_body(cell, band, tslot_a, tslot_b))

    def query_region(
        self,
        *,
        geometry: str | None = None,
        bbox: Sequence[float] | None = None,
        max_cells: int | None = None,
        bands: Sequence[str] | None = None,
        agg: str | None = None,
    ) -> Any:
        return self._post(
            "/v1/query_region",
            _query_region_body(geometry, bbox, max_cells, bands, agg),
        )

    def verify(self, claim: Mapping[str, Any], cell: str, mode: str = "fast") -> Any:
        return self._post("/v1/verify", _verify_body(claim, cell, mode))

    def ask(
        self,
        q: str,
        *,
        place: str | None = None,
        cell: str | None = None,
        lat: float | None = None,
        lng: float | None = None,
        include_image: bool = False,
        verbose: bool = False,
    ) -> Any:
        return self._post(
            "/v1/ask",
            _ask_body(q, place, cell, lat, lng, include_image, verbose),
        )

    def fetch(
        self,
        *,
        cid: str | None = None,
        cell: str | None = None,
        band: str | None = None,
        tslot: int | None = None,
    ) -> Any:
        return self._post("/v1/fetch", _fetch_body(cid, cell, band, tslot))

    def backfill(
        self,
        cell: str,
        band: str,
        *,
        start_unix: int | None = None,
        end_unix: int | None = None,
        max_facts: int | None = None,
    ) -> Any:
        return self._post(
            "/v1/backfill",
            _backfill_body(cell, band, start_unix, end_unix, max_facts),
        )

    def intent(self, q: str, *, place: str | None = None, cell: str | None = None) -> Any:
        return self._post("/v1/intent", _intent_body(q, place, cell))

    # ── Physics solvers ────────────────────────────────────────────────
    def heat_solve(
        self,
        cell: str,
        *,
        hours_ahead: float = 6.0,
        diffusivity_m2_per_s: float = 1.0e-6,
    ) -> Any:
        return self._post(
            "/v1/heat_solve",
            _heat_solve_body(cell, hours_ahead, diffusivity_m2_per_s),
        )

    def wave_solve(
        self,
        coastal_cell: str,
        offshore_height_m: float,
        period_s: float,
        *,
        n_offshore_cells: int = 8,
    ) -> Any:
        return self._post(
            "/v1/wave_solve",
            _wave_solve_body(coastal_cell, offshore_height_m, period_s, n_offshore_cells),
        )

    def jepa_predict(
        self,
        cell: str,
        *,
        band: str = "indices.ndvi",
        lookback_months: int = 6,
        forecast_horizon_months: int = 1,
    ) -> Any:
        return self._post(
            "/v1/jepa_predict",
            _jepa_predict_body(cell, band, lookback_months, forecast_horizon_months),
        )

    def jepa_predict_v2(self, cell: str, *, band: str = "indices.ndvi", k_history: int = 5) -> Any:
        return self._post(
            "/v1/jepa_predict_v2", _jepa_predict_v2_body(cell, band, k_history)
        )

    # ── Boring lat/lng shortcuts ───────────────────────────────────────
    def _boring_get(self, path: str, *, lat: float | None, lng: float | None, place: str | None) -> Any:
        return self._get(path, _boring_get_params(lat, lng, place))

    def ndvi(self, *, lat: float | None = None, lng: float | None = None, place: str | None = None) -> Any:
        return self._boring_get("/v1/ndvi", lat=lat, lng=lng, place=place)

    def elevation(self, *, lat: float | None = None, lng: float | None = None, place: str | None = None) -> Any:
        return self._boring_get("/v1/elevation", lat=lat, lng=lng, place=place)

    def air(self, *, lat: float | None = None, lng: float | None = None, place: str | None = None) -> Any:
        return self._boring_get("/v1/air", lat=lat, lng=lng, place=place)

    def lst(self, *, lat: float | None = None, lng: float | None = None, place: str | None = None) -> Any:
        return self._boring_get("/v1/lst", lat=lat, lng=lng, place=place)

    def soil(self, *, lat: float | None = None, lng: float | None = None, place: str | None = None) -> Any:
        return self._boring_get("/v1/soil", lat=lat, lng=lng, place=place)

    def water(self, *, lat: float | None = None, lng: float | None = None, place: str | None = None) -> Any:
        return self._boring_get("/v1/water", lat=lat, lng=lng, place=place)

    def forest(self, *, lat: float | None = None, lng: float | None = None, place: str | None = None) -> Any:
        return self._boring_get("/v1/forest", lat=lat, lng=lng, place=place)

    def weather(self, *, lat: float | None = None, lng: float | None = None, place: str | None = None) -> Any:
        return self._boring_get("/v1/weather", lat=lat, lng=lng, place=place)

    # ── Introspection ─────────────────────────────────────────────────
    def bands(self) -> Any:
        return self._get("/v1/bands")

    def algorithms(self, key: str | None = None) -> Any:
        return self._get(_algorithms_path(key))

    def sources(self) -> Any:
        return self._get("/v1/sources")

    def schema(self) -> Any:
        return self._get("/v1/schema")

    def manifests(self) -> Any:
        return self._get("/v1/manifests")

    def topics(self) -> Any:
        return self._get("/v1/topics")

    def grid_info(self) -> Any:
        return self._get("/v1/grid_info")

    def coverage_matrix(self) -> Any:
        return self._get("/v1/coverage_matrix")

    def agent_card(self) -> Any:
        return self._get("/v1/agent_card")

    def discover(self) -> Any:
        return self._get("/v1/discover")

    def openapi(self) -> Any:
        return self._get("/openapi.json")

    def health(self) -> Any:
        return self._get("/health")

    # ── extended methods (v0.0.8) ────────────────────────────────────
    def at(
        self,
        *,
        place: str | None = None,
        cell: str | None = None,
        lat: float | None = None,
        lng: float | None = None,
        bands: Sequence[str] | None = None,
    ) -> Any:
        return self._post("/v1/at", _at_body(place, cell, lat, lng, bands))

    def verify_receipt(
        self,
        receipt: Mapping[str, Any],
        *,
        pubkey_b32: str | None = None,
    ) -> Any:
        return self._post("/v1/verify_receipt", _verify_receipt_body(receipt, pubkey_b32))

    def hunt(
        self,
        event: str,
        *,
        region: str | None = None,
        polygon_bbox: Sequence[float] | None = None,
        polygon_geojson: Mapping[str, Any] | None = None,
        n_cells: int | None = None,
    ) -> Any:
        return self._post("/v1/hunt", _hunt_body(event, region, polygon_bbox, polygon_geojson, n_cells))

    def eudr_dds(
        self,
        plots: Sequence[Mapping[str, Any]],
        *,
        operator: Mapping[str, Any] | None = None,
        include_evidence: bool = False,
    ) -> Any:
        return self._post("/v1/eudr_dds", _eudr_dds_body(plots, operator, include_evidence))

    def attest(
        self,
        fact: Mapping[str, Any],
        signature_b32: str,
        pubkey_b32: str,
    ) -> Any:
        return self._post("/v1/attest", _attest_body(fact, signature_b32, pubkey_b32))

    def state(
        self,
        cell: str,
        *,
        view: str = 'encoder',
        encoder: str | None = None,
    ) -> Any:
        return self._post("/v1/state", _state_body(cell, view, encoder))

    def state_multi(
        self,
        cell: str,
    ) -> Any:
        return self._post("/v1/state_multi", _state_multi_body(cell))

    def state_diff(
        self,
        cell: str,
        *,
        encoder: str = 'geotessera',
        tslot_a: int,
        tslot_b: int,
    ) -> Any:
        return self._post("/v1/state_diff", _state_diff_body(cell, encoder, tslot_a, tslot_b))

    def field_boundaries(
        self,
        *,
        place: str | None = None,
        polygon_bbox: Sequence[float] | None = None,
        polygon_geojson: Mapping[str, Any] | None = None,
        max_polygons: int | None = None,
    ) -> Any:
        return self._post("/v1/field_boundaries", _field_boundaries_body(place, polygon_bbox, polygon_geojson, max_polygons))

    def temporal_route(
        self,
        q: str,
        *,
        place: str | None = None,
        cell: str | None = None,
    ) -> Any:
        return self._post("/v1/temporal_route", _temporal_route_body(q, place, cell))

    def memory_token(
        self,
        cell: str,
        fact_cid: str,
    ) -> Any:
        return self._post("/v1/memory_token", _memory_token_body(cell, fact_cid))

    def memory_token_resolve(
        self,
        token: str,
    ) -> Any:
        return self._post("/v1/memory_token/resolve", _memory_token_resolve_body(token))

    def memory_contradictions(
        self,
        cell: str,
        *,
        band: str | None = None,
        tslot: int | None = None,
    ) -> Any:
        return self._post("/v1/memory_contradictions", _memory_contradictions_body(cell, band, tslot))

    def reviews(
        self,
        subject_id: str,
        outcome: str,
        *,
        rating: int | None = None,
        notes: str | None = None,
    ) -> Any:
        return self._post("/v1/reviews", _reviews_body(subject_id, outcome, rating, notes))

    def benchmark_grade(
        self,
        answers: Mapping[str, Any],
    ) -> Any:
        return self._post("/v1/benchmark/grade", _benchmark_grade_body(answers))

    def cell_info(self, cell64: str) -> Any:
        return self._get(f"/v1/cells/{cell64}/info")

    def cell_geojson(self, cell64: str) -> Any:
        return self._get(f"/v1/cells/{cell64}/geojson")

    def cell_recall_geojson(self, cell64: str) -> Any:
        return self._get(f"/v1/cells/{cell64}/recall_geojson")

    def fact_by_cid(self, cid: str) -> Any:
        return self._get(f"/v1/facts/{cid}")

    def review_by_subject(self, subject_id: str) -> Any:
        return self._get(f"/v1/reviews/{subject_id}")

    def contributor_by_pubkey(self, pubkey_b32: str) -> Any:
        return self._get(f"/v1/contributors/{pubkey_b32}")

    def agent_quickref(self) -> Any:
        return self._get("/v1/agent_quickref")

    def agent_stats(self) -> Any:
        return self._get("/v1/agent_stats")

    def capabilities(self) -> Any:
        return self._get("/v1/capabilities")

    def contributors(self) -> Any:
        return self._get("/v1/contributors")

    def corpus_state_stats(self) -> Any:
        return self._get("/v1/corpus_state_stats")

    def coverage(self) -> Any:
        return self._get("/v1/coverage")

    def data_availability(self) -> Any:
        return self._get("/v1/data_availability")

    def demos(self) -> Any:
        return self._get("/v1/demos")

    def errors(self) -> Any:
        return self._get("/v1/errors")

    def fleet(self) -> Any:
        return self._get("/v1/fleet")

    def functions(self) -> Any:
        return self._get("/v1/functions")

    def materializers(self) -> Any:
        return self._get("/v1/materializers")

    def quickstart(self) -> Any:
        return self._get("/v1/quickstart")

    def tools(self) -> Any:
        return self._get("/v1/tools")

    def benchmark(self) -> Any:
        return self._get("/v1/benchmark")



# ── Endpoint body builders ──────────────────────────────────────────────
# Shared between Client and AsyncClient so the wire bodies they emit for
# identical inputs are byte-for-byte identical. Adding a new endpoint
# means adding a builder here and one method-pair below; the parity test
# guarantees both clients pick it up.
def _locate_body(place: str | None, lat: float | None, lng: float | None) -> dict[str, Any]:
    return {"place": place, "lat": lat, "lng": lng}


def _recall_body(
    cell: str, bands: Sequence[str] | None, tslot: int | None
) -> dict[str, Any]:
    return {"cell": cell, "bands": list(bands) if bands else None, "tslot": tslot}


def _recall_many_body(
    cells: Sequence[str], bands: Sequence[str] | None
) -> dict[str, Any]:
    return {"cells": list(cells), "bands": list(bands) if bands else None}


def _recall_polygon_body(
    place: str | None,
    polygon_bbox: Sequence[float] | None,
    polygon_geojson: Mapping[str, Any] | None,
    bands: Sequence[str] | None,
    max_cells: int | None,
    cells_per_sqkm: float | None,
    drill_on_water: bool | None,
) -> dict[str, Any]:
    return {
        "place": place,
        "polygon_bbox": list(polygon_bbox) if polygon_bbox else None,
        "polygon_geojson": polygon_geojson,
        "bands": list(bands) if bands else None,
        "max_cells": max_cells,
        "cells_per_sqkm": cells_per_sqkm,
        "drill_on_water": drill_on_water,
    }


def _find_similar_body(key: str, k: int, band: str, mode: str) -> dict[str, Any]:
    return {"key": key, "k": k, "band": band, "mode": mode}


def _compare_body(a: str, b: str, family: str | None) -> dict[str, Any]:
    return {"a": a, "b": b, "family": family}


def _compare_bands_body(
    cell: str,
    a: str,
    b: str,
    tslot_a: int | None,
    tslot_b: int | None,
    predicate: Mapping[str, Any] | None,
) -> dict[str, Any]:
    return {
        "cell": cell,
        "a": a,
        "b": b,
        "tslot_a": tslot_a,
        "tslot_b": tslot_b,
        "predicate": predicate,
    }


def _trajectory_body(cell: str, band: str, window: tuple[int, int]) -> dict[str, Any]:
    return {"cell": cell, "band": band, "window": list(window)}


def _diff_body(cell: str, band: str, tslot_a: int, tslot_b: int) -> dict[str, Any]:
    return {"cell": cell, "band": band, "tslot_a": tslot_a, "tslot_b": tslot_b}


def _query_region_body(
    geometry: str | None,
    bbox: Sequence[float] | None,
    max_cells: int | None,
    bands: Sequence[str] | None,
    agg: str | None,
) -> dict[str, Any]:
    return {
        "geometry": geometry,
        "bbox": list(bbox) if bbox else None,
        "max_cells": max_cells,
        "bands": list(bands) if bands else None,
        "agg": agg,
    }


def _verify_body(claim: Mapping[str, Any], cell: str, mode: str) -> dict[str, Any]:
    return {"claim": dict(claim), "cell": cell, "mode": mode}


def _ask_body(
    q: str,
    place: str | None,
    cell: str | None,
    lat: float | None,
    lng: float | None,
    include_image: bool,
    verbose: bool,
) -> dict[str, Any]:
    return {
        "q": q,
        "place": place,
        "cell": cell,
        "lat": lat,
        "lng": lng,
        "include_image": include_image,
        "verbose": verbose,
    }


def _fetch_body(
    cid: str | None, cell: str | None, band: str | None, tslot: int | None
) -> dict[str, Any]:
    return {"cid": cid, "cell": cell, "band": band, "tslot": tslot}


def _backfill_body(
    cell: str,
    band: str,
    start_unix: int | None,
    end_unix: int | None,
    max_facts: int | None,
) -> dict[str, Any]:
    return {
        "cell": cell,
        "band": band,
        "start_unix": start_unix,
        "end_unix": end_unix,
        "max_facts": max_facts,
    }


def _intent_body(q: str, place: str | None, cell: str | None) -> dict[str, Any]:
    return {"q": q, "place": place, "cell": cell}


def _heat_solve_body(
    cell: str, hours_ahead: float, diffusivity_m2_per_s: float
) -> dict[str, Any]:
    return {
        "cell": cell,
        "hours_ahead": hours_ahead,
        "diffusivity_m2_per_s": diffusivity_m2_per_s,
    }


def _wave_solve_body(
    coastal_cell: str,
    offshore_height_m: float,
    period_s: float,
    n_offshore_cells: int,
) -> dict[str, Any]:
    return {
        "coastal_cell": coastal_cell,
        "offshore_height_m": offshore_height_m,
        "period_s": period_s,
        "n_offshore_cells": n_offshore_cells,
    }


def _jepa_predict_body(
    cell: str,
    band: str,
    lookback_months: int,
    forecast_horizon_months: int,
) -> dict[str, Any]:
    return {
        "cell": cell,
        "band": band,
        "lookback_months": lookback_months,
        "forecast_horizon_months": forecast_horizon_months,
    }


def _jepa_predict_v2_body(cell: str, band: str, k_history: int) -> dict[str, Any]:
    return {"cell": cell, "band": band, "k_history": k_history}


def _boring_get_params(
    lat: float | None, lng: float | None, place: str | None
) -> dict[str, Any]:
    return {"lat": lat, "lon": lng, "place": place}


def _algorithms_path(key: str | None) -> str:
    return f"/v1/algorithms/{key}" if key else "/v1/algorithms"



# ── extended endpoint body builders (v0.0.8) ─────────────────────────
def _at_body(place: str | None, cell: str | None, lat: float | None, lng: float | None, bands: Sequence[str] | None) -> dict[str, Any]:
    return {
        "place": place,
        "cell": cell,
        "lat": lat,
        "lng": lng,
        "bands": list(bands) if bands else None,
    }

def _verify_receipt_body(receipt: Mapping[str, Any], pubkey_b32: str | None) -> dict[str, Any]:
    return {
        "receipt": dict(receipt),
        "pubkey_b32": pubkey_b32,
    }

def _hunt_body(event: str, region: str | None, polygon_bbox: Sequence[float] | None, polygon_geojson: Mapping[str, Any] | None, n_cells: int | None) -> dict[str, Any]:
    return {
        "event": event,
        "region": region,
        "polygon_bbox": list(polygon_bbox) if polygon_bbox else None,
        "polygon_geojson": polygon_geojson,
        "n_cells": n_cells,
    }

def _eudr_dds_body(plots: Sequence[Mapping[str, Any]], operator: Mapping[str, Any] | None, include_evidence: bool) -> dict[str, Any]:
    return {
        "plots": [dict(p) for p in plots],
        "operator": dict(operator) if operator else None,
        "include_evidence": include_evidence,
    }

def _attest_body(fact: Mapping[str, Any], signature_b32: str, pubkey_b32: str) -> dict[str, Any]:
    return {
        "fact": dict(fact),
        "signature_b32": signature_b32,
        "pubkey_b32": pubkey_b32,
    }

def _state_body(cell: str, view: str, encoder: str | None) -> dict[str, Any]:
    return {
        "cell": cell,
        "view": view,
        "encoder": encoder,
    }

def _state_multi_body(cell: str) -> dict[str, Any]:
    return {
        "cell": cell,
    }

def _state_diff_body(cell: str, encoder: str, tslot_a: int, tslot_b: int) -> dict[str, Any]:
    return {
        "cell": cell,
        "encoder": encoder,
        "tslot_a": tslot_a,
        "tslot_b": tslot_b,
    }

def _field_boundaries_body(place: str | None, polygon_bbox: Sequence[float] | None, polygon_geojson: Mapping[str, Any] | None, max_polygons: int | None) -> dict[str, Any]:
    return {
        "place": place,
        "polygon_bbox": list(polygon_bbox) if polygon_bbox else None,
        "polygon_geojson": polygon_geojson,
        "max_polygons": max_polygons,
    }

def _temporal_route_body(q: str, place: str | None, cell: str | None) -> dict[str, Any]:
    return {
        "q": q,
        "place": place,
        "cell": cell,
    }

def _memory_token_body(cell: str, fact_cid: str) -> dict[str, Any]:
    return {
        "cell": cell,
        "fact_cid": fact_cid,
    }

def _memory_token_resolve_body(token: str) -> dict[str, Any]:
    return {
        "token": token,
    }

def _memory_contradictions_body(cell: str, band: str | None, tslot: int | None) -> dict[str, Any]:
    return {
        "cell": cell,
        "band": band,
        "tslot": tslot,
    }

def _reviews_body(subject_id: str, outcome: str, rating: int | None, notes: str | None) -> dict[str, Any]:
    return {
        "subject_id": subject_id,
        "outcome": outcome,
        "rating": rating,
        "notes": notes,
    }

def _benchmark_grade_body(answers: Mapping[str, Any]) -> dict[str, Any]:
    return {
        "answers": dict(answers),
    }


class AsyncClient:
    """Asynchronous twin of :class:`Client`. Same wire surface, awaitable.

    Every endpoint method on :class:`Client` has a coroutine sibling here
    with the same name, signature, and request body shape — guarded by
    ``tests/test_client_parity.py``.
    """

    def __init__(
        self,
        base_url: str = DEFAULT_BASE_URL,
        timeout: float = DEFAULT_TIMEOUT,
        transport: httpx.AsyncBaseTransport | None = None,
    ) -> None:
        self.base_url = base_url.rstrip("/")
        self._http = httpx.AsyncClient(
            base_url=self.base_url,
            timeout=timeout,
            headers={"user-agent": USER_AGENT, "accept": "application/json"},
            transport=transport,
        )

    async def __aenter__(self) -> "AsyncClient":
        return self

    async def __aexit__(self, *_exc: object) -> None:
        await self.aclose()

    async def aclose(self) -> None:
        await self._http.aclose()

    async def _post(self, path: str, body: Mapping[str, Any]) -> Any:
        return await self._request("POST", path, json=_strip_none(body))

    async def _get(self, path: str, params: Mapping[str, Any] | None = None) -> Any:
        return await self._request("GET", path, params=_strip_none(params or {}))

    async def _request(self, method: str, path: str, **kwargs: Any) -> Any:
        url = path if path.startswith("/") else f"/{path}"
        resp = await self._http.request(method, url, **kwargs)
        if resp.status_code >= 400:
            try:
                body: Any = resp.json()
            except ValueError:
                body = resp.text
            raise EmemHTTPError(resp.status_code, str(resp.url), body)
        if resp.headers.get("content-type", "").startswith("application/json"):
            return resp.json()
        return resp.text

    # ── Geocoder ────────────────────────────────────────────────────────
    async def locate(
        self,
        place: str | None = None,
        *,
        lat: float | None = None,
        lng: float | None = None,
    ) -> Any:
        return await self._post("/v1/locate", _locate_body(place, lat, lng))

    # ── Read primitives ────────────────────────────────────────────────
    async def recall(
        self,
        cell: str,
        bands: Sequence[str] | None = None,
        tslot: int | None = None,
    ) -> Any:
        return await self._post("/v1/recall", _recall_body(cell, bands, tslot))

    async def recall_many(
        self, cells: Sequence[str], bands: Sequence[str] | None = None
    ) -> Any:
        return await self._post("/v1/recall_many", _recall_many_body(cells, bands))

    async def recall_polygon(
        self,
        place: str | None = None,
        *,
        polygon_bbox: Sequence[float] | None = None,
        polygon_geojson: Mapping[str, Any] | None = None,
        bands: Sequence[str] | None = None,
        max_cells: int | None = None,
        cells_per_sqkm: float | None = None,
        drill_on_water: bool | None = None,
    ) -> Any:
        return await self._post(
            "/v1/recall_polygon",
            _recall_polygon_body(
                place,
                polygon_bbox,
                polygon_geojson,
                bands,
                max_cells,
                cells_per_sqkm,
                drill_on_water,
            ),
        )

    async def find_similar(
        self,
        key: str,
        k: int = 10,
        band: str = "geotessera",
        mode: str = "cosine",
    ) -> Any:
        return await self._post("/v1/find_similar", _find_similar_body(key, k, band, mode))

    async def compare(self, a: str, b: str, family: str | None = None) -> Any:
        return await self._post("/v1/compare", _compare_body(a, b, family))

    async def compare_bands(
        self,
        cell: str,
        a: str,
        b: str,
        *,
        tslot_a: int | None = None,
        tslot_b: int | None = None,
        predicate: Mapping[str, Any] | None = None,
    ) -> Any:
        return await self._post(
            "/v1/compare_bands",
            _compare_bands_body(cell, a, b, tslot_a, tslot_b, predicate),
        )

    async def trajectory(self, cell: str, band: str, window: tuple[int, int]) -> Any:
        return await self._post("/v1/trajectory", _trajectory_body(cell, band, window))

    async def diff(self, cell: str, band: str, tslot_a: int, tslot_b: int) -> Any:
        return await self._post("/v1/diff", _diff_body(cell, band, tslot_a, tslot_b))

    async def query_region(
        self,
        *,
        geometry: str | None = None,
        bbox: Sequence[float] | None = None,
        max_cells: int | None = None,
        bands: Sequence[str] | None = None,
        agg: str | None = None,
    ) -> Any:
        return await self._post(
            "/v1/query_region",
            _query_region_body(geometry, bbox, max_cells, bands, agg),
        )

    async def verify(self, claim: Mapping[str, Any], cell: str, mode: str = "fast") -> Any:
        return await self._post("/v1/verify", _verify_body(claim, cell, mode))

    async def ask(
        self,
        q: str,
        *,
        place: str | None = None,
        cell: str | None = None,
        lat: float | None = None,
        lng: float | None = None,
        include_image: bool = False,
        verbose: bool = False,
    ) -> Any:
        return await self._post(
            "/v1/ask",
            _ask_body(q, place, cell, lat, lng, include_image, verbose),
        )

    async def fetch(
        self,
        *,
        cid: str | None = None,
        cell: str | None = None,
        band: str | None = None,
        tslot: int | None = None,
    ) -> Any:
        return await self._post("/v1/fetch", _fetch_body(cid, cell, band, tslot))

    async def backfill(
        self,
        cell: str,
        band: str,
        *,
        start_unix: int | None = None,
        end_unix: int | None = None,
        max_facts: int | None = None,
    ) -> Any:
        return await self._post(
            "/v1/backfill",
            _backfill_body(cell, band, start_unix, end_unix, max_facts),
        )

    async def intent(
        self, q: str, *, place: str | None = None, cell: str | None = None
    ) -> Any:
        return await self._post("/v1/intent", _intent_body(q, place, cell))

    # ── Physics solvers ────────────────────────────────────────────────
    async def heat_solve(
        self,
        cell: str,
        *,
        hours_ahead: float = 6.0,
        diffusivity_m2_per_s: float = 1.0e-6,
    ) -> Any:
        return await self._post(
            "/v1/heat_solve", _heat_solve_body(cell, hours_ahead, diffusivity_m2_per_s)
        )

    async def wave_solve(
        self,
        coastal_cell: str,
        offshore_height_m: float,
        period_s: float,
        *,
        n_offshore_cells: int = 8,
    ) -> Any:
        return await self._post(
            "/v1/wave_solve",
            _wave_solve_body(
                coastal_cell, offshore_height_m, period_s, n_offshore_cells
            ),
        )

    async def jepa_predict(
        self,
        cell: str,
        *,
        band: str = "indices.ndvi",
        lookback_months: int = 6,
        forecast_horizon_months: int = 1,
    ) -> Any:
        return await self._post(
            "/v1/jepa_predict",
            _jepa_predict_body(cell, band, lookback_months, forecast_horizon_months),
        )

    async def jepa_predict_v2(
        self, cell: str, *, band: str = "indices.ndvi", k_history: int = 5
    ) -> Any:
        return await self._post(
            "/v1/jepa_predict_v2", _jepa_predict_v2_body(cell, band, k_history)
        )

    # ── Boring lat/lng shortcuts ───────────────────────────────────────
    async def _boring_get(
        self, path: str, *, lat: float | None, lng: float | None, place: str | None
    ) -> Any:
        return await self._get(path, _boring_get_params(lat, lng, place))

    async def ndvi(
        self,
        *,
        lat: float | None = None,
        lng: float | None = None,
        place: str | None = None,
    ) -> Any:
        return await self._boring_get("/v1/ndvi", lat=lat, lng=lng, place=place)

    async def elevation(
        self,
        *,
        lat: float | None = None,
        lng: float | None = None,
        place: str | None = None,
    ) -> Any:
        return await self._boring_get("/v1/elevation", lat=lat, lng=lng, place=place)

    async def air(
        self,
        *,
        lat: float | None = None,
        lng: float | None = None,
        place: str | None = None,
    ) -> Any:
        return await self._boring_get("/v1/air", lat=lat, lng=lng, place=place)

    async def lst(
        self,
        *,
        lat: float | None = None,
        lng: float | None = None,
        place: str | None = None,
    ) -> Any:
        return await self._boring_get("/v1/lst", lat=lat, lng=lng, place=place)

    async def soil(
        self,
        *,
        lat: float | None = None,
        lng: float | None = None,
        place: str | None = None,
    ) -> Any:
        return await self._boring_get("/v1/soil", lat=lat, lng=lng, place=place)

    async def water(
        self,
        *,
        lat: float | None = None,
        lng: float | None = None,
        place: str | None = None,
    ) -> Any:
        return await self._boring_get("/v1/water", lat=lat, lng=lng, place=place)

    async def forest(
        self,
        *,
        lat: float | None = None,
        lng: float | None = None,
        place: str | None = None,
    ) -> Any:
        return await self._boring_get("/v1/forest", lat=lat, lng=lng, place=place)

    async def weather(
        self,
        *,
        lat: float | None = None,
        lng: float | None = None,
        place: str | None = None,
    ) -> Any:
        return await self._boring_get("/v1/weather", lat=lat, lng=lng, place=place)

    # ── Introspection ─────────────────────────────────────────────────
    async def bands(self) -> Any:
        return await self._get("/v1/bands")

    async def algorithms(self, key: str | None = None) -> Any:
        return await self._get(_algorithms_path(key))

    async def sources(self) -> Any:
        return await self._get("/v1/sources")

    async def schema(self) -> Any:
        return await self._get("/v1/schema")

    async def manifests(self) -> Any:
        return await self._get("/v1/manifests")

    async def topics(self) -> Any:
        return await self._get("/v1/topics")

    async def grid_info(self) -> Any:
        return await self._get("/v1/grid_info")

    async def coverage_matrix(self) -> Any:
        return await self._get("/v1/coverage_matrix")

    async def agent_card(self) -> Any:
        return await self._get("/v1/agent_card")

    async def discover(self) -> Any:
        return await self._get("/v1/discover")

    async def openapi(self) -> Any:
        return await self._get("/openapi.json")

    async def health(self) -> Any:
        return await self._get("/health")

    # ── extended methods (v0.0.8) ────────────────────────────────────
    async def at(
        self,
        *,
        place: str | None = None,
        cell: str | None = None,
        lat: float | None = None,
        lng: float | None = None,
        bands: Sequence[str] | None = None,
    ) -> Any:
        return await self._post("/v1/at", _at_body(place, cell, lat, lng, bands))

    async def verify_receipt(
        self,
        receipt: Mapping[str, Any],
        *,
        pubkey_b32: str | None = None,
    ) -> Any:
        return await self._post("/v1/verify_receipt", _verify_receipt_body(receipt, pubkey_b32))

    async def hunt(
        self,
        event: str,
        *,
        region: str | None = None,
        polygon_bbox: Sequence[float] | None = None,
        polygon_geojson: Mapping[str, Any] | None = None,
        n_cells: int | None = None,
    ) -> Any:
        return await self._post("/v1/hunt", _hunt_body(event, region, polygon_bbox, polygon_geojson, n_cells))

    async def eudr_dds(
        self,
        plots: Sequence[Mapping[str, Any]],
        *,
        operator: Mapping[str, Any] | None = None,
        include_evidence: bool = False,
    ) -> Any:
        return await self._post("/v1/eudr_dds", _eudr_dds_body(plots, operator, include_evidence))

    async def attest(
        self,
        fact: Mapping[str, Any],
        signature_b32: str,
        pubkey_b32: str,
    ) -> Any:
        return await self._post("/v1/attest", _attest_body(fact, signature_b32, pubkey_b32))

    async def state(
        self,
        cell: str,
        *,
        view: str = 'encoder',
        encoder: str | None = None,
    ) -> Any:
        return await self._post("/v1/state", _state_body(cell, view, encoder))

    async def state_multi(
        self,
        cell: str,
    ) -> Any:
        return await self._post("/v1/state_multi", _state_multi_body(cell))

    async def state_diff(
        self,
        cell: str,
        *,
        encoder: str = 'geotessera',
        tslot_a: int,
        tslot_b: int,
    ) -> Any:
        return await self._post("/v1/state_diff", _state_diff_body(cell, encoder, tslot_a, tslot_b))

    async def field_boundaries(
        self,
        *,
        place: str | None = None,
        polygon_bbox: Sequence[float] | None = None,
        polygon_geojson: Mapping[str, Any] | None = None,
        max_polygons: int | None = None,
    ) -> Any:
        return await self._post("/v1/field_boundaries", _field_boundaries_body(place, polygon_bbox, polygon_geojson, max_polygons))

    async def temporal_route(
        self,
        q: str,
        *,
        place: str | None = None,
        cell: str | None = None,
    ) -> Any:
        return await self._post("/v1/temporal_route", _temporal_route_body(q, place, cell))

    async def memory_token(
        self,
        cell: str,
        fact_cid: str,
    ) -> Any:
        return await self._post("/v1/memory_token", _memory_token_body(cell, fact_cid))

    async def memory_token_resolve(
        self,
        token: str,
    ) -> Any:
        return await self._post("/v1/memory_token/resolve", _memory_token_resolve_body(token))

    async def memory_contradictions(
        self,
        cell: str,
        *,
        band: str | None = None,
        tslot: int | None = None,
    ) -> Any:
        return await self._post("/v1/memory_contradictions", _memory_contradictions_body(cell, band, tslot))

    async def reviews(
        self,
        subject_id: str,
        outcome: str,
        *,
        rating: int | None = None,
        notes: str | None = None,
    ) -> Any:
        return await self._post("/v1/reviews", _reviews_body(subject_id, outcome, rating, notes))

    async def benchmark_grade(
        self,
        answers: Mapping[str, Any],
    ) -> Any:
        return await self._post("/v1/benchmark/grade", _benchmark_grade_body(answers))

    async def cell_info(self, cell64: str) -> Any:
        return await self._get(f"/v1/cells/{cell64}/info")

    async def cell_geojson(self, cell64: str) -> Any:
        return await self._get(f"/v1/cells/{cell64}/geojson")

    async def cell_recall_geojson(self, cell64: str) -> Any:
        return await self._get(f"/v1/cells/{cell64}/recall_geojson")

    async def fact_by_cid(self, cid: str) -> Any:
        return await self._get(f"/v1/facts/{cid}")

    async def review_by_subject(self, subject_id: str) -> Any:
        return await self._get(f"/v1/reviews/{subject_id}")

    async def contributor_by_pubkey(self, pubkey_b32: str) -> Any:
        return await self._get(f"/v1/contributors/{pubkey_b32}")

    async def agent_quickref(self) -> Any:
        return await self._get("/v1/agent_quickref")

    async def agent_stats(self) -> Any:
        return await self._get("/v1/agent_stats")

    async def capabilities(self) -> Any:
        return await self._get("/v1/capabilities")

    async def contributors(self) -> Any:
        return await self._get("/v1/contributors")

    async def corpus_state_stats(self) -> Any:
        return await self._get("/v1/corpus_state_stats")

    async def coverage(self) -> Any:
        return await self._get("/v1/coverage")

    async def data_availability(self) -> Any:
        return await self._get("/v1/data_availability")

    async def demos(self) -> Any:
        return await self._get("/v1/demos")

    async def errors(self) -> Any:
        return await self._get("/v1/errors")

    async def fleet(self) -> Any:
        return await self._get("/v1/fleet")

    async def functions(self) -> Any:
        return await self._get("/v1/functions")

    async def materializers(self) -> Any:
        return await self._get("/v1/materializers")

    async def quickstart(self) -> Any:
        return await self._get("/v1/quickstart")

    async def tools(self) -> Any:
        return await self._get("/v1/tools")

    async def benchmark(self) -> Any:
        return await self._get("/v1/benchmark")
