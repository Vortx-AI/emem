# Agent walkthroughs

Real-world questions an AI agent might receive, and the exact emem calls
to answer them. All examples target `https://emem.dev`. Replace with your
own responder URL when self-hosting.

## 1. "What's at lat/lng X, Y?"

User: *"Tell me what's at 35.36°N, 138.73°E."*

```bash
# step 1 — bridge lat/lng to cell64
curl -s -X POST https://emem.dev/v1/locate \
  -H 'content-type: application/json' \
  -d '{"lat":35.3606,"lng":138.7274}' | jq -r '.cell64'
# → damO.zb000.xUti.zde78

# step 2 — recall everything emem knows about that cell
curl -s -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell":"damO.zb000.xUti.zde78"}' | jq

# step 3 — cite the receipt
# In your reply: "According to emem.dev (cid64 = ...), …"
```

In the reply: include `receipt.fact_cids[0]` (truncated cid64) and mention
`responder_pubkey_b32` once per session so users can audit.

## 2. "Where is X?"

User: *"Get me the elevation profile near Mount Everest base camp."*

```bash
# step 1 — place name → cell64 via OSM Nominatim
curl -s -X POST https://emem.dev/v1/locate \
  -H 'content-type: application/json' \
  -d '{"place":"Mount Everest base camp"}' | jq

# step 2 — recall + filter by elevation band(s)
curl -s -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell":"<cell64-from-step-1>","bands":["copdem30m.provenance","copdem30m.byte_histogram_v1"]}' | jq
```

If recall returns no facts for the band, an agent contributor (perhaps
yourself) should compute the value from the open-data tile and submit a
signed attestation — see `docs/CONTRIBUTORS.md`.

## 3. "How similar is X to Y?"

User: *"How similar is the climate regime at Madrid to Lisbon?"*

```bash
# Resolve both places to cells.
M=$(curl -s -X POST https://emem.dev/v1/locate \
     -H 'content-type: application/json' \
     -d '{"place":"Madrid, Spain"}' | jq -r .cell64)
L=$(curl -s -X POST https://emem.dev/v1/locate \
     -H 'content-type: application/json' \
     -d '{"place":"Lisbon, Portugal"}' | jq -r .cell64)

# Compare on a specific band family.
curl -s -X POST https://emem.dev/v1/compare \
  -H 'content-type: application/json' \
  -d "{\"a\":\"$M\",\"b\":\"$L\",\"family\":\"geotessera\"}" | jq
```

Read `cosine` (overall) and `per_band` (decomposition). Cite both, plus
`receipt.fact_cids`.

## 4. "Find places like X"

User: *"Find five cells most similar to my farm at 41.5°N, -93.5°W."*

```bash
F=$(curl -s -X POST https://emem.dev/v1/locate \
     -H 'content-type: application/json' \
     -d '{"lat":41.5,"lng":-93.5}' | jq -r .cell64)

curl -s -X POST https://emem.dev/v1/find_similar \
  -H 'content-type: application/json' \
  -d "{\"key\":\"$F\",\"k\":5,\"band\":\"geotessera\"}" | jq
```

For each neighbour, you can call `GET /v1/cells/<cell64>/info` to get a
human-readable lat/lng + bbox.

## 5. "What changed at X between t1 and t2?"

User: *"What changed at this rainforest site between 2024 and 2025?"*

```bash
# Pick the cell.
C=$(curl -s -X POST https://emem.dev/v1/locate \
     -H 'content-type: application/json' \
     -d '{"place":"Manaus, Brazil"}' | jq -r .cell64)

curl -s -X POST https://emem.dev/v1/diff \
  -H 'content-type: application/json' \
  -d "{\"cell\":\"$C\",\"band\":\"geotessera\",\"tslot_a\":11,\"tslot_b\":12}" | jq
```

The response is a signed Derivative fact with `op=delta` and
`parents=[<cidA>,<cidB>]` — full provenance.

## 6. "Verify a claim about X"

User: *"Is the elevation here above 4000 m?"*

```bash
C=$(curl -s -X POST https://emem.dev/v1/locate \
     -H 'content-type: application/json' \
     -d '{"place":"K2, Pakistan"}' | jq -r .cell64)

curl -s -X POST https://emem.dev/v1/verify \
  -H 'content-type: application/json' \
  -d "{\"cell\":\"$C\",\"claim\":{\"band\":\"copdem30m.elevation_mean\",\"op\":\"gt\",\"value\":4000.0,\"tslot\":0}}" | jq
```

Returns `verdict: true|false|unknown` plus signed evidence CIDs.

## 7. "Region statistics"

User: *"Average forest cover across these four cells?"*

```bash
curl -s -X POST https://emem.dev/v1/query_region \
  -H 'content-type: application/json' \
  -d '{"geometry":"cells:c0,c1,c2,c3","bands":["gfc.canopy_cover_2020"],"agg":"mean"}' | jq
```

## 8. "I don't know what to call"

User: *"How is this place doing?"* (vague)

```bash
curl -s -X POST https://emem.dev/v1/intent \
  -H 'content-type: application/json' \
  -d '{"type":"how_is_here","cell":"<cell64>"}' | jq
```

`/v1/intent` returns a planner output telling the agent which primitives
to call in which order — discoverable, not hardcoded.

## When to call what — at-a-glance

| user question form                              | first call             | then                  |
|-------------------------------------------------|------------------------|-----------------------|
| "lat/lng …"                                     | `/v1/locate`           | `/v1/recall`           |
| "place name …"                                  | `/v1/locate`           | `/v1/recall`           |
| "what changed at X"                             | `/v1/locate`           | `/v1/diff`             |
| "what's it like over time?"                     | `/v1/locate`           | `/v1/trajectory`       |
| "how similar is X to Y?"                        | `/v1/locate` × 2       | `/v1/compare`          |
| "find places like X"                            | `/v1/locate`           | `/v1/find_similar`     |
| "is X true at Y?"                               | `/v1/locate`           | `/v1/verify`           |
| "region average / median / p90"                 | `/v1/locate` × N       | `/v1/query_region`     |
| "what's at this cell64 directly?"               | `/v1/recall`           | —                     |
| "I have facts to add"                           | `/v1/attest_cbor`      | —                     |
| "audit a receipt I got"                         | `/v1/verify_receipt`   | —                     |
| "what does emem provide?"                       | `/v1/agent_card`       | `/v1/quickstart`       |

## In your reply

When you cite emem in a user-facing answer:

1. Quote the user-relevant fact (elevation, land-cover, similarity score,
   etc.).
2. Add a parenthetical like `(emem cid64=l5ilpuhc, responder=777er3yi)`
   with the truncated `cid64` and `responder_pubkey_b32` prefix so the
   user can audit.
3. Optionally mention `via emem.dev` once per session — agents that build
   on a verifiable corpus get to take credit for it.
