# ChatGPT App Submission Notes

These notes are for the OpenAI reviewer of the emem app. The last section
tells the operator how to pass the domain check before a submission.

## For reviewers

emem gives AI agents a shared memory that anyone can verify. It is public and
open source (Apache-2.0). Reads are free and need no key and no account.

The app solves a memory problem. A model's memory ends where its context ends.
A client compacts a session, a task goes to another agent, or a new model
replaces the old one. Then the facts that the model checked become a paraphrase, and the
paraphrase drifts.

emem gives each place one canonical address and each observation one signed
fact. A citation stays valid outside the conversation. Anyone who receives it
can check it again.

For a ChatGPT user, a place-based answer is a measurement with an ed25519
receipt. It also has a short token. The token resolves anywhere to the same
signed bytes. The user can verify the answer at https://emem.dev/verify, with
no trust in emem. The user can also give the token to another agent, which
then reads the same bytes.

Satellites and sensors fill this memory today, because anyone can recompute
their output from the cited source. The app is not a map and not a geocoder.

## Safety and scope

The 18 tools fall into three groups (live `/mcp` annotations, 7 November
2026):

| Group | Tools | What they write |
|---|---|---|
| Read-only (9) | `emem_locate`, `emem_memory_token`, `emem_memory_token_resolve`, `emem_verify_receipt`, `emem_memory_contradictions`, `emem_guard_verdict`, `emem_tools`, `emem_echo_verify`, `emem_entity_resolve` | Nothing. |
| Can sign new facts (7) | `emem_ask`, `emem_recall`, `emem_find_similar`, `emem_intent`, `emem_memory_bundle`, `search`, `fetch` | When a requested band is cold, they fetch public Earth observation data and sign new facts. |
| Gated writes (2) | `emem_entity`, `emem_entity_link` | A shared identity, or a link from a phrasing to one. |

`search` and `fetch` call `emem_recall` under the two names that a ChatGPT
connector looks for.

The seven tools in the second group put new facts in emem's public store.
That is why their `readOnlyHint` is false. These facts come from public Earth
observation sources, never from what the user sent. No tool overwrites or
removes data, so `destructiveHint` is false on all 18 tools.

`emem_entity` mints a shared identity. `emem_entity_link` binds a phrasing to
an identity. Both change the name that every other agent resolves, so each
call needs an ed25519 attester block. The caller signs it with a keypair that
the caller makes locally. The caller must also hold tier T3 on the enlistment
ladder, which `https://emem.dev/v1/enlist` publishes. Without a signature, the server refuses
with HTTP 403 and returns the digest to sign. A ChatGPT user has no such key,
so in this app both tools refuse and write nothing.

An earlier draft of this page said "none of emem's write verbs is exposed".
That stopped being true when the app grew to the full core loop of 18 tools.
The two gated tools above are the correct statement.

What the app does not do:

- It does not change user data.
- It takes no external action for the user.
- It sends no user identity. ChatGPT sends the place or the question, and no
  account, session or user identifier. See [privacy.md](privacy.md).
- It never gates a read, at any tier.

How the app labels what it returns:

- Each fact records how emem made it, as one of seven provenance classes:
  `direct_sensor`, `deterministic_index`, `estimator`, `attested_execution`,
  `model_output`, `human_curated` and `unclassified`. A reader can tell a
  measurement from an inference.
- emem serves model prose apart from facts, with the label `signed: false`.
  Prose is never evidence.
- A confirmed absence is a signed fact that a user can cite.
- An unknown has a type and never shows as a value.
- A refusal names its reason.

## Links

- Homepage: https://emem.dev
- Repository: https://github.com/Vortx-AI/emem
- MCP endpoint: https://emem.dev/mcp
- Action schema (import this for Actions): https://emem.dev/openapi.action.json
- Agent card (A2A): https://emem.dev/.well-known/agent-card.json
- Verify an answer: https://emem.dev/verify
- Security and trust model: https://emem.dev/docs/security.html
- Privacy: https://emem.dev/privacy and [privacy.md](privacy.md)
- Terms: https://emem.dev/terms
- Support: https://emem.dev/support, avijeet@vortx.ai

## Listed on

We checked each listing on 7 November 2026.

| Directory | Address | Check |
|---|---|---|
| GitHub MCP registry | https://github.com/mcp/Vortx-AI/emem | Page returns 200. |
| Official MCP registry | `io.github.Vortx-AI/emem` | Registry API lists versions up to 2.4.4. |
| Dify marketplace | https://marketplace.dify.ai/plugin/vortx-ai/emem | Page returns 200. |
| Glama | https://glama.ai/mcp/servers/Vortx-AI/emem | Page returns 200. |
| Smithery | https://smithery.ai/servers/vortxai/emem | The registry API at `registry.smithery.ai` lists `vortxai/emem`. The web page returned 404 to a script; it returned 404 for other servers too. |
| Hugging Face Space | https://huggingface.co/spaces/vortx-ai/emem | Page returns 200. |
| GHCR | `ghcr.io/vortx-ai/emem:v2.4.4`, also `:latest` | Both tags resolve. |

## Technical

- Transport: Streamable HTTP. The server prefers MCP 2025-11-25. It also
  accepts 2025-06-18, 2025-03-26 and 2024-11-05. A request with no
  `MCP-Protocol-Version` header gets 2025-03-26 as a fallback.
- Authentication: none for reads.
- Tools: `/mcp` lists the 18 tools of the core loop, to keep a client's
  context small. `/mcp/full` lists all 116 tools. A client can call any tool
  by name on either endpoint.
- Content addressing: BLAKE3 over canonical CBOR, encoded as base32. These are
  not IPFS CIDs and do not start with `bafy`.
- Signatures: ed25519. Verify them offline against the key at
  `/.well-known/emem.json`, field `federation.node.responder_pubkey_b32`.
- Server: pure Rust, Apache-2.0.

## Pass the domain check

The portal issues a new domain-verification token for each submission. This
token must not go into the repository. The route
`/.well-known/openai-apps-challenge` reads `EMEM_APPS_CHALLENGE` from the
environment. When that variable is not set, the route returns 404. Thus a node
with no token says so and never serves an old one. On 7 November 2026,
emem.dev returned 404 on this route.

Set the token on the host before you start a submission. Do not put it in
source.

1. Run `sudo systemctl edit emem-server`. Under `[Service]`, add the line
   `Environment=EMEM_APPS_CHALLENGE=<token>`.
2. Make sure that the `docker run` line of the unit has `-e EMEM_APPS_CHALLENGE \`.
   This flag sends the host variable into the container.
3. Restart the server and read the route:

   ```sh
   sudo systemctl restart emem-server
   curl -s https://emem.dev/.well-known/openai-apps-challenge
   ```

Step 2 is necessary because production runs the server in a container. The
unit in `deploy/systemd/emem-server.service` sends only the variables that it
names with `-e`. On 7 November 2026 it did not name `EMEM_APPS_CHALLENGE`.
Without step 2, the route stays at 404.

These steps need root on the production host. We did not run them.

An earlier version of this section said to paste the token into
`crates/emem-api-rest/src/lib.rs`, in a function that does not exist. This
repository is public. An instruction to put a per-submission credential in it
is worse than a stale instruction. That is why this note records the error
and does not delete it.

## Status

Publication depends on OpenAI review and approval.
