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

**The MCP server** (`.mcp.json`) — `https://emem.dev/mcp`, streamable
HTTP, no auth. One `tools/list` returns the whole 18-tool core loop in a
single page; `emem_tools` maps the rest, and `tools/call` dispatches any
tool by name.

**Eleven skills**, each a worked procedure rather than a description. Five are about the protocol, six about the Earth data it carries:

| Skill | For |
|---|---|
| `emem-locate-and-recall` | a place name to a canonical cell, then signed facts at it |
| `emem-recall-polygon` | the same over an extent rather than a point |
| `emem-field-tokens` | the actual raster field over an area, or over time, as a signed artifact |
| `emem-find-similar` | analogues by cosine over a stored embedding — read its coverage warning first |
| `emem-verify-receipt` | check a receipt's ed25519 signature offline, without re-contacting the responder |
| `emem-sign-and-attest` | write with your own key; the responder teaches you the bytes to sign |
| `emem-shared-identity` | make two agents refer to the same object, and know what each token proves |
| `emem-a2a-collaboration` | hand findings to other agents as tokens they can verify |
| `emem-referential-drift` | pin a value to a citation, grade what you are about to say, ask why a number moved |
| `emem-agent-handoff` | cross a trust boundary with bytes that verify rather than prose someone must believe |
| `emem-verify-before-publish` | resolve every citation in a draft and check it supports the sentence around it |

## What it does not do

No tool here writes anything you have not signed, and no read needs a
key. The four foundation-encoder bands are retired on emem.dev: stored
vectors still read, new ones are not computed, and a request for one
answers `band_retired_at_this_responder` rather than failing vaguely.

Source: <https://github.com/Vortx-AI/emem> · Docs: <https://emem.dev>
