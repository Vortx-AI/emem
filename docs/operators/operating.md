# operating emem

This doc is for the operator running an emem responder in production.
It assumes you already understand `docs/developers/architecture.md` (system
topology) and `docs/protocol.md` (wire bytes). For build, test, and
contribution workflows see `docs/developers/developing.md`.

## What you're running

A single Rust binary `emem-server` listening on one port for HTTP plus
the MCP JSON-RPC endpoint at `/mcp`. Optional Python sidecar over a
Unix Domain Socket for GPU inference (Prithvi-EO-2.0, Galileo,
JEPA-v2 dynamics). Local sled DB for the hot cache; an append-only
Merkle log under `<EMEM_DATA>/log/`. ed25519 responder identity
persisted at `<EMEM_DATA>/identity.secret.b32` with mode 0600.

Default ports:

- `5051` — plain HTTP, the dev default. Behind a reverse proxy in
  most production setups.
- `443` — TLS via `rustls` + `rustls-acme` (Let's Encrypt
  TLS-ALPN-01). Activated by setting `EMEM_TLS_DOMAINS`.

Port `5050` is permanently unavailable on the canonical dev host: it
is held by an unrelated `tile_server.py` for a different project.
Anywhere you see "5050" in old notes, treat it as 5051 today.

## Quick deploy

### Docker

```
docker run -d \
  --name emem \
  --restart unless-stopped \
  -p 5051:5051 \
  -v emem_data:/var/emem \
  -e EMEM_BIND=0.0.0.0:5051 \
  -e EMEM_DATA=/var/emem \
  ghcr.io/vortx-ai/emem:0.0
```

The image runs as UID 65532 on `debian:trixie-slim` (glibc 2.41 —
required by `ort-sys`). It pre-applies
`cap_net_bind_service=+ep` on `/usr/local/bin/emem-server` so binding
:443 inside the container does not need `--cap-add`. The healthcheck
uses the bash builtin `</dev/tcp/127.0.0.1/${EMEM_BIND##*:}` so the
runtime image does not need curl.

The `:0.0` major-tag is the recommended pin. Do not pin `:latest`:
the HuggingFace Space used to ride `:latest` and was vulnerable to a
silent regression there; it now pins `:0.0` (set 2026-05-08).

### Bare-metal (systemd user unit)

The maintained dev host runs this way. Walk through:

1. Build:

   ```
   cargo build --release -p emem-cli
   ```

2. Copy the example unit:

   ```
   cp ops/systemd/emem-server.service.example \
     ~/.config/systemd/user/emem-server.service
   ```

3. Edit the `Environment=` lines to match the host. The example pins
   `WorkingDirectory=/home/ubuntu/emem`, `EMEM_DATA=/home/ubuntu/emem/var/emem`,
   `EMEM_BIND=127.0.0.1:5051`, and `EMEM_TLS_DOMAINS=emem.dev,www.emem.dev`.

4. Reload and start:

   ```
   systemctl --user daemon-reload
   systemctl --user enable --now emem-server.service
   systemctl --user status emem-server
   ```

5. Cap journald retention to 30 days to match the privacy posture:

   ```
   sudo install -m 0644 ops/systemd/journald-30day-retention.conf \
     /etc/systemd/journald.conf.d/30day-retention.conf
   sudo systemctl restart systemd-journald
   sudo journalctl --vacuum-time=30d
   ```

CAP_NET_BIND_SERVICE handling depends on whether the unit is user-
or system-installed:

- **User unit** (this template, installed under `~/.config/systemd/user/`):
  the kernel does not honour `AmbientCapabilities=` for the user-mode
  systemd manager because there is no UID transition for it to prime.
  The working pattern is `setcap cap_net_bind_service=+ep` on the
  binary; `scripts/redeploy.sh` re-applies it after every
  `cargo build --release` (the strip removes the file cap).
- **System unit** (installed under `/etc/systemd/system/`, runs with
  `User=emem`): uncomment the `AmbientCapabilities=CAP_NET_BIND_SERVICE`
  line in the service file. The system manager primes the cap before
  the UID transition, so the binary doesn't need a file cap.

