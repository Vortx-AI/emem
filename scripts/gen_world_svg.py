#!/usr/bin/env python3
"""Generate web/art/world-mithila.svg — the homepage map, drawn in-house.

Natural Earth 110m land, equirectangular into a 1200x600 viewBox (so the
homepage's marker overlay maps lat/lng linearly onto the same box), styled
in the site's own tokens with the Mithila frame the rest of the page uses.
The OUTPUT is committed; this script exists so the map is regenerable, not
hand-maintained. Source: naturalearthdata.com (public domain), via the
martynafford/natural-earth-geojson mirror.
"""
from __future__ import annotations

import json
import sys
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
SRC = "https://raw.githubusercontent.com/martynafford/natural-earth-geojson/master/110m/physical/ne_110m_land.json"
W, H = 1200, 600


def xy(lng: float, lat: float) -> tuple[float, float]:
    return (lng + 180.0) / 360.0 * W, (90.0 - lat) / 180.0 * H


def ring_path(ring: list) -> str:
    pts = []
    last = None
    for lng, lat in ring:
        x, y = xy(lng, lat)
        p = (round(x, 1), round(y, 1))
        if p != last:
            pts.append(p)
            last = p
    if len(pts) < 3:
        return ""
    d = f"M{pts[0][0]} {pts[0][1]}"
    for x, y in pts[1:]:
        d += f"L{x} {y}"
    return d + "Z"


def main() -> int:
    cache = REPO / "var" / "ne_110m_land.json"
    if cache.exists():
        gj = json.loads(cache.read_text())
    else:
        with urllib.request.urlopen(SRC, timeout=60) as r:
            raw = r.read()
        gj = json.loads(raw)

    paths = []
    for feat in gj["features"]:
        geom = feat["geometry"]
        polys = geom["coordinates"] if geom["type"] == "MultiPolygon" else [geom["coordinates"]]
        for poly in polys:
            for ring in poly:
                d = ring_path(ring)
                if d:
                    paths.append(d)
    land = "".join(paths)

    svg = f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" role="img" aria-label="The world, drawn in the site's own hand: land in ink on paper, framed the way every emem panel is framed. Markers overlaid by the page show the places the agents' signed messages cite.">
<style>
:root{{--paper:#f2f0ec;--land:#e2ded2;--coast:#8f8b82;--rule:#a7a49e;--ink:#171613;--accent:#326234;--ochre:#d59a2a;--indigo:#344a8c}}
@media (prefers-color-scheme: dark){{:root{{--paper:#151411;--land:#26241f;--coast:#565248;--rule:#4a4742;--ink:#e9e8e4;--accent:#80cd82;--ochre:#b98a2f;--indigo:#6c82c4}}}}
</style>
<rect width="{W}" height="{H}" fill="var(--paper)"/>
<defs>
<pattern id="sea" width="14" height="14" patternUnits="userSpaceOnUse">
  <circle cx="2" cy="2" r="0.7" fill="var(--rule)" opacity="0.35"/>
</pattern>
</defs>
<rect width="{W}" height="{H}" fill="url(#sea)"/>
<path d="{land}" fill="var(--land)" stroke="var(--coast)" stroke-width="0.7" fill-rule="evenodd"/>
<!-- the Mithila frame: the double rule and corner gems every panel wears -->
<rect x="3" y="3" width="{W-6}" height="{H-6}" fill="none" stroke="var(--ink)" stroke-width="2"/>
<rect x="9" y="9" width="{W-18}" height="{H-18}" fill="none" stroke="var(--rule)" stroke-width="1"/>
<g stroke="var(--ink)" stroke-width="1.2">
  <rect x="-1" y="-1" width="14" height="14" transform="rotate(45 6 6)" fill="var(--ochre)"/>
  <rect x="{W-13}" y="-1" width="14" height="14" transform="rotate(45 {W-6} 6)" fill="var(--indigo)"/>
  <rect x="-1" y="{H-13}" width="14" height="14" transform="rotate(45 6 {H-6})" fill="var(--indigo)"/>
  <rect x="{W-13}" y="{H-13}" width="14" height="14" transform="rotate(45 {W-6} {H-6})" fill="var(--ochre)"/>
</g>
</svg>
"""
    out = REPO / "web" / "art" / "world-mithila.svg"
    out.write_text(svg)
    print(f"wrote {out}: {len(svg)//1024} KB, {len(paths)} rings")
    return 0


if __name__ == "__main__":
    sys.exit(main())
