---
name: emem-transparency-log
description: Audits emem's append-only RFC 6962 transparency log offline. Verifies a signed tree head, proves a log entry or a memory note is committed under it, proves a later head extends an earlier one without rewriting history, and reads which independent witnesses countersigned. Use when the question is not "is this fact signed" but "could this responder have shown someone else a different history", when pinning evidence for an audit, or before relying on a note whose deletion or rewrite would matter. Ships a small Python verifier.
---

# emem-transparency-log

| | |
|---|---|
| Use it when | You must show that the responder did not rewrite its history, or show two parties two histories. |
| You need | Python 3.8 or later. No key. No network for the offline checks. |
| You get | `VALID` or `INVALID` for a signed tree head, an inclusion proof and a consistency proof, with an exit code. |
| Calls | `GET /v1/log/*` (or the `emem_log_*` MCP tools), then `scripts/verify_log.py` (offline). |

> **Network use.** The commands in this skill send JSON to `https://emem.dev` and receive JSON from it. Where a step says so, they also receive an image or a raster file. `https://emem.dev` is the service that the MCP server of the plugin connects to. The commands do not execute what they download. The files under `scripts/` read local files and make no network calls.

A receipt proves that the responder signed some fact cids. It does not
prove that the responder showed every reader the same history. The
transparency log does that. Each attestation and each memory write is
a leaf in one RFC 6962 Merkle tree. The responder signs the head of
that tree. Pin a head today. Tomorrow you can prove that the log only
grew.

## Which call to use

All calls are reads. They need no key.

| Call | MCP tool | What it returns |
|---|---|---|
| `GET /v1/log/sth` | `emem_log_sth` | The signed tree head: `tree_size`, `root_b32`, `signed_at` and the signature. |
| `GET /v1/log/inclusion?leaf_index=<i>` | `emem_log_inclusion` | The audit path from one leaf to the current root. |
| `GET /v1/log/consistency?first=<n>&second=<m>` | `emem_log_consistency` | The proof that the tree of size n is a prefix of the tree of size m. |
| `GET /v1/log/witnesses?limit=<k>` | `emem_log_witnesses` | Co-signatures over `(tree_size, root)` from other keys. |
| `GET /v1/log/entries?start=<i>&end=<j>` | none | The raw entries, at most 256 for each call. |

The inclusion call also accepts `entry_hash=<base32>` or
`fact_cid=<cid>` in place of `leaf_index`. Add `tree_size=<n>` to prove
against an earlier head.

The tree hashes with BLAKE3. A leaf is `blake3(0x00 || entry_hash)`. A
node is `blake3(0x01 || left || right)`. A lone node moves up one level.
The tree does not duplicate it.

The MCP tools in the table are not in the short tool list of the MCP
server. The server runs them by name through `tools/call`. If your host
calls only listed tools, or has no MCP, use the `GET` routes.

## Step 1: pin a head and prove one leaf

`scripts/verify_log.py` ships with this skill. Claude Code sets
`${CLAUDE_SKILL_DIR}`. The script makes no network calls. Its only
import outside the standard library is `scripts/emem_crypto.py`, which
also ships with this skill. That module is a plain-Python BLAKE3 and
Ed25519 verifier.

Check the module against the official test vectors first:

```sh
python3 "${CLAUDE_SKILL_DIR}/scripts/verify_log.py" --self-test
```

The last line is `SELF-TEST PASS`.

Get the current head and an inclusion proof for one leaf:

```sh
curl -s https://emem.dev/v1/log/sth -o old_sth.json
curl -s 'https://emem.dev/v1/log/inclusion?leaf_index=12345' -o inc.json
```

With MCP, call `emem_log_sth` with `{}`. Then call `emem_log_inclusion`
with `{"leaf_index": 12345}`. Save each result as a JSON file.

Check both files:

```sh
python3 "${CLAUDE_SKILL_DIR}/scripts/verify_log.py" sth old_sth.json
python3 "${CLAUDE_SKILL_DIR}/scripts/verify_log.py" inclusion inc.json
```

The `inclusion` check also verifies the head that the response carries.
On 7 November 2026 at 00:44 UTC these commands printed:

```
sth          VALID    tree_size=3243860 signed_at=2026-11-07T00:44:43Z signer=777er3yihgifqmv5hmc2wwmyszgddzderzhsx6rex4yoakwomvka
inclusion    VALID    leaf_index=12345 path=22 hashes
```

## Step 2: prove that the log only grew

Later, get a new head and read both tree sizes:

```sh
curl -s https://emem.dev/v1/log/sth -o new_sth.json
jq .sth.tree_size old_sth.json new_sth.json
```

Ask for the consistency proof from the old size to the new size. Put
your two sizes in the query:

```sh
curl -s 'https://emem.dev/v1/log/consistency?first=3243860&second=3243861' -o cons.json
python3 "${CLAUDE_SKILL_DIR}/scripts/verify_log.py" consistency old_sth.json new_sth.json cons.json
```

With MCP, call `emem_log_consistency` with
`{"first": 3243860, "second": 3243861}`.

On 7 November 2026 at 00:45 UTC the check printed:

```
sth          VALID    tree_size=3243860 signed_at=2026-11-07T00:44:43Z signer=777er3yihgifqmv5hmc2wwmyszgddzderzhsx6rex4yoakwomvka
sth          VALID    tree_size=3243861 signed_at=2026-11-07T00:45:03Z signer=777er3yihgifqmv5hmc2wwmyszgddzderzhsx6rex4yoakwomvka
consistency  VALID    3243860 -> 3243861, 14 hashes
```