See "Known operational risks" below for the 2026-04-30 incident this
covers.

### TLS

Set `EMEM_TLS_DOMAINS=emem.dev,www.emem.dev` and (optionally)
`EMEM_TLS_BIND=0.0.0.0:443`. The server uses `rustls-acme` with
TLS-ALPN-01. On first start it provisions certs and caches them under
`<EMEM_DATA>/acme.cache/`. There is no Caddy, no nginx, no Cloudflare
in the path — the binary terminates TLS itself.

| Variable | Default | Purpose |
|---|---|---|
| `EMEM_TLS_DOMAINS` | unset | comma-separated FQDNs; activates ACME |
| `EMEM_TLS_BIND` | `0.0.0.0:443` | TLS listener |
| `EMEM_TLS_CONTACT` | `mailto:avijeet@vortx.ai` | Let's Encrypt ACME contact and `/.well-known/security.txt` Contact |
| `EMEM_TLS_STAGING` | unset | `=1` to use the Let's Encrypt staging directory while testing the deploy path |

When TLS is on, the plain HTTP listener at `EMEM_BIND` stays up in
parallel for local MCP clients and the live-demo binary. Set
`EMEM_REDIRECT_HTTPS=1` if you want plain HTTP to 308-redirect to the
TLS host instead.

### Content-Security-Policy

The server emits a strict CSP header on every response. The default
policy allows `https://esm.sh` in **both** `script-src` and
`connect-src` so the `/humans` page can dynamically import
`@noble/curves@1.6.0/ed25519` and `@noble/hashes@1.5.0/blake3` to
verify Ed25519 receipts in the browser. Without those allowances the
page silently degrades to the server-side `/v1/verify_receipt` path
and labels itself "CDN libs unavailable" — which reads as a verify
failure even though the math behind it works.

```text
content-security-policy: default-src 'self';
  script-src 'self' https://www.googletagmanager.com https://esm.sh 'unsafe-inline';
  connect-src 'self' https://www.google-analytics.com https://esm.sh;
  img-src 'self' data: https:;
  style-src 'self' 'unsafe-inline' https://fonts.googleapis.com;
  font-src 'self' data: https://fonts.gstatic.com;
  frame-ancestors 'self' https://huggingface.co https://*.hf.space;
  base-uri 'self'; form-action 'self'
```

If you put a reverse proxy in front of emem, do not override this
header with a stricter policy unless you also drop `/humans` from your
deployment. The header is built once at router construction in
`crates/emem-api-rest/src/lib.rs` (`security_headers` middleware) and
intentionally has no env-var override — the offline-verify path is a
load-bearing trust claim and must remain wired by default.

## Environment variables

The full set the responder reads. Defaults are what the binary picks
when the variable is unset.

