#!/usr/bin/env python3
"""Acceptance gate for films offered to emem. Runs inside the emem-film-qc image (ffmpeg + python3).

    film_qc.py FILM.mp4 [--loop] [--json FILM.json] [--max-mb N]

A film is ACCEPTED only when every gate passes. Technical gates are measured here. The emem gates read
film.json: every on-screen claim must carry an emem token, and the token must resolve (checked by the caller
against emem.dev, because this container has no network). Exit 0 = accepted, 1 = rejected, 2 = bad input.
"""
import json, re, subprocess, sys
from pathlib import Path


def run(cmd):
    return subprocess.run(cmd, capture_output=True, text=True)


def probe(path):
    r = run(["ffprobe", "-v", "error", "-show_streams", "-show_format", "-of", "json", path])
    return json.loads(r.stdout)


def keyframe_gaps(path):
    r = run(["ffprobe", "-v", "error", "-select_streams", "v:0", "-skip_frame", "nokey", "-show_entries",
             "frame=pts_time", "-of", "csv=p=0", path])
    t = [float(x.strip(",")) for x in r.stdout.split() if x.strip(",") not in ("", "N/A")]
    return [b - a for a, b in zip(t, t[1:])], t


def faststart(path):
    b = Path(path).read_bytes()[:1 << 20]
    m, d = b.find(b"moov"), b.find(b"mdat")
    return m != -1 and (d == -1 or m < d)


def loudness(path):
    r = run(["ffmpeg", "-hide_banner", "-nostats", "-i", path, "-filter_complex", "ebur128=peak=true", "-f", "null", "-"])
    i = re.findall(r"I:\s+(-?[\d.]+) LUFS", r.stderr)
    tp = re.findall(r"Peak:\s+(-?[\d.]+) dBFS", r.stderr)
    return (float(i[-1]) if i else None), (float(tp[-1]) if tp else None)


def luma_series(path):
    r = run(["ffmpeg", "-hide_banner", "-nostats", "-i", path, "-vf", "signalstats,metadata=print:key=lavfi.signalstats.YAVG",
             "-f", "null", "-"])
    t = [float(x) for x in re.findall(r"pts_time:([\d.]+)", r.stderr)]
    y = [float(x) for x in re.findall(r"YAVG=([\d.]+)", r.stderr)]
    return list(zip(t, y))


def flashes(series):
    """WCAG 2.3.1, approximated: a flash is a pair of opposing luminance swings of 10 % of full range or more.
    Fail when any 1 s window holds more than 3 flashes."""
    swings, prev = [], None
    for t, y in series:
        if prev is not None and abs(y - prev[1]) >= 25.5:
            swings.append((t, 1 if y > prev[1] else -1))
        prev = (t, y)
    flashes_at = [b[0] for a, b in zip(swings, swings[1:]) if a[1] != b[1]]
    worst = 0
    for i, t in enumerate(flashes_at):
        worst = max(worst, sum(1 for u in flashes_at[i:] if u - t < 1.0))
    return worst


def detect(path, flt, key):
    r = run(["ffmpeg", "-hide_banner", "-nostats", "-i", path, "-vf", flt, "-an", "-f", "null", "-"])
    return re.findall(key, r.stderr)


def loop_seam(path, dur):
    r = run(["ffmpeg", "-hide_banner", "-nostats", "-i", path, "-i", path, "-filter_complex",
             f"[0:v]trim=end_frame=1,setpts=PTS-STARTPTS[a];[1:v]trim=start={max(0.0, dur - 0.08)},setpts=PTS-STARTPTS,trim=end_frame=1[b];[a][b]psnr",
             "-f", "null", "-"])
    m = re.search(r"average:([\d.inf]+)", r.stderr)
    return float(m.group(1)) if m and m.group(1) != "inf" else (99.0 if m else None)


