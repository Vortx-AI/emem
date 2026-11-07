# Proposal: new media for README.md

README.md is a signed Merkle tree, so this branch does not edit it. This file lists each proposed swap. A
maintainer can apply it and re-sign the tree. No swap deletes a file that README.md uses today.

The new files follow [visual-language.md](visual-language.md).

## Swaps

| README line (7 November 2026) | Now | Proposed | Why |
|---|---|---|---|
| 25, the image under the badges | `docs/media/readme/20-encode-decode.webp` (76,972 bytes, a drawing with no data; on a 360 px phone its labels are about 5 px high) | `docs/media/stills/what-crosses.webp` (218,024 bytes), with `docs/media/stills/what-crosses-phone.webp` (204,422 bytes) for narrow screens | The drawing and the table at line 40 say that the data stays at the source and a token crosses. The still shows this with three live tokens. One names a tensor of a GPT-2 weights file. One is an execution trace of an enrolled Linux host. One is the Mount Fuji fact. Each column says what stays, what crosses and which check passes. It also states the limits: the device is a Linux host, not a satellite or robot, and the device gate admits no hardware anchor yet. |
| 211, section "Why you can trust it" | `docs/media/readme/02-verify.gif` (1,034,577 bytes, a screen recording of the verify page) | Poster `docs/media/films/emem-fact-to-check-poster.webp` linked to `docs/media/films/emem-fact-to-check.mp4` | The section says that one changed byte changes the id (point 1) and that the receipt verifies (point 2). The film shows point 2 with live values: the receipt passes, then one changed character in the receipt fails with `signature_invalid`. A second film for point 1 is below the markup. |
| 225, under the paragraph about `emem-guard` | `docs/media/readme/04-guard.gif` (231,519 bytes, a screen recording) | `docs/media/stills/guard-check.webp` (184,624 bytes) | The paragraph says that the guard answers allow or deny with a reason and a `fix`. It also says that a stricter claim rule is off by default. The still shows live verdicts on one Table Mountain fact: allow, then deny `PROV_VALUE` with `correct_value`. An uncited number passes by default and fails with `claim_gating=true`. |
| 264, under "One record, many readers" | `docs/media/readme/11-two-agents.gif` (1,072,956 bytes, a screen recording of two Claude sessions) | `docs/media/stills/bundle-handoff.webp` (228,238 bytes) | The paragraph says that agent A hands over one bundle line and agent B resolves it and checks the signature. The still shows that handoff with live reads. A bundle from 23 September 2026 resolves on 7 November 2026 to four facts in Bengaluru. Each receipt is valid. One changed character in a fact id makes the bundle receipt fail. |
| 285, before "Earth is the first corpus, not the limit" | `docs/media/readme/10-research.gif` (1,263,818 bytes, a screen recording of a Claude session) | `docs/media/stills/ask-trail.webp` (228,282 bytes) | The caption says that every number in the answer is a signed record. The still shows one live `/v1/ask` answer: NDVI 0.79 in Central Park. It also shows the four reasoning stages, the three signed facts and the checks. The still is not a Claude session, so the caption at line 287 must change too (text below). |

## Markup for the swap at line 25

```html
<p align="center"><picture><source media="(max-width: 700px)" srcset="docs/media/stills/what-crosses-phone.webp"><img src="docs/media/stills/what-crosses.webp" alt="Three live examples from emem.dev on 7 November 2026. A file: the GPT-2 weights file stays; one tree token for the tensor h.3.attn.c_proj.bias crosses. Its leaf hash and 7 path hashes give the index root offline, and one changed bit fails. A device: the raw OS event logs of a Linux host stay, 13,870 events in 4 layers; one signed execution trace crosses. trace_verify returns admit, the key is operator endorsed, and no hardware anchor is admitted yet. An open archive: the Copernicus DEM tile stays; one fact token for Mount Fuji, 3,539.04 m, crosses. Its receipt and log entry verify offline, and one changed character fails." width="880"></picture></p>
```

Keep `20-encode-decode.webp` in the repository. The one-token hero in "Additions" (line 36) shows the same Mount
Fuji fact in more detail. With this swap, that addition is optional.

## Markup for the swap at line 211

```html
<a href="docs/media/films/emem-fact-to-check.mp4"><img src="docs/media/films/emem-fact-to-check-poster.webp" alt="Film, 33 seconds, silent. Mount Fuji becomes the cell defi.zb592.nUkO.zEzE. emem signs one fact there: copdem30m.elevation_mean 3539.04 m. Agent A passes the emem:fact token to Agent B. Agent B resolves it and verify_receipt returns valid. One changed character in the receipt gives signature_invalid. Live calls to emem.dev on 7 November 2026." width="720"></a>
```

