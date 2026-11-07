# emem skills

*For agents. Machine-first: runnable procedures, not an explanation of why. A person should read https://emem.dev/how-it-works first.*

These recipes are for AI agents that build on emem. emem is a shared,
verifiable memory for AI agents. It gives each place and object one
citable identity, so different models reason from the same object and
not from different descriptions. Signed Earth observation is the base.
Each recipe combines two or three `/v1/*` calls into one useful step.

This page is the short cookbook. The same procedures also ship as a
Claude Code plugin at
[plugins/emem/](https://github.com/Vortx-AI/emem/tree/main/plugins/emem).
The plugin has more detail and bundled verifier scripts. See
§ Installing as Claude Skills below.

The endpoint is `https://emem.dev`, or the URL of your own node. Reads
need no key. The caller signs each write with Ed25519, and the write tier
depends on its reach (`GET /v1/enlist`). Each response that serves facts
carries an Ed25519 receipt over a deterministic preimage. Verify it
offline with the responder key from `/.well-known/emem.json`.

Each fact has a content address and a signature. Thus any conformant
responder returns the same bytes for the same content id, and any client
can verify the receipt offline. emem is a protocol, not a single endpoint.

## Quick reference

| Recipe | Calls |
|---|---|
| **locate-and-recall** | `POST /v1/locate` → `POST /v1/recall` |
| **verify-receipt-offline** | (any receipt) → BLAKE3 + Ed25519 in-process |
| **find-similar-places** | `POST /v1/locate` → `POST /v1/find_similar` |
| **scene-thumbnail** | `GET /v1/cells/{cell64}/scene.png` or `GET /v1/scene.png?bbox=w,s,e,n` |
| **recall-many-cells** | `POST /v1/locate` (×N) → `POST /v1/recall_many` |
| **area-recall** | `POST /v1/recall_polygon` |
| **trajectory-over-time** | `POST /v1/trajectory` |
| **field-signals** | `POST /v1/field_boundaries` → `POST /v1/field_burn_scar`, `POST /v1/field_actual_et` |
| **eudr-due-diligence** | `POST /v1/eudr_dds` |
| **document-evidence** | `POST /v1/ocr` → `POST /v1/lab_report_parse` or `POST /v1/land_record_parse` |
| **transparency-log-audit** | `GET /v1/log/sth` → `GET /v1/log/inclusion`, `GET /v1/log/consistency` |
| **heat-solve-at-a-cell** | `POST /v1/heat_solve` |
| **dashboard-tile** | `POST /v1/field_series` (area) or `POST /v1/trajectory` (one cell) |
| **storyboard** | `POST /v1/field_series` frames, `POST /v1/ask` → `GET /v1/state/{cid}` |

---

## 1. `locate-and-recall`: name a place, get signed facts

Resolve a place name to a `cell64`, then recall one or more bands there.

| Field in the recall response | What it holds |
|---|---|
| `facts[]` | Every stored reading, oldest first. |
| `current_by_band` | The `fact_cid` of the latest reading, for each band. |
| `facts[].memory_token` | `emem:fact:<cell64>:<fact_cid>`. This handle gets the same bytes again in any year. |
| `receipt` | The Ed25519 receipt over the call. Section 2 verifies it. |

A `fact_cid` is a digest of the value, the signer and the signing time.
Thus it names one attestation, not the observation behind it. A second
responder that measures the same thing makes a different cid. Use
`emem:entity:` for an identity that is the same across responders.

### curl

```sh
BASE=https://emem.dev

# 1. Place name → cell64.
CELL=$(curl -sf -X POST $BASE/v1/locate \
  -H 'content-type: application/json' \
  -d '{"q":"Bengaluru, India"}' | jq -r '.cell64')

# 2. Recall the 2 m air temperature and take the latest reading.
curl -sf -X POST $BASE/v1/recall \
  -H 'content-type: application/json' \
  -d "{\"cell\":\"$CELL\",\"bands\":[\"weather.temperature_2m\"]}" \
  | jq '.current_by_band["weather.temperature_2m"] as $c
        | .facts[] | select(.fact_cid == $c)
        | {band, value_verbatim, unit, observed_at, memory_token}'
```

### python

```py
import httpx
BASE = "https://emem.dev"
loc = httpx.post(f"{BASE}/v1/locate", json={"q": "Bengaluru, India"}).json()
rec = httpx.post(f"{BASE}/v1/recall", json={
    "cell": loc["cell64"],
    "bands": ["weather.temperature_2m"],
}, timeout=60).json()
cid = rec["current_by_band"]["weather.temperature_2m"]
fact = next(f for f in rec["facts"] if f["fact_cid"] == cid)
print(fact["band"], "=", fact["value_verbatim"], fact.get("unit") or "")
print("cite:", fact["memory_token"])
```

### MCP (JSON-RPC over `POST /mcp`)

The MCP reply puts the result JSON as text in `result.content[0].text`.
It keeps that text below 12,000 bytes. If `facts[]` is longer, the text
has an `_emem_truncation` block. That block gives the first omitted
index and the call that returns the rest.

```json
{ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
  "params": { "name": "emem_locate", "arguments": {"q": "Bengaluru, India"} } }
```

```json
{ "jsonrpc": "2.0", "id": 2, "method": "tools/call",
  "params": { "name": "emem_recall",
              "arguments": {"cell": "defi.zb493.zezo.zcb35",
                            "bands": ["weather.temperature_2m"]} } }
```

### Common pitfalls

- `facts[0]` is the oldest reading, not the current one. Use
  `current_by_band` to find the latest.
- If `/v1/locate` returns `disambiguation_required: true`, read
  `alternatives`. Ask the user which place they mean. Do not cite rank 0.
- On a cold cell, recall fetches from the upstream connector. Then the
  response can have a `materialize_notes[]` entry with
  `reason_class: "timeout"` and `retryable: true`. This means that the
  fetch continues in the background. The same call a few seconds later
  usually gets the stored facts.

---

## 2. `verify-receipt-offline`: BLAKE3 + Ed25519, no callback

You can verify each receipt without a new call to the responder. Rebuild
the canonical preimage and hash it with BLAKE3. Then run
`ed25519.verify(signature, digest, pubkey)`. The byte layout is in
`crates/emem-attest` (`receipt_preimage_v1`, `receipt_preimage_v2`).
`GET /v1/verifier_spec` publishes the same layout from the compiled
constants.

### Preimage (v2, current receipts)

```
blake3(
    "emem.preimage.v1\x00" || u32le(len("receipt")) || "receipt"
    || seg(0x01, request_id) || seg(0x02, served_at)
    || [seg(0x03, scope_hex)] || [seg(0x04, as_of_hex)]
    || [seg(0x05, edges_hex)] || [seg(0x06, manifest_hex)]
    || seg(0x07, primitive)
    || seg_list(0x08, cells) || seg_list(0x09, fact_cids)
    || [seg(0x0a, field_hex)]
    || seg(0x0b, merkle_hex)
)
```

| Term | Meaning |
|---|---|
| `seg(tag, bytes)` | `tag \|\| u32le(len) \|\| bytes` |
| `seg_list(tag, items)` | `tag`, a u32le count, then `u32le(len) \|\| bytes` for each item |
| `[ ... ]` | The segment is in the preimage only when the receipt has that field. |
| `manifest_hex` | Lowercase hex of BLAKE3 over the CBOR map of a non-empty `source_versions`. Keys are in plain string order. |
| `merkle_hex` | Hex digest of `PreimageV1("merkle")` over the receipt's `merkle_proof`: root, u32le leaf_index, the path joined, rule version. |

Under v2 the preimage always has segment `0x0b`. If the receipt has no
Merkle proof, `merkle_hex` covers an explicit absence marker. Thus a
receipt with its proof removed fails the signature check.

| `preimage_version` | Rule |
|---|---|
| 2 | The layout above. Current receipts use it. |
| 1 | The same layout without segment `0x0b`. |
| absent | The legacy pipe rule. `POST /v1/verify_receipt` still checks it. |

### python (fully offline)

The script needs `httpx`, `cbor2`, `blake3` and `cryptography`. It
refuses receipts that have a `scope` or `edge_cids` segment. Send those
to `POST /v1/verify_receipt`. On 7 November 2026 it printed `True` for a
live recall receipt.

```py
import httpx, cbor2
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
from blake3 import blake3

def b32decode(s):
    table = "abcdefghijklmnopqrstuvwxyz234567"
    bits = "".join(format(table.index(c), "05b") for c in s.lower())
    return bytes(int(bits[i:i+8], 2) for i in range(0, len(bits) - len(bits) % 8, 8))

def sub(domain, segs):
    # a scalar segment is (tag, bytes); a list segment is (tag, [bytes, ...])
    h = blake3(b"emem.preimage.v1\x00" + len(domain).to_bytes(4, "little") + domain)
    for tag, data in segs:
        if isinstance(data, list):
            h.update(bytes([tag]) + len(data).to_bytes(4, "little"))
            for item in data:
                h.update(len(item).to_bytes(4, "little") + item)
        else:
            h.update(bytes([tag]) + len(data).to_bytes(4, "little") + data)
    return h

def verify(receipt, pubkey_b32):
    pv = receipt["preimage_version"]
    assert pv in (1, 2), "legacy receipt: use POST /v1/verify_receipt"
    assert not (receipt.get("scope") or receipt.get("edge_cids")), "use POST /v1/verify_receipt"
    segs = [(0x01, receipt["request_id"].encode()), (0x02, receipt["served_at"].encode())]
    if receipt.get("source_versions"):
        cbor = cbor2.dumps(dict(sorted(receipt["source_versions"].items())))
        segs.append((0x06, blake3(cbor).hexdigest().encode()))
    segs.append((0x07, receipt["primitive"].encode()))
    segs.append((0x08, [c.encode() for c in receipt.get("cells", [])]))
    segs.append((0x09, [c.encode() for c in receipt.get("fact_cids", [])]))
    if receipt.get("field"):
        f = receipt["field"]
        fh = sub(b"field", [(1, f["aoi_cid"].encode()), (2, f["derivation_cid"].encode())])
        segs.append((0x0a, fh.hexdigest().encode()))
    if pv == 2:
        p = receipt.get("merkle_proof")
        if p:
            mh = sub(b"merkle", [(1, bytes(p["root"])), (2, p["leaf_index"].to_bytes(4, "little")),
                                 (3, b"".join(bytes(x) for x in p["path"])),
                                 (4, bytes([p.get("version", 0)]))])
        else:
            mh = sub(b"merkle", [(5, b"")])
        segs.append((0x0b, mh.hexdigest().encode()))
    digest = sub(b"receipt", segs).digest()
    Ed25519PublicKey.from_public_bytes(b32decode(pubkey_b32)).verify(bytes(receipt["signature"]), digest)
    return True

BASE = "https://emem.dev"
pubkey_b32 = httpx.get(f"{BASE}/.well-known/emem.json").json()["responder"]["pubkey_b32"]
rec = httpx.post(f"{BASE}/v1/recall", json={"cell": "defi.zb493.zezo.zcb35",
                 "bands": ["weather.temperature_2m"]}, timeout=60).json()
print(verify(rec["receipt"], pubkey_b32))
```

The plugin's `emem-verify-receipt` skill ships the same check as a
script (`verify.py`). The script has exit codes and clearer errors, and
it needs only Python and the files next to it.

### Browser (what `/verify` does)

`https://emem.dev/verify` checks a receipt signature and the author of a
signed message in your browser. It does not send them to a server. It
uses a bundled copy of `@noble/hashes` 1.5.0 and `@noble/curves` 1.6.0
(`/emem-verify-core.js`). Paste a fact CID, an `emem:` token or a
`/memories/` path.

### When to use

- You do not trust the responder that served the receipt.
- You keep receipts in a cache and must prove later that they are true.
- Your user needs an answer that the responder cannot deny later.

---

## 3. `find-similar-places`: given a place, return neighbours by embedding

`/v1/find_similar` ranks cells by cosine similarity. It uses a stored
128-D surface-texture embedding (`geotessera`, 2024 vintage by default).
It returns up to `k` neighbours, each with cell64, lat/lng, a cached
place label and a score. **emem.dev retired the embedding band.** The
responder still answers from stored vectors, but it computes no new
ones. A seed with no stored vector returns `cid_not_found`. A
`/v1/recall` for the band returns `band_retired_at_this_responder`. A
retry changes neither result.

```sh
BASE=https://emem.dev
CELL=$(curl -sf -X POST $BASE/v1/locate -H 'content-type: application/json' \
  -d '{"q":"Bengaluru, India"}' | jq -r '.cell64')

curl -s -X POST $BASE/v1/find_similar -H 'content-type: application/json' \
  -d "{\"key\":\"$CELL\",\"k\":12}" \
  | jq 'if .code then {code, message}
        else [.neighbors[] | {cell, score, place: .place_label_cached}] end'
```

On 7 November 2026 this returned 11 neighbours for `k: 12`. Ten were
other cells of Bengaluru, with scores from 0.93 to 0.86.

### Pitfalls

- To test coverage, call find_similar directly. `/v1/recall` on the
  retired band can return nothing for a seed that find_similar still
  answers.
- The similarity is surface texture (vegetation, urban density, water).
  It is not climate or society. The top neighbours of a city are often
  other cells of the same city. Ask for a larger `k` to reach other
  places.
- `band: "geotessera.2020"` (2017 to 2024) selects a vintage.
  `mode: "hamming"` uses the binary index.

---

## 4. `scene-thumbnail`: visual preview

`GET /v1/cells/{cell64}/scene.png` returns a small true-colour
Sentinel-2 chip at a cell. `GET /v1/scene.png?bbox=w,s,e,n` crops a
scene to a plot at native 10 m, up to 1024 px a side. The
`x-emem-scene-*` response headers give the scene id, capture time,
cloud cover and CRS.

```sh
BASE=https://emem.dev
curl -sf -D headers.txt -o plot.png \
  "$BASE/v1/scene.png?bbox=75.80,30.90,75.804,30.903&datetime=2025-10-01/2025-10-31&max_cloud=30"
grep -i '^x-emem-scene' headers.txt
```

On 7 November 2026 this returned scene
`S2C_MSIL2A_20251028T053951_R005_T43REQ_20251028T084920` with 1.96 %
cloud. Without `datetime` and `max_cloud`, the same bbox returned
502 `scene_upstream_failed` ("no valid red pixels"). If you get that
error, give a date window or raise `max_cloud`.

A picture is a view, not a signed artifact. For pixels that a third
party can hash again, use `POST /v1/band_raster` (the
`emem-field-tokens` skill).

---

## 5. `recall-many-cells`: batch a recall across N places

`POST /v1/recall_many` takes a list of cell64s and bands. It returns
the facts and a receipt for each cell. The limit is 256 cells for each
call. Give `budget_ms` to get a partial answer instead of a timeout. A
partial answer has `converged: false` and a typed `pending[]` list.

```sh
BASE=https://emem.dev
curl -sf -X POST $BASE/v1/recall_many -H 'content-type: application/json' \
  -d '{
    "cells": ["defi.zb493.zezo.zcb35", "defi.zb592.nUkO.zEzE"],
    "bands": ["weather.temperature_2m"],
    "budget_ms": 15000
  }' | jq '{converged, by_cell: [.by_cell | to_entries[] | {cell: .key, facts: (.value.facts | length)}]}'
```

`facts` counts every stored reading of the band at the cell, not only
the latest. On 7 November 2026 the first cell had 13
`weather.temperature_2m` facts.

---

## 6. `area-recall`: recall across an area

`POST /v1/recall_polygon` samples cells inside an area and recalls the
bands at each cell. Give one of these fields:

| Field | What it is |
|---|---|
| `place` | Free text, for example a region name. |
| `polygon_bbox` | An object with named corners. The responder refuses an array (400 `invalid_argument`). |
| `polygon_geojson` | A GeoJSON polygon. It sets the area and masks the sample to its boundary. `geojson` and `geometry_geojson` are aliases. |

The response has `by_cell`, `cells_sampled`, `coverage_fraction`,
`area_km2` and a receipt for each cell.

```sh
BASE=https://emem.dev
curl -sf -X POST $BASE/v1/recall_polygon -H 'content-type: application/json' \
  -d '{
    "polygon_bbox": {"min_lat": 12.95, "max_lat": 13.05, "min_lng": 77.55, "max_lng": 77.65},
    "bands": ["copdem30m.elevation_mean"], "max_cells": 16, "budget_ms": 20000
  }' | jq '{cells_sampled, facts_returned, converged, coverage_fraction, area_km2}'
```

On 7 November 2026 this sampled 16 cells over 120.05 km2, and
`coverage_fraction` was 0.0000133. A sample is not the area. Give
`coverage_fraction` with any mean or total that you compute. There is
no receipt for a total, only one receipt for each cell.

---

## 7. `trajectory-over-time`: time series at one cell

`POST /v1/trajectory` returns the stored readings for one cell and band
over a window. Each item of `series[]` has these fields:

| Field | What it is |
|---|---|
| `tslot` | The time slot of the reading. |
| `slot_start` | The YYYY-MM-DD on which the band's tempo slot starts. The fact itself carries the scene's capture date. |
| `value` | The reading. |
| `fact_cid` | The content id of the signed fact. |
| `token` | The `emem:fact:` handle for the point. |

Trajectory does not fetch past readings that the responder did not
store. `POST /v1/backfill` does that.

```sh
BASE=https://emem.dev
curl -sf -X POST $BASE/v1/trajectory -H 'content-type: application/json' \
  -d '{"cell":"defi.zb493.zezo.zcb35","band":"indices.ndvi",
       "from_date":"2024-01-01","to_date":"2026-09-30"}' \
  | jq '{points: (.series | length), series: [.series[] | {slot_start, value, token}]}'
```

On 7 November 2026 this returned 8 points. The first had `slot_start`
2025-10-09. You can use `window: [start_tslot, end_tslot]` in place of
the two dates.

---

## 8. `field-signals`: one farm field

```sh
BASE=https://emem.dev
curl -sf -X POST $BASE/v1/field_boundaries -H 'content-type: application/json' \
  -d '{"polygon_bbox":{"min_lat":30.90,"max_lat":30.903,"min_lng":75.80,"max_lng":75.804},"clean":true}' \
  > fields.json
jq '{geometry_geojson: .geojson.features[0].geometry, start: "2025-10-01", end: "2025-11-30"}' fields.json \
  | curl -sf -X POST $BASE/v1/field_burn_scar -H 'content-type: application/json' -d @- \
  | jq '{verdict, burn_events, observations}'

curl -sf -X POST $BASE/v1/field_actual_et -H 'content-type: application/json' \
  -d '{"geometry_geojson":{"bbox":[77.03,32.57,77.036,32.573]},"start":"2026-05-01","end":"2026-06-30"}' \
  | jq '{verdict, et_mm, et_m3_per_ha, coverage}'
```

`field_burn_scar` gives one of these verdicts: `burn`, `signal_only`,
`no_burn` or `inconclusive`. `inconclusive` means too few clear
Sentinel-2 dates. Try again after the warm-up. `field_actual_et` sums
MODIS MOD16 8-day composites over one 463 m pixel. The result is the
water that the crop used, not the irrigation applied. Both calls return
the fact cids that they used and a receipt.

On 7 November 2026 the burn call gave `signal_only` with 0 burn events
over 16 cells. The ET call gave `complete`, 148.04 mm
(1480.38 m3/ha), with coverage 1.0.

---

## 9. `eudr-due-diligence`: a signed EUDR statement for a plot

`POST /v1/eudr_dds` evaluates plots against Regulation (EU) 2023/1115:
JRC GFC2020 forest at the 2020-12-31 cut-off plus Hansen GFC loss year,
per sampled cell, with a JRC TMF cross-check reported beside the
verdict. It returns an Annex II shaped envelope with a per-plot verdict
(`pass`, `fail`, `not_in_scope`, `indeterminate`, `below_mmu`), signed
fact cids for every per-cell verdict, and explicit legality and
degradation disclaimers.

```sh
BASE=https://emem.dev
curl -sf -m 300 -X POST $BASE/v1/eudr_dds -H 'content-type: application/json' \
  -d '{"operator":{"name":"Example Importer"},
       "plots":[{"plot_id":"CIV-001",
                 "geometry_geojson":{"type":"Point","coordinates":[-5.5471,6.8276]},
                 "country_of_production":"CIV","commodity_hs":"180100","quantity_kg":1200}]}' \
  | jq '{verdict: .due_diligence_statement.verdict, forest_baseline_dataset,
         review_required: .traces_nt_envelope.statementOfComplianceReviewRequired,
         plots: [.per_plot_results[] | {plot_id, verdict, support: .verdict_support.level,
                                        tmf_agreement: .tmf_cross_check.agreement}]}'
```

A cold plot can take minutes: warm it with a recall of
`jrc_gfc2020.forest_2020`, `forest_change.lossyear` and
`forest_change.treecover2000` on one cell first. The schema is at
`GET /v1/schemas/eudr_dds.json`; the plugin's `emem-eudr-due-diligence`
skill walks the response.

Every sampled cell's facts are stored. Each plot's `per_cell_rows_cid`
is blake3 hex over one line per cell,
`<cell>|<verdict>|<fact_cids joined by ,>\n`. Send
`per_cell_detail: "anchor"` to leave the rows out and keep the cid, or
`rows` (default) to inline them. Page the rows with `per_cell_offset`
and follow `per_cell_next_offset` until it is `null`. Coverage is
`sampled_polygon_fraction`, never a count of rows. Read the top-level
`statement_of_compliance_signable` before you offer a statement for
signature.

---

## 10. `document-evidence`: OCR and parse, every step signed

```sh
BASE=https://emem.dev
base64 -w0 report.png | jq -Rs '{image_b64: ., lang: "eng"}' \
  | curl -sf -X POST $BASE/v1/ocr -H 'content-type: application/json' -d @- > ocr.json
jq '{ocr: .}' ocr.json \
  | curl -sf -X POST $BASE/v1/lab_report_parse -H 'content-type: application/json' -d @- \
  | jq '{verdict: .result.verdict, rows: [.result.rows[] | {analyte, result_mg_kg, mrl_mg_kg, exceeds_printed_mrl}]}'
```

The OCR receipt (`emem.ocr.v1`) signs which image, engine and language
produced the text; the parse receipt (`emem.doc_parse.v1`) signs the
text hash and the result hash. `POST /v1/land_record_parse` reads
owners, parcel ids, areas, places and dates from land records the same
way. The OCR text is a reading, not a fact: check it before trusting a
parsed verdict.

---

## 11. `transparency-log-audit`: prove the history only grew

```sh
BASE=https://emem.dev
curl -sf $BASE/v1/log/sth > old_sth.json                       # pin a head
curl -sf "$BASE/v1/log/inclusion?leaf_index=12345" | jq '{leaf_index, tree_size, root_b32}'
curl -sf $BASE/v1/log/sth > new_sth.json                       # later
OLD=$(jq .sth.tree_size old_sth.json); NEW=$(jq .sth.tree_size new_sth.json)
curl -sf "$BASE/v1/log/consistency?first=$OLD&second=$NEW" | jq '{first_size, second_size, proof: (.consistency_proof_b32 | length)}'
```

RFC 6962 over BLAKE3: leaf `blake3(0x00 || entry_hash)`, node
`blake3(0x01 || left || right)`. The STH is signed over
`PreimageV1("emem.translog.sth.v1")`. The plugin's
`emem-transparency-log` skill ships `verify_log.py`, which checks all
three offline. `GET /v1/log/witnesses` lists independent
co-signatures.

---

## 12. `heat-solve-at-a-cell`: a 2-D heat-equation forecast

`POST /v1/heat_solve` runs an explicit finite-difference heat equation
over the cell's 3 x 3 MODIS land-surface-temperature stencil and returns
the centre's forecast, with the input fact cids and the algorithms and
bands manifest cids a peer needs to re-run it bit for bit.

```sh
BASE=https://emem.dev
curl -sf -X POST $BASE/v1/heat_solve -H 'content-type: application/json' \
  -d '{"cell": "defi.zb493.zezo.zcb35", "hours_ahead": 24}' \
  | jq '{n_steps, hours_ahead, forecast_k, forecast_unit, delta_k, input_fact_cids}'
```

`hours_ahead` is capped at 168; `diffusivity_m2_per_s` defaults to 1e-6
(urban surfaces).

---

## 13. `dashboard-tile`: an area over time, with its uncertainty and its evidence

A tile about a field, a plot or a catchment should not be one pixel's line.
`POST /v1/field_series` reads every pinned Sentinel-2 overpass in the window
over the polygon, masks each pixel by its scene classification, and gives per
scene the share of the area the sky let through (`clear_fraction`) and the
distribution over the clear pixels inside it. Chart the `median` with a
`p10`..`p90` band, draw `quality: partial` points lighter, and list
`excluded` scenes as gaps with their reason. The scenes, mask, formula and
quantile rule are pinned in the signed record behind `token`. For years
before Sentinel-2, send `"sensor":"landsat"`: the same rows from Landsat 8
and 9 at 30 m, from 2013, signed as `landsat_index_series@1`.

```sh
BASE=https://emem.dev
curl -sf -X POST $BASE/v1/field_series -H 'content-type: application/json' \
  -d '{"geometry":{"type":"Polygon","coordinates":[[[75.738,30.948],[75.748,30.948],[75.748,30.957],[75.738,30.957],[75.738,30.948]]]},
       "index":"ndvi","start_date":"2023-10-01","end_date":"2026-09-30"}' \
  | jq '{token, estimand: .estimand.statistic,
         rows: [.rows[] | [.date, .median, .p10, .p90, .clear_fraction, .quality]],
         excluded: [.excluded[] | [.date, .reason]],
         anomaly, trend: .trend.slope_per_year, cautions}'
```

What the tile states beside the line, each already computed:

- `anomaly`: the latest scene against earlier years' scenes within 30 days of
  its day of year. This is the number for "is it greener than usual", because
  a seasonal index compared across seasons measures the season.
- `trend`: Theil-Sen slope with Kendall's S and p, seasonal cycle NOT removed.
  `cautions` says when the seasonal swing dwarfs it.
- `change_map`: GeoJSON blocks, latest against its same-season partner
  (`pairing`), ready for a map layer.
- `estimand`: support (10 m pixels inside the polygon), statistic, operator,
  mask, unit. Put it in the tile's footnote.

**Several fields on one chart.** Send `areas: [{id, geometry | bbox}]`, up to
6, in place of `geometry`. Each area is its own signed series with its own
`token`, and `chart_rows` carries every area's scenes in one table (`date`,
`area`, `median`, `p10`, `p90`, `clear_fraction`, `quality`), sorted by date.
Draw one line per area. Areas have their own scene dates, so do not join them
on date. Over MCP, `emem_intent {type: "area_over_time", areas, start_date,
end_date}` takes the same list, and a dashboard card can call it as its live
query.

