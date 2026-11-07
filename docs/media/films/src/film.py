"""Frame-accurate film from a local HTML page that exposes window.film = {duration, fps, ready, seek(t)}.

Method from sugarcane/fieldseg/film.py: Chromium (GPU flags under Xvfb, swiftshader otherwise), seek per frame,
lossless PNG per frame, then ffmpeg. Outputs <out>/<name>.mp4 (H.264), <name>.webm (VP9), <name>-poster.webp.
Needs Playwright for Python with Chromium, and ffmpeg with libx264 and libvpx-vp9.
usage: xvfb-run -a -s "-screen 0 1280x1280x24" python film.py PAGE.html NAME OUT_DIR [--size WxH] [--poster T]
       python film.py PAGE.html NAME OUT_DIR --stills 3 14 25   (PNG stills only, to check the layout)
Example: python film.py emem-fact-to-check.html emem-fact-to-check .. --size 1080x960 --poster 25.5
"""
import argparse
import os
import shutil
import subprocess
from pathlib import Path

ARGS = ["--use-gl=angle", "--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--font-render-hinting=none",
        "--disable-renderer-backgrounding", "--disable-background-timer-throttling"]
ARGS_GPU = ["--use-angle=vulkan", "--enable-features=Vulkan,VulkanFromANGLE,DefaultANGLEVulkan", "--disable-vulkan-surface",
            "--ignore-gpu-blocklist", "--disable-renderer-backgrounding", "--disable-background-timer-throttling",
            "--font-render-hinting=none"]

ap = argparse.ArgumentParser()
ap.add_argument("page")
ap.add_argument("name")
ap.add_argument("out")
ap.add_argument("--size", default="1080x1080")
ap.add_argument("--poster", type=float, default=None)
ap.add_argument("--stills", type=float, nargs="*")
ap.add_argument("--crf-mp4", default="30")
ap.add_argument("--crf-webm", default="44")
a = ap.parse_args()
W, H = map(int, a.size.split("x"))
page = Path(a.page).resolve()
out = Path(a.out).resolve()
out.mkdir(parents=True, exist_ok=True)
frames = out / f"{a.name}-frames"

from playwright.sync_api import sync_playwright  # noqa: E402

with sync_playwright() as p:
    gpu = bool(os.environ.get("DISPLAY"))
    if gpu:
        os.environ.setdefault("VK_ICD_FILENAMES", "/usr/share/vulkan/icd.d/nvidia_icd.json")
    b = p.chromium.launch(args=ARGS_GPU if gpu else ARGS, headless=not gpu)
    pg = b.new_page(viewport={"width": W, "height": H}, device_scale_factor=1)
    errs = []
    pg.on("pageerror", lambda x: errs.append(str(x)))
    pg.goto(page.as_uri(), wait_until="load")
    pg.evaluate("window.film.ready")
    dur, fps = pg.evaluate("window.film.duration"), pg.evaluate("window.film.fps")
    if a.stills:
        for t in a.stills:
            pg.evaluate(f"window.film.seek({t})")
            pg.screenshot(path=str(out / f"{a.name}-still-{t:05.2f}.png"))
        b.close()
        print("stills", a.stills, "errors", errs)
        raise SystemExit(0)
    if frames.exists():
        shutil.rmtree(frames)
    frames.mkdir()
    n = int(round(dur * fps))
    for k in range(n):
        pg.evaluate(f"window.film.seek({k / fps})")
        pg.screenshot(path=str(frames / f"{k:05d}.png"))
    b.close()
print("frames", n, "gpu", gpu, "errors", errs)

enc = ["ffmpeg", "-y", "-loglevel", "error", "-framerate", str(fps), "-i", str(frames / "%05d.png")]
mp4, webm = out / f"{a.name}.mp4", out / f"{a.name}.webm"
subprocess.run(enc + ["-c:v", "libx264", "-profile:v", "high", "-preset", "veryslow", "-crf", a.crf_mp4, "-tune", "animation",
                      "-pix_fmt", "yuv420p", "-movflags", "+faststart", "-an", str(mp4)], check=True)
subprocess.run(enc + ["-c:v", "libvpx-vp9", "-crf", a.crf_webm, "-b:v", "0", "-row-mt", "1", "-deadline", "good", "-cpu-used", "2",
                      "-pix_fmt", "yuv420p", "-an", str(webm)], check=True)
pt = a.poster if a.poster is not None else dur * 0.5
poster = out / f"{a.name}-poster.webp"
subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-i", str(frames / f"{int(pt * fps):05d}.png"), "-vf", "scale=720:-1",
                "-quality", "82", str(poster)], check=True)
shutil.rmtree(frames)
for f in (mp4, webm, poster):
    print(f.name, f.stat().st_size)