Always send `second`. Without it, the server uses the current size. The
log grows every few seconds, so that size can differ from the head you
saved. The script refuses sizes that do not match the two heads.

If the two heads have the same `tree_size`, there is nothing to prove
yet. Get the new head again a little later.

## Exit codes and a tamper check

| Exit code | Meaning |
|---|---|
| `0` | Every check passed. |
| `1` | One check failed. The line says `INVALID`. |
| `2` | Bad input, for example a file that holds an error body and not a log object. |

To see a failure, change one value in a copy and check the copy:

```sh
jq '.leaf_index=12346' inc.json > bad_inc.json
python3 "${CLAUDE_SKILL_DIR}/scripts/verify_log.py" inclusion bad_inc.json
```

On 7 November 2026 this printed `inclusion INVALID leaf_index=12346`
and exited with 1. A changed `tree_size` in a head also gave `INVALID`.

## Step 3: prove that a memory note is in the log

A memory write is also a log entry. The JSON view of a note gives the
inclusion path in its `log` field:

```sh
curl -s https://emem.dev/memories/by_attester/k572x7go/readme/tree-20261106.md \
  -H 'accept: application/json' | jq -c '{file_cid, log}'
```

On 7 November 2026 `log.inclusion` was
`/v1/log/inclusion?entry_hash=vxtja5r45kg6uiu6kjmstondfq6fxhvnwyojsqaogehtavzqwqwq`.
A note with `"log": null` has no log entry to prove.

You can also send the BLAKE3 of the note content as `entry_hash`. The
value must be base32 without padding, in lower case. A hex value gets
`400 invalid_argument`. Convert `authorship.body_hash_hex` like this:

```sh
python3 -c "import base64,sys; print(base64.b32encode(bytes.fromhex(sys.argv[1])).decode().rstrip('=').lower())" \
  2d04e9a347fc7cf59ef91e99a13274a95296f0d84aac32e641db7a887fdf511b
```

Then get and check the proof:

```sh
curl -s 'https://emem.dev/v1/log/inclusion?entry_hash=fucoti2h7r6plhxzd2m2cmtuvfjjn4gyjkwdfzsb3n5iq767kenq' -o note_inc.json
jq -c '{matched, memory_note_file_cid, leaf_index}' note_inc.json
python3 "${CLAUDE_SKILL_DIR}/scripts/verify_log.py" inclusion note_inc.json
```

On 7 November 2026 the response had `matched: "memory_note_content"`,
`memory_note_file_cid: "fucoti2h7r6plhxzd2m2cmtuve"` and
`leaf_index: 3235700`. The path verified.

The proof covers the log entry. It does not cover the content itself.
To close the loop, get that entry from `/v1/log/entries` and decode its
CBOR. Its `content_blake3` must equal the hash that you sent. The
`verify` field of the response says the same.

To prove the entry that carried a fact, send `fact_cid=<cid>`. On
7 November 2026,
`fact_cid=nflpddk7zsncywguwjzk5koksseqfyx4jnngkuryrnd4aykqlpfq` matched
at `leaf_index` 2457934 and its path verified.

| Response | Cause |
|---|---|
| `400 invalid_argument`, `entry_hash must be base32-nopad` | You sent hex or padded base32. |
| `404 cid_not_found`, `no log entry with entry_hash=...` | No entry and no logged note has that hash. |
| `404 cid_not_found`, `fact ... has no log entry here` | This responder did not sign the fact, or the responder has not indexed its log link yet. |

## Step 4: read the witnesses

```sh
curl -s 'https://emem.dev/v1/log/witnesses?limit=5' | jq -c '{independent_operator_count,
  independent_operator_domains, head_is_witnessed_by_independent_operator,
  freshest_independent_operator_entries_behind, witness_keys_by_tier}'
```

With MCP, call `emem_log_witnesses` with `{"limit": 5}`.

A witness attests only the prefix that it signed. The response gives
three levels of independence:

| Field | What it counts |
|---|---|
| `head_is_witnessed` | Any co-signature, including the node's own liveness canary. It is false when any entry came after the last co-signature. At 01:34 UTC on 7 November 2026 it was false: the freshest co-signature was 13 entries behind. |
| `head_is_independently_witnessed`, `independent_witness_count` | Keys other than the node's own. Keys are free, so many fresh keys can raise this count. |
| `head_is_witnessed_by_independent_operator`, `independent_operator_count` | Keys with fresh DNS or well-known evidence under a domain that is not the node's own. This count costs something to move. |

On 7 November 2026 at 00:45 UTC the log had 112 independent witness
keys. Only one independent operator domain had fresh evidence:
`eudr.dev`. Its freshest co-signature was 25 entries behind the head, so
`head_is_witnessed_by_independent_operator` was false.

To check a co-signature, verify it offline. Then run a consistency proof
from its `tree_size` to the head that you hold. Add
`at_or_after=<the tree_size you pinned>` to the witnesses call. The
`nearest_witnessed` field then gives the nearest co-signed head at or
after your size, with that consistency proof.

## What this does not prove

- That a fact is true. It proves that the history was not rewritten.
- That the `merkle_proof` in a receipt is this tree. That proof binds a
  fact into its signing batch. To check it, use
  [`emem-verify-receipt`](../emem-verify-receipt/SKILL.md).
- A split view that nobody compares. Pin heads, share them with other
  agents, and check consistency between the heads that each of you saw.
