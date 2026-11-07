# A session checkpoint that outlives its agent

The other examples in this repository hand over a geospatial fact.
This one hands over the state of an agent.
Run 1 writes a checkpoint as one signed memory file.
Run 2 is a different process and uses no key.
It reads the checkpoint and checks it.

## What you need

- The `ememdev` CLI with the signing extra. Install it from this clone with `pip install -e "sdks/emem-py[signing]"`.
  The PyPI package `ememdev[signing]` 2.4.4 has the same `whoami` and `sign` commands.
- An `emem-server` release binary at `target/release/emem-server`. Build it once with `cargo build --release --bin emem-server`.
- `curl` and `python3`. Port 5099 must be free, or set `PORT`.

## Run it

```bash
pip install -e "sdks/emem-py[signing]"
cargo build --release --bin emem-server
./examples/agent-handoff/run.sh
```

The script writes, but only to a throwaway local responder.
Run 1 gets a new temporary `HOME`, so the script does not use your agent key.
That responder has its own temporary data directory and its own key.
The script does not touch emem.dev or a real identity, and it deletes everything on exit.

On 7 November 2026 we ran `run.sh` end to end with `ememdev` 2.4.4 and an `emem-server` built from commit `49443d9a`.
It took 1.5 seconds and printed this (the `<pubkey8>` changes on each run):

```text
==> boot a throwaway responder (own data dir, own key)
==> run 1: do some work, then park the state as ONE signed file
    parked at /memories/by_attester/<pubkey8>/checkpoint-survey-task.md
    file_cid  nmfkkvepfsgvrs4oinqzqkhcny
    the ONE LINE run 1 leaves behind (in a report, an issue, anywhere):
    /memories/by_attester/<pubkey8>/checkpoint-survey-task.md  cid nmfkkvepfsgvrs4oinqzqkhcny
==> run 2: a different process with no key resumes and checks the bytes
    content round-tripped; file_cid matches what run 1 parked
    signature_valid: True
==> done: the state outlived its session, and the successor verified it
```

The `file_cid` is the same on each run, because it is a hash of the checkpoint bytes only.

### Why the script does not use `ememdev write`

`ememdev write` in 2.4.4 signs the v1 write digest.
The code on `main` refuses the v1 digest for every write since 5 November 2026 (commit `23fa7fd6`).
The refusal has `reason: "v1_retired"`. We saw this refusal from a local node built from commit `49443d9a` on 7 November 2026.
So the script signs the v2 digest with `ememdev sign --verb create --base absent`.
Then it sends `emem_memory_create` itself.
`--base absent` tells the responder that nothing is at the path yet.

## What you get

The script prints each step and stops with an error if a check fails.

1. Run 1 writes the state.
   The checkpoint holds the completed step, the next action, the findings and a resume hint.
   `ememdev sign` signs the create with the local Ed25519 key of the agent.
   The file goes under the namespace of that agent, `/memories/by_attester/<pubkey8>/`.
   The responder returns a `file_cid` and a signed receipt.
   Run 1 leaves one line behind: the path and the `file_cid`.
2. Run 2 is a different process and uses no key. It is plain `curl`.
   It calls `emem_memory_view` on the path and compares the returned `file_cid` with the one from run 1.
   It checks that the content holds `next_action`. It does not act on it.
   Then it sends the receipt to `POST /v1/verify_receipt` and stops with an error unless `signature_valid` is true.

## What this shows, and what it does not

- The checkpoint is not in a context window. It is a signed file that a later agent reads back.
  So it survives compaction, a crash, a model change or a vendor change.
- `file_cid` is the first 16 bytes of the BLAKE3 hash of the file bytes, in lowercase base32.
- The script compares two `file_cid` values that the responder returned. It does not hash the content itself.
  `POST /v1/verify_receipt` is the responder checking its own signature.
  Neither check is independent of the responder.
- For an independent check, hash `content` on the client and compare it with `file_cid`.
  Then verify the receipt on the client with `em.verify_receipt` from the Python SDK
  (see [docs/quickstart.md](../../docs/quickstart.md)) or on the [`/verify`](https://emem.dev/verify) page.
  This Python reads a public note on emem.dev and does the hash check.
  `blake3` comes with `ememdev[signing]`.
  On 7 November 2026 it printed `citmouad4wp4gbcvharhifvsnm`.
  The same two hash lines also matched the checkpoint above.

  ```python
  import base64, json, urllib.request
  import blake3

  path = "/memories/by_attester/ye65e64r/task-add-cosmos-media-input-2026-08-22.md"
  call = {"jsonrpc": "2.0", "id": 1, "method": "tools/call",
          "params": {"name": "emem_memory_view", "arguments": {"path": path}}}
  req = urllib.request.Request("https://emem.dev/mcp", data=json.dumps(call).encode(),
                               headers={"content-type": "application/json"})
  body = json.loads(json.load(urllib.request.urlopen(req))["result"]["content"][0]["text"])

  mine = base64.b32encode(blake3.blake3(body["content"].encode()).digest()[:16]).decode().rstrip("=").lower()
  assert mine == body["file_cid"]
  print(mine)
  ```

  For a local checkpoint, change the URL to your responder and `path` to the path that run 1 printed.

- On the hosted node, anyone can read `/memories/by_attester/`. Only the attester can write there.
  So every agent can read a checkpoint on emem.dev. It is not private.
  Keep private agent state on a self-hosted responder, as this script does, until owner-scoped reads ship.
  [docs/roadmap.md](../../docs/roadmap.md) gives the details.
