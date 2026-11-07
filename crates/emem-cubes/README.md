# emem-cubes

`emem-cubes` is a small handle type for AgriSynth cube files. An AgriSynth
cube is a `.npz` file with one 1,792-value vector per pixel. The crate does
not read the file contents. It checks that the path exists and estimates
the pixel count from the file size.

No code in the workspace calls this crate on 7 November 2026. The responder
does not use it, and no endpoint serves cube data. You need this page only
if you plan to remove the crate or to give it a real parser.

| | |
|---|---|
| Version | 2.4.4 (workspace version in the root `Cargo.toml`) |
| Size | `src/lib.rs` (54 lines) |
| Listed as a dependency by | `emem-cli` (`Cargo.toml`), but no `emem-cli` source file uses it |
| Tests | None |
| I/O | One `std::fs::metadata` call in `Cube::open` |

## Check that nothing calls it

This command searches every Rust file outside the crate for its name:

```sh
grep -rn "emem_cubes" crates --include=*.rs | grep -v "^crates/emem-cubes/"
# (no output)
```

It ran on 7 November 2026 in a checkout of this branch. It needs no key
and no network.

## What the crate holds

| Item | What it does |
|---|---|
| `Cube` | A struct with `n_pixels`, `dims` and `path`. |
| `Cube::open(path)` | Reads the file size. Sets `dims` to 1792. Sets `n_pixels` to the file size divided by 3,584 (2 bytes times 1,792 values). Returns `CubeError::Io` when the path does not exist. |
| `CubeError::Io` | Wraps a `std::io::Error`. |
| `CubeError::BadNpz` | Declared, but no code makes it. |

## Limits of the pixel count

The value of `n_pixels` is an estimate. It is correct only for an
uncompressed float16 array with no header. A real `.npz` file is a ZIP
archive with headers, and its entries can use compression. The module comment says
that v1 cubes use float32. For those files the estimate is two times too
large.

## Where the text and the code do not agree

| Text | Code on 7 November 2026 |
|---|---|
| `Cargo.toml` description: "lance index reader" | There is no Lance code in the crate. |
| Doc comment on `Cube`: "backed by an mmap'd `.npz`" | `Cube::open` reads metadata only. It maps no memory. |
| Module comment: the parser lives in `tools/load_cube.py` | `tools/` has no such file, and `git log --all` shows no commit that added it. |
| `Cargo.toml` lists `emem-core` as a dependency | `src/lib.rs` does not use `emem-core`. |

These are code issues. This page reports them and does not change the code.

## Read more

- [docs/developers/architecture.md](../../docs/developers/architecture.md)
  lists every crate with a one-line role.
- [docs/developers/developing.md](../../docs/developers/developing.md)
  gives the crate table and the test rules.
