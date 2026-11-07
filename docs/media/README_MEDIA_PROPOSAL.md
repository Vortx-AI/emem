# Proposal: new media for README.md

README.md is a signed Merkle tree, so this branch does not edit it. This file lists each proposed swap. A
maintainer can apply it and re-sign the tree. No swap deletes a file that README.md uses today.

The new files follow [visual-language.md](visual-language.md).

## Swaps

| README line (7 November 2026) | Now | Proposed | Why |
|---|---|---|---|
| 211, section "Why you can trust it" | `docs/media/readme/02-verify.gif` (1,034,577 bytes, a screen recording of the verify page) | Poster `docs/media/films/emem-fact-to-check-poster.webp` linked to `docs/media/films/emem-fact-to-check.mp4` | The section says that one changed byte changes the id and that the receipt verifies. The film shows both with live values: the receipt passes, then one changed character in the receipt fails with `signature_invalid`. |
| 225, under the paragraph about `emem-guard` | `docs/media/readme/04-guard.gif` (231,519 bytes, a screen recording) | `docs/media/stills/guard-check.webp` (184,624 bytes) | The paragraph says that the guard answers allow or deny with a reason and a `fix`. It also says that a stricter claim rule is off by default. The still shows live verdicts on one Table Mountain fact: allow, then deny `PROV_VALUE` with `correct_value`. An uncited number passes by default and fails with `claim_gating=true`. |

## Markup for the swap at line 211

```html
<a href="docs/media/films/emem-fact-to-check.mp4"><img src="docs/media/films/emem-fact-to-check-poster.webp" alt="Film, 33 seconds, silent. Mount Fuji becomes the cell defi.zb592.nUkO.zEzE. emem signs one fact there: copdem30m.elevation_mean 3539.04 m. Agent A passes the emem:fact token to Agent B. Agent B resolves it and verify_receipt returns valid. One changed character in the receipt gives signature_invalid. Live calls to emem.dev on 7 November 2026." width="720"></a>
```

## Markup for the swap at line 225

```html
<img src="docs/media/stills/guard-check.webp" alt="Three drafts checked by live calls to POST /v1/guard/verdict on 7 November 2026. The cited fact is the Table Mountain elevation, 1069.01 m. Draft 1 states 1069.01 m and cites the fact: citation verified, allow. Draft 2 cites the same fact but states 1250 m: deny, PROV_VALUE, fix correct_value. Draft 3 states 1250 m with no citation: allow by default, deny CLAIM_UNGROUNDED with claim_gating=true. Each receipt verifies." width="720">
```

Do not delete `02-verify.gif` or `04-guard.gif` before the signed README stops using them.

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
```

Markup for line 90:

```html
<img src="docs/media/stills/recall-one-cell.webp" alt="One recall at a Lake Geneva cell, from live emem.dev calls on 7 November 2026. Eight signed facts from six sources, oldest first: climate class Cfb, elevation 368 m, water occurrence 98 percent, land cover permanent water, NDVI 0.055 on a water pixel, NDWI 0.226, precipitation 0 mm, air temperature 9.9 degrees C. Three are recomputable from the source. One receipt covers all eight; signature valid." width="540">
```

Markup for line 266:

```html
<img src="docs/media/stills/a2a-exchange.webp" alt="Three live A2A steps against emem.dev on 7 November 2026. The agent card (emem 2.4.4, A2A 1.0, 18 skills listed of 119) has a valid EdDSA signature. One message/send, 'what is the elevation of Table Mountain?', returns a completed task answered by emem_ask: 1069.01 m from the Copernicus DEM. The receipt in the task checks valid offline; with one character of the fact id changed, it fails." width="540">
```

## Orbit GIFs of the 3D worlds

README.md does not use the six `docs/media/world-*.gif` files (5.4 to 17.0 MB each). Only
`examples/3d-worlds/README.md` and `capture.mjs` name them. This branch adds MP4 and WebM copies of
166 to 439 KB, with posters, in `docs/media/worlds/`. `examples/3d-worlds/README.md` now shows the posters.
`capture.mjs` writes only GIFs, and changing that is a code change. So this branch keeps the GIFs.
