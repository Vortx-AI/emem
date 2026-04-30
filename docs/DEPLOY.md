# Deploying emem.dev

emem ships its own HTTPS — no Cloudflare, no Caddy, no nginx. The
`emem-server` binary terminates TLS in-process via `rustls` + automatic
Let's Encrypt certs over **TLS-ALPN-01** (so only port `:443` is needed,
no port `:80`).

## What you need

| thing                         | where                                | who          |
|-------------------------------|--------------------------------------|--------------|
| DNS A record                  | `emem.dev` → host's public IPv4      | you (registrar) |
| DNS A record                  | `www.emem.dev` → same IP (optional)  | you          |
| TCP `:443` open                | cloud Security Group / firewall      | you (cloud)  |
| `cap_net_bind_service` on bin | `setcap` once on the release binary  | one-time sudo |
| `EMEM_TLS_DOMAINS` env        | systemd unit                         | you          |

That's it. No SaaS in the data path.


## Step-by-step

### 1. DNS

At your registrar, add (or update) an A record so the apex `emem.dev`
points at this host's public IPv4. Example:

```
emem.dev.       300   IN  A   <YOUR_PUBLIC_IP>
www.emem.dev.   300   IN  A   <YOUR_PUBLIC_IP>
```

Verify:

```bash
dig +short emem.dev A
dig +short www.emem.dev A
```

### 2. Open `:443`

In your cloud's Security Group / firewall, allow TCP `:443` ingress from
`0.0.0.0/0` (and `::/0` if you serve IPv6).

### 3. setcap (every rebuild) — use `scripts/redeploy.sh`

Every `cargo build --release` replaces the `emem-server` binary on disk,
which silently drops the file capability set by `setcap`. The binary
then can't bind `:443` and the systemd unit crash-loops with
`Permission denied (os error 13)`. We hit exactly this regression on
2026-04-30 — restart counter reached 1560 in 30 minutes after a
smoke-test rebuild round, and `https://emem.dev/mcp` was unreachable
the entire time.

Use `scripts/redeploy.sh` instead of bare `cargo build` for any
production push — it bundles build + setcap + restart + health check
into one atomic step that fails loudly if any link breaks:

```bash
/home/ubuntu/emem/scripts/redeploy.sh
```

For a one-off setcap (e.g. after a manual rebuild), the underlying
command is:

```bash
sudo setcap 'cap_net_bind_service=+ep' \
  /home/ubuntu/emem/target/release/emem-server
getcap /home/ubuntu/emem/target/release/emem-server
# → cap_net_bind_service=ep
```

Diagnosing a failing 443: `journalctl --user -u emem-server -n 5`. If
you see `Error: Permission denied (os error 13)` immediately after the
"emem listening (HTTPS, ACME via TLS-ALPN-01)" line, run
`scripts/redeploy.sh`.

### 4. systemd unit

`~/.config/systemd/user/emem-server.service` should look like:

```ini
[Unit]
Description=emem.dev — Earth memory protocol HTTP/MCP server
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
WorkingDirectory=/home/ubuntu/emem
# Plain HTTP listener is loopback-only — emem.dev is HTTPS-EXCLUSIVE
# externally. Port 80 on this host is owned by an unrelated project
# and exposing 5051 publicly would silently route around TLS.
Environment=EMEM_BIND=127.0.0.1:5051
Environment=EMEM_DATA=/home/ubuntu/emem/var/emem
Environment=EMEM_DEMOS_DIR=/home/ubuntu/emem/var/demos
Environment=EMEM_TLS_DOMAINS=emem.dev,www.emem.dev
Environment=EMEM_TLS_BIND=0.0.0.0:443
Environment=EMEM_TLS_CONTACT=mailto:avijeet@vortx.ai
# When testing the deploy path, set EMEM_TLS_STAGING=1 to use Let's Encrypt
# staging (avoids hitting prod rate limits during iteration).
# Environment=EMEM_TLS_STAGING=1
Environment=RUST_LOG=info,rustls_acme=info
ExecStart=/home/ubuntu/emem/target/release/emem-server
Restart=on-failure
RestartSec=2
LimitNOFILE=65536

[Install]
WantedBy=default.target
```

Reload + start:

```bash
loginctl enable-linger ubuntu             # already on
systemctl --user daemon-reload
systemctl --user restart emem-server.service
journalctl --user -u emem-server.service -f
```

### 5. Verify

