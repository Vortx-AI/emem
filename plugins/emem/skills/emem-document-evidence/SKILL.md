---
name: emem-document-evidence
description: Turns a scanned document into signed, checkable evidence with emem. It runs OCR on an image. Then it parses a pesticide-residue lab report: analytes, results in mg/kg, LOQ, printed MRL, exceedances. Or it parses a land record: owners, parcel ids, areas in hectares, places, dates. Each value carries its line and byte offset, and emem signs each step. Use when an agent must extract fields from a lab certificate, a 7/12 extract, a khasra or a CAR record. Use it when the agent must hand them on with proof of the image and text they came from. Ships an offline verifier.
---

# emem-document-evidence

| | |
|---|---|
| Use it when | An agent must read fields from a residue lab report or a land record, and must show which image and text each field came from. |
| You need | A PNG, JPEG, TIFF, WebP, GIF or BMP image of 8 MiB or less, or the document text. No key. |
| You get | The OCR text with a signed receipt, the parsed fields with a second signed receipt, and the line and byte offset of each value. |
| Calls | `POST /v1/ocr` (MCP `emem_ocr`), `POST /v1/lab_report_parse`, `POST /v1/land_record_parse`. |

> **Network use.** The commands in this skill send JSON to `https://emem.dev` and receive JSON from it. `https://emem.dev` is the service that the MCP server of the plugin connects to. The commands do not execute what they download. The files under `scripts/` read local files and make no network calls.

Each call signs its output. A hash chains each call to the call before it:

| Call | What the receipt signs | Provenance |
|---|---|---|
| `POST /v1/ocr` | The image (BLAKE3), the source, the language, the engine and the text that the engine produced. | `model_output`: a reading, not a fact. |
| `POST /v1/lab_report_parse` | The text hash, and the hash of the fields that a deterministic parser read from that text. | `deterministic_index` |
| `POST /v1/land_record_parse` | The same, for a land record. | `deterministic_index` |

The responder does not store the document or the result. Each receipt
is complete in itself. The verifier in step 3 needs only the response,
and the document if you have it.

## Limits

These limits come from `crates/emem-api-rest/src/reader.rs`. A
responder operator can change the two quotas.

| Limit | Value |
|---|---|
| Image size | 8 MiB (`8 << 20` bytes). A larger image returns HTTP 400 `too_large`. |
| OCR calls | 200 for each client IP each day (`EMEM_OCR_DAILY_QUOTA`). A parse that runs OCR counts against this quota. |
| Parse calls with `text` | 2000 for each client IP each day (`EMEM_DOC_PARSE_DAILY_QUOTA`). |
| OCR at the same time | 2 for each responder. |

## Step 1: run OCR on the image

Encode the image and send it to `/v1/ocr`:

```sh
python3 -c 'import base64, json, sys; print(json.dumps({"image_b64": base64.b64encode(open(sys.argv[1], "rb").read()).decode(), "lang": "eng"}))' report.png \
  | curl -s -X POST https://emem.dev/v1/ocr -H 'content-type: application/json' -d @- -o ocr.json
jq '{engine, lang, image_blake3_b32, text_blake3_b32, provenance_class}' ocr.json
jq -r .text ocr.json
```

Through MCP, call `emem_ocr` with the same object as its arguments:
`{"image_b64": "<base64 of report.png>", "lang": "eng"}`. `emem_ocr`
is not in the short tool list of the plugin. The server runs it by name
through `tools/call`.

You can send `"url"` instead of `"image_b64"`. Send one of the two, not
both. `lang` takes Tesseract language codes. Join two or more with `+`,
for example `eng+hin+mar`. The default is `eng`.

**Read the text before you trust a field parsed from it.** The OCR
receipt proves which image and engine produced the text. It does not
prove that the reading is correct.

## Step 2: parse the text

Each parse route takes one of these inputs:

| Input | What the parser does |
|---|---|
| `ocr` | Takes the full `/v1/ocr` response. The parser checks that this responder signed it and that `text` hashes to `text_blake3_b32`. This binds the parse to the signed reading. |
| `text` | Parses your text. No OCR receipt is in the chain. |
| `url` or `image_b64` | Runs OCR and then parses, in one call. The OCR response is in the `ocr` field of the result. |

The parse routes have no MCP tool. Use REST. To parse the OCR result
from step 1:

```sh
jq '{ocr: .}' ocr.json \
  | curl -s -X POST https://emem.dev/v1/lab_report_parse -H 'content-type: application/json' -d @- -o lab.json
jq '{verdict: .result.verdict, exceedances: .result.exceedances, sample_id: .result.sample_id.value,
     rows: [.result.rows[] | {analyte, raw: .result.raw, kind: .result.kind, result_mg_kg,
                              mrl_mg_kg, exceeds_printed_mrl, line_no}]}' lab.json
```

To parse your own text and compare each row with the EU MRL in force,
add `product`:

```sh
jq -Rs '{text: ., product: "rice"}' report.txt \
  | curl -s -X POST https://emem.dev/v1/lab_report_parse -H 'content-type: application/json' -d @- -o lab_text.json
jq '{verdict: .result.verdict, lookup: .result.eu_mrl_lookup.status,
     rows: [.result.rows[] | {analyte, result_mg_kg, eu_mrl: .eu_mrl.display, exceeds: .eu_mrl.exceeds}]}' lab_text.json
```

For a land record, send the same body to
`POST https://emem.dev/v1/land_record_parse`. When the parse runs OCR
for a land record, the default language is `eng+hin+mar`.

### Lab report fields

| Field in `result` | Content |
|---|---|
| `rows[]` | One row for each analyte: `analyte`, `result` (`kind` is `value`, `not_detected` or `below_limit`, with `raw`), `result_mg_kg`, `loq_mg_kg`, `mrl_mg_kg`, `exceeds_printed_mrl`, `unit`. |
| `verdict` | `exceeds_printed_mrl`, `within_printed_mrls`, `undetermined_rows` (a row has a printed MRL that the parser cannot compare), `no_mrl_printed` or `no_rows_found`. A `product` lookup that finds an exceedance sets `exceeds_eu_mrl`. |
| `exceedances` | The number of rows above the printed MRL. |
| `sample_id`, `report_date` | The labelled value, or null when no label says so. |
| `dates[]` | Each date on each line, with its `label` and a day-first `iso` reading. `iso` is null for a date that does not exist. |
| `accreditation`, `methods`, `columns`, `header_line_no` | What the report prints about the laboratory, the methods and the table header. |
| `eu_mrl_lookup` | Only with `product`. `status` is `applied`, `product_not_found`, `product_ambiguous` (with `candidates`) or `unavailable`. It also gives the product code, `rows_matched` and `rows_unmatched`. |

Without `product`, the parser compares each result only with the MRL
that the report prints. With `product`, the responder reads the MRLs in
force from the EU Pesticides Database (DG SANTE, Regulation (EC) No
396/2005). Then it adds `eu_mrl` to each matched row. The `reading` text in
the response still says that the parser does not check the EU MRL
database. That text is out of date for a call with `product`.

### Land record fields

| Field in `result` | Content |
|---|---|
| `owners` | Owner names. |
| `parcel_ids` | Survey, gat, khasra, CAR and matricula numbers. |
| `areas` | Each area with `value.raw`, `value.unit` and `value.hectares`. |
| `places`, `village`, `taluka`, `district` | One place for each label, and the three admin names as separate fields. |
| `dates` | Each date with its line. |
| `fields_found`, `fields_missing` | What the parser found, and what it did not find. It does not fill a missing field. |

The parser converts acres, gunthas and a three-part 7/12 area such as
`1.20.50` (read as ha.are.m2) to hectares. It reports bigha raw, because
the size of a bigha changes from place to place. It reads text in the
Indian scripts, Portuguese, Spanish, French and Indonesian.

Each extracted value carries `line`, `line_no` and `byte_offset`. Use
`line` to quote the exact line. `line_no` counts from 0.

## Step 3: verify offline

`scripts/verify_doc.py` ships with this skill. Claude Code sets
`${CLAUDE_SKILL_DIR}`. The script makes no network calls. It gets
BLAKE3 and Ed25519 from `scripts/emem_crypto.py`, which also ships with
this skill. Both are plain Python with no third-party packages.

```sh
python3 "${CLAUDE_SKILL_DIR}/scripts/verify_doc.py" --self-test
python3 "${CLAUDE_SKILL_DIR}/scripts/verify_doc.py" ocr.json report.png
python3 "${CLAUDE_SKILL_DIR}/scripts/verify_doc.py" lab.json
python3 "${CLAUDE_SKILL_DIR}/scripts/verify_doc.py" lab_text.json report.txt
```

The second argument is optional. For an OCR response it is the image.
For a parse response it is the text file.

