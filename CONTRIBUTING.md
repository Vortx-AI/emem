# Contributing to emem

emem is a Cargo workspace. Rust 1.91, edition 2021, 19 crates, one binary
(`emem-server`) at the centre. The agent-facing surface is a single axum
router in `crates/emem-api-rest/src/lib.rs`; everything else feeds into it.

## Where to start

You need Rust 1.91 or later, `g++`, `protoc` with the protobuf
well-known types, and `mdbook` 0.5.2. Build the docs site first: the
server embeds `docs/book/`, so `cargo build` fails without it.

```bash
git clone https://github.com/Vortx-AI/emem && cd emem
mdbook build docs && rm -f docs/book/book.toml
cargo build --workspace
cargo run --release --bin emem-server   # 0.0.0.0:5051
```

Open `http://127.0.0.1:5051/v1/discover` to see the bootstrap document.
An agent gets the same JSON. `emem-livedemo` runs every primitive and
writes the signed receipts under `var/demos/<UTC>/`.
[docs/developers/developing.md](docs/developers/developing.md) has the
setup details and the daily workflows.

## Ground rules

**No stubs, no silent fallbacks.** Do not commit `todo!()`, `unimplemented!()`,
"lands in vX" comments, hardcoded fake values, or empty handlers in
the router. If a code path needs an upstream that is not reachable, return a
typed `ErrorCode` (for example `SourceFetchFailed`, `Unauthorized` or
`CidNotFound`) with a clear message. If a feature is out of scope,
delete it. Do not leave a placeholder. An empty result must tell
"wrong query" from "place is empty": `recall` returns
`bands_already_attested_at_cell` so the caller can tell.

**Receipts must round-trip.** Any change to a primitive's response, signing
preimage, or fact-CBOR layout must keep `verify_receipt` green against
old receipts. Run `cargo test -p emem-fact` and replay a receipt from
`var/demos/` before you open the PR.

**Open licences, open data.** Every dependency stays MIT, Apache-2.0, BSD
or ISC. No CI job checks licences, so check a new dependency yourself.
CI runs `cargo audit` for security advisories. New default-build
connectors must use open data that needs no key (for example Copernicus,
JRC, Hansen, ESA, OSM, met.no, Open-Meteo). A provider that needs a key
goes behind an opt-in connector that the operator registers.

**Docs follow the house prose convention.** Short sentences, plain words,
no em or en dashes (use a comma, a colon, or a full stop). State what
ships and what is open in the same paragraph; never present roadmap as
capability. Every number in prose must trace to a registry or a
measurement (`python3 scripts/sync_counts.py` checks the known ones), and
every claim must be verifiable against the code as written. A withdrawn
claim is recorded, not silently deleted; the whitepaper's section 18 is
the pattern. `scripts/doc_lint.py` enforces the mechanical parts in CI
(dashes, buzzwords, links). `scripts/ste_lint.py` checks sentence length and
plain words; run it on your changed lines. [docs/writing-docs.md](docs/writing-docs.md)
gives the rules and both commands.

## Adding a band

1. Add the entry to `crates/emem-core/data/bands-v0.json`. Each band has
   `key`, `offset`, `dims`, `tempo` and `privacy`. The `dims` of all bands
   must add up to 1792.
2. Connect the materializer. Add a connector module under
   `crates/emem-fetch/src/<your_source>.rs` and declare it in
   `crates/emem-fetch/src/lib.rs`. Add the source to
   `crates/emem-core/data/sources-v0.json`.
3. Map the band to the connector in `crates/emem-api-rest/src/lib.rs`.
   Search for an existing band that is like yours (for example
   `weather.temperature_2m`).
4. The new band shows in `/v1/bands` when the manifest CID changes. You
   do not change the router.

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
   (`kind`, `fetch`, and `fetch_range` for COG sources).
2. Register the source in `crates/emem-core/data/sources-v0.json`
   (`scheme`, `tempo`, `native_resolution_m`, and a `providers` list
   with `id`, `kind`, `url_template`, `auth` and `license` for each).
3. Add an integration test under `crates/emem-fetch/tests/`. Use
   `live_cog_fetch.rs` as the model. Live tests skip when you
   set `EMEM_NO_NETWORK=1`.

## Tests

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
EMEM_NO_NETWORK=1 cargo test --workspace --lib --bins --tests
cargo test -p emem-fetch --test live_cog_fetch   # live network tests
```

CI runs the tests with the network blocked and `EMEM_NO_NETWORK=1`.
The live tests read public COGs and STAC endpoints (Copernicus DEM,
Hansen, JRC GSW, Sentinel-2 L2A). Run them after you change a connector
or the source registry.
[docs/developers/developing.md](docs/developers/developing.md#run-the-tests-and-lints)
lists the CI jobs.

## Commits and PRs

- One logical change per PR.
- Branch from `main`. Descriptive name: `feat/diff-derivative`,
  `fix/sled-prefix-scan`, `docs/agent-card-examples`.
- Write the commit subject in sentence case, 72 characters or fewer. The
  body says why, and names any wire or protocol effect.
- Conventional prefixes (`feat:`, `fix:`, `docs:`, `refactor:`, `test:`,
  `chore:`) are optional.
- Never use `--no-verify`. If a hook fails, fix the cause and push a new
  commit.
- Never add `Co-Authored-By: Claude` (or any AI co-author trailer) to
  commit messages.
- If the change touches a primitive, attach a new
  `var/demos/<UTC>/trace.json` from `emem-livedemo` that shows it green.

## Reporting bugs

Open a GitHub issue. Include:

- The server version (`version` and `responder_pubkey_b32` in `GET /health`).
- A short reproduction (the curl command and body, or the MCP
  `tools/call` payload).
- The expected and the actual result. Paste the receipt or the error
  envelope as it came.
- For a receipt problem, the full receipt JSON. The maintainers replay
  the preimage and verify it offline.

## Security

For a vulnerability, read [SECURITY.md](SECURITY.md). Do not open a public
issue. Send an email to `avijeet@vortx.ai`.

## Code of conduct

By participating you agree to abide by [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).

## License

Apache-2.0 (see [LICENSE](LICENSE)). No CLA. By contributing you agree your
contributions are licensed under the same terms as the rest of the repo.
