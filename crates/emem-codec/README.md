# emem-codec

`emem-codec` turns emem addresses into short text and back. It gives a place
its `cell64`, a time slot its `t.` form and a fact id its short `cid64` form.
It also defines the byte layout of a field grid.

You need this crate when you change how emem names a place or a time. You
also need it to decode a `cell64` in your own code, without a server call. To only use
emem, you do not need it: `POST /v1/locate` returns the `cell64` for you.

| | |
|---|---|
| Version | 2.4.4 (workspace version in the root `Cargo.toml`) |
| Used by | `emem-api-rest`, `emem-cli`, `emem-fact`, `emem-fetch`, `emem-primitives`, `emem-storage` |
| Tests | 30 unit tests, inline in `src/` |
| Specification | [docs/protocol.md](../../docs/protocol.md), sections 1, 2.4 and 3.1 |

## Try it

The live responder runs this codec. Send a latitude and longitude to get the
`cell64`, then decode that `cell64` back to its square:

```sh
curl -s -X POST https://emem.dev/v1/locate \
  -H 'Content-Type: application/json' \
  -d '{"lat":0,"lng":0}' | jq -r .cell64
# defi.zb400.bafi.baba

curl -s https://emem.dev/v1/cells/defi.zb400.bafi.baba/info \
  | jq -c '{centre, approx_size_m}'
# {"centre":{"lat_deg":0.00004291536470191204,"lng_deg":0.000042915354470096645},
#  "approx_size_m":{"lat":9.554676797235869,"lng":9.55467451922018}}
```

Both commands ran on 7 November 2026. We wrapped the output of the second
command to fit the page.

To run the crate tests from a checkout (needs Rust 1.91 or later):

```sh
cargo test -p emem-codec
```

We did not run `cargo test` for this page, because the host had no Rust
toolchain. The count of 30 tests comes from the `#[test]` lines in `src/`.

## What each module does

| Module | Converts | Text form |
|---|---|---|
| `cell64` | a 64-bit cell id to four symbols | `defi.zb592.nUkO.zEzE` |
| `geo` | latitude and longitude to a cell id, and back | the same `cell64` |
| `tslot_text` | a time slot (`u64`) to text | `t.` + base32 of a LEB128 varint, for example `t.qaea` for 1024 |
| `cid64` | the first 8 bytes of a 32-byte fact id | 13 lower-case base32 characters |
| `vec64` | a vector to a short id: BLAKE3 over the fp16 bytes, first 12 bytes | base32 |
| `grid` | a field grid to bytes: a 64-byte header, then `f32` values, little-endian | binary, hashed into an `artifact_cid` |
| `alphabet` | a symbol number (0 to 65,535) to a symbol, and back | `defi`, `zb592` |
| `hilbert` | Hilbert curve helpers, used by tests | none |

A `cid64` is a prefix. It is not unique enough to identify a fact. Canonical
CBOR always uses the full 32-byte fact id.

## How a cell64 holds a place

A `cell64` holds one 64-bit cell id as four 16-bit numbers. Each number maps
to one symbol of the alphabet. Dots separate the four symbols.

```text
bits 63..60  mode        1 (geo)
bits 59..52  resolution  21
bits 51..44  base        0xab
bit  43      reserved    0
bits 42..22  lat_q       21 bits
bits 21..0   lng_q       22 bits
```

The latitude axis has 21 bits and the longitude axis has 22 bits. Thus a cell
is about 9.55 m on each side at the equator. Away from the equator, a cell
becomes narrower by the cosine of the latitude. At Mount Fuji (35.36 degrees
north), `/v1/cells/defi.zb592.nUkO.zEzE/info` gave a width of 7.79 m on
7 November 2026.

The alphabet has 65,536 symbols. The first 44,100 are four letters in the
order consonant, vowel, consonant, vowel (21 × 10 × 21 × 10). The other
21,436 are `z` and four hex digits, for example `zb592`. The code in
`src/alphabet.rs` builds the alphabet on first use. No data file
holds it.

Nearby cells often share their first symbols, because the latitude bits come
before the longitude bits. Do not use shared symbols to find neighbours. For
the exact neighbours of a cell, read `neighborhood_cells` from
`POST /v1/locate`.

The decoder refuses an id that is not a geo cell at resolution 21. An older
grid of about 305 m used a different resolution tag. Its strings fail with
`NotGeoCell`, so a fact cannot move to a wrong place without an error.

## Rules for changes

- Do not edit `src/alphabet.rs`. A change gives a new `cell64` to every cell
  in the corpus. The root [AGENTS.md](../../AGENTS.md) lists it as off-limits
  without a registry CID rotation plan.
- Keep `grid` output byte-identical. Two responders that compute the same
  field must write the same bytes, or the same field gets two `artifact_cid`
  values. The layout is in [docs/plans/field-tokens.md](../../docs/plans/field-tokens.md),
  section 1.
- Every cell size in the workspace must come from `CELL_PITCH_M_EQUATOR` in
  `src/geo.rs`. Do not write a new constant for it.

## What does not ship

The cell is a square in latitude and longitude, not an equal-area cell. A
comment in `src/geo.rs` names the H3 hexagon grid as a later migration. No
H3 code ships in this crate. To weight cells by area across latitudes, use
`equal_area_weight_for`.

## Read more

- [docs/protocol.md](../../docs/protocol.md), section 1: the full cell64
  encode and decode rules, with a worked example.
- [docs/protocol.md](../../docs/protocol.md), sections 2 and 3: time slots,
  fact ids and `cid64`.
- [docs/glossary.md](../../docs/glossary.md): the terms cell, `cell64` and
  tslot.
- [docs/developers/architecture.md](../../docs/developers/architecture.md):
  where this crate sits in the workspace.
