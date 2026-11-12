---
name: emem-document-evidence
description: Turns a scanned document into signed, checkable evidence with emem. It runs OCR on an image. Then it parses a pesticide-residue lab report: analytes, results in mg/kg, LOQ, printed MRL, exceedances. Or it parses a land record: owners, parcel ids, areas in hectares, places, dates. Each value carries its line and byte offset, and emem signs each step. Use when an agent must extract fields from a lab certificate, a 7/12 extract, a khasra or a CAR record. Use it when the agent must hand them on with proof of the image and text they came from. It states which hashes and signatures to compute again, so that a third party can check the chain with its own tools and no trust in emem.
---

# emem-document-evidence

| | |
|---|---|
| Use it when | An agent must read fields from a residue lab report or a land record, and must show which image and text each field came from. |
| You need | A PNG, JPEG, TIFF, WebP, GIF or BMP image of 8 MiB or less, or the document text. No key. For the independent check, a BLAKE3 tool and an Ed25519 tool that you already trust. |
| You get | The OCR text with a signed receipt, the parsed fields with a second signed receipt, and the line and byte offset of each value. |
| Calls | `POST /v1/ocr` (MCP `emem_ocr`), `POST /v1/lab_report_parse`, `POST /v1/land_record_parse`, and `GET /.well-known/emem.json` for the published key. |

> **Network use.** The commands in this skill send JSON to `https://emem.dev` and receive JSON from it. `https://emem.dev` is the service that the MCP server of the plugin connects to. The commands do not execute what they download. This skill ships no code. Its commands read no environment variable, key or credential. They read only the image or text that you give them, and responses that an earlier step saved.

Each call signs its output. A hash chains each call to the call before it:

| Call | What the receipt signs | Provenance |
|---|---|---|
| `POST /v1/ocr` | The image (BLAKE3), the source, the language, the engine and the text that the engine produced. | `model_output`: a reading, not a fact. |
| `POST /v1/lab_report_parse` | The text hash, and the hash of the fields that a deterministic parser read from that text. | `deterministic_index` |
| `POST /v1/land_record_parse` | The same, for a land record. | `deterministic_index` |

The responder does not store the document or the result. Each receipt
is complete in itself. The check in step 3 needs only the response,
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

Build the request body from the image:

```sh
base64 -w0 report.png | jq -Rs '{image_b64: ., lang: "eng"}' > ocr_req.json
```

The body has this shape:

```json
{"image_b64": "<the base64 of report.png>", "lang": "eng"}
```

Send `ocr_req.json` as the body of `POST https://emem.dev/v1/ocr`.
Through MCP, call `emem_ocr` with the same object as its arguments.
`emem_ocr` is not in the short tool list of the plugin. The server runs
it by name through `tools/call`. Save the answer as `ocr.json`. Then
read it:

```sh
jq '{engine, lang, image_blake3_b32, text_blake3_b32, provenance_class}' ocr.json
jq -r .text ocr.json
```

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
jq '{ocr: .}' ocr.json > parse_req.json
```

Send `parse_req.json` as the body of
`POST https://emem.dev/v1/lab_report_parse`. Save the answer as
`lab.json`. Then read it:

```sh
jq '{verdict: .result.verdict, exceedances: .result.exceedances, sample_id: .result.sample_id.value,
     rows: [.result.rows[] | {analyte, raw: .result.raw, kind: .result.kind, result_mg_kg,
                              mrl_mg_kg, exceeds_printed_mrl, line_no}]}' lab.json
```

To parse your own text and compare each row with the EU MRL in force,
add `product`:

```sh
jq -Rs '{text: ., product: "rice"}' report.txt > text_req.json
```

The body has this shape:

```json
{"text": "<the text of report.txt>", "product": "rice"}
```

Send `text_req.json` as the body of
`POST https://emem.dev/v1/lab_report_parse`. Save the answer as
`lab_text.json`. Then read it:

