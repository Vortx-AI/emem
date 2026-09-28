---
name: emem-document-evidence
description: Turns a scanned document into signed, checkable evidence with emem. Runs OCR on an image, then parses a pesticide-residue lab report (analytes, results in mg/kg, LOQ, printed MRL, exceedances) or a land record (owners, parcel ids, areas in hectares, places, dates), with every value tied to its line and byte offset and every step signed. Use when an agent must extract fields from a lab certificate, a 7/12 extract, a khasra or a CAR record and hand them on with proof of which image and text they came from. Ships an offline verifier.
---

# emem-document-evidence

Three calls, each signed, each chained to the one before by a hash:

| Call | Signs | Provenance |
|---|---|---|
| `POST /v1/ocr` (MCP `emem_ocr`) | which image (blake3), engine, language and text produced this reading | `model_output`: a reading, not a fact |
| `POST /v1/lab_report_parse` | the fields a deterministic parser read from that text | `deterministic_index` |
| `POST /v1/land_record_parse` | the same, for land records | `deterministic_index` |

Nothing is persisted. The receipts are self-contained: the verifier
below needs only the response and, if you have it, the document.

## When to invoke

- "Read this residue certificate and tell me whether anything exceeds
  the MRL."
- "Pull the owner, survey number and area out of this 7/12 extract."
- An EUDR or food-safety file needs the lab or land document bound to
  the same evidence trail as the satellite facts
  ([`emem-eudr-due-diligence`](../emem-eudr-due-diligence/SKILL.md)).

## Step 1: OCR the image

```sh
base64 -w0 report.png | jq -Rs '{image_b64: ., lang: "eng"}' > ocr_req.json
curl -sf -X POST https://emem.dev/v1/ocr \
  -H 'content-type: application/json' -d @ocr_req.json > ocr.json
jq '{engine, lang, image_blake3_b32, text_blake3_b32}' ocr.json
jq -r .text ocr.json
```

Or pass `"url"` instead of `image_b64` (png, jpeg, tiff, webp, gif, bmp;
8 MiB). `lang` takes Tesseract codes, joined with `+` (`eng+hin+mar`).
A responder with no OCR engine answers `501 ocr_unavailable`.

**Read the text before you trust anything parsed from it.** The OCR
receipt proves which image and engine produced the text, not that the
reading is right.

## Step 2: parse

Send the OCR response itself as `ocr`: the parser refuses it with
`422 text_hash_mismatch` unless its text hashes to its
`text_blake3_b32`, so the parse is bound to the signed reading. Or send
your own `text`, or a `url` / `image_b64` to OCR and parse in one call.

```sh
jq '{ocr: .}' ocr.json \
  | curl -sf -X POST https://emem.dev/v1/lab_report_parse \
      -H 'content-type: application/json' -d @- > lab.json
jq '{verdict: .result.verdict, exceedances: .result.exceedances,
     rows: [.result.rows[] | {analyte, raw: .result.raw, result_mg_kg,
                              mrl_mg_kg, exceeds_printed_mrl, line_no}]}' lab.json
```

Lab report `result`: `rows[]` (analyte, `result.kind` of `value`,
`not_detected` or `below_limit`, `result_mg_kg`, `loq_mg_kg`,
`mrl_mg_kg`, `exceeds_printed_mrl`), `verdict`, `exceedances`,
`sample_id`, `dates`, `accreditation`, `methods`. It compares against
the MRL the report prints; it does not look anything up in the EU MRL
database.

Land record `result`: `owners`, `parcel_ids` (survey, gat, khasra, CAR,
matricula), `areas` (`hectares` where the unit has one meaning: acres,
gunthas, and an unlabelled three-part `1.20.50` 7/12 area read as
ha.are.m2; bigha is reported raw), `places`, `dates`, and
`fields_missing`, which lists what it did not find rather than filling
it. Text in the Indian scripts, Portuguese, Spanish, French and
Indonesian is read.

Every extracted value carries `line`, `line_no` and `byte_offset`, so
you can quote the exact line it came from.

## Step 3: verify offline

`verify_doc.py` ships beside this file (`${CLAUDE_SKILL_DIR}`, filled in
by Claude Code; needs `pip install blake3 cryptography`, no network):

```sh
python3 "${CLAUDE_SKILL_DIR}/verify_doc.py" ocr.json report.png
python3 "${CLAUDE_SKILL_DIR}/verify_doc.py" lab.json
```

It checks the image hash, the text hash, the result hash and the
Ed25519 signature for each step, and for a parse that carries its OCR
response, that the parsed text is the text the OCR receipt signs.

## Recorded on 2026-09-28

A rendered residue report (three analytes, printed MRL 0.01 mg/kg) went
through both paths:

- Parsed from its **text**: Tricyclazole 0.012 mg/kg, `verdict:
  "exceeds_printed_mrl"`, `exceedances: 1`. `verify_doc.py` printed
  `text MATCH`, `result MATCH`, `sig VALID`.
- Parsed from its **OCR** (`tesseract 5.5.0`): Tesseract split the MRL
  column into a separate block, so every row came back with
  `mrl_mg_kg: null` and `verdict: "no_mrl_printed"`. Every hash and both
  signatures still verified.

That second result is the point of this skill. The chain proved exactly
which image and text produced the answer; it could not make the reading
right. When a verdict depends on a column, check the OCR text for it.

Both runs also picked `Test Report No: ER/2026/0912` as `sample_id`
although the report prints `Sample ID: BAS-RICE-044`. Quote the line the
parser cites, and correct it by hand when the document says otherwise.

## Pitfalls

- **A signed reading is still a reading.** `model_output` on the OCR
  receipt means what it says.
- **An unlabelled area is guessed at.** `1.20.50` alone is read as
  ha.are.m2; the same number followed by `ha.are.m2` was not converted
  on 2026-09-28. Check `areas[].value.unit` before quoting hectares.
- **The parser is not a registry.** It does not check a land record
  against the land registry, or a signature on the document.
