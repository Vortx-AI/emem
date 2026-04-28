# Contributing to emem

Thanks for considering a contribution. emem is built **for AI agents**, but
contributors are people — here's how to make a useful change.

## Ground rules

1. **Real implementations only.** No `todo!()`, no "lands in a future
   release", no placeholder data, no hardcoded values that should be
   manifest-driven.
2. **Open licences only.** No deps under GPL/AGPL/SSPL/BUSL etc. Verify
   `cargo deny check licenses` before submitting.
3. **Open data only.** New default-build sources must be no-auth public
   datasets (Copernicus, JRC, Hansen, ESA, OSM, etc.). Authenticated
   providers go behind operator-registered connectors.
4. **Receipts must remain verifiable offline.** Any change to receipt-
   producing code must keep the existing preimage shape so old receipts
   stay verifiable.
5. **Token economy matters.** The wire surface targets ≤ 4 tokens per
   cell, ≤ 2 per tslot, ≤ 3 per vec/cid. Don't introduce verbose JSON
   fields when a short alias works.

## Dev loop

```bash
# 1) Fork, clone.
git clone https://github.com/<you>/emem && cd emem

# 2) Format + lint + test.
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# 3) Run the server locally and exercise it end-to-end.
cargo run --release --bin emem-server &
cargo run --release --bin emem-livedemo
cargo run --release --bin emem-realdemo

# 4) Open a PR with a clear "what + why" and a link to a demo trace
#    under `var/demos/` if the change touches a primitive.
```

## Branch + commit

- Branch from `main`. Use a descriptive name: `feat/diff-derivative`,
  `fix/sled-prefix-scan`, `docs/agent-card-examples`.
- Conventional-style commit prefixes are welcome but not enforced:
  `feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`.
- One logical change per PR. CI will run fmt + clippy + tests on x86_64
  Linux + macOS.

## Adding a primitive

1. Add the request / response types and async function under
   `crates/emem-primitives/src/<your_primitive>.rs`.
2. Re-export from `emem_primitives::lib.rs`.
3. Wire a handler under `crates/emem-api-rest/src/lib.rs` and register the
   route under `pub fn router(...)`.
4. Register an MCP tool in `crates/emem-mcp/src/lib.rs` with `name`,
   `description`, `when_to_use`, `input_schema`, `example_args`.
5. Add a step in `crates/emem-cli/src/bin/emem-livedemo.rs` so the trace
   suite exercises it.
6. Update `docs/SPEC.md`, `docs/AGENTS.md`, and `web/llms.txt`.
7. PR with a fresh `var/demos/<UTC>/trace.json` showing it green.

## Adding a band

1. Update `crates/emem-core/data/bands-default.json` (the on-wire manifest).
2. Update the canonical layout offsets in the data plane that produces
   facts for that band.
3. The new band is automatically discoverable via `/v1/bands` once it lands
   in the manifest CID — no API change needed.

## Reporting bugs

Open a GitHub issue with:

- Server version (from `/health` JSON `responder_pubkey_b32` is fine as ID)
- Minimal reproduction (curl command + body)
- Expected vs. actual (paste the receipt or error envelope)
- For receipt-related issues: include the full receipt JSON; we'll
  recompute the preimage and verify offline.

## Security

Please read [SECURITY.md](SECURITY.md). For sensitive issues, email
`avijeet@vortx.ai` directly rather than opening a public issue.

## Code of conduct

By participating you agree to abide by [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
