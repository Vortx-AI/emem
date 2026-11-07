# emem-api-rest

`emem-api-rest` is the HTTP surface of the emem responder. It builds one
axum `Router`. The router serves the REST API under `/v1/` and MCP JSON-RPC
at `/mcp` and `/mcp/full`. It also serves the OpenAPI document, the
`.well-known` files and the web pages in `web/`. It also holds most of the
inline materializers, which fetch, compute and sign a fact when a recall
misses.

You need this crate when you add or change a route, an MCP tool handler, a
materializer or a limit of the server. To only call emem, you do not need
it: use `https://emem.dev` or the `emem-server` binary from `emem-cli`.

| | |
|---|---|
| Version | 2.4.4 (workspace version in the root `Cargo.toml`) |
| Used by | `emem-cli`, which builds the `emem-server` binary |
| Uses | 13 emem crates (for example `emem-storage`, `emem-primitives`, `emem-mcp`, `emem-guard`, `emem-fetch`), `axum`, `tower`, `tower-http`, `tokio`, `ort`, `tokenizers`, `model2vec-rs` |
| Public API | `router(state: AppState) -> Router`, with `AppState = Arc<emem_storage::Server>`. Two modules are public: `enlistment` and `topic_router`. |
| Source | `src/lib.rs` (107,763 lines) and 24 other modules, 133,967 lines in total |
| Tests | 738 unit tests inline in `src/` (579 `#[test]`, 159 `#[tokio::test]`, 3 `#[ignore]`) |

All counts on this page come from the source on 7 November 2026.

## Try it

The live responder at `https://emem.dev` runs this crate. These commands
need `curl` and `jq`. They ran on 7 November 2026.

Check that the server is up and see its version and key:

```sh
curl -s https://emem.dev/health | jq -c '{ok, name, version, responder_pubkey_b32}'
# {"ok":true,"name":"emem","version":"2.4.4",
#  "responder_pubkey_b32":"777er3yihgifqmv5hmc2wwmyszgddzderzhsx6rex4yoakwomvka"}
```

See which commit built the running binary:

```sh
curl -s https://emem.dev/.well-known/emem.json \
  | jq -c '.operator_attestation | {git_commit, build_timestamp}'
# {"git_commit":"c11aef779bc79e6648a2cdb9fa846e1b550f8ded",
#  "build_timestamp":"2026-11-06T21:26:31Z"}
```

`build.rs` puts these two values into the binary at build time. If the
build has no git checkout and no `EMEM_GIT_COMMIT` variable, the commit
is `unknown`.

Count the paths in the OpenAPI document:

```sh
curl -s https://emem.dev/openapi.json | jq '.paths | length'
# 194
```

A path that the router does not know gives a typed error, not an empty
body:

```sh
curl -s https://emem.dev/v1/no_such_route | jq -c '{schema, code}'
# {"schema":"emem.error.v1","code":"not_found"}
```

We wrapped some output lines to fit the page.

To run the crate tests from a checkout (needs Rust 1.91 or later):

```sh
cargo test -p emem-api-rest
```

The `ort` dependency uses its `download-binaries` feature, so the first
build downloads ONNX Runtime. We did not run `cargo test` for this page,
because the host had no Rust toolchain. The test count comes from the
test attributes in the source.

## How `router` builds the server

`emem-server` (in `emem-cli`) opens the store, loads the identity and
builds an `emem_storage::Server`. Then it calls `emem_api_rest::router`
with that server in an `Arc`. The `router` function does these steps:

1. It starts background tasks, for example the transparency log head,
   the memory ledger cache and the GeoNames index.
2. It registers the routes. The source has 434 `.route(` calls for 420
   distinct paths. 188 of the paths start with `/v1/`.
3. It adds the shared layers to all routes.
4. It merges three sub-routers that need a longer timeout: `/splats`,
   `/v1/perception/*path`, and the EUDR router with `/v1/eudr_dds`,
   `/v1/field_burn_scar` and `/v1/field_actual_et`.

The live OpenAPI document lists 194 paths. The router has more paths,
because web pages, `.well-known` files and aliases are not in the
OpenAPI document.

### Shared layers and their defaults

| Layer | Default | Variable |
|---|---|---|
| Requests in flight on the main router. Request number 129 gets a fast 503. The three sub-routers do not get this limit. | 128 | `EMEM_MAX_INFLIGHT` (8 to 4096) |
| Gateway timeout. A slow request gets a typed 504. | 40 s | `EMEM_TIMEOUT_SECS` (1 to 600) |
| Rate limit per client IP (token bucket) | 10 requests per second, burst 120 | `EMEM_RATE_LIMIT_RPS`, `EMEM_RATE_LIMIT_BURST` |
| Request body limit | 16 MB | `EMEM_BODY_LIMIT_MB` (1 to 256) |
| Timeout of the EUDR and field-signal routes | 610 s | `EMEM_EUDR_TIMEOUT_SECS` plus 10 s |
| Requests in flight on the EUDR and field-signal routes | 3 | `EMEM_EUDR_MAX_INFLIGHT` (1 to 64) |
| Timeout of `/splats` | 180 s | `EMEM_SPLATS_TIMEOUT_S` (40 to 600) |
| Timeout of `/v1/perception/*path` | 120 s | `EMEM_PERCEPTION_TIMEOUT_S` (40 to 600) |

