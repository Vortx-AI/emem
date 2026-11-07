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
- Stills are laid out at 800 px and rendered at 1080 / 800 scale. Then the smallest text, 22 px in the
  layout, is about 10 px on a phone. The heading is about 20 px.
- Diagrams in SVG keep the smallest text at 13 px in a 640 px view box.

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
