# emem skills

Composed recipes for AI agents building on emem, a shared, verifiable
memory for AI agents: a citeable identity layer that stops referential
drift, so different models reason from the same world object instead of
divergent descriptions, grounded in signed Earth observation. Each skill
is a small workflow that combines two or three `/v1/*` calls into a
useful primitive: locate-and-recall, find-similar-and-verify,
recall-polygon-and-solve, and so on.

This page is the cookbook view. The same skills also ship as an
installable bundle at [claude-skills/](https://github.com/Vortx-AI/emem/tree/main/claude-skills)
for Claude Code users; see § Installing as Claude Skills below.

The endpoint is `https://emem.dev` (or your self-host URL). Reads need
no auth. Every response carries an Ed25519 receipt signed over a
deterministic preimage; verify it offline with the responder's pubkey
from `/.well-known/emem.json`.

Every fact is content-addressed and signed, so any conformant responder
returns byte-identical bytes for the same content id and any client
verifies the receipt offline. emem is a protocol, not a single endpoint.

## Quick reference

| Skill                              | Calls                                                                |
|------------------------------------|----------------------------------------------------------------------|
| **locate-and-recall**              | `POST /v1/locate` → `POST /v1/recall`                                |
| **verify-receipt-offline**         | (any receipt) → BLAKE3 + Ed25519 in-process                          |
| **find-similar-places**            | `POST /v1/locate` → `POST /v1/find_similar`                          |
| **scene-thumbnail**                | `POST /v1/locate` → `GET  /v1/cells/{cell64}/scene.png`              |
| **recall-many-cells**              | `POST /v1/locate` (×N) → `POST /v1/recall_many`                      |
| **lasso-polygon-recall**           | (polygon) → `POST /v1/recall_polygon`                                |
| **trajectory-over-time**           | `POST /v1/locate` → `POST /v1/trajectory`                            |
| **compose-flood-risk**             | `POST /v1/recall_polygon` (DEM + climate) → `POST /v1/heat_solve`    |

---

## 1. `locate-and-recall`: name a place, get signed facts

The most common flow. Resolve a place name to a `cell64`, then recall
one or more bands at that cell. The recall response carries a Receipt;
the `fact_cids[0]` is the durable handle that re-fetches the same
bytes from the responder that signed them, in any year. A fact_cid
digests the signer and the signing moment along with the value, so it
names one attestation rather than the observation behind it: a second
responder measuring the same thing mints a different cid. Use
`emem:entity:` for identity that crosses responders.

### curl

```sh
BASE=https://emem.dev

# 1. Place name → cell64.
CELL=$(curl -sf -X POST $BASE/v1/locate \
  -H 'content-type: application/json' \
  -d '{"q":"Bengaluru, India"}' | jq -r '.cell64')

# 2. Recall current 2 m air temperature at that cell.
curl -sf -X POST $BASE/v1/recall \
  -H 'content-type: application/json' \
  -d "{\"cell\":\"$CELL\",\"bands\":[\"weather.temperature_2m\"]}" \
  | jq '.facts[0] | {band, value, unit, signed_at}, .receipt.fact_cids[0]'
```

### python

```py
import httpx
BASE = "https://emem.dev"
loc = httpx.post(f"{BASE}/v1/locate", json={"q": "Bengaluru, India"}).json()
rec = httpx.post(f"{BASE}/v1/recall", json={
    "cell": loc["cell64"],
    "bands": ["weather.temperature_2m"],
}).json()
fact = rec["facts"][0]
print(fact["band"], "=", fact["value"], fact.get("unit",""))
print("fact_cid:", rec["receipt"]["fact_cids"][0])
```

### MCP (JSON-RPC over `POST /mcp`)

```json
{ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
  "params": { "name": "emem_locate", "arguments": {"q": "Bengaluru, India"} } }
```

```json
{ "jsonrpc": "2.0", "id": 2, "method": "tools/call",
  "params": { "name": "emem_recall",
              "arguments": {"cell": "defi.zb493.xuqA.zcb5f",
                            "bands": ["weather.temperature_2m"]} } }
```

### When to use

- The agent's user gave a free-form place name and you need facts.
- The cell64 isn't known in advance.
- You want a receipt the user can verify later.

### Common pitfalls

- `/v1/locate` accepts free-form strings; results carry a `via` field
  showing whether they came from cache, embedded list, Photon, or
  Nominatim. Treat `via=fallback` as low-confidence.
- A cold cell with `EMEM_AUTO_MATERIALIZE=1` (the default on `emem.dev`)
  triggers a fetch from the upstream connector. First call may be
  slow (seconds); the second is cache-warm.

---

## 2. `verify-receipt-offline`: BLAKE3 + Ed25519, no callback

Every receipt verifies without re-contacting the responder. Rebuild the
canonical preimage, BLAKE3 it, then `ed25519.verify(signature, digest,
pubkey)`. The pubkey is at `/.well-known/emem.json`. The byte layout
is defined at `crates/emem-storage/src/server.rs::sign_receipt`.

### Preimage (v1, current receipts)

Receipts carry `preimage_version: 1`. The digest is blake3 over a
domain-separated stream of tagged, length-prefixed segments:

```
blake3(
    "emem.preimage.v1\x00" || len("receipt") || "receipt"
    || seg(0x01, request_id) || seg(0x02, served_at)
    || [seg(0x03, scope_hex)] || [seg(0x04, as_of_hex)]
    || [seg(0x05, edges_hex)] || [seg(0x06, manifest_hex)]
    || seg(0x07, primitive)
    || seg_list(0x08, cells) || seg_list(0x09, fact_cids)
)
```

`seg(tag, bytes)` = `tag || u32-LE length || bytes`; `seg_list` adds a
u32-LE count. Bracketed segments appear only when the receipt body has
the matching field (`manifest_hex` = lowercase blake3 hex of the
canonical-CBOR of a non-empty `source_versions`). The byte layout lives
in `crates/emem-attest` (`receipt_preimage_v1`); `POST /v1/verify_receipt`
is the server-side reference check. Receipts with no `preimage_version`
field use the legacy rule instead: pipes between sections, commas after
each list element (including the last), no leading or trailing
whitespace.

### python (fully offline, ~35 lines)

```py
import httpx, cbor2
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
from blake3 import blake3

BASE = "https://emem.dev"
pubkey_b32 = httpx.get(f"{BASE}/.well-known/emem.json").json()["responder"]["pubkey_b32"]

def b32decode(s):
    table = "abcdefghijklmnopqrstuvwxyz234567"
    bits = "".join(format(table.index(c), "05b") for c in s.lower())
    return bytes(int(bits[i:i+8], 2) for i in range(0, len(bits) - len(bits)%8, 8))

def verify(receipt):
    assert receipt.get("preimage_version") == 1, "v1 rule; legacy receipts use the concatenation rule"
    h = blake3()
    h.update(b"emem.preimage.v1\x00")
    h.update(len(b"receipt").to_bytes(4, "little")); h.update(b"receipt")
    def seg(tag, data):
        h.update(bytes([tag])); h.update(len(data).to_bytes(4, "little")); h.update(data)
    def seg_list(tag, items):
        h.update(bytes([tag]))
        body = b"".join(len(i.encode()).to_bytes(4, "little") + i.encode() for i in items)
        h.update(len(items).to_bytes(4, "little")); h.update(body)
    seg(0x01, receipt["request_id"].encode())
    seg(0x02, receipt["served_at"].encode())
    if receipt.get("source_versions"):
        mh = blake3(cbor2.dumps(dict(sorted(receipt["source_versions"].items())))).hexdigest()
        seg(0x06, mh.encode())
    seg(0x07, receipt["primitive"].encode())
    seg_list(0x08, receipt.get("cells", []))
    seg_list(0x09, receipt.get("fact_cids", []))
    sig = bytes(receipt["signature"])
    Ed25519PublicKey.from_public_bytes(b32decode(pubkey_b32)).verify(sig, h.digest())
    return True
```

### Browser (what `/humans` does)

`https://emem.dev/humans` imports `@noble/curves@1.6.0/ed25519` and
`@noble/hashes@1.5.0/blake3` from `esm.sh` (CSP allows the host) and
verifies every visible receipt locally. Click any star on the page to
see the verifier run inline.

### When to use

- You don't trust the responder you fetched from.
- You're caching receipts and want to prove they're authentic later.
- You're an LLM and your user wants a non-repudiable answer.

---

## 3. `find-similar-places`: given a place, return neighbours by embedding

`/v1/find_similar` runs cosine similarity over the 128-D Tessera
embedding (`geotessera`, 2024 vintage by default) and returns top-K
neighbours with their cell64s, lat/lng, place labels (cached), and
scores in `[0, 1]`. Cells without an attested geotessera vector
return a structured `cid_not_found` 404; call `/v1/recall` with
`bands:["geotessera"]` first to materialise it on the responder.

### curl (with auto-materialise)

```sh
BASE=https://emem.dev
CELL=$(curl -sf -X POST $BASE/v1/locate -H 'content-type: application/json' \
  -d '{"q":"Hyderabad, India"}' | jq -r '.cell64')

# Materialise the embedding (idempotent if already attested).
curl -sf -X POST $BASE/v1/recall -H 'content-type: application/json' \
  -d "{\"cell\":\"$CELL\",\"bands\":[\"geotessera\"]}" > /dev/null

# Find similar.
curl -sf -X POST $BASE/v1/find_similar -H 'content-type: application/json' \
  -d "{\"key\":\"$CELL\",\"k\":12}" \
  | jq '.neighbors[] | {cell, score, place: .place_label_cached}'
```

### When to use

- "Find me places that look like Bengaluru."
- "Are there cities globally with the same urban canopy index as Mumbai?"
- Discovery / exploration of the corpus.

### Pitfalls

- Cosine over Tessera reflects *land-cover archetype*: vegetation
  density, urban density, water proximity. It doesn't model
  socio-economic features. A "similar" place may be visually similar
  but socially different.
- Default vintage is 2024. To query a specific year, use
  `bands:["geotessera.2020"]` etc.; the responder ships 2017–2024.

---

## 4. `scene-thumbnail`: visual preview for a cell

`GET /v1/cells/{cell64}/scene.png` returns a small RGB raster
synthesised from Sentinel-2 bands at the cell. Useful for grounding
an agent's spatial answer with a visible thumbnail.

```sh
BASE=https://emem.dev
CELL=$(curl -sf -X POST $BASE/v1/locate -H 'content-type: application/json' \
  -d '{"q":"Sundarbans"}' | jq -r '.cell64')
curl -sf "$BASE/v1/cells/$CELL/scene.png" -o scene.png
```

### When to use

- Embedding a thumbnail in an agent's reply.
- Sanity-checking a cell64 visually before issuing further calls.

---

## 5. `recall-many-cells`: batch a recall across N places

`POST /v1/recall_many` takes a list of cell64s and a list of bands,
returns per-cell facts + per-cell receipts. Cap is 256 cells per
call. Each cell carries its own signed receipt; verifying the bulk
call means verifying each cell's receipt independently.

```sh
BASE=https://emem.dev
curl -sf -X POST $BASE/v1/recall_many -H 'content-type: application/json' \
  -d '{
    "cells": ["defi.zb493.xuqA.zcb5f", "defi.zb5cf.nura.zd83c",
              "defi.zb0ff.bdne.zb73e"],
    "bands": ["weather.temperature_2m", "indices.ndvi"]
  }' | jq '.by_cell | to_entries[] | {cell: .key, facts: (.value.facts|length)}'
```

### When to use

- Comparing a single band across a list of named places.
- Rendering a small leaderboard for an agent's reply.
- Building a candidate pool for `/v1/find_similar`.

---

## 6. `lasso-polygon-recall`: recall everything inside a polygon

Drag-select on the `/humans` constellation and the same payload that
the page sends to `POST /v1/recall_polygon` works programmatically.
Returns `cells_sampled`, per-cell facts, and an `area_km2` summary.

```sh
BASE=https://emem.dev
curl -sf -X POST $BASE/v1/recall_polygon -H 'content-type: application/json' \
  -d '{
    "polygon": [[77.55,12.95],[77.65,12.95],[77.65,13.05],[77.55,13.05],[77.55,12.95]],
    "bands": ["indices.ndvi"]
  }' | jq '{cells: (.by_cell|length), facts: (.by_cell|to_entries|map(.value.facts|length)|add)}'
```

### When to use

- "What's the average NDVI inside this watershed?"
- Exploratory queries over a region rather than a point.

---

## 7. `trajectory-over-time`: time series at one cell

`POST /v1/trajectory` returns a per-tslot fact list at a single cell
across a `[from, to]` window. Useful for plotting change over time.

```sh
BASE=https://emem.dev
CELL=$(curl -sf -X POST $BASE/v1/locate -H 'content-type: application/json' \
  -d '{"q":"Lake Chad"}' | jq -r '.cell64')

curl -sf -X POST $BASE/v1/trajectory -H 'content-type: application/json' \
  -d "{\"cell\":\"$CELL\",\"band\":\"surface_water.recurrence\",\"from\":1640995200,\"to\":1735689599}" \
  | jq '.points | length'
```

### When to use

- Plotting deforestation, lake-level change, drought.
- Computing trends or first-difference time series.

---

## 8. `compose-flood-risk`: recall_polygon + heat_solve

Compose two primitives: pull the elevation + climate stack inside a
polygon, then run the 2-D heat solver to estimate diffusion of a
heat anomaly across the region. Real example used by `/humans`
agent demos.

```sh
BASE=https://emem.dev
curl -sf -X POST $BASE/v1/heat_solve -H 'content-type: application/json' \
  -d '{
    "cell": "defi.zb493.xuqA.zcb5f",
    "horizon_hours": 24,
    "step_seconds": 3600
  }' | jq '{n_steps, hours_ahead, forecast_k, forecast_unit}' (and either add the recall_polygon step the title promises, or reword the intro at line 312 to say the solver recalls its own LST stencil and the receipt carries the input band CIDs)
```

The solver carries a receipt warning chain: `frozen_pretrained_encoder`
where applicable, plus the underlying band CIDs.

---

## Installing as Claude Skills

The same flows ship as installable Anthropic Skills (with bundled
scripts, auto-triggering, frontmatter metadata) at
[`claude-skills/`](https://github.com/Vortx-AI/emem/tree/main/claude-skills)
in this repo. To install on a Claude Code workstation:

```sh
# Clone the repo
git clone https://github.com/Vortx-AI/emem.git
# Copy the skill bundle into your Claude Code project's .claude/skills/
mkdir -p .claude/skills
cp -r emem/claude-skills/emem-* .claude/skills/
```

The skills auto-trigger when a user asks about Earth-data lookup,
verification, or similarity in a Claude Code session. See each
`SKILL.md` for the description that gates auto-loading.

## Discovery surface

- `https://emem.dev/llms.txt`: high-level summary + behavioural rules
- `https://emem.dev/openapi.json`: full machine surface (every documented REST path under /v1/*)
- `https://emem.dev/.well-known/emem.json`: manifest CIDs + responder pubkey
- `https://emem.dev/v1/agent_card`: discover-first card with band taxonomy
- `https://emem.dev/agents.md`: consumer-agent ontology + recipes
- `https://emem.dev/humans`: interactive console where every `/v1/*`
  call prints in a live log pane
- `https://emem.dev/mcp`: JSON-RPC 2.0 MCP endpoint. `tools/list` advertises
  the loop that the rest of the surface serves: ground a place, cite the fact
  as `emem:fact:<cell64>:<fact_cid>`, hand that line to another agent, and let
  them resolve it to the identical signed bytes and check the receipt without
  trusting you. 92 tools in total (14 core, 78 extended);
  `https://emem.dev/mcp/full` advertises all 92, and `tools/call` reaches every
  tool by name from either, so the narrower list costs no capability. Call
  `emem_tools` to map the surface, filter it by the shape of the answer you
  need (`{"shape":"raster"}`) or by job (`{"bundle":"robotics"}`), or fetch one
  tool's schema (`{"name":"emem_ndvi"}`).

## License

Apache-2.0. All emem responses are content-addressed and verifiable;
copy them, sign your own derivatives, build whatever you want.
