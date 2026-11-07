# emem-primitives

`emem-primitives` holds the read primitives of the emem responder: `recall`,
`query_region`, `compare`, `compare_bands`, `diff`, `trajectory`, `verify`,
`find_similar` and `edges_recall`. It also holds the rules for memory
writes, memory bundles, entities, caller derivations and memory search.

You need this crate when you change what a read returns, or the rules that
check a memory write. To only use emem, you do not need it. The responder
serves each primitive at a `POST /v1/...` route and as an MCP tool.

| | |
|---|---|
| Version | 2.4.4 (workspace version in the root `Cargo.toml`) |
| Used by | `emem-api-rest` (all routes below). `emem-mcp` uses one request type in its tests. `emem-cli` lists the crate in `Cargo.toml` but does not use it. |
| Uses | `emem-core`, `emem-fact`, `emem-claim`, `emem-cache`, `emem-storage`, `emem-codec`, `emem-fetch` |
| Tests | 134 unit tests inline in `src/` (92 `#[test]`, 42 `#[tokio::test]`, 1 of them `#[ignore]`), 21 integration tests in `tests/` |
| Specification | [docs/protocol.md](../../docs/protocol.md) |

## Try it

`compare_bands` compares two bands at one cell. This call compares the
Copernicus DEM elevation with the GMRT topography at Mount Fuji:

```sh
curl -s -X POST https://emem.dev/v1/compare_bands \
  -H 'Content-Type: application/json' \
  -d '{"cell":"defi.zb592.nUkO.zEzE","a":"copdem30m.elevation_mean","b":"gmrt.topobathy_mean"}' \
  | jq -c '{metric, value, a: .a.fact_cid, b: .b.fact_cid}'
# {"metric":"delta","value":18.957763671875,
#  "a":"qejpz7cgvk347ordd6vrdtmwzprqlzcnkmotsjton2x4y2jjbplq",
#  "b":"zta6fvifz4hsel3yeyswznjmcpgkuw2u2fl6ph4igpegvol6v7zq"}
```

The command ran on 7 November 2026. We wrapped the output to fit the page.
For two scalar bands, `value` is `b - a` in the unit of the bands. Thus the
GMRT value is about 18.96 m higher than the DEM value in this cell. The
response also has a signed receipt that cites both fact ids.

To run the crate tests from a checkout (needs Rust 1.91 or later):

```sh
cargo test -p emem-primitives
```

We did not run `cargo test` for this page, because the host had no Rust
toolchain. The test counts come from the test attributes in `src/` and
`tests/`.

## Read primitives

Each primitive is an `async` function that takes a request struct and a
`&Server` from `emem-storage`. Each returns a response with a signed
receipt.

| Function | Route | What it returns |
|---|---|---|
| `recall` | `POST /v1/recall` | the facts at one cell, with optional band, `tslot` and as-of filters |
| `query_region` | `POST /v1/query_region` | facts over a cell, a list of cells or a box, with an optional `mean`, `median`, `p90` or `vector_centroid` |
| `compare` | `POST /v1/compare` | two cells over the bands they share: cosine for vectors, `b - a` for scalars |
| `compare_bands` | `POST /v1/compare_bands` | two bands at one cell, as a delta or a cosine |
| `diff` | `POST /v1/diff` | a derivative fact with `op = "delta"` between two time slots of one band |
| `trajectory` | `POST /v1/trajectory` | the time series of one band at one cell, inside a window |
| `verify` | `POST /v1/verify` | a verdict on a claim, with the fact ids used as evidence |
| `find_similar` | `POST /v1/find_similar` | the nearest cells by cosine over an embedding band |
| `edges_recall` | `POST /v1/edges/recall` | the signed temporal edges that start at a fact |

The primitives read only facts that storage already holds. They do not
fetch from upstream. `verify` in `Resolve` mode calls `materialize_many`,
which also does not fetch. A miss gives `MaterializeMiss`. The fetch on a `recall` miss happens in
`emem-api-rest` (`recall_with_auto_materialize`), before it calls
`recall`.

`trajectory` needs a window. A request without `window` or
`from_date` and `to_date` gets HTTP 400 `invalid_argument`.

## Memory, identity and helper modules

| Module | What it does | Route or user |
|---|---|---|
| `memory_acl` | builds the preimage that an attester signs for a memory write, and checks the Ed25519 signature and the namespace owner | memory write routes |
| `memory_bundle` | puts many `(cell, band, tslot)` triples into one signed `emem:bundle:` token | `POST /v1/memory_bundle` |
| `memory_contradictions` | finds `(cell, band, tslot)` keys where signed facts disagree, with a severity from 0 to 1 | `POST /v1/memory_contradictions` |
| `memory_search` | searches memory notes by meaning, with a BGE-base-en-v1.5 text model and a Lance index | `POST /v1/memory/search` |
| `memory_events` | the event types that a memory write publishes to stream subscribers | `emem-api-rest` event stream |
| `memory_typing` | the memory kinds and the default time to live of each kind | memory write routes |
| `memory_consolidation` | reads the settings for the background memory workers | workers in `emem-api-rest` |
| `entity` | the content-addressed ids of real-world objects, and alias claims | `POST /v1/entity` and its sub-routes |
| `derive` | the canonical body that a caller signs for `POST /v1/derive` | `POST /v1/derive` |
| `lance_index` | an on-disk Lance vector index that makes `find_similar` faster | `find_similar` |
| `binary_embedding` | packs a 128-D embedding to 128 sign bits after a fixed rotation | `emem-api-rest` |
| `cbor_ops` | reads numbers and vectors from CBOR values, and computes cosine | most primitives |
| `refinement` | a coarse-to-fine resolution schedule | no caller |

