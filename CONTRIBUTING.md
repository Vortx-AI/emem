# Contributing to emem

emem is a Cargo workspace. Rust 1.88, edition 2021, 14 crates, one binary
(`emem-server`) at the centre. The agent-facing surface is a single axum
router in `crates/emem-api-rest/src/lib.rs`; everything else feeds into it.

## Where to start

```bash
git clone https://github.com/Vortx-AI/emem && cd emem
cargo build --workspace
cargo run --release --bin emem-server   # 0.0.0.0:5051
```

Hit `http://127.0.0.1:5051/v1/discover` to see the full bootstrap; that's
the same JSON an agent gets. `crates/emem-cli/src/bin/emem-livedemo.rs`
exercises every primitive end-to-end and dumps signed receipts under
`var/demos/<UTC>/`.

## Ground rules

**No stubs, no silent fallbacks.** Don't commit `todo!()`, `unimplemented!()`,
"lands in vX" comments, hardcoded fake values, or empty handlers wired into
the router. If a code path needs an upstream that isn't reachable, return a
typed `ErrorCode` (`SourceFetchFailed`, `Unauthorized`, `BandNotMaterialised`)
with a clear message — that's real semantics. If a feature is out of scope,
delete it rather than leaving a placeholder. Empty results must distinguish
"wrong query" from "place is empty" — `recall` returns `bands_available` so
the caller can tell.

**Receipts must round-trip.** Any change to a primitive's response, signing
preimage, or fact-CBOR layout has to keep `verify_receipt` green against
old receipts. Run `cargo test -p emem-fact` and replay a receipt from
`var/demos/` before opening the PR.

**Open licences, open data.** Every dep stays MIT / Apache-2.0 / BSD / ISC
(`cargo deny check licenses`). New default-build connectors must use
no-auth open data (Copernicus, JRC, Hansen, ESA, OSM, met.no, Open-Meteo,
Tessera, …). Keyed providers go behind an operator-registered connector,
opt-in.

## Adding a band

1. Add the entry to `crates/emem-core/data/bands-v0.json`. Each band has
   `dim_offset`, `dim_count`, `tempo`, `privacy_class`, `materializer`.
2. Wire the materialiser. Add a connector module under
   `crates/emem-fetch/src/<your_source>.rs` and register it in
   `crates/emem-fetch/src/connectors.rs`.
3. Map band → connector in the dispatch table in
   `crates/emem-api-rest/src/lib.rs` (search for the existing band you're
   modelling on, e.g. `weather.temperature_2m`).
4. The new band auto-discovers via `/v1/bands` once the manifest CID rolls
   forward. No router change needed.

## Adding an algorithm

1. Append to `crates/emem-core/data/algorithms-v0.json` with a unique
   versioned key (`my_score@1`), `inputs` (band names), `formula` (string,
   evaluable), `output_unit`, `references`.
2. The algorithm is now visible at `GET /v1/algorithms` and routable from
   `/v1/locate`'s `algorithms_for_topic`.
3. If the formula needs evaluation logic the existing engine doesn't
   handle, extend `crates/emem-primitives/src/<…>` rather than inlining it
   in the router.

## Adding a connector

1. Create `crates/emem-fetch/src/<source>.rs`. Implement `SourceConnector`
   with `kind`, `fetch`, and `range_read` for COG sources.
2. Register the source in `crates/emem-core/data/sources-v0.json`
   (`scheme`, `endpoint`, `licence`, `attribution`).
3. Add an integration test under `crates/emem-fetch/tests/` patterned on
   `live_cog_fetch.rs`. Live tests skip when `EMEM_NO_NETWORK=1` is set.

## Tests

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
EMEM_NO_NETWORK=1 cargo test --workspace   # CI default — skips network
```

Live network tests run when `EMEM_NO_NETWORK` is unset; they hit real
public COGs and STAC endpoints (Cop-DEM, Hansen, JRC GSW, Sentinel-2
L2A) and tolerate upstream flakiness so they don't break CI.

## Commits and PRs

- One logical change per PR.
- Branch from `main`. Descriptive name: `feat/diff-derivative`,
  `fix/sled-prefix-scan`, `docs/agent-card-examples`.
- Conventional prefixes welcome (`feat:`, `fix:`, `docs:`, `refactor:`,
  `test:`, `chore:`) but not enforced.
- Never use `--no-verify`. If a hook fails, fix the underlying issue and
  push a new commit; don't bypass.
- Never add `Co-Authored-By: Claude` (or any AI co-author trailer) to
  commit messages.
- If the change touches a primitive, attach a fresh
  `var/demos/<UTC>/trace.json` from `emem-livedemo` showing it green.

## Reporting bugs

GitHub issues. Include:

- Server version (`/health` JSON: `version`, `responder_pubkey_b32`).
- Minimal reproduction (curl command + body, or MCP `tools/call` payload).
- Expected vs actual (paste the receipt or error envelope verbatim).
- For receipt issues: include the full receipt JSON. We replay the
  preimage and verify offline — short receipts speed this up.

## Security

Vulnerability disclosure: see [SECURITY.md](SECURITY.md). Email
`avijeet@vortx.ai` directly for sensitive issues rather than opening a
public issue.

## Code of conduct

By participating you agree to abide by [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).

## License

Apache-2.0 (see [LICENSE](LICENSE)). No CLA. By contributing you agree your
contributions are licensed under the same terms as the rest of the repo.
