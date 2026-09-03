#!/usr/bin/env bash
# emem health watchdog.
#
# The responder has silently stalled three times (2026-05-31, -06-12, -06-15):
# the process stays alive and listening on :443, but the tokio runtime wedges
# (CLOSE-WAIT sockets pile up, the accept backlog saturates), so every request
# times out for hours until a human restarts it. See the project_outage_*
# memories. This watchdog turns a multi-hour silent outage into a ~2-minute
# self-heal: it polls the loopback /health (same runtime as :443, so a stall
# shows here too) and restarts the service after THRESHOLD consecutive misses.
#
# Run by emem-watchdog.timer every 60s. State (consecutive-fail counter) lives
# in $XDG_RUNTIME_DIR so it survives between timer firings but resets on reboot.
set -u

# LIVENESS is probed with /live, not /health, and the difference is the whole
# point of this file.
#
# /health reports corpus statistics and pays for a storage.scan_index pass to do
# it. Measured on this host: 80 ms idle, 905 ms under twenty concurrent recalls,
# and it degrades further under a materialize storm or a cold bake. /live touches
# no storage at all and costs about 0.5 ms under the same load, a difference of
# roughly three orders of magnitude.
#
# Both run on the same tokio runtime, so a genuine runtime stall stops BOTH. That
# makes /live strictly the better liveness signal: it still fails when the server
# is actually wedged, and it does not fail merely because a corpus scan got slow
# while the server was serving traffic normally.
#
# This matters because for 235 snapshots this watchdog probed /health and killed
# the process when a scan exceeded 8 s. Every one of those snapshots shows a
# socket profile indistinguishable from a healthy host, normal memory, and every
# thread sleeping rather than blocked. An unknown number of the "wedges" in that
# record are very likely this watchdog SIGKILLing a working server.
#
# /health is still probed, as a DIAGNOSTIC only. When /live answers and /health
# does not, that is recorded and nothing is restarted, which is exactly the case
# that was previously indistinguishable from a stall.
LIVE="${EMEM_WATCHDOG_URL:-http://127.0.0.1:5051/live}"
HEALTH="${EMEM_WATCHDOG_HEALTH_URL:-http://127.0.0.1:5051/health}"
THRESHOLD="${EMEM_WATCHDOG_THRESHOLD:-2}"
TIMEOUT="${EMEM_WATCHDOG_TIMEOUT:-8}"
HEALTH_TIMEOUT="${EMEM_WATCHDOG_HEALTH_TIMEOUT:-20}"
STATE="${XDG_RUNTIME_DIR:-/tmp}/emem_watchdog_fails"

snapshot_wedge() {
  pid=$(systemctl --user show -p MainPID --value emem-server.service 2>/dev/null)
  [ -n "$pid" ] && [ "$pid" != 0 ] && [ -d "/proc/$pid" ] || return 0
  dir="${EMEM_WEDGE_DIR:-/home/ubuntu/emem/var/wedge}"
  mkdir -p "$dir"
  out="$dir/wedge-$(date -u +%Y%m%dT%H%M%SZ).txt"
  {
    echo "== emem-watchdog wedge snapshot $(date -u '+%Y-%m-%d %H:%M:%S UTC') pid=$pid =="
    echo "-- process --"
    grep -E 'State|Threads|VmRSS' "/proc/$pid/status" 2>/dev/null
    echo "-- sockets --"
    ss -tan 2>/dev/null | awk 'NR>1{c[$1]++} END{for (s in c) print s, c[s]}'
    echo "-- threads (tid comm state wchan syscall) --"
    for t in "/proc/$pid/task"/*; do
      tid=${t##*/}
      printf '%s %s state=%s wchan=%s syscall=%s\n' \
        "$tid" \
        "$(cat "$t/comm" 2>/dev/null)" \
        "$(awk '/^State/{print $2}' "$t/status" 2>/dev/null)" \
        "$(cat "$t/wchan" 2>/dev/null)" \
        "$(cut -d' ' -f1 "$t/syscall" 2>/dev/null)"
    done
    # The 2026-07-03 19:25 snapshot showed every thread parked (state S,
    # wchan 0) across TWO tokio runtimes — thread states alone cannot say
    # where they parked, only backtraces can. gdb attach on an
    # already-wedged process costs nothing the restart wasn't about to
    # destroy anyway.
    # ptrace_scope is 1 on this host, which blocks attaching to a process
    # that is not your child. The watchdog is not emem-server's parent
    # (systemd is), so the unprivileged gdb below failed on EVERY wedge
    # from 2026-07-03 to 2026-07-20: 231 snapshots, zero backtraces, each
    # one recording only "Could not attach to process". That is why five
    # incidents stayed "cause unknown" while the data to explain them was
    # being thrown away. sudo is passwordless for this user, so attach
    # through it and keep the unprivileged path as a fallback.
    if command -v gdb >/dev/null 2>&1; then
      echo "-- gdb thread backtraces --"
      if sudo -n true 2>/dev/null; then
        sudo -n timeout 90 gdb -p "$pid" -batch -ex "set pagination off" \
          -ex "thread apply all bt 30" 2>&1
      else
        echo "(no passwordless sudo; ptrace_scope=$(cat /proc/sys/kernel/yama/ptrace_scope 2>/dev/null) will likely block this)"
        timeout 90 gdb -p "$pid" -batch -ex "set pagination off" \
          -ex "thread apply all bt 30" 2>&1
      fi
    fi
  } >"$out" 2>&1
  echo "emem-watchdog: wedge snapshot written to $out"
}

