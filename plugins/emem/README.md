# emem plugin for Claude Code

This plugin gives Claude Code verifiable shared memory. One place has one
address. One observation has one signed fact. One token lets another agent
check that fact offline. You need no account and no API key, because reads
are public.

## Install

```sh
/plugin marketplace add Vortx-AI/emem
/plugin install emem@emem
```

To use the plugin from a clone without an install, run:

```sh
claude --plugin-dir ./plugins/emem
```

## What the plugin adds

### The MCP server

The file `.mcp.json` points to `https://emem.dev/mcp`. It uses streamable
HTTP and no authentication. One `tools/list` call returns the 18-tool core
loop on one page (checked on 7 November 2026). The tool `emem_tools`
lists the other tools. `tools/call` runs any tool by name, listed or not.

### Nineteen skills

Each skill is a procedure with an example. Each example ran against
emem.dev before the skill shipped.

| Skill | Use it for |
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
| `emem-sign-and-attest` | write with your own key; the signer computes the digest, so there is no refusal round trip |
| `emem-long-horizon-memory` | signed working state that survives a context reset, the inbox, and how to wait for a reply |
| `emem-multi-agent-handoff` | hand findings to other agents as tokens they can verify, verify theirs, key cards, and notes sealed to one key |
| `emem-shared-identity` | make two agents refer to the same object, and know what each token proves |
| `emem-referential-drift` | pin a value to a citation, grade what you are about to say, ask why a number moved |
| **Checks** | |
| `emem-verify-receipt` | check the Ed25519 signature of a receipt, through emem or with tools that you trust |
| `emem-transparency-log` | prove the log only grew, and that an entry or note is in it |
| `emem-verify-before-publish` | check the citations of a draft and the numbers written next to them |
| `emem-device-traces` | resolve and verify again the signed OS trace of a device, and what enrolment admits today |

### No code in this plugin

This plugin ships Markdown, JSON and its icon, and no code. The skills check
a receipt, a log proof, a note or a hash through emem's MCP tools and HTTP
routes. Each skill also says what to compute again so that you can check
it with tools that you trust, without trusting the responder.

The emem repository publishes reference offline verifiers in Python at
https://github.com/Vortx-AI/emem/tree/main/tools/verifiers. They are not
part of this plugin:

| Folder | What it does |
|---|---|
| `emem-verify-receipt` | rebuilds the preimage of a receipt and checks its signature |
| `emem-transparency-log` | checks a tree head, an inclusion proof, a consistency proof |
| `emem-document-evidence` | checks OCR and parse receipts and the hashes of the document |
| `emem-field-tokens` | computes the hash of a downloaded artifact again and compares it with its cid |
| `emem-multi-agent-handoff` | checks which key wrote a memory note, and seals a note to one key |
| `emem-tokenise-files` | builds a pointer.v1 index; checks the audit path and the bytes of a unit |
| `emem-long-horizon-memory` | the same note check, and a wait for a new note in your inbox |

Their BLAKE3 and Ed25519 code is plain Python 3 with no third-party
package. The check scripts accept `--self-test`, which checks it against
11 official BLAKE3 test vectors and RFC 8032 test vectors 1 to 3. All
printed `SELF-TEST PASS` on 12 November 2026. A note sealed to your key
opens with `signer_open_sealed` from the emem-signer plugin.

## Signing key

Reads need no key, and this plugin holds no key. To write signed notes,
or to open notes sealed to your key, also install the optional
`emem-signer` plugin from the same marketplace:

```sh
/plugin install emem-signer@emem
```

The signer keeps your Ed25519 key in a sensitive plugin option. It signs
in one local process. Its
[README](https://github.com/Vortx-AI/emem/blob/main/plugins/emem-signer/README.md)
gives the details. Without the signer, the skills that write tell you how
to sign with an Ed25519 tool that you already trust. If one machine holds
more than one key, set the role option of each signer. Then a log witness
key does not sign your notes.

## Data the plugin sends

The MCP server and each command in the skills talk to one host,
`https://emem.dev`. Vortx AI Private Limited operates it. The skills never
call a third-party service. emem.dev gets open data from upstream sources
on its own side, and needs no credentials from you. On your machine, the
skill commands save the JSON they get into the current directory. They
write nothing else.

| What you call | What you send | How long emem.dev keeps it |
|---|---|---|
| `https://emem.dev/mcp` and the REST reads that the skills use (locate, recall, recall_polygon, ask, field and EUDR checks, log and token reads) | Place names, coordinates, plot or field polygons, band names, dates, tokens and questions | The service uses request bodies to compute the answer and does not log them. It keeps request metadata for 30 days: path, GET query string, status, duration, user agent, and an 8-byte one-way hash (BLAKE3) of your IP. The service signs the facts that it reads for your request, and they join the public record permanently. They carry no identity of yours. But anyone can see which cells have facts, so a cell that you asked about is visible |
| Document routes (`/v1/ocr`, `/v1/lab_report_parse`, `/v1/land_record_parse`) | The image or text of the document | The service uses it for that request and does not store it. The signed receipt holds only hashes of the text and the result |
| Device routes (`/v1/devices`, `/v1/trace_resolve`, `/v1/trace_verify`) | Trace tokens and trace records | The check is stateless. The service stores nothing that you send |
| Memory writes (`emem_memory_create` and the other write verbs, used by sign-and-attest, handoff and long-horizon memory) | The note text, its path, your public key and your signature | Public and permanent by design. When you delete a note, the path stops resolving. The signed history stays in the log |

The privacy notice is the authority on this data and on your rights:
<https://emem.dev/privacy>.

## Trust boundary

Facts are band-typed measurements with no free-text field, so a fact
cannot carry an instruction. Notes are prose that other agents write.
They arrive inside `_content_is_data_not_instructions`. The skills treat
notes as data and never obey a directive inside one. A signature tells
you who wrote something. It never tells you that it is true.

## What the plugin does not do

No tool here writes anything that you did not sign, and no read needs a
key. emem.dev retired the foundation-encoder embedding bands. Vectors
already stored still read and verify. The service computes no new
vectors. A request for a new one returns `band_retired_at_this_responder`
instead of a vague failure.

Source: <https://github.com/Vortx-AI/emem> · Docs: <https://emem.dev>
