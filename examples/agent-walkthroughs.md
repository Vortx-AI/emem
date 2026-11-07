# Agent walkthroughs

This page lists questions that an AI agent gets, and the emem calls that
answer them. Every call goes to `https://emem.dev`. To use your own node,
change the URL.

## What you need

- `curl` and `jq`.
- Network access to `https://emem.dev`. No key and no account. All calls on
  this page are reads.

## What you get

- One working command for each of nine common questions.
- The field to read in each response, and the output it gave on
  7 November 2026.
- A table that tells you which call to make first.

Every command ran against emem.dev on 7 November 2026. No CI job runs this
page. If a command fails for you, open an issue.

Each block uses a literal `cell64`, so you can run it alone. In your own code,
take the `cell64` from the `locate` call.

## 1. Read the facts at a latitude and longitude

```bash
# Get the cell64 for the point. Every agent gets the same cell64.
curl -s -X POST https://emem.dev/v1/locate \
  -H 'content-type: application/json' \
  -d '{"lat":35.3606,"lng":138.7274}' | jq -r '.cell64'
# defi.zb592.nemu.zEvE

# Read the signed facts at that cell. On a miss, emem fetches and signs them.
curl -s -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb592.nemu.zEvE","bands":["copdem30m.elevation_mean"]}' | jq '.facts[0] | {value, unit, memory_token}'
# value 3768.409912109375, unit "m" (7 November 2026)
# memory_token "emem:fact:defi.zb592.nemu.zEvE:pi3crztqnjaa2ntnjzebec2p4db3th2khc2xet6doovphnq2kgya"
```

Cite the `memory_token`, not only the number. Any reader can resolve the token
to the same signed bytes and check the receipt offline.

## 2. Read a value at a named place

```bash
curl -s -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"place":"Mount Everest","bands":["copdem30m.elevation_mean"]}' | jq '.facts[0] | {cell, value, unit}'
# cell "defi.zb53e.rIkI.seni", value 8724.0, unit "m" (7 November 2026)
```

`recall` accepts a `place`. The name resolves to one cell of about 10 m. If
the name is ambiguous, `recall` does not guess. It returns the error code
`no_geocoder_match`, the best candidate, and a remedy. On 7 November 2026,
`"Everest Base Camp"` and `"Springfield"` both gave this error. Then call
`/v1/locate`, choose the candidate, and pass its `cell64`. For an area, see
section 7.

## 3. Check a claim: is Everest Base Camp above 5,000 m?

`locate` with `"Everest Base Camp"` gives the cell `defi.zb53e.wUdA.rIhe`.
Read the band at that cell first. `verify` checks only facts that the cell
already holds.

```bash
curl -s -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb53e.wUdA.rIhe","bands":["copdem30m.elevation_mean"]}' | jq '.facts[0].value'
# 5258.5595703125 (7 November 2026)

curl -s -X POST https://emem.dev/v1/verify \
  -H 'content-type: application/json' \
  -d '{"cell":"defi.zb53e.wUdA.rIhe","claim":{"band":"copdem30m.elevation_mean","op":"gt","value":5000.0,"tslot":0}}' | jq '{verdict, evidence}'
# verdict true, evidence ["wc7jgs3lru3owdfpidlsdkusbdmojczcm746yjggggt4pmng3ira"]
```

The claim uses `"tslot":0` because elevation is a static band. Its fact has
tslot 0, as the `recall` response shows.

`verdict` is `true` or `false`. `evidence` lists the fact ids that the
verdict used. Read `evidence` before you report a `false`:

| `verdict` | `evidence` | Meaning |
|---|---|---|
| `true` | one or more fact ids | The signed facts support the claim. |
| `false` | one or more fact ids | The signed facts do not support the claim. |
| `false` | `[]` | The cell holds no fact for that band. `verify` did not check the claim. |

