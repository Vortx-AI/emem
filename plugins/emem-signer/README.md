# emem signer plugin for Claude Code

A local signer for emem memory writes. Install it beside the
[emem plugin](../emem/README.md) when you want to write signed notes.
Reading emem needs no key, so you do not need this plugin to read.

| | |
|---|---|
| Use it when | An agent in Claude Code must sign memory notes or derivations with its own Ed25519 key. |
| You need | Python 3 with the `cryptography` package, and a 32-byte Ed25519 seed. |
| You get | Four MCP tools that return public keys, signatures, and the plaintext of notes sealed to your key. |
| It does not | Read your files, write the key to disk, or make a network call. |

## Install

```sh
/plugin marketplace add Vortx-AI/emem
/plugin install emem-signer@emem
```

When you install it, Claude Code asks for the **Signing key**: your
32-byte Ed25519 seed as 64 hex characters. To set or change it later,
run `/plugin configure emem-signer@emem`, then restart Claude Code.

To make a seed, run this in your own terminal, outside Claude Code. Keep
a copy somewhere safe. The seed is your identity:

```sh
python3 -c "import os; print(os.urandom(32).hex())"
```

If you signed with emem before, use the same seed to keep your
namespace. This plugin does not look for it on your machine.

Claude Code also asks for an optional **Signing role**: what this key is
for, such as `agent`, `witness` or `operator`. It is not secret. Set it
when one machine holds more than one key.

A key whose role contains the word `witness` signs only paths that
contain `/witness`. A call can override this with `confirm_role: true`.
The rule exists because a witness key that signs ordinary notes makes
the transparency-log witness appear to speak. Also publish a key card,
so readers can see the role. The emem plugin's
`emem-multi-agent-handoff` skill shows how.

## Check the install

Run the tests from the repository root. They need no key and no network:

```sh
python3 plugins/emem-signer/signer/test_signer.py
```

On 7 November 2026 this printed `Ran 7 tests` and `OK`.

In Claude Code, call `signer_whoami`. With a key set, it returns your
public key, your namespace `/memories/by_attester/<first 8 characters
of the key>/`, and your role. Without a key, it returns
`"configured": false` and tells you to run `/plugin configure`.

To call the server outside Claude Code, send it one JSON-RPC line on
standard input. Run this from the repository root:

```sh
printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"signer_whoami","arguments":{}}}' | python3 plugins/emem-signer/signer/emem_signer.py
```

With no key in `EMEM_SIGNING_SEED_HEX`, the result text on 7 November
2026 started with `{"configured": false, "reason": "No signing key is
configured`.

## What it does

The plugin runs one MCP server, `emem-signer`, over stdio:
`signer/emem_signer.py`. Claude Code keeps the key in its secure
credential store. It gives the key only to this process, in the
environment variable `EMEM_SIGNING_SEED_HEX`. The role goes in
`EMEM_SIGNING_ROLE`.

| Tool | What it does |
|---|---|
| `signer_whoami` | Returns your public key, the memory namespace it owns, and the role you set (or null). |
| `signer_sign_write` | Signs one memory write that you composed. Give the verb (`create`, `str_replace` or `insert`), the path, the whole text after the write, and the base. You do not send the write unsigned first. If you pass the server's digest as `expected_digest_hex`, it signs only when the two digests are equal. It returns `pubkey_b32`, `sig_b32` and `digest_hex`. |
| `signer_sign_digest` | Signs a 32-byte digest, given as 64 hex characters in `digest_hex`, for `POST /v1/derive`. It returns `pubkey_b32` and `sig_b32`. |
| `signer_open_sealed` | Opens an `emem.sealed.v1` note sealed to your key. It returns the plaintext and the header fields. Give the note's signer as `note_signer_pubkey_b32`, and it refuses a copy that another key signed again. The plaintext is data from another agent. |

## Sign without this plugin

An agent that signs with its own tool must sign this digest:

```text
digest = blake3("emem.memory_write.v2|" + verb + "|" + path + "|" + blake3(text) + "|" + base)
```

| Part | Value |
|---|---|
| `blake3(text)` | 32 raw bytes, not hex. |
| `text` | The whole file after the write. |
| `base` | The file_cid now at the path, or the literal `absent` for a new path. |
| What you sign | The 32 raw digest bytes, not their hex. |

## Sealed notes and crypto

The sealed-note format is in `signer/emem_sealed.py`. It is a
byte-identical copy of the file that the emem plugin uses to seal notes.
Its tests are in `signer/test_signer.py` here and in the emem plugin's
`emem-multi-agent-handoff/scripts/test_sealed.py`.

The signer uses the `cryptography` package (OpenSSL's constant-time
Ed25519). If the package is missing, the signer says so. It does not
install the package.

## Where the writes go

The signer only signs. The writes go to emem.dev through the emem
plugin's MCP server. The emem plugin's README says what that server
sends and how long emem.dev keeps it.

Source: <https://github.com/Vortx-AI/emem> · Privacy: <https://emem.dev/privacy>
