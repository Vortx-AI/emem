# developing emem

This doc is for engineers cutting code against the emem.dev workspace at
`github.com/Vortx-AI/emem`. It covers setup, the day-to-day workflows the
maintainers actually run, and the conventions the repo enforces. It does
not duplicate `docs/architecture.md` (system topology) or
`docs/protocol.md` (wire bytes).

## Setup

| | |
|---|---|
| Rust toolchain | 1.88 (workspace `rust-version` in `Cargo.toml`) |
| Edition | 2021 |
| Workspace version | 0.0.6 |
| OS for production | Linux, `debian:trixie-slim` (glibc 2.41 — required by `ort-sys` 2.0.0-rc.12) |
| OS for dev | Linux or macOS; older glibc is fine if you skip the topic-router ONNX backend |
| Repo | `github.com/Vortx-AI/emem`, branch `main` |

```
git clone https://github.com/Vortx-AI/emem
cd emem
cargo build --workspace --release
./target/release/emem-server  # binds 0.0.0.0:5051 by default
```

The first build pulls a few hundred crates. Allow ~12 minutes on a clean
host with cold caches; ~90 seconds on warm. The big-ticket transitive
dep is `ort-sys` (bundled ONNX parser; needs `g++` and glibc 2.38+).

Port note: `5051` is emem's default. Port `5050` on the dev host is
held by an unrelated `tile_server.py` for a different project, so it is
permanently unavailable for emem; do not try to switch defaults to 5050.

## Workspace tour

The workspace is 14 Rust crates plus a Python sidecar. One-line role for
each:

| Crate | Role |
|---|---|
| `emem-core` | bands, algorithms, functions, sources, topics, schema, taxonomy, manifest, privacy, tslot, cell, bbox |
| `emem-codec` | cell64 / cid64 / tslot_text / vec64 / hilbert / geo / alphabet codecs |
| `emem-fact` | Fact / Receipt / Attestation CBOR + ed25519 signing primitives |
| `emem-claim` | Claim predicate (`Op` enum + value, no signature) |
| `emem-cache` | sled wrapper |
| `emem-fetch` | open-data connectors (cog, dmsp_ols, firms, hansen_gfc, koppen, overture, terraclimate, wdpa, worldpop, stac, proj, template, cache_window, connectors) |
| `emem-storage` | sled hot cache, attesters reputation, append-only merkle log, `Server` + `MaterializingStorage` |
| `emem-cubes` | AgriSynth `.npz` handle (Python authoritative) |
| `emem-primitives` | recall, find_similar, trajectory, compare, compare_bands, diff, verify, query_region, binary_embedding, refinement, cbor_ops |
| `emem-attest` | pure `merkle_root` + `merkle_root_and_paths` |
| `emem-intent` | 7-variant `Intent` enum → `Plan{calls[]}` rule-based planner |
| `emem-mcp` | MCP tool registry (single file) |
| `emem-api-rest` | HTTP/MCP router, ~167 `.route()` registrations (mapping to 74 distinct REST paths in `openapi.json`) + 36 MCP tools, all inline materializers |
| `emem-cli` | 7 binaries (see below) |

The bulk of the codebase is concentrated. `crates/emem-api-rest/src/lib.rs`
is one file at ~23.5 k lines; it is the central router and holds every
inline materializer. `crates/emem-fetch` is ~8.8 k lines spread across 16
modules (cache_window, chirps, cog, connectors, dmsp_ols, firms, hansen_gfc,
koppen, lib, overture, proj, stac, template, terraclimate, wdpa, worldpop).
Most contributions touch one or two crates at most — usually `api-rest`
plus one of `fetch`, `primitives`, or `core`.

Out of the workspace:

- `python/jepa_v2_sidecar/` — FastAPI over UDS for Prithvi-EO-2.0,
  Galileo, and JEPA-v2 dynamics inference on GPU.
