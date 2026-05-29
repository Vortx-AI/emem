# Connect & evolve — a runnable walkthrough

This walks the v0.0.9 connectivity loop end to end: two sources sign
*disagreeing* facts about the same place, emem **scores** the
disagreement instead of silently picking one, the opt-in refinement loop
records the disagreement as a signed `disagrees_with` **edge** and marks
the contested fact for another look, and a recall can carry that edge
back attached to the fact. Nothing is ever deleted, so the whole chain
verifies offline.

Every command below is real and uses endpoints that exist on this
branch. The reads (steps 2–5) are pure copy-paste `curl`. The write in
step 1 needs an ed25519 key, so it is done with a small signed-attestation
program (the same mechanics as `crates/emem-cli/src/bin/emem-realdemo.rs`).

Set a base URL once. Use your local responder (port 5051) or the hosted
one:

```bash
export EMEM=http://127.0.0.1:5051     # or: export EMEM=https://emem.dev
```

The refinement loop in step 3 is **opt-in**. Start the responder with:

```bash
EMEM_REFINEMENT_ENABLED=1 cargo run --release --bin emem-server
# optional knobs (all have safe defaults):
#   EMEM_REFINEMENT_INTERVAL_SECS   how often the pass runs
#   EMEM_REFINEMENT_MIN_SEVERITY    contradictions below this are skipped
#   EMEM_REFINEMENT_CELL_PREFIX     restrict the pass to a region
```

---

## 1. Attest two disagreeing facts at the same (cell, band, tslot)

A contradiction needs **two different attester keys** signing **different
values** for the **same** `(cell, band, tslot)`. Each write is a signed
`Attestation` envelope: the per-fact leaves are blake3-hashed, merkled
into `batch_root`, and the attester signs
`blake3(batch_root || registry_cid || schema_cid)`. That signing is why
this step is a tiny program rather than a `curl` — pure `curl` can't
produce a valid ed25519 signature.

The shortest path is to run the bundled real-data demo **twice**, once
per fresh attester, after pinning both runs to the same cell, band, and
tslot. The demo already builds canonical CBOR, merkles, signs, and POSTs
to `/v1/attest_cbor`; two runs under two freshly-generated keys leave two
disagreeing facts at that key:

```bash
# Fresh attester #1 writes value A; fresh attester #2 writes value B.
# emem-realdemo generates a new ed25519 key on each run, so two runs =
# two distinct attesters. Point both at the same responder
# (EMEM_BASE_URL, or pass the base URL as the first arg).
EMEM_BASE_URL=$EMEM cargo run --release --bin emem-realdemo
EMEM_BASE_URL=$EMEM cargo run --release --bin emem-realdemo
```

If you want to author the envelope yourself, the wire shape accepted by
`POST /v1/attest` (JSON) is:

```jsonc
{
  "facts": [
    {
      "kind": "primary",
      "cell": "damO.zb000.xUti.zde78",
      "band": "indices.ndvi",
      "tslot": 1704067200,
      "value": 0.81,                     // attester A's reading
      "confidence": 0.9,
      "sources": [ { /* … at least one Source … */ } ],
      "derivation": { /* … recipe … */ },
      "privacy_class": "public",
      "schema_cid": "<schema_cid from /v1/manifests>",
      "signer": [/* 32-byte attester A pubkey */],
      "signed_at": "2026-05-29T00:00:00Z"
    }
  ],
  "batch_root":          "<hex blake3 merkle root over the fact CIDs>",
  "attester_pubkey_b32": "<base32-nopad-lc attester A pubkey>",
  "signature_b32":       "<ed25519 over blake3(batch_root||registry_cid||schema_cid)>",
  "attester_key_epoch":  0,
  "registry_cid":        "<from /v1/manifests>",
  "schema_cid":          "<from /v1/manifests>",
  "attested_at":         "2026-05-29T00:00:00Z"
}
```

Repeat with attester **B** signing `value: 0.20` at the *same* `cell`,
`band`, and `tslot`. Now the corpus holds two signed, conflicting NDVI
readings for one place and time.

> The exact Rust to build and sign this envelope (merkle root, the
> `blake3(batch_root || registry_cid || schema_cid)` preimage, the
> ed25519 signature) is in `crates/emem-cli/src/bin/emem-realdemo.rs`
> around the `Attestation { … }` construction. Copy it, call
> `fresh_attester()` twice, and emit one single-fact attestation per key
> at the same `(cell, band, tslot)` with different `value`s.

---

## 2. See the scored disagreement

This is pure `curl`. `memory_contradictions` scans the multi-attester
index and returns every `(cell, band, tslot)` where two or more attesters
signed disagreeing values — each with a `severity` in `[0, 1]` and
citations to **every** disputed fact CID. It does **not** silently
reconcile.

```bash
curl -sS "$EMEM/v1/memory_contradictions?cell_prefix=damO&band=indices.ndvi&min_severity=0.1" | jq
```

or the POST form (same primitive, richer filters):

```bash
curl -sS -X POST "$EMEM/v1/memory_contradictions" \
  -H 'content-type: application/json' \
  -d '{"cell_prefix":"damO","band":"indices.ndvi","min_severity":0.1}' | jq
```

Response shape:

