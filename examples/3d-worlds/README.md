# 3D gaussian splat worlds from live emem

This directory turns an area of emem memory into a 3D gaussian splat world
that turns in a browser. There is no build step. Each splat is one cell of
signed facts recalled from an emem responder. Each gaussian parameter comes
from a fact with a `fact_cid`, and you can check that fact at
[`/verify`](https://emem.dev/verify).

## What you need and what you get

| You need | You get |
| --- | --- |
| A browser and Python 3 (standard library only) | Four browser templates that draw a world from live recalls |
| Network access to `https://emem.dev`, or your own node | `make_splats.py`, which writes `.ply`, `.splat`, a scene file and a provenance file |
| Node 22 or later, Chromium and ffmpeg, only for `capture.mjs` and the JS tests | Committed scene data in [`scenes/`](scenes/), so you can render with no network |

On 7 November 2026 we ran every Python command on this page against
emem.dev. We also ran the JS selftest with Node 24.19.0. Chromium is not on
our test host, so the commands that need it did not run. This page marks each
of them.

Each command block on this page starts with the same `cd` line. It works
from any directory in your clone, and you can run it again with no error.

These worlds are *sparse*: one splat for each signed cell, with no
interpolation. The *dense* counterpart is the hosted demo at
[emem.dev/splats](https://emem.dev/splats). It labels each splat `measured`,
`interpolated` or `synthesized`.

| Template | What it shows |
| --- | --- |
| [`single-band-world.html`](single-band-world.html) | One band sets height and colour. The default is Copernicus DEM elevation over the Grand Canyon. |
| [`multi-band-world.html`](multi-band-world.html) | Three bands for each splat over Interlaken: elevation for height, Sentinel-2 NDVI for vegetation colour, JRC water recurrence for lakes. |
| [`semantic-world.html`](semantic-world.html) | Colour from the 128-D GeoTessera embedding, reduced to RGB by PCA, over Cairo and Giza. See the note below: emem.dev has retired this band. |
| [`carbon-world.html`](carbon-world.html) | Height is ESA CCI above-ground biomass, thickness is its standard error, colour is the Hansen loss year (Rondonia). |

The semantic world reads the `geotessera` band. emem.dev has retired that
band. On 7 November 2026 a live `recall_many` for `geotessera` returned
no facts. A live semantic world therefore gets elevation but no colour data.
Use the committed `scenes/semantic.scene.json` to see the world as baked.

## Run a template

Serve the directory and open a template:

```bash
cd "$(git rev-parse --show-toplevel)/examples/3d-worlds"
python3 -m http.server 8080
# then open http://localhost:8080/carbon-world.html
```

`three.min.js` is in this directory. The data comes from the responder in
the config block, through `fetch`.

A cold area must materialise and sign each sampled cell, so the first run
can take a few minutes. A warm area is fast: on 7 November 2026 a full
1,024-cell Grand Canyon fetch took 4.7 s. The loader shows progress in the
HUD. It splits a batch that times out at the gateway and tries again.

## How measurements set each gaussian

The templates use standard 3D Gaussian Splatting. Each parameter comes from
signed facts or from the geometry of the sampled grid:

| Parameter | Source |
| --- | --- |
| centre | The fact's cell (lat and lng in `derivation.args`) and its height band. |
| ground footprint | Half the median nearest-neighbour spacing of the sampled grid. |
| tilt | The local tangent plane. Slope and aspect come from finite differences over the neighbour cells' height facts, as in `POST /v1/terrain`. A responder that wires slope and aspect bands can set them per cell. Vertical exaggeration `zx` shears space, so the tilt is `theta' = atan(zx * tan(theta))`. |
| thickness | One of three sources. The first is the RMS residual of the neighbours when you remove the gradient plane. The second is a band's own standard error; the carbon world uses the biomass SE band. The third is relief bands, as `sigma_z = (p90 - p10) / 2.5631`. The constant is the width of the 10th to 90th percentile interval of a normal, in sigma. |
| opacity | The fact's attested `confidence`. |

Each covariance is `Sigma = R S^2 R^T`, with `R = [t1 t2 n]` the tangent
frame and `S = diag(sigma_ground, sigma_ground, sigma_z)`. The renderer
projects it with the EWA Jacobian (Zwicker et al. 2001; Kerbl et al. 2023):
`Sigma' = J W Sigma W^T J^T`. It adds a 0.3 px^2 low-pass filter. It draws
an instanced quad over the 3-sigma screen ellipse with
`alpha = opacity * exp(-r^2/2)`. It sorts by depth on the CPU and composites
back to front with premultiplied alpha.

If a shape fact is missing, the gaussian is isotropic. If a cell has no fact
for the height band, it gets no splat. The renderer does not interpolate or
invent values.

The math is in [`splat-math.js`](splat-math.js). The fixture
[`test/golden-scene.json`](test/golden-scene.json) holds hand-derived sigmas
and quaternions. The JS and the Python exporter must both match it to 1e-6:

```bash
cd "$(git rev-parse --show-toplevel)/examples/3d-worlds"
python3 make_splats.py --selftest
# selftest ok: 1213 checks
node -e "console.log(require('./splat-math.js').selftest('./test/golden-scene.json'), 'checks')"
# 1217 checks
node test/render-checks.mjs     # pixel checks for gaussian profile and sort order; not run: needs Chromium
```

## Change a template

Edit the `window.EMEM_WORLD` block at the top of a template:

| Key | What it does |
| --- | --- |
| `responder` | The emem node to read from. For a local node, run `docker run -p 127.0.0.1:5051:5051 ghcr.io/vortx-ai/emem:latest` and use `http://localhost:5051`. Docker is not on our test host, so we did not run this command. |
| `bbox` | `[west, south, east, north]` in WGS-84 degrees. |
| `maxCells` | The number of cells to sample. The responder picks the grid and caps it at 1,024. |
| `bands` | The bands to recall. `heightBand` picks the band that sets height. |
| `shape` | The measurements that shape the gaussians (see the next list). Delete the block for isotropic splats. Old configs without it still work. |
| `colorize(row, color, ctx)` | Your mapping from a cell's values to a colour. `row` holds one value per band. `ctx.hMin` and `ctx.hMax` give the height range. |
| `prepare(rows, ctx)` | Optional. A pass over the whole scene before `colorize`. `semantic-world.html` runs its PCA here. |

The `shape` block takes these keys:

- `gridNormals` and `gridRelief` get tilt and thickness from the grid's own
  height facts. They need only the height band.
- `sigmaBand` sets thickness per cell from a standard-error band, in
  height-band units.
- `reliefBands: [p10, p90]` sets thickness per cell from two percentile
  bands.
- `slopeBand` and `aspectBands: [sin, cos]` set tilt, if the responder wires
  those bands.
- `sigmaFloorM`, `footprintScale` and `opacity` tune the result.

On 7 November 2026 emem.dev auto-fetched 115 band names on a recall miss.
This count is `band_taxonomy.materializer_wired` in `GET /v1/agent_card`.
The list is at `GET /v1/materializers`. `GET /v1/bands` lists the 43 cube
slots, which is a different count.

## How a template fetches data

1. `POST /v1/query_region` with the bbox returns the sampled cell list in
   `receipt.cells`. It does not fetch from upstream sources.
2. `POST /v1/recall_many` recalls those cells in batches, one band at a
   time. It materialises misses, signs the facts and returns a receipt for
   each cell under `by_cell`.
3. Positions come from each fact's `derivation.args` (lat, lng). Heights,
   shapes and colours come from the fact values.

A batch is 256 cells, or 128 when more than two bands are in use. A cold
batch must finish inside the gateway timeout. When a batch times out, the
loader splits it in half and sends it again.

## Export signed splats

[`make_splats.py`](make_splats.py) uses only the Python standard library. It
does the same fetch and the same gaussian math, then writes files you can
share. The presets are `canyon`, `interlaken`, `semantic` and `carbon`.

```bash
cd "$(git rev-parse --show-toplevel)/examples/3d-worlds"
python3 make_splats.py --preset carbon --out out/rondonia
# fetched 1024 cells, 2878 signed facts
# wrote out/rondonia (.ply 40.7 KB, .splat 18.9 KB, 606 splats)
```

On 7 November 2026 at 09:50 UTC this run took 9 s. Only 606 of the 1,024
cells returned a biomass fact, so the world had 606 splats. This count can
go up between runs. An earlier run on the same day gave 498.

The command writes four files:

| File | Contents |
| --- | --- |
| `out/rondonia.ply` | A standard 3D Gaussian Splatting PLY. It holds positions, `f_dc_* = (rgb - 0.5) / 0.28209479` (the degree-0 spherical-harmonic coefficient), `opacity` as a logit, `scale_*` as log sigmas and `rot_*` as a normalized `(w, x, y, z)` quaternion. It opens in SuperSplat, gsplat, antimatter15/splat and PlayCanvas. It uses the y-down COLMAP frame those viewers expect. The sidecar records the axis flip. |
| `out/rondonia.splat` | The antimatter15 format, 32 bytes per splat, sorted by volume times opacity for progressive loading. |
| `out/rondonia.provenance.json` | Schema `emem.splat_provenance.v1`. It holds the responder public key and the `fact_cid` of each band for each splat. It also holds the signed receipts as received, the sha256 of each artifact, the scene transform and a re-check recipe. |
| `out/rondonia.scene.json` | The fetched scene. Serve it as `window.EMEM_DATA` to render offline, or give it to `capture.mjs`. |

`--verify` sends each stored receipt to `POST /v1/verify_receipt`. The run
fails if any signature does not check. `--verify` makes one call for each
receipt, so use it with care on a full world: carbon has more than 4,000
receipts. For a small test, add `--max-cells 16`:

```bash
cd "$(git rev-parse --show-toplevel)/examples/3d-worlds"
python3 make_splats.py --preset canyon --max-cells 16 --out /tmp/canyon16 --verify
# wrote /tmp/canyon16 (.ply 1.5 KB, .splat 0.5 KB, 16 splats)
# verify_receipt: 17 valid, 0 invalid of 17
```

There is one receipt for each cell and band that the responder answers, plus
one for the `query_region` call. A cell with no value for a band still gets a
receipt. Canyon reads one band, so 16 cells give 16 + 1 = 17. Carbon reads
four bands over 1,024 cells. On 7 November 2026 the live carbon run above
held 4,097 receipts (4 x 1,024 + 1). The committed
`scenes/carbon.provenance.json` holds 4,057, because 40 cell and band pairs
came back with no receipt at bake time.
`/v1/verify_receipt` is the responder checking its own signature. For an
independent check, verify offline (see
[docs/tutorials/first-verified-memory.md](../../docs/tutorials/first-verified-memory.md)).
You can open any splat's `fact_cid` at `/verify/<cid>` in a browser.

The sha256 values in the sidecar bind the splat files to the receipts. If
you change a gaussian, its file hash no longer matches. If you change a
fact, its signature no longer checks.

## Densify and keep provenance

A sampled world has one gaussian per cell, on the grid. `--densify F` splits
each grid quad into `F x F` and writes a gaussian at each fine node. With
`--densify 3`, a 32 x 32 grid becomes 94 x 94, so 1,024 cells give 8,836
splats. The terrain then looks continuous, and each extra splat stays
checkable:

```bash
cd "$(git rev-parse --show-toplevel)/examples/3d-worlds"
python3 make_splats.py --preset canyon --densify 3 --out out/canyon_dense
# densified 3x: 1024 measured + 7812 derived = 8836 splats
# derived re-derivation: 7812 band-checks, 0 mismatched
```

The sidecar uses schema `emem.splat_provenance.v2`, which labels each splat:

- A node on an original cell has `kind: "measured"`. It points to that
  cell's own `fact_cid`.
- Any other node has `kind: "derived"`. It records up to four source cells
  and a bilinear `weight` for each. The weights sum to 1.

The sidecar stores the measured `cids` and `values` once, in a `cells`
table. You can re-derive each continuous band of a derived splat:

```
value[band] == sum_i weight_i * cells[source_i].vals[band]
```

Each source `fact_cid` still checks against the responder's key. A derived
splat gets its tilt from the gradient of the same bilinear patch.

Two rules keep the derived values true:

- The exporter does not blend a categorical band. Examples are a Hansen
  loss year or a class code. List these bands in `categoricalBands`. A
  derived node takes the value of its nearest signed corner.
  `--check-derived` accepts `value[band] == cells[nearest].vals[band]` for
  those bands.
- A node on an original cell is that signed cell, with its exact value. The
  exporter writes it even at a ragged grid edge where the quad is
  incomplete. So densifying never drops or relabels a measured fact.

Check a v2 sidecar offline. You need only the file:

```bash
cd "$(git rev-parse --show-toplevel)/examples/3d-worlds"
python3 make_splats.py --check-derived out/canyon_dense.provenance.json
# emem.splat_provenance.v2: 7812 derived splats, 7812 band-checks, 0 mismatched
```

The live `/worlds` viewer densifies with the same math. `splat-math.js` has
the same bilinear and nearest-corner code, and the golden fixture pins both
to 1e-6. Its pick panel shows each splat's measured `fact_cid` or its signed
source cells.

## Data in the repository

[`scenes/`](scenes/) holds the data behind the four worlds, so you can
render them with no live responder. On 7 November 2026 it had these files
for each preset:

- `<preset>.scene.json`: the fetched scene, one record per cell, with the
  signed fact values as recalled. Serve it as `window.EMEM_DATA`, or pass it
  to `capture.mjs --scene`.
- `<preset>.ply` and `<preset>.splat`: the exported gaussians.
- `<preset>.provenance.json`: schema v1, with each splat's `fact_cid`s, the
  signed receipts and the artifact hashes.

The committed receipts still verify. On 7 November 2026 we sent three
receipts from `scenes/carbon.provenance.json` to `POST /v1/verify_receipt`,
and all three returned `valid: true`. The committed carbon world has 848
splats from 1,024 sampled cells, because the other cells have no biomass
fact. A live run on 7 November 2026 gave 606 splats (see
[Export signed splats](#export-signed-splats)). The committed files stay as
baked.

## Serve baked worlds

A cold build takes minutes of materialise and sign work. A baked world is a
disk read.

- [`scripts/bake_worlds.sh`](../../scripts/bake_worlds.sh) runs this
  exporter for each preset against the local responder. It uses small
  batches with `--sleep` between them, so the responder can still serve
  other traffic. It verifies each receipt. It then swaps the finished files
  into `EMEM_WORLDS_DIR` (default `var/worlds`) in one step.
- [`scripts/stage_worlds.py`](../../scripts/stage_worlds.py) copies the
  committed `scenes/` files there instead. A fresh clone can then serve
  worlds with no upstream fetch.

The responder then serves these routes:

| Route | Returns |
| --- | --- |
| `GET /worlds` | The viewer. It checks the scene's sha256 against the provenance sidecar before it draws. |
| `GET /v1/worlds` | Each baked world with its counts, hashes and sizes. |
| `GET /v1/worlds/<preset>/<file>` | `world.ply`, `world.splat`, `world.scene.json`, `world.provenance.json` or `meta.json`. |

`ops/systemd/emem-worlds-bake.timer` starts a re-bake each Sunday at 03:30
UTC. On 7 November 2026, `GET /v1/worlds` on emem.dev listed four worlds,
all with `baked_at` on 4 July 2026. So do not expect the hosted worlds to be
newer than that date. A browser never starts a build.

In the viewer you can turn, zoom and pan. A legend for each world says what
height, thickness and colour mean. Click a splat to see its measured band
values, open its `fact_cid` at `/verify`, or copy its `emem:fact:` token.
You can also:

- recolour by another signed band in the scene,
- change the vertical exaggeration and splat size,
- show Esri World Imagery for the scene's bbox under the splats. The viewer
  labels it as reference only. It is not a fact.

You can collapse each panel. Panels start collapsed on a narrow screen.

The templates have the same controls through `window.__ememWorld` (and
`EMEM_WORLD.onReady`): `api.pick`, `api.recolor`, `api.rebuild` and
`api.setBasemap`. The capture path (`EMEM_CAPTURE`) does not use them, so
captured GIFs stay byte-for-byte reproducible.

## Capture the GIFs

The world GIFs in `docs/media/` come from these templates.
[`capture.mjs`](capture.mjs) renders them in headless Chromium with no other
dependencies. It needs Node 22 or later. These commands did not run on our
test host, because it has no Chromium:

```bash
cd "$(git rev-parse --show-toplevel)/examples/3d-worlds"
python3 make_splats.py --preset carbon --out out/rondonia
node capture.mjs --template carbon-world.html --scene out/rondonia.scene.json \
     --frames 240 --size 1200x900 --gif ../../docs/media/world-rondonia.gif
```

With `--scene`, the capture uses no network. The engine reads
`window.EMEM_DATA` and `window.EMEM_CAPTURE`. It exposes
`window.__renderFrame(i, total)` to draw fixed orbit frames. Without
`--scene`, the capture is live. Node's `fetch` relays the page's requests.
This works in sandboxes whose egress policy resets Chromium's own TLS. Set
`CHROME` and `FFMPEG` to change the binary paths.