The other shared layers add security headers and CORS, catch a panic as a
typed 500, write the access log and compress responses with gzip. The
concurrency limit does not apply to `/health`, `/v1/health`,
`/a2a/tasks/health`, `/live`, `/ping` and `/metrics`, so a probe always gets
an answer.

The source reads 167 distinct `EMEM_*` variables. The operator guide
describes the ones an operator sets. See
[docs/operators/operating.md](../../docs/operators/operating.md).

## Where to find things in `src/`

| You want to change | Look here |
|---|---|
| Route registrations and shared layers | `router` in `lib.rs` |
| MCP JSON-RPC (`/mcp`, `/mcp/full`, `tools/call`) | `mcp_jsonrpc_inner` in `lib.rs`. `tools/call` finds the handler with a `match` on the tool name. The tool descriptors are in `emem-mcp`. |
| The OpenAPI document | `openapi_spec` in `lib.rs`. A person writes each path entry by hand. `/openapi.action.json` is a filtered copy for ChatGPT actions. |
| Recall with fetch on a miss | `recall_with_auto_materialize` and `post_recall` in `lib.rs` |
| Inline materializers | `lib.rs`, search `^async fn materialize_` (47 functions) |
| `/v1/ask` classifiers (hunter mode, corpus audit) | `ask_foundation.rs` |
| `/v1/ask` topic routing | `topic_router.rs` |
| Physics solvers (heat, wave, NDVI forecast) | `physics.rs` |
| Field rasters and index series as signed derivations | `band_raster.rs`, `field_series.rs` |
| Who may write and at which tier | `enlistment.rs` |
| Encrypted `kind=vault` memory | `vault.rs` |
| `emem:tree` audit paths into a pointer note | `tree.rs` |
| `POST /v1/read`, `POST /v1/ocr`, document parsers | `reader.rs`, `doc_parse.rs`, `eu_mrl.rs` |
| `POST /v1/range_hash` | `range_hash.rs` |
| Build provenance (`EMEM_GIT_COMMIT`, `EMEM_BUILD_TIMESTAMP`) | `build.rs` |

The other modules hold one algorithm or one route each, for example
`terrain.rs`, `rain_series.rs` and `next_acquisition.rs`. The first lines
of each file say what it does.

To add a primitive with a route and an MCP tool, follow
[Add a primitive](../../docs/developers/developing.md#add-a-primitive)
in the developer guide. That guide calls the OpenAPI builder
`openapi_json()`. In the code its name is `openapi_spec`.

## The topic router for `/v1/ask`

`/v1/ask` sends a question to one or more topics. The backend comes from
`EMEM_TOPIC_BACKEND`:

| Value | What it does |
|---|---|
| `ort` (default) | Runs a BERT-type ONNX model with `ort`. The model directory comes from `EMEM_TOPIC_MODEL_DIR`. When only `EMEM_DATA` is set, the directory is `<EMEM_DATA>/models/bge-base-en-v1.5`. If neither variable is set, `ort` cannot load a model. |
| `model2vec` | Uses the static `minishlab/potion-base-8M` embedder. Pure Rust, no ONNX Runtime. |
| `keyword` | Uses keyword tables only. Loads no model. |

When `ort` fails to load, the router tries `model2vec`. When `model2vec`
also fails, the router uses `keyword`. Each fallback writes a warning to
the log.

On 7 November 2026, `GET /v1/agent_card` on emem.dev gave
`manifests.topic_router_backend` = `ort`. To put the model in place
before the first request, use `scripts/install-topic-model.sh`.

## What this crate does not do

- It does not open the store or load the key. `emem-server` does that and
  passes the result to `router`.
- It does not sign facts itself. It calls `emem_storage::Server` to
  sign them. See
  [crates/emem-storage/README.md](../emem-storage/README.md).
- It does not hold the MCP tool catalog. That is in
  [crates/emem-mcp](../emem-mcp/README.md).
- It runs no GPU extension. `GET /v1/capabilities` always returns an
  empty `extensions` list.

## Read more

- [crates/emem-cli/README.md](../emem-cli/README.md): how to build and
  start `emem-server`
- [docs/developers/architecture.md](../../docs/developers/architecture.md):
  how the crates fit together
- [docs/developers/developing.md](../../docs/developers/developing.md):
  test commands and how to add a primitive
- [docs/operators/operating.md](../../docs/operators/operating.md):
  environment variables, TLS and CSP
- [docs/protocol.md](../../docs/protocol.md): the wire format and the
  receipt preimage