- `python/jepa_v2/` — training scripts (assemble_data, train, export_baseline).
- `web/` — static SSR HTML landing page (no React/Vue, no API calls).
- `sdks/emem-py`, `sdks/emem-ts` — empty placeholders today; integrate
  via REST or MCP.
- `examples/` — LangChain, LlamaIndex, MCP host configs (claude-code,
  cursor, cline, claude-desktop, gemini-extension, openai-gpt-action).
- `ops/systemd/` — service unit + journald retention drop-in.
- `Dockerfile` — multi-stage build, runtime stage on `debian:trixie-slim`.
- `huggingface-space/` — Docker SDK wrapper around the GHCR image.

## The 7 CLI binaries

`crates/emem-cli` produces seven distinct binaries. Pick by intent:

| Binary | Role |
|---|---|
| `emem` | introspection: `manifests`, `bands`, `functions`, `sources`, `errors`, `keygen`, `cell <cell64>` decode, `cell-encode <u64>`, `verify [path|-]` for offline receipt verify |
| `emem-server` | the production HTTP+MCP server; single port serves both |
| `emem-demo` | smoke test: 3 cells × 1 band against an existing endpoint |
| `emem-livedemo` | full audit-trail run: 4 cells × 2 bands × 3 tslots = 24 facts; writes per-step JSON + curl repro + blake3 hashes + `trace.json` to `var/demos/<UTC>/` |
| `emem-realdemo` | live-data run against Cop-DEM 30 m S3 tiles for Mt Fuji, Mt Everest, Grand Canyon |
| `emem-ask-eval` | CI regression for `/v1/ask` topic routing — 12 fixed questions plus 1 intentional out-of-scope |
| `emem-purge-fnkey` | admin cache-purge by `derivation.fn_key`; **server must be stopped** (sled holds an exclusive lock) |

The three demo binaries are intentionally non-overlapping. Use
`emem-demo` for the 30-second sanity check, `emem-livedemo` when you
want a directory you can paste in a PR, and `emem-realdemo` to confirm
network-dependent ingest still works.

## Common workflows

### Run a server locally

```
EMEM_DATA=:memory: cargo run --release --bin emem-server &
sleep 2
curl -s http://127.0.0.1:5051/health | jq
curl -s -X POST http://127.0.0.1:5051/v1/locate \
  -H 'content-type: application/json' \
  -d '{"q":"Bengaluru"}' | jq '.cell64'
```

`:memory:` skips the sled file and gives you a fresh ephemeral
responder identity per run. For anything reproducible — receipt
round-tripping, livedemo, re-attesting after a restart — use a real
data dir so `<EMEM_DATA>/identity.secret.b32` (mode 0600) persists the
ed25519 key across runs.

### Add a new band

1. Edit `crates/emem-core/data/bands-v0.json`. Append the band entry.
   Verify cube offsets do not collide and that the total dim count
   still equals 1792. The validator is in
   `crates/emem-core/src/bands.rs`.
2. `cargo test -p emem-core bands` — the registry tests catch overlap,
   gap, and dim-sum errors.
3. If the band derives from another band, add a recipe in
   `crates/emem-core/data/functions-v0.json` and re-run the validators
   in `crates/emem-core/src/functions.rs`.
4. Wire a materializer. Inline materializers live in
   `crates/emem-api-rest/src/lib.rs` (search for the band registry
   `match` near the materializer index). Heavier connectors get a
   module under `crates/emem-fetch/src/`.
5. Register `band_materializer_meta` so `/v1/data_availability` reports
   `history_available_from_unix` / `history_available_to_unix` /
   `tempo_seconds` honestly. A null history bound is fine for static
   climatology and for nowcast-only sources; the agent surface depends
   on this signal being accurate.
6. `cargo test -p emem-api-rest` — endpoint contract tests catch
   regressions in the materializer fan-out.

### Add a new connector

A connector is the bytes-on-the-wire layer that sits behind a
materializer. Steps:

1. New file under `crates/emem-fetch/src/<scheme>.rs`.
2. Re-export from `crates/emem-fetch/src/lib.rs`.
3. Use the `cog::range_read` helper for COG/GeoTIFF or the `stac`
   helper for STAC catalogues — both already wrap `vsicurl` semantics
   and a 30-second per-upstream timeout (overridden by
   `EMEM_MATERIALIZER_TIMEOUT_SECS`).
4. Update `crates/emem-core/data/sources-v0.json` so the source
   shows up in `/v1/sources` and gets a real `sources_cid`.
5. Document the source in `docs/data-sources.md`. License, native
   resolution, vintage cadence, primary URL, mirror if any. The doc is
   `include_str!()`'d into the binary so it ships with the server.

### Add a primitive

1. New file under `crates/emem-primitives/src/<name>.rs` with a `Req`,
   `Resp`, and an async function.
2. Re-export from `crates/emem-primitives/src/lib.rs`.
3. Wire into `crates/emem-api-rest/src/lib.rs` (axum router) and
   `crates/emem-mcp/src/lib.rs` (MCP tool list).
4. Add an OpenAPI schema entry in `api-rest/src/lib.rs`'s
   `openapi_json()` builder.
5. Sign the receipt:
   `srv.sign_receipt(name, cells, fact_cids, was_cached, started, intent)`.
   See `crates/emem-storage/src/server.rs:119` for the canonical
   preimage layout.

### Test invariants

| Goal | Command |
|---|---|
| Unit + bin tests, no network | `cargo test --workspace --lib --bins --tests` |
| Live (network-dependent) tests | `cargo test --workspace --test live_cog_fetch` (or run the file directly — there is no `live` cargo feature, the network-gated tests live in `crates/emem-fetch/tests/live_cog_fetch.rs` and are skipped automatically when offline) |
| Format (local) | `cargo fmt --all` |
| Format (CI gate) | `cargo fmt --all --check` |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` |

Unit tests live inline in `src/` next to the code under test
(`#[cfg(test)] mod tests`); the only crate-level integration tests are
`crates/emem-fact/tests/round_trip.rs` and `crates/emem-fetch/tests/live_cog_fetch.rs`.

The "no stubs" rule is enforced by `feedback_no_stubs.md` and by the
contract tests:

- `recall` returns `bands_already_attested_at_cell` so an empty
  response distinguishes "wrong query" from "place is empty"; the
  same envelope on `recall_many` repeats the list per cell.
- `compare_bands` returns `bands_with_no_history` for bands the
  responder has never attested anywhere, separately from bands it
  has attested but not at this cell.
- `recall` on a band with no materializer returns a structured
  `MaterializeMiss` error, not an empty list.
- `compare_bands` with a mismatched fact type errors instead of
  silently coercing.

When you add a primitive, write a test that locks at least one of
those failure modes. Empty results with no shape information will be
caught in review.

### Working with the GPU sidecar

The sidecar is Python (FastAPI over a Unix Domain Socket). It runs
Prithvi-EO-2.0-300M-TL, Galileo, and the JEPA-v2 dynamics network on
CUDA. Cold start on first `/predict` call.

Bring it up manually:

```
cd python/jepa_v2_sidecar
uv venv
uv pip install -r requirements.txt
EMEM_SIDECAR_SOCK=/tmp/emem-sidecar.sock \
  python -m uvicorn server:app --uds /tmp/emem-sidecar.sock
```

Or via systemd, the unit ships at
`python/jepa_v2_sidecar/emem-jepa-sidecar.service`:

```
systemctl --user start emem-jepa-sidecar.service
```

The Rust server reads `EMEM_SIDECAR_SOCK`,
`EMEM_SIDECAR_TIMEOUT_MS` (default 5000 ms), and
`EMEM_SIDECAR_VRAM_BUDGET_GB` (default 10) for fan-out. Sidecar
unavailable → `/v1/jepa_predict_v2` returns 503; there is no silent
in-process fallback.

