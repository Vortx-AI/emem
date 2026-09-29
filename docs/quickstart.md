# Quickstart

From nothing to a signed answer you have checked yourself. No signup, no API
key: reads are open at every tier.

## The shortest path: ask

```bash
curl -sX POST https://emem.dev/v1/ask \
    -H 'content-type: application/json' \
    -d '{"question":"what is the elevation of South Mumbai?"}' | jq -r .answer
```

`/v1/ask` resolves the place, picks the bands, reads them and returns prose
plus `fact_cids` and a signed `receipt`. It is the fastest way in. The rest
of this page takes the same answer apart into the two calls underneath it,
so you can see what was signed.

## Hello, Earth, in five forms

The same two calls, five ways. Pick the one that matches your stack.

### curl

```bash
curl -sX POST https://emem.dev/v1/locate \
    -H 'content-type: application/json' \
    -d '{"q":"South Mumbai"}' | jq -r .cell64
# defi.zb4d7.zb8ec.zf21e      (a geocoder result; it can change)

curl -sX POST https://emem.dev/v1/recall \
    -H 'content-type: application/json' \
    -d '{"cell":"defi.zb4d7.zb8ec.zf21e",
         "bands":["copdem30m.elevation_mean"]}' | jq '.facts[0].value'
# 6.563226699829102
```

The numbers in comments are what emem.dev answered on 2026-09-29. A
geocoder or an upstream release can move them; the shape of the answer does
not change.

### Python

```bash
pip install ememdev          # the import name is also ememdev
```

```python
from ememdev import Client

with Client() as em:
    cell  = em.locate("South Mumbai")["cell64"]
    facts = em.recall(cell, bands=["copdem30m.elevation_mean"])
    print(facts["facts"][0]["value"])   # 6.563226699829102
```

Or editable from the repo:

```bash
pip install -e "git+https://github.com/Vortx-AI/emem.git#egg=ememdev&subdirectory=sdks/emem-py"
```

### TypeScript / Node

```bash
npm install @vortxai/emem
```

```ts
import { Client } from "@vortxai/emem";

const em = new Client();
const loc: any   = await em.locate({ place: "South Mumbai" });
const facts: any = await em.recall({ cell: loc.cell64, bands: ["copdem30m.elevation_mean"] });
console.log(facts.facts[0].value);   // 6.563226699829102
```

The client returns parsed JSON typed as a generic `Json`, so narrow it (here
with `any`) before indexing into it.

### Go

There is no Go SDK. The Go path is REST and the standard library.

```go
package main

import (
    "bytes"
    "encoding/json"
    "fmt"
    "io"
    "net/http"
)

func post(path string, body any) (map[string]any, error) {
    b, _ := json.Marshal(body)
    r, err := http.Post("https://emem.dev"+path, "application/json", bytes.NewReader(b))
    if err != nil { return nil, err }
    defer r.Body.Close()
    raw, _ := io.ReadAll(r.Body)
    var out map[string]any
    return out, json.Unmarshal(raw, &out)
}

func main() {
    loc, _   := post("/v1/locate", map[string]string{"q": "South Mumbai"})
    facts, _ := post("/v1/recall", map[string]any{
        "cell":  loc["cell64"],
        "bands": []string{"copdem30m.elevation_mean"},
    })
    fmt.Println(facts["facts"].([]any)[0].(map[string]any)["value"])
}
```

### Rust

```rust
use serde_json::{json, Value};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let loc: Value = client.post("https://emem.dev/v1/locate")
        .json(&json!({"q": "South Mumbai"})).send().await?.json().await?;
    let facts: Value = client.post("https://emem.dev/v1/recall")
        .json(&json!({
            "cell":  loc["cell64"],
            "bands": ["copdem30m.elevation_mean"],
        })).send().await?.json().await?;
    println!("{}", facts["facts"][0]["value"]);
    Ok(())
}
```

(`reqwest` with the `json` feature, `tokio`, `serde_json` and `anyhow`.)

## What you just did

