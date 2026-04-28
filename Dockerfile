# syntax=docker/dockerfile:1.7

# Build stage — Rust 1.88+ required by transitive deps (time 0.3.47,
# icu_* 2.2.0). Pinned to 1.91 for headroom on Bookworm slim.
FROM rust:1.91-slim-bookworm AS build
WORKDIR /usr/src/emem

# OpenSSL is *not* needed (we use rustls-acme), but build tools and a
# few tiny C deps for sled/blake3-asm are.
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    apt-get update && \
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        pkg-config ca-certificates && \
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
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/src/emem/target \
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