### Round-trip a receipt

```
curl -s -X POST http://127.0.0.1:5051/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell": "BGLR-1234", "band": "geotessera"}' \
  > /tmp/receipt.json

./target/release/emem verify /tmp/receipt.json \
  --base-url http://127.0.0.1:5051
```

`emem verify` rebuilds the blake3 preimage byte-for-byte using the
same code path as `POST /v1/verify_receipt`. Pubkey resolution is
`--pubkey > --base-url's /.well-known/emem.json > the receipt's
embedded responder`. Round-trip tests live at
`crates/emem-fact/tests/round_trip.rs`.

A third verification surface lives in the browser: `web/humans.html`
(served at `/humans`) imports `@noble/curves@1.6.0/ed25519` and
`@noble/hashes@1.5.0/blake3` from `https://esm.sh` and reproduces the
exact preimage in JavaScript, so any star you click on the public
page verifies its own receipt locally without the page calling back
to the responder. The CSP header is configured to allow esm.sh in
both `script-src` and `connect-src` (see
`crates/emem-api-rest/src/lib.rs` `security_headers`); the page falls
back to `POST /v1/verify_receipt` automatically if the noble libs
fail to load (CDN slowdown, blocked egress) and labels itself
accordingly.

### Driving every primitive against a local server

Once `emem-server` is up on `:5051`, this sequence touches every
agent-facing primitive in roughly the order an integration would:

```
BASE=http://127.0.0.1:5051
JQ='jq -c'

# 1. Discover what the responder thinks it is.
curl -sS $BASE/health | $JQ
curl -sS $BASE/v1/manifests | $JQ
curl -sS $BASE/v1/grid_info | $JQ '.cell_kind, .pitch_m'

# 2. Geocode + recall.
CELL=$(curl -sS -X POST $BASE/v1/locate \
  -H 'content-type: application/json' \
  -d '{"q":"Bengaluru"}' | jq -r '.cell64')
echo "cell=$CELL"
curl -sS -X POST $BASE/v1/recall \
  -H 'content-type: application/json' \
  -d "{\"cell\":\"$CELL\",\"band\":\"geotessera\"}" | $JQ

# 3. Compare two cells on a shared band.
curl -sS -X POST $BASE/v1/compare \
  -H 'content-type: application/json' \
  -d "{\"cell_a\":\"$CELL\",\"cell_b\":\"$CELL\",\"band\":\"geotessera\"}" | $JQ

# 4. Trajectory across tslots.
curl -sS -X POST $BASE/v1/trajectory \
  -H 'content-type: application/json' \
  -d "{\"cell\":\"$CELL\",\"band\":\"geotessera\",
       \"from_unix\":1704067200,\"to_unix\":1735689599}" | $JQ

# 5. Verify any one of those receipts offline.
curl -sS -X POST $BASE/v1/verify_receipt \
  -H 'content-type: application/json' \
  -d @/tmp/receipt.json | $JQ
```

If any step returns 500 with `MaterializeMiss`, set
`EMEM_AUTO_MATERIALIZE=1` and re-run; if a band has no materializer
the response will instead be a clean `bands_with_no_history` list.
Empty results that do not surface either signal are a regression.

### Working with the live demo run

`emem-livedemo` is the canonical "show me everything" run. It writes
a directory under `var/demos/<UTC>/` with one JSON file per step
plus a `trace.json` index, and the server exposes them at
`GET /v1/demos`, `GET /v1/demos/:run`, `GET /v1/demos/:run/:file`.

```
EMEM_DEMOS_DIR=$PWD/var/demos \
./target/release/emem-livedemo http://127.0.0.1:5051

# inspect
curl -s http://127.0.0.1:5051/v1/demos | jq '.runs[-1]'
ls var/demos/$(curl -s http://127.0.0.1:5051/v1/demos \
  | jq -r '.runs[-1].run')
```

