# Self-host

The public responder at `https://emem.dev` is free and has a rate limit. Run your own node when you need one of
these:

- more throughput than the public rate limit gives,
- a responder key of your own that your agents pin,
- your own facts next to the open data,
- a node near your agents, or a node with no internet access.

A node is one binary, `emem-server`. It serves REST and MCP on one port.

## Run the public image

The image is `ghcr.io/vortx-ai/emem:latest`. It has builds for `linux/amd64` and `linux/arm64` (registry checked on
6 November 2026).

```bash
docker run --rm -p 5051:5051 -v emem-data:/var/emem ghcr.io/vortx-ai/emem:latest
```

Then send two requests:

```bash
curl http://localhost:5051/live
curl -sX POST http://localhost:5051/v1/locate \
    -H 'content-type: application/json' \
    -d '{"q":"South Mumbai"}'
```

`/live` is the cheap probe. Its reply has `ok`, `version` and the node's `responder_pubkey_b32`. The locate reply has
the `cell64` of the place. `/health` also reports corpus statistics and can be slower. The node needs no key and no
account. On the first recall of a cell, it reads the open archives, signs the values and stores them.

On 6 November 2026 the host that wrote this page had no Docker. A registry query found the image manifest, and
the two requests ran against `https://emem.dev`, which runs the same image. The image facts below come from
[`Dockerfile`](https://github.com/Vortx-AI/emem/blob/main/Dockerfile).

| Image setting | Value |
|---|---|
| User | `emem`, uid 65532 |
| `EMEM_DATA` | `/var/emem`, declared as a volume |
| Ports | `5051` (HTTP), `443` (TLS, only when you turn TLS on) |
| Listen address | `EMEM_BIND`, else `0.0.0.0:${PORT}`, else `0.0.0.0:5051` |
| Health check | a TCP connect to the listen port |

### Keep the data

The command above uses a named volume, `emem-data`. Docker gives it the owner of `/var/emem` in the image, so the
`emem` user can write to it.

To use a host directory, run the container as the owner of that directory:

```bash
mkdir -p "$HOME/.emem"
docker run --rm -p 5051:5051 \
    --user "$(id -u):$(id -g)" \
    -v "$HOME/.emem:/var/emem" \
    ghcr.io/vortx-ai/emem:latest
```

Without `--user`, the uid 65532 process cannot write to a directory that your own user owns.

### Use Docker Compose

[`docker-compose.yml`](https://github.com/Vortx-AI/emem/blob/main/docker-compose.yml) starts a node for the witness role in
[Federation](federation.md). It binds `127.0.0.1:5051` and stores data in the volume `emem-data`, mounted at `/data`
(`EMEM_DATA=/data`). It witnesses `https://emem.dev` unless you set `EMEM_PEERS`.

```bash
docker compose up -d
python3 scripts/verify_node.py http://localhost:5051
```

The compose health check calls `curl`. The image does not contain `curl` (see the runtime stage of the
`Dockerfile`), so Docker can mark the container unhealthy while the node works. Check the node with `/live` from the
host.

## What is in the data directory

All state is under `EMEM_DATA`. The binary default is `./var/emem`. The image sets `/var/emem`.

| Path | What it holds |
|---|---|
| `identity.secret.b32` | The Ed25519 secret key of the responder. The node makes it on the first start. |
| `facts.redb` | The hot store of signed facts and their Merkle proofs. |
| `cache.sled/` | Memory notes, attesters and other small indexes. Older fact proofs stay here until the backfill copies them to `facts.redb`. |
| `log/` | The append-only Merkle log of attestations. |
| `tile_store/` | Local copies of the static forest rasters, up to `EMEM_TILE_STORE_GB` gigabytes (default 40). `0` turns it off. |
| `geocoder.sled/` | The cache of place-name lookups and Overture division polygons. |
| `lance/` | Vector indexes for `find_similar` and memory search. The node loads them on start. |
| `acme.cache/` | The Let's Encrypt certificate, when TLS is on. |
| `models/` | Optional model files, for example `bge-base-en-v1.5` for memory search. |

Back up `identity.secret.b32`. If you lose it, the node makes a new key. Agents that pinned your old
`responder_pubkey_b32` then refuse every receipt that the new key signs. Your public key is in the reply from `/live`.

The paths come from `MaterializingStorage::rooted` in `crates/emem-storage/src/lib.rs`,
`crates/emem-cli/src/bin/emem-server.rs` and `geocoder_db_path` in `crates/emem-api-rest/src/lib.rs`.

## Build from source

Install these tools first:

| Tool | Why |
|---|---|
| Rust 1.91 or later | The workspace MSRV. |
| `g++` | `ort-sys` and `esaxx-rs` compile C++ code. |
| `make` | `tikv-jemalloc-sys` runs the `configure` and `make` of jemalloc. |
| `protobuf-compiler`, `libprotobuf-dev` | `lance-encoding` needs `protoc` and the protobuf well-known types. |
| `mdbook` 0.5.2 | The binary embeds the rendered docs site from `docs/book/`. |
| glibc 2.38 or later | `ort-sys` links symbols that glibc 2.36 does not have. |

```bash
git clone https://github.com/Vortx-AI/emem.git
cd emem
mdbook build docs && rm -f docs/book/book.toml
cargo build --release --bin emem-server
./target/release/emem-server
```

The `mdbook` step must come first. Without `docs/book/`, `cargo build` stops with an `include_dir!` error. The `rm`
step keeps the mdbook configuration out of the served site. The `Dockerfile` does the same two steps.

The binary listens on `0.0.0.0:5051` and stores data in `./var/emem`. `Cargo.lock` lists 737 packages (7 November
2026), so the first build takes much longer than later builds. [Develop emem](developers/developing.md) has the
full build and test workflow.

## Environment variables

Each default below comes from the code on 6 November 2026.

| Variable | Default | What it does |
|---|---|---|
| `EMEM_BIND` | `0.0.0.0:5051`; `127.0.0.1:5051` when TLS is on (binary only; the image always sets it) | The plain HTTP listener. Give `host:port`, not only a host. |
| `EMEM_DATA` | `./var/emem` (image: `/var/emem`) | The data directory. `:memory:` keeps the store in RAM only, and the node loses it at exit. |
| `EMEM_TLS_DOMAINS` | unset | Comma-separated domains. When set, the node gets certificates from Let's Encrypt and serves TLS. |
| `EMEM_TLS_BIND` | `0.0.0.0:443` | The TLS listener. The node uses it only when `EMEM_TLS_DOMAINS` has a value. |
| `EMEM_TLS_CONTACT` | unset | Your ACME contact, for example `mailto:ops@example.com`. With TLS on, the node does not start without it. |
| `EMEM_TLS_STAGING` | unset | `1` uses the Let's Encrypt staging directory. |
| `EMEM_AUTO_MATERIALIZE` | on | `0` or `false` stops the node from reading the archives on a miss. It then serves only what it holds. |
| `EMEM_PUBLIC_URL` | unset | The origin that a reader types. It names the `did:web` at `/.well-known/did.json`. If unset, the node uses the first TLS domain. |
| `EMEM_PEERS` | unset | Comma-separated origins that this node witnesses. See [Federation](federation.md) and `scripts/witness_peers.py`. |
| `EMEM_WITNESS_PUBKEY_B32` | unset | The key that your witness job signs with. The node publishes it in `did.json`. |
| `EMEM_AGENTS_WELL_KNOWN` | `config/emem-agents.json`, then `$EMEM_DATA/emem-agents.json` | The keys that you vouch for, served at `/.well-known/emem-agents.json`. |
| `EMEM_ENLISTMENT_ENFORCE` | on | Refuses shared-entity-space writes below tier T3. `0`, `off`, `false` or `shadow` accepts them and reports the tier. |
| `EMEM_FACT_PLANE_WRITERS` | unset | Comma-separated base32 public keys that may also write facts. The responder key and enrolled devices can always write them. |
| `EMEM_FACT_PLANE_OPEN` | unset | `1` lets any signing key write facts. Use it only on a test node or a closed network. |
| `EMEM_SLED_CACHE_BYTES` | `8g` | The page cache of the hot store, from `256m` to `64g`. The sled default of 1 GiB stalled a 58 GB store. |
| `EMEM_SLED_FLUSH_MS` | `200` | How often sled makes its log stable, from 50 to 5000. A read of a page written in this window waits for it. |
| `EMEM_TOPIC_BACKEND` | `ort` | The topic router for `/v1/ask`. `model2vec` is a pure-Rust fallback. `keyword` loads no model. If `ort` fails to load, the node uses `model2vec`. |

[Operate a public node](operators/operating.md) lists more variables for a large public node.

### Memory variables

| Variable | Default | What it does |
|---|---|---|
| `EMEM_DISABLE_LANCE` | unset | `1` makes `find_similar` and memory search use a full scan. |
| `EMEM_SSE_MAX_SUBS` | `256` | The maximum number of subscribers on `/v1/memory/sse`. More get HTTP 503. |
| `EMEM_MEMORY_SEARCH_POLL_SECS` | `60` | How often the search indexer looks for new notes. |
| `EMEM_MEMORY_SEARCH_MODEL_DIR` | `$EMEM_DATA/models/bge-base-en-v1.5` | The BGE model for memory search. `EMEM_TOPIC_MODEL_DIR` also works. |
| `EMEM_MEMORY_TTL_ENABLED` | unset | `1` starts the TTL pass. |
| `EMEM_MEMORY_TTL_INTERVAL_SECS` | `3600` | How often the TTL pass runs. |
| `EMEM_MEMORY_TTL_RESOURCE_DAYS` | `90` | The TTL of a `resource` note. `0` means no expiry. |
| `EMEM_MEMORY_TTL_EPISODIC_DAYS` | `30` | The TTL of an `episodic` note. |
| `EMEM_MEMORY_TTL_SEMANTIC_DAYS` | `0` | The TTL of a `semantic` note. |
| `EMEM_MEMORY_TTL_PROCEDURAL_DAYS` | `0` | The TTL of a `procedural` note. |
| `EMEM_MEMORY_TTL_CORE_DAYS` | `0` | The TTL of a `core` note. |
| `EMEM_MEMORY_CONSOLIDATION_ENABLED` | unset | `1` starts the daily combine pass, which merges old episodic notes into one note. |
| `EMEM_MEMORY_CONSOLIDATION_INTERVAL_SECS` | `86400` | How often the combine pass runs. |
| `EMEM_MEMORY_CONSOLIDATION_MIN_FILES` | `50` | The minimum number of episodic notes in one folder before the combine pass runs. |
| `EMEM_MEMORY_CONSOLIDATION_MIN_AGE_DAYS` | `7` | The combine pass skips notes newer than this. |
| `EMEM_EDGES_DEFAULT_LIMIT` | `100` | The number of edges that `edges_recall` returns when the caller gives no `limit`. |
| `EMEM_EDGES_MAX_LIMIT` | `1000` | The maximum `limit` for `edges_recall`. |
| `EMEM_RECALL_EDGES_PER_FACT` | `32` | The maximum edges per fact for `recall` with `include:["edges"]`. |
| `EMEM_RECALL_EDGES_TOTAL_CAP` | `256` | The maximum edges in total for `recall` with `include:["edges"]`. |

The TTL pass and the combine pass are off by default. You can run each of them again with the same result. The TTL pass
moves an expired path from `emem.memory_files` to `emem.memory_files_expired`. It does not delete the note bytes,
which stay in `emem.memory_file_blobs`. The combine pass writes one `semantic` note at `.consolidated/<unix_ts>.md`. It
marks each source note with `superseded_by`. Memory search needs the BGE model; without it, `/v1/memory/search`
returns HTTP 503.

### MCP task variables

These control the task registry for task-augmented `tools/call` (`tasks/get`, `tasks/result`, `tasks/list`,
`tasks/cancel`), MCP spec 2025-11-25.

| Variable | Default | What it does |
|---|---|---|
| `EMEM_MCP_MAX_TASKS` | `256` | The maximum number of live tasks, from 1 to 100000. |
| `EMEM_MCP_TASK_DEFAULT_TTL_MS` | `300000` | How long a finished task stays when the caller gives no `ttl`. It cannot be more than the maximum. |
| `EMEM_MCP_TASK_MAX_TTL_MS` | `1800000` | The maximum `ttl` that a caller can ask for. |
| `EMEM_MCP_TASK_POLL_INTERVAL_MS` | `1000` | The poll interval that the node sends back in `Task.pollInterval`. |

## Say who runs the node

Your node publishes a contact and an operator block in the agent card, the plugin manifest, the discovery card, the
operator attestation and `security.txt`. The binary has no default for these fields. Until you set them, the fields
are absent.

| Variable | Where it appears |
|---|---|
| `EMEM_CONTACT` | `contact` and `contact_email`, and the User-Agent of outbound requests |
| `EMEM_OPERATOR_NAME` | the `operator` block and the `author` field |
| `EMEM_OPERATOR_VENDOR` | the `vendor` slug in the plugin and MCP manifests. If unset, the manifests use the name. |
| `EMEM_OPERATOR_COUNTRY` | the `operator` block |
| `EMEM_OPERATOR_URL` | the `operator` block |

`security.txt` must have a `Contact` (RFC 9116). With no `EMEM_CONTACT`, it names the security policy of the project.
A comment says that this is not the operator of the node. Set `EMEM_CONTACT` so that reports about your node come to
you.

> Until 2026-08-27 the binary had these values built in. A self-hosted node then published the company and e-mail of
> the project author as its security contact. If your build is older, check your `/.well-known/security.txt`.

### Replace `/privacy` and `/terms`

The node serves `/privacy` and `/terms` from `PRIVACY.md` and `TERMS.md`, compiled into the binary. They describe the
emem.dev deployment: its retention, its lawful basis and its jurisdiction. They do not describe your node. Replace
both files and build again before other people use your node. No environment variable can do this.

## Turn on TLS

The binary can serve TLS itself, with `rustls` and Let's Encrypt (`rustls-acme`, TLS-ALPN-01). You do not need a
reverse proxy.

```bash
EMEM_TLS_DOMAINS=emem.example.com \
EMEM_TLS_CONTACT=mailto:ops@example.com \
./target/release/emem-server
```

The node then serves HTTPS on `0.0.0.0:443`. The plain listener moves to `127.0.0.1:5051`, so signed writes do not cross
the public network without encryption. Set `EMEM_BIND` only if you want the plain listener on another address.

The image works differently. Its entrypoint always sets `EMEM_BIND`, by default to `0.0.0.0:5051`. With TLS on in the
image, the plain listener therefore stays open on all interfaces. To keep it on loopback, add
`-e EMEM_BIND=127.0.0.1:5051` to `docker run`, or do not publish port 5051.

The first start takes about 15 to 30 seconds while Let's Encrypt checks the domain. Later starts use the certificate
in `$EMEM_DATA/acme.cache/`. Set `EMEM_TLS_STAGING=1` while you test, because the production service of Let's Encrypt
has rate limits.

To bind port 443 as a user that is not root, give the binary this permission after each build:

```bash
sudo setcap 'cap_net_bind_service=+ep' ./target/release/emem-server
```

The image already has this permission.

## Run the binary under systemd

This user unit runs a binary that you built. It does not need root.

```ini
# ~/.config/systemd/user/emem-server.service
[Unit]
Description=emem node
After=network-online.target

[Service]
Type=exec
Environment=EMEM_TLS_DOMAINS=emem.example.com
Environment=EMEM_TLS_CONTACT=mailto:ops@example.com
Environment=EMEM_DATA=%h/.emem
ExecStart=%h/emem/target/release/emem-server
Restart=on-failure
RestartSec=2s

[Install]
WantedBy=default.target
```

```bash
systemctl --user enable --now emem-server.service
loginctl enable-linger "$USER"   # keeps the unit running after you log out
```

emem.dev does not use this unit. It runs the image with `docker run` under a system unit,
[`deploy/systemd/emem-server.service`](https://github.com/Vortx-AI/emem/blob/main/deploy/systemd/emem-server.service). That unit is for a large store (about
98 GB). Read it for the stop timeout and the OOM settings, but do not copy it for a small node.

## Run a Hugging Face Space

The Space at <https://huggingface.co/spaces/vortx-ai/emem> runs the same image. Its source is in
[`huggingface-space/`](https://github.com/Vortx-AI/emem/tree/main/huggingface-space). Click "Duplicate this Space" to run a copy in your own Hugging Face
account. This is a quick way to test without a VM.

The Space pins an image digest, not `:latest`, so it can be older than emem.dev.

## What changes when you run your own node

| Topic | On your node |
|---|---|
| Rate limit | No emem.dev rate limit. Your hardware and the limits of the upstream archives apply. |
| Responder key | Your node signs with its own key. Your agents pin that key. |
| No internet | Set `EMEM_AUTO_MATERIALIZE=0`. The node serves only facts that it already holds. |
| Your own facts | Your responder key can write facts. Add other writer keys with `EMEM_FACT_PLANE_WRITERS`. |
| Same bytes | The same canonical CBOR gives the same `fact_cid`. Ed25519 gives the same signature for the same key and bytes. |
| Federation | Nobody has built read federation between nodes yet. Nodes can witness each other's log heads. See [Federation](federation.md). |

## Read more

[Operate a public node](operators/operating.md) covers the process model, backup and restore, sled tuning, geocoder warmup and
production incidents.
