# Quickstart

Sixty seconds from cold to a signed answer. No signup, no API key.

## Hello, Earth — in five copy-pasteable forms

The same call, five ways. Pick the one that matches your stack.

### curl

```bash
curl -sX POST https://emem.dev/v1/locate \
    -H 'content-type: application/json' \
    -d '{"q":"South Mumbai"}' | jq .cell64
# "defi.zb4d9.pefa.zf619"

curl -sX POST https://emem.dev/v1/recall \
    -H 'content-type: application/json' \
    -d '{"cell":"defi.zb4d9.pefa.zf619",
         "bands":["copdem30m.elevation_mean"]}' | jq '.facts[0].value'
# 6.0
```

### Python

```python
from emem import Client

with Client() as em:
    cell  = em.locate("South Mumbai")["cell64"]
    facts = em.recall(cell, bands=["copdem30m.elevation_mean"])
    print(facts["facts"][0]["value"])   # 6.0
```

Install from the repo while the PyPI release is in flight:

```bash
pip install -e "git+https://github.com/Vortx-AI/emem.git#egg=emem&subdirectory=sdks/emem-py"
```

### TypeScript / Node

```ts
import { Client } from "@emem/client";

const em    = new Client();
const loc   = await em.locate({ place: "South Mumbai" });
const facts = await em.recall({ cell: loc.cell64, bands: ["copdem30m.elevation_mean"] });
console.log(facts.facts[0].value);   // 6.0
```

Until the npm release lands, install from the repo:

```bash
git clone https://github.com/Vortx-AI/emem.git
cd emem/sdks/emem-ts && npm install && npm run build
```

### Go

The Go path is REST + your favourite HTTP client. No SDK yet.

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
    fmt.Println(facts["facts"].([]any)[0].(map[string]any)["value"])  // 6
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
    println!("{}", facts["facts"][0]["value"]);  // 6.0
    Ok(())
}
```

## What you just did

1. `POST /v1/locate` resolved a place name to a **cell64** — a 64-bit
   address for a ~9.55 m × 9.55 m square on WGS-84. Hilbert-ordered, so
   neighbours share string prefixes.
2. `POST /v1/recall` returned a **signed fact** at that cell: value,
   unit, upstream provenance, fact CID, and an Ed25519 receipt.
3. The same call from anywhere on the planet now returns the same bytes
   (CID is deterministic). First call ≈ 180 ms (one upstream fetch),
   every call after ≈ 10 ms (warm cache).

## Verify the receipt offline

Every recall response carries `receipt.signature` and
`receipt.responder_pubkey_b32`. Recompute the signature without calling
back to emem.dev:

```bash
# Get a receipt
curl -sX POST https://emem.dev/v1/recall \
    -H 'content-type: application/json' \
    -d '{"cell":"defi.zb4d9.pefa.zf619",
         "bands":["copdem30m.elevation_mean"]}' > out.json

# Verify it (server endpoint, but the math is reproducible client-side)
curl -sX POST https://emem.dev/v1/verify_receipt \
    -H 'content-type: application/json' \
    --data-binary "{\"receipt\": $(jq .receipt out.json)}" | jq
```

Or paste the receipt JSON into [https://emem.dev/verify](/verify) — the
page recomputes `blake3(preimage) → ed25519.verify(sig, digest, pubkey)`
in your browser with `@noble/curves` + `@noble/hashes`. No server call.

## Memory tokens — share one signed fact in one string

```python
from emem import Client
em = Client()
cell  = em.locate("Mount Fuji")["cell64"]
facts = em.recall(cell, bands=["copdem30m.elevation_mean"])
fcid  = facts["facts"][0]["fact_cid"]

token = em.memory_token(cell, fcid)["token"]
# 'memt:defi.zb592.nemu.zEvE:vd5wzmaxh...j7bca'

# Paste this anywhere — LLM prompt, log line, Slack — any reader does:
em.memory_token_resolve(token)["fact"]
```

The token *is* the citation; two agents resolving the same token get
byte-identical bytes back.

## Next moves

- [Whitepaper](./whitepaper.html) — the math, the bit layouts, the trust proof
- [Protocol](./protocol.html) — wire format and signing rules
- [Agents](./agents.html) — how AI agents discover and call the protocol (MCP + REST)
- [Errors](./errors.html) — common error shapes and how to handle each
- [Registries](./registries.html) — bands, algorithms, sources, topics
- [Developers / Architecture](./developers/architecture.html) — what runs inside the box

## Where things live

- **Live API**: `https://emem.dev` (REST + MCP on the same port)
- **MCP endpoint**: `https://emem.dev/mcp` (Streamable HTTP, JSON-RPC 2.0)
- **OpenAPI**: `https://emem.dev/openapi.json`
- **GitHub**: `https://github.com/Vortx-AI/emem` — code, issues, roadmap
- **Self-host**: `docker run -p 5051:5051 ghcr.io/vortx-ai/emem:latest` (see [self-host.md](./self-host.html))
- **Status**: `GET /health` — single-line JSON; check it from your monitor
