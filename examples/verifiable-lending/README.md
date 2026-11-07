# Verifiable lending due diligence

A bank gives a loan against a piece of land. The credit memo makes claims about
that land: its elevation, its vegetation, and its flood risk. This example
writes the memo two times. The first memo has no evidence. The second memo
binds each physical claim to a signed fact. Then emem-guard checks
both memos.

## What you need

- Python 3 (standard library only). No `pip install`, no key, no account.
- Network access to `https://emem.dev`. The script sends only reads and guard
  checks, and no signed writes. On a recall miss, the responder fetches and
  signs the band itself.

## What you get

- A `cell64` for the parcel and one Ed25519 receipt over all the signed facts
  that the recall returned.
- For each of the 14 bands, the newest signed fact and its `emem:fact:` token.
- Two published algorithms applied term by term, and checked against the
  value that the responder computed.
- Two guard verdicts, and findings that the script computes from the
  responses.

## Run it

Run these commands from the root of the repository.

```sh
python3 examples/verifiable-lending/run.py            # the full walkthrough
python3 examples/verifiable-lending/run.py --probe    # four checks on what the guard catches and what it misses
python3 examples/verifiable-lending/run.py --lat 19.0760 --lng 72.8777    # any coordinate
```

On 7 November 2026 the first full walkthrough took about 9 seconds, because the
responder fetched some bands from their sources. Later runs took less than
2 seconds. The probe took less than 1 second. The default parcel is lat 12.9352,
lng 77.6245 (Koramangala, Bengaluru). Its cell is `defi.zb493.hufo.zccc6`.

## What the walkthrough does

| step | call | result on 7 November 2026 |
|---|---|---|
| 1. Find the parcel address | `POST /v1/locate` with `lat`, `lng` | `cell64` `defi.zb493.hufo.zccc6` |
| 2. Recall signed facts | `POST /v1/recall` with 14 bands | 14 of 14 bands materialised; 29 facts and 29 receipt `fact_cids` at 09:18 UTC (this count grows with new readings) |
| 3. Apply published algorithms | `GET /v1/algorithms/<key>` | `vegetation_class_from_ndvi@1` gives `bare`; `flood_risk@2` gives 0.0244 (low); both agree with the responder |
| 4. Write two memos | none | a naive memo and a grounded memo with the same numbers |
| 5. Get two verdicts | `POST /v1/guard/verdict?shape=native` | naive: `deny` `CLAIM_UNGROUNDED`; grounded: `allow`, 5 citations, 5 `fact_cids` |
| 6. Print findings | none | computed from the responses, not written before the run |

In step 2 the recall returns every signed fact for those bands at the cell,
oldest first. At 09:18 UTC on 7 November 2026 that was 29 facts. Weather bands
had 5 readings each, and NDVI, MODIS NDVI and radar had 2 each. The script
prints only the newest fact for each band. The receipt covers all 29.

Two agents that locate the same point get the same `cell64`. So two memos about
the parcel refer to one parcel.

Step 3 takes both formulas from the content-addressed algorithm registry. The
script does not define them.

## What the guard checks and what it does not check

`--probe` sends four texts. They differ only in the citation on one elevation
claim. Results on 7 November 2026:

| check | claim | citation | verdict |
|---|---|---|---|
| A | measurable, anchored to a place name | none | `deny` `CLAIM_UNGROUNDED` |
| B | same | a real token with 4 characters changed | `deny` `CLAIM_UNGROUNDED` |
| C | same | a token that resolves | `allow` |
| D | false number (5000 m) | the correct token for that fact | `deny` `PROV_VALUE` |

`action: allow` does not mean that the guard approved the text. It means that no
rule fired. To tell a real citation from an invented one, compare two fields:

```
len(receipt.fact_cids) == citations_found
```

B fails this test (0 and 1). C and D pass it (1 and 1). D passes because its
citation is real: right cell, right band, and it resolves. The `PROV_VALUE` rule
catches D. It reads the value in the fact and compares it with the number in the
sentence. Its fix is `correct_value`, because the citation is correct and the
prose is wrong.

The guard compares values at the precision that you wrote. `889.6 m` for a fact
of `889.6439208984375` passes. `5000 m` fails.

The guard does not check relevance. In every probe text the prose says "at
Jagraon", but the cited fact is from the Bengaluru cell. Check C still gets
`allow`. A token that resolves is not evidence about the place in the sentence.
Step 3 is the partial answer: the script recomputes the published algorithms
and compares them with the value from the responder.

The guard on the full memo also misses some claims. In step 6 on 7 November
2026, the naive memo had 5 sentences with a number. The guard denied 1 (the
elevation, a length) and allowed 4 (an NDVI value, a percentage, a percentage,
and an application number).

Guard verdicts from `emem.dev` are advisory: the verdict carries
`advisory: true` and blocks nothing. To enforce them, run your own node
(`GET /v1/guard/selfhost`). Your own node can also check a corpus that this
responder does not hold.

## About `expected_output.txt`

The file is a real run of the first two commands in "Run it", taken on
7 November 2026. Your run will not match it line for line. NDVI, weather and
cloud cover are live readings that change, and their fact ids change with them.
Elevation, surface-water recurrence and the `cell64` stay the same.

Check B changed on 8 August 2026. Before that date, a citation that did not
resolve removed the denial. So any well-formed token got an `allow` with an
empty `fact_cids`. Now it is a `deny`. When your run disagrees with the
documented expectation, the script prints the difference. Trust your run over
this file.

## Limits

- One cell is about 10 m by 10 m at the equator. The script reads one cell. A
  claim about hectares needs many cells.
- When a band does not materialise, the script prints its `reason_class`. A
  `no_materializer` or an `upstream_error` is silence, not an observation. Only
  a signed `Absence` means that the corpus looked and found nothing.
- The script does not price land or judge a borrower. It checks physical claims
  about a place.