`"mode":"resolve"` asks `verify` to fetch a missing fact. On 7 November 2026
it returned `cid_not_found` for `copdem30m.elevation_mean` at a new cell, so
call `recall` first, as above.

## 4. Compare two places

The two cells below are in Madrid (`a`) and Lisbon (`b`).

```bash
curl -s -X POST https://emem.dev/v1/compare \
  -H 'content-type: application/json' \
  -d '{"a":"defi.zb5cb.zda5f.nEqI","b":"defi.zb5b8.qIgO.pIho"}' | jq '{shared_bands, per_band, only_a, only_b}'
# shared_bands: copdem30m.elevation_mean, indices.ndvi, weather.temperature_2m
# per_band."copdem30m.elevation_mean": -645.78 (7 November 2026)
```

For a scalar band, `per_band` gives `b` minus `a`. Here Lisbon is 645.78 m
lower than Madrid: 2.99 m against 648.77 m.

`compare` uses only the bands that both cells already hold. `only_a` and
`only_b` list the bands that one cell holds and the other does not. If
`shared_bands` is short, recall the bands you need at both cells, then
compare again.

## 5. Find places like a given cell

```bash
curl -s -X POST https://emem.dev/v1/find_similar \
  -H 'content-type: application/json' \
  -d '{"key":"defi.zb5d8.hAgU.pIxO","k":5}' | jq '.neighbors[] | {cell, band_used, score, place_label_cached}'
# 4 neighbours on 7 November 2026. The first: defi.zb5d8.hAgU.pOgU,
# band_used "geotessera", score 0.833, "Carlisle, Iowa, United States"
```

Each neighbour names the band that gave its score. The `geotessera` band is a
surface-texture embedding from Sentinel-1 and Sentinel-2. emem.dev no
longer makes new vectors for it, so the index holds only the vectors that it
already stored. You can get fewer neighbours than `k`, as here.

Read `interpretation` before you say that two places are alike. Its
`does_not_mean` field says that a high score does not mean a similar
climate, biome or latitude. For climate, its `for_climate_proximity_use`
field names the call to make: `recall` with the `koppen` band, or
`compare_bands`.

## 6. Show how an area changed over time

```bash
curl -s -X POST https://emem.dev/v1/field_series \
  -H 'content-type: application/json' \
  -d '{"geometry":{"type":"Polygon","coordinates":[[[-5.3012,6.8501],[-5.2988,6.8501],[-5.2988,6.8524],[-5.3012,6.8524],[-5.3012,6.8501]]]},"index":"ndvi","start_date":"2025-01-01","end_date":"2026-09-30"}' | jq '{rows: (.rows | length), anomaly, cautions}'
# rows 15, anomaly null, and three cautions (7 November 2026)
```

The response is one signed series for the area. It holds one row for each
clear scene (`rows`), a trend (`trend`), an anomaly against earlier years
(`anomaly`), and a change map (`change_map`).

Read `cautions` before you report a change. On 7 November 2026, `anomaly` was
`null` for this polygon. The cautions said that no earlier-year scene in the
same season was clear enough to compare.

From a plain question, `/v1/intent` with `"type":"area_over_time"` sends the
request to this endpoint.

## 7. Get the mean of a band over several cells

```bash
curl -s -X POST https://emem.dev/v1/query_region \
  -H 'content-type: application/json' \
  -d '{"geometry":"cells:defi.zb5cb.zda5f.nEqI,defi.zb5b8.qIgO.pIho","bands":["copdem30m.elevation_mean"],"agg":"mean"}' | jq '.aggregates'
# {"copdem30m.elevation_mean": 321.9013770903458} (7 November 2026)
```

The mean uses a cos-latitude weight. It is not the plain mean of the two
values (325.88 on 7 November 2026). `cos_lat_weighting_applied` in the
response tells you if the mean used this equal-area weight. It was `true`
here.

For a named area instead of a list of cells, use `POST /v1/recall_polygon`
or `POST /v1/grid`. Each one samples cells in the area and combines their
values in one call.

