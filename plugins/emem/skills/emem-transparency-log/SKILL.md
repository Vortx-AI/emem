---
name: emem-transparency-log
description: Audits emem's append-only transparency log, an RFC 6962 tree built with BLAKE3. Gets a signed tree head, and a proof that a log entry or a memory note is committed under it. Gets a proof that a later head extends an earlier one without rewriting history, and reads which independent witnesses countersigned. States the hash and signature rules to check each proof offline in the user's own code, because a proof that the responder checks for itself is not an audit. Use when the question is "could this responder have shown someone else a different history", not "is this fact signed". Also use when you pin evidence for an audit, or before you rely on a note whose deletion or rewrite would matter. This plugin ships no code.
---

# emem-transparency-log

| | |
|---|---|
| Use it when | You must show that the responder did not rewrite its history, or show two parties two histories. |
| You need | No key. To check a proof yourself: BLAKE3 and Ed25519 code that you already trust. These checks need no network. |
| You get | A signed tree head, an inclusion proof and a consistency proof as JSON, the witness counts, and the rules to check each proof offline. |
| Calls | `GET /v1/log/*` (or the `emem_log_*` MCP tools). The checks run in your own code. |

> **Network use.** The commands in this skill send JSON to `https://emem.dev` and receive JSON from it. Where a step says so, they also receive an image or a raster file. `https://emem.dev` is the service that the MCP server of the plugin connects to. The commands do not execute what they download.

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
| `GET /v1/log/inclusion?leaf_index=<i>` | `emem_log_inclusion` | The audit path from one leaf to a root. |
| `GET /v1/log/consistency?first=<n>&second=<m>` | `emem_log_consistency` | The proof that the tree of size n is a prefix of the tree of size m. |
| `GET /v1/log/witnesses?limit=<k>` | `emem_log_witnesses` | Co-signatures over `(tree_size, root)` from other keys. |
| `GET /v1/log/entries?start=<i>&end=<j>` | none | The raw entries, at most 256 for each call. |
| `GET /v1/log/test_vectors` | none | Known answers for every leaf of trees of size 1 to 33, to test your own checker. |

The inclusion call also accepts `entry_hash=<base32>` or
`fact_cid=<cid>` in place of `leaf_index`. Add `tree_size=<n>` to prove
against an earlier head.

The MCP tools in the table are not in the short tool list of the MCP
server. The server runs them by name through `tools/call`. If your host
calls only listed tools, or has no MCP, use the `GET` routes.

## What an answer from the server proves

