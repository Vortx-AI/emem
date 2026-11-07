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
