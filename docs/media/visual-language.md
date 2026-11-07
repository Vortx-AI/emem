# Visual language for emem media

Use these rules for every diagram, still and film in `docs/media/`. Then a reader sees one system across the pages.
[README.md](README.md) shows eleven of these pictures in the order of the story, from a place name to a check.

## One question per image

Each image answers one question. Write the question first, then draw only what answers it. Put the answer in the
title of the image.

## Real data only

Every value in an image comes from a live call to emem.dev, from a registry in `crates/emem-core/data/`, or from the
code. Write the date of the call in the image footer, for example "Live calls to emem.dev on 7 November 2026". Do
not draw a mock-up of a response.

## Colour

Each image has its own dark background. It reads the same on a light page and on a dark page.

| Role | Colour | Use for |
|---|---|---|
| Background | `#0d1117` | The full frame |
| Panel | `#161b22`, border `#30363d` | Cards and boxes |
| Text | `#e6edf3` | Titles and values |
| Muted text | `#8b949e` | Labels, routes, footers |
| Id | `#79c0ff` | A `cell64`, a `fact_cid`, a hash |
| Token | `#ffa657` | An `emem:fact:` or other token |
| Signature | `#d2a8ff` | A key or a signature |
| Check passes | `#7ee787` | `true`, `valid` |
| Check fails | `#ff7b72` | `false`, a refusal, a changed byte |

Do not use colour as the only signal. Write the word too, for example `valid` or `signature_invalid`.

## Type

- Sans text: DejaVu Sans, then `system-ui`. Code, ids and routes: DejaVu Sans Mono, then `ui-monospace`.
- A phone shows a 1080 px image at about 360 px, so all text becomes one third of its size.
- Films are 1080 px wide. Body text is 32 px or more, which is about 11 px on a phone. The foot line is
  30 px, which is about 10 px on a phone. Keep it to two short lines. The page text next to the film must
  also repeat what the foot line says.
- A still script lays out the page at 800 px and renders it at 1080 / 800 scale. Then the smallest text, 22 px in the
  layout, is about 10 px on a phone. The heading is about 20 px. If a long code name makes a table too
  wide, let it break at its dots (`<wbr>`). Do not make it smaller.
- `hero.py --phone` lays out at 540 px and renders at 2x, so its smallest text (18 px) is 12 px on a phone.
- Diagrams in SVG keep the smallest text at 16 px in a 640 px view box, which is about 8.5 px on a phone.
  Body text is 18 px. Break a long line into two lines rather than make the text smaller.
- A wide still with columns (the hero is 1800 px) is too small on a phone. Give it a one-column phone copy,
  1080 px wide, and embed both in a `<picture>` element. The `<source>` has
  `media="(max-width: 700px)"` and names the phone copy. `hero.py --phone` makes one.
- Before you commit a diagram, run `docs/media/diagrams/src/check_text.py` on it. It lists text that
  leaves the card or overlaps other text.

## Layout

- A film has a row of numbered steps at the top. The current step has a blue border.
- One card per step. The label is small and muted, above the value.
- Long ids break at a fixed place. Do not shorten a value that the reader must compare. Shorten other ids with an
  ellipsis at the end, for example `777er3yi…`.

## Films

- Silent. Each scene shows its sentence on screen, so no sound is necessary.
- Render frame by frame from an HTML page with `window.film.seek(t)`. Do not record a screen. The pages and the
  renderer are in [films/src/](films/src/). Run `python film.py --help` there for the commands.
- Ship an MP4 (H.264) and a WebM (VP9), each 2 MB or less, and a WebP poster of 300 KB or less.
- On a docs page, show the poster and link it to the MP4. Give the WebM link next to it.
  When a still of the same data is directly above, give text links only, so the page does not show the
  same picture twice. `agents.md` does this for `emem-ndvi-season.mp4`.

## Stills

- A still is 1080 px wide and 300 KB or less, in WebP. Before you commit it, scale it to 360 px wide and
  read every line. It uses the same colours, type and cards as a film.
- Draw a still from saved live responses, not from typed values. Keep the responses and the script in
  [stills/src/](stills/src/), so anyone can draw it again. The script checks that the values agree, for example
  that the scene id in the image headers equals the scene id in the series row.
