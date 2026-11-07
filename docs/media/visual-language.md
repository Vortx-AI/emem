# Visual language for emem media

Use these rules for every diagram, still and film in `docs/media/`. Then a reader sees one system across the pages.

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
  layout, is about 10 px on a phone. The heading is about 20 px.
- Diagrams in SVG keep the smallest text at 16 px in a 640 px view box, which is about 8.5 px on a phone.
  Body text is 18 px. Break a long line into two lines rather than make the text smaller.
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

## Media that follows these rules

Each row gives the question the image answers and the page that shows it. Paths are relative to `docs/media/`.

| File | Question it answers | Page | Source |
|---|---|---|---|
| `diagrams/fact-chain.svg` | How does a place become a fact that another agent can check? | `start-here.md` | Hand-written SVG |
| `diagrams/two-layers.svg` | Who may write a fact, and who may write a note? | `intro.md` | Hand-written SVG |
| `diagrams/signed-note-write.svg` | What happens to an unsigned note write? | `intro.md` | Hand-written SVG |
| `diagrams/time-filters.svg` | Which time filter selects which fact? | `intro.md` | Hand-written SVG |
| `films/emem-fact-to-check.mp4` | Can another agent check a fact from a token, and does one changed character fail? | `start-here.md` | `films/src/emem-fact-to-check.html` |
| `films/emem-note-write.mp4` | How does a refused note write become an accepted one? | `memory.md` | `films/src/emem-note-write.html` |
| `stills/recall-one-cell.webp` | What does one recall return at one cell? | `agents.md` | `stills/src/recall_cell.py` |
| `stills/similar-places.webp` | Which places look like Dubai from orbit? | `agents.md` | `stills/src/similar_places.py` |
| `stills/ndvi-two-seasons.webp` | How does NDVI change over two seasons on one field? | `agents.md` | `stills/src/ndvi_season.py` |
| `stills/ask-trail.webp` | Which stages are behind one `/v1/ask` answer? | `agents.md` | `stills/src/ask_trail.py` |
| `stills/guard-check.webp` | When does the guard allow a draft and when does it deny it? | `agents.md` | `stills/src/guard_check.py` |
| `stills/log-proof.webp` | Is this fact in the log, and does the log only grow? | `protocol.md` | `stills/src/log_proof.py` |
| `stills/bundle-handoff.webp` | Can a second agent check a bundle of four facts? | `emem-in-depth.md` | `stills/src/bundle_handoff.py` |
| `stills/a2a-exchange.webp` | Can a reader check an A2A answer offline? | `emem-in-depth.md` | `stills/src/a2a_exchange.py` |
| `stills/signed-thread.webp` | Who wrote each note in a signed thread? | `emem-in-depth.md` | `stills/src/signed_thread.py` |

Each film also has a WebM copy and a `-poster.webp` file with the same name. The `fetch_*.py` scripts in
`stills/src/` make the live reads. The drawing scripts read only the saved responses in `stills/src/data/`.

## Draw an image again

You need Python 3 with Playwright (Chromium) and Pillow, `ffmpeg` with libx264 and libvpx-vp9, and `xvfb-run` for
films. `recall_cell.py` also needs `pyproj`. Run these commands from the repository root.

Draw a still from its saved data. The output is byte-identical to the committed file, for example for
`guard-check.webp` on 7 November 2026:

    python docs/media/stills/src/guard_check.py /tmp/guard-check.webp

Check one film frame before a full render. This command writes a PNG of the frame at 25 s:

    xvfb-run -a -s "-screen 0 1280x1280x24" python docs/media/films/src/film.py \
      docs/media/films/src/emem-fact-to-check.html fact /tmp --size 1080x960 --stills 25

Remove `--stills 25` and add `--poster 25.5` to render the MP4, the WebM and the poster. A full render takes about
8 minutes on a GPU host. Check the text of a diagram:

    python docs/media/diagrams/src/check_text.py docs/media/diagrams/fact-chain.svg
