#!/bin/sh
# Take one machine from nothing to a signed, verified custody record.
#
#   ./quickstart.sh [workdir]
#
# It fetches the two published images unless they are already here, makes the
# four directories the node expects, runs the decoder once so the node has an identity, runs the encoder
# sidecar against the same identity, runs the decoder again so a payload can
# cite a trace, and then verifies what landed. Nothing here needs Rust, a
# clone of this repository, or network access from the node itself.
#
# Written for /bin/sh so it runs on a BusyBox initramfs as well as a laptop.
# Every step prints the command it is about to run, because the point of this
# script is to be read once and then not needed.
set -eu

WORK="${1:-./emem-airgap-quickstart}"
DECODER="${EMEM_AIRGAP_IMAGE:-ghcr.io/vortx-ai/emem-airgap:latest}"
ENCODER="${EMEM_ENCODE_IMAGE:-ghcr.io/vortx-ai/emem-encode:latest}"
# The substrate profile and platform id this rehearsal writes under. Override
# both for real hardware: exec.trace.v1 is the generic-Linux profile and
# generic.linux-host is the vendorless platform, so a Jetson would say
# orbital.satellite.v1 and nvidia.jetson-orin.
PROFILE="${EMEM_AIRGAP_PROFILE:-exec.trace.v1}"
PLATFORM="${EMEM_AIRGAP_PLATFORM:-generic.linux-host}"

say() { printf '\n\033[1m%s\033[0m\n' "$*"; }
run() { printf '  $ %s\n' "$*"; "$@"; }

if ! command -v docker >/dev/null 2>&1; then
  echo "docker is not on PATH. This script only needs docker; see the README" >&2
  echo "for building the two binaries from source instead." >&2
  exit 1
fi

say "1. directories"
# in/  payloads the host drops here, never modified by the node
# out/ custody records, the only thing that leaves
# data/ node_identity.json, must survive a restart
# traces/ what the encoder writes and the decoder reads
for d in in out data traces; do
  run mkdir -p "$WORK/$d"
done
# The images run as uid 65532. Give that uid somewhere to write, without
# assuming this machine has a 65532 user.
chmod 777 "$WORK/out" "$WORK/data" "$WORK/traces"
printf 'a payload that stands in for a science frame\n' > "$WORK/in/frame_001.tif"

say "2. images"
# Pull only what is not already here. A node that received its images on a USB
# stick via `docker load` has them and must not be sent to a registry it
# cannot reach, which is the whole situation this crate is built for.
for img in "$DECODER" "$ENCODER"; do
  if docker image inspect "$img" >/dev/null 2>&1; then
    printf '  already present: %s\n' "$img"
  else
    run docker pull "$img"
  fi
done

say "3. decode once, which creates this node's identity"
# --network none is not decoration: the binary links no networking crate, so
# the flag agrees with the image rather than merely being asked of it.
run docker run --rm --network none \
  -v "$(cd "$WORK/in" && pwd):/in:ro" \
  -v "$(cd "$WORK/out" && pwd):/out" \
  -v "$(cd "$WORK/data" && pwd):/data" \
  "$DECODER" \
  --input /in --output /out --data /data \
  --profile "$PROFILE" --platform "$PLATFORM" \
  --observed-at "$(date -u +%Y-%m-%dT%H:%M:%SZ)"

say "4. capture one execution trace, signed by the SAME identity"
# The sidecar shares --data and never creates a key: two halves of one node
# must sign as one node. Without tracefs mounted it captures what it can and
# says plainly which layers were absent; it never invents one.
run docker run --rm --network none \
  -v "$(cd "$WORK/traces" && pwd):/traces" \
  -v "$(cd "$WORK/data" && pwd):/data" \
  -v "$(cd "$WORK/in" && pwd):/payloads:ro" \
  "$ENCODER" \
  --out /traces --data /data --payloads /payloads \
  --profile "$PROFILE" --platform "$PLATFORM"

say "5. decode again, so a payload the encoder watched can cite its trace"
run docker run --rm --network none \
  -v "$(cd "$WORK/in" && pwd):/in:ro" \
  -v "$(cd "$WORK/out" && pwd):/out" \
  -v "$(cd "$WORK/data" && pwd):/data" \
  -v "$(cd "$WORK/traces" && pwd):/traces:ro" \
  "$DECODER" \
  --input /in --output /out --data /data --traces /traces \
  --profile "$PROFILE" --platform "$PLATFORM" \
  --observed-at "$(date -u +%Y-%m-%dT%H:%M:%SZ)"

say "6. check the record against the payload it names"
RECORD=$(ls "$WORK"/out/frame_001.tif.*.custody.json | head -1)
run docker run --rm --network none \
  -v "$(cd "$WORK/out" && pwd):/out:ro" \
  -v "$(cd "$WORK/in" && pwd):/in:ro" \
  "$DECODER" verify "/out/$(basename "$RECORD")" /in/frame_001.tif

say "what you have now"
cat <<EOF
  $WORK/out/
    frame_001.tif.<node>.custody.json   signed, verifies on its own
    join_request.<node>.json            carry this out to be endorsed
    run.<node>.json                     what the run did, and every skip

  The payload never left $WORK/in. Only the record did.

  Next:
    * Read crates/emem-airgap/README.md for what a custody record does and
      does NOT claim. It is deliberately weaker than an execution trace and
      says so in its own signed body.
    * If the trace was not cited, the run report says which layers were
      missing. On a real host, mount tracefs into the encoder:
        --cap-add DAC_READ_SEARCH -v /sys/kernel/tracing:/sys/kernel/tracing:ro
    * Run the encoder with --interval 60 to keep capturing, each window
      chained to the one before it.
EOF
