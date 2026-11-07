# emem-core

`emem-core` holds the base types and the registries of the emem protocol.
It defines the cell and time slot types, the error codes and the key types.
It also loads the ten registry files in `data/` and computes the content id
of each.

You need this crate when you add a band, a source, an algorithm or an error
code. You also need it when you write Rust code that reads a registry. To
only use emem, you do not need it: `GET /v1/manifests` gives the registry ids.

| | |
|---|---|
| Version | 2.4.4 (workspace version in the root `Cargo.toml`) |
| Used by | 14 crates, for example `emem-api-rest`, `emem-codec`, `emem-fact` and `emem-fetch` |
| Dependencies | `blake3`, `ed25519-dalek`, `serde`, `serde_json`, `ciborium`, `thiserror`, `data-encoding` |
| Tests | 147 unit tests, inline in `src/` |
| Reference | [docs/registries.md](../../docs/registries.md), [docs/errors.md](../../docs/errors.md) |

## Try it

The live responder loads the same registry files. This command shows the
number of entries in six of them:

```sh
curl -s https://emem.dev/v1/manifests | jq -c '.covers | del(.note)'
# {"algorithms":172,"bands":43,"device_platforms":17,"sources":46,
#  "substrates":18,"trace_encodings":8}
```

The same response gives one `*_cid` field for each registry. A bad address
gives a code from the error catalog in this crate:

```sh
curl -s https://emem.dev/v1/cells/notacell/info
# {"code":"invalid_cell","message":"cell64 must have 4 bigrams, got 1",
#  "schema":"emem.error.v1"}
```

Both commands ran on 7 November 2026. We wrapped the output to fit the page.

To run the crate tests from a checkout (needs Rust 1.91 or later):

```sh
cargo test -p emem-core
```

We did not run `cargo test` for this page, because the host had no Rust
toolchain. The count of 147 tests comes from the `#[test]` lines in `src/`.

## The registry files

The build puts each file in `data/` into the binary with `include_str!`.
The loader parses the file at first use and checks it. If an embedded file is
not valid, the process stops with a panic.

| File | `manifest` field | Entries | Loader |
|---|---|---|---|
| `bands-v0.json` | `emem-bands` | 43 bands | `bands::DEFAULT` |
| `algorithms-v0.json` | `emem-algorithms` | 172 algorithms | `algorithms::DEFAULT` |
| `functions-v0.json` | `emem-functions` | 23 functions | `functions::DEFAULT` |
| `sources-v0.json` | `emem-sources` | 46 sources | `sources::DEFAULT` |
| `topics-v0.json` | none | 27 topics | `topics::DEFAULT` |
| `schema-v0.json` | `emem-schema` | 8 schema fragments | `schema::DEFAULT` |
| `substrates-v0.json` | `emem-substrates` | 18 substrate profiles | `substrates::DEFAULT` |
| `device-platforms-v0.json` | `emem-device-platforms` | 17 platforms | `device_platforms::DEFAULT` |
| `trace-encodings-v0.json` | `emem-trace-encodings` | 8 encodings | `trace_encodings::DEFAULT` |
| `climate_archetype_centroids_v1.json` | none | 12 climate archetypes | none in Rust code |

We counted the entries in the files on 7 November 2026. The live counts in
the "Try it" section agree.

`/v1/manifests` gives ten id fields for nine registries. `registry_cid` is
the protocol name for the functions registry id. `functions_cid` is an
older name for the same value. The climate archetype file has no id.

### How the crate makes a registry id

`manifest::manifest_cid` encodes the parsed registry as CBOR with
`ciborium`. It hashes those bytes with BLAKE3 and writes the 32-byte hash as
lower-case base32 without padding. The id comes from the parsed structure,
not from the JSON text. Thus a change to spacing in a file does not change
the id. A change to any value does.

### How to change a registry

