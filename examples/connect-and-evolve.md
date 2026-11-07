# Connect and evolve: a walkthrough

This page shows how emem handles two signed facts that disagree about
one place. emem scores the disagreement and keeps both facts. An opt-in
refinement loop then records the disagreement as a signed
`disagrees_with` edge and marks one fact as contested. A recall can
return that edge with the fact. emem deletes nothing. You can verify
every fact and edge offline, because a signed attestation covers each
one. The contested marker has no signature. For the concepts, see
[../docs/connect-and-evolve.md](../docs/connect-and-evolve.md).

## What you need

| part | needs |
|---|---|
| Part 1, read disagreements on emem.dev | `curl` and `jq`. No key, no account. |
| Part 2, run the full loop | A Rust toolchain and a local `emem-server`. The loop writes signed facts and edges, and https://emem.dev refuses those writes. |

## What you get

- A scored disagreement: two signed values at one `(cell, band, tslot)`,
  each with its `fact_cid`, and a `severity` from 0 to 1.
- The typed edges that start at a fact, read with `POST /v1/edges/recall`.
- A recall that carries its edges in the same response.
- A receipt for each read that you can verify.

The Part 1 outputs come from https://emem.dev on 7 November 2026. The
Part 2 outputs come from a local node on the same date.

```bash
export EMEM=https://emem.dev
```

---

## Part 1: read disagreements on emem.dev

These calls are reads. Copy them as they are.

### 1. Find a scored disagreement

`memory_contradictions` returns each `(cell, band, tslot)` where signed
values disagree. It does not choose a winner. It cites every disputed
fact CID and gives a `severity` in `[0, 1]`.

By default it reports two kinds of disagreement. The field
`disagreement_scope` names the kind:

| `disagreement_scope` | meaning |
|---|---|
| `multi_attester` | Two or more different keys signed different values. |
| `same_attester_provider_substitution` | One key answered from two upstreams (a different `derivation.fn_key` or `sources[].scheme`). |

Send `"include_same_attester_sources": false` to get only the first kind.
For the cell below, that request returns no contradictions.

```bash
curl -sS -X POST "$EMEM/v1/memory_contradictions" \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb27e.qEfa.ze033","min_severity":0.1}' \
  | jq '.contradictions[0] | {cell, band, tslot, severity, kind, disagreement_scope,
        attestations: [.attestations[] | {fact_cid, value, signed_at}]}'
```

Output on 7 November 2026. This cell has one contradiction:

```json
{
  "cell": "defi.zb27e.qEfa.ze033",
  "band": "overture.transportation.road_length_m",
  "tslot": 56,
  "severity": 1.0,
  "kind": "scalar",
  "disagreement_scope": "same_attester_provider_substitution",
  "attestations": [
    { "fact_cid": "6tww5qimzfma6xx2uly2cerpvxkficl4fkw6nxckrz22ajkn3tkq", "value": 8.452728843233139, "signed_at": "2026-06-29T04:53:38Z" },
    { "fact_cid": "qklwofpxehwv54y4znfymseau3h3p2tl6sptjmtxe7nzba765m3q", "value": 0.0, "signed_at": "2026-10-11T11:21:10Z" }
  ]
}
```

Here one responder key read the road length from
`overture_road_length_m@1` and later from `overture_road_length_m@2`.
The two values differ, so both stay on record.

To see the reader of each fact, get the fact by its CID from
`GET /v1/facts/{cid}`:

```bash
curl -sS "$EMEM/v1/facts/6tww5qimzfma6xx2uly2cerpvxkficl4fkw6nxckrz22ajkn3tkq" \
  | jq -c '{fn_key: .derivation.fn_key, value}'
curl -sS "$EMEM/v1/facts/qklwofpxehwv54y4znfymseau3h3p2tl6sptjmtxe7nzba765m3q" \
  | jq -c '{fn_key: .derivation.fn_key, value}'
```

Output on 7 November 2026:

```json
{"fn_key":"overture_road_length_m@1","value":8.452728843233139}
{"fn_key":"overture_road_length_m@2","value":0.0}
```

A plain `POST /v1/recall` for this band returns only the newer fact:

```bash
curl -sS -X POST "$EMEM/v1/recall" \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb27e.qEfa.ze033","bands":["overture.transportation.road_length_m"]}' \
  | jq -c '[.facts[] | {fact_cid, value, tslot}]'
```

Output on 7 November 2026:

