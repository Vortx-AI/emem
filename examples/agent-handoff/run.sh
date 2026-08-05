#!/usr/bin/env bash
# A session checkpoint outlives its agent: run 1 parks state as one
# signed memory file, run 2 (a different identity, a different
# process, later) resumes from it and verifies it trusted nobody.
#
# Everything runs against a THROWAWAY local responder with its own
# temp data dir, so this script touches no shared node and no real
# identity. Requires: the emem-server binary (cargo build --release
# --bin emem-server) and the Python SDK with the signing extra
# (pip install -e "sdks/emem-py[signing]").
set -euo pipefail

REPO="$(cd "$(dirname "$0")/../.." && pwd)"
WORK="$(mktemp -d)"
PORT="${PORT:-5099}"
BASE="http://127.0.0.1:${PORT}"
trap 'kill "${NODE_PID:-}" 2>/dev/null || true; rm -rf "$WORK"' EXIT

echo "==> boot a throwaway responder (own data dir, own key)"
EMEM_DATA="$WORK/node-data" EMEM_BIND="127.0.0.1:${PORT}" \
  EMEM_OVERTURE_SKIP_WARMUP=1 \
  "$REPO/target/release/emem-server" >"$WORK/node.log" 2>&1 &
NODE_PID=$!
for i in $(seq 1 20); do
  sleep 1
  curl -fsS -m 2 "$BASE/health" >/dev/null 2>&1 && break
done
curl -fsS -m 2 "$BASE/health" >/dev/null

echo "==> run 1: do some work, then park the state as ONE signed file"
mkdir -p "$WORK/agent-run-1"
cat >"$WORK/checkpoint.md" <<'CHECKPOINT'
# checkpoint: survey-task, end of run 1

step_completed: 3 of 7
next_action: wire the resolve surface; the parser is already written
findings:
  - the geocoder cache must not be shared between processes
  - anchors are best-effort by owner decision
resume_hint: read docs/plans/field-tokens.md section 4 first
CHECKPOINT

RUN1_ID=$(HOME="$WORK/agent-run-1" ememdev whoami | python3 -c \
  "import sys,json; print(json.load(sys.stdin)['pubkey_short'])")
CHECKPOINT_PATH="/memories/by_attester/${RUN1_ID}/checkpoint-survey-task.md"
WRITE=$(HOME="$WORK/agent-run-1" ememdev write \
  --path "$CHECKPOINT_PATH" \
  --kind episodic \
  --base-url "$BASE" \
  --body-file "$WORK/checkpoint.md")
FILE_CID=$(echo "$WRITE" | python3 -c "import sys,json; print(json.load(sys.stdin)['file_cid'])")
echo "    parked at $CHECKPOINT_PATH"
echo "    file_cid  $FILE_CID"
echo "    the ONE LINE run 1 leaves behind (in a report, an issue, anywhere):"
echo "    $CHECKPOINT_PATH  cid $FILE_CID"

echo "==> run 2: a DIFFERENT identity resumes, trusting nobody"
curl -s -X POST "$BASE/mcp" -H 'content-type: application/json' -d "{
  \"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",
  \"params\":{\"name\":\"emem_memory_view\",\"arguments\":{\"path\":\"$CHECKPOINT_PATH\"}}}" \
  >"$WORK/resume.json"
python3 - "$WORK" "$FILE_CID" <<'PY'
import json, sys
work, want_cid = sys.argv[1], sys.argv[2]
d = json.load(open(f"{work}/resume.json"))
body = json.loads(d["result"]["content"][0]["text"])
assert body["file_cid"] == want_cid, "file_cid drifted: the bytes are not what run 1 parked"
assert "next_action" in body["content"], "checkpoint content did not round-trip"
json.dump({"receipt": body["receipt"]}, open(f"{work}/receipt.json", "w"))
print("    content round-tripped; file_cid matches what run 1 parked")
PY
VALID=$(curl -s -X POST "$BASE/v1/verify_receipt" \
  -H 'content-type: application/json' --data-binary @"$WORK/receipt.json" \
  | python3 -c "import sys,json; print(json.load(sys.stdin)['signature_valid'])")
echo "    signature_valid: $VALID"
[ "$VALID" = "True" ] || [ "$VALID" = "true" ]

echo "==> done: the state outlived its session, and the successor verified it"
