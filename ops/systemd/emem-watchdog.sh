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

if [ "$fails" -ge "$THRESHOLD" ]; then
  echo "emem-watchdog: runtime appears stalled; restarting emem-server.service"
  systemctl --user restart emem-server.service
  echo 0 >"$STATE"
fi
