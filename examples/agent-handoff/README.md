# A session checkpoint that outlives its agent

Most examples in this repository hand over a geospatial fact. This one
hands over the agent's own working state. Run 1 stores a checkpoint as
one signed note. Run 2 uses a different key in a different process. It
reads the checkpoint, checks it, and continues the task.

## Run it

```bash
pip install -e "sdks/emem-py[signing]"     # installs the ememdev CLI
cargo build --release --bin emem-server    # one time
./examples/agent-handoff/run.sh
```

The script starts a throwaway responder on `127.0.0.1:5099` (set `PORT`
to change it). The responder has its own temporary data directory and
its own key. Each run gets a new temporary `HOME`, so the script does
not use your own agent key. The note goes only to this local
responder, never to emem.dev. On exit, the script stops the responder
and deletes the temporary files.

We did not run the script on 7 November 2026, because it needs a cargo
build. On that day we ran the run 2 read and check against emem.dev on
an existing note. The calls are below.

## What the script does

1. Run 1 stores the state. The checkpoint holds the step it completed,
   the next action, its findings and a resume hint. `ememdev write`
   signs a create with the run 1 key. It writes the note to
   `/memories/by_attester/<pubkey8>/checkpoint-survey-task.md`. The
   responder returns a `file_cid` and a signed receipt. A `file_cid` is
   the first 16 bytes of `blake3(bytes)`, in lowercase base32. Run 1
   prints one line to hand on: the path and the `file_cid`.
2. Run 2 reads the note. It calls `emem_memory_view` on the path. It
   compares the `file_cid` in the answer with the one from run 1. A
   change to any byte gives a different `file_cid`. Then it sends the
   receipt to `/v1/verify_receipt`, which checks the responder's
   Ed25519 signature. Only after both checks does it read `next_action`.

## The run 2 check on emem.dev

These two calls ran on 7 November 2026 against a note that already
exists on emem.dev. Both are reads.

```bash
curl -s -X POST https://emem.dev/mcp -H 'content-type: application/json' -d '{
  "jsonrpc":"2.0","id":1,"method":"tools/call",
  "params":{"name":"emem_memory_view","arguments":{
    "path":"/memories/by_attester/k572x7go/note-20260929-line-url-to-ddzmyzhn.md"}}}' \
  | python3 -c "import sys,json; b=json.loads(json.load(sys.stdin)['result']['content'][0]['text']); \
print(b['file_cid']); json.dump({'receipt': b['receipt']}, open('receipt.json','w'))"

curl -s -X POST https://emem.dev/v1/verify_receipt \
  -H 'content-type: application/json' --data-binary @receipt.json \
  | python3 -c "import sys,json; print(json.load(sys.stdin)['signature_valid'])"
```

The first call printed `uexdskdusjv5fsekzw3cdjdily`. The second printed
`True`.

## What this shows, and what it does not

- The checkpoint is not in a context window. It is a signed file that
  the next agent reads and checks. It stays after a context reset, a
  crash or a model change.
- Run 2 needs no shared credentials. It checks the content address and
  the receipt signature.
- The receipt proves which responder stored the note. The note's own
  Ed25519 signature proves which key wrote it. The script checks only
  the receipt, and it asks the responder to do the check.
- To check which key wrote the note, with no trust in any server, use
  `check_author.py` in
  [docs/memory.md](../../docs/memory.md#write-your-first-signed-note).
- On the hosted node, anyone can read the
  `/memories/by_attester/<pubkey8>/` namespace. Only writes need the
  key. So any agent can read a checkpoint that you store there.
  [docs/roadmap.md](../../docs/roadmap.md) lists owner-scoped reads as
  open work. Until they ship, keep private agent state on a
  self-hosted responder, like the one this script starts.
