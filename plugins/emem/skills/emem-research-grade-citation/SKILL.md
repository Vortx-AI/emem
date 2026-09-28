---
name: emem-research-grade-citation
description: Prepares emem facts for use in research, where a reviewer needs to know exactly what a number is, how it was read, and how to reproduce it. States the estimand each fact leaves implicit (support, statistic, source pixel), reads units, the valid range enforced at signing, the reader stamp and pinned source, explains the stated ULP window on derived means and sums and what change_attribution does and does not split, and writes a methods paragraph and data-availability statement with verifiable tokens. Use when citing emem values in a paper, report, thesis or dataset, when a reviewer asks how a figure was obtained, or before comparing emem values with another dataset.
---

# emem-research-grade-citation

A signed fact proves who served which bytes. It does not by itself say
what quantity those bytes estimate. A citation a reviewer can use needs
both: the token that pins the bytes, and a sentence that names the
estimand. This skill fills in the second from the fields emem does
return, and says where the fields stop.

## What a fact carries

```sh
curl -sf -X POST https://emem.dev/v1/recall -H 'content-type: application/json' \
  -d '{"cell":"defi.zb493.zezo.zcb35","bands":["copdem30m.elevation_mean","esa_worldcover.lc_2021"]}' \
  -o facts.json
jq -c '.facts[] | {band, value, unit, observed_at, fn: .derivation.fn_key,
       args: .derivation.args, source: .sources[0].id, token: .memory_token}' facts.json
jq '.spatial_basis' facts.json
```

Recorded on 2026-09-28 in Bengaluru:

```
{"band":"copdem30m.elevation_mean","value":918.637939453125,"unit":"m","observed_at":"2021-04-30T00:00:00Z",
 "fn":"copernicus_dem_30m_aws_pixel@1","args":[12.97679089393182,77.59005965949524,"reader=cog-pixel-floor@2"],
 "source":"https://copernicus-dem-30m.s3.amazonaws.com/Copernicus_DSM_COG_10_N12_00_E077_00_DEM/...DEM.tif",
 "token":"emem:fact:defi.zb493.zezo.zcb35:gzfrgbobepbuxzgizwp2a4if6nlsgporsezb7hfthgds6pfnapka"}
```

| Field | Tells you |
|---|---|
| `value`, `unit` | the number and its unit, as signed |
| `derivation.fn_key` | the materialiser and its version (`@1`) |
| `derivation.args` | the sampled coordinates, tile or scene id, release, and the reader stamp (`reader=cog-pixel-floor@2`: which pixel a coordinate maps to) |
| `sources[]` | the upstream file or release, with a listing hash where the publisher rotates releases |
| `observed_at`, `tslot` | when the source observed it, not when emem read it (`signed_at`) |
| `spatial_basis` | that the value is a sample of the source pixel, not an average over the cell |

## Name the estimand yourself

No fact has an `estimand` or `support` field. Write it from the fields
above, one sentence per band:

> Elevation is the Copernicus DEM GLO-30 (observed 2021-04-30) value of the
> 30 m pixel containing 12.97679 N, 77.59006 E, read with
> `cog-pixel-floor@2`; it is a surface model (DSM), not bare earth.

Three things to get right:

- **Support.** A cell is about 10 m; most sources are coarser. The fact
  is the source pixel's value, so state the source resolution (30 m
  DEM, 10 m Sentinel-2 and WorldCover, 1 km WorldPop), and never read a
  difference between neighbouring cells as a gradient.
- **Statistic.** A band name like `elevation_mean` names the source
  product, not an average emem computed. Some products are themselves
  statistics (a GEDI or ATL08 canopy height is a percentile over a
  footprint); say so when you use one.
- **Time.** Use `observed_at`, and give the vintage of static layers.

## Units and valid range

`GET https://emem.dev/v1/materializers` lists each band's `valid_range`
with `enforced_at_signing: true`: a value outside it is refused at
signing, never served. On 2026-09-28 `copdem30m.elevation_mean` read
`{"min":-500.0,"max":9000.0,"unit":"m"}`. `band_metadata.value_range`
in a recall is a descriptive range and can differ; the materialiser's
range is the one the signature enforces.

## Derived values and the ULP window

A value you compute over facts goes through `POST /v1/derive`
([`emem-sign-and-attest`](../emem-sign-and-attest/SKILL.md)). For a pure
op with a pinned `code_cid` the responder recomputes it: `delta` must
match exactly, while `mean` and `sum` over more than two parents are
accepted inside a stated 4-ULP window, because floating-point order
changes the last bits. The recomputation receipt reports `rule` and the
measured `ulp_gap`. For a bit-identical claim, require `ulp_gap: 0` and
say so in the methods.

## Why a number changed

```sh
curl -sf -X POST https://emem.dev/v1/change_attribution -H 'content-type: application/json' \
  -d '{"cell":"defi.zb493.zezo.zcb35"}' -o change.json
jq '{algorithm_key, split, attribution_note, input_fact_cids, terms: (.terms|keys), ledger: .ledger_fact.token}' change.json
```

On 2026-09-28 this returned `change_attribution@1` with 5 input facts,
2 environmental evidence bands, an embedding change of 0.096 over 8
vintages, and `split: null`. It attributes change by **evidence**
(environment, sensor, geometry, encoder, noise terms, each with fact
ids), not by magnitude: no term is given a share of the delta. Do not
report one. The ledger itself is a fact with its own token, so the
evidence you relied on is citable.

## Writing it up

Methods, adapted per band:

> Values were retrieved from the emem responder (https://emem.dev,
> responder key `777er3yihgifqmv5hmc2wwmyszgddzderzhsx6rex4yoakwomvka`)
> on 2026-09-28. Each value is an Ed25519-signed fact identified by the
> `emem:fact:` token listed in Table S1; the token resolves to the
> byte-identical signed record, and its receipt verifies offline. [One
> estimand sentence per band.]

Data availability: list every token (Table S1), and for a set, one
`emem:bundle:` token with the per-fact tokens beside it (a bundle names
a set, it does not pin its members). To fix the date of the record,
pin a signed tree head and include the inclusion proofs
([`emem-transparency-log`](../emem-transparency-log/SKILL.md)).

Before submitting, resolve every token and compare every number in the
text with [`emem-verify-before-publish`](../emem-verify-before-publish/SKILL.md),
and check each receipt with
[`emem-verify-receipt`](../emem-verify-receipt/SKILL.md).

## Reproducing without emem

`sources[].id` is the upstream file and `derivation.args` the exact
coordinates, so a reviewer can re-read the pixel from the publisher
with their own tools. A signature proves emem served the value; it does
not prove the upstream product is accurate. Accuracy comes from the
product's own validation literature, which you cite as usual.
