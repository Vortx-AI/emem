# Develop emem

This page is for engineers who change the code in
`github.com/Vortx-AI/emem`. It covers setup, the daily workflows and the
rules that the repository enforces. For the system design, read
[architecture.md](architecture.md). For the wire bytes, read
[protocol.md](../protocol.md). For the contribution rules, read
[CONTRIBUTING.md](https://github.com/Vortx-AI/emem/blob/main/CONTRIBUTING.md).

The numbers on this page come from the code and from `https://emem.dev`
on 7 November 2026. The `curl` commands ran against `https://emem.dev`
on that date. The `cargo` commands are the ones that CI runs on every
pull request to `main` (`.github/workflows/ci.yml`).

## Prepare a build host

| Item | Value |
|---|---|
| Rust | 1.91 or later (`rust-version` in `Cargo.toml`). CI also runs clippy and tests on current stable. |
| Edition | 2021 |
| Workspace version | 2.4.4 |
| Build tools | `g++`, `protoc` and the protobuf well-known types (`protobuf-compiler` and `libprotobuf-dev` on Debian), `mdbook` 0.5.2 |
| glibc | 2.38 or later. `ort-sys` links symbols that glibc 2.36 does not have. The Docker image builds on `debian:trixie-slim` (glibc 2.41). |
| Operating system | Linux or macOS. CI runs the tests on `ubuntu-latest` and `macos-latest`. |
| Shell tools for the examples | `curl` and `jq` |

Build the docs site before you build the code. The server embeds
`docs/book/` with `include_dir!`, so `cargo build` fails when that
directory does not exist. `docs/book/` is in `.gitignore`.

```sh
git clone https://github.com/Vortx-AI/emem
cd emem
mdbook build docs && rm -f docs/book/book.toml
cargo build --workspace
cargo run --release --bin emem-server   # listens on 0.0.0.0:5051
```

The `rm` step removes the mdbook build configuration from the output, so
that the binary does not serve it. The Dockerfile and CI do the same step.

The default port is 5051. Do not change it to 5050: on the maintainers'
host, another service uses port 5050.

## Know the workspace

The workspace has 19 member crates. A twentieth directory,
`crates/zstd-0.13-compat`, is a patch shim and not a member.

| Crate | Role |
|---|---|
| `emem-core` | Cells, tslots, taxonomy, registry loaders (bands, functions, sources, topics), signing keys, error catalog |
| `emem-codec` | Text codecs: cell64, vec64, cid64, tslot text |
| `emem-fact` | Fact types, canonical CBOR, content addressing |
| `emem-claim` | Claim algebra (band, operator, value, tslot) and its evaluator |
| `emem-cache` | Fact cache tiers |
| `emem-fetch` | Source connectors that fetch upstream data on demand |
| `emem-storage` | Cache, fetch, materializer and attestation log together; lazy materialization |
| `emem-cubes` | Loaders for the AgriSynth 1792-dimension cubes |
| `emem-primitives` | Read primitives: recall, query_region, compare, find_similar, verify, diff, trajectory |
| `emem-attest` | Attestation envelope, receipt preimage, Merkle batching |
| `emem-trace` | OS execution trace schema and its verification engine |
| `emem-intent` | Intent grammar and a rule-based planner |
| `emem-mcp` | MCP tool registry |
| `emem-api-rest` | HTTP router: REST, MCP JSON-RPC, OpenAPI, and the inline materializers |
| `emem-cli` | The `emem` CLI, `emem-server` and the demo and admin binaries |
| `emem-guard` | Signed verdict server for AI inference hooks |
| `emem-airgap` | Air-gapped node: a folder in, a folder out, no network |
| `emem-scorecard` | MemoryAgentBench-style scorecard for a local responder |
| `emem-sleep-agent` | Opt-in agent that merges contradicted memory into new signed memory |

Most of the code is in two places. `crates/emem-api-rest/src/lib.rs` is
one file of about 108,000 lines (7 November 2026). It holds the router
and every inline materializer. `crates/emem-fetch/src/` has 36 modules:
one for each upstream source, plus helpers such as `cog.rs`, `stac.rs`
and `proj.rs`. Most changes touch `emem-api-rest` and one of
`emem-fetch`, `emem-primitives` or `emem-core`.

Outside the workspace:

| Path | Contents |
|---|---|
| `web/` | Static HTML pages, embedded with `include_str!` |
| `sdks/` | `emem-py`, `emem-ts`, `emem-langmem`, `llama-index-tools-emem` |
| `examples/` | MCP host configurations and framework examples (LangChain, LlamaIndex, CrewAI, AutoGen, and others) |
| `ops/systemd/` | Example units, the watchdog and a journald retention drop-in |
| `Dockerfile` | Multi-stage build, runtime on `debian:trixie-slim` |
| `huggingface-space/` | A Docker wrapper around the GHCR image |

## Pick a binary

`crates/emem-cli` builds ten binaries.

| Binary | Use |
|---|---|
| `emem` | Inspect registries (`manifests`, `bands`, `functions`, `sources`, `errors`), make a key (`keygen`), decode or encode a cell (`cell`, `cell-encode`), and verify a receipt offline (`verify`) |
| `emem-server` | The HTTP and MCP server. One port serves both. |
| `emem-demo` | Short round trip: signs 3 facts with a temporary key, posts them to `/v1/attest`, then runs the read primitives |
| `emem-livedemo` | Runs every primitive and writes each request, response and a `trace.json` index to a directory |
| `emem-realdemo` | Reads Copernicus DEM 30 m tiles from public S3 and runs the read primitives on them |
| `emem-connect-demo` | Runs the "connect and evolve" loop from `examples/connect-and-evolve.md` |
| `emem-demo-device` | Enrols this host as a labelled demo device and publishes a trace |
| `emem-ask-eval` | Checks how `/v1/ask` routes a fixed set of questions |
| `emem-purge-fnkey` | Admin: removes cached facts by `derivation.fn_key` from `cache.sled/` only. It has no effect on the default redb store; use it with `EMEM_HOT_BACKEND=sled`. Stop the server first, because sled holds an exclusive lock. |
| `emem-sled-slim` | Admin: copies the live sled trees into a new store. Stop the server first. |

Other crates build `emem-guard`, `emem-airgap`, `emem-encode`,
`emem-scorecard` and `emem-sleep-agentd`.

`emem-demo`, `emem-livedemo`, `emem-realdemo` and `emem-connect-demo`
sign facts with a new key and write them to the server. Run them against
your own node, not against `https://emem.dev`. A default node refuses
facts from a key it does not know (`fact_plane_closed`). For the demos,
start a test node with `EMEM_FACT_PLANE_OPEN=1`:

```sh
EMEM_DATA=:memory: EMEM_FACT_PLANE_OPEN=1 cargo run --release --bin emem-server
```

Do not set `EMEM_FACT_PLANE_OPEN=1` on a node that other people use.

## Run a local server

```sh
EMEM_DATA=:memory: cargo run --release --bin emem-server
```

`EMEM_DATA=:memory:` keeps no files and makes a new responder key at each
start. For work that must survive a restart, such as replaying receipts,
use a real directory. The server then keeps its Ed25519 key in
`<EMEM_DATA>/identity.secret.b32` (mode 0600). The default directory is
`./var/emem`.

To run the published image instead of a local build, and to connect a
client to the node, see [Self-host](../self-host.md).

## Call each primitive

This sequence calls the main read primitives in the order that an
integration uses them. Set `BASE` to your node. The responses below are
from `https://emem.dev` on 7 November 2026.

```sh
BASE=http://127.0.0.1:5051
H='content-type: application/json'

# 1. What the responder is.
curl -sS $BASE/health | jq -c '{version}'          # {"version":"2.4.4"}
curl -sS $BASE/v1/manifests | jq -c 'keys'          # 10 registry CIDs and "covers"
curl -sS $BASE/v1/grid_info | jq -c '.active_encoding.encoded_string_form'

# 2. Find a place, then read its facts.
CELL=$(curl -sS -X POST $BASE/v1/locate -H "$H" \
  -d '{"place":"Bengaluru"}' | jq -r '.cell64')   # defi.zb493.xuqA.zcb5f
curl -sS -X POST $BASE/v1/recall -H "$H" \
  -d "{\"cell\":\"$CELL\",\"band\":\"copdem30m.elevation_mean\"}" > /tmp/recall.json
jq -c '{facts: (.facts|length), bands: (.bands_already_attested_at_cell|length)}' /tmp/recall.json
# {"facts":1,"bands":68}

# 3. Compare two cells.
curl -sS -X POST $BASE/v1/compare -H "$H" \
  -d "{\"a\":\"$CELL\",\"b\":\"defi.zb592.nemu.zEvE\"}" | jq -c 'keys'
# ["asymmetry_note","cosine","only_a","only_b","per_band","receipt","shared_bands"]

# 4. A time series for one band.
curl -sS -X POST $BASE/v1/trajectory -H "$H" \
  -d "{\"cell\":\"$CELL\",\"band\":\"indices.ndvi\",\"from_date\":\"2024-01-01\",\"to_date\":\"2024-12-31\"}" \
  | jq -c '.series|length'                          # 4

# 5. Check the receipt from step 2.
jq '{receipt}' /tmp/recall.json | curl -sS -X POST $BASE/v1/verify_receipt \
  -H "$H" -d @- | jq -c '{valid, signature_valid, preimage_version}'
```

Step 5 returned `{"valid":true,"signature_valid":true,"preimage_version":2}`.
`/v1/verify_receipt` takes the receipt inside a `receipt` field. `/v1/compare`
takes the cells as `a` and `b`. `/v1/trajectory` needs a window:
`from_date` and `to_date`, or `window: [a, b]` in Unix seconds.

An empty answer must say why it is empty. `recall` returns
`bands_already_attested_at_cell`, so a caller can tell a wrong query from
an empty place. `compare_bands` returns `bands_with_no_history` for a
band that has no signed fact at that cell (reason `EmptyHistory`).

## Verify a receipt offline

Save the receipt object to a file, then run the `emem` CLI:

```sh
jq '.receipt' /tmp/recall.json > /tmp/receipt.json
cargo run --release --bin emem -- verify /tmp/receipt.json \
  --base-url http://127.0.0.1:5051
```

`emem verify` rebuilds the BLAKE3 preimage with the same code as
`POST /v1/verify_receipt`. It picks the public key in this order:
`--pubkey`, then the key at `--base-url` (or `EMEM_BASE_URL`) in
`/.well-known/emem.json`, then the responder key in the receipt.

The browser has a third path. `web/verify.html`, served at `/verify`,
loads `/emem-verify-core.js` from the same responder. That script
rebuilds the preimage and checks the Ed25519 signature in the tab.

## Add a band

1. Choose the band key. A dotted key, such as `copdem30m.elevation_mean`,
   needs no change to `crates/emem-core/data/bands-v0.json`. Only a new
   cube slot changes that file. Each slot has `key`, `offset`, `dims`,
   `tempo`, `privacy` and descriptive fields. The `dims` of all slots must
   add up to 1792. Each `offset` must equal the `offset` plus `dims` of
   the slot before it, so a gap also fails.
2. If you changed `bands-v0.json`, run `cargo test -p emem-core bands`.
   The registry tests find overlaps, gaps and a wrong total.
3. If the band comes from other bands, add a recipe to
   `crates/emem-core/data/functions-v0.json`.
4. Connect a materializer. Inline materializers are in
   `crates/emem-api-rest/src/lib.rs` (search for `async fn materialize_`).
   A larger connector gets its own module in `crates/emem-fetch/src/`.
5. Add the band key to `all_materializable_bands` in the same file. If
   the key is not there, `/v1/bands`, `/v1/materializers` and
   `/v1/data_availability` do not show the band.
6. Add the band to `band_materializer_meta` in the same file, so that
   `/v1/data_availability` reports the true history window and tempo.
   A null history bound is correct for static climatology and for
   nowcast-only sources.
7. Run `cargo test -p emem-api-rest`.

## Add a connector

1. Make a file `crates/emem-fetch/src/<source>.rs`.
2. Declare the module in `crates/emem-fetch/src/lib.rs`.
3. For COG or GeoTIFF data, use the range-read helpers in `cog.rs`. For
   STAC catalogues, use `stac.rs`. The per-upstream timeout is 14 seconds;
   `EMEM_MATERIALIZER_TIMEOUT_SECS` changes it.
4. Add the source to `crates/emem-core/data/sources-v0.json`, so that it
   shows in `/v1/sources` and in `sources_cid`. Give the scheme's tempo
   and native resolution there, and the licence of each provider.
5. Add a row for the module to the connector table in
   [data-sources.md](data-sources.md).

[data-sources.md](data-sources.md#add-a-connector) gives all ten steps,
with the band, materializer and function registry entries.

## Add a primitive

1. Make a file `crates/emem-primitives/src/<name>.rs` with a request
   type, a response type and an async function.
2. Export it from `crates/emem-primitives/src/lib.rs`.
3. Add the route in `crates/emem-api-rest/src/lib.rs` and the tool in
   `crates/emem-mcp/src/lib.rs`.
4. Add the path to the OpenAPI document (`openapi_spec()` in
   `crates/emem-api-rest/src/lib.rs`).
5. Sign the receipt with
   `srv.sign_receipt(primitive, cells, fact_cids, was_cached, started, intent)`
   (`crates/emem-storage/src/server.rs`).
6. Write a test that proves an empty answer says why it is empty.
   For example, `recall` returns `bands_already_attested_at_cell` and
   `materialize_notes`, and `compare_bands` returns
   `bands_with_no_history`. A caller can then tell a wrong query from
   an empty place.

## Run the tests and lints

| Goal | Command |
|---|---|
| Unit, binary and integration tests, no network | `cargo test --workspace --lib --bins --tests` |
| Live network tests | `cargo test -p emem-fetch --test live_cog_fetch` |
| Format | `cargo fmt --all` |
| Format, CI gate | `cargo fmt --all --check` |
| Lint, CI gate | `cargo clippy --workspace --all-targets -- -D warnings` |

There is no `live` cargo feature. The live tests are in
`crates/emem-fetch/tests/live_cog_fetch.rs`. They skip when you set
`EMEM_NO_NETWORK=1`. Run them after you change a connector or the
source registry.

Unit tests are in `src/`, next to the code (`#[cfg(test)] mod tests`).
Five crates also have tests in `tests/`: `emem-airgap`, `emem-fact`,
`emem-fetch`, `emem-primitives` and `emem-trace`.

CI (`.github/workflows/ci.yml`) runs on pushes to `main` and on pull
requests to `main`. It runs these jobs:

| Job | What it runs |
|---|---|
| `fmt` | `cargo fmt --all --check` |
| `clippy` | `cargo clippy --workspace --all-targets -- -D warnings` on current stable |
| `test` | `cargo test --workspace --release` on Linux and macOS, with the network blocked and `EMEM_NO_NETWORK=1` |
| `msrv` | `cargo build --workspace` on Rust 1.91 |
| `audit` | `cargo audit` for security advisories |
| `python-sdks` | The Python SDK tests |
| `whitepaper` | `python3 scripts/render_whitepaper.py --check` |
| `doc-lint` | `scripts/doc_lint.py` and other repository checks |
| `guard-conformance` | Builds `emem-guard` and checks it against a real socket |
| `contract` | Builds this commit's server, starts it on `127.0.0.1:5097` and runs the contract gates against it |
| `ts-sdk` | Type-checks and builds the TypeScript SDK in `sdks/emem-ts` |

Clippy in CI uses the current stable toolchain. Run `rustup update stable`
before you run clippy, or a new lint can pass locally and fail in CI.
Do not run it while a build runs.

## Debug the topic router

`/v1/ask` sends a question to one of 27 topics in
`crates/emem-core/data/topics-v0.json`. The default backend (`ort`) uses
the `BAAI/bge-base-en-v1.5` model on the CPU.

| Variable | Effect |
|---|---|
| `EMEM_TOPIC_BACKEND` | `ort` (default), `model2vec` or `keyword`. Use `keyword` in tests: it loads no model. |
| `EMEM_TOPIC_MODEL_DIR` | The directory that holds `model.onnx` and `tokenizer.json` |
| `EMEM_TOPIC_MODEL` | A `minishlab/` model for the `model2vec` backend. The server ignores other names. |

Run `scripts/install-topic-model.sh` once to download the model (about
435 MB) before the first `/v1/ask` call. The script writes it to
`EMEM_TOPIC_MODEL_DIR`. If that is not set, it writes to
`<EMEM_DATA>/models/bge-base-en-v1.5/`, with `EMEM_DATA` default
`./var/emem`.

The server looks for the model in `EMEM_TOPIC_MODEL_DIR`, then in
`<EMEM_DATA>/models/bge-base-en-v1.5/`. It does not use the `./var/emem`
default for this search. If you do not set `EMEM_DATA`, or you set it to
`:memory:`, set `EMEM_TOPIC_MODEL_DIR` too:

```sh
EMEM_DATA=:memory: EMEM_TOPIC_MODEL_DIR=./var/emem/models/bge-base-en-v1.5 \
  cargo run --release --bin emem-server
```

Without the model, the server does not stop. It uses the next backend.

If the `ort` model does not load, the server tries `model2vec`. That
backend downloads `minishlab/potion-base-8M` (about 32 MB) from Hugging
Face into `~/.cache/huggingface`. Set `HF_HOME` to move that cache. If
the download also fails, the server uses `keyword`. The log shows a
warning for each step (`topic_router.rs`).

After a routing change, run the routing check by hand. No CI job runs
it. The tool sends 14 questions to `/v1/ask`. Without `--base`, it uses
`EMEM_BASE_URL`, then `http://127.0.0.1:5051`.

```sh
cargo run -p emem-cli --bin emem-ask-eval -- --base http://127.0.0.1:5051
```

The tool prints one line for each question: pass or fail, the first
matched topic, the fact count and the receipt CID count. The last line
gives the pass rate, for example `trigger-rate: 14/14`. Exit code 0 means
that every question went to its expected topic and returned a signed
receipt. One question expects no topic (out of scope). On 7 November
2026, the same 14 requests to `https://emem.dev` all passed.

## Record a demo run

`emem-livedemo` takes a base URL and an output directory. Without a URL
argument, it uses `EMEM_BASE_URL`, then `http://localhost:5051`. The
default directory is `./var/demos/<UTC stamp>`.

The server serves runs from `EMEM_DEMOS_DIR`, default `var/demos` in its
own working directory. Run the demo from the directory where the server
runs. If you cannot, set `EMEM_DEMOS_DIR` on the server.

```sh
cargo run --release --bin emem-livedemo -- http://127.0.0.1:5051
curl -s http://127.0.0.1:5051/v1/demos | jq -c 'keys'   # ["count","root","runs"]
```

The directory has a `<step>.req.json` and a `<step>.resp.json` file for
each step, and a `trace.json` index. The server lists runs at
`GET /v1/demos`. `GET /v1/demos/:run` gives the index of one run, and
`GET /v1/demos/:run/:file` gives one file. To find a wire-shape
regression, compare `trace.json` with the one from the last good run.

## Commit and open a pull request

The full rules are in [CONTRIBUTING.md](https://github.com/Vortx-AI/emem/blob/main/CONTRIBUTING.md) and
[AGENTS.md](https://github.com/Vortx-AI/emem/blob/main/AGENTS.md). In short:

- Write the subject in sentence case, 72 characters or fewer. The body
  says why.
- Do not add a `Co-Authored-By` trailer for an AI tool.
- Do not use `--no-verify`. If a hook fails, fix the cause.
- A new endpoint updates the OpenAPI document and the MCP tool list in
  the same commit.
- A new band, algorithm or source updates its registry file in
  `crates/emem-core/data/` and adds a registry test.
- A change to receipt signing or canonical CBOR also updates
  `docs/protocol.md` and the browser verifier. `cargo test -p emem-fact`
  checks the round trip.

## Find things in the code

| Item | Location |
|---|---|
| Routes | `crates/emem-api-rest/src/lib.rs` (search `.route(`) |
| MCP tools | `crates/emem-mcp/src/lib.rs` |
| Cell64 encoding | `crates/emem-codec/src/geo.rs` |
| Fact CID (canonical CBOR, BLAKE3, base32) | `crates/emem-fact/src/cbor.rs`, `crates/emem-fact/src/cid.rs` |
| Receipt preimage | `crates/emem-attest/src/lib.rs`: `receipt_preimage_v2` signs; `receipt_preimage_v1` only checks old receipts. Signer: `sign_receipt` in `crates/emem-storage/src/server.rs` |
| Merkle root and paths | `crates/emem-attest/src/lib.rs` |
| Lazy materialization | `crates/emem-storage/src/lib.rs` (`materialize_many`) |
| Physics solvers | `crates/emem-api-rest/src/physics.rs` |
| Topic router | `crates/emem-api-rest/src/topic_router.rs` |
| Geocoder order | `crates/emem-api-rest/src/lib.rs`, near `EMEM_NOMINATIM_BASE` |

## Report a bug

Open an issue at `github.com/Vortx-AI/emem/issues`. Include:

- The server version (`version` in `GET /health`) and `rustc --version`.
- The output of `curl -s http://127.0.0.1:5051/v1/manifests`, so that the
  registry CIDs are in the report.
- The `curl` command, the expected response and the actual response.

Do not report a security problem in a public issue. Read
[SECURITY.md](https://github.com/Vortx-AI/emem/blob/main/SECURITY.md). A running responder also serves
`/.well-known/security.txt`, with the contact from `EMEM_TLS_CONTACT`.

For deployment, read [operating.md](../operators/operating.md).
