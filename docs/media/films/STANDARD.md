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

`docs/media/` is compiled into the server, so the size caps are hard.

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
- in `film.json` when the film has them: `segments`, each `{start, end, role: "evidence" | "illustration",
  scenes: [...]}` (an evidence segment names its scenes), and `voice: {synthetic, model, consent, watermark,
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
and the missing `film.json`, so those five are below this standard until they are cut again.
