# Where these frames come from

The four PNG files here are real Sentinel-2 true-colour crops, one per cell.
They come from the scene route of the public responder (fetch date 2026-07-28).

| file | cell64 | route |
|---|---|---|
| `frame_0.png` | `defi.zb55e.natI.tUpu` | `/v1/cells/defi.zb55e.natI.tUpu/scene.png` |
| `frame_1.png` | `defi.zb55e.natI.vada` | `/v1/cells/defi.zb55e.natI.vada/scene.png` |
| `frame_2.png` | `defi.zb55e.rElA.tUpu` | `/v1/cells/defi.zb55e.rElA.tUpu/scene.png` |
| `frame_3.png` | `defi.zb55e.rElA.vada` | `/v1/cells/defi.zb55e.rElA.vada/scene.png` |

On 7 November 2026, `GET https://emem.dev/v1/cells/defi.zb55e.natI.tUpu/scene.png` returned a PNG (HTTP 200).
That PNG was 192,621 bytes, and `frame_0.png` is 194,030 bytes.
The route serves the current scene, so a new fetch does not give the committed bytes again.

The cells are in the Nile Delta, near 30.79 N, 31.00 E.
The `orin_stream` example writes NDVI readouts for the same ground.
The frames stand in for the camera captures of an Orin NX.
The example binds the BLAKE3 digest of each file into a signed OS execution trace.
It never puts the pixels into a fact.

## Use your own frames

Set `EMEM_FRAMES_DIR` to a directory of image files, then run the example without changes.
The example reads each file as bytes, in name order, so any image format works.
It skips `.md` files and files that have no extension.
We did not run this command for this page, because this host has no Rust toolchain (7 November 2026).

```bash
EMEM_FRAMES_DIR=/path/to/your/frames \
  cargo run -p emem-primitives --example orin_stream
```

## Attribution

Contains modified Copernicus Sentinel data (2026), processed by ESA, served as cell crops by emem.dev.
