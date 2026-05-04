# syntax=docker/dockerfile:1.7

# Build stage — Rust 1.88+ required by transitive deps (time 0.3.47,
# icu_* 2.2.0). Pinned to 1.88 (workspace MSRV).
FROM rust:1.88-slim-bookworm AS build
ARG TARGETARCH
WORKDIR /usr/src/emem

# OpenSSL is *not* needed (we use rustls-acme), but build tools are.
# g++ is required by transitive C++ deps that came in with the
# 0.0.3 transformer router (model2vec-rs → tokenizers → esaxx-rs);
# the runtime stage is a fresh debian:bookworm-slim so it does not
# inherit g++ — this only adds ~50 MB to the build stage.
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    apt-get update && \
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        pkg-config ca-certificates g++ && \
    rm -rf /var/lib/apt/lists/*

# Cache `cargo fetch` against the workspace manifest before pulling in
# source — keeps re-builds fast when only Rust files change.
# crates/emem-api-rest pulls files from web/, docs/, examples/ via
# include_str!() so they have to ride along in the build context.
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY web/ web/
COPY docs/ docs/
COPY examples/ examples/
# Root-level markdown is include_str!'d directly by emem-api-rest.
# Without these the build fails with `couldn't read PRIVACY.md`.
COPY PRIVACY.md TERMS.md SUPPORT.md SECURITY.md ./
# BuildKit cache-mount IDs are scoped by ${TARGETARCH} so the parallel
# linux/amd64 + linux/arm64 build jobs don't race each other unpacking
# the same crate into a shared cache (`File exists (os error 17)` on
# `.cargo-ok`). Each arch keeps its own warm cache across runs.
RUN --mount=type=cache,id=cargo-registry-${TARGETARCH},target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=emem-target-${TARGETARCH},target=/usr/src/emem/target,sharing=locked \
    cargo build --release --bin emem-server && \
    cp target/release/emem-server /usr/local/bin/emem-server

# Runtime stage — minimal Debian, non-root, with cap_net_bind_service
# pre-applied so EMEM_BIND=0.0.0.0:443 works without docker run --cap-add.
FROM debian:bookworm-slim AS runtime
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