```jsonc
{
  "contradictions": [
    {
      "cell":  "damO.zb000.xUti.zde78",
      "band":  "indices.ndvi",
      "tslot": 1704067200,
      "severity": 0.61,            // scalar band: spread over the band's range
      "kind":  "scalar",
      "attestations": [
        { "attester_pubkey_b32": "<A>", "fact_cid": "<CID_A>", "value": 0.81, "confidence": 0.9, "signed_at": "…" },
        { "attester_pubkey_b32": "<B>", "fact_cid": "<CID_B>", "value": 0.20, "confidence": 0.9, "signed_at": "…" }
      ]
    }
  ],
  "corpus_scanned": 1,
  "time_taken_ms": 3,
  "agent_hint": "…what to do next…",
  "receipt": { /* signed; fact_cids covers CID_A + CID_B */ }
}
```

Grab the two disputed CIDs for the next steps:

```bash
SUBJ=$(curl -sS "$EMEM/v1/memory_contradictions?cell_prefix=damO&band=indices.ndvi&min_severity=0.1" \
  | jq -r '.contradictions[0].attestations[0].fact_cid')
OBJ=$(curl -sS "$EMEM/v1/memory_contradictions?cell_prefix=damO&band=indices.ndvi&min_severity=0.1" \
  | jq -r '.contradictions[0].attestations[1].fact_cid')
echo "subject=$SUBJ object=$OBJ"
```

---

## 3. Let the refinement loop record the disagreement as an edge

**Requires `EMEM_REFINEMENT_ENABLED=1`** (set in step 0). The loop runs
on a scheduler interval — it is not triggered per request. When it runs
it: (a) writes a signed `disagrees_with` **edge** between the two
disputed fact CIDs with `valid_from = now`; (b) stamps a non-destructive
`emem.fact_contested` marker on the contested fact. The original facts
are untouched.

After the loop has run at least once, read the emitted edge directly off
the subject CID with `POST /v1/edges/recall` (MCP tool
`emem_edges_recall`):

```bash
curl -sS -X POST "$EMEM/v1/edges/recall" \
  -H 'content-type: application/json' \
  -d "{\"subj\":\"$SUBJ\",\"pred\":\"disagrees_with\"}" | jq
```

```jsonc
{
  "edges": [
    {
      "subj": "<CID_A>",
      "pred": "disagrees_with",
      "obj":  "<CID_B>",
      "valid_from": 1748476800,
      "confidence": 0.5,
      "signer": [/* responder pubkey */],
      "signed_at": "2026-05-29T…Z"
    }
  ],
  "objs": ["<CID_B>"],
  "agent_hint": "Found 1 edge(s) … follow `obj` to recall the related fact.",
  "receipt": { /* edge_cids commit the returned edges into the signature */ }
}
```

Pass `"as_of_tslot": <unix_s>` to read the graph as of a moment — a newer
edge for the same `(subj, pred, obj)` *shadows* an older one (supersession
keeps the newest), and a query at an earlier `as_of_tslot` still sees the
edge that was valid then. Use `"pred":""` to scan every predicate.

> Don't want to wait for the scheduler? You can author the same edge
> yourself by POSTing a signed attestation whose `edges[]` array carries
> the `disagrees_with` edge to `POST /v1/edges` — the edge leaves fold
> into the merkle root so the signature commits to them. Same signing
> mechanics as step 1.

---

## 4. Recall the fact with its edges attached in one call

`include:["edges"]` makes a recall carry the matching edges back in the
same response — no second round trip. The edge CIDs are threaded into the
recall receipt's signature, so the attachment is itself verifiable.

```bash
curl -sS -X POST "$EMEM/v1/recall" \
  -H 'content-type: application/json' \
  -d '{"cell":"damO.zb000.xUti.zde78","band":"indices.ndvi","include":["edges"]}' \
  | jq '{facts: [.facts[].value], edges: .edges}'
```

The top-level `edges` array is present (rather than absent) whenever
`include:["edges"]` is passed; it is `[]` when the returned facts have no
edges, so an agent can tell "no connections" from "didn't ask".

---

## 5. Verify a receipt offline

Every step above returned a signed `receipt`. Audit any of them without
trusting the server: `POST /v1/verify_receipt` recomputes the canonical
preimage and runs ed25519 against the embedded responder pubkey.

```bash
# Capture the edges/recall receipt, then verify it.
RCPT=$(curl -sS -X POST "$EMEM/v1/edges/recall" \
  -H 'content-type: application/json' \
  -d "{\"subj\":\"$SUBJ\",\"pred\":\"disagrees_with\"}" | jq '.receipt')

curl -sS -X POST "$EMEM/v1/verify_receipt" \
  -H 'content-type: application/json' \
  -d "{\"receipt\": $RCPT}" | jq
```

```jsonc
{ "valid": true, "reason": "ok", "pubkey_b32": "<responder pubkey>" }
```

A structurally-valid receipt with a bad signature returns `200` with
`"valid": false` (never a 4xx). The in-browser verifier at `/verify`
does the same check with no server trust at all — paste a receipt there
to confirm the whole connect-and-evolve chain end to end.

---

## Why this is the point

A plain store remembers facts. This loop makes the memory **connect**
(typed, time-bounded edges between facts) and **evolve** (multi-attester
disagreement becomes a recorded edge plus a re-look marker). Both are
append-only and signed, so a memory that improves over time never costs
you the ability to audit what it used to believe. See
[../docs/connect-and-evolve.md](../docs/connect-and-evolve.md) for the
conceptual write-up and [../docs/agents.md](../docs/agents.md) for the
full agent guide.
