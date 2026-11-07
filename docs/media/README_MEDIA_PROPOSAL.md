# Proposal: new media for README.md

README.md is a signed Merkle tree, so this branch does not edit it. This file lists each proposed swap. A
maintainer can apply it and re-sign the tree. No swap deletes a file that README.md uses today.

The new files follow [visual-language.md](visual-language.md).

## Swaps

| README line (7 November 2026) | Now | Proposed | Why |
|---|---|---|---|
| 211, section "Why you can trust it" | `docs/media/readme/02-verify.gif` (1,034,577 bytes, a screen recording of the verify page) | Poster `docs/media/films/emem-fact-to-check-poster.webp` linked to `docs/media/films/emem-fact-to-check.mp4` | The section says that one changed byte changes the id and that the receipt verifies. The film shows both with live values: the receipt passes, then one changed character in the receipt fails with `signature_invalid`. |

## Markup for the swap at line 211

```html
<a href="docs/media/films/emem-fact-to-check.mp4"><img src="docs/media/films/emem-fact-to-check-poster.webp" alt="Film, 33 seconds, silent. Mount Fuji becomes the cell defi.zb592.nUkO.zEzE. emem signs one fact there: copdem30m.elevation_mean 3539.04 m. Agent A passes the emem:fact token to Agent B. Agent B resolves it and verify_receipt returns valid. One changed character in the receipt gives signature_invalid. Live calls to emem.dev on 7 November 2026." width="720"></a>
```

Do not delete `02-verify.gif` before the signed README stops using it.

## Additions

These add an image where README.md has none today. They remove nothing.

| README line (7 November 2026) | Proposed | Why |
|---|---|---|
| 218, after item 6 in "Why you can trust it" | `docs/media/stills/log-proof.webp` (189,188 bytes) | Item 6 says that a reader can prove one entry under a signed head and that the log only grows. The still shows both for one live fact: a 22-hash inclusion path and a 14-hash consistency proof. |
| 291, after the first paragraph of "Earth is the first corpus, not the limit" | `docs/media/stills/ndvi-two-seasons.webp` (203,294 bytes) | The paragraph says that public archives fill the memory. The still shows one signed `field_series` answer: 17 clear Sentinel-2 scenes over two seasons, and the 8 cloudy scenes it left out. |

The numbers in both stills are from 7 November 2026. Item 6 in README.md gives a log size from 10 October 2026.
The still and the text show different sizes, and each is true for its date.

Markup for line 218:

```html
<img src="docs/media/stills/log-proof.webp" alt="The Mount Fuji elevation fact, 3539.04 m, is entry 2,463,431 of 3,248,898 in the log. A ladder of 22 hashes is its audit path to the signed tree head, and the Ed25519 signature is valid. One character changed in one hash gives a different root, and inclusion is invalid. A 14-hash consistency proof links the head of size 3,248,892 to the head of size 3,248,898: valid. Live calls to emem.dev on 7 November 2026." width="720">
```

Markup for line 291:

```html
<img src="docs/media/stills/ndvi-two-seasons.webp" alt="One signed field_series answer for a farm box near Ames, Iowa, from live emem.dev calls on 7 November 2026. Median NDVI of 17 clear Sentinel-2 scenes from February 2024 to November 2025 has two summer peaks. True-colour crops: 1 August 2024, NDVI 0.88, and 22 October 2024, NDVI 0.26. Eight cloudy scenes are marked as left out." width="540">
```

## Orbit GIFs of the 3D worlds

README.md does not use the six `docs/media/world-*.gif` files (5.4 to 17.0 MB each). Only
`examples/3d-worlds/README.md` and `capture.mjs` name them. This branch adds MP4 and WebM copies of
166 to 439 KB, with posters, in `docs/media/worlds/`. `examples/3d-worlds/README.md` now shows the posters.
`capture.mjs` writes only GIFs, and changing that is a code change. So this branch keeps the GIFs.