| Variable | Default | Purpose |
|---|---|---|
| `EMEM_BIND` | `0.0.0.0:5051` | plain HTTP bind |
| `EMEM_DATA` | `./var/emem` | data dir; `:memory:` for ephemeral |
| `EMEM_SECRET_B32` | (loaded from `<EMEM_DATA>/identity.secret.b32`) | ed25519 secret override, base32-nopad |
| `EMEM_AUTO_MATERIALIZE` | unset (off) | enable lazy fetch on `/v1/recall` miss |
| `EMEM_TIMEOUT_SECS` | 180 | per-request gateway timeout (clamped 1..=600) |
| `EMEM_MATERIALIZER_TIMEOUT_SECS` | 30 | per-upstream timeout (clamped 2..=240) |
| `EMEM_MATERIALIZER_RETRIES` | 2 | per-upstream retries (clamped 1..=5) |
| `EMEM_BODY_LIMIT_MB` | 8 | request body cap (clamped 1..=256) |
| `EMEM_RATE_LIMIT_RPS` | 50 | global token-bucket fill (clamped 0.01..=1000) |
| `EMEM_RATE_LIMIT_BURST` | 200 | burst capacity (clamped 1..=100000) |
| `EMEM_TRUST_FORWARDED` | unset | `=1` to honour `X-Forwarded-For` for rate limiting |
| `EMEM_REDIRECT_HTTPS` | unset | `=1` to redirect plain HTTP to TLS |
| `EMEM_ALLOWED_ORIGINS` | `*` | CORS allowlist; comma-separated |
| `EMEM_PUBLIC_URL` | derived from `EMEM_TLS_DOMAINS` | canonical origin for `/.well-known/emem.json` and User-Agent |
| `EMEM_SECURITY_POLICY_URL` | unset | `Policy:` line in `/.well-known/security.txt` |
| `EMEM_SIDECAR_SOCK` | `%t/emem/jepa_sidecar.sock` | UDS path the Rust server dials |
| `EMEM_SIDECAR_TIMEOUT_MS` | 5000 | sidecar request timeout |
| `EMEM_SIDECAR_VRAM_BUDGET_GB` | binary default 10; the deployed `emem-jepa-sidecar.service` overrides to 20 | sidecar's per-process VRAM cap; 20 GB seats the four co-resident encoders (Prithvi-EO-2.0, Galileo, JEPA-v2 dynamics, the topic-router warmup buffer) |
| `EMEM_GALILEO_VARIANT` | `base` | Galileo variant — one of `tiny`, `base`, `nano`. The advertised capability becomes `galileo-<variant>` in `/v1/capabilities.extensions[]`. Production deploys default to `base`. |
| `EMEM_GALILEO_SNAPSHOT` | unset | pin a Galileo checkpoint snapshot |
| `EMEM_PRITHVI_SNAPSHOT` | unset | pin a Prithvi-EO checkpoint snapshot |
| `EMEM_OVERTURE_RELEASE` | (auto-discover newest) | pin Overture monthly release |
| `EMEM_OVERTURE_PARALLEL` | per-host default | parallel range reads against the Overture S3 bucket |
| `EMEM_SCAN_CELL_LIMIT` | 10000 | sled scan-row cap |
| `EMEM_COVERAGE_MATRIX_LIMIT` | 32768 | per-band index scan cap for `/v1/coverage_matrix` |
| `EMEM_DEMOS_DIR` | `<EMEM_DATA>/demos` (or `var/demos`) | livedemo / realdemo output dir |
| `EMEM_TOPIC_MODEL_DIR` | `<EMEM_DATA>/models/bge-base-en-v1.5` | topic-router ONNX directory |
| `EMEM_TOPIC_BACKEND` | ort | force `=keyword` to skip the BERT path |
| `EMEM_TOPIC_USE_GPU` | unset | `=1` to enable CUDA EP on the topic router (requires CUDA dylib) |
| `EMEM_TOPIC_THRESHOLD` | 0.35 | cosine threshold for ort backend topic match |
| `EMEM_GA_MEASUREMENT_ID` | unset | substituted into `web/index.html` at startup; unset strips the GA block entirely |
| `EMEM_NOMINATIM_BASE` | upstream OSM | self-host override for the secondary geocoder |
| `EMEM_PHOTON_BASE` | komoot Photon | primary geocoder override |
| `EMEM_GEOCODER_TTL_SECS` | sane default | cache TTL for `/v1/locate` |
| `EMEM_NO_NETWORK` | unset | `=1` to forbid all upstream calls (offline mode for tests) |
| `ORT_DYLIB_PATH` | (none) | path to `libonnxruntime.so.*` for the topic router and ort 2.x |
| `LD_LIBRARY_PATH` | (none) | extra library paths (CUDA 12 runtime, ORT bundle) |
| `RUST_LOG` | `info` | tracing filter, e.g. `info,emem_storage=debug,rustls_acme=info` |
| `HF_HUB_OFFLINE` | `1` (enforced) | forbid the HuggingFace hub client from touching the network at runtime; all model snapshots must be pre-staged under `<EMEM_DATA>/hf_cache/` and `<EMEM_DATA>/models/` |

## Disk layout

`<EMEM_DATA>/` after a few weeks of running:

