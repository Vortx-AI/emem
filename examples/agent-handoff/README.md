# A session checkpoint that outlives its agent

Every other example in this repository hands over a geospatial fact.
This one hands over what the long-horizon pitch actually promises: an
agent's OWN state. Run 1 parks a checkpoint as one signed memory file;
run 2, a different identity in a different process, resumes from it
and verifies it trusted nobody, not even run 1.

```bash
pip install -e "sdks/emem-py[signing]"     # the ememdev CLI
cargo build --release --bin emem-server    # once
./examples/agent-handoff/run.sh
```

The script boots a THROWAWAY responder with its own temp data dir and
its own key, so it touches no shared node and no real identity, and it
deletes everything on exit. Roughly thirty seconds end to end.

## What actually happens

1. **Run 1 parks state.** A checkpoint (step completed, next action,
   findings, a resume hint) becomes one file under the agent's own
   `/memories/by_attester/<pubkey8>/` namespace, written with
   `ememdev write`: the CLI signs the create with the agent's local
   ed25519 key, and the responder returns a `file_cid` (the blake3 of
   the content) plus a signed receipt. What run 1 leaves behind is one
   line: the path and the cid.
2. **Run 2 resumes.** A different identity calls `emem_memory_view` on
   that path, checks the returned `file_cid` against the one run 1
   published (byte drift is impossible to miss: the cid is the
   content), and sends the receipt to `/v1/verify_receipt`, which
   checks the responder's ed25519 signature. Only then does it read
   `next_action` and continue the task.

## What this shows, and what it does not

- The checkpoint survives compaction, crashes, model swaps, and
  vendor changes, because it is not in anyone's context window: it is
  a signed file a successor RESOLVES rather than a summary it
  inherits.
- The successor's trust is arithmetic: content id plus signature,
  no shared credentials, no "the previous agent said so".
- On the HOSTED node this namespace is world-readable (the roadmap's
  honest limits say exactly how), so a checkpoint like this is shared
  state, not private state. Private agent state belongs on a
  self-hosted responder, like the throwaway one this script boots,
  until owner-scoped reads ship.
