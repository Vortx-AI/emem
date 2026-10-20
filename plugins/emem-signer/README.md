# emem signer plugin for Claude Code

A local signer for emem memory writes. Install it beside the
[emem plugin](../emem/README.md) when you want to write signed notes.
Reading emem needs no key, so you do not need this plugin to read.

## Install

```sh
/plugin marketplace add Vortx-AI/emem
/plugin install emem-signer@emem
```

When you install it, Claude Code asks for the **Signing key**: your
32-byte Ed25519 seed as 64 hex characters. To set or change it later,
run `/plugin configure emem-signer@emem`, then restart Claude Code.

To make a seed, run this in your own terminal, outside Claude Code, and
keep a copy somewhere safe. The seed is your identity:

```sh
python3 -c "import os; print(os.urandom(32).hex())"
```

If you signed with emem before, use the same seed to keep your
namespace. This plugin does not look for it on your machine.

## What it does

The plugin runs one MCP server, `emem-signer`, over stdio:
`signer/emem_signer.py`. Claude Code keeps the key in its secure
credential store and gives it only to this process, in its
environment. The signer:

- does not read your files;
- does not write the key to disk;
- makes no network call;
- returns only public keys and signatures.

Its tools:

| Tool | Does |
|---|---|
| `signer_whoami` | Returns your public key and the memory namespace it owns. |
| `signer_sign_write` | Signs one memory write you composed. It signs only when its own digest of the write equals the digest the server names. |
| `signer_sign_digest` | Signs a 32-byte digest, for `POST /v1/derive`. |

It signs with the `cryptography` package (OpenSSL's constant-time
Ed25519). If the package is missing, the signer says so and does not
install it.

The writes themselves go to emem.dev through the emem plugin's MCP
server. The emem plugin's README says what is sent and how long it is
kept.

Source: <https://github.com/Vortx-AI/emem> · Privacy: <https://emem.dev/privacy>
