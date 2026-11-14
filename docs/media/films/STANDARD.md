# The standard a film must meet to go on an emem page

emem takes a film onto emem.dev, vortx.ai, the README, the docs or the GitHub examples only when it passes
every gate below. A film that is well made but says nothing about emem is not taken. A film about emem that
fails a technical gate is sent back with the report.

## 1. It must be about emem

- It shows emem doing its work: a place resolved to a cell, a signed fact read, a claim checked against it, a
  token handed on or verified. It is not satellite footage with a logo on it.
- It asks one question and ends on an honest verdict: TRUE, NOT TRUE, CAN'T TELL or VOID, or it is a plain
  explainer (NONE). A film that cannot fail its own claim does not show a check.
- Every number on screen comes from a signed fact or a saved, hashed session, and `film.json` names it. Each
  token resolves on emem.dev, and the value on screen is the signed value, rounded no further than it shows.
- A number that no single fact holds, such as a count over many cells, is cited one of two ways:
  - Prefer `emem_derive` over its parent facts, where the responder can compute it again.
  - Otherwise cite it as one unit of the author's signed pointer.v1 index over a facts card: `emem:tree:<cid>#row=<i>`.
    The unit names the value, the method and the scenes, so anyone can count again. The screen says whose count it
    is. It is the author's signed claim, not an emem fact.
- An explainer (verdict NONE) may show values that a public script in this repository computes offline. Such a
  claim names the script (`reproduce`), the file it writes (`output`) and that file's `output_sha256`, in place
  of a token.
- A share or count over an area, such as "72 % of the park reads as snow", is cited as an `emem:bundle` of the
  signed per-cell facts for one date, with the rule (`band`, `op`, `threshold`) and the statistic. The gate
  resolves every member and counts again. Cells with no clear reading are not in the set, so the screen states
  the denominator ("of 46 cells with a clear reading"). `emem_derive` cannot do this: its operations that the
  responder recomputes are delta, mean and sum.
- Over more than 256 cells, cite the rasters instead: the `emem:raster` tokens of one scene over the box, from
  `POST /v1/band_raster`, with the index, the rule and the statistic. The gate checks each artifact's bytes
  against its cid, computes the index on the finest grid and counts the pixels again. Do not split a large area
  into many bundles: each member is a signed read on emem.dev, and thousands of them load the service for
  everyone. Where no raster is served, the author's tree unit with "count: <author>" stays the citation.
- A count can require several rules at once, and its denominator can be a mask from another pass over the same
  box: ice on a January pass inside the water of a June pass. The screen says the rule, not a word the rule does
  not measure. NDSI of 0.4 or more with a bright near-infrared band is the usual snow test (open water is dark
  in near-infrared, most cloud is bright in short-wave infrared), and it reads "snow or ice". "Frozen" is a
  reading of it, said in the voice as one, and it stays out of the stamp line.
- A verdict that rests on no pass, such as "no optical pass for 98 days", is a claim. emem's dated read says
  "nothing is signed for" a day, and that does not say the satellite did not pass: over Santa Claus Village it
  passed on 1, 5 and 8 November 2024 under 98 to 100 % cloud. The claim names the passes on each side and the
  gate searches both public catalogues for any pass between them.
- A pass that the catalogue lists under cloud can be shown: "the satellite passed; it saw cloud". The picture is
  the catalogue's own preview of that item, so it is an illustration, not evidence, and no verdict rests on it.
  Its segment's `source` names the item URL (`stac_item`). Its label is a pass label, and a cloud figure on it is
  the catalogue's figure for the whole tile, checked against the item: the screen says "of the tile", never
  that the place was under cloud.
- A verdict that joins an observation and the weather since, such as whether the snow seen on 31 October lasted
  to 3 December, is a `fused` claim on a chain from `POST /v1/persist`. The gate runs the rule again on the
  series the chain read. The screen says it is a model reading, and it shows both melt readings when they
  disagree.
- A picture from a map provider (a high-resolution base image, a 3D city) is an illustration. While it shows,
  the screen says "drawing of data" or "map image", and the provider's attribution is on the picture. Its segment
  in `film.json` names the provider, the product, the licence and the capture date, or "unknown" when the
  provider does not give one. The provider's terms must permit use in a film. An open orthophoto under a stated
  licence is better than a commercial layer where one exists. No verdict rests on it.
- Footage from a stock site is an illustration too. Its segment's `source` names the site, the contributor, the
  licence, the page and the date the page gives (`posted`). The screen says "posted", not "filmed", unless the
  page states when it was filmed. The date is a label, not a checked claim: the gate cannot read a page that
  changes or refuses a script. When the film asks about that day, the question says "the day it was posted".