`docs/media/films/emem-one-byte.mp4` (33 seconds, 1080 by 960) shows point 1 at line 213. It draws the 743 bytes
of the Mount Fuji elevation fact. One changed bit in the value gives a new id, and emem.dev does not hold that id.
`docs/whitepaper.md` section 5 already shows it. To show point 1 before point 2, put this film at line 211 and the
film above after line 219. To keep one film, use the one above.

```html
<a href="docs/media/films/emem-one-byte.mp4"><img src="docs/media/films/emem-one-byte-poster.webp" alt="Film, 33 seconds, silent. A grid of 743 squares, one for each byte of the Mount Fuji elevation fact, hashes with BLAKE3 to the id qejpz7cg. One changed bit in the 4-byte value turns 3539.0422 m into 3539.0420 m; both round to 3539.04 m. The new bytes hash to 26zhvwvf, and 49 of the 52 characters differ. On emem.dev the old id returns the same 743 bytes and the new id returns 404 cid_not_found. Live reads on 7 November 2026." width="540"></a>
<br><sub>Silent, 33 seconds.</sub>
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

Put this caption at line 287, in place of its first sentence. "One live answer from `/v1/ask`: the number, the stages
that reached it, and the signed facts under it." Keep the video link.

Do not delete `02-verify.gif`, `04-guard.gif`, `10-research.gif` or `11-two-agents.gif` before the signed README stops
using them.

## Kept as it is

`docs/media/readme/14-common-decoder.gif` at line 260 ("One record, many readers") stays. It records three
models from three vendors that read one token. This branch makes only read calls to emem.dev and calls no model,
so it cannot record that again with live data. A still of the emem side only would not show the claim of the
section, which is about the models.

## Additions

These add an image where README.md has none today. They remove nothing.

| README line (7 November 2026) | Proposed | Why |
|---|---|---|
| 218, after item 6 in "Why you can trust it" | `docs/media/stills/log-proof.webp` (256,380 bytes) | Item 6 says that a reader can prove one entry under a signed head and that the log only grows. The still shows both for one live fact: a 22-hash inclusion path and a 14-hash consistency proof. |
| 90, after step 3 of the quick start ("Read a signed measurement") | `docs/media/stills/recall-one-cell.webp` (255,266 bytes) | Step 3 reads one band at one place. The still shows what one `recall` holds at one cell: 8 facts from 6 sources, each with its own date and class, under one receipt. |
| 266, after the paragraph about two Claude sessions, the signed channel and A2A | `docs/media/stills/a2a-exchange.webp` (242,500 bytes) | The paragraph says that A2A reaches every tool as a skill. The still shows one live A2A exchange with no key: the signed agent card, one `message/send` answered by `emem_ask`, and the receipt in the task checked offline. |
| 291, after the first paragraph of "Earth is the first corpus, not the limit" | `docs/media/stills/ndvi-two-seasons.webp` (236,182 bytes) | The paragraph says that public archives fill the memory. The still shows one signed `field_series` answer: 17 clear Sentinel-2 scenes over two seasons, and the 8 cloudy scenes it left out. |
| 36, after the paragraph that starts "A token lets a reader resolve", before "## What crosses, and what stays" | `docs/media/stills/one-token-hero.webp` (232,556 bytes), with `docs/media/stills/one-token-hero-phone.webp` (243,430 bytes) for narrow screens | The section says that the source file stays and a token crosses. The still shows this for one live fact. "Mount Fuji" becomes one cell. emem signs the elevation there (3,539.04 m) from a file that stays in its archive. The token resolves to the same fact id. The receipt and a 22-hash log path check valid offline. One changed character fails. |
| 331, after the list in "What the checks do not prove", before "Version 2.4.4" | `docs/media/stills/one-place-many-names.webp` (290,194 bytes) | The list says that a place name resolves to one 10 m cell. The still shows what that means for five names of one mountain: four cells, one 13.3 km away with `disambiguation_required` true. It ends with the advice to hand on the cell64 or the token, not the name. |

Markup for line 36:

```html
<p align="center"><picture>
<source media="(max-width: 700px)" srcset="docs/media/stills/one-token-hero-phone.webp">
<img src="docs/media/stills/one-token-hero.webp" alt="Live reads of emem.dev on 7 November 2026, in three steps. 1: the name Mount Fuji resolves to the cell defi.zb592.nUkO.zEzE, ringed on a Sentinel-2 image of the summit crater from 12 December 2025. 2: emem signs one fact there: copdem30m.elevation_mean 3,539.04 m, read from a Copernicus GLO-30 DEM tile, fact id qejpz7cg...bplq, signed 28 September 2026. 3: another agent gets the 83-character emem:fact token. Resolve gives the same fact id and value. The receipt signature is valid offline. Log entry 2,463,431 of 3,248,898 proves with 22 hashes. One changed character of the fact id fails." width="880">
</picture>
<br><sub>Contains modified Copernicus Sentinel data 2025.</sub></p>
```

This still does not replace the hero image at line 25. The hero names files and machines as sources too, and
this still shows one Earth fact only. On a screen 700 pixels wide or less, the browser shows the phone copy. It
has the same content in one column, and all of its text is easy to read at 360 pixels wide.

The numbers in these stills are from 7 November 2026. Item 6 in README.md gives a log size from 10 October 2026.
The still and the text show different sizes, and each is true for its date.

Markup for line 218. Put a blank line before and after the image. GitHub then starts item 7 as a new list
that keeps the number 7.

```html
<img src="docs/media/stills/log-proof.webp" alt="The Mount Fuji elevation fact, 3539.04 m, was signed in a batch of one fact. That batch is entry 2,463,431 of 3,248,898 in the log. A ladder of 22 hashes is its audit path to the signed tree head, and the Ed25519 signature is valid. One character changed in one hash gives a different root, and inclusion is invalid. A 14-hash consistency proof links the head of size 3,248,892 to the head of size 3,248,898: valid. Live calls to emem.dev on 7 November 2026." width="720">
```

A film can take the place of this still at line 218. `docs/media/films/emem-log-climb.mp4` shows the same proof in
motion. The entry climbs one hash at a time, and the view zooms out from 1 entry to the whole log. Then one changed
character breaks the proof, and a second proof shows that the log only grew. Use the still or the film, not both:

```html
<a href="docs/media/films/emem-log-climb.mp4"><img src="docs/media/films/emem-log-climb-poster.webp" alt="Film, 38 seconds, silent, from live emem.dev reads on 7 November 2026. The Mount Fuji elevation fact sits in entry 2,463,431 of 3,248,898 in emem's log. The poster shows hash 17 of 22 in its audit path: a bar of 424,992 entries around the entry, 131,072 entries covered so far and the next block of 131,072. The film ends at the signed root, then shows one changed character giving inclusion invalid and a 14-hash consistency proof." width="540"></a>
<br><sub>Silent, 38 seconds.</sub>
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
<br><sub>Silent, 32 seconds. Contains modified Copernicus Sentinel data 2024-2025.</sub>
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

