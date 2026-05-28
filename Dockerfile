# syntax=docker/dockerfile:1.7

# Build stage — Rust 1.91+ required by Lance 6.x (lance-namespace,
# lance-table, lance-tokenizer all MSRV 1.91; roaring 0.11 MSRV 1.90).
# Lance backs find_similar's embedded ANN index. Pinned to 1.91
# (workspace MSRV).
#
# Trixie (Debian 13, glibc 2.41) is required because ort-sys 2.0.0-rc.12
# bundles ONNX's parser.cc which references __isoc23_strtoull /
# __isoc23_strtol — symbols that only exist in glibc ≥ 2.38. On
# bookworm-slim (glibc 2.36) the link fails with "undefined reference".
FROM rust:1.91-slim-trixie AS build
ARG TARGETARCH
WORKDIR /usr/src/emem

# OpenSSL is *not* needed (we use rustls-acme), but build tools are.
# g++ is required by transitive C++ deps:
#   • ort-sys → bundled ONNX parser.cc (compiled via cc-crate)
#   • model2vec-rs → tokenizers → esaxx-rs
# protobuf-compiler (protoc) is required by Lance 6.x build scripts:
#   • lance-encoding / lance-table use prost-build, which invokes protoc
#     at build time to generate Rust bindings from .proto files.
# The runtime stage is a fresh debian:trixie-slim so it does not
# inherit g++ / protoc — this only adds ~50 MB to the build stage.
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    apt-get update && \
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        pkg-config ca-certificates g++ protobuf-compiler && \
    rm -rf /var/lib/apt/lists/*

# Install mdbook *before* the COPY layers so this layer caches across
# every Rust / docs / web edit. emem-api-rest's lib.rs embeds the
# rendered /docs/ site via `include_dir!("$CARGO_MANIFEST_DIR/../../docs/book")`,
# so the cargo build below cannot proceed until docs/book/ exists.
# `cargo install` works on every arch the build matrix supports (the
# Rust toolchain is already present); no need to fish out a prebuilt
# mdbook binary per ${TARGETARCH}.
RUN --mount=type=cache,id=cargo-registry-${TARGETARCH}-trixie,target=/usr/local/cargo/registry,sharing=locked \
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
ENV CARGO_PROFILE_RELEASE_LTO=off \
    CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16
RUN --mount=type=cache,id=cargo-registry-${TARGETARCH}-trixie,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=emem-target-${TARGETARCH}-trixie,target=/usr/src/emem/target,sharing=locked \
    cargo build --release --bin emem-server && \
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
      org.opencontainers.image.description="Earth memory protocol — content-addressed, ed25519-signed memory of every place on Earth" \
      org.opencontainers.image.url="https://emem.dev" \
      org.opencontainers.image.source="https://github.com/Vortx-AI/emem" \
      org.opencontainers.image.licenses="Apache-2.0" \
      org.opencontainers.image.vendor="Vortx-AI"

# Persistent storage volume (sled cache + identity key).
RUN mkdir -p /var/emem && chown -R emem:emem /var/emem
VOLUME ["/var/emem"]

USER emem
ENV EMEM_BIND=0.0.0.0:5051 \
    EMEM_DATA=/var/emem \
    RUST_LOG=info

# 5051 — plain HTTP for local / proxy deployments.
# 443  — HTTPS via rustls + Let's Encrypt ACME (set EMEM_TLS_DOMAINS).
EXPOSE 5051 443

# Lightweight container healthcheck against /health. Use bash builtin
# /dev/tcp so the runtime image stays free of curl / wget.
HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD bash -c '</dev/tcp/127.0.0.1/${EMEM_BIND##*:}' || exit 1

ENTRYPOINT ["/usr/local/bin/emem-server"]