- A label that names the picture on screen, such as "Sentinel-2, 30 May 2026, 11:06 UTC", is a claim. A wrong
  date misleads as much as a wrong count. The claim names the catalogue item (`stac_item`) and what the screen
  shows (`shown_utc`), and the gate reads the item's time.
- A conversion, such as "18 pitches of 105 x 68 m" from a count of pixels, names the claim it converts (`of`), its
  factors and its formula in words. The gate converts the signed value, not the rounded one on screen. The size
  of the pitch, or of any unit a reader would not know, is on screen.
- A homepage loop asks no question (verdict NONE), but it still shows emem at work. At least the place's cell
  outline with its cell64, and one signed value read there, each readable for 2 s or more, with that fact's
  `emem:fact` token in `film.json`. A loop of pictures with no cell and no signed value is footage with a logo.
- It says what the check does not show. A signature says who signed, not that a claim is true.
- It shows nothing private: no farm or field outline, no client or partner name, no coordinates of a person.
- A pixel that decides the verdict is an untouched source pixel from a named pass, stretched only where it says
  so. A drape, a model, an interpolated frame or a composite is an illustration: the screen says "drawing of
  data" while it is up, and the verdict stamp never sits on one. In a time-lapse, every frame is a real pass with
  its date on it.
- A synthetic voice says so on screen at its first line, and in the captions. Its watermark check is a claim in
  `film.json`.
- A character never shows a feeling about a verdict, and is never on screen with the evidence. Its seal moves only
  when the real check returns. It stays out of homepage and verification cuts.
- No prompt to guess before the check. Say what would decide it instead.

## 2. Technical gates, measured by `tools/film_qc.py`

| Gate | Web cut (16:9) | Homepage loop |
|---|---|---|
| Video | H.264 High, yuv420p, 1280x720 (or 1920x1080), constant 24, 25 or 30 fps | the same |
| Colour | tagged BT.709 primaries, transfer and matrix, limited range | the same |
| Keyframes | every 2 s or less | the same |
| Container | moov before mdat (+faststart) | the same |
| Size | 1.5 MB at most | 600 kB at most, 3 to 6.5 s |
| Audio | AAC-LC 48 kHz, -16 LUFS +-1 integrated, true peak -1.5 dBTP or lower | none |
| Loop | | first and last frames match (PSNR 30 dB or more) |
| Flashes | 3 or fewer in any 1 s (WCAG 2.3.1) | the same |
| Warnings | black 0.5 s or longer, frozen 4 s or longer | the same |

A figure for a docs page only may be 1080x960 or 1080x1080 (`film_qc.py --docs`). Every other gate is the same,
and it never goes on a homepage or into a web cut. `docs/media/` is compiled into the server, so the size caps are
hard.

x264 writes colour tags into the stream only through its own parameters:
`-x264-params colorprim=bt709:transfer=bt709:colormatrix=bt709:range=tv`, with `-g` at twice the frame rate,
`-sc_threshold 0`, `-pix_fmt yuv420p` and `-movflags +faststart`.

## 3. What a film arrives with

One folder per film, `docs/media/films/<id>/`:

- `film.mp4`, plus `loop.mp4` for a homepage loop;
- `poster.webp`, a frame that states the claim, for readers who set reduced motion;
- `captions.vtt` for a voiced cut, with lines of 42 characters at most, each on screen for 1 to 6 s;
- `film.json`: `id`, `version`, `verdict`, `privacy_check: true`, and `claims`, one entry per claim on screen,
  each with `text`, `token` (an `emem:` token) and, for a value, the `value` shown and the `session_sha256` of
  the saved session that checked it;
- a pass label in `claims` has `text`, `stac_item` (the item's URL on planetarycomputer.microsoft.com or
  earth-search.aws.element84.com) and `shown_utc` (`2026-05-30` or `2026-05-30T11:06Z`) in place of a token. The
  item's time, cut to that precision as a clock shows it, must equal it, and an evidence segment must list the
  item id;
- a share or count in `claims` has `text`, `token` (`emem:bundle:`), `rule` (`{band, op, threshold}`, op one of
  `ge`, `gt`, `le`, `lt`), `statistic` (`count`, `share` or `percent`), the `value` shown and, when the screen
  shows it, `total`. Every member must resolve, read the rule's band on one date and carry no erratum;
- a raster share or count in `claims` has `text`, `rasters` (the `emem:raster:` tokens, one per band), `index`
  (`value`, `ndvi`, `ndwi`, `mndwi` or `ndsi`), `rule` (`{op, threshold}`), `statistic`, the `value` shown and,
  when shown, `total`. Indices are computed on harmonised DN, where the reflectance scale cancels; a `value` rule
  on one band is in harmonised DN (reflectance x 10,000). The check needs the `blake3` module;