```
<EMEM_DATA>/
├── identity.secret.b32          ed25519 secret, mode 0600
├── cache.sled/                  sled hot cache (4 trees: canonical_index, facts, attesters, fact_proofs)
├── log/
│   └── merkle.log.{0,1,...}     append-only segments, ~1 GiB rotation
├── geocoder.sled/               /v1/locate cache (separate sled DB)
├── hf_cache/                    HuggingFace model snapshots (Prithvi, Galileo if downloaded server-side)
├── models/
│   └── bge-base-en-v1.5/        topic-router ONNX (~435 MB) + tokenizer.json
├── jepa_v2/
│   ├── dynamics_v2.onnx         baseline (residual-zero-init, 8 KB)
│   └── dynamics_v2.metadata.json
├── acme.cache/                  Let's Encrypt cert cache
└── demos/                       livedemo / realdemo output (override with EMEM_DEMOS_DIR)
```

The whole `<EMEM_DATA>/` is portable. Snapshot it to S3 or GCS for
backup; because the responder is content-addressed end-to-end, replay
on a fresh host produces byte-identical receipts as long as the
schema CID resolves and the responder identity is the same.

If you migrate to a new host and want a stable responder pubkey, copy
`identity.secret.b32` first. Otherwise the server generates a fresh
key on first start and persists it.

## Monitoring

| Endpoint | Purpose |
|---|---|
| `GET /health` | liveness — uptime, manifest CIDs, responder pubkey, started_at_unix_s, version |
| `GET /metrics` | Prometheus exporter — request counts, latency histograms by path, materialization counters |
| `GET /v1/coverage_matrix` | per-band live status: `has_materializer`, `facts_count`, `last_attested_unix_s`, `tempo_seconds`, `responder_pubkey_b32`, `wired_subkeys` for family aliases |
| `GET /v1/data_availability` | temporal coverage — `history_available_from_unix` / `history_available_to_unix` per band |
| `GET /v1/manifests` | active `bands_cid`, `algorithms_cid`, `sources_cid`, `schema_cid`, `registry_cid` |
| `GET /v1/agent_stats` | aggregate tool call counts by agent platform |
| `GET /v1/demos` | list of saved livedemo/realdemo runs |

For the systemd deploy, tail logs with:

```
journalctl --user -u emem-server -f
```

Bump verbosity per-module with `RUST_LOG=info,emem_storage=debug,
rustls_acme=info`. The interesting subsystems are `emem_storage`
(materialization, attestation), `emem_api_rest` (request shape),
`rustls_acme` (cert renewal), and `emem::topic_router` (ort warmup).

A useful one-liner for "is this responder healthy":

```
curl -sS http://127.0.0.1:5051/health | jq '{
  ok, version, uptime_seconds, responder_pubkey_b32,
  bands_cid, schema_cid
}'
```

Confirm the route surface and bands at the same time:

```
curl -sS http://127.0.0.1:5051/v1/coverage_matrix \
  | jq '.totals, (.bands | length)'
```

The expected shape today is ~118 distinct band names surfaced (those
118 names route into the 35 cube slots that sum to 1792-D); lower
numbers mean a registry CID rolled back or a materializer was removed.
`/v1/materializers` reports the 20 live materializer registrations
directly.

## Tuning

### Request path

- Big bbox queries: `/v1/query_region` caps at 4096 cells (bbox
  geometry) and 65,536 facts total. Beyond that, switch to an
  explicit `cells:c1,c2,...` list or split the bbox client-side.
- `/v1/find_similar` with a structured filter: per-cell claim
  evaluation memoizes verdicts across repeated tslots. Corpora
  larger than ~1 M cells should layer Lance or FAISS in front; the
  in-process scan is honest but linear.
- Recall on a miss with `EMEM_AUTO_MATERIALIZE=1`: 30-second
  per-upstream timeout, retried twice. Six concurrent fetches will
  saturate a typical host's outbound bandwidth without disk
  pressure.
- `/v1/recall_many` accepts up to 256 cells in one request and
  returns ETag/304 on repeat calls (same canonical request → same
  CID → 304). Use it instead of N×`/v1/recall`.

### sled

- Hot cache lives at `<EMEM_DATA>/cache.sled`. Log-structured;
  auto-compacts in the background.
- Exclusive lock per process. `emem-purge-fnkey --apply` requires the
  server stopped or sled returns `IO error: Resource temporarily
  unavailable`.
