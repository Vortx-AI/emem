---
name: emem-research-grade-citation
description: Prepares emem facts for use in research, where a reviewer must know what a number is, how emem read it, and how to reproduce it. States the estimand that each fact leaves implicit (support, statistic, source pixel). Reads the unit, the valid range enforced at signing, the reader stamp and the pinned source. Explains the stated ULP window on derived means and sums, and what change_attribution splits and does not split. Writes a methods paragraph and a data-availability statement with verifiable tokens. Use when you cite emem values in a paper, report, thesis or dataset. Also use when a reviewer asks how you got a figure, or before you compare emem values with another dataset.
---

# emem-research-grade-citation

| | |
|---|---|
| Use it when | You cite emem values in a paper, report, thesis or dataset, or a reviewer asks how you got a figure. |
| You need | A `cell64` and the bands you cite. No key. |
| You get | The fields that define each value, an estimand sentence for each band, and a methods and data-availability text with tokens. |
| Calls | `emem_recall`, then `emem_change_attribution` if a value changed (MCP). Or `POST /v1/recall`, `GET /v1/materializers` and `POST /v1/change_attribution`. |

> **Network use.** The commands in this skill send JSON to `https://emem.dev` and receive JSON from it. Where a step says so, they also receive an image or a raster file. `https://emem.dev` is the service that the MCP server of the plugin connects to. The commands do not execute what they download. This skill ships no scripts. The commands read no environment variable, key or credential. They read only local files that an earlier step saved from `https://emem.dev`.

A signed fact proves which responder served which bytes. It does not
say which quantity those bytes estimate. A reviewer needs both: the
token that pins the bytes, and a sentence that names the estimand.
This skill writes that sentence from the fields that emem returns. It
also says where those fields stop.

## Step 1: read the fields of each fact

Call `emem_recall` with this body:

```json
{"cell": "defi.zb493.zezo.zcb35", "bands": ["copdem30m.elevation_mean", "esa_worldcover.lc_2021"]}
```

Without MCP, send the same body to the REST endpoint:

```sh
curl -s -o facts.json -X POST https://emem.dev/v1/recall \
  -H 'content-type: application/json' \
  -d '{"cell": "defi.zb493.zezo.zcb35", "bands": ["copdem30m.elevation_mean", "esa_worldcover.lc_2021"]}'
jq -c '.facts[] | {band, value, unit, observed_at, fn: .derivation.fn_key,
       args: .derivation.args, source: .sources[0].id, token: .memory_token}' facts.json
jq '.spatial_basis' facts.json
```

The elevation line for this Bengaluru cell on 7 November 2026:

```
{"band":"copdem30m.elevation_mean","value":918.637939453125,"unit":"m","observed_at":"2021-04-30T00:00:00Z",
 "fn":"copernicus_dem_30m_aws_pixel@1","args":[12.97679089393182,77.59005965949524,"reader=cog-pixel-floor@2"],
 "source":"https://copernicus-dem-30m.s3.amazonaws.com/Copernicus_DSM_COG_10_N12_00_E077_00_DEM/...DEM.tif",
 "token":"emem:fact:defi.zb493.zezo.zcb35:gzfrgbobepbuxzgizwp2a4if6nlsgporsezb7hfthgds6pfnapka"}
```

This value and token were the same on 28 September 2026. The DEM is a
static layer, so the fact does not change. The WorldCover line gives
the value 10 (`lccs_class`, observed 2022-10-28) from tile `N12E075`.
Class 10 is "Tree cover" in the ESA WorldCover legend.

| Field | What it tells you |
|---|---|
| `value`, `unit` | The number and its unit, as signed. |
| `derivation.fn_key` | The materialiser and its version (`@1`). |
| `derivation.args` | The sampled coordinates, the tile or scene id, the release and the reader stamp. `reader=cog-pixel-floor@2` says which pixel a coordinate maps to. |
| `sources[]` | The upstream file or release. Where the publisher rotates releases, it also has a listing hash. |
| `observed_at`, `tslot` | When the source observed the value. `signed_at` is when emem read it. |
| `memory_token` | The `emem:fact:` token that you put in the citation. |
| `spatial_basis` | The value is a sample of the source pixel, not an average over the cell. |

On 7 November 2026, `spatial_basis` gave a cell edge of 9.55 m
(latitude) by 9.31 m (longitude) for this cell. It does not give a
resolution for each band. Take the resolution from the source product.

## Step 2: write the estimand for each band

No fact has an `estimand` or `support` field. Write one sentence for
each band from the fields in step 1. An example:

> Elevation is the Copernicus DEM GLO-30 value (observed 2021-04-30) of
> the 30 m pixel that contains 12.97679 N, 77.59006 E, read with
> `cog-pixel-floor@2`. It is a surface model (DSM), not bare earth.

Get these three items right in each sentence:

| Item | What to write |
|---|---|
| Support | The fact is the value of the source pixel. A cell is about 10 m, and most sources are coarser. State the source resolution, for example 30 m for the DEM, 10 m for Sentinel-2 and WorldCover, 1 km for WorldPop. Do not read a difference between neighbouring cells as a gradient: two cells can sample the same pixel. |
| Statistic | A band name such as `elevation_mean` names the source product. It is not an average that emem computed. Some products are statistics already. For example, a GEDI or ATL08 canopy height is a percentile over a footprint. Say so when you use one. |
| Time | Use `observed_at`. Give the vintage of each static layer. |

The `band_metadata.pitfalls` field of a fact often names a trap for the
estimand. For `copdem30m.elevation_mean` on 7 November 2026, it said
that GLO-30 is a surface model that includes building and canopy
heights.

