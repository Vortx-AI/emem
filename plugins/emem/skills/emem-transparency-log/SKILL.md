---
name: emem-transparency-log
description: Audits emem's append-only RFC 6962 transparency log offline. Verifies a signed tree head, proves a log entry or a memory note is committed under it, proves a later head extends an earlier one without rewriting history, and reads which independent witnesses countersigned. Use when the question is not "is this fact signed" but "could this responder have shown someone else a different history", when pinning evidence for an audit, or before relying on a note whose deletion or rewrite would matter. Ships a small Python verifier.
---

# emem-transparency-log

> **Network use.** The commands in this skill send and receive JSON (and, where a step says so, an image or a raster file) to `https://emem.dev` only, the service the plugin's MCP server connects to. Nothing they download is executed. The files under `scripts/` read local files and make no network calls.

A receipt proves the responder signed some fact cids. It cannot prove
the responder showed everyone the same history. The transparency log
does: every attestation and every memory write is a leaf in one RFC 6962
Merkle tree, and the responder signs its head. Pin a head today, and
tomorrow you can prove the log only grew.

| Call | MCP | Gives |
|---|---|---|
| `GET /v1/log/sth` | `emem_log_sth` | the signed tree head: `tree_size`, `root_b32`, `signed_at`, signature |
| `GET /v1/log/inclusion?leaf_index=<i>` or `?entry_hash=<b32>` | `emem_log_inclusion` | the audit path from one leaf to the current root |
| `GET /v1/log/consistency?first=<n>&second=<m>` | `emem_log_consistency` | the proof that the size-n tree is a prefix of the size-m tree |
| `GET /v1/log/witnesses` | `emem_log_witnesses` | independent co-signatures over `(tree_size, root)` |
| `GET /v1/log/entries?start=<i>&end=<j>` | | the raw entries, so a third party can read what else is in the tree |

All reads, no key. The tree hashes with BLAKE3: leaf =
`blake3(0x00 || entry_hash)`, node = `blake3(0x01 || left || right)`,
lone nodes promoted rather than duplicated.

## Verify a head, an inclusion, and growth

`scripts/verify_log.py` ships with this skill (`${CLAUDE_SKILL_DIR}` is filled
in by Claude Code). It makes no network calls and imports nothing outside
the standard library except `scripts/emem_crypto.py`, shipped with this skill, a
plain-Python BLAKE3 and Ed25519 verifier; `--self-test` checks that
module against the official test vectors. Save each response to a file
and pass the file:

```sh
V="${CLAUDE_SKILL_DIR}/scripts/verify_log.py"
curl -sf https://emem.dev/v1/log/sth > old_sth.json
python3 "$V" sth old_sth.json

curl -sf "https://emem.dev/v1/log/inclusion?leaf_index=12345" > inc.json
python3 "$V" inclusion inc.json

# later: a new head, and the proof that links the two
curl -sf https://emem.dev/v1/log/sth > new_sth.json
OLD=$(jq .sth.tree_size old_sth.json); NEW=$(jq .sth.tree_size new_sth.json)
curl -sf "https://emem.dev/v1/log/consistency?first=$OLD&second=$NEW" > cons.json
python3 "$V" consistency old_sth.json new_sth.json cons.json
```

Recorded on 2026-09-28:

```
sth          VALID    tree_size=2466701 signed_at=2026-09-28T13:32:19Z signer=777er3yihgifqmv5hmc2wwmyszgddzderzhsx6rex4yoakwomvka
inclusion    VALID    leaf_index=12345 path=22 hashes
consistency  VALID    2466701 -> 2466702, 12 hashes
```

With one byte changed (tree size bumped, an audit-path hash swapped, a
consistency hash swapped) each check printed `INVALID` and exited 1.

Always pass `second` explicitly. The log grows every few seconds, so a
consistency proof against "the current size" will not match the head
you fetched a moment earlier; the script refuses mismatched sizes
rather than guessing.

## Prove a memory note is in the log

A memory write is a log entry too. Pass the base32 BLAKE3 of the note's
content as `entry_hash`, and the response says
`matched: "memory_note_content"` with the note's `file_cid`:

```sh
H=<base32 blake3 of the note content, e.g. from authorship.body_hash_hex>
curl -sf "https://emem.dev/v1/log/inclusion?entry_hash=$H" > note_inc.json
jq '{matched, memory_note_file_cid, leaf_index}' note_inc.json
python3 "$V" inclusion note_inc.json
```

Recorded on 2026-09-28: a note in `/memories/by_attester/k572x7go/`
matched at `leaf_index` 2194799 and its path verified. The proof covers
the log entry, not the content itself: to close the loop, fetch that
entry from `/v1/log/entries` and check its `content_blake3` equals the
hash you asked with, as the response's `verify` field says.

## Witnesses

```sh
curl -sf "https://emem.dev/v1/log/witnesses?limit=5" \
  | jq '{independent_witness_count, independent_operator_domains,
         head_is_independently_witnessed, freshest_independent_witness_entries_behind}'
```

A witness attests only the prefix it signed. On 2026-09-28 there were
111 independent witnesses and one independent operator (`geo.qa`), and
the freshest independent co-signature was 732 entries behind the head,
so `head_is_independently_witnessed` was false. Read those fields rather
than `head_is_witnessed`, which also counts the node's own liveness
canary. To check a co-signature, verify it offline, then run a
consistency proof from its `tree_size` to the head you hold.

## What this does not prove

- That a fact is true. It proves the history was not rewritten.
- That the receipt's own `merkle_proof` is this tree. That proof binds a
  fact into its signing batch; it is checked by
  [`emem-verify-receipt`](../emem-verify-receipt/SKILL.md).
- Split views you never compare. Pin heads, share them with other
  agents, and check consistency between what each of you saw.