1. Edit the JSON file in `data/`.
2. Run `cargo test -p emem-core`. The loader checks run in the tests.
3. Rebuild and restart the server. There is no setting that loads a registry
   file at run time.

A new id is a protocol change for everyone who pinned the old id. Read
[docs/registries.md](../../docs/registries.md), section "Publishing a new
manifest", before you do this.

One registry has a run-time overlay. If `EMEM_OPERATOR_ENDORSEMENTS` names
a file, `device_platforms::DEFAULT` adds the devices in it. That node then
has a different `device_platforms_cid`. If the file is not valid, the node
logs it and ignores it.

## What each module does

| Module | Contents |
|---|---|
| `cell` | `Cell`, a 64-bit cell id, and `Mode`, `Resolution`, `BaseCell` |
| `tslot` | `Tslot` (a time bucket) and `Tempo` (7 cadences, from `static` to hourly) |
| `error` | `ErrorCode`, the 27 wire-stable error codes, and `Error` |
| `key` | `AttesterKey` (32-byte Ed25519 public key), `KeyEpoch`, `Signature` (64 bytes) |
| `manifest` | the `Manifest` trait, `manifest_cid` and the `manifest` field values |
| `bands`, `algorithms`, `functions`, `sources`, `topics`, `schema` | one registry each |
| `substrates`, `device_platforms`, `trace_encodings` | the registries for device and trace evidence |
| `privacy` | `PrivacyClass`: `public`, `aggregate_only`, `l2_only_with_model_cid`, `prohibited` |
| `taxonomy` | `Lcv1`, the land-cover classes: 8 families with 8 leaves each |
| `bbox` | `Bbox`, a WGS84 box that connectors use to find upstream tiles |
| `polygon` | `Polygon`, a point-in-polygon test for cells inside a region |
| `data_dir` | `data_dir()`: `EMEM_DATA`, or `./var/emem` when that variable is absent |
| `outbound` | `user_agent()`: the `User-Agent` for upstream requests |

## Rules for changes

- An error code number does not change. MCP sends the number as a negative
  value, so `tool error (-25)` is code 25. Add a new code at the end with
  the next free number. The test `error_codes_are_pinned_to_names` checks
  this. The numbers go from 1 to 27 today.
- Use `data_dir()` for the data directory. Do not read `EMEM_DATA` directly.
  The default must stay a relative path.
- The default `User-Agent` names the project, not a person:
  `emem.dev/2.4.4 (+https://emem.dev)`. An operator sets `EMEM_CONTACT` to
  put their own contact there.
- The crate has `#![forbid(unsafe_code)]` and `#![warn(missing_docs)]`.

## What this crate does not do

- It does not encode a `cell64` text address. That is in
  [emem-codec](../emem-codec/README.md).
- The bit layout in `cell.rs` is not the layout of the live `cell64`. The
  `cell.rs` accessors read an H3-style layout: 4 mode bits at 62..59 and a
  base cell at 51..45. A geo `cell64` uses the layout in
  `crates/emem-codec/src/geo.rs`. Thus `Cell::mode()`, `resolution()` and
  `parent()` give wrong values for a geo cell. For example, `mode()` reads
  mode 2 (`DirectedEdge`) where the codec wrote mode 1. `Cell::from_raw` and
  the raw `u64` are safe to use.
- It does not compute a fact id. `emem-cache` does that, with the types in
  [emem-fact](../emem-fact/README.md).
- It does not sign anything. `key` holds the key and signature types only.
  The signing code is in [emem-storage](../emem-storage/README.md).
- It defines `PrivacyClass` and `permits_resolution`. It does not apply them
  to a request. The server decides where to apply them.

## Read more

- [docs/registries.md](../../docs/registries.md): every registry, its fields
  and its checks.
- [docs/errors.md](../../docs/errors.md): every error code and the HTTP
  status that goes with it.
- [docs/protocol.md](../../docs/protocol.md): the wire format.
- [emem-codec](../emem-codec/README.md): the `cell64` text form.
