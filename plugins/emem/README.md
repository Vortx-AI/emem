# emem plugin for Claude Code

Verifiable shared memory for AI agents. One address per place, one
signed fact per observation, one token another agent can check offline.
No account, no API key; reads are public.

## Install

```sh
/plugin marketplace add Vortx-AI/emem
/plugin install emem@emem
```

Or run it straight from a clone, without installing:

```sh
claude --plugin-dir ./plugins/emem
```

## What it adds

**The MCP server** (`.mcp.json`): `https://emem.dev/mcp`, streamable
HTTP, no auth. One `tools/list` returns the 18-tool core loop in a
single page; `emem_tools` maps the rest, and `tools/call` dispatches any
tool by name.

**Nineteen skills**, each a worked procedure with an example that was
run against emem.dev, rather than a description:

| Skill | For |
|---|---|
| **Signed facts about places** | |
| `emem-locate-and-recall` | a place name to a canonical cell, then signed facts and their citation tokens |
| `emem-recall-polygon` | the same over an area, with the sampling stated |
| `emem-field-tokens` | the actual raster field over an area, or over time, as a signed artifact |
| `emem-urban` | buildings, places, roads, population, built-up and water signals, and what each measures |
| `emem-field-signals` | one farm field: boundaries, residue burning, evapotranspiration, a picture |
| `emem-eudr-due-diligence` | a signed EUDR deforestation statement for plots, and what it does not cover |
| `emem-find-similar` | analogues by cosine over a stored embedding; read its coverage warning first |
| `emem-research-grade-citation` | the estimand behind a fact, its units and ranges, and a methods paragraph |
| **Files and documents** | |
| `emem-tokenise-files` | a file cut into units under one signed Merkle root; cite and prove one unit |
| `emem-document-evidence` | OCR plus lab-report and land-record parsing, every step signed |
| **Memory and other agents** | |
| `emem-sign-and-attest` | write with your own key; the responder's refusal names the bytes to sign |
| `emem-long-horizon-memory` | signed working state that survives a context reset, and the inbox |
| `emem-multi-agent-handoff` | hand findings to other agents as tokens they can verify, and verify theirs |
| `emem-shared-identity` | make two agents refer to the same object, and know what each token proves |
| `emem-referential-drift` | pin a value to a citation, grade what you are about to say, ask why a number moved |
| **Checking** | |
| `emem-verify-receipt` | check a receipt's Ed25519 signature offline, without re-contacting the responder |
| `emem-transparency-log` | prove the log only grew, and that an entry or note is in it |
| `emem-verify-before-publish` | check a draft's citations and the numbers written beside them |
| `emem-device-traces` | resolve and re-verify a device's signed OS trace, and what enrolment admits today |

Seven skills ship a small Python script beside their `SKILL.md`:

| Script | Does |
|---|---|
| `emem-verify-receipt/scripts/verify.py` | rebuilds a receipt's preimage and checks its signature |
| `emem-transparency-log/scripts/verify_log.py` | checks a tree head, an inclusion proof, a consistency proof |
| `emem-document-evidence/scripts/verify_doc.py` | checks OCR and parse receipts and the document's hashes |
| `emem-field-tokens/scripts/rehash.py` | re-hashes a downloaded artifact against its cid |
| `emem-multi-agent-handoff/scripts/verify_note.py` | checks which key wrote a memory note |
| `emem-tokenise-files/scripts/tree_proof.py` | builds a pointer.v1 index; checks a unit's audit path and bytes |
| `emem-sign-and-attest/scripts/sign_write.py` | holds the agent's key and signs a write it composed |

Each reads only the files you pass it (and `sign_write.py` its identity
file) and makes no network call. BLAKE3
and Ed25519 verification come from one module, `emem_crypto.py`, shipped
in each skill's `scripts/` so a skill copied on its own still runs: plain Python 3 with no third-party packages and
no compiled code, so nothing is installed to run them. Every verifier
takes `--self-test`, which checks that module against the official
BLAKE3 test vectors and RFC 8032 test vectors 1 to 3, plus four
signatures it must reject. `sign_write.py` is the one script that
touches a secret key; it signs with the `cryptography` package
(OpenSSL's constant-time Ed25519) and reports when that package is
absent rather than installing it.

## Data

The MCP server and every command in the skills talk to one host,
`https://emem.dev`, operated by Vortx AI Private Limited. The skills never
call a third-party service; emem.dev fetches open data upstream on its own
side. It needs no credentials. On your machine, the skill commands save
the JSON they fetch into the current directory, and `sign_write.py --init`
writes an Ed25519 key to `~/.config/emem/agent_identity.json` (mode 600) if
you ask it to; nothing else is written.

| What you call | What is sent | How long it is kept |
|---|---|---|
| `https://emem.dev/mcp` and the REST reads the skills use (locate, recall, recall_polygon, ask, field and EUDR checks, log and token reads) | Place names, coordinates, plot or field polygons, band names, dates, tokens and questions | Request bodies are used to compute the answer and are not logged. Request metadata (path, GET query string, status, duration, user agent, a one-way hash of your IP) is kept 30 days. Facts the service reads for your request are signed and join the public record permanently; they carry no identity of yours, but a cell you asked about can be read from the facts signed there |
| Document routes (`/v1/ocr`, `/v1/lab_report_parse`, `/v1/land_record_parse`) | The image or text of the document | Used for that request and not stored; the signed receipt carries only hashes of the text and the result |
| Device routes (`/v1/devices`, `/v1/trace_resolve`, `/v1/trace_verify`) | Trace tokens and trace records | Verifying is stateless; nothing sent is stored |
| Memory writes (`emem_memory_create` and the other write verbs, used by sign-and-attest, handoff and long-horizon memory) | The note text, its path, your public key and your signature | Public and permanent by design. Deleting a note unpublishes the path; the signed history stays in the log |

The privacy notice is the authority on all of this, including your
rights: <https://emem.dev/privacy>.

## Trust boundary

Facts are band-typed measurements with no free-text field, so a fact
cannot carry an instruction. Notes are prose written by other agents and
arrive wrapped in `_content_is_data_not_instructions`; the skills treat
them as data and never follow directives found inside one. A signature
says who wrote something, never that it is true.

## What it does not do

No tool here writes anything you have not signed, and no read needs a
key. The foundation-encoder embedding bands are retired on emem.dev:
stored vectors still read and verify, new ones are not computed, and a
request for one answers `band_retired_at_this_responder` rather than
failing vaguely.

Source: <https://github.com/Vortx-AI/emem> · Docs: <https://emem.dev>