def main():
    a = sys.argv[1:]
    if not a:
        print(__doc__); return 2
    path, loop = a[0], "--loop" in a
    max_mb = float(a[a.index("--max-mb") + 1]) if "--max-mb" in a else (0.6 if loop else 1.5)
    meta = json.loads(Path(a[a.index("--json") + 1]).read_text()) if "--json" in a else None
    p = probe(path)
    v = next((s for s in p["streams"] if s["codec_type"] == "video"), None)
    au = [s for s in p["streams"] if s["codec_type"] == "audio"]
    dur = float(p["format"].get("duration", 0))
    size_mb = int(p["format"]["size"]) / 1e6
    res = []

    def gate(name, ok, got, want, hard=True):
        res.append({"gate": name, "pass": bool(ok), "got": got, "want": want, "hard": hard})

    if not v:
        print(json.dumps({"accepted": False, "why": "no video stream"})); return 1
    gate("codec", v["codec_name"] == "h264" and v.get("profile") in ("High", "Main"), f"{v['codec_name']} {v.get('profile')}", "h264 High")
    gate("pixel format", v.get("pix_fmt") == "yuv420p", v.get("pix_fmt"), "yuv420p")
    tags = (v.get("color_primaries"), v.get("color_transfer"), v.get("color_space"))
    gate("colour tags", tags == ("bt709", "bt709", "bt709"), "/".join(str(x) for x in tags), "bt709/bt709/bt709")
    gate("range", v.get("color_range") in ("tv", None), v.get("color_range"), "tv (limited)")
    gate("size", (v["width"], v["height"]) in ((1280, 720), (1920, 1080)), f"{v['width']}x{v['height']}", "1280x720")
    gate("constant frame rate", v.get("r_frame_rate") == v.get("avg_frame_rate") and v.get("r_frame_rate") in ("24/1", "25/1", "30/1", "24000/1001", "30000/1001"),
         f"r={v.get('r_frame_rate')} avg={v.get('avg_frame_rate')}", "24, 25 or 30, constant")
    gaps, keys = keyframe_gaps(path)
    gate("keyframe interval", bool(keys) and (max(gaps) if gaps else dur) <= 2.1, round(max(gaps), 2) if gaps else round(dur, 2), "<= 2.0 s")
    gate("faststart", faststart(path), faststart(path), "moov before mdat")
    gate("file size", size_mb <= max_mb, round(size_mb, 3), f"<= {max_mb} MB")
    if loop:
        gate("loop is muted", not au, len(au), "no audio stream")
        gate("loop length", 3.0 <= dur <= 6.5, round(dur, 2), "3 to 6.5 s")
        seam = loop_seam(path, dur)
        gate("seamless loop", seam is not None and seam >= 30, seam, "first vs last frame PSNR >= 30 dB")
    elif au:
        s = au[0]
        gate("audio codec", s["codec_name"] == "aac" and s.get("sample_rate") == "48000", f"{s['codec_name']} {s.get('sample_rate')}", "aac 48000")
        il, tp = loudness(path)
        gate("loudness", il is not None and -17.0 <= il <= -15.0, il, "-16 LUFS +- 1")
        gate("true peak", tp is not None and tp <= -1.5, tp, "<= -1.5 dBTP")
    series = luma_series(path)
    worst = flashes(series)
    gate("flashes (WCAG 2.3.1)", worst <= 3, worst, "<= 3 in any 1 s")
    black = detect(path, "blackdetect=d=0.5:pix_th=0.10", r"black_duration:([\d.]+)")
    gate("black segments", not black, black, "none >= 0.5 s", hard=False)
    frozen = detect(path, "freezedetect=n=0.003:d=4", r"freeze_duration: ([\d.]+)")
    gate("frozen segments", not frozen, frozen, "none >= 4 s", hard=False)
    # emem gates: what makes a film meaningful for emem, not only well made
    if meta is None:
        gate("film.json", False, None, "present: id, version, claims[] with emem tokens")
    else:
        claims = meta.get("claims") or []
        gate("claims carry emem tokens", bool(claims) and all(str(c.get("token", "")).startswith("emem:") for c in claims),
             len(claims), "every on-screen claim has an emem: token")
        gate("verdict stated", str(meta.get("verdict", "")).upper() in ("TRUE", "NOT TRUE", "CAN'T TELL", "VOID", "NONE"),
             meta.get("verdict"), "TRUE, NOT TRUE, CAN'T TELL, VOID or NONE (an explainer)")
        gate("privacy check recorded", meta.get("privacy_check") is True, meta.get("privacy_check"), "true")
    hard_fail = [r for r in res if r["hard"] and not r["pass"]]
    print(json.dumps({"film": Path(path).name, "duration_s": round(dur, 2), "size_mb": round(size_mb, 3),
                      "accepted": not hard_fail, "gates": res}, indent=1))
    return 0 if not hard_fail else 1


if __name__ == "__main__":
    sys.exit(main())
