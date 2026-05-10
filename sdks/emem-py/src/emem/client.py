"""Thin synchronous + asynchronous HTTP client for emem.dev.

Coverage map (REST → method):

  POST /v1/locate            → Client.locate
  POST /v1/recall            → Client.recall
  POST /v1/recall_many       → Client.recall_many
  POST /v1/recall_polygon    → Client.recall_polygon
  POST /v1/find_similar      → Client.find_similar
  POST /v1/compare           → Client.compare
  POST /v1/compare_bands     → Client.compare_bands
  POST /v1/trajectory        → Client.trajectory
  POST /v1/diff              → Client.diff
  POST /v1/query_region      → Client.query_region
  POST /v1/verify            → Client.verify
  POST /v1/ask               → Client.ask
  POST /v1/fetch             → Client.fetch
  POST /v1/backfill          → Client.backfill
  POST /v1/intent            → Client.intent
  POST /v1/heat_solve        → Client.heat_solve
  POST /v1/wave_solve        → Client.wave_solve
  POST /v1/jepa_predict      → Client.jepa_predict
  POST /v1/jepa_predict_v2   → Client.jepa_predict_v2
  GET  /v1/bands             → Client.bands
  GET  /v1/algorithms        → Client.algorithms
  GET  /v1/sources           → Client.sources
  GET  /v1/schema            → Client.schema
  GET  /v1/manifests         → Client.manifests
  GET  /v1/topics            → Client.topics
  GET  /v1/grid_info         → Client.grid_info
  GET  /v1/coverage_matrix   → Client.coverage_matrix
  GET  /v1/agent_card        → Client.agent_card
  GET  /v1/discover          → Client.discover
  GET  /openapi.json         → Client.openapi
  GET  /health               → Client.health

Boring lat/lng shortcuts (skip locate→recall):

  Client.ndvi / elevation / air / lst / soil / water / forest / weather

All calls return parsed JSON. HTTP non-2xx responses raise EmemHTTPError
with the response status, URL, and parsed body (when JSON).
"""

from __future__ import annotations

import os
from typing import Any, Iterable, Mapping, Sequence

import httpx

