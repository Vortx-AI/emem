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

HEALTH="${EMEM_WATCHDOG_URL:-http://127.0.0.1:5051/health}"
THRESHOLD="${EMEM_WATCHDOG_THRESHOLD:-2}"
TIMEOUT="${EMEM_WATCHDOG_TIMEOUT:-8}"
STATE="${XDG_RUNTIME_DIR:-/tmp}/emem_watchdog_fails"

fails=$(cat "$STATE" 2>/dev/null || echo 0)
case "$fails" in ''|*[!0-9]*) fails=0 ;; esac

if curl -fsS -m "$TIMEOUT" "$HEALTH" >/dev/null 2>&1; then
  # Healthy — reset the counter.
  [ "$fails" -ne 0 ] && echo "emem-watchdog: /health recovered after $fails miss(es)"
  echo 0 >"$STATE"
  exit 0
fi

fails=$((fails + 1))
echo "$fails" >"$STATE"
echo "emem-watchdog: /health did not respond within ${TIMEOUT}s (${fails}/${THRESHOLD})"

# Snapshot the wedged process before restarting it. Every restart so far
# has destroyed the evidence, which is why the stall root cause is still
# unknown after five incidents. No gdb on this box, but /proc alone says
# a lot: per-thread state + wchan + syscall shows whether the tokio
# workers are parked in a futex (lock cycle), stuck in disk I/O, or
# genuinely idle while the accept loop starves, and the socket-state
# summary quantifies the CLOSE-WAIT pileup seen in earlier incidents.
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

if [ "$fails" -ge "$THRESHOLD" ]; then
  echo "emem-watchdog: runtime appears stalled; restarting emem-server.service"
  snapshot_wedge
  systemctl --user restart emem-server.service
  echo 0 >"$STATE"
fi
