# The air-gapped emem node.
#
# Built to run on hardware the operator does not own, with no route out. Every
# choice below removes something rather than adding it, and the final image
# contains one statically linked binary and nothing else: no shell, no package
# manager, no libc, nothing to pivot into if the process is ever compromised.
#
#   docker build -f crates/emem-airgap/Dockerfile -t emem-airgap .
#   docker run --rm \
#     --network none \
#     --read-only \
#     --cap-drop ALL \
#     --security-opt no-new-privileges \
#     --user 65532:65532 \
#     -v /host/in:/in:ro \
#     -v /host/out:/out \
#     -v /host/data:/data \
#     emem-airgap \
#       --input /in --output /out --data /data \
#       --profile orbital.satellite.v1 \
#       --platform nvidia.jetson-orin \
#       --observed-at 2026-08-20T09:00:00Z
#
# --network none is the one that matters most, and it is checkable rather than
# promised: the binary links no networking crate at all, so the flag and the
# build agree. Mounting the input read-only means the host does not have to
# take the node's word that it leaves the directory alone.

# No --platform pin on the FROM line.
#
# $BUILDPLATFORM and $TARGETARCH are supplied by BuildKit and are EMPTY under
# the legacy builder, where the pin then fails outright with an invalid-platform
# error before a single layer runs. A developer building for their own machine
# should not have to install buildx first, so the arch is detected below when
# the build system does not say.
# Declared before the first FROM so it can select the final stage further
# down. A build argument used in a FROM line must be in the global scope, and
# it has to be re-declared inside any stage that also reads it.
ARG ROLE=decode

FROM rust:1-bookworm AS build
ARG TARGETARCH=""
# Which half to build. Two images from one crate, on purpose.
#
# The decoder runs --cap-drop ALL and its entire claim is that it cannot do
# anything. The encoder needs a tracefs mount and the privilege to read it. One
# image with two entrypoints would mean an operator who copies the encoder's
# flags onto the decoder silently discards that claim, and nothing would
# complain. Separate images make the two postures impossible to confuse, while
# the crate keeps the schema, the identity file and the signing shared.
ARG ROLE=decode
WORKDIR /src

# musl, so the result is static and the runtime image can be empty. An Orin NX
# is aarch64; the same file builds amd64 for a laptop rehearsal.
RUN ARCH="$TARGETARCH"; \
    if [ -z "$ARCH" ]; then \
      case "$(uname -m)" in \
        aarch64|arm64) ARCH=arm64 ;; \
        x86_64|amd64)  ARCH=amd64 ;; \
        *) echo "cannot infer arch from $(uname -m); pass --build-arg TARGETARCH=arm64|amd64" >&2; exit 1 ;; \
      esac; \
      echo "TARGETARCH not set by the builder, inferred $ARCH from $(uname -m)"; \
    fi && \
    case "$ARCH" in \
      arm64) echo aarch64-unknown-linux-musl > /target.txt ;; \
      amd64) echo x86_64-unknown-linux-musl  > /target.txt ;; \
      *) echo "unsupported TARGETARCH: $ARCH (expected arm64 or amd64)" >&2; exit 1 ;; \
    esac && \
    rustup target add "$(cat /target.txt)" && \
    apt-get update && apt-get install -y --no-install-recommends musl-tools && \
    rm -rf /var/lib/apt/lists/*

# Named, not `COPY . .`.
#
# Cargo needs every workspace member's manifest to resolve the graph, so the
# whole crates/ tree comes in, but nothing else does. The blanket copy pulled
# in whatever the ignore file had not thought of, which on this repository was
# fifteen gigabytes of Python sidecars that this binary has no use for.
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/

# Only this crate. The workspace holds a REST server, a vector store and an
# ONNX runtime, and none of them belong on a spacecraft.
# blake3/no_neon on aarch64, and the reason is a build failure not a preference.
#
# blake3 is the only dependency in this graph that compiles C, for its SIMD
# paths. Cross-compiling to aarch64-musl therefore needs aarch64-linux-musl-gcc,
# which musl-tools does not provide, and the build dies with
# "failed to find tool aarch64-linux-musl-gcc". Disabling NEON selects blake3's
# portable Rust implementation, the C compile disappears, and the crate builds
# with nothing but rustup.
#
# The cost is hashing throughput. This node hashes what arrives in a directory,
# a few megabytes per pass, so the portable path is far below the noise; adding
# a cross toolchain to the image to win it back would be the wrong trade.
# Symbols are stripped by the compiler, not by binutils. The host's `strip` is
# x86_64 and cannot read an aarch64 binary ("unable to recognise the format"),
# whereas -C strip=symbols works for whatever target rustc just built.
#
# rust-lld for the cross link, for the same reason: the host's /usr/bin/ld is
# x86_64 and refuses aarch64 objects with "file in wrong format". rustup ships
# rust-lld, which links either, so no binutils cross package is needed either.
RUN FEATURES=""; \
    case "$(cat /target.txt)" in \
      aarch64-*) \
        FEATURES="--features blake3/no_neon"; \
        export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=rust-lld; \
        ;; \
    esac; \
    export RUSTFLAGS="-C strip=symbols"; \
    case "$ROLE" in \
      decode) BIN=emem-airgap ;; \
      encode) BIN=emem-encode ;; \
      *) echo "ROLE must be decode or encode, got: $ROLE" >&2; exit 1 ;; \
    esac; \
    cargo build --release --locked -p emem-airgap --bin "$BIN" \
      --target "$(cat /target.txt)" $FEATURES && \
    cp "target/$(cat /target.txt)/release/$BIN" /emem-node

# Two final stages, one per role, so the binary inside carries the name of the
# thing it is.
#
# Both images used to entrypoint at /emem-node. They take different flags and
# mean different things, and after a `docker load` the path is most of what an
# operator has to go on; identical entrypoints invite running one half's flags
# against the other. Reported from a real deployment. An unknown flag is
# refused now, so the mistake is loud rather than silent, but it should not
# have been invited.
#
# Two stages rather than one with a variable: the exec form of ENTRYPOINT does
# not expand build arguments, and a shell form would need a shell, which a
# scratch image does not have and should not gain.

# scratch, not distroless or alpine: there is nothing else this binary needs.
FROM scratch AS decode
COPY --from=build /emem-node /emem-airgap
# Numeric, because scratch has no /etc/passwd to resolve a name against. 65532
# is the conventional unprivileged id; the run command should set it too, so
# the image is safe even if someone forgets.
USER 65532:65532
ENTRYPOINT ["/emem-airgap"]

FROM scratch AS encode
COPY --from=build /emem-node /emem-encode
USER 65532:65532
ENTRYPOINT ["/emem-encode"]

# Selected by the same ROLE argument that chose which binary to build, so the
# two can never disagree about which half this image is.
FROM ${ROLE} AS final