**Indices.** `index` is `ndvi` (default), `ndwi`, `ndmi` (`lswi` is an
alias), or `evi`, all from Sentinel-2. `vv`, `vh` and `vh_vv` read
Sentinel-1 backscatter in dB on one relative orbit. Pass `orbit` to pick
it; the default is the orbit with the most scenes. Never join two orbits
in one line. A radar row has no `clear_fraction`. It carries `mean` (10
log10 of the mean linear power), `se_db`, `pixel_enl`,
`below_nesz_share`, `bright_rejected`, `n_valid` and `valid_fraction`,
and the trend is on `median`. Over an area, a radar answer also has
`blocks`, `change_map` and `anomaly`, in dB. A block's value is the
median of per-pixel dB, the change map pairs scenes on the one orbit,
and `blocks.n_px` gives each block's pixel count. Speckle makes a small
block noisy, so read `cautions` before you colour a map by it.

**Pixel rules.** `inner_buffer_m` (0 to 60, default 0) counts a pixel
only when its centre is that far inside the edge. `min_support_px` (4 to
30, default 30) is the fewest pixels a scene may rest on. Each answer
carries `polygon_cid`, an id over the outline rounded to about 1 cm,
beside `aoi_cid`, the bbox hash the receipt binds.

**Many fields in one window.** Send `fields: [{id, geometry}]`, up to
500, in place of `geometry`. Each scene is read once over the fields'
joint box, which must fit in 512 px a side. The `fields` block has one
row per field and scene.