```json
[{"fact_cid":"qklwofpxehwv54y4znfymseau3h3p2tl6sptjmtxe7nzba765m3q","value":0.0,"tslot":56}]
```

The full `memory_contradictions` response also has `corpus_scanned`,
`time_taken_ms`, `scan_truncated`, `agent_hint` and a signed `receipt`. The receipt's
`fact_cids` cite every disputed fact. For this cell, it cites the two
facts above. A scan that stops at its time budget returns
`resume_after`. Send that value back to continue.

Keep `cell_prefix` (alias `cell`) narrow. A full cell64 scans few keys.
On 7 November 2026, this cell scanned 4 keys (`corpus_scanned`) and
answered in less than 0.1 s. A short
prefix scans up to 50,000 keys. On 7 November 2026, the prefix `defi`
took 8 s, scanned about 44,700 keys, found 74 contradictions and
stopped with `scan_truncated: true`. On 6 November 2026, one
10-character prefix took 108 s.

A GET form also exists. It takes the same fields as query parameters.
For this cell and band, it returns the same contradiction:

```bash
curl -sS "$EMEM/v1/memory_contradictions?cell_prefix=defi.zb27e.qEfa.ze033&band=overture.transportation.road_length_m&min_severity=0.1" \
  | jq -c '[.contradictions[] | {band, tslot, severity, n: (.attestations | length)}]'
```

Output on 7 November 2026:

```json
[{"band":"overture.transportation.road_length_m","tslot":56,"severity":1.0,"n":2}]
```

### 2. Read the edges that start at a fact

An edge is a signed link between two fact CIDs, such as
`disagrees_with`, `supersedes` or `relates_to`. Read the edges that
start at a fact with `POST /v1/edges/recall` (MCP tool
`emem_edges_recall`):

```bash
curl -sS -X POST "$EMEM/v1/edges/recall" \
  -H 'content-type: application/json' \
  -d '{"subj":"6tww5qimzfma6xx2uly2cerpvxkficl4fkw6nxckrz22ajkn3tkq","pred":""}' \
  | jq '{direction, edges, objs, agent_hint}'
```

Output on 7 November 2026:

```json
{
  "direction": "out",
  "edges": [],
  "objs": [],
  "agent_hint": "No edges originate at subject '6tww5qimzfma6xx2uly2cerpvxkficl4fkw6nxckrz22ajkn3tkq' under any predicate. This is the honest 'no relation known', not a lookup failure — write edges by POSTing a signed Attestation with an `edges` array to /v1/edges."
}
```

An empty list means "no relation known". It is not an error. This pair
has no edge because the refinement loop skips one-attester disagreements
(see Part 2, demo steps 4 and 5).

Request fields:

| field | meaning |
|---|---|
| `subj` | Start fact CID. Required for `direction: "out"` (the default). |
| `obj` | End fact CID. Setting it reads the reverse direction, `"in"`: what points at this fact. |
| `pred` | Predicate filter. `""` (the default) returns every predicate. |
| `as_of_tslot` | Read the graph at this tslot. It uses the same scale as the edge's `valid_from`. An edge counts when `valid_from <= as_of_tslot` and it has no `valid_to` or `valid_to >= as_of_tslot`. The newest edge per neighbour wins. |
| `limit` | Default 100, maximum 1000. |

### 3. Recall a fact with its edges attached

`include:["edges"]` adds a top-level `edges` array to a recall. You get
the facts and their edges in one call.

```bash
curl -sS -X POST "$EMEM/v1/recall" \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb592.nemu.zEvE","bands":["copdem30m.elevation_mean"],"include":["edges"]}' \
  | jq -c '{facts: [.facts[].value], edges: .edges}'
```

Output on 7 November 2026:

```json
{"facts":[3768.409912109375],"edges":[]}
```

When you pass `include:["edges"]`, the `edges` array is always present.
It is `[]` when the facts have no edges. An agent can thus tell "no
connections" from "did not ask". When the recall attaches edges, the
receipt binds their CIDs into the signature.

### 4. Verify a receipt

Every response above has a signed `receipt`. The `jq` filters in steps
1 to 3 do not show it. `POST /v1/verify_receipt` rebuilds the canonical
preimage and checks the Ed25519 signature.

