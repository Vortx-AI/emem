# Self-host

The public responder at `https://emem.dev` is free, agent-friendly, and
rate-limited. If you need unlimited throughput, deterministic latency,
private data alongside open data, or a regional replica close to your
agents, run emem yourself.

## Docker (one line)

```bash
docker run --rm -p 5051:5051 ghcr.io/vortx-ai/emem:latest
```

Hit it:

```bash
curl http://localhost:5051/health
curl -sX POST http://localhost:5051/v1/locate \
    -H 'content-type: application/json' \
    -d '{"q":"South Mumbai"}'
```

That's a functional emem responder on port 5051 with REST + MCP on the
same port. No keys, no external sync, lazy-materialization from open
data on first recall.

## Persistent storage

State lives under `EMEM_DATA` (default `./var/emem/`). Mount a host path
so identity + cache survive container restarts:

```bash
docker run --rm \
    -v $HOME/.emem:/data \
    -e EMEM_DATA=/data \
    -p 5051:5051 \
    ghcr.io/vortx-ai/emem:latest
```

What's inside `EMEM_DATA`:

- `identity.secret.b32` — the responder's ed25519 secret key. **Back this up.**
  If you lose it, agents that pinned your `responder_pubkey_b32` will reject
  every receipt you sign with the new identity.
- `cache/` — sled hot cache (signed facts, lazy-materialized payloads)
- `attest/` — append-only Merkle log of attestations (write surface)

## Build from source

```bash
git clone https://github.com/Vortx-AI/emem.git
cd emem
cargo build --release --bin emem-server
./target/release/emem-server          # binds 0.0.0.0:5051
```

Requirements: Rust 1.85+, ~12 GB free disk (for `target/`), 4 GB RAM
warm. First build ~5 min; incremental ~30 s.

## Environment

| Var | Default | Notes |
|-----|---------|-------|
| `EMEM_BIND`             | `0.0.0.0:5051`     | plain HTTP listener |
| `EMEM_DATA`             | `./var/emem`       | persistent storage root |
| `EMEM_TLS_BIND`         | `0.0.0.0:443`      | TLS listener (only when TLS active) |
| `EMEM_TLS_DOMAINS`      | unset              | comma-separated; enables ACME + TLS |
| `EMEM_TLS_CONTACT`      | unset              | required when TLS is on |
| `EMEM_TLS_STAGING`      | `0`                | use Let's Encrypt staging |
| `EMEM_AUTO_MATERIALIZE` | `1`                | empty recall fetches upstream + signs |
| `EMEM_TOPIC_BACKEND`    | `ort`              | or `model2vec` (pure-Rust fallback) |
| `EMEM_GALILEO_VARIANT`  | `base`             | Galileo encoder variant |
| `EMEM_HUNT_CONCURRENCY` | `32`               | parallel cell sweeps for `/v1/hunt` |

`EMEM_BIND` requires `host:port` (not bare host). To bind `:443` as a
non-root user under systemd, set the file capability after every build:

```bash
sudo setcap 'cap_net_bind_service=+ep' /path/to/emem-server
```

## TLS

emem-server speaks TLS via embedded `axum-server` + `rustls-acme` —
no nginx in front needed.

```bash
EMEM_BIND=0.0.0.0:5051 \
EMEM_TLS_BIND=0.0.0.0:443 \
EMEM_TLS_DOMAINS=emem.example.com \
EMEM_TLS_CONTACT=mailto:ops@example.com \
./target/release/emem-server
```

First boot: ~15–30 s while ACME validates. Subsequent boots: ~2 s
because the certificate is cached in `EMEM_DATA/tls-acme/`. Use
`EMEM_TLS_STAGING=1` while testing — Let's Encrypt prod rate-limits.

## systemd (user unit)

The canonical deploy on `emem.dev` is a systemd user unit. A minimal
version:

```ini
# ~/.config/systemd/user/emem-server.service
[Unit]
Description=emem.dev — Earth memory protocol server
After=network-online.target

[Service]
Type=exec
Environment=EMEM_BIND=0.0.0.0:5051
Environment=EMEM_TLS_BIND=0.0.0.0:443
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
loginctl enable-linger $USER   # so the unit survives logout
```

## Hugging Face Space

There's a Gradio-wrapped responder you can run as a Hugging Face Space.

```
https://github.com/Vortx-AI/emem/tree/main/huggingface-space
```

Click "Duplicate this space" — it boots a personal replica in your own
HF account. Useful for small evaluations without operating a VM.

## What you get when you self-host

- **Unlimited throughput** — no public rate limit
- **Your own responder pubkey** — agents in your stack pin yours
- **Air-gap-friendly** — turn off `EMEM_AUTO_MATERIALIZE` and only serve
  what you've explicitly attested
- **Private bands alongside open ones** — attest your own facts under
  schemes that route through your storage
- **Determinism guarantees** — same canonical CBOR, same CID, same
  signature, every time

## Operational reference

For the long version (process model, backup/restore, sled tuning, jepa
sidecar, geocoder warmup, all the things that bit us in production), see
[Operators / Operating](./operators/operating.html).