- a count with several rules has `rules`, a list of `{index, op, threshold}` (a `value` rule adds `band`), in place
  of `index` and `rule`. A mask adds `mask`: `{rasters, index, rule}` or `{rasters, rules}`, from one scene over
  the same box. The rules must read exactly the rasters given;
- an absence in `claims` has `text`, `absence` (`{collection, lng, lat, after, before}`, the days of the passes on
  each side, and `max_cloud` for "no clear pass") and, when the screen shows the gap in days, `value`;
- a pass label may add `cloud`, the catalogue's cloud figure shown, which must equal the item's
  `eo:cloud_cover`; the text says it is the tile's. The catalogue's preview of a pass is an illustration segment
  whose `source.stac_item` is the item URL;
- a chain verdict in `claims` has `text`, `token` (the chain's `emem:fact`) and `fused`: `{verdict, field,
  value}`, the verdict shown and a number shown with the JSON pointer of its field in the record
  (`/snowfall/total_cm`). The check needs the `blake3` module;
- a conversion in `claims` has `text`, `of` (the index of the claim it converts), `multiply`, `divide`,
  `formula` and the `value` shown;
- in `film.json` when the film has them: `segments`, each `{start, end, role: "evidence" | "illustration",
  scenes: [...]}` (an evidence segment names its scenes; an illustration from a map provider adds `source`:
  `{provider, product, licence, captured}`), and `voice: {synthetic, model, consent, watermark,
  watermark_check_sha256}`.

## 4. How to run the gate

```sh
docker build -q -t emem-film-qc -f tools/film_qc.Dockerfile tools
docker run --rm -v "$PWD":/w:ro emem-film-qc python3 /w/tools/film_qc.py \
  /w/docs/media/films/<id>/film.mp4 --json /w/docs/media/films/<id>/film.json
```

It prints every gate with what it measured, and exits 0 when the film is accepted. Then check the claims
against emem.dev:

```sh
python3 tools/film_claims.py docs/media/films/<id>/film.json
```

It resolves each `emem:fact` and compares the value. For each `emem:tree` unit it runs `tree_proof.py check` on
the row, the note named in `index_note` and the source the note indexes, then compares the unit's `- value:` line.

On 12 November 2026 the gate accepted a test pattern encoded to the spec and rejected a 4 s clip that flashed
11 times a second. It rejected all five older films in this folder on colour tags, frame size, keyframe interval
and the missing `film.json`. They were rendered again the same day with `src/film.py`, which now converts with the
BT.709 matrix, tags BT.709 and keys every 2 s. Each has a `<name>.film.json`. They are docs figures (`--docs`).

On 13 November the claim check learnt pass labels and conversions. A test of ten claims passed the four true
ones (a signed count, its area, its pitches and a real Sentinel-2 pass) and failed the six controls: a wrong
area, a wrong minute, a wrong day in the text, a conversion of itself, a catalogue outside the two, and a wrong
count.

The same day it learnt shares and counts over a bundle. A test over three signed NDSI cells of Victoria Park on
14 December 2022 passed four true claims (100 %, 2 of 3, 67 % and 0.67) and failed five controls: 66 %, a bundle
of which only 3 of 6 members resolved, a rule on another band, a wrong total, and an operator outside the four.

It then learnt rasters. Over a 41 x 36 px box of Victoria Park on 14 December 2022 (B03 at 10 m, B11 at 20 m),
it counted 1,059 of 1,476 pixels at NDSI 0.4 or more, the same as an independent count by other arithmetic. It
passed three true claims and failed five controls: one pixel more, an index the rasters cannot make, a wrong
total, a single-band rule over two rasters, and a token that is not a raster.

Later on 13 November it learnt rules, masks and absences. Over the same box, with B08 of the same December pass
and B03 and B08 of a June pass from another tile, it counted 771 of 1,476 pixels at NDSI 0.4 or more with B08
over 4000, and 292 of the 448 pixels a June NDWI under -0.4 selects, the same as an independent count. Over
Santa Claus Village it passed "no pass for 87 days" (8 November 2024 to 3 February 2025) and "no pass under 50 %
cloud for 95 days" (31 October to 3 February). It failed nine controls, among them the 98 days of a draft film
(seven passes between), a gap to 6 February that missed a clear 3 February pass only one catalogue lists, a
bracket day with no pass, one pixel more, and a mask whose rule needs a band it does not have.