```bash
RCPT=$(curl -sS -X POST "$EMEM/v1/edges/recall" \
  -H 'content-type: application/json' \
  -d '{"subj":"6tww5qimzfma6xx2uly2cerpvxkficl4fkw6nxckrz22ajkn3tkq","pred":"disagrees_with"}' \
  | jq '.receipt')

curl -sS -X POST "$EMEM/v1/verify_receipt" \
  -H 'content-type: application/json' \
  -d "{\"receipt\": $RCPT}" \
  | jq '{valid, signature_valid, merkle_proof_valid, reason, primitive, signer_pubkey_b32}'
```

Output on 7 November 2026:

```json
{
  "valid": true,
  "signature_valid": true,
  "merkle_proof_valid": true,
  "reason": null,
  "primitive": "emem.edges_recall",
  "signer_pubkey_b32": "777er3yihgifqmv5hmc2wwmyszgddzderzhsx6rex4yoakwomvka"
}
```

A receipt with a changed field returns HTTP 200 with `"valid": false`.
To see this, change one field of the same receipt and send it again:

```bash
echo "$RCPT" | jq '{receipt: (.primitive = "emem.recall")}' \
  | curl -sS -X POST "$EMEM/v1/verify_receipt" \
      -H 'content-type: application/json' -d @- \
  | jq -c '{valid, reason}'
```

Output on 7 November 2026:

```json
{"valid":false,"reason":"signature_invalid"}
```

A body that is not a receipt at all returns HTTP 400
`invalid_argument`. This check runs on the responder, so it is not an
independent check. To check a receipt with no trust in the responder,
paste it into the in-browser verifier at https://emem.dev/verify.

---

## Part 2: run the full loop on a local node

Part 2 writes. It needs a node that accepts your keys. We ran it on 7
November 2026 on a local node built from commit `49443d9a` (version
2.4.4, release build). The output below comes from that run. Fact CIDs
and keys are new on every run.

### Why emem.dev refuses the writes

By default the fact plane refuses other keys. A node accepts a fact at
an address `(cell, band, tslot)` only from its own key, from an enrolled
device, or from a key in `EMEM_FACT_PLANE_WRITERS`. Every other attestation gets
403 `level_too_low` (see `GET /v1/enlist`, surface `fact_plane`, tier
T4). A local node can open the plane:

| setting | effect |
|---|---|
| `EMEM_FACT_PLANE_OPEN=1` | Admits any signing key. Use it for the demo, which makes new keys on each run. |
| `EMEM_FACT_PLANE_WRITERS=<key>,<key>` | Admits only the listed base32 Ed25519 public keys. Use it when you sign with fixed keys. |

Edges take no address, so `POST /v1/edges` stays open at tier T1.

### Start the node

The refinement loop is off by default. Turn it on with
`EMEM_REFINEMENT_ENABLED=1`.

With no `EMEM_DATA`, the node keeps its data in `./var/emem`.
Give the demo its own empty directory, so its test facts stay out of
any other node data. `EMEM_BIND` keeps the node on the local interface.

```bash
EMEM_DATA="$(mktemp -d)" \
EMEM_BIND=127.0.0.1:5051 \
EMEM_FACT_PLANE_OPEN=1 \
EMEM_REFINEMENT_ENABLED=1 \
EMEM_REFINEMENT_INTERVAL_SECS=3 \
EMEM_REFINEMENT_MIN_SEVERITY=0.1 \
cargo run --release --bin emem-server
```

The first release build takes a long time. On our host it took more
than 25 minutes. When the node is ready, this check returns `true`:

```bash
curl -s http://127.0.0.1:5051/health | jq '.ok'
```

Refinement settings, from `crates/emem-primitives/src/memory_consolidation.rs`:

| variable | default | meaning |
|---|---|---|
| `EMEM_REFINEMENT_ENABLED` | off | Turns the loop on. |
| `EMEM_REFINEMENT_INTERVAL_SECS` | 86400 | Time between passes. The loop runs on this timer, not per request. |
| `EMEM_REFINEMENT_MIN_SEVERITY` | 0.5 | Contradictions below this severity get no edge. |
| `EMEM_REFINEMENT_CELL_PREFIX` | unset | Limits the pass to cells with this prefix. |
| `EMEM_REFINEMENT_MAX_PAIRS` | 1 | Edges per contradiction. |

### Run the demo

In a second shell:

```bash
cargo run --release -p emem-cli --bin emem-connect-demo http://127.0.0.1:5051
# or: EMEM_BASE_URL=http://127.0.0.1:5051 cargo run --release -p emem-cli --bin emem-connect-demo
```

After the server build, this build took more than 10 minutes on our
host. The demo run itself took 16 s.

