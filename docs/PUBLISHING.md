# Publishing emem across platforms

The MCP server is meant to be **discoverable** in three places: GitHub
Container Registry (canonical), Docker Hub (mirror), and HuggingFace
Spaces (one-click hosted). This page documents the moving parts; the
CI does the work.

## 1. GitHub Container Registry (always on)

Every push to `main` and every release tag triggers
`.github/workflows/publish.yml`, which builds the multi-arch
(`linux/amd64` + `linux/arm64`) image and pushes:

```
ghcr.io/vortx-ai/emem:latest
ghcr.io/vortx-ai/emem:<short-sha>
ghcr.io/vortx-ai/emem:<vX.Y.Z>     # on tag
```

No secrets needed beyond the default `GITHUB_TOKEN`. Provenance and
SBOM are attached.

## 2. Docker Hub mirror (opt-in)

Set the repo-level variable `PUBLISH_DOCKERHUB=true` and the secrets
`DOCKERHUB_USERNAME` + `DOCKERHUB_TOKEN`. The same workflow then
mirrors to:

```
docker.io/vortxai/emem:<same-tags>
```

The Docker Hub PAT only needs `Public Repo: Read & Write` scope.

## 3. HuggingFace Space (opt-in)

The `huggingface-space/` directory is a Docker-SDK Space scaffold that
**re-uses** the GHCR image — no rebuild on the HF side. To enable the
sync job:

1. Create the Space at `huggingface.co/spaces/<owner>/emem` with
   SDK = Docker.
2. Set the repo-level variable `PUBLISH_HF_SPACE=true` and the
   variable `HF_SPACE_REPO_ID=<owner>/emem`.
3. Set the secret `HUGGINGFACE_TOKEN` to a token with **Write** scope
   for that Space.

CI pushes `huggingface-space/` to the Space on every `main` push.

## 4. MCP registry metadata

`mcp-server.json` at the repo root is the machine-readable manifest
crawled by MCP registries (e.g. the [official MCP servers
list](https://github.com/modelcontextprotocol/servers)). Keep it in
sync with the tool list in `crates/emem-mcp/src/lib.rs`; CI verifies
the tool names line up via a unit test.

## 5. Verifying a release

After a tag push:

```bash
# pull
docker pull ghcr.io/vortx-ai/emem:latest

# inspect labels
docker inspect ghcr.io/vortx-ai/emem:latest \
  --format '{{ json .Config.Labels }}' | jq

# verify provenance (cosign)
cosign verify-attestation \
  --type slsaprovenance \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  --certificate-identity-regexp "https://github.com/Vortx-AI/emem/.github/workflows/publish.yml@.*" \
  ghcr.io/vortx-ai/emem:latest

# smoke test
docker run --rm -p 5051:5051 ghcr.io/vortx-ai/emem:latest &
sleep 2
curl -s http://localhost:5051/health
```

## 6. Token rotation policy

The HuggingFace `HUGGINGFACE_TOKEN` and Docker Hub `DOCKERHUB_TOKEN`
are stored as encrypted GitHub Actions secrets. Rotate at least
quarterly and after any suspected exposure. The CI never logs them.

## 7. Local-only workflow

If you do not want to publish:

```bash
docker build -t emem:local .
docker run --rm -p 5051:5051 emem:local
```

The `Dockerfile` at the repo root is the single source of truth for
the runtime image.