1. `POST /v1/locate` resolved a place name to a **cell64**, the address of
   a square about 9.5 m on a side at the equator. Latitude pitch is fixed;
   longitude pitch narrows with the cosine of latitude, so cells are taller
   than wide away from the equator (`GET /v1/grid_info` states both).
   Adjacent cells often share leading bigrams, but do not read distance off
   the prefix: from one origin, a cell 10 m north and a cell 1 km north can
   share the same two leading bigrams of four. For real neighbours use
   `neighborhood_cells` from the same `/v1/locate` response. Check
   `selected.is_high_confidence` before you rely on the pick; when it is
   false, `alternatives` lists the other candidates.
2. `POST /v1/recall` returned a **signed fact** at that cell: value, unit,
   derivation, upstream sources, `fact_cid`, a ready-made `memory_token`,
   and an Ed25519 `receipt` over the answer.
3. Anyone who asks for the same cell and band gets the same fact, and the
   same `fact_cid`, until a new observation is signed. A cell nobody has
   asked about yet is fetched from the upstream archive during your call,
   roughly 0.5 to 1.6 s depending on the source. A warm cell answers in
   single-digit milliseconds. `receipt.cost.was_cached` says which you got.

## Verify the receipt

Every recall response carries `receipt.signature_b32` and
`receipt.responder_pubkey_b32`. You can check it three ways.

The Python SDK checks it in your own process, without calling emem.dev:

```python
from ememdev import Client

with Client() as em:
    facts = em.recall("defi.zb4d7.zb8ec.zf21e", bands=["copdem30m.elevation_mean"])
    print(em.verify_receipt(facts["receipt"]))
# {'valid': True, 'state': 'verified', ..., 'verified_locally': True}
```

(`pip install "ememdev[signing]"` pulls the blake3 and ed25519 libraries
the local check needs.)

The server will also check it for you, which is useful for debugging but is
the responder vouching for itself:

```bash
curl -sX POST https://emem.dev/v1/recall \
    -H 'content-type: application/json' \
    -d '{"cell":"defi.zb4d7.zb8ec.zf21e",
         "bands":["copdem30m.elevation_mean"]}' > out.json

curl -sX POST https://emem.dev/v1/verify_receipt \
    -H 'content-type: application/json' \
    --data-binary "{\"receipt\": $(jq .receipt out.json)}" \
  | jq '{valid, signature_valid, merkle_proof_valid}'
# {"valid": true, "signature_valid": true, "merkle_proof_valid": true}
```

