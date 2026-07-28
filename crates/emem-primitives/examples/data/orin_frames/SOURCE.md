# Where these frames come from

Four real Sentinel-2 true-colour crops, one per cell, fetched from the
public responder's scene route on 2026-07-28:

| file | cell64 | route |
|---|---|---|
| `frame_0.png` | `defi.zb55e.natI.tUpu` | `/v1/cells/defi.zb55e.natI.tUpu/scene.png` |
| `frame_1.png` | `defi.zb55e.natI.vada` | `/v1/cells/defi.zb55e.natI.vada/scene.png` |
| `frame_2.png` | `defi.zb55e.rElA.tUpu` | `/v1/cells/defi.zb55e.rElA.tUpu/scene.png` |
| `frame_3.png` | `defi.zb55e.rElA.vada` | `/v1/cells/defi.zb55e.rElA.vada/scene.png` |

The cells sit in the Nile Delta around 30.79 N, 31.00 E, the same ground
the `orin_stream` example writes NDVI readouts for. They stand in for
the camera captures an Orin NX would produce; the example binds each
file's blake3 digest inside a signed OS execution trace and never puts
the pixels themselves into a fact.

To stream your own captures instead, point `EMEM_FRAMES_DIR` at a
directory of image files (any format; files are read as bytes, sorted
by name) and run the example unchanged:

```bash
EMEM_FRAMES_DIR=/path/to/your/frames \
  cargo run -p emem-primitives --example orin_stream
```

Attribution: contains modified Copernicus Sentinel data (2026),
processed by ESA, served as cell crops by emem.dev.
