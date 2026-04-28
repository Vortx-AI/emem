# emem.dev Product Memory

## Product Promise

emem.dev is the world intelligence layer for people and AI agents. Every point on Earth snaps to a 3-meter memory cell. Every cell carries a three-word human address, a stable cell id, and a `geotessera1792` intelligence contract: a 1792-dimension embedding of real earth-observation signal.

- Humans see a memorable address, bounded square, nearest place, and a shareable map link.
- Agents see the same context plus the 1792D intelligence contract — 32 named bands across foundation embeddings, optical/radar sensors, terrain, climate, soil, vegetation, land cover, water, human presence, and native vision features — with explicit per-band provenance: `live`, `procedural`, `deferred`, or `unavailable`.
- The native copilot speaks in intents and tool-calls (`resolve_cell`, `fetch_bands`, `similar_cells`, `explain_confidence`). Streaming SSE when the client asks, JSON when it doesn't.

## 2026 Architecture

**Frontend** — React + TypeScript + Vite, MapLibre GL, resizable spatial workstation. New: provider ribbon (shows whether the current session is running procedural or agri-remote intelligence), band matrix (14 grouped families × live/procedural/deferred status), per-family drill-down.

**Backend** — Express. Routes through a `VectorProvider` for every address-producing endpoint, so the API response always carries the 1792D intelligence contract:

- `GET /api/health` → provider status
- `GET /api/bands` → full 1792D band registry (ontology)
- `GET /api/convert-to-3wa` → TesseraAddress with `intelligence`
- `GET /api/convert-to-coordinates` → TesseraAddress with `intelligence`
- `POST /api/agent/resolve` → JSON by default; SSE stream of tool events when `Accept: text/event-stream`

**Intelligence data plane** — a split:

- `ProceduralProvider` (default, local) — honest placeholder. Populates the 128D foundation block with deterministic pseudo-values. Every other band is marked `deferred` with a pointer to the real source. Reserved bands are `unavailable`.
- `AgriRemoteProvider` (activated by `AGRI_CUBE_URL`) — calls the user's GPU server at `POST {url}/cell` with `{cellId, lat, lng}`. Expects `{ coverage: BandCoverage[], checksum, capturedAt, vector128? }`. Falls back to procedural on network failure; the failure reason is surfaced on the provider ribbon.

This is the seam where the agri research stack plugs in: a tile_server.py / FAISS / projector pipeline running on the user's GPU server just needs to speak the `/cell` contract, and emem lights up with real bands without any other change.

**Band registry** — `src/lib/bands.ts`. Mirrors `integrate_10m.py:BAND_OFFSETS` from Vortx-AI/agri: 32 bands, 1792 dimensions, grouped into 14 families, each tagged with tempo (`static` · `slow` · `medium` · `fast` · `ultra_fast`) and native spatial resolution. The registry self-validates at load.

## Agent Contract

Every cell renders as:

```
emem://<words>
cellId: <bigint>
intelligence: {
  model, dimensions: 1792, provider, capturedAt,
  liveDims, proceduralDims, deferredDims, unavailableDims,
  coverage: [{ key, family, tempo, source, status, sampleValues?, summary? }]
}
geotessera128: { ... }   // legacy compact embedding, first-128 slice
```

Agents receive tool-call events:

1. `intent` — router picks `decode_words` / `encode_coordinates` / `place_search` / `intelligence_report` / `band_inspect` / `guidance`.
2. `tool_call` → `tool_result` — `resolve_cell`, `fetch_bands`, `similar_cells`, `explain_confidence`.
3. `token` chunks of the natural-language narration.
4. `final` — the full `AgentResponse`.

Un-populated tools surface honest notes: `similar_cells` returns empty when the remote is off, with a note explaining why.

## Principles

- **Honest provenance over silent placeholder.** A real band is never faked. If it's not live, it's labeled.
- **The contract is the product.** Humans click cells; agents read the contract. Both see the same truth.
- **Latent-first, not chat-first.** The copilot is a thin reasoner over structured spatial state, not a wall of text.
- **Downstream-swappable data plane.** Procedural vs agri-remote is an env flip, not a rewrite.

## Legal / IP Guardrail

Do not copy what3words branding, API payloads, word assignments, or algorithms. emem.dev is functionally inspired by the category but original in brand, lexicon, UI, and implementation. Agri-sourced bands retain their upstream licensing (AlphaEarth, Sentinel, SRTM, Hansen, WDPA, etc.); emem exposes derived signal, not redistributable raster tiles.
