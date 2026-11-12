# Reference offline verifiers

These scripts check emem's signed output without trusting emem.dev: a
receipt, a transparency-log proof, a memory note, a Merkle tree row, an
artifact hash and a document receipt. Each folder is named after the skill
of the emem plugin that describes the check.

They used to ship inside the plugin, in each skill's `scripts/`. Anthropic's
plugin directory takes only Markdown, text, JSON, YAML and images from a
publisher it has not verified, so on 12 November 2026 they moved here. The
skills now check through emem's MCP tools and HTTP routes, and say what to
compute again by hand. These files are the reference for that computation.

| Folder | Script | What it does |
|---|---|---|
| `emem-verify-receipt` | `verify.py` | rebuilds the preimage of a receipt (v1 or v2) and checks its Ed25519 signature against a pinned key; with `--facts`, checks the printed values too |
| `emem-transparency-log` | `verify_log.py` | checks a signed tree head against a pinned key, an inclusion proof and a consistency proof |
| `emem-document-evidence` | `verify_doc.py` | checks OCR and parse receipts and the hashes of the document |
| `emem-field-tokens` | `rehash.py` | computes the BLAKE3 of an artifact again and compares it with its cid |
| `emem-multi-agent-handoff` | `verify_note.py`, `seal_note.py` | checks which key wrote a memory note; seals a note to one key (`emem_sealed.py`) |
| `emem-tokenise-files` | `tree_proof.py` | builds a pointer.v1 index; checks a row's audit path and the bytes of a unit |
| `emem-long-horizon-memory` | `verify_note.py`, `watch_inbox.py` | the same note check; waits for a new note in an inbox |

Each folder carries its own `emem_crypto.py`, so a folder copied alone
still runs. That module is plain Python 3 BLAKE3 and Ed25519 with no
third-party package. `scripts/plugin_crypto_copies.py` keeps every copy,
and the signer's, byte-identical. `seal_note.py` needs the `cryptography`
package for X25519 and ChaCha20-Poly1305.

Run a check script with `--self-test` to test the module against 11
official BLAKE3 vectors and RFC 8032 vectors 1 to 3:

```sh
python3 tools/verifiers/emem-verify-receipt/verify.py --self-test
```

`verify.py` and `verify_log.py` check a signature against a pinned key:
emem.dev's published key unless you pass `--pubkey`. A receipt or a head
names its own signer, so a check against that name alone would pass for
anyone's key. A receipt signs the fact ids, not the values printed beside
them; `verify.py --facts DIR` checks each value against its fact's CBOR
(`GET /v1/facts/<cid>` with `Accept: application/cbor`, saved as
`<cid>.cbor`).

Read a script before you run it. None of them reads a key or a seed.
`verify_note.py` makes one GET for an author's key card only when given
`--read-key-card`, and `watch_inbox.py` polls an inbox. The others make
no network call.