- `EMEM_SCAN_CELL_LIMIT` (default 10000) caps the per-cell prefix
  scan inside the cache. Increase only if you have a known
  high-density corpus and have measured the latency hit.

### Topic router

- Default backend is ort + `BAAI/bge-base-en-v1.5` (CPU). Warmup
  budget at startup is 90 s; if the load takes longer the server
  starts anyway and `/v1/ask` falls back to the keyword backend
  until the OnceLock initializer completes.
- Pre-stage the model with `scripts/install-topic-model.sh`, which
  pulls `tokenizer.json` and `model.onnx` (~435 MB) into
  `$EMEM_TOPIC_MODEL_DIR`. This avoids the cold-start fetch and
  keeps the server's first call fast.
- GPU off by default. To use CUDA EP, set
  `ORT_DYLIB_PATH=/opt/onnxruntime-1.22.0-cuda12/lib/libonnxruntime.so.1.22.0`
  and `EMEM_TOPIC_USE_GPU=1`. Operationally the CPU path is fine for
  the 27-topic registry — ~110 ms warm end-to-end.

### Sidecar (jepa_v2)

- Cold-start cost is one-time per process; warm cache after the
  first `/predict` call on each model.
- The deployed `emem-jepa-sidecar.service` user unit sets
  `EMEM_SIDECAR_VRAM_BUDGET_GB=20` so all four encoders (Prithvi-EO-2.0,
  Galileo, JEPA-v2 dynamics, and the topic-router warmup buffer) sit
  co-resident on the GPU. The binary default is 10 if the variable is
  unset. Lower it if the GPU is shared; the sidecar refuses requests
  that would exceed the budget and the Rust server returns 503. There
  is no silent fallback to CPU inference.
- The unit at `python/jepa_v2_sidecar/emem-jepa-sidecar.service` is a
  user systemd unit; the Rust server reads `EMEM_SIDECAR_SOCK` (default
  `%t/emem/jepa_sidecar.sock` ⇒
  `/run/user/<UID>/emem/jepa_sidecar.sock`).
- JEPA v2 is untrained today. The endpoint short-circuits via
  `is_trained()` against a metadata-only `OnceLock`, returns the
  last-attested-vintage identity baseline, and labels the receipt
  `via: short_circuit_untrained` with `untrained_baseline: true`.
  Do not describe `/v1/jepa_predict_v2` as a working dynamics head
  until the training run lands.
- Galileo (variant selectable via `EMEM_GALILEO_VARIANT`, default `base`) has only the S2 modality wired today; S1, ERA5, TC,
  VIIRS, SRTM, Dynamic World, WorldCover, LandScan, and the location
  channels are zero-masked at inference. The multimodal scaffold is
  present and the embeddings are honest, but the missing modalities
  show up as zeros in the input tensor, not as a structured Absence.

## Fail-over and HA

The deployment model today is single-host. There is no built-in
clustering, no leader election, no replicated sled. What you can
do:

- **Read replicas**: point a second `emem-server` at a snapshot of
  `<EMEM_DATA>/` and serve reads. Receipts continue to verify
  because the responder identity is the file-pinned ed25519 key, and
  facts are content-addressed.
- **Writes**: attestations still need a single primary because the
  append-only Merkle log is single-writer. If you need multi-writer
  attestation today, run separate responders with separate
  identities and federate at the agent layer — `(responder_pubkey,
  fact_cid)` is the trust unit.

Mirroring on the connector side (multiple upstreams configured for
the same materializer):

| Connector | Mirroring |
|---|---|
| Sentinel-2 / Sentinel-1 STAC | dual: Element84 primary, MS Planetary Computer fallback |
| Terraclimate NCSS | dual: UI primary, NCAR RDA secondary (added 2026-05-08) |
| Cop-DEM 30 m | single (Copernicus S3) |
| Hansen GFC | single |
| WorldPop | single (slow; see below) |
| DMSP-OLS | single, frozen 1992-2013 |
| ESA WorldCover | single |
| OSM / Overture | single (Overture snapshot pinning via `EMEM_OVERTURE_RELEASE`) |