DEFAULT_BASE_URL = os.environ.get("EMEM_BASE_URL", "https://emem.dev")
DEFAULT_TIMEOUT = float(os.environ.get("EMEM_TIMEOUT_SECS", "180"))
USER_AGENT = "emem-py/0.0.4 (+https://emem.dev)"


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
        return self._post("/v1/locate", {"place": place, "lat": lat, "lng": lng})

    # ── Read primitives ────────────────────────────────────────────────
    def recall(
        self,
        cell: str,
        bands: Sequence[str] | None = None,
        tslot: int | None = None,
    ) -> Any:
        return self._post("/v1/recall", {"cell": cell, "bands": list(bands) if bands else None, "tslot": tslot})

    def recall_many(self, cells: Sequence[str], bands: Sequence[str] | None = None) -> Any:
        return self._post("/v1/recall_many", {"cells": list(cells), "bands": list(bands) if bands else None})

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
            {
                "place": place,
                "polygon_bbox": list(polygon_bbox) if polygon_bbox else None,
                "polygon_geojson": polygon_geojson,
                "bands": list(bands) if bands else None,
                "max_cells": max_cells,
                "cells_per_sqkm": cells_per_sqkm,
                "drill_on_water": drill_on_water,
            },
        )

    def find_similar(
        self,
        key: str,
        k: int = 10,
        band: str = "geotessera",
        mode: str = "cosine",
    ) -> Any:
        return self._post("/v1/find_similar", {"key": key, "k": k, "band": band, "mode": mode})

    def compare(self, a: str, b: str, family: str | None = None) -> Any:
        return self._post("/v1/compare", {"a": a, "b": b, "family": family})

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
            {
                "cell": cell,
                "a": a,
                "b": b,
                "tslot_a": tslot_a,
                "tslot_b": tslot_b,
                "predicate": predicate,
            },
        )

    def trajectory(self, cell: str, band: str, window: tuple[int, int]) -> Any:
        return self._post("/v1/trajectory", {"cell": cell, "band": band, "window": list(window)})

    def diff(self, cell: str, band: str, tslot_a: int, tslot_b: int) -> Any:
        return self._post("/v1/diff", {"cell": cell, "band": band, "tslot_a": tslot_a, "tslot_b": tslot_b})

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
            {
                "geometry": geometry,
                "bbox": list(bbox) if bbox else None,
                "max_cells": max_cells,
                "bands": list(bands) if bands else None,
                "agg": agg,
            },
        )

    def verify(self, claim: Mapping[str, Any], cell: str, mode: str = "fast") -> Any:
        return self._post("/v1/verify", {"claim": dict(claim), "cell": cell, "mode": mode})

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
            {
                "q": q,
                "place": place,
                "cell": cell,
                "lat": lat,
                "lng": lng,
                "include_image": include_image,
                "verbose": verbose,
            },
        )

    def fetch(
        self,
        *,
        cid: str | None = None,
        cell: str | None = None,
        band: str | None = None,
        tslot: int | None = None,
    ) -> Any:
        return self._post("/v1/fetch", {"cid": cid, "cell": cell, "band": band, "tslot": tslot})

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
            {
                "cell": cell,
                "band": band,
                "start_unix": start_unix,
                "end_unix": end_unix,
                "max_facts": max_facts,
            },
        )

    def intent(self, q: str, *, place: str | None = None, cell: str | None = None) -> Any:
        return self._post("/v1/intent", {"q": q, "place": place, "cell": cell})

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
            {"cell": cell, "hours_ahead": hours_ahead, "diffusivity_m2_per_s": diffusivity_m2_per_s},
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
            {
                "coastal_cell": coastal_cell,
                "offshore_height_m": offshore_height_m,
                "period_s": period_s,
                "n_offshore_cells": n_offshore_cells,
            },
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
            {
                "cell": cell,
                "band": band,
                "lookback_months": lookback_months,
                "forecast_horizon_months": forecast_horizon_months,
            },
        )

    def jepa_predict_v2(self, cell: str, *, band: str = "indices.ndvi", k_history: int = 5) -> Any:
        return self._post("/v1/jepa_predict_v2", {"cell": cell, "band": band, "k_history": k_history})

    # ── Boring lat/lng shortcuts ───────────────────────────────────────
    def _boring_get(self, path: str, *, lat: float | None, lng: float | None, place: str | None) -> Any:
        return self._get(path, {"lat": lat, "lon": lng, "place": place})

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
        return self._get(f"/v1/algorithms/{key}" if key else "/v1/algorithms")

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


class AsyncClient:
    """Asynchronous twin of :class:`Client`. Same surface, awaitable."""

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

    async def locate(
        self,
        place: str | None = None,
        *,
        lat: float | None = None,
        lng: float | None = None,
    ) -> Any:
        return await self._post("/v1/locate", {"place": place, "lat": lat, "lng": lng})

    async def recall(
        self,
        cell: str,
        bands: Sequence[str] | None = None,
        tslot: int | None = None,
    ) -> Any:
        return await self._post(
            "/v1/recall",
            {"cell": cell, "bands": list(bands) if bands else None, "tslot": tslot},
        )

    async def find_similar(
        self,
        key: str,
        k: int = 10,
        band: str = "geotessera",
        mode: str = "cosine",
    ) -> Any:
        return await self._post(
            "/v1/find_similar", {"key": key, "k": k, "band": band, "mode": mode}
        )

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
            {
                "q": q,
                "place": place,
                "cell": cell,
                "lat": lat,
                "lng": lng,
                "include_image": include_image,
                "verbose": verbose,
            },
        )

    async def health(self) -> Any:
        return await self._get("/health")
