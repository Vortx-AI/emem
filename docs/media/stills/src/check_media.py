"""Check the media files and the docs text that describes them.

Run from the repository root:  python3 docs/media/stills/src/check_media.py

1. Sizes: a still or a poster is 300 KB or less, a film file is 2 MB or less.
2. Every still and film in docs/media/ has a row in visual-language.md.
3. Each number of 3 or more digits in a still's alt text occurs in the saved
   responses or the scripts in stills/src/, after rounding to the places the
   alt text shows. Commas and signs are ignored.
4. A paragraph that links a film and says "N seconds" agrees with the film
   length from ffprobe, rounded. Skipped when ffprobe is not installed.
5. README_MEDIA_PROPOSAL.md: each stated byte size equals the file size, each
   named file exists, and each swap row names the image that README.md shows
   on that line today.

Exit status 1 when a check fails.
"""

import pathlib
import re
import shutil
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[4]
MEDIA = ROOT / "docs/media"
SRC = MEDIA / "stills/src"
SKIP_PAGES = {"collaboration-log.md"}

fails = []


def check_sizes():
    limits = [("stills/*.webp", 300_000), ("films/*.webp", 300_000),
              ("films/*.mp4", 2_000_000), ("films/*.webm", 2_000_000)]
    for pattern, limit in limits:
        for path in sorted(MEDIA.glob(pattern)):
            size = path.stat().st_size
            if size > limit:
                fails.append(f"size: {path.relative_to(ROOT)} is {size} bytes, limit {limit}")


def check_listed():
    table = (MEDIA / "visual-language.md").read_text()
    for path in sorted(MEDIA.glob("stills/*.webp")) + sorted(MEDIA.glob("films/*.mp4")):
        name = f"{path.parent.name}/{path.name}"
        if name not in table:
            fails.append(f"listed: {name} has no row in visual-language.md")


def doc_pages():
    for md in sorted((ROOT / "docs").rglob("*.md")):
        if md.name not in SKIP_PAGES:
            yield md, md.read_text()


# Values the still scripts compute from saved responses, so no response holds them.
# Checked by running the named code on 7 November 2026.
COMPUTED = {
    "wire-forms": {"4442", "46482", "22968", "43466", "1460582", "3713482", "790"},  # wire_data.decode()
    "receipt-preimage": {"328"},  # receipt_data.build(): stream_len
    "area-receipts": {"3349"},  # area_receipts.py: fmean of the 16 merged_facts values
    "experiment-chain": {"10154"},  # experiment_chain.py: len(record.cbor)
    "one-place-many-names": {"13.3"},  # one_place.py: metres() from the first to the fifth locate point
    "what-crosses": {"13870"},  # what_crosses.py: sum of event_count over the 4 trace segments
}


def saved_numbers():
    text = ""
    for path in list(SRC.rglob("*.json")) + list(SRC.glob("*.py")):
        text += path.read_text(errors="ignore")
    rounded = set()
    for token in re.findall(r"\d+(?:\.\d+)?(?:[eE][-+]?\d+)?", text.replace(",", "")):
        value = float(token)
        for places in range(7):
            rounded.add((places, round(value, places)))
    return rounded


def check_alt_numbers():
    saved = saved_numbers()
    embed = re.compile(r"!\[([^\]]*)\]\((?:[^)]*/)?stills/([a-z0-9-]+)\.webp\)"
                       r"|<img[^>]*src=\"(?:[^\"]*/)?stills/([a-z0-9-]+)\.webp\"[^>]*alt=\"([^\"]*)\"")
    # The lookarounds skip digits inside a hex digest or a base32 id.
    number = re.compile(r"(?<![0-9A-Za-z])(?:\d{1,3}(?:,\d{3})+(?:\.\d+)?|\d+(?:\.\d+)?)(?![0-9A-Za-z])")
    count = 0
    for md, text in doc_pages():
        for m in embed.finditer(text):
            alt = m.group(1) if m.group(1) is not None else m.group(4)
            still = m.group(2) or m.group(3)
            line = text[: m.start()].count("\n") + 1
            count += 1
            for n in number.findall(alt):
                plain = n.replace(",", "")
                if len(plain.replace(".", "")) < 3 or re.fullmatch(r"20\d\d", plain):
                    continue
                places = len(plain.split(".")[1]) if "." in plain else 0
                # A power of two is a subtree width on the log-proof ladder, computed, not saved.
                power_of_two = places == 0 and int(plain) & (int(plain) - 1) == 0
                if plain in COMPUTED.get(still, ()) or power_of_two:
                    continue
                if (places, round(float(plain), places)) not in saved:
                    fails.append(f"alt: {md.relative_to(ROOT)}:{line} {still}: {n} is in no saved response")
    return count


def check_durations():
    if shutil.which("ffprobe") is None:
        print("durations: skipped, no ffprobe")
        return
    length = {}
    for path in MEDIA.glob("films/*.mp4"):
        out = subprocess.run(["ffprobe", "-v", "error", "-show_entries", "format=duration",
                              "-of", "csv=p=0", str(path)], capture_output=True, text=True, check=True)
        length[path.stem] = round(float(out.stdout))
    film = re.compile(r"(?<![a-z0-9-])films/([a-z0-9-]+?)\.(?:mp4|webm)")
    seconds = re.compile(r"(\d+) seconds")
    for md, text in doc_pages():
        for para in re.split(r"\n\s*\n", text):
            names = set(film.findall(para))
            said = [int(s) for s in seconds.findall(para)]
            if len(names) != 1 or not said:
                continue
            name = names.pop()
            for s in said:
                if s != length.get(name):
                    fails.append(f"duration: {md.relative_to(ROOT)} says {s} seconds for {name}, "
                                 f"the file is {length.get(name)}")


def check_proposal():
    text = (MEDIA / "README_MEDIA_PROPOSAL.md").read_text()
    readme = (ROOT / "README.md").read_text().splitlines()
    for m in re.finditer(r"`(docs/media/[^`]+)` \(([\d,]+) bytes", text):
        path, said = ROOT / m.group(1), int(m.group(2).replace(",", ""))
        size = path.stat().st_size if path.exists() else None
        if size != said:
            fails.append(f"proposal: {m.group(1)} is said to be {said} bytes, the file is {size}")
    for name in sorted(set(re.findall(r"docs/media/[\w./-]+\.(?:webp|mp4|webm|svg|gif)", text))):
        if not (ROOT / name).exists():
            fails.append(f"proposal: {name} does not exist")
    # A swap row names the README line and the image that README.md shows there today.
    for m in re.finditer(r"^\| (\d+), [^|]*\| `(docs/media/readme/[^`]+)`", text, re.M):
        line, now = int(m.group(1)), m.group(2)
        if line > len(readme) or now not in readme[line - 1]:
            fails.append(f"proposal: README.md line {line} does not show {now}")


check_sizes()
check_listed()
check_proposal()
embeds = check_alt_numbers()
check_durations()
for f in fails:
    print(f)
print(f"{embeds} still embeds checked, {len(fails)} failures")
sys.exit(1 if fails else 0)
