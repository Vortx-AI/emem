#!/bin/bash
# Capture a real ftrace window for the four layers host.counters.v1 requires,
# one private tracefs instance per layer, for emem-demo-device to sign.
#
#   sudo scripts/manual/capture_ftrace.sh <outdir> [seconds]
#
# Writes scheduler.log, memory.log, storage.log, network.log (raw `trace`
# output, trace_clock=mono) and chowns them to the invoking user. Each
# instance has its own small ring buffer and is removed on exit, so the
# global tracer and anyone else's instances are left as they were.
set -euo pipefail
out=${1:?usage: capture_ftrace.sh <outdir> [seconds]}
secs=${2:-2}
T=/sys/kernel/tracing
[ -d "$T/instances" ] || { echo "tracefs is not mounted at $T" >&2; exit 1; }
mkdir -p "$out"

declare -A EVENTS=(
  [scheduler]="sched/sched_switch"
  [memory]="kmem/mm_page_alloc"
  [storage]="block/block_rq_complete"
  [network]="net/net_dev_xmit net/netif_receive_skb"
)
made=()
cleanup() { for d in "${made[@]}"; do rmdir "$d" 2>/dev/null || true; done; }
trap cleanup EXIT

for layer in "${!EVENTS[@]}"; do
  d="$T/instances/emem-demo-$layer-$$"
  mkdir "$d"; made+=("$d")
  echo mono > "$d/trace_clock"
  echo 1024 > "$d/buffer_size_kb"
  for ev in ${EVENTS[$layer]}; do
    [ -e "$d/events/$ev/enable" ] || { echo "event $ev is not available" >&2; exit 1; }
    echo 1 > "$d/events/$ev/enable"
  done
done
for d in "${made[@]}"; do echo 1 > "$d/tracing_on"; done
# Real work in every layer during the window, so no log is empty by accident.
( dd if=/dev/zero of="$out/.io" bs=1M count=8 oflag=direct status=none; rm -f "$out/.io" ) &
base=${EMEM_BASE:-https://emem.dev}
getent hosts "$(echo "$base" | sed -E 's#^https?://##; s#[/:].*##')" >/dev/null || true
curl -s -o /dev/null "$base/live" || true
sleep "$secs"
wait
for d in "${made[@]}"; do echo 0 > "$d/tracing_on"; done
for layer in "${!EVENTS[@]}"; do
  cat "$T/instances/emem-demo-$layer-$$/trace" > "$out/$layer.log"
done
if [ -n "${SUDO_USER:-}" ]; then chown "$SUDO_USER" "$out" "$out"/*.log; fi
wc -l "$out"/*.log
