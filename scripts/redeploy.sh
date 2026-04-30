#!/usr/bin/env bash
# Rebuild emem-server, restore cap_net_bind_service (so the binary can
# bind :443 as a non-root systemd user unit), and bounce the unit.
#
# Why this script exists: every `cargo build --release` replaces the
# emem-server binary, which silently drops the file capability set by
# `setcap`. Without it, `axum-server::bind(0.0.0.0:443)` returns
# `Permission denied (os error 13)` and the systemd user unit
# crash-loops at ~RestartSec=2s, eating CPU and producing thousands of
# log lines per hour. Hit this exact regression on 2026-04-30 (restart
# counter 1560 in 30 minutes after a smoke-test rebuild round) — this
# script makes the redeploy ritual atomic so the cap can't be forgotten.
#
# Run from anywhere; uses absolute paths.

set -euo pipefail

REPO=/home/ubuntu/emem
BIN="$REPO/target/release/emem-server"
UNIT=emem-server.service

cd "$REPO"

echo "==> cargo build --release -p emem-cli"
cargo build --release -p emem-cli

if [[ ! -x "$BIN" ]]; then
  echo "FAIL: $BIN missing after build" >&2
  exit 1
fi

echo "==> sudo setcap 'cap_net_bind_service=+ep' $BIN"
sudo setcap 'cap_net_bind_service=+ep' "$BIN"

echo "==> verifying cap"
caps=$(getcap "$BIN")
case "$caps" in
  *cap_net_bind_service*ep*) echo "ok: $caps" ;;
  *) echo "FAIL: cap not set on $BIN (got: $caps)" >&2; exit 2 ;;
esac

echo "==> systemctl --user restart $UNIT"
systemctl --user restart "$UNIT"

# Give axum-server a beat to bind, then sanity-check.
for i in 1 2 3 4 5; do
  sleep 2
  if curl -fsS -m 5 https://emem.dev/health >/dev/null 2>&1; then
    echo "==> live at https://emem.dev/health (after ${i}x2s)"
    curl -sS https://emem.dev/health | head -c 200; echo
    exit 0
  fi
  echo "  waiting for HTTPS to come up... ($i/5)"
done

echo "FAIL: https://emem.dev/health did not come up; tail logs:" >&2
journalctl --user -u "$UNIT" -n 20 --no-pager
exit 3
