# emem skills

*For agents. Machine-first: runnable procedures, not an explanation of why. A person should read https://emem.dev/how-it-works first.*

Composed recipes for AI agents building on emem, a shared, verifiable
memory for AI agents: a citeable identity layer that stops referential
drift, so different models reason from the same world object instead of
divergent descriptions, grounded in signed Earth observation. Each
recipe combines two or three `/v1/*` calls into one useful step.

This page is the cookbook view. The same procedures, in more depth and
with bundled verifier scripts, ship as an installable plugin at
[plugins/emem/](https://github.com/Vortx-AI/emem/tree/main/plugins/emem)
for Claude Code; see § Installing as Claude Skills below.

The endpoint is `https://emem.dev` (or your self-host URL). Reads need
no key; writes are Ed25519-signed by the caller and tiered by reach
(`GET /v1/enlist`). Every response that serves facts carries an Ed25519
receipt over a deterministic preimage; verify it offline with the
responder's key from `/.well-known/emem.json`.

Every fact is content-addressed and signed, so any conformant responder
returns byte-identical bytes for the same content id and any client
verifies the receipt offline. emem is a protocol, not a single endpoint.

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

---

## 1. `locate-and-recall`: name a place, get signed facts

Resolve a place name to a `cell64`, then recall one or more bands there.
`facts[]` holds every stored reading, oldest first; `current_by_band`
names the latest reading's `fact_cid` per band, and every fact carries
its own `memory_token` (`emem:fact:<cell64>:<fact_cid>`), the handle
that re-fetches the same bytes in any year. A fact_cid digests the
signer and the signing moment along with the value, so it names one
attestation rather than the observation behind it: a second responder
measuring the same thing mints a different cid. Use `emem:entity:` for
identity that crosses responders.

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

- `facts[0]` is the oldest reading, not the current one.
- When `/v1/locate` returns `disambiguation_required: true`, read
  `alternatives` and ask which place was meant rather than citing rank 0.
- A cold cell triggers a fetch from the upstream connector. A
  `materialize_notes[]` entry with `reason_class: "timeout"` and
  `retryable: true` means the fetch continues in the background; the
  same call a few seconds later usually answers warm.

---

## 2. `verify-receipt-offline`: BLAKE3 + Ed25519, no callback

Every receipt verifies without re-contacting the responder. Rebuild the
canonical preimage, BLAKE3 it, then `ed25519.verify(signature, digest,
pubkey)`. The byte layout lives in `crates/emem-attest`
(`receipt_preimage_v1`, `receipt_preimage_v2`); `GET /v1/verifier_spec`
publishes it from the compiled constants.

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

`seg(tag, bytes)` = `tag || u32le(len) || bytes`; `seg_list` writes a
u32le count, then `u32le(len) || bytes` per item. Bracketed segments
appear only when present (`manifest_hex` = lowercase hex of blake3 over
the CBOR map of a non-empty `source_versions`, keys in plain string
order). `merkle_hex` is always written under v2: the hex digest of
`PreimageV1("merkle")` over the receipt's `merkle_proof` (root,
u32le leaf_index, concatenated path, rule version) or, when there is no
proof, over an explicit absence marker, so a stripped proof breaks the
signature. Receipts with `preimage_version: 1` omit segment `0x0b`;
receipts with no `preimage_version` use the legacy pipe rule, which
`POST /v1/verify_receipt` still checks.

### python (fully offline)

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
script (`verify.py`) with exit codes and clearer errors.

### Browser (what `/verify` does)

`https://emem.dev/verify` imports `@noble/curves` and `@noble/hashes`
and verifies, locally in your browser, a receipt's signature and a
signed message's authorship. Paste a fact CID, an `emem:` token, or a
`/memories/` path.

### When to use

- You don't trust the responder you fetched from.
- You're caching receipts and want to prove they're authentic later.
- Your user wants a non-repudiable answer.

---

## 3. `find-similar-places`: given a place, return neighbours by embedding

`/v1/find_similar` ranks cells by cosine over a stored 128-D
surface-texture embedding (`geotessera`, 2024 vintage by default) and
returns top-K neighbours with cell64, lat/lng, cached place labels and
scores. **The embedding band is retired on emem.dev**: stored vectors
still answer, and none are computed. A seed with no stored vector
returns `cid_not_found`, and asking `/v1/recall` for the band returns
`band_retired_at_this_responder`; retrying changes neither.

```sh
BASE=https://emem.dev
CELL=$(curl -sf -X POST $BASE/v1/locate -H 'content-type: application/json' \
  -d '{"q":"Bengaluru, India"}' | jq -r '.cell64')

curl -s -X POST $BASE/v1/find_similar -H 'content-type: application/json' \
  -d "{\"key\":\"$CELL\",\"k\":12}" \
  | jq 'if .code then {code, message}
        else [.neighbors[] | {cell, score, place: .place_label_cached}] end'
```

