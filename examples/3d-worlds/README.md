# 3D gaussian splat worlds from live emem

Four self-contained templates that turn an area of the shared memory into a
rotating 3D gaussian splat world, in a browser, with no build step. Every
splat is one cell of signed facts recalled from an emem responder, and every
gaussian parameter is a measurement with a `fact_cid` that re-checks at
[`/verify`](https://emem.dev/verify) like any other answer.

| Template | What it shows |
| --- | --- |
| [`single-band-world.html`](single-band-world.html) | one band drives height and colour (ships configured for Copernicus DEM over the Grand Canyon) |
| [`multi-band-world.html`](multi-band-world.html) | three bands fused per splat: elevation for height, Sentinel-2 NDVI for vegetation colour, JRC water recurrence for lakes (Interlaken) |
| [`semantic-world.html`](semantic-world.html) | colour from the 128-D GeoTessera foundation embedding, PCA to RGB: the memory's own notion of what kind of place each cell is (Cairo and Giza) |
| [`carbon-world.html`](carbon-world.html) | height is ESA CCI above-ground biomass, thickness its published standard error, colour the Hansen loss year (Rondônia deforestation frontier) |

## Run one

Open the file in a browser. That is the whole setup: `three.min.js` is
vendored next to the templates, and the data comes from the responder in the
config block over plain `fetch`.

```bash
python3 -m http.server -d examples/3d-worlds 8080
# then open http://localhost:8080/carbon-world.html
```

The first run over a cold area materializes and signs every sampled cell, so
it can take a few minutes; repeats are warm. The loader reports progress in
the HUD, splits batches that hit the gateway timeout, and retries.

## The gaussians are measurements

Earlier versions of these templates drew additive point sprites. They now do
standard 3D Gaussian Splatting, and every parameter of every gaussian comes
from a signed fact rather than a heuristic:

| Parameter | Source |
| --- | --- |
| centre | the fact's cell (lat/lng in `derivation.args`) and its height band |
| ground footprint | half the median nearest-neighbour spacing of the sampled grid |
| tilt | the local tangent plane: slope and aspect by finite differences over the neighbouring cells' signed height facts (the same neighbourhood computation as `/v1/terrain`), or per-cell slope/aspect bands where a responder wires them; vertical exaggeration `zx` shears space, so the tilted plane uses `theta' = atan(zx * tan(theta))` |
| thickness | the RMS residual after removing that gradient plane from the neighbours (detrended roughness), a band's own standard error (the carbon world uses the biomass SE band), or per-cell relief bands as `sigma_z = (p90 - p10) / 2.5631` (that constant is the width of the 10th-90th percentile interval in sigma of a normal) |
| opacity | the fact's attested `confidence` |

Each covariance is `Sigma = R S^2 R^T` with `R = [t1 t2 n]` the tangent
frame and `S = diag(sigma_ground, sigma_ground, sigma_z)`. The renderer
projects it with the EWA Jacobian (Zwicker et al. 2001; Kerbl et al. 2023),
`Sigma' = J W Sigma W^T J^T`, adds the 0.3 px^2 low-pass, draws an instanced
quad over the 3-sigma screen ellipse with `alpha = opacity * exp(-r^2/2)`,
and composites back-to-front with premultiplied alpha after a CPU depth
sort. Where a shape fact is missing the gaussian falls back to isotropic;
where a cell has no fact for the height band, there is no splat. Nothing is
interpolated or invented.

The construction lives in [`splat-math.js`](splat-math.js) and is pinned by
[`test/golden-scene.json`](test/golden-scene.json), a fixture with
hand-derived sigmas and quaternions that both the JS and the Python exporter
must reproduce to 1e-6:

```bash
node -e "console.log(require('./splat-math.js').selftest('./test/golden-scene.json'), 'checks')"
python3 make_splats.py --selftest
node test/render-checks.mjs     # pixel checks: gaussian profile, sort order
```

## Make it yours

Edit the `window.EMEM_WORLD` block at the top of any template:

- `responder`: any emem node, including `http://localhost:5051` from
  `docker run -p 5051:5051 ghcr.io/vortx-ai/emem:latest`.
- `bbox`: `[west, south, east, north]` in WGS-84 degrees.
- `maxCells`: up to 1,024 sampled cells (the responder picks the grid).
- `bands`: any of the 124 wired bands (`GET /v1/bands`); `heightBand` picks
  which one extrudes.
- `shape`: which signed measurements shape the gaussians —
  `gridNormals`/`gridRelief` derive tilt and thickness from the sampled
  grid's own height facts and need nothing beyond the height band;
  `sigmaBand` (a std-error band, in height-band units) or
  `reliefBands: [p10, p90]` set thickness per cell, `slopeBand` +
  `aspectBands: [sin, cos]` set tilt, where a responder wires those bands.
  Plus `sigmaFloorM`, `footprintScale`, `opacity`. Delete the block for
  isotropic splats; old configs without it still work.
- `colorize(row, color, ctx)`: your mapping from a cell's fact values to a
  colour. `row` holds one value per band; `ctx.hMin`/`ctx.hMax` give the
  height range.
- `prepare(rows, ctx)`: optional whole-scene pass before colorize —
  `semantic-world.html` runs its PCA here.

## How it fetches

`POST /v1/query_region` with the bbox returns a sampled cell list in its
receipt without touching upstream sources. The template then recalls those
cells in batches with `POST /v1/recall_many`, which materializes misses,
signs everything, and returns a receipt per cell. Positions come from each
fact's own `derivation.args` (lat/lng), heights, shapes, and colours from
the fact values. Batches are kept small (256 cells max per call, 128 when
more than two bands are in play) because a fully cold batch must finish
materializing inside the gateway timeout; a batch that still times out is
split in half and retried.

## Export signed splats

[`make_splats.py`](make_splats.py) (stdlib only) runs the same fetch and the
same gaussian math, then writes portable artifacts:

```bash
python3 make_splats.py --preset carbon --out out/rondonia --verify
```

- `out/rondonia.ply` — standard 3D Gaussian Splatting PLY: positions,
  `f_dc_* = (rgb - 0.5) / 0.28209479` (the degree-0 spherical-harmonic
  coefficient), `opacity` as a logit, `scale_*` as log sigmas, `rot_*` a
  normalized `(w, x, y, z)` quaternion. Opens in SuperSplat, gsplat,
  antimatter15/splat, PlayCanvas. Exported in the y-down COLMAP-style frame
  those viewers expect; the exact flip is recorded in the sidecar.
- `out/rondonia.splat` — antimatter15 32-byte-per-splat format, sorted by
  volume x opacity for progressive loading.
- `out/rondonia.provenance.json` — `emem.splat_provenance.v1`: the responder
  pubkey, per-splat `fact_cid`s for every band, the verbatim signed receipts,
  sha256 of both artifacts, the scene transform, and the re-check recipe.
- `out/rondonia.scene.json` — the fetched scene; serve it as
  `window.EMEM_DATA` for offline rendering, or hand it to `capture.mjs`.

`--verify` round-trips every stored receipt through `POST /v1/verify_receipt`
and fails if any signature does not check out. Any individual splat's
`fact_cid` re-checks at `/verify/<cid>` in a browser. The splat file itself
is bound to the receipts by the sha256 in the sidecar: change a gaussian and
the hash breaks; change a fact and its signature breaks.

## The data ships with the repo

[`scenes/`](scenes/) holds the exact data behind the four README worlds,
raw and processed, so nothing requires a live responder:

- `<preset>.scene.json` — the fetched scene: one record per cell with the
  signed fact values exactly as recalled. Serve it as `window.EMEM_DATA`
  (or pass it to `capture.mjs --scene`) to render offline.
- `<preset>.ply`, `<preset>.splat` — the exported gaussians; the PLY opens
  directly in SuperSplat, gsplat, or any 3DGS viewer.
- `<preset>.provenance.json` — per-splat `fact_cid`s, the verbatim signed
  receipts, and the artifact hashes; any receipt re-checks via
  `POST /v1/verify_receipt`, any fact at `/verify/<cid>`.

## Serve baked worlds

Building a world is minutes of materialize-and-sign work; serving one is a
disk read. [`scripts/bake_worlds.sh`](../../scripts/bake_worlds.sh) runs this
exporter for every preset against the local responder in gentle mode (small
batches, `--sleep` spacing so interactive traffic keeps flowing), verifies
every receipt, and atomically swaps the finished artifacts into
`EMEM_WORLDS_DIR` (default `var/worlds`);
[`scripts/stage_worlds.py`](../../scripts/stage_worlds.py) instead lays the
committed [`scenes/`](scenes/) artifacts out there, so a fresh clone serves
worlds without a single upstream fetch. From there the responder serves them
instantly:

- `GET /worlds` — the interactive viewer; it hash-checks the fetched scene
  against the provenance sidecar before drawing the first splat.
- `GET /v1/worlds` — every baked world with counts, hashes, and sizes.
- `GET /v1/worlds/<preset>/<file>` — `world.ply`, `world.splat`,
  `world.scene.json`, `world.provenance.json`, `meta.json`.

`ops/systemd/emem-worlds-bake.timer` re-bakes weekly to pick up fresh
vintages. Browsers never trigger the build themselves.

The viewer is orbit/zoom/pan draggable and self-explaining: a per-world
legend says what height, thickness, and colour mean; **click any splat** to
read that cell's measured band values, follow its `fact_cid` to `/verify`,
or copy its `memt:` token. You can recolour by any other signed band in the
scene, change the vertical exaggeration and splat size live, and drape
**real satellite imagery** (Esri World Imagery, fetched for the scene's own
bounding box) under the signed geometry as reference — labelled
reference-only, never confused with the facts. Every panel minimises (and
defaults to minimised on a narrow screen) so the render can fill the view.
The same interaction ships in
the editable templates via `window.__ememWorld` (and `EMEM_WORLD.onReady`):
`api.pick`, `api.recolor`, `api.rebuild`, `api.setBasemap`. The deterministic
capture path (`EMEM_CAPTURE`) is untouched, so the README GIFs stay
byte-for-byte reproducible.

## Capture the GIFs

The orbit GIFs in the main README come from these exact templates rendered
deterministically in headless Chromium by [`capture.mjs`](capture.mjs)
(no dependencies, Node >= 22):

```bash
python3 make_splats.py --preset carbon --out out/rondonia
node capture.mjs --template carbon-world.html --scene out/rondonia.scene.json \
     --frames 240 --size 1200x900 --gif ../../docs/media/world-rondonia.gif
```

With `--scene` no network is touched: the engine sees `window.EMEM_DATA` and
`window.EMEM_CAPTURE` and exposes `window.__renderFrame(i, total)` for
deterministic orbit frames. Without `--scene` the capture is live; the page's
requests are relayed through node's fetch, which also survives sandboxes
whose egress policies reset Chromium's own TLS. `CHROME` and `FFMPEG`
override the binary paths.
