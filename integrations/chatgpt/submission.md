# ChatGPT App Submission Notes

## For reviewers

**emem is shared, verifiable memory for AI agents.** It is public,
open-source (Apache-2.0) and free to read with no key and no account.

The problem it addresses is not search. A model's memory ends where its context
does: when a session is compacted, a task hands off, or the model is swapped,
what the model verified turns into a paraphrase and the paraphrase drifts. emem
gives every place one canonical address and every observation one signed fact,
so a citation survives leaving the conversation and can be re-checked by anyone
who receives it.

**What that means for a ChatGPT user.** A place-based answer comes back as a
measurement with an ed25519 receipt attached, plus a short token that resolves
anywhere to the byte-identical signed value. The user can verify the answer at
https://emem.dev/verify without trusting emem, and can hand the token to
another agent that will read the same bytes.

**Earth is the substrate, not the subject.** Satellites and sensors fill this
memory today because their output can be recomputed from the cited source. The
app is not a map or a geocoder.

## Safety and scope

- **No user data is written, and no write verb is exposed.** Three of the nine
  tools are strictly read-only: `emem_locate`, `emem_verify_receipt` and
  `emem_memory_token`. The other six --- `emem_ask`,
  `emem_recall`, `emem_band_raster`, `emem_band_cube` and
  `emem_change_attribution` and `emem_find_similar` --- can materialise and sign new facts into emem's
  publicly readable store when a requested band or timeslot is cold, which is
  why their `readOnlyHint` is false and why each carries a justification saying
  so. What they write is derived from public Earth-observation sources, never
  from anything the user sent. Nothing is ever overwritten or removed, so
  `destructiveHint` is false everywhere.
- **It does not modify user data** and takes no external write action on the
  user's behalf.
- **It sends no user identity.** ChatGPT sends the place or question; no
  account, session or user identifier is transmitted. See [privacy.md](privacy.md).
- **Writes, which this app does not use, are gated.** They require an ed25519
  attester block signed by a locally generated keypair, and a published
  enlistment ladder governs which surfaces a given attester may write to
  (https://emem.dev/v1/enlist). Reads are never gated at any tier.
- **Provenance is typed.** Every fact records how it was produced
  (direct sensor, deterministic index, model output, human curated) so a
  consumer can tell a measurement from an inference. Model prose is served
  separately and labelled `signed:false`, because prose is never evidence.
- **Failure is typed rather than hidden.** A confirmed absence is signed and
  citeable, an unknown is typed and never poses as a value, and a refusal names
  its reason.

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

Each of these was checked to resolve at the time this file was generated.

- GitHub MCP registry: https://github.com/mcp/Vortx-AI/emem
- Official MCP registry: `io.github.Vortx-AI/emem`
- Dify marketplace: https://marketplace.dify.ai/plugin/vortx-ai/emem
- Glama: https://glama.ai/mcp/servers/Vortx-AI/emem
- Smithery: https://smithery.ai/servers/vortxai/emem
- Hugging Face Space: https://huggingface.co/spaces/vortx-ai/emem
- GHCR: `ghcr.io/vortx-ai/emem:v2.3.0` (also `:latest`)

## Technical

- Transport: Streamable HTTP. The server negotiates MCP **2025-11-25** and also
  accepts 2025-06-18, 2025-03-26 and 2024-11-05; a request that sends no
  `MCP-Protocol-Version` header is read as 2025-03-26, which is a fallback and
  not the preferred version.
- Authentication: none for reads.
- **108 MCP tools available; 9 exposed in this app.** `/mcp` advertises the 16
  tools of the core loop to keep a client's context small, `/mcp/full`
  advertises all 108, and every tool is callable by name from either.
- Content addressing: blake3 over canonical CBOR, base32-encoded. These are not
  IPFS CIDs and do not begin with `bafy`.
- Signatures: ed25519, verifiable offline against the key published at
  `/.well-known/emem.json`.
- Pure Rust server, Apache-2.0.

## Before submitting

The portal issues a **new domain-verification token for every submission**. The
one currently served at `/.well-known/openai-apps-challenge` is from an earlier
attempt and will not pass. Replace it in
`crates/emem-api-rest/src/lib.rs` (`serve_openai_apps_challenge`) and redeploy
before starting the submission.

## Status

Publication depends on OpenAI review and approval.