When a dual-mirror connector falls back, the receipt's `Source.url`
records which mirror actually served the bytes — you can
post-process `/v1/sources` traffic to confirm primary vs secondary
hit rate.

## Known operational risks

| Risk | Today's mitigation | Future |
|---|---|---|
| `cap_net_bind_service` stripped on `cargo build --release` | `redeploy.sh` re-applies `setcap` after every release build; system-mode units can also use `AmbientCapabilities=CAP_NET_BIND_SERVICE` (does not work for user units — kernel restriction) | unchanged |
| WorldPop 2-4 s/cell upstream latency | accepted; cap on `/v1/query_region` keeps blast radius small | pre-bake the global 1 km² raster to COG in S3 (deferred, infra decisions outstanding) |
| Terraclimate NCSS single endpoint | NCAR RDA fallback registered (2026-05-08); receipt records which mirror served | watch SLA on both |
| DMSP-OLS frozen at 2013 | `/v1/data_availability` reports `history_available_to_unix=2013-12-31` honestly | dataset is genuinely complete; no fix needed |
| HF Space dependency on `:latest` | now pinned to `ghcr.io/vortx-ai/emem:0.0` (2026-05-08) | bump tag deliberately on each release |
| sled lock contention | `emem-purge-fnkey` requires server stopped; documented | none planned |
| Sidecar OOM | 503 on `/v1/jepa_predict_v2`; no silent CPU fallback | accept; tune `EMEM_SIDECAR_VRAM_BUDGET_GB` |
| Topic router cold load >90 s | server starts anyway; keyword backend handles `/v1/ask` until ort thread returns | pre-warm with `scripts/install-topic-model.sh` |
| Empty SDK directories (`sdks/emem-{py,ts}`) | integrate via REST or MCP — see `docs/developers/developing.md` | populate when API surface stabilises |

The cap_net_bind_service incident: on 2026-04-30 a sequence of
`cargo build --release` smoke tests stripped the file capability
between rebuilds, the systemd user unit could not bind :443, and
`Restart=on-failure` with `RestartSec=2` produced **1560 restarts in
30 minutes** before the operator noticed. The remediation pattern is
`scripts/redeploy.sh`, which re-runs `setcap` after every release
build. A 2026-05-08 sweep tried to add `AmbientCapabilities=` as a
systemd-native alternative and discovered the kernel does not honour
that directive for user-mode systemd: there's no UID transition for
the user manager to prime, so the unit fails with
`status=218/CAPABILITIES`. The directive only works for system-mode
units (installed under `/etc/systemd/system/` with an explicit
`User=` line). The corrected guidance in
`ops/systemd/emem-server.service.example` now documents both paths
explicitly; user-mode deployments stay on `setcap` via redeploy.sh.

## Troubleshooting recipes

**Service flapping right after a `cargo build --release`.**
Symptom: `systemctl --user status emem-server` shows
`Restart=` triggering every 2 s; logs show `Permission denied (os
error 13)` on bind. Diagnose with
`getcap target/release/emem-server` (empty output = no cap).
Fix: either rely on `AmbientCapabilities=` (newer unit file, no
action) or run `sudo setcap cap_net_bind_service=+ep
target/release/emem-server` and `systemctl --user restart
emem-server.service`. If you didn't update the unit file yet, use
`scripts/redeploy.sh` — that's its job.

**`/v1/recall` returns 500 with `MaterializeMiss`.**
Either no materializer is wired for that band, or
`EMEM_AUTO_MATERIALIZE=1` is not set. Check
`curl -s http://127.0.0.1:5051/v1/coverage_matrix | jq '.bands[]
| select(.band=="<your_band>")'`. If `has_materializer:false` the
band needs a third-party Attestation; if `true`, set
`EMEM_AUTO_MATERIALIZE=1` and retry.