The demo (`crates/emem-cli/src/bin/emem-connect-demo.rs`) sends real
signed bytes. It prints a header for each of its seven steps. It stops
with a non-zero exit on any HTTP error, so you can use it as a smoke
test. The output of our run on 7 November 2026, shortened:

```text
=== STEP 1: mint two ephemeral attesters ===
  attester A pubkey_b32 = 7a7qzshfbno32hrpr5pepufnnzbyl32227sdtqwgg2zbefswmyba
  attester B pubkey_b32 = 7r26yhfm25bix5iyap4w333nazrocas7ncouxiye23n5jdz4tyha

=== STEP 2: attest two disagreeing facts at the same (cell, band, tslot) ===
  cell=defi.zb3d8.wOsu.zf07c band=indices.ndvi tslot=1704067200
  A wrote value 0.61 → fact_cid = dd2j2ezwhpcsia2krmmpskszgcpzj2nq2nzjmcgy2xpdjv37a7pq
  B wrote value 0.19 → fact_cid = 3zxy6qtiqkh2ziebdp36pc3lbp4gosnzdvilwszq3w34xykok3hq

=== STEP 3: signed edges + bi-temporal supersession ===
  /v1/edges/recall as_of=15 → 1 edge(s), newest valid_from=Some(10) (expect 10)
  /v1/edges/recall as_of=25 → 1 edge(s), newest valid_from=Some(20) (expect 20)
  bi-temporal supersession CONFIRMED (15→vf10, 25→vf20)

=== STEP 4: scored disagreement via /v1/memory_contradictions ===
  contradiction: cell="defi.zb3d8.wOsu.zf07c" band="indices.ndvi" tslot=1704067200 severity=0.20999999344348907 kind="scalar"
  scored disagreement CONFIRMED

=== STEP 5: refinement-loop auto disagrees_with edge (opt-in) ===
  no auto disagrees_with edge within ~16s — either the refinement loop is disabled or it hasn't run yet. ...

=== STEP 6: recall cell with include:["edges"] ===
  recalled 1 fact(s); attached edges array present = true
    edge: subj="dd2j2ezw..." pred="relates_to" obj="3zxy6qti..." valid_from=20

=== STEP 7: verify a returned receipt via /v1/verify_receipt ===
  valid=true merkle_proof_valid=true reason=null
  receipt signature CONFIRMED valid

All connect-and-evolve steps green.
```

Step 5 says that no edge came, but the loop made the edge. The node log
showed `edges_emitted=1` on the pass after step 2. The section on steps
4 and 5 below explains why the demo can miss the edge.

### Demo steps 1 and 2: two keys sign different values at one address

The demo makes two new Ed25519 keys, A and B. Each signs one
`indices.ndvi` fact at the same address:

| field | value |
|---|---|
| cell | `defi.zb3d8.wOsu.zf07c` (lat -3.4653, lng -62.2159) |
| band | `indices.ndvi` |
| tslot | `1704067200` |
| value from A | `0.61` |
| value from B | `0.19` |

Each fact goes to `POST /v1/attest_cbor` in its own signed
`Attestation` envelope. A plain `curl` cannot make this envelope, because
it needs an Ed25519 signature. The demo builds it with
`Attestation::build_and_sign_v1`, the one canonical signing path. That
function hashes each fact's canonical CBOR, builds a Merkle root over the
sorted leaves, and signs the root under preimage version 1.

To write your own envelope, copy the `scalar_fact` and
`sign_attestation` functions from the demo. `POST /v1/attest` takes the
same envelope as JSON. The JSON field names come from
`crates/emem-fact/src/attest.rs`:

```jsonc
{
  "facts": [
    {
      "kind": "primary",
      "cell": "defi.zb3d8.wOsu.zf07c",
      "band": "indices.ndvi",
      "tslot": 1704067200,
      "value": 0.61,
      "unit": null,
      "confidence": 0.9,
      "uncertainty": null,
      "sources": [{ "scheme": "demo", "id": "connect-demo-0.61" /* other Source fields */ }],
      "derivation": { "fn_key": "demo@1", "args": null },
      "privacy_class": "public",
      "schema_cid": "<schema_cid from GET /health>",
      "signer": [/* 32 bytes: the attester public key */],
      "signed_at": "2026-05-29T00:00:00Z"
    }
  ],
  "batch_root": [/* 32 bytes: Merkle root over the leaves */],
  "attester": [/* 32 bytes: the attester public key */],
  "attester_key_epoch": 0,
  "registry_cid": "<registry_cid from GET /health>",
  "schema_cid": "<schema_cid from GET /health>",
  "signature": [/* 64 bytes: Ed25519 signature */],
  "attested_at": "2026-05-29T00:00:00Z",
  "preimage_version": 1
}
```

