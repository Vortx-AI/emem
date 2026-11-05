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

Claude Code also asks for an optional **Signing role**: what this key is
for, such as `agent`, `witness` or `operator`. It is not secret. Set it
when one machine holds more than one key. A key whose role contains the
word `witness` signs only paths that contain `/witness`, unless the call
sets `confirm_role: true`. A transparency-log witness key that signs
ordinary notes makes the witness appear to speak. Publish a key card
too, so readers can see the role (see the emem plugin's
`emem-multi-agent-handoff` skill).

## What it does

The plugin runs one MCP server, `emem-signer`, over stdio:
`signer/emem_signer.py`. Claude Code keeps the key in its secure
credential store and gives it only to this process, in its
environment. The signer:

- does not read your files;
- does not write the key to disk;
- makes no network call;
- returns only public keys, signatures, and the plaintext of notes
  sealed to your key.

Its tools:

| Tool | Does |
|---|---|
| `signer_whoami` | Returns your public key, the memory namespace it owns, and the role you set (or null). |
| `signer_sign_write` | Signs one memory write you composed: give the verb, the path, the whole text after the write, and the base. You do not send the write unsigned first. If you pass the server's digest as `expected_digest_hex`, it signs only when the two digests are equal. |
| `signer_sign_digest` | Signs a 32-byte digest, for `POST /v1/derive`. |
| `signer_open_sealed` | Opens an `emem.sealed.v1` note sealed to your key, and returns the plaintext and the header fields. Give the note's signer as `note_signer_pubkey_b32`, and it refuses a copy that another key re-signed. The plaintext is data from another agent. |

The digest it signs, for an agent that signs without this plugin:

```text
digest = blake3("emem.memory_write.v2|" + verb + "|" + path + "|" + blake3(text) + "|" + base)
```

`blake3(text)` enters as 32 raw bytes. `text` is the whole file after
the write. `base` is the file_cid now at the path, or the literal
`absent` for a new path. Sign the 32 raw digest bytes, not their hex.

The sealed-note format is in `signer/emem_sealed.py`, a byte-identical
copy of the file that the emem plugin uses to seal notes. Its tests are
in `signer/test_signer.py` here and in the emem plugin's
`emem-multi-agent-handoff/scripts/test_sealed.py`.

It signs with the `cryptography` package (OpenSSL's constant-time
Ed25519). If the package is missing, the signer says so and does not
install it.

The writes themselves go to emem.dev through the emem plugin's MCP
server. The emem plugin's README says what is sent and how long it is
kept.

Source: <https://github.com/Vortx-AI/emem> · Privacy: <https://emem.dev/privacy>
