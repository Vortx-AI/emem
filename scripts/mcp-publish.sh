#!/usr/bin/env bash
# Publish emem's server.json to the official MCP Registry
# (https://registry.modelcontextprotocol.io).
#
# Use this only for the FIRST publish or for ad-hoc updates between
# release tags. The canonical path is the GitHub Actions workflow at
# .github/workflows/mcp-publish.yml, which fires on every `v*` tag
# and authenticates via GitHub OIDC — no human in the loop.
#
# This script runs from a developer laptop and uses GitHub OAuth
# device flow. You must be a member of the Vortx-AI org (case-
# insensitive) to publish under the `io.github.Vortx-AI/...`
# namespace.
#
# Versions are immutable once published. Bump server.json's version
# field before running.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

if ! command -v mcp-publisher >/dev/null 2>&1; then
  echo "==> installing mcp-publisher"
  ARCH=$(uname -m | sed 's/x86_64/amd64/;s/aarch64/arm64/')
  OS=$(uname -s | tr '[:upper:]' '[:lower:]')
  URL="https://github.com/modelcontextprotocol/registry/releases/latest/download/mcp-publisher_${OS}_${ARCH}.tar.gz"
  TMP=$(mktemp -d)
  curl -fsSL "$URL" | tar -xz -C "$TMP" mcp-publisher
  sudo mv "$TMP/mcp-publisher" /usr/local/bin/mcp-publisher
  rmdir "$TMP"
fi

echo "==> server.json:"
jq -r '"  name:    \(.name)\n  version: \(.version)\n  schema:  \(."$schema")"' server.json

echo "==> login (GitHub device flow — paste the code in your browser):"
mcp-publisher login github

echo "==> publish:"
mcp-publisher publish

NAME=$(jq -r .name server.json)
VER=$(jq -r .version server.json)
echo "==> confirm listing on the registry (may take a few seconds):"
for i in 1 2 3 4 5 6 7 8 9 10; do
  sleep 3
  if curl -fsS "https://registry.modelcontextprotocol.io/v0/servers?search=$NAME" \
       | jq -e --arg n "$NAME" --arg v "$VER" \
             '.servers[]? | select(.name==$n and .version==$v)' >/dev/null 2>&1; then
    echo "    listed: $NAME @ $VER"
    echo
    echo "github.com/mcp ingests on its own cadence and surfaces"
    echo "the listing within minutes-to-hours, not seconds."
    exit 0
  fi
  echo "    not visible yet ($i/10), waiting..."
done

echo "registry did not echo the listing back within ~30 s."
echo "publish returned 200 (otherwise the script would have exit-1'd"
echo "above) — give the index a minute and re-check at"
echo "    https://registry.modelcontextprotocol.io/v0/servers?search=$NAME"
