# syntax=docker/dockerfile:1.7

# Build stage — Rust ≥ 1.91 required by Lance 6.x (lance-namespace,
# lance-table, lance-tokenizer all MSRV 1.91; roaring 0.11 MSRV 1.90).
# We use `rust:1-slim-trixie` (latest stable in the 1.x series) rather
# than pinning to 1.91, because cargo build of the Lance + datafusion
# graph against exactly 1.91 was failing (no specific rustc error
# surfaced; latest stable builds cleanly locally on 1.95).
#
# Trixie (Debian 13, glibc 2.41) is required because ort-sys 2.0.0-rc.12
# bundles ONNX's parser.cc which references __isoc23_strtoull /
# __isoc23_strtol — symbols that only exist in glibc ≥ 2.38. On
# bookworm-slim (glibc 2.36) the link fails with "undefined reference".
FROM rust:1-slim-trixie AS build
ARG TARGETARCH
WORKDIR /usr/src/emem

# OpenSSL is *not* needed (we use rustls-acme), but build tools are.
# g++ is required by transitive C++ deps:
#   • ort-sys → bundled ONNX parser.cc (compiled via cc-crate)
#   • model2vec-rs → tokenizers → esaxx-rs
# protobuf-compiler (protoc) + libprotobuf-dev (well-known protos) are
# required by Lance 6.x build scripts:
#   • lance-encoding's encodings_v2_0.proto imports
#     google/protobuf/empty.proto — a Google well-known type. Debian
#     ships the protoc binary in `protobuf-compiler` and the actual
#     .proto definitions in `libprotobuf-dev` (under
#     /usr/include/google/protobuf/). Both are needed; protoc alone
#     dies with "File not found" at lance-encoding-*/build-script-build.
# The runtime stage is a fresh debian:trixie-slim so it does not
# inherit g++ / protoc — this only adds ~50 MB to the build stage.
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    apt-get update && \
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        pkg-config ca-certificates g++ \
        protobuf-compiler libprotobuf-dev && \
    rm -rf /var/lib/apt/lists/*

# Install mdbook *before* the COPY layers so this layer caches across
# every Rust / docs / web edit. emem-api-rest's lib.rs embeds the
# rendered /docs/ site via `include_dir!("$CARGO_MANIFEST_DIR/../../docs/book")`,
# so the cargo build below cannot proceed until docs/book/ exists.
# `cargo install` works on every arch the build matrix supports (the
# Rust toolchain is already present); no need to fish out a prebuilt
# mdbook binary per ${TARGETARCH}.
RUN --mount=type=cache,id=cargo-registry-${TARGETARCH}-trixie-r2,target=/usr/local/cargo/registry,sharing=locked \
    cargo install --locked --version 0.5.2 mdbook

# Cache `cargo fetch` against the workspace manifest before pulling in
# source — keeps re-builds fast when only Rust files change.
# crates/emem-api-rest pulls files from web/, docs/, examples/ via
# include_str!() so they have to ride along in the build context.
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY web/ web/
COPY docs/ docs/
COPY examples/ examples/
COPY claude-skills/ claude-skills/
# Root-level markdown is include_str!'d directly by emem-api-rest.
# Without these the build fails with `couldn't read PRIVACY.md`.
COPY PRIVACY.md TERMS.md SUPPORT.md SECURITY.md ./

# Render the /docs/ mdbook site. The post-build `rm` drops
# `docs/book/book.toml` — mdbook copies our build config into the output
# because `src = "."` pulls in every non-md file; we don't want a leaked
# build config riding inside the embedded tree.
RUN mdbook build docs && rm -f docs/book/book.toml

# BuildKit cache-mount IDs are scoped by ${TARGETARCH} so the parallel
# linux/amd64 + linux/arm64 build jobs don't race each other unpacking
# the same crate into a shared cache (`File exists (os error 17)` on
# `.cargo-ok`). Each arch keeps its own warm cache across runs.
# Cache id includes "trixie" so the bookworm-era target/ from previous
# builds (which baked __isoc23_strtoull-referencing parser.cc.o under
# different headers) is not reused — fresh trixie build from scratch.
#
# CARGO_PROFILE_RELEASE_LTO=off + CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16
# override the workspace's `lto="thin", codegen-units=1` for the
# docker build only — the LTO link step over Lance + datafusion +
# arrow (530+ crates) peaks past the 16 GB ubuntu-latest RAM ceiling
# and the linker dies with cargo exit 101 around 7-9 min. Disabling
# LTO + parallelising codegen drops peak memory by ~10x; the binary
# is ~5% larger and a few % slower on hot loops, but it actually
# builds. Local builds and the GHA `ci` workflow keep full
# thin-LTO via the workspace profile.
#
# CARGO_BUILD_JOBS=2 caps parallel compile jobs at 2 so the largest
# crates (datafusion / lance-datagen) don't stack their rustc peak
# RAM across all 4 runner cores at the same moment. Trades ~30 %
# wall-clock for ~50 % peak RAM headroom.
ENV CARGO_PROFILE_RELEASE_LTO=off \
    CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 \
    CARGO_BUILD_JOBS=2
# Use bash so PIPESTATUS works (sh doesn't). On failure, dump the last
# 200 lines of cargo output so the actual rustc error is visible in the
# buildx step log (annotations API only shows "cargo exit 101" otherwise).
SHELL ["/bin/bash", "-c"]
RUN --mount=type=cache,id=cargo-registry-${TARGETARCH}-trixie-r2,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=emem-target-${TARGETARCH}-trixie-r2,target=/usr/src/emem/target,sharing=locked \
    set -o pipefail; \
    cargo build --release --bin emem-server 2>&1 | tee /tmp/build.log; \
    rc=${PIPESTATUS[0]}; \
    if [ "$rc" -ne 0 ]; then \
      echo "==== CARGO BUILD FAILED (rc=$rc) — tail of build.log ===="; \
      tail -200 /tmp/build.log; \
      exit "$rc"; \
    fi; \
    cp target/release/emem-server /usr/local/bin/emem-server

# Runtime stage — minimal Debian, non-root, with cap_net_bind_service
# pre-applied so EMEM_BIND=0.0.0.0:443 works without docker run --cap-add.
# Must match the build stage's libc (glibc 2.41 on trixie) so the
# binary's __isoc23_* references resolve at runtime.
FROM debian:trixie-slim AS runtime
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    apt-get update && \
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        ca-certificates libcap2-bin bash && \
    rm -rf /var/lib/apt/lists/* && \
    useradd --system --uid 65532 --no-create-home --shell /usr/sbin/nologin emem

COPY --from=build /usr/local/bin/emem-server /usr/local/bin/emem-server
RUN setcap 'cap_net_bind_service=+ep' /usr/local/bin/emem-server

# OCI annotations — keep aligned with server.json. The MCP Registry
# uses io.modelcontextprotocol.server.name to verify ownership of the
# image; the rest are standard org.opencontainers.image.* labels for
# generic OCI tooling (cosign, syft, GHCR UI).
LABEL io.modelcontextprotocol.server.name="io.github.Vortx-AI/emem" \
      org.opencontainers.image.title="emem" \
      org.opencontainers.image.description="The verifiable memory protocol for the physical world: content-addressed, ed25519-signed observations AI agents can cite" \
      org.opencontainers.image.url="https://emem.dev" \
      org.opencontainers.image.source="https://github.com/Vortx-AI/emem" \
      org.opencontainers.image.licenses="Apache-2.0" \
      org.opencontainers.image.vendor="Vortx-AI"

# Persistent storage volume (sled cache + identity key). /var/emem is
# the image default; /data is pre-created for platforms that mount their
# managed volume there (Glama does) and set EMEM_DATA=/data in env.
RUN mkdir -p /var/emem /data && chown -R emem:emem /var/emem /data
VOLUME ["/var/emem"]

USER emem
# EMEM_BIND is intentionally NOT baked: the entrypoint derives it from
# PORT (the PaaS convention Glama and friends inject) with 5051 as the
# fallback, and an explicit -e EMEM_BIND=... still wins.
ENV EMEM_DATA=/var/emem \
    RUST_LOG=info

# 5051 — plain HTTP for local / proxy deployments.
# 443  — HTTPS via rustls + Let's Encrypt ACME (set EMEM_TLS_DOMAINS).
EXPOSE 5051 443

# Lightweight container healthcheck against /health. Use bash builtin
# /dev/tcp so the runtime image stays free of curl / wget.
HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD bash -c 'b="${EMEM_BIND:-0.0.0.0:${PORT:-5051}}"; </dev/tcp/127.0.0.1/${b##*:}' || exit 1

# PORT (PaaS convention) -> EMEM_BIND translation. TLS stays off unless
# EMEM_TLS_DOMAINS is set explicitly; platforms terminate TLS at their
# gateway and probe GET /ping (served as an alias of /health).
ENTRYPOINT ["/bin/bash", "-c", "EMEM_BIND=\"${EMEM_BIND:-0.0.0.0:${PORT:-5051}}\" exec /usr/local/bin/emem-server"]
