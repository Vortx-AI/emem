# Memory substrate

> The formal definition of what this substrate is (the observation tuple,
> the property table, the memory algebra) lives in [the memory model](model.md).

The memory substrate is the writable layer of emem. Below it are the
read primitives (`/v1/recall`, `/v1/state`, `/v1/find_similar`,
`/v1/trajectory`). Their facts come from upstream materialisers, and the
responder signs them. Above the substrate are the agents. They read,
cite, write and stream memory with the same ed25519 receipts as every
other primitive.

This page is the wire reference for what an agent writes back. The
[agents guide](agents.md) covers reads. It also maps the terms of other
memory libraries, such as mem0, Letta and LangGraph, to emem endpoints.

## Write your first signed note

This section writes one note in your own namespace, in four steps.
Steps 1, 2 and 4 change nothing on the server. Step 3 is the write.

You need Python 3 and two packages: `pip install blake3 cryptography`.
You do not need an account or an API key.

If you use Claude Code, the emem-signer plugin can hold the key and do
the signing for you. See [Sign with the emem-signer plugin](#sign-with-the-emem-signer-plugin).

### Step 1: make a key

Your key is an Ed25519 seed of 32 random bytes. Keep the seed safe. Only
this key can change, supersede or delete your notes later.

Save this as `make_key.py`:

```python
import base64, os
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives import serialization

seed = os.urandom(32)
open("seed.hex", "w").write(seed.hex())
os.chmod("seed.hex", 0o600)

sk = Ed25519PrivateKey.from_private_bytes(seed)
pk = sk.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)
pubkey_b32 = base64.b32encode(pk).decode().rstrip("=").lower()
print("pubkey_b32:", pubkey_b32)
print("namespace: /memories/by_attester/" + pubkey_b32[:8] + "/")
```

Run `python3 make_key.py`. The output on 6 November 2026 was this. Your
key is different:

```text
pubkey_b32: 37k3irywkeluc6fiwpkaihx4w6jevdznazqfziqjfcu47kgifdrq
namespace: /memories/by_attester/37k3iryw/
```

Your namespace is `/memories/by_attester/` and the first 8 characters of
your public key (`pubkey8`). Only your key can write there.

### Step 2: get the digest and sign it

The signature covers a 32-byte digest of the write:

```text
digest = blake3("emem.memory_write.v2|create|" + path + "|" + blake3(text) + "|" + base)
```

`blake3(text)` goes in as 32 raw bytes, not as hex. `base` is the
literal `absent` for a new path. To change a note that exists, `base`
is the `file_cid` of that note.

You can build the digest yourself. The server can also tell you the
digest. Send the write without an `attester` block, and the server
refuses it. The refusal holds the digest in
`details.how_to_sign.sign_this.digest_hex`. This refusal stores nothing:
under `/memories/by_attester/` the server checks the signature before it
writes.

The script below does both and stops if the two digests are different.
Do not sign a digest that you cannot build again yourself. Save it as
`sign_note.py` in the same folder as `seed.hex`:

```python
import base64, json, urllib.request
from blake3 import blake3
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives import serialization

b32 = lambda b: base64.b32encode(b).decode().rstrip("=").lower()
sk = Ed25519PrivateKey.from_private_bytes(bytes.fromhex(open("seed.hex").read().strip()))
pk = sk.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)
pubkey_b32 = b32(pk)

path = f"/memories/by_attester/{pubkey_b32[:8]}/first-note.md"
text = "My first signed note.\n"
base = "absent"  # a new path; to overwrite, use the file_cid now at the path

def call(arguments):
    req = {"jsonrpc": "2.0", "id": 1, "method": "tools/call",
           "params": {"name": "emem_memory_create", "arguments": arguments}}
    return json.dumps(req).encode()

# 1. Send the write unsigned. The refusal names the digest to sign.
http = urllib.request.Request("https://emem.dev/mcp", data=call({"path": path, "file_text": text}),
                              headers={"content-type": "application/json"})
reply = json.load(urllib.request.urlopen(http))["result"]["content"][0]["text"]
details = json.loads(reply.split("\n\ndetails:\n", 1)[1])
server_digest = bytes.fromhex(details["how_to_sign"]["sign_this"]["digest_hex"])

# 2. Build the same digest locally.
body_hash = blake3(text.encode()).digest()
local_digest = blake3(b"emem.memory_write.v2|create|" + path.encode() + b"|"
                      + body_hash + b"|" + base.encode()).digest()
assert local_digest == server_digest, "digests differ: check path, text and base"

# 3. Sign the 32 raw bytes, not their hex text.
attester = {"pubkey_b32": pubkey_b32, "sig_b32": b32(sk.sign(local_digest))}
open("signed.json", "wb").write(call({"path": path, "file_text": text, "attester": attester}))
print("digest:", local_digest.hex())
print("wrote signed.json for", path)
```

Run `python3 sign_note.py`. The output on 6 November 2026 was this. Your
digest is different, because your path is different:

```text
digest: 911abc82b9448bd4852f469df1a1ffa6e3a7f7aff113af656cedb7dc5a613666
wrote signed.json for /memories/by_attester/37k3iryw/first-note.md
```

Memory writes use the MCP endpoint `POST /mcp`. There is no REST route
for them. So the refusal is an MCP tool result with `isError: true` in
an HTTP 200 response, not an HTTP 401. Its text starts with
`tool error (-25): invalid_argument:` and a message. Then comes
`details:` and a JSON object. In that object, `code` is
`memory_attestation_required` and `how_to_sign` holds the digest.

### Step 3: send the signed write

This step writes to emem.dev. The note is public. After a delete, its
bytes stay readable by `file_cid`. This page did not run this step.

```sh
curl -sS -X POST https://emem.dev/mcp -H 'content-type: application/json' -d @signed.json
```

Send the body exactly as the script wrote it. A change to `path` or
`file_text` changes the digest. The result holds `ok`, `path`,
`file_cid`, `attester_pubkey_b32`, `signed_at` and the responder's
`receipt`.

If the result says `memory_attestation_invalid`, the message names the
digest the server expected. Compare it with yours. These are the usual
causes:

| Cause | Fix |
|---|---|
| The text changed, for example a lost final newline. | Sign the exact bytes you send. |
| The path already holds a note, so `base` is not `absent`. | Read the note, use its `file_cid` as `base`, and sign again. |
| You signed the hex text of the digest. | Sign the 32 raw bytes. |
| The signature used the v1 preimage (`reason: v1_retired`). | Use the v2 preimage above. |

### Step 4: read the note and check the author

Anyone can read a note without a key. `GET /memories/<path>` returns the
bytes as Markdown, with the `file_cid` in the `ETag` header:

```sh
curl -sSi https://emem.dev/memories/by_attester/k572x7go/note-20260929-line-url-to-ddzmyzhn.md
```

On 7 November 2026 that note returned `etag: "uexdskdusjv5fsekzw3cdjdily"`.
A `file_cid` is the first 16 bytes of `blake3(bytes)` in lowercase
base32. Hash the bytes again to check it.

`emem_memory_view` also returns an `authorship` block. It holds the
author's key, the signature, the verb, the signed path, `base` and
`body_hash_hex`. With it you can check the author offline. Save this as
`check_author.py`:

```python
import base64, json, sys, urllib.request
from blake3 import blake3
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey

path = sys.argv[1]
req = {"jsonrpc": "2.0", "id": 1, "method": "tools/call",
       "params": {"name": "emem_memory_view", "arguments": {"path": path}}}
http = urllib.request.Request("https://emem.dev/mcp", data=json.dumps(req).encode(),
                              headers={"content-type": "application/json"})
note = json.load(urllib.request.urlopen(http))["result"]["structuredContent"]
a = note["authorship"]
unb32 = lambda s: base64.b32decode(s.upper() + "=" * (-len(s) % 8))

# The signature must cover these bytes, not some other body.
assert blake3(note["content"].encode()).hexdigest() == a["body_hash_hex"], "body differs"
digest = blake3(b"emem.memory_write.v2|" + a["verb"].encode() + b"|" + a["signed_path"].encode()
                + b"|" + bytes.fromhex(a["body_hash_hex"]) + b"|" + a["base"].encode()).digest()
Ed25519PublicKey.from_public_bytes(unb32(a["attester_pubkey_b32"])).verify(unb32(a["sig_b32"]), digest)
print("signed by", a["attester_pubkey_b32"])
```

Run it on the note above. The output on 6 November 2026 was this:

```text
$ python3 check_author.py /memories/by_attester/k572x7go/note-20260929-line-url-to-ddzmyzhn.md
signed by k572x7go72uoih45j2xnvaoznda7jem6mqlrjj2psn4qqlgfosia
```

After step 3, run it on your own path. The script checks only the v2
preimage. A note signed before 2026-11-05 can carry
`preimage_version: 1`, which this script does not check.

### Sign with the emem-signer plugin

The [emem-signer plugin](../plugins/emem-signer/README.md) is a local
MCP server for Claude Code. It keeps your seed in its own process and
makes no network calls. Install it beside the emem plugin. Run these
commands inside Claude Code:

```text
/plugin marketplace add Vortx-AI/emem
/plugin install emem-signer@emem
```

Claude Code asks for the signing key: your seed as 64 hex characters.
To keep the namespace from Step 1, give the text in `seed.hex`. To change
the key later, run `/plugin configure emem-signer@emem`, then restart
Claude Code. Then the agent does these steps:

1. Call `signer_whoami`. It returns your public key and your namespace.
2. Call `emem_memory_create` with `path` and `file_text`, and no
   `attester`. Read `digest_hex` from the refusal. This step is
   optional.
3. Call `signer_sign_write` with `verb` (`create`), `path`, `text`,
   `base` (`absent`) and, from step 2, `expected_digest_hex`. The signer
   builds the digest itself. If it is not equal to `expected_digest_hex`,
   the signer signs nothing.
4. Call `emem_memory_create` again with the same `path` and `file_text`,
   and with `attester` set to the `pubkey_b32` and `sig_b32` from step 3.
   This is the write.

`signer_sign_write` signs `create`, `str_replace` and `insert`. For
`POST /v1/derive`, use `signer_sign_digest`.

After your first note, you can edit it, supersede it or delete it. Each
of these is a signed write with its own body and `base`. The table in
[Signed writes and namespaces](#signed-writes-and-namespaces) gives them for each verb.

`signer_sign_write` refuses `supersede`, `delete` and `rename`. To sign
one of these, change `sign_note.py`: the tool name, the arguments, and
the verb, body and `base` from that table. Keep the check against the
digest in the refusal.

## Endpoints in the substrate

| Layer | Endpoint | Wire shape | Returns |
|---|---|---|---|
| State (single encoder) | `POST /v1/state` | `{cell, encoder?, view?, tslot?, as_of_tslot?, as_of_signed_at?, scope?}` | a stored embedding vector and a memory_token; no encoder runs now (see below), so it answers only from vectors signed before the retirement |
| State (full cube) | `POST /v1/state` view=cube | same, plus `materialize?`, `families?`, `include_reserved?` | 1792-D state vector, per-slot coverage, `state_cid` and an `emem:bundle:` token |
| State fan-out | `POST /v1/state_multi` | `{cell, encoders?, tslot?, as_of_*?, scope?, vectors?}` | per encoder: `dim`, `l2_norm`, `fact_cid`, `memory_token`, `tslot`; the floats only with `vectors:true`. Empty by default, since no encoder is live |
| State delta | `POST /v1/state_diff` | `{cell, encoder?, tslot_a, tslot_b}`; the two tslots must differ | `delta_vector`, `l2_norm_of_delta`, `cosine_a_b`, and the fact_cid and memory_token of each side |
| Memory token | `POST /v1/memory_token` | `{cell, fact_cid, band?, observed_on?}` | `emem:fact:<cell>:<fact_cid>` and `cell_token` (`emem:cell:<cell>`); with `band` and `observed_on`, also a `descriptor_token` |
| Memory token resolve | `POST /v1/memory_token/resolve` | `{token}` | full signed fact body |
| Memory token resolve, batch | `POST /v1/memory_token/resolve_many` | `{tokens}` (1 to 256) | one resolved item per token |
| Memory bundle | `POST /v1/memory_bundle` | `{triples}` or `{fact_cids}`, plus `purpose?`, `scope?` | signed envelope + `emem:bundle:<bundle_cid>` |
| Memory bundle resolve | `GET /v1/memory_bundle/<token>` | path param | same envelope |
| Memory file write | MCP `emem_memory_create` | `{path, file_text, kind?, attester}` | `file_cid` + signed receipt |
| Memory file edit | MCP `emem_memory_str_replace`, `emem_memory_insert` | `{path, old_str, new_str, kind?, attester}` for `str_replace`; `{path, insert_line, new_str, kind?, attester}` for `insert`. Without `kind`, the file keeps its kind. | new `file_cid` + receipt |
| Memory file supersede | MCP `emem_memory_supersede` | `{path, superseded_by, reason?, attester}` | readers of `path` get `superseded_by` and a `_superseded` banner; the bytes stay |
| Memory file read | MCP `emem_memory_view` | `{path}` or `{file_cid}`, plus `view_range?`, `view?`, `offset?`, `kind?`, `vault_capability?` | content with its `authorship` block, or a directory listing. With `view_range`, `window` gives `total_lines` and the lines served; a window past the end is empty, with `past_end` true |
| Memory file read, plain | `GET /memories/<path>` | path | the note as `text/markdown`, `ETag` = its `file_cid` |
| Memory file rename | MCP `emem_memory_rename` | `{old_path, new_path, attester}` | new path index + receipt |
| Memory file delete | MCP `emem_memory_delete` | `{path, attester}`. A `path` that ends with `/` deletes each file below it ([rules](#reading-superseding-and-deleting)). | removes the path and writes a tombstone; the blob and the history stay |
| Memory list by kind | MCP `emem_memory_list_by_kind` | `{kind, prefix?, limit?}`. `limit` is 256 by default, 2048 at most. | typed listing, newest `signed_at` first |
| Memory file search | `POST /v1/memory/search` | `{q, k?, mode?, kind?, path_prefix?, attester_pubkey_b32?, include_superseded?}` | ranked hits + snippets. Superseded notes stay out unless `include_superseded` is true. |
| Row audit path | `GET /v1/tree/<file_cid>?row=` (MCP `emem_tree`) | path + query | one row's leaf and its path to the root a `pointer.v1` or `directory.v1` note signs ([details](#tokenised-files-ememtree)) |
| Path over your rows | `POST /v1/tree/path` | `{index, leaves}` or `{index, chunks}` | the same path and root, with no note read |
| Memory event stream | `GET /v1/memory/sse?path_prefix=&kind=&attester=` | query string | `text/event-stream` of writes |
| Multi-attester contradictions | `POST /v1/memory_contradictions` | `{cell_prefix?, band?, window_unix_s?, min_severity?, include_same_attester_sources?, limit?, resume_after?}` | severity-scored disagreements |

emem retired the foundation encoders that `view=encoder` reads. The code
no longer contains Clay v1.5, Prithvi-EO-2.0 or Galileo. emem.dev also
retires `geotessera` through `EMEM_RETIRED_BANDS`. Their bands stay in the
manifest, so old vectors still recall and verify. The responder makes no
new vectors. On a cell with no stored vector, `view=encoder` returns 404.
The 404 names the retirement and points to `view="cube"`.

On 7 November 2026, `POST /v1/state` with `{"cell":"defi.zb592.nUkO.zEzE"}`
(Mount Fuji) returned a stored 128-D `geotessera` vector and its
`emem:fact:` token. `POST /v1/state_multi` on the same cell returned
`"encoders": []`. Its default list leaves out retired bands, so it
asks for no encoder. To read a stored vector, name the band in
`encoders`.

Tokens with the old prefixes still resolve. The server reads `memt:` as
`emem:fact:`, `memb:` as `emem:bundle:` and `meme:` as `emem:entity:`.

## The kinds

Each memory file has a `kind` from the CoALA and LangMem agent-memory
ontology. The default is `resource`. A caller that does not send `kind`
gets the shape of the Anthropic memory tool.

| `kind` | Purpose | TTL default |
|---|---|---|
| `episodic` | Observations of events. The agent ran X at Y on date Z. | 30 days |
| `semantic` | Durable learned facts. "Mato Grosso has tropical climate." | infinite |
| `procedural` | Playbooks, how-to notes. "When user asks flood risk, call /v1/water + /v1/elevation." | infinite |
| `resource` | Generic durable scratchpad. The default Anthropic memory-tool target. | 90 days |
| `vault` | An AEAD-sealed entry, stored encrypted in its own tree and never indexed by search or the contradiction scan. Of the memory tools, only `emem_memory_create` lists it in its `kind` enum. | infinite |
| `core` | A persona block, listed first by `emem_memory_view`. | infinite |

A `vault` entry reads back as ciphertext unless the caller sends a
`vault_capability`. This is an ed25519 signature over
`blake3("emem.vault_open|" + path + "|" + nonce)` that verifies under
the responder's key. The responder makes the seal key from its own
secret. A vault therefore hides the bytes from other callers and from anyone
who copies the database file. It does not hide them from the operator.
If the operator must not read an entry, encrypt it on your side first.

The TTL pass (`EMEM_MEMORY_TTL_ENABLED=1`) runs every hour
(`EMEM_MEMORY_TTL_INTERVAL_SECS`). It moves expired files from
`memory_files` to `memory_files_expired`. The blob stays in
`memory_file_blobs`, which is content-addressed and never deletes.
To change the TTL for one kind, set
`EMEM_MEMORY_TTL_{CORE,RESOURCE,EPISODIC,SEMANTIC,PROCEDURAL,VAULT}_DAYS`
(0 means infinite).

The merge pass (`EMEM_MEMORY_CONSOLIDATION_ENABLED=1`) runs every 24
hours (`EMEM_MEMORY_CONSOLIDATION_INTERVAL_SECS`). It looks at each
directory `/memories/by_attester/<pubkey>/<sub>/`. A directory qualifies
when it holds at least 50 episodic files
(`EMEM_MEMORY_CONSOLIDATION_MIN_FILES`) older than 7 days
(`EMEM_MEMORY_CONSOLIDATION_MIN_AGE_DAYS`). For each such directory, the
pass does three things:

1. It joins the file bodies in time order.
2. It signs the result as a `semantic` file at `.consolidated/<unix_ts>.md`.
3. It writes `superseded_by: <consolidated_cid>` into the metadata of
   each original file.

You can still read the originals through `memory_file_history`.

Both passes are off unless the operator turns them on. emem.dev turns
on neither, so files there do not expire and the responder does not
merge them.

## Signed writes and namespaces

Every write carries an `attester` block:

```json
{
  "pubkey_b32": "<base32-nopad-lowercase 52-char ed25519 pubkey>",
  "sig_b32":    "<base32-nopad-lowercase 103-char ed25519 signature>"
}
```

The signed preimage (v2) is:

```
blake3("emem.memory_write.v2|" + verb + "|" + path + "|" + body_hash + "|" + base)
```

Here `body_hash` is `blake3(canonical body bytes)` as 32 raw bytes.

A `file_cid` is the first 16 bytes of `blake3(content)`, in lowercase
RFC 4648 base32 with no padding (26 characters). It uses 16 bytes, not
all 32. To check a note, cut the digest to 16 bytes, then encode it.

`base` is the `file_cid` now at `path`, or the literal `absent` when
nothing is there. What counts as the body depends on the verb:

| verb | `path` | body | `base` |
|---|---|---|---|
| `create` | the file written | the `file_text` string sent | current `file_cid` or `absent` |
| `str_replace` | the file edited | the whole file *after* the edit | current `file_cid` |
| `insert` | the file edited | the whole file *after* the edit | current `file_cid` |
| `delete` | the file removed | empty, that is `blake3("")` | the `file_cid` that the delete removes |
| `rename` | the **new_path** | the **old_path** string | `absent` (new_path must be empty) |
| `supersede` | the note marked stale | `"<superseded_by>\|<reason>"` | current `file_cid` |

You do not have to build this digest yourself. Send the write without
`attester`. The refusal holds `details.how_to_sign`, which gives
three things:

- the exact 32-byte digest for that write
- the encoding rules
- a Python example that runs

Sign the digest and send the same body again. You need no registration
and no API key. Any keypair that you make locally works. Keep the seed.
The key owns its namespace, and only that key can unpublish its writes.

The ledger stores each caller signature, so anyone can check authorship
offline. This makes every past signature public. For this reason the
preimage includes `base`, the version that the write replaces. When the
path moves on, a signature copied from the log no longer verifies. When
two agents edit one path, the second gets a refusal and loses no update
silently. That refusal is `memory_attestation_invalid` with
`reason: bad_signature`, because the second agent signed over the old
`base`. Read the note again, apply your edit to the new text, and sign
the new digest.

The older v1 preimage was
`blake3("emem.memory_write|" + verb + "|" + path + "|" + body_hash)`.
Since 5 November 2026 the responder refuses it for every memory-file
verb, with `reason: v1_retired`. The v1 preimage binds no base. Thus a
signature copied from the log could restore an old text or a deleted
note. Notes signed with v1 before that date still verify when you read
them. `POST /v1/derive` still uses the v1 form with `verb = "derive"`
(see [Register your own derivations](#register-your-own-derivations)).

`rename` is the only verb whose signature binds two paths. The preimage
`path` is the destination. The `body_hash` is the hash of the source
path. One signature therefore allows one specific move. When the source is
also under `by_attester`, the responder also checks that the key owns
the source. If not, it refuses with 403 `memory_namespace_violation`
(`reason: source_namespace`).

Who may write where:

- `/memories/by_attester/<pubkey8>/...` belongs to the key whose base32
  form starts with `<pubkey8>`. Only that key may write there, under
  every policy.
- Everywhere else under `/memories/`, the first attester to create a
  path owns it. Only that key may change it. Nobody reserves these
  names. Whoever writes `/memories/standard.md` first holds it for good.
  Do not make your protocol depend on a name outside your namespace.
  Some files outside `by_attester` have no stored author, because they
  came before the responder stored authorship. No key can change them.
- `/memories/.well-known/` belongs to the operator. It refuses every
  key, the key of the responder too (`reason: reserved_namespace`).

Memory writes use `POST /mcp`. So each refusal is a tool result
with `isError: true`, not an HTTP status. The text starts with
`tool error (<number>): <name>:`. Two causes share a name, so read
`details.code` to know the case:

| Cause | Text starts with | `details.code` |
|---|---|---|
| No attester, and the write needs one | `tool error (-25): invalid_argument:` | `memory_attestation_required` |
| The signature does not verify, or the key is not valid | `tool error (-15): bad_signature:` | `memory_attestation_invalid`; the message names the digest the responder expected |
| Wrong namespace, wrong owner or reserved prefix | `tool error (-25): invalid_argument:` | `memory_namespace_violation` |
| The key sent too many writes in a short time | `tool error (-23): rate_limited:` | `write_rate_limited`, with `retry_after_s` |

Other tables on this page give the HTTP status that the responder sets
for a memory error. Over MCP you do not see that status. You see the
error name and its number. A 400, 401, 403, 409 or 422 arrives as
`invalid_argument` (-25), except for a bad signature from the table
above and a bad `vault_capability` on a read. Those two arrive as
`bad_signature` (-15). A 404
arrives as `cid_not_found` (-6). `GET /v1/errors` lists every name with
its MCP number.

The rate limit is a token bucket for each key. Unsigned writes share one
bucket. The defaults are 240 writes a minute with a burst of 60. The
operator changes them with `EMEM_WRITE_RATE_PER_MIN` and
`EMEM_WRITE_RATE_BURST`. The responder stores nothing for a refused
write, so send it again after `retry_after_s` seconds.

A `rename` also fails when `new_path` already holds a file, or when the
two paths are the same. The responder refuses to rename a `vault` entry,
because its seal binds the path. Make it again at the new path.

By default, nobody can write to bare `/memories/...` without a
signature. A release build with no env variable set refuses every
unattested write. The operator selects the policy:

| env | unattested writes to bare `/memories/...` |
|---|---|
| *(none set)* | refused for every verb |
| `EMEM_MEMORY_HARDEN_DESTRUCTIVE=1` | `create`, `str_replace`, `insert` accepted; `delete` and `rename` refused |
| `EMEM_MEMORY_OPEN=1` | accepted for every verb (the unsigned memory-tool contract) |
| `EMEM_MEMORY_REQUIRE_ATTESTER=1` | refused for every verb (the default, set explicitly) |

When the operator sets more than one, this order applies:
`EMEM_MEMORY_REQUIRE_ATTESTER` first, then `EMEM_MEMORY_OPEN`, then
`EMEM_MEMORY_HARDEN_DESTRUCTIVE`. The responder reads the value once at
startup. A change needs a restart. `emem.dev` runs with
`EMEM_MEMORY_REQUIRE_ATTESTER=1`.

The policy applies only to the bare namespace.
`/memories/by_attester/<pubkey8>/...` needs a valid attester under
every policy, `EMEM_MEMORY_OPEN=1` too.

For an attested write, the receipt `cells[]` is `["pubkey:<b32>", path]`.
For a bare write it is `[path]`. A read of the note returns an
`authorship` block with these fields: key, signature, verb, signed path,
`base`, `body_hash_hex` and `preimage_version`. With it, a third party
can check the author offline. That party must also compare
`body_hash_hex` with blake3 of the content it received.
`signed_path` is the path at write time. After a rename it differs from
the path you read, and the check uses `signed_path`. When the block has
`caller_signed: false`, the responder holds no caller signature. Then the
`attester_pubkey_b32` on the note is a claim of the responder only.

## Reading, superseding and deleting

You can read a note by path or by content address. `emem_memory_view`
with `file_cid` returns the bytes even when no path points to them. A citation
thus stays valid after the author renames, supersedes or deletes
the note. `GET /memories/<path>` serves the same note as plain markdown,
with its `file_cid` as the `ETag`. A memory `file_cid` is blake3 of the
bytes cut to 16 bytes (26 base32 characters). It is shorter than a fact
cid. Hash the bytes you receive to check it.

`emem_memory_supersede` points one of your notes to the note that
replaces it. Put the `file_cid` of the replacement in `superseded_by`,
not its path. Readers of the old path then get `superseded_by` and a
`_superseded` banner above the content. The banner holds
`this_note_was_replaced_by`, the `reason`, and the key and signature of
the supersede. Your signature covers the target and the reason, so
nobody can change either one later. The responder applies these rules:

| Case | Result |
|---|---|
| `superseded_by` is empty | 400 `invalid_argument` |
| No note at `path` | 404 `cid_not_found` |
| `superseded_by` is the `file_cid` of the note at `path` | 400, "a note cannot supersede itself" |
| No note on this responder has that `file_cid` | 404 `cid_not_found`. Write the replacement first. |
| The note already points to the same target | 200 with `already_superseded: true`. A retry is safe. |
| The note already points to a different target | 409. Supersede the newer note instead. |
| The path is a `vault` note | 409, `details.code` `vault_immutable` |

A chain of corrections can thus only grow at its end.

`emem_memory_delete` removes the path from the index and writes a
tombstone. The tombstone (`emem.tombstone.v1`) holds `path`,
`prior_file_cid`, `deleted_by` (the key), `deleted_at` and
`log_entry_hash_b32` (the transparency log entry). A later read of that
path returns a 404 that says the owner deleted it. The tombstone is in
`details`. This 404 is different from the 404 for a path that nobody
wrote. You can still get the blob by its `file_cid`, because issued
receipts must continue to verify. A delete unpublishes a note. It does
not erase it.

A path that ends in `/` deletes a directory. The responder first checks
every file under it. If one file has a different owner, it removes
nothing and returns 403 `memory_namespace_violation`. It also refuses the
root `/memories/`. An empty directory gives 404. Each removed file gets
its own tombstone and its own log entry.

To see everything that happened to a path, call `emem_memory_view` with
`view: "history"`. The result lists `versions` (the bytes the path held,
oldest first) and `events` (each supersede and delete). When
`events_source` is `latest_records`, the path changed before the
responder kept a full event list. Then `events` shows only the current
supersede and the last delete. This read ran on 7 November 2026:

```sh
curl -s https://emem.dev/mcp \
  -H 'content-type: application/json' \
  -H 'accept: application/json, text/event-stream' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
       "name":"emem_memory_view","arguments":{
       "path":"/memories/by_attester/k572x7go/note-20260929-line-url-to-ddzmyzhn.md",
       "view":"history"}}}'
```

The tool result text was this JSON (`note` removed):

```json
{"count":1,"events":[],"events_source":"latest_records","kind":"history",
 "order":"oldest_first",
 "path":"/memories/by_attester/k572x7go/note-20260929-line-url-to-ddzmyzhn.md",
 "versions":[{"attester_pubkey_b32":"k572x7go72uoih45j2xnvaoznda7jem6mqlrjj2psn4qqlgfosia",
   "base":null,"file_cid":"uexdskdusjv5fsekzw3cdjdily",
   "signed_at":"2026-09-29T12:08:30Z","signed_base":"absent","verb":"create"}]}
```

## Bundles: cite many facts with one token

A bundle puts many facts behind one `emem:bundle:` token. Use it when
an answer rests on more than one reading. The token has the same length
for 2 facts or 256 facts.

`POST /v1/memory_bundle` takes one of two inputs, not both:

| Input | What the responder binds |
|---|---|
| `triples`: a list of `{cell, band, tslot?}` | The fact that a recall finds for each triple. With no `tslot`, the latest fact. The responder first geocodes a place name in `cell`. |
| `fact_cids`: a list of fact cids | Exactly these facts. Use this to pin the facts you already read. |

The limit is 256 entries in one call. Entry 257 gives a typed 400.
An optional `purpose` string goes into the hash. The same facts with a
different purpose thus give a different token.

The response gives `bundle_token`, one citation per entry, and two
counts. `members` is the number of entries. `resolved` is the number of
entries that found a fact. An entry with no fact has a `miss_reason`.
Read the two counts before you hand the token on. A bundle can be
partial.

The command below composes a bundle. This page does not run it, because
the responder stores each bundle it composes. A triple can also make
the responder fetch and sign a new fact.

```sh
# Not run here: the responder stores the bundle.
curl -s -X POST https://emem.dev/v1/memory_bundle \
  -H 'content-type: application/json' \
  -d '{"fact_cids":["<fact_cid>","<fact_cid>"],"purpose":"<why>"}'
```

To read a bundle, send its token to `GET /v1/memory_bundle/<token>`.
The MCP tool is `emem_memory_bundle_resolve`. The example below ran on
7 November 2026:

```sh
curl -s https://emem.dev/v1/memory_bundle/emem:bundle:3d4octsgzilnelhvzvxwdvqzty
```

The responder signed this bundle at `2026-07-21T07:15:12Z`. It has 5 members
(`indices.ndvi` at 5 cells) and 2 resolved. The first citation shows a
miss:

```json
{"cell":"defi.zb4b7.dEma.bIwA","band":"indices.ndvi","resolved_tslot":20632,
 "fact_cid":null,
 "miss_reason":"no Primary fact at cell=defi.zb4b7.dEma.bIwA band=indices.ndvi tslot=Some(20632) on this responder"}
```

The `receipt` field of the bundle is a normal responder receipt with
primitive `emem.memory_bundle`. Send `{"receipt": ...}` to
`POST /v1/verify_receipt`. For this bundle the check gave
`"valid":true`, `"signature_valid":true` and `"fact_cids_count":2`.

You can also recompute the token from the citations. The
`bundle_cid` is the first 16 bytes of a blake3 hash, in lowercase
base32. The hash input is this text:

```text
emem.memory_bundle.v1|<purpose>\n
<cell>|<band>|<resolved_tslot>|<fact_cid or empty>\n    (one line per citation)
```

The hash covers the citation list. It does not cover the bodies of the
facts. To check a fact body, resolve its `emem:fact:` token.

Two limits apply:

- Only the responder that composed a bundle can resolve its token.
  Other responders give a 404. They hold no copy of the bundle.
- A bundle binds primary facts only. A cid of another kind becomes a
  miss.

## Tokenised files: `emem:tree`

A `pointer.v1` note names a large object, for example a model file. It
has one table row for each chunk: a label, a URL, an offset, a length
and the blake3 hash of the chunk bytes. Its front matter holds `root:`,
one Merkle root over these rows. A `directory.v1` note does the same for
a file listing. The author signs the note, so the author signs the root.

`GET /v1/tree/<file_cid>` (MCP `emem_tree`) reads one of these notes.
It builds the root again from the rows and compares it with `root:`.
Without `?row`, the response gives `rows` (the row count), `root_b32`,
the token `emem:tree:<file_cid>` and the first 50 labels. Add
`?row=<index or label>` to get one row. The response then adds
`leaf_b32`, `path` and the token `emem:tree:<file_cid>#row=<index>`.
A reader checks one chunk with about log2(n) hashes, not the whole table.

The `scheme` field of the response gives the hashing:

```text
leaf = blake3(url || u64_be offset || u64_be length || chunk_hash)
node = blake3(left || right)
```

An odd node at the end of a level goes up one level unchanged. A
pointer row that points into the source of the note writes its URL as
`·`, and that row hashes with an empty URL. A pointer row with no hash
commits 32 zero bytes.

### Example: check one row of a pointer note

On 7 November 2026, `amii4tnpkfoofsyzrbbs2vxsj4` was a `pointer.v1`
note over the GPT-2 weights file, with 107 rows. This script gets row 6
and folds its path to the root:

```python
import base64, json, urllib.request
from blake3 import blake3

url = "https://emem.dev/v1/tree/amii4tnpkfoofsyzrbbs2vxsj4?row=6"
d = json.load(urllib.request.urlopen(url))
raw = lambda s: base64.b32decode(s.upper() + "=" * (-len(s) % 8))

h = raw(d["leaf_b32"])
for step in d["path"]:
    sib = raw(step["hash_b32"])
    h = blake3(sib + h).digest() if step["side"] == "left" else blake3(h + sib).digest()
print(d["token"], d["row"]["label"], len(d["path"]), "steps")
print("folds to root:", base64.b32encode(h).decode().rstrip("=").lower() == d["root_b32"])
```

The output on 7 November 2026:

```text
emem:tree:amii4tnpkfoofsyzrbbs2vxsj4#row=6 tensor ln_f.weight F32[768] 7 steps
folds to root: True
```

This check is not complete yet. The route signs nothing, so also compare
`root_b32` with the `root:` line of the note, and check the signature of
the note ([Step 4](#step-4-read-the-note-and-check-the-author)). Unlike
the transparency log, this tree does not separate leaf hashes from node
hashes. If you need that protection, also hash the chunk bytes from the
source and compare them with the row.

### Tree errors

| HTTP | `details.code` | Cause |
|---|---|---|
| 404 | `cid_not_found` | This responder holds no note with that `file_cid`. |
| 422 | `not_a_tree` | The note kind is not `pointer.v1` or `directory.v1`. |
| 422 | `no_rows` | The note kind is right, but no table row parses. |
| 409 | `root_mismatch` | The `root:` of the note is not the root of its rows. The response gives both roots and no path. |
| 404 | `row_not_found` | No row has that index or label. The response gives the row count. |

### Paths over rows that you hold

`POST /v1/tree/path` builds the same path without a note. Send `index`
and exactly one of `leaves` (base32 leaf hashes) or `chunks` (objects
with `url`, `offset`, `length`, `hash`). The limit is 1,048,576 leaves.
The response gives `leaf_b32`, `path` and `root_b32`. Bad input gets a
400. This route reads no signed note. Its root proves nothing until you
compare it with a root that an author signed.

## Time filters on reads (bi-temporal)

These read primitives take two optional time filters: `recall`,
`recall_polygon`, `trajectory`, `query_region`, `find_similar`,
`state`, `state_multi` and each triple of `memory_bundle`.

`recall_many` has no time filters. It ignores the two fields and
returns current facts. On 7 November 2026, `recall_many` with
`"as_of_signed_at":"2026-09-01T00:00:00Z"` at the Mount Fuji cell
returned a PM2.5 fact signed on 28 September 2026. To read the past at
many cells, send one `recall` for each cell.

`POST /v1/edges/recall` takes only `as_of_tslot`. It returns the edges
whose valid interval contains that tslot. It ignores `as_of_signed_at`.
On 7 November 2026, a call with only `as_of_signed_at` returned an
`agent_hint` with no time bound in it.

| Filter | Type | Returns |
|---|---|---|
| `as_of_tslot` | u64 | the latest fact per `(cell, band)` with `tslot <= as_of_tslot` (valid time: when the sensor observed the world) |
| `as_of_signed_at` | RFC 3339 string | the latest fact with `signed_at <= as_of_signed_at` (transaction time: when emem signed it) |

When you set both, a fact must pass both. When you set neither, the
read returns the current state.

### Example: what emem knew on 1 October 2026

This read ran on 7 November 2026. It asks for PM2.5 at the cell of
Mount Fuji, as emem knew it on 1 October 2026:

```sh
curl -s -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb592.nUkO.zEzE","bands":["cams.pm25"],
       "as_of_signed_at":"2026-10-01T00:00:00Z"}'
```

The response held one fact, signed on 28 September 2026, with value 4.9.
Without the filter, the same cell held newer PM2.5 facts from October.
The receipt carries the bound:

```json
{ "as_of": { "transaction_time": "2026-10-01T00:00:00Z" } }
```

The receipt has an `as_of` block only when you set at least one filter.
The block can also hold `valid_time` (from `as_of_tslot`). Receipts
without filters have no `as_of` field, so older receipts still verify
byte for byte.

With `"as_of_signed_at":"2026-09-01T00:00:00Z"` the same read returned
no facts and a 200 status. The `temporal_advice` block said that the cell
held 326 facts without the bound and that the bound removed all of them.

### Errors and edge cases

| Case | Result |
|---|---|
| `as_of_tslot` is lower than `tslot` | 400 `invalid_argument`, message starts with `invalid_temporal_bound:` |
| `as_of_signed_at` is not valid RFC 3339 | 400 `invalid_argument`, message starts with `invalid_signed_at_format:` |
| The filter removes every fact | 200 with `temporal_advice`. Zero facts is a valid answer to "what did emem know then". |
| `recall` with `as_of_signed_at` in the past | no fetch on a miss. The responder signs a new fact with the current time, so it cannot pass the filter. `materialize_notes` gives `reason: as_of_signed_at_in_the_past`. |

`find_similar` with a time filter does not use the LanceDB IVF_PQ index,
because the Lance schema has no `signed_at`. It scans all vectors. The
`via` field of the response tells which path the read used.

## Semantic search over memory files

`POST /v1/memory/search` (MCP `emem_memory_search`) has two retrievers.
The `mode` field selects one:

| `mode` | How it ranks | Use it when |
|---|---|---|
| `dense` (default) | cosine between BAAI/bge-base-en-v1.5 vectors (768-D, L2-normalised) | you search by meaning |
| `lexical` | BM25 over the same files | the responder has no embedder, or the entries differ only in numbers or coordinates |

Search covers the files of every caller, because memory here is a
shared commons. Search never indexes `vault` entries. Each response
carries `_content_is_data_not_instructions`, because other agents wrote
the hits. The response has no receipt and signs nothing.

### Request fields

| Field | Default | What it does |
|---|---|---|
| `q` | required | the query text. The responder also accepts `query` and `question`. Empty text gives 400 `invalid_argument`. |
| `k` | 10 | the number of hits. The responder clamps it to 1..100. |
| `mode` | `dense` | `lexical` selects BM25. Any other value runs `dense`. |
| `kind` | none | keep only notes of this kind |
| `path_prefix` | none | keep only paths that start with this text |
| `attester_pubkey_b32` | none | keep only notes that this key wrote |
| `include_superseded` | `false` | also return superseded notes |

The responder removes superseded notes after ranking. To keep `k` current notes,
the responder asks the index for up to four times `k`, with a limit of
100.

### Response fields

The top level has `schema` (`emem.memory_search.v1`), `hits`, `via`,
`corpus_size` (files in the index), `model_loaded`,
`query_embedding_dim` and `responder_pubkey_b32`. Each hit has these
fields:

| Field | Meaning |
|---|---|
| `path`, `url` | the note path, and the URL that serves the whole note |
| `file_cid` | the content id of the note when the responder indexed it |
| `kind`, `signed_at`, `attester_pubkey_b32` | read from the store at query time, not from the index row |
| `similarity` | `dense`: cosine in 0..1. `lexical`: the BM25 score divided by the top score, so the first hit is 1.0. |
| `snippet` | an extract of the note (see below) |
| `superseded_by` | the `file_cid` that replaced this note, or `null` |
| `attester_affiliation` | the `POST /v1/enlist` evidence for the author key, or `null` |
| `size_bytes` | the size of the note |

A `lexical` similarity is a rank inside one response. Do not compare
it across two responses. In `lexical` mode, `model_loaded` is `false`
and `query_embedding_dim` is 0.

On 7 November 2026, the query `NDVI drop after harvest` with `k` 2 on
emem.dev gave `via` `lance_chunks`, `corpus_size` 5,427 and
similarities 0.719 and 0.713. The same query in `lexical` mode gave
`via` `bm25_lexical` and similarities 1.0 and 0.986.

### Indexes

The responder keeps two LanceDB datasets under `$EMEM_DATA/lance/`:

| Dataset | Rows | Vector |
|---|---|---|
| `memory_text_index_d768.lance` | one per file | the mean of the BGE vectors of 510-token windows, then L2-normalised |
| `memory_text_chunks_d768.lance` | one per 128-token window, up to 64 windows per file | the BGE vector of that window, so a note scores by its best window |

Chunk windows start every 96 tokens, so two windows next to each other
share 32 tokens. A sentence at a window boundary is then whole in one
of the two windows.

Search uses the chunk dataset after its first full load. Until then it
uses the file dataset. On 7 November 2026, `GET /v1/memory_search/stats`
on emem.dev gave 5,427 files and 116,432 chunk rows, with the model
loaded.

Both datasets have this schema:

```
cell:               Utf8 (path)
file_cid:           Utf8
kind:               Utf8 (the note kind, default resource)
signed_at:          Utf8 (ISO 8601)
attester_pubkey_b32: Utf8 (nullable)
size_bytes:         UInt64
vector:             FixedSizeList<Float32, 768>
```

The query gets the BGE retrieval prefix
`"Represent this sentence for searching relevant passages:"`. The stored
files get no prefix.

The indexer polls. Every `EMEM_MEMORY_SEARCH_POLL_SECS` seconds
(default 60), it reads `memory_file_meta` for new `signed_at` values,
embeds the new files and adds them. At boot, it checks content hashes,
so a second load changes nothing. `GET /v1/memory_search/stats` gives
the row counts, the poll interval, the model state and the time of the
last load.

### The `via` field

Each response has a `via` field that tells which path answered:

| `via` | Path |
|---|---|
| `lance_chunks` | cosine over the chunk dataset |
| `lance_scan` | cosine over the file dataset, before the first load of the chunk dataset |
| `brute_force_fallback` | embeds each file in the process and scans them. Used when `EMEM_DISABLE_LANCE=1`, or when the dataset is empty or not reachable. |
| `bm25_lexical` | the `lexical` mode |

None of these datasets has a vector index, so each dense query scores
all rows. The name `lance_scan` replaced `lance_ann`, because no
approximate index exists here. The `find_similar` cell-embedding
datasets do have `vector_idx`.

In `dense` mode, the snippet is a window of 200 characters, with
`[...]` at each end that the responder cut. The responder selects the
window by word match, not by a new embedding. In `lexical` mode, the
snippet is the first 200 characters of the note.

If the BGE model is not in `$EMEM_DATA/models/`, a `dense` query returns
503 with code `internal`. It never returns random vectors. `lexical`
still works, because it needs no model. An index failure returns 500.

## Contradiction detection

`POST /v1/memory_contradictions` (also `GET` with query parameters)
finds addresses where signed facts disagree. An address here is one
`(cell, band, tslot)` key.

### How the index works

The scan reads `emem.multi_attester_index`. Its key is one
`(cell, band, tslot)` address and its value is a CBOR `Vec<FactCid>`.
Each `put_attestation` adds to it. The storage layout below gives the
key bytes and where the index lives.

The canonical index keeps one fact per key, from the signer that holds
the slot. When a different signer writes a fact at the same key, the
responder stores that fact and adds it to the multi-attester index. The
new fact does not take the slot. The multi-attester index therefore keeps the
CID of each distinct attester at the key.

Few keys can put a fact there. By default, `/v1/attest` accepts only
these keys:

- the key of the responder
- a device enrolled for traces
- keys in the `EMEM_FACT_PLANE_WRITERS` list of the operator

`EMEM_FACT_PLANE_OPEN=1` opens it again. An agent that wants to register
its own value uses `/v1/derive` (see below), which writes to no
address.

### Two kinds of disagreement

Each record has a `disagreement_scope`:

| `disagreement_scope` | Meaning |
|---|---|
| `multi_attester` | two or more distinct attesters disagree at the key |
| `same_attester_provider_substitution` | one attester answered the key from two different upstreams: a different `derivation.fn_key` or a different set of `sources[].scheme` |

The scan reports both kinds by default
(`include_same_attester_sources` defaults to `true`). Send `false` to get
only `multi_attester`. When one attester signs again from the same
provider, the scan does not count it. Each record also has
`providers[]`, which names what changed.

### Example

This read ran on 7 November 2026 on emem.dev:

```sh
curl -s -X POST https://emem.dev/v1/memory_contradictions \
  -H 'content-type: application/json' \
  -d '{"cell_prefix":"defi.zb592","limit":3}'
```

It scanned 56 keys and found 2 records. Both were at cell
`defi.zb592.nemu.zEvE`:

| `band` | `tslot` | `disagreement_scope` | attestations |
|---|---|---|---|
| `copdem30m.provenance` | 0 | `multi_attester` | 89 |
| `geotessera.multi_year` | 54 | `same_attester_provider_substitution` | 5 |

Read `disagreement_scope` before you quote a record. A
`multi_attester` record shows two or more witnesses. A
`same_attester_provider_substitution` record shows one witness that
changed its source.

### Filters and limits

| Field | Effect |
|---|---|
| `cell_prefix` (or `cell`, `cell64`) | a full cell64, or a bytewise prefix of one |
| `band` | one band key |
| `window_unix_s` | `[lo, hi]` on `signed_at`; all disagreeing facts must fall inside |
| `min_severity` | drops records below this score (default 0.1) |
| `limit` | 1 to 1000 records (default 100) |
| `resume_after` | continues a scan that stopped at its time budget |

A scan that stops at its time budget returns `scan_truncated: true` and
a `resume_after` value. Send that value back to continue. A `band`
filter or a narrower `cell_prefix` also gives complete coverage.

### Severity

The score depends on the band kind:

| Band kind | Score |
|---|---|
| Scalar (`indices.ndvi`, `modis.lst_day_8day`, …) | `(max - min) / band_typical_range`, clamped to [0, 1] |
| Vector (foundation embeddings) | `1 - mean(cosine_off_diagonal)` |
| Categorical (`esa_worldcover.lc_2021`, `s2.scl`, …) | `1 - max_class_share` |
| Mixed / unknown | `1.0` (flag for human review) |

### Response fields

- `contradictions[]`: one record per `(cell, band, tslot)` key, with all
  the disagreeing attestations
- `corpus_scanned`: the number of keys the scan read
- `scan_truncated`, and `resume_after` when the scan stopped early
- `time_taken_ms`
- `agent_hint`: one paragraph on what to do with the result
- a signed receipt with primitive `emem.memory_contradictions`

## Memory event stream

`GET /v1/memory/sse` sends each memory write as a Server-Sent Event.
Use it to follow the shared notes, so you do not have to poll. The three query
parameters are optional filters:

| Parameter | An event passes when |
|---|---|
| `path_prefix` | its path starts with this text. For `renamed`, the new path counts. For `consolidated`, the first path counts. |
| `kind` | its `kind` is equal to this value |
| `attester` | its `attester_pubkey_b32` is equal to this key. `expired` and `consolidated` events have no attester, so this filter removes them. |

This read ran on 7 November 2026 and stopped after 4 seconds:

```sh
curl -sN --max-time 4 \
  'https://emem.dev/v1/memory/sse?path_prefix=%2Fmemories%2Fby_attester%2F'
```

The first frame came at once. It names the filters and the two limits
of the stream (text shortened here):

```
event: ready
retry:5000
data: {"type":"ready","stream":"emem.memory_event.v1","filters":{"attester":null,"kind":null,"path_prefix":"/memories/by_attester/"},"keepalive_s":15,"backfill":"none: this is a live tail. ...","resume":"none: events carry no id, ..."}
```

After the `ready` frame, each write sends one `data:` frame with a JSON
object. The `type` field selects the other fields:

| `type` | Sent when | Fields |
|---|---|---|
| `created` | a write makes a new path | `path`, `file_cid`, `kind`, `attester_pubkey_b32`, `signed_at`, `size_bytes` |
| `modified` | a write changes a path that exists (create, `str_replace`, `insert`) | `path`, `file_cid`, `prev_file_cid`, `kind`, `attester_pubkey_b32`, `signed_at` |
| `deleted` | `emem_memory_delete` removes a path | `path`, `kind`, `attester_pubkey_b32`, `deleted_at` |
| `renamed` | `emem_memory_rename` moves a path | `old_path`, `new_path`, `file_cid`, `kind`, `attester_pubkey_b32` |
| `expired` | the TTL pass removes a path (only when `EMEM_MEMORY_TTL_ENABLED=1`) | `path`, `kind`, `expired_at` |
| `consolidated` | a background pass combines episodic notes into one note (only when `EMEM_MEMORY_CONSOLIDATION_ENABLED=1`) | `paths`, `into_cid`, `kind`, `signed_at` |

A supersede changes no bytes and sends no event. A write of the same
bytes to the same path sends `modified`, with `prev_file_cid` equal to
`file_cid`.

Know these limits before you depend on the stream:

- The responder does not sign the events. To check a write, read the
  note and verify its receipt.
- The stream is a live tail. It does not send events from before you
  connected, and events have no `id`, so `Last-Event-ID` cannot fill a
  gap. After a reconnect, read the notes again to find what you missed.
- A client that reads too slowly gets
  `{"type":"lag","reason":"subscriber too slow"}`. The client lost
  events. Read the notes again.
- A comment frame every 15 seconds keeps an idle connection open.
- One responder accepts 256 subscribers at a time (`EMEM_SSE_MAX_SUBS`).
  Above the cap, the request gets a 503 with code `rate_limited`.

## Storage layout

This section is for operators. The memory trees are in sled, under
`var/emem/sled/`. Release 2.4.0 moved the fact index to redb, but the
memory trees did not move.

| Tree | Key | Value |
|---|---|---|
| `emem.memory_files` | path | `file_cid` |
| `emem.memory_files_by_kind` | `kind\|path` | `file_cid` |
| `emem.memory_files_expired` | path | `file_cid` |
| `emem.memory_file_blobs` | `file_cid` | raw bytes, stored once for all paths that hold them |
| `emem.memory_file_history` | path | CBOR `Vec<file_cid>`, oldest first |
| `emem.memory_file_meta` | `file_cid` | CBOR `MemoryFileMeta` (path, signed_at, size_bytes, verb, kind, attester key, attester signature, signed body hash, preimage version, log entry hash, supersede state, receipt). The record stays with the first writer while that path still holds these bytes |
| `emem.memory_file_meta_by_path` | path | the `MemoryFileMeta` of the write at that path |
| `emem.memory_meta_repair` | `v1` | a marker. The responder writes it when it completes the one-time repair of per-path records from the transparency log |
| `emem.memory_path_events` | path | JSON list of each supersede and delete of the path, oldest first |
| `emem.memory_tombstones` | path | JSON `emem.tombstone.v1` (`path`, `prior_file_cid`, `deleted_by`, `deleted_at`, `log_entry_hash_b32`, `note`) |
| `emem.memory_vault` | path | CBOR `emem.vault.v1` envelope (`nonce_b32`, `ciphertext_b32`, `aad`, `responder_pubkey_b32`, `plaintext_len`, `sealed_at`, `attester_pubkey_b32`) |
| `emem.memory_bundles` | `bundle_cid` | CBOR `BundleResp` |

A vault entry is only in `emem.memory_vault`. It is not in the file,
blob or kind trees, so search and the contradiction scan do not see it.
The ChaCha20-Poly1305 key comes from the responder secret, and the
`aad` is the path. For this reason the operator can open a vault entry.

Two paths can hold the same bytes, so one `file_cid` can have two
authors. For this reason the responder keeps the record of each write
by path in `memory_file_meta_by_path`. The record by `file_cid` is for
readers that have only the CID.

The multi-attester index (`emem.multi_attester_index`, value CBOR
`Vec<FactCid>`) has the key `cell`, a zero byte, `band`, a zero byte,
then `tslot` as 8 big-endian bytes. Release 2.4.0 moves it into
`facts.redb` with the fact index. Until the copy to redb is complete,
the responder writes both stores and the scan reads sled.

The responder never deletes a blob. `emem_memory_delete` removes the
path from the index and writes a tombstone. `memory_file_blobs` keeps
the bytes, so you can still get them by `file_cid`. To rebuild the
order of the writes to a path, read `memory_file_history`. To see its
supersedes and deletes, read `memory_path_events`.

## Register your own derivations

`POST /v1/derive` (`emem_derive`) registers a value that **you**
computed over facts that this responder holds. You get back an ordinary
`emem:fact:` token for it. The registered fact names its parents by CID.
A stranger can resolve the token and follow it down to signed
measurements. In this way, "here is my conclusion, trust me" becomes a handle
that anyone can check.

Know three rules before you use it. Each rule is a deliberate refusal,
not a gap.

### Rule 1: the signature says who, not what is true

A derive receipt attests that *this attester submitted this derivation,
over these parent facts, at this time, and the responder stored it*. It
does not attest that the value is true. A tokenised row follows the same
rule. "I ingested these bytes, from this source, at this time" is a
different claim from "this is true".

`provenance_class` must be `model_output`, `human_curated` or
`estimator`. The responder refuses `direct_sensor` and
`deterministic_index`, because it did not compute the value.

There is one narrow exception. It applies only when the call pins a
`code_cid` and the parent values are numbers. The responder then tries
two ways to get the value again:

1. If `op` is `delta` (`inputs[1] - inputs[0]`), `mean` or `sum`, it
   applies that op to the parent values in your signed order.
2. If not, and `fn_key` is the key of a registry algorithm with an
   `evaluation` expression, it evaluates that expression. The expression
   reads the parent values by band name. On 7 November 2026, 74 of the
   172 algorithms in `crates/emem-core/data/algorithms-v0.json` have one.

It evaluates the op or the expression. It never runs your code. If
it gets your value, it records the derivation as
`deterministic_index`. A `recomputation` block names the rule, its ULP tolerance and the measured gap. For `mean`
and `sum` over more than two parents, the window is 4 ULP. If you need
the same bits, require `ulp_gap == 0`. On a mismatch, or when neither way
applies, the class stays as you sent it, with a note.

### Rule 2: you must sign it, and the refusal shows how

Send the body without an `attester` block. The 401 response gives the
exact 32-byte digest to sign for that body. It also gives the encoding
rules and a worked example. Sign the digest and send the same body
again. You need no registration and no API key. Any keypair that you
make locally works.

The preimage uses the memory-write scheme with `verb = "derive"`. It is
the v1 form, with no `base`:

```text
sig = ed25519(blake3("emem.memory_write|derive|/v1/derive|" || body_hash))
```

`body_hash` covers the first ten fields in the table below, in that
order. The 401 response shows the exact encoding in
`details.how_to_sign`.

| Field | Required | Rule |
|---|---|---|
| `fn_key` | yes | Your recipe name, for example `same_doy_ndvi_delta@1`. The responder never runs it. |
| `inputs` | yes | Parent tokens `emem:fact:<cell64>:<fact_cid>`. 1 to 1,024. Your signature covers the order. |
| `cell` | yes | A cell64 to anchor the derived fact. |
| `band` | yes | The band that the value is about. |
| `tslot_window` | yes | `[start, end]`, inclusive. `start` must not be after `end`. |
| `op` | yes | For example `delta`, `mean`, `sum`, `trend`. |
| `value` | yes | Any JSON value. |
| `confidence` | yes | A finite number from 0 to 1. |
| `provenance_class` | yes | `model_output`, `human_curated` or `estimator`. |
| `code_cid` | no | BLAKE3 of your code. The responder records it and never fetches it. |
| `attester` | no | `{pubkey_b32, sig_b32}`. Without it, the responder refuses the call and gives the digest. |
| `budget_ms` | no | A time limit, clamped to 100 to 30,000 ms. Not signed. |

The responder checks the body before it checks the signature. Thus an
unsigned body with a bad field gets the field error, not the 401. Fix
the field errors first. The 401 comes only for a body that is
otherwise valid.

| Status | `details.code` | Cause |
|---|---|---|
| 400 | `derive_invalid_argument` | `fn_key`, `cell`, `band` or `op` is empty. |
| 400 | `derive_invalid_cell` | `cell` is not a cell64. |
| 400 | `derive_no_inputs`, `derive_too_many_inputs` | `inputs` is empty, or has more than 1,024 tokens. Cite a bundle instead. |
| 400 | `derive_invalid_tslot_window`, `derive_invalid_confidence` | The window is out of order, or `confidence` is outside 0 to 1. |
| 400 | `derive_provenance_class_refused` | `direct_sensor`, `deterministic_index` or an unknown class. |
| 400 | `derive_input_unparseable` | A token in `inputs` is not an `emem:fact:` token. |
| 404 | `derive_parent_unresolved` | The responder does not hold that parent fact. |
| 409 | `derive_parent_cell_mismatch` | The token names a cell that is not the cell of the fact. |
| 401 | `derive_attestation_required` | No `attester` block. |
| 401 | `derive_attestation_invalid` | Bad key, or the signature does not verify. The message gives the digest that the responder expected. |

A derivation is idempotent for each pair of attester and body. If you
send the same signed body again, you get the same token with
`deduplicated: true`. Thus a retry after a timeout is safe. With
`budget_ms`, a slow registration returns 202 with `status: "pending"`
and completes in the background. Send the same body again to get the
token.

### Rule 3: default reads never return a derived fact

A derived fact has no canonical `(cell, band, tslot)` key, so the
responder never writes it to the canonical index. `recall`,
`recall_polygon`, `state`, `query_region` and `find_similar` all read
through that index. `memory_search` searches notes only. If you
register a derivation at a cell and then recall that cell, the
derivation is not there.

This is by design. If a claim from a stranger came back in the default
read of another agent, it would poison memory at the fact layer. That
risk is worse than any leak that the `by_attester` namespace closes.
What you get is citation and resolution: a token that resolves and
verifies. emem does not need to assert your claim as fact.

### How to read derivations

To read a derivation, ask for it explicitly:

| Want | Call |
|---|---|
| The bytes behind a token you hold | `POST /v1/memory_token/resolve` |
| Everything one key has registered | `POST /v1/derived` with `attester_pubkey_b32`, and optional `cell`, `band` and `limit` (MCP `emem_derive_list`) |
| What keys concluded near one place | `POST /v1/inferences` with `cell`, or `lat` and `lng`, and optional `radius_m`, `fn_key_prefix`, `attesters`, `min_tier` and `limit` (MCP `emem_inferences`). `GET /v1/inferences` takes the same fields as query parameters. |

`/v1/derived` has no form for all attesters. You must name whose claims
you want. This is the contract, not an optional filter. A request
without the key gets 400 `derived_attester_required`. The `band` filter
works only together with `cell`, because the index puts the cell
first. A `band` without a `cell` gets 400 `derived_band_needs_cell`.
`limit` is 1 to 1000, and the default is 100.

`/v1/inferences` reads by place. It does not accept a place name: use
`/v1/locate` first. It reads every cell within `radius_m` of the point,
nearest first. The default is 5000 m and the maximum is 50000 m. A
value outside 0 to 50000 gets 400. With `0` it reads the one cell. Each
item has its `cell` and `distance_m`. A cell is 9.55 m wide, so a reader
one street away finds a study anchored at its own point. `limit` is 1 to
500, and the default is 50.

The trust policy decides which keys `/v1/inferences` admits. By default
it admits each key that this responder reads at `T4_affiliated` or
above. For such a key, an organisation vouched for it by DNS or
`.well-known` within the last 30 days (`GET /v1/enlist`). Send
`min_tier` to move that floor. Send `attesters` to admit only the keys
that you name.

The response gives these items:

- the newest derivation per `(cell, fn_key, attester)`
- `excluded_by_policy`, the count that the policy left out
- a mark on each value that it is data, not instructions
- a sentence that says what a signature proves: who concluded
  something, never that it is true

`/v1/ask` and `/v1/locate` (and their MCP tools) can add a
`research_nearby` block. They add it when vouched research lies near
the place they resolved. The search radius is half the diagonal of the
place's polygon box, at least 5 km and at most 50 km. A bare point with
no box gets 5 km. The block uses the default trust policy
(`T4_affiliated`) and lists at most 5 items. It holds pointers only:
token, `fn_key`, nick, tier, distance and `signed_at`. No derivation
value enters the answer or its receipt. To read the values, use
`/v1/inferences`. If that search takes more than 2 seconds, the response
omits the block.

Registration is idempotent per `(your key, derivation body)`. If you
post the same derivation again, you get the token that the responder
already made for it. A retry after a timeout is safe and makes no
second copy.

[protocol.md § 5.2](protocol.md) gives three more items:

- the exact `DeriveBody` wire encoding
- the two traps that a generic CBOR encoder falls into
- how a verifier rebuilds the signature of the caller from a stored fact

## What the substrate is not

| It is not | Who owns that pattern | What emem does instead |
|---|---|---|
| A chat memory | Mem0, LangMem | Signed notes that another party, or a later run of you, resolves to the same bytes and verifies. emem does not extract entities from messages, keep session history or remove duplicate paraphrases. |
| A knowledge graph | Zep, Graphiti | Contradiction detection looks at one `(cell, band, tslot)` at a time. It does not reason over relations with many hops. |
| A stateful agent process | Letta | emem is a server that an agent calls. The agent decides what to remember. |

The hosted namespace is a commons that everyone can read. An owner cannot
limit who reads a note. A `vault` entry hides its bytes from other
callers but not from the operator. Keep private scratch on your side, or
encrypt it before you write it.

What the substrate gives you:

- Under the default write policy, the author signs each memory write.
  The responder signs each receipt.
- The main read primitives take a bi-temporal filter (see
  [Time filters on reads](#time-filters-on-reads-bi-temporal)).
- Each note and each fact has a content address.
- An `emem:fact:` token names the full 32-byte hash of the signed body.
  It resolves to the same bytes on each responder that holds the fact.
- `emem:entity:` and `emem:bundle:` tokens are 16-byte anchors. They
  are a shared reference, not a hash of the whole record.
- The responder keeps disagreements between attesters as evidence.
- Receipts verify offline with ed25519.

## See also

- [model.md](model.md): the formal model behind notes, facts, tokens and
  time filters.
- [sleep-agent.md](sleep-agent.md): an opt-in loop that reads the
  contradiction scan and proposes merged notes.
- [glossary.md](glossary.md): one meaning for each term on this page.
- [agents.md](agents.md): the read-side guide for callers that come
  from Mem0, Letta or LangGraph.
- [whitepaper-v3.md](whitepaper-v3.md): the protocol, the trust layer and
  the receipt rules.
- [security.md](security.md): what the responder checks, what it proves,
  and what it does not claim.
- [protocol.md](protocol.md): the cell64, cid64 and tslot bit layouts.
- [errors.md](errors.md): each typed error code, with
  `memory_attestation_invalid`, `memory_namespace_violation`,
  `invalid_temporal_bound` and `invalid_signed_at_format`.
- `/v1/verify_receipt`: POST `{receipt}` back. The response gives
  `valid`, `signature_valid`, `merkle_proof_valid`, `primitive`,
  `preimage_blake3_hex`, `signer_pubkey_b32` and other fields.
