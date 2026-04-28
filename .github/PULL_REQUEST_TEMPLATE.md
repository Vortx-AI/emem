# Summary
<!-- 1-3 bullets: what changed, why, what user-visible behaviour shifts. -->

## Type of change
- [ ] Bug fix
- [ ] New primitive / endpoint
- [ ] New band / source / function recipe
- [ ] Documentation
- [ ] Hardening (security / perf / observability)
- [ ] Refactor (no behaviour change)

## Verifications
- [ ] `cargo fmt --all --check` clean
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean
- [ ] `cargo test --workspace --release` green
- [ ] If a primitive / endpoint changed: a fresh `var/demos/<UTC>/`
      trace is attached or referenced
- [ ] If a manifest changed (bands / sources / functions / schema):
      manifest CIDs documented + receipt invariants preserved

## Receipt invariants
- [ ] Existing receipts still verify offline against this build.
- [ ] No `unwrap()` / `panic!` introduced on a hot request path.