The responder builds every proof that it serves, and it signs only the
head. If you read its answer and check nothing, you trust the
responder. That is not an audit. The audit is the check in
[Check it yourself](#check-it-yourself).

Two comparisons need no hash code:

- In a consistency answer, `first_root_b32` must equal the `root_b32` of
  the head that you pinned. `second_root_b32` must equal the root of the
  new head.
- In an inclusion answer with `tree_size` set to your pinned size,
  `root_b32` must equal the root of your pinned head.

A mismatch is evidence: the responder's own answer disagrees with a
head that it signed. These comparisons catch a log that was rebuilt or
lost. They do not catch a responder that lies on purpose, because one
party writes both sides.

Keep the JSON files in all cases. The head is signed. You, or anyone
you give the files to, can check them later.

## Step 1: pin a head and get a proof for one leaf

Get the current head. Send a GET to `https://emem.dev/v1/log/sth` and
save the answer as `old_sth.json`. With MCP, call `emem_log_sth` with
`{}` and save the result.

Get an inclusion proof for one leaf under that head. Put the
`sth.tree_size` of `old_sth.json` in the query. Send a GET to
`https://emem.dev/v1/log/inclusion?leaf_index=12345&tree_size=3319412`
and save the answer as `inc.json`. With MCP, call `emem_log_inclusion`
with `{"leaf_index": 12345, "tree_size": 3319412}`.

Read `leaf_index`, `tree_size`, `root_b32`, `entry_hash_b32`,
`leaf_hash_b32` and `audit_path_b32` in the answer. The `sth` in the
answer is the newest head, not yours. The log grows every few seconds.
When the log has grown, `root_is` says that `root_b32` is the unsigned
root at your size. The walk must then reach the root of your pinned
head. Without `tree_size`, the proof is for the newest head.

On 12 November 2026 at 19:44 UTC the head had `tree_size` 3319412,
`signed_at` `2026-11-12T19:44:29Z` and `responder_pubkey_b32`
`777er3yihgifqmv5hmc2wwmyszgddzderzhsx6rex4yoakwomvka`. Its signature
verified. At 19:48 UTC the log had grown to 3319442. The proof for leaf
12345 at `tree_size=3319412` had 22 hashes, and the walk reached the
pinned root. With one character changed in one path hash, the walk gave
a different root.

## Step 2: prove that the log only grew

Later, get a new head and read both tree sizes. Send a GET to
`https://emem.dev/v1/log/sth` again and save the answer as
`new_sth.json`. Then:

```sh
jq .sth.tree_size old_sth.json new_sth.json
```

Ask for the consistency proof from the old size to the new size. Put
your two sizes in the query. Send a GET to
`https://emem.dev/v1/log/consistency?first=3319412&second=3319442` and
save the answer as `cons.json`. With MCP, call `emem_log_consistency`
with `{"first": 3319412, "second": 3319442}`.

The answer holds `first_size`, `second_size`, `first_root_b32`,
`second_root_b32` and `consistency_proof_b32`. Check the proof against
the roots of the two heads that you hold, not against the roots in the
answer.

On 12 November 2026 the proof from 3319412 to 3319442 had 14 hashes.
`first_root_b32` was the pinned root, and the proof verified against
both signed heads.

Always send `second`. Without it, the server uses the current size. The
log grows every few seconds, so that size can differ from the head you
saved. Then the proof does not match your new head.

If the two heads have the same `tree_size`, there is nothing to prove
yet. Get the new head again a little later.

## Step 3: prove that a memory note is in the log

A memory write is also a log entry. The JSON view of a note gives the
inclusion path in its `log` field. Send a GET to
`https://emem.dev/memories/by_attester/k572x7go/readme/tree-20261106.md`
with the header `accept: application/json`. Read `file_cid` and `log`
in the answer.

On 7 November 2026 `log.inclusion` was
`/v1/log/inclusion?entry_hash=vxtja5r45kg6uiu6kjmstondfq6fxhvnwyojsqaogehtavzqwqwq`.
A note with `"log": null` has no log entry to prove.

You can also send the BLAKE3 of the note content as `entry_hash`. The
value must be base32 without padding, in lower case. A hex value gets
`400 invalid_argument`. To convert `authorship.body_hash_hex`, write
the 32 bytes, not the hex text, as base32 (RFC 4648). Remove the `=`
padding and use lower case. The result has 52 characters. For this
note, `2d04e9a347fc7cf59ef91e99a13274a95296f0d84aac32e641db7a887fdf511b`
gives `fucoti2h7r6plhxzd2m2cmtuvfjjn4gyjkwdfzsb3n5iq767kenq`.

Then get the proof. Send a GET to
`https://emem.dev/v1/log/inclusion?entry_hash=fucoti2h7r6plhxzd2m2cmtuvfjjn4gyjkwdfzsb3n5iq767kenq`
and save the answer as `note_inc.json`. Then:

```sh
jq -c '{matched, memory_note_file_cid, leaf_index}' note_inc.json
```

On 7 November 2026 the response had `matched: "memory_note_content"`,
`memory_note_file_cid: "fucoti2h7r6plhxzd2m2cmtuve"` and
`leaf_index: 3235700`. On 12 November 2026 it had the same three values.
Check the path as in [Check it yourself](#check-it-yourself).

The proof covers the log entry. It does not cover the content itself.
To close the loop, get that entry from `/v1/log/entries` and decode its
CBOR. Its `content_blake3` must equal the hash that you sent. The
`verify` field of the response says the same. Add `summary=1` for a
light row with `path`, `verb` and `signed_at`, but without the bytes to
hash.

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
curl -sf 'https://emem.dev/v1/log/witnesses?limit=5'
```

Read `independent_operator_count`, `independent_operator_domains`,
`head_is_witnessed_by_independent_operator`,
`freshest_independent_operator_entries_behind` and
`witness_keys_by_tier` in the answer.

With MCP, call `emem_log_witnesses` with `{"limit": 5}`.

A witness attests only the prefix that it signed. The response gives
three levels of independence:

| Field | What it counts |
|---|---|
| `head_is_witnessed` | Any co-signature, including the node's own liveness canary. It is false when any entry came after the last co-signature. At 01:34 UTC on 7 November 2026 it was false: the freshest co-signature was 13 entries behind. |
| `head_is_independently_witnessed`, `independent_witness_count` | Keys other than the node's own. Keys are free, so many fresh keys can raise this count. |
| `head_is_witnessed_by_independent_operator`, `independent_operator_count` | Keys with fresh DNS or well-known evidence under a domain that is not the node's own. This count costs something to move. |

On 7 November 2026 at 02:33 UTC the log had 112 independent witness
keys. Only one independent operator domain had fresh evidence:
`eudr.dev`. Its freshest co-signature was 58 entries behind the head, so
`head_is_witnessed_by_independent_operator` was false. The value of
`head_is_witnessed` was also false.

To check a co-signature, verify it offline with the witness rule in
[Check it yourself](#check-it-yourself). Then check a consistency proof
from its `tree_size` to the head that you hold. Add
`at_or_after=<the tree_size you pinned>` to the witnesses call. The
`nearest_witnessed` field then gives the nearest co-signed head at or
after your size, with that consistency proof.

## Check it yourself

Do these checks in code that you trust, not in code that the responder
gives you. This plugin ships no code. The emem repository publishes a
reference offline verifier at
https://github.com/Vortx-AI/emem/tree/main/tools/verifiers/emem-transparency-log
for anyone who wants to read or run it themselves. The full rules, with
measured examples, are in
[docs/protocol.md §9.6](https://github.com/Vortx-AI/emem/blob/main/docs/protocol.md#96-transparency-log),
with the walks in
[§9.6.1](https://github.com/Vortx-AI/emem/blob/main/docs/protocol.md#961-check-an-inclusion-proof-by-hand)
and
[§9.6.1.1](https://github.com/Vortx-AI/emem/blob/main/docs/protocol.md#9611-check-a-consistency-proof-by-hand).

Every hash is BLAKE3 with a 32-byte output. Every `_b32` value is
base32 (RFC 4648), lower case, without `=` padding. To decode one,
change it to upper case and add `=` until its length is a multiple of 8.

The tree:

```text
entry_hash = blake3(entry bytes)          the decoded entry_cbor_b32 from /v1/log/entries
leaf       = blake3(0x00 || entry_hash)
node       = blake3(0x01 || left || right)
```

The tree splits n leaves at the largest power of two below n. A lone
node moves up one level unchanged. The tree does not duplicate it. This
is a different tree from the batch tree of a receipt's `merkle_proof`.

1. **Head signature.** Rebuild the 32-byte digest, then verify
   `signature_b32` (64 bytes) over it with Ed25519 and
   `responder_pubkey_b32`. Compare that key with
   `responder.pubkey_b32` at `https://emem.dev/.well-known/emem.json`.
   A key that only the answer names proves only that its holder signed.

   ```text
   digest = blake3("emem.preimage.v1" || 0x00
                   || u32_le(20) || "emem.translog.sth.v1"
                   || 0x01 || u32_le(8)  || u64_be(tree_size)
                   || 0x02 || u32_le(32) || root
                   || 0x03 || u32_le(len(signed_at)) || signed_at as UTF-8
                   || 0x04 || u32_le(32) || responder_pubkey)
   ```

2. **Leaf.** Compute `blake3(0x00 || entry_hash)` from `entry_hash_b32`.
   It must equal `leaf_hash_b32`. Do not take `leaf_hash_b32` on trust.
3. **Inclusion walk** (RFC 9162 §2.1.3.2). The result must equal the
   root of the signed head at `tree_size`.

   ```text
   fn = leaf_index;  sn = tree_size - 1;  r = leaf
   for each p in audit_path_b32:
       if sn == 0: fail
       if fn is odd or fn == sn:
           r = node(p, r)
           while fn is even and fn != 0: fn >>= 1; sn >>= 1
       else:
           r = node(r, p)
       fn >>= 1;  sn >>= 1
   accept if sn == 0 and r == root
   ```

4. **Consistency walk** (RFC 9162 §2.1.4.2). `m` and `n` are the sizes
   of your old and new heads. `old_root` is the root of the head that you
   pinned, never `first_root_b32` from the answer. If `m == n`, accept
   only an empty proof and equal roots.

   ```text
   proof = consistency_proof_b32
   if m is a power of two: put old_root at the start of proof
   fn = m - 1;  sn = n - 1
   while fn is odd: fn >>= 1; sn >>= 1
   fr = sr = proof[0]
   for each c in the rest of proof:
       if sn == 0: fail
       if fn is odd or fn == sn:
           fr = node(c, fr);  sr = node(c, sr)
           while fn is even and fn != 0: fn >>= 1; sn >>= 1
       else:
           sr = node(sr, c)
       fn >>= 1;  sn >>= 1
   accept if sn == 0 and fr == old_root and sr == new_root
   ```

5. **Witness co-signature.** Use the digest of step 1 with the domain
   `emem.translog.witness.v1` (24 bytes) and three segments: `0x01`
   `tree_size`, `0x02` `root`, `0x03` `witness_pubkey`. There is no
   time segment. Verify `signature_b32` with `witness_pubkey_b32`.
6. **Entry bytes.** Optional. Decode `entry_cbor_b32` from
   `/v1/log/entries`. Its BLAKE3 must equal `entry_hash_b32`.

Test your code before you trust it. Get the known answers:

```sh
curl -sf https://emem.dev/v1/log/test_vectors
```

In `log.trees`, each tree of size 1 to 33 has `entries_b32`,
`leaf_hashes_b32`, `root_b32`, an `inclusion` list with the
`audit_path_b32` of every leaf, and `consistency_to_this_size` (sizes up
to 16) with `first_size` and `path_b32`. Check every leaf of every
size. A walk that is wrong for a leaf in a short right subtree passes
most live samples and fails here. The `batch` part is for the receipt
tree, which has other rules. Then change one character of one hash and
make sure that your check fails. A check that cannot fail proves
nothing.

## What this does not prove

- That a fact is true. It proves that the history was not rewritten.
- That the `merkle_proof` in a receipt is this tree. That proof binds a
  fact into its signing batch. To check it, use
  [`emem-verify-receipt`](../emem-verify-receipt/SKILL.md).
- A split view that nobody compares. Pin heads, share them with other
  agents, and check consistency between the heads that each of you saw.