**Scene check (optical only).** A scene the cloud mask passed, but both
neighbouring scenes refute in the same direction, gets
`quality: "suspect"` and a `scene_check` object. It stays in the rows
and is left out of the trend. In a `fields` answer the rows get a
`suspect` column, and `suspect_dates` lists the dates where a large
share of the fields failed the check. Radar takes no scene check,
because a sudden drop there is often a flood.

For one cell's stored readings, `POST /v1/trajectory` returns rows with
`slot_start`, `value` and an `emem:fact:` `token` per point. Over MCP both are
`emem_field_series` and `emem_trajectory`, callable by name through `tools/call`.

---

## 14. `storyboard`: frames an animated explainer can play, each one checkable

Two kinds of frame come back signed, so every scene of an animation cites what
it shows.

**The field, frame by frame.** Each `field_series` row carries
`block_medians`, the area cut into `blocks.per_side` squared blocks in
`blocks.order`, with each block's quad in `blocks.quads` (WGS-84, from the
inverse projection). One row is one frame: colour the quads by the medians,
title it with the row's `date` and `clear_fraction`, and end on `change_map`.
A null block was under cloud in that frame; leave it grey, never fill it in.

**The reasoning, step by step.** `POST /v1/ask` returns `reasoning.states[]`,
one `emem:state:` per stage (`located`, `routed`, `recalled`, `composed` when
a time-window recipe ran, `scored`). Each state commits to the stage before it
and to the facts it added. The `scored` state also commits to the values it
computed. Each state has `says`, one sentence that tells what the stage did.
Use it as the caption of the scene.
`GET /v1/state/{cid}` returns the bytes and recomputes the address
(`address_holds`). The same inputs give the same token, so two renders of one
explainer show exactly which step changed.

