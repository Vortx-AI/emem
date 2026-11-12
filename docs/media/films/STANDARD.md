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
  token resolves on emem.dev, and the value on screen is the signed value.
- It says what the check does not show. A signature says who signed, not that a claim is true.
- It shows nothing private: no farm or field outline, no client or partner name, no coordinates of a person.

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
  the saved session that checked it.

## 4. How to run the gate

```sh
docker build -q -t emem-film-qc -f tools/film_qc.Dockerfile tools
docker run --rm -v "$PWD":/w:ro emem-film-qc python3 /w/tools/film_qc.py \
  /w/docs/media/films/<id>/film.mp4 --json /w/docs/media/films/<id>/film.json
```

It prints every gate with what it measured, and exits 0 when the film is accepted. Then resolve each token in
`film.json` with `POST /v1/memory_token/resolve` and compare its `value` with the `value` in the claim.

On 12 November 2026 the gate accepted a test pattern encoded to the spec and rejected a 4 s clip that flashed
11 times a second. It rejected all five older films in this folder on colour tags, frame size, keyframe interval
and the missing `film.json`, so those five are below this standard until they are cut again.