Memory search on emem.dev had 5,437 indexed files and 116,550 indexed
chunks on 7 November 2026 (`GET /v1/memory_search/stats`). The text model
loads on the first call. Without the model on disk, search returns a typed
"model not present" error.

## Settings

The main environment variables are below. The defaults come from the
code.

| Variable | Default | Effect |
|---|---|---|
| `EMEM_CONTRADICTIONS_SCAN_CAP` | 50,000 | most keys that one contradiction scan reads |
| `EMEM_CONTRADICTIONS_TIME_BUDGET_MS` | 8,000 | time limit of one scan; a stopped scan returns `resume_after` |
| `EMEM_DISABLE_LANCE` | not set | `1` turns off the Lance index; `find_similar` then compares every vector |
| `EMEM_FIND_SIMILAR_RERANK_OVERSAMPLE` | 32 (4 to 512) | how many Lance candidates per result go to the exact re-rank |
| `EMEM_MEMORY_SEARCH_MODEL_DIR` | `$EMEM_DATA/models/bge-base-en-v1.5/` | where the text model is; `EMEM_TOPIC_MODEL_DIR` is the fallback |
| `EMEM_MEMORY_SEARCH_POLL_SECS` | 60 | how often the search index looks for changed notes |
| `EMEM_MEMORY_TTL_ENABLED` | off | turns on the memory TTL worker |
| `EMEM_MEMORY_TTL_INTERVAL_SECS` | 3,600 | how often the TTL worker runs |
| `EMEM_MEMORY_CONSOLIDATION_ENABLED` | off | turns on the worker that merges old memory files into one summary file |
| `EMEM_MEMORY_CONSOLIDATION_INTERVAL_SECS` | 86,400 | how often the merge worker runs |
| `EMEM_MEMORY_CONSOLIDATION_MIN_FILES` | 50 | fewest files before the merge worker starts |
| `EMEM_MEMORY_CONSOLIDATION_MIN_AGE_DAYS` | 7 | youngest file age, in days, that the merge worker reads |
| `EMEM_REFINEMENT_ENABLED` | off | turns on the contradiction-fed refinement worker |
| `EMEM_REFINEMENT_INTERVAL_SECS` | 86,400 | how often the refinement worker runs |
| `EMEM_REFINEMENT_MIN_SEVERITY` | 0.5 | lowest severity that the refinement worker uses |
| `EMEM_REFINEMENT_MAX_PAIRS` | 1 | pairs of facts that the worker uses for each contradiction |
| `EMEM_REFINEMENT_CELL_PREFIX` | not set | limits the refinement worker to cells with this prefix |
| `EMEM_SSE_MAX_SUBS` | 256 | most memory-event stream subscribers |
| `EMEM_RECALL_EDGES_PER_FACT` | 32 | most temporal edges that `recall` adds for one fact |
| `EMEM_RECALL_EDGES_TOTAL_CAP` | 256 | most temporal edges that one `recall` adds |
| `EMEM_EDGES_DEFAULT_LIMIT` | 100 | edges that `edges_recall` returns when the call gives no `limit` |
| `EMEM_EDGES_MAX_LIMIT` | 1,000 | most edges that `edges_recall` returns |
| `EMEM_MEMORY_TTL_<KIND>_DAYS` | 30 for `EPISODIC`, 90 for `RESOURCE`, 0 for the others | days before a note of that kind expires; 0 means no expiry. `<KIND>` is `CORE`, `RESOURCE`, `EPISODIC`, `SEMANTIC`, `PROCEDURAL` or `VAULT` |

## Examples and benchmarks

Two examples run a full device write story on an ephemeral server, with no
network:

```sh
cargo run -p emem-primitives --example satellite_downlink
cargo run -p emem-primitives --example orin_stream
```

`satellite_downlink` enrols a satellite key, shows two refused writes and
then writes three facts with an admitted trace. `orin_stream` sends four
camera frames as a chain of signed traces. The frames are in
`examples/data/orin_frames/`. We did not run either example for this page
(no Rust toolchain on the host).

The benchmark `benches/hot_paths.rs` times cosine, CBOR decode and the
`find_similar` scan. [benches/BASELINE.md](benches/BASELINE.md) has the
recorded numbers and how to run it.

## Rules for changes

- Do not use `unwrap()` on a request path. Return a `StorageError`, so the
  caller gets a structured error.
- `memory_acl` and `derive` define bytes that a client signs. A change to
  them breaks every client signer. Update the signers and
  [docs/protocol.md](../../docs/protocol.md) in the same change.
- Keep the derive body field order as it is. The body is a CBOR map in
  declaration order, not in RFC 8949 sorted key order.

## What does not ship

- `refinement::schedule` has no caller. Its module comment asks for
  streamed responses from coarse to fine resolution. No route streams that
  way.
- The `geotessera` embedding band that `find_similar` reads is retired on
  emem.dev. `GET /v1/bands` gave its materializer kind as `retired` on
  7 November 2026. The index holds only the vectors that were already
  stored.

## Read more

- [docs/protocol.md](../../docs/protocol.md): the wire format of requests,
  receipts and tokens.
- [docs/agents.md](../../docs/agents.md): how an agent uses these routes.
- [crates/emem-storage/README.md](../emem-storage/README.md): the `Server`
  that each primitive reads, and the receipt signer.
- [crates/emem-mcp/README.md](../emem-mcp/README.md): the MCP tools that
  call these primitives.
- [docs/developers/architecture.md](../../docs/developers/architecture.md):
  where this crate sits in the workspace.
