# Go-live checklist

Everything I (Claude / agent) could prepare from inside the repo is
done. The remaining steps are clicks on github.com / huggingface.co /
hub.docker.com that need a human session. Follow them in order — each
step is independently safe and idempotent.

## Status snapshot (when this doc was written)

| channel                  | state             | gate                                                       |
|--------------------------|-------------------|------------------------------------------------------------|
| `github.com/Vortx-AI/emem` | pushed (private) | needs visibility flip                                      |
| `ghcr.io/vortx-ai/emem`  | image not built   | needs publish.yml workflow run on a public repo            |
| `docker.io/vortxai/emem` | not published     | needs `PUBLISH_DOCKERHUB=true` + DH credentials secrets    |
| HuggingFace Space         | does not exist    | needs Space creation + `PUBLISH_HF_SPACE=true` + token     |
| MCP Server Registry       | not submitted     | needs `mcp-publisher publish` after image is public        |
| awesome-mcp-servers       | not submitted     | needs PR (patch ready)                                     |

## Step 1 — flip the GitHub repo to public

```
github.com/Vortx-AI/emem → Settings → General →
  scroll to "Danger Zone" →
  "Change repository visibility" → "Make public" →
  type the repo name to confirm.
```

This single click unblocks everything else: GHCR images become
pull-anonymously, the README badges resolve, the registry patches
have a working URL, and `api.github.com/repos/Vortx-AI/emem` starts
returning 200 instead of 404.

## Step 2 — set repo Actions variables and secrets

`github.com/Vortx-AI/emem → Settings → Secrets and variables → Actions`

**Variables** (no secrecy needed, read-only):

| name                  | value                | needed for       |
|-----------------------|----------------------|------------------|
| `PUBLISH_HF_SPACE`    | `true`               | HF Space sync    |
| `HF_SPACE_REPO_ID`    | `vortx-ai/emem`      | HF Space target  |
| `PUBLISH_DOCKERHUB`   | `true` *(optional)*  | Docker Hub mirror|

**Secrets** (encrypted):

| name                | value                                                            |
|---------------------|------------------------------------------------------------------|
| `HUGGINGFACE_TOKEN` | new HF token, **Write** scope on the Space — created AFTER the leaked one is rotated |
| `DOCKERHUB_USERNAME`| `vortxai` *(optional)*                                           |
| `DOCKERHUB_TOKEN`   | Docker Hub PAT, "Public Repo: Read & Write" scope *(optional)*  |

`GITHUB_TOKEN` is auto-provided to workflows — no need to add it.

## Step 3 — kick the publish workflow

The workflow is wired to `push` on `main` and `tag` on `v*`. It will
*also* run automatically the moment Step 1 + Step 2 are in place
because the next push triggers it. If you want to force a run without
a code change:

```
github.com/Vortx-AI/emem/actions/workflows/publish.yml →
  "Run workflow" → branch: main → "Run workflow"
```

After ~3 min you'll see the multi-arch image published at
`ghcr.io/vortx-ai/emem:latest` and `:<short-sha>`.

## Step 4 — make the GHCR package public

By default GHCR packages inherit the repo's visibility, but the
package-level setting can lag. Verify:

```
github.com/orgs/Vortx-AI/packages/container/emem/settings →
  scroll to "Danger Zone" →
  "Change package visibility" → "Public" → confirm.
```

Then anonymous pulls work:

```bash
docker pull ghcr.io/vortx-ai/emem:latest
```

## Step 5 — create the HuggingFace Space

```
huggingface.co/new-space →
  Owner: vortx-ai
  Space name: emem
  Space SDK: Docker (NOT Gradio / Streamlit)
  Visibility: Public
  Hardware: CPU basic (free) is enough for a working demo
```

After the empty Space is created, the next push to `main` (or a
manual workflow re-run) will sync `huggingface-space/` into the Space
via `huggingface-cli upload`. The Space then builds the embedded
Dockerfile, which simply re-uses the GHCR image — so it goes live in
~30 s with no Rust toolchain on HF's side.

If you want to test the upload locally:

```bash
pip install "huggingface_hub[cli]>=0.24"
HF_TOKEN=hf_xxx huggingface-cli upload \
  vortx-ai/emem huggingface-space/ . --repo-type=space
```

## Step 6 — submit to the official MCP Server Registry

The Registry replaces the old `modelcontextprotocol/servers` PR
mechanism. It pulls metadata from `server.json` at the repo root
(already committed) and verifies ownership via the
`io.modelcontextprotocol.server.name` LABEL on the GHCR image (also
already in the Dockerfile).