### Pitfalls

- Call find_similar directly to test coverage. `/v1/recall` on the
  retired band can come back empty for a seed find_similar still
  answers.
- The similarity is surface texture (vegetation, urban density, water),
  not climate or society. The top neighbours of a city are often other
  cells of the same city; ask for a larger `k` to reach elsewhere.
- `band: "geotessera.2020"` (2017 to 2024) picks a vintage, and
  `mode: "hamming"` uses the binary index.

---

## 4. `scene-thumbnail`: visual preview

`GET /v1/cells/{cell64}/scene.png` returns a small true-colour
Sentinel-2 chip at a cell; `GET /v1/scene.png?bbox=w,s,e,n` crops one
to a plot at native 10 m, up to 1024 px a side. The `x-emem-scene-*`
headers name the scene, capture time, cloud cover and CRS.

```sh
BASE=https://emem.dev
curl -sf -D headers.txt -o plot.png "$BASE/v1/scene.png?bbox=75.80,30.90,75.804,30.903"
grep -i '^x-emem-scene' headers.txt
```

A picture is a view, not a signed artifact. For pixels a third party can
re-hash, use `POST /v1/band_raster` (the `emem-field-tokens` skill).

---

## 5. `recall-many-cells`: batch a recall across N places

`POST /v1/recall_many` takes a list of cell64s and bands and returns
per-cell facts with per-cell receipts. Cap is 256 cells per call. Pass
`budget_ms` for a partial answer (`converged: false`, typed `pending[]`)
instead of a timeout.

```sh
BASE=https://emem.dev
curl -sf -X POST $BASE/v1/recall_many -H 'content-type: application/json' \
  -d '{
    "cells": ["defi.zb493.zezo.zcb35", "defi.zb592.nUkO.zEzE"],
    "bands": ["weather.temperature_2m"],
    "budget_ms": 15000
  }' | jq '{converged, by_cell: [.by_cell | to_entries[] | {cell: .key, facts: (.value.facts | length)}]}'
```

---

## 6. `area-recall`: recall across an area

`POST /v1/recall_polygon` samples cells inside an area and recalls the
bands at each. Pass `place` (free text) or `polygon_bbox` (an object
with named corners; arrays are refused), optionally with
`polygon_geojson` to mask to a real boundary. Returns `by_cell`,
`cells_sampled`, `coverage_fraction`, `area_km2` and per-cell receipts.

```sh
BASE=https://emem.dev
curl -sf -X POST $BASE/v1/recall_polygon -H 'content-type: application/json' \
  -d '{
    "polygon_bbox": {"min_lat": 12.95, "max_lat": 13.05, "min_lng": 77.55, "max_lng": 77.65},
    "bands": ["copdem30m.elevation_mean"], "max_cells": 16, "budget_ms": 20000
  }' | jq '{cells_sampled, facts_returned, converged, coverage_fraction, area_km2}'
```

A sample is not the area: report `coverage_fraction` with any
aggregate, and note that there is no aggregate receipt, only per-cell
ones.

---

## 7. `trajectory-over-time`: time series at one cell

`POST /v1/trajectory` returns the stored readings for one cell and band
over a window, as `series[]` of `{tslot, value, fact_cid}`. It does not
fetch past readings that were never stored; `POST /v1/backfill` does.

```sh
BASE=https://emem.dev
curl -sf -X POST $BASE/v1/trajectory -H 'content-type: application/json' \
  -d '{"cell":"defi.zb493.zezo.zcb35","band":"indices.ndvi",
       "from_date":"2024-01-01","to_date":"2026-09-30"}' \
  | jq '{points: (.series | length), series: [.series[] | {tslot, value}]}'
```

`window: [start_tslot, end_tslot]` works in place of the two dates.

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

`field_burn_scar` answers `burn`, `signal_only`, `no_burn` or
`inconclusive` (too few clear Sentinel-2 dates; retry after the
warm-up). `field_actual_et` sums MODIS MOD16 8-day composites over one
463 m pixel; it is water used, not irrigation applied. Both return the
fact cids they used and a receipt.

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
| `emem-sign-and-attest` | write with your own key; the refusal names the bytes to sign (`sign_write.py`) |
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
  trusting you. 114 tools in total (18 core, 96 extended);
  `https://emem.dev/mcp/full` advertises all 114, and `tools/call` reaches every
  tool by name from either, so the narrower list costs no capability. Call
  `emem_tools` to map the surface, filter it by the shape of the answer you
  need (`{"shape":"raster"}`) or by job (`{"bundle":"robotics"}`), or fetch one
  tool's schema (`{"name":"emem_ndvi"}`).

## License

Apache-2.0. All emem responses are content-addressed and verifiable;
copy them, sign your own derivatives, build whatever you want.