**Sidecar 503 on `/v1/jepa_predict_v2`.**
`systemctl --user status emem-jepa-sidecar` first. If running, hit
`nvidia-smi`; if VRAM is exhausted, lower
`EMEM_SIDECAR_VRAM_BUDGET_GB`. If the sidecar shows a torch
`load_state_dict` error with `strict=true`, the checkpoint and the
in-process architecture have drifted — one or the other has been
upgraded without the matching update; pin both via
`EMEM_PRITHVI_SNAPSHOT` / `EMEM_GALILEO_SNAPSHOT`.

**Attestation rejected.**
`journalctl --user -u emem-server | grep AttestationInvalid` for
the reason. Most common: leaves not sorted before merkle_root, or
the signature is over the wrong preimage. Both shapes are tested
in `crates/emem-fact/tests/round_trip.rs`; reproduce there before
re-attesting.

**Receipt verifies on-host but client rejects.**
Preimage mismatch. The server preimage is
`blake3(request_id|served_at|primitive|cells,...|fact_cids,...)`
with literal `|` separators between fields and `,` between list
elements (trailing comma included). The reference is
`crates/emem-storage/src/server.rs:119`. The CLI implements the
same byte sequence in `crates/emem-cli/src/main.rs::run_verify`.

**`/v1/locate` returns 504 or empty.**
The cascade is `wide_bbox_lookup → embedded_gazetteer_lookup →
geonames-68k → sled cache → Photon → Nominatim`. Photon is the
primary network upstream; if it is down `EMEM_PHOTON_BASE` lets you
self-host or swap, and `EMEM_NOMINATIM_BASE` lets you redirect the
secondary. Polygon geometry is resolved by
`overture.rs::division_polygon_near` (the Overture `divisions/
division_area` parquet); Nominatim is only the long-tail polygon
fallback. Set `EMEM_NO_NETWORK=1` to confirm the embedded + cache
layers work in isolation; the response's `via` field names the layer
that answered.

**ACME cert renewal failing.**
`journalctl --user -u emem-server | grep -i acme`. Common
problems: port 443 not reachable from the public internet (ACME
TLS-ALPN-01 needs an inbound connection), wrong contact email,
or hitting the Let's Encrypt rate limit during testing. For
testing iterate against staging with `EMEM_TLS_STAGING=1`.

## Upgrades

- `0.0.x` bumps are CBOR-stable for facts: the schema CID pins
  the exact byte layout. Old facts continue to verify under their
  old `schema_cid`.
- Manifest CIDs (`bands_cid`, `algorithms_cid`, `sources_cid`)
  may change between minors; old facts under the old CIDs
  continue to verify because the receipt and the fact itself
  reference the CID that was active at attestation time.
- The protocol's content-addressing means a fresh deploy can
  serve old facts as long as the schema CID resolves;
  `<EMEM_DATA>/` is portable across hosts and across versions.
- Before rolling forward, capture
  `curl -s http://127.0.0.1:5051/v1/manifests > /tmp/pre.json`,
  upgrade, and diff against `/v1/manifests` after restart. Any
  CID that changed should match the changelog entry for the
  release.

## Demos for diagnosis

When you need a reproducible bundle for a bug report or a
regression test, the demo binaries write everything to disk:

| Binary | Output | Purpose |
|---|---|---|
| `emem-demo` | console only | smoke test, 3 cells × 1 band |
| `emem-livedemo` | `var/demos/<UTC>/` (override `EMEM_DEMOS_DIR`) | full audit trail: 4 cells × 2 bands × 3 tslots, every primitive, per-step JSON + curl repro + blake3 |
| `emem-realdemo` | `var/demos/realdata_<UTC>/` | live-ingest validation, Cop-DEM 30 m for Mt Fuji / Mt Everest / Grand Canyon |
| `emem-ask-eval` | console + exit code | CI regression for `/v1/ask` topic routing |
| `emem-purge-fnkey` | sled mutations (default dry-run) | drop facts by `derivation.fn_key`; **server must be stopped** |

The livedemo output is the canonical artifact for "what was the
protocol doing on date X". `GET /v1/demos`, `GET /v1/demos/:run`,
`GET /v1/demos/:run/:file` serve them straight from disk; agents
can replay any past run without re-executing it.

For wire-byte details and CID rules, see `docs/protocol.md`. For
contributing changes, see `docs/developers/developing.md`.