```sh
BASE=https://emem.dev
curl -sf -X POST $BASE/v1/ask -H 'content-type: application/json' \
  -d '{"question":"how green is the farmland near Ludhiana?"}' \
  | jq '{answer, scenes: [.reasoning.states[] | {stage, says, at_ms, state}]}'
```

---

## Installing as Claude Skills

The plugin at
[`plugins/emem/`](https://github.com/Vortx-AI/emem/tree/main/plugins/emem)
wires the MCP server and adds nineteen skills. Install it in Claude Code:

```sh
/plugin marketplace add Vortx-AI/emem
/plugin install emem@emem
```

Or copy the skills without the plugin:

```sh
git clone https://github.com/Vortx-AI/emem.git
mkdir -p .claude/skills
cp -r emem/plugins/emem/skills/emem-* .claude/skills/
```

| Skill | For |
|---|---|
| `emem-locate-and-recall` | a place name to a cell, then signed facts and their citation tokens |
| `emem-recall-polygon` | the same over an area, with the sampling stated |
| `emem-field-tokens` | the raster field over an area, or over time, as a signed artifact (`rehash.py`) |
| `emem-urban` | buildings, places, roads, population, built-up and water signals, and what each measures |
| `emem-field-signals` | a farm field: boundaries, residue burning, evapotranspiration, a picture |
| `emem-eudr-due-diligence` | a signed EUDR deforestation statement for plots |
| `emem-find-similar` | analogues over the frozen embedding index; read its coverage note |
| `emem-research-grade-citation` | the estimand behind a fact, its units and ranges, and a methods paragraph |
| `emem-tokenise-files` | a file cut into units under one signed Merkle root; prove one unit (`tree_proof.py`) |
| `emem-document-evidence` | OCR plus lab-report and land-record parsing, every step signed (`verify_doc.py`) |
| `emem-sign-and-attest` | write with your own key; the refusal names the bytes to sign, and the optional `emem-signer` plugin signs them locally |
| `emem-long-horizon-memory` | signed working state that survives a context reset, and the inbox |
| `emem-multi-agent-handoff` | hand findings to other agents as tokens, and check who wrote a note (`verify_note.py`) |
| `emem-shared-identity` | make two agents refer to one object, and know what each token proves |
| `emem-referential-drift` | pin a value to a citation, grade what you are about to say, ask why a number moved |
| `emem-verify-receipt` | check a receipt's signature offline (`verify.py`) |
| `emem-transparency-log` | prove the log only grew, and that an entry is in it (`verify_log.py`) |
| `emem-verify-before-publish` | check a draft's citations and the numbers beside them |
| `emem-device-traces` | resolve and re-verify a device's signed OS trace, and what enrolment admits today |

The scripts need only Python 3. Their BLAKE3 and Ed25519 verification
come from `emem_crypto.py`, which ships in each skill's `scripts/`
directory, so any one skill copied on its own still runs.

Each `SKILL.md` is readable directly in the repository under
`plugins/emem/skills/<name>/SKILL.md`, and every one above is also
served at `https://emem.dev/skills/<name>/SKILL.md`. The
self-hosting procedure for the guard, as an agent-runnable skill, is
`crates/emem-guard/SKILL.md`, also served by `GET /v1/guard/selfhost`.

## Discovery surface

- `https://emem.dev/llms.txt`: high-level summary + behavioural rules
- `https://emem.dev/openapi.json`: full machine surface (every documented REST path under /v1/*)
- `https://emem.dev/.well-known/emem.json`: manifest CIDs + responder pubkey
- `https://emem.dev/v1/agent_card`: discover-first card with band taxonomy
- `https://emem.dev/agents.md`: consumer-agent ontology + recipes
- `https://emem.dev/verify`: in-browser receipt + authorship verifier;
  paste a fact CID, an `emem:` token, or a signed message path
- `https://emem.dev/mcp`: JSON-RPC 2.0 MCP endpoint. `tools/list` advertises
  the loop that the rest of the surface serves: ground a place, cite the fact
  as `emem:fact:<cell64>:<fact_cid>`, hand that line to another agent, and let
  them resolve it to the identical signed bytes and check the receipt without
  trusting you. 116 tools in total (18 core, 98 extended);
  `https://emem.dev/mcp/full` advertises all 116, and `tools/call` reaches every
  tool by name from either, so the narrower list costs no capability. Call
  `emem_tools` to map the surface, filter it by the shape of the answer you
  need (`{"shape":"raster"}`) or by job (`{"bundle":"robotics"}`), or fetch one
  tool's schema (`{"name":"emem_ndvi"}`).

## License

Apache-2.0. All emem responses are content-addressed and verifiable;
copy them, sign your own derivatives, build whatever you want.
