# Operate a public node

This page is for the operator of a public emem node, such as `https://emem.dev`. Start with
[Self-host](../self-host.md). It shows how to run a node, what is in the data directory, and the main environment
variables. This page adds what a large node needs: image pinning, TLS, limits, monitoring, tuning, backup and fixes
for known problems.

For build, test and contribution steps, read [Developing](../developers/developing.md). For the wire bytes, read
[Protocol](../protocol.md).

Each default and count on this page comes from the code or from `https://emem.dev` on 7 November 2026.

## What runs

A node is one process, `emem-server`. It serves REST and MCP (`/mcp`) on one port. It needs no GPU and no model
sidecar.

| Listener | Address | When |
|---|---|---|
| Plain HTTP | `EMEM_BIND`, default `0.0.0.0:5051` | Always. With TLS on, the default moves to `127.0.0.1:5051`. |
| HTTPS | `EMEM_TLS_BIND`, default `0.0.0.0:443` | Only when you set `EMEM_TLS_DOMAINS`. |

All state is under `EMEM_DATA`. See [What is in the data directory](#what-is-in-the-data-directory).

## Deploy with the image

```bash
docker run -d --name emem --restart unless-stopped \
    --stop-timeout 60 \
    -p 5051:5051 \
    -v emem-data:/var/emem \
    ghcr.io/vortx-ai/emem:v2.4.4
```

The image sets `EMEM_DATA=/var/emem`. The entry point sets `EMEM_BIND` to `0.0.0.0:${PORT:-5051}` when you do not
set it.

`docker stop` waits 10 seconds by default, then kills the process. At stop, the node closes `facts.redb` within
`EMEM_STOP_GRACE_S`, default 60 seconds. A kill during that close marks the file for recovery, and the next start
can take many minutes. `--stop-timeout 60` makes the container wait the same 60 seconds. If you change one value,
change the other.

### Image tags

The `publish` workflow (`.github/workflows/publish.yml`) pushes these tags. The registry had each example tag on
7 November 2026.

| Tag | Example | Moves |
|---|---|---|
| Release, git ref name | `:v2.4.4` | No |
| Release, bare | `:2.4.4` | No |
| Minor | `:2.4` | Yes, to the newest patch of that minor |
| Branch | `:main` | Yes, on each push to `main` |
| Default branch | `:latest` | Yes, on each push to `main` |
| Commit | `:<short-sha>` | No |

Release `v1.4.0` has only the `:v1.4.0` form. The bare forms started after that release. There is no `:0.0` tag.

`:latest` and `:main` follow `main`, so they can hold code that is in no release. On 7 November 2026, `:latest` and
`:v2.4.4` had different digests.

For a long-lived node, pin the digest, not a tag. A maintainer can move or delete a tag. Nobody can change a digest.
On 7 November 2026, `:v2.4.4` had this digest:

```text
ghcr.io/vortx-ai/emem@sha256:1b8ec823e13caaa2f5813628e4787f7d038e1713c097e62bd511b71d93f859a8
```

The Hugging Face Space pins a digest in
[`huggingface-space/Dockerfile`](https://github.com/Vortx-AI/emem/blob/main/huggingface-space/Dockerfile).

### What the image contains

These facts come from the runtime stage of [`Dockerfile`](https://github.com/Vortx-AI/emem/blob/main/Dockerfile).

| Setting | Value |
|---|---|
| Base | `debian:trixie-slim` |
| User | `emem`, uid 65532 |
| Port 443 | The binary has `cap_net_bind_service=+ep`, so it binds 443 without `--cap-add` |
| Health check | `bash` opens a TCP connection to the listen port. The image has no `curl`. |
| Ports | `5051` and `443` |

### How emem.dev runs

emem.dev runs the image with `docker run` under the system unit
[`deploy/systemd/emem-server.service`](https://github.com/Vortx-AI/emem/blob/main/deploy/systemd/emem-server.service).
That unit runs `:latest` with `--network host` and `--user 1000:1000`. It sets `TimeoutStopSec=150`, mounts
`config/` read-only, and turns on the warmer. It is for a store of about 98 GB. Read it for these settings, but do
not copy it for a small node.

The maintainers deploy emem.dev with `pending_deploy.sh` after the `publish` workflow passes. See
[AGENTS.md](https://github.com/Vortx-AI/emem/blob/main/AGENTS.md#build). Do not use `scripts/redeploy.sh` on that
host. That script builds on the host and restarts a user unit that production does not use.

## Deploy the binary under systemd

[Self-host](../self-host.md#run-the-binary-under-systemd) has a minimal user unit. The repository also has a longer
example,
[`ops/systemd/emem-server.service.example`](https://github.com/Vortx-AI/emem/blob/main/ops/systemd/emem-server.service.example).
Its values are from one maintainer's host. Change `WorkingDirectory=`, `EMEM_DATA`, `EMEM_TLS_DOMAINS` and
`EMEM_TLS_CONTACT` before you use it. Remove the `ORT_DYLIB_PATH`, `LD_LIBRARY_PATH` and `EMEM_GA_MEASUREMENT_ID`
lines: the current binary does not use them to run.

```bash
cp ops/systemd/emem-server.service.example ~/.config/systemd/user/emem-server.service
systemctl --user daemon-reload
systemctl --user enable --now emem-server.service
systemctl --user status emem-server
```

To keep journal logs for 30 days at most, as the emem.dev privacy page says:

```bash
sudo mkdir -p /etc/systemd/journald.conf.d
sudo install -m 0644 ops/systemd/journald-30day-retention.conf \
    /etc/systemd/journald.conf.d/30day-retention.conf
sudo systemctl restart systemd-journald
sudo journalctl --vacuum-time=30d
```

### Bind port 443 as a user that is not root

The way to give the binary permission to bind port 443 depends on the unit type.

| Unit type | What works |
|---|---|
| User unit, in `~/.config/systemd/user/` | `sudo setcap 'cap_net_bind_service=+ep' target/release/emem-server` after each build. `AmbientCapabilities=` fails with `status=218/CAPABILITIES`, because the user manager has no UID change that can give the permission. |
| System unit, in `/etc/systemd/system/` with `User=` | `AmbientCapabilities=CAP_NET_BIND_SERVICE` in the unit. The binary does not need the file permission. |

A new build removes the file permission. On 30 April 2026 this stopped the emem.dev user unit from binding port 443.
`Restart=on-failure` with `RestartSec=2` then restarted it 1560 times in 30 minutes. `scripts/redeploy.sh` applies
`setcap` again after each build. Use it only on a node that runs the binary directly.

## Turn on TLS

The binary serves TLS itself, with `rustls` and Let's Encrypt (`rustls-acme`, TLS-ALPN-01). You do not need a
reverse proxy.

| Variable | Default | What it does |
|---|---|---|
| `EMEM_TLS_DOMAINS` | unset | Comma-separated domains. When set, the node gets certificates and serves TLS. |
| `EMEM_TLS_BIND` | `0.0.0.0:443` | The TLS listener. |
| `EMEM_TLS_CONTACT` | unset | Your ACME contact, for example `mailto:ops@example.com`. With TLS on, the node does not start without it. |
| `EMEM_TLS_STAGING` | unset | `1` uses the Let's Encrypt staging directory. Use it while you test. |
| `EMEM_REDIRECT_HTTPS` | unset | `1` makes the plain listener answer with a 308 redirect to HTTPS when the `Host` header is one of `EMEM_TLS_DOMAINS`. A request to another host, such as `127.0.0.1:5051`, gets a normal answer. A request with `X-Forwarded-Proto: https` also gets a normal answer. |

The node keeps certificates in `$EMEM_DATA/acme.cache/`. Port 443 must be open to the internet, because TLS-ALPN-01
connects to it.

In the image, the plain listener does not move to loopback when you turn on TLS. The entry point always sets
`EMEM_BIND`, so add `-e EMEM_BIND=127.0.0.1:5051`. This example needs a domain that points to the host. It did not
run for this page, because the test host has no public domain.

```bash
docker run -d --name emem --restart unless-stopped \
    --stop-timeout 60 \
    -p 443:443 \
    -v emem-data:/var/emem \
    -e EMEM_TLS_DOMAINS=node.example.com \
    -e EMEM_TLS_CONTACT=mailto:ops@example.com \
    -e EMEM_BIND=127.0.0.1:5051 \
    ghcr.io/vortx-ai/emem:v2.4.4
```

The image health check connects to `127.0.0.1` on the port in `EMEM_BIND`, so it still works. See also
[Self-host, Turn on TLS](../self-host.md#turn-on-tls).

## Content-Security-Policy

The node sends a strict CSP header on each response. `build_csp` in `crates/emem-api-rest/src/lib.rs` makes it. The
policy has no environment override. On 7 November 2026, emem.dev sent this policy. This page does not show the
`'sha256-…'` hashes.

```text
default-src 'self';
script-src 'self' https://esm.sh https://cdn.redocly.com 'sha256-…';
connect-src 'self' https://esm.sh https://server.arcgisonline.com;
img-src 'self' data: blob: https:;
style-src 'self' https://fonts.googleapis.com 'sha256-…';
style-src-attr 'unsafe-inline';
font-src 'self' data: https://fonts.gstatic.com;
worker-src 'self' blob:;
frame-ancestors 'self' https://huggingface.co https://*.hf.space https://geo.qa https://*.geo.qa;
base-uri 'self'; form-action 'self'
```

The node computes the hashes at start from the inline scripts and styles of the pages compiled into the binary. There
is no `'unsafe-inline'` for scripts.

The `/verify` page imports the Ed25519 and BLAKE3 code from `https://esm.sh`. Without `esm.sh` in `script-src` and
`connect-src`, the page cannot verify in the browser. It then uses `/v1/verify_receipt` on the server and shows
"CDN libs unavailable". If you put a reverse proxy in front of the node, do not replace this header.

## Environment variables for a public node

[Self-host](../self-host.md#environment-variables) lists the main variables. This table adds the ones for limits,
upstreams and background work. Each default comes from the code.

### Limits

| Variable | Default | What it does |
|---|---|---|
| `EMEM_RATE_LIMIT_RPS` | `10` | Tokens per second in the rate limit of each client address, from 0.01 to 1000. |
| `EMEM_RATE_LIMIT_BURST` | `120` | The bucket size of the rate limit, from 1 to 100000. |
| `EMEM_TRUST_FORWARDED` | unset | `1` takes the client address from `X-Forwarded-For`. Set it only behind a proxy that you control. |
| `EMEM_TIMEOUT_SECS` | `40` | The timeout of one request, from 1 to 600 seconds. |
| `EMEM_ASK_TIMEOUT_SECS` | `30` | The time budget of `/v1/ask`, from 10 to 180 seconds. |
| `EMEM_BODY_LIMIT_MB` | `16` | The maximum request body, from 1 to 256 MB. |
| `EMEM_ALLOWED_ORIGINS` | unset | Comma-separated CORS origins. If unset, the node sends `access-control-allow-origin: *`. |
| `EMEM_MCP_ALLOWED_ORIGINS` | unset | More origins that may call every MCP method. The binary always allows `https://claude.ai`, `https://chatgpt.com`, `https://chat.openai.com`, `https://cursor.com`, `https://emem.dev`, `https://www.emem.dev`, the origin `null`, and requests with no `Origin`. Sandboxed frames and `file:` pages send `Origin: null`. |
| `EMEM_MCP_BROWSER_READS` | on | From any other origin, MCP answers the handshake, the lists and the read-only tools, and refuses the rest. `0` refuses all of them. Use `0` when your tools reach private data. |
| `EMEM_SCAN_CELL_LIMIT` | `10000` | The maximum rows in one cell scan of the hot store. |
| `EMEM_COVERAGE_MATRIX_LIMIT` | `50000` | The maximum index rows that `/v1/coverage_matrix` reads, from 1000 to 100000. |
| `EMEM_RECALL_MANY_CONCURRENCY` | `16` | Cells that `/v1/recall_many` resolves at the same time, from 1 to 256. |

### Upstream reads

| Variable | Default | What it does |
|---|---|---|
| `EMEM_MATERIALIZE_PER_MIN` | `600` | Upstream reads per minute on a recall miss, for all clients together, from 1 to 100000. |
| `EMEM_MATERIALIZE_SHARE` | `0.5` | The part of `EMEM_MATERIALIZE_PER_MIN` that one client address can use. A value must be more than 0 and 1 at most, or the node uses `0.5`. |
| `EMEM_MATERIALIZE_GLOBAL_LIMIT` | `12` | Band materializations that run at the same time in the whole process, from 1 to 256. |
| `EMEM_MATERIALIZE_CONCURRENCY` | `8` | Band materializations that one request runs at the same time, from 1 to 32. |
| `EMEM_ASK_COLD_BAND_CAP` | `8` | Bands with no stored fact that one `/v1/ask` call materializes, from 1 to 64. The node defers the other bands with a note. |
| `EMEM_HUNTER_SLOW_BAND_CAP` | `8` | Cells that one hunter sweep reads for the slow MODIS land surface temperature bands (`modis.lst_day_8day`, `modis.lst_night_8day`), from 1 to 32. |
| `EMEM_MATERIALIZER_TIMEOUT_SECS` | `14` | The timeout of one upstream read, from 2 to 240 seconds. |
| `EMEM_MATERIALIZER_RETRIES` | `2` | How many times the node tries one ORNL MODIS read, from 1 to 5. |
| `EMEM_ABSENCE_RECHECK_SECS` | `21600` | How long the node answers with its own signed absence before it asks the upstream again. `0` asks each time. |
| `EMEM_TILE_STORE_GB` | `40` | Disk for whole local copies of static forest rasters (Hansen GFC, JRC GFC2020, JRC TMF, ESA WorldCover) in `$EMEM_DATA/tile_store/`. The node removes the least recently used copy first. It does not store a copy that leaves less than 30 GB free. It does not copy a file larger than 8 GB; it reads that file by range. `0` turns the store off. |
| `EMEM_S2_CATALOGUES` | `mpc,element84` | The Sentinel-2 STAC catalogues, in order. The node knows only `mpc` and `element84` and ignores other names. |
| `EMEM_RETIRED_BANDS` | empty | A comma-separated list of bands that this node does not materialize, added to the bands that the code retires. emem.dev sets `geotessera,clay_v1,prithvi_eo2,galileo`. |
| `EMEM_OVERTURE_RELEASE` | newest | Pins one Overture release. |
| `EMEM_OVERTURE_PARALLEL` | `32` | Parallel range reads to the Overture bucket, from 1 to 256. |
| `EMEM_OVERTURE_SKIP_WARMUP` | unset | `1` skips the Overture warmup at start. Use it on a node with no internet access. |
| `EMEM_EARTHDATA_TOKEN` or `EMEM_EARTHDATA_TOKEN_FILE` | unset | A NASA Earthdata token. Without it, an `opera_dist.*` recall gets an error (`not_enabled`). With it, the recall also gets an error, because the code has no fetch yet (`NotImplemented`, `opera_dist.rs`). In both cases the node signs nothing. |
| `EMEM_WRI_GDM_CACHE` | unset | The absolute path of a local copy of the WRI drivers of forest loss COG (295 MB, from Zenodo). Without it, a `wri_gdm.driver_class` recall gets an error and the node signs nothing. |
| `EMEM_PHOTON_BASE` | `https://photon.komoot.io` | The first network geocoder for `/v1/locate`. |
| `EMEM_NOMINATIM_BASE` | `https://nominatim.openstreetmap.org` | The second network geocoder. |
| `EMEM_GEOCODER_TTL_SECS` | `2592000` (30 days) | How long the node keeps a geocoder answer in `geocoder.sled/`. |

### Background work

| Variable | Default | What it does |
|---|---|---|
| `EMEM_WARM_INTERVAL_SECS` | `0` (off) | Runs the warmer every this many seconds. The warmer recalls each entry in its list, so the first visitor finds the facts ready. emem.dev uses `21600`. |
| `EMEM_WARM_PRIORITY` | `$EMEM_DATA/warm_priority.json` | The warmer's list. Each entry has `cells`, `bands` or `band`, and an optional `start_unix` and `end_unix`. emem.dev uses `/config/warm_priority.json`, from [`config/warm_priority.json`](https://github.com/Vortx-AI/emem/blob/main/config/warm_priority.json). |
| `EMEM_WARM_BUDGET_MS` | `30000` | The time for one entry in one pass. Work that does not finish continues in the background. |
| `EMEM_WARM_MAX_IO_PRESSURE` | `10` | A pass waits while IO stalls are above this percent (`/proc/pressure/io`, `some avg60`). It waits one interval at most. |
| `EMEM_SHUTDOWN_GRACE_S` | `3` | Seconds that open connections get to finish at stop, 30 at most. |
| `EMEM_STOP_GRACE_S` | `60` | The stop grace of the container, from 5 to 3600 seconds. Set it to your `docker stop -t` value. The node closes `facts.redb` before this time ends, so a kill does not leave the file for recovery. |

The memory TTL pass, the combine pass and the refinement pass are also background work. All three are off by
default. The files in `deploy/` do not turn them on. [Self-host, Memory variables](../self-host.md#memory-variables)
lists their variables.

### Identity, discovery and logs

| Variable | Default | What it does |
|---|---|---|
| `EMEM_SECRET_B32` | unset | The responder secret in base32. If unset, the node reads `$EMEM_DATA/identity.secret.b32`, or makes it. |
| `EMEM_PUBLIC_URL` | the first TLS domain | The origin in `/.well-known/emem.json`, `did.json` and the outbound User-Agent. |
| `EMEM_SECURITY_POLICY_URL` | unset | Adds a `Policy:` line to `/.well-known/security.txt`. |
| `EMEM_TOPIC_MODEL_DIR` | `$EMEM_DATA/models/bge-base-en-v1.5` | The BGE model for the topic router and memory search. The default applies only when you set `EMEM_DATA`, as the image does. With both unset, the router uses its keyword backend and logs a warning. |
| `EMEM_TOPIC_THRESHOLD` | `0.35` | The cosine threshold for a topic match in `/v1/ask`. If unset, the node uses `_routing.threshold` in `topics-v0.json`, which is also `0.35`. |
| `EMEM_DEMOS_DIR` | `var/demos`, from the working directory | The directory that `/v1/demos` serves. |
| `EMEM_WORLDS_DIR` | `var/worlds`, from the working directory | The directory that `/v1/worlds` serves. The image starts in `/`, so set an absolute path there. |
| `EMEM_SPLATS_DIR` | `$EMEM_DATA/splats-site` | The static files that `/splats/` serves. The repository does not include them. If the directory is empty, `/splats/` gives 404. |
| `RUST_LOG` | `info` | The log filter, for example `info,emem_storage=debug,rustls_acme=info`. |

The binary reads more variables than these tables show. Helper functions read some of them, and
`EMEM_WARM_PRIORITY` uses `env::var_os`. A search for `env::var("EMEM_` misses them. So search for every quoted
`EMEM_` name. On 7 November 2026 this gave 273 names. The list can also hold a quoted name that is not an
environment variable.

```bash
grep -rhoE '"EMEM_[A-Z0-9_]+"' crates | sort -u
```

## What is in the data directory

[Self-host](../self-host.md#what-is-in-the-data-directory) lists the main paths. A large node also has these:

```text
$EMEM_DATA/
├── identity.secret.b32     the responder secret, mode 0600
├── facts.redb              fact rows and bodies (the default hot backend)
├── cache.sled/             memory notes, attesters, fact proofs, the device gate, agent stats
├── log/
│   ├── merkle.log.<n>      append-only log segments, a new one after 1 GiB
│   └── merkle.leaves.<n>.v1
├── lance/                  vector indexes
├── geocoder.sled/          the /v1/locate cache
├── tile_store/             local copies of static forest rasters
├── models/                 optional model files
├── splats-site/            the files that /splats/ serves, when you add them
├── acme.cache/             TLS certificates
├── warm_priority.json      the warmer's list, when you put it here
└── warm_state.json         the time of the last warmer pass
```

`EMEM_HOT_BACKEND=sled` puts fact rows back in `cache.sled/`. It is the rollback path. `emem-sled-slim` copies the
live sled trees into a new `cache.sled.slim/` and leaves out the fact trees that `facts.redb` now holds. Before it
leaves out a tree, it copies into `facts.redb` each key of that tree that `facts.redb` does not have. It never
overwrites a key that `facts.redb` holds. It deletes nothing. You rename the directories yourself. On emem.dev,
the copy was 40 GB from 41 GB, so the maintainers kept the original (header of `emem-sled-slim.rs`).

### Back up and move a node

Stop the node, then copy the whole `$EMEM_DATA/`. sled and redb lock their files, and a copy of a running store
can be inconsistent.

Each fact has a content address, so a copy on a new host serves the same `fact_cid` values. Old receipts still
verify, because the signature covers the bytes and not the host. New receipts get a new `request_id` and time.

Copy `identity.secret.b32` first. Without it, the node makes a new key. Agents that pinned the old
`responder_pubkey_b32` then refuse its receipts.

## Monitor

| Endpoint | What it gives |
|---|---|
| `GET /live` | Liveness. It reads no storage, so you can poll it during a deploy. |
| `GET /health` | Uptime, version, manifest CIDs, the responder key and corpus statistics. It can be slower. |
| `GET /metrics` | Prometheus text: requests, rate-limited requests, attestations, recalls, MCP calls and latency buckets. |
| `GET /v1/coverage_matrix` | Each band: `has_materializer`, `facts_count`, `last_attested_unix_s`, `tempo_seconds`. |
| `GET /v1/materializers` | The registered materializers, in pages of 20. `pagination.total` is the count. |
| `GET /v1/data_availability` | The time range that each band covers. |
| `GET /v1/manifests` | The active registry CIDs and the entry count of each registry. |
| `GET /v1/agent_stats` | Requests and errors by client family (`by_family`), calls by tool (`by_tool`), status classes and latency. |
| `GET /v1/demos` | Saved demo runs. |

Check the node:

```bash
curl -sS http://127.0.0.1:5051/health | jq '{ok, version, uptime_seconds, responder_pubkey_b32, bands_cid, schema_cid}'
curl -sS http://127.0.0.1:5051/v1/coverage_matrix | jq '.totals'
curl -sS 'http://127.0.0.1:5051/v1/materializers?page_size=1' | jq '.pagination.total'
```

On 7 November 2026, emem.dev gave these values:

| Value | emem.dev |
|---|---|
| `bands_declared` | 94 |
| `bands_with_materializer` | 69 |
| `bands_with_facts` | 49 |
| Materializers (`pagination.total`) | 113 |
| Bands in the band manifest (`/v1/manifests`, `covers.bands`) | 43 |

`total_facts` in `/v1/coverage_matrix` stops at `EMEM_COVERAGE_MATRIX_LIMIT`. It is not the corpus size. A fall in
`bands_with_materializer` after an upgrade means a materializer is missing, or the registry went back to an older
version.

To read the logs of a user unit:

```bash
journalctl --user -u emem-server -f
```

For the container, use `docker logs -f emem`.

## Tune

### Big area queries

`/v1/query_region` reads stored facts only. It does not fetch from upstreams. It returns at most 65,536 facts. The
area that one call covers depends on the request field:

| Field | Cells | Area |
|---|---|---|
| `bbox`, an array `[west, south, east, north]` | A sample. By default, 64 to 1024 cells, from the area. `max_cells` sets 1 to 1024. | Any size |
| `geometry: "bbox:..."` | Every cell in the box, 4096 at most | About 0.6 km by 0.6 km |
| `geometry: "cells:..."` | The listed cells, 4096 at most | The listed cells |

The cell pitch is about 9.55 m. On 7 November 2026, emem.dev accepted a `bbox:` geometry of 0.005 degrees on each
side near Tokyo. It refused 0.0058 degrees with `invalid_cell`. That error text says the cap covers 6.4 km. That
figure is wrong: 4096 cells form a square of 64 by 64 cells.

To read every cell of a bigger area, get the cells in pages from `/v1/cells_in_bbox` (4096 cells per page at most).
Then send each page to `/v1/recall_many` (256 cells per call at most) with a `budget_ms`. `/v1/recall_many` fetches a
missing band from its upstream.

### The topic router

`/v1/ask` uses the `ort` backend with `BAAI/bge-base-en-v1.5` on the CPU. It routes to the 27 topics in
`crates/emem-core/data/topics-v0.json`. Put the model on disk before the first start:

```bash
scripts/install-topic-model.sh
```

The script downloads `model.onnx` (about 435 MB) and `tokenizer.json` to `$EMEM_TOPIC_MODEL_DIR`. If the warmup
takes more than 90 seconds, the node starts anyway. The first `/v1/ask` then waits for the warmup to finish.
`EMEM_TOPIC_BACKEND=keyword` loads no model.

The router runs on the CPU only. The build links the bundled CPU ONNX Runtime and has no CUDA feature, so no
variable turns on a GPU. A comment in `topic_router.rs` names `EMEM_TOPIC_USE_GPU`, but no code reads it.

## Run more than one node

A node runs on one host. There is no clustering, no leader election and no replicated store.

- For more reads, start a second node on a copy of `$EMEM_DATA/`. Its receipts verify, because it has the same key
  and the facts have content addresses.
- Writes need one primary, because the Merkle log has one writer. For more writers, run separate nodes with
  separate keys. The unit of trust is `(responder_pubkey, fact_cid)`.
- Nodes can witness each other's log heads. See [Federation](../federation.md).

### Upstream mirrors

| Connector | Mirrors |
|---|---|
| Sentinel-2 STAC | Microsoft Planetary Computer, then Element84. `EMEM_S2_CATALOGUES` changes the order. |
| TerraClimate | `thredds.northwestknowledge.net`, then the NCAR RDA mirror |
| Other connectors | One upstream each |

The `Source.url` of a fact names the mirror that served the bytes.

## Fix common problems

### The unit restarts every 2 seconds after a build

The log shows `Permission denied (os error 13)` on bind. The build removed the file permission. Check it:

```bash
getcap target/release/emem-server
```

Empty output means no permission. Run `sudo setcap 'cap_net_bind_service=+ep' target/release/emem-server`, then
`systemctl --user restart emem-server`. See [Bind port 443](#bind-port-443-as-a-user-that-is-not-root).

### A recall returns no fact for a band

Find the band in the coverage matrix:

```bash
curl -sS http://127.0.0.1:5051/v1/coverage_matrix | jq '.bands[] | select(.band=="<band>")'
```

| Result | Cause |
|---|---|
| `has_materializer: false` | The node cannot read this band from an upstream. Only a signed write can add it. |
| `has_materializer: true` | Check that `EMEM_AUTO_MATERIALIZE` is not `0`. Then read the log for the upstream error. |

### The node refuses an attestation

The node does not log the reason. It returns the reason to the client. In most cases the answer is HTTP 422 with
code `bad_signature` and a message that starts with `attestation verification failed:`. A subject outside every
address space gives HTTP 400 with code `unaddressable_subject`.

The usual causes are leaves in the wrong order before the Merkle root, or a signature over the wrong preimage.
`crates/emem-fact/tests/round_trip.rs` tests both. Reproduce the problem there first.

### A receipt verifies on the node but a client refuses it

The client builds a different preimage. New receipts use preimage v1 or v2, with tagged and length-prefixed segments.
Old receipts use v0. [Protocol](../protocol.md) has the byte-by-byte preimage. The reference code is
`sign_receipt_v1_inner` in `crates/emem-storage/src/server.rs`. Test the receipt with `POST /v1/verify_receipt`.

### `/v1/locate` is slow or empty

The `via` field of the answer names the layer that answered. The node tries its embedded gazetteers and its cache
first, then Photon, then Nominatim. If Photon is down, set `EMEM_PHOTON_BASE` to another instance.
`EMEM_NOMINATIM_BASE` does the same for Nominatim.

### Certificate renewal fails

```bash
journalctl --user -u emem-server | grep -i acme
docker logs emem 2>&1 | grep -i acme        # for the container
```

The usual causes: the internet cannot reach port 443, `EMEM_TLS_CONTACT` is wrong, or you hit the Let's Encrypt rate
limit. Use `EMEM_TLS_STAGING=1` while you test.

### Known risks

| Risk | What reduces it now |
|---|---|
| A build removes the file permission | `scripts/redeploy.sh` applies `setcap` again. A system unit can use `AmbientCapabilities=`. |
| A slow upstream, such as the WorldPop Stats API, holds a large recall open | `/v1/recall_many` takes 256 cells at most. `budget_ms` returns the cells that are ready. `EMEM_MATERIALIZE_PER_MIN` caps upstream reads. |
| DMSP-OLS stops at 2013 | The dataset ends there. `/v1/data_availability` says so. |
| A topic model load takes more than 90 seconds | The node starts anyway. Run `scripts/install-topic-model.sh` before the first start. |

## Upgrade

1. Save the manifests: `curl -sS http://127.0.0.1:5051/v1/manifests > pre.json`.
2. Pull the new image, or build the new binary, and restart.
3. Get `/v1/manifests` again and compare it with `pre.json`.
4. Find each changed CID in [CHANGELOG.md](https://github.com/Vortx-AI/emem/blob/main/CHANGELOG.md) for that release.

For the container, a restart does not change the image. Remove the container and start a new one on the same volume.
Replace `<new-tag>` with the release tag or digest.

```bash
curl -sS http://127.0.0.1:5051/v1/manifests > pre.json
docker pull ghcr.io/vortx-ai/emem:<new-tag>
docker stop -t 60 emem && docker rm emem
docker run -d --name emem --restart unless-stopped --stop-timeout 60 \
    -p 5051:5051 -v emem-data:/var/emem ghcr.io/vortx-ai/emem:<new-tag>
curl -sS http://127.0.0.1:5051/v1/manifests > post.json
diff <(jq -S . pre.json) <(jq -S . post.json)
```

`/v1/manifests` has no time fields. Two reads from emem.dev on 7 November 2026 gave the same bytes. So each line
that `diff` shows is a registry change.

The schema CID pins the byte layout of a fact. Old facts keep their old `schema_cid`, `bands_cid` and
`sources_cid`, so they verify after an upgrade.

## Demo binaries for a bug report

These binaries are in `crates/emem-cli/src/bin/`. The first five run against a local node by default, port 5051.
`EMEM_BASE_URL` changes the node. `emem-demo`, `emem-livedemo`, `emem-realdemo` and `emem-connect-demo` sign
attestations with a new key and write them to the node. Run them only against your own node. A default node
refuses these writes (`fact_plane_closed`), so start a test node with `EMEM_FACT_PLANE_OPEN=1` for them. Do not set
it on a node that other people use.

| Binary | Output | Use |
|---|---|---|
| `emem-demo` | console | Writes 3 facts, then calls each read primitive |
| `emem-livedemo` | `var/demos/<UTC>/` | Each primitive, with the JSON, a curl command and a BLAKE3 hash per step |
| `emem-realdemo` | `var/demos/realdata_<UTC>/` | Reads Cop-DEM 30 m tiles, attests two bands per place, then calls each read primitive |
| `emem-connect-demo` | console and exit code | Runs [connect and evolve](https://github.com/Vortx-AI/emem/blob/main/examples/connect-and-evolve.md): two keys sign facts that disagree, then it reads the edges and the contradiction |
| `emem-ask-eval` | console and exit code | A check of `/v1/ask` routing. No CI job runs it. |
| `emem-demo-device` | enrolls a device and posts a trace | Needs an operator endorser key and a root ftrace capture. The default node is `https://emem.dev`, so set `--base` to your node. `--dry-run` sends nothing. |
| `emem-purge-fnkey` | changes the store (dry run by default) | Removes facts by `derivation.fn_key` from `cache.sled/` only, so it acts only with `EMEM_HOT_BACKEND=sled`. Stop the node first. |
| `emem-sled-slim` | console; with `--apply`, a new `cache.sled.slim/` | Lists each sled tree and its size (dry run by default). `--apply` copies the live trees into the new directory, first copies missing fact keys into `facts.redb`, and deletes nothing. Stop the node first. |

The node serves saved runs at `GET /v1/demos`, `GET /v1/demos/:run` and `GET /v1/demos/:run/:file`.