fails=$(cat "$STATE" 2>/dev/null || echo 0)
case "$fails" in ''|*[!0-9]*) fails=0 ;; esac

if curl -fsS -m "$TIMEOUT" "$LIVE" >/dev/null 2>&1; then
  # The runtime is serving. Reset, then note whether the expensive probe is
  # struggling: that is a load signal worth having, not a reason to kill.
  [ "$fails" -ne 0 ] && echo "emem-watchdog: /live recovered after $fails miss(es)"
  echo 0 >"$STATE"
  # Second signal: the store can wedge while /live still answers, because
  # /live touches no storage. On 2026-09-02 that state lasted an hour
  # (21:29 to 22:28 UTC): every storage-touching request hung, CI's example
  # gate timed out, and this script kept reporting healthy until the runtime
  # itself died. scripts/storage_liveness.py asserts the SHAPE (live fast,
  # warm read slow; both slow is load and reads as undetermined, exit 2).
  # Three consecutive confirmations and a cold-start grace period stand
  # between it and a restart; EMEM_WATCHDOG_DRY_RUN=1 logs instead of acting.
  STORAGE_STATE="${XDG_RUNTIME_DIR:-/tmp}/emem_watchdog_storage_fails"
  STORAGE_THRESHOLD="${EMEM_WATCHDOG_STORAGE_THRESHOLD:-3}"
  STORAGE_GRACE_S="${EMEM_WATCHDOG_STORAGE_GRACE_S:-300}"
  STORAGE_PROBE="${EMEM_WATCHDOG_STORAGE_PROBE:-/usr/bin/python3 /home/ubuntu/emem/scripts/storage_liveness.py --origin http://127.0.0.1:5051}"
  sfails=$(cat "$STORAGE_STATE" 2>/dev/null || echo 0)
  case "$sfails" in ''|*[!0-9]*) sfails=0 ;; esac
  started=$(systemctl --user show -p ActiveEnterTimestampMonotonic --value emem-server.service 2>/dev/null)
  now_mono=$(awk '{printf "%d", $1*1000000}' /proc/uptime)
  if [ -n "$started" ] && [ "$started" != 0 ] && [ $((now_mono - started)) -lt $((STORAGE_GRACE_S * 1000000)) ]; then
    echo 0 >"$STORAGE_STATE"
  else
    $STORAGE_PROBE >/dev/null 2>&1; src=$?
    if [ "$src" -eq 1 ]; then
      sfails=$((sfails + 1)); echo "$sfails" >"$STORAGE_STATE"
      echo "emem-watchdog: /live OK but storage is wedged (${sfails}/${STORAGE_THRESHOLD}): warm read stuck while /live answers"
      if [ "$sfails" -ge "$STORAGE_THRESHOLD" ]; then
        echo "emem-watchdog: storage wedged for ${sfails} consecutive checks; restarting emem-server.service"
        echo 0 >"$STORAGE_STATE"
        snapshot_wedge
        if [ "${EMEM_WATCHDOG_DRY_RUN:-0}" = 1 ]; then echo "emem-watchdog: DRY RUN, not restarting"; else systemctl --user restart emem-server.service; fi
        exit 0
      fi
    else
      [ "$sfails" -ne 0 ] && [ "$src" -eq 0 ] && echo "emem-watchdog: storage recovered after $sfails wedged check(s)"
      [ "$src" -eq 0 ] && echo 0 >"$STORAGE_STATE"
    fi
  fi
  if ! curl -fsS -m "$HEALTH_TIMEOUT" "$HEALTH" >/dev/null 2>&1; then
    echo "emem-watchdog: /live OK but /health exceeded ${HEALTH_TIMEOUT}s — corpus scan is slow under load, NOT restarting (this is the case that used to trigger a false restart)"
  fi
  exit 0
fi

fails=$((fails + 1))
echo "$fails" >"$STATE"
echo "emem-watchdog: /live did not respond within ${TIMEOUT}s (${fails}/${THRESHOLD}) — this is a real runtime stall, /live touches no storage"

# Snapshot the wedged process before restarting it. Every restart so far
# has destroyed the evidence, which is why the stall root cause is still
# unknown after five incidents. No gdb on this box, but /proc alone says
# a lot: per-thread state + wchan + syscall shows whether the tokio
# workers are parked in a futex (lock cycle), stuck in disk I/O, or
# genuinely idle while the accept loop starves, and the socket-state
# summary quantifies the CLOSE-WAIT pileup seen in earlier incidents.

if [ "$fails" -ge "$THRESHOLD" ]; then
  echo "emem-watchdog: runtime appears stalled; restarting emem-server.service"
  snapshot_wedge
  systemctl --user restart emem-server.service
  echo 0 >"$STATE"
fi