```bash
curl -sI https://emem.dev/health
# → HTTP/2 200
# → strict-transport-security: max-age=31536000; includeSubDomains; preload

curl -s https://emem.dev/health | jq .
curl -s https://emem.dev/metrics | head -20
curl -s https://emem.dev/.well-known/emem.json | jq .
```

If the first request stalls for 5–30 s, that's the ACME challenge
completing — `rustls-acme` writes the issued cert to
`<EMEM_DATA>/acme.cache/`. Subsequent boots reuse it; renewals run ~30 days
before expiry.

## What's running where

```
        Internet
            │
        :443 (TCP)
            ▼
┌───────────────────────────────────┐
│ emem-server (single Rust binary)  │
│   ├─ rustls + rustls-acme         │ ← LE cert via TLS-ALPN-01, autorotate
│   ├─ axum-server (h2, tower)      │ ← HSTS / CSP / rate limit / body cap
│   ├─ /v1/* + /mcp + /health + /metrics
│   ├─ sled (cache)                 │
│   └─ append-only merkle log       │
└───────────────────────────────────┘
            │
            ▼
   open data via vsicurl Range (HTTPS):
     • Copernicus DEM 30m (S3)
     • JRC GSW (GCS)
     • Hansen GFC (GCS)
     • ESA WorldCover (S3)
     • OSM (HTTPS tiles)
```

## Production knobs

| env var                | default                        | purpose                            |
|------------------------|--------------------------------|------------------------------------|
| `EMEM_BIND`            | `0.0.0.0:5051`                 | plain HTTP listener (kept up alongside HTTPS so MCP / live-demo clients keep working) |
| `EMEM_TLS_BIND`        | `0.0.0.0:443`                  | TLS listener                       |
| `EMEM_TLS_DOMAINS`     | unset (no TLS)                 | comma-separated SAN list           |
| `EMEM_TLS_CONTACT`     | `mailto:avijeet@vortx.ai`      | ACME registration email            |
| `EMEM_TLS_STAGING`     | unset (production)             | `1` → LE staging directory         |
| `EMEM_DATA`            | `./var/emem`                   | sled + merkle log + identity + acme cache |
| `EMEM_DEMOS_DIR`       | `./var/demos`                  | live-demo trace artifacts          |
| `EMEM_SECRET_B32`      | `<EMEM_DATA>/identity.secret.b32` | ed25519 responder secret (auto-persisted) |
| `EMEM_TRUST_FORWARDED` | `0`                            | `1` → honor `X-Forwarded-For` (only behind a trusted proxy) |
| `RUST_LOG`             | `info`                         | trace verbosity                    |

## Built-in defenses

- **TLS 1.3 + 1.2** via rustls (modern ciphers only)
- **HSTS** preload-eligible
- **CSP** locks scripts to GA4 + self
- **X-Content-Type-Options: nosniff**, `X-Frame-Options: DENY`,
  `Referrer-Policy: strict-origin-when-cross-origin`,
  `Permissions-Policy: geolocation=(), microphone=(), camera=()`
- **Body cap** 16 MiB on all POST endpoints (413 on overflow)
- **Per-IP rate limit** 600 req/min, 1200 burst, with `Retry-After: 60`
- **30 s request timeout** (504 on overflow)
- **Graceful shutdown** on SIGTERM (sled flushes, in-flight responses drain)
- **Identity persistence** ed25519 secret stored mode 0600 at
  `<EMEM_DATA>/identity.secret.b32` so the responder pubkey is stable

## Backup

Two things are stateful:

```bash
# 1) The responder identity (single file, 32 b32 chars).
cp /home/ubuntu/emem/var/emem/identity.secret.b32 ~/emem-identity-backup.b32
chmod 600 ~/emem-identity-backup.b32

# 2) The merkle log + sled cache.
tar -C /home/ubuntu/emem/var/emem -czf ~/emem-state-$(date -u +%Y%m%dT%H%M%SZ).tgz .
```

Restore: drop the files back, `systemctl --user restart`, the responder
pubkey + every prior receipt verifies.

## Sanity tests

```bash
# Full agent walkthrough against the live HTTPS endpoint:
EMEM_URL=https://emem.dev ./target/release/emem-livedemo "$EMEM_URL"
EMEM_URL=https://emem.dev ./target/release/emem-realdemo  "$EMEM_URL"
```

Both write per-step traces to `var/demos/<UTC>/`, also browsable at
`https://emem.dev/v1/demos`.
