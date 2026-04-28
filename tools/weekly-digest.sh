#!/usr/bin/env bash
set -euo pipefail
# Weekly digest of agent activity on https://emem.dev. Captures the
# week's snapshot of contributors, agent_stats, and metrics into a single
# rolling file under var/digests/. The intent is a low-volume archive
# that can be diffed week-over-week to see how usage evolves —
# ClickHouse / Loki are the right destination once they're wired in,
# but a flat-file digest is a fine zero-dependency starting point.
cd /home/ubuntu/emem
STAMP=$(date -u +%Y-%m-%d)
DIGEST_DIR="var/digests"
mkdir -p "$DIGEST_DIR"
OUT="$DIGEST_DIR/digest_${STAMP}.txt"
{
  echo "# emem.dev weekly digest — ${STAMP}"
  echo
  echo "## contributors leaderboard"
  curl -s https://emem.dev/v1/contributors | python3 -m json.tool 2>/dev/null || echo "(failed to fetch)"
  echo
  echo "## agent_stats"
  curl -s https://emem.dev/v1/agent_stats | python3 -m json.tool 2>/dev/null || echo "(failed to fetch)"
  echo
  echo "## metrics"
  curl -s https://emem.dev/metrics
  echo
  echo "## health"
  curl -s https://emem.dev/health | python3 -m json.tool 2>/dev/null || echo "(failed to fetch)"
} > "$OUT"
echo "weekly digest saved to $OUT"