```bash
# Install the publisher CLI
curl -L "https://github.com/modelcontextprotocol/registry/releases/latest/download/mcp-publisher_$(uname -s | tr '[:upper:]' '[:lower:]')_$(uname -m | sed 's/x86_64/amd64/;s/aarch64/arm64/').tar.gz" \
  | tar xz mcp-publisher
sudo mv mcp-publisher /usr/local/bin/

# Authenticate (GitHub OAuth — the registry maps io.github.vortx-ai/*
# to the Vortx-AI GitHub org)
mcp-publisher login

# Publish
cd /home/ubuntu/emem
mcp-publisher publish
```

You'll then appear at
[`registry.modelcontextprotocol.io/?search=emem`](https://registry.modelcontextprotocol.io/?search=emem).

Re-run `mcp-publisher publish` whenever `version` in `server.json`
bumps.

## Step 7 — submit the awesome-mcp-servers PR

Patch is at `docs/registries/awesome-mcp-servers-emem.patch`.

```bash
# Fork punkpeye/awesome-mcp-servers to your account on github.com
# Then locally:
git clone git@github.com:<your-fork>/awesome-mcp-servers.git
cd awesome-mcp-servers
git checkout -b add-emem
git apply /path/to/emem/docs/registries/awesome-mcp-servers-emem.patch
git commit -am "Add Vortx-AI/emem to Environment & Nature + Location Services"
git push origin add-emem
# Open PR via the github.com UI; CI is light (markdown lint only).
```

Suggested PR body (copy-paste ready):

```
emem is an Apache-2.0 Rust MCP server (also on GHCR + HuggingFace
Space) that gives AI agents content-addressed, ed25519-signed memory
of every place on Earth. 26 tools, 47 read primitives, 68 algorithms
implementing real published formulas (NWS Rothfusz heat-index,
Fosberg FFWI, FAO-56 vapour-pressure deficit, Imhoff UHI regression,
GHSL dasymetric population, Haurwitz/Kasten clear-sky GHI). Reads
from open data only — Copernicus DEM, JRC Surface Water, Sentinel-2
L2A, OSM, Overture Maps, MET Norway. Hosted at https://emem.dev/mcp,
GHCR at ghcr.io/vortx-ai/emem, source at
https://github.com/Vortx-AI/emem.
```

I added the entry under both **Environment & Nature** and
**Location Services** because the server straddles both — feel free
to drop one if the maintainers prefer a single placement.

## Step 8 — verify everything

After Steps 1–7 land, run this verifier:

```bash
echo "=== github ==="
curl -sf "https://api.github.com/repos/Vortx-AI/emem" >/dev/null && echo "  public ✓"
echo "=== ghcr ==="
TOKEN=$(curl -s "https://ghcr.io/token?scope=repository:vortx-ai/emem:pull&service=ghcr.io" \
  | python3 -c "import sys,json;print(json.load(sys.stdin)['token'])")
curl -sf -H "Authorization: Bearer $TOKEN" \
  "https://ghcr.io/v2/vortx-ai/emem/manifests/latest" >/dev/null && echo "  public image ✓"
echo "=== hf space ==="
curl -sf "https://huggingface.co/spaces/vortx-ai/emem" >/dev/null && echo "  space live ✓"
echo "=== mcp registry ==="
curl -sf "https://registry.modelcontextprotocol.io/v0/servers?search=emem" \
  | python3 -c "import sys,json; d=json.load(sys.stdin); print('  ', len(d.get('servers',[])), 'matches')"
echo "=== mcp endpoint reachable ==="
curl -sf -X POST "https://emem.dev/mcp" -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' \
  | python3 -c "import sys,json; d=json.load(sys.stdin); print('  ', len(d.get('result',{}).get('tools',[])), 'tools')"
```

Each line should print a check / count. Anything that errors tells
you which step still needs flipping.

## Step 9 — rotate the leaked HuggingFace token

The token sent earlier in cleartext is in
`~/.config/emem/secrets.env` (mode 600). After Step 5 succeeds with a
fresh token, revoke the old one:

```
huggingface.co/settings/tokens →
  find the leaked token → "Manage" → "Revoke"
```

## Why we don't need a `modelcontextprotocol/servers` PR

That repo no longer accepts third-party server submissions — see
[its CONTRIBUTING.md](https://github.com/modelcontextprotocol/servers/blob/main/CONTRIBUTING.md).
The replacement is the official MCP Registry covered in Step 6.