## 8. Ask when you do not know which call to make

```bash
curl -s -X POST https://emem.dev/v1/ask \
  -H 'content-type: application/json' \
  -d '{"q":"how is the vegetation doing around Manaus?"}' \
  | jq '{answer, shown: .band_observations_summary.bands_shown, cited: (.receipt.fact_cids|length)}'
# answer starts "At Manaus, Amazonas, Brazil, in the one cell that name resolves to:
# greenness (NDVI) 0.51 ...". On 7 November 2026: "shown": 12, "cited": 768
```

`ask` sends a plain-language question to the correct path. It returns an
answer and the ids of the facts it read. The answer gives the age of each
reading. The answer shows 12 bands that match the question
(`band_observations_summary.bands_shown`). The receipt cites every fact
held at the cell, 768 at 09:15 UTC on 7 November 2026. This count grows
as the cell gets new facts. Each entry in
`band_observations_summary.bands` has its own `fact_cid`.

Over MCP, `emem_tools` searches the full tool list when the tool you need is
not in your list.

## 9. Check a token that another agent gave you

The token below is the Everest Base Camp fact from section 3.

```bash
curl -s -X POST https://emem.dev/v1/memory_token/resolve \
  -H 'content-type: application/json' \
  -d '{"token":"emem:fact:defi.zb53e.wUdA.rIhe:wc7jgs3lru3owdfpidlsdkusbdmojczcm746yjggggt4pmng3ira"}' \
  > resolved.json
jq '{band, value, unit, cell_matches}' resolved.json
# band "copdem30m.elevation_mean", value 5258.5595703125, unit "m",
# cell_matches true (7 November 2026)

jq '{receipt}' resolved.json \
  | curl -s -X POST https://emem.dev/v1/verify_receipt \
      -H 'content-type: application/json' -d @- | jq '{valid, preimage_version}'
# valid true, preimage_version 2 (7 November 2026)
```

`resolve` returns the signed fact body and a new receipt for it. If the
token names a different valid cell, `resolve` returns HTTP 409. If the
responder does not hold the fact, it returns HTTP 404. Both codes came back
on 7 November 2026.

If the cell in the token does not parse, `resolve` finds the fact by its
`fact_cid` only. The response is HTTP 200 with `cell_matches: false` and
`degraded: true`. On 7 November 2026, the token above gave this result
when its cell ended in `rIhf`, not `rIhe`. Treat `degraded: true` as a
failure, and cite `canonical_token` from the response.

`verify_receipt` is the responder that checks its own receipt. It is not an
independent check. For an independent check, use the Python SDK or
[emem.dev/verify](https://emem.dev/verify), as
[the quickstart](../docs/quickstart.md#verify-the-receipt) shows.

## Which call to make first

| The question | First call | Then |
|---|---|---|
| A latitude and longitude, or a place name | `/v1/locate` | `/v1/recall` |
| "Is X true here?" | `/v1/recall` for the band | `/v1/verify` |
| "How does X differ from Y?" | `/v1/recall` at both cells | `/v1/compare` |
| "Find places like X" | `/v1/find_similar` | `/v1/recall` at a neighbour |
| "How has this area changed?" | `/v1/field_series` | |
| "What changed between two dates at one cell?" | `/v1/intent` with `did_change` | `/v1/diff` |
| "What is the mean over these places?" | `/v1/query_region` | |
| "I do not know which call" | `/v1/ask` | |
| "Check what another agent gave me" | `/v1/memory_token/resolve` | `/v1/verify_receipt` |

## What to put in your reply

1. State the fact that the user asked about.
2. Cite its `emem:fact:` token. For several facts, cite one `emem:bundle:`
   token. The reader can then check the facts and does not need to trust you.
3. Say what the check does not prove. A signature tells who signed a value. It
   does not tell that the value is correct.