Keys, roots and signatures are JSON arrays of byte values, not strings.

### Demo step 3: two signed edges show supersession

Key A signs two `relates_to` edges from A's fact to B's fact. They are
the same edge with `valid_from` 10 and 20. Each goes to `POST /v1/edges`
inside a signed attestation. The edge hashes are part of the Merkle
leaves, so the signature covers them.

The demo then reads the graph at two times:

| `as_of_tslot` | edge returned |
|---|---|
| 15 | the edge with `valid_from` 10 |
| 25 | the edge with `valid_from` 20 |

A newer edge for the same `(subj, pred, obj)` hides an older one. A read
at an earlier time still sees the edge that was valid then.

### Demo steps 4 and 5: the scored disagreement and the refinement edge

The demo reads `POST /v1/memory_contradictions` with `cell_prefix` set
to the first four characters of the cell, `band` `indices.ndvi` and
`min_severity` 0.1. It expects one contradiction with `severity` above 0
that cites both fact CIDs.

On the next pass of the refinement loop, the node does two things. The
code is `run_refinement_pass` in `crates/emem-api-rest/src/lib.rs`.

1. It signs a `disagrees_with` edge with its own key. The subject is the
   smaller of the two fact CIDs and the object is the larger.
   `valid_from` is the contradiction's `tslot`. `confidence` is the
   contradiction's `severity`. `note` gives the severity, kind and band.
2. It stores an `emem.fact_contested` marker on the fact with the lower
   confidence. When the two confidences are equal, it marks the fact with
   the smaller CID. The facts themselves do not change.

The loop only links `multi_attester` disagreements. It skips
`same_attester_provider_substitution` disagreements, because one key that
changes upstream is not a second witness. It also skips a pair that
already has the same edge.

The demo polls `POST /v1/edges/recall` with `pred` `disagrees_with` for
about 16 s. It always sends A's fact CID as `subj`. The edge starts at
the smaller CID, so the demo finds the edge only when A's CID is the
smaller one. In both of our runs on 7 November 2026, B's CID was smaller
(`3zxy...` before `dd2j...`, and `f7pq...` before `t5pu...`). The demo
then printed its "no auto disagrees_with edge" note and went on. The
same note appears when the node runs without the refinement settings.

To read the edge yourself, start from either fact. Use `subj` for the
smaller CID, or `obj` for the larger one:

```bash
curl -sS -X POST http://127.0.0.1:5051/v1/edges/recall \
  -H 'content-type: application/json' \
  -d '{"obj":"<larger fact_cid>","pred":"disagrees_with"}' | jq '.edges'
```

In our second run, this read returned one edge (fields shortened):

```json
{
  "subj": "f7pqpn7mcjkoj7v4olatu56biwwsqfmccz5etxzcokx467mycudq",
  "obj": "t5pux53zexyio2ngcftv2kd6ysl2f3475zz2f5utokabzdj5g25q",
  "pred": "disagrees_with",
  "valid_from": 1704067200,
  "confidence": 0.20999999344348907,
  "note": "severity=0.210 kind=scalar band=indices.ndvi"
}
```

A read with `subj` set to the larger CID returns an empty `edges` list.

You can also sign a `disagrees_with` edge yourself and send it to
`POST /v1/edges`, the same way as demo step 3. You then do not wait for
the loop.

### Demo steps 6 and 7: recall with edges, then verify

The demo recalls the cell with `"include":["edges"]` and prints the
attached edges. It then sends the recall receipt to
`POST /v1/verify_receipt` and expects `"valid": true`. These are the same
calls as Part 1, steps 3 and 4.

---

## What the loop adds to a plain store

A plain store keeps facts. This loop also connects facts with typed
edges that have a start time (`valid_from`) and an optional end time
(`valid_to`). It records a disagreement as an edge and a contested
marker, so the memory changes over time. A signed attestation covers
every edge, and the store only appends edges. The loop deletes no
fact. You can still audit what the memory held before. The contested
marker is an unsigned overlay row on the node, not a signed record.
For the full agent guide, see [../docs/agents.md](../docs/agents.md).
