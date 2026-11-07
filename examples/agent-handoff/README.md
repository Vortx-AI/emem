# A session checkpoint that outlives its agent

The other examples in this repository hand over a geospatial fact.
This one hands over the state of an agent.
Run 1 writes a checkpoint as one signed memory file.
Run 2 uses a different identity in a different process.
It reads the checkpoint, checks it, and continues the task.

## What you need

- The `ememdev` CLI with the signing extra. Install it from this clone with `pip install -e "sdks/emem-py[signing]"`.
  The PyPI package `ememdev[signing]` 2.4.4 has the same `whoami` and `write` commands.
- An `emem-server` release binary at `target/release/emem-server`. Build it once with `cargo build --release --bin emem-server`.
- `curl` and `python3`. Port 5099 must be free, or set `PORT`.

## Run it

```bash
pip install -e "sdks/emem-py[signing]"
cargo build --release --bin emem-server
./examples/agent-handoff/run.sh
```

The script writes, but only to a throwaway local responder.
That responder has its own temporary data directory and its own key.
The script does not touch emem.dev or a real identity, and it deletes everything on exit.

We did not run `run.sh` end to end for this page, because the docs host has no Rust toolchain to build `emem-server`.
On 7 November 2026 we checked its parts separately:

| Part | Check on 7 November 2026 |
|---|---|
| `ememdev whoami` | Returns `pubkey_b32`, `pubkey_short` and `namespace_root`. A different `HOME` gives a different key. |
| `ememdev write --path --kind --base-url --body-file` | All four flags exist in `ememdev write --help` (2.4.4). |
| `emem_memory_view` on a note on emem.dev | Returns `file_cid`, `content` and `receipt`, as the script expects. |
| `POST /v1/verify_receipt` with that receipt | Returns `signature_valid: true`. |

## What you get

The script prints each step and stops with an error if a check fails.

1. Run 1 writes the state.
   The checkpoint holds the completed step, the next action, the findings and a resume hint.
   `ememdev write` signs the create with the local Ed25519 key of the agent.
   The file goes under the namespace of that agent, `/memories/by_attester/<pubkey8>/`.
   The responder returns a `file_cid` and a signed receipt.
   Run 1 leaves one line behind: the path and the `file_cid`.
2. Run 2 resumes with a different identity.
   It calls `emem_memory_view` on the path and compares the returned `file_cid` with the one from run 1.
   It sends the receipt to `POST /v1/verify_receipt`.
   Only after both checks pass does it read `next_action` and continue.

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
  On 7 November 2026 this Python matched `file_cid` for a note on emem.dev:

  ```python
  import base64, blake3
  mine = base64.b32encode(blake3.blake3(body["content"].encode()).digest()[:16]).decode().rstrip("=").lower()
  assert mine == body["file_cid"]
  ```

- On the hosted node, anyone can read `/memories/by_attester/`. Only the attester can write there.
  So every agent can read a checkpoint on emem.dev. It is not private.
  Keep private agent state on a self-hosted responder, as this script does, until owner-scoped reads ship.
  [docs/roadmap.md](../../docs/roadmap.md) gives the details.