- Sentinel-2 imagery carries the credit "Contains modified Copernicus Sentinel data" and the years.
- When crops sit side by side for comparison, put them on one colour scale. `scene.png` stretches each image on
  its own and names the stretch in its `x-emem-scene-stretch-*` headers. Undo it, as
  [stills/src/similar_places.py](stills/src/similar_places.py) does. With a stretch per image, bare sand can look
  like cloud.

## Captions and alt text

The alt text states what the image shows, with the values. The caption says what the image does not prove when that
matters, for example "A signature shows who signed. It does not show that the value is true."

Before you commit, run `python3 docs/media/stills/src/check_media.py` from the repository root. It
fails when a file is over its size limit or has no row in the table below. It fails when a page gives
the wrong length for a film. It also fails when a number in a still's alt text is in no saved response.
A value that a script computes, such as a byte count, goes in its `COMPUTED` list with the code that makes it.
For [README_MEDIA_PROPOSAL.md](README_MEDIA_PROPOSAL.md), it also checks each byte size, each file name, and
that README.md still shows the old image on each line a swap names.

## Media that follows these rules

Each row gives the question the image answers and the page that shows it. Paths are relative to `docs/media/`.
PR #30, PR #32 and PR #33 rewrite the pages that will show five of the diagrams. This PR does not edit those
pages, so the PRs can merge without a conflict. The description of PR #34 gives the embed line for each.

