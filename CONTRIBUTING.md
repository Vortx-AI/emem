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
a feature. Every number in prose must trace to a registry or a
measurement. `python3 scripts/sync_counts.py` checks the known ones.
Every claim must agree with the code as written. Record a withdrawn
claim; do not silently delete it. Section 18 of
[docs/whitepaper.md](docs/whitepaper.md) shows the pattern. `scripts/doc_lint.py` enforces the mechanical parts in CI
(dashes, buzzwords, links). `scripts/ste_lint.py` checks sentence length and
plain words; run it on your changed lines. [docs/writing-docs.md](docs/writing-docs.md)
gives the rules and both commands.

## Add a data source

A new band needs a fetch module in `emem-fetch` and a materializer in
`emem-api-rest`. [Add a connector](docs/developers/data-sources.md#add-a-connector)
in `data-sources.md` gives all ten steps. Three of them are easy to miss:

1. Add the band key to `all_materializable_bands` in
   `crates/emem-api-rest/src/lib.rs`. If the key is not there,
   `/v1/bands`, `/v1/materializers` and `/v1/data_availability` do not show it.
2. A dotted key, such as `copdem30m.elevation_mean`, needs no change to
   `crates/emem-core/data/bands-v0.json`. Only a new cube slot changes that
   file. The `dims` of all bands must then still add up to 1792.
3. Return `Ok(None)` for a real empty result and `Err` for an upstream
   failure. Never invent a default value.

The `SourceConnector` trait in `crates/emem-fetch/src/connectors.rs` is the
transport layer (HTTPS, GCS, IPLD). A new data source does not use it.

Test the parser offline with `cargo test -p emem-fetch <name>`. Put
network tests in `crates/emem-fetch/tests/live_cog_fetch.rs`. They skip when
you set `EMEM_NO_NETWORK=1`.

## Add an algorithm

1. Append an entry to `crates/emem-core/data/algorithms-v0.json`. Each entry
   needs `key` (unique and versioned, for example `my_score@1`), `kind`,
   `inputs`, `formula`, `output`, `when_to_use`, `primitive` and `citation`.
   The `formula` is plain math for people. The registry does not load if two
   keys are the same or if `inputs` is empty.
2. To let the node compute the value, add an `evaluation` expression tree.
   Without it, the node only lists the recipe and the agent applies the
   formula. If the tree needs a new operation, add it to `Expr` in
   `crates/emem-core/src/algorithms.rs`.
3. Give each entry in `parameters` a `basis`: `literature`,
   `operational_default` or `fitted`. A `fitted` basis needs more fields.
4. The algorithm shows at `GET /v1/algorithms`. To route `/v1/ask` and
   `/v1/locate` to it, add its key to the `algorithms` list of a topic in
   `crates/emem-core/data/topics-v0.json`.

A change to a registry file moves its manifest CID. Run
`cargo test -p emem-core` after the change.

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

## Report a bug

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

When you take part, you agree to follow [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).

## License

Apache-2.0 (see [LICENSE](LICENSE)). No CLA. When you contribute, you agree
to license your contribution under the same terms as the rest of the repo.