| Line | What it checks |
|---|---|
| `image MATCH` | The image file hashes to `image_blake3_b32`. |
| `text MATCH` | The text hashes to `text_blake3_b32`. |
| `result MATCH` | The `result` object hashes to `result_blake3_b32`. |
| `sig VALID` | The Ed25519 signature verifies over the rebuilt preimage. |
| `ocr VALID`, `ocr MATCH` | For a parse that carries its OCR response: the OCR signature verifies, and the parsed text is the text that the OCR receipt signs. |

A failed check prints `MISMATCH` or `INVALID`. The exit code is 0 when
every check passed, 1 when a check failed, and 2 for bad input.

The script checks the signature with the key in the response. Compare
the `signer` that it prints with the published key of the responder:

```sh
curl -s https://emem.dev/.well-known/emem.json | jq -r '.responder.pubkey_b32'
```

On 7 November 2026 this returned
`777er3yihgifqmv5hmc2wwmyszgddzderzhsx6rex4yoakwomvka`.

## Example run on 7 November 2026

The test image was a rendered residue report with three analytes and a
printed MRL of 0.01 mg/kg. It also had `Test Report No: ER/2026/0912`
and `Sample ID: BAS-RICE-044`. The text in `report.txt` was the same.

| Path | Result |
|---|---|
| OCR (`tesseract 5.5.0`), then parse with `ocr` | Tesseract put the LOQ and MRL columns in separate blocks. Each row came back with `mrl_mg_kg: null`, and the verdict was `no_mrl_printed`. All hashes and both signatures verified. |
| Parse with `text` and `product: "rice"` | Tricyclazole 0.012 mg/kg against the printed MRL 0.01: `exceeds_printed_mrl: true`. The EU lookup matched 3 of 3 rows to product 0500060 Rice, with MRL `0.01*` for each. The verdict was `exceeds_eu_mrl`, with 1 exceedance. `verify_doc.py` printed `text MATCH`, `result MATCH` and `sig VALID`. |

Both paths gave `sample_id` `BAS-RICE-044`. A run on 28 September 2026
gave the report number there. The parser no longer makes that mistake.

The OCR path shows the purpose of this skill. The chain proved exactly
which image and text gave the answer. It did not make the reading
correct. When a verdict depends on a column, find that column in the OCR
text.

After the run, a test changed one result value in `lab_text.json`. The
script then printed `result MISMATCH` and exited with 1.

## Errors

The error body has `schema: "emem.error.v1"`. The top-level `code` is
`invalid_argument` for the 400 and 422 rows, and `source_fetch_failed`
for the 502 rows. Read the specific code in `details.code`.

| HTTP | `details.code` | Cause |
|---|---|---|
| 400 | `too_large` | The image is larger than 8 MiB, or the text is too long. |
| 400 | `invalid_argument` | No input, both `url` and `image_b64`, a bad base64 string, or a `lang` that is not a Tesseract code. The `message` names the cause. |
| 400 | `url_refused` | The responder does not fetch this `url`. On 7 November 2026, an `http://` URL gave the message `only https urls are fetched`. |
| 422 | `not_an_image` | The bytes are not PNG, JPEG, TIFF, WebP, GIF or BMP. On 7 November 2026, base64 of the text `hello world` gave this code. |
| 422 | `ocr_receipt_invalid` | Someone changed the `ocr` object, or another responder signed it. On 7 November 2026, one space added to `ocr.text` gave this code. |
| 422 | `text_hash_mismatch` | `ocr.text_blake3_b32` does not match `ocr.text`. On 7 November 2026, a changed hash field gave this code. |
| 422 | `ocr_failed` | Tesseract did not read the image. |
| 429 | (no `details`) | Your IP reached a daily quota. This body is different: `code` is `ocr_quota` or `doc_parse_quota`, and the header `retry-after` is 86400. |
| 502 | `upstream_status` | The host of `url` did not answer with 2xx. On 7 November 2026, a missing file gave `emem.dev answered 404 Not Found`. |
| 502 | `fetch_failed`, `too_large` | The fetch of `url` failed, or the host sent more bytes than the cap. |
| 501 | `ocr_unavailable` | The responder has no OCR engine. |
| 503 | `busy` | No OCR slot was free. Try again. |

## Pitfalls

- **A signed reading is still a reading.** `model_output` on the OCR
  receipt means what it says.
- **Check the unit before you quote hectares.** Read
  `areas[].value.unit` for each area. A bigha has no `hectares` value.
- **The parser is not a registry.** It does not check a land record
  against the land registry. It does not check a signature on the
  document.
- **The EU lookup is a match by name.** Read `rows_unmatched`. An
  analyte that the lookup cannot match has no `eu_mrl`, and it does not
  add to the EU exceedance count.
