# 3D worlds from live emem

Two self-contained templates that turn any area of the shared memory into a
rotating 3D gaussian splat world, in a browser, with no build step. Every
splat is one signed fact recalled from an emem responder; its `fact_cid`
re-checks at [`/verify`](https://emem.dev/verify) like any other answer.

| Template | What it shows |
| --- | --- |
| [`single-band-world.html`](single-band-world.html) | one band drives height and colour (ships configured for Copernicus DEM over the Grand Canyon) |
| [`multi-band-world.html`](multi-band-world.html) | three bands fused per splat: elevation for height, Sentinel-2 NDVI for vegetation colour, JRC water recurrence for lakes (ships configured for Interlaken) |

## Run one

Open the file in a browser. That is the whole setup: `three.min.js` is
vendored next to the templates, and the data comes from the responder in the
config block over plain `fetch`.

```bash
python3 -m http.server -d examples/3d-worlds 8080
# then open http://localhost:8080/single-band-world.html
```

The first run over a cold area materializes and signs every sampled cell, so
it can take a few minutes; repeats are warm. The loader reports progress in
the HUD.

## Make it yours

Edit the `window.EMEM_WORLD` block at the top of either template:

- `responder`: any emem node, including `http://localhost:5051` from
  `docker run -p 5051:5051 ghcr.io/vortx-ai/emem:latest`.
- `bbox`: `[west, south, east, north]` in WGS-84 degrees.
- `maxCells`: up to 1,024 sampled cells (the responder picks the grid).
- `bands`: any of the 124 wired bands (`GET /v1/bands`); `heightBand` picks
  which one extrudes.
- `colorize(row, color, ctx)`: your mapping from a cell's fact values to a
  colour. `row` holds one value per band; `ctx.hMin`/`ctx.hMax` give the
  height range.

## How it fetches

`POST /v1/query_region` with the bbox returns a sampled cell list in its
receipt without touching upstream sources. The template then recalls those
cells in batches with `POST /v1/recall_many`, which materializes misses,
signs everything, and returns a receipt per batch. Positions come from each
fact's own `derivation.args` (lat/lng), heights and colours from the fact
values. Nothing in the scene is interpolated or invented: if a cell has no
fact for a band, it simply has no splat.

Batches are kept small (256 cells max per call; the loader uses less) because
a fully cold batch must finish materializing inside the gateway timeout.

## Headless capture

The GIFs in the main README come from these exact templates rendered in
headless Chromium: set `window.EMEM_CAPTURE = {}` plus `window.EMEM_DATA` to
a pre-fetched scene, and the engine exposes `window.__renderFrame(i, total)`
for deterministic orbit frames instead of the free-running animation loop.
