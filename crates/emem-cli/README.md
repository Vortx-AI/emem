# emem-cli

`emem-cli` holds the programs you run: the `emem-server` responder, the
`emem` command-line tool, demo clients and two admin tools. It has no
library code. Each program is one file in `src/`.

You need this crate to run your own emem node, or to check a receipt or a
`cell64` without a server. To only call emem.dev, you do not need it.

| | |
|---|---|
| Version | 2.4.4 (workspace version in the root `Cargo.toml`) |
| Rust | 1.91 or later |
| Programs | 10 binaries, listed in `Cargo.toml` |
| Tests | 8 unit tests: 6 in `emem-server.rs`, 1 in `emem-sled-slim.rs`, 1 in `emem-demo-device.rs` |
| Ships as | the `emem-server` binary in the container image. The project publishes no other binary; build them from source. |

## Try it

`emem verify` checks the Ed25519 signature on a receipt. It uses the same
preimage function as `POST /v1/verify_receipt`. First get a receipt:

```sh
curl -s -X POST https://emem.dev/v1/recall \
  -H 'Content-Type: application/json' \
  -d '{"cell":"defi.zb592.nUkO.zEzE","bands":["copdem30m.elevation_mean"]}' \
  | jq .receipt > receipt.json
```

Then check it offline with the CLI (from a checkout of the repository):

```sh
cargo run -p emem-cli --bin emem -- verify receipt.json
```

The command prints JSON with `valid`, `signer_pubkey_b32`, `pubkey_source`
and `preimage_blake3_hex`. The exit code is 0 for a valid signature and 1
for an invalid one.

We did not run `cargo` for this page, because the host had no Rust
toolchain. The flags and output fields come from `src/main.rs`. We ran the
`curl` command and sent the same receipt to the server check on
7 November 2026:

```sh
jq '{receipt: .}' receipt.json \
  | curl -s -X POST https://emem.dev/v1/verify_receipt \
      -H 'Content-Type: application/json' -d @- \
  | jq -c '{valid, preimage_version, signer_pubkey_b32}'
# {"valid":true,"preimage_version":2,"signer_pubkey_b32":"777er3yihgifqmv5hmc2wwmyszgddzderzhsx6rex4yoakwomvka"}
```

## The `emem` command

| Subcommand | What it prints |
|---|---|
| `manifests` | the content ids of the band, function and source registries |
| `bands`, `functions`, `sources` | the registry as JSON |
| `errors` | the error code catalog as JSON (24 codes; see the note below) |
| `keygen` | a new Ed25519 key pair in lower-case base32. The output includes the secret key. |
| `cell <cell64>` | the 64-bit cell id, in decimal and hex, with its resolution and base cell |
| `cell-encode <u64>` | the `cell64` text for a 64-bit cell id |
| `verify <path or ->` | the signature check of one receipt. Use `-` to read the receipt from standard input. |

`verify` selects the public key in this order:

1. The `--pubkey <base32>` flag.
2. `--base-url <url>`, or the `EMEM_BASE_URL` environment variable. The
   CLI then reads `responder.pubkey_b32` from `/.well-known/emem.json`.
3. The `responder` field in the receipt.

Option 3 only proves that the receipt matches the key inside it. It does not prove
who signed it. To prove that, use option 1 or 2.

`emem errors` lists 24 codes. The `ErrorCode` enum in `emem-core` has 27.
The list does not include `NoGeocoderMatch`, `UnaddressableSubject` and
`InvalidArgument`.

## Run your own node

```sh
cargo build --release --bin emem-server
EMEM_DATA=./var/emem ./target/release/emem-server
```

The server listens on `0.0.0.0:5051` with plain HTTP. On the first start it
makes a responder key and writes it to `<EMEM_DATA>/identity.secret.b32`
with mode 0600. Later starts read the same key, so old receipts still
verify. We did not run these commands for this page (no Rust toolchain).

| Variable | Default | Effect |
|---|---|---|
| `EMEM_BIND` | `0.0.0.0:5051`, or `127.0.0.1:5051` when TLS is on | plain HTTP listener |
| `EMEM_DATA` | `./var/emem` | data directory. `:memory:` keeps nothing and makes a new key at each start. |
| `EMEM_SECRET_B32` | none | responder secret key. It replaces the key file. |
| `EMEM_TLS_DOMAINS` | none | comma-separated domains. When set, the server gets a certificate through ACME (TLS-ALPN-01). |
| `EMEM_TLS_BIND` | `0.0.0.0:443` | TLS listener |
| `EMEM_TLS_CONTACT` | none | ACME contact, for example `mailto:you@example.org`. The server does not start with TLS on and this unset. |
| `EMEM_TLS_STAGING` | off | `1` uses the staging ACME directory |
| `EMEM_TILE_STORE_GB` | `40` | disk budget for local copies of forest rasters. `0` turns it off. |
| `EMEM_OVERTURE_SKIP_WARMUP` | off | `1` skips the Overture cache warm-up at start. Use it on a node without internet. |
| `EMEM_SHUTDOWN_GRACE_S` | `3` (30 at most) | seconds that open requests get after a stop signal |
| `EMEM_STOP_GRACE_S` | `60` | the container stop timeout. The server uses it to limit the database close. |

Other modules read more variables, for example `EMEM_FACT_PLANE_OPEN` and
`EMEM_EARTHDATA_TOKEN`. For deployment and more variables, read
[docs/operators/operating.md](../../docs/operators/operating.md). That page
does not list every variable.

Production (emem.dev) does not run a binary built on the host. It runs the
container image from the root `Dockerfile`. That image holds only
`emem-server`.

## Demo clients and admin tools

All demo clients use `http://localhost:5051` (or `127.0.0.1:5051`) by
default. Give another URL as the first argument or in `EMEM_BASE_URL`.

| Binary | What it does | Writes to the server |
|---|---|---|
| `emem-demo` | signs 3 facts with a new key, posts them, then calls the read primitives | yes, `/v1/attest_cbor` |
| `emem-livedemo` | calls every primitive and saves each request and response under `./var/demos/<UTC time>/` | yes, `/v1/attest_cbor` |
| `emem-realdemo` | reads Copernicus DEM tile headers from public S3 and attests two bands per place | yes, `/v1/attest_cbor` |
| `emem-connect-demo` | runs the steps of [examples/connect-and-evolve.md](../../examples/connect-and-evolve.md) | yes, `/v1/attest_cbor` and `/v1/edges` |
| `emem-ask-eval` | sends a fixed set of questions to `/v1/ask`. It exits 1 if one routes to the wrong topic or has no receipt. | no |
| `emem-demo-device` | enrolls this host as a demo device and publishes an ftrace window. Default URL `https://emem.dev`. Needs the operator endorser key. | yes |
| `emem-sled-slim` | copies the live sled trees to a new store. Dry run by default; `--apply` writes. | local store only |
| `emem-purge-fnkey` | removes cached facts with a given `fn_key` from the sled store. Dry run by default. | local store only |

Run the demo clients against your own node, not emem.dev. Stop the server
before you run the two admin tools: sled locks the store.

## Read more

- [docs/developers/developing.md](../../docs/developers/developing.md): build, test and lint rules
- [docs/operators/operating.md](../../docs/operators/operating.md): deployment, environment variables, TLS
- [docs/protocol.md](../../docs/protocol.md): receipt preimages, section 7
- [emem-storage](../emem-storage/README.md): the receipt signer and the log
- [emem-codec](../emem-codec/README.md): the `cell64` form that `emem cell` decodes