| File | Question it answers | Page | Source |
|---|---|---|---|
| `diagrams/fact-chain.svg` | How does a place become a fact that another agent can check? | `start-here.md` | Hand-written SVG |
| `diagrams/two-layers.svg` | Who may write a fact, and who may write a note? | `intro.md` | Hand-written SVG |
| `diagrams/signed-note-write.svg` | What happens to an unsigned note write? | `intro.md` | Hand-written SVG |
| `diagrams/time-filters.svg` | Which time filter selects which fact? | `intro.md` | Hand-written SVG |
| `diagrams/agent-loop.svg` | What does an agent do with emem, step by step? | `agents.md` after PR #30 merges | Hand-written SVG, values from one live ask |
| `diagrams/data-flow.svg` | How does one pixel become a fact with a content id? | `developers/data-sources.md` after PR #33 merges | `diagrams/src/data_flow.py`, bytes of one live fact |
| `diagrams/architecture.svg` | What answers when an agent calls emem? | `whitepaper.md`; `developers/architecture.md` after PR #33 merges | `diagrams/src/architecture.py`, five live reads |
| `diagrams/address.svg` | What does a `cell64` tell you about a place, and what does it not? | `whitepaper.md`; `protocol.md` section 1.3 after PR #32 merges | `diagrams/src/address.py`, three live `locate` reads |
| `diagrams/receipt-check.svg` | How do I check a receipt without trusting the server? | `whitepaper.md`; `protocol.md` receipt section after PR #32 merges | `diagrams/src/receipt.py`, one live `recall` receipt checked offline |
| `films/emem-fact-to-check.mp4` | Can another agent check a fact from a token, and does one changed character fail? | `start-here.md` | `films/src/emem-fact-to-check.html` |
| `films/emem-note-write.mp4` | How does a refused note write become an accepted one? | `memory.md` | `films/src/emem-note-write.html` |
| `films/emem-ndvi-season.mp4` | What does one signed `field_series` answer say, scene by scene? | `agents.md` | `films/src/emem-ndvi-season.html`, data from `season_data.py` |
| `films/emem-log-climb.mp4` | How do 22 hashes prove one entry in a log of 3,248,898? | `whitepaper.md` section 8.4 | `films/src/emem-log-climb.html`, data from `log_climb_data.py` |
| `films/emem-one-byte.mp4` | If one bit of a fact changes, does its id change? | `whitepaper.md` section 5 | `films/src/emem-one-byte.html`, data from `one_byte_data.py` |
| `stills/recall-one-cell.webp` | What does one recall return at one cell? | `agents.md`, `whitepaper.md` under Figure 6 | `stills/src/recall_cell.py` |
| `stills/change-ledger.webp` | When a number moves at one cell, what does emem say about why? | `whitepaper.md` under Figure 2 | `stills/src/change_ledger.py`, read-only resolves from `fetch_change.py` |
| `stills/similar-places.webp` | Which places look like Dubai from orbit? | `agents.md` | `stills/src/similar_places.py` |
| `stills/ndvi-two-seasons.webp` | How does NDVI change over two seasons on one field? | `agents.md` | `stills/src/ndvi_season.py` |
| `stills/ask-trail.webp` | Which stages are behind one `/v1/ask` answer? | `agents.md` | `stills/src/ask_trail.py` |
| `stills/guard-check.webp` | When does the guard allow a draft and when does it deny it? | `agents.md` | `stills/src/guard_check.py` |
| `stills/log-proof.webp` | Is this fact in the log, and does the log only grow? | `protocol.md` | `stills/src/log_proof.py` |
| `stills/bundle-handoff.webp` | Can a second agent check a bundle of four facts? | `emem-in-depth.md` | `stills/src/bundle_handoff.py` |
| `stills/a2a-exchange.webp` | Can a reader check an A2A answer offline? | `emem-in-depth.md` | `stills/src/a2a_exchange.py` |
| `stills/signed-thread.webp` | Who wrote each note in a signed thread? | `emem-in-depth.md` | `stills/src/signed_thread.py` |
| `stills/field-outlines.webp` | Which fields in one farm box turned green first, and can I check it? | `agents.md` | `stills/src/field_outlines.py`, live reads from `fetch_fields.py` |
| `stills/contradictions.webp` | Two signed facts at one address disagree: which one does a read return, and why? | `intro.md` | `stills/src/contradictions.py`, live reads from `fetch_contradictions.py` |
| `stills/trajectory.webp` | What does a trajectory return, and what does an empty one mean? | `robots.md`: embed asked of the memory stream (PR #31 rewrites the page) | `stills/src/trajectory.py`, live reads from `fetch_trajectory.py` |
| `stills/one-token-hero.webp`, `stills/one-token-hero-phone.webp` | What is emem, shown with one live fact? | `intro.md` (a `<picture>` element); `whitepaper.md` after Figure 3; README line 36 in the proposal | `stills/src/hero.py`, saved reads in `stills/src/data/log/` and `stills/src/data/hero/` |
| `stills/what-crosses.webp`, `stills/what-crosses-phone.webp` | For a file, a device and an archive, what stays, what crosses and how is it checked? | README line 25 in the proposal | `stills/src/what_crosses.py`, reads from `fetch_crosses.py` (7 November 2026 15:22 UTC) and the Mount Fuji reads |
| `stills/one-place-many-names.webp` | Do two names for one place give the same cell? | `start-here.md` | `stills/src/one_place.py`, live reads from `fetch_one_place.py` |
| `stills/time-filters-live.webp` | What do the two time filters return on real data? | `intro.md` under `time-filters.svg` | `stills/src/time_filters.py`, live reads from `fetch_asof.py` |
| `stills/wire-forms.webp` | What does each id in one real fact decode to? | `protocol.md` section 1, under `09-address-algebra.svg` | `stills/src/wire_forms.py`, decoder and checks in `wire_data.py`, reads in `data/wire/` (7 November 2026 15:40 UTC) |
| `stills/receipt-preimage.webp` | What bytes does a receipt signature cover, and what breaks it? | `protocol.md` section 7.2, after the worked example | `stills/src/receipt_preimage.py`, rebuild and checks in `receipt_data.py`, reads from `fetch_receipt.py` in `data/receipt/` (7 November 2026 15:44 UTC) |
| `stills/area-receipts.webp` | What does an area read sign, and what does it leave to me? | `agents.md`, section on polygon receipts | `stills/src/area_receipts.py` (asserts inside), reads from `fetch_polygon.py` in `data/polygon/` (7 November 2026 16:08 and 16:09 UTC) |
| `stills/glossary-map.webp` | Which part of a real answer does each glossary term name? | `glossary.md`, under the opening lines | `stills/src/glossary_map.py` (asserts inside), reads from `fetch_glossary.py` in `data/glossary/` (7 November 2026 16:03 UTC) |
| `stills/canopy-rules.webp` | Which canopy-loss rule flags less stable forest, and what does it cost in lost forest found? | `research/canopy-loss-2026-10.md`, section 5.9 | `stills/src/canopy_rules.py` (checks the note signatures and parses the signed tables), reads from `fetch_canopy.py` in `data/canopy/` (7 November 2026 16:15 UTC) |
| `stills/method-selection.webp` | Did choosing a method per forest beat the champion? | `research/method-selection.md`, section 9 | `stills/src/method_selection.py` (checks the note signatures and parses the signed tables), reads from `fetch_selection.py` in `data/selection/` (7 November 2026 16:20 UTC) |
| `stills/experiment-chain.webp` | In what order did one experiment's plan, record and result enter the log, and do they cite each other? | `research/program.md`, section 2 | `stills/src/experiment_chain.py` (checks signatures, the CBOR hash, plan citations and three inclusion proofs), reads from `fetch_experiment.py` in `data/experiment/` (7 November 2026 16:26 UTC) |
| `stills/citation-cost.webp` | How many characters does it cost to carry N facts as values, tokens or one bundle? | `paper-section-statistics-and-threats.md`, section 1 | `stills/src/citation_cost.py` (checks each token against its grammar and that the descriptor token resolves), reads from `fetch_cost.py` in `data/cost/`, `data/glossary/` and `data/bundle/` (7 November 2026) |

Each film also has a WebM copy and a `-poster.webp` file with the same name. The `fetch_*.py` scripts in
`stills/src/` make the live reads. The drawing scripts read only the saved responses in `stills/src/data/`.

On 7 November 2026 at about 11:30 UTC, new live reads gave the same numbers as five stills:

| Still | Live read | Result |
|---|---|---|
| `recall-one-cell.webp` | `POST /v1/recall` at `defi.zb610.tAhU.hejI` | Same 8 facts, values and dates |
| `similar-places.webp` | `POST /v1/find_similar`, key Dubai, k 8 | Same 7 scores, 0.81 to 0.77 |
| `ndvi-two-seasons.webp` | `POST /v1/memory_token/resolve` on the series token | Same 17 clear scenes, 8 left out, 2200 pixels |
| `guard-check.webp` | `POST /v1/memory_token/resolve` on the cited fact | Same elevation, 1069.01 m |
| `ask-trail.webp` | `POST /v1/ask`, q `what is the NDVI in Central Park, New York?`, read at 12:01 UTC | Same answer, 0.79, and the same 3 fact ids |

The `ask-trail.webp` result depends on the exact `q` string. At 12:01 UTC the shorter
`what is the NDVI in Central Park?` resolved to a different place label and cell. It gave NDVI 0.75 and 1 fact
id.

Values that come from weather or new scenes can change after this date. Run the `fetch_*.py` script again
before you reuse a number.

## Draw an image again

You need Python 3 with Playwright (Chromium) and Pillow, `ffmpeg` with libx264 and libvpx-vp9, and `xvfb-run` for
films. `recall_cell.py` and `hero.py` also need `pyproj`. Run these commands from the repository root.

Draw a still from its saved data. The output is byte-identical to the committed file, for example for
`guard-check.webp` on 7 November 2026:

    python docs/media/stills/src/guard_check.py /tmp/guard-check.webp

Check that the stills and films still match emem.dev. This command resolves every fact and bundle token in their
saved data and compares each live value with the saved copies. It reads the file and device tokens of
`what-crosses.webp` again. It gets the CBOR bytes of the facts in `wire-forms.webp` and the one-byte film again
and compares them with the saved files. It resolves the descriptor token of `citation-cost.webp` again and
compares the fact id, cell and value with the saved answer. Then it proves offline that the log head in `log-proof.webp` is a prefix
of the current head. It uses only the standard library and makes read-only calls:

    python3 docs/media/stills/src/recheck.py

On 7 November 2026 at 16:45 UTC, 233 tokens resolved and 0 values differed. The tree root, leaf and 107 rows
matched, and `trace_verify` still gave `admit`. The 790 and 743 CBOR bytes of the two facts were equal to the
saved files. The descriptor token gave the same fact id, cell and value. The head of 3,248,898 entries in
`log-proof.webp` extended to the current head of 3,253,869 entries with an 18-hash consistency proof.

Check one film frame before a full render. This command writes a PNG of the frame at 25 s:

    xvfb-run -a -s "-screen 0 1280x1280x24" python docs/media/films/src/film.py \
      docs/media/films/src/emem-fact-to-check.html fact /tmp --size 1080x960 --stills 25

The season film reads `films/src/season-data.js`. `fetch_season.py` gets one true-colour crop per scene of the
saved `field_series` answer. It does not save a crop from another scene. `season_data.py` (needs `pyproj`) writes the
data file from the saved responses.

Remove `--stills 25` and add `--poster 25.5` to render the MP4, the WebM and the poster. A full render takes about
8 minutes on a GPU host. Check the text of a diagram:

    python docs/media/diagrams/src/check_text.py docs/media/diagrams/fact-chain.svg
