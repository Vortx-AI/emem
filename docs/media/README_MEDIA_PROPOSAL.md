# Proposal: new media for README.md

README.md is a signed Merkle tree, so this branch does not edit it. This file lists each proposed swap. A
maintainer can apply it and re-sign the tree. No swap deletes a file that README.md uses today.

The new files follow [visual-language.md](visual-language.md).

## Swaps

| README line (7 November 2026) | Now | Proposed | Why |
|---|---|---|---|
| 211, section "Why you can trust it" | `docs/media/readme/02-verify.gif` (1,034,577 bytes, a screen recording of the verify page) | Poster `docs/media/films/emem-fact-to-check-poster.webp` linked to `docs/media/films/emem-fact-to-check.mp4` | The section says that one changed byte changes the id and that the receipt verifies. The film shows both with live values: the receipt passes, then one changed character in the receipt fails with `signature_invalid`. |
| 225, under the paragraph about `emem-guard` | `docs/media/readme/04-guard.gif` (231,519 bytes, a screen recording) | `docs/media/stills/guard-check.webp` (184,624 bytes) | The paragraph says that the guard answers allow or deny with a reason and a `fix`. It also says that a stricter claim rule is off by default. The still shows live verdicts on one Table Mountain fact: allow, then deny `PROV_VALUE` with `correct_value`. An uncited number passes by default and fails with `claim_gating=true`. |
| 264, under "One record, many readers" | `docs/media/readme/11-two-agents.gif` (1,072,956 bytes, a screen recording of two Claude sessions) | `docs/media/stills/bundle-handoff.webp` (222,750 bytes) | The paragraph says that agent A hands over one bundle line and agent B resolves it and checks the signature. The still shows that handoff with live reads. A bundle from 23 September 2026 resolves on 7 November 2026 to four facts in Bengaluru. Each receipt is valid. One changed character in a fact id makes the bundle receipt fail. |
| 285, before "Earth is the first corpus, not the limit" | `docs/media/readme/10-research.gif` (1,263,818 bytes, a screen recording of a Claude session) | `docs/media/stills/ask-trail.webp` (230,432 bytes) | The caption says that every number in the answer is a signed record. The still shows one live `/v1/ask` answer: NDVI 0.79 in Central Park. It also shows the four reasoning stages, the three signed facts and the checks. The still is not a Claude session, so the caption at line 287 must change too (text below). |

## Markup for the swap at line 211

```html
<a href="docs/media/films/emem-fact-to-check.mp4"><img src="docs/media/films/emem-fact-to-check-poster.webp" alt="Film, 33 seconds, silent. Mount Fuji becomes the cell defi.zb592.nUkO.zEzE. emem signs one fact there: copdem30m.elevation_mean 3539.04 m. Agent A passes the emem:fact token to Agent B. Agent B resolves it and verify_receipt returns valid. One changed character in the receipt gives signature_invalid. Live calls to emem.dev on 7 November 2026." width="720"></a>
```

## Markup for the swap at line 225

```html
<img src="docs/media/stills/guard-check.webp" alt="Three drafts checked by live calls to POST /v1/guard/verdict on 7 November 2026. The cited fact is the Table Mountain elevation, 1069.01 m. Draft 1 states 1069.01 m and cites the fact: citation verified, allow. Draft 2 cites the same fact but states 1250 m: deny, PROV_VALUE, fix correct_value. Draft 3 states 1250 m with no citation: allow by default, deny CLAIM_UNGROUNDED with claim_gating=true. Each receipt verifies." width="720">
```

## Markup for the swap at line 264

```html
<img src="docs/media/stills/bundle-handoff.webp" alt="Live reads of emem.dev on 7 November 2026. Agent A hands over one 38-character line, emem:bundle:y2k6h2laeo4libfsiebucdw4mu, minted on 23 September 2026: four facts at one cell in Bengaluru. Agent B resolves the bundle and each fact token: elevation 915.07 m, NDVI 0.33, PM2.5 27.8 micrograms per cubic metre and air temperature 20.1 degrees C, each with its own date. Each fact receipt is valid. The bundle receipt is valid; with one character of a fact id changed, it fails." width="540">
```

## Markup for the swap at line 285

```html
<img src="docs/media/stills/ask-trail.webp" alt="One live call to POST /v1/ask on 7 November 2026: what is the NDVI in Central Park, New York? The answer is NDVI 0.79. Four reasoning stages (located, routed, recalled, scored) each have an address that commits to the stage before it. The recall found three signed Sentinel-2 NDVI facts: 0.60, 0.88 and 0.79; the answer quotes the newest. The receipt is valid, all four stage addresses recompute, and one changed byte moves an address." width="540">
<br><sub>Contains modified Copernicus Sentinel data 2026.</sub>
```

Put this caption at line 287, in place of its first sentence. "One live answer from `/v1/ask`: the number, the stages that reached it, and the signed facts under it." Keep the video link.