## Step 3: read the unit and the valid range

`GET https://emem.dev/v1/materializers` lists the `valid_range` of each
band with `enforced_at_signing: true`. The responder refuses to sign a
value outside that range, so it never serves one. The list has pages
(113 materialisers on 7 November 2026). The largest page is 100
entries. The responder clamps a larger `page_size` to 100 and does not
give an error. Read both pages, then select the band:

```sh
curl -s -o mat1.json 'https://emem.dev/v1/materializers?page_size=100&page=1'
curl -s -o mat2.json 'https://emem.dev/v1/materializers?page_size=100&page=2'
jq -c '.materializers[] | select(.band == "copdem30m.elevation_mean") | .valid_range' mat1.json mat2.json
```

On 7 November 2026 this printed
`{"enforced_at_signing":true,"max":9000.0,"min":-500.0,"unit":"m"}`.

The `band_metadata.value_range` field in a recall is a different range.
It describes the band and can disagree with the enforced range. On
7 November 2026 the WorldCover fact had the value 10 and a
`band_metadata.value_range` of `[0.0, 1.0]`. The enforced range for
`esa_worldcover.lc_2021` was 0 to 100 (`class`). Cite the
materialiser range: it is the range that the signature enforces.

## Step 4: state the ULP window on derived values

You send a value that you computed over facts through `POST /v1/derive`
(see [`emem-sign-and-attest`](../emem-sign-and-attest/SKILL.md)). That
call is a signed write. For a pure operation with a pinned `code_cid`,
the responder computes the value again and compares it with your value.

| Operation | Rule in the receipt | Accepted |
|---|---|---|
| `mean` or `sum` over more than two parents | `reduction_ulp_window` | A gap of 4 ULP or less (`ulp_tolerance`). The order of floating-point additions changes the last bits. |
| `delta`, a classification, or a reduction over two parents | `canonical_float_equality` | Only the same value, bit for bit. |

The recomputation receipt always reports `rule` and the measured
`ulp_gap`, on success and on failure. If your claim must be
bit-identical, require `ulp_gap: 0` and say so in the methods.

## Step 5: explain why a number changed

Call `emem_change_attribution` with this body:

```json
{"cell": "defi.zb493.zezo.zcb35"}
```

The plugin's MCP server does not list `emem_change_attribution` in its
short tool list. The server runs it by name through `tools/call`. If
your host calls only listed tools, or has no MCP, send the same body to
the REST endpoint:

```sh
curl -s -o change.json -X POST https://emem.dev/v1/change_attribution \
  -H 'content-type: application/json' -d '{"cell": "defi.zb493.zezo.zcb35"}'
jq '{algorithm_key, split, n_env_evidence_bands, embedding: .observed.embedding.change,
     inputs: (.input_fact_cids | length), terms: (.terms | keys), ledger: .ledger_fact.token}' change.json
```

On 7 November 2026 this call returned `change_attribution@1` with these
values:

| Field | Value |
|---|---|
| `input_fact_cids` | 9 fact ids (5 on 28 September 2026) |
| `n_env_evidence_bands` | 3 |
| `observed.embedding.change` | 0.096, over 8 vintages |
| `terms` | `encoder`, `env`, `geo`, `noise`, `sensor` |
| `split` | `null` |
| `ledger_fact.token` | `emem:fact:defi.zb493.zezo.zcb35:ssd2ueqbvtnpfya4qhy6fsl4bamj5qap3mm5ll4vkirmwsd2m5jq` |

The call attributes change by evidence, not by size. Each term gives
its evidence and its fact ids. No term gets a share of the delta, so do
not report one. The `attribution_note` field says that the numeric
split is roadmap work. The ledger is a fact with its own token
(`ledger_fact.token`), so you can cite the evidence that you used.
A second call with the same inputs gave the same token. The token
resolves through `POST /v1/memory_token/resolve` to a fact with the
band `change_attribution.ledger`.

## Step 6: write the methods and data-availability text

A methods paragraph. Change it for each band:

> Values were retrieved from the emem responder (https://emem.dev,
> responder key `777er3yihgifqmv5hmc2wwmyszgddzderzhsx6rex4yoakwomvka`)
> on 2026-11-07. Each value is an Ed25519-signed fact identified by the
> `emem:fact:` token listed in Table S1. The token resolves to the
> byte-identical signed record, and its receipt verifies offline. [One
> estimand sentence per band.]

The responder key is the `responder.pubkey_b32` field of
`https://emem.dev/.well-known/emem.json`. It had this value on
7 November 2026. Check it again on the day you write.

For data availability, list every token in a table such as Table S1.
For a set of facts, you can also give one `emem:bundle:` token. Put
the token of each fact next to it. A bundle names a set, but it does
not pin its members. To fix the date of the record, pin a signed tree
head and include the inclusion proofs (see
[`emem-transparency-log`](../emem-transparency-log/SKILL.md)).

Before you submit, do these checks:

1. Resolve every token and compare every number in the text with
   [`emem-verify-before-publish`](../emem-verify-before-publish/SKILL.md).
2. Check each receipt with
   [`emem-verify-receipt`](../emem-verify-receipt/SKILL.md).

## Reproduce a value without emem

`sources[].id` is the upstream file, and `derivation.args` holds the
exact coordinates. Thus a reviewer can read the pixel again from the
publisher with their own tools. A signature proves that emem served
the value. It does not prove that the upstream product is accurate.
Accuracy comes from the validation literature of the product. Cite
that literature as usual.
