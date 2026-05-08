# AGENTS.md

This file is the README for *coding agents* working on the emem source.
Per the [agents.md convention](https://agents.md) (OpenAI Codex, 2025;
Linux Foundation AAIF, Dec 2025): build commands, test rules, code style,
and what's off-limits during autonomous runs. End users of the protocol
should read [docs/agents.md](docs/agents.md) and [skills.md](web/skills.md)
instead — those describe how to *use* emem from an agent.

## Repo shape

Rust workspace, 14 crates, version 0.0.4, MSRV 1.88. The bulk of the code
lives in `crates/emem-api-rest/src/lib.rs` (~23.5 k lines: HTTP/MCP router
plus every inline materializer) and `crates/emem-fetch/src/*.rs` (16
connectors over open-data APIs). FastAPI sidecar in `python/jepa_v2_sidecar/`
serves Prithvi / Galileo / JEPA-v2 over a Unix socket. Web surface in
`web/` is plain HTML — no build step, included via `include_str!`.

## Build

```sh
# Full workspace, debug
cargo build --workspace

# Production binary (what /home/ubuntu/.config/systemd/user/emem-server.service runs)
cargo build --release --bin emem-server

# After release rebuild on systems that need port 443 binding:
sudo setcap 'cap_net_bind_service=+ep' target/release/emem-server
systemctl --user restart emem-server.service
```

`scripts/redeploy.sh` does build + setcap + restart in one shot.

## Test

```sh
# Unit + bin + crate-level integration, no network
cargo test --workspace --lib --bins --tests

# Network-gated tests live in crates/emem-fetch/tests/live_cog_fetch.rs
# and skip automatically when offline. There is NO `live` cargo feature —
# don't write `--features live`, that flag doesn't exist.
cargo test --workspace --test live_cog_fetch
```

Unit tests live inline in `src/` next to the code they cover (`#[cfg(test)] mod tests`).
Crate-level integration tests live under `crates/<crate>/tests/`. Only two
crates have those today: `emem-fact` (round-trip) and `emem-fetch` (live COG fetch).

## Lint + format

```sh
cargo fmt --all                                                  # local
cargo fmt --all --check                                          # CI gate
cargo clippy --workspace --all-targets -- -D warnings            # CI gate
```

Both gates run in `.github/workflows/ci.yml` against Linux + macOS, plus
an MSRV 1.88 build job. Don't ship a commit that fails either — the CI
will reject it and the next agent will have to figure out why.

## Code style

- **No `unsafe`** — every crate carries `#![forbid(unsafe_code)]` at the top.
- **No `unwrap()` on user-facing paths.** Use `?` with structured errors;
  the `MaterializeMiss` / `CidNotFound` types are designed to surface
  honest gaps without panicking. `unwrap()` is fine inside tests.
- **Don't widen `pub`** — keep crate boundaries tight. New public API
  needs a justification in the commit message.
- **Comments**: only when the *why* is non-obvious (a hidden constraint, a
  workaround for a specific bug, a subtle invariant). Don't explain *what*
  the code does — well-named identifiers do that. Don't reference the
  current task or PR — those belong in the PR description.
- **Receipts and CIDs are load-bearing**. Any change to
  `crates/emem-storage/src/server.rs::sign_receipt` or the canonical-CBOR
  layout in `crates/emem-fact/src/` requires a corresponding update to
  `docs/protocol.md` (the byte-by-byte preimage example) and to the
  in-browser verifier at `web/humans.html` (the BLAKE3 + Ed25519 path).
  These three places must always agree.

## Commit + PR conventions

Commit messages are sentence-case, ≤72 chars on the subject line. Body
explains the *why* and any non-obvious wire/protocol implications. Tag
with the version prefix when relevant — `0.0.4: …`.

**Never** add a `Co-Authored-By: Claude` trailer or any AI attribution
trailer. The user has stated this preference in
`/home/ubuntu/.claude/projects/-home-ubuntu-emem/memory/feedback_no_claude_coauthor.md`.

Don't commit on the user's behalf without an explicit instruction. Don't
force-push to `main`. Don't `--no-verify` past pre-commit hooks; if a
hook fails, fix the underlying issue.

## Off-limits during autonomous runs

- `var/emem/identity.secret.b32` — the responder's Ed25519 secret. Never
  read, copy, or commit this file. It is mode `0600` for a reason.
- `var/emem/sled/` — the live attestation log. Never delete or rewrite;
  agent-replayable bugs are debugged from this log.
- `crates/emem-codec/src/alphabet.rs` — the cell64 alphabet builder.
  Changing this rotates every cell64 in the corpus. Don't touch without
  a registry CID rotation plan.
- `web/index.html` GA injection — the consent banner is GDPR-compliant
  by design; don't simplify it without re-reading
  `docs/operating.md` § Privacy/GA.

## Where to find things

| concern | path |
|---|---|
| HTTP/MCP router, route registrations | `crates/emem-api-rest/src/lib.rs` |
| Inline materializers | `crates/emem-api-rest/src/lib.rs` (search `^async fn materialize_`) |
| Open-data connectors | `crates/emem-fetch/src/*.rs` (16 modules) |
| Receipt signing + preimage | `crates/emem-storage/src/server.rs::sign_receipt` |
| Canonical CBOR + FactCid | `crates/emem-fact/src/{cbor,cid}.rs` |
| Cell64 / tslot / alphabet | `crates/emem-codec/src/` |
| Merkle log + per-fact proofs | `crates/emem-storage/src/{merkle_log,server}.rs` |
| Registries (8 manifests) | `crates/emem-core/data/*.json` + `src/` |
| MCP tool registry | `crates/emem-mcp/src/lib.rs` |
| 11 read primitives | `crates/emem-primitives/src/*.rs` |
| Physics solvers (heat / wave / NDVI / JEPA-v2) | `crates/emem-api-rest/src/physics.rs` |
| Sidecar (FastAPI over UDS) | `python/jepa_v2_sidecar/server.py` |
| `/humans` interactive console | `web/humans.html` |
| Static web surface | `web/` (served via `include_str!` from api-rest) |
| Demos | `crates/emem-cli/src/bin/emem-{demo,livedemo,realdemo}.rs` |

## Where to read more

- [docs/architecture.md](docs/architecture.md) — what shape the protocol takes
- [docs/protocol.md](docs/protocol.md) — wire format, preimages, encodings
- [docs/whitepaper.md](docs/whitepaper.md) — math + design rationale
- [docs/operating.md](docs/operating.md) — deployment, env vars, TLS, CSP
- [docs/developing.md](docs/developing.md) — dev workflow + test invariants
- [docs/agents.md](docs/agents.md) — *consumer*-agent guide (how to USE emem)
- [web/skills.md](web/skills.md) — composed recipes for agent integrations
- [.claude/skills/](claude-skills/) — installable Anthropic Skills bundle