The directory contains `request.json`, `response.json`, and
`receipt.json` for every step plus a `repro.curl` file you can
literally pipe to `bash`. When you are debugging a wire shape
regression, that's the fastest way to bisect — diff `trace.json`
against the previous good run.

### When the topic router misbehaves

`/v1/ask` routes a natural-language question to one of 26 topics
via `BAAI/bge-base-en-v1.5` (CPU ort by default). Pre-stage with
`scripts/install-topic-model.sh` so the first request is not paying
a 90-second cold-start. Force the keyword backend for tests with
`EMEM_TOPIC_BACKEND=keyword`. Override the model path with
`EMEM_TOPIC_MODEL_DIR=/path/to/bge-base-en-v1.5`. The CI regression
that locks routing accuracy is `emem-ask-eval`:

```
./target/release/emem-ask-eval --base http://127.0.0.1:5051
```

Exit 0 means every corpus question routed to its expected topic
and returned a signed receipt; non-zero exit prints the failing
question and what the router chose instead.

## Commit + PR rules

- Conventional commits: `feat:`, `fix:`, `docs:`, `refactor:`, `test:`,
  `chore:`. No subject-line emoji. Imperative mood.
- Never add a `Co-Authored-By: Claude ...` trailer.
- Never `--no-verify` a pre-commit hook. If the hook fails, fix the
  underlying issue and create a new commit.
- A new endpoint must update both the OpenAPI builder
  (`api-rest/src/lib.rs::openapi_json`) and the MCP tool list
  (`emem-mcp/src/lib.rs`) in the same commit.
- A new band, algorithm, or source must update `bands-v0.json`,
  `algorithms-v0.json`, or `sources-v0.json` and add a registry test.
- Receipts must round-trip: ed25519 signature over canonical CBOR
  preimage. The check in `emem-fact/tests/round_trip.rs` locks this.

## Repo navigation

| Looking for | File |
|---|---|
| The route table | `crates/emem-api-rest/src/lib.rs` (grep `route(`) |
| MCP tool catalog | `crates/emem-mcp/src/lib.rs` |
| Cell64 encoding | `crates/emem-codec/src/geo.rs` |
| CID derivation (canonical CBOR → blake3 → b32-nopad-lower) | `crates/emem-fact/src/cbor.rs` + `crates/emem-fact/src/cid.rs` |
| Receipt preimage builder | `crates/emem-storage/src/server.rs:119` |
| Merkle root + path verification | `crates/emem-attest/src/lib.rs` |
| Lazy materialization plumbing | `crates/emem-storage/src/lib.rs` (`MaterializingStorage::materialize_many`) |
| Sidecar protocol | `python/jepa_v2_sidecar/server.py` |
| Physics solvers (heat, wave, AR(2)) | `crates/emem-api-rest/src/physics.rs` |
| Topic router (ort + tokenizers BERT) | `crates/emem-api-rest/src/topic_router.rs` |
| Geocoder layering (embedded → cache → Photon → Nominatim) | `crates/emem-api-rest/src/lib.rs` near `EMEM_NOMINATIM_BASE` |

## Filing issues

GitHub issues at `github.com/Vortx-AI/emem/issues`. Include:

- emem version (Cargo.toml workspace `version`); `rustc --version`.
- Output of `curl -s http://127.0.0.1:5051/v1/manifests` so the active
  manifest CIDs are pinned in the report.
- Reproducer: the curl command, expected response, actual response.
- For sidecar issues: `nvidia-smi` snapshot and the line from
  `journalctl --user -u emem-jepa-sidecar` that names the failing
  model.

## Security disclosures

Out-of-band, not GitHub issues. See `SECURITY.md` and
`/.well-known/security.txt` (served by the running responder under
`EMEM_TLS_CONTACT`, default `mailto:avijeet@vortx.ai`).

For deployment and runtime concerns see `docs/operating.md`.
