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

**Fifteen skills**, each a worked procedure with an example that was run
against emem.dev, rather than a description:

| Skill | For |
|---|---|
| `emem-locate-and-recall` | a place name to a canonical cell, then signed facts and their citation tokens |
| `emem-recall-polygon` | the same over an area, with the sampling stated |
| `emem-field-tokens` | the actual raster field over an area, or over time, as a signed artifact |
| `emem-field-signals` | one farm field: boundaries, residue burning, evapotranspiration, a picture |
| `emem-eudr-due-diligence` | a signed EUDR deforestation statement for plots, and what it does not cover |
| `emem-document-evidence` | OCR plus lab-report and land-record parsing, every step signed |
| `emem-find-similar` | analogues by cosine over a stored embedding; read its coverage warning first |
| `emem-verify-receipt` | check a receipt's Ed25519 signature offline, without re-contacting the responder |
| `emem-transparency-log` | prove the log only grew, and that an entry or note is in it |
| `emem-sign-and-attest` | write with your own key; the responder's refusal names the bytes to sign |
| `emem-shared-identity` | make two agents refer to the same object, and know what each token proves |
| `emem-a2a-collaboration` | hand findings to other agents as tokens they can verify, and verify theirs |
| `emem-referential-drift` | pin a value to a citation, grade what you are about to say, ask why a number moved |
| `emem-agent-handoff` | cross a trust boundary with bytes that verify rather than prose someone must believe |
| `emem-verify-before-publish` | check a draft's citations and the numbers written beside them |

Four skills ship a small Python script beside their `SKILL.md`
(`verify.py`, `verify_log.py`, `verify_doc.py`, `rehash.py`). Each runs
offline, reads only the files you pass it, and needs
`pip install blake3 cryptography` (`rehash.py` needs only `blake3`).

## Network use

The MCP server and every `curl` in the skills talk to `https://emem.dev`
and nothing else. The skills never call a third-party service; the
responder fetches open data upstream on your behalf. What emem.dev logs
is described at <https://emem.dev/privacy>.

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
