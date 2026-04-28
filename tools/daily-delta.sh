#!/usr/bin/env bash
set -euo pipefail
# Daily delta against https://emem.dev — captures contributor leaderboard,
# /metrics counters, and runs realdemo so the corpus stays fresh.
cd /home/ubuntu/emem
STAMP=$(date -u +%Y%m%d_%H%M%S)
DEMO_DIR="var/demos/daily_${STAMP}"
mkdir -p "$DEMO_DIR"
{
  echo "=== contributors ==="
  curl -s https://emem.dev/v1/contributors
  echo
  echo "=== agent_stats ==="
  curl -s https://emem.dev/v1/agent_stats
  echo
  echo "=== metrics ==="
  curl -s https://emem.dev/metrics
  echo
  echo "=== health ==="
  curl -s https://emem.dev/health
} > "$DEMO_DIR/snapshot.txt"
./target/release/emem-realdemo https://emem.dev "$DEMO_DIR/realrun" || true
echo "daily snapshot saved to $DEMO_DIR"