Markup for line 331. The still is tall (1080 by 3781 pixels), so show it narrow:

```html
<img src="docs/media/stills/one-place-many-names.webp" alt="Five phrasings of Mount Fuji sent to locate on 7 November 2026 gave four cells. &quot;Mount Fuji&quot; and &quot;mount fuji&quot; gave defi.zb592.nUkO.zEzE, elevation 3539.04 m. &quot;Fujisan&quot; gave the Wikidata point 390 m away, 3766.73 m. The coordinates 35.3606, 138.7274 gave the next cell, 10 m from that point, 3768.41 m. &quot;Fuji-san, Japan&quot; gave a shrine 13.3 km away, 871.37 m, with disambiguation_required true. The emem:fact: token for the first cell resolves to the same signed bytes for any agent. All four receipts check valid offline." width="360">
```

## Diagrams for a phone screen

These four SVG files are 640 units wide, with text of 16 px or more. Each answers one question with values
from live reads of emem.dev on 7 November 2026. They add an image where README.md has none today.

| README line (7 November 2026) | Proposed | Why |
|---|---|---|
| 47, after the paragraph that starts "What each check proves" (the table ends at line 45), before `## Quickstart` | `docs/media/diagrams/data-flow.svg` (6,251 bytes) | The last table row says that a signed fact names its source bytes. The diagram follows one value from a 43.4 MB open file to a fact id. Then it changes one byte and shows that the id changes. |
| 221, after the paragraph that ends with "generated from the running code" | `docs/media/diagrams/receipt-check.svg` (6,376 bytes) | The paragraph says that a reader can re-check any receipt. The diagram does it for one live receipt: 8 fields, 328 bytes, a BLAKE3 digest and an Ed25519 check. One changed character makes the check fail. |
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
<img src="docs/media/diagrams/receipt-check.svg" alt="One live recall receipt at Mount Fuji, 3539.04 m, checked offline on 7 November 2026. A fixed header and seven receipt fields make a 328-byte preimage. Its BLAKE3 digest verifies with Ed25519 against the key from /.well-known/emem.json: VALID. Change served_at by one character and the digest changes: INVALID. A valid check shows which key signed these fact ids, not that the value is true." width="540">
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
`examples/3d-worlds/README.md` and `capture.mjs` name them. This branch adds MP4 copies of
192 to 278 KB, with posters, in `docs/media/worlds/`. `examples/3d-worlds/README.md` now shows the posters.
`capture.mjs` writes only GIFs, and changing that is a code change. So this branch keeps the GIFs.