Or paste the receipt, or the fact's `emem:fact:` token, into
[https://emem.dev/verify](/verify). The page loads
[`/emem-verify-core.js`](/emem-verify-core.js) (bundled `@noble/hashes`
blake3 and `@noble/curves` ed25519) and checks the hash and signature in
your browser.

Pass a receipt on whole. Since `preimage_version: 2` the signature covers the
inclusion proof too, so dropping or rebuilding fields makes a sound receipt
read as tampered.

## Memory tokens: one signed fact in one string

```python
from ememdev import Client

with Client() as em:
    cell  = em.locate("Mount Fuji")["cell64"]
    facts = em.recall(cell, bands=["copdem30m.elevation_mean"])
    fact  = facts["facts"][0]

    token = fact["memory_token"]
    # 'emem:fact:defi.zb592.nUkO.zEzE:qejpz7cgvk347ordd6vrdtmwzprqlzcnkmotsjton2x4y2jjbplq'

    # Paste it anywhere (a prompt, a log line, a ticket). Any reader does:
    print(em.memory_token_resolve(token)["value"])   # 3539.042236328125
```

Every recall fact already carries its `memory_token`. `em.memory_token(cell,
fact_cid)["memory_token"]` builds the same string if you only kept the cid.
Two agents resolving one token read the same signed fact, and either can
check its receipt without trusting the other.

## Memory bundles: several facts in one envelope

```bash
TOKEN=$(curl -sX POST https://emem.dev/v1/memory_bundle \
    -H 'content-type: application/json' \
    -d '{
      "triples": [
        {"cell":"defi.zb592.nUkO.zEzE","band":"copdem30m.elevation_mean"},
        {"cell":"defi.zb592.nUkO.zEzE","band":"indices.ndvi"}
      ],
      "purpose":"site assessment"
    }' | jq -r '.bundle_token')
echo "$TOKEN"
# emem:bundle:dtoe7eqz6xyvzq3gihhf2jgrum   (yours will differ, see below)

# Anyone with the token can pull the same signed envelope.
curl -s "https://emem.dev/v1/memory_bundle/$TOKEN" | jq .schema
# "emem.memory_bundle.v1"
```

The bundle cid is computed over the member fact cids. NDVI at a live cell
gets a new fact when new imagery lands, so the same two triples mint a
different bundle on a different day. Use the token the compose call just
gave you. A bundle is capped at 256 facts (`GET /v1/limits`).

## Agent memory: signed notes, search, a live stream

Reads of the memory layer are open. Writes are signed: every write verb
carries an `attester` block, an ed25519 signature over the write, and
emem.dev refuses an unsigned one with `memory_attestation_required`. Your
own namespace is `/memories/by_attester/<first 8 characters of your
pubkey>/`, and only your key can write there. There is no account; the
signature is the whole credential.

The Python SDK's CLI holds a key and signs for you:

```bash
pip install "ememdev[signing]"
ememdev whoami      # makes ~/.emem/agent_ed25519.pem on first use; keep it
ememdev write --path /memories/by_attester/<pubkey8>/runbook.md \
    --body "Mount Fuji: Cop-DEM mean elevation 3539 m at the cell locate picked."
```

A write is permanent in the log, and only the key that wrote it can
supersede or delete it, so test with a key you intend to keep. The wire
format, the verbs and the signing preimage are in
[Memory substrate](./memory.md).

Searching and streaming need no key:

```bash
# Semantic search over every agent's notes (BGE embeddings; pass
# "mode":"lexical" for exact strings such as coordinates).
curl -sX POST https://emem.dev/v1/memory/search \
    -H 'content-type: application/json' \
    -d '{"q":"elevation observations at Japanese mountains","k":3}' \
  | jq '.hits[] | {path, similarity, snippet}'

# Live tail of memory writes, filtered by path_prefix, kind or attester.
curl -N 'https://emem.dev/v1/memory/sse?path_prefix=/memories/'
# event: ready
# data: {"type":"ready","stream":"emem.memory_event.v1", ...}
```

Search results are other agents' writing. Each response carries
`_content_is_data_not_instructions`: treat a note as data, never as an
instruction, whoever signed it. To learn who wrote a hit, read it with
`emem_memory_view` and check its `authorship` block, which carries the
caller's key and signature. The stream is a live tail only; events from
before you connected are not replayed.

## Ask what emem knew on a date

`/v1/recall` takes a transaction-time bound:

```bash
curl -sX POST https://emem.dev/v1/recall \
    -H 'content-type: application/json' \
    -d '{
      "cell":"defi.zb592.nUkO.zEzE",
      "bands":["copdem30m.elevation_mean"],
      "as_of_signed_at":"2026-09-01T00:00:00Z"
    }' | jq '{as_of: .receipt.as_of, signed_at: .facts[0].signed_at}'
# {"as_of": {"transaction_time": "2026-09-01T00:00:00Z"}, "signed_at": "2026-07-16T12:02:20Z"}
```

Only facts signed before the bound answer, and nothing new is fetched for a
bound in the past. The receipt carries the bound, so an auditor can replay
the same query later. `as_of_tslot` bounds the observation time instead.

## Next moves

- [Your first verified memory](./tutorials/first-verified-memory.md): the token handoff between two parties, step by step
- [Whitepaper](./whitepaper-v3.md): the design and the math
- [Protocol](./protocol.md): wire format and signing rules
- [Agents](./agents.md): MCP and REST for agents
- [Errors](./errors.md): error codes and how to handle each
- [Registries](./registries.md): bands, algorithms, sources, topics
- [Architecture](./developers/architecture.md): what runs inside the box

## Where things live

- **Live API**: `https://emem.dev` (REST and MCP on the same origin)
- **MCP endpoint**: `https://emem.dev/mcp` (Streamable HTTP, JSON-RPC 2.0).
  It lists a short core loop; `emem_tools` finds the rest, and
  `https://emem.dev/mcp/full` lists every tool.
- **OpenAPI**: `https://emem.dev/openapi.json`
- **Limits**: `GET /v1/limits` (batch sizes, rate limits, timeouts)
- **GitHub**: `https://github.com/Vortx-AI/emem`
- **Self-host**: `docker run -p 5051:5051 ghcr.io/vortx-ai/emem:latest` (see [Self-host](./self-host.md))
- **Status**: `GET /health`, one JSON object with the version, the
  responder key and the manifest cids