Do not delete `02-verify.gif`, `04-guard.gif`, `10-research.gif` or `11-two-agents.gif` before the signed README stops using them.

## Kept as it is

`docs/media/readme/14-common-decoder.gif` at line 260 ("One record, many readers") stays. It records three
models from three vendors that read one token. This branch makes only read calls to emem.dev and calls no model,
so it cannot record that again with live data. A still of the emem side only would not show the claim of the
section, which is about the models.

## Additions

These add an image where README.md has none today. They remove nothing.

| README line (7 November 2026) | Proposed | Why |
|---|---|---|
| 218, after item 6 in "Why you can trust it" | `docs/media/stills/log-proof.webp` (249,976 bytes) | Item 6 says that a reader can prove one entry under a signed head and that the log only grows. The still shows both for one live fact: a 22-hash inclusion path and a 14-hash consistency proof. |
| 90, after step 3 of the quick start ("Read a signed measurement") | `docs/media/stills/recall-one-cell.webp` (255,266 bytes) | Step 3 reads one band at one place. The still shows what one `recall` holds at one cell: 8 facts from 6 sources, each with its own date and class, under one receipt. |
| 266, after the paragraph about two Claude sessions, the signed channel and A2A | `docs/media/stills/a2a-exchange.webp` (242,500 bytes) | The paragraph says that A2A reaches every tool as a skill. The still shows one live A2A exchange with no key: the signed agent card, one `message/send` answered by `emem_ask`, and the receipt in the task checked offline. |
| 291, after the first paragraph of "Earth is the first corpus, not the limit" | `docs/media/stills/ndvi-two-seasons.webp` (236,182 bytes) | The paragraph says that public archives fill the memory. The still shows one signed `field_series` answer: 17 clear Sentinel-2 scenes over two seasons, and the 8 cloudy scenes it left out. |

The numbers in these stills are from 7 November 2026. Item 6 in README.md gives a log size from 10 October 2026.
The still and the text show different sizes, and each is true for its date.

Markup for line 218:

```html
<img src="docs/media/stills/log-proof.webp" alt="The Mount Fuji elevation fact, 3539.04 m, is entry 2,463,431 of 3,248,898 in the log. A ladder of 22 hashes is its audit path to the signed tree head, and the Ed25519 signature is valid. One character changed in one hash gives a different root, and inclusion is invalid. A 14-hash consistency proof links the head of size 3,248,892 to the head of size 3,248,898: valid. Live calls to emem.dev on 7 November 2026." width="720">
```

Markup for line 291:

```html
<img src="docs/media/stills/ndvi-two-seasons.webp" alt="One signed field_series answer for a farm box near Ames, Iowa, from live emem.dev calls on 7 November 2026. Median NDVI of 17 clear Sentinel-2 scenes from February 2024 to November 2025 has two summer peaks. True-colour crops: 1 August 2024, NDVI 0.88, and 22 October 2024, NDVI 0.26. Eight cloudy scenes are marked as left out." width="540">
<br><sub>Contains modified Copernicus Sentinel data 2024-2025.</sub>
```

A film can take the place of this still at line 291. `docs/media/films/emem-ndvi-season.mp4` shows the same
signed answer, one scene at a time. Each step shows the true-colour crop, the NDVI of the 36 blocks and the median
on a growing chart. It ends on the one token and `verify_receipt` valid. Use the still or the film, not both:

```html
<a href="docs/media/films/emem-ndvi-season.mp4"><img src="docs/media/films/emem-ndvi-season-poster.webp" alt="Film, 32 seconds, silent, from live emem.dev reads on 7 November 2026. A farm box near Ames, Iowa, scene by scene. The poster shows 1 August 2024: a Sentinel-2 true-colour crop, all 36 blocks green, and median NDVI 0.88 with all 2,200 pixels clear. The film steps through 17 clear scenes from February 2024 to November 2025, marks the 8 dates with too few clear pixels, and ends on the one token for the series and verify_receipt valid." width="540"></a>
<br><sub>Silent, 32 seconds. <a href="docs/media/films/emem-ndvi-season.webm">WebM</a>. Contains modified Copernicus Sentinel data 2024-2025.</sub>
```

Markup for line 90:

```html
<img src="docs/media/stills/recall-one-cell.webp" alt="One recall at a Lake Geneva cell, from live emem.dev calls on 7 November 2026. Eight signed facts from six sources, oldest first: climate class Cfb, elevation 368 m, water occurrence 98 percent, land cover permanent water, NDVI 0.055 on a water pixel, NDWI 0.226, precipitation 0 mm, air temperature 9.9 degrees C. Three are recomputable from the source. One receipt covers all eight; signature valid." width="540">
<br><sub>Contains modified Copernicus Sentinel data 2026.</sub>
```

Markup for line 266:

```html
<img src="docs/media/stills/a2a-exchange.webp" alt="Three live A2A steps against emem.dev on 7 November 2026. The agent card (emem 2.4.4, A2A 1.0, 18 skills listed of 119) has a valid EdDSA signature. One message/send, 'what is the elevation of Table Mountain?', returns a completed task answered by emem_ask: 1069.01 m from the Copernicus DEM. The receipt in the task checks valid offline; with one character of the fact id changed, it fails." width="540">
<br><sub>Contains modified Copernicus Sentinel data 2024.</sub>
```

## Diagrams for a phone screen

These four SVG files are 640 units wide, with text of 16 px or more. Each answers one question with values
from live reads of emem.dev on 7 November 2026. They add an image where README.md has none today.

| README line (7 November 2026) | Proposed | Why |
|---|---|---|
| 47, after the table in "What crosses, and what stays" | `docs/media/diagrams/data-flow.svg` (6,251 bytes) | The last table row says that a signed fact names its source bytes. The diagram follows one value from a 43.4 MB open file to a fact id. Then it changes one byte and shows that the id changes. |
| 221, after the paragraph that ends with `/v1/verifier_spec` | `docs/media/diagrams/receipt-check.svg` (6,376 bytes) | The paragraph says that a reader can re-check any receipt. The diagram does it for one live receipt: 8 fields, 328 bytes, a BLAKE3 digest and an Ed25519 check. One changed character makes the check fail. |
| 308, after the paragraph that ends with "the exact digest to sign" | `docs/media/diagrams/agent-loop.svg` (6,220 bytes) | The paragraph says where an agent uses emem and how it writes. The diagram shows one turn: ask, recall, cite, act, then write back a note signed with the agent's own key. |
| 312, after "The hosted node runs the binary in this repo" and its `docker run` block | `docs/media/diagrams/architecture.svg` (8,484 bytes) | The section says that one binary runs the node. The diagram shows what that binary does when a call comes in: find, fetch on a miss, hash, sign, log, answer. |

`docs/media/diagrams/address.svg` (what a `cell64` tells you about a place) is not on this list. README.md does
not name `cell64`, so the diagram has no sentence to support there. PR #32 can embed it in `docs/protocol.md`.

The log size in `architecture.svg` (3,250,469 entries at 12:22 UTC) is newer than the size in README.md
(2,811,200 on 10 October 2026). Each is true for its date.

Markup for line 47:

```html
<img src="docs/media/diagrams/data-flow.svg" alt="How one value becomes a fact, from live emem.dev bytes on 7 November 2026. emem reads 64 KiB and one tile of a 43.4 MB Copernicus DEM file with HTTP Range. A stateless connector samples 3539.04 m at Mount Fuji. Canonical CBOR makes 743 bytes; their BLAKE3 hash is the fact_cid qejpz7cg...2jjbplq, the same when hashed again on another machine. The responder signs a Merkle root over fact ids, and each read gets a signed receipt. One changed byte gives 3539.0420 m and a different id." width="540">
```

Markup for line 221:

```html
<img src="docs/media/diagrams/receipt-check.svg" alt="One live recall receipt at Mount Fuji, 3539.04 m, checked offline on 7 November 2026. Eight fields make a 328-byte preimage. Its BLAKE3 digest verifies with Ed25519 against the key from /.well-known/emem.json: VALID. Change served_at by one character and the digest changes: INVALID. A valid check shows which key signed these fact ids, not that the value is true." width="540">
```

Markup for line 308:

```html
<img src="docs/media/diagrams/agent-loop.svg" alt="One agent turn with emem, from a live ask on 7 November 2026. 1 Ask: what is the elevation of Mount Fuji? 2 Recall: copdem30m.elevation_mean 3539.04 m at cell defi.zb592.nUkO.zEzE, receipt valid. 3 Cite: put the emem:fact token next to the number. 4 Act: the agent uses the number; emem has no part in this step. 5 Write back: a note under /memories/by_attester/ and the first 8 characters of the agent's key, signed with that key. A signature shows who wrote a note, not that it is true." width="540">
```

Markup for line 312, after the `docker run` block:

```html
<img src="docs/media/diagrams/architecture.svg" alt="What answers when an agent calls emem.dev, from live reads on 7 November 2026 at 12:22 UTC. REST (183 paths under /v1) and MCP (116 tools) reach the same handlers in one Rust binary, emem-server 2.4.4. It looks up a redb fact store, fetches from 46 registered open-data sources on a miss, hashes canonical CBOR with BLAKE3 into a fact id, signs with one Ed25519 key, and appends to an RFC 6962 log of 3,250,469 entries. Anyone checks the receipt offline with the public key at /.well-known/emem.json." width="540">
```

## Orbit GIFs of the 3D worlds

README.md does not use the six `docs/media/world-*.gif` files (5.4 to 17.0 MB each). Only
`examples/3d-worlds/README.md` and `capture.mjs` name them. This branch adds MP4 and WebM copies of
166 to 439 KB, with posters, in `docs/media/worlds/`. `examples/3d-worlds/README.md` now shows the posters.
`capture.mjs` writes only GIFs, and changing that is a code change. So this branch keeps the GIFs.
