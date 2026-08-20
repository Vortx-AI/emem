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

FROM --platform=$BUILDPLATFORM rust:1-bookworm AS build
ARG TARGETARCH
WORKDIR /src

# musl, so the result is static and the runtime image can be empty. An Orin NX
# is aarch64; the same file builds amd64 for a laptop rehearsal.
RUN case "$TARGETARCH" in \
      arm64) echo aarch64-unknown-linux-musl > /target.txt ;; \
      amd64) echo x86_64-unknown-linux-musl  > /target.txt ;; \
      *) echo "unsupported TARGETARCH: $TARGETARCH" >&2; exit 1 ;; \
    esac && \
    rustup target add "$(cat /target.txt)" && \
    apt-get update && apt-get install -y --no-install-recommends musl-tools && \
    rm -rf /var/lib/apt/lists/*

COPY . .
# Only this crate. The workspace holds a REST server, a vector store and an
# ONNX runtime, and none of them belong on a spacecraft.
RUN cargo build --release --locked -p emem-airgap \
      --target "$(cat /target.txt)" && \
    cp "target/$(cat /target.txt)/release/emem-airgap" /emem-airgap && \
    strip /emem-airgap

# scratch, not distroless or alpine: there is nothing else this binary needs.
FROM scratch
COPY --from=build /emem-airgap /emem-airgap
# Numeric, because scratch has no /etc/passwd to resolve a name against. 65532
# is the conventional unprivileged id; the run command should set it too, so
# the image is safe even if someone forgets.
USER 65532:65532
ENTRYPOINT ["/emem-airgap"]