```sh
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

## Step 3: check the chain

### Through emem

emem has one server check for this chain. Send an OCR response,
unchanged, as `ocr` to a parse route, as step 2 does. Before it parses,
the responder rebuilds the `emem.ocr.v1` preimage from the fields of
the response and checks the signature against its own key. It also
checks that `text` hashes to `text_blake3_b32`. A changed response gets
HTTP 422 with `ocr_receipt_invalid` or `text_hash_mismatch` (see
[Errors](#errors)). A parse with `ocr` has no daily quota.

This check has limits:

- The responder checks its own signature. The check is not independent.
- It accepts only an OCR response that this responder signed.
- It does not have the image, so it cannot check `image_blake3_b32`.
- No route checks a parse receipt (`emem.doc_parse.v1`).
  `emem_verify_receipt` does not take it, because it reads only the
  receipts of fact reads.

Without `product`, the parser is deterministic. Send the same text
again, and `text_blake3_b32` and `result_blake3_b32` come back the
same. On 12 November 2026 two parses of one report text, 6 s apart,
gave the same two hashes. This shows that the responder still reads
that text the same way. It is again the responder that checks itself.

### Independently

Use this part when the result must not depend on emem. Use tools that
you already trust for BLAKE3-256 and Ed25519. If you have none, say
that only the responder checked the chain.

Section 9.7 of https://github.com/Vortx-AI/emem/blob/main/docs/protocol.md
gives both preimages. The `receipt.preimage` text in each response
repeats its segment list.

First compute the hashes. Each `_b32` value is a 32-byte BLAKE3-256
digest in base32: the RFC 4648 alphabet, lowercase, no padding, 52
characters.

| Check | Compute | Must equal |
|---|---|---|
| Image | BLAKE3 of the bytes of the image file | `image_blake3_b32` |
| Text | BLAKE3 of the UTF-8 bytes of `text` (OCR), or of the text that you sent (parse) | `text_blake3_b32` |
| Result | BLAKE3 of `result` as compact JSON: no spaces, object keys in sorted order, non-ASCII text as raw UTF-8 | `result_blake3_b32` |
| Chain | For a parse that carries `ocr` | `ocr.text_blake3_b32` equals `text_blake3_b32` |

The responder writes `result` with serde_json. If your JSON tool writes
a number in a different form, the result hash does not match, even for
an unchanged result. Check the form of the numbers before you report a
mismatch.

Then check the signature:

```
seg(tag, bytes) = tag || u32le(len(bytes)) || bytes
stream = "emem.preimage.v1\0" || u32le(len(domain)) || domain || seg(1, ...) || seg(2, ...) || ...
digest = blake3(stream)                              32 bytes
valid  = ed25519_verify(responder_pubkey, signature, digest)
```

| `receipt.domain` | Segments, from tag 1 |
|---|---|
| `emem.ocr.v1` | `image_blake3_b32`, `source`, `lang`, `engine`, `text_blake3_b32`, `read_at`, `receipt.responder_pubkey_b32` |
| `emem.doc_parse.v1` | `signal`, `text_blake3_b32`, `result_blake3_b32`, `parser`, `parsed_at`, `receipt.responder_pubkey_b32` |

A tag is 1 byte and `u32le(n)` is `n` as 4 bytes, little-endian. A hash
and the key enter as their 32 raw bytes, decoded from base32. Every
other field enters as its UTF-8 bytes. The signature is
`receipt.signature_b32`, 64 bytes after decoding. Ed25519 signs the
32-byte digest, not the stream. For a parse that carries `ocr`, check
the OCR receipt inside it the same way.

The signature names the key in the response. Compare that key with the
published key of the responder:

```sh
curl -fsSL https://emem.dev/.well-known/emem.json
```

Read `responder.pubkey_b32` in the answer.

On 12 November 2026 this returned
`777er3yihgifqmv5hmc2wwmyszgddzderzhsx6rex4yoakwomvka`. On the same
day, a parse of a three-row residue report sent as `text` was checked
in this way. The text hash, the result hash in the JSON form above and
the signature under that key all matched.

The emem repository publishes a reference offline verifier at
https://github.com/Vortx-AI/emem/tree/main/tools/verifiers/emem-document-evidence
for anyone who wants to read or run it themselves. This plugin ships no
code.

## Example run on 7 November 2026

The test image was a rendered residue report with three analytes and a
printed MRL of 0.01 mg/kg. It also had `Test Report No: ER/2026/0912`
and `Sample ID: BAS-RICE-044`. The text in `report.txt` was the same.

| Path | Result |
|---|---|
| OCR (`tesseract 5.5.0`), then parse with `ocr` | Tesseract put the LOQ and MRL columns in separate blocks. Each row came back with `mrl_mg_kg: null`, and the verdict was `no_mrl_printed`. The parse route accepted the signed OCR response, so the responder had checked its signature and its text hash. |
| Parse with `text` and `product: "rice"` | Tricyclazole 0.012 mg/kg against the printed MRL 0.01: `exceeds_printed_mrl: true`. The EU lookup matched 3 of 3 rows to product 0500060 Rice, with MRL `0.01*` for each. The verdict was `exceeds_eu_mrl`, with 1 exceedance. |

Both paths gave `sample_id` `BAS-RICE-044`. A run on 28 September 2026
gave the report number there. The parser no longer makes that mistake.

The OCR path shows the purpose of this skill. The chain proved exactly
which image and text gave the answer. It did not make the reading
correct. When a verdict depends on a column, find that column in the OCR
text.

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
